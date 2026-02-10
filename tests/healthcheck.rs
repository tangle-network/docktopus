mod common;

use bollard::query_parameters::{CreateImageOptionsBuilder, InspectContainerOptions};
use color_eyre::Result;
use common::{is_docker_running, with_docker_cleanup};
use docktopus::config::{HealthCheck, Method};
use docktopus::{ComposeConfig, DockerBuilder, Service};
use futures_util::TryStreamExt;
use std::{collections::HashMap, time::Duration};

#[tokio::test]
async fn test_healthcheck() -> Result<()> {
    with_docker_cleanup(|test_id| {
        Box::pin(async move {
            if !is_docker_running() {
                println!("Skipping test: Docker is not running");
                return Ok(());
            }

            let builder = DockerBuilder::new().await.unwrap();
            let network_name = format!("test-network-{}", test_id);

            let mut network_labels = HashMap::new();
            network_labels.insert("test_id".to_string(), test_id.clone());

            // Create network with retry mechanism
            builder
                .create_network_with_retry(
                    &network_name,
                    3,
                    Duration::from_secs(2),
                    Some(network_labels),
                )
                .await?;

            let service_name = format!("healthy-service-{}", test_id);

            // Pull nginx image first
            let create_opts = CreateImageOptionsBuilder::default()
                .from_image("nginx")
                .tag("latest")
                .build();
            builder
                .client()
                .create_image(Some(create_opts), None, None)
                .try_collect::<Vec<_>>()
                .await?;

            // Create a service with healthcheck
            let mut services = HashMap::new();
            let mut labels = HashMap::new();
            labels.insert("test_id".to_string(), test_id.clone());

            services.insert(
                service_name.clone(),
                Service {
                    image: Some("nginx:latest".to_string()),
                    healthcheck: Some(HealthCheck {
                        endpoint: "http://localhost/".to_string(),
                        method: Method::Get,
                        expected_status: 200,
                        body: None,
                        interval: Duration::from_secs(1),
                        timeout: Duration::from_secs(3),
                        retries: 3,
                    }),
                    ports: Some(vec!["8080:80".to_string()]),
                    networks: Some(vec![network_name.clone()]),
                    labels: Some(labels),
                    ..Default::default()
                },
            );

            let mut config = ComposeConfig {
                version: "3".to_string(),
                services,
                volumes: HashMap::new(),
            };

            let container_ids = builder.deploy_compose(&mut config).await?;
            let container_id = container_ids.get(&service_name).unwrap();

            // Wait for container to be healthy
            builder.wait_for_container(container_id).await?;

            // Verify healthcheck configuration
            let inspect = builder
                .client()
                .inspect_container(container_id, None::<InspectContainerOptions>)
                .await?;

            if let Some(config) = inspect.config {
                if let Some(healthcheck) = config.healthcheck {
                    assert_eq!(
                        healthcheck.test,
                        Some(vec![
                            "CMD-SHELL".to_string(),
                            format!(
								"curl -X GET {} -s -f -o /dev/null -w '%{{http_code}}' | grep -q {}",
								"http://localhost/", "200"
							)
                        ])
                    );
                    assert_eq!(healthcheck.interval, Some(1_000_000_000));
                    assert_eq!(healthcheck.timeout, Some(3_000_000_000));
                    assert_eq!(healthcheck.retries, Some(3));
                }
            }

            Ok(())
        })
    })
    .await
}
