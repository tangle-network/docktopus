//! Utilities for spinning up and managing Docker containers

use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, InspectContainerOptions, ListContainersOptions,
    StartContainerOptions, StopContainerOptions, WaitContainerOptions,
};
use bollard::models::{
    ContainerConfig, ContainerCreateResponse, ContainerInspectResponse, HostConfig,
    MountPointTypeEnum, PortMap, RestartPolicy,
};
use core::str::FromStr;
use futures_util::{Stream, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Attempted to connect to a non-existent container")]
    ContainerNotFound,
    #[error("Found an invalid status for the container: `{0}`")]
    BadContainerStatus(String),
    #[error("{0}")]
    Bollard(#[from] bollard::errors::Error),
}

/// The status of a Docker container
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ContainerStatus {
    /// Created, but never started
    Created,
    /// Actively running
    Running,
    /// Paused via `docker pause`
    Paused,
    /// Restarting according to the restart policy
    Restarting,
    /// Container was started, and is no longer running
    Exited,
    /// In the process of being removed
    Removing,
    /// Defunct, partially removed
    Dead,
}

impl FromStr for ContainerStatus {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "created" => Ok(ContainerStatus::Created),
            "running" => Ok(ContainerStatus::Running),
            "paused" => Ok(ContainerStatus::Paused),
            "restarting" => Ok(ContainerStatus::Restarting),
            "exited" => Ok(ContainerStatus::Exited),
            "removing" => Ok(ContainerStatus::Removing),
            "dead" => Ok(ContainerStatus::Dead),
            _ => Err(Error::BadContainerStatus(s.to_string())),
        }
    }
}

impl ContainerStatus {
    #[must_use]
    pub fn is_active(self) -> bool {
        matches!(self, ContainerStatus::Running)
    }

    #[must_use]
    pub fn is_usable(self) -> bool {
        !matches!(self, ContainerStatus::Removing | ContainerStatus::Dead)
    }
}

/// A [Docker](https://en.wikipedia.org/wiki/Docker_(software)) container
#[derive(Debug)]
pub struct Container {
    id: Option<String>,
    image: String,
    client: Arc<Docker>,
    options: ContainerOptions,
}

#[derive(Debug, Default, Clone)]
struct ContainerOptions {
    env: Option<Vec<String>>,
    cmd: Option<Vec<String>>,
    binds: Option<Vec<String>>,
    extra_hosts: Option<Vec<String>>,
    runtime: Option<String>,
    port_bindings: Option<PortMap>,
    restart_policy: Option<RestartPolicy>,
}

impl Container {
    /// Create a new `Container`
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    /// use docktopus::container::Container;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), docktopus::container::Error> {
    /// let connection = DockerBuilder::new().await?;
    /// let mut container = Container::new(connection.client(), "rustlang/rust");
    ///
    /// // We can now start our container
    /// container.start(true).await?;
    /// # Ok(()) }
    /// ```
    pub fn new<T>(client: Arc<Docker>, image: T) -> Self
    where
        T: Into<String>,
    {
        Self {
            id: None,
            image: image.into(),
            client,
            options: ContainerOptions::default(),
        }
    }

    /// Attempt to fetch an existing container by its ID
    ///
    /// # Errors
    ///
    /// * Docker inspect fails
    /// * The container isn't found
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    /// use docktopus::container::Container;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), docktopus::container::Error> {
    /// let connection = DockerBuilder::new().await?;
    /// let mut container = Container::new(connection.client(), "rustlang/rust");
    ///
    /// // We can now start our container and grab its id
    /// container.start(false).await?;
    ///
    /// let id = container.id().unwrap();
    ///
    /// let container2 = Container::from_id(connection.client(), id).await?;
    ///
    /// assert_eq!(container.id(), container2.id());
    /// # Ok(()) }
    /// ```
    pub async fn from_id<T>(client: Arc<Docker>, id: T) -> Result<Self, Error>
    where
        T: AsRef<str>,
    {
        let ContainerInspectResponse {
            id: Some(id),
            config:
                Some(ContainerConfig {
                    env,
                    cmd,
                    image: Some(image),
                    ..
                }),
            mounts,
            host_config,
            ..
        } = client
            .inspect_container(id.as_ref(), None::<InspectContainerOptions>)
            .await?
        else {
            return Err(Error::ContainerNotFound);
        };

        let binds = mounts.map(|mounts| {
            mounts
                .into_iter()
                .filter_map(|mount| {
                    if !matches!(mount.typ, Some(MountPointTypeEnum::BIND)) {
                        return None;
                    }
                    let source = mount.source?;
                    let dest = mount.destination?;
                    let mut bind = format!("{}:{}", source, dest);
                    if let Some(mode) = mount.mode {
                        bind.push(':');
                        bind.push_str(&mode);
                    }
                    Some(bind)
                })
                .collect::<Vec<_>>()
        });

        let mut extra_hosts = None;
        let mut runtime = None;
        let mut restart_policy = None;
        let mut port_bindings = None;
        if let Some(hc) = host_config {
            extra_hosts = hc.extra_hosts;
            runtime = hc.runtime;
            restart_policy = hc.restart_policy;
            port_bindings = hc.port_bindings;
        }

        let options = ContainerOptions {
            env,
            cmd,
            binds,
            extra_hosts,
            runtime,
            port_bindings,
            restart_policy,
        };

        Ok(Self {
            id: Some(id),
            image,
            client,
            options,
        })
    }

