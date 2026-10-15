mod error;
mod models;
mod ssh;
mod storage;
mod terminal;
mod tui;

use clap::{CommandFactory, Parser, Subcommand};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Default)]
pub enum ListSort {
    #[default]
    Recent,
    Name,
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
        /// Server tags (repeatable)
        #[arg(long = "tag", action = clap::ArgAction::Append)]
        tag: Vec<String>,
        /// SSH jump host (saved server name or [user@]host[:port])
        #[arg(long = "jump")]
        jump: Option<String>,
        /// Port forwarding preset (repeatable, e.g. 5432, L:8080:localhost:80, R:2222:host:22, D:1080)
        #[arg(long = "forward", action = clap::ArgAction::Append)]
        forward: Vec<String>,
        /// Default tmux session name to attach on connect
        #[arg(long = "tmux")]
        tmux: Option<String>,
    },

    /// Modify a saved SSH server
    Edit {
        /// Server name to edit
        name: String,
        /// New server host (IP or domain)
        #[arg(long)]
        host: Option<String>,
        /// New SSH username
        #[arg(long)]
        username: Option<String>,
        /// New SSH port
        #[arg(long)]
        port: Option<u16>,
        /// New SSH key file path
        #[arg(long, conflicts_with = "password")]
        key: Option<String>,
        /// Set password authentication (interactive prompt)
        #[arg(long, conflicts_with = "key")]
        password: bool,
        /// Enable or disable SSH agent forwarding
        #[arg(long, value_name = "true|false", action = clap::ArgAction::Set)]
        agent_forwarding: Option<bool>,
        /// New server name (identifier)
        #[arg(long)]
        new_name: Option<String>,
        /// Set server tags (repeatable, replaces current tags)
        #[arg(long = "tag", conflicts_with = "clear_tags", action = clap::ArgAction::Append)]
        tag: Option<Vec<String>>,
        /// Clear all tags from server
        #[arg(long, conflicts_with = "tag")]
        clear_tags: bool,
        /// SSH jump host (saved server name or [user@]host[:port])
        #[arg(long = "jump", conflicts_with = "clear_jump")]
        jump: Option<String>,
        /// Clear jump host from server
        #[arg(long, conflicts_with = "jump")]
        clear_jump: bool,
        /// Set port forwarding presets (repeatable, replaces current forwards)
        #[arg(long = "forward", conflicts_with = "clear_forwards", action = clap::ArgAction::Append)]
        forward: Option<Vec<String>>,
        /// Clear all port forwarding presets from server
        #[arg(long, conflicts_with = "forward")]
        clear_forwards: bool,
        /// Set default tmux session name to attach on connect
        #[arg(long = "tmux", conflicts_with = "clear_tmux")]
        tmux: Option<String>,
        /// Clear default tmux session name from server
        #[arg(long, conflicts_with = "tmux")]
        clear_tmux: bool,
    },

    /// List all saved SSH servers
    List {
        /// Filter servers by tag (repeatable, must have all tags)
        #[arg(long = "tag", action = clap::ArgAction::Append)]
        tag: Vec<String>,
        /// Sort order for listed servers (recent: favourites first, then most recently used; name: alphabetical)
        #[arg(long = "sort", default_value = "recent")]
        sort: ListSort,
    },

    /// Connect to a saved SSH server
    Connect {
        /// Server name to connect to
        name: String,
        /// Connect directly in current terminal instead of spawning an external window
        #[arg(short, long)]
        current: bool,
        /// Ad-hoc port forwarding (repeatable)
        #[arg(long = "forward", action = clap::ArgAction::Append)]
        forward: Vec<String>,
        /// Do not apply saved port forwarding presets
        #[arg(long = "no-forwards")]
        no_forwards: bool,
        /// Attach to a tmux session (defaults to "ssher")
        #[arg(
            long = "tmux",
            num_args = 0..=1,
            default_missing_value = "ssher",
            conflicts_with = "no_tmux"
        )]
        tmux: Option<String>,
        /// Do not attach to a tmux session
        #[arg(long = "no-tmux", conflicts_with = "tmux")]
        no_tmux: bool,
    },

    /// Execute a remote command inline in the current terminal
    Exec {
        /// Server name to execute command on
        #[arg(
            conflicts_with_all = ["tag", "all"],
            required_unless_present_any = ["tag", "all"]
        )]
        target: Option<String>,

        /// Filter servers by tag (repeatable, must have all tags)
        #[arg(long = "tag", conflicts_with = "all", action = clap::ArgAction::Append)]
        tag: Vec<String>,

        /// Run on all configured servers
        #[arg(long = "all", conflicts_with = "tag")]
        all: bool,

        /// Command to execute on the remote server
        #[arg(required = true, num_args = 1.., last = true)]
        command: Vec<String>,
    },

    /// Install public key on a saved SSH server using ssh-copy-id
    #[command(name = "copy-id")]
    CopyId {
        /// Server name to copy SSH key to
        name: String,

        /// Path to public key file (e.g. ~/.ssh/id_ed25519.pub)
        #[arg(long = "pub", conflicts_with = "key")]
        pub_path: Option<String>,

        /// Path to private or public key file (e.g. ~/.ssh/id_ed25519)
        #[arg(long = "key", conflicts_with = "pub_path")]
        key: Option<String>,

        /// Switch server to key authentication after successful copy
        #[arg(long = "switch-to-key")]
        switch_to_key: bool,
    },

    /// Upload files or directories to a remote server using scp
    Put {
        /// Server name to upload files to
        name: String,

        /// Copy directories recursively
        #[arg(short, long)]
        recursive: bool,

        /// Local source paths followed by remote destination directory
        #[arg(required = true, num_args = 2..)]
        paths: Vec<String>,
    },

    /// Download files or directories from a remote server using scp
    Get {
        /// Server name to download files from
        name: String,

        /// Copy directories recursively
        #[arg(short, long)]
        recursive: bool,

        /// Remote source paths followed by local destination directory
        #[arg(required = true, num_args = 2..)]
        paths: Vec<String>,
    },

    /// Remove a saved SSH server
    Remove {
        /// Server name to remove
        name: String,
    },

    /// Mark a saved SSH server as favorite
    Fav {
        /// Server name to favorite
        name: String,
    },

    /// Remove a saved SSH server from favorites
    Unfav {
        /// Server name to unfavorite
        name: String,
    },

    /// Import SSH servers from an ssh_config file
    Import {
        /// Path to SSH configuration file (default: ~/.ssh/config)
        #[arg(short, long)]
        file: Option<std::path::PathBuf>,
        /// Preview what would be imported without modifying configuration
        #[arg(long)]
        dry_run: bool,
        /// Skip hosts that collide with existing server names
        #[arg(long)]
        skip_existing: bool,
    },

    /// Export servers and configuration to JSON
    Export {
        /// File path to export to (writes to stdout if omitted)
        #[arg(long)]
        file: Option<std::path::PathBuf>,

        /// Filter servers to export by tag (repeatable, must have all tags)
        #[arg(long = "tag", action = clap::ArgAction::Append)]
        tag: Vec<String>,

        /// Overwrite destination file if it already exists
        #[arg(long)]
        force: bool,
    },

    /// Import servers from a backup JSON file
    #[command(name = "import-backup")]
    ImportBackup {
        /// Path to the backup JSON file
        file: std::path::PathBuf,

        /// Overwrite existing servers with matching names
        #[arg(long)]
        overwrite: bool,

        /// Preview what would be imported without modifying configuration
        #[arg(long)]
        dry_run: bool,
    },

    /// Generate shell completion scripts
    Completions {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
    },

    /// Check TCP reachability of saved servers
    Ping {
        /// Server name to ping
        #[arg(conflicts_with = "tag")]
        name: Option<String>,

        /// Filter servers by tag (repeatable, must have all tags)
        #[arg(long = "tag", conflicts_with = "name", action = clap::ArgAction::Append)]
        tag: Vec<String>,

        /// Connection timeout in seconds (default: 3)
        #[arg(long, default_value = "3")]
        timeout: u64,
    },

    #[command(name = "__complete-servers", hide = true)]
    CompleteServers,

    #[command(name = "__complete-tags", hide = true)]
    CompleteTags,

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
            tag,
            jump,
            forward,
            tmux,
        } => add_server(
            name,
            AddArgs {
                host,
                username,
                port,
                key,
                use_password: password,
                agent_forwarding,
                tags: tag,
                jump,
                forwards: forward,
                tmux,
            },
        ),
        Commands::Edit {
            name,
            host,
            username,
            port,
            key,
            password,
            agent_forwarding,
            new_name,
            tag,
            clear_tags,
            jump,
            clear_jump,
            forward,
            clear_forwards,
            tmux,
            clear_tmux,
        } => {
            let tags = if clear_tags { Some(Vec::new()) } else { tag };
            let jump_host = if clear_jump {
                Some(None)
            } else {
                jump.map(Some)
            };
            let forwards = if clear_forwards {
                Some(Vec::new())
            } else {
                forward
                    .map(|f| Storage::normalize_forwards(&f))
                    .transpose()?
            };
            let tmux = if clear_tmux {
                Some(None)
            } else {
                tmux.map(Some)
            };
            edit_server(
                name,
                EditArgs {
                    host,
                    username,
                    port,
                    key,
                    password,
                    agent_forwarding,
                    new_name,
                    tags,
                    jump_host,
                    forwards,
                    tmux,
                },
            )
        }
        Commands::List { tag, sort } => list_servers(&tag, sort),
        Commands::Connect {
            name,
            current,
            forward,
            no_forwards,
            tmux,
            no_tmux,
        } => connect_server(&name, current, &forward, no_forwards, tmux, no_tmux),
        Commands::Exec {
            target,
            tag,
            all,
            command,
        } => exec_servers(target, tag, all, command),
        Commands::CopyId {
            name,
            pub_path,
            key,
            switch_to_key,
        } => copy_id_server(
            &name,
            CopyIdArgs {
                explicit_key: pub_path.or(key),
                switch_to_key,
            },
        ),
        Commands::Put {
            name,
            recursive,
            paths,
        } => put_files(&name, recursive, &paths),
        Commands::Get {
            name,
            recursive,
            paths,
        } => get_files(&name, recursive, &paths),
        Commands::Remove { name } => remove_server(&name),
        Commands::Fav { name } => set_favorite(&name, true),
        Commands::Unfav { name } => set_favorite(&name, false),
        Commands::Import {
            file,
            dry_run,
            skip_existing,
        } => import_servers(ImportArgs {
            file,
            dry_run,
            skip_existing,
        }),
        Commands::Export { file, tag, force } => export_servers(file, tag, force),
        Commands::ImportBackup {
            file,
            overwrite,
            dry_run,
        } => import_backup(file, overwrite, dry_run),
        Commands::Completions { shell } => generate_completions(shell),
        Commands::Ping { name, tag, timeout } => ping_servers_cli(name.as_deref(), &tag, timeout),
        Commands::CompleteServers => complete_servers(),
        Commands::CompleteTags => complete_tags(),
        Commands::Config {
            action: ConfigAction::Terminal { name },
        } => set_terminal_config(&name),
    }
}

