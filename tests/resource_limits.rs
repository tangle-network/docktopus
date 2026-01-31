mod common;

use bollard::query_parameters::InspectContainerOptions;
use color_eyre::Result;
use common::{is_docker_running, with_docker_cleanup};
use docktopus::config::{SystemRequirements, parse_memory_string};
use docktopus::{ComposeConfig, DockerBuilder, Service};
use std::collections::HashMap;
use std::time::Duration;

#[tokio::test]
#[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
async fn test_resource_limits() -> Result<()> {
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

            let service_name = format!("test-service-{}", test_id);

            // Create a service with resource limits
            let mut services = HashMap::new();
            let mut labels = HashMap::new();
            labels.insert("test_id".to_string(), test_id.to_string());

            services.insert(
                service_name.clone(),
                Service {
                    image: Some("alpine:latest".to_string()),
                    command: Some(vec!["sleep".to_string(), "30".to_string()]),
                    requirements: Some(SystemRequirements {
                        min_memory_gb: 1,
                        min_disk_gb: 1,
                        min_bandwidth_mbps: 100,
                        required_ports: vec![],
                        data_directory: "/tmp".to_string(),
                        cpu_limit: Some(0.5),
                        memory_limit: Some("512M".to_string()),
                        memory_swap: Some("1G".to_string()),
                        memory_reservation: Some("256M".to_string()),
                        cpu_shares: Some(512),
                        cpuset_cpus: Some("0,1".to_string()),
                    }),
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

            // Verify container configuration
            let inspect = builder
                .client()
                .inspect_container(container_id, None::<InspectContainerOptions>)
                .await?;

            if let Some(host_config) = inspect.host_config {
                // Verify memory limits
                assert_eq!(
                    host_config.memory,
                    Some(parse_memory_string("512M").unwrap() as i64)
                );
                assert_eq!(
                    host_config.memory_swap,
                    Some(parse_memory_string("1G").unwrap() as i64)
                );
                assert_eq!(
                    host_config.memory_reservation,
                    Some(parse_memory_string("256M").unwrap() as i64)
                );

                // Verify CPU limits
                assert_eq!(host_config.nano_cpus, Some((0.5 * 1e9) as i64));
                assert_eq!(host_config.cpu_shares, Some(512));
                assert_eq!(host_config.cpuset_cpus, Some("0,1".to_string()));
            }

            Ok(())
        })
    })
    .await
}
