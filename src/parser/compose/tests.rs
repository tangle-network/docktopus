#![allow(clippy::literal_string_with_formatting_args)]

use crate::parser::ComposeParser;
use crate::parser::env;
use crate::test_fixtures::{get_local_reth_compose, get_reth_archive_compose};
use crate::{ComposeConfig, Service, Volume};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tempfile::NamedTempFile;

#[test]
fn test_compose_parsing() {
    let yaml = r#"
        version: "3.8"
        services:
          reth:
            image: ghcr.io/paradigmxyz/reth:latest
            ports:
              - "8545:8545"
              - "9000:9000"
            command: [
              "/reth/target/release/reth",
              "node",
              "--metrics",
              "reth:9000",
              "--debug.tip",
              "${RETH_TIP:-0x7d5a4369273c723454ac137f48a4f142b097aa2779464e6505f1b1c5e37b5382}",
              "--log.directory",
              "$HOME"
            ]
            volumes:
              - source: ./data
                target: /data
                type: bind
                read_only: false
    "#;

    let config: ComposeConfig = serde_yaml::from_str(yaml).unwrap();
    let service = config.services.get("reth").unwrap();

    assert!(service.command.is_some());
    let command = service.command.as_ref().unwrap();
    assert_eq!(command.len(), 8);
    assert_eq!(command[0], "/reth/target/release/reth");
    assert_eq!(command[1], "node");

    let volumes = service.volumes.as_ref().unwrap();
    assert_eq!(volumes.len(), 1);

    match &volumes[0] {
        Volume::Bind {
            source,
            target,
            read_only,
        } => {
            assert_eq!(source, "./data");
            assert_eq!(target, "/data");
            assert!(!read_only);
        }
        _ => panic!("Expected bind mount"),
    }
}

#[test]
fn test_reth_archive_compose_parsing() {
    let content = std::fs::read(get_reth_archive_compose()).unwrap();
    let config = ComposeParser::new().parse(&mut content.as_slice()).unwrap();

    assert_eq!(config.version, "2");
    assert_eq!(config.services.len(), 2);

    // Test reth service
    let reth = config.services.get("reth").unwrap();
    assert_eq!(
        reth.image,
        Some("ghcr.io/paradigmxyz/reth:latest".to_string())
    );
    assert_eq!(
        reth.ports,
        Some(vec![
            "8543:8543".to_string(),
            "8544:8544".to_string(),
            "30304:30304/tcp".to_string(),
            "30304:30304/udp".to_string(),
        ])
    );

    // Test nimbus service
    let nimbus = config.services.get("nimbus").unwrap();
    assert_eq!(
        nimbus.image,
        Some("statusim/nimbus-eth2:multiarch-latest".to_string())
    );
    assert_eq!(
        nimbus.ports,
        Some(vec![
            "9001:9001/tcp".to_string(),
            "9001:9001/udp".to_string(),
        ])
    );

    // Test volumes are defined
    assert!(config.volumes.contains_key("reth_data"));
    assert!(config.volumes.contains_key("reth_jwt"));
    assert!(config.volumes.contains_key("nimbus_data"));

    // Test reth service volumes
    let reth_volumes = reth.volumes.as_ref().unwrap();
    assert_eq!(reth_volumes.len(), 2);
    assert!(matches!(&reth_volumes[0], Volume::Named(name) if name == "reth_data:/data"));
    assert!(matches!(&reth_volumes[1], Volume::Named(name) if name == "reth_jwt:/jwt:ro"));

    // Test nimbus service volumes
    let nimbus_volumes = nimbus.volumes.as_ref().unwrap();
    assert_eq!(nimbus_volumes.len(), 2);
    assert!(matches!(&nimbus_volumes[0], Volume::Named(name) if name == "nimbus_data:/data"));
    assert!(matches!(&nimbus_volumes[1], Volume::Named(name) if name == "reth_jwt:/jwt/reth:ro"));
}

