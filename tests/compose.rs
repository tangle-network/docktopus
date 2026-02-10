mod common;

use bollard::query_parameters::ListContainersOptionsBuilder;
use color_eyre::Result;
use common::{is_docker_running, with_docker_cleanup};
use docktopus::{BuildConfig, ComposeConfig, DockerBuilder, Service};
use std::{collections::HashMap, time::Duration};

#[tokio::test]
async fn test_compose_deployment() -> Result<()> {
    with_docker_cleanup(|test_id| {
        Box::pin(async move {
            if !is_docker_running() {
                println!("Skipping test: Docker is not running");
                return Ok(());
            }

            let builder = DockerBuilder::new().await?;
            let network_name = format!("test-network-{}", test_id);

            let mut labels = HashMap::new();
            labels.insert("test_id".to_string(), test_id.clone());

            // Create network with retry mechanism
            builder
                .create_network_with_retry(&network_name, 3, Duration::from_secs(2), Some(labels))
                .await?;

            // Create a simple test compose config
            let mut services = HashMap::new();
            let mut env = HashMap::new();
            env.insert("TEST".to_string(), "value".to_string());

            let mut labels = HashMap::new();
            labels.insert("test_id".to_string(), test_id.clone());

            let service_name = format!("test-service-{}", test_id);
            services.insert(
                service_name,
                Service {
                    image: Some("alpine:latest".to_string()),
                    ports: Some(vec!["8080:80".to_string()]),
                    environment: Some(env.into()),
                    volumes: None,
                    networks: Some(vec![network_name.clone()]),
                    labels: Some(labels),
                    ..Service::default()
                },
            );

            let mut config = ComposeConfig {
                version: "3".to_string(),
                services,
                volumes: HashMap::new(),
            };

            let container_ids = builder.deploy_compose(&mut config).await?;
            assert_eq!(container_ids.len(), 1);

            // Add a small delay to ensure Docker has time to start the container
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Verify containers are running
            for (_, container_id) in container_ids {
                let mut filters: HashMap<String, Vec<String>> = HashMap::new();
                filters.insert("id".to_string(), vec![container_id.clone()]);
                filters.insert("label".to_string(), vec![format!("test_id={}", test_id)]);

                let mut retries = 5;
                let mut containers_found = false;
                while retries > 0 {
                    let list_opts = ListContainersOptionsBuilder::default()
                        .all(true)
                        .filters(&filters)
                        .build();
                    match builder.client().list_containers(Some(list_opts)).await {
                        Ok(containers) => {
                            if containers.len() == 1
                                && containers[0].id.as_ref().unwrap() == &container_id
                            {
                                containers_found = true;
                                break;
                            }
                        }
                        Err(e) => println!("Error listing containers: {:?}", e),
                    }
                    retries -= 1;
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }

                assert!(containers_found, "Container not found or not running");
            }

            Ok(())
        })
    })
    .await
}

#[tokio::test]
async fn test_compose_with_build() -> Result<()> {
    with_docker_cleanup(|_test_id| {
        Box::pin(async move {
            let builder = DockerBuilder::new().await.unwrap();

            // Create a compose config with build context
            let mut services = HashMap::new();
            services.insert(
                "test-build-service".to_string(),
                Service {
                    image: None,
                    build: Some(BuildConfig {
                        context: "./".to_string(),
                        dockerfile: Some("Dockerfile".to_string()),
                    }),
                    ports: None,
                    environment: None,
                    volumes: None,
                    networks: None,
                    requirements: None,
                    depends_on: None,
                    healthcheck: None,
                    restart: None,
                    command: None,
                    user: None,
                    labels: None,
                    platform: None,
                    env_file: None,
                },
            );

            let mut config = ComposeConfig {
                version: "3".to_string(),
                services,
                volumes: HashMap::new(),
            };

            let result = builder.deploy_compose(&mut config).await;
            // This should fail because we don't have a Dockerfile in the current directory
            assert!(result.is_err());

            Ok(())
        })
    })
    .await
}