    /// Set the environment variables for the container
    ///
    /// NOTE: This will override any existing variables.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    /// use docktopus::container::Container;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), docktopus::container::Error> {
    /// let connection = DockerBuilder::new().await?;
    /// let mut container = Container::new(connection.client(), "rustlang/rust");
    ///
    /// container.env(["FOO=BAR", "BAZ=QUX"]);
    ///
    /// // We can now start our container, and the "FOO" and "BAZ" env vars will be set
    /// container.start(true).await?;
    /// # Ok(()) }
    /// ```
    pub fn env(&mut self, env: impl IntoIterator<Item = impl Into<String>>) -> &mut Self {
        self.options.env = Some(env.into_iter().map(Into::into).collect());
        self
    }

    /// Set the command to run
    ///
    /// The command is provided as a list of strings.
    ///
    /// NOTE: This will override any existing command
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    /// use docktopus::container::Container;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), docktopus::container::Error> {
    /// let connection = DockerBuilder::new().await?;
    /// let mut container = Container::new(connection.client(), "rustlang/rust");
    ///
    /// container.cmd(["echo", "Hello!"]);
    ///
    /// // We can now start our container, and the command "echo Hello!" will run
    /// container.start(true).await?;
    /// # Ok(()) }
    /// ```
    pub fn cmd(&mut self, cmd: impl IntoIterator<Item = impl Into<String>>) -> &mut Self {
        self.options.cmd = Some(cmd.into_iter().map(Into::into).collect());
        self
    }

    /// Set a list of volume binds
    ///
    /// These binds are in the standard `host:dest[:options]` format. For more information, see
    /// the [Docker documentation](https://docs.docker.com/engine/storage/bind-mounts/).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    /// use docktopus::container::Container;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), docktopus::container::Error> {
    /// let connection = DockerBuilder::new().await?;
    /// let mut container = Container::new(connection.client(), "rustlang/rust");
    ///
    /// // Mount './my-host-dir' at '/some/container/dir' and make it read-only
    /// container.binds(["./my-host-dir:/some/container/dir:ro"]);
    ///
    /// // We can now start our container
    /// container.start(true).await?;
    /// # Ok(()) }
    /// ```
    pub fn binds(&mut self, binds: impl IntoIterator<Item = impl Into<String>>) -> &mut Self {
        self.options.binds = Some(binds.into_iter().map(Into::into).collect());
        self
    }

    /// Add entries to the container’s `/etc/hosts` (equivalent to `--add-host`)
    ///
    /// Each item should be `"hostname:IP"` (e.g. `"host.docker.internal:host-gateway"`).
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    /// use docktopus::container::Container;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), docktopus::container::Error> {
    /// let connection = DockerBuilder::new().await?;
    /// let mut container = Container::new(connection.client(), "rustlang/rust");
    ///
    /// // Bind `host.docker.internal` (in the container) to the host gateway
    /// container.extra_hosts(["host.docker.internal:host-gateway"]);
    ///
    /// // We can now start our container
    /// container.start(true).await?;
    /// # Ok(()) }
    /// ```
    pub fn extra_hosts(&mut self, hosts: impl IntoIterator<Item = impl Into<String>>) -> &mut Self {
        self.options.extra_hosts = Some(hosts.into_iter().map(Into::into).collect());
        self
    }

