# Docktopus

A high-performance Rust library for programmatic container orchestration, offering type-safe Docker and Docker Compose
operations with advanced deployment capabilities.

## Overview

Docktopus bridges the gap between static container configurations and dynamic runtime orchestration. It provides a
robust, type-safe interface for programmatically managing containers, with first-class support for both Dockerfile and
Docker Compose workflows.

## Key Features

### 🔒 Type-Safe Container Orchestration

- **Compile-time Validation**: Catch configuration errors before deployment with Rust's type system
- **Safe API Abstractions**: Memory-safe wrappers around Docker Engine API
- **Error Recovery**: Structured error handling with automatic cleanup and rollback capabilities

### 🚀 Advanced Deployment Capabilities

- **Intelligent Build Context**: Automatic dependency tracking and efficient layer caching
- **Variable Substitution**: Advanced environment variable interpolation with fallback support
- **Resource Management**: Fine-grained control over CPU, memory, and network resources
- **Health Monitoring**: Sophisticated health check system with customizable probes

### 🔄 Docker Compose Integration

- **Declarative & Programmatic**: Use YAML configs or build service definitions in code
- **Dependency Resolution**: Automatic service ordering based on dependency graphs
- **Network Isolation**: Automatic network creation and service discovery
- **Volume Management**: Type-safe volume mounting with automatic cleanup

### 🛠 Developer Experience

- **Async-first Design**: Built on `tokio` for high-performance async I/O
- **Modular Architecture**: Use only what you need with feature flags
- **Comprehensive Logging**: Detailed operation logging for debugging
- **Testing Utilities**: Rich testing infrastructure for container-based tests

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
# For parsing only (no deployment features)
docktopus = { version = "0.1.0" }

# For full functionality including deployment (default)
docktopus = "0.1.0"

# Or explicitly with all features
docktopus = { version = "0.1.0", features = ["docker"] }
```

## Usage

### Feature Flags

- `parser` - Enables Dockerfile and Docker Compose parsing functionality (minimal dependencies)
- `deploy` - Enables deployment features using Bollard (includes parser features)

### Parser-Only Usage

```rust
use docktopus::{DockerfileConfig, DockerfileParser, ComposeParser};

// Parse a Dockerfile
let dockerfile_content = std::fs::read_to_string("Dockerfile")?;
let config = DockerfileParser::parse(&dockerfile_content)?;

// Parse a Docker Compose file
let compose_content = std::fs::read_to_string("docker-compose.yml")?;
let compose_config = ComposeParser::parse(&compose_content)?;
```

### Full Deployment Usage

When the `deploy` feature is enabled:

```rust
use docktopus::DockerBuilder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let builder = DockerBuilder::new()?;
    // ... deployment functionality available
    Ok(())
}
```

### Basic Dockerfile Deployment

```rust
use docktopus::{DockerBuilder, DockerfileConfig, DockerCommand};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let builder = DockerBuilder::new()?;

    // Parse existing Dockerfile
    let config = builder.from_dockerfile("path/to/Dockerfile").await?;

    // Or create programmatically
    let config = DockerfileConfig {
        base_image: "rust:1.70".to_string(),
        commands: vec![
            DockerCommand::Run {
                command: "cargo build".to_string()
            },
            DockerCommand::Copy {
                source: "./target".to_string(),
                dest: "/app".to_string(),
                chown: None,
            },
        ],
    };

    // Deploy container
    let container_id = builder.deploy_dockerfile(&config, "my-app:latest").await?;
    Ok(())
}
```

### Docker Compose Deployment

```rust
use docktopus::{DockerBuilder, ComposeConfig};
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let builder = DockerBuilder::new()?;

    // Parse compose file
    let config = builder.from_compose("docker-compose.yml").await?;

    // Deploy services
    let container_ids: HashMap<String, String> = builder.deploy_compose(&config).await?;
    Ok(())
}
```

### Resource Management

```rust
use docktopus::{Service, ResourceLimits};

