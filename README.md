# ssher

A Linux CLI application to manage and launch SSH connections to saved servers. Securely store server configurations, credentials, and launch connections with a simple command.

## Features

- **Server Management**: Add, list, and remove SSH server configurations
- **Flexible Authentication**: Support both SSH key and password-based authentication
- **Secure Password Storage**: Passwords stored in OS keyring, never in plain text
- **Terminal Flexibility**: User-configurable terminal emulator (gnome-terminal, xterm, etc.)
- **SSH Options**: Support for custom ports, SSH agent forwarding, and identity files
- **JSON Configuration**: Server configs stored in `~/.config/ssh_manager/servers.json`

## Requirements

### System Requirements
- Linux operating system
- Rust 1.70+ (for building)
- One of the following terminal emulators: `gnome-terminal` or `xterm`
- For password-based authentication: `sshpass` package

### Optional

- `openssh-client` (usually pre-installed on most Linux distributions)

## Installation

### From Source

1. Clone the repository:
   ```bash
   cd ~/Documents/Projects/ssh-manager
   ```

2. Build the project:
   ```bash
   cargo build --release
   ```

3. Install the binary (optional):
   ```bash
   # Install to ~/.cargo/bin/ (make sure ~/.cargo/bin is in your PATH)
   cargo install --path .
   ```

   Or run directly:
   ```bash
   ./target/release/ssh-manager <command>
   ```

### Setup

The application automatically creates `~/.config/ssh_manager/` on first run.

## Usage

### Add a Server

Add a server with SSH key authentication:
```bash
ssher add myserver example.com username --key /path/to/key
```

Add a server with password authentication:
```bash
ssher add myserver example.com username --password
```

Add a server with custom port and agent forwarding:
```bash
ssher add myserver example.com username --key /path/to/key --port 2222 --agent-forwarding
```

### List All Servers

```bash
ssher list
```

Output:
```
Name                 Host                 Username        Port       Auth         Agent FW
─────────────────────────────────────────────────────────────────────────────────────────
myserver             example.com          username        22         key          no
prod-server          prod.example.com     root            22         password     yes
```

### Connect to a Server

```bash
ssher connect myserver
```

This will:
1. Open a new terminal window
2. Execute the SSH command with the stored credentials
3. Keep the terminal open after disconnection for review

### Remove a Server

```bash
ssher remove myserver
```

This will remove the server configuration and clean up any stored password from the keyring.

### Configure Terminal

Set your preferred terminal emulator:
```bash
ssher config terminal gnome-terminal
ssher config terminal xterm
```

The application will use your preferred terminal for all connections. If the preferred terminal is not available, it will fall back to the first available option (gnome-terminal, then xterm).

## Configuration Files

All configuration is stored in `~/.config/ssh_manager/`:

- **servers.json**: Stores server configurations (JSON format)
- **terminal_config.json**: Stores user's terminal preference

### Example servers.json

```json
[
  {
    "name": "myserver",
    "host": "example.com",
    "username": "username",
    "port": 22,
    "auth_method": "key",
    "key_path": "/home/user/.ssh/id_rsa",
    "use_agent_forwarding": false
  },
  {
    "name": "prod-server",
    "host": "prod.example.com",
    "username": "root",
    "port": 22,
    "auth_method": "password",
    "key_path": null,
    "use_agent_forwarding": true
  }
]
```

## Password Security

Passwords are stored securely in your system's keyring:
- **Linux with GNOME/KDE**: Stored in Secret Service
- **Other Linux systems**: May use pass, kwallet, or other keyring providers

Passwords are **never** stored in plain text on disk.

## Error Handling

The application provides clear error messages for common issues:

- `Server not found: myserver` — The specified server doesn't exist
- `SSH key file not found: /path/to/key` — Key file path is invalid
- `Terminal not found` — Neither gnome-terminal nor xterm is installed
- `sshpass is not installed` — Required for password authentication
- `Invalid configuration: ...` — Config file is corrupted or invalid

## Command Reference

```bash
# Add servers
ssher add <name> <host> <username> [OPTIONS]
  --key <path>                  SSH key file path
  --password                    Use password authentication (interactive prompt)
  --port <port>                Default: 22
  --agent-forwarding           Enable SSH agent forwarding

# List servers
ssher list

# Connect to a server
ssher connect <name>

# Remove a server
ssher remove <name>

# Configure settings
ssher config terminal <terminal-name>
```

## Building from Source

Requirements:
- Rust 1.70 or later (install from https://rustup.rs/)

Build steps:
```bash
cargo build --release
```

Run tests:
```bash
cargo test
```

## Troubleshooting

### "Terminal not found" error
Install gnome-terminal or xterm:
```bash
# Ubuntu/Debian
sudo apt install gnome-terminal xterm

# Fedora
sudo dnf install gnome-terminal xterm

# Arch
sudo pacman -S gnome-terminal xterm
```

### "sshpass is not installed" error
Install sshpass (required only for password-based authentication):
```bash
# Ubuntu/Debian
sudo apt install sshpass

# Fedora
sudo dnf install sshpass

# Arch
sudo pacman -S sshpass
```

### Password not being stored in keyring
Ensure your system has a working keyring service:
- **GNOME/KDE systems**: Usually pre-configured
- **Other systems**: Install and start your keyring provider (e.g., `pass`, `kwallet`)

### SSH key not found error
Verify the key file path:
```bash
ls -la /path/to/key
```

The path must be absolute and the file must exist and be readable.

## Development

To build in debug mode:
```bash
cargo build
```

To run with debug output:
```bash
./target/debug/ssh-manager <command>
```

## License

MIT

## Author

SSH Manager Contributors