struct AddArgs {
    host: String,
    username: String,
    port: u16,
    key: Option<String>,
    use_password: bool,
    agent_forwarding: bool,
    tags: Vec<String>,
    jump: Option<String>,
    forwards: Vec<String>,
    tmux: Option<String>,
}

fn add_server(name: String, args: AddArgs) -> Result<()> {
    // Validate inputs
    Storage::validate_server_name(&name)?;
    Storage::validate_host(&args.host)?;
    Storage::validate_username(&args.username)?;
    Storage::validate_port(args.port)?;
    let normalized_tags = Storage::normalize_tags(&args.tags)?;
    let normalized_forwards = Storage::normalize_forwards(&args.forwards)?;
    if let Some(session) = &args.tmux {
        Storage::validate_tmux_session(session)?;
    }

    let mut config = Storage::load_config()?;

    // Check if server already exists
    if !Storage::server_name_is_unique(&config, &name, None) {
        return Err(error::SshManagerError::ServerAlreadyExists(name));
    }

    if let Some(jump) = &args.jump {
        Storage::validate_server_jump(&config, &name, jump)?;
    }

    // Determine authentication method
    let auth_method;
    let key_path;

    if let Some(key_file) = args.key {
        // Key-based authentication
        Storage::validate_key_path(&key_file)?;
        auth_method = AuthMethod::Key;
        key_path = Some(key_file);
    } else if args.use_password {
        // Password-based authentication
        print!("Enter password for {}@{}: ", args.username, args.host);
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

    let mut server = Server::new(
        name.clone(),
        args.host,
        args.username,
        args.port,
        auth_method,
        key_path,
    );
    server.use_agent_forwarding = args.agent_forwarding;
    server.tags = normalized_tags;
    server.jump_host = args.jump;
    server.forwards = normalized_forwards;
    server.tmux = args.tmux;

    config.servers.push(server);
    Storage::save_config(&config)?;

    println!("Server '{}' added successfully!", name);
    Ok(())
}

fn list_servers(filter_tags: &[String], sort_order: ListSort) -> Result<()> {
    let config = Storage::load_config()?;

    if config.servers.is_empty() {
        println!("No servers configured.");
        return Ok(());
    }

    for tag in filter_tags {
        Storage::validate_tag(tag)?;
    }

    let normalized_filters: Vec<String> = filter_tags.iter().map(|t| t.to_lowercase()).collect();

    let mut matching_servers: Vec<&Server> = config
        .servers
        .iter()
        .filter(|server| {
            normalized_filters
                .iter()
                .all(|ft| server.tags.iter().any(|st| st == ft))
        })
        .collect();

    if matching_servers.is_empty() {
        println!("No servers match the specified tag(s).");
        return Ok(());
    }

    match sort_order {
        ListSort::Recent => matching_servers.sort_by(|a, b| models::sort_servers(a, b)),
        ListSort::Name => matching_servers.sort_by(|a, b| a.name.cmp(&b.name)),
    }

    let now = models::current_time_epoch_secs();

    println!(
        "{:<20} {:<20} {:<15} {:<8} {:<10} {:<10} {:<16} {:<8} {:<10} {:<5} {:<12} {:<20}",
        "Name",
        "Host",
        "Username",
        "Port",
        "Auth",
        "Agent FW",
        "Via",
        "Fwd",
        "Tmux",
        "Fav",
        "Last used",
        "Tags"
    );
    println!("{}", "─".repeat(161));

    for server in matching_servers {
        let auth = match server.auth_method {
            AuthMethod::Key => "key",
            AuthMethod::Password => "password",
        };
        let agent_fw = if server.use_agent_forwarding {
            "yes"
        } else {
            "no"
        };
        let via_str = server.jump_host.as_deref().unwrap_or("-");
        let fwd_str = models::format_forwards_summary(&server.forwards);
        let tmux_str = server.tmux.as_deref().unwrap_or("-");
        let fav_str = if server.favorite { "*" } else { "-" };
        let last_used_str = models::format_last_used(server.last_used, now);
        let tags_str = if server.tags.is_empty() {
            "-".to_string()
        } else {
            server.tags.join(", ")
        };

        println!(
            "{:<20} {:<20} {:<15} {:<8} {:<10} {:<10} {:<16} {:<8} {:<10} {:<5} {:<12} {:<20}",
            server.name,
            server.host,
            server.username,
            server.port,
            auth,
            agent_fw,
            via_str,
            fwd_str,
            tmux_str,
            fav_str,
            last_used_str,
            tags_str
        );
    }

    Ok(())
}

fn set_favorite(name: &str, favorite: bool) -> Result<()> {
    let mut config = Storage::load_config()?;
    let server = config.resolve_server(name)?;
    let server_name = server.name.clone();

    if models::set_server_favorite(&mut config.servers, &server_name, favorite) {
        Storage::save_config(&config)?;
    }

    if favorite {
        println!("Server '{}' marked as favorite!", server_name);
    } else {
        println!("Server '{}' removed from favorites!", server_name);
    }
    Ok(())
}

struct EditArgs {
    host: Option<String>,
    username: Option<String>,
    port: Option<u16>,
    key: Option<String>,
    password: bool,
    agent_forwarding: Option<bool>,
    new_name: Option<String>,
    tags: Option<Vec<String>>,
    jump_host: Option<Option<String>>,
    forwards: Option<Vec<models::Forward>>,
    tmux: Option<Option<String>>,
}

fn edit_server(name: String, args: EditArgs) -> Result<()> {
    if args.host.is_none()
        && args.username.is_none()
        && args.port.is_none()
        && args.key.is_none()
        && !args.password
        && args.agent_forwarding.is_none()
        && args.new_name.is_none()
        && args.tags.is_none()
        && args.jump_host.is_none()
        && args.forwards.is_none()
        && args.tmux.is_none()
    {
        return Err(error::SshManagerError::NothingToChange);
    }

    let mut config = Storage::load_config()?;

    // Fail early if server doesn't exist
    let _ = config
        .find_server(&name)
        .ok_or_else(|| error::SshManagerError::ServerNotFound(name.clone()))?;

    let (auth, new_password) = if let Some(key_path) = args.key {
        (Some(models::AuthEdit::Key(key_path)), None)
    } else if args.password {
        let display_name = args.new_name.as_deref().unwrap_or(&name);
        print!("Enter password for server '{}': ", display_name);
        io::stdout().flush()?;
        let password = rpassword::read_password().map_err(|e| {
            error::SshManagerError::InvalidConfig(format!("Failed to read password: {}", e))
        })?;
        if password.is_empty() {
            return Err(error::SshManagerError::InvalidConfig(
                "Password cannot be empty".to_string(),
            ));
        }
        (Some(models::AuthEdit::Password), Some(password))
    } else {
        (None, None)
    };

    let edit = models::ServerEdit {
        new_name: args.new_name,
        host: args.host,
        username: args.username,
        port: args.port,
        auth,
        agent_forwarding: args.agent_forwarding,
        tags: args.tags,
        jump_host: args.jump_host,
        forwards: args.forwards,
        tmux: args.tmux,
    };

    Storage::execute_server_edit(&mut config, &name, edit, new_password)?;
    Ok(())
}

fn connect_server(
    name: &str,
    force_current: bool,
    adhoc_specs: &[String],
    no_forwards: bool,
    adhoc_tmux: Option<String>,
    no_tmux: bool,
) -> Result<()> {
    let config = Storage::load_config()?;

    let server = config.resolve_server(name)?;
    let jump = models::resolve_jump(&config, server)?;

    let adhoc_forwards = Storage::normalize_forwards(adhoc_specs)?;

    let mut effective_forwards = if no_forwards {
        Vec::new()
    } else {
        server.forwards.clone()
    };

    for f in adhoc_forwards {
        if !effective_forwards.contains(&f) {
            effective_forwards.push(f);
        }
    }

    let effective_tmux = if no_tmux {
        None
    } else {
        adhoc_tmux.as_deref().or(server.tmux.as_deref())
    };

    let remote_command = if let Some(session) = effective_tmux {
        Storage::validate_tmux_session(session)?;
        Some(Ssh::build_tmux_command(session))
    } else {
        None
    };

    // Get terminal to use
    let terminal_name = if force_current {
        "current".to_string()
    } else {
        Terminal::get_terminal_command(config.preferred_terminal.as_deref())?
    };

    println!("Connecting to {}...", server.name);
    let server_name = server.name.clone();
    Ssh::launch_connection(
        server,
        &terminal_name,
        jump.as_deref(),
        &effective_forwards,
        remote_command.as_deref(),
    )?;
    if let Err(e) = Storage::record_server_launch(&server_name) {
        eprintln!("Warning: failed to save last used timestamp: {}", e);
    }

    Ok(())
}

fn exec_servers(
    target: Option<String>,
    tags: Vec<String>,
    all: bool,
    command: Vec<String>,
) -> Result<()> {
    let config = Storage::load_config()?;
    let joined_cmd = command.join(" ");
    let is_multi = all || !tags.is_empty();

    if !is_multi {
        let target_name = target.expect("target must be present when not multi-target");
        let server = config.resolve_server(&target_name)?;
        let jump = models::resolve_jump(&config, server)?;
        let status = Ssh::execute_command(server, jump.as_deref(), false, &joined_cmd)?;
        if let Err(e) = Storage::record_server_launch(&server.name) {
            eprintln!("Warning: failed to save last used timestamp: {}", e);
        }
        if !status.success() {
            let code = status.code().unwrap_or_else(|| {
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    status.signal().map(|s| 128 + s).unwrap_or(1)
                }
                #[cfg(not(unix))]
                {
                    1
                }
            });
            std::process::exit(code);
        }
        Ok(())
    } else {
        let targets = models::select_exec_targets(&config.servers, None, &tags, all)?;
        let mut ok_count = 0;
        let mut failed_count = 0;

        for server in targets {
            println!("== {} ==", server.name);
            let _ = io::stdout().flush();

            let jump = match models::resolve_jump(&config, server) {
                Ok(j) => j,
                Err(e) => {
                    eprintln!("Error resolving jump host: {}", e);
                    failed_count += 1;
                    continue;
                }
            };

            match Ssh::execute_command(server, jump.as_deref(), true, &joined_cmd) {
                Ok(status) => {
                    if status.success() {
                        ok_count += 1;
                        if let Err(e) = Storage::record_server_launch(&server.name) {
                            eprintln!("Warning: failed to save last used timestamp: {}", e);
                        }
                    } else {
                        failed_count += 1;
                    }
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    failed_count += 1;
                }
            }
        }

        println!("ok {} / failed {}", ok_count, failed_count);
        if failed_count > 0 {
            std::process::exit(1);
        }
        Ok(())
    }
}

