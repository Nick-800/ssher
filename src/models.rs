use crate::error::{Result, SshManagerError};
use crate::storage::Storage;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;
use std::io;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AuthMethod {
    #[serde(rename = "key")]
    Key,
    #[serde(rename = "password")]
    Password,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Forward {
    Local {
        bind_port: u16,
        host: String,
        host_port: u16,
    },
    Remote {
        bind_port: u16,
        host: String,
        host_port: u16,
    },
    Dynamic {
        bind_port: u16,
    },
}

impl Forward {
    pub fn parse(spec: &str) -> Result<Self> {
        let trimmed = spec.trim();
        if trimmed.is_empty() {
            return Err(SshManagerError::InvalidForward(
                "Forward specification cannot be empty".to_string(),
            ));
        }

        if trimmed.chars().all(|c| c.is_ascii_digit()) {
            let port: u16 = trimmed.parse().map_err(|_| SshManagerError::InvalidPort)?;
            Storage::validate_port(port)?;
            return Ok(Forward::Local {
                bind_port: port,
                host: "localhost".to_string(),
                host_port: port,
            });
        }

        if trimmed.starts_with("L:") || trimmed.starts_with("l:") {
            let parts: Vec<&str> = trimmed[2..].split(':').collect();
            if parts.len() != 3 {
                return Err(SshManagerError::InvalidForward(format!(
                    "Invalid local forward specification '{}': expected L:<bind_port>:<host>:<host_port>",
                    spec
                )));
            }
            let bind_port: u16 = parts[0].parse().map_err(|_| SshManagerError::InvalidPort)?;
            Storage::validate_port(bind_port)?;
            Storage::validate_forward_host(parts[1])?;
            let host_port: u16 = parts[2].parse().map_err(|_| SshManagerError::InvalidPort)?;
            Storage::validate_port(host_port)?;
            return Ok(Forward::Local {
                bind_port,
                host: parts[1].to_string(),
                host_port,
            });
        }

        if trimmed.starts_with("R:") || trimmed.starts_with("r:") {
            let parts: Vec<&str> = trimmed[2..].split(':').collect();
            if parts.len() != 3 {
                return Err(SshManagerError::InvalidForward(format!(
                    "Invalid remote forward specification '{}': expected R:<bind_port>:<host>:<host_port>",
                    spec
                )));
            }
            let bind_port: u16 = parts[0].parse().map_err(|_| SshManagerError::InvalidPort)?;
            Storage::validate_port(bind_port)?;
            Storage::validate_forward_host(parts[1])?;
            let host_port: u16 = parts[2].parse().map_err(|_| SshManagerError::InvalidPort)?;
            Storage::validate_port(host_port)?;
            return Ok(Forward::Remote {
                bind_port,
                host: parts[1].to_string(),
                host_port,
            });
        }

        if trimmed.starts_with("D:") || trimmed.starts_with("d:") {
            let remainder = &trimmed[2..];
            if remainder.is_empty() || remainder.contains(':') {
                return Err(SshManagerError::InvalidForward(format!(
                    "Invalid dynamic forward specification '{}': expected D:<bind_port>",
                    spec
                )));
            }
            let bind_port: u16 = remainder
                .parse()
                .map_err(|_| SshManagerError::InvalidPort)?;
            Storage::validate_port(bind_port)?;
            return Ok(Forward::Dynamic { bind_port });
        }

        Err(SshManagerError::InvalidForward(format!(
            "Invalid forward specification '{}': expected L:..., R:..., D:..., or port shorthand",
            spec
        )))
    }

    pub fn validate(&self) -> Result<()> {
        match self {
            Forward::Local {
                bind_port,
                host,
                host_port,
            }
            | Forward::Remote {
                bind_port,
                host,
                host_port,
            } => {
                Storage::validate_port(*bind_port)?;
                Storage::validate_forward_host(host)?;
                Storage::validate_port(*host_port)?;
            }
            Forward::Dynamic { bind_port } => {
                Storage::validate_port(*bind_port)?;
            }
        }
        Ok(())
    }

    pub fn dedupe(forwards: &[Forward]) -> Vec<Forward> {
        let mut result = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for f in forwards {
            if seen.insert(f.clone()) {
                result.push(f.clone());
            }
        }
        result
    }
}

impl std::fmt::Display for Forward {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Forward::Local {
                bind_port,
                host,
                host_port,
            } => write!(f, "L:{}:{}:{}", bind_port, host, host_port),
            Forward::Remote {
                bind_port,
                host,
                host_port,
            } => write!(f, "R:{}:{}:{}", bind_port, host, host_port),
            Forward::Dynamic { bind_port } => write!(f, "D:{}", bind_port),
        }
    }
}

impl std::str::FromStr for Forward {
    type Err = SshManagerError;

    fn from_str(s: &str) -> Result<Self> {
        Forward::parse(s)
    }
}

impl Serialize for Forward {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Forward {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Forward::parse(&s).map_err(serde::de::Error::custom)
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Server {
    pub name: String,
    pub host: String,
    pub username: String,
    pub port: u16,
    pub auth_method: AuthMethod,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_path: Option<String>,
    #[serde(default)]
    pub use_agent_forwarding: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump_host: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forwards: Vec<Forward>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub favorite: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub servers: Vec<Server>,
    #[serde(default)]
    pub preferred_terminal: Option<String>,
}

impl Config {
    pub fn new() -> Self {
        Config {
            servers: Vec::new(),
            preferred_terminal: None,
        }
    }

    pub fn find_server(&self, name: &str) -> Option<&Server> {
        self.servers.iter().find(|s| s.name == name)
    }

    pub fn resolve_server<'a>(&'a self, query: &str) -> Result<&'a Server> {
        resolve_server(&self.servers, query)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    pub fn new(
        name: String,
        host: String,
        username: String,
        port: u16,
        auth_method: AuthMethod,
        key_path: Option<String>,
    ) -> Self {
        Server {
            name,
            host,
            username,
            port,
            auth_method,
            key_path,
            use_agent_forwarding: false,
            tags: Vec::new(),
            jump_host: None,
            forwards: Vec::new(),
            favorite: false,
            last_used: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ServerEdit {
    pub new_name: Option<String>,
    pub host: Option<String>,
    pub username: Option<String>,
    pub port: Option<u16>,
    pub auth: Option<AuthEdit>,
    pub agent_forwarding: Option<bool>,
    pub tags: Option<Vec<String>>,
    pub jump_host: Option<Option<String>>,
    pub forwards: Option<Vec<Forward>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AuthEdit {
    Key(String),
    Password,
}

impl ServerEdit {
    pub fn is_empty(&self) -> bool {
        self.new_name.is_none()
            && self.host.is_none()
            && self.username.is_none()
            && self.port.is_none()
            && self.auth.is_none()
            && self.agent_forwarding.is_none()
            && self.tags.is_none()
            && self.jump_host.is_none()
            && self.forwards.is_none()
    }

    pub fn apply(&self, server: &mut Server, config: &Config) -> Result<()> {
        if self.is_empty() {
            return Err(SshManagerError::NothingToChange);
        }

        if let Some(new_name) = &self.new_name {
            Storage::validate_server_name(new_name)?;
            if !Storage::server_name_is_unique(config, new_name, Some(&server.name)) {
                return Err(SshManagerError::ServerAlreadyExists(new_name.clone()));
            }
        }

        if let Some(host) = &self.host {
            Storage::validate_host(host)?;
        }

        if let Some(username) = &self.username {
            Storage::validate_username(username)?;
        }

        if let Some(port) = self.port {
            Storage::validate_port(port)?;
        }

        if let Some(AuthEdit::Key(key_path)) = &self.auth {
            Storage::validate_key_path(key_path)?;
        }

        let normalized_tags = if let Some(tags) = &self.tags {
            Some(Storage::normalize_tags(tags)?)
        } else {
            None
        };

        let effective_name = self.new_name.as_deref().unwrap_or(&server.name);
        let effective_jump = match &self.jump_host {
            Some(Some(j)) => Some(j.as_str()),
            Some(None) => None,
            None => server.jump_host.as_deref(),
        };

        if let Some(jump) = effective_jump {
            if self.jump_host.is_some() {
                Storage::validate_jump_host(jump)?;
            }
            if jump == effective_name || (self.new_name.is_some() && jump == server.name) {
                return Err(SshManagerError::SelfReferentialJumpHost(
                    effective_name.to_string(),
                ));
            }
            let mut visited = vec![effective_name.to_string()];
            if effective_name != server.name {
                visited.push(server.name.clone());
            }
            let mut current_spec = jump.to_string();
            loop {
                if visited.contains(&current_spec) {
                    let mut cycle_path = visited.clone();
                    cycle_path.push(current_spec);
                    return Err(SshManagerError::JumpHostCycle(cycle_path.join(" -> ")));
                }
                if let Some(target) = config.find_server(&current_spec) {
                    visited.push(target.name.clone());
                    if let Some(next) = &target.jump_host {
                        current_spec = next.clone();
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
        }

        if let Some(new_name) = &self.new_name {
            server.name = new_name.clone();
        }
        if let Some(host) = &self.host {
            server.host = host.clone();
        }
        if let Some(username) = &self.username {
            server.username = username.clone();
        }
        if let Some(port) = self.port {
            server.port = port;
        }
        match &self.auth {
            Some(AuthEdit::Key(key_path)) => {
                server.auth_method = AuthMethod::Key;
                server.key_path = Some(key_path.clone());
            }
            Some(AuthEdit::Password) => {
                server.auth_method = AuthMethod::Password;
                server.key_path = None;
            }
            None => {}
        }
        if let Some(af) = self.agent_forwarding {
            server.use_agent_forwarding = af;
        }
        if let Some(tags) = normalized_tags {
            server.tags = tags;
        }
        if let Some(jump) = &self.jump_host {
            server.jump_host = jump.clone();
        }
        if let Some(forwards) = &self.forwards {
            for f in forwards {
                f.validate()?;
            }
            server.forwards = Forward::dedupe(forwards);
        }

        Ok(())
    }
}

/// Format a count or compact summary of forwards for tabular display ("-" when none).
pub fn format_forwards_summary(forwards: &[Forward]) -> String {
    if forwards.is_empty() {
        "-".to_string()
    } else {
        forwards.len().to_string()
    }
}

/// Check if a server matches a case-insensitive query across its name, host, tags, jump host, or forwards.
/// An empty query matches every server.
pub fn server_matches_query(server: &Server, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let q = query.to_lowercase();
    server.name.to_lowercase().contains(&q)
        || server.host.to_lowercase().contains(&q)
        || server.tags.iter().any(|t| t.to_lowercase().contains(&q))
        || server
            .jump_host
            .as_ref()
            .map(|j| j.to_lowercase().contains(&q))
            .unwrap_or(false)
        || server
            .forwards
            .iter()
            .any(|f| f.to_string().to_lowercase().contains(&q))
}

/// Resolve the `-J` argument for a server, expanding saved servers into `user@host:port`
/// and chaining jump hosts outermost-first, with cycle detection and depth limit (5).
pub fn resolve_jump(config: &Config, server: &Server) -> Result<Option<String>> {
    let jump_spec = match &server.jump_host {
        Some(j) => j,
        None => return Ok(None),
    };

    let mut hops: Vec<String> = Vec::new();
    let mut visited: Vec<String> = Vec::new();
    visited.push(server.name.clone());

    let mut current_spec = jump_spec.clone();

    loop {
        if hops.len() >= 5 {
            return Err(SshManagerError::JumpHostDepthExceeded);
        }

        if visited.contains(&current_spec) {
            let mut cycle_path = visited.clone();
            cycle_path.push(current_spec);
            return Err(SshManagerError::JumpHostCycle(cycle_path.join(" -> ")));
        }

        if let Some(target_server) = config.find_server(&current_spec) {
            hops.push(format!(
                "{}@{}:{}",
                target_server.username, target_server.host, target_server.port
            ));
            visited.push(target_server.name.clone());

            if let Some(next_jump) = &target_server.jump_host {
                current_spec = next_jump.clone();
            } else {
                break;
            }
        } else {
            hops.push(current_spec);
            break;
        }
    }

    if hops.is_empty() {
        return Ok(None);
    }

    hops.reverse();
    Ok(Some(hops.join(",")))
}

/// Find all servers that reference the given server name as their jump host.
pub fn find_jump_dependents(servers: &[Server], server_name: &str) -> Vec<String> {
    servers
        .iter()
        .filter(|s| s.jump_host.as_deref() == Some(server_name))
        .map(|s| s.name.clone())
        .collect()
}

/// Update all jump_host references from old_name to new_name. Returns the number of updated references.
pub fn update_jump_host_references(
    servers: &mut [Server],
    old_name: &str,
    new_name: &str,
) -> usize {
    let mut count = 0;
    for s in servers {
        if s.jump_host.as_deref() == Some(old_name) {
            s.jump_host = Some(new_name.to_string());
            count += 1;
        }
    }
    count
}

/// Filter a list of servers by query, returning indices into the slice.
pub fn filter_server_indices(servers: &[Server], query: &str) -> Vec<usize> {
    servers
        .iter()
        .enumerate()
        .filter_map(|(idx, s)| {
            if server_matches_query(s, query) {
                Some(idx)
            } else {
                None
            }
        })
        .collect()
}

/// Resolve a server by query name: exact match wins; otherwise unique partial substring match on
/// server name (case-insensitive) succeeds; ambiguous match returns AmbiguousServerName;
/// zero matches returns ServerNotFound.
pub fn resolve_server<'a>(servers: &'a [Server], query: &str) -> Result<&'a Server> {
    if query.is_empty() {
        return Err(SshManagerError::ServerNotFound(query.to_string()));
    }

    if let Some(s) = servers.iter().find(|s| s.name == query) {
        return Ok(s);
    }

    let q_lower = query.to_lowercase();
    let candidates: Vec<&'a Server> = servers
        .iter()
        .filter(|s| s.name.to_lowercase().contains(&q_lower))
        .collect();

    match candidates.len() {
        0 => Err(SshManagerError::ServerNotFound(query.to_string())),
        1 => Ok(candidates[0]),
        _ => {
            let mut names: Vec<&str> = candidates.iter().map(|s| s.name.as_str()).collect();
            names.sort();
            Err(SshManagerError::AmbiguousServerName(
                query.to_string(),
                names.join(", "),
            ))
        }
    }
}

/// Format server names for shell completion (one per line).
pub fn format_server_completions(servers: &[Server]) -> String {
    let mut output = String::new();
    for server in servers {
        output.push_str(&server.name);
        output.push('\n');
    }
    output
}

/// Format distinct tags across all servers for shell completion (one per line, sorted).
pub fn format_tag_completions(servers: &[Server]) -> String {
    let mut tags: Vec<String> = servers
        .iter()
        .flat_map(|s| s.tags.iter().cloned())
        .collect();
    tags.sort();
    tags.dedup();
    let mut output = String::new();
    for tag in tags {
        output.push_str(&tag);
        output.push('\n');
    }
    output
}

/// Select servers for exec subcommand based on target name, tags, or all flag.
///
/// If `target` is provided, resolves the single server via `resolve_server`.
/// If `all` is true, returns all servers in list order (or `NoServersMatch` if empty).
/// If `tags` are provided, validates tags and returns all servers having all specified tags
/// in list order (or `NoServersMatch` if none match).
pub fn select_exec_targets<'a>(
    servers: &'a [Server],
    target: Option<&str>,
    tags: &[String],
    all: bool,
) -> Result<Vec<&'a Server>> {
    if let Some(t) = target {
        let server = resolve_server(servers, t)?;
        return Ok(vec![server]);
    }

    if all {
        if servers.is_empty() {
            return Err(SshManagerError::NoServersMatch);
        }
        return Ok(servers.iter().collect());
    }

    if !tags.is_empty() {
        for tag in tags {
            Storage::validate_tag(tag)?;
        }
        let normalized: Vec<String> = tags.iter().map(|t| t.to_lowercase()).collect();
        let matches: Vec<&'a Server> = servers
            .iter()
            .filter(|server| {
                normalized
                    .iter()
                    .all(|ft| server.tags.iter().any(|st| st == ft))
            })
            .collect();

        if matches.is_empty() {
            return Err(SshManagerError::NoServersMatch);
        }
        return Ok(matches);
    }

    Err(SshManagerError::NoServersMatch)
}

/// TCP reachability status of a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PingStatus {
    Ok(Duration),
    Refused,
    Timeout,
    DnsError,
}

impl PingStatus {
    pub fn is_up(&self) -> bool {
        matches!(self, PingStatus::Ok(_))
    }
}

impl fmt::Display for PingStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PingStatus::Ok(d) => write!(f, "OK {}ms", d.as_millis()),
            PingStatus::Refused => write!(f, "refused"),
            PingStatus::Timeout => write!(f, "timeout"),
            PingStatus::DnsError => write!(f, "dns error"),
        }
    }
}