#[test]
fn test_local_reth_compose_parsing() {
    let content = std::fs::read(get_local_reth_compose()).unwrap();
    let config = ComposeParser::new().parse(&mut content.as_slice()).unwrap();

    assert_eq!(config.version, "3.9");
    assert_eq!(config.services.len(), 3);

    // Test reth service
    let reth = config.services.get("reth").unwrap();
    assert_eq!(reth.image, None); // Local build, no image specified
    assert_eq!(reth.ports, Some(vec!["9000:9000".to_string()]));
    assert!(reth.build.is_some());
    let build = reth.build.as_ref().unwrap();
    assert_eq!(build.context, "./reth");
    assert_eq!(build.dockerfile, Some("Dockerfile".to_string()));
    assert!(reth.command.is_some());
    assert!(reth.volumes.is_some());
    assert_eq!(reth.restart, Some("always".to_string()));

    // Test prometheus service
    let prometheus = config.services.get("prometheus").unwrap();
    assert_eq!(prometheus.image, Some("prom/prometheus".to_string()));
    assert_eq!(prometheus.ports, Some(vec!["9090:9090".to_string()]));
    assert!(prometheus.volumes.is_some());
    assert_eq!(prometheus.restart, Some("always".to_string()));
    assert_eq!(prometheus.user, Some("root".to_string()));
    assert!(prometheus.depends_on.is_some());
    assert!(prometheus.command.is_some());

    // Test grafana service
    let grafana = config.services.get("grafana").unwrap();
    assert_eq!(grafana.image, Some("grafana/grafana".to_string()));
    assert_eq!(grafana.ports, Some(vec!["3000:3000".to_string()]));
    assert!(grafana.volumes.is_some());
    assert_eq!(grafana.restart, Some("always".to_string()));
    assert_eq!(grafana.user, Some("472".to_string()));
    assert!(grafana.depends_on.is_some());

    // Test volumes are defined
    assert!(config.volumes.contains_key("rethdata"));
    assert!(config.volumes.contains_key("rethlogs"));
    assert!(config.volumes.contains_key("prometheusdata"));
    assert!(config.volumes.contains_key("grafanadata"));
}

#[test]
fn test_volume_validation() {
    let mut config = ComposeConfig {
        version: "3.8".to_string(),
        services: HashMap::new(),
        volumes: HashMap::new(),
    };

    let service = Service {
        volumes: Some(vec![
            Volume::Named("test-data:/data".to_string()),
            Volume::Bind {
                source: PathBuf::from("/host").to_string_lossy().to_string(),
                target: "/container".to_string(),
                read_only: false,
            },
        ]),
        ..Default::default()
    };

    config.services.insert("test".to_string(), service);

    // Test validation of named volumes
    assert!(config.validate_required_volumes(&["test-data"]).is_ok());

    // Test validation of bind mounts by target path
    assert!(config.validate_required_volumes(&["/container"]).is_ok());

    // Test validation of missing volumes
    assert!(config.validate_required_volumes(&["missing"]).is_err());

    // Test validation of missing bind mount targets
    assert!(config.validate_required_volumes(&["/missing"]).is_err());
}

// Sync tests that don't need Docker cleanup
#[test]
fn test_volume_serialization() {
    let volume = Volume::Bind {
        source: PathBuf::from("/host").to_string_lossy().to_string(),
        target: "/container".to_string(),
        read_only: true,
    };

    let service = Service {
        volumes: Some(vec![volume]),
        ..Default::default()
    };

    let serialized = serde_yaml::to_string(&service).unwrap();
    let deserialized: Service = serde_yaml::from_str(&serialized).unwrap();

    assert_eq!(service.volumes, deserialized.volumes);
}

#[test]
fn test_service_deployment() {
    let mut config = ComposeConfig::default();
    let service = Service {
        image: Some("nginx:latest".to_string()),
        volumes: Some(vec![crate::Volume::Bind {
            source: PathBuf::from("/host/data").to_string_lossy().to_string(),
            target: "/container/data".to_string(),
            read_only: false,
        }]),
        ..Default::default()
    };

    config.services.insert("web".to_string(), service);
    assert!(config.services.contains_key("web"));
}

#[test]
fn test_full_compose_parsing() {
    let compose_content = r#"
        version: "3.8"
        services:
          web:
            image: nginx:${NGINX_VERSION:-latest}
            ports:
              - "${PORT:-80}:80"
        "#;

    let env_content = "NGINX_VERSION=1.21\nPORT=8080";
    let temp_file = NamedTempFile::new().unwrap();
    fs::write(&temp_file, env_content).unwrap();

    let config = ComposeParser::new()
        .env_file(temp_file.path())
        .parse(&mut compose_content.as_bytes())
        .unwrap();

    if let Some(web_service) = config.services.get("web") {
        assert_eq!(web_service.image.as_deref().unwrap(), "nginx:1.21");
        assert_eq!(
            web_service.ports.as_ref().unwrap().first().unwrap(),
            "8080:80"
        );
    } else {
        panic!("Web service not found in parsed config");
    }
}

