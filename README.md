# ssher

A Linux CLI application to manage and launch SSH connections to saved servers. Securely store server configurations, credentials, and launch connections with a simple command.

## Features

- **Server Management**: Add, list, and remove SSH server configurations
- **Flexible Authentication**: Support both SSH key and password-based authentication
- **Secure Password Storage**: Passwords stored in OS keyring, never in plain text
- **Terminal Flexibility**: User-configurable terminal emulator (gnome-terminal, xterm, etc.)
- **SSH Options**: Support for custom ports, SSH agent forwarding, and identity files
- **Shell Completions**: Native completions for Bash, Zsh, Fish, Elvish, and PowerShell, with dynamic server-name completion
- **JSON Configuration**: Server configs stored in `~/.config/ssh_manager/servers.json`

## Requirements

### System Requirements
- Linux operating system
- Rust 1.70+ (for building)
- One of the following terminal emulators: `gnome-terminal`, `konsole`, `xfce4-terminal`, or `xterm`
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

Connect by full server name or unambiguous partial name:
```bash
ssher connect myserver
ssher connect my       # connects to myserver if it is the only matching server
```

Options:
- `-c`, `--current`: Connect directly in the current terminal instead of spawning an external window

This will:
1. Open a new terminal window (or connect in the current terminal with `-c`)
2. Execute the SSH command with the stored credentials
3. Keep the terminal open after disconnection for review

Exact name matches always take precedence. If multiple servers match a partial query, an error lists all candidate matches.

### Edit a Server

Modify an existing server configuration in place:

```bash
# Update host and port
ssher edit myserver --host new.example.com --port 2222

# Rename a server (automatically transfers keyring password if using password auth)
ssher edit myserver --new-name prodserver

# Switch to key authentication
ssher edit myserver --key /path/to/key

# Switch to password authentication (interactive password prompt)
ssher edit myserver --password

# Enable or disable agent forwarding
ssher edit myserver --agent-forwarding false
```

### Remove a Server

```bash
ssher remove myserver
```

This will remove the server configuration and clean up any stored password from the keyring.

### Interactive TUI & Server Browser

Launch the interactive TUI by running `ssher` with no arguments:
```bash
ssher
```

Select **Browse Servers** to open the in-terminal server browser.

#### Browser Keybindings:
- `j` / `k` or `↓` / `↑`: Move selection up/down
- `Enter`: Connect to selected server
- `a`: Add a new server
- `e`: Edit selected server in place (prompts show current values in brackets)
- `d`: Delete selected server (with y/n confirmation)
- `/`: Filter servers by name or host (case-insensitive substring)
  - In filter mode: typed characters append to query, `Backspace` deletes, `Enter` applies filter, `Esc` clears filter
- `Esc` / `q`: Return to main menu (in normal mode, `Esc` clears an active filter first if present)

### Configure Terminal

Set your preferred terminal emulator:
```bash
ssher config terminal gnome-terminal
ssher config terminal xterm
```

The application will use your preferred terminal for all connections. If the preferred terminal is not available, it will fall back to the first available option (gnome-terminal, then xterm).

### Shell Completions

Generate completion scripts for your shell using `ssher completions`:

```bash
ssher completions <bash|zsh|fish|elvish|powershell>
```

#### Quick Setup

**Bash:**
```bash
# Add to ~/.bashrc:
source <(ssher completions bash)
```
Or persist into your user completions directory:
```bash
ssher completions bash > ~/.local/share/bash-completion/completions/ssher
```

**Zsh:**
```zsh
# Add to ~/.zshrc:
source <(ssher completions zsh)
```
Or save to a completion directory in your `$fpath` (e.g. `~/.zfunc/_ssher`):
```zsh
ssher completions zsh > ~/.zfunc/_ssher
```

**Fish:**
```fish
# Add to ~/.config/fish/config.fish:
ssher completions fish | source
```
Or persist into your fish completions folder:
```fish
ssher completions fish > ~/.config/fish/completions/ssher.fish
```

**Elvish:**
```elvish
eval (ssher completions elvish | slurp)
```

**PowerShell:**
```powershell
ssher completions powershell | Out-String | Invoke-Expression
```

#### Dynamic Server Name Completion