/// Result of a TCP health check for a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PingResult {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub status: PingStatus,
}

impl PingResult {
    pub fn is_up(&self) -> bool {
        self.status.is_up()
    }
}

/// Classify an `io::ErrorKind` into a `PingStatus`.
pub fn classify_error_kind(kind: io::ErrorKind) -> PingStatus {
    match kind {
        io::ErrorKind::ConnectionRefused | io::ErrorKind::ConnectionReset => PingStatus::Refused,
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => PingStatus::Timeout,
        io::ErrorKind::NotFound => PingStatus::DnsError,
        _ => PingStatus::Timeout,
    }
}

/// Classify an `io::ErrorKind` into a `PingStatus` (alias).
#[allow(dead_code)]
pub fn classify_io_error(kind: io::ErrorKind) -> PingStatus {
    classify_error_kind(kind)
}

/// Classify an `io::Error` into a `PingStatus`.
#[allow(dead_code)]
pub fn status_from_io_error(err: &io::Error) -> PingStatus {
    classify_error_kind(err.kind())
}

/// Select servers for ping subcommand based on name or tags.
///
/// If both `name` and non-empty `tags` are provided, returns `InvalidConfig`.
/// If `name` is provided, resolves the single server via `resolve_server`.
/// If `tags` are provided, validates tags and returns all servers having all specified tags
/// in list order (or `NoServersMatch` if none match).
/// If neither is provided, returns all servers in list order (or `NoServersMatch` if empty).
pub fn select_ping_targets<'a>(
    servers: &'a [Server],
    name: Option<&str>,
    tags: &[String],
) -> Result<Vec<&'a Server>> {
    if name.is_some() && !tags.is_empty() {
        return Err(SshManagerError::InvalidConfig(
            "Cannot specify both server name and --tag".to_string(),
        ));
    }

    if let Some(n) = name {
        let server = resolve_server(servers, n)?;
        return Ok(vec![server]);
    }

    if !tags.is_empty() {
        for tag in tags {
            Storage::validate_tag(tag)?;
        }
        let normalized: Vec<String> = tags.iter().map(|t| t.to_lowercase()).collect();
        let matches: Vec<&'a Server> = servers
            .iter()
            .filter(|server| {
                normalized
                    .iter()
                    .all(|ft| server.tags.iter().any(|st| st == ft))
            })
            .collect();

        if matches.is_empty() {
            return Err(SshManagerError::NoServersMatch);
        }
        return Ok(matches);
    }

    if servers.is_empty() {
        return Err(SshManagerError::NoServersMatch);
    }

    Ok(servers.iter().collect())
}

/// Format a single ping result line: `name  host:port  OK 12ms | refused | timeout | dns error`.
pub fn format_ping_result(name: &str, host: &str, port: u16, status: &PingStatus) -> String {
    format!("{}  {}:{}  {}", name, host, port, status)
}

/// Format the ping summary line: `up N / down M`.
pub fn format_ping_summary(up: usize, down: usize) -> String {
    format!("up {} / down {}", up, down)
}

/// Check TCP reachability of a single host:port via `TcpStream::connect_timeout`.
pub fn ping_server_tcp(host: &str, port: u16, timeout: Duration) -> PingStatus {
    let clean_host = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);

    let addrs = match (clean_host, port).to_socket_addrs() {
        Ok(iter) => {
            let list: Vec<SocketAddr> = iter.collect();
            if list.is_empty() {
                return PingStatus::DnsError;
            }
            list
        }
        Err(_) => return PingStatus::DnsError,
    };

    let deadline = Instant::now() + timeout;
    let mut last_status = PingStatus::Timeout;

    for addr in addrs {
        let now = Instant::now();
        if now >= deadline {
            last_status = PingStatus::Timeout;
            break;
        }
        let remaining = deadline - now;
        let start = Instant::now();
        match TcpStream::connect_timeout(&addr, remaining) {
            Ok(_stream) => {
                return PingStatus::Ok(start.elapsed());
            }
            Err(e) => {
                last_status = classify_error_kind(e.kind());
            }
        }
    }

    last_status
}

/// Check servers concurrently with std::thread (concurrency capped at 16) and preserve list order.
pub fn ping_servers(servers: &[&Server], timeout: Duration) -> Vec<PingResult> {
    ping_servers_with_runner(servers, timeout, 16, ping_server_tcp)
}

/// Check servers concurrently with a custom runner function (concurrency capped at 16) and preserve list order.
pub fn ping_servers_with_runner<F>(
    servers: &[&Server],
    timeout: Duration,
    max_concurrency: usize,
    runner: F,
) -> Vec<PingResult>
where
    F: Fn(&str, u16, Duration) -> PingStatus + Sync + Send + Copy + 'static,
{
    if servers.is_empty() {
        return Vec::new();
    }

    let concurrency = max_concurrency.clamp(1, 16).min(servers.len());
    let queue = Arc::new(Mutex::new(
        servers
            .iter()
            .enumerate()
            .map(|(i, &s)| (i, s.name.clone(), s.host.clone(), s.port))
            .collect::<VecDeque<_>>(),
    ));

    let (tx, rx) = mpsc::channel();
    let mut handles = Vec::with_capacity(concurrency);

    for _ in 0..concurrency {
        let q = Arc::clone(&queue);
        let sender = tx.clone();
        let handle = thread::spawn(move || loop {
            let item = {
                let mut locked = q.lock().unwrap();
                locked.pop_front()
            };
            match item {
                Some((idx, name, host, port)) => {
                    let status = runner(&host, port, timeout);
                    let result = PingResult {
                        name,
                        host,
                        port,
                        status,
                    };
                    if sender.send((idx, result)).is_err() {
                        break;
                    }
                }
                None => break,
            }
        });
        handles.push(handle);
    }
    drop(tx);

    let mut indexed_results: Vec<Option<PingResult>> = (0..servers.len()).map(|_| None).collect();
    for (idx, res) in rx {
        if idx < indexed_results.len() {
            indexed_results[idx] = Some(res);
        }
    }

    for h in handles {
        let _ = h.join();
    }

    indexed_results.into_iter().flatten().collect()
}

/// Pure comparator ordering favourites first, then most-recently-used first, then name.
pub fn sort_servers(a: &Server, b: &Server) -> std::cmp::Ordering {
    b.favorite
        .cmp(&a.favorite)
        .then_with(|| b.last_used.cmp(&a.last_used))
        .then_with(|| a.name.cmp(&b.name))
}

