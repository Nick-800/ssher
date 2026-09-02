use crate::error::{Result, SshManagerError};
use crate::models::{AuthMethod, Server};
use crate::terminal::Terminal;
use keyring::Entry;
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

pub struct Ssh;

impl Ssh {
    const KEYRING_SERVICE: &'static str = "ssh_manager";

    /// Build the argv for `ssh` itself, NOT including any password helper.
    /// The caller is responsible for configuring `SSH_ASKPASS` etc. if the
    /// server uses password authentication.
    pub fn build_ssh_args(server: &Server) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();

        if let Some(key_path) = &server.key_path {
            args.push("-i".to_string());
            args.push(key_path.clone());
        }

        if server.use_agent_forwarding {
            args.push("-A".to_string());
        }

        if server.port != 22 {
            args.push("-p".to_string());
            args.push(server.port.to_string());
        }

        args.push(format!("{}@{}", server.username, server.host));
        args
    }

    /// Store password in OS keyring
    pub fn store_password(server_name: &str, password: &str) -> Result<()> {
        let entry = Self::keyring_entry(server_name)?;
        entry
            .set_password(password)
            .map_err(|e| SshManagerError::KeyringError(e.to_string()))?;
        Ok(())
    }

    /// Retrieve password from OS keyring
    pub fn get_password(server_name: &str) -> Result<String> {
        let entry = Self::keyring_entry(server_name)?;
        entry
            .get_password()
            .map_err(|_| SshManagerError::PasswordNotInKeyring(server_name.to_string()))
    }

    /// Delete password from keyring (no-op if missing)
    pub fn delete_password(server_name: &str) -> Result<()> {
        let entry = Self::keyring_entry(server_name)?;
        let _ = entry.delete_password();
        Ok(())
    }

    fn keyring_entry(server_name: &str) -> Result<Entry> {
        Entry::new(
            Self::KEYRING_SERVICE,
            &format!("{}_ssh_password", server_name),
        )
        .map_err(|e| SshManagerError::KeyringError(e.to_string()))
    }

    /// Launch SSH connection in a terminal. Passwords are passed via
    /// `SSH_ASKPASS`, never as command-line arguments.
    pub fn launch_connection(server: &Server, terminal: &str) -> Result<()> {
        let ssh_args = Self::build_ssh_args(server);
        let mut env: Vec<(String, String)> = Vec::new();
        let mut askpass_dir: Option<PathBuf> = None;

        if server.auth_method == AuthMethod::Password {
            let password = Self::get_password(&server.name)?;
            let dir = setup_askpass(&password)?;
            let script = dir.join("askpass.sh");
            env.push((
                "SSH_ASKPASS".to_string(),
                script.to_string_lossy().into_owned(),
            ));
            env.push(("SSH_ASKPASS_REQUIRE".to_string(), "force".to_string()));
            askpass_dir = Some(dir);
        }

        // Best-effort cleanup of the askpass directory after a short grace
        // period (ssh should have already read the password by the time the
        // terminal window is up).
        if let Some(dir) = askpass_dir.as_ref() {
            let dir_clone = dir.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(5));
                let _ = fs::remove_dir_all(&dir_clone);
            });
        }

        let arg_refs: Vec<&str> = ssh_args.iter().map(String::as_str).collect();
        let env_refs: Vec<(&str, &str)> =
            env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        Terminal::spawn_terminal(terminal, &arg_refs, &env_refs)?;
        Ok(())
    }
}

/// Set up a temporary directory containing a shell helper script and a
/// password file, returning the directory path. The caller should set
/// `SSH_ASKPASS=<dir>/askpass.sh` and `SSH_ASKPASS_REQUIRE=force` on the
/// environment of the spawned ssh process.
fn setup_askpass(password: &str) -> Result<PathBuf> {
    let tmp = env::temp_dir().join(format!("ssher-askpass-{}", std::process::id()));
    fs::create_dir_all(&tmp)
        .map_err(|e| SshManagerError::AskspassSetupFailed(format!("create askpass dir: {}", e)))?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o700))
        .map_err(|e| SshManagerError::AskspassSetupFailed(format!("chmod askpass dir: {}", e)))?;

    let pwd_file = tmp.join("password");
    fs::write(&pwd_file, password)
        .map_err(|e| SshManagerError::AskspassSetupFailed(format!("write password: {}", e)))?;
    fs::set_permissions(&pwd_file, fs::Permissions::from_mode(0o600))
        .map_err(|e| SshManagerError::AskspassSetupFailed(format!("chmod password: {}", e)))?;

    let script = tmp.join("askpass.sh");
    fs::write(&script, "#!/bin/sh\ncat \"$(dirname \"$0\")/password\"\n")
        .map_err(|e| SshManagerError::AskspassSetupFailed(format!("write askpass: {}", e)))?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700))
        .map_err(|e| SshManagerError::AskspassSetupFailed(format!("chmod askpass: {}", e)))?;

    Ok(tmp)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_server() -> Server {
        Server::new(
            "alpha".to_string(),
            "example.com".to_string(),
            "alice".to_string(),
            22,
            AuthMethod::Key,
            Some("/home/alice/.ssh/id_ed25519".to_string()),
        )
    }

    fn password_server() -> Server {
        Server::new(
            "beta".to_string(),
            "example.com".to_string(),
            "bob".to_string(),
            2222,
            AuthMethod::Password,
            None,
        )
    }

    fn forward_server() -> Server {
        let mut s = key_server();
        s.use_agent_forwarding = true;
        s
    }

    #[test]
    fn build_ssh_args_key_default_port() {
        let args = Ssh::build_ssh_args(&key_server());
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "alice@example.com".to_string(),
            ]
        );
    }

    #[test]
    fn build_ssh_args_password_no_key_no_port_when_default() {
        // password_server above uses port 2222; reset to default to test
        // the "no -p when default" branch.
        let mut s = password_server();
        s.port = 22;
        let args = Ssh::build_ssh_args(&s);
        assert_eq!(args, vec!["bob@example.com".to_string()]);
    }

    #[test]
    fn build_ssh_args_custom_port() {
        let args = Ssh::build_ssh_args(&password_server());
        assert_eq!(
            args,
            vec![
                "-p".to_string(),
                "2222".to_string(),
                "bob@example.com".to_string(),
            ]
        );
    }

    #[test]
    fn build_ssh_args_agent_forwarding() {
        let args = Ssh::build_ssh_args(&forward_server());
        assert!(args.contains(&"-A".to_string()));
        assert!(args.contains(&"-i".to_string()));
    }

    #[test]
    fn build_ssh_args_password_never_present() {
        // Regression: the password must never leak into the argv.
        let mut s = password_server();
        s.key_path = Some("/should-not-see".to_string());
        let args = Ssh::build_ssh_args(&s);
        let joined = args.join("\n");
        assert!(!joined.contains("hunter2"));
        assert!(!joined.contains("'"));
    }

    #[test]
    fn keyring_entry_uses_server_name() {
        let entry = Ssh::keyring_entry("alpha").expect("entry");
        // Just check the service constant and that the entry can be created.
        assert_eq!(Ssh::KEYRING_SERVICE, "ssh_manager");
        // Touching the entry's internal get_password should yield a
        // NoEntry error in a clean test env, not something else.
        let result = entry.get_password();
        assert!(result.is_err());
    }
}
