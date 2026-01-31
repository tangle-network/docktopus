mod common;

use bollard::query_parameters::{
    CreateImageOptionsBuilder, InspectContainerOptions, ListContainersOptionsBuilder,
};
use common::{is_docker_running, with_docker_cleanup};
use docktopus::DockerBuilder;
use docktopus::config::{DockerCommand, DockerfileConfig};
use futures_util::TryStreamExt;
use std::collections::HashMap;
use std::time::Duration;

#[tokio::test]
async fn test_dockerfile_deployment() -> color_eyre::Result<()> {
    with_docker_cleanup(|test_id| {
        Box::pin(async move {
            if !is_docker_running() {
                println!("Skipping test: Docker is not running");
                return Ok(());
            }

            let builder = DockerBuilder::new().await?;
            let network_name = format!("test-network-{}", test_id);

            let mut network_labels = HashMap::new();
            network_labels.insert("test_id".to_string(), test_id.to_string());

            // Create network with retry mechanism
            builder
                .create_network_with_retry(
                    &network_name,
                    3,
                    Duration::from_secs(2),
                    Some(network_labels),
                )
                .await?;

            // Pull alpine image first
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

            // Create a simple test Dockerfile config
            let config = DockerfileConfig {
                base_image: "alpine:latest".to_string(),
                commands: vec![
                    DockerCommand::Run {
                        command: "echo 'test' > /test.txt".to_string(),
                    },
                    DockerCommand::Label {
                        labels: {
                            let mut labels = HashMap::new();
                            labels.insert("test_id".to_string(), test_id.to_string());
                            labels
                        },
                    },
                    DockerCommand::Cmd {
                        command: vec!["sleep".to_string(), "30".to_string()], // Keep container running
                    },
                ],
            };

            let tag = format!("test-dockerfile-{}", test_id);
            println!("Building image with tag: {}", tag);

            // Deploy using our config with network
            let container_id = builder
                .deploy_dockerfile(&config, &tag, None, None, Some(network_name.clone()), None)
                .await?;
            println!("Container created with ID: {}", container_id);

            // Add a small delay after creation
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Verify container is running
            let mut filters: HashMap<String, Vec<String>> = HashMap::new();
            filters.insert("id".to_string(), vec![container_id.clone()]);
            filters.insert("label".to_string(), vec![format!("test_id={}", test_id)]);

            let mut retries = 5;
            let mut container_running = false;
            while retries > 0 {
                println!("Checking container state, attempt {}", 6 - retries);
                let list_opts = ListContainersOptionsBuilder::default()
                    .all(true)
                    .filters(&filters)
                    .build();
                if let Ok(containers) = builder.client().list_containers(Some(list_opts)).await {
                    if containers.is_empty() {
                        println!("No containers found matching filters");
                    } else {
                        println!("Container found and running");
                        container_running = true;
                        break;
                    }
                } else {
                    println!("Error listing containers");
                }
                retries -= 1;
                tokio::time::sleep(Duration::from_millis(500)).await;
            }

            // If container not running, get more details
            if !container_running {
                println!("Container not found with filters. Checking container inspect...");
                if let Ok(inspect) = builder
                    .client()
                    .inspect_container(&container_id, None::<InspectContainerOptions>)
                    .await
                {
                    println!("Container inspect result: {:?}", inspect);
                } else {
                    println!("Failed to inspect container");
                }
            }

            assert!(container_running, "Container should be running");

            Ok(())
        })
    })
    .await
}
