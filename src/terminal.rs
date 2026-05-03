use crate::error::{Result, SshManagerError};
use std::process::Command;

pub struct Terminal;

impl Terminal {
    /// Get the terminal command to use, respecting user preference
    pub fn get_terminal_command(preferred: Option<&str>) -> Result<String> {
        // If user has a preference, try to use it
        if let Some(pref) = preferred {
            if Self::terminal_exists(pref) {
                return Ok(pref.to_string());
            }
        }

        // Try default order: gnome-terminal, then xterm
        let terminals = vec!["gnome-terminal", "xterm"];

        for terminal in terminals {
            if Self::terminal_exists(terminal) {
                return Ok(terminal.to_string());
            }
        }

        Err(SshManagerError::TerminalNotFound)
    }

    /// Check if a terminal emulator exists in PATH
    fn terminal_exists(terminal: &str) -> bool {
        Command::new("which")
            .arg(terminal)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    /// Spawn a terminal with an SSH command
    pub fn spawn_terminal(terminal: &str, ssh_command: &str) -> Result<()> {
        match terminal {
            "gnome-terminal" => {
                Command::new("gnome-terminal")
                    .arg("--")
                    .arg("bash")
                    .arg("-c")
                    .arg(&format!("{}; read -p 'Press Enter to close...'", ssh_command))
                    .spawn()
                    .map_err(|e| {
                        SshManagerError::SshLaunchFailed(format!(
                            "Failed to launch gnome-terminal: {}",
                            e
                        ))
                    })?;
            }
            "xterm" => {
                Command::new("xterm")
                    .arg("-e")
                    .arg("bash")
                    .arg("-c")
                    .arg(&format!("{}; read -p 'Press Enter to close...'", ssh_command))
                    .spawn()
                    .map_err(|e| {
                        SshManagerError::SshLaunchFailed(format!(
                            "Failed to launch xterm: {}",
                            e
                        ))
                    })?;
            }
            _ => {
                return Err(SshManagerError::TerminalNotFound);
            }
        }

        Ok(())
    }
}
