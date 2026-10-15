# ssher

A Linux CLI application to manage and launch SSH connections to saved servers. Securely store server configurations, credentials, and launch connections with a simple command.

## Features

- **Server Management**: Add, list, and remove SSH server configurations
- **Flexible Authentication**: Support both SSH key and password-based authentication
- **Jump Host / ProxyJump**: Support for bastion/jump hosts via saved server references or raw SSH jump specs, with chain resolution and cycle detection
- **Port Forwarding Presets**: Local (-L), Remote (-R), and Dynamic (-D) port forwarding presets and ad-hoc connection forwards with port shorthand support
- **tmux Session Attach**: Attach or create remote tmux sessions automatically on connection with per-server defaults, ad-hoc session overrides, and interactive TUI prompts
- **Tags & Filtering**: Organize servers with tags, filter by tag sets, and search tags in the browser
- **Favourites & Recency Tracking**: Mark servers as favourites, track last used connection timestamps, and sort by recent usage or alphabetically
- **Secure Password Storage**: Passwords stored in OS keyring, never in plain text
- **Terminal Flexibility**: User-configurable terminal emulator (gnome-terminal, xterm, etc.)
- **SSH Options**: Support for custom ports, SSH agent forwarding, identity files, and jump hosts
- **SSH Config Import**: Import host configurations from `~/.ssh/config` or custom files with dry-run support
- **Export & Backup**: Export servers and configuration to pretty JSON (`ssher export`) to file or stdout (with mode 0600 permissions), and import backups (`ssher import-backup`) with validation, collision handling, and password-auth reminders
- **Shell Completions**: Native completions for Bash, Zsh, Fish, Elvish, and PowerShell, with dynamic server-name and jump host completion
- **Remote Command Execution**: Run one-off remote commands inline (`ssher exec`) on single servers, tag sets, or all servers with sequential execution and BatchMode protection
- **Public Key Installation**: Install public SSH keys on saved servers (`ssher copy-id`), with jump host support, `SSH_ASKPASS` password delivery, and optional switching to key authentication (`--switch-to-key`)
- **File Transfer (SCP)**: Upload and download files or directories (`ssher put` and `ssher get`) wrapping `scp` with recursive transfer (`-r`), jump host support, `SSH_ASKPASS` password delivery, and automatic IPv6 formatting
- **Health Check (TCP Ping)**: Check TCP reachability (`ssher ping`) of saved servers' host:port concurrently (capped at 16 threads), reporting connection latency, refused, timeout, or DNS failure with non-zero exit code on failure, plus single-server ping in the TUI browser (`p`)
- **JSON Configuration**: Server configs stored in `~/.config/ssh_manager/servers.json`

## Requirements

### System Requirements
- Linux operating system
- Rust 1.70+ (for building)
- One of the following terminal emulators: `gnome-terminal`, `konsole`, `xfce4-terminal`, or `xterm`
- `openssh-client` (providing `ssh`, `scp`, and `ssh-copy-id`, usually pre-installed on most Linux distributions)

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

Add a server with SSH key authentication and tags:
```bash
ssher add myserver example.com username --key /path/to/key --tag prod --tag web
```

Add a server with password authentication:
```bash
ssher add myserver example.com username --password
```

Add a server with custom port and agent forwarding:
```bash
ssher add myserver example.com username --key /path/to/key --port 2222 --agent-forwarding
```

Add a server with a jump host (either a saved server name or a raw `[user@]host[:port]` spec):
```bash
ssher add internal-node 10.0.0.5 admin --key /path/to/key --jump bastion
ssher add internal-node 10.0.0.5 admin --key /path/to/key --jump jumpuser@jump.example.com:2222
```

