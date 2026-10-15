use crate::error::{Result, SshManagerError};
use crate::models::{AuthMethod, Forward, Server};
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
    pub fn build_ssh_args(
        server: &Server,
        jump: Option<&str>,
        forwards: &[Forward],
    ) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();

        if let Some(key_path) = &server.key_path {
            args.push("-i".to_string());
            args.push(key_path.clone());
        }

        if server.use_agent_forwarding {
            args.push("-A".to_string());
        }

        if let Some(jump_spec) = jump {
            args.push("-J".to_string());
            args.push(jump_spec.to_string());
        }

        if server.port != 22 {
            args.push("-p".to_string());
            args.push(server.port.to_string());
        }

        for forward in forwards {
            match forward {
                Forward::Local {
                    bind_port,
                    host,
                    host_port,
                } => {
                    args.push("-L".to_string());
                    args.push(format!("{}:{}:{}", bind_port, host, host_port));
                }
                Forward::Remote {
                    bind_port,
                    host,
                    host_port,
                } => {
                    args.push("-R".to_string());
                    args.push(format!("{}:{}:{}", bind_port, host, host_port));
                }
                Forward::Dynamic { bind_port } => {
                    args.push("-D".to_string());
                    args.push(bind_port.to_string());
                }
            }
        }

        args.push(format!("{}@{}", server.username, server.host));
        args
    }

    /// Build the full argv for spawning `ssh`, with `"ssh"` as argv[0].
    pub fn build_ssh_argv(
        server: &Server,
        jump: Option<&str>,
        forwards: &[Forward],
    ) -> Vec<String> {
        let mut argv = vec!["ssh".to_string()];
        argv.extend(Self::build_ssh_args(server, jump, forwards));
        argv
    }

    /// Build the argv for non-interactive `ssh` remote command execution.
    /// Forwards (-L/-R/-D) are omitted. BatchMode is enabled ONLY for key-auth
    /// servers when `batch_mode` is true.
    pub fn build_exec_args(
        server: &Server,
        jump: Option<&str>,
        batch_mode: bool,
        command: &str,
    ) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();

        if let Some(key_path) = &server.key_path {
            args.push("-i".to_string());
            args.push(key_path.clone());
        }

        if server.use_agent_forwarding {
            args.push("-A".to_string());
        }

        if let Some(jump_spec) = jump {
            args.push("-J".to_string());
            args.push(jump_spec.to_string());
        }

        if server.port != 22 {
            args.push("-p".to_string());
            args.push(server.port.to_string());
        }

        if batch_mode && server.auth_method == AuthMethod::Key {
            args.push("-o".to_string());
            args.push("BatchMode=yes".to_string());
        }

        args.push(format!("{}@{}", server.username, server.host));
        args.push("--".to_string());
        args.push(command.to_string());
        args
    }

    /// Build the full argv for spawning `ssh` exec, with `"ssh"` as argv[0].
    pub fn build_exec_argv(
        server: &Server,
        jump: Option<&str>,
        batch_mode: bool,
        command: &str,
    ) -> Vec<String> {
        let mut argv = vec!["ssh".to_string()];
        argv.extend(Self::build_exec_args(server, jump, batch_mode, command));
        argv
    }

    /// Build the argv for `ssh-copy-id`, NOT including `ssh-copy-id` as argv[0].
    pub fn build_copy_id_args(server: &Server, jump: Option<&str>, pub_path: &str) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();

        args.push("-i".to_string());
        args.push(pub_path.to_string());

        // Do NOT add `-o IdentityFile=<saved key>` here: ssh-copy-id's
        // "already installed?" probe reuses these options, so the saved key
        // would log in and every new key would be skipped as a duplicate.
        // Key-auth servers authenticate through ssh-agent / default identities.
        if let Some(jump_spec) = jump {
            args.push("-o".to_string());
            args.push(format!("ProxyJump={}", jump_spec));
        }

        if server.port != 22 {
            args.push("-p".to_string());
            args.push(server.port.to_string());
        }

        args.push(format!("{}@{}", server.username, server.host));
        args
    }

    /// Build the full argv for spawning `ssh-copy-id`, with `"ssh-copy-id"` as argv[0].
    pub fn build_copy_id_argv(server: &Server, jump: Option<&str>, pub_path: &str) -> Vec<String> {
        let mut argv = vec!["ssh-copy-id".to_string()];
        argv.extend(Self::build_copy_id_args(server, jump, pub_path));
        argv
    }

    /// Normalize uncommon local TERM values (e.g. `xterm-kitty`, `alacritty`)
    /// to `xterm-256color` so remote servers lacking specialized terminfo databases
    /// do not produce "unknown terminal type" errors.
    pub fn normalize_term(term: &str) -> Option<&'static str> {
        if term.contains("kitty") || term == "alacritty" {
            Some("xterm-256color")
        } else {
            None
        }
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
    pub fn launch_connection(
        server: &Server,
        terminal: &str,
        jump: Option<&str>,
        forwards: &[Forward],
    ) -> Result<()> {
        let ssh_argv = Self::build_ssh_argv(server, jump, forwards);
        let mut env: Vec<(String, String)> = Vec::new();
        let mut askpass_dir: Option<PathBuf> = None;

        if let Ok(current_term) = env::var("TERM") {
            if let Some(normalized) = Self::normalize_term(&current_term) {
                env.push(("TERM".to_string(), normalized.to_string()));
            }
        }

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

        if Terminal::is_current_terminal(terminal) {
            let mut cmd = std::process::Command::new(&ssh_argv[0]);
            cmd.args(&ssh_argv[1..]);
            for (k, v) in &env {
                cmd.env(k, v);
            }
            let status = cmd.status().map_err(|e| {
                SshManagerError::SshLaunchFailed(format!(
                    "Failed to execute {}: {}",
                    ssh_argv[0], e
                ))
            })?;
            if let Some(dir) = askpass_dir {
                let _ = fs::remove_dir_all(dir);
            }
            if !status.success() {
                if let Some(code) = status.code() {
                    eprintln!("\n[ssh exited with status {}]", code);
                } else {
                    eprintln!("\n[ssh terminated by signal]");
                }
            }
            return Ok(());
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

        let arg_refs: Vec<&str> = ssh_argv.iter().map(String::as_str).collect();
        let env_refs: Vec<(&str, &str)> =
            env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        Terminal::spawn_terminal(terminal, &arg_refs, &env_refs)?;
        Ok(())
    }

    /// Execute a remote command inline in the current terminal, returning ssh's exit status.
    /// Password authentication reuses the temporary SSH_ASKPASS helper.
    pub fn execute_command(
        server: &Server,
        jump: Option<&str>,
        batch_mode: bool,
        command: &str,
    ) -> Result<std::process::ExitStatus> {
        let ssh_argv = Self::build_exec_argv(server, jump, batch_mode, command);
        let mut env: Vec<(String, String)> = Vec::new();
        let mut askpass_dir: Option<PathBuf> = None;

        if let Ok(current_term) = env::var("TERM") {
            if let Some(normalized) = Self::normalize_term(&current_term) {
                env.push(("TERM".to_string(), normalized.to_string()));
            }
        }

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

        let mut cmd = std::process::Command::new(&ssh_argv[0]);
        cmd.args(&ssh_argv[1..]);
        for (k, v) in &env {
            cmd.env(k, v);
        }

        let status_res = cmd.status();

        if let Some(dir) = askpass_dir {
            let _ = fs::remove_dir_all(dir);
        }

        let status = status_res.map_err(|e| {
            SshManagerError::SshLaunchFailed(format!("Failed to execute {}: {}", ssh_argv[0], e))
        })?;

        Ok(status)
    }

    /// Check if `ssh-copy-id` is installed on PATH.
    pub fn copy_id_exists() -> bool {
        crate::terminal::Terminal::command_exists("ssh-copy-id")
    }

    /// Execute `ssh-copy-id` in the current terminal with inherited stdio.
    /// Password authentication reuses the temporary SSH_ASKPASS helper.
    pub fn execute_copy_id(
        server: &Server,
        jump: Option<&str>,
        pub_path: &str,
    ) -> Result<std::process::ExitStatus> {
        if !Self::copy_id_exists() {
            return Err(SshManagerError::SshCopyIdNotFound);
        }

        let copy_id_argv = Self::build_copy_id_argv(server, jump, pub_path);
        let mut env: Vec<(String, String)> = Vec::new();
        let mut askpass_dir: Option<PathBuf> = None;

        if let Ok(current_term) = env::var("TERM") {
            if let Some(normalized) = Self::normalize_term(&current_term) {
                env.push(("TERM".to_string(), normalized.to_string()));
            }
        }

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

        let mut cmd = std::process::Command::new(&copy_id_argv[0]);
        cmd.args(&copy_id_argv[1..]);
        for (k, v) in &env {
            cmd.env(k, v);
        }

        let status_res = cmd.status();

        if let Some(dir) = askpass_dir {
            let _ = fs::remove_dir_all(dir);
        }

        let status = status_res.map_err(|e| {
            SshManagerError::SshLaunchFailed(format!(
                "Failed to execute {}: {}",
                copy_id_argv[0], e
            ))
        })?;

        Ok(status)
    }

    /// Format a remote target path for scp (`user@host:path`), bracketing IPv6 hosts.
    pub fn format_remote_target(username: &str, host: &str, path: &str) -> String {
        let bracketed_host =
            if host.contains(':') && !(host.starts_with('[') && host.ends_with(']')) {
                format!("[{}]", host)
            } else {
                host.to_string()
            };
        format!("{}@{}:{}", username, bracketed_host, path)
    }

    /// Protect a local path that starts with `-` by prefixing with `./`.
    pub fn protect_local_path(path: &str) -> String {
        if path.starts_with('-') && !path.starts_with("./") {
            format!("./{}", path)
        } else {
            path.to_string()
        }
    }

    /// Build the argv for `scp` upload (put), NOT including `scp` as argv[0].
    pub fn build_put_args<S: AsRef<str>>(
        server: &Server,
        jump: Option<&str>,
        recursive: bool,
        local_sources: &[S],
        remote_dest: &str,
    ) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();

        if recursive {
            args.push("-r".to_string());
        }

        if server.port != 22 {
            args.push("-P".to_string());
            args.push(server.port.to_string());
        }

        if let Some(key_path) = &server.key_path {
            args.push("-i".to_string());
            args.push(key_path.clone());
        }

        if let Some(jump_spec) = jump {
            args.push("-J".to_string());
            args.push(jump_spec.to_string());
        }

        args.push("--".to_string());

        for src in local_sources {
            args.push(Self::protect_local_path(src.as_ref()));
        }

        args.push(Self::format_remote_target(
            &server.username,
            &server.host,
            remote_dest,
        ));

        args
    }

    /// Build the full argv for spawning `scp` upload (put), with `"scp"` as argv[0].
    pub fn build_put_argv<S: AsRef<str>>(
        server: &Server,
        jump: Option<&str>,
        recursive: bool,
        local_sources: &[S],
        remote_dest: &str,
    ) -> Vec<String> {
        let mut argv = vec!["scp".to_string()];
        argv.extend(Self::build_put_args(
            server,
            jump,
            recursive,
            local_sources,
            remote_dest,
        ));
        argv
    }

    /// Build the argv for `scp` download (get), NOT including `scp` as argv[0].
    pub fn build_get_args<S: AsRef<str>>(
        server: &Server,
        jump: Option<&str>,
        recursive: bool,
        remote_sources: &[S],
        local_dest: &str,
    ) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();

        if recursive {
            args.push("-r".to_string());
        }

        if server.port != 22 {
            args.push("-P".to_string());
            args.push(server.port.to_string());
        }

        if let Some(key_path) = &server.key_path {
            args.push("-i".to_string());
            args.push(key_path.clone());
        }

        if let Some(jump_spec) = jump {
            args.push("-J".to_string());
            args.push(jump_spec.to_string());
        }

        args.push("--".to_string());

        for src in remote_sources {
            args.push(Self::format_remote_target(
                &server.username,
                &server.host,
                src.as_ref(),
            ));
        }

        args.push(Self::protect_local_path(local_dest));

        args
    }

    /// Build the full argv for spawning `scp` download (get), with `"scp"` as argv[0].
    pub fn build_get_argv<S: AsRef<str>>(
        server: &Server,
        jump: Option<&str>,
        recursive: bool,
        remote_sources: &[S],
        local_dest: &str,
    ) -> Vec<String> {
        let mut argv = vec!["scp".to_string()];
        argv.extend(Self::build_get_args(
            server,
            jump,
            recursive,
            remote_sources,
            local_dest,
        ));
        argv
    }

    /// Check if `scp` is installed on PATH.
    pub fn scp_exists() -> bool {
        crate::terminal::Terminal::command_exists("scp")
    }

    /// Execute `scp` in the current terminal with inherited stdio.
    /// Password authentication reuses the temporary SSH_ASKPASS helper.
    pub fn execute_scp(server: &Server, argv: &[String]) -> Result<std::process::ExitStatus> {
        if !Self::scp_exists() {
            return Err(SshManagerError::ScpNotFound);
        }

        let mut env: Vec<(String, String)> = Vec::new();
        let mut askpass_dir: Option<PathBuf> = None;

        if let Ok(current_term) = env::var("TERM") {
            if let Some(normalized) = Self::normalize_term(&current_term) {
                env.push(("TERM".to_string(), normalized.to_string()));
            }
        }

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

        let mut cmd = std::process::Command::new(&argv[0]);
        cmd.args(&argv[1..]);
        for (k, v) in &env {
            cmd.env(k, v);
        }

        let status_res = cmd.status();

        if let Some(dir) = askpass_dir {
            let _ = fs::remove_dir_all(dir);
        }

        let status = status_res.map_err(|e| {
            SshManagerError::SshLaunchFailed(format!("Failed to execute {}: {}", argv[0], e))
        })?;

        Ok(status)
    }

    /// Execute `scp` put in the current terminal with inherited stdio.
    pub fn execute_put<S: AsRef<str>>(
        server: &Server,
        jump: Option<&str>,
        recursive: bool,
        local_sources: &[S],
        remote_dest: &str,
    ) -> Result<std::process::ExitStatus> {
        let argv = Self::build_put_argv(server, jump, recursive, local_sources, remote_dest);
        Self::execute_scp(server, &argv)
    }

    /// Execute `scp` get in the current terminal with inherited stdio.
    pub fn execute_get<S: AsRef<str>>(
        server: &Server,
        jump: Option<&str>,
        recursive: bool,
        remote_sources: &[S],
        local_dest: &str,
    ) -> Result<std::process::ExitStatus> {
        let argv = Self::build_get_argv(server, jump, recursive, remote_sources, local_dest);
        Self::execute_scp(server, &argv)
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
        let args = Ssh::build_ssh_args(&key_server(), None, &[]);
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
        let args = Ssh::build_ssh_args(&s, None, &[]);
        assert_eq!(args, vec!["bob@example.com".to_string()]);
    }

    #[test]
    fn build_ssh_args_custom_port() {
        let args = Ssh::build_ssh_args(&password_server(), None, &[]);
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
        let args = Ssh::build_ssh_args(&forward_server(), None, &[]);
        assert!(args.contains(&"-A".to_string()));
        assert!(args.contains(&"-i".to_string()));
    }

    #[test]
    fn build_ssh_argv_starts_with_ssh() {
        let argv = Ssh::build_ssh_argv(&forward_server(), None, &[]);
        assert_eq!(argv.first().map(String::as_str), Some("ssh"));
        assert!(argv.contains(&"-A".to_string()));
    }

    #[test]
    fn build_ssh_args_password_never_present() {
        // Regression: the password must never leak into the argv.
        let mut s = password_server();
        s.key_path = Some("/should-not-see".to_string());
        let args = Ssh::build_ssh_args(&s, None, &[]);
        let joined = args.join("\n");
        assert!(!joined.contains("hunter2"));
        assert!(!joined.contains("'"));
    }

    #[test]
    fn build_ssh_args_with_jump_host() {
        let args = Ssh::build_ssh_args(&key_server(), Some("bastion.example.com"), &[]);
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "-J".to_string(),
                "bastion.example.com".to_string(),
                "alice@example.com".to_string(),
            ]
        );
    }

    #[test]
    fn build_ssh_argv_with_jump_host() {
        let argv = Ssh::build_ssh_argv(&forward_server(), Some("jump1,jump2"), &[]);
        assert_eq!(argv.first().map(String::as_str), Some("ssh"));
        assert!(argv.contains(&"-J".to_string()));
        assert!(argv.contains(&"jump1,jump2".to_string()));
        assert!(argv.contains(&"-A".to_string()));
    }

    #[test]
    fn build_ssh_args_with_local_forward() {
        let f = Forward::Local {
            bind_port: 5432,
            host: "localhost".to_string(),
            host_port: 5432,
        };
        let args = Ssh::build_ssh_args(&key_server(), None, &[f]);
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "-L".to_string(),
                "5432:localhost:5432".to_string(),
                "alice@example.com".to_string(),
            ]
        );
    }

    #[test]
    fn build_ssh_args_with_remote_and_dynamic_forwards() {
        let forwards = vec![
            Forward::Remote {
                bind_port: 2222,
                host: "internal.box".to_string(),
                host_port: 22,
            },
            Forward::Dynamic { bind_port: 1080 },
        ];
        let args = Ssh::build_ssh_args(&key_server(), None, &forwards);
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "-R".to_string(),
                "2222:internal.box:22".to_string(),
                "-D".to_string(),
                "1080".to_string(),
                "alice@example.com".to_string(),
            ]
        );
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

    #[test]
    fn normalize_term_replaces_kitty_and_alacritty() {
        assert_eq!(Ssh::normalize_term("xterm-kitty"), Some("xterm-256color"));
        assert_eq!(Ssh::normalize_term("kitty"), Some("xterm-256color"));
        assert_eq!(Ssh::normalize_term("alacritty"), Some("xterm-256color"));
        assert_eq!(Ssh::normalize_term("xterm-256color"), None);
        assert_eq!(Ssh::normalize_term("linux"), None);
    }

    #[test]
    fn build_exec_args_key_server() {
        let args = Ssh::build_exec_args(&key_server(), None, false, "uptime");
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "alice@example.com".to_string(),
                "--".to_string(),
                "uptime".to_string(),
            ]
        );
    }

    #[test]
    fn build_exec_args_key_server_batch_mode() {
        let args = Ssh::build_exec_args(&key_server(), None, true, "uptime");
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "-o".to_string(),
                "BatchMode=yes".to_string(),
                "alice@example.com".to_string(),
                "--".to_string(),
                "uptime".to_string(),
            ]
        );
    }

    #[test]
    fn build_exec_args_password_server() {
        let args = Ssh::build_exec_args(&password_server(), None, false, "uname -a");
        assert_eq!(
            args,
            vec![
                "-p".to_string(),
                "2222".to_string(),
                "bob@example.com".to_string(),
                "--".to_string(),
                "uname -a".to_string(),
            ]
        );
        let joined = args.join("\n");
        assert!(!joined.contains("password"));
    }

    #[test]
    fn build_exec_args_password_server_batch_mode_ignored() {
        // Even when batch_mode is true, BatchMode=yes must NOT be added for password servers.
        let args = Ssh::build_exec_args(&password_server(), None, true, "uname -a");
        assert!(!args.contains(&"-o".to_string()));
        assert!(!args.contains(&"BatchMode=yes".to_string()));
    }

    #[test]
    fn build_exec_args_with_jump_host() {
        let args =
            Ssh::build_exec_args(&key_server(), Some("jump.example.com"), false, "echo hello");
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "-J".to_string(),
                "jump.example.com".to_string(),
                "alice@example.com".to_string(),
                "--".to_string(),
                "echo hello".to_string(),
            ]
        );
    }

    #[test]
    fn build_exec_args_omits_forwards() {
        let mut s = key_server();
        s.forwards = vec![
            Forward::Local {
                bind_port: 8080,
                host: "localhost".to_string(),
                host_port: 80,
            },
            Forward::Remote {
                bind_port: 2222,
                host: "remote.host".to_string(),
                host_port: 22,
            },
            Forward::Dynamic { bind_port: 1080 },
        ];
        let args = Ssh::build_exec_args(&s, None, false, "df -h");
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "alice@example.com".to_string(),
                "--".to_string(),
                "df -h".to_string(),
            ]
        );
        assert!(!args.contains(&"-L".to_string()));
        assert!(!args.contains(&"-R".to_string()));
        assert!(!args.contains(&"-D".to_string()));
    }

    #[test]
    fn build_exec_argv_starts_with_ssh() {
        let argv = Ssh::build_exec_argv(&key_server(), None, false, "hostname");
        assert_eq!(argv[0], "ssh");
        assert_eq!(argv.last().map(String::as_str), Some("hostname"));
    }

    #[test]
    fn build_copy_id_args_default_port() {
        let args = Ssh::build_copy_id_args(&key_server(), None, "/home/alice/.ssh/id_ed25519.pub");
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519.pub".to_string(),
                "alice@example.com".to_string(),
            ]
        );
    }

    #[test]
    fn build_copy_id_args_custom_port() {
        let args = Ssh::build_copy_id_args(&password_server(), None, "/home/bob/.ssh/id_rsa.pub");
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/bob/.ssh/id_rsa.pub".to_string(),
                "-p".to_string(),
                "2222".to_string(),
                "bob@example.com".to_string(),
            ]
        );
    }

    #[test]
    fn build_copy_id_args_with_jump_host() {
        let args = Ssh::build_copy_id_args(
            &key_server(),
            Some("bastion.example.com"),
            "/home/alice/.ssh/id_ed25519.pub",
        );
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519.pub".to_string(),
                "-o".to_string(),
                "ProxyJump=bastion.example.com".to_string(),
                "alice@example.com".to_string(),
            ]
        );
    }

    #[test]
    fn build_copy_id_args_with_chained_jump_host_and_custom_port() {
        let args = Ssh::build_copy_id_args(
            &password_server(),
            Some("jump1@box:22,jump2@box:22"),
            "/home/bob/.ssh/id_rsa.pub",
        );
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/bob/.ssh/id_rsa.pub".to_string(),
                "-o".to_string(),
                "ProxyJump=jump1@box:22,jump2@box:22".to_string(),
                "-p".to_string(),
                "2222".to_string(),
                "bob@example.com".to_string(),
            ]
        );
    }

    #[test]
    fn build_copy_id_argv_starts_with_ssh_copy_id() {
        let argv = Ssh::build_copy_id_argv(&key_server(), None, "/home/alice/.ssh/id_ed25519.pub");
        assert_eq!(argv[0], "ssh-copy-id");
        assert_eq!(argv[1], "-i");
        assert_eq!(argv[2], "/home/alice/.ssh/id_ed25519.pub");
        assert_eq!(argv.last().map(String::as_str), Some("alice@example.com"));
    }

    #[test]
    fn build_copy_id_args_password_never_present() {
        let s = password_server();
        let args = Ssh::build_copy_id_args(&s, None, "/home/bob/.ssh/id_rsa.pub");
        let joined = args.join("\n");
        assert!(!joined.contains("password"));
        assert!(!joined.contains("hunter2"));
    }

    #[test]
    fn format_remote_target_standard_host() {
        assert_eq!(
            Ssh::format_remote_target("alice", "example.com", "/var/www"),
            "alice@example.com:/var/www"
        );
        assert_eq!(
            Ssh::format_remote_target("bob", "192.168.1.10", "file.txt"),
            "bob@192.168.1.10:file.txt"
        );
    }

    #[test]
    fn format_remote_target_ipv6_host_brackets() {
        assert_eq!(
            Ssh::format_remote_target("alice", "2001:db8::1", "/data"),
            "alice@[2001:db8::1]:/data"
        );
        assert_eq!(
            Ssh::format_remote_target("alice", "::1", "file.txt"),
            "alice@[::1]:file.txt"
        );
        assert_eq!(
            Ssh::format_remote_target("alice", "[2001:db8::1]", "/data"),
            "alice@[2001:db8::1]:/data"
        );
    }

    #[test]
    fn protect_local_path_dash_leading() {
        assert_eq!(Ssh::protect_local_path("-foo.txt"), "./-foo.txt");
        assert_eq!(Ssh::protect_local_path("-r"), "./-r");
        assert_eq!(Ssh::protect_local_path("--"), "./--");
        assert_eq!(Ssh::protect_local_path("-"), "./-");
        assert_eq!(Ssh::protect_local_path("./-foo.txt"), "./-foo.txt");
        assert_eq!(Ssh::protect_local_path("/path/to/-file"), "/path/to/-file");
        assert_eq!(Ssh::protect_local_path("normal.txt"), "normal.txt");
    }

    #[test]
    fn build_put_args_key_server() {
        let args = Ssh::build_put_args(&key_server(), None, false, &["local.txt"], "/remote/dir");
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "--".to_string(),
                "local.txt".to_string(),
                "alice@example.com:/remote/dir".to_string(),
            ]
        );
    }

    #[test]
    fn build_get_args_key_server() {
        let args = Ssh::build_get_args(&key_server(), None, false, &["/remote/file.txt"], ".");
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "--".to_string(),
                "alice@example.com:/remote/file.txt".to_string(),
                ".".to_string(),
            ]
        );
    }

    #[test]
    fn build_put_args_password_server() {
        let args = Ssh::build_put_args(
            &password_server(),
            None,
            false,
            &["local.txt"],
            "/remote/dir",
        );
        assert_eq!(
            args,
            vec![
                "-P".to_string(),
                "2222".to_string(),
                "--".to_string(),
                "local.txt".to_string(),
                "bob@example.com:/remote/dir".to_string(),
            ]
        );
        let joined = args.join("\n");
        assert!(!joined.contains("password"));
        assert!(!joined.contains("hunter2"));
    }

    #[test]
    fn build_get_args_password_server() {
        let args = Ssh::build_get_args(
            &password_server(),
            None,
            false,
            &["/remote/file.txt"],
            "./downloads",
        );
        assert_eq!(
            args,
            vec![
                "-P".to_string(),
                "2222".to_string(),
                "--".to_string(),
                "bob@example.com:/remote/file.txt".to_string(),
                "./downloads".to_string(),
            ]
        );
        let joined = args.join("\n");
        assert!(!joined.contains("password"));
        assert!(!joined.contains("hunter2"));
    }

    #[test]
    fn build_put_args_with_jump_host() {
        let args = Ssh::build_put_args(
            &key_server(),
            Some("bastion.example.com"),
            false,
            &["local.txt"],
            "/remote/dir",
        );
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "-J".to_string(),
                "bastion.example.com".to_string(),
                "--".to_string(),
                "local.txt".to_string(),
                "alice@example.com:/remote/dir".to_string(),
            ]
        );
    }

    #[test]
    fn build_get_args_with_jump_host() {
        let args = Ssh::build_get_args(
            &key_server(),
            Some("bastion.example.com"),
            false,
            &["/remote/file.txt"],
            "local.txt",
        );
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "-J".to_string(),
                "bastion.example.com".to_string(),
                "--".to_string(),
                "alice@example.com:/remote/file.txt".to_string(),
                "local.txt".to_string(),
            ]
        );
    }

    #[test]
    fn build_put_args_custom_port() {
        let mut server = key_server();
        server.port = 8022;
        let args = Ssh::build_put_args(&server, None, false, &["file.tar.gz"], "/tmp");
        assert_eq!(
            args,
            vec![
                "-P".to_string(),
                "8022".to_string(),
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "--".to_string(),
                "file.tar.gz".to_string(),
                "alice@example.com:/tmp".to_string(),
            ]
        );
    }

    #[test]
    fn build_get_args_custom_port() {
        let mut server = key_server();
        server.port = 8022;
        let args = Ssh::build_get_args(&server, None, false, &["/tmp/file.tar.gz"], ".");
        assert_eq!(
            args,
            vec![
                "-P".to_string(),
                "8022".to_string(),
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "--".to_string(),
                "alice@example.com:/tmp/file.tar.gz".to_string(),
                ".".to_string(),
            ]
        );
    }

    #[test]
    fn build_put_args_recursive() {
        let args = Ssh::build_put_args(&key_server(), None, true, &["./folder"], "/remote/dir");
        assert_eq!(
            args,
            vec![
                "-r".to_string(),
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "--".to_string(),
                "./folder".to_string(),
                "alice@example.com:/remote/dir".to_string(),
            ]
        );
    }

    #[test]
    fn build_get_args_recursive() {
        let args = Ssh::build_get_args(&key_server(), None, true, &["/remote/dir"], "./dest");
        assert_eq!(
            args,
            vec![
                "-r".to_string(),
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "--".to_string(),
                "alice@example.com:/remote/dir".to_string(),
                "./dest".to_string(),
            ]
        );
    }

    #[test]
    fn build_put_args_ipv6_host() {
        let mut server = key_server();
        server.host = "2001:db8::cafe".to_string();
        let args = Ssh::build_put_args(&server, None, false, &["file.txt"], "/opt");
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "--".to_string(),
                "file.txt".to_string(),
                "alice@[2001:db8::cafe]:/opt".to_string(),
            ]
        );

        server.host = "[2001:db8::cafe]".to_string();
        let args2 = Ssh::build_put_args(&server, None, false, &["file.txt"], "/opt");
        assert_eq!(args2.last().unwrap(), "alice@[2001:db8::cafe]:/opt");
    }

    #[test]
    fn build_get_args_ipv6_host() {
        let mut server = key_server();
        server.host = "::1".to_string();
        let args = Ssh::build_get_args(&server, None, false, &["/var/log/syslog"], ".");
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "--".to_string(),
                "alice@[::1]:/var/log/syslog".to_string(),
                ".".to_string(),
            ]
        );
    }

    #[test]
    fn build_put_args_dash_leading_local_path() {
        let args = Ssh::build_put_args(
            &key_server(),
            None,
            false,
            &["-dangerous-file.txt", "normal.txt"],
            "/remote/dir",
        );
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "--".to_string(),
                "./-dangerous-file.txt".to_string(),
                "normal.txt".to_string(),
                "alice@example.com:/remote/dir".to_string(),
            ]
        );
    }

    #[test]
    fn build_get_args_dash_leading_local_path() {
        let args = Ssh::build_get_args(
            &key_server(),
            None,
            false,
            &["/remote/file.txt"],
            "-local-dest",
        );
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "--".to_string(),
                "alice@example.com:/remote/file.txt".to_string(),
                "./-local-dest".to_string(),
            ]
        );
    }

    #[test]
    fn build_put_args_multiple_sources() {
        let args = Ssh::build_put_args(
            &key_server(),
            None,
            false,
            &["src1.txt", "src2.txt", "src3.txt"],
            "/remote/dest",
        );
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "--".to_string(),
                "src1.txt".to_string(),
                "src2.txt".to_string(),
                "src3.txt".to_string(),
                "alice@example.com:/remote/dest".to_string(),
            ]
        );
    }

    #[test]
    fn build_get_args_multiple_sources() {
        let args = Ssh::build_get_args(
            &key_server(),
            None,
            false,
            &["/remote/1", "/remote/2"],
            "./dest",
        );
        assert_eq!(
            args,
            vec![
                "-i".to_string(),
                "/home/alice/.ssh/id_ed25519".to_string(),
                "--".to_string(),
                "alice@example.com:/remote/1".to_string(),
                "alice@example.com:/remote/2".to_string(),
                "./dest".to_string(),
            ]
        );
    }

    #[test]
    fn build_put_argv_starts_with_scp() {
        let argv = Ssh::build_put_argv(&key_server(), None, false, &["file.txt"], "/tmp");
        assert_eq!(argv[0], "scp");
        assert_eq!(argv[1], "-i");
        assert_eq!(argv.last().unwrap(), "alice@example.com:/tmp");
    }

    #[test]
    fn build_get_argv_starts_with_scp() {
        let argv = Ssh::build_get_argv(&key_server(), None, false, &["/tmp/file.txt"], ".");
        assert_eq!(argv[0], "scp");
        assert_eq!(argv[1], "-i");
        assert_eq!(argv.last().unwrap(), ".");
    }
}
