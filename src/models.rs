use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AuthMethod {
    #[serde(rename = "key")]
    Key,
    #[serde(rename = "password")]
    Password,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Server {
    pub name: String,
    pub host: String,
    pub username: String,
    pub port: u16,
    pub auth_method: AuthMethod,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
    #[serde(default)]
    pub use_agent_forwarding: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub servers: Vec<Server>,
    #[serde(default)]
    pub preferred_terminal: Option<String>,
}

impl Config {
    pub fn new() -> Self {
        Config {
            servers: Vec::new(),
            preferred_terminal: None,
        }
    }

    pub fn find_server(&self, name: &str) -> Option<&Server> {
        self.servers.iter().find(|s| s.name == name)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    pub fn new(
        name: String,
        host: String,
        username: String,
        port: u16,
        auth_method: AuthMethod,
        key_path: Option<String>,
    ) -> Self {
        Server {
            name,
            host,
            username,
            port,
            auth_method,
            key_path,
            use_agent_forwarding: false,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TerminalConfig {
    pub terminal: String,
}
