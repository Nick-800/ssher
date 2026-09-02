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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_method_serializes_lowercase() {
        let k = serde_json::to_string(&AuthMethod::Key).unwrap();
        let p = serde_json::to_string(&AuthMethod::Password).unwrap();
        assert_eq!(k, "\"key\"");
        assert_eq!(p, "\"password\"");
    }

    #[test]
    fn server_round_trips_through_json() {
        let mut s = Server::new(
            "alpha".to_string(),
            "example.com".to_string(),
            "alice".to_string(),
            22,
            AuthMethod::Key,
            Some("/home/alice/.ssh/id_ed25519".to_string()),
        );
        s.use_agent_forwarding = true;
        let json = serde_json::to_string(&s).unwrap();
        let back: Server = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "alpha");
        assert_eq!(back.auth_method, AuthMethod::Key);
        assert!(back.use_agent_forwarding);
    }

    #[test]
    fn server_key_path_omitted_when_none() {
        let s = Server::new(
            "alpha".to_string(),
            "example.com".to_string(),
            "alice".to_string(),
            22,
            AuthMethod::Password,
            None,
        );
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("key_path"), "json was: {json}");
    }

    #[test]
    fn server_agent_forwarding_defaults_false() {
        let s = Server::new(
            "alpha".to_string(),
            "example.com".to_string(),
            "alice".to_string(),
            22,
            AuthMethod::Key,
            None,
        );
        assert!(!s.use_agent_forwarding);
    }

    #[test]
    fn config_find_server_returns_match() {
        let mut cfg = Config::new();
        cfg.servers.push(Server::new(
            "alpha".to_string(),
            "example.com".to_string(),
            "alice".to_string(),
            22,
            AuthMethod::Key,
            None,
        ));
        assert!(cfg.find_server("alpha").is_some());
        assert!(cfg.find_server("missing").is_none());
    }

    #[test]
    fn config_round_trip_includes_preferred_terminal() {
        let mut cfg = Config::new();
        cfg.preferred_terminal = Some("xterm".to_string());
        let json = serde_json::to_string(&cfg).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.preferred_terminal.as_deref(), Some("xterm"));
    }
}