Add a server with port forwarding presets (repeatable `--forward`):
```bash
# Shorthand bare-port notation expands to L:<port>:localhost:<port>
ssher add db-server 10.0.0.6 dbuser --key /path/to/key --forward 5432

# Explicit Local (-L), Remote (-R), and Dynamic SOCKS (-D) forwards
ssher add web-server 10.0.0.7 admin --key /path/to/key \
  --forward L:8080:localhost:80 \
  --forward R:2222:localhost:22 \
  --forward D:1080

# Add a server with a default tmux session
ssher add dev-server 10.0.0.8 developer --key /path/to/key --tmux dev
```

### List All Servers

List all configured servers (defaults to `--sort recent` with favourites first, then most recently used):
```bash
ssher list
```

Sort alphabetically by server name:
```bash
ssher list --sort name
```

Filter servers by tag (repeatable; matches servers having all specified tags):
```bash
ssher list --tag prod --tag web
```

Output:
```
Name                 Host                 Username        Port       Auth         Agent FW   Via              Fwd      Tmux       Fav   Last used    Tags
──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
myserver             example.com          username        22         key          no         -                1        -          *     3h ago       prod, web
internal-node        10.0.0.5             admin           22         key          no         bastion          -        dev        -     never        -
prod-server          prod.example.com     root            22         password     yes        -                -        -          -     never        -
```

### Favorite and Unfavorite Servers

Mark a server as a favorite:
```bash
ssher fav myserver
```

Remove a server from favorites:
```bash
ssher unfav myserver
```

Favorite servers are marked with `*` in the `Fav` column and sorted to the top when using default or `--sort recent` ordering.
You can also toggle favorite status in the interactive TUI server browser with the `f` key.

### Connect to a Server

Connect by full server name or unambiguous partial name:
```bash
ssher connect myserver
ssher connect my       # connects to myserver if it is the only matching server
```

Options:
- `-c`, `--current`: Connect directly in the current terminal instead of spawning an external window
- `--forward <spec>`: Add ad-hoc port forwarding for this connection only (repeatable; accepts `L:<bind>:<host>:<port>`, `R:<bind>:<host>:<port>`, `D:<port>`, or bare port shorthand like `5432`)
- `--no-forwards`: Do not apply saved port forwarding presets for this connection
- `--tmux [<session>]`: Attach to or create a remote tmux session (`tmux new-session -A -s <session>`) with allocated TTY (`-t`). Defaults to `ssher` if session name is omitted
- `--no-tmux`: Do not attach to saved default tmux session for this connection

Ad-hoc port forwarding and bypassing presets:
```bash
# Connect with an ad-hoc local forward and dynamic proxy:
ssher connect myserver --forward 8080 --forward D:1080

# Connect without applying saved presets:
ssher connect myserver --no-forwards

# Connect and attach to remote tmux session (default name 'ssher'):
ssher connect myserver --tmux

# Connect with a custom tmux session name:
ssher connect myserver --tmux custom-work

# Connect without attaching to saved default tmux session:
ssher connect myserver --no-tmux
```

This will:
1. Open a new terminal window (or connect in the current terminal with `-c`)
2. Execute the SSH command with the stored credentials
3. Keep the terminal open after disconnection for review

Exact name matches always take precedence. If multiple servers match a partial query, an error lists all candidate matches.

If the server has a configured jump host (`--jump`), `ssher` resolves the jump chain (expanding saved server references to `user@host:port` and chaining outer jump hosts up to 5 hops with cycle detection) and appends `-J <resolved>` to the SSH arguments. Note that `SSH_ASKPASS` applies to the final host only; intermediate jump hosts must use SSH key or agent authentication.

### Execute One-Off Commands

Execute a remote command inline in the current terminal (without spawning an external terminal emulator window) and return SSH's exit status:

```bash
# Execute on a single server (by exact name or unique partial name)
ssher exec myserver -- uptime
ssher exec myserver -- uname -a

# Execute sequentially across multiple servers matching all specified tags
ssher exec --tag prod --tag web -- df -h

# Execute sequentially across all configured servers
ssher exec --all -- uptime
```

