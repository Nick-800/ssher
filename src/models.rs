use crate::error::{Result, SshManagerError};
use crate::storage::Storage;
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

    pub fn resolve_server<'a>(&'a self, query: &str) -> Result<&'a Server> {
        resolve_server(&self.servers, query)
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

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ServerEdit {
    pub new_name: Option<String>,
    pub host: Option<String>,
    pub username: Option<String>,
    pub port: Option<u16>,
    pub auth: Option<AuthEdit>,
    pub agent_forwarding: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AuthEdit {
    Key(String),
    Password,
}

impl ServerEdit {
    pub fn is_empty(&self) -> bool {
        self.new_name.is_none()
            && self.host.is_none()
            && self.username.is_none()
            && self.port.is_none()
            && self.auth.is_none()
            && self.agent_forwarding.is_none()
    }

    pub fn apply(&self, server: &mut Server, config: &Config) -> Result<()> {
        if self.is_empty() {
            return Err(SshManagerError::NothingToChange);
        }

        if let Some(new_name) = &self.new_name {
            Storage::validate_server_name(new_name)?;
            if !Storage::server_name_is_unique(config, new_name, Some(&server.name)) {
                return Err(SshManagerError::ServerAlreadyExists(new_name.clone()));
            }
        }

        if let Some(host) = &self.host {
            Storage::validate_host(host)?;
        }

        if let Some(username) = &self.username {
            Storage::validate_username(username)?;
        }

        if let Some(port) = self.port {
            Storage::validate_port(port)?;
        }

        if let Some(AuthEdit::Key(key_path)) = &self.auth {
            Storage::validate_key_path(key_path)?;
        }

        if let Some(new_name) = &self.new_name {
            server.name = new_name.clone();
        }
        if let Some(host) = &self.host {
            server.host = host.clone();
        }
        if let Some(username) = &self.username {
            server.username = username.clone();
        }
        if let Some(port) = self.port {
            server.port = port;
        }
        match &self.auth {
            Some(AuthEdit::Key(key_path)) => {
                server.auth_method = AuthMethod::Key;
                server.key_path = Some(key_path.clone());
            }
            Some(AuthEdit::Password) => {
                server.auth_method = AuthMethod::Password;
                server.key_path = None;
            }
            None => {}
        }
        if let Some(af) = self.agent_forwarding {
            server.use_agent_forwarding = af;
        }

        Ok(())
    }
}

/// Check if a server matches a case-insensitive query across its name or host.
/// An empty query matches every server.
pub fn server_matches_query(server: &Server, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let q = query.to_lowercase();
    server.name.to_lowercase().contains(&q) || server.host.to_lowercase().contains(&q)
}

/// Filter a list of servers by query, returning indices into the slice.
pub fn filter_server_indices(servers: &[Server], query: &str) -> Vec<usize> {
    servers
        .iter()
        .enumerate()
        .filter_map(|(idx, s)| {
            if server_matches_query(s, query) {
                Some(idx)
            } else {
                None
            }
        })
        .collect()
}

/// Resolve a server by query name: exact match wins; otherwise unique partial substring match on
/// server name (case-insensitive) succeeds; ambiguous match returns AmbiguousServerName;
/// zero matches returns ServerNotFound.
pub fn resolve_server<'a>(servers: &'a [Server], query: &str) -> Result<&'a Server> {
    if query.is_empty() {
        return Err(SshManagerError::ServerNotFound(query.to_string()));
    }

    if let Some(s) = servers.iter().find(|s| s.name == query) {
        return Ok(s);
    }

    let q_lower = query.to_lowercase();
    let candidates: Vec<&'a Server> = servers
        .iter()
        .filter(|s| s.name.to_lowercase().contains(&q_lower))
        .collect();

    match candidates.len() {
        0 => Err(SshManagerError::ServerNotFound(query.to_string())),
        1 => Ok(candidates[0]),
        _ => {
            let mut names: Vec<&str> = candidates.iter().map(|s| s.name.as_str()).collect();
            names.sort();
            Err(SshManagerError::AmbiguousServerName(
                query.to_string(),
                names.join(", "),
            ))
        }
    }
}

