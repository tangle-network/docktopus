use crate::DockerBuilder;
use crate::error::DockerError;
use bollard::exec::{CreateExecOptions, StartExecOptions};
use bollard::models::{NetworkCreateRequest, VolumeCreateOptions};
use bollard::query_parameters::{
    CreateImageOptionsBuilder, InspectContainerOptions, ListNetworksOptions, ListVolumesOptions,
    LogsOptionsBuilder, RemoveVolumeOptions,
};
use futures_util::{StreamExt, TryStreamExt};
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::sleep;

impl DockerBuilder {
    /// Creates a network with extra creation settings
    ///
    /// # Errors
    ///
    /// Will return a `DockerError::BollardError` if the operation fails after the given number of retries
    ///
    /// # Examples
    /// ```no_run
    /// use docktopus::DockerBuilder;
    /// use std::collections::HashMap;
    /// use std::time::Duration;
    ///
    /// # async fn example(builder: DockerBuilder) -> Result<(), docktopus::DockerError> {
    /// // Create a network with retries
    /// let mut labels = HashMap::new();
    /// labels.insert("env".to_string(), "prod".to_string());
    ///
    /// builder
    ///     .create_network_with_retry("my-network", 3, Duration::from_secs(1), Some(labels))
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub async fn create_network_with_retry(
        &self,
        name: &str,
        max_retries: u32,
        initial_delay: Duration,
        labels: Option<HashMap<String, String>>,
    ) -> Result<(), DockerError> {
        let mut delay = initial_delay;
        let mut attempts = 0;

        while attempts < max_retries {
            let result = self
                .client()
                .create_network(NetworkCreateRequest {
                    name: name.to_string(),
                    driver: Some("bridge".to_string()),
                    labels: labels.clone(),
                    ..Default::default()
                })
                .await;

            match result {
                Ok(_) => return Ok(()),
                Err(e) => {
                    if attempts == max_retries - 1 {
                        return Err(DockerError::BollardError(e));
                    }
                    attempts += 1;
                    sleep(delay).await;
                    delay *= 2;
                }
            }
        }

        Ok(())
    }

    /// Removes a Docker network with the specified name
    ///
    /// This method attempts to remove a Docker network by its name. It will fail if the network
    /// does not exist or if there are containers still connected to it.
    ///
    /// # Arguments
    ///
    /// * `name` - Name of the network to remove
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing unit `()` on success, or a `DockerError` if removal fails
    ///
    /// # Errors
    ///
    /// Will return a `DockerError::BollardError` if the operation fails
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    ///
    /// # async fn example() -> Result<(), docktopus::DockerError> {
    /// let builder = DockerBuilder::new().await?;
    /// builder.remove_network("my-network").await?;
    /// # Ok(()) }
    /// ```
    pub async fn remove_network(&self, name: &str) -> Result<(), DockerError> {
        self.client()
            .remove_network(name)
            .await
            .map_err(DockerError::BollardError)
    }

    /// Pulls a Docker image with optional platform specification
    ///
    /// This method attempts to pull a Docker image from a registry. It supports specifying
    /// a target platform for multi-architecture images.
    ///
    /// # Arguments
    ///
    /// * `image` - Name of the image to pull (e.g., "ubuntu:latest")
    /// * `platform` - Optional platform specification (e.g., "linux/amd64", "linux/arm64")
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing unit `()` on success, or a `DockerError` if the pull fails
    ///
    /// # Errors
    ///
    /// Will return a `DockerError::BollardError` if the pull fails
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    ///
    /// # async fn example() -> Result<(), docktopus::DockerError> {
    /// let builder = DockerBuilder::new().await?;
    ///
    /// // Pull with default platform
    /// builder.pull_image("ubuntu:latest", None).await?;
    ///
    /// // Pull with specific platform
    /// builder
    ///     .pull_image("ubuntu:latest", Some("linux/arm64"))
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub async fn pull_image(&self, image: &str, platform: Option<&str>) -> Result<(), DockerError> {
        let create_opts = CreateImageOptionsBuilder::default()
            .from_image(image)
            .platform(platform.unwrap_or(""))
            .build();

        let mut pull_stream = self.client.create_image(Some(create_opts), None, None);

        while let Some(pull_result) = pull_stream.next().await {
            if let Err(e) = pull_result {
                return Err(DockerError::BollardError(e));
            }
        }

        Ok(())
    }

