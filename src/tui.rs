use crate::error::{Result, SshManagerError};
use crate::models::{AuthEdit, AuthMethod, Config, Server, ServerEdit};
use crate::ssh::Ssh;
use crate::storage::Storage;
use crate::terminal::Terminal;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Terminal as RattuiTerminal;
use std::io::{self, Write};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq)]
enum MenuItem {
    Browse,
    List,
    Add,
    Connect,
    Remove,
    Config,
    Exit,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum BrowserMode {
    Normal,
    Filter,
}

pub struct Tui {
    current_menu: MenuItem,
    menu_items: Vec<MenuItem>,
    needs_redraw: bool,
}

impl Tui {
    pub fn new() -> Self {
        Tui {
            current_menu: MenuItem::Browse,
            menu_items: vec![
                MenuItem::Browse,
                MenuItem::List,
                MenuItem::Add,
                MenuItem::Connect,
                MenuItem::Remove,
                MenuItem::Config,
                MenuItem::Exit,
            ],
            needs_redraw: true,
        }
    }

    pub fn run(&mut self) -> Result<()> {
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            Hide,
            Clear(ClearType::All),
            MoveTo(0, 0)
        )?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = RattuiTerminal::new(backend)?;

        // Run main loop
        let result = self.main_loop(&mut terminal);

        // Restore terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            Clear(ClearType::All),
            LeaveAlternateScreen,
            Show
        )?;
        terminal.show_cursor()?;