#### Behavior & Options:
- `<target>`: Server name (supports exact match or unique partial-name resolution). Omitted when `--tag` or `--all` is used.
- `--tag <tag>`: Filter servers by tag (repeatable; must have all specified tags). Runs on each matching server sequentially in saved list order.
- `--all`: Run sequentially on all configured servers in saved list order.
- `-- <command...>`: The remote command and its arguments. Everything after `--` is passed to the remote execution.

#### Multi-Server Execution:
When `--tag` or `--all` is used:
1. `ssher` prints a `== <name> ==` header before each server's output.
2. Servers are contacted sequentially in list order.
3. For SSH key-authenticated servers, `-o BatchMode=yes` is automatically added to avoid blocking on interactive prompts.
4. If a server fails (non-zero exit status, connection failure, or keyring lookup failure), `ssher` prints the failure/status and continues to the remaining servers.
5. At the end of execution, `ssher` outputs a summary line: `ok N / failed M`.
6. `ssher` exits with a non-zero exit status if any server failed.

#### Authentication:
- **Key authentication**: Identity files and agent forwarding flags configured on the server are used.
- **Password authentication**: Securely reuses the temporary `SSH_ASKPASS` helper. Passwords are read from the OS keyring and never appear on the command line, environment variables, or process table. Intermediate jump hosts must use SSH keys or agent authentication.
- Port forwarding presets (`-L`, `-R`, `-D`) are omitted during command execution.

#### Command Quoting Caveat:
Command arguments after `--` are joined by single spaces into ONE remote command string argument passed to SSH (`ssh [options] user@host -- <command>`). SSH then delivers this command string to the remote user's default login shell for evaluation.
As a result, shell operators (such as pipes `|`, redirects `>`, or backgrounding `&`) and variable expansions will be evaluated by the remote shell. If you wish to pass arguments containing spaces or literal quotes to the remote command, ensure they are nested or escaped appropriately so they survive remote shell word-splitting:
```bash
# Evaluates 'hostname' and 'uptime' remotely through remote shell
ssher exec myserver -- "hostname && uptime"

# Preserves literal argument with spaces for remote program
ssher exec myserver -- echo "'hello world'"
```

### Install SSH Public Key (copy-id)

Install your public key on a saved SSH server wrapping `ssh-copy-id`:

```bash
# Install the default public key (<key_path>.pub) on a key-auth server
ssher copy-id myserver

# Install a specific public key (or private key path, which appends .pub)
ssher copy-id myserver --pub ~/.ssh/id_ed25519.pub
ssher copy-id myserver --key ~/.ssh/id_ed25519

# Install public key on a password-authenticated server and switch it to key auth
ssher copy-id password-server --key ~/.ssh/id_ed25519 --switch-to-key
```

#### Behavior & Options:
- `<name>`: Saved server name (supports exact match or unique partial-name resolution).
- Default public key: for key-auth servers, defaults to the server's configured `key_path` + `.pub`. For password-auth servers, `--key <private-or-pub path>` or `--pub <path>` is required.
- `--pub <path>`: Path to public key file.
- `--key <path>`: Path to private or public key file (if path does not end in `.pub`, `.pub` is automatically appended).
- Missing key file: if the `.pub` file does not exist, `ssher` exits with a clear error naming the missing path (keys are never generated automatically).
- Jump hosts: if the server is configured with a jump host, `-o ProxyJump=...` is automatically passed with the fully resolved jump chain.
- Password authentication: for password servers, `ssher` supplies the stored password securely using the `SSH_ASKPASS` helper (the password never appears on a command line or in the process table).
- Key-auth servers: `ssh-copy-id` logs in with your ssh-agent or default identities, not the server's saved `key_path`. If the saved key is non-default, load it first with `ssh-add <key>`. (ssher deliberately does not pass the saved key as an `IdentityFile` option: `ssh-copy-id` reuses those options to check whether the new key is already installed, so the saved key would make every new key look like a duplicate and nothing would be copied.)
- `--switch-to-key`: upon successful exit of `ssh-copy-id` on a password-authenticated server, updates the server configuration to Key authentication with the private key path (`pub path minus ".pub"`), saves the configuration, and removes the password from the OS keyring.

