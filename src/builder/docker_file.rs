use crate::config::docker_file::DockerfileConfig;
use crate::error::DockerError;
use bollard::body_full;
use bollard::models::ContainerCreateBody;
use bollard::query_parameters::{
    BuildImageOptionsBuilder, CreateContainerOptions, StartContainerOptions,
};
use bollard::service::HostConfig;
use bytes::Bytes;
use futures_util::StreamExt;

use super::DockerBuilder;

impl DockerBuilder {
    /// Deploys a Dockerfile configuration with optional settings
    ///
    /// This method builds a Docker image from a Dockerfile configuration and creates a container from it.
    /// It handles:
    /// - Creating a temporary build context
    /// - Building the Docker image
    /// - Creating and starting a container with the specified options
    ///
    /// # Arguments
    ///
    /// * `config` - The Dockerfile configuration to deploy
    /// * `tag` - Tag to apply to the built image
    /// * `command` - Optional command to override the default container command
    /// * `volumes` - Optional volume mounts for the container
    /// * `network` - Optional network to connect the container to
    /// * `env` - Optional environment variables for the container
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing the ID of the created container, or a `DockerError` if deployment fails
    ///
    /// # Errors
    ///
    /// * Unable to create a temporary build context directory
    /// * Unable to build the Docker image
    /// * Unable to create or start the container
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    /// use docktopus::config::docker_file::{DockerCommand, DockerfileConfig};
    ///
    /// # async fn example() -> Result<(), docktopus::DockerError> {
    /// let builder = DockerBuilder::new().await?;
    /// let config = DockerfileConfig {
    ///     // Your Dockerfile config
    ///     base_image: "ubuntu:latest".to_string(),
    ///     commands: vec![
    ///         DockerCommand::Run {
    ///             command: "apt-get update".to_string(),
    ///         },
    ///         DockerCommand::Copy {
    ///             source: "app".to_string(),
    ///             dest: "/app".to_string(),
    ///             chown: None,
    ///         },
    ///     ],
    /// };
    /// let container_id = builder
    ///     .deploy_dockerfile(
    ///         &config,
    ///         "my-image:latest",
    ///         Some(vec!["echo".to_string(), "hello".to_string()]),
    ///         None,
    ///         None,
    ///         None,
    ///     )
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub async fn deploy_dockerfile(
        &self,
        config: &DockerfileConfig,
        tag: &str,
        command: Option<Vec<String>>,
        volumes: Option<Vec<String>>,
        network: Option<String>,
        env: Option<Vec<String>>,
    ) -> Result<String, DockerError> {
        // Create a temporary directory for the build context
        let temp_dir = tempfile::tempdir()?;
        let dockerfile_path = temp_dir.path().join("Dockerfile");

        // Write the Dockerfile content from our config
        tokio::fs::write(&dockerfile_path, config.to_string()).await?;

        // Create tar archive with the Dockerfile
        let tar_path = temp_dir.path().join("context.tar");
        let tar_file = std::fs::File::create(&tar_path)?;
        let mut tar_builder = tar::Builder::new(tar_file);
        tar_builder.append_path_with_name(&dockerfile_path, "Dockerfile")?;
        tar_builder.finish()?;

        // Read the tar file
        let context = tokio::fs::read(&tar_path).await?;

        // Build the image
        let build_opts = BuildImageOptionsBuilder::default()
            .dockerfile("Dockerfile")
            .t(tag)
            .q(false)
            .build();

        let body = body_full(Bytes::from(context));
        let mut build_stream = self.client.build_image(build_opts, None, Some(body));

        while let Some(build_result) = build_stream.next().await {
            if let Err(e) = build_result {
                return Err(DockerError::BollardError(e));
            }
        }

        // Create and start container from our image
        let container_config = ContainerCreateBody {
            image: Some(tag.to_string()),
            cmd: command.map(|v| v.iter().map(ToString::to_string).collect()),
            env: env.map(|v| v.iter().map(ToString::to_string).collect()),
            host_config: Some(HostConfig {
                binds: volumes.map(|v| v.iter().map(ToString::to_string).collect()),
                network_mode: network,
                ..Default::default()
            }),
            ..Default::default()
        };

        let container_info = self
            .client
            .create_container(None::<CreateContainerOptions>, container_config)
            .await
            .map_err(DockerError::BollardError)?;

        self.client
            .start_container(&container_info.id, None::<StartContainerOptions>)
            .await
            .map_err(DockerError::BollardError)?;

        Ok(container_info.id)
    }
}
