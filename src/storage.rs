use crate::error::{Result, SshManagerError};
use crate::models::{AuthEdit, AuthMethod, Config, Server, ServerEdit, TerminalConfig};
use crate::ssh::Ssh;
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

    /// Legacy file from v0.1.0; migrated into servers.json on first load.
    fn get_terminal_config_file() -> Result<PathBuf> {
        let config_dir = Self::ensure_config_dir()?;
        Ok(config_dir.join("terminal_config.json"))
    }

    pub fn load_config() -> Result<Config> {
        let servers_file = Self::get_servers_file()?;

        if !servers_file.exists() {
            return Ok(Config::new());
        }

        let content = fs::read_to_string(&servers_file)?;
        let mut config = parse_servers_file(&content)?;

        // Backwards-compatible migration: if preferred_terminal was never
        // written to servers.json (pre-v0.1.1), pull it from the legacy file
        // once and rewrite servers.json with the merged value.
        if config.preferred_terminal.is_none() {
            if let Some(terminal) = Self::read_legacy_terminal()? {
                config.preferred_terminal = Some(terminal);
                Self::save_config(&config)?;
                let _ = fs::remove_file(Self::get_terminal_config_file()?);
            }
        } else {
            // preferred_terminal is in servers.json; ensure the legacy file is gone.
            let legacy = Self::get_terminal_config_file()?;
            if legacy.exists() {
                let _ = fs::remove_file(legacy);
            }
        }

        Ok(config)
    }

    pub fn save_config(config: &Config) -> Result<()> {
        let servers_file = Self::get_servers_file()?;
        let json = serde_json::to_string_pretty(config)?;
        fs::write(&servers_file, json)?;

        // If we still happen to have a legacy terminal_config.json lying around,
        // remove it now that the canonical state lives in servers.json.
        let legacy = Self::get_terminal_config_file()?;
        if legacy.exists() {
            let _ = fs::remove_file(legacy);
        }

        Ok(())
    }

    fn read_legacy_terminal() -> Result<Option<String>> {
        let path = Self::get_terminal_config_file()?;
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path)?;
        match serde_json::from_str::<TerminalConfig>(&content) {
            Ok(tc) => Ok(Some(tc.terminal)),
            Err(_) => Ok(None),
        }
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

    pub fn validate_tag(tag: &str) -> Result<()> {
        if tag.is_empty() {
            return Err(SshManagerError::EmptyTag);
        }
        if tag.len() > 32 {
            return Err(SshManagerError::TagTooLong(tag.to_string()));
        }
        if tag.contains(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-') {
            return Err(SshManagerError::InvalidTag(tag.to_string()));
        }
        Ok(())
    }

    pub fn validate_jump_host(spec: &str) -> Result<()> {
        if spec.is_empty() {
            return Err(SshManagerError::EmptyJumpHost);
        }
        if spec.chars().any(|c| {
            c.is_whitespace()
                || c.is_control()
                || matches!(
                    c,
                    ';' | '&'
                        | '|'
                        | '<'
                        | '>'
                        | '`'
                        | '$'
                        | '\\'
                        | '"'
                        | '\''
                        | '*'
                        | '?'
                        | '['
                        | ']'
                        | '('
                        | ')'
                        | '{'
                        | '}'
                        | '!'
                        | '^'
                        | '~'
                )
        }) {
            return Err(SshManagerError::InvalidJumpHost(spec.to_string()));
        }
        Ok(())
    }

    pub fn validate_server_jump(config: &Config, server_name: &str, jump_spec: &str) -> Result<()> {
        Self::validate_jump_host(jump_spec)?;
        if jump_spec == server_name {
            return Err(SshManagerError::SelfReferentialJumpHost(
                server_name.to_string(),
            ));
        }
        let mut visited = vec![server_name.to_string()];
        let mut current_spec = jump_spec.to_string();
        loop {
            if visited.contains(&current_spec) {
                let mut cycle_path = visited.clone();
                cycle_path.push(current_spec);
                return Err(SshManagerError::JumpHostCycle(cycle_path.join(" -> ")));
            }
            if let Some(target) = config.find_server(&current_spec) {
                visited.push(target.name.clone());
                if let Some(next) = &target.jump_host {
                    current_spec = next.clone();
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        Ok(())
    }

    pub fn validate_server_removal(config: &Config, server_name: &str) -> Result<()> {
        let mut dependents = crate::models::find_jump_dependents(&config.servers, server_name);
        if !dependents.is_empty() {
            dependents.sort();
            return Err(SshManagerError::ServerReferencedAsJumpHost(
                server_name.to_string(),
                dependents.join(", "),
            ));
        }
        Ok(())
    }

    pub fn normalize_tags<S: AsRef<str>>(tags: &[S]) -> Result<Vec<String>> {
        let mut result = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for tag in tags {
            let t = tag.as_ref();
            Self::validate_tag(t)?;
            let lower = t.to_lowercase();
            if seen.insert(lower.clone()) {
                result.push(lower);
            }
        }

        Ok(result)
    }

    /// Execute server edit, handling validation, keyring side-effects, and config persistence.
    pub fn execute_server_edit(
        config: &mut Config,
        target_name: &str,
        edit: ServerEdit,
        new_password: Option<String>,
    ) -> Result<String> {
        let idx = config
            .servers
            .iter()
            .position(|s| s.name == target_name)
            .ok_or_else(|| SshManagerError::ServerNotFound(target_name.to_string()))?;

        let old_name = config.servers[idx].name.clone();
        let old_auth = config.servers[idx].auth_method.clone();

        let mut updated = config.servers[idx].clone();
        edit.apply(&mut updated, config)?;
        config.servers[idx] = updated;

        let new_name = config.servers[idx].name.clone();

        if old_name != new_name {
            crate::models::update_jump_host_references(&mut config.servers, &old_name, &new_name);
        }

        // Write new keyring entries first, but only delete old ones after the
        // config is saved, so a failed save can't lose the stored password.
        let mut stale_entry = None;
        match &edit.auth {
            Some(AuthEdit::Key(_)) => {
                stale_entry = Some(old_name.clone());
            }
            Some(AuthEdit::Password) => {
                let pw = new_password.ok_or_else(|| {
                    SshManagerError::InvalidConfig("Password cannot be empty".to_string())
                })?;
                Ssh::store_password(&new_name, &pw)?;
                if old_name != new_name {
                    stale_entry = Some(old_name.clone());
                }
            }
            None => {
                if old_auth == AuthMethod::Password && old_name != new_name {
                    let old_pw = Ssh::get_password(&old_name)?;
                    Ssh::store_password(&new_name, &old_pw)?;
                    stale_entry = Some(old_name.clone());
                }
            }
        }

        Self::save_config(config)?;
        if let Some(name) = stale_entry {
            let _ = Ssh::delete_password(&name);
        }
        println!("Server '{}' updated successfully!", new_name);
        Ok(new_name)
    }
}

/// Parse the contents of `servers.json`, accepting both the new object
/// format (`{"servers":[...], "preferred_terminal":...}`) and the legacy
/// bare-array format (`[...]`) for backwards compatibility.
fn parse_servers_file(content: &str) -> Result<Config> {
    if let Ok(cfg) = serde_json::from_str::<Config>(content) {
        return Ok(cfg);
    }
    if let Ok(servers) = serde_json::from_str::<Vec<Server>>(content) {
        return Ok(Config {
            servers,
            preferred_terminal: None,
        });
    }
    Err(SshManagerError::InvalidConfig(
        "Invalid servers.json format".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    /// Override XDG_CONFIG_HOME (and HOME on linux) so the Storage helpers
    /// read/write inside a temp directory for the duration of one test.
    ///
    /// Tests using this guard are serialized via a process-wide mutex to
    /// avoid stomping on each other's environment variables when run in
    /// parallel.
    struct ScopedConfigDir {
        original: Option<String>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    fn env_lock() -> &'static std::sync::Mutex<()> {
        // One-shot init via OnceLock; the Mutex itself is only locked to
        // serialize tests that mutate the process environment.
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    impl ScopedConfigDir {
        fn new() -> Self {
            let lock = env_lock().lock().unwrap_or_else(|e| e.into_inner());
            let dir = tempfile::tempdir().expect("tempdir");
            let path = dir.path().to_str().unwrap().to_string();
            env::set_var("XDG_CONFIG_HOME", &path);
            env::set_var("HOME", &path);
            // Keep the tempdir alive for the rest of the test by leaking it.
            std::mem::forget(dir);
            Self {
                original: Some(path),
                _lock: lock,
            }
        }
    }

    impl Drop for ScopedConfigDir {
        fn drop(&mut self) {
            env::remove_var("XDG_CONFIG_HOME");
            env::remove_var("HOME");
            if let Some(p) = &self.original {
                let _ = fs::remove_dir_all(p);
            }
        }
    }

    fn sample_server(name: &str) -> Server {
        Server::new(
            name.to_string(),
            "example.com".to_string(),
            "alice".to_string(),
            22,
            crate::models::AuthMethod::Key,
            Some("/tmp/key".to_string()),
        )
    }

    #[test]
    fn validate_server_name_accepts_alnum_underscore_dash() {
        assert!(Storage::validate_server_name("prod-1_db").is_ok());
        assert!(Storage::validate_server_name("a").is_ok());
    }

    #[test]
    fn validate_server_name_rejects_empty() {
        assert!(matches!(
            Storage::validate_server_name(""),
            Err(SshManagerError::EmptyServerName)
        ));
    }

    #[test]
    fn validate_server_name_rejects_invalid_chars() {
        for bad in ["bad name", "name!", "name/with/slash", "name.with.dot"] {
            assert!(
                matches!(
                    Storage::validate_server_name(bad),
                    Err(SshManagerError::InvalidServerName)
                ),
                "expected '{bad}' to be rejected"
            );
        }
    }

    #[test]
    fn validate_port_zero_is_invalid() {
        assert!(matches!(
            Storage::validate_port(0),
            Err(SshManagerError::InvalidPort)
        ));
    }

    #[test]
    fn validate_port_edges() {
        assert!(Storage::validate_port(1).is_ok());
        assert!(Storage::validate_port(65535).is_ok());
    }

    #[test]
    fn validate_username_rejects_empty() {
        assert!(matches!(
            Storage::validate_username(""),
            Err(SshManagerError::EmptyUsername)
        ));
        assert!(Storage::validate_username("root").is_ok());
    }

    #[test]
    fn validate_host_rejects_empty() {
        assert!(matches!(
            Storage::validate_host(""),
            Err(SshManagerError::EmptyHost)
        ));
        assert!(Storage::validate_host("example.com").is_ok());
    }

    #[test]
    fn validate_tag_accepts_valid() {
        assert!(Storage::validate_tag("a").is_ok());
        assert!(Storage::validate_tag("prod").is_ok());
        assert!(Storage::validate_tag("web-server_01").is_ok());
        assert!(Storage::validate_tag("ABC-123_xyz").is_ok());
        let exact_32 = "a".repeat(32);
        assert!(Storage::validate_tag(&exact_32).is_ok());
    }

    #[test]
    fn validate_tag_rejects_empty() {
        assert!(matches!(
            Storage::validate_tag(""),
            Err(SshManagerError::EmptyTag)
        ));
    }

    #[test]
    fn validate_tag_rejects_too_long() {
        let too_long = "a".repeat(33);
        assert!(matches!(
            Storage::validate_tag(&too_long),
            Err(SshManagerError::TagTooLong(_))
        ));
    }

    #[test]
    fn validate_tag_rejects_invalid_chars() {
        for bad in [
            "has space",
            "tag@1",
            "tag.dot",
            "tag:colon",
            "tag#hash",
            "tag!",
        ] {
            assert!(
                matches!(
                    Storage::validate_tag(bad),
                    Err(SshManagerError::InvalidTag(_))
                ),
                "expected '{bad}' to be rejected"
            );
        }
    }

    #[test]
    fn normalize_tags_lowercases_deduplicates_and_preserves_order() {
        let input = vec![
            "Web".to_string(),
            "PROD".to_string(),
            "web".to_string(),
            "DB-01".to_string(),
            "prod".to_string(),
            "stage_2".to_string(),
        ];
        let normalized = Storage::normalize_tags(&input).expect("valid tags");
        assert_eq!(
            normalized,
            vec![
                "web".to_string(),
                "prod".to_string(),
                "db-01".to_string(),
                "stage_2".to_string()
            ]
        );
    }

    #[test]
    fn normalize_tags_rejects_invalid() {
        let input = vec!["valid".to_string(), "invalid tag".to_string()];
        assert!(matches!(
            Storage::normalize_tags(&input),
            Err(SshManagerError::InvalidTag(_))
        ));
    }

    #[test]
    fn server_name_is_unique_basic() {
        let mut cfg = Config::new();
        cfg.servers.push(sample_server("a"));
        assert!(Storage::server_name_is_unique(&cfg, "b", None));
        assert!(!Storage::server_name_is_unique(&cfg, "a", None));
    }

    #[test]
    fn server_name_is_unique_with_excluding() {
        let mut cfg = Config::new();
        cfg.servers.push(sample_server("a"));
        assert!(Storage::server_name_is_unique(&cfg, "a", Some("a")));
        assert!(!Storage::server_name_is_unique(&cfg, "a", Some("b")));
    }

    #[test]
    fn round_trip_preserves_config() {
        let _scope = ScopedConfigDir::new();
        let mut cfg = Config::new();
        cfg.servers.push(sample_server("alpha"));
        cfg.servers.push(sample_server("beta"));
        cfg.preferred_terminal = Some("xterm".to_string());

        Storage::save_config(&cfg).expect("save");
        let loaded = Storage::load_config().expect("load");

        assert_eq!(loaded.servers.len(), 2);
        assert_eq!(loaded.servers[0].name, "alpha");
        assert_eq!(loaded.servers[1].name, "beta");
        assert_eq!(loaded.preferred_terminal.as_deref(), Some("xterm"));
    }

    #[test]
    fn legacy_bare_array_format_still_loads() {
        let _scope = ScopedConfigDir::new();
        let servers_file = Storage::get_servers_file().expect("servers path");
        // Pre-v0.1.1 servers.json was a bare array.
        let legacy_json = r#"[{"name":"a","host":"example.com","username":"alice","port":22,"auth_method":"key","use_agent_forwarding":false}]"#;
        fs::write(&servers_file, legacy_json).unwrap();

        let loaded = Storage::load_config().expect("load");
        assert_eq!(loaded.servers.len(), 1);
        assert_eq!(loaded.servers[0].name, "a");
        assert!(loaded.servers[0].tags.is_empty());
    }

    #[test]
    fn legacy_object_format_without_tags_still_loads() {
        let _scope = ScopedConfigDir::new();
        let servers_file = Storage::get_servers_file().expect("servers path");
        let legacy_json = r#"{"servers":[{"name":"srv1","host":"srv1.com","username":"bob","port":22,"auth_method":"password","use_agent_forwarding":false}],"preferred_terminal":"xterm"}"#;
        fs::write(&servers_file, legacy_json).unwrap();

        let loaded = Storage::load_config().expect("load");
        assert_eq!(loaded.servers.len(), 1);
        assert_eq!(loaded.servers[0].name, "srv1");
        assert!(loaded.servers[0].tags.is_empty());
        assert_eq!(loaded.preferred_terminal.as_deref(), Some("xterm"));
    }

    #[test]
    fn load_missing_file_returns_empty_config() {
        let _scope = ScopedConfigDir::new();
        let cfg = Storage::load_config().expect("load");
        assert!(cfg.servers.is_empty());
        assert!(cfg.preferred_terminal.is_none());
    }

    #[test]
    fn legacy_terminal_file_is_migrated_and_removed() {
        let _scope = ScopedConfigDir::new();
        let servers_file = Storage::get_servers_file().expect("servers path");
        let legacy = Storage::get_terminal_config_file().expect("legacy path");

        let cfg = Config {
            servers: vec![sample_server("a")],
            preferred_terminal: None,
        };
        fs::write(&servers_file, serde_json::to_string_pretty(&cfg).unwrap()).unwrap();
        fs::write(
            &legacy,
            serde_json::to_string_pretty(&TerminalConfig {
                terminal: "gnome-terminal".to_string(),
            })
            .unwrap(),
        )
        .unwrap();

        let loaded = Storage::load_config().expect("load");
        assert_eq!(loaded.preferred_terminal.as_deref(), Some("gnome-terminal"));
        assert!(
            !legacy.exists(),
            "legacy file should be removed after migration"
        );
    }

    #[test]
    fn validate_jump_host_accepts_valid() {
        assert!(Storage::validate_jump_host("bastion").is_ok());
        assert!(Storage::validate_jump_host("jump-1.prod").is_ok());
        assert!(Storage::validate_jump_host("user@jump.example.com").is_ok());
        assert!(Storage::validate_jump_host("user@10.0.0.1:2222").is_ok());
        assert!(Storage::validate_jump_host("jump.example.com:22").is_ok());
    }

    #[test]
    fn validate_jump_host_rejects_empty_and_whitespace() {
        assert!(matches!(
            Storage::validate_jump_host(""),
            Err(SshManagerError::EmptyJumpHost)
        ));
        assert!(matches!(
            Storage::validate_jump_host("   "),
            Err(SshManagerError::InvalidJumpHost(_))
        ));
        assert!(matches!(
            Storage::validate_jump_host("bastion host"),
            Err(SshManagerError::InvalidJumpHost(_))
        ));
    }

    #[test]
    fn validate_jump_host_rejects_shell_metacharacters() {
        for ch in [
            ';', '&', '|', '`', '$', '\\', '"', '\'', '*', '?', '[', ']', '(', ')', '{', '}', '!',
            '^', '~',
        ] {
            let spec = format!("bastion{}host", ch);
            assert!(
                matches!(
                    Storage::validate_jump_host(&spec),
                    Err(SshManagerError::InvalidJumpHost(_))
                ),
                "should reject metacharacter '{}'",
                ch
            );
        }
    }

    #[test]
    fn validate_server_jump_rejects_self_reference() {
        let cfg = Config::new();
        assert!(matches!(
            Storage::validate_server_jump(&cfg, "myserver", "myserver"),
            Err(SshManagerError::SelfReferentialJumpHost(_))
        ));
    }

    #[test]
    fn validate_server_jump_rejects_cycle() {
        let mut cfg = Config::new();
        let mut b = sample_server("B");
        b.jump_host = Some("A".to_string());
        cfg.servers.push(b);

        let res = Storage::validate_server_jump(&cfg, "A", "B");
        assert!(matches!(res, Err(SshManagerError::JumpHostCycle(_))));
    }

    #[test]
    fn validate_server_removal_refuses_when_dependents_exist() {
        let mut cfg = Config::new();
        let mut s1 = sample_server("s1");
        s1.jump_host = Some("bastion".to_string());
        let mut s2 = sample_server("s2");
        s2.jump_host = Some("bastion".to_string());
        cfg.servers.push(s1);
        cfg.servers.push(s2);

        let err = Storage::validate_server_removal(&cfg, "bastion").unwrap_err();
        assert!(matches!(
            err,
            SshManagerError::ServerReferencedAsJumpHost(name, deps)
                if name == "bastion" && deps == "s1, s2"
        ));

        // When no dependents, removal validation succeeds
        assert!(Storage::validate_server_removal(&cfg, "s1").is_ok());
    }

    #[test]
    fn execute_server_edit_rename_updates_jump_host_dependents() {
        let _scope = ScopedConfigDir::new();
        let mut cfg = Config::new();
        let s_jump = sample_server("bastion");
        let mut s_dep = sample_server("app");
        s_dep.jump_host = Some("bastion".to_string());
        cfg.servers.push(s_jump);
        cfg.servers.push(s_dep);
        Storage::save_config(&cfg).expect("save");

        let edit = ServerEdit {
            new_name: Some("bastion-v2".to_string()),
            ..Default::default()
        };
        Storage::execute_server_edit(&mut cfg, "bastion", edit, None).expect("edit");

        let loaded = Storage::load_config().expect("load");
        let app = loaded.find_server("app").expect("app");
        assert_eq!(app.jump_host.as_deref(), Some("bastion-v2"));
    }
}