For Bash, Zsh, and Fish, the generated completion scripts automatically wire dynamic completion of saved server names for `connect`, `edit`, and `remove` via the hidden subcommand:

```bash
ssher __complete-servers
```

`ssher __complete-servers` prints saved server names one per line (and produces no output with exit status 0 if configuration is missing or unreadable).

If you generate static base completions or wish to wire dynamic server completion manually, append or source the snippet for your shell:

**Bash snippet:**
```bash
_ssher_complete() {
    local cur="${2:-${COMP_WORDS[COMP_CWORD]}}"
    local prev="${3:-${COMP_WORDS[COMP_CWORD-1]}}"

    case "${prev}" in
        --host|--username|--port|--key|--agent-forwarding|--new-name)
            _ssher "$@"
            return 0
            ;;
    esac

    local subcmd=""
    for (( i=1; i < COMP_CWORD; i++ )); do
        case "${COMP_WORDS[i]}" in
            connect|edit|remove)
                subcmd="${COMP_WORDS[i]}"
                break
                ;;
            add|list|completions|config|help)
                break
                ;;
        esac
    done

    if [[ -n "${subcmd}" && "${cur}" != -* ]]; then
        local servers
        servers="$(ssher __complete-servers 2>/dev/null)"
        if [[ -n "${servers}" ]]; then
            COMPREPLY=( $(compgen -W "${servers}" -- "${cur}") )
            return 0
        fi
    fi

    _ssher "$@"
}

if [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -ge 4 || "${BASH_VERSINFO[0]}" -gt 4 ]]; then
    complete -F _ssher_complete -o nosort -o bashdefault -o default ssher
else
    complete -F _ssher_complete -o bashdefault -o default ssher
fi
```

**Zsh snippet:**
```zsh
_ssher_servers() {
    local -a servers
    servers=(${(f)"$(ssher __complete-servers 2>/dev/null)"})
    _describe -t servers 'server' servers "$@"
}
```

**Fish snippet:**
```fish
complete -c ssher -n "__fish_ssher_using_subcommand connect edit remove" -f -a "(ssher __complete-servers)"
```

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

Passwords are **never** stored in plain text on disk. When connecting, the
password is delivered to `ssh` through `SSH_ASKPASS` — it never appears in
the parent process's command line, environment of the spawned terminal, or
the system process table.

## Error Handling

The application provides clear error messages for common issues:

- `Server not found: myserver` — The specified server doesn't exist
- `SSH key file not found: /path/to/key` — Key file path is invalid
- `Terminal not found` — None of the supported terminal emulators are installed
- `Failed to set up SSH_ASKPASS helper: ...` — Could not write the temporary
  askpass helper (used for password auth)
- `Invalid configuration: ...` — Config file is corrupted or invalid

## Command Reference

```bash
# Add servers
ssher add <name> <host> <username> [OPTIONS]
  --key <path>                  SSH key file path
  --password                    Use password authentication (interactive prompt)
  --port <port>                Default: 22
  --agent-forwarding           Enable SSH agent forwarding

# Edit a server
ssher edit <name> [OPTIONS]
  --host <host>                 New server host
  --username <username>         New SSH username
  --port <port>                 New SSH port
  --key <path>                  New SSH key path (switches to key auth)
  --password                    Set new password (interactive prompt)
  --agent-forwarding <true|false> Enable or disable SSH agent forwarding
  --new-name <name>             New server identifier name

# List servers
ssher list

# Connect to a server (exact name or unambiguous partial match)
ssher connect <name> [OPTIONS]
  -c, --current                 Connect inline in current terminal

# Remove a server
ssher remove <name>

# Configure settings
ssher config terminal <terminal-name>

# Generate shell completion script
ssher completions <bash|zsh|fish|elvish|powershell>
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

The full test suite is run in CI (`.github/workflows/ci.yml`) and covers
config validation, serde round-trips, SSH argument construction, the
`SSH_ASKPASS` helper setup, and the terminal-emulator discovery logic.

## Troubleshooting

### "Terminal not found" error
Install one of the supported terminal emulators:
```bash
# Ubuntu/Debian
sudo apt install gnome-terminal xterm

# Fedora
sudo dnf install gnome-terminal xterm

# Arch
sudo pacman -S gnome-terminal xterm
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
