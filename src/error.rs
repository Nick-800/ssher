use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SshManagerError {
    #[error("Server not found: {0}")]
    ServerNotFound(String),

    #[error("Server already exists: {0}")]
    ServerAlreadyExists(String),

    #[error("SSH key file not found: {0}")]
    KeyFileNotFound(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("IO error: {0}")]
    IoError(#[from] io::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Keyring error: {0}")]
    KeyringError(String),

    #[error("Terminal not found. Please install gnome-terminal or xterm")]
    TerminalNotFound,

    #[error("Failed to launch SSH connection: {0}")]
    SshLaunchFailed(String),

    #[error("Configuration directory error: {0}")]
    ConfigDirError(String),

    #[error("Username cannot be empty")]
    EmptyUsername,

    #[error("Host cannot be empty")]
    EmptyHost,

    #[error("Server name cannot be empty")]
    EmptyServerName,

    #[error("Server name contains invalid characters")]
    InvalidServerName,

    #[error("Port must be between 1 and 65535")]
    InvalidPort,

    #[error("Password not found in keyring for server: {0}")]
    PasswordNotInKeyring(String),

    #[error("sshpass is not installed. Install it to use password authentication")]
    SshpassNotInstalled,
}

pub type Result<T> = std::result::Result<T, SshManagerError>;
