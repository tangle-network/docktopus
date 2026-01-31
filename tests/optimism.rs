mod common;

use bollard::query_parameters::ListContainersOptionsBuilder;
use color_eyre::Result;
use common::with_docker_cleanup;
use docktopus::parser::ComposeParser;
use docktopus::{ComposeConfig, DockerBuilder, DockerError};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs;
use uuid::Uuid;

const OPTIMISM_FIXTURES: &str = "./fixtures/simple-optimism-node";
const TEST_BASE_DIR: &str = "./test-data";

pub struct OptimismTestContext {
    pub builder: DockerBuilder,
    pub config: ComposeConfig,
    pub test_dir: PathBuf,
}

#[allow(clippy::missing_errors_doc, clippy::missing_panics_doc, missing_docs)]
impl OptimismTestContext {
    pub async fn new(test_id: &str) -> Result<Self> {
        let builder = DockerBuilder::new().await?;

        // Create test directory under the common test base
        let test_dir = PathBuf::from(TEST_BASE_DIR)
            .join("optimism")
            .join(format!("test-{}", Uuid::new_v4()));

        // Set up initial config
        let compose_path = PathBuf::from(OPTIMISM_FIXTURES).join("docker-compose.yml");
        let env_path = PathBuf::from("./tests/optimism_env.env");

        // Parse config with environment variables
        let mut config = ComposeParser::new()
            .env_file(env_path)
            .parse_from_path(&compose_path)?;

        // Add test_id label to each service
        for service in config.services.values_mut() {
            let mut labels = service.labels.clone().unwrap_or_default();
            labels.insert("test_id".to_string(), test_id.to_string());
            service.labels = Some(labels);

            // Add platform for specific services that need it
            if let Some(
                "ethereumoptimism/replica-healthcheck:latest"
                | "ethereumoptimism/l2geth:latest"
                | "us-docker.pkg.dev/oplabs-tools-artifacts/images/op-geth:v1.101411.4",
            ) = service.image.as_deref()
            {
                service.platform = Some("linux/amd64".to_string());
            }
        }

        Ok(Self {
            builder,
            config,
            test_dir,
        })
    }

    pub async fn deploy(&mut self) -> Result<HashMap<String, String>> {
        // First ensure all directories exist
        self.setup_directories().await?;

        // Verify build context exists
        let dockerfile_path = self
            .test_dir
            .join("docker/dockerfiles/Dockerfile.bedrock-init");
        if !dockerfile_path.exists() {
            return Err(DockerError::ValidationError(format!(
                "Dockerfile not found at expected path: {}",
                dockerfile_path.display()
            ))
            .into());
        }
        println!("Found Dockerfile at: {}", dockerfile_path.display());

        // Now deploy the services using the test directory as base for bind mounts
        self.builder
            .deploy_compose_with_base_dir(&mut self.config, self.test_dir.clone())
            .await
            .map_err(Into::into)
    }