fn ping_servers_cli(name: Option<&str>, tags: &[String], timeout_secs: u64) -> Result<()> {
    if timeout_secs == 0 {
        return Err(error::SshManagerError::InvalidConfig(
            "Timeout must be greater than 0".to_string(),
        ));
    }

    let config = Storage::load_config()?;
    let targets = models::select_ping_targets(&config.servers, name, tags)?;
    let timeout = std::time::Duration::from_secs(timeout_secs);
    let results = models::ping_servers(&targets, timeout);

    let mut up_count = 0;
    let mut down_count = 0;

    for res in &results {
        if res.is_up() {
            up_count += 1;
        } else {
            down_count += 1;
        }
        println!(
            "{}",
            models::format_ping_result(&res.name, &res.host, res.port, &res.status)
        );
    }

    println!("{}", models::format_ping_summary(up_count, down_count));

    if down_count > 0 {
        std::process::exit(1);
    }

    Ok(())
}

struct CopyIdArgs {
    explicit_key: Option<String>,
    switch_to_key: bool,
}

fn copy_id_server(name: &str, args: CopyIdArgs) -> Result<()> {
    let mut config = Storage::load_config()?;
    let server = config.resolve_server(name)?;
    let server_name = server.name.clone();

    let raw_pub_path = models::derive_copy_id_pub_path(server, args.explicit_key.as_deref())?;
    let pub_path_buf = expand_tilde_path(std::path::Path::new(&raw_pub_path))?;
    let pub_path_str = pub_path_buf.to_string_lossy().into_owned();

    if !pub_path_buf.exists() || !pub_path_buf.is_file() {
        return Err(error::SshManagerError::KeyFileNotFound(pub_path_str));
    }

    let jump = models::resolve_jump(&config, server)?;

    let status = Ssh::execute_copy_id(server, jump.as_deref(), &pub_path_str)?;

    if !status.success() {
        let code = status.code().unwrap_or_else(|| {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                status.signal().map(|s| 128 + s).unwrap_or(1)
            }
            #[cfg(not(unix))]
            {
                1
            }
        });
        std::process::exit(code);
    }

    if args.switch_to_key {
        let idx = config
            .servers
            .iter()
            .position(|s| s.name == server_name)
            .ok_or_else(|| error::SshManagerError::ServerNotFound(server_name.clone()))?;

        let old_auth = config.servers[idx].auth_method.clone();
        let priv_path = models::switch_server_to_key(&mut config.servers[idx], &pub_path_str)?;

        Storage::save_config(&config)?;

        if old_auth == models::AuthMethod::Password {
            let _ = Ssh::delete_password(&server_name);
        }

        println!(
            "Server '{}' switched to key authentication ({})",
            server_name, priv_path
        );
    }

    Ok(())
}

fn put_files(name: &str, recursive: bool, paths: &[String]) -> Result<()> {
    let (sources, dest) = models::split_transfer_paths(paths)?;
    models::validate_local_sources(sources)?;

    let config = Storage::load_config()?;
    let server = config.resolve_server(name)?;
    let server_name = server.name.clone();
    let jump = models::resolve_jump(&config, server)?;

    let status = Ssh::execute_put(server, jump.as_deref(), recursive, sources, dest)?;

    if let Err(e) = Storage::record_server_launch(&server_name) {
        eprintln!("Warning: failed to save last used timestamp: {}", e);
    }

    if !status.success() {
        let code = status.code().unwrap_or_else(|| {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                status.signal().map(|s| 128 + s).unwrap_or(1)
            }
            #[cfg(not(unix))]
            {
                1
            }
        });
        std::process::exit(code);
    }

    Ok(())
}

fn get_files(name: &str, recursive: bool, paths: &[String]) -> Result<()> {
    let (sources, dest) = models::split_transfer_paths(paths)?;

    let config = Storage::load_config()?;
    let server = config.resolve_server(name)?;
    let server_name = server.name.clone();
    let jump = models::resolve_jump(&config, server)?;

    let status = Ssh::execute_get(server, jump.as_deref(), recursive, sources, dest)?;

    if let Err(e) = Storage::record_server_launch(&server_name) {
        eprintln!("Warning: failed to save last used timestamp: {}", e);
    }

    if !status.success() {
        let code = status.code().unwrap_or_else(|| {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                status.signal().map(|s| 128 + s).unwrap_or(1)
            }
            #[cfg(not(unix))]
            {
                1
            }
        });
        std::process::exit(code);
    }

    Ok(())
}

