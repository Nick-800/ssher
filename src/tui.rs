use crate::error::{Result, SshManagerError};
use crate::models::{AuthMethod, Server};
use crate::ssh::Ssh;
use crate::storage::Storage;
use crate::terminal::Terminal;
use crossterm::event::{self, Event, KeyCode, KeyEvent};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Terminal as RattuiTerminal;
use std::io::{self, Write};

#[derive(Debug, Clone, Copy, PartialEq)]
enum MenuItem {
    List,
    Add,
    Connect,
    Remove,
    Config,
    Exit,
}

pub struct Tui {
    current_menu: MenuItem,
    menu_items: Vec<MenuItem>,
}

impl Tui {
    pub fn new() -> Self {
        Tui {
            current_menu: MenuItem::List,
            menu_items: vec![
                MenuItem::List,
                MenuItem::Add,
                MenuItem::Connect,
                MenuItem::Remove,
                MenuItem::Config,
                MenuItem::Exit,
            ],
        }
    }

    pub fn run(&mut self) -> Result<()> {
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = RattuiTerminal::new(backend)?;

        // Run main loop
        let result = self.main_loop(&mut terminal);

        // Restore terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen
        )?;
        terminal.show_cursor()?;

        result
    }

    fn main_loop(&mut self, terminal: &mut RattuiTerminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        loop {
            terminal.draw(|f| {
                self.draw(f);
            })?;

            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => self.previous(),
                    KeyCode::Down | KeyCode::Char('j') => self.next(),
                    KeyCode::Enter => {
                        if self.current_menu == MenuItem::Exit {
                            break;
                        }
                        self.handle_selection()?;
                    }
                    KeyCode::Esc | KeyCode::Char('q') => break,
                    _ => {}
                }
            }
        }

        Ok(())
    }

    fn draw(&self, f: &mut ratatui::Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(2)
            .constraints([
                Constraint::Length(8),
                Constraint::Min(10),
                Constraint::Length(3),
            ])
            .split(f.size());

        // Draw banner
        let banner = Self::get_banner();
        let banner_widget = Paragraph::new(banner)
            .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            .alignment(Alignment::Center);
        f.render_widget(banner_widget, chunks[0]);

        // Draw menu
        let menu_items: Vec<ListItem> = self
            .menu_items
            .iter()
            .map(|item| {
                let label = self.get_menu_label(*item);
                if *item == self.current_menu {
                    ListItem::new(format!("▶ {}", label))
                        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                } else {
                    ListItem::new(format!("  {}", label))
                        .style(Style::default().fg(Color::White))
                }
            })
            .collect();

        let menu_list = List::new(menu_items)
            .block(Block::default().title("Menu").borders(Borders::ALL))
            .style(Style::default().fg(Color::White));

        f.render_widget(menu_list, chunks[1]);

        // Draw instructions
        let instructions = vec![
            Line::from(vec![
                Span::styled("↑/↓ or j/k", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw(" to navigate • "),
                Span::styled("Enter", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw(" to select • "),
                Span::styled("q/Esc", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw(" to quit"),
            ]),
        ];

        let instructions_widget = Paragraph::new(instructions)
            .block(Block::default().borders(Borders::ALL))
            .alignment(Alignment::Center);

        f.render_widget(instructions_widget, chunks[2]);
    }

    fn get_banner() -> String {
        let lines = vec![
            "  ███████╗███████╗██╗  ██╗███████╗██████╗ ",
            "  ██╔════╝██╔════╝██║  ██║██╔════╝██╔══██╗",
            "  ███████╗███████╗███████║█████╗  ██████╔╝",
            "  ╚════██║╚════██║██╔══██║██╔══╝  ██╔══██╗",
            "  ███████║███████║██║  ██║███████╗██║  ██║",
            "  ╚══════╝╚══════╝╚═╝  ╚═╝╚══════╝╚═╝  ╚═╝",
        ];
        lines.join("\n")
    }

    fn get_menu_label(&self, item: MenuItem) -> String {
        match item {
            MenuItem::List => "List Servers".to_string(),
            MenuItem::Add => "Add New Server".to_string(),
            MenuItem::Connect => "Connect to Server".to_string(),
            MenuItem::Remove => "Remove Server".to_string(),
            MenuItem::Config => "Configure Settings".to_string(),
            MenuItem::Exit => "Exit".to_string(),
        }
    }

    fn next(&mut self) {
        let idx = self
            .menu_items
            .iter()
            .position(|&m| m == self.current_menu)
            .unwrap_or(0);
        self.current_menu = self.menu_items[(idx + 1) % self.menu_items.len()];
    }

    fn previous(&mut self) {
        let idx = self
            .menu_items
            .iter()
            .position(|&m| m == self.current_menu)
            .unwrap_or(0);
        self.current_menu = self.menu_items[(idx + self.menu_items.len() - 1) % self.menu_items.len()];
    }

    fn handle_selection(&self) -> Result<()> {
        disable_raw_mode()?;
        execute!(io::stdout(), LeaveAlternateScreen)?;

        match self.current_menu {
            MenuItem::List => self.show_list(),
            MenuItem::Add => self.show_add(),
            MenuItem::Connect => self.show_connect(),
            MenuItem::Remove => self.show_remove(),
            MenuItem::Config => self.show_config(),
            MenuItem::Exit => Ok(()),
        }?;

        println!("\nPress Enter to return to menu...");
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;

        Ok(())
    }

    fn show_list(&self) -> Result<()> {
        let config = crate::storage::Storage::load_config()?;

        println!("\n{}", "=".repeat(95));
        println!("{:^95}", "SSH SERVERS");
        println!("{}", "=".repeat(95));

        if config.servers.is_empty() {
            println!("\nNo servers configured.\n");
            return Ok(());
        }

        println!(
            "{:<20} {:<20} {:<15} {:<10} {:<12} {:<8}",
            "Name", "Host", "Username", "Port", "Auth", "Agent FW"
        );
        println!("{}", "─".repeat(95));

        for server in &config.servers {
            let auth = match server.auth_method {
                AuthMethod::Key => "key",
                AuthMethod::Password => "password",
            };
            let agent_fw = if server.use_agent_forwarding {
                "yes"
            } else {
                "no"
            };

            println!(
                "{:<20} {:<20} {:<15} {:<10} {:<12} {:<8}",
                server.name, server.host, server.username, server.port, auth, agent_fw
            );
        }
        println!();

        Ok(())
    }

    fn show_add(&self) -> Result<()> {
        println!("\n{}", "=".repeat(60));
        println!("{:^60}", "ADD NEW SERVER");
        println!("{}", "=".repeat(60));

        print!("\nServer name: ");
        io::stdout().flush()?;
        let mut name = String::new();
        io::stdin().read_line(&mut name)?;
        let name = name.trim();

        print!("Host (IP or domain): ");
        io::stdout().flush()?;
        let mut host = String::new();
        io::stdin().read_line(&mut host)?;
        let host = host.trim();

        print!("Username: ");
        io::stdout().flush()?;
        let mut username = String::new();
        io::stdin().read_line(&mut username)?;
        let username = username.trim();

        print!("Port (default 22): ");
        io::stdout().flush()?;
        let mut port_str = String::new();
        io::stdin().read_line(&mut port_str)?;
        let port: u16 = port_str.trim().parse().unwrap_or(22);

        println!("\nAuthentication method:");
        println!("1. SSH Key");
        println!("2. Password");
        print!("Choose (1 or 2): ");
        io::stdout().flush()?;
        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;

        let (auth_method, key_path) = if choice.trim() == "2" {
            println!("\nEnter password:");
            let password = rpassword::read_password().map_err(|e| {
                SshManagerError::InvalidConfig(format!("Failed to read password: {}", e))
            })?;

            Ssh::store_password(name, &password)?;
            (AuthMethod::Password, None)
        } else {
            print!("SSH key path: ");
            io::stdout().flush()?;
            let mut key_path = String::new();
            io::stdin().read_line(&mut key_path)?;
            let key_path = key_path.trim();
            Storage::validate_key_path(key_path)?;
            (AuthMethod::Key, Some(key_path.to_string()))
        };

        print!("Enable agent forwarding? (y/n): ");
        io::stdout().flush()?;
        let mut agent_choice = String::new();
        io::stdin().read_line(&mut agent_choice)?;
        let agent_forwarding = agent_choice.trim().to_lowercase() == "y";

        // Validate inputs
        Storage::validate_server_name(name)?;
        Storage::validate_host(host)?;
        Storage::validate_username(username)?;
        Storage::validate_port(port)?;

        let mut config = Storage::load_config()?;

        if !Storage::server_name_is_unique(&config, name, None) {
            return Err(SshManagerError::ServerAlreadyExists(name.to_string()));
        }

        let mut server = Server::new(
            name.to_string(),
            host.to_string(),
            username.to_string(),
            port,
            auth_method,
            key_path,
        );
        server.use_agent_forwarding = agent_forwarding;

        config.servers.push(server);
        Storage::save_config(&config)?;

        println!("\n✓ Server '{}' added successfully!", name);

        Ok(())
    }

    fn show_connect(&self) -> Result<()> {
        println!("\n{}", "=".repeat(60));
        println!("{:^60}", "CONNECT TO SERVER");
        println!("{}", "=".repeat(60));

        let config = Storage::load_config()?;

        if config.servers.is_empty() {
            println!("\nNo servers configured.\n");
            return Ok(());
        }

        println!("\nAvailable servers:");
        for (idx, server) in config.servers.iter().enumerate() {
            println!("{}. {} ({})", idx + 1, server.name, server.host);
        }

        print!("\nSelect server number: ");
        io::stdout().flush()?;
        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;
        let idx: usize = choice.trim().parse().unwrap_or(0);

        if idx == 0 || idx > config.servers.len() {
            println!("Invalid selection.");
            return Ok(());
        }

        let server = &config.servers[idx - 1];
        let terminal_name = Terminal::get_terminal_command(config.preferred_terminal.as_deref())?;

        println!("\nConnecting to {}...", server.name);
        Ssh::launch_connection(server, &terminal_name)?;

        Ok(())
    }

    fn show_remove(&self) -> Result<()> {
        println!("\n{}", "=".repeat(60));
        println!("{:^60}", "REMOVE SERVER");
        println!("{}", "=".repeat(60));

        let config = Storage::load_config()?;

        if config.servers.is_empty() {
            println!("\nNo servers configured.\n");
            return Ok(());
        }

        println!("\nServers:");
        for (idx, server) in config.servers.iter().enumerate() {
            println!("{}. {}", idx + 1, server.name);
        }

        print!("\nSelect server number to remove: ");
        io::stdout().flush()?;
        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;
        let idx: usize = choice.trim().parse().unwrap_or(0);

        if idx == 0 || idx > config.servers.len() {
            println!("Invalid selection.");
            return Ok(());
        }

        let server_name = config.servers[idx - 1].name.clone();

        print!("Are you sure? (y/n): ");
        io::stdout().flush()?;
        let mut confirm = String::new();
        io::stdin().read_line(&mut confirm)?;

        if confirm.trim().to_lowercase() != "y" {
            println!("Cancelled.");
            return Ok(());
        }

        let mut config = Storage::load_config()?;
        config.servers.retain(|s| s.name != server_name);
        Storage::save_config(&config)?;
        let _ = Ssh::delete_password(&server_name);

        println!("\n✓ Server '{}' removed successfully!", server_name);

        Ok(())
    }

    fn show_config(&self) -> Result<()> {
        println!("\n{}", "=".repeat(60));
        println!("{:^60}", "CONFIGURE SETTINGS");
        println!("{}", "=".repeat(60));

        println!("\n1. Set terminal emulator");
        println!("2. Back to menu");

        print!("\nChoose option: ");
        io::stdout().flush()?;
        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;

        if choice.trim() == "1" {
            print!("\nTerminal name (gnome-terminal, xterm, etc.): ");
            io::stdout().flush()?;
            let mut terminal = String::new();
            io::stdin().read_line(&mut terminal)?;
            let terminal = terminal.trim();

            Terminal::get_terminal_command(Some(terminal))?;

            let mut config = Storage::load_config()?;
            config.preferred_terminal = Some(terminal.to_string());
            Storage::save_config(&config)?;

            println!("\n✓ Terminal preference set to '{}'", terminal);
        }

        Ok(())
    }
}
