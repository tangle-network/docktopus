use super::EnvironmentVars;
use super::volume::Volume;
use crate::config::health::HealthCheck;
use crate::config::requirements::SystemRequirements;
use crate::error::DockerError;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Configuration for a single service in a Docker Compose file
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    pub image: Option<String>,
    pub build: Option<BuildConfig>,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_command")]
    pub command: Option<Vec<String>>,
    pub environment: Option<EnvironmentVars>,
    pub env_file: Option<Vec<String>>,
    pub volumes: Option<Vec<Volume>>,
    pub depends_on: Option<Vec<String>>,
    pub ports: Option<Vec<String>>,
    pub networks: Option<Vec<String>>,
    pub requirements: Option<SystemRequirements>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub healthcheck: Option<HealthCheck>,
    pub restart: Option<String>,
    pub user: Option<String>,
    #[serde(default)]
    pub labels: Option<HashMap<String, String>>,
    #[serde(default)]
    pub platform: Option<String>,
}

fn deserialize_command<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let value = serde_yaml::Value::deserialize(deserializer)?;

    match value {
        serde_yaml::Value::String(s) => Ok(Some(vec![s])),
        serde_yaml::Value::Sequence(seq) => {
            let items: Result<Vec<String>, _> = seq
                .into_iter()
                .map(|v| {
                    v.as_str()
                        .map(String::from)
                        .ok_or_else(|| Error::custom("Invalid command item"))
                })
                .collect();
            Ok(Some(items?))
        }
        serde_yaml::Value::Null => Ok(None),
        _ => Err(Error::custom("Invalid command format")),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildConfig {
    pub context: String,
    pub dockerfile: Option<String>,
}

/// Configuration for a Docker Compose deployment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeConfig {
    /// Docker Compose file version
    #[serde(default = "version_3")]
    pub version: String,
    /// Map of service name to service configuration
    pub services: HashMap<String, Service>,
    /// Map of volume name to volume configuration
    #[serde(default)]
    pub volumes: HashMap<String, Volume>,
}

fn version_3() -> String {
    "3".to_string()
}

impl Default for ComposeConfig {
    fn default() -> Self {
        ComposeConfig {
            version: "3".to_string(),
            services: HashMap::new(),
            volumes: HashMap::new(),
        }
    }
}

impl ComposeConfig {
    /// Validates that required environment variables are present
    ///
    /// # Errors
    ///
    /// Will return an error if any required environment variable is missing///
    pub fn validate_required_env_vars(&self, vars: &[&str]) -> Result<(), DockerError> {
        for (service_name, service) in &self.services {
            if let Some(env) = &service.environment {
                for var in vars {
                    if !env.contains_key(var) {
                        return Err(DockerError::ValidationError(format!(
                            "Service '{}' is missing required environment variable: {}",
                            service_name, var
                        )));
                    }
                }
            } else if !vars.is_empty() {
                return Err(DockerError::ValidationError(format!(
                    "Service '{}' has no environment variables configured",
                    service_name
                )));
            }
        }
        Ok(())
    }

