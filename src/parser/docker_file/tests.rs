use crate::DockerError;
use crate::config::docker_file::{DockerCommand, DockerfileConfig};
use crate::test_fixtures::get_tangle_dockerfile;
use std::collections::HashMap;

#[tokio::test]
async fn test_dockerfile_parsing() {
    let config = DockerfileConfig::parse_from_path(get_tangle_dockerfile()).unwrap();

    assert_eq!(config.base_image, "ubuntu:22.04");
    assert_eq!(config.commands.len(), 11);

    let commands = &config.commands;

    // Test COPY
    match &commands[2] {
        DockerCommand::Copy {
            source,
            dest,
            chown,
        } => {
            assert_eq!(source, "./target/release/tangle");
            assert_eq!(dest, "/usr/local/bin/");
            assert!(chown.is_none());
        }
        _ => panic!("Expected COPY command"),
    }

    // Verify RUN command with multiple operations
    match &commands[3] {
        DockerCommand::Run { command } => {
            assert!(command.contains("useradd -m -u 5000"));
            assert!(command.contains("mkdir -p /data /tangle/.local/share"));
            assert!(command.contains("chown -R tangle:tangle /data"));
        }
        _ => panic!("Expected RUN command"),
    }

    // Verify USER command
    match &commands[4] {
        DockerCommand::User { user, group } => {
            assert_eq!(user, "tangle");
            assert!(group.is_none());
        }
        _ => panic!("Expected USER command"),
    }

    // Verify EXPOSE commands
    let expected_ports = [30333, 9933, 9944, 9615];
    for (i, port) in expected_ports.iter().enumerate() {
        match &commands[i + 5] {
            DockerCommand::Expose { port: p, protocol } => {
                assert_eq!(p, port);
                assert!(protocol.is_none());
            }
            _ => panic!("Expected EXPOSE command"),
        }
    }
}

#[tokio::test]
async fn test_dockerfile_content_generation() {
    let config = DockerfileConfig {
        base_image: "rust:1.70".to_string(),
        commands: vec![
            DockerCommand::Run {
                command: "cargo build".to_string(),
            },
            DockerCommand::Copy {
                source: "./target".to_string(),
                dest: "/app".to_string(),
                chown: None,
            },
            DockerCommand::Env {
                key: "RUST_LOG".to_string(),
                value: "debug".to_string(),
            },
            DockerCommand::Workdir {
                path: "/app".to_string(),
            },
            DockerCommand::Expose {
                port: 8080,
                protocol: None,
            },
        ],
    };

    let content = config.to_string();
    let expected = r"FROM rust:1.70
RUN cargo build
COPY ./target /app
ENV RUST_LOG=debug
WORKDIR /app
EXPOSE 8080
";

    assert_eq!(content, expected);
}

