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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
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
            tags: Vec::new(),
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
    pub tags: Option<Vec<String>>,
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
            && self.tags.is_none()
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

        let normalized_tags = if let Some(tags) = &self.tags {
            Some(Storage::normalize_tags(tags)?)
        } else {
            None
        };

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
        if let Some(tags) = normalized_tags {
            server.tags = tags;
        }

        Ok(())
    }
}

/// Check if a server matches a case-insensitive query across its name, host, or tags.
/// An empty query matches every server.
pub fn server_matches_query(server: &Server, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let q = query.to_lowercase();
    server.name.to_lowercase().contains(&q)
        || server.host.to_lowercase().contains(&q)
        || server.tags.iter().any(|t| t.to_lowercase().contains(&q))
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

/// Format distinct tags across all servers for shell completion (one per line, sorted).
pub fn format_tag_completions(servers: &[Server]) -> String {
    let mut tags: Vec<String> = servers
        .iter()
        .flat_map(|s| s.tags.iter().cloned())
        .collect();
    tags.sort();
    tags.dedup();
    let mut output = String::new();
    for tag in tags {
        output.push_str(&tag);
        output.push('\n');
    }
    output
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TerminalConfig {
    pub terminal: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportedHost {
    pub name: String,
    pub host: String,
    pub username: Option<String>,
    pub port: u16,
    pub identity_file: Option<String>,
    pub forward_agent: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SshConfigParseResult {
    pub hosts: Vec<ImportedHost>,
    pub skipped: Vec<(String, String)>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportResult {
    pub to_import: Vec<Server>,
    pub skipped: Vec<(String, String)>,
}

fn strip_comment(line: &str) -> &str {
    let mut in_quote = false;
    let mut quote_char = ' ';
    let mut escape = false;
    for (idx, ch) in line.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if ch == '\\' {
            escape = true;
            continue;
        }
        if in_quote {
            if ch == quote_char {
                in_quote = false;
            }
        } else if ch == '"' || ch == '\'' {
            in_quote = true;
            quote_char = ch;
        } else if ch == '#' {
            return &line[..idx];
        }
    }
    line
}

fn split_key_value(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let delim_pos = trimmed.find(|c: char| c.is_whitespace() || c == '=')?;
    let key = &trimmed[..delim_pos];
    let rest = trimmed[delim_pos..].trim_start();
    let value = if let Some(stripped) = rest.strip_prefix('=') {
        stripped.trim_start()
    } else {
        rest
    };
    Some((key, value.trim()))
}

fn parse_tokens(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut quote_char = ' ';
    let mut escape = false;

    for ch in s.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }
        if ch == '\\' {
            escape = true;
            continue;
        }
        if in_quote {
            if ch == quote_char {
                in_quote = false;
            } else {
                current.push(ch);
            }
        } else if ch == '"' || ch == '\'' {
            in_quote = true;
            quote_char = ch;
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn is_wildcard_or_negated(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?') || pattern.contains('!')
}

fn strip_quotes(s: &str) -> &str {
    let trimmed = s.trim();
    if (trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2)
        || (trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2)
    {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    }
}

fn expand_tilde(path: &str, home: Option<&str>) -> String {
    if let Some(home_path) = home {
        let home_clean = home_path.trim_end_matches('/');
        if path == "~" {
            home_clean.to_string()
        } else if let Some(rest) = path.strip_prefix("~/") {
            format!("{}/{}", home_clean, rest)
        } else {
            path.to_string()
        }
    } else {
        path.to_string()
    }
}

/// Pure parser for SSH config content.
#[allow(dead_code)]
pub fn parse_ssh_config(content: &str, home: Option<&str>) -> Vec<ImportedHost> {
    parse_ssh_config_detailed(content, home).hosts
}

/// Parse SSH config content with full details including skipped hosts and warnings.
pub fn parse_ssh_config_detailed(content: &str, home: Option<&str>) -> SshConfigParseResult {
    let mut hosts = Vec::new();
    let mut skipped = Vec::new();
    let mut warnings = Vec::new();
    let mut warned_include = false;
    let mut warned_match = false;

    struct CurrentBlock {
        aliases: Vec<String>,
        hostname: Option<String>,
        user: Option<String>,
        port: Option<u16>,
        identity_file: Option<String>,
        forward_agent: Option<bool>,
    }

    let mut current_block: Option<CurrentBlock> = None;
    let mut in_match_block = false;

    let finalize_block = |block: CurrentBlock,
                          hosts: &mut Vec<ImportedHost>,
                          skipped: &mut Vec<(String, String)>| {
        for alias in block.aliases {
            match &block.identity_file {
                Some(key) => {
                    hosts.push(ImportedHost {
                        name: alias.clone(),
                        host: block.hostname.clone().unwrap_or_else(|| alias.clone()),
                        username: block.user.clone(),
                        port: block.port.unwrap_or(22),
                        identity_file: Some(key.clone()),
                        forward_agent: block.forward_agent.unwrap_or(false),
                    });
                }
                None => {
                    skipped.push((
                        alias,
                        "no IdentityFile; add manually with --password".to_string(),
                    ));
                }
            }
        }
    };

    for raw_line in content.lines() {
        let line_without_comment = strip_comment(raw_line).trim();
        if line_without_comment.is_empty() {
            continue;
        }

        let Some((key, val)) = split_key_value(line_without_comment) else {
            continue;
        };

        let key_lower = key.to_lowercase();
        match key_lower.as_str() {
            "include" => {
                if !warned_include {
                    warnings.push("Include directive is not followed".to_string());
                    warned_include = true;
                }
            }
            "match" => {
                if !warned_match {
                    warnings.push("Match directive is not followed".to_string());
                    warned_match = true;
                }
                if let Some(b) = current_block.take() {
                    finalize_block(b, &mut hosts, &mut skipped);
                }
                in_match_block = true;
            }
            "host" => {
                if let Some(b) = current_block.take() {
                    finalize_block(b, &mut hosts, &mut skipped);
                }
                in_match_block = false;

                let patterns = parse_tokens(val);
                let concrete_aliases: Vec<String> = patterns
                    .into_iter()
                    .filter(|p| !is_wildcard_or_negated(p))
                    .collect();

                current_block = Some(CurrentBlock {
                    aliases: concrete_aliases,
                    hostname: None,
                    user: None,
                    port: None,
                    identity_file: None,
                    forward_agent: None,
                });
            }
            _ => {
                if in_match_block {
                    continue;
                }
                if let Some(block) = current_block.as_mut() {
                    match key_lower.as_str() {
                        "hostname" if block.hostname.is_none() => {
                            block.hostname = Some(strip_quotes(val).to_string());
                        }
                        "user" if block.user.is_none() => {
                            block.user = Some(strip_quotes(val).to_string());
                        }
                        "port" if block.port.is_none() => {
                            let unquoted = strip_quotes(val);
                            match unquoted.parse::<u16>() {
                                Ok(p) if p > 0 => {
                                    block.port = Some(p);
                                }
                                _ => {
                                    warnings.push(format!(
                                        "Invalid port '{}', defaulting to 22",
                                        unquoted
                                    ));
                                    block.port = Some(22);
                                }
                            }
                        }
                        "identityfile" if block.identity_file.is_none() => {
                            let unquoted = strip_quotes(val);
                            let expanded = expand_tilde(unquoted, home);
                            block.identity_file = Some(expanded);
                        }
                        "forwardagent" if block.forward_agent.is_none() => {
                            let unquoted = strip_quotes(val).to_lowercase();
                            match unquoted.as_str() {
                                "yes" | "true" => block.forward_agent = Some(true),
                                "no" | "false" => block.forward_agent = Some(false),
                                _ => {}
                            }
                        }
                        _ => {
                            // unknown keywords ignored
                        }
                    }
                }
            }
        }
    }

    if let Some(b) = current_block.take() {
        finalize_block(b, &mut hosts, &mut skipped);
    }

    SshConfigParseResult {
        hosts,
        skipped,
        warnings,
    }
}

/// Pure helper to map and validate imported hosts against existing config.
pub fn filter_and_map_imported_hosts(
    existing_config: &Config,
    imported_hosts: Vec<ImportedHost>,
    default_user: Option<&str>,
    validate_key_exists: impl Fn(&str) -> bool,
) -> ImportResult {
    let mut to_import = Vec::new();
    let mut skipped = Vec::new();

    for host in imported_hosts {
        if let Err(e) = Storage::validate_server_name(&host.name) {
            skipped.push((host.name.clone(), format!("Invalid server name: {}", e)));
            continue;
        }

        let exists_in_config = !Storage::server_name_is_unique(existing_config, &host.name, None);
        let exists_in_batch = to_import.iter().any(|s: &Server| s.name == host.name);
        if exists_in_config || exists_in_batch {
            skipped.push((host.name.clone(), "Server already exists".to_string()));
            continue;
        }

        if let Err(e) = Storage::validate_host(&host.host) {
            skipped.push((host.name.clone(), format!("Invalid host: {}", e)));
            continue;
        }

        let username = match &host.username {
            Some(u) if !u.is_empty() => u.clone(),
            _ => match default_user {
                Some(u) if !u.is_empty() => u.to_string(),
                _ => {
                    skipped.push((
                        host.name.clone(),
                        "Username cannot be empty and $USER is not set".to_string(),
                    ));
                    continue;
                }
            },
        };
        if let Err(e) = Storage::validate_username(&username) {
            skipped.push((host.name.clone(), format!("Invalid username: {}", e)));
            continue;
        }

        if let Err(e) = Storage::validate_port(host.port) {
            skipped.push((host.name.clone(), format!("Invalid port: {}", e)));
            continue;
        }

        let key_path = match &host.identity_file {
            Some(k) => k.clone(),
            None => {
                skipped.push((
                    host.name.clone(),
                    "no IdentityFile; add manually with --password".to_string(),
                ));
                continue;
            }
        };

        if !validate_key_exists(&key_path) {
            skipped.push((
                host.name.clone(),
                format!("SSH key file not found: {}", key_path),
            ));
            continue;
        }

        let mut server = Server::new(
            host.name.clone(),
            host.host.clone(),
            username,
            host.port,
            AuthMethod::Key,
            Some(key_path),
        );
        server.use_agent_forwarding = host.forward_agent;
        to_import.push(server);
    }

    ImportResult { to_import, skipped }
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
        s.tags = vec!["prod".to_string(), "web".to_string()];
        let json = serde_json::to_string(&s).unwrap();
        let back: Server = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "alpha");
        assert_eq!(back.auth_method, AuthMethod::Key);
        assert!(back.use_agent_forwarding);
        assert_eq!(back.tags, vec!["prod".to_string(), "web".to_string()]);
    }

    #[test]
    fn server_tags_omitted_when_empty() {
        let s = Server::new(
            "alpha".to_string(),
            "example.com".to_string(),
            "alice".to_string(),
            22,
            AuthMethod::Password,
            None,
        );
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("tags"), "json was: {json}");
    }

    #[test]
    fn server_parses_from_json_without_tags_backward_compat() {
        let legacy_json = r#"{"name":"legacy","host":"legacy.net","username":"user","port":22,"auth_method":"key"}"#;
        let s: Server = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(s.name, "legacy");
        assert!(s.tags.is_empty());
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

        let edit_tags = ServerEdit {
            tags: Some(vec!["PROD".to_string(), "Web".to_string()]),
            ..Default::default()
        };
        assert!(edit_tags.apply(&mut s, &cfg).is_ok());
        assert_eq!(s.tags, vec!["prod".to_string(), "web".to_string()]);

        let edit_tags_clear = ServerEdit {
            tags: Some(Vec::new()),
            ..Default::default()
        };
        assert!(edit_tags_clear.apply(&mut s, &cfg).is_ok());
        assert!(s.tags.is_empty());
    }

    #[test]
    fn edit_apply_validation_failures() {
        let mut s = sample_server("alpha");
        let cfg = Config::new();

        let bad_tag = ServerEdit {
            tags: Some(vec!["invalid tag!".to_string()]),
            ..Default::default()
        };
        assert!(matches!(
            bad_tag.apply(&mut s, &cfg),
            Err(SshManagerError::InvalidTag(_))
        ));

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

        // Match on tag (case-insensitive)
        let mut s_with_tags = s.clone();
        s_with_tags.tags = vec!["backend".to_string(), "critical".to_string()];
        assert!(server_matches_query(&s_with_tags, "back"));
        assert!(server_matches_query(&s_with_tags, "BACKEND"));
        assert!(server_matches_query(&s_with_tags, "crit"));
        assert!(!server_matches_query(&s_with_tags, "frontend"));

        // filter_server_indices
        let mut s1 = sample_server("web-prod");
        s1.tags = vec!["frontend".to_string()];
        let mut s2 = sample_server("db-prod");
        s2.tags = vec!["database".to_string()];
        let mut s3 = sample_server("staging-web");
        s3.tags = vec!["frontend".to_string()];

        let servers = vec![s1, s2, s3];
        let indices = filter_server_indices(&servers, "prod");
        assert_eq!(indices, vec![0, 1]);

        let indices_tag = filter_server_indices(&servers, "frontend");
        assert_eq!(indices_tag, vec![0, 2]);

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

    #[test]
    fn format_tag_completions_empty() {
        let servers: Vec<Server> = Vec::new();
        assert_eq!(format_tag_completions(&servers), "");
    }

    #[test]
    fn format_tag_completions_multiple_deduplicated_and_sorted() {
        let mut s1 = sample_server("s1");
        s1.tags = vec!["web".to_string(), "prod".to_string()];
        let mut s2 = sample_server("s2");
        s2.tags = vec!["prod".to_string(), "api".to_string()];
        let mut s3 = sample_server("s3");
        s3.tags = vec![];
        let servers = vec![s1, s2, s3];
        assert_eq!(format_tag_completions(&servers), "api\nprod\nweb\n");
    }

    #[test]
    fn parse_ssh_config_basic_host() {
        let input = r#"
Host prod
    HostName 10.0.0.1
    User alice
    Port 2222
    IdentityFile ~/.ssh/id_ed25519
    ForwardAgent yes
"#;
        let hosts = parse_ssh_config(input, Some("/home/alice"));
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "prod");
        assert_eq!(hosts[0].host, "10.0.0.1");
        assert_eq!(hosts[0].username.as_deref(), Some("alice"));
        assert_eq!(hosts[0].port, 2222);
        assert_eq!(
            hosts[0].identity_file.as_deref(),
            Some("/home/alice/.ssh/id_ed25519")
        );
        assert!(hosts[0].forward_agent);
    }

    #[test]
    fn parse_ssh_config_multiple_patterns() {
        let input = r#"
Host web1 web2
    HostName cluster.internal
    IdentityFile /keys/deploy
"#;
        let hosts = parse_ssh_config(input, None);
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0].name, "web1");
        assert_eq!(hosts[0].host, "cluster.internal");
        assert_eq!(hosts[0].identity_file.as_deref(), Some("/keys/deploy"));
        assert_eq!(hosts[1].name, "web2");
        assert_eq!(hosts[1].host, "cluster.internal");
        assert_eq!(hosts[1].identity_file.as_deref(), Some("/keys/deploy"));
    }

    #[test]
    fn parse_ssh_config_wildcard_skipped() {
        let input = r#"
Host *.example.com
    IdentityFile /keys/star

Host host?
    IdentityFile /keys/q

Host !badhost goodhost
    IdentityFile /keys/good

Host *
    IdentityFile /keys/default
"#;
        let hosts = parse_ssh_config(input, None);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "goodhost");
        assert_eq!(hosts[0].identity_file.as_deref(), Some("/keys/good"));
    }

    #[test]
    fn parse_ssh_config_key_value_form() {
        let input = r#"
Host=alpha
HostName=alpha.example.com
User=deployer
Port=2201
IdentityFile=/keys/alpha
ForwardAgent=yes
"#;
        let hosts = parse_ssh_config(input, None);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "alpha");
        assert_eq!(hosts[0].host, "alpha.example.com");
        assert_eq!(hosts[0].username.as_deref(), Some("deployer"));
        assert_eq!(hosts[0].port, 2201);
        assert_eq!(hosts[0].identity_file.as_deref(), Some("/keys/alpha"));
        assert!(hosts[0].forward_agent);
    }

    #[test]
    fn parse_ssh_config_quotes() {
        let input = r#"
Host "quoted-host"
    HostName "quoted.net"
    User "john doe"
    IdentityFile "/path with spaces/id_rsa"
"#;
        let hosts = parse_ssh_config(input, None);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "quoted-host");
        assert_eq!(hosts[0].host, "quoted.net");
        assert_eq!(hosts[0].username.as_deref(), Some("john doe"));
        assert_eq!(
            hosts[0].identity_file.as_deref(),
            Some("/path with spaces/id_rsa")
        );
    }

    #[test]
    fn parse_ssh_config_missing_identity_file_skipped() {
        let input = r#"
Host no-key
    HostName nokey.example.com
    User alice

Host has-key
    HostName haskey.example.com
    IdentityFile /keys/id_rsa
"#;
        let res = parse_ssh_config_detailed(input, None);
        assert_eq!(res.hosts.len(), 1);
        assert_eq!(res.hosts[0].name, "has-key");
        assert_eq!(res.skipped.len(), 1);
        assert_eq!(res.skipped[0].0, "no-key");
        assert!(res.skipped[0]
            .1
            .contains("no IdentityFile; add manually with --password"));
    }

    #[test]
    fn parse_ssh_config_tilde_expansion() {
        let input = r#"
Host home-test
    IdentityFile ~/.ssh/custom_key
"#;
        let hosts_expanded = parse_ssh_config(input, Some("/fake/home/user"));
        assert_eq!(hosts_expanded.len(), 1);
        assert_eq!(
            hosts_expanded[0].identity_file.as_deref(),
            Some("/fake/home/user/.ssh/custom_key")
        );

        let hosts_unexpanded = parse_ssh_config(input, None);
        assert_eq!(hosts_unexpanded.len(), 1);
        assert_eq!(
            hosts_unexpanded[0].identity_file.as_deref(),
            Some("~/.ssh/custom_key")
        );
    }

    #[test]
    fn parse_ssh_config_invalid_port() {
        let input = r#"
Host bad-port-alpha
    Port notanumber
    IdentityFile /keys/k1

Host bad-port-zero
    Port 0
    IdentityFile /keys/k2

Host bad-port-overflow
    Port 70000
    IdentityFile /keys/k3
"#;
        let res = parse_ssh_config_detailed(input, None);
        assert_eq!(res.hosts.len(), 3);
        assert_eq!(res.hosts[0].port, 22);
        assert_eq!(res.hosts[1].port, 22);
        assert_eq!(res.hosts[2].port, 22);
        assert_eq!(res.warnings.len(), 3);
    }

    #[test]
    fn parse_ssh_config_comments() {
        let input = r#"
# Global comment
# Another comment
Host myhost # inline comment after host
    # Indented comment
    HostName 10.1.1.1 # inline comment on hostname
    Port 22 # default port
    IdentityFile /keys/mykey
"#;
        let hosts = parse_ssh_config(input, None);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "myhost");
        assert_eq!(hosts[0].host, "10.1.1.1");
        assert_eq!(hosts[0].port, 22);
        assert_eq!(hosts[0].identity_file.as_deref(), Some("/keys/mykey"));
    }

    #[test]
    fn parse_ssh_config_unknown_keywords_ignored() {
        let input = r#"
Host complex
    ServerAliveInterval 30
    StrictHostKeyChecking no
    ProxyJump bastion.example.com
    UserKnownHostsFile /dev/null
    IdentityFile /keys/id_rsa
"#;
        let hosts = parse_ssh_config(input, None);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "complex");
        assert_eq!(hosts[0].identity_file.as_deref(), Some("/keys/id_rsa"));
    }

    #[test]
    fn parse_ssh_config_include_and_match_warned() {
        let input = r#"
Include /etc/ssh/extra_config
Include /etc/ssh/more_config
Host srv
    IdentityFile /keys/id_rsa
Match host complex
    Port 2200
Match host other
    Port 2201
"#;
        let res = parse_ssh_config_detailed(input, None);
        assert_eq!(res.hosts.len(), 1);
        assert_eq!(res.hosts[0].name, "srv");
        assert_eq!(res.warnings.len(), 2);
        assert!(res
            .warnings
            .iter()
            .any(|w| w.contains("Include directive is not followed")));
        assert!(res
            .warnings
            .iter()
            .any(|w| w.contains("Match directive is not followed")));
    }

    #[test]
    fn filter_and_map_imported_hosts_basic() {
        let cfg = Config::new();
        let imported = vec![ImportedHost {
            name: "srv1".to_string(),
            host: "1.2.3.4".to_string(),
            username: Some("alice".to_string()),
            port: 22,
            identity_file: Some("/valid/key".to_string()),
            forward_agent: true,
        }];

        let res = filter_and_map_imported_hosts(&cfg, imported, None, |_| true);
        assert_eq!(res.to_import.len(), 1);
        assert_eq!(res.skipped.len(), 0);
        assert_eq!(res.to_import[0].name, "srv1");
        assert_eq!(res.to_import[0].auth_method, AuthMethod::Key);
        assert_eq!(res.to_import[0].key_path.as_deref(), Some("/valid/key"));
        assert!(res.to_import[0].use_agent_forwarding);
    }

    #[test]
    fn filter_and_map_imported_hosts_collision_skipped() {
        let mut cfg = Config::new();
        cfg.servers.push(sample_server("existing"));

        let imported = vec![
            ImportedHost {
                name: "existing".to_string(),
                host: "2.2.2.2".to_string(),
                username: Some("bob".to_string()),
                port: 22,
                identity_file: Some("/key".to_string()),
                forward_agent: false,
            },
            ImportedHost {
                name: "batch_dup".to_string(),
                host: "3.3.3.3".to_string(),
                username: Some("bob".to_string()),
                port: 22,
                identity_file: Some("/key".to_string()),
                forward_agent: false,
            },
            ImportedHost {
                name: "batch_dup".to_string(),
                host: "4.4.4.4".to_string(),
                username: Some("bob".to_string()),
                port: 22,
                identity_file: Some("/key".to_string()),
                forward_agent: false,
            },
        ];

        let res = filter_and_map_imported_hosts(&cfg, imported, None, |_| true);
        assert_eq!(res.to_import.len(), 1);
        assert_eq!(res.to_import[0].name, "batch_dup");
        assert_eq!(res.skipped.len(), 2);
        assert_eq!(res.skipped[0].0, "existing");
        assert_eq!(res.skipped[0].1, "Server already exists");
        assert_eq!(res.skipped[1].0, "batch_dup");
        assert_eq!(res.skipped[1].1, "Server already exists");
    }

    #[test]
    fn filter_and_map_imported_hosts_missing_key_skipped() {
        let cfg = Config::new();
        let imported = vec![ImportedHost {
            name: "srv".to_string(),
            host: "1.2.3.4".to_string(),
            username: Some("alice".to_string()),
            port: 22,
            identity_file: Some("/missing/key".to_string()),
            forward_agent: false,
        }];

        let res = filter_and_map_imported_hosts(&cfg, imported, None, |p| p != "/missing/key");
        assert_eq!(res.to_import.len(), 0);
        assert_eq!(res.skipped.len(), 1);
        assert!(res.skipped[0].1.contains("SSH key file not found"));
    }

    #[test]
    fn filter_and_map_imported_hosts_user_fallback() {
        let cfg = Config::new();
        let imported = vec![
            ImportedHost {
                name: "srv1".to_string(),
                host: "1.2.3.4".to_string(),
                username: None,
                port: 22,
                identity_file: Some("/key".to_string()),
                forward_agent: false,
            },
            ImportedHost {
                name: "srv2".to_string(),
                host: "1.2.3.5".to_string(),
                username: None,
                port: 22,
                identity_file: Some("/key".to_string()),
                forward_agent: false,
            },
        ];

        // With default_user Some("fallback_user")
        let res_with_fallback =
            filter_and_map_imported_hosts(&cfg, imported.clone(), Some("fallback_user"), |_| true);
        assert_eq!(res_with_fallback.to_import.len(), 2);
        assert_eq!(res_with_fallback.to_import[0].username, "fallback_user");

        // Without default_user
        let res_without_fallback = filter_and_map_imported_hosts(&cfg, imported, None, |_| true);
        assert_eq!(res_without_fallback.to_import.len(), 0);
        assert_eq!(res_without_fallback.skipped.len(), 2);
        assert!(res_without_fallback.skipped[0]
            .1
            .contains("Username cannot be empty"));
    }

    #[test]
    fn filter_and_map_imported_hosts_invalid_server_name() {
        let cfg = Config::new();
        let imported = vec![ImportedHost {
            name: "bad server name!".to_string(),
            host: "1.2.3.4".to_string(),
            username: Some("alice".to_string()),
            port: 22,
            identity_file: Some("/key".to_string()),
            forward_agent: false,
        }];

        let res = filter_and_map_imported_hosts(&cfg, imported, None, |_| true);
        assert_eq!(res.to_import.len(), 0);
        assert_eq!(res.skipped.len(), 1);
        assert!(res.skipped[0].1.contains("Invalid server name"));
    }
}