        result
    }

    fn main_loop(
        &mut self,
        terminal: &mut RattuiTerminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<()> {
        loop {
            let mut had_event = false;
            if event::poll(Duration::from_millis(33))? {
                had_event = true;
                match event::read()? {
                    Event::Key(key) => match key.code {
                        KeyCode::Up | KeyCode::Char('k') => self.previous(),
                        KeyCode::Down | KeyCode::Char('j') => self.next(),
                        KeyCode::Enter => {
                            if self.current_menu == MenuItem::Exit {
                                break;
                            }
                            self.handle_selection(terminal)?;
                        }
                        KeyCode::Esc | KeyCode::Char('q') => break,
                        _ => {}
                    },
                    Event::Resize(_, _) => {
                        self.needs_redraw = true;
                    }
                    _ => {}
                }
            }

            // Only redraw when something changed. Idle ticks no longer burn CPU.
            if had_event || self.needs_redraw {
                terminal.draw(|f| {
                    self.draw(f);
                })?;
                self.needs_redraw = false;
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
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(Alignment::Center);
        f.render_widget(banner_widget, chunks[0]);

        // Draw menu
        let menu_items: Vec<ListItem> = self
            .menu_items
            .iter()
            .map(|item| {
                let label = self.get_menu_label(*item);
                if *item == self.current_menu {
                    ListItem::new(format!("▶ {}", label)).style(
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    ListItem::new(format!("  {}", label)).style(Style::default().fg(Color::White))
                }
            })
            .collect();

        let menu_list = List::new(menu_items)
            .block(Block::default().title("Menu").borders(Borders::ALL))
            .style(Style::default().fg(Color::White));

        f.render_widget(menu_list, chunks[1]);

        // Draw instructions
        let instructions = vec![Line::from(vec![
            Span::styled(
                "↑/↓ or j/k",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" to navigate • "),
            Span::styled(
                "Enter",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" to select • "),
            Span::styled(
                "q/Esc",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" to quit"),
        ])];

        let instructions_widget = Paragraph::new(instructions)
            .block(Block::default().borders(Borders::ALL))
            .alignment(Alignment::Center);

        f.render_widget(instructions_widget, chunks[2]);
    }

    fn get_banner() -> String {
        let lines = [
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
            MenuItem::Browse => "Browse Servers".to_string(),
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
        self.current_menu =
            self.menu_items[(idx + self.menu_items.len() - 1) % self.menu_items.len()];
    }

    fn handle_selection(
        &mut self,
        terminal: &mut RattuiTerminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<()> {
        match self.current_menu {
            MenuItem::Browse => self.run_browser(terminal)?,
            MenuItem::List => Self::run_in_plain_screen(terminal, Self::show_list)?,
            MenuItem::Add => Self::run_in_plain_screen(terminal, Self::show_add)?,
            MenuItem::Connect => Self::run_in_plain_screen(terminal, Self::show_connect)?,
            MenuItem::Remove => Self::run_in_plain_screen(terminal, Self::show_remove)?,
            MenuItem::Config => Self::run_in_plain_screen(terminal, Self::show_config)?,
            MenuItem::Exit => {}
        }
        self.needs_redraw = true;
        Ok(())
    }

    fn run_in_plain_screen<F>(
        terminal: &mut RattuiTerminal<CrosstermBackend<io::Stdout>>,
        action: F,
    ) -> Result<()>
    where
        F: FnOnce() -> Result<()>,
    {
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            Clear(ClearType::All),
            LeaveAlternateScreen,
            Show
        )?;

        let action_result = action();
        if let Err(e) = action_result {
            println!("\nError: {}", e);
        }

        println!("\nPress Enter to return...");
        let mut input = String::new();
        let _ = io::stdin().read_line(&mut input);

        enable_raw_mode()?;
        execute!(terminal.backend_mut(), EnterAlternateScreen, Hide)?;
        terminal.clear()?;

        Ok(())
    }

    fn connect_to_server(server: &Server, config: &Config) -> Result<()> {
        println!("\n{}", "=".repeat(60));
        println!("{:^60}", "CONNECT TO SERVER");
        println!("{}", "=".repeat(60));

        let terminal_name = Terminal::get_terminal_command(config.preferred_terminal.as_deref())?;
        let jump = crate::models::resolve_jump(config, server)?;

        println!("\nConnecting to {}...", server.name);
        Ssh::launch_connection(server, &terminal_name, jump.as_deref())?;
        Ok(())
    }

    fn prompt_delete_server(server_name: &str) -> Result<()> {
        println!("\n{}", "=".repeat(60));
        println!("{:^60}", "REMOVE SERVER");
        println!("{}", "=".repeat(60));

        let mut config = Storage::load_config()?;
        Storage::validate_server_removal(&config, server_name)?;

        print!(
            "\nAre you sure you want to remove '{}'? (y/n): ",
            server_name
        );
        io::stdout().flush()?;
        let mut confirm = String::new();
        io::stdin().read_line(&mut confirm)?;

        if confirm.trim().to_lowercase() != "y" {
            println!("Cancelled.");
            return Ok(());
        }

        config.servers.retain(|s| s.name != server_name);
        Storage::save_config(&config)?;
        let _ = Ssh::delete_password(server_name);

        println!("\n✓ Server '{}' removed successfully!", server_name);
        Ok(())
    }

    fn prompt_edit_server(server: &Server) -> Result<()> {
        println!("\n{}", "=".repeat(60));
        println!("{:^60}", format!("EDIT SERVER: {}", server.name));
        println!("{}", "=".repeat(60));

        print!("\nServer name [{}]: ", server.name);
        io::stdout().flush()?;
        let mut new_name = String::new();
        io::stdin().read_line(&mut new_name)?;
        let new_name = if new_name.trim().is_empty() {
            None
        } else {
            Some(new_name.trim().to_string())
        };

        print!("Host (IP or domain) [{}]: ", server.host);
        io::stdout().flush()?;
        let mut host = String::new();
        io::stdin().read_line(&mut host)?;
        let host = if host.trim().is_empty() {
            None
        } else {
            Some(host.trim().to_string())
        };

        print!("Username [{}]: ", server.username);
        io::stdout().flush()?;
        let mut username = String::new();
        io::stdin().read_line(&mut username)?;
        let username = if username.trim().is_empty() {
            None
        } else {
            Some(username.trim().to_string())
        };

        print!("Port [{}]: ", server.port);
        io::stdout().flush()?;
        let mut port_str = String::new();
        io::stdin().read_line(&mut port_str)?;
        let port = if port_str.trim().is_empty() {
            None
        } else {
            let p: u16 = port_str
                .trim()
                .parse()
                .map_err(|_| SshManagerError::InvalidPort)?;
            Some(p)
        };

        let current_auth_desc = match server.auth_method {
            AuthMethod::Key => "1 (Key)",
            AuthMethod::Password => "2 (Password)",
        };
        println!("\nAuthentication method [current: {}]:", current_auth_desc);
        println!("1. SSH Key");
        println!("2. Password");
        print!("Choose (1 or 2, press Enter to keep current): ");
        io::stdout().flush()?;
        let mut auth_choice = String::new();
        io::stdin().read_line(&mut auth_choice)?;
        let auth_choice = auth_choice.trim();

        let (auth, new_password) = match auth_choice {
            "1" => {
                let current_key = server.key_path.as_deref().unwrap_or("");
                if current_key.is_empty() {
                    print!("SSH key path: ");
                } else {
                    print!("SSH key path [{}]: ", current_key);
                }
                io::stdout().flush()?;
                let mut key_path = String::new();
                io::stdin().read_line(&mut key_path)?;
                let key_path = key_path.trim();
                let chosen_key = if key_path.is_empty() {
                    if current_key.is_empty() {
                        return Err(SshManagerError::InvalidConfig(
                            "SSH key path cannot be empty".to_string(),
                        ));
                    }
                    current_key.to_string()
                } else {
                    key_path.to_string()
                };
                (Some(AuthEdit::Key(chosen_key)), None)
            }
            "2" => {
                if server.auth_method == AuthMethod::Password {
                    println!("\nEnter new password (press Enter to keep current):");
                    let pw = rpassword::read_password().map_err(|e| {
                        SshManagerError::InvalidConfig(format!("Failed to read password: {}", e))
                    })?;
                    if pw.is_empty() {
                        (None, None)
                    } else {
                        (Some(AuthEdit::Password), Some(pw))
                    }
                } else {
                    println!("\nEnter password:");
                    let pw = rpassword::read_password().map_err(|e| {
                        SshManagerError::InvalidConfig(format!("Failed to read password: {}", e))
                    })?;
                    if pw.is_empty() {
                        return Err(SshManagerError::InvalidConfig(
                            "Password cannot be empty".to_string(),
                        ));
                    }
                    (Some(AuthEdit::Password), Some(pw))
                }
            }
            "" => (None, None),
            _ => {
                return Err(SshManagerError::InvalidConfig("Invalid choice".to_string()));
            }
        };

        let current_af_desc = if server.use_agent_forwarding {
            "y"
        } else {
            "n"
        };
        print!("Enable agent forwarding? (y/n) [{}]: ", current_af_desc);
        io::stdout().flush()?;
        let mut af_choice = String::new();
        io::stdin().read_line(&mut af_choice)?;
        let af_choice = af_choice.trim().to_lowercase();
        let agent_forwarding = if af_choice.is_empty() {
            None
        } else {
            Some(af_choice == "y" || af_choice == "yes")
        };

        let current_tags_desc = if server.tags.is_empty() {
            "none".to_string()
        } else {
            server.tags.join(", ")
        };
        print!("Tags (comma-separated) [{}]: ", current_tags_desc);
        io::stdout().flush()?;
        let mut tags_str = String::new();
        io::stdin().read_line(&mut tags_str)?;
        let tags = if tags_str.trim().is_empty() {
            None
        } else {
            let parsed: Vec<String> = tags_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            Some(Storage::normalize_tags(&parsed)?)
        };

        let current_jump_desc = server.jump_host.as_deref().unwrap_or("none");
        print!("Jump host [{}]: ", current_jump_desc);
        io::stdout().flush()?;
        let mut jump_str = String::new();
        io::stdin().read_line(&mut jump_str)?;
        let jump_trimmed = jump_str.trim();
        let jump_host = if jump_trimmed.is_empty() {
            None
        } else if jump_trimmed == "-" {
            Some(None)
        } else {
            Some(Some(jump_trimmed.to_string()))
        };

        let edit = ServerEdit {
            new_name,
            host,
            username,
            port,
            auth,
            agent_forwarding,
            tags,
            jump_host,
        };

        let mut config = Storage::load_config()?;
        Storage::execute_server_edit(&mut config, &server.name, edit, new_password)?;
        Ok(())
    }

    fn run_browser(
        &mut self,
        terminal: &mut RattuiTerminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<()> {
        let mut config = Storage::load_config()?;
        let mut selected_index: usize = 0;
        let mut filter_query = String::new();
        let mut mode = BrowserMode::Normal;
        let mut needs_browser_redraw = true;

        loop {
            let filtered_indices =
                crate::models::filter_server_indices(&config.servers, &filter_query);

            if filtered_indices.is_empty() {
                selected_index = 0;
            } else if selected_index >= filtered_indices.len() {
                selected_index = filtered_indices.len() - 1;
            }

            let mut had_event = false;
            if event::poll(Duration::from_millis(33))? {
                had_event = true;
                match event::read()? {
                    Event::Key(key) => match mode {
                        BrowserMode::Normal => match key.code {
                            KeyCode::Down | KeyCode::Char('j')
                                if !filtered_indices.is_empty()
                                    && selected_index + 1 < filtered_indices.len() =>
                            {
                                selected_index += 1;
                                needs_browser_redraw = true;
                            }
                            KeyCode::Up | KeyCode::Char('k') if selected_index > 0 => {
                                selected_index -= 1;
                                needs_browser_redraw = true;
                            }
                            KeyCode::Enter if !filtered_indices.is_empty() => {
                                let server =
                                    config.servers[filtered_indices[selected_index]].clone();
                                let cfg_clone = config.clone();
                                Self::run_in_plain_screen(terminal, || {
                                    Self::connect_to_server(&server, &cfg_clone)
                                })?;
                                config = Storage::load_config()?;
                                needs_browser_redraw = true;
                            }
                            KeyCode::Char('a') => {
                                Self::run_in_plain_screen(terminal, Self::show_add)?;
                                config = Storage::load_config()?;
                                needs_browser_redraw = true;
                            }
                            KeyCode::Char('e') if !filtered_indices.is_empty() => {
                                let server =
                                    config.servers[filtered_indices[selected_index]].clone();
                                Self::run_in_plain_screen(terminal, || {
                                    Self::prompt_edit_server(&server)
                                })?;
                                config = Storage::load_config()?;
                                needs_browser_redraw = true;
                            }
                            KeyCode::Char('d') if !filtered_indices.is_empty() => {
                                let server_name = config.servers[filtered_indices[selected_index]]
                                    .name
                                    .clone();
                                Self::run_in_plain_screen(terminal, || {
                                    Self::prompt_delete_server(&server_name)
                                })?;
                                config = Storage::load_config()?;
                                needs_browser_redraw = true;
                            }
                            KeyCode::Char('/') => {
                                mode = BrowserMode::Filter;
                                needs_browser_redraw = true;
                            }
                            KeyCode::Esc => {
                                if !filter_query.is_empty() {
                                    filter_query.clear();
                                    needs_browser_redraw = true;
                                } else {
                                    break;
                                }
                            }
                            KeyCode::Char('q') => {
                                break;
                            }
                            _ => {}
                        },
                        BrowserMode::Filter => match key.code {
                            KeyCode::Enter => {
                                mode = BrowserMode::Normal;
                                needs_browser_redraw = true;
                            }
                            KeyCode::Esc => {
                                filter_query.clear();
                                mode = BrowserMode::Normal;
                                needs_browser_redraw = true;
                            }
                            KeyCode::Backspace => {
                                filter_query.pop();
                                needs_browser_redraw = true;
                            }
                            KeyCode::Char(c) => {
                                filter_query.push(c);
                                needs_browser_redraw = true;
                            }
                            _ => {}
                        },
                    },
                    Event::Resize(_, _) => {
                        needs_browser_redraw = true;
                    }
                    _ => {}
                }
            }

            if had_event || needs_browser_redraw {
                terminal.draw(|f| {
                    Self::draw_browser(
                        f,
                        &config,
                        &filtered_indices,
                        selected_index,
                        &filter_query,
                        mode,
                    );
                })?;
                needs_browser_redraw = false;
            }
        }

        self.needs_redraw = true;
        Ok(())
    }

    fn draw_browser(
        f: &mut ratatui::Frame,
        config: &Config,
        filtered_indices: &[usize],
        selected_index: usize,
        filter_query: &str,
        mode: BrowserMode,
    ) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(2)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(10),
                Constraint::Length(3),
            ])
            .split(f.size());

        // Header
        let title = if filter_query.is_empty() {
            format!(
                "SSHER SERVER BROWSER  •  Servers ({})",
                config.servers.len()
            )
        } else {
            format!(
                "SSHER SERVER BROWSER  •  Servers (matching {} of {})",
                filtered_indices.len(),
                config.servers.len()
            )
        };
        let header = Paragraph::new(Line::from(vec![Span::styled(
            title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )]))
        .block(Block::default().borders(Borders::ALL))
        .alignment(Alignment::Center);
        f.render_widget(header, chunks[0]);

        // Server list
        if config.servers.is_empty() {
            let empty_msg = Paragraph::new("No servers configured. Press a to add.")
                .style(Style::default().fg(Color::Yellow))
                .block(Block::default().title("Servers").borders(Borders::ALL))
                .alignment(Alignment::Center);
            f.render_widget(empty_msg, chunks[1]);
        } else if filtered_indices.is_empty() {
            let no_match_msg = Paragraph::new(format!(
                "No servers match filter '{}'. Press Esc to clear filter.",
                filter_query
            ))
            .style(Style::default().fg(Color::Yellow))
            .block(Block::default().title("Servers").borders(Borders::ALL))
            .alignment(Alignment::Center);
            f.render_widget(no_match_msg, chunks[1]);
        } else {
            let col_header = format!(
                "  {:<18} {:<22} {:<12} {:<6} {:<10} {:<10} {:<15} {}",
                "NAME", "HOST", "USER", "PORT", "AUTH", "AGENT FW", "VIA", "TAGS"
            );
            let separator = format!("  {}", "─".repeat(105));

            let mut items = Vec::new();
            items.push(
                ListItem::new(col_header).style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
            );
            items.push(ListItem::new(separator).style(Style::default().fg(Color::DarkGray)));

            for (view_idx, &real_idx) in filtered_indices.iter().enumerate() {
                let s = &config.servers[real_idx];
                let auth_str = match s.auth_method {
                    AuthMethod::Key => "key",
                    AuthMethod::Password => "password",
                };
                let af_str = if s.use_agent_forwarding { "yes" } else { "no" };
                let via_str = s.jump_host.as_deref().unwrap_or("-");
                let tags_str = if s.tags.is_empty() {
                    "-".to_string()
                } else {
                    s.tags.join(", ")
                };

                if view_idx == selected_index {
                    let line = format!(
                        "▶ {:<18} {:<22} {:<12} {:<6} {:<10} {:<10} {:<15} {}",
                        s.name, s.host, s.username, s.port, auth_str, af_str, via_str, tags_str
                    );
                    items.push(
                        ListItem::new(line).style(
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ),
                    );
                } else {
                    let line = format!(
                        "  {:<18} {:<22} {:<12} {:<6} {:<10} {:<10} {:<15} {}",
                        s.name, s.host, s.username, s.port, auth_str, af_str, via_str, tags_str
                    );
                    items.push(ListItem::new(line).style(Style::default().fg(Color::White)));
                }
            }

            let mut list_state = ratatui::widgets::ListState::default();
            list_state.select(Some(selected_index + 2));
            let list_widget =
                List::new(items).block(Block::default().title("Servers").borders(Borders::ALL));
            f.render_stateful_widget(list_widget, chunks[1], &mut list_state);
        }

        // Footer
        let footer_widget = match mode {
            BrowserMode::Normal => {
                let mut spans = Vec::new();
                if !filter_query.is_empty() {
                    spans.push(Span::styled(
                        "Filter: ",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ));
                    spans.push(Span::styled(
                        format!("\"{}\" • ", filter_query),
                        Style::default().fg(Color::Yellow),
                    ));
                }
                spans.extend([
                    Span::styled(
                        "j/k/↑/↓",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" Move • "),
                    Span::styled(
                        "Enter",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" Connect • "),
                    Span::styled(
                        "a",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" Add • "),
                    Span::styled(
                        "e",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" Edit • "),
                    Span::styled(
                        "d",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" Delete • "),
                    Span::styled(
                        "/",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" Filter • "),
                    Span::styled(
                        "Esc/q",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" Back"),
                ]);
                Paragraph::new(Line::from(spans))
                    .block(Block::default().borders(Borders::ALL))
                    .alignment(Alignment::Center)
            }
            BrowserMode::Filter => {
                let spans = vec![
                    Span::styled(
                        "Filter: ",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        filter_query,
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("█", Style::default().fg(Color::Yellow)),
                    Span::raw("   "),
                    Span::styled(
                        "Enter",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" Apply • "),
                    Span::styled(
                        "Esc",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" Clear • "),
                    Span::styled(
                        "Backspace",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" Delete"),
                ];
                Paragraph::new(Line::from(spans))
                    .block(Block::default().borders(Borders::ALL))
                    .alignment(Alignment::Center)
            }
        };
        f.render_widget(footer_widget, chunks[2]);
    }

    fn show_list() -> Result<()> {
        let config = crate::storage::Storage::load_config()?;

        println!("\n{}", "=".repeat(122));
        println!("{:^122}", "SSH SERVERS");
        println!("{}", "=".repeat(122));

        if config.servers.is_empty() {
            println!("\nNo servers configured.\n");
            return Ok(());
        }

        println!(
            "{:<20} {:<20} {:<15} {:<8} {:<10} {:<10} {:<16} {:<20}",
            "Name", "Host", "Username", "Port", "Auth", "Agent FW", "Via", "Tags"
        );
        println!("{}", "─".repeat(122));

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
            let via_str = server.jump_host.as_deref().unwrap_or("-");
            let tags_str = if server.tags.is_empty() {
                "-".to_string()
            } else {
                server.tags.join(", ")
            };

            println!(
                "{:<20} {:<20} {:<15} {:<8} {:<10} {:<10} {:<16} {:<20}",
                server.name,
                server.host,
                server.username,
                server.port,
                auth,
                agent_fw,
                via_str,
                tags_str
            );
        }
        println!();

        Ok(())
    }

    fn show_add() -> Result<()> {
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

        print!("Tags (comma-separated, optional): ");
        io::stdout().flush()?;
        let mut tags_str = String::new();
        io::stdin().read_line(&mut tags_str)?;
        let tags = if tags_str.trim().is_empty() {
            Vec::new()
        } else {
            let parsed: Vec<String> = tags_str
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            Storage::normalize_tags(&parsed)?
        };

        // Validate inputs
        Storage::validate_server_name(name)?;
        Storage::validate_host(host)?;
        Storage::validate_username(username)?;
        Storage::validate_port(port)?;

        let mut config = Storage::load_config()?;

        if !Storage::server_name_is_unique(&config, name, None) {
            return Err(SshManagerError::ServerAlreadyExists(name.to_string()));
        }

        print!("Jump host (saved server name or [user@]host[:port], optional): ");
        io::stdout().flush()?;
        let mut jump_str = String::new();
        io::stdin().read_line(&mut jump_str)?;
        let jump_trimmed = jump_str.trim();
        let jump_host = if jump_trimmed.is_empty() {
            None
        } else {
            Storage::validate_server_jump(&config, name, jump_trimmed)?;
            Some(jump_trimmed.to_string())
        };

        let mut server = Server::new(
            name.to_string(),
            host.to_string(),
            username.to_string(),
            port,
            auth_method,
            key_path,
        );
        server.use_agent_forwarding = agent_forwarding;
        server.tags = tags;
        server.jump_host = jump_host;

        config.servers.push(server);
        Storage::save_config(&config)?;

        println!("\n✓ Server '{}' added successfully!", name);

        Ok(())
    }

    fn show_connect() -> Result<()> {
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
        let jump = crate::models::resolve_jump(&config, server)?;

        println!("\nConnecting to {}...", server.name);
        Ssh::launch_connection(server, &terminal_name, jump.as_deref())?;

        Ok(())
    }

    fn show_remove() -> Result<()> {
        println!("\n{}", "=".repeat(60));
        println!("{:^60}", "REMOVE SERVER");
        println!("{}", "=".repeat(60));

        let mut config = Storage::load_config()?;

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
        Storage::validate_server_removal(&config, &server_name)?;

        print!("Are you sure? (y/n): ");
        io::stdout().flush()?;
        let mut confirm = String::new();
        io::stdin().read_line(&mut confirm)?;

        if confirm.trim().to_lowercase() != "y" {
            println!("Cancelled.");
            return Ok(());
        }

        config.servers.retain(|s| s.name != server_name);
        Storage::save_config(&config)?;
        let _ = Ssh::delete_password(&server_name);

        println!("\n✓ Server '{}' removed successfully!", server_name);

        Ok(())
    }

    fn show_config() -> Result<()> {
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
            print!("\nTerminal name (current, gnome-terminal, xterm, etc.): ");
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