#[tokio::test]
async fn test_all_dockerfile_commands() {
    let content = r#"
        FROM ubuntu:22.04
        LABEL version="1.0" description="Test image"
        MAINTAINER Test User <test@example.com>
        ARG BUILD_VERSION
        ARG DEBUG=false
        ENV APP_VERSION=${BUILD_VERSION}
        ENV DEBUG_MODE=${DEBUG}
        WORKDIR /app
        ADD --chown=user:group http://example.com/file.tar.gz /app/
        ADD ["file1.txt", "file2.txt", "/app/"]
        COPY --chown=user:group src/ /app/
        RUN apt-get update && \
            apt-get install -y python3
        EXPOSE 8080/tcp
        EXPOSE 8081
        USER myuser:mygroup
        VOLUME ["/data", "/config"]
        HEALTHCHECK --interval=30s --timeout=10s --retries=3 CMD curl -f http://localhost/ || exit 1
        SHELL ["/bin/bash", "-c"]
        STOPSIGNAL SIGTERM
        ONBUILD ADD . /app/src
        ENTRYPOINT ["./entrypoint.sh"]
        CMD ["--help"]
    "#;

    let config = DockerfileConfig::parse(content).unwrap();
    assert_eq!(config.base_image, "ubuntu:22.04");

    let mut commands_iter = config.commands.iter();

    // Test LABEL
    if let Some(DockerCommand::Label { labels }) = commands_iter.next() {
        assert_eq!(labels.get("version").unwrap(), "1.0");
        assert_eq!(labels.get("description").unwrap(), "Test image");
    } else {
        panic!("Expected LABEL command");
    }

    // Test MAINTAINER
    if let Some(DockerCommand::Maintainer { name }) = commands_iter.next() {
        assert_eq!(name, "Test User <test@example.com>");
    } else {
        panic!("Expected MAINTAINER command");
    }

    // Test ARGs
    if let Some(DockerCommand::Arg {
        name,
        default_value,
    }) = commands_iter.next()
    {
        assert_eq!(name, "BUILD_VERSION");
        assert!(default_value.is_none());
    } else {
        panic!("Expected ARG command");
    }

    if let Some(DockerCommand::Arg {
        name,
        default_value,
    }) = commands_iter.next()
    {
        assert_eq!(name, "DEBUG");
        assert_eq!(default_value.as_deref(), Some("false"));
    } else {
        panic!("Expected ARG command");
    }

    // Test ENVs
    if let Some(DockerCommand::Env { key, value }) = commands_iter.next() {
        assert_eq!(key, "APP_VERSION");
        assert_eq!(value, "${BUILD_VERSION}");
    } else {
        panic!("Expected ENV command");
    }

    if let Some(DockerCommand::Env { key, value }) = commands_iter.next() {
        assert_eq!(key, "DEBUG_MODE");
        assert_eq!(value, "${DEBUG}");
    } else {
        panic!("Expected ENV command");
    }

    // Test WORKDIR
    if let Some(DockerCommand::Workdir { path }) = commands_iter.next() {
        assert_eq!(path, "/app");
    } else {
        panic!("Expected WORKDIR command");
    }

    // Test ADD
    if let Some(DockerCommand::Add {
        sources,
        dest,
        chown,
    }) = commands_iter.next()
    {
        assert_eq!(*sources, vec!["http://example.com/file.tar.gz"]);
        assert_eq!(dest, "/app/");
        assert_eq!(chown.as_deref(), Some("user:group"));
    } else {
        panic!("Expected ADD command");
    }

    if let Some(DockerCommand::Add {
        sources,
        dest,
        chown,
    }) = commands_iter.next()
    {
        assert_eq!(*sources, vec!["file1.txt", "file2.txt"]);
        assert_eq!(dest, "/app/");
        assert!(chown.is_none());
    } else {
        panic!("Expected ADD command");
    }

    // Test COPY
    if let Some(DockerCommand::Copy {
        source,
        dest,
        chown,
    }) = commands_iter.next()
    {
        assert_eq!(source, "src/");
        assert_eq!(dest, "/app/");
        assert_eq!(chown.as_deref(), Some("user:group"));
    } else {
        panic!("Expected COPY command");
    }

    // Test RUN
    if let Some(DockerCommand::Run { command }) = commands_iter.next() {
        assert_eq!(command, "apt-get update &&  apt-get install -y python3");
    } else {
        panic!("Expected RUN command");
    }

    // Test EXPOSE
    if let Some(DockerCommand::Expose { port, protocol }) = commands_iter.next() {
        assert_eq!(*port, 8080);
        assert_eq!(protocol.as_deref(), Some("tcp"));
    } else {
        panic!("Expected EXPOSE command");
    }

    if let Some(DockerCommand::Expose { port, protocol }) = commands_iter.next() {
        assert_eq!(*port, 8081);
        assert!(protocol.is_none());
    } else {
        panic!("Expected EXPOSE command");
    }

    // Test USER
    if let Some(DockerCommand::User { user, group }) = commands_iter.next() {
        assert_eq!(user, "myuser");
        assert_eq!(group.as_deref(), Some("mygroup"));
    } else {
        panic!("Expected USER command");
    }

    // Test VOLUME
    if let Some(DockerCommand::Volume { paths }) = commands_iter.next() {
        assert_eq!(*paths, vec!["/data", "/config"]);
    } else {
        panic!("Expected VOLUME command");
    }

    // Test HEALTHCHECK
    if let Some(DockerCommand::Healthcheck {
        command,
        interval,
        timeout,
        retries,
        ..
    }) = commands_iter.next()
    {
        assert_eq!(
            *command,
            vec!["curl", "-f", "http://localhost/", "||", "exit", "1"]
        );
        assert_eq!(interval.as_deref(), Some("30s"));
        assert_eq!(timeout.as_deref(), Some("10s"));
        assert_eq!(*retries, Some(3));
    } else {
        panic!("Expected HEALTHCHECK command");
    }

    // Test SHELL
    if let Some(DockerCommand::Shell { shell }) = commands_iter.next() {
        assert_eq!(*shell, vec!["/bin/bash", "-c"]);
    } else {
        panic!("Expected SHELL command");
    }

    // Test STOPSIGNAL
    if let Some(DockerCommand::StopSignal { signal }) = commands_iter.next() {
        assert_eq!(signal, "SIGTERM");
    } else {
        panic!("Expected STOPSIGNAL command");
    }

    // Test ONBUILD
    if let Some(DockerCommand::Onbuild { command }) = commands_iter.next() {
        match command.as_ref() {
            DockerCommand::Add {
                sources,
                dest,
                chown,
            } => {
                assert_eq!(*sources, vec!["."]);
                assert_eq!(dest, "/app/src");
                assert!(chown.is_none());
            }
            _ => panic!("Expected ONBUILD ADD command"),
        }
    } else {
        panic!("Expected ONBUILD command");
    }

    // Test ENTRYPOINT
    if let Some(DockerCommand::Entrypoint { command }) = commands_iter.next() {
        assert_eq!(*command, vec!["./entrypoint.sh"]);
    } else {
        panic!("Expected ENTRYPOINT command");
    }

    // Test CMD
    if let Some(DockerCommand::Cmd { command }) = commands_iter.next() {
        assert_eq!(*command, vec!["--help"]);
    } else {
        panic!("Expected CMD command");
    }
}

