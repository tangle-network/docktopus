use bollard::service::{Mount, MountTypeEnum};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum VolumeType {
    Named(String),
    Bind {
        source: String,
        target: String,
        read_only: bool,
    },
    Config {
        name: String,
        driver: Option<String>,
        driver_opts: Option<HashMap<String, String>>,
    },
}

// For top-level volume definitions
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum VolumeSpec {
    Empty(Option<()>),
    Full {
        driver: Option<String>,
        driver_opts: Option<HashMap<String, String>>,
    },
}

impl<'de> Deserialize<'de> for VolumeType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum VolumeInput {
            String(String),
            Long {
                source: String,
                target: String,
                #[serde(rename = "type")]
                typ: Option<String>,
                #[serde(default)]
                read_only: bool,
            },
            TopLevel(VolumeSpec),
        }

        let input = VolumeInput::deserialize(deserializer)?;
        match input {
            VolumeInput::String(s) => {
                let parts: Vec<&str> = s.split(':').collect();
                match parts.len() {
                    2 => {
                        if parts[0].starts_with('/') || parts[0].starts_with("./") {
                            Ok(VolumeType::Bind {
                                source: parts[0].to_string(),
                                target: parts[1].to_string(),
                                read_only: false,
                            })
                        } else {
                            Ok(VolumeType::Named(s.to_string()))
                        }
                    }
                    3 if parts[2] == "ro" => {
                        if parts[0].starts_with('/') || parts[0].starts_with("./") {
                            Ok(VolumeType::Bind {
                                source: parts[0].to_string(),
                                target: parts[1].to_string(),
                                read_only: true,
                            })
                        } else {
                            Ok(VolumeType::Named(s.to_string()))
                        }
                    }
                    _ => Ok(VolumeType::Named(s.to_string())),
                }
            }
            VolumeInput::Long {
                source,
                target,
                typ,
                read_only,
            } => match typ.as_deref() {
                Some("bind") => Ok(VolumeType::Bind {
                    source,
                    target,
                    read_only,
                }),
                Some("volume") | None => {
                    let name = if read_only {
                        format!("{}:{}:ro", source, target)
                    } else {
                        format!("{}:{}", source, target)
                    };
                    Ok(VolumeType::Named(name))
                }
                Some(t) => Err(serde::de::Error::custom(format!(
                    "Invalid volume type: {}",
                    t
                ))),
            },
            VolumeInput::TopLevel(spec) => match spec {
                VolumeSpec::Empty(_) => Ok(VolumeType::Config {
                    name: String::new(),
                    driver: None,
                    driver_opts: None,
                }),
                VolumeSpec::Full {
                    driver,
                    driver_opts,
                } => Ok(VolumeType::Config {
                    name: String::new(),
                    driver,
                    driver_opts,
                }),
            },
        }
    }
}

impl Serialize for VolumeType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            VolumeType::Named(name) => serializer.serialize_str(name),
            VolumeType::Bind {
                source,
                target,
                read_only,
            } => {
                if *read_only {
                    serializer.serialize_str(&format!("{}:{}:ro", source, target))
                } else {
                    serializer.serialize_str(&format!("{}:{}", source, target))
                }
            }
            VolumeType::Config {
                driver,
                driver_opts,
                ..
            } => match (driver, driver_opts) {
                (None, None) => serializer.serialize_none(),
                (Some(driver), Some(opts)) => {
                    use serde::ser::SerializeMap;
                    let mut map = serializer.serialize_map(Some(2))?;
                    map.serialize_entry("driver", driver)?;
                    map.serialize_entry("driver_opts", opts)?;
                    map.end()
                }
                (Some(driver), None) => {
                    use serde::ser::SerializeMap;
                    let mut map = serializer.serialize_map(Some(1))?;
                    map.serialize_entry("driver", driver)?;
                    map.end()
                }
                (None, Some(opts)) => {
                    use serde::ser::SerializeMap;
                    let mut map = serializer.serialize_map(Some(1))?;
                    map.serialize_entry("driver_opts", opts)?;
                    map.end()
                }
            },
        }
    }
}

impl VolumeType {
    pub fn to_mount(&self) -> Mount {
        match self {
            VolumeType::Named(name) => {
                let parts: Vec<&str> = name.split(':').collect();
                Mount {
                    target: Some(parts[1].to_string()),
                    source: Some(parts[0].to_string()),
                    typ: Some(MountTypeEnum::VOLUME),
                    ..Default::default()
                }
            }
            VolumeType::Bind {
                source,
                target,
                read_only,
            } => Mount {
                target: Some(target.to_string()),
                source: Some(source.to_string()),
                typ: Some(MountTypeEnum::BIND),
                read_only: Some(*read_only),
                ..Default::default()
            },
            VolumeType::Config { name, .. } => Mount {
                source: Some(name.clone()),
                typ: Some(MountTypeEnum::VOLUME),
                ..Default::default()
            },
        }
    }

    pub fn matches_name(&self, name: &str) -> bool {
        match self {
            VolumeType::Named(volume_name) => volume_name.split(':').next().unwrap_or("") == name,
            VolumeType::Bind { target, .. } => target == name,
            VolumeType::Config {
                name: volume_name, ..
            } => volume_name == name,
        }
    }
}