    /// Lists all Docker networks
    ///
    /// This method retrieves a list of all Docker networks present on the system.
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing a `Vec<String>` of network names on success, or a `DockerError` if the operation fails
    ///
    /// # Errors
    ///
    /// Will return a `DockerError::BollardError` if the operation fails
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    ///
    /// # async fn example() -> Result<(), docktopus::DockerError> {
    /// let builder = DockerBuilder::new().await?;
    /// let networks = builder.list_networks().await?;
    /// for network in networks {
    ///     println!("Found network: {}", network);
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn list_networks(&self) -> Result<Vec<String>, DockerError> {
        let networks = self
            .client()
            .list_networks(None::<ListNetworksOptions>)
            .await
            .map_err(DockerError::BollardError)?;

        Ok(networks.into_iter().filter_map(|n| n.name).collect())
    }

    /// Creates a Docker volume with the specified name
    ///
    /// This method creates a new Docker volume with the given name using the local driver.
    ///
    /// # Arguments
    ///
    /// * `name` - Name to assign to the new volume
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on successful volume creation, or a `DockerError` if creation fails
    ///
    /// # Errors
    ///
    /// Will return a `DockerError::BollardError` if the operation fails
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    ///
    /// # async fn example() -> Result<(), docktopus::DockerError> {
    /// let builder = DockerBuilder::new().await?;
    /// builder.create_volume("my_volume").await?;
    /// # Ok(()) }
    /// ```
    pub async fn create_volume(&self, name: &str) -> Result<(), DockerError> {
        self.client()
            .create_volume(VolumeCreateOptions {
                name: Some(name.to_string()),
                driver: Some("local".to_string()),
                ..Default::default()
            })
            .await
            .map_err(DockerError::BollardError)?;

        Ok(())
    }

    /// Removes a Docker volume with the specified name
    ///
    /// This method removes an existing Docker volume with the given name.
    ///
    /// # Arguments
    ///
    /// * `name` - Name of the volume to remove
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` on successful volume removal, or a `DockerError` if removal fails
    ///
    /// # Errors
    ///
    /// Will return a `DockerError::BollardError` if the operation fails
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    ///
    /// # async fn example() -> Result<(), docktopus::DockerError> {
    /// let builder = DockerBuilder::new().await?;
    /// builder.remove_volume("my_volume").await?;
    /// # Ok(()) }
    /// ```
    pub async fn remove_volume(&self, name: &str) -> Result<(), DockerError> {
        self.client()
            .remove_volume(name, None::<RemoveVolumeOptions>)
            .await
            .map_err(DockerError::BollardError)
    }

    /// Lists all Docker volumes with optional filters
    ///
    /// This method retrieves a list of all Docker volumes on the system, with optional filtering
    /// capabilities.
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing a vector of volume names as strings, or a `DockerError` if the operation fails
    ///
    /// # Errors
    ///
    /// Will return a `DockerError::BollardError` if the operation fails
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    ///
    /// # async fn example() -> Result<(), docktopus::DockerError> {
    /// let builder = DockerBuilder::new().await?;
    /// let volumes = builder.list_volumes().await?;
    /// for volume in volumes {
    ///     println!("Found volume: {}", volume);
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn list_volumes(&self) -> Result<Vec<String>, DockerError> {
        let volumes = self
            .client()
            .list_volumes(None::<ListVolumesOptions>)
            .await
            .map_err(DockerError::BollardError)?;

        Ok(volumes
            .volumes
            .unwrap_or_default()
            .into_iter()
            .map(|v| v.name)
            .collect())
    }