### File Transfer (put / get)

Upload or download files and directories wrapping `scp`:

```bash
# Upload a local file to a remote directory
ssher put myserver ./app.tar.gz /var/www/

# Upload multiple local files to a remote directory
ssher put myserver file1.txt file2.txt /tmp/

# Upload a directory recursively
ssher put -r myserver ./dist /var/www/html/

# Download a remote file to a local directory
ssher get myserver /var/log/nginx/access.log ./logs/

# Download multiple remote files to a local directory
ssher get myserver /etc/hosts /etc/resolv.conf ./backup/

# Download a directory recursively
ssher get -r myserver /var/www/uploads ./downloads/
```

#### Behavior & Options:
- `<name>`: Saved server name (supports exact match or unique partial-name resolution).
- `-r`, `--recursive`: Copy entire directories recursively (passes `-r` to `scp`).
- `ssher put <name> <local>... <remote_dir>`: Uploads one or more local files/directories to the remote directory. Validates that each local source exists before launching `scp`.
- `ssher get <name> <remote>... <local_dir>`: Downloads one or more remote files/directories to the local destination directory.
- Jump hosts: If the server is configured with a jump host (`--jump`), `-J <resolved>` is automatically passed with the fully resolved jump chain.
- Custom ports: If the server uses a non-default port, `-P <port>` is passed to `scp`.
- IPv6 hosts: IPv6 addresses are automatically enclosed in brackets (`user@[ipv6]:path`).
- Safe path handling: Local paths starting with `-` are protected with `./` prefixing and passed after `--` to prevent flag injection.
- Password authentication: Securely reuses the temporary `SSH_ASKPASS` helper. Passwords are read from the OS keyring and never appear on the command line or in the process table.

### Ping / Health Check

Check TCP reachability of saved servers' `host:port` without SSH login:

```bash
# Ping all configured servers concurrently (capped at 16 threads)
ssher ping

# Ping a specific server by name or unique prefix
ssher ping myserver

# Ping servers matching all specified tags
ssher ping --tag prod --tag web

# Ping with custom connection timeout in seconds (default: 3)
ssher ping --timeout 5
```

Output:
```
myserver    example.com:22   OK 12ms
db-server   10.0.0.6:5432    refused
old-box     10.0.0.99:22     timeout
invalid-box bad.host:22      dns error
up 1 / down 3
```

Results are printed in list order: `name  host:port  OK <ms>ms | refused | timeout | dns error`.
If any checked server is down (`refused`, `timeout`, or `dns error`), `ssher ping` exits with a non-zero exit code (1).

### Edit a Server

Modify an existing server configuration in place:

```bash
# Update host and port
ssher edit myserver --host new.example.com --port 2222

# Rename a server (automatically transfers keyring password if using password auth)
# and updates jump host references in dependent servers
ssher edit myserver --new-name prodserver

# Switch to key authentication
ssher edit myserver --key /path/to/key

# Switch to password authentication (interactive password prompt)
ssher edit myserver --password

# Enable or disable agent forwarding
ssher edit myserver --agent-forwarding false

# Replace tags (repeatable, sets full tag set)
ssher edit myserver --tag staging --tag api

# Clear all tags
ssher edit myserver --clear-tags

# Set or update jump host (saved server name or raw spec)
ssher edit myserver --jump bastion

# Clear jump host
ssher edit myserver --clear-jump

# Replace port forwarding presets (repeatable, replaces current forwards)
ssher edit myserver --forward 5432 --forward D:1080

# Clear all port forwarding presets
ssher edit myserver --clear-forwards

# Set or update default tmux session
ssher edit myserver --tmux dev

# Clear default tmux session
ssher edit myserver --clear-tmux
```