/// Format server names for shell completion (one per line).
pub fn format_server_completions(servers: &[Server]) -> String {
    let mut output = String::new();
    for server in servers {
        output.push_str(&server.name);
        output.push('\n');
    }
    output
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

    fn sample_server(name: &str) -> Server {
        Server::new(
            name.to_string(),
            "10.0.0.1".to_string(),
            "alice".to_string(),
            22,
            AuthMethod::Password,
            None,
        )
    }

    #[test]
    fn edit_apply_nothing_to_change() {
        let mut s = sample_server("alpha");
        let cfg = Config::new();
        let edit = ServerEdit::default();
        let res = edit.apply(&mut s, &cfg);
        assert!(matches!(res, Err(SshManagerError::NothingToChange)));
    }

    #[test]
    fn edit_apply_each_field() {
        let mut s = sample_server("alpha");
        let cfg = Config::new();

        let edit_host = ServerEdit {
            host: Some("example.com".to_string()),
            ..Default::default()
        };
        assert!(edit_host.apply(&mut s, &cfg).is_ok());
        assert_eq!(s.host, "example.com");

        let edit_user = ServerEdit {
            username: Some("bob".to_string()),
            ..Default::default()
        };
        assert!(edit_user.apply(&mut s, &cfg).is_ok());
        assert_eq!(s.username, "bob");

        let edit_port = ServerEdit {
            port: Some(2222),
            ..Default::default()
        };
        assert!(edit_port.apply(&mut s, &cfg).is_ok());
        assert_eq!(s.port, 2222);

        let edit_af = ServerEdit {
            agent_forwarding: Some(true),
            ..Default::default()
        };
        assert!(edit_af.apply(&mut s, &cfg).is_ok());
        assert!(s.use_agent_forwarding);

        let edit_af_off = ServerEdit {
            agent_forwarding: Some(false),
            ..Default::default()
        };
        assert!(edit_af_off.apply(&mut s, &cfg).is_ok());
        assert!(!s.use_agent_forwarding);

        let edit_name = ServerEdit {
            new_name: Some("beta".to_string()),
            ..Default::default()
        };
        assert!(edit_name.apply(&mut s, &cfg).is_ok());
        assert_eq!(s.name, "beta");
    }

    #[test]
    fn edit_apply_validation_failures() {
        let mut s = sample_server("alpha");
        let cfg = Config::new();

        let bad_name = ServerEdit {
            new_name: Some("".to_string()),
            ..Default::default()
        };
        assert!(matches!(
            bad_name.apply(&mut s, &cfg),
            Err(SshManagerError::EmptyServerName)
        ));

        let invalid_name = ServerEdit {
            new_name: Some("bad name!".to_string()),
            ..Default::default()
        };
        assert!(matches!(
            invalid_name.apply(&mut s, &cfg),
            Err(SshManagerError::InvalidServerName)
        ));

        let bad_host = ServerEdit {
            host: Some("".to_string()),
            ..Default::default()
        };
        assert!(matches!(
            bad_host.apply(&mut s, &cfg),
            Err(SshManagerError::EmptyHost)
        ));

        let bad_user = ServerEdit {
            username: Some("".to_string()),
            ..Default::default()
        };
        assert!(matches!(
            bad_user.apply(&mut s, &cfg),
            Err(SshManagerError::EmptyUsername)
        ));

        let bad_port = ServerEdit {
            port: Some(0),
            ..Default::default()
        };
        assert!(matches!(
            bad_port.apply(&mut s, &cfg),
            Err(SshManagerError::InvalidPort)
        ));

        let bad_key = ServerEdit {
            auth: Some(AuthEdit::Key("/nonexistent/key/file/path".to_string())),
            ..Default::default()
        };
        assert!(matches!(
            bad_key.apply(&mut s, &cfg),
            Err(SshManagerError::KeyFileNotFound(_))
        ));
    }

    #[test]
    fn edit_apply_key_password_switch() {
        let key_file = tempfile::NamedTempFile::new().unwrap();
        let key_path = key_file.path().to_str().unwrap().to_string();

        let mut s = sample_server("alpha");
        let cfg = Config::new();
        assert_eq!(s.auth_method, AuthMethod::Password);
        assert_eq!(s.key_path, None);

        // Switch to Key
        let switch_to_key = ServerEdit {
            auth: Some(AuthEdit::Key(key_path.clone())),
            ..Default::default()
        };
        assert!(switch_to_key.apply(&mut s, &cfg).is_ok());
        assert_eq!(s.auth_method, AuthMethod::Key);
        assert_eq!(s.key_path, Some(key_path));

        // Switch back to Password
        let switch_to_pw = ServerEdit {
            auth: Some(AuthEdit::Password),
            ..Default::default()
        };
        assert!(switch_to_pw.apply(&mut s, &cfg).is_ok());
        assert_eq!(s.auth_method, AuthMethod::Password);
        assert_eq!(s.key_path, None);
    }

    #[test]
    fn edit_apply_rename_uniqueness() {
        let mut cfg = Config::new();
        cfg.servers.push(sample_server("alpha"));
        cfg.servers.push(sample_server("beta"));

        let mut s = cfg.servers[0].clone();

        // Renaming to already existing "beta" should fail
        let dup = ServerEdit {
            new_name: Some("beta".to_string()),
            ..Default::default()
        };
        assert!(matches!(
            dup.apply(&mut s, &cfg),
            Err(SshManagerError::ServerAlreadyExists(_))
        ));

        // Renaming to same name "alpha" should succeed
        let same = ServerEdit {
            new_name: Some("alpha".to_string()),
            ..Default::default()
        };
        assert!(same.apply(&mut s, &cfg).is_ok());

        // Renaming to unused "gamma" should succeed
        let unused = ServerEdit {
            new_name: Some("gamma".to_string()),
            ..Default::default()
        };
        assert!(unused.apply(&mut s, &cfg).is_ok());
        assert_eq!(s.name, "gamma");
    }

    #[test]
    fn filter_matching_tests() {
        let s = Server::new(
            "prod-db".to_string(),
            "database.internal.net".to_string(),
            "admin".to_string(),
            5432,
            AuthMethod::Password,
            None,
        );

        // Empty matches everything
        assert!(server_matches_query(&s, ""));

        // Match on name (case-insensitive)
        assert!(server_matches_query(&s, "prod"));
        assert!(server_matches_query(&s, "PROD-DB"));
        assert!(server_matches_query(&s, "-db"));

        // Match on host (case-insensitive)
        assert!(server_matches_query(&s, "internal"));
        assert!(server_matches_query(&s, "DATABASE"));

        // Non-matching
        assert!(!server_matches_query(&s, "staging"));
        assert!(!server_matches_query(&s, "xyz"));

        // filter_server_indices
        let servers = vec![
            sample_server("web-prod"),
            sample_server("db-prod"),
            sample_server("staging-web"),
        ];
        let indices = filter_server_indices(&servers, "prod");
        assert_eq!(indices, vec![0, 1]);

        let indices_none = filter_server_indices(&servers, "nomatch");
        assert!(indices_none.is_empty());
    }

    #[test]
    fn resolve_server_exact_wins() {
        let servers = vec![sample_server("web"), sample_server("web-prod")];
        let resolved = resolve_server(&servers, "web").expect("should resolve");
        assert_eq!(resolved.name, "web");
    }

    #[test]
    fn resolve_server_unique_partial() {
        let servers = vec![sample_server("web-prod"), sample_server("db-staging")];
        let resolved = resolve_server(&servers, "staging").expect("should resolve");
        assert_eq!(resolved.name, "db-staging");

        // Case-insensitive partial
        let resolved_ci = resolve_server(&servers, "STAGING").expect("should resolve");
        assert_eq!(resolved_ci.name, "db-staging");
    }

    #[test]
    fn resolve_server_ambiguous() {
        let servers = vec![sample_server("prod-web"), sample_server("prod-db")];
        let err = resolve_server(&servers, "prod").unwrap_err();
        match err {
            SshManagerError::AmbiguousServerName(query, candidates) => {
                assert_eq!(query, "prod");
                assert_eq!(candidates, "prod-db, prod-web");
            }
            other => panic!("expected AmbiguousServerName, got {other:?}"),
        }
    }

    #[test]
    fn resolve_server_none() {
        let servers = vec![sample_server("prod-web")];
        let err = resolve_server(&servers, "staging").unwrap_err();
        assert!(matches!(err, SshManagerError::ServerNotFound(q) if q == "staging"));

        // Empty query
        let empty_err = resolve_server(&servers, "").unwrap_err();
        assert!(matches!(empty_err, SshManagerError::ServerNotFound(q) if q.is_empty()));
    }

    #[test]
    fn format_server_completions_empty() {
        let servers: Vec<Server> = Vec::new();
        assert_eq!(format_server_completions(&servers), "");
    }

    #[test]
    fn format_server_completions_single() {
        let servers = vec![sample_server("alpha")];
        assert_eq!(format_server_completions(&servers), "alpha\n");
    }

    #[test]
    fn format_server_completions_multiple() {
        let servers = vec![
            sample_server("web-prod"),
            sample_server("db-staging"),
            sample_server("backup_01"),
        ];
        assert_eq!(
            format_server_completions(&servers),
            "web-prod\ndb-staging\nbackup_01\n"
        );
    }
}