fn remove_server(name: &str) -> Result<()> {
    let mut config = Storage::load_config()?;

    // Check if server exists
    if !config.servers.iter().any(|s| s.name == name) {
        return Err(error::SshManagerError::ServerNotFound(name.to_string()));
    }

    Storage::validate_server_removal(&config, name)?;

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
    if Terminal::get_terminal_command(Some(terminal_name)).is_err() {
        return Err(error::SshManagerError::TerminalNotFound);
    }

    let mut config = Storage::load_config()?;
    config.preferred_terminal = Some(terminal_name.to_string());
    Storage::save_config(&config)?;

    println!("Terminal preference set to '{}'", terminal_name);
    Ok(())
}

struct ImportArgs {
    file: Option<std::path::PathBuf>,
    dry_run: bool,
    skip_existing: bool,
}

fn expand_tilde_path(p: &std::path::Path) -> Result<std::path::PathBuf> {
    models::expand_tilde_path(p)
}

fn import_servers(args: ImportArgs) -> Result<()> {
    let file_path = match args.file {
        Some(p) => expand_tilde_path(&p)?,
        None => {
            let home = dirs::home_dir().ok_or_else(|| {
                error::SshManagerError::ConfigDirError(
                    "Could not determine home directory. $HOME not set?".to_string(),
                )
            })?;
            home.join(".ssh").join("config")
        }
    };

    if !file_path.exists() {
        return Err(error::SshManagerError::InvalidConfig(format!(
            "SSH config file not found: {}",
            file_path.display()
        )));
    }

    let content = std::fs::read_to_string(&file_path)?;
    let home_dir = dirs::home_dir();
    let home_str = home_dir.as_ref().and_then(|p| p.to_str());

    let parse_res = models::parse_ssh_config_detailed(&content, home_str);

    for warn in &parse_res.warnings {
        eprintln!("Warning: {}", warn);
    }

    let mut config = Storage::load_config()?;
    let env_user = std::env::var("USER").ok();

    let map_res = models::filter_and_map_imported_hosts(
        &config,
        parse_res.hosts,
        env_user.as_deref(),
        |key_path| Storage::validate_key_path(key_path).is_ok(),
    );

    let mut all_skipped = parse_res.skipped;
    all_skipped.extend(map_res.skipped);

    if args.dry_run {
        println!("Dry run mode: no changes written.");
        for s in &map_res.to_import {
            println!(
                "Would import: '{}' ({}@{}:{}, key: {})",
                s.name,
                s.username,
                s.host,
                s.port,
                s.key_path.as_deref().unwrap_or("none")
            );
        }
        for (name, reason) in &all_skipped {
            println!("Would skip: '{}' ({})", name, reason);
        }
        println!(
            "Summary: {} would be imported, {} would be skipped",
            map_res.to_import.len(),
            all_skipped.len()
        );
        return Ok(());
    }

    for (name, reason) in &all_skipped {
        if args.skip_existing && reason == "Server already exists" {
            eprintln!(
                "Warning: skipping '{}' (--skip-existing): server already exists",
                name
            );
        } else {
            eprintln!("Warning: skipping '{}': {}", name, reason);
        }
    }

    let imported_count = map_res.to_import.len();
    config.servers.extend(map_res.to_import);
    Storage::save_config(&config)?;

    println!(
        "Summary: imported {}, skipped {}",
        imported_count,
        all_skipped.len()
    );
    if !all_skipped.is_empty() {
        println!("Skipped hosts:");
        for (name, reason) in &all_skipped {
            println!("  - {}: {}", name, reason);
        }
    }

    Ok(())
}

fn export_servers(file: Option<std::path::PathBuf>, tags: Vec<String>, force: bool) -> Result<()> {
    let config = Storage::load_config()?;
    let export_cfg = models::export_config(&config, &tags)?;
    let json = serde_json::to_string_pretty(&export_cfg)?;

    match file {
        Some(path) => {
            Storage::export_to_file(&path, &json, force)?;
            println!(
                "Exported {} server(s) to {}",
                export_cfg.servers.len(),
                path.display()
            );
        }
        None => {
            eprintln!("passwords are not exported; password-auth servers must be re-saved");
            let mut stdout = io::stdout();
            stdout.write_all(json.as_bytes())?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
        }
    }

    Ok(())
}

fn import_backup(file: std::path::PathBuf, overwrite: bool, dry_run: bool) -> Result<()> {
    let content = std::fs::read_to_string(&file)?;
    let backup_config = Storage::parse_servers_file(&content)?;

    let mut config = Storage::load_config()?;
    let had_password_auth: std::collections::HashSet<String> = config
        .servers
        .iter()
        .filter(|s| s.auth_method == AuthMethod::Password)
        .map(|s| s.name.clone())
        .collect();

    let result = models::import_backup_servers(
        &mut config,
        backup_config,
        overwrite,
        |key_path| Storage::validate_key_path(key_path).is_ok(),
        |name| had_password_auth.contains(name),
    );

    for warning in &result.warnings {
        eprintln!("Warning: {}", warning);
    }

    if !dry_run {
        Storage::save_config(&config)?;
    } else {
        println!("Dry run mode: no changes written to configuration.");
    }

    println!("{}", result.summary_text());
    Ok(())
}

pub fn generate_completion_script(shell: clap_complete::Shell) -> String {
    let mut cmd = Cli::command();
    let mut buf = Vec::new();
    clap_complete::generate(shell, &mut cmd, "ssher", &mut buf);
    let raw = String::from_utf8(buf).expect("completion script is valid UTF-8");

    match shell {
        clap_complete::Shell::Bash => {
            let bash_snippet = r#"
# Dynamic server completion for connect, edit, remove
_ssher_complete() {
    local cur="${2:-${COMP_WORDS[COMP_CWORD]}}"
    local prev="${3:-${COMP_WORDS[COMP_CWORD-1]}}"

    case "${prev}" in
        --host|--username|--port|--key|--agent-forwarding|--new-name|--forward|--pub|--tmux)
            _ssher "$@"
            return 0
            ;;
        --sort)
            COMPREPLY=( $(compgen -W "recent name" -- "${cur}") )
            return 0
            ;;
        --jump)
            local servers
            servers="$(ssher __complete-servers 2>/dev/null)"
            if [[ -n "${servers}" ]]; then
                COMPREPLY=( $(compgen -W "${servers}" -- "${cur}") )
                return 0
            fi
            ;;
        --tag)
            local tags
            tags="$(ssher __complete-tags 2>/dev/null)"
            if [[ -n "${tags}" ]]; then
                COMPREPLY=( $(compgen -W "${tags}" -- "${cur}") )
                return 0
            fi
            ;;
    esac

    local subcmd=""
    local subcmd_idx=0
    for (( i=1; i < COMP_CWORD; i++ )); do
        case "${COMP_WORDS[i]}" in
            connect|edit|remove|fav|unfav|exec|copy-id|put|get|ping)
                subcmd="${COMP_WORDS[i]}"
                subcmd_idx=$i
                break
                ;;
            add|list|import|completions|config|help|export|import-backup)
                break
                ;;
        esac
    done

    if [[ -n "${subcmd}" && "${cur}" != -* ]]; then
        local pos_count=0
        for (( i=subcmd_idx+1; i < COMP_CWORD; i++ )); do
            if [[ "${COMP_WORDS[i]}" != -* ]]; then
                ((pos_count++))
            fi
        done
        if [[ "${subcmd}" != "put" && "${subcmd}" != "get" ]] || [[ $pos_count -eq 0 ]]; then
            local servers
            servers="$(ssher __complete-servers 2>/dev/null)"
            if [[ -n "${servers}" ]]; then
                COMPREPLY=( $(compgen -W "${servers}" -- "${cur}") )
                return 0
            fi
        fi
    fi

    _ssher "$@"
}

if [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -ge 4 || "${BASH_VERSINFO[0]}" -gt 4 ]]; then
    complete -F _ssher_complete -o nosort -o bashdefault -o default ssher
else
    complete -F _ssher_complete -o bashdefault -o default ssher