    /// Validates that required volumes are present
    ///
    /// # Errors
    ///
    /// Will return an error if any required volume is missing
    pub fn validate_required_volumes(&self, required: &[&str]) -> Result<(), DockerError> {
        for (service_name, service) in &self.services {
            if let Some(volumes) = &service.volumes {
                for required_volume in required {
                    if !volumes.iter().any(|v| v.matches_name(required_volume)) {
                        return Err(DockerError::ValidationError(format!(
                            "Service '{}' is missing required volume: {}",
                            service_name, required_volume
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    /// Resolves the order in which services should be deployed based on dependencies
    ///
    /// # Errors
    ///
    /// Will return an error in the presence of circular dependencies
    pub fn resolve_service_order(&self) -> Result<Vec<String>, DockerError> {
        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let mut temp_visited = HashSet::new();

        // Build dependency graph
        let mut graph: HashMap<&str, Vec<&str>> = HashMap::new();
        for (service_name, service) in &self.services {
            let deps = service
                .depends_on
                .as_ref()
                .map(|deps| deps.iter().map(String::as_str).collect())
                .unwrap_or_default();
            graph.insert(service_name, deps);
        }

        // Helper function for topological sort
        fn visit(
            node: &str,
            graph: &HashMap<&str, Vec<&str>>,
            visited: &mut HashSet<String>,
            temp_visited: &mut HashSet<String>,
            result: &mut Vec<String>,
        ) -> Result<(), DockerError> {
            if temp_visited.contains(node) {
                return Err(DockerError::ValidationError(
                    "Circular dependency detected".to_string(),
                ));
            }
            if visited.contains(node) {
                return Ok(());
            }

            temp_visited.insert(node.to_string());

            if let Some(deps) = graph.get(node) {
                for &dep in deps {
                    visit(dep, graph, visited, temp_visited, result)?;
                }
            }

            temp_visited.remove(node);
            visited.insert(node.to_string());
            result.push(node.to_string());

            Ok(())
        }

        // Perform topological sort
        for service in self.services.keys() {
            if !visited.contains(service) {
                visit(
                    service,
                    &graph,
                    &mut visited,
                    &mut temp_visited,
                    &mut result,
                )?;
            }
        }

        Ok(result)
    }

    /// Collects all volumes used in services and adds them to the volumes section
    pub fn collect_volumes(&mut self) {
        let mut used_volumes = HashMap::new();

        // Collect all named volumes from services
        for service in self.services.values() {
            if let Some(volumes) = &service.volumes {
                for volume in volumes {
                    if let Volume::Named(name) = volume {
                        let volume_name = name.split(':').next().unwrap_or(name).to_string();
                        if !self.volumes.contains_key(&volume_name) {
                            used_volumes.insert(volume_name, volume.clone());
                        }
                    }
                }
            }
        }

        // Add missing volumes to the volumes section
        self.volumes.extend(used_volumes);
    }

    /// Resolves environment variables in the configuration
    pub fn resolve_env(&mut self, env_vars: &HashMap<String, String>) {
        // Helper function to resolve env vars in a volume
        fn resolve_volume(volume: &mut Volume, env_vars: &HashMap<String, String>) {
            match volume {
                Volume::Named(name) => {
                    *name = ComposeConfig::resolve_env_value(name, env_vars);
                }
                Volume::Bind { source, target, .. } => {
                    *source = ComposeConfig::resolve_env_value(source, env_vars);
                    *target = ComposeConfig::resolve_env_value(target, env_vars);
                }
                Volume::Config {
                    name,
                    driver,
                    driver_opts,
                } => {
                    *name = ComposeConfig::resolve_env_value(name, env_vars);
                    if let Some(d) = driver {
                        *d = ComposeConfig::resolve_env_value(d, env_vars);
                    }
                    if let Some(opts) = driver_opts {
                        for value in opts.values_mut() {
                            *value = ComposeConfig::resolve_env_value(value, env_vars);
                        }
                    }
                }
            }
        }

        // Resolve environment variables in services
        for service in self.services.values_mut() {
            // Resolve service environment
            if let Some(environment) = &mut service.environment {
                for value in environment.values_mut() {
                    *value = Self::resolve_env_value(value, env_vars);
                }
            }

            // Resolve service volumes
            if let Some(volumes) = &mut service.volumes {
                for volume in volumes.iter_mut() {
                    resolve_volume(volume, env_vars);
                }
            }
        }

        // Resolve environment variables in volume configurations
        for volume in self.volumes.values_mut() {
            resolve_volume(volume, env_vars);
        }
    }

    fn resolve_env_value(value: &str, env_vars: &HashMap<String, String>) -> String {
        let re = Regex::new(r"\$\{([^}]+)}|\$([a-zA-Z_][a-zA-Z0-9_]*)").unwrap();
        let mut result = value.to_string();

        for cap in re.captures_iter(value) {
            let full_match = cap.get(0).unwrap();
            let var_name = cap.get(1).or_else(|| cap.get(2)).map_or("", |m| m.as_str());

            let (name, default) = if let Some((n, d)) = var_name.split_once(":-") {
                (n, Some(d))
            } else {
                (var_name, None)
            };

            let replacement = env_vars
                .get(name)
                .cloned()
                .or_else(|| std::env::var(name).ok())
                .or_else(|| default.map(ToString::to_string))
                .unwrap_or_default();

            result = result.replace(full_match.as_str(), &replacement);
        }
        result
    }
}