let service = Service {
    image: Some("nginx:latest".to_string()),
    requirements: Some(ResourceLimits {
        cpu_limit: Some(0.5),                        // Half a CPU
        memory_limit: Some("512M".to_string()),      // 512MB memory limit
        memory_swap: Some("1G".to_string()),         // 1GB swap limit
        memory_reservation: Some("256M".to_string()), // 256MB soft limit
        cpus_shares: Some(512),                      // CPU shares (relative weight)
        cpuset_cpus: Some("0,1".to_string()),        // Run on CPUs 0 and 1
    }),
    ..Default::default()
};
```

### Health Checks

```rust
use docktopus::{Service, config::compose::HealthCheck};

let service = Service {
    image: Some("nginx:latest".to_string()),
    healthcheck: Some(HealthCheck {
        test: vec![
            "CMD-SHELL".to_string(),
            "curl -f http://localhost/ || exit 1".to_string()
        ],
        interval: Some(1_000_000_000),    // 1 second
        timeout: Some(3_000_000_000),     // 3 seconds
        retries: Some(3),
        start_period: Some(2_000_000_000), // 2 seconds
        start_interval: None,
    }),
    ..Default::default()
};
```

### Container Management

```rust
use docktopus::DockerBuilder;
use std::collections::HashMap;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let builder = DockerBuilder::new()?;

    // Get container logs
    let logs = builder.get_container_logs("container_id").await?;

    // Execute command in container
    let output = builder.exec_in_container(
        "container_id",
        vec!["ls", "-la"],
        Some(HashMap::new()),
    ).await?;

    // Create and manage volumes
    builder.create_volume("my_volume").await?;
    let volumes = builder.list_volumes().await?;

    Ok(())
}
```

## Error Handling

The library provides detailed error types for proper error handling:

```rust
use docktopus::DockerError;

match result {
    Err(DockerError::FileError(e)) => println!("File error: {}", e),
    Err(DockerError::YamlError(e)) => println!("YAML parsing error: {}", e),
    Err(DockerError::DockerfileError(e)) => println!("Dockerfile parsing error: {}", e),
    Err(DockerError::BollardError(e)) => println!("Docker API error: {}", e),
    Err(DockerError::InvalidIpamConfig) => println!("Invalid network configuration"),
    Err(DockerError::ContainerNotRunning(id)) => println!("Container {} not running", id),
    Err(DockerError::NetworkCreationError(e)) => println!("Network creation failed: {}", e),
    Err(DockerError::InvalidResourceLimit(e)) => println!("Invalid resource limit: {}", e),
    Ok(_) => println!("Operation succeeded"),
}
```

## Development Status

This library is in active development. Current focus areas:

- Enhanced error handling with recovery strategies
- Comprehensive integration testing
- Additional Docker Compose features
- Improved resource management
- Better developer ergonomics

## License

This project is licensed under either of

* Apache License, Version 2.0
  ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
* MIT license
  ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your discretion.

## Local qualification

Automatic pull-request and main-push CI is retired. Before merging source changes, merge the current default branch and run the retired formatting, Clippy, build/test and documentation checks through the maintained admitted gate:

```bash
beelink-gate auto https://github.com/tangle-network/docktopus.git <full-sha> -- bash -lc 'cargo fmt -- --check && cargo clippy --tests --examples -- -D warnings && cargo build --tests && cargo nextest run --profile ci && cargo test --doc'
```

The admitted host needs Rust with rustfmt/Clippy, cargo-nextest, the repository submodule fixtures and Docker for container integration checks. Select any fixture preparation through its maintained owner before running Docker checks; a source-only check does not prove container deployment. Workflow/documentation-only changes use YAML, actionlint and content/exact-diff checks. Qualification does not publish a crate or authorize production container changes.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request. When contributing:

1. Fork the repository
2. Create a feature branch
3. Add tests for new functionality
4. Ensure all tests pass
5. Submit a pull request

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.

## Acknowledgments

- [Bollard](https://github.com/fussybeaver/bollard) - The underlying Docker API client
- [Docker](https://www.docker.com/) - Container platform