### Remove a Server

```bash
ssher remove myserver
```

This will remove the server configuration and clean up any stored password from the keyring. If other saved servers reference this server as their jump host, removal is refused with an error listing the dependent servers until their jump host references are updated or cleared.

### Import from SSH Config

Import host configurations from `~/.ssh/config` (or a specified file):

```bash
# Import from default ~/.ssh/config
ssher import

# Preview import without saving changes
ssher import --dry-run

# Import from a custom config file
ssher import --file /path/to/ssh_config

# Skip existing servers without errors
ssher import --skip-existing
```

The import command parses `Host` blocks with key-based authentication (`IdentityFile`), honoring `HostName`, `User`, `Port`, and `ForwardAgent`. Host entries without an `IdentityFile` are skipped with a warning.

### Export Servers

Export server configurations (and preferred terminal settings) to pretty JSON:

```bash
# Export all servers to stdout (passwords are stored in the keyring and never exported)
ssher export

# Export to a file with mode 0600 permissions
ssher export --file /path/to/backup.json

# Overwrite an existing backup file
ssher export --file /path/to/backup.json --force

# Export servers filtered by tag (repeatable; matches servers with all specified tags)
ssher export --tag prod --tag web --file /path/to/prod_backup.json
```

When exporting to stdout, a warning is emitted to stderr reminding that passwords are not exported and password-auth servers must be re-saved.

### Import from Backup

Import servers from a backup JSON file (or legacy bare-array format):

```bash
# Import servers from backup file
ssher import-backup /path/to/backup.json

# Overwrite existing servers with matching names
ssher import-backup /path/to/backup.json --overwrite

# Preview what would be imported without modifying configuration
ssher import-backup /path/to/backup.json --dry-run
```

The import validates each server with storage validators (missing key files produce a warning while keeping the server). Name collisions are skipped unless `--overwrite` is specified. Keyrings are never modified; any imported password-auth servers without stored passwords are listed in the summary with the exact command to configure their password (`ssher edit <name> --password`).

### Interactive TUI & Server Browser

Launch the interactive TUI by running `ssher` with no arguments:
```bash
ssher
```

Select **Browse Servers** to open the in-terminal server browser.

The browser displays jump hosts, port forward presets count, tmux session, favorite status, last used timestamp, and tags in dedicated `VIA`, `FWD`, `TMUX` (on displays with width >= 125, or detailed in the title block), `FAV`, `LAST USED`, and `TAGS` columns for quick visibility.

#### Browser Keybindings:
- `j` / `k` or `↓` / `↑`: Move selection up/down
- `Enter`: Connect to selected server (automatically routing through configured jump hosts, port forwards, and tmux session attach)
- `a`: Add a new server (prompts include comma-separated tags, optional jump host, comma-separated port forwards, and optional tmux session)
- `e`: Edit selected server in place (prompts show current values in brackets; empty keeps current values, `-` clears optional jump host, port forwards, or tmux session)
- `d`: Delete selected server (with y/n confirmation; blocked if referenced as a jump host)
- `f`: Toggle favorite status for selected server (re-sorts favourites to top and keeps selection pointed at the server)
- `p`: Ping highlighted server for TCP reachability and display health check status
- `/`: Filter servers by name, host, tags, jump host, port forwards, or tmux session (case-insensitive substring)
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

#### Dynamic Server Name and Tag Completion

For Bash, Zsh, and Fish, the generated completion scripts automatically wire dynamic completion of saved server names (for `connect`, `edit`, `remove`, `copy-id`, and `--jump`) and distinct server tags (for `--tag`) via the hidden subcommands:

```bash
ssher __complete-servers
ssher __complete-tags
```