#[test]
fn test_environment_variable_formats() {
    // Test both map and list formats
    let content = r#"version: "3"
services:
    app1:
        environment:
            KEY1: value1
            KEY2: value2
    app2:
        environment:
            - KEY3=value3
            - KEY4=value4"#;

    let config = ComposeParser::new().parse(&mut content.as_bytes()).unwrap();

    // Check map format
    let app1 = config.services.get("app1").unwrap();
    if let Some(env) = &app1.environment {
        assert_eq!(env.get("KEY1").map(String::as_str), Some("value1"));
        assert_eq!(env.get("KEY2").map(String::as_str), Some("value2"));
    } else {
        panic!("app1 environment should be Some");
    }

    // Check list format
    let app2 = config.services.get("app2").unwrap();
    if let Some(env) = &app2.environment {
        assert_eq!(env.get("KEY3").map(String::as_str), Some("value3"));
        assert_eq!(env.get("KEY4").map(String::as_str), Some("value4"));
    } else {
        panic!("app2 environment should be Some");
    }
}

#[test]
fn test_environment_variable_edge_cases() {
    let content = r#"version: "3"
services:
    app1:
        environment:
            EMPTY: ""
            QUOTED: "quoted value"
            SPACES: "  value with spaces  "
    app2:
        environment:
            - EMPTY=
            - QUOTED="quoted value"
            - SPACES="  value with spaces  ""#;

    let config = ComposeParser::new().parse(&mut content.as_bytes()).unwrap();

    // Test both formats handle edge cases the same way
    for service_name in ["app1", "app2"] {
        let service = config.services.get(service_name).unwrap();
        if let Some(env) = &service.environment {
            assert_eq!(env.get("EMPTY").map(String::as_str), Some(""));
            assert_eq!(env.get("QUOTED").map(String::as_str), Some("quoted value"));
            assert_eq!(
                env.get("SPACES").map(String::as_str),
                Some("  value with spaces  ")
            );
        } else {
            panic!("{} environment should be Some", service_name);
        }
    }
}

#[test]
fn test_environment_variable_substitution() {
    let content = r#"version: "3"
services:
    app1:
        image: nginx:${VERSION:-latest}
        environment:
            PORT: "${PORT:-8080}"
            DEBUG: "${DEBUG:-false}""#;

    let mut env_vars = HashMap::new();
    env_vars.insert("VERSION".to_string(), "1.21".to_string());
    env_vars.insert("DEBUG".to_string(), "true".to_string());

    let processed = env::substitute_env_vars(content, &env_vars);
    let mut config = ComposeParser::new()
        .parse(&mut processed.as_bytes())
        .unwrap();
    config.resolve_env(&env_vars);

    let app1 = config.services.get("app1").unwrap();
    assert_eq!(app1.image.as_deref(), Some("nginx:1.21"));
    if let Some(env) = &app1.environment {
        assert_eq!(env.get("PORT").map(String::as_str), Some("8080")); // Uses default
        assert_eq!(env.get("DEBUG").map(String::as_str), Some("true")); // Uses env var
    } else {
        panic!("app1 environment should be Some");
    }
}

#[test]
fn test_compose_missing_version() {
    let yaml = r#"
        services:
          reth:
            image: ghcr.io/paradigmxyz/reth:latest
            ports:
              - "8545:8545"
              - "9000:9000"
            command: [
              "/reth/target/release/reth",
              "node",
              "--metrics",
              "reth:9000",
              "--debug.tip",
              "${RETH_TIP:-0x7d5a4369273c723454ac137f48a4f142b097aa2779464e6505f1b1c5e37b5382}",
              "--log.directory",
              "$HOME"
            ]
            volumes:
              - source: ./data
                target: /data
                type: bind
                read_only: false
    "#;

    let config: ComposeConfig = serde_yaml::from_str(yaml).unwrap();
    let service = config.services.get("reth").unwrap();

    assert!(service.command.is_some());
    let command = service.command.as_ref().unwrap();
    assert_eq!(command.len(), 8);
    assert_eq!(command[0], "/reth/target/release/reth");
    assert_eq!(command[1], "node");

    let volumes = service.volumes.as_ref().unwrap();
    assert_eq!(volumes.len(), 1);

    match &volumes[0] {
        Volume::Bind {
            source,
            target,
            read_only,
        } => {
            assert_eq!(source, "./data");
            assert_eq!(target, "/data");
            assert!(!read_only);
        }
        _ => panic!("Expected bind mount"),
    }
}
