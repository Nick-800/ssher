mod error;
mod models;
mod ssh;
mod storage;
mod terminal;
mod tui;

use clap::{Parser, Subcommand};
use error::Result;
use models::{AuthMethod, Server};
use ssh::Ssh;
use std::io::{self, Write};
use storage::Storage;
use terminal::Terminal;

#[derive(Parser)]
#[command(name = "ssher")]
#[command(about = "Manage and launch SSH connections to saved servers", long_about = None)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new SSH server
    Add {
        /// Server name (identifier)
        name: String,
        /// Server host (IP or domain)
        host: String,
        /// SSH username
        username: String,
        /// SSH port (default: 22)
        #[arg(short, long, default_value = "22")]
        port: u16,
        /// SSH key file path (for key-based authentication)
        #[arg(short, long)]
        key: Option<String>,
        /// Use password authentication (will prompt for password)
        #[arg(long)]
        password: bool,
        /// Enable SSH agent forwarding
        #[arg(short, long)]
        agent_forwarding: bool,
    },

    /// List all saved SSH servers
    List,

    /// Connect to a saved SSH server
    Connect {
        /// Server name to connect to
        name: String,
    },

    /// Remove a saved SSH server
    Remove {
        /// Server name to remove
        name: String,
    },

    /// Configure SSH manager settings
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Set the preferred terminal emulator
    Terminal {
        /// Terminal emulator name (gnome-terminal, xterm, etc.)
        name: String,
    },
}

fn main() {
    // If no arguments provided, launch interactive TUI
    let args: Vec<String> = std::env::args().collect();
    
    if args.len() == 1 {
        // No arguments, launch TUI
        let mut ui = tui::Tui::new();
        if let Err(e) = ui.run() {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    } else {
        // Parse CLI arguments
        let cli = Cli::parse();

        if let Err(e) = run(cli) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Add {
            name,
            host,
            username,
            port,
            key,
            password,
            agent_forwarding,
        } => {
            add_server(name, host, username, port, key, password, agent_forwarding)
        }
        Commands::List => list_servers(),
        Commands::Connect { name } => connect_server(&name),
        Commands::Remove { name } => remove_server(&name),
        Commands::Config {
            action: ConfigAction::Terminal { name },
        } => set_terminal_config(&name),
    }
}

fn add_server(
    name: String,
    host: String,
    username: String,
    port: u16,
    key: Option<String>,
    use_password: bool,
    agent_forwarding: bool,
) -> Result<()> {
    // Validate inputs
    Storage::validate_server_name(&name)?;
    Storage::validate_host(&host)?;
    Storage::validate_username(&username)?;
    Storage::validate_port(port)?;

    let mut config = Storage::load_config()?;

    // Check if server already exists
    if !Storage::server_name_is_unique(&config, &name, None) {
        return Err(error::SshManagerError::ServerAlreadyExists(name));
    }

    // Determine authentication method
    let auth_method;
    let key_path;

    if let Some(key_file) = key {
        // Key-based authentication
        Storage::validate_key_path(&key_file)?;
        auth_method = AuthMethod::Key;
        key_path = Some(key_file);
    } else if use_password {
        // Password-based authentication
        print!("Enter password for {}@{}: ", username, host);
        io::stdout().flush()?;

        let password = rpassword::read_password().map_err(|e| {
            error::SshManagerError::InvalidConfig(format!("Failed to read password: {}", e))
        })?;

        if password.is_empty() {
            return Err(error::SshManagerError::InvalidConfig(
                "Password cannot be empty".to_string(),
            ));
        }

        auth_method = AuthMethod::Password;
        key_path = None;

        // Store password in keyring
        Ssh::store_password(&name, &password)?;
    } else {
        return Err(error::SshManagerError::InvalidConfig(
            "Must specify either --key or --password".to_string(),
        ));
    }

    let mut server = Server::new(name.clone(), host, username, port, auth_method, key_path);
    server.use_agent_forwarding = agent_forwarding;

    config.servers.push(server);
    Storage::save_config(&config)?;

    println!("Server '{}' added successfully!", name);
    Ok(())
}

fn list_servers() -> Result<()> {
    let config = Storage::load_config()?;

    if config.servers.is_empty() {
        println!("No servers configured.");
        return Ok(());
    }

    println!("{:<20} {:<20} {:<15} {:<10} {:<12} {:<8}", "Name", "Host", "Username", "Port", "Auth", "Agent FW");
    println!("{}", "─".repeat(95));

    for server in &config.servers {
        let auth = match server.auth_method {
            AuthMethod::Key => "key",
            AuthMethod::Password => "password",
        };
        let agent_fw = if server.use_agent_forwarding { "yes" } else { "no" };

        println!(
            "{:<20} {:<20} {:<15} {:<10} {:<12} {:<8}",
            server.name, server.host, server.username, server.port, auth, agent_fw
        );
    }

    Ok(())
}

fn connect_server(name: &str) -> Result<()> {
    let config = Storage::load_config()?;

    let server = config
        .find_server(name)
        .ok_or_else(|| error::SshManagerError::ServerNotFound(name.to_string()))?;

    // Get terminal to use
    let terminal_name = Terminal::get_terminal_command(config.preferred_terminal.as_deref())?;

    println!("Connecting to {}...", name);
    Ssh::launch_connection(server, &terminal_name)?;

    Ok(())
}

fn remove_server(name: &str) -> Result<()> {
    let mut config = Storage::load_config()?;

    // Check if server exists
    if !config.servers.iter().any(|s| s.name == name) {
        return Err(error::SshManagerError::ServerNotFound(name.to_string()));
    }

    // Remove server
    config.servers.retain(|s| s.name != name);
    Storage::save_config(&config)?;

    // Clean up password from keyring if it exists
    let _ = Ssh::delete_password(name);

    println!("Server '{}' removed successfully!", name);
    Ok(())
}

fn set_terminal_config(terminal_name: &str) -> Result<()> {
    // Validate that terminal exists
    if !Terminal::get_terminal_command(Some(terminal_name)).is_ok() {
        return Err(error::SshManagerError::TerminalNotFound);
    }

    let mut config = Storage::load_config()?;
    config.preferred_terminal = Some(terminal_name.to_string());
    Storage::save_config(&config)?;

    println!("Terminal preference set to '{}'", terminal_name);
    Ok(())
}