`ssher __complete-servers` prints saved server names one per line. `ssher __complete-tags` prints distinct configured tags one per line in sorted order. Both produce no output with exit status 0 if configuration is missing or unreadable.

If you generate static base completions or wish to wire dynamic completion manually, append or source the snippet for your shell:

**Bash snippet:**
```bash
_ssher_complete() {
    local cur="${2:-${COMP_WORDS[COMP_CWORD]}}"
    local prev="${3:-${COMP_WORDS[COMP_CWORD-1]}}"

    case "${prev}" in
        --host|--username|--port|--key|--agent-forwarding|--new-name|--forward|--pub|--tmux)
            _ssher "$@"
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
    for (( i=1; i < COMP_CWORD; i++ )); do
        case "${COMP_WORDS[i]}" in
            connect|edit|remove|fav|unfav|exec|copy-id)
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
_ssher_tags() {
    local -a tags
    tags=(${(f)"$(ssher __complete-tags 2>/dev/null)"})
    _describe -t tags 'tag' tags "$@"
}
```

**Fish snippet:**
```fish
complete -c ssher -n "__fish_ssher_using_subcommand connect edit remove fav unfav" -f -a "(ssher __complete-servers)"
complete -c ssher -l tag -f -a "(ssher __complete-tags)"
complete -c ssher -l jump -f -a "(ssher __complete-servers)"
complete -c ssher -l sort -f -a "recent name"
```

## Configuration Files

All configuration is stored in `~/.config/ssh_manager/`:

- **servers.json**: Stores server configurations (JSON format)
- **terminal_config.json**: Stores user's terminal preference

### Example servers.json

```json
[
  {
    "name": "bastion",
    "host": "bastion.example.com",
    "username": "jumpuser",
    "port": 22,
    "auth_method": "key",
    "key_path": "/home/user/.ssh/id_rsa",
    "use_agent_forwarding": false
  },
  {
    "name": "myserver",
    "host": "10.0.0.5",
    "username": "username",
    "port": 22,
    "auth_method": "key",
    "key_path": "/home/user/.ssh/id_rsa",
    "use_agent_forwarding": false,
    "jump_host": "bastion",
    "forwards": [
      "L:5432:localhost:5432"
    ],
    "tmux": "main"
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
the system process table. Note that `SSH_ASKPASS` applies to the final
destination host only; intermediate jump hosts must authenticate via SSH
keys or an SSH agent.

## Error Handling

The application provides clear error messages for common issues:

- `Server not found: myserver` — The specified server doesn't exist
- `SSH key file not found: /path/to/key` — Key file path is invalid
- `Terminal not found` — None of the supported terminal emulators are installed
- `Failed to set up SSH_ASKPASS helper: ...` — Could not write the temporary
  askpass helper (used for password auth)
- `Invalid configuration: ...` — Config file is corrupted or invalid
- `Cannot remove server '...': referenced as jump host by: ...` — Server is referenced by other configurations
- `Jump host cycle detected: ...` — Jump host chains cannot form a cycle
- `Jump host chain exceeds maximum depth of 5 hops` — Jump chains cannot exceed 5 hops
- `Invalid port forwarding specification: ...` — Forward specification is malformed or port is outside 1-65535
- `Tmux session name cannot be empty` — Tmux session name must contain at least one character
- `Invalid tmux session name '...': must be 1-32 characters of [A-Za-z0-9_.-]` — Session name contains invalid characters or exceeds 32 characters
- `scp is not installed. Please install OpenSSH client tools` — `scp` was not found in PATH
- `Local source file not found: ...` — Local file or directory specified for upload does not exist
- `At least one source and a destination must be specified` — Missing source or destination argument

## Command Reference

```bash
# Add servers
ssher add <name> <host> <username> [OPTIONS]
  --key <path>                  SSH key file path
  --password                    Use password authentication (interactive prompt)
  --port <port>                Default: 22
  --agent-forwarding           Enable SSH agent forwarding
  --tag <tag>                  Server tag (repeatable)
  --jump <spec>                Jump host (saved server name or [user@]host[:port])
  --forward <spec>             Port forward preset (repeatable; L:..., R:..., D:..., or port shorthand)
  --tmux <session>             Default tmux session name (1-32 chars of [A-Za-z0-9_.-])