fi
"#;
            format!("{raw}{bash_snippet}")
        }
        clap_complete::Shell::Fish => {
            let fish_snippet = r#"
# Dynamic server completion for connect, edit, remove, fav, unfav, exec, copy-id, put, get, ping
complete -c ssher -n "__fish_ssher_using_subcommand connect edit remove fav unfav exec copy-id put get ping" -f -a "(ssher __complete-servers)"
complete -c ssher -l tag -f -a "(ssher __complete-tags)"
complete -c ssher -l jump -f -a "(ssher __complete-servers)"
complete -c ssher -l sort -f -a "recent name"
"#;
            format!("{raw}{fish_snippet}")
        }
        clap_complete::Shell::Zsh => {
            let modified = raw
                .replace(
                    ":name -- Server name to connect to:_default",
                    ":name -- Server name to connect to:_ssher_servers",
                )
                .replace(
                    ":name -- Server name to edit:_default",
                    ":name -- Server name to edit:_ssher_servers",
                )
                .replace(
                    ":name -- Server name to remove:_default",
                    ":name -- Server name to remove:_ssher_servers",
                )
                .replace(
                    ":name -- Server name to favorite:_default",
                    ":name -- Server name to favorite:_ssher_servers",
                )
                .replace(
                    ":name -- Server name to unfavorite:_default",
                    ":name -- Server name to unfavorite:_ssher_servers",
                )
                .replace(
                    ":name -- Server name to copy SSH key to:_default",
                    ":name -- Server name to copy SSH key to:_ssher_servers",
                )
                .replace(
                    ":name -- Server name to upload files to:_default",
                    ":name -- Server name to upload files to:_ssher_servers",
                )
                .replace(
                    ":name -- Server name to download files from:_default",
                    ":name -- Server name to download files from:_ssher_servers",
                )
                .replace(
                    "::name -- Server name to ping:_default",
                    "::name -- Server name to ping:_ssher_servers",
                )
                .replace(
                    ":name -- Server name to ping:_default",
                    ":name -- Server name to ping:_ssher_servers",
                )
                .replace(":JUMP:_default", ":JUMP:_ssher_servers")
                .replace(
                    "::target -- Server name to execute command on:_default",
                    "::target -- Server name to execute command on:_ssher_servers",
                )
                .replace(
                    ":target -- Server name to execute command on:_default",
                    ":target -- Server name to execute command on:_ssher_servers",
                );
            let zsh_snippet = r#"
_ssher_servers() {
    local -a servers
    servers=(${(f)"$(ssher __complete-servers 2>/dev/null)"})
    _describe -t servers 'server' servers "$@"
}
_ssher_tags() {
    local -a tags
    tags=(${(f)"$(ssher __complete-tags 2>/dev/null)"})
    _describe -t tags 'tag' tags "$@"
}
"#;
            format!("{modified}{zsh_snippet}")
        }
        _ => raw,
    }
}

fn generate_completions(shell: clap_complete::Shell) -> Result<()> {
    let script = generate_completion_script(shell);
    io::stdout().write_all(script.as_bytes())?;
    Ok(())
}

fn complete_servers() -> Result<()> {
    if let Ok(config) = Storage::load_config() {
        let output = models::format_server_completions(&config.servers);
        let _ = io::stdout().write_all(output.as_bytes());
        let _ = io::stdout().flush();
    }
    Ok(())
}