    pub async fn setup_directories(&self) -> Result<()> {
        println!("\n=== Setting up test directories ===");
        println!("Test directory: {}", self.test_dir.display());

        // Create test directory
        fs::create_dir_all(&self.test_dir).await.map_err(|e| {
            DockerError::ValidationError(format!("Failed to create test directory: {}", e))
        })?;

        // Define directories to copy from fixtures
        let dirs_to_copy = [
            "docker/prometheus",
            "docker/grafana/provisioning",
            "docker/grafana/dashboards",
            "docker/influxdb",
            "scripts",
            "envs/common",
            "envs/op-mainnet",
            "envs/op-mainnet/config",
        ];

        // Copy each directory from fixtures to test directory
        for dir in dirs_to_copy {
            let src_dir = Path::new(OPTIMISM_FIXTURES).join(dir);
            let dst_dir = self.test_dir.join(dir);

            println!("\nCopying directory:");
            println!("  From: {}", src_dir.display());
            println!("  To: {}", dst_dir.display());

            // Create parent directory
            if let Some(parent) = dst_dir.parent() {
                fs::create_dir_all(parent).await.map_err(|e| {
                    DockerError::ValidationError(format!(
                        "Failed to create parent directory {}: {}",
                        parent.display(),
                        e
                    ))
                })?;
            }

            // Copy directory contents
            if src_dir.exists() {
                tokio::process::Command::new("cp")
                    .arg("-r")
                    .arg(&src_dir)
                    .arg(dst_dir.parent().unwrap())
                    .status()
                    .await
                    .map_err(|e| {
                        DockerError::ValidationError(format!(
                            "Failed to copy directory {}: {}",
                            dir, e
                        ))
                    })?;
            } else {
                println!(
                    "Warning: Source directory does not exist: {}",
                    src_dir.display()
                );
                fs::create_dir_all(&dst_dir).await.map_err(|e| {
                    DockerError::ValidationError(format!(
                        "Failed to create directory {}: {}",
                        dst_dir.display(),
                        e
                    ))
                })?;
            }
        }

        // Create additional required directories
        println!("\n=== Creating additional directories ===");
        let additional_dirs = ["shared", "downloads", "torrents/op-mainnet"];

        for dir in additional_dirs {
            let dir_path = self.test_dir.join(dir);
            println!("Creating directory: {}", dir_path.display());
            fs::create_dir_all(&dir_path).await.map_err(|e| {
                DockerError::ValidationError(format!(
                    "Failed to create directory {}: {}",
                    dir_path.display(),
                    e
                ))
            })?;
        }

        // Copy docker-compose.yml
        println!("\n=== Copying compose files ===");
        let compose_src = Path::new(OPTIMISM_FIXTURES).join("docker-compose.yml");
        let compose_dst = self.test_dir.join("docker-compose.yml");
        println!("Copying docker-compose.yml:");
        println!("  From: {}", compose_src.display());
        println!("  To: {}", compose_dst.display());
        fs::copy(&compose_src, &compose_dst).await.map_err(|e| {
            DockerError::ValidationError(format!("Failed to copy docker-compose.yml: {}", e))
        })?;

        // Copy optimism_env.env to .env
        let env_src = PathBuf::from("tests/optimism_env.env");
        let env_dst = self.test_dir.join(".env");
        println!("Copying env file:");
        println!("  From: {}", env_src.display());
        println!("  To: {}", env_dst.display());
        fs::copy(&env_src, &env_dst).await.map_err(|e| {
            DockerError::ValidationError(format!("Failed to copy optimism_env.env to .env: {}", e))
        })?;

        // Set up Docker build context
        println!("\n=== Setting up Docker build context ===");
        let dockerfiles_dir = self.test_dir.join("docker/dockerfiles");
        fs::create_dir_all(&dockerfiles_dir).await.map_err(|e| {
            DockerError::ValidationError(format!("Failed to create dockerfiles directory: {}", e))
        })?;

        // Copy Dockerfile with explicit fs::copy
        let dockerfile_src =
            Path::new(OPTIMISM_FIXTURES).join("docker/dockerfiles/Dockerfile.bedrock-init");
        let dockerfile_dst = dockerfiles_dir.join("Dockerfile.bedrock-init");

        println!("Copying Dockerfile:");
        println!("  From: {}", dockerfile_src.display());
        println!("  To: {}", dockerfile_dst.display());

        fs::copy(&dockerfile_src, &dockerfile_dst)
            .await
            .map_err(|e| {
                DockerError::ValidationError(format!(
                    "Failed to copy Dockerfile.bedrock-init: {}",
                    e
                ))
            })?;

        // Verify the Dockerfile was copied correctly
        match fs::read_to_string(&dockerfile_dst).await {
            Ok(content) => {
                println!("Verified Dockerfile contents ({} bytes)", content.len());
                println!("First few lines:");
                for line in content.lines().take(5) {
                    println!("  {}", line);
                }
            }
            Err(e) => println!("Warning: Could not read copied Dockerfile: {}", e),
        }

        // List the final dockerfiles directory
        println!("\nFinal dockerfiles directory contents:");
        if let Ok(mut entries) = fs::read_dir(&dockerfiles_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                println!("  {}", entry.path().display());
            }
        }

        Ok(())
    }
}

#[tokio::test]
async fn test_optimism_node_deployment() -> Result<()> {
    with_docker_cleanup(|test_id| {
        Box::pin(async move {
            let mut ctx = OptimismTestContext::new(&test_id).await?;

            // Deploy all services and verify they're running
            let container_ids = ctx.deploy().await?;

            // Verify services are running
            for (service_name, container_id) in container_ids {
                println!(
                    "Verifying container for service {} with id {}",
                    service_name, container_id
                );

                // Verify container is running
                let mut retries = 5;
                let mut container_running = false;
                while retries > 0 {
                    let mut filters: HashMap<String, Vec<String>> = HashMap::new();
                    filters.insert("id".to_string(), vec![container_id.clone()]);
                    let list_opts = ListContainersOptionsBuilder::default()
                        .all(true)
                        .filters(&filters)
                        .build();
                    if let Ok(containers) =
                        ctx.builder.client().list_containers(Some(list_opts)).await
                    {
                        if !containers.is_empty() {
                            container_running = true;
                            break;
                        }
                    }
                    retries -= 1;
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
                assert!(
                    container_running,
                    "Container for service {} should be running",
                    service_name
                );
            }

            Ok(())
        })
    })
    .await
}