# Edit a server
ssher edit <name> [OPTIONS]
  --host <host>                 New server host
  --username <username>         New SSH username
  --port <port>                 New SSH port
  --key <path>                  New SSH key path (switches to key auth)
  --password                    Set new password (interactive prompt)
  --agent-forwarding <true|false> Enable or disable SSH agent forwarding
  --new-name <name>             New server identifier name
  --tag <tag>                  Set server tags (repeatable, replaces current tags)
  --clear-tags                 Clear all tags from server
  --jump <spec>                Set jump host (saved server name or [user@]host[:port])
  --clear-jump                 Clear jump host from server
  --forward <spec>             Set port forwarding presets (repeatable, replaces current forwards)
  --clear-forwards             Clear all port forwarding presets from server
  --tmux <session>             Set default tmux session name
  --clear-tmux                 Clear default tmux session from server

# List servers
ssher list [OPTIONS]
  --tag <tag>                  Filter by tag (repeatable, must match all)
  --sort <recent|name>         Sort order (default: recent, favourites first then recently used; name: plain alphabetical)

# Favorite / Unfavorite a server
ssher fav <name>
ssher unfav <name>

# Connect to a server (exact name or unambiguous partial match)
ssher connect <name> [OPTIONS]
  -c, --current                 Connect inline in current terminal
  --forward <spec>             Ad-hoc port forward for this connection (repeatable)
  --no-forwards                Do not apply saved port forwarding presets
  --tmux [<session>]           Attach/create remote tmux session (default: ssher)
  --no-tmux                    Do not attach to saved tmux session

# Execute one-off remote command inline in current terminal
ssher exec <target> -- <command...>
ssher exec --tag <tag> [--tag <tag2>] -- <command...>
ssher exec --all -- <command...>
  --tag <tag>                   Filter servers by tag (repeatable, must match all)
  --all                         Run on all configured servers

# Check TCP reachability of servers
ssher ping [<name>] [OPTIONS]
  --tag <tag>                   Filter servers by tag (repeatable, must match all)
  --timeout <secs>              Connection timeout in seconds (default: 3)

# Install public key on server (wrapping ssh-copy-id)
ssher copy-id <name> [OPTIONS]
  --pub <path>                  Public key file path (e.g. ~/.ssh/id_ed25519.pub)
  --key <path>                  Private or public key file path (e.g. ~/.ssh/id_ed25519)
  --switch-to-key               Switch server to key authentication upon success

# Upload files/directories to server (wrapping scp)
ssher put <name> <local>... <remote_dir> [OPTIONS]
  -r, --recursive               Copy directories recursively

# Download files/directories from server (wrapping scp)
ssher get <name> <remote>... <local_dir> [OPTIONS]
  -r, --recursive               Copy directories recursively

# Remove a server
ssher remove <name>

# Import servers from SSH config
ssher import [OPTIONS]
  -f, --file <path>             Path to SSH config file (default: ~/.ssh/config)
  --dry-run                     Preview import without modifying configuration
  --skip-existing               Skip hosts that collide with existing server names

# Export servers and configuration to JSON
ssher export [OPTIONS]
  --file <path>                 Path to output backup file (writes to stdout if omitted)
  --tag <tag>                   Filter servers to export by tag (repeatable, must match all)
  --force                       Overwrite existing destination file

# Import servers from backup JSON file
ssher import-backup <file> [OPTIONS]
  --overwrite                   Overwrite existing servers with matching names
  --dry-run                     Preview import without modifying configuration

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