#[tokio::test]
async fn test_invalid_dockerfile_syntax() {
    let content = "COPY";
    let result = DockerfileConfig::parse(content);
    assert!(matches!(
        result,
        Err(DockerError::DockerfileError(msg)) if msg == "Invalid command syntax"
    ));

    let content = "COPY src";
    let result = DockerfileConfig::parse(content);
    assert!(matches!(
        result,
        Err(DockerError::DockerfileError(msg)) if msg == "COPY requires source and destination"
    ));

    let content = "UNKNOWN command";
    let result = DockerfileConfig::parse(content);
    assert!(matches!(
        result,
        Err(DockerError::DockerfileError(msg)) if msg == "Unknown command: UNKNOWN"
    ));
}

#[tokio::test]
async fn test_invalid_onbuild() {
    let content = "ONBUILD";
    let result = DockerfileConfig::parse(content);
    assert!(matches!(
        result,
        Err(DockerError::DockerfileError(msg)) if msg == "Invalid command syntax"
    ));

    let content = "ONBUILD INVALID something";
    let result = DockerfileConfig::parse(content);
    assert!(matches!(
        result,
        Err(DockerError::DockerfileError(msg)) if msg == "Unknown command: INVALID"
    ));

    // Test valid ONBUILD commands
    let content = "ONBUILD ADD . /usr/src/app";
    let config = DockerfileConfig::parse(content).unwrap();
    match &config.commands[0] {
        DockerCommand::Onbuild { command } => match command.as_ref() {
            DockerCommand::Add {
                sources,
                dest,
                chown,
            } => {
                assert_eq!(sources, &vec!["."]);
                assert_eq!(dest, "/usr/src/app");
                assert!(chown.is_none());
            }
            _ => panic!("Expected ADD command inside ONBUILD"),
        },
        _ => panic!("Expected ONBUILD command"),
    }

    let content = "ONBUILD RUN mvn install";
    let config = DockerfileConfig::parse(content).unwrap();
    match &config.commands[0] {
        DockerCommand::Onbuild { command } => match command.as_ref() {
            DockerCommand::Run { command } => {
                assert_eq!(command, "mvn install");
            }
            _ => panic!("Expected RUN command inside ONBUILD"),
        },
        _ => panic!("Expected ONBUILD command"),
    }
}

#[tokio::test]
async fn test_invalid_healthcheck() {
    let content = "HEALTHCHECK --invalid-flag CMD curl localhost";
    let result = DockerfileConfig::parse(content);
    assert!(matches!(
        result,
        Err(DockerError::DockerfileError(msg)) if msg == "Invalid HEALTHCHECK flag: --invalid-flag"
    ));

    let content = "HEALTHCHECK --interval";
    let result = DockerfileConfig::parse(content);
    assert!(matches!(
        result,
        Err(DockerError::DockerfileError(msg)) if msg == "Missing value for --interval flag"
    ));

    let content = "HEALTHCHECK --interval 30s";
    let result = DockerfileConfig::parse(content);
    assert!(matches!(
        result,
        Err(DockerError::DockerfileError(msg)) if msg == "HEALTHCHECK must include CMD"
    ));
}

#[tokio::test]
async fn test_comments_and_empty_lines() {
    let content = r#"
        # This is a comment
        FROM ubuntu:22.04

        # Another comment
        RUN echo "test"

        # Final comment
    "#;

    let config = DockerfileConfig::parse(content).unwrap();
    assert_eq!(config.base_image, "ubuntu:22.04");
    assert_eq!(config.commands.len(), 1);
}

