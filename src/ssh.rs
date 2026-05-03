use crate::error::{Result, SshManagerError};
use crate::models::{AuthMethod, Server};
use crate::terminal::Terminal;
use keyring::Entry;
use std::process::Command;

pub struct Ssh;

impl Ssh {
    const KEYRING_SERVICE: &'static str = "ssh_manager";

    /// Build the SSH command for a server
    fn build_ssh_command(server: &Server, password: Option<&str>) -> Result<String> {
        let mut cmd = if let Some(pwd) = password {
            // Validate sshpass is installed
            if !Self::sshpass_exists() {
                return Err(SshManagerError::SshpassNotInstalled);
            }
            format!("sshpass -p '{}' ssh", pwd.replace("'", "'\\''"))
        } else {
            "ssh".to_string()
        };

        // Add identity file for key-based auth
        if let Some(key_path) = &server.key_path {
            cmd.push_str(&format!(" -i '{}'", key_path.replace("'", "'\\''"))); 
        }

        // Add agent forwarding if enabled
        if server.use_agent_forwarding {
            cmd.push_str(" -A");
        }

        // Add port if not default
        if server.port != 22 {
            cmd.push_str(&format!(" -p {}", server.port));
        }

        // Add user and host
        cmd.push_str(&format!(
            " '{}@{}'",
            server.username.replace("'", "'\\''"),
            server.host.replace("'", "'\\''")
        ));

        Ok(cmd)
    }

    /// Store password in OS keyring
    pub fn store_password(server_name: &str, password: &str) -> Result<()> {
        let entry = Entry::new(Self::KEYRING_SERVICE, &format!("{}_ssh_password", server_name))
            .map_err(|e| SshManagerError::KeyringError(e.to_string()))?;

        entry
            .set_password(password)
            .map_err(|e| SshManagerError::KeyringError(e.to_string()))?;

        Ok(())
    }

    /// Retrieve password from OS keyring
    fn get_password(server_name: &str) -> Result<String> {
        let entry = Entry::new(Self::KEYRING_SERVICE, &format!("{}_ssh_password", server_name))
            .map_err(|e| SshManagerError::KeyringError(e.to_string()))?;

        entry
            .get_password()
            .map_err(|_| SshManagerError::PasswordNotInKeyring(server_name.to_string()))
    }

    /// Delete password from keyring
    pub fn delete_password(server_name: &str) -> Result<()> {
        let entry = Entry::new(Self::KEYRING_SERVICE, &format!("{}_ssh_password", server_name))
            .map_err(|e| SshManagerError::KeyringError(e.to_string()))?;

        // Ignore error if password doesn't exist
        let _ = entry.delete_password();

        Ok(())
    }

    /// Launch SSH connection in a terminal
    pub fn launch_connection(server: &Server, terminal: &str) -> Result<()> {
        // Get password if using password auth
        let password = if server.auth_method == AuthMethod::Password {
            Some(Self::get_password(&server.name)?)
        } else {
            None
        };

        // Build SSH command
        let ssh_command = Self::build_ssh_command(server, password.as_deref())?;

        // Spawn terminal with SSH command
        Terminal::spawn_terminal(terminal, &ssh_command)?;

        Ok(())
    }

    /// Check if sshpass is installed
    fn sshpass_exists() -> bool {
        Command::new("which")
            .arg("sshpass")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }
}
