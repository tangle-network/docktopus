use bollard::Docker;
use bollard::query_parameters::{
    ListContainersOptionsBuilder, ListNetworksOptionsBuilder, ListVolumesOptionsBuilder,
    RemoveContainerOptionsBuilder, RemoveVolumeOptions, StopContainerOptions,
};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::process::Command;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use uuid::Uuid;

#[allow(dead_code)]
pub fn is_docker_running() -> bool {
    Command::new("docker")
        .arg("info")
        .output()
        .is_ok_and(|output| output.status.success())
}

pub struct DockerTestContext {
    client: Docker,
    test_id: String,
}

impl DockerTestContext {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let client = Docker::connect_with_local_defaults().unwrap();
        let test_id = Uuid::new_v4().to_string();
        Self { client, test_id }
    }

    pub fn get_test_id(&self) -> &str {
        &self.test_id
    }

    pub async fn cleanup(&self) {
        println!("Starting cleanup for test_id: {}", self.test_id);

        // Clean up containers by label
        let mut label_filters: HashMap<String, Vec<String>> = HashMap::new();
        label_filters.insert(
            String::from("label"),
            vec![format!("test_id={}", self.test_id)],
        );

        // Also match containers by name pattern
        let mut name_filters: HashMap<String, Vec<String>> = HashMap::new();
        name_filters.insert(
            String::from("name"),
            vec![
                format!("healthy-service-{}", self.test_id),
                format!("test-service-{}", self.test_id),
                format!("optimism-test-{}", self.test_id),
            ],
        );

        // Try both label and name filters
        for filters in [label_filters, name_filters] {
            let options = ListContainersOptionsBuilder::default()
                .all(true)
                .filters(&filters)
                .build();

            if let Ok(containers) = self.client.list_containers(Some(options)).await {
                for container in containers {
                    if let Some(id) = container.id {
                        println!("Found container to remove: {}", id);
                        // Try to stop the container first
                        let stop_result = self
                            .client
                            .stop_container(&id, None::<StopContainerOptions>)
                            .await;
                        match stop_result {
                            Ok(()) => println!("Stopped container: {}", id),
                            Err(e) => println!("Error stopping container {}: {}", id, e),
                        }

                        // Try to remove the container
                        let remove_opts =
                            RemoveContainerOptionsBuilder::default().force(true).build();
                        match self.client.remove_container(&id, Some(remove_opts)).await {
                            Ok(()) => println!("Removed container: {}", id),
                            Err(e) => println!("Error removing container {}: {}", id, e),
                        }
                    }
                }
            }
        }

        // Clean up networks - handle both label and name patterns
        let mut network_filters: HashMap<String, Vec<String>> = HashMap::new();
        // Match networks with test_id label
        network_filters.insert(
            String::from("label"),
            vec![format!("test_id={}", self.test_id)],
        );

        let network_opts = ListNetworksOptionsBuilder::default()
            .filters(&network_filters)
            .build();

        if let Ok(networks) = self.client.list_networks(Some(network_opts)).await {
            for network in networks {
                if let Some(id) = network.id {
                    println!("Removing network by label: {}", id);
                    match self.client.remove_network(&id).await {
                        Ok(()) => println!("Removed network: {}", id),
                        Err(e) => println!("Error removing network {}: {}", id, e),
                    }
                }
            }
        }

        // Also match networks by name pattern
        let mut name_filters: HashMap<String, Vec<String>> = HashMap::new();
        name_filters.insert(
            String::from("name"),
            vec![
                format!("test-network-{}", self.test_id),
                format!("compose_network_{}", self.test_id),
                format!("optimism-test-{}", self.test_id),
            ],
        );

        let name_network_opts = ListNetworksOptionsBuilder::default()
            .filters(&name_filters)
            .build();

        if let Ok(networks) = self.client.list_networks(Some(name_network_opts)).await {
            for network in networks {
                if let Some(id) = network.id {
                    println!("Removing network by name: {}", id);
                    match self.client.remove_network(&id).await {
                        Ok(()) => println!("Removed network: {}", id),
                        Err(e) => println!("Error removing network {}: {}", id, e),
                    }
                }
            }
        }

        // Clean up volumes by label
        let mut label_filters: HashMap<String, Vec<String>> = HashMap::new();
        label_filters.insert(
            String::from("label"),
            vec![format!("test_id={}", self.test_id)],
        );

        // Also match volumes by name pattern
        let mut vol_name_filters: HashMap<String, Vec<String>> = HashMap::new();
        vol_name_filters.insert(
            String::from("name"),
            vec![
                format!("test-volume-{}", self.test_id),
                format!("reth_data_{}", self.test_id),
                format!("reth_jwt_{}", self.test_id),
                format!("nimbus_data_{}", self.test_id),
                format!("optimism_data_{}", self.test_id),
            ],
        );

        // Try both label and name filters for volumes
        for filters in [label_filters, vol_name_filters] {
            let vol_opts = ListVolumesOptionsBuilder::default()
                .filters(&filters)
                .build();

            if let Ok(volumes) = self.client.list_volumes(Some(vol_opts)).await {
                if let Some(volume_list) = volumes.volumes {
                    for volume in volume_list {
                        println!("Removing volume: {}", volume.name);
                        match self
                            .client
                            .remove_volume(&volume.name, None::<RemoveVolumeOptions>)
                            .await
                        {
                            Ok(()) => println!("Removed volume: {}", volume.name),
                            Err(e) => println!("Error removing volume {}: {}", volume.name, e),
                        }
                    }
                }
            }
        }
        println!("Cleanup complete for test_id: {}", self.test_id);
    }
}

// Drop guard to ensure cleanup happens even if test panics
pub struct TestGuard {
    ctx: DockerTestContext,
    cleanup_thread: Option<JoinHandle<()>>,
    cleanup_sender: mpsc::SyncSender<String>,
}

impl TestGuard {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let ctx = DockerTestContext::new();

        // Create a synchronous channel for cleanup communication
        let (tx, rx) = mpsc::sync_channel(0);

        // Spawn a thread to handle cleanup
        let cleanup_thread = thread::spawn(move || {
            if let Ok(test_id) = rx.recv() {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let client = Docker::connect_with_local_defaults().unwrap();
                let ctx = DockerTestContext { client, test_id };
                rt.block_on(ctx.cleanup());
            }
        });

        Self {
            ctx,
            cleanup_thread: Some(cleanup_thread),
            cleanup_sender: tx,
        }
    }

    pub fn get_test_id(&self) -> &str {
        self.ctx.get_test_id()
    }
}

impl Drop for TestGuard {
    fn drop(&mut self) {
        // Send test_id to cleanup thread and wait for it to complete
        let _ = self.cleanup_sender.send(self.ctx.test_id.clone());
        if let Some(thread) = self.cleanup_thread.take() {
            let _ = thread.join();
        }
    }
}

/// Helper to create a test context and ensure cleanup
pub async fn with_docker_cleanup<F>(mut test_body: F) -> color_eyre::Result<()>
where
    F: FnMut(String) -> Pin<Box<dyn Future<Output = color_eyre::Result<()>> + Send + 'static>>,
{
    let guard = TestGuard::new();
    let test_id = guard.get_test_id().to_string();

    // Clean up any leftover resources using the same test_id
    let client = Docker::connect_with_local_defaults()?;
    let ctx = DockerTestContext {
        client,
        test_id: test_id.clone(),
    };
    ctx.cleanup().await;

    // Run the test with the guard's test_id
    test_body(test_id).await?;

    Ok(())
}