#[tokio::test]
async fn test_expose_multiple_ports() {
    let content = "EXPOSE 30333 9933 9944 9615";
    let config = DockerfileConfig::parse(content).unwrap();

    let expected_ports = [30333, 9933, 9944, 9615];
    assert_eq!(config.commands.len(), expected_ports.len());

    for (i, port) in expected_ports.iter().enumerate() {
        match &config.commands[i] {
            DockerCommand::Expose { port: p, protocol } => {
                assert_eq!(p, port);
                assert!(protocol.is_none());
            }
            _ => panic!("Expected EXPOSE command"),
        }
    }

    // Test mixed format
    let content = "EXPOSE 80/tcp 443 8080/udp 9000";
    let config = DockerfileConfig::parse(content).unwrap();
    assert_eq!(config.commands.len(), 4);

    let expected = [
        (80, Some("tcp")),
        (443, None),
        (8080, Some("udp")),
        (9000, None),
    ];

    for (i, (port, protocol)) in expected.iter().enumerate() {
        match &config.commands[i] {
            DockerCommand::Expose {
                port: p,
                protocol: proto,
            } => {
                assert_eq!(p, port);
                assert_eq!(proto.as_deref(), *protocol);
            }
            _ => panic!("Expected EXPOSE command"),
        }
    }
}

#[tokio::test]
async fn test_tangle_expose_format() {
    let content = "EXPOSE 30333 9933 9944 9615";
    let config = DockerfileConfig::parse(content).unwrap();

    let expected_ports = [30333, 9933, 9944, 9615];
    assert_eq!(config.commands.len(), expected_ports.len());

    for (i, expected_port) in expected_ports.iter().enumerate() {
        match &config.commands[i] {
            DockerCommand::Expose { port, protocol } => {
                assert_eq!(port, expected_port);
                assert!(protocol.is_none());
            }
            _ => panic!("Expected EXPOSE command"),
        }
    }

    // Test error case
    let content = "EXPOSE 30333 invalid 9944";
    let result = DockerfileConfig::parse(content);
    assert!(matches!(
        result,
        Err(DockerError::DockerfileError(msg)) if msg == "Invalid port number: invalid"
    ));
}

#[tokio::test]
async fn test_onbuild_commands() {
    // Test all possible ONBUILD combinations
    let test_cases = vec![
        (
            "ONBUILD ADD . /app",
            DockerCommand::Add {
                sources: vec![".".to_string()],
                dest: "/app".to_string(),
                chown: None,
            },
        ),
        (
            "ONBUILD COPY --chown=user:group src/ /app/",
            DockerCommand::Copy {
                source: "src/".to_string(),
                dest: "/app/".to_string(),
                chown: Some("user:group".to_string()),
            },
        ),
        (
            "ONBUILD RUN cargo build",
            DockerCommand::Run {
                command: "cargo build".to_string(),
            },
        ),
        (
            "ONBUILD ENV APP_VERSION=1.0",
            DockerCommand::Env {
                key: "APP_VERSION".to_string(),
                value: "1.0".to_string(),
            },
        ),
        (
            "ONBUILD WORKDIR /app",
            DockerCommand::Workdir {
                path: "/app".to_string(),
            },
        ),
        (
            "ONBUILD EXPOSE 8080",
            DockerCommand::Expose {
                port: 8080,
                protocol: None,
            },
        ),
        (
            "ONBUILD USER app:app",
            DockerCommand::User {
                user: "app".to_string(),
                group: Some("app".to_string()),
            },
        ),
        (
            "ONBUILD VOLUME /data",
            DockerCommand::Volume {
                paths: vec!["/data".to_string()],
            },
        ),
        (
            "ONBUILD LABEL version=1.0",
            DockerCommand::Label {
                labels: {
                    let mut map = HashMap::new();
                    map.insert("version".to_string(), "1.0".to_string());
                    map
                },
            },
        ),
    ];

    for (content, expected_cmd) in test_cases {
        let config = DockerfileConfig::parse(content).unwrap();
        match &config.commands[0] {
            DockerCommand::Onbuild { command } => {
                assert_eq!(command.as_ref(), &expected_cmd);
            }
            _ => panic!("Expected ONBUILD command"),
        }
    }
}

#[tokio::test]
async fn test_volume_formats() {
    // Test space-separated format
    let content = "VOLUME /data /config /cache";
    let config = DockerfileConfig::parse(content).unwrap();
    match &config.commands[0] {
        DockerCommand::Volume { paths } => {
            assert_eq!(paths, &vec!["/data", "/config", "/cache"]);
        }
        _ => panic!("Expected VOLUME command"),
    }

    // Test JSON array format
    let content = r#"VOLUME ["/data", "/config"]"#;
    let config = DockerfileConfig::parse(content).unwrap();
    match &config.commands[0] {
        DockerCommand::Volume { paths } => {
            assert_eq!(paths, &vec!["/data", "/config"]);
        }
        _ => panic!("Expected VOLUME command"),
    }

    // Test error case
    let content = "VOLUME [invalid json";
    let result = DockerfileConfig::parse(content);
    assert!(matches!(
        result,
        Err(DockerError::DockerfileError(msg)) if msg.contains("Invalid JSON array")
    ));
}