fn complete_tags() -> Result<()> {
    if let Ok(config) = Storage::load_config() {
        let output = models::format_tag_completions(&config.servers);
        let _ = io::stdout().write_all(output.as_bytes());
        let _ = io::stdout().flush();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_parse_edit_flags() {
        let cli = Cli::try_parse_from([
            "ssher",
            "edit",
            "myserver",
            "--host",
            "example.com",
            "--username",
            "alice",
            "--port",
            "2222",
            "--agent-forwarding",
            "true",
            "--new-name",
            "newserver",
        ])
        .unwrap();

        match cli.command {
            Commands::Edit {
                name,
                host,
                username,
                port,
                key,
                password,
                agent_forwarding,
                new_name,
                tag,
                clear_tags,
                jump,
                clear_jump,
                forward,
                clear_forwards,
                tmux,
                clear_tmux,
            } => {
                assert_eq!(name, "myserver");
                assert_eq!(host.as_deref(), Some("example.com"));
                assert_eq!(username.as_deref(), Some("alice"));
                assert_eq!(port, Some(2222));
                assert_eq!(key, None);
                assert!(!password);
                assert_eq!(agent_forwarding, Some(true));
                assert_eq!(new_name.as_deref(), Some("newserver"));
                assert_eq!(tag, None);
                assert!(!clear_tags);
                assert_eq!(jump, None);
                assert!(!clear_jump);
                assert_eq!(forward, None);
                assert!(!clear_forwards);
                assert_eq!(tmux, None);
                assert!(!clear_tmux);
            }
            _ => panic!("expected Commands::Edit"),
        }
    }

    #[test]
    fn cli_parse_edit_agent_forwarding_false() {
        let cli = Cli::try_parse_from(["ssher", "edit", "myserver", "--agent-forwarding", "false"])
            .unwrap();

        match cli.command {
            Commands::Edit {
                agent_forwarding, ..
            } => {
                assert_eq!(agent_forwarding, Some(false));
            }
            _ => panic!("expected Commands::Edit"),
        }
    }

    #[test]
    fn cli_parse_add_tags() {
        let cli = Cli::try_parse_from([
            "ssher",
            "add",
            "srv",
            "1.2.3.4",
            "root",
            "--password",
            "--tag",
            "web",
            "--tag",
            "prod",
        ])
        .unwrap();

        match cli.command {
            Commands::Add { tag, .. } => {
                assert_eq!(tag, vec!["web".to_string(), "prod".to_string()]);
            }
            _ => panic!("expected Commands::Add"),
        }
    }

    #[test]
    fn cli_parse_edit_tags() {
        let cli =
            Cli::try_parse_from(["ssher", "edit", "myserver", "--tag", "web", "--tag", "prod"])
                .unwrap();

        match cli.command {
            Commands::Edit {
                tag, clear_tags, ..
            } => {
                assert_eq!(tag, Some(vec!["web".to_string(), "prod".to_string()]));
                assert!(!clear_tags);
            }
            _ => panic!("expected Commands::Edit"),
        }
    }

    #[test]
    fn cli_parse_edit_clear_tags() {
        let cli = Cli::try_parse_from(["ssher", "edit", "myserver", "--clear-tags"]).unwrap();

        match cli.command {
            Commands::Edit {
                tag, clear_tags, ..
            } => {
                assert_eq!(tag, None);
                assert!(clear_tags);
            }
            _ => panic!("expected Commands::Edit"),
        }
    }

    #[test]
    fn cli_parse_edit_tags_and_clear_tags_conflict() {
        let res =
            Cli::try_parse_from(["ssher", "edit", "myserver", "--tag", "web", "--clear-tags"]);
        assert!(res.is_err());
    }

    #[test]
    fn cli_parse_list_tags() {
        let cli = Cli::try_parse_from(["ssher", "list", "--tag", "web", "--tag", "prod"]).unwrap();
        match cli.command {
            Commands::List { tag, sort } => {
                assert_eq!(tag, vec!["web".to_string(), "prod".to_string()]);
                assert_eq!(sort, ListSort::Recent);
            }
            _ => panic!("expected Commands::List"),
        }
    }

    #[test]
    fn cli_parse_list_sort() {
        let cli = Cli::try_parse_from(["ssher", "list", "--sort", "name"]).unwrap();
        match cli.command {
            Commands::List { sort, .. } => {
                assert_eq!(sort, ListSort::Name);
            }
            _ => panic!("expected Commands::List"),
        }

        let cli_recent = Cli::try_parse_from(["ssher", "list", "--sort", "recent"]).unwrap();
        match cli_recent.command {
            Commands::List { sort, .. } => {
                assert_eq!(sort, ListSort::Recent);
            }
            _ => panic!("expected Commands::List"),
        }

        assert!(Cli::try_parse_from(["ssher", "list", "--sort", "invalid"]).is_err());
    }

    #[test]
    fn cli_parse_fav_and_unfav() {
        let cli_fav = Cli::try_parse_from(["ssher", "fav", "myserver"]).unwrap();
        match cli_fav.command {
            Commands::Fav { name } => assert_eq!(name, "myserver"),
            _ => panic!("expected Commands::Fav"),
        }

        let cli_unfav = Cli::try_parse_from(["ssher", "unfav", "myserver"]).unwrap();
        match cli_unfav.command {
            Commands::Unfav { name } => assert_eq!(name, "myserver"),
            _ => panic!("expected Commands::Unfav"),
        }
    }

    #[test]
    fn cli_parse_edit_key_and_password_conflict() {
        let res = Cli::try_parse_from([
            "ssher",
            "edit",
            "myserver",
            "--key",
            "/path/to/key",
            "--password",
        ]);
        assert!(res.is_err());
    }

    #[test]
    fn cli_connect_partial_name_arg() {
        let cli = Cli::try_parse_from(["ssher", "connect", "prod"]).unwrap();
        match cli.command {
            Commands::Connect {
                name,
                current,
                forward,
                no_forwards,
                tmux,
                no_tmux,
            } => {
                assert_eq!(name, "prod");
                assert!(!current);
                assert!(forward.is_empty());
                assert!(!no_forwards);
                assert_eq!(tmux, None);
                assert!(!no_tmux);
            }
            _ => panic!("expected Commands::Connect"),
        }
    }

    #[test]
    fn cli_parse_add_forwards() {
        let cli = Cli::try_parse_from([
            "ssher",
            "add",
            "srv",
            "1.2.3.4",
            "root",
            "--key",
            "/path/to/key",
            "--forward",
            "5432",
            "--forward",
            "R:2222:host:22",
            "--forward",
            "D:1080",
        ])
        .unwrap();

        match cli.command {
            Commands::Add { forward, .. } => {
                assert_eq!(
                    forward,
                    vec![
                        "5432".to_string(),
                        "R:2222:host:22".to_string(),
                        "D:1080".to_string(),
                    ]
                );
            }
            _ => panic!("expected Commands::Add"),
        }
    }

    #[test]
    fn cli_parse_edit_forwards() {
        let cli = Cli::try_parse_from([
            "ssher",
            "edit",
            "srv",
            "--forward",
            "L:8080:localhost:80",
            "--forward",
            "3000",
        ])
        .unwrap();

        match cli.command {
            Commands::Edit {
                forward,
                clear_forwards,
                ..
            } => {
                assert_eq!(
                    forward,
                    Some(vec!["L:8080:localhost:80".to_string(), "3000".to_string()])
                );
                assert!(!clear_forwards);
            }
            _ => panic!("expected Commands::Edit"),
        }
    }

    #[test]
    fn cli_parse_edit_clear_forwards() {
        let cli = Cli::try_parse_from(["ssher", "edit", "srv", "--clear-forwards"]).unwrap();

        match cli.command {
            Commands::Edit {
                forward,
                clear_forwards,
                ..
            } => {
                assert_eq!(forward, None);
                assert!(clear_forwards);
            }
            _ => panic!("expected Commands::Edit"),
        }
    }

    #[test]
    fn cli_parse_edit_forwards_and_clear_forwards_conflict() {
        let res = Cli::try_parse_from([
            "ssher",
            "edit",
            "srv",
            "--forward",
            "5432",
            "--clear-forwards",
        ]);
        assert!(res.is_err());
    }

    #[test]
    fn cli_parse_connect_forwards() {
        let cli = Cli::try_parse_from([
            "ssher",
            "connect",
            "prod",
            "--forward",
            "8080",
            "--forward",
            "D:1080",
        ])
        .unwrap();

        match cli.command {
            Commands::Connect {
                name,
                forward,
                no_forwards,
                ..
            } => {
                assert_eq!(name, "prod");
                assert_eq!(forward, vec!["8080".to_string(), "D:1080".to_string()]);
                assert!(!no_forwards);
            }
            _ => panic!("expected Commands::Connect"),
        }
    }

    #[test]
    fn cli_parse_connect_no_forwards() {
        let cli = Cli::try_parse_from(["ssher", "connect", "prod", "--no-forwards"]).unwrap();

        match cli.command {
            Commands::Connect {
                name,
                forward,
                no_forwards,
                ..
            } => {
                assert_eq!(name, "prod");
                assert!(forward.is_empty());
                assert!(no_forwards);
            }
            _ => panic!("expected Commands::Connect"),
        }
    }

    #[test]
    fn cli_parse_connect_forwards_with_no_forwards() {
        let cli = Cli::try_parse_from([
            "ssher",
            "connect",
            "prod",
            "--forward",
            "8080",
            "--no-forwards",
        ])
        .unwrap();

        match cli.command {
            Commands::Connect {
                name,
                forward,
                no_forwards,
                ..
            } => {
                assert_eq!(name, "prod");
                assert_eq!(forward, vec!["8080".to_string()]);
                assert!(no_forwards);
            }
            _ => panic!("expected Commands::Connect"),
        }
    }

    #[test]
    fn cli_parse_connect_tmux_flag_no_val() {
        let cli = Cli::try_parse_from(["ssher", "connect", "prod", "--tmux"]).unwrap();

        match cli.command {
            Commands::Connect {
                name,
                tmux,
                no_tmux,
                ..
            } => {
                assert_eq!(name, "prod");
                assert_eq!(tmux.as_deref(), Some("ssher"));
                assert!(!no_tmux);
            }
            _ => panic!("expected Commands::Connect"),
        }
    }

    #[test]
    fn cli_parse_connect_tmux_flag_with_val() {
        let cli = Cli::try_parse_from(["ssher", "connect", "prod", "--tmux", "work"]).unwrap();

        match cli.command {
            Commands::Connect {
                name,
                tmux,
                no_tmux,
                ..
            } => {
                assert_eq!(name, "prod");
                assert_eq!(tmux.as_deref(), Some("work"));
                assert!(!no_tmux);
            }
            _ => panic!("expected Commands::Connect"),
        }
    }

    #[test]
    fn cli_parse_connect_no_tmux() {
        let cli = Cli::try_parse_from(["ssher", "connect", "prod", "--no-tmux"]).unwrap();

        match cli.command {
            Commands::Connect {
                name,
                tmux,
                no_tmux,
                ..
            } => {
                assert_eq!(name, "prod");
                assert_eq!(tmux, None);
                assert!(no_tmux);
            }
            _ => panic!("expected Commands::Connect"),
        }
    }

    #[test]
    fn cli_parse_connect_tmux_and_no_tmux_conflict() {
        let res = Cli::try_parse_from(["ssher", "connect", "prod", "--tmux", "--no-tmux"]);
        assert!(res.is_err());
    }

    #[test]
    fn cli_parse_completions_subcommand() {
        for (shell_arg, expected) in [
            ("bash", clap_complete::Shell::Bash),
            ("zsh", clap_complete::Shell::Zsh),
            ("fish", clap_complete::Shell::Fish),
            ("elvish", clap_complete::Shell::Elvish),
            ("powershell", clap_complete::Shell::PowerShell),
        ] {
            let cli = Cli::try_parse_from(["ssher", "completions", shell_arg]).unwrap();
            match cli.command {
                Commands::Completions { shell } => assert_eq!(shell, expected),
                _ => panic!("expected Commands::Completions"),
            }
        }

        assert!(Cli::try_parse_from(["ssher", "completions", "unknown"]).is_err());
    }

    #[test]
    fn cli_parse_complete_servers_subcommand() {
        let cli = Cli::try_parse_from(["ssher", "__complete-servers"]).unwrap();
        assert!(matches!(cli.command, Commands::CompleteServers));
    }

    #[test]
    fn cli_parse_complete_tags_subcommand() {
        let cli = Cli::try_parse_from(["ssher", "__complete-tags"]).unwrap();
        assert!(matches!(cli.command, Commands::CompleteTags));
    }

    #[test]
    fn completion_scripts_generated_for_all_shells() {
        for shell in [
            clap_complete::Shell::Bash,
            clap_complete::Shell::Zsh,
            clap_complete::Shell::Fish,
            clap_complete::Shell::Elvish,
            clap_complete::Shell::PowerShell,
        ] {
            let script = generate_completion_script(shell);
            assert!(
                !script.is_empty(),
                "script should not be empty for {shell:?}"
            );
            assert!(
                script.contains("ssher"),
                "script should mention ssher for {shell:?}"
            );
        }

        let bash = generate_completion_script(clap_complete::Shell::Bash);
        assert!(bash.contains("__complete-servers"));
        assert!(bash.contains("__complete-tags"));
        assert!(bash.contains("_ssher_complete"));
        assert!(bash.contains("--jump"));
        assert!(bash.contains("--forward"));
        assert!(bash.contains("--sort"));
        assert!(bash.contains("--tmux"));
        assert!(bash.contains("connect|edit|remove|fav|unfav|exec|copy-id|put|get|ping"));

        let zsh = generate_completion_script(clap_complete::Shell::Zsh);
        assert!(zsh.contains("__complete-servers"));
        assert!(zsh.contains("__complete-tags"));
        assert!(zsh.contains("_ssher_servers"));
        assert!(zsh.contains("_ssher_tags"));
        assert!(zsh.contains(":JUMP:_ssher_servers"));
        assert!(zsh.contains(":name -- Server name to favorite:_ssher_servers"));
        assert!(zsh.contains(":name -- Server name to unfavorite:_ssher_servers"));
        assert!(zsh.contains(":name -- Server name to copy SSH key to:_ssher_servers"));
        assert!(zsh.contains(":name -- Server name to upload files to:_ssher_servers"));
        assert!(zsh.contains(":name -- Server name to download files from:_ssher_servers"));
        assert!(zsh.contains(":name -- Server name to ping:_ssher_servers"));

        let fish = generate_completion_script(clap_complete::Shell::Fish);
        assert!(fish.contains("__complete-servers"));
        assert!(fish.contains("__complete-tags"));
        assert!(fish.contains("__fish_ssher_using_subcommand"));
        assert!(fish.contains("connect edit remove fav unfav exec copy-id put get ping"));
        assert!(fish.contains("-l jump"));
        assert!(fish.contains("-l sort"));
    }

    #[test]
    fn cli_parse_add_jump() {
        let cli = Cli::try_parse_from([
            "ssher", "add", "srv", "1.2.3.4", "root", "--key", "/key", "--jump", "bastion",
        ])
        .unwrap();

        match cli.command {
            Commands::Add { jump, .. } => {
                assert_eq!(jump.as_deref(), Some("bastion"));
            }
            _ => panic!("expected Commands::Add"),
        }
    }

    #[test]
    fn cli_parse_add_tmux() {
        let cli = Cli::try_parse_from([
            "ssher",
            "add",
            "srv",
            "1.2.3.4",
            "root",
            "--key",
            "/key",
            "--tmux",
            "my-session",
        ])
        .unwrap();

        match cli.command {
            Commands::Add { tmux, .. } => {
                assert_eq!(tmux.as_deref(), Some("my-session"));
            }
            _ => panic!("expected Commands::Add"),
        }
    }

    #[test]
    fn cli_parse_edit_jump() {
        let cli = Cli::try_parse_from(["ssher", "edit", "srv", "--jump", "user@proxy:22"]).unwrap();

        match cli.command {
            Commands::Edit {
                jump, clear_jump, ..
            } => {
                assert_eq!(jump.as_deref(), Some("user@proxy:22"));
                assert!(!clear_jump);
            }
            _ => panic!("expected Commands::Edit"),
        }
    }

    #[test]
    fn cli_parse_edit_clear_jump() {
        let cli = Cli::try_parse_from(["ssher", "edit", "srv", "--clear-jump"]).unwrap();

        match cli.command {
            Commands::Edit {
                jump, clear_jump, ..
            } => {
                assert_eq!(jump, None);
                assert!(clear_jump);
            }
            _ => panic!("expected Commands::Edit"),
        }
    }

    #[test]
    fn cli_parse_edit_jump_and_clear_jump_conflict() {
        assert!(
            Cli::try_parse_from(["ssher", "edit", "srv", "--jump", "bastion", "--clear-jump"])
                .is_err()
        );
    }

    #[test]
    fn cli_parse_edit_tmux() {
        let cli = Cli::try_parse_from(["ssher", "edit", "srv", "--tmux", "session2"]).unwrap();

        match cli.command {
            Commands::Edit {
                tmux, clear_tmux, ..
            } => {
                assert_eq!(tmux.as_deref(), Some("session2"));
                assert!(!clear_tmux);
            }
            _ => panic!("expected Commands::Edit"),
        }
    }

    #[test]
    fn cli_parse_edit_clear_tmux() {
        let cli = Cli::try_parse_from(["ssher", "edit", "srv", "--clear-tmux"]).unwrap();

        match cli.command {
            Commands::Edit {
                tmux, clear_tmux, ..
            } => {
                assert_eq!(tmux, None);
                assert!(clear_tmux);
            }
            _ => panic!("expected Commands::Edit"),
        }
    }

    #[test]
    fn cli_parse_edit_tmux_and_clear_tmux_conflict() {
        assert!(
            Cli::try_parse_from(["ssher", "edit", "srv", "--tmux", "sess", "--clear-tmux"])
                .is_err()
        );
    }

    #[test]
    fn cli_parse_import_defaults() {
        let cli = Cli::try_parse_from(["ssher", "import"]).unwrap();
        match cli.command {
            Commands::Import {
                file,
                dry_run,
                skip_existing,
            } => {
                assert_eq!(file, None);
                assert!(!dry_run);
                assert!(!skip_existing);
            }
            _ => panic!("expected Commands::Import"),
        }
    }

    #[test]
    fn cli_parse_import_all_flags() {
        let cli = Cli::try_parse_from([
            "ssher",
            "import",
            "--file",
            "/tmp/custom_config",
            "--dry-run",
            "--skip-existing",
        ])
        .unwrap();
        match cli.command {
            Commands::Import {
                file,
                dry_run,
                skip_existing,
            } => {
                assert_eq!(file, Some(std::path::PathBuf::from("/tmp/custom_config")));
                assert!(dry_run);
                assert!(skip_existing);
            }
            _ => panic!("expected Commands::Import"),
        }
    }

    #[test]
    fn cli_parse_exec_single_target() {
        let cli = Cli::try_parse_from(["ssher", "exec", "myserver", "--", "uptime"]).unwrap();
        match cli.command {
            Commands::Exec {
                target,
                tag,
                all,
                command,
            } => {
                assert_eq!(target.as_deref(), Some("myserver"));
                assert!(tag.is_empty());
                assert!(!all);
                assert_eq!(command, vec!["uptime"]);
            }
            _ => panic!("expected Commands::Exec"),
        }
    }

    #[test]
    fn cli_parse_exec_all() {
        let cli = Cli::try_parse_from(["ssher", "exec", "--all", "--", "uname", "-a"]).unwrap();
        match cli.command {
            Commands::Exec {
                target,
                tag,
                all,
                command,
            } => {
                assert_eq!(target, None);
                assert!(tag.is_empty());
                assert!(all);
                assert_eq!(command, vec!["uname", "-a"]);
            }
            _ => panic!("expected Commands::Exec"),
        }
    }

    #[test]
    fn cli_parse_exec_tags() {
        let cli = Cli::try_parse_from([
            "ssher", "exec", "--tag", "prod", "--tag", "web", "--", "df", "-h",
        ])
        .unwrap();
        match cli.command {
            Commands::Exec {
                target,
                tag,
                all,
                command,
            } => {
                assert_eq!(target, None);
                assert_eq!(tag, vec!["prod", "web"]);
                assert!(!all);
                assert_eq!(command, vec!["df", "-h"]);
            }
            _ => panic!("expected Commands::Exec"),
        }
    }

    #[test]
    fn cli_parse_exec_target_and_all_conflict() {
        assert!(
            Cli::try_parse_from(["ssher", "exec", "myserver", "--all", "--", "uptime"]).is_err()
        );
    }

    #[test]
    fn cli_parse_exec_target_and_tag_conflict() {
        assert!(Cli::try_parse_from([
            "ssher", "exec", "myserver", "--tag", "prod", "--", "uptime"
        ])
        .is_err());
    }

    #[test]
    fn cli_parse_exec_all_and_tag_conflict() {
        assert!(
            Cli::try_parse_from(["ssher", "exec", "--all", "--tag", "prod", "--", "uptime"])
                .is_err()
        );
    }

    #[test]
    fn cli_parse_exec_missing_target_and_flags_errors() {
        assert!(Cli::try_parse_from(["ssher", "exec", "--", "uptime"]).is_err());
    }

    #[test]
    fn cli_parse_exec_missing_command_errors() {
        assert!(Cli::try_parse_from(["ssher", "exec", "myserver"]).is_err());
        assert!(Cli::try_parse_from(["ssher", "exec", "--all"]).is_err());
        assert!(Cli::try_parse_from(["ssher", "exec", "myserver", "--"]).is_err());
    }

    #[test]
    fn cli_parse_copy_id_defaults() {
        let cli = Cli::try_parse_from(["ssher", "copy-id", "myserver"]).unwrap();
        match cli.command {
            Commands::CopyId {
                name,
                pub_path,
                key,
                switch_to_key,
            } => {
                assert_eq!(name, "myserver");
                assert_eq!(pub_path, None);
                assert_eq!(key, None);
                assert!(!switch_to_key);
            }
            _ => panic!("expected Commands::CopyId"),
        }
    }

    #[test]
    fn cli_parse_copy_id_pub_flag() {
        let cli = Cli::try_parse_from([
            "ssher",
            "copy-id",
            "myserver",
            "--pub",
            "/home/user/.ssh/id_ed25519.pub",
        ])
        .unwrap();
        match cli.command {
            Commands::CopyId {
                name,
                pub_path,
                key,
                switch_to_key,
            } => {
                assert_eq!(name, "myserver");
                assert_eq!(pub_path, Some("/home/user/.ssh/id_ed25519.pub".to_string()));
                assert_eq!(key, None);
                assert!(!switch_to_key);
            }
            _ => panic!("expected Commands::CopyId"),
        }
    }

    #[test]
    fn cli_parse_copy_id_key_flag() {
        let cli = Cli::try_parse_from([
            "ssher",
            "copy-id",
            "myserver",
            "--key",
            "/home/user/.ssh/id_ed25519",
        ])
        .unwrap();
        match cli.command {
            Commands::CopyId {
                name,
                pub_path,
                key,
                switch_to_key,
            } => {
                assert_eq!(name, "myserver");
                assert_eq!(pub_path, None);
                assert_eq!(key, Some("/home/user/.ssh/id_ed25519".to_string()));
                assert!(!switch_to_key);
            }
            _ => panic!("expected Commands::CopyId"),
        }
    }

    #[test]
    fn cli_parse_copy_id_switch_to_key() {
        let cli = Cli::try_parse_from([
            "ssher",
            "copy-id",
            "myserver",
            "--pub",
            "/home/user/.ssh/id_ed25519.pub",
            "--switch-to-key",
        ])
        .unwrap();
        match cli.command {
            Commands::CopyId {
                name,
                pub_path,
                key,
                switch_to_key,
            } => {
                assert_eq!(name, "myserver");
                assert_eq!(pub_path, Some("/home/user/.ssh/id_ed25519.pub".to_string()));
                assert_eq!(key, None);
                assert!(switch_to_key);
            }
            _ => panic!("expected Commands::CopyId"),
        }
    }

    #[test]
    fn cli_parse_copy_id_pub_and_key_conflict() {
        assert!(Cli::try_parse_from([
            "ssher",
            "copy-id",
            "myserver",
            "--pub",
            "/path/to/key.pub",
            "--key",
            "/path/to/key",
        ])
        .is_err());
    }

    #[test]
    fn cli_parse_copy_id_missing_name_errors() {
        assert!(Cli::try_parse_from(["ssher", "copy-id"]).is_err());
    }

    #[test]
    fn cli_parse_put_defaults() {
        let cli =
            Cli::try_parse_from(["ssher", "put", "myserver", "file1.txt", "/remote/dir"]).unwrap();
        match cli.command {
            Commands::Put {
                name,
                recursive,
                paths,
            } => {
                assert_eq!(name, "myserver");
                assert!(!recursive);
                assert_eq!(paths, vec!["file1.txt", "/remote/dir"]);
            }
            _ => panic!("expected Commands::Put"),
        }
    }

    #[test]
    fn cli_parse_put_recursive() {
        let cli = Cli::try_parse_from(["ssher", "put", "-r", "myserver", "./dir", "/remote/dir"])
            .unwrap();
        match cli.command {
            Commands::Put {
                name,
                recursive,
                paths,
            } => {
                assert_eq!(name, "myserver");
                assert!(recursive);
                assert_eq!(paths, vec!["./dir", "/remote/dir"]);
            }
            _ => panic!("expected Commands::Put"),
        }
    }

    #[test]
    fn cli_parse_get_defaults() {
        let cli =
            Cli::try_parse_from(["ssher", "get", "myserver", "/remote/file.txt", "."]).unwrap();
        match cli.command {
            Commands::Get {
                name,
                recursive,
                paths,
            } => {
                assert_eq!(name, "myserver");
                assert!(!recursive);
                assert_eq!(paths, vec!["/remote/file.txt", "."]);
            }
            _ => panic!("expected Commands::Get"),
        }
    }

    #[test]
    fn cli_parse_get_recursive() {
        let cli = Cli::try_parse_from([
            "ssher",
            "get",
            "--recursive",
            "myserver",
            "/remote/dir",
            "./local",
        ])
        .unwrap();
        match cli.command {
            Commands::Get {
                name,
                recursive,
                paths,
            } => {
                assert_eq!(name, "myserver");
                assert!(recursive);
                assert_eq!(paths, vec!["/remote/dir", "./local"]);
            }
            _ => panic!("expected Commands::Get"),
        }
    }

    #[test]
    fn cli_parse_put_missing_args_errors() {
        assert!(Cli::try_parse_from(["ssher", "put", "myserver"]).is_err());
        assert!(Cli::try_parse_from(["ssher", "put", "myserver", "only_one"]).is_err());
        assert!(Cli::try_parse_from(["ssher", "put"]).is_err());
    }

    #[test]
    fn cli_parse_get_missing_args_errors() {
        assert!(Cli::try_parse_from(["ssher", "get", "myserver"]).is_err());
        assert!(Cli::try_parse_from(["ssher", "get", "myserver", "only_one"]).is_err());
        assert!(Cli::try_parse_from(["ssher", "get"]).is_err());
    }

    #[test]
    fn cli_parse_ping_defaults() {
        let cli = Cli::try_parse_from(["ssher", "ping"]).unwrap();
        match cli.command {
            Commands::Ping { name, tag, timeout } => {
                assert_eq!(name, None);
                assert!(tag.is_empty());
                assert_eq!(timeout, 3);
            }
            _ => panic!("expected Commands::Ping"),
        }
    }

    #[test]
    fn cli_parse_ping_name() {
        let cli = Cli::try_parse_from(["ssher", "ping", "myserver"]).unwrap();
        match cli.command {
            Commands::Ping { name, tag, timeout } => {
                assert_eq!(name.as_deref(), Some("myserver"));
                assert!(tag.is_empty());
                assert_eq!(timeout, 3);
            }
            _ => panic!("expected Commands::Ping"),
        }
    }

    #[test]
    fn cli_parse_ping_tags_and_timeout() {
        let cli = Cli::try_parse_from([
            "ssher",
            "ping",
            "--tag",
            "prod",
            "--tag",
            "web",
            "--timeout",
            "10",
        ])
        .unwrap();
        match cli.command {
            Commands::Ping { name, tag, timeout } => {
                assert_eq!(name, None);
                assert_eq!(tag, vec!["prod", "web"]);
                assert_eq!(timeout, 10);
            }
            _ => panic!("expected Commands::Ping"),
        }
    }

    #[test]
    fn cli_parse_ping_name_and_tag_conflict() {
        assert!(Cli::try_parse_from(["ssher", "ping", "myserver", "--tag", "prod"]).is_err());
    }

    #[test]
    fn cli_parse_export_defaults() {
        let cli = Cli::try_parse_from(["ssher", "export"]).unwrap();
        match cli.command {
            Commands::Export { file, tag, force } => {
                assert!(file.is_none());
                assert!(tag.is_empty());
                assert!(!force);
            }
            _ => panic!("expected Commands::Export"),
        }
    }

    #[test]
    fn cli_parse_export_file_and_tags_and_force() {
        let cli = Cli::try_parse_from([
            "ssher",
            "export",
            "--file",
            "/tmp/backup.json",
            "--tag",
            "prod",
            "--tag",
            "web",
            "--force",
        ])
        .unwrap();
        match cli.command {
            Commands::Export { file, tag, force } => {
                assert_eq!(
                    file.as_deref(),
                    Some(std::path::Path::new("/tmp/backup.json"))
                );
                assert_eq!(tag, vec!["prod", "web"]);
                assert!(force);
            }
            _ => panic!("expected Commands::Export"),
        }
    }

    #[test]
    fn cli_parse_import_backup_defaults() {
        let cli = Cli::try_parse_from(["ssher", "import-backup", "/tmp/backup.json"]).unwrap();
        match cli.command {
            Commands::ImportBackup {
                file,
                overwrite,
                dry_run,
            } => {
                assert_eq!(file, std::path::PathBuf::from("/tmp/backup.json"));
                assert!(!overwrite);
                assert!(!dry_run);
            }
            _ => panic!("expected Commands::ImportBackup"),
        }
    }

    #[test]
    fn cli_parse_import_backup_all_flags() {
        let cli = Cli::try_parse_from([
            "ssher",
            "import-backup",
            "/tmp/backup.json",
            "--overwrite",
            "--dry-run",
        ])
        .unwrap();
        match cli.command {
            Commands::ImportBackup {
                file,
                overwrite,
                dry_run,
            } => {
                assert_eq!(file, std::path::PathBuf::from("/tmp/backup.json"));
                assert!(overwrite);
                assert!(dry_run);
            }
            _ => panic!("expected Commands::ImportBackup"),
        }
    }

    #[test]
    fn cli_parse_import_backup_missing_file_errors() {
        assert!(Cli::try_parse_from(["ssher", "import-backup"]).is_err());
    }
}