    /// Waits for a container to be in a running state
    ///
    /// This method polls the container status until it is running or the maximum number of retries
    /// is reached. It will retry up to 5 times with a 500ms delay between attempts.
    ///
    /// # Arguments
    ///
    /// * `container_id` - ID of the container to wait for
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the container is running, or a `DockerError` if the container fails to start
    /// after maximum retries.
    ///
    /// # Errors
    ///
    /// * Unable to inspect the container
    /// * The container is not running after 5 retries
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    ///
    /// # async fn example() -> Result<(), docktopus::DockerError> {
    /// let builder = DockerBuilder::new().await?;
    /// builder.wait_for_container("container_id").await?;
    /// # Ok(()) }
    /// ```
    pub async fn wait_for_container(&self, container_id: &str) -> Result<(), DockerError> {
        let mut retries = 5;
        while retries > 0 {
            let inspect = self
                .client()
                .inspect_container(container_id, None::<InspectContainerOptions>)
                .await
                .map_err(DockerError::BollardError)?;

            if let Some(state) = inspect.state {
                if let Some(running) = state.running {
                    if running {
                        return Ok(());
                    }
                }
            }
            sleep(Duration::from_millis(500)).await;
            retries -= 1;
        }
        Err(DockerError::ValidationError(format!(
            "Container {} not running after retries",
            container_id
        )))
    }

    /// Retrieves logs from a Docker container
    ///
    /// This method fetches both stdout and stderr logs from the specified container with timestamps.
    /// The logs are returned as a single string with each log line separated by newlines.
    ///
    /// # Arguments
    ///
    /// * `container_id` - ID of the container to get logs from
    ///
    /// # Returns
    ///
    /// Returns `Ok(String)` containing the container logs, or a `DockerError` if there was an error
    /// retrieving the logs.
    ///
    /// # Errors
    ///
    /// Will return a `DockerError::BollardError` if the operation fails
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    ///
    /// # async fn example() -> Result<(), docktopus::DockerError> {
    /// let builder = DockerBuilder::new().await?;
    /// let logs = builder.get_container_logs("container_id").await?;
    /// println!("Container logs: {}", logs);
    /// # Ok(()) }
    /// ```
    pub async fn get_container_logs(&self, container_id: &str) -> Result<String, DockerError> {
        let mut output = String::new();
        let logs_opts = LogsOptionsBuilder::default()
            .stdout(true)
            .stderr(true)
            .timestamps(true)
            .follow(false)
            .tail("all")
            .build();

        let mut stream = self.client().logs(container_id, Some(logs_opts));

        while let Some(log) = stream.try_next().await.map_err(DockerError::BollardError)? {
            output.push_str(&log.to_string());
            output.push('\n');
        }

        Ok(output)
    }

    /// Executes a command inside a Docker container
    ///
    /// This method executes the specified command inside the specified container and returns the
    /// output as a string.
    ///
    /// # Arguments
    ///
    /// * `container_id` - ID of the container to execute the command in
    /// * `cmd` - Command to execute in the container
    /// * `env` - Environment variables to set in the container
    ///
    /// # Errors
    ///
    /// Will return a `DockerError::BollardError` if the operation fails
    pub async fn exec_in_container(
        &self,
        container_id: &str,
        cmd: Vec<&str>,
        env: Option<HashMap<String, String>>,
    ) -> Result<String, DockerError> {
        let exec = self
            .client()
            .create_exec(
                container_id,
                CreateExecOptions::<String> {
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    cmd: Some(cmd.into_iter().map(ToString::to_string).collect()),
                    env: env.map(|e| e.into_iter().map(|(k, v)| format!("{}={}", k, v)).collect()),
                    ..Default::default()
                },
            )
            .await
            .map_err(DockerError::BollardError)?;

        let output = self
            .client()
            .start_exec(&exec.id, None::<StartExecOptions>)
            .await
            .map_err(DockerError::BollardError)?;

        match output {
            bollard::exec::StartExecResults::Attached { mut output, .. } => {
                let mut bytes = Vec::new();
                while let Some(chunk) =
                    output.try_next().await.map_err(DockerError::BollardError)?
                {
                    bytes.extend_from_slice(&chunk.into_bytes());
                }
                Ok(String::from_utf8_lossy(&bytes).into_owned())
            }
            _ => Ok(String::new()),
        }
    }
}
