mod common;

use common::with_docker_cleanup;
use docktopus::DockerBuilder;
use std::collections::HashMap;
use std::time::Duration;
use uuid::Uuid;

#[tokio::test]
async fn test_network_management() -> color_eyre::Result<()> {
    with_docker_cleanup(|test_id| {
        Box::pin(async move {
            let builder = DockerBuilder::new().await?;
            let network_name = format!("test-network-{}", Uuid::new_v4());

            let mut network_labels = HashMap::new();
            network_labels.insert("test_id".to_string(), test_id.clone());

            // Create network with retry
            builder
                .create_network_with_retry(
                    &network_name,
                    5,
                    Duration::from_millis(100),
                    Some(network_labels),
                )
                .await?;

            // Verify network exists
            let networks = builder.list_networks().await?;
            assert!(
                networks.contains(&network_name),
                "Created network should be in the list"
            );

            Ok(())
        })
    })
    .await
}
