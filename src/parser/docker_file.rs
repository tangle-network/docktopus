#[cfg(test)]
mod tests;

use crate::config::docker_file::{DockerCommand, DockerfileConfig};
use crate::error::DockerError;
use std::collections::HashMap;

/// Parse a Dockerfile
///
/// # Errors
///
/// This will error if at any point the file is malformed.
pub fn parse(content: &str) -> Result<DockerfileConfig, DockerError> {
    let mut config = DockerfileConfig {
        base_image: String::new(),
        commands: Vec::new(),
    };

    let mut current_command = String::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Handle line continuations
        if let Some(s) = line.strip_suffix('\\') {
            current_command.push_str(s);
            current_command.push(' ');
            continue;
        }

        if !current_command.is_empty() {
            current_command.push_str(line);
            parse_command(&mut config, &current_command)?;
            current_command.clear();
            continue;
        }

        parse_command(&mut config, line)?;
    }

    Ok(config)
}

fn parse_command(config: &mut DockerfileConfig, line: &str) -> Result<(), DockerError> {
    let parts: Vec<&str> = line.splitn(2, ' ').collect();
    if parts.len() != 2 {
        return Err(DockerError::DockerfileError(
            "Invalid command syntax".to_string(),
        ));
    }

    let (command, args) = (parts[0].to_uppercase(), parts[1].trim());
    match command.as_str() {
        "FROM" => config.base_image = args.to_string(),
        "COPY" => {
            let parts: Vec<&str> = args.split_whitespace().collect();
            if parts.len() < 2 {
                return Err(DockerError::DockerfileError(
                    "COPY requires source and destination".to_string(),
                ));
            }
            let (chown, parts) = parse_chown_option(args);
            let parts: Vec<&str> = parts.split_whitespace().collect();
            config.commands.push(DockerCommand::Copy {
                source: parts[0].to_string(),
                dest: parts[1].to_string(),
                chown,
            });
        }
        "EXPOSE" => {
            // Split on whitespace first to handle multiple ports
            let ports = args.split_whitespace();
            for port_spec in ports {
                let parts: Vec<&str> = port_spec.split('/').collect();
                let port = parts[0].trim().parse::<u16>().map_err(|_| {
                    DockerError::DockerfileError(format!("Invalid port number: {}", parts[0]))
                })?;

                config.commands.push(DockerCommand::Expose {
                    port,
                    protocol: parts.get(1).map(|p| p.trim().to_string()),
                });
            }
        }
        "ONBUILD" => {
            let args = args.trim();
            if args.is_empty() {
                return Err(DockerError::DockerfileError(
                    "ONBUILD requires one argument".to_string(),
                ));
            }

            // Create a new config to parse the ONBUILD command
            let mut onbuild_config = DockerfileConfig {
                base_image: String::new(),
                commands: Vec::new(),
            };

            // Parse the ONBUILD command recursively
            parse_command(&mut onbuild_config, args)?;

            // Get the parsed command
            if let Some(cmd) = onbuild_config.commands.pop() {
                config.commands.push(DockerCommand::Onbuild {
                    command: Box::new(cmd),
                });
            } else {
                return Err(DockerError::DockerfileError(
                    "Invalid ONBUILD command".to_string(),
                ));
            }
        }
        "ADD" => {
            let (chown, parts) = parse_chown_option(args);
            let sources_and_dest = if args.trim().starts_with('[') {
                // Handle JSON array format
                serde_json::from_str::<Vec<String>>(args).map_err(|e| {
                    DockerError::DockerfileError(format!("Invalid JSON array: {}", e))
                })?
            } else {
                // Handle space-separated format
                shell_words::split(&parts)
                    .map_err(|e| DockerError::DockerfileError(e.to_string()))?
            };

            if sources_and_dest.len() >= 2 {
                let dest = sources_and_dest.last().unwrap().clone();
                let sources = sources_and_dest[..sources_and_dest.len() - 1].to_vec();
                config.commands.push(DockerCommand::Add {
                    sources,
                    dest,
                    chown,
                });
            } else {
                return Err(DockerError::DockerfileError(
                    "ADD requires at least one source and destination".to_string(),
                ));
            }
        }
        "ARG" => {
            let parts: Vec<&str> = args.split('=').collect();
            config.commands.push(DockerCommand::Arg {
                name: parts[0].trim().to_string(),
                default_value: parts.get(1).map(|v| v.trim().to_string()),
            });
        }
        "CMD" => {
            let command = if args.trim().starts_with('[') {
                // Handle JSON array format
                serde_json::from_str::<Vec<String>>(args).map_err(|e| {
                    DockerError::DockerfileError(format!("Invalid JSON array: {}", e))
                })?
            } else {
                // Handle space-separated format
                shell_words::split(args).map_err(|e| DockerError::DockerfileError(e.to_string()))?
            };
            config.commands.push(DockerCommand::Cmd { command });
        }
        "ENTRYPOINT" => {
            let command = if args.trim().starts_with('[') {
                // Handle JSON array format
                serde_json::from_str::<Vec<String>>(args).map_err(|e| {
                    DockerError::DockerfileError(format!("Invalid JSON array: {}", e))
                })?
            } else {
                // Handle space-separated format
                shell_words::split(args).map_err(|e| DockerError::DockerfileError(e.to_string()))?
            };
            config.commands.push(DockerCommand::Entrypoint { command });
        }
        "ENV" => {
            let parts: Vec<&str> = args.splitn(2, '=').collect();
            if parts.len() == 2 {
                config.commands.push(DockerCommand::Env {
                    key: parts[0].trim().to_string(),
                    value: parts[1].trim().to_string(),
                });
            }
        }
        "HEALTHCHECK" => {
            if args.trim() == "NONE" {
                return Ok(());
            }

            let mut parts = args.split_whitespace();
            let mut interval = None;
            let mut timeout = None;
            let mut start_period = None;
            let mut retries = None;
            let mut command = Vec::new();

            // Find CMD part
            let mut found_cmd = false;
            let mut cmd_parts = Vec::new();
            while let Some(part) = parts.next() {
                if part == "CMD" {
                    found_cmd = true;
                    cmd_parts = parts.collect();
                    break;
                }

                // Handle --flag=value format
                if part.starts_with("--") {
                    let flag_part: Vec<&str> = part.splitn(2, '=').collect();
                    match flag_part.first() {
                        Some(val @ (&"--interval" | &"--timeout" | &"--start-period")) => {
                            let value = if flag_part.len() == 2 {
                                Some(flag_part[1].to_string())
                            } else {
                                parts.next().map(ToString::to_string)
                            };

                            if value.is_none() {
                                return Err(DockerError::DockerfileError(format!(
                                    "Missing value for {} flag",
                                    flag_part[0]
                                )));
                            }

                            match *val {
                                "--interval" => interval = value,
                                "--timeout" => timeout = value,
                                "--start-period" => start_period = value,
                                _ => unreachable!(),
                            }
                        }
                        Some(&"--retries") => {
                            let value = if flag_part.len() == 2 {
                                flag_part[1].parse().ok()
                            } else {
                                parts.next().and_then(|s| s.parse().ok())
                            };

                            if value.is_none() {
                                return Err(DockerError::DockerfileError(
                                    "Invalid value for --retries flag".to_string(),
                                ));
                            }
                            retries = value;
                        }
                        _ => {
                            return Err(DockerError::DockerfileError(format!(
                                "Invalid HEALTHCHECK flag: {}",
                                flag_part[0]
                            )));
                        }
                    }
                }
            }

            if !found_cmd {
                return Err(DockerError::DockerfileError(
                    "HEALTHCHECK must include CMD".to_string(),
                ));
            }

            if let Ok(cmd) = shell_words::split(&cmd_parts.join(" ")) {
                command = cmd;
            }

            if !command.is_empty() {
                config.commands.push(DockerCommand::Healthcheck {
                    command,
                    interval,
                    timeout,
                    start_period,
                    retries,
                });
            }
        }
        "LABEL" => {
            let mut labels = HashMap::new();
            let mut current_key = String::new();
            let mut current_value = String::new();
            let mut in_quotes = false;

            for c in args.chars() {
                match c {
                    '"' => in_quotes = !in_quotes,
                    '=' if !in_quotes && current_key.is_empty() => {
                        current_key = current_value.trim().to_string();
                        current_value.clear();
                    }
                    ' ' if !in_quotes && !current_key.is_empty() => {
                        if !current_value.is_empty() {
                            labels.insert(
                                current_key.trim_matches('"').to_string(),
                                current_value.trim_matches('"').to_string(),
                            );
                            current_key.clear();
                            current_value.clear();
                        }
                    }
                    _ => {
                        current_value.push(c);
                    }
                }
            }

            // Handle the last key-value pair
            if !current_key.is_empty() && !current_value.is_empty() {
                labels.insert(
                    current_key.trim_matches('"').to_string(),
                    current_value.trim_matches('"').to_string(),
                );
            }

            config.commands.push(DockerCommand::Label { labels });
        }
        "MAINTAINER" => {
            config.commands.push(DockerCommand::Maintainer {
                name: args.to_string(),
            });
        }
        "RUN" => {
            config.commands.push(DockerCommand::Run {
                command: args.to_string(),
            });
        }
        "SHELL" => {
            let shell = if args.trim().starts_with('[') {
                // Handle JSON array format
                serde_json::from_str::<Vec<String>>(args).map_err(|e| {
                    DockerError::DockerfileError(format!("Invalid JSON array: {}", e))
                })?
            } else {
                // Handle space-separated format
                shell_words::split(args).map_err(|e| DockerError::DockerfileError(e.to_string()))?
            };
            config.commands.push(DockerCommand::Shell { shell });
        }
        "STOPSIGNAL" => {
            config.commands.push(DockerCommand::StopSignal {
                signal: args.to_string(),
            });
        }
        "USER" => {
            let parts: Vec<&str> = args.split(':').collect();
            config.commands.push(DockerCommand::User {
                user: parts[0].to_string(),
                group: parts.get(1).map(ToString::to_string),
            });
        }
        "VOLUME" => {
            let paths = if args.trim().starts_with('[') {
                // Handle JSON array format
                serde_json::from_str::<Vec<String>>(args).map_err(|e| {
                    DockerError::DockerfileError(format!("Invalid JSON array: {}", e))
                })?
            } else {
                // Handle space-separated format
                shell_words::split(args).map_err(|e| DockerError::DockerfileError(e.to_string()))?
            };
            config.commands.push(DockerCommand::Volume { paths });
        }
        "WORKDIR" => {
            config.commands.push(DockerCommand::Workdir {
                path: args.to_string(),
            });
        }
        _ => {
            return Err(DockerError::DockerfileError(format!(
                "Unknown command: {}",
                command
            )));
        }
    }
    Ok(())
}

fn parse_chown_option(args: &str) -> (Option<String>, String) {
    if args.starts_with("--chown=") {
        let parts: Vec<&str> = args.splitn(2, ' ').collect();
        let chown = parts[0].trim_start_matches("--chown=").to_string();
        (Some(chown), (*parts.get(1).unwrap_or(&"")).to_string())
    } else {
        (None, args.to_string())
    }
}
