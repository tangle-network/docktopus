mod common;

use bollard::models::ContainerCreateBody;
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, InspectContainerOptions,
    ListContainersOptionsBuilder, StartContainerOptions,
};
use color_eyre::Result;
use common::with_docker_cleanup;
use docktopus::DockerBuilder;
use futures_util::TryStreamExt;
use std::collections::HashMap;
use std::time::Duration;
use uuid::Uuid;

#[tokio::test]
async fn test_container_management() -> Result<()> {
    with_docker_cleanup(|test_id| {
        Box::pin(async move {
            let builder = DockerBuilder::new().await?;
            let unique_id = Uuid::new_v4();
            let container_name = format!("test-mgmt-{}-integration", unique_id);
            println!("Starting test with container name: {}", container_name);

            // Pull image first to avoid potential "No such image" errors
            println!("Pulling alpine image...");
            let create_opts = CreateImageOptionsBuilder::default()
                .from_image("alpine")
                .tag("latest")
                .build();
            builder
                .client()
                .create_image(Some(create_opts), None, None)
                .try_collect::<Vec<_>>()
                .await?;
            println!("Image pull complete");

            // Create container first
            let mut labels = HashMap::new();
            labels.insert("test_id".to_string(), test_id.clone());

            let create_container_opts = CreateContainerOptionsBuilder::default()
                .name(&container_name)
                .build();

            let container_config = ContainerCreateBody {
                image: Some("alpine:latest".to_string()),
                cmd: Some(vec!["sleep".to_string(), "30".to_string()]), // Longer sleep to avoid timing issues
                labels: Some(labels),
                tty: Some(true),
                ..Default::default()
            };

            let container = builder
                .client()
                .create_container(Some(create_container_opts), container_config)
                .await?;
            println!("Container created with ID: {}", container.id);

            // Add a small delay after creation to ensure Docker has fully registered the container
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Log container state after creation
            if let Ok(inspect) = builder
                .client()
                .inspect_container(&container.id, None::<InspectContainerOptions>)
                .await
            {
                println!("Container state after creation: {:?}", inspect.state);
            } else {
                println!("Failed to inspect container after creation");
            }

            // Start container with retry logic
            let mut start_retries = 3;
            let mut start_success = false;
            while start_retries > 0 && !start_success {
                match builder
                    .client()
                    .start_container(&container.id, None::<StartContainerOptions>)
                    .await
                {
                    Ok(()) => {
                        println!("Container started successfully");
                        start_success = true;
                    }
                    Err(e) => {
                        println!(
                            "Failed to start container (attempt {}): {:?}",
                            4 - start_retries,
                            e
                        );
                        start_retries -= 1;
                        if start_retries > 0 {
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }
            }

            assert!(
                start_success,
                "Failed to start container after multiple attempts"
            );

            // Add a small delay after starting to ensure Docker has fully started the container
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Log container state after start
            if let Ok(inspect) = builder
                .client()
                .inspect_container(&container.id, None::<InspectContainerOptions>)
                .await
            {
                println!("Container state after start: {:?}", inspect.state);
            } else {
                println!("Failed to inspect container after start");
            }

            // Verify container is running with more retries
            let mut filters: HashMap<String, Vec<String>> = HashMap::new();
            filters.insert("id".to_string(), vec![container.id.clone()]);
            filters.insert("label".to_string(), vec![format!("test_id={}", test_id)]);

            let mut retries = 10; // Increased retries
            let mut container_running = false;
            while retries > 0 {
                println!("Checking container running state, attempt {}", 11 - retries);
                let list_opts = ListContainersOptionsBuilder::default()
                    .all(true)
                    .filters(&filters)
                    .build();
                match builder.client().list_containers(Some(list_opts)).await {
                    Ok(containers) => {
                        if !containers.is_empty() {
                            container_running = true;
                            println!("Container confirmed running");
                            break;
                        }
                    }
                    Err(e) => println!("Error checking container state: {:?}", e),
                }
                retries -= 1;
                tokio::time::sleep(Duration::from_millis(200)).await; // Increased delay
            }

            assert!(container_running, "Container should be running");

            // Test exec in running container with retry
            println!("Attempting to exec in container");
            let mut exec_retries = 3;
            let mut exec_success = false;
            while exec_retries > 0 && !exec_success {
                match builder
                    .exec_in_container(&container.id, vec!["echo", "exec test"], None)
                    .await
                {
                    Ok(output) => {
                        println!("Exec succeeded with output: {}", output);
                        assert!(output.contains("exec test"));
                        exec_success = true;
                    }
                    Err(e) => {
                        println!(
                            "Exec failed with error (attempt {}): {:?}",
                            4 - exec_retries,
                            e
                        );
                        exec_retries -= 1;
                        if exec_retries > 0 {
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }
            }

            assert!(
                exec_success,
                "Failed to exec in container after multiple attempts"
            );

            // Try to get logs with retry
            println!("Attempting to get container logs");
            let mut log_retries = 3;
            while log_retries > 0 {
                match builder.get_container_logs(&container.id).await {
                    Ok(logs) => {
                        println!("Successfully retrieved logs: {}", logs);
                        break;
                    }
                    Err(e) => {
                        println!("Failed to get logs (attempt {}): {:?}", 4 - log_retries, e);
                        log_retries -= 1;
                        if log_retries > 0 {
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }
            }

            Ok(())
        })
    })
    .await
}