    /// Add a mapping of container ports to host ports
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::bollard::models::{PortBinding, PortMap};
    /// use docktopus::{Container, Runtime};
    /// use std::collections::HashMap;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), docktopus::container::Error> {
    /// // Bind port 80 on the host to port 8080 in the container
    /// let mut bindings = HashMap::new();
    /// bindings.insert(
    ///     String::from("8080/tcp"),
    ///     Some(vec![PortBinding {
    ///         host_ip: Some("127.0.0.1".into()),
    ///         host_port: Some("80".into()),
    ///     }]),
    /// );
    ///
    /// let runtime = Runtime::detect().await.expect("No runtime found")?;
    /// let mut container = Container::builder()
    ///     .image("rustlang/rust")
    ///     .port_bindings(bindings)
    ///     .build(runtime);
    ///
    /// // We can now start our container
    /// container.start(true).await?;
    /// # Ok(()) }
    /// ```
    pub fn port_bindings(mut self, port_bindings: PortMap) -> Self {
        self.options.port_bindings = Some(port_bindings);
        self
    }

    /// Set the runtime to use for this container (equivalent to `--runtime`)
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::{Container, Runtime};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), docktopus::container::Error> {
    /// let runtime = Runtime::detect().await.expect("No runtime found")?;
    /// let mut container = Container::builder()
    ///     .image("rustlang/rust")
    ///     // Use the Sysbox runtime
    ///     .runtime("sysbox-runc")
    ///     .build(runtime);
    ///
    /// // We can now start our container
    /// container.start(true).await?;
    /// # Ok(()) }
    /// ```
    pub fn runtime(mut self, runtime: impl Into<String>) -> Self {
        self.options.runtime = Some(runtime.into());
        self
    }

    /// Set the container's restart policy (equivalent to `--restart`)
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::bollard::models::{RestartPolicy, RestartPolicyNameEnum};
    /// use docktopus::{Container, Runtime};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), docktopus::container::Error> {
    /// let runtime = Runtime::detect().await.expect("No runtime found")?;
    /// let mut container = Container::builder()
    ///     .image("rustlang/rust")
    ///     // Always restart the container, unless stopped manually
    ///     .restart_policy(RestartPolicy {
    ///         name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
    ///         ..Default::default()
    ///     })
    ///     .build(runtime);
    ///
    /// // We can now start our container
    /// container.start(true).await?;
    /// # Ok(()) }
    /// ```
    pub fn restart_policy(mut self, restart_policy: RestartPolicy) -> Self {
        self.options.restart_policy = Some(restart_policy);
        self
    }

    /// Get the container ID if it has been created
    ///
    /// This will only have a value if [`Container::create`] or [`Container::start`] has been
    /// called prior.
    #[must_use]
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// Attempt to create the container
    ///
    /// This will take the following into account:
    ///
    /// * [`Container::env`]
    /// * [`Container::cmd`]
    /// * [`Container::binds`]
    ///
    /// Be sure to set these before calling this!
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    /// use docktopus::container::Container;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), docktopus::container::Error> {
    /// let connection = DockerBuilder::new().await?;
    /// let mut container = Container::new(connection.client(), "rustlang/rust");
    ///
    /// container.env(["FOO=BAR", "BAZ=QUX"]);
    /// container.cmd(["echo", "Hello!"]);
    /// container.binds(["./host-data:/container-data"]);
    ///
    /// // The container is created using the above settings
    /// container.create().await?;
    ///
    /// // Now it can be started
    /// container.start(true).await?;
    /// # Ok(()) }
    /// ```
    #[tracing::instrument(skip_all)]
    pub async fn create(&mut self) -> Result<(), bollard::errors::Error> {
        log::debug!("Creating container");

        let config = Config {
            image: Some(self.image.clone()),
            cmd: self.options.cmd.clone(),
            env: self.options.env.clone(),
            attach_stdout: Some(true),
            host_config: Some(HostConfig {
                binds: self.options.binds.clone(),
                extra_hosts: self.options.extra_hosts.clone(),
                port_bindings: self.options.port_bindings.clone(),
                restart_policy: self.options.restart_policy.clone(),
                runtime: self.options.runtime.clone(),
                ..Default::default()
            }),
            ..Default::default()
        };

        let ContainerCreateResponse { id, warnings } = self
            .client
            .create_container(None::<CreateContainerOptions<String>>, config)
            .await?;
        for warning in warnings {
            log::warn!("{}", warning);
        }

        self.id = Some(id);
        Ok(())
    }

    /// Attempt to start the container
    ///
    /// NOTE: If the container has not yet been created, this will attempt to call [`Container::create`] first.
    ///
    /// `wait_for_exit` will wait for the container to exit before returning.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    /// use docktopus::container::Container;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), docktopus::container::Error> {
    /// let connection = DockerBuilder::new().await?;
    /// let mut container = Container::new(connection.client(), "rustlang/rust");
    ///
    /// container.cmd(["echo", "Hello!"]);
    ///
    /// // We can now start our container, and the command "echo Hello!" will run.
    /// let wait_for_exit = true;
    /// container.start(wait_for_exit).await?;
    ///
    /// // Since we waited for the container to exit, we don't have to stop it.
    /// // It can now just be removed.
    /// container.remove(None).await?;
    /// # Ok(()) }
    /// ```
    #[tracing::instrument(skip(self))]
    pub async fn start(&mut self, wait_for_exit: bool) -> Result<(), bollard::errors::Error> {
        if self.id.is_none() {
            self.create().await?;
        }

        log::debug!("Starting container");
        let id = self.id.as_ref().unwrap();
        self.client
            .start_container(id, None::<StartContainerOptions<String>>)
            .await?;

        if wait_for_exit {
            self.wait().await?;
        }

        Ok(())
    }

    /// Checks if the container has not exited and is marked as `healthy`
    ///
    /// NOTE: If the container has not yet been created, this will immediately return `None`.
    ///
    /// # Errors
    ///
    /// * Failed to get the list of containers
    /// * The container status could not be parsed
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    /// use docktopus::container::Container;
    /// use std::time::Duration;
    /// use tokio::time;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), docktopus::container::Error> {
    /// let connection = DockerBuilder::new().await?;
    /// let mut container = Container::new(connection.client(), "rustlang/rust");
    ///
    /// container.cmd(["echo", "Hello!"]);
    ///
    /// let wait_for_exit = false;
    /// container.start(wait_for_exit).await?;
    ///
    /// loop {
    ///     let status = container.status().await?.unwrap();
    ///     if status.is_active() {
    ///         time::sleep(Duration::from_secs(5)).await;
    ///         continue;
    ///     }
    ///
    ///     println!("Container exited!");
    ///     break;
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn status(&self) -> Result<Option<ContainerStatus>, Error> {
        let Some(id) = self.id.as_deref() else {
            return Ok(None);
        };

        let mut filters = HashMap::new();
        let _ = filters.insert("id", vec![id]);

        let options = Some(ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        });

        let containers = self.client.list_containers(options).await?;
        let Some(status) = &containers[0].state else {
            return Ok(None);
        };

        ContainerStatus::from_str(status.as_str()).map(Some)
    }

    /// Stop a running container
    ///
    /// NOTE: It is not an error to call this on a container that has not been started,
    ///       it will simply do nothing.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    /// use docktopus::container::Container;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), docktopus::container::Error> {
    /// let connection = DockerBuilder::new().await?;
    ///
    /// let mut container = Container::new(connection.client(), "rustlang/rust");
    ///
    /// // Does nothing, the container isn't started
    /// container.stop().await?;
    ///
    /// // Stops the running container
    /// container.start(false).await?;
    /// container.stop().await?;
    /// # Ok(()) }
    /// ```
    #[tracing::instrument(skip_all)]
    pub async fn stop(&mut self) -> Result<(), bollard::errors::Error> {
        let Some(id) = &self.id else {
            log::warn!("Container not started");
            return Ok(());
        };

        self.client
            .stop_container(id, None::<StopContainerOptions>)
            .await?;

        Ok(())
    }

    /// Remove a container
    ///
    /// NOTE: To remove a running container, a [`RemoveContainerOptions`] must be provided
    ///       with the `force` flag set.
    ///
    /// See also: [`bollard::container::RemoveContainerOptions`]
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    /// use docktopus::container::Container;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), docktopus::container::Error> {
    /// let connection = DockerBuilder::new().await?;
    ///
    /// let mut container = Container::new(connection.client(), "rustlang/rust");
    ///
    /// // Start our container
    /// container.start(false).await?;
    ///
    /// let remove_container_options = bollard::container::RemoveContainerOptions {
    ///     force: true,
    ///     ..Default::default()
    /// };
    ///
    /// // Kills the container and removes it
    /// container.remove(Some(remove_container_options)).await?;
    /// # Ok(()) }
    /// ```
    ///
    /// [`RemoveContainerOptions::force`]: bollard::container::RemoveContainerOptions::force
    #[tracing::instrument(skip(self))]
    pub async fn remove(
        mut self,
        options: Option<bollard::container::RemoveContainerOptions>,
    ) -> Result<(), bollard::errors::Error> {
        let Some(id) = self.id.take() else {
            log::warn!("Container not started");
            return Ok(());
        };

        self.client.remove_container(&id, options).await?;
        Ok(())
    }

    /// Wait for a container to exit
    ///
    /// NOTE: It is not an error to call this on a container that has not been started,
    ///       it will simply do nothing.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    /// use docktopus::container::Container;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), docktopus::container::Error> {
    /// let connection = DockerBuilder::new().await?;
    ///
    /// let mut container = Container::new(connection.client(), "rustlang/rust");
    ///
    /// // Start our container
    /// container.start(false).await?;
    ///
    /// // Once this returns, we know that the container has exited.
    /// container.wait().await?;
    /// # Ok(()) }
    /// ```
    #[tracing::instrument(skip_all)]
    pub async fn wait(&self) -> Result<(), bollard::errors::Error> {
        let Some(id) = &self.id else {
            log::warn!("Container not created");
            return Ok(());
        };

        wait_for_container(&self.client, id).await?;
        Ok(())
    }

    /// Fetch the container log stream
    ///
    /// NOTE: It is not an error to call this on a container that has not been started,
    ///       it will simply do nothing and return `None`.
    ///
    /// See also:
    ///
    /// * [`bollard::container::LogsOptions`]
    /// * [`bollard::container::LogOutput`]
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use docktopus::DockerBuilder;
    /// use docktopus::container::Container;
    /// use futures::StreamExt;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), docktopus::container::Error> {
    /// let connection = DockerBuilder::new().await?;
    /// let mut container = Container::new(connection.client(), "rustlang/rust");
    ///
    /// // Start our container and wait for it to exit
    /// container.start(true).await?;
    ///
    /// // We want to collect logs from stderr
    /// let logs_options = bollard::container::LogsOptions {
    ///     stderr: true,
    ///     follow: true,
    ///     ..Default::default()
    /// };
    ///
    /// // Get our log stream
    /// let mut logs = container
    ///     .logs(Some(logs_options))
    ///     .await
    ///     .expect("logs should be present");
    ///
    /// // Now we want to print anything from stderr
    /// while let Some(Ok(out)) = logs.next().await {
    ///     if let bollard::container::LogOutput::StdErr { message } = out {
    ///         eprintln!("Uh oh! Something was written to stderr: {:?}", message);
    ///     }
    /// }
    /// # Ok(()) }
    /// ```
    #[tracing::instrument(skip(self))]
    pub async fn logs(
        &self,
        logs_options: Option<bollard::container::LogsOptions<String>>,
    ) -> Option<impl Stream<Item = Result<bollard::container::LogOutput, bollard::errors::Error>>>
    {
        let Some(id) = &self.id else {
            log::warn!("Container not created");
            return None;
        };

        Some(self.client.logs(id, logs_options))
    }
}

async fn wait_for_container(docker: &Docker, id: &str) -> Result<(), bollard::errors::Error> {
    let options = WaitContainerOptions {
        condition: "not-running",
    };

    let mut wait_stream = docker.wait_container(id, Some(options));

    while let Some(msg) = wait_stream.next().await {
        match msg {
            Ok(msg) => {
                if msg.status_code == 0 {
                    break;
                }

                if let Some(err) = msg.error {
                    log::error!("Failed to wait for container: {:?}", err.message);
                    // TODO: These aren't the same error type, is this correct?
                    return Err(bollard::errors::Error::DockerContainerWaitError {
                        error: err.message.unwrap_or_default(),
                        code: msg.status_code,
                    });
                }
            }
            Err(e) => {
                match &e {
                    bollard::errors::Error::DockerContainerWaitError { error, code } => {
                        log::error!("Container failed with status code `{}`: {error}", code);
                    }
                    _ => log::error!("Container failed with error: {:?}", e),
                }
                return Err(e);
            }
        }
    }

    Ok(())
}
