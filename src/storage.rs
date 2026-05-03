use crate::error::{Result, SshManagerError};
use crate::models::{Config, TerminalConfig};
use std::fs;
use std::path::PathBuf;

pub struct Storage;

impl Storage {
    pub fn get_config_dir() -> Result<PathBuf> {
        dirs::config_dir()
            .map(|p| p.join("ssh_manager"))
            .ok_or_else(|| {
                SshManagerError::ConfigDirError(
                    "Could not determine config directory. $HOME not set?".to_string(),
                )
            })
    }

    pub fn ensure_config_dir() -> Result<PathBuf> {
        let config_dir = Self::get_config_dir()?;
        if !config_dir.exists() {
            fs::create_dir_all(&config_dir).map_err(|e| {
                SshManagerError::ConfigDirError(format!(
                    "Failed to create config directory {}: {}",
                    config_dir.display(),
                    e
                ))
            })?;
        }
        Ok(config_dir)
    }

    pub fn get_servers_file() -> Result<PathBuf> {
        let config_dir = Self::ensure_config_dir()?;
        Ok(config_dir.join("servers.json"))
    }

    pub fn get_terminal_config_file() -> Result<PathBuf> {
        let config_dir = Self::ensure_config_dir()?;
        Ok(config_dir.join("terminal_config.json"))
    }

    pub fn load_config() -> Result<Config> {
        let servers_file = Self::get_servers_file()?;

        if !servers_file.exists() {
            // Return empty config if file doesn't exist yet
            return Ok(Config::new());
        }

        let content = fs::read_to_string(&servers_file)?;
        let servers: Vec<crate::models::Server> = serde_json::from_str(&content).map_err(|e| {
            SshManagerError::InvalidConfig(format!(
                "Invalid servers.json format: {}",
                e
            ))
        })?;

        let mut config = Config {
            servers,
            preferred_terminal: None,
        };

        // Load terminal preference if it exists
        if let Ok(terminal_file) = Self::get_terminal_config_file() {
            if terminal_file.exists() {
                if let Ok(terminal_content) = fs::read_to_string(&terminal_file) {
                    if let Ok(terminal_config) = serde_json::from_str::<TerminalConfig>(&terminal_content) {
                        config.preferred_terminal = Some(terminal_config.terminal);
                    }
                }
            }
        }

        Ok(config)
    }

    pub fn save_config(config: &Config) -> Result<()> {
        let servers_file = Self::get_servers_file()?;
        let json = serde_json::to_string_pretty(&config.servers)?;
        fs::write(&servers_file, json)?;

        // Save terminal preference if set
        if let Some(terminal) = &config.preferred_terminal {
            let terminal_file = Self::get_terminal_config_file()?;
            let terminal_config = TerminalConfig {
                terminal: terminal.clone(),
            };
            let json = serde_json::to_string_pretty(&terminal_config)?;
            fs::write(&terminal_file, json)?;
        }

        Ok(())
    }

    pub fn server_name_is_unique(config: &Config, name: &str, excluding: Option<&str>) -> bool {
        config.servers.iter().all(|s| {
            if let Some(excluding_name) = excluding {
                s.name != name || s.name == excluding_name
            } else {
                s.name != name
            }
        })
    }

    pub fn validate_key_path(key_path: &str) -> Result<()> {
        let path = PathBuf::from(key_path);
        if !path.exists() {
            return Err(SshManagerError::KeyFileNotFound(key_path.to_string()));
        }
        if !path.is_file() {
            return Err(SshManagerError::KeyFileNotFound(format!(
                "{} is not a file",
                key_path
            )));
        }
        Ok(())
    }

    pub fn validate_server_name(name: &str) -> Result<()> {
        if name.is_empty() {
            return Err(SshManagerError::EmptyServerName);
        }
        if name.contains(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
            return Err(SshManagerError::InvalidServerName);
        }
        Ok(())
    }

    pub fn validate_username(username: &str) -> Result<()> {
        if username.is_empty() {
            return Err(SshManagerError::EmptyUsername);
        }
        Ok(())
    }

    pub fn validate_host(host: &str) -> Result<()> {
        if host.is_empty() {
            return Err(SshManagerError::EmptyHost);
        }
        Ok(())
    }

    pub fn validate_port(port: u16) -> Result<()> {
        if port == 0 {
            return Err(SshManagerError::InvalidPort);
        }
        Ok(())
    }
}