/// Format a relative timestamp (e.g. "never", "just now", "3h ago", "2d ago") given `now` in epoch seconds.
pub fn format_last_used(last_used: Option<u64>, now: u64) -> String {
    let ts = match last_used {
        Some(ts) => ts,
        None => return "never".to_string(),
    };

    if ts > now {
        return "just now".to_string();
    }

    let diff = now - ts;
    if diff < 60 {
        "just now".to_string()
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

/// Get current Unix epoch timestamp in seconds.
pub fn current_time_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Stamp `last_used` on a server using a provided time function.
pub fn stamp_server_last_used<F>(server: &mut Server, now_fn: F)
where
    F: FnOnce() -> u64,
{
    server.last_used = Some(now_fn());
}

/// Stamp `last_used` on a server by name in a slice of servers using a provided time function.
/// Returns true if the server was found and updated, false otherwise.
pub fn stamp_server_in_slice<F>(servers: &mut [Server], server_name: &str, now_fn: F) -> bool
where
    F: FnOnce() -> u64,
{
    if let Some(s) = servers.iter_mut().find(|s| s.name == server_name) {
        stamp_server_last_used(s, now_fn);
        true
    } else {
        false
    }
}

/// Find the index in `filtered_indices` corresponding to `server_name` in `servers`.
/// Useful for maintaining browser selection after reordering.
pub fn find_selected_index(
    servers: &[Server],
    filtered_indices: &[usize],
    server_name: &str,
) -> Option<usize> {
    filtered_indices
        .iter()
        .position(|&idx| servers.get(idx).map(|s| s.name.as_str()) == Some(server_name))
}

/// Set a server's favorite status by name in a slice of servers.
/// Returns true if the server was found and updated, false otherwise.
pub fn set_server_favorite(servers: &mut [Server], name: &str, favorite: bool) -> bool {
    if let Some(s) = servers.iter_mut().find(|s| s.name == name) {
        s.favorite = favorite;
        true
    } else {
        false
    }
}

/// Toggle a server's favorite status by name in a slice of servers.
/// Returns Some(new_favorite_state) if found, or None if not found.
pub fn toggle_server_favorite(servers: &mut [Server], name: &str) -> Option<bool> {
    if let Some(s) = servers.iter_mut().find(|s| s.name == name) {
        s.favorite = !s.favorite;
        Some(s.favorite)
    } else {
        None
    }
}

/// Derive the public key path for `copy-id`.
///
/// If an explicit path is provided via `--pub` or `--key`, it is used (appending `.pub`
/// if it does not already end in `.pub`).
/// If no explicit path is provided, key-auth servers default to `<key_path>.pub`.
/// Password-auth servers without an explicit path return an error instructing the user
/// to provide `--key` or `--pub`.
pub fn derive_copy_id_pub_path(server: &Server, explicit_path: Option<&str>) -> Result<String> {
    if let Some(path) = explicit_path {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return Err(SshManagerError::InvalidConfig(
                "Public key path cannot be empty".to_string(),
            ));
        }
        if trimmed.ends_with(".pub") {
            Ok(trimmed.to_string())
        } else {
            Ok(format!("{}.pub", trimmed))
        }
    } else {
        match server.auth_method {
            AuthMethod::Key => {
                if let Some(key_path) = &server.key_path {
                    if key_path.ends_with(".pub") {
                        Ok(key_path.clone())
                    } else {
                        Ok(format!("{}.pub", key_path))
                    }
                } else {
                    Err(SshManagerError::InvalidConfig(format!(
                        "Server '{}' is configured for key authentication but has no key path",
                        server.name
                    )))
                }
            }
            AuthMethod::Password => Err(SshManagerError::InvalidConfig(format!(
                "Server '{}' uses password authentication; specify a key with --key or --pub (e.g. ssher copy-id {} --key ~/.ssh/id_ed25519)",
                server.name, server.name
            ))),
        }
    }
}

/// Pure transformation switching a server to key authentication using the private key
/// corresponding to `pub_path` (pub_path minus `.pub`).
pub fn switch_server_to_key(server: &mut Server, pub_path: &str) -> Result<String> {
    let priv_path = pub_path.strip_suffix(".pub").ok_or_else(|| {
        SshManagerError::InvalidConfig(format!(
            "Public key path '{}' does not end in '.pub'",
            pub_path
        ))
    })?;

    server.auth_method = AuthMethod::Key;
    server.key_path = Some(priv_path.to_string());
    Ok(priv_path.to_string())
}

/// Expand leading `~` or `~/` to the user's home directory.
pub fn expand_tilde_path(p: &std::path::Path) -> Result<std::path::PathBuf> {
    let s = p.to_string_lossy();
    if s == "~" {
        dirs::home_dir().ok_or_else(|| {
            SshManagerError::ConfigDirError(
                "Could not determine home directory. $HOME not set?".to_string(),
            )
        })
    } else if let Some(rest) = s.strip_prefix("~/") {
        let home = dirs::home_dir().ok_or_else(|| {
            SshManagerError::ConfigDirError(
                "Could not determine home directory. $HOME not set?".to_string(),
            )
        })?;
        Ok(home.join(rest))
    } else {
        Ok(p.to_path_buf())
    }
}

/// Split transfer paths into source paths and destination path.
/// Requires at least one source and a non-empty destination.
pub fn split_transfer_paths(paths: &[String]) -> Result<(&[String], &str)> {
    if paths.len() < 2 {
        return Err(SshManagerError::MissingSourceOrDestination);
    }
    let (sources, dest) = paths.split_at(paths.len() - 1);
    let dest_str = dest[0].trim();
    if dest_str.is_empty() {
        return Err(SshManagerError::MissingSourceOrDestination);
    }
    Ok((sources, dest[0].as_str()))
}

/// Validate that every local source exists on disk.
pub fn validate_local_sources(sources: &[String]) -> Result<()> {
    if sources.is_empty() {
        return Err(SshManagerError::MissingSourceOrDestination);
    }
    for src in sources {
        let p = std::path::Path::new(src);
        let exists = if let Ok(expanded) = expand_tilde_path(p) {
            expanded.exists()
        } else {
            p.exists()
        };
        if !exists {
            return Err(SshManagerError::LocalSourceNotFound(src.clone()));
        }
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TerminalConfig {
    pub terminal: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportedHost {
    pub name: String,
    pub host: String,
    pub username: Option<String>,
    pub port: u16,
    pub identity_file: Option<String>,
    pub forward_agent: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SshConfigParseResult {
    pub hosts: Vec<ImportedHost>,
    pub skipped: Vec<(String, String)>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportResult {
    pub to_import: Vec<Server>,
    pub skipped: Vec<(String, String)>,
}

fn strip_comment(line: &str) -> &str {
    let mut in_quote = false;
    let mut quote_char = ' ';
    let mut escape = false;
    for (idx, ch) in line.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if ch == '\\' {
            escape = true;
            continue;
        }
        if in_quote {
            if ch == quote_char {
                in_quote = false;
            }
        } else if ch == '"' || ch == '\'' {
            in_quote = true;
            quote_char = ch;
        } else if ch == '#' {
            return &line[..idx];
        }
    }
    line
}

fn split_key_value(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let delim_pos = trimmed.find(|c: char| c.is_whitespace() || c == '=')?;
    let key = &trimmed[..delim_pos];
    let rest = trimmed[delim_pos..].trim_start();
    let value = if let Some(stripped) = rest.strip_prefix('=') {
        stripped.trim_start()
    } else {
        rest
    };
    Some((key, value.trim()))
}

fn parse_tokens(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut quote_char = ' ';
    let mut escape = false;

    for ch in s.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }
        if ch == '\\' {
            escape = true;
            continue;
        }
        if in_quote {
            if ch == quote_char {
                in_quote = false;
            } else {
                current.push(ch);
            }
        } else if ch == '"' || ch == '\'' {
            in_quote = true;
            quote_char = ch;
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn is_wildcard_or_negated(pattern: &str) -> bool {
    pattern.contains('*') || pattern.contains('?') || pattern.contains('!')
}

fn strip_quotes(s: &str) -> &str {
    let trimmed = s.trim();
    if (trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2)
        || (trimmed.starts_with('\'') && trimmed.ends_with('\'') && trimmed.len() >= 2)
    {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    }
}

fn expand_tilde(path: &str, home: Option<&str>) -> String {
    if let Some(home_path) = home {
        let home_clean = home_path.trim_end_matches('/');
        if path == "~" {
            home_clean.to_string()
        } else if let Some(rest) = path.strip_prefix("~/") {
            format!("{}/{}", home_clean, rest)
        } else {
            path.to_string()
        }
    } else {
        path.to_string()
    }
}

/// Pure parser for SSH config content.
#[allow(dead_code)]
pub fn parse_ssh_config(content: &str, home: Option<&str>) -> Vec<ImportedHost> {
    parse_ssh_config_detailed(content, home).hosts
}

/// Parse SSH config content with full details including skipped hosts and warnings.
pub fn parse_ssh_config_detailed(content: &str, home: Option<&str>) -> SshConfigParseResult {
    let mut hosts = Vec::new();
    let mut skipped = Vec::new();
    let mut warnings = Vec::new();
    let mut warned_include = false;
    let mut warned_match = false;

    struct CurrentBlock {
        aliases: Vec<String>,
        hostname: Option<String>,
        user: Option<String>,
        port: Option<u16>,
        identity_file: Option<String>,
        forward_agent: Option<bool>,
    }

    let mut current_block: Option<CurrentBlock> = None;
    let mut in_match_block = false;

    let finalize_block = |block: CurrentBlock,
                          hosts: &mut Vec<ImportedHost>,
                          skipped: &mut Vec<(String, String)>| {
        for alias in block.aliases {
            match &block.identity_file {
                Some(key) => {
                    hosts.push(ImportedHost {
                        name: alias.clone(),
                        host: block.hostname.clone().unwrap_or_else(|| alias.clone()),
                        username: block.user.clone(),
                        port: block.port.unwrap_or(22),
                        identity_file: Some(key.clone()),
                        forward_agent: block.forward_agent.unwrap_or(false),
                    });
                }
                None => {
                    skipped.push((
                        alias,
                        "no IdentityFile; add manually with --password".to_string(),
                    ));
                }
            }
        }
    };

    for raw_line in content.lines() {
        let line_without_comment = strip_comment(raw_line).trim();
        if line_without_comment.is_empty() {
            continue;
        }

        let Some((key, val)) = split_key_value(line_without_comment) else {
            continue;
        };

        let key_lower = key.to_lowercase();
        match key_lower.as_str() {
            "include" => {
                if !warned_include {
                    warnings.push("Include directive is not followed".to_string());
                    warned_include = true;
                }
            }
            "match" => {
                if !warned_match {
                    warnings.push("Match directive is not followed".to_string());
                    warned_match = true;
                }
                if let Some(b) = current_block.take() {
                    finalize_block(b, &mut hosts, &mut skipped);
                }
                in_match_block = true;
            }
            "host" => {
                if let Some(b) = current_block.take() {
                    finalize_block(b, &mut hosts, &mut skipped);
                }
                in_match_block = false;

                let patterns = parse_tokens(val);
                let concrete_aliases: Vec<String> = patterns
                    .into_iter()
                    .filter(|p| !is_wildcard_or_negated(p))
                    .collect();

                current_block = Some(CurrentBlock {
                    aliases: concrete_aliases,
                    hostname: None,
                    user: None,
                    port: None,
                    identity_file: None,
                    forward_agent: None,
                });
            }
            _ => {
                if in_match_block {
                    continue;
                }
                if let Some(block) = current_block.as_mut() {
                    match key_lower.as_str() {
                        "hostname" if block.hostname.is_none() => {
                            block.hostname = Some(strip_quotes(val).to_string());
                        }
                        "user" if block.user.is_none() => {
                            block.user = Some(strip_quotes(val).to_string());
                        }
                        "port" if block.port.is_none() => {
                            let unquoted = strip_quotes(val);
                            match unquoted.parse::<u16>() {
                                Ok(p) if p > 0 => {
                                    block.port = Some(p);
                                }
                                _ => {
                                    warnings.push(format!(
                                        "Invalid port '{}', defaulting to 22",
                                        unquoted
                                    ));
                                    block.port = Some(22);
                                }
                            }
                        }
                        "identityfile" if block.identity_file.is_none() => {
                            let unquoted = strip_quotes(val);
                            let expanded = expand_tilde(unquoted, home);
                            block.identity_file = Some(expanded);
                        }
                        "forwardagent" if block.forward_agent.is_none() => {
                            let unquoted = strip_quotes(val).to_lowercase();
                            match unquoted.as_str() {
                                "yes" | "true" => block.forward_agent = Some(true),
                                "no" | "false" => block.forward_agent = Some(false),
                                _ => {}
                            }
                        }
                        _ => {
                            // unknown keywords ignored
                        }
                    }
                }
            }
        }
    }

    if let Some(b) = current_block.take() {
        finalize_block(b, &mut hosts, &mut skipped);
    }

    SshConfigParseResult {
        hosts,
        skipped,
        warnings,
    }
}

/// Pure helper to map and validate imported hosts against existing config.
pub fn filter_and_map_imported_hosts(
    existing_config: &Config,
    imported_hosts: Vec<ImportedHost>,
    default_user: Option<&str>,
    validate_key_exists: impl Fn(&str) -> bool,
) -> ImportResult {
    let mut to_import = Vec::new();
    let mut skipped = Vec::new();

    for host in imported_hosts {
        if let Err(e) = Storage::validate_server_name(&host.name) {
            skipped.push((host.name.clone(), format!("Invalid server name: {}", e)));
            continue;
        }

        let exists_in_config = !Storage::server_name_is_unique(existing_config, &host.name, None);
        let exists_in_batch = to_import.iter().any(|s: &Server| s.name == host.name);
        if exists_in_config || exists_in_batch {
            skipped.push((host.name.clone(), "Server already exists".to_string()));
            continue;
        }

        if let Err(e) = Storage::validate_host(&host.host) {
            skipped.push((host.name.clone(), format!("Invalid host: {}", e)));
            continue;
        }

        let username = match &host.username {
            Some(u) if !u.is_empty() => u.clone(),
            _ => match default_user {
                Some(u) if !u.is_empty() => u.to_string(),
                _ => {
                    skipped.push((
                        host.name.clone(),
                        "Username cannot be empty and $USER is not set".to_string(),
                    ));
                    continue;
                }
            },
        };
        if let Err(e) = Storage::validate_username(&username) {
            skipped.push((host.name.clone(), format!("Invalid username: {}", e)));
            continue;
        }

        if let Err(e) = Storage::validate_port(host.port) {
            skipped.push((host.name.clone(), format!("Invalid port: {}", e)));
            continue;
        }

        let key_path = match &host.identity_file {
            Some(k) => k.clone(),
            None => {
                skipped.push((
                    host.name.clone(),
                    "no IdentityFile; add manually with --password".to_string(),
                ));
                continue;
            }
        };

        if !validate_key_exists(&key_path) {
            skipped.push((
                host.name.clone(),
                format!("SSH key file not found: {}", key_path),
            ));
            continue;
        }

        let mut server = Server::new(
            host.name.clone(),
            host.host.clone(),
            username,
            host.port,
            AuthMethod::Key,
            Some(key_path),
        );
        server.use_agent_forwarding = host.forward_agent;
        to_import.push(server);
    }

    ImportResult { to_import, skipped }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_method_serializes_lowercase() {
        let k = serde_json::to_string(&AuthMethod::Key).unwrap();
        let p = serde_json::to_string(&AuthMethod::Password).unwrap();
        assert_eq!(k, "\"key\"");
        assert_eq!(p, "\"password\"");
    }

    #[test]
    fn server_round_trips_through_json() {
        let mut s = Server::new(
            "alpha".to_string(),
            "example.com".to_string(),
            "alice".to_string(),
            22,
            AuthMethod::Key,
            Some("/home/alice/.ssh/id_ed25519".to_string()),
        );
        s.use_agent_forwarding = true;
        s.tags = vec!["prod".to_string(), "web".to_string()];
        let json = serde_json::to_string(&s).unwrap();
        let back: Server = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "alpha");
        assert_eq!(back.auth_method, AuthMethod::Key);
        assert!(back.use_agent_forwarding);
        assert_eq!(back.tags, vec!["prod".to_string(), "web".to_string()]);
    }

    #[test]
    fn server_tags_omitted_when_empty() {
        let s = Server::new(
            "alpha".to_string(),
            "example.com".to_string(),
            "alice".to_string(),
            22,
            AuthMethod::Password,
            None,
        );
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("tags"), "json was: {json}");
    }

    #[test]
    fn server_parses_from_json_without_tags_backward_compat() {
        let legacy_json = r#"{"name":"legacy","host":"legacy.net","username":"user","port":22,"auth_method":"key"}"#;
        let s: Server = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(s.name, "legacy");
        assert!(s.tags.is_empty());
    }

    #[test]
    fn server_key_path_omitted_when_none() {
        let s = Server::new(
            "alpha".to_string(),
            "example.com".to_string(),
            "alice".to_string(),
            22,
            AuthMethod::Password,
            None,
        );
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("key_path"), "json was: {json}");
    }

    #[test]
    fn server_agent_forwarding_defaults_false() {
        let s = Server::new(
            "alpha".to_string(),
            "example.com".to_string(),
            "alice".to_string(),
            22,
            AuthMethod::Key,
            None,
        );
        assert!(!s.use_agent_forwarding);
    }

    #[test]
    fn config_find_server_returns_match() {
        let mut cfg = Config::new();
        cfg.servers.push(Server::new(
            "alpha".to_string(),
            "example.com".to_string(),
            "alice".to_string(),
            22,
            AuthMethod::Key,
            None,
        ));
        assert!(cfg.find_server("alpha").is_some());
        assert!(cfg.find_server("missing").is_none());
    }

    #[test]
    fn config_round_trip_includes_preferred_terminal() {
        let mut cfg = Config::new();
        cfg.preferred_terminal = Some("xterm".to_string());
        let json = serde_json::to_string(&cfg).unwrap();
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.preferred_terminal.as_deref(), Some("xterm"));
    }

    fn sample_server(name: &str) -> Server {
        Server::new(
            name.to_string(),
            "10.0.0.1".to_string(),
            "alice".to_string(),
            22,
            AuthMethod::Password,
            None,
        )
    }

    #[test]
    fn edit_apply_nothing_to_change() {
        let mut s = sample_server("alpha");
        let cfg = Config::new();
        let edit = ServerEdit::default();
        let res = edit.apply(&mut s, &cfg);
        assert!(matches!(res, Err(SshManagerError::NothingToChange)));
    }

    #[test]
    fn edit_apply_each_field() {
        let mut s = sample_server("alpha");
        let cfg = Config::new();

        let edit_host = ServerEdit {
            host: Some("example.com".to_string()),
            ..Default::default()
        };
        assert!(edit_host.apply(&mut s, &cfg).is_ok());
        assert_eq!(s.host, "example.com");

        let edit_user = ServerEdit {
            username: Some("bob".to_string()),
            ..Default::default()
        };
        assert!(edit_user.apply(&mut s, &cfg).is_ok());
        assert_eq!(s.username, "bob");

        let edit_port = ServerEdit {
            port: Some(2222),
            ..Default::default()
        };
        assert!(edit_port.apply(&mut s, &cfg).is_ok());
        assert_eq!(s.port, 2222);

        let edit_af = ServerEdit {
            agent_forwarding: Some(true),
            ..Default::default()
        };
        assert!(edit_af.apply(&mut s, &cfg).is_ok());
        assert!(s.use_agent_forwarding);

        let edit_af_off = ServerEdit {
            agent_forwarding: Some(false),
            ..Default::default()
        };
        assert!(edit_af_off.apply(&mut s, &cfg).is_ok());
        assert!(!s.use_agent_forwarding);

        let edit_name = ServerEdit {
            new_name: Some("beta".to_string()),
            ..Default::default()
        };
        assert!(edit_name.apply(&mut s, &cfg).is_ok());
        assert_eq!(s.name, "beta");

        let edit_tags = ServerEdit {
            tags: Some(vec!["PROD".to_string(), "Web".to_string()]),
            ..Default::default()
        };
        assert!(edit_tags.apply(&mut s, &cfg).is_ok());
        assert_eq!(s.tags, vec!["prod".to_string(), "web".to_string()]);

        let edit_tags_clear = ServerEdit {
            tags: Some(Vec::new()),
            ..Default::default()
        };
        assert!(edit_tags_clear.apply(&mut s, &cfg).is_ok());
        assert!(s.tags.is_empty());
    }

    #[test]
    fn edit_apply_validation_failures() {
        let mut s = sample_server("alpha");
        let cfg = Config::new();

        let bad_tag = ServerEdit {
            tags: Some(vec!["invalid tag!".to_string()]),
            ..Default::default()
        };
        assert!(matches!(
            bad_tag.apply(&mut s, &cfg),
            Err(SshManagerError::InvalidTag(_))
        ));

        let bad_name = ServerEdit {
            new_name: Some("".to_string()),
            ..Default::default()
        };
        assert!(matches!(
            bad_name.apply(&mut s, &cfg),
            Err(SshManagerError::EmptyServerName)
        ));

        let invalid_name = ServerEdit {
            new_name: Some("bad name!".to_string()),
            ..Default::default()
        };
        assert!(matches!(
            invalid_name.apply(&mut s, &cfg),
            Err(SshManagerError::InvalidServerName)
        ));

        let bad_host = ServerEdit {
            host: Some("".to_string()),
            ..Default::default()
        };
        assert!(matches!(
            bad_host.apply(&mut s, &cfg),
            Err(SshManagerError::EmptyHost)
        ));

        let bad_user = ServerEdit {
            username: Some("".to_string()),
            ..Default::default()
        };
        assert!(matches!(
            bad_user.apply(&mut s, &cfg),
            Err(SshManagerError::EmptyUsername)
        ));

        let bad_port = ServerEdit {
            port: Some(0),
            ..Default::default()
        };
        assert!(matches!(
            bad_port.apply(&mut s, &cfg),
            Err(SshManagerError::InvalidPort)
        ));

        let bad_key = ServerEdit {
            auth: Some(AuthEdit::Key("/nonexistent/key/file/path".to_string())),
            ..Default::default()
        };
        assert!(matches!(
            bad_key.apply(&mut s, &cfg),
            Err(SshManagerError::KeyFileNotFound(_))
        ));
    }

    #[test]
    fn edit_apply_key_password_switch() {
        let key_file = tempfile::NamedTempFile::new().unwrap();
        let key_path = key_file.path().to_str().unwrap().to_string();

        let mut s = sample_server("alpha");
        let cfg = Config::new();
        assert_eq!(s.auth_method, AuthMethod::Password);
        assert_eq!(s.key_path, None);

        // Switch to Key
        let switch_to_key = ServerEdit {
            auth: Some(AuthEdit::Key(key_path.clone())),
            ..Default::default()
        };
        assert!(switch_to_key.apply(&mut s, &cfg).is_ok());
        assert_eq!(s.auth_method, AuthMethod::Key);
        assert_eq!(s.key_path, Some(key_path));

        // Switch back to Password
        let switch_to_pw = ServerEdit {
            auth: Some(AuthEdit::Password),
            ..Default::default()
        };
        assert!(switch_to_pw.apply(&mut s, &cfg).is_ok());
        assert_eq!(s.auth_method, AuthMethod::Password);
        assert_eq!(s.key_path, None);
    }

    #[test]
    fn edit_apply_rename_uniqueness() {
        let mut cfg = Config::new();
        cfg.servers.push(sample_server("alpha"));
        cfg.servers.push(sample_server("beta"));

        let mut s = cfg.servers[0].clone();

        // Renaming to already existing "beta" should fail
        let dup = ServerEdit {
            new_name: Some("beta".to_string()),
            ..Default::default()
        };
        assert!(matches!(
            dup.apply(&mut s, &cfg),
            Err(SshManagerError::ServerAlreadyExists(_))
        ));

        // Renaming to same name "alpha" should succeed
        let same = ServerEdit {
            new_name: Some("alpha".to_string()),
            ..Default::default()
        };
        assert!(same.apply(&mut s, &cfg).is_ok());

        // Renaming to unused "gamma" should succeed
        let unused = ServerEdit {
            new_name: Some("gamma".to_string()),
            ..Default::default()
        };
        assert!(unused.apply(&mut s, &cfg).is_ok());
        assert_eq!(s.name, "gamma");
    }

    #[test]
    fn filter_matching_tests() {
        let s = Server::new(
            "prod-db".to_string(),
            "database.internal.net".to_string(),
            "admin".to_string(),
            5432,
            AuthMethod::Password,
            None,
        );

        // Empty matches everything
        assert!(server_matches_query(&s, ""));

        // Match on name (case-insensitive)
        assert!(server_matches_query(&s, "prod"));
        assert!(server_matches_query(&s, "PROD-DB"));
        assert!(server_matches_query(&s, "-db"));

        // Match on host (case-insensitive)
        assert!(server_matches_query(&s, "internal"));
        assert!(server_matches_query(&s, "DATABASE"));

        // Non-matching
        assert!(!server_matches_query(&s, "staging"));
        assert!(!server_matches_query(&s, "xyz"));

        // Match on tag (case-insensitive)
        let mut s_with_tags = s.clone();
        s_with_tags.tags = vec!["backend".to_string(), "critical".to_string()];
        assert!(server_matches_query(&s_with_tags, "back"));
        assert!(server_matches_query(&s_with_tags, "BACKEND"));
        assert!(server_matches_query(&s_with_tags, "crit"));
        assert!(!server_matches_query(&s_with_tags, "frontend"));

        // filter_server_indices
        let mut s1 = sample_server("web-prod");
        s1.tags = vec!["frontend".to_string()];
        let mut s2 = sample_server("db-prod");
        s2.tags = vec!["database".to_string()];
        let mut s3 = sample_server("staging-web");
        s3.tags = vec!["frontend".to_string()];

        let servers = vec![s1, s2, s3];
        let indices = filter_server_indices(&servers, "prod");
        assert_eq!(indices, vec![0, 1]);

        let indices_tag = filter_server_indices(&servers, "frontend");
        assert_eq!(indices_tag, vec![0, 2]);

        let indices_none = filter_server_indices(&servers, "nomatch");
        assert!(indices_none.is_empty());
    }

    #[test]
    fn resolve_server_exact_wins() {
        let servers = vec![sample_server("web"), sample_server("web-prod")];
        let resolved = resolve_server(&servers, "web").expect("should resolve");
        assert_eq!(resolved.name, "web");
    }

    #[test]
    fn resolve_server_unique_partial() {
        let servers = vec![sample_server("web-prod"), sample_server("db-staging")];
        let resolved = resolve_server(&servers, "staging").expect("should resolve");
        assert_eq!(resolved.name, "db-staging");

        // Case-insensitive partial
        let resolved_ci = resolve_server(&servers, "STAGING").expect("should resolve");
        assert_eq!(resolved_ci.name, "db-staging");
    }

    #[test]
    fn resolve_server_ambiguous() {
        let servers = vec![sample_server("prod-web"), sample_server("prod-db")];
        let err = resolve_server(&servers, "prod").unwrap_err();
        match err {
            SshManagerError::AmbiguousServerName(query, candidates) => {
                assert_eq!(query, "prod");
                assert_eq!(candidates, "prod-db, prod-web");
            }
            other => panic!("expected AmbiguousServerName, got {other:?}"),
        }
    }

    #[test]
    fn resolve_server_none() {
        let servers = vec![sample_server("prod-web")];
        let err = resolve_server(&servers, "staging").unwrap_err();
        assert!(matches!(err, SshManagerError::ServerNotFound(q) if q == "staging"));

        // Empty query
        let empty_err = resolve_server(&servers, "").unwrap_err();
        assert!(matches!(empty_err, SshManagerError::ServerNotFound(q) if q.is_empty()));
    }

    #[test]
    fn format_server_completions_empty() {
        let servers: Vec<Server> = Vec::new();
        assert_eq!(format_server_completions(&servers), "");
    }

    #[test]
    fn format_server_completions_single() {
        let servers = vec![sample_server("alpha")];
        assert_eq!(format_server_completions(&servers), "alpha\n");
    }

    #[test]
    fn format_server_completions_multiple() {
        let servers = vec![
            sample_server("web-prod"),
            sample_server("db-staging"),
            sample_server("backup_01"),
        ];
        assert_eq!(
            format_server_completions(&servers),
            "web-prod\ndb-staging\nbackup_01\n"
        );
    }

    #[test]
    fn format_tag_completions_empty() {
        let servers: Vec<Server> = Vec::new();
        assert_eq!(format_tag_completions(&servers), "");
    }

    #[test]
    fn format_tag_completions_multiple_deduplicated_and_sorted() {
        let mut s1 = sample_server("s1");
        s1.tags = vec!["web".to_string(), "prod".to_string()];
        let mut s2 = sample_server("s2");
        s2.tags = vec!["prod".to_string(), "api".to_string()];
        let mut s3 = sample_server("s3");
        s3.tags = vec![];
        let servers = vec![s1, s2, s3];
        assert_eq!(format_tag_completions(&servers), "api\nprod\nweb\n");
    }

    #[test]
    fn parse_ssh_config_basic_host() {
        let input = r#"
Host prod
    HostName 10.0.0.1
    User alice
    Port 2222
    IdentityFile ~/.ssh/id_ed25519
    ForwardAgent yes
"#;
        let hosts = parse_ssh_config(input, Some("/home/alice"));
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "prod");
        assert_eq!(hosts[0].host, "10.0.0.1");
        assert_eq!(hosts[0].username.as_deref(), Some("alice"));
        assert_eq!(hosts[0].port, 2222);
        assert_eq!(
            hosts[0].identity_file.as_deref(),
            Some("/home/alice/.ssh/id_ed25519")
        );
        assert!(hosts[0].forward_agent);
    }

    #[test]
    fn parse_ssh_config_multiple_patterns() {
        let input = r#"
Host web1 web2
    HostName cluster.internal
    IdentityFile /keys/deploy
"#;
        let hosts = parse_ssh_config(input, None);
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0].name, "web1");
        assert_eq!(hosts[0].host, "cluster.internal");
        assert_eq!(hosts[0].identity_file.as_deref(), Some("/keys/deploy"));
        assert_eq!(hosts[1].name, "web2");
        assert_eq!(hosts[1].host, "cluster.internal");
        assert_eq!(hosts[1].identity_file.as_deref(), Some("/keys/deploy"));
    }

    #[test]
    fn parse_ssh_config_wildcard_skipped() {
        let input = r#"
Host *.example.com
    IdentityFile /keys/star

Host host?
    IdentityFile /keys/q

Host !badhost goodhost
    IdentityFile /keys/good

Host *
    IdentityFile /keys/default
"#;
        let hosts = parse_ssh_config(input, None);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "goodhost");
        assert_eq!(hosts[0].identity_file.as_deref(), Some("/keys/good"));
    }

    #[test]
    fn parse_ssh_config_key_value_form() {
        let input = r#"
Host=alpha
HostName=alpha.example.com
User=deployer
Port=2201
IdentityFile=/keys/alpha
ForwardAgent=yes
"#;
        let hosts = parse_ssh_config(input, None);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "alpha");
        assert_eq!(hosts[0].host, "alpha.example.com");
        assert_eq!(hosts[0].username.as_deref(), Some("deployer"));
        assert_eq!(hosts[0].port, 2201);
        assert_eq!(hosts[0].identity_file.as_deref(), Some("/keys/alpha"));
        assert!(hosts[0].forward_agent);
    }

    #[test]
    fn parse_ssh_config_quotes() {
        let input = r#"
Host "quoted-host"
    HostName "quoted.net"
    User "john doe"
    IdentityFile "/path with spaces/id_rsa"
"#;
        let hosts = parse_ssh_config(input, None);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "quoted-host");
        assert_eq!(hosts[0].host, "quoted.net");
        assert_eq!(hosts[0].username.as_deref(), Some("john doe"));
        assert_eq!(
            hosts[0].identity_file.as_deref(),
            Some("/path with spaces/id_rsa")
        );
    }

    #[test]
    fn parse_ssh_config_missing_identity_file_skipped() {
        let input = r#"
Host no-key
    HostName nokey.example.com
    User alice

Host has-key
    HostName haskey.example.com
    IdentityFile /keys/id_rsa
"#;
        let res = parse_ssh_config_detailed(input, None);
        assert_eq!(res.hosts.len(), 1);
        assert_eq!(res.hosts[0].name, "has-key");
        assert_eq!(res.skipped.len(), 1);
        assert_eq!(res.skipped[0].0, "no-key");
        assert!(res.skipped[0]
            .1
            .contains("no IdentityFile; add manually with --password"));
    }

    #[test]
    fn parse_ssh_config_tilde_expansion() {
        let input = r#"
Host home-test
    IdentityFile ~/.ssh/custom_key
"#;
        let hosts_expanded = parse_ssh_config(input, Some("/fake/home/user"));
        assert_eq!(hosts_expanded.len(), 1);
        assert_eq!(
            hosts_expanded[0].identity_file.as_deref(),
            Some("/fake/home/user/.ssh/custom_key")
        );

        let hosts_unexpanded = parse_ssh_config(input, None);
        assert_eq!(hosts_unexpanded.len(), 1);
        assert_eq!(
            hosts_unexpanded[0].identity_file.as_deref(),
            Some("~/.ssh/custom_key")
        );
    }

    #[test]
    fn parse_ssh_config_invalid_port() {
        let input = r#"
Host bad-port-alpha
    Port notanumber
    IdentityFile /keys/k1

Host bad-port-zero
    Port 0
    IdentityFile /keys/k2

Host bad-port-overflow
    Port 70000
    IdentityFile /keys/k3
"#;
        let res = parse_ssh_config_detailed(input, None);
        assert_eq!(res.hosts.len(), 3);
        assert_eq!(res.hosts[0].port, 22);
        assert_eq!(res.hosts[1].port, 22);
        assert_eq!(res.hosts[2].port, 22);
        assert_eq!(res.warnings.len(), 3);
    }

    #[test]
    fn parse_ssh_config_comments() {
        let input = r#"
# Global comment
# Another comment
Host myhost # inline comment after host
    # Indented comment
    HostName 10.1.1.1 # inline comment on hostname
    Port 22 # default port
    IdentityFile /keys/mykey
"#;
        let hosts = parse_ssh_config(input, None);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "myhost");
        assert_eq!(hosts[0].host, "10.1.1.1");
        assert_eq!(hosts[0].port, 22);
        assert_eq!(hosts[0].identity_file.as_deref(), Some("/keys/mykey"));
    }

    #[test]
    fn parse_ssh_config_unknown_keywords_ignored() {
        let input = r#"
Host complex
    ServerAliveInterval 30
    StrictHostKeyChecking no
    ProxyJump bastion.example.com
    UserKnownHostsFile /dev/null
    IdentityFile /keys/id_rsa
"#;
        let hosts = parse_ssh_config(input, None);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "complex");
        assert_eq!(hosts[0].identity_file.as_deref(), Some("/keys/id_rsa"));
    }

    #[test]
    fn parse_ssh_config_include_and_match_warned() {
        let input = r#"
Include /etc/ssh/extra_config
Include /etc/ssh/more_config
Host srv
    IdentityFile /keys/id_rsa
Match host complex
    Port 2200
Match host other
    Port 2201
"#;
        let res = parse_ssh_config_detailed(input, None);
        assert_eq!(res.hosts.len(), 1);
        assert_eq!(res.hosts[0].name, "srv");
        assert_eq!(res.warnings.len(), 2);
        assert!(res
            .warnings
            .iter()
            .any(|w| w.contains("Include directive is not followed")));
        assert!(res
            .warnings
            .iter()
            .any(|w| w.contains("Match directive is not followed")));
    }

    #[test]
    fn filter_and_map_imported_hosts_basic() {
        let cfg = Config::new();
        let imported = vec![ImportedHost {
            name: "srv1".to_string(),
            host: "1.2.3.4".to_string(),
            username: Some("alice".to_string()),
            port: 22,
            identity_file: Some("/valid/key".to_string()),
            forward_agent: true,
        }];

        let res = filter_and_map_imported_hosts(&cfg, imported, None, |_| true);
        assert_eq!(res.to_import.len(), 1);
        assert_eq!(res.skipped.len(), 0);
        assert_eq!(res.to_import[0].name, "srv1");
        assert_eq!(res.to_import[0].auth_method, AuthMethod::Key);
        assert_eq!(res.to_import[0].key_path.as_deref(), Some("/valid/key"));
        assert!(res.to_import[0].use_agent_forwarding);
    }

    #[test]
    fn filter_and_map_imported_hosts_collision_skipped() {
        let mut cfg = Config::new();
        cfg.servers.push(sample_server("existing"));

        let imported = vec![
            ImportedHost {
                name: "existing".to_string(),
                host: "2.2.2.2".to_string(),
                username: Some("bob".to_string()),
                port: 22,
                identity_file: Some("/key".to_string()),
                forward_agent: false,
            },
            ImportedHost {
                name: "batch_dup".to_string(),
                host: "3.3.3.3".to_string(),
                username: Some("bob".to_string()),
                port: 22,
                identity_file: Some("/key".to_string()),
                forward_agent: false,
            },
            ImportedHost {
                name: "batch_dup".to_string(),
                host: "4.4.4.4".to_string(),
                username: Some("bob".to_string()),
                port: 22,
                identity_file: Some("/key".to_string()),
                forward_agent: false,
            },
        ];

        let res = filter_and_map_imported_hosts(&cfg, imported, None, |_| true);
        assert_eq!(res.to_import.len(), 1);
        assert_eq!(res.to_import[0].name, "batch_dup");
        assert_eq!(res.skipped.len(), 2);
        assert_eq!(res.skipped[0].0, "existing");
        assert_eq!(res.skipped[0].1, "Server already exists");
        assert_eq!(res.skipped[1].0, "batch_dup");
        assert_eq!(res.skipped[1].1, "Server already exists");
    }

    #[test]
    fn filter_and_map_imported_hosts_missing_key_skipped() {
        let cfg = Config::new();
        let imported = vec![ImportedHost {
            name: "srv".to_string(),
            host: "1.2.3.4".to_string(),
            username: Some("alice".to_string()),
            port: 22,
            identity_file: Some("/missing/key".to_string()),
            forward_agent: false,
        }];

        let res = filter_and_map_imported_hosts(&cfg, imported, None, |p| p != "/missing/key");
        assert_eq!(res.to_import.len(), 0);
        assert_eq!(res.skipped.len(), 1);
        assert!(res.skipped[0].1.contains("SSH key file not found"));
    }

    #[test]
    fn filter_and_map_imported_hosts_user_fallback() {
        let cfg = Config::new();
        let imported = vec![
            ImportedHost {
                name: "srv1".to_string(),
                host: "1.2.3.4".to_string(),
                username: None,
                port: 22,
                identity_file: Some("/key".to_string()),
                forward_agent: false,
            },
            ImportedHost {
                name: "srv2".to_string(),
                host: "1.2.3.5".to_string(),
                username: None,
                port: 22,
                identity_file: Some("/key".to_string()),
                forward_agent: false,
            },
        ];

        // With default_user Some("fallback_user")
        let res_with_fallback =
            filter_and_map_imported_hosts(&cfg, imported.clone(), Some("fallback_user"), |_| true);
        assert_eq!(res_with_fallback.to_import.len(), 2);
        assert_eq!(res_with_fallback.to_import[0].username, "fallback_user");

        // Without default_user
        let res_without_fallback = filter_and_map_imported_hosts(&cfg, imported, None, |_| true);
        assert_eq!(res_without_fallback.to_import.len(), 0);
        assert_eq!(res_without_fallback.skipped.len(), 2);
        assert!(res_without_fallback.skipped[0]
            .1
            .contains("Username cannot be empty"));
    }

    #[test]
    fn filter_and_map_imported_hosts_invalid_server_name() {
        let cfg = Config::new();
        let imported = vec![ImportedHost {
            name: "bad server name!".to_string(),
            host: "1.2.3.4".to_string(),
            username: Some("alice".to_string()),
            port: 22,
            identity_file: Some("/key".to_string()),
            forward_agent: false,
        }];

        let res = filter_and_map_imported_hosts(&cfg, imported, None, |_| true);
        assert_eq!(res.to_import.len(), 0);
        assert_eq!(res.skipped.len(), 1);
        assert!(res.skipped[0].1.contains("Invalid server name"));
    }

    #[test]
    fn resolve_jump_none_when_no_jump_host() {
        let cfg = Config::new();
        let server = sample_server("srv");
        assert_eq!(resolve_jump(&cfg, &server).unwrap(), None);
    }

    #[test]
    fn resolve_jump_raw_spec_passthrough() {
        let cfg = Config::new();
        let mut server = sample_server("srv");
        server.jump_host = Some("user@jump.example.com:2222".to_string());
        assert_eq!(
            resolve_jump(&cfg, &server).unwrap(),
            Some("user@jump.example.com:2222".to_string())
        );
    }

    #[test]
    fn resolve_jump_expands_saved_server_single_hop() {
        let mut cfg = Config::new();
        let mut bastion = sample_server("bastion");
        bastion.username = "bastion_user".to_string();
        bastion.host = "bastion.corp.net".to_string();
        bastion.port = 2200;
        cfg.servers.push(bastion);

        let mut srv = sample_server("srv");
        srv.jump_host = Some("bastion".to_string());
        assert_eq!(
            resolve_jump(&cfg, &srv).unwrap(),
            Some("bastion_user@bastion.corp.net:2200".to_string())
        );
    }

    #[test]
    fn resolve_jump_chains_saved_servers_outermost_first() {
        let mut cfg = Config::new();
        // A has no jump host
        let mut a = sample_server("A");
        a.username = "usera".to_string();
        a.host = "hosta".to_string();
        a.port = 22;
        cfg.servers.push(a);

        // B jumps through A
        let mut b = sample_server("B");
        b.username = "userb".to_string();
        b.host = "hostb".to_string();
        b.port = 2222;
        b.jump_host = Some("A".to_string());
        cfg.servers.push(b);

        // C jumps through B
        let mut c = sample_server("C");
        c.username = "userc".to_string();
        c.host = "hostc".to_string();
        c.port = 22;
        c.jump_host = Some("B".to_string());
        cfg.servers.push(c.clone());

        assert_eq!(
            resolve_jump(&cfg, &c).unwrap(),
            Some("usera@hosta:22,userb@hostb:2222".to_string())
        );
    }

    #[test]
    fn resolve_jump_chains_saved_server_with_raw_spec() {
        let mut cfg = Config::new();
        // B jumps through raw spec
        let mut b = sample_server("B");
        b.username = "userb".to_string();
        b.host = "hostb".to_string();
        b.port = 2222;
        b.jump_host = Some("jump.raw.com:2200".to_string());
        cfg.servers.push(b);

        // C jumps through B
        let mut c = sample_server("C");
        c.jump_host = Some("B".to_string());

        assert_eq!(
            resolve_jump(&cfg, &c).unwrap(),
            Some("jump.raw.com:2200,userb@hostb:2222".to_string())
        );
    }

    #[test]
    fn resolve_jump_cycle_detection() {
        let mut cfg = Config::new();
        let mut a = sample_server("A");
        a.jump_host = Some("B".to_string());
        let mut b = sample_server("B");
        b.jump_host = Some("A".to_string());
        cfg.servers.push(a.clone());
        cfg.servers.push(b);

        let err = resolve_jump(&cfg, &a).unwrap_err();
        assert!(matches!(err, SshManagerError::JumpHostCycle(msg) if msg.contains("A -> B -> A")));
    }

    #[test]
    fn resolve_jump_depth_limit() {
        let mut cfg = Config::new();
        // Create a chain of 6 servers: s1 -> s2 -> s3 -> s4 -> s5 -> s6
        for i in 1..=6 {
            let mut s = sample_server(&format!("s{}", i));
            if i < 6 {
                s.jump_host = Some(format!("s{}", i + 1));
            }
            cfg.servers.push(s);
        }

        // Target s0 jumps through s1 -> s2 -> s3 -> s4 -> s5 (5 hops allowed)
        let mut s0 = sample_server("s0");
        s0.jump_host = Some("s1".to_string());
        // Truncate config so s5 has no jump host: chain s1..s5 is 5 hops
        cfg.servers[4].jump_host = None;
        let res5 = resolve_jump(&cfg, &s0);
        assert!(res5.is_ok());

        // Restore s5 -> s6: chain s1..s6 is 6 hops -> exceeds depth 5
        cfg.servers[4].jump_host = Some("s6".to_string());
        let res6 = resolve_jump(&cfg, &s0);
        assert!(matches!(res6, Err(SshManagerError::JumpHostDepthExceeded)));
    }

    #[test]
    fn update_jump_host_references_updates_dependents() {
        let mut servers = vec![sample_server("A"), sample_server("B"), sample_server("C")];
        servers[1].jump_host = Some("A".to_string());
        servers[2].jump_host = Some("A".to_string());

        let count = update_jump_host_references(&mut servers, "A", "A_new");
        assert_eq!(count, 2);
        assert_eq!(servers[1].jump_host.as_deref(), Some("A_new"));
        assert_eq!(servers[2].jump_host.as_deref(), Some("A_new"));
    }

    #[test]
    fn find_jump_dependents_finds_all() {
        let mut servers = vec![sample_server("A"), sample_server("B"), sample_server("C")];
        servers[1].jump_host = Some("A".to_string());
        servers[2].jump_host = Some("A".to_string());

        let mut deps = find_jump_dependents(&servers, "A");
        deps.sort();
        assert_eq!(deps, vec!["B".to_string(), "C".to_string()]);

        let deps_empty = find_jump_dependents(&servers, "B");
        assert!(deps_empty.is_empty());
    }

    #[test]
    fn edit_apply_jump_host() {
        let cfg = Config::new();
        let mut s = sample_server("srv");
        assert_eq!(s.jump_host, None);

        let edit = ServerEdit {
            jump_host: Some(Some("bastion.example.com".to_string())),
            ..Default::default()
        };
        edit.apply(&mut s, &cfg).unwrap();
        assert_eq!(s.jump_host.as_deref(), Some("bastion.example.com"));

        // Clear jump host
        let clear_edit = ServerEdit {
            jump_host: Some(None),
            ..Default::default()
        };
        clear_edit.apply(&mut s, &cfg).unwrap();
        assert_eq!(s.jump_host, None);
    }

    #[test]
    fn edit_apply_jump_host_validation_failures() {
        let mut cfg = Config::new();
        let mut bastion = sample_server("bastion");
        bastion.jump_host = Some("srv".to_string());
        cfg.servers.push(bastion);

        let mut s = sample_server("srv");

        // Empty jump host
        let err_empty = ServerEdit {
            jump_host: Some(Some("".to_string())),
            ..Default::default()
        }
        .apply(&mut s, &cfg);
        assert!(matches!(err_empty, Err(SshManagerError::EmptyJumpHost)));

        // Shell metacharacter in jump host
        let err_meta = ServerEdit {
            jump_host: Some(Some("bastion; rm -rf /".to_string())),
            ..Default::default()
        }
        .apply(&mut s, &cfg);
        assert!(matches!(err_meta, Err(SshManagerError::InvalidJumpHost(_))));

        // Self referential jump host
        let err_self = ServerEdit {
            jump_host: Some(Some("srv".to_string())),
            ..Default::default()
        }
        .apply(&mut s, &cfg);
        assert!(matches!(
            err_self,
            Err(SshManagerError::SelfReferentialJumpHost(_))
        ));

        // Cycle: srv -> bastion -> srv
        let err_cycle = ServerEdit {
            jump_host: Some(Some("bastion".to_string())),
            ..Default::default()
        }
        .apply(&mut s, &cfg);
        assert!(matches!(err_cycle, Err(SshManagerError::JumpHostCycle(_))));
    }

    #[test]
    fn server_parses_from_json_without_jump_host_backward_compat() {
        let json = r#"{
            "name": "srv",
            "host": "1.2.3.4",
            "username": "alice",
            "port": 22,
            "auth_method": "key"
        }"#;
        let s: Server = serde_json::from_str(json).unwrap();
        assert_eq!(s.jump_host, None);
    }

    #[test]
    fn server_round_trips_through_json_with_jump_host() {
        let mut s = sample_server("srv");
        s.jump_host = Some("bastion:2222".to_string());
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"jump_host\":\"bastion:2222\""));

        let deserialized: Server = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.jump_host.as_deref(), Some("bastion:2222"));
    }

    #[test]
    fn server_matches_query_with_jump_host() {
        let mut s = sample_server("srv");
        s.jump_host = Some("bastion.corp".to_string());
        assert!(server_matches_query(&s, "bastion"));
        assert!(!server_matches_query(&s, "otherproxy"));
    }

    #[test]
    fn forward_parse_shorthand() {
        let f = Forward::parse("5432").unwrap();
        assert_eq!(
            f,
            Forward::Local {
                bind_port: 5432,
                host: "localhost".to_string(),
                host_port: 5432,
            }
        );
        assert_eq!(f.to_string(), "L:5432:localhost:5432");
    }

    #[test]
    fn forward_parse_canonical_and_case_insensitive() {
        let local = Forward::parse("L:8080:127.0.0.1:80").unwrap();
        assert_eq!(
            local,
            Forward::Local {
                bind_port: 8080,
                host: "127.0.0.1".to_string(),
                host_port: 80,
            }
        );
        assert_eq!(local.to_string(), "L:8080:127.0.0.1:80");

        let local_lower = Forward::parse("l:8080:127.0.0.1:80").unwrap();
        assert_eq!(local_lower, local);

        let remote = Forward::parse("R:2222:internal.host:22").unwrap();
        assert_eq!(
            remote,
            Forward::Remote {
                bind_port: 2222,
                host: "internal.host".to_string(),
                host_port: 22,
            }
        );
        assert_eq!(remote.to_string(), "R:2222:internal.host:22");

        let dynamic = Forward::parse("D:1080").unwrap();
        assert_eq!(dynamic, Forward::Dynamic { bind_port: 1080 });
        assert_eq!(dynamic.to_string(), "D:1080");

        let dynamic_lower = Forward::parse("d:1080").unwrap();
        assert_eq!(dynamic_lower, dynamic);
    }

    #[test]
    fn forward_parse_invalid() {
        assert!(Forward::parse("").is_err());
        assert!(Forward::parse("   ").is_err());
        assert!(Forward::parse("0").is_err());
        assert!(Forward::parse("65536").is_err());
        assert!(Forward::parse("L:0:localhost:80").is_err());
        assert!(Forward::parse("L:80:localhost:0").is_err());
        assert!(Forward::parse("L:80::80").is_err());
        assert!(Forward::parse("L:80:bad host:80").is_err());
        assert!(Forward::parse("L:80:bad;host:80").is_err());
        assert!(Forward::parse("L:80:bad$host:80").is_err());
        assert!(Forward::parse("D:0").is_err());
        assert!(Forward::parse("D:1080:extra").is_err());
        assert!(Forward::parse("X:1080").is_err());
    }

    #[test]
    fn forward_dedupe() {
        let forwards = vec![
            Forward::parse("5432").unwrap(),
            Forward::parse("L:5432:localhost:5432").unwrap(),
            Forward::parse("D:1080").unwrap(),
            Forward::parse("D:1080").unwrap(),
        ];
        let deduped = Forward::dedupe(&forwards);
        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].to_string(), "L:5432:localhost:5432");
        assert_eq!(deduped[1].to_string(), "D:1080");
    }

    #[test]
    fn server_parses_from_json_without_forwards_backward_compat() {
        let json = r#"{
            "name": "srv",
            "host": "1.2.3.4",
            "username": "alice",
            "port": 22,
            "auth_method": "key"
        }"#;
        let s: Server = serde_json::from_str(json).unwrap();
        assert!(s.forwards.is_empty());
    }

    #[test]
    fn server_forwards_omitted_when_empty() {
        let s = sample_server("srv");
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("forwards"));
    }

    #[test]
    fn server_round_trips_through_json_with_forwards() {
        let mut s = sample_server("srv");
        s.forwards = vec![
            Forward::parse("5432").unwrap(),
            Forward::parse("R:2222:host:22").unwrap(),
            Forward::parse("D:1080").unwrap(),
        ];
        let json = serde_json::to_string(&s).unwrap();
        assert!(
            json.contains("\"forwards\":[\"L:5432:localhost:5432\",\"R:2222:host:22\",\"D:1080\"]")
        );

        let deserialized: Server = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.forwards, s.forwards);
    }

    #[test]
    fn edit_apply_forwards() {
        let mut s = sample_server("srv");
        let cfg = Config::new();
        let edit = ServerEdit {
            forwards: Some(vec![
                Forward::parse("5432").unwrap(),
                Forward::parse("D:1080").unwrap(),
            ]),
            ..Default::default()
        };
        edit.apply(&mut s, &cfg).unwrap();
        assert_eq!(s.forwards.len(), 2);

        // Clear forwards
        let clear_edit = ServerEdit {
            forwards: Some(Vec::new()),
            ..Default::default()
        };
        clear_edit.apply(&mut s, &cfg).unwrap();
        assert!(s.forwards.is_empty());
    }

    #[test]
    fn edit_apply_forwards_validation_failures() {
        let mut s = sample_server("srv");
        let cfg = Config::new();
        let bad_edit = ServerEdit {
            forwards: Some(vec![Forward::Local {
                bind_port: 0,
                host: "localhost".to_string(),
                host_port: 80,
            }]),
            ..Default::default()
        };
        assert!(bad_edit.apply(&mut s, &cfg).is_err());
    }

    #[test]
    fn server_matches_query_with_forwards() {
        let mut s = sample_server("srv");
        s.forwards = vec![Forward::parse("5432").unwrap()];
        assert!(server_matches_query(&s, "5432"));
        assert!(server_matches_query(&s, "localhost"));
        assert!(!server_matches_query(&s, "9999"));
    }

    #[test]
    fn server_parses_from_json_without_favorite_and_last_used_backward_compat() {
        let json = r#"{
            "name": "srv",
            "host": "1.2.3.4",
            "username": "alice",
            "port": 22,
            "auth_method": "key"
        }"#;
        let s: Server = serde_json::from_str(json).unwrap();
        assert!(!s.favorite);
        assert_eq!(s.last_used, None);
    }

    #[test]
    fn server_favorite_and_last_used_omitted_when_default() {
        let s = sample_server("srv");
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("favorite"));
        assert!(!json.contains("last_used"));
    }

    #[test]
    fn server_round_trips_through_json_with_favorite_and_last_used() {
        let mut s = sample_server("srv");
        s.favorite = true;
        s.last_used = Some(1700000000);
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"favorite\":true"));
        assert!(json.contains("\"last_used\":1700000000"));

        let deserialized: Server = serde_json::from_str(&json).unwrap();
        assert!(deserialized.favorite);
        assert_eq!(deserialized.last_used, Some(1700000000));
    }

    #[test]
    fn sort_servers_favorites_first_then_mru_then_name() {
        let mut s_fav_old = sample_server("b_fav_old");
        s_fav_old.favorite = true;
        s_fav_old.last_used = Some(100);

        let mut s_fav_recent = sample_server("a_fav_recent");
        s_fav_recent.favorite = true;
        s_fav_recent.last_used = Some(200);

        let mut s_fav_never = sample_server("c_fav_never");
        s_fav_never.favorite = true;
        s_fav_never.last_used = None;

        let mut s_nonfav_recent = sample_server("z_nonfav_recent");
        s_nonfav_recent.favorite = false;
        s_nonfav_recent.last_used = Some(500);

        let mut s_nonfav_old = sample_server("y_nonfav_old");
        s_nonfav_old.favorite = false;
        s_nonfav_old.last_used = Some(50);

        let s_nonfav_never_b = sample_server("b_nonfav_never");
        let s_nonfav_never_a = sample_server("a_nonfav_never");

        let mut servers = [
            s_nonfav_never_b,
            s_nonfav_old,
            s_fav_never,
            s_nonfav_never_a,
            s_fav_old,
            s_nonfav_recent,
            s_fav_recent,
        ];

        servers.sort_by(sort_servers);

        let names: Vec<&str> = servers.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "a_fav_recent",    // fav, last_used: 200
                "b_fav_old",       // fav, last_used: 100
                "c_fav_never",     // fav, last_used: None
                "z_nonfav_recent", // non-fav, last_used: 500
                "y_nonfav_old",    // non-fav, last_used: 50
                "a_nonfav_never",  // non-fav, last_used: None, name tiebreak 'a'
                "b_nonfav_never",  // non-fav, last_used: None, name tiebreak 'b'
            ]
        );
    }

    #[test]
    fn format_last_used_relative_formatting() {
        let now = 100_000_000;

        assert_eq!(format_last_used(None, now), "never");
        assert_eq!(format_last_used(Some(now), now), "just now");
        assert_eq!(format_last_used(Some(now - 10), now), "just now");
        assert_eq!(format_last_used(Some(now - 59), now), "just now");
        assert_eq!(format_last_used(Some(now - 60), now), "1m ago");
        assert_eq!(format_last_used(Some(now - 120), now), "2m ago");
        assert_eq!(format_last_used(Some(now - 3599), now), "59m ago");
        assert_eq!(format_last_used(Some(now - 3600), now), "1h ago");
        assert_eq!(format_last_used(Some(now - 3 * 3600), now), "3h ago");
        assert_eq!(format_last_used(Some(now - 86399), now), "23h ago");
        assert_eq!(format_last_used(Some(now - 86400), now), "1d ago");
        assert_eq!(format_last_used(Some(now - 2 * 86400), now), "2d ago");
        assert_eq!(format_last_used(Some(now - 30 * 86400), now), "30d ago");
        // Future timestamp (clock skew)
        assert_eq!(format_last_used(Some(now + 100), now), "just now");
    }

    #[test]
    fn stamp_server_and_slice_pure_logic() {
        let mut s = sample_server("alpha");
        assert_eq!(s.last_used, None);

        stamp_server_last_used(&mut s, || 123456);
        assert_eq!(s.last_used, Some(123456));

        let mut servers = vec![sample_server("alpha"), sample_server("beta")];
        let found = stamp_server_in_slice(&mut servers, "beta", || 987654);
        assert!(found);
        assert_eq!(servers[1].last_used, Some(987654));
        assert_eq!(servers[0].last_used, None);

        let not_found = stamp_server_in_slice(&mut servers, "gamma", || 1111);
        assert!(!not_found);
    }

    #[test]
    fn edit_apply_preserves_favorite_and_last_used() {
        let mut s = sample_server("srv");
        s.favorite = true;
        s.last_used = Some(424242);

        let cfg = Config::new();
        let edit = ServerEdit {
            host: Some("updated.example.com".to_string()),
            port: Some(2222),
            ..Default::default()
        };

        edit.apply(&mut s, &cfg).unwrap();
        assert_eq!(s.host, "updated.example.com");
        assert_eq!(s.port, 2222);
        assert!(s.favorite, "favorite must be preserved untouched");
        assert_eq!(
            s.last_used,
            Some(424242),
            "last_used must be preserved untouched"
        );
    }

    #[test]
    fn find_selected_index_maintains_position() {
        let s1 = sample_server("alpha");
        let s2 = sample_server("beta");
        let s3 = sample_server("gamma");
        let servers = vec![s1, s2, s3];

        let filtered_indices = vec![0, 1, 2];
        assert_eq!(
            find_selected_index(&servers, &filtered_indices, "beta"),
            Some(1)
        );

        // After reorder, beta is at index 0
        let reordered = vec![sample_server("beta"), sample_server("alpha")];
        let reordered_filtered = vec![0, 1];
        assert_eq!(
            find_selected_index(&reordered, &reordered_filtered, "beta"),
            Some(0)
        );
        assert_eq!(
            find_selected_index(&reordered, &reordered_filtered, "missing"),
            None
        );
    }

    #[test]
    fn set_and_toggle_server_favorite_pure() {
        let mut servers = vec![sample_server("srv1"), sample_server("srv2")];

        assert!(!servers[0].favorite);
        assert!(set_server_favorite(&mut servers, "srv1", true));
        assert!(servers[0].favorite);

        let toggled = toggle_server_favorite(&mut servers, "srv1");
        assert_eq!(toggled, Some(false));
        assert!(!servers[0].favorite);

        let toggled2 = toggle_server_favorite(&mut servers, "srv1");
        assert_eq!(toggled2, Some(true));
        assert!(servers[0].favorite);

        assert_eq!(toggle_server_favorite(&mut servers, "nonexistent"), None);
        assert!(!set_server_favorite(&mut servers, "nonexistent", true));
    }

    #[test]
    fn select_exec_targets_target_exact_and_partial() {
        let s1 = sample_server("alpha-node");
        let s2 = sample_server("beta-node");
        let servers = vec![s1, s2];

        let res_exact = select_exec_targets(&servers, Some("alpha-node"), &[], false).unwrap();
        assert_eq!(res_exact.len(), 1);
        assert_eq!(res_exact[0].name, "alpha-node");

        let res_partial = select_exec_targets(&servers, Some("beta"), &[], false).unwrap();
        assert_eq!(res_partial.len(), 1);
        assert_eq!(res_partial[0].name, "beta-node");

        let err_missing = select_exec_targets(&servers, Some("gamma"), &[], false);
        assert!(matches!(
            err_missing,
            Err(SshManagerError::ServerNotFound(_))
        ));
    }

    #[test]
    fn select_exec_targets_all_preserves_order() {
        let s1 = sample_server("alpha");
        let s2 = sample_server("gamma");
        let s3 = sample_server("beta");
        let servers = vec![s1, s2, s3];

        let res = select_exec_targets(&servers, None, &[], true).unwrap();
        assert_eq!(res.len(), 3);
        assert_eq!(res[0].name, "alpha");
        assert_eq!(res[1].name, "gamma");
        assert_eq!(res[2].name, "beta");
    }

    #[test]
    fn select_exec_targets_all_empty_err() {
        let servers: Vec<Server> = Vec::new();
        let err = select_exec_targets(&servers, None, &[], true).unwrap_err();
        assert!(matches!(err, SshManagerError::NoServersMatch));
        assert!(err.to_string().to_lowercase().contains("no servers match"));
    }

    #[test]
    fn select_exec_targets_by_tag_single_and_multiple() {
        let mut s1 = sample_server("s1");
        s1.tags = vec!["prod".to_string(), "web".to_string()];
        let mut s2 = sample_server("s2");
        s2.tags = vec!["prod".to_string(), "db".to_string()];
        let mut s3 = sample_server("s3");
        s3.tags = vec!["staging".to_string(), "web".to_string()];
        let servers = vec![s1, s2, s3];

        // Single tag: "prod" matches s1 and s2 in list order
        let res_prod = select_exec_targets(&servers, None, &[String::from("prod")], false).unwrap();
        assert_eq!(res_prod.len(), 2);
        assert_eq!(res_prod[0].name, "s1");
        assert_eq!(res_prod[1].name, "s2");

        // Multiple tags (must have all): "prod" AND "web" matches only s1
        let res_both = select_exec_targets(
            &servers,
            None,
            &[String::from("prod"), String::from("web")],
            false,
        )
        .unwrap();
        assert_eq!(res_both.len(), 1);
        assert_eq!(res_both[0].name, "s1");

        // Case insensitivity
        let res_upper =
            select_exec_targets(&servers, None, &[String::from("PROD")], false).unwrap();
        assert_eq!(res_upper.len(), 2);

        // No matches returns empty-selection error
        let err = select_exec_targets(
            &servers,
            None,
            &[String::from("prod"), String::from("staging")],
            false,
        )
        .unwrap_err();
        assert!(matches!(err, SshManagerError::NoServersMatch));
        assert!(err.to_string().to_lowercase().contains("no servers match"));
    }

    #[test]
    fn select_exec_targets_empty_selection_when_no_flags() {
        let servers = vec![sample_server("s1")];
        let err = select_exec_targets(&servers, None, &[], false).unwrap_err();
        assert!(matches!(err, SshManagerError::NoServersMatch));
    }

    #[test]
    fn derive_copy_id_pub_path_key_server_default() {
        let server = Server::new(
            "srv".to_string(),
            "example.com".to_string(),
            "user".to_string(),
            22,
            AuthMethod::Key,
            Some("/home/user/.ssh/id_ed25519".to_string()),
        );
        let path = derive_copy_id_pub_path(&server, None).unwrap();
        assert_eq!(path, "/home/user/.ssh/id_ed25519.pub");
    }

    #[test]
    fn derive_copy_id_pub_path_key_server_explicit_pub() {
        let server = Server::new(
            "srv".to_string(),
            "example.com".to_string(),
            "user".to_string(),
            22,
            AuthMethod::Key,
            Some("/home/user/.ssh/id_ed25519".to_string()),
        );
        let path = derive_copy_id_pub_path(&server, Some("/custom/key.pub")).unwrap();
        assert_eq!(path, "/custom/key.pub");
    }

    #[test]
    fn derive_copy_id_pub_path_key_server_explicit_private() {
        let server = Server::new(
            "srv".to_string(),
            "example.com".to_string(),
            "user".to_string(),
            22,
            AuthMethod::Key,
            Some("/home/user/.ssh/id_ed25519".to_string()),
        );
        let path = derive_copy_id_pub_path(&server, Some("/custom/key")).unwrap();
        assert_eq!(path, "/custom/key.pub");
    }

    #[test]
    fn derive_copy_id_pub_path_password_server_explicit_pub() {
        let server = Server::new(
            "srv".to_string(),
            "example.com".to_string(),
            "user".to_string(),
            22,
            AuthMethod::Password,
            None,
        );
        let path = derive_copy_id_pub_path(&server, Some("/home/user/.ssh/id_rsa.pub")).unwrap();
        assert_eq!(path, "/home/user/.ssh/id_rsa.pub");
    }

    #[test]
    fn derive_copy_id_pub_path_password_server_explicit_private() {
        let server = Server::new(
            "srv".to_string(),
            "example.com".to_string(),
            "user".to_string(),
            22,
            AuthMethod::Password,
            None,
        );
        let path = derive_copy_id_pub_path(&server, Some("/home/user/.ssh/id_rsa")).unwrap();
        assert_eq!(path, "/home/user/.ssh/id_rsa.pub");
    }

    #[test]
    fn derive_copy_id_pub_path_password_server_missing_explicit_errors() {
        let server = Server::new(
            "srv".to_string(),
            "example.com".to_string(),
            "user".to_string(),
            22,
            AuthMethod::Password,
            None,
        );
        let err = derive_copy_id_pub_path(&server, None).unwrap_err();
        assert!(matches!(err, SshManagerError::InvalidConfig(_)));
        let msg = err.to_string();
        assert!(msg.contains("password authentication"));
        assert!(msg.contains("--key") || msg.contains("--pub"));
    }

    #[test]
    fn derive_copy_id_pub_path_empty_explicit_errors() {
        let server = Server::new(
            "srv".to_string(),
            "example.com".to_string(),
            "user".to_string(),
            22,
            AuthMethod::Key,
            Some("/home/user/.ssh/id_ed25519".to_string()),
        );
        let err = derive_copy_id_pub_path(&server, Some("   ")).unwrap_err();
        assert!(matches!(err, SshManagerError::InvalidConfig(_)));
    }

    #[test]
    fn switch_server_to_key_password_server() {
        let mut server = Server::new(
            "srv".to_string(),
            "example.com".to_string(),
            "user".to_string(),
            22,
            AuthMethod::Password,
            None,
        );
        assert_eq!(server.auth_method, AuthMethod::Password);
        assert_eq!(server.key_path, None);

        let priv_path =
            switch_server_to_key(&mut server, "/home/user/.ssh/id_ed25519.pub").unwrap();
        assert_eq!(priv_path, "/home/user/.ssh/id_ed25519");
        assert_eq!(server.auth_method, AuthMethod::Key);
        assert_eq!(
            server.key_path,
            Some("/home/user/.ssh/id_ed25519".to_string())
        );
    }

    #[test]
    fn switch_server_to_key_rejects_non_pub_path() {
        let mut server = Server::new(
            "srv".to_string(),
            "example.com".to_string(),
            "user".to_string(),
            22,
            AuthMethod::Password,
            None,
        );
        let err = switch_server_to_key(&mut server, "/home/user/.ssh/id_ed25519").unwrap_err();
        assert!(matches!(err, SshManagerError::InvalidConfig(_)));
    }

    #[test]
    fn server_parses_from_json_copy_id_backward_compat() {
        let json = r#"{
            "name": "legacy-pw",
            "host": "example.com",
            "username": "bob",
            "port": 2222,
            "auth_method": "password"
        }"#;
        let mut server: Server = serde_json::from_str(json).unwrap();
        assert_eq!(server.auth_method, AuthMethod::Password);
        assert_eq!(server.key_path, None);

        let derived =
            derive_copy_id_pub_path(&server, Some("/home/bob/.ssh/id_ed25519.pub")).unwrap();
        assert_eq!(derived, "/home/bob/.ssh/id_ed25519.pub");

        let priv_key = switch_server_to_key(&mut server, &derived).unwrap();
        assert_eq!(priv_key, "/home/bob/.ssh/id_ed25519");
        assert_eq!(server.auth_method, AuthMethod::Key);
        assert_eq!(
            server.key_path,
            Some("/home/bob/.ssh/id_ed25519".to_string())
        );
    }

    #[test]
    fn split_transfer_paths_single_source() {
        let paths = vec!["local.txt".to_string(), "/remote/dir".to_string()];
        let (sources, dest) = split_transfer_paths(&paths).unwrap();
        assert_eq!(sources, &["local.txt"]);
        assert_eq!(dest, "/remote/dir");
    }

    #[test]
    fn split_transfer_paths_multiple_sources() {
        let paths = vec![
            "a.txt".to_string(),
            "b.txt".to_string(),
            "/remote/dir".to_string(),
        ];
        let (sources, dest) = split_transfer_paths(&paths).unwrap();
        assert_eq!(sources, &["a.txt", "b.txt"]);
        assert_eq!(dest, "/remote/dir");
    }

    #[test]
    fn split_transfer_paths_too_few_args_errors() {
        let empty: Vec<String> = Vec::new();
        assert!(matches!(
            split_transfer_paths(&empty).unwrap_err(),
            SshManagerError::MissingSourceOrDestination
        ));

        let single = vec!["only_one".to_string()];
        assert!(matches!(
            split_transfer_paths(&single).unwrap_err(),
            SshManagerError::MissingSourceOrDestination
        ));
    }

    #[test]
    fn split_transfer_paths_empty_destination_errors() {
        let paths = vec!["file.txt".to_string(), "   ".to_string()];
        assert!(matches!(
            split_transfer_paths(&paths).unwrap_err(),
            SshManagerError::MissingSourceOrDestination
        ));
    }

    #[test]
    fn validate_local_sources_succeeds_for_existing_file_and_dir() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "content").unwrap();

        let sources = vec![
            file_path.to_string_lossy().into_owned(),
            dir.path().to_string_lossy().into_owned(),
        ];
        assert!(validate_local_sources(&sources).is_ok());
    }

    #[test]
    fn validate_local_sources_errors_for_missing_file() {
        let missing = vec!["/nonexistent/path/to/file_ssher_test.txt".to_string()];
        let err = validate_local_sources(&missing).unwrap_err();
        match err {
            SshManagerError::LocalSourceNotFound(p) => {
                assert_eq!(p, "/nonexistent/path/to/file_ssher_test.txt");
            }
            _ => panic!("expected LocalSourceNotFound"),
        }
    }

    #[test]
    fn format_ping_result_formatting() {
        assert_eq!(
            format_ping_result(
                "myserver",
                "example.com",
                22,
                &PingStatus::Ok(Duration::from_millis(12))
            ),
            "myserver  example.com:22  OK 12ms"
        );
        assert_eq!(
            format_ping_result("srv1", "10.0.0.1", 2222, &PingStatus::Refused),
            "srv1  10.0.0.1:2222  refused"
        );
        assert_eq!(
            format_ping_result("srv2", "10.0.0.2", 22, &PingStatus::Timeout),
            "srv2  10.0.0.2:22  timeout"
        );
        assert_eq!(
            format_ping_result("srv3", "bad.host", 22, &PingStatus::DnsError),
            "srv3  bad.host:22  dns error"
        );
    }

    #[test]
    fn format_ping_summary_formatting() {
        assert_eq!(format_ping_summary(3, 1), "up 3 / down 1");
        assert_eq!(format_ping_summary(0, 2), "up 0 / down 2");
        assert_eq!(format_ping_summary(5, 0), "up 5 / down 0");
    }

    #[test]
    fn ping_status_display_and_is_up() {
        let ok = PingStatus::Ok(Duration::from_millis(42));
        assert!(ok.is_up());
        assert_eq!(ok.to_string(), "OK 42ms");

        let refused = PingStatus::Refused;
        assert!(!refused.is_up());
        assert_eq!(refused.to_string(), "refused");

        let timeout = PingStatus::Timeout;
        assert!(!timeout.is_up());
        assert_eq!(timeout.to_string(), "timeout");

        let dns = PingStatus::DnsError;
        assert!(!dns.is_up());
        assert_eq!(dns.to_string(), "dns error");
    }

    #[test]
    fn status_classification_from_error_kind() {
        assert_eq!(
            classify_error_kind(io::ErrorKind::ConnectionRefused),
            PingStatus::Refused
        );
        assert_eq!(
            classify_error_kind(io::ErrorKind::ConnectionReset),
            PingStatus::Refused
        );
        assert_eq!(
            classify_error_kind(io::ErrorKind::TimedOut),
            PingStatus::Timeout
        );
        assert_eq!(
            classify_error_kind(io::ErrorKind::WouldBlock),
            PingStatus::Timeout
        );
        assert_eq!(
            classify_error_kind(io::ErrorKind::NotFound),
            PingStatus::DnsError
        );
        assert_eq!(
            classify_error_kind(io::ErrorKind::Other),
            PingStatus::Timeout
        );
        assert_eq!(
            classify_io_error(io::ErrorKind::ConnectionRefused),
            PingStatus::Refused
        );
        let io_err = io::Error::new(io::ErrorKind::TimedOut, "timed out");
        assert_eq!(status_from_io_error(&io_err), PingStatus::Timeout);
    }

    #[test]
    fn select_ping_targets_by_name() {
        let servers = vec![sample_server("web-prod"), sample_server("db-prod")];
        let targets = select_ping_targets(&servers, Some("web-prod"), &[]).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "web-prod");

        let targets = select_ping_targets(&servers, Some("db"), &[]).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "db-prod");
    }

    #[test]
    fn select_ping_targets_by_tag() {
        let mut s1 = sample_server("s1");
        s1.tags = vec!["prod".to_string(), "web".to_string()];
        let mut s2 = sample_server("s2");
        s2.tags = vec!["prod".to_string(), "db".to_string()];
        let mut s3 = sample_server("s3");
        s3.tags = vec!["staging".to_string()];

        let servers = vec![s1, s2, s3];

        let targets = select_ping_targets(&servers, None, &[String::from("prod")]).unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].name, "s1");
        assert_eq!(targets[1].name, "s2");

        let targets =
            select_ping_targets(&servers, None, &[String::from("prod"), String::from("web")])
                .unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].name, "s1");
    }

    #[test]
    fn select_ping_targets_all_servers_in_list_order() {
        let servers = vec![
            sample_server("srv-c"),
            sample_server("srv-a"),
            sample_server("srv-b"),
        ];
        let targets = select_ping_targets(&servers, None, &[]).unwrap();
        assert_eq!(targets.len(), 3);
        assert_eq!(targets[0].name, "srv-c");
        assert_eq!(targets[1].name, "srv-a");
        assert_eq!(targets[2].name, "srv-b");
    }

    #[test]
    fn select_ping_targets_errors() {
        let servers = vec![sample_server("srv1")];

        let err = select_ping_targets(&servers, Some("srv1"), &[String::from("prod")]).unwrap_err();
        assert!(matches!(err, SshManagerError::InvalidConfig(_)));

        let empty: Vec<Server> = Vec::new();
        let err = select_ping_targets(&empty, None, &[]).unwrap_err();
        assert!(matches!(err, SshManagerError::NoServersMatch));

        let err = select_ping_targets(&servers, None, &[String::from("nonexistent")]).unwrap_err();
        assert!(matches!(err, SshManagerError::NoServersMatch));

        let err = select_ping_targets(&servers, None, &[String::from("bad tag!")]).unwrap_err();
        assert!(matches!(err, SshManagerError::InvalidTag(_)));

        let err = select_ping_targets(&servers, Some("missing"), &[]).unwrap_err();
        assert!(matches!(err, SshManagerError::ServerNotFound(_)));
    }

    #[test]
    fn ping_servers_ordering_preserved_concurrently() {
        let mut s0 = sample_server("srv-0");
        s0.port = 20;
        let mut s1 = sample_server("srv-1");
        s1.port = 21;
        let mut s2 = sample_server("srv-2");
        s2.port = 22;
        let mut s3 = sample_server("srv-3");
        s3.port = 23;
        let mut s4 = sample_server("srv-4");
        s4.port = 24;
        let test_servers = vec![&s0, &s1, &s2, &s3, &s4];

        let mock_runner = |_host: &str, port: u16, _timeout: Duration| -> PingStatus {
            let delay_ms = match port {
                20 => 40,
                21 => 25,
                22 => 15,
                23 => 5,
                _ => 1,
            };
            std::thread::sleep(Duration::from_millis(delay_ms));
            PingStatus::Ok(Duration::from_millis(delay_ms))
        };

        let results =
            ping_servers_with_runner(&test_servers, Duration::from_secs(1), 16, mock_runner);
        assert_eq!(results.len(), 5);
        assert_eq!(results[0].name, "srv-0");
        assert_eq!(results[1].name, "srv-1");
        assert_eq!(results[2].name, "srv-2");
        assert_eq!(results[3].name, "srv-3");
        assert_eq!(results[4].name, "srv-4");
    }

    #[test]
    fn ping_server_tcp_loopback_listener_ok() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let handle = std::thread::spawn(move || {
            let _ = listener.accept();
        });

        let status = ping_server_tcp("127.0.0.1", port, Duration::from_secs(1));
        assert!(matches!(status, PingStatus::Ok(_)));
        let _ = handle.join();
    }

    #[test]
    fn ping_server_tcp_loopback_closed_port_refused() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let status = ping_server_tcp("127.0.0.1", port, Duration::from_millis(500));
        assert_eq!(status, PingStatus::Refused);
    }

    #[test]
    fn ping_server_tcp_dns_error() {
        let status = ping_server_tcp(
            "invalid.invalid.doesnotexist.test",
            22,
            Duration::from_millis(200),
        );
        assert_eq!(status, PingStatus::DnsError);
    }
}
