use crate::error::{Result, SshManagerError};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

pub struct Terminal;

impl Terminal {
    /// Get the terminal command to use, respecting user preference.
    pub fn get_terminal_command(preferred: Option<&str>) -> Result<String> {
        if let Some(pref) = preferred {
            if Self::terminal_exists(pref) {
                return Ok(pref.to_string());
            }
        }

        for terminal in ["gnome-terminal", "konsole", "xfce4-terminal", "xterm"] {
            if Self::terminal_exists(terminal) {
                return Ok(terminal.to_string());
            }
        }

        Err(SshManagerError::TerminalNotFound)
    }

    /// Check if a terminal emulator exists in PATH. Exposed as `pub(crate)`
    /// for tests that need to seed `PATH` with a fake binary.
    pub(crate) fn terminal_exists(terminal: &str) -> bool {
        find_in_path(terminal).is_some()
    }

    /// Spawn a terminal running the given argv. The first element of `argv`
    /// is the program name as the user would type it (e.g. `"ssh"`). For
    /// password-auth flows the caller arranges for `SSH_ASKPASS` /
    /// `SSH_ASKPASS_REQUIRE=force` to be set on the command's environment.
    pub fn spawn_terminal(terminal: &str, argv: &[&str], env: &[(&str, &str)]) -> Result<()> {
        if argv.is_empty() {
            return Err(SshManagerError::SshLaunchFailed(
                "spawn_terminal called with empty argv".to_string(),
            ));
        }

        let mut cmd = match terminal {
            "gnome-terminal" => {
                let mut c = Command::new("gnome-terminal");
                c.arg("--").arg("bash").arg("-c");
                c
            }
            "konsole" => {
                let mut c = Command::new("konsole");
                c.arg("-e").arg("bash").arg("-c");
                c
            }
            "xfce4-terminal" => {
                let mut c = Command::new("xfce4-terminal");
                c.arg("--command").arg("bash").arg("-c");
                c
            }
            "xterm" => {
                let mut c = Command::new("xterm");
                c.arg("-e").arg("bash").arg("-c");
                c
            }
            other => {
                // Generic fallback: `-e <program> <args...>`.
                let mut c = Command::new(other);
                c.arg("-e");
                c
            }
        };

        for (k, v) in env {
            cmd.env(k, v);
        }

        // Build a shell command that runs the argv (with safe quoting) and
        // then waits for Enter so the window doesn't disappear on disconnect.
        let shell_cmd = build_shell_command(argv);
        cmd.arg(&shell_cmd);

        // Detach: become a new session leader so we don't get killed when
        // the parent terminal exits.
        // SAFETY: setsid is async-signal-safe and has no preconditions here.
        unsafe {
            cmd.pre_exec(|| {
                #[cfg(unix)]
                {
                    libc::setsid();
                }
                Ok(())
            });
        }

        cmd.spawn().map_err(|e| {
            SshManagerError::SshLaunchFailed(format!("Failed to launch {}: {}", terminal, e))
        })?;

        Ok(())
    }
}

/// Search PATH for an executable named `name`. Returns the first match.
fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            // Cheap executable check: must have at least one executable bit.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if candidate
                    .metadata()
                    .map(|m| m.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false)
                {
                    return Some(candidate);
                }
            }
            #[cfg(not(unix))]
            {
                return Some(candidate);
            }
        }
    }
    None
}

/// Build a single shell-quoted command string from argv. We use POSIX shell
/// single-quote escaping, which is unambiguous.
fn build_shell_command(argv: &[&str]) -> String {
    let mut out = String::new();
    for (i, a) in argv.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push('\'');
        for c in a.chars() {
            if c == '\'' {
                out.push_str("'\\''");
            } else {
                out.push(c);
            }
        }
        out.push('\'');
    }
    out.push_str(
        "; rc=$?; echo; echo \"[ssh exited with status $rc. Press Enter to close.]\"; read _",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    /// Build a tiny executable in a fresh tempdir and prepend that dir to
    /// PATH so `find_in_path` can resolve it. Returns a guard that restores
    /// the original PATH on drop. Tests using this are serialized.
    struct PathOverride {
        original: Option<String>,
        _dir: tempfile::TempDir,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    fn env_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    impl PathOverride {
        fn with_fake_binary(name: &str) -> Self {
            let lock = env_lock().lock().unwrap_or_else(|e| e.into_inner());
            let dir = tempfile::tempdir().expect("tempdir");
            let path = dir.path().join(name);
            fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            let original = std::env::var("PATH").ok();
            // Build the new PATH as a String so the ':' separator isn't
            // mangled by PathBuf's path-separator handling on Unix.
            let new_path = match &original {
                Some(p) => format!("{}:{}", dir.path().display(), p),
                None => dir.path().to_string_lossy().into_owned(),
            };
            std::env::set_var("PATH", &new_path);
            Self {
                original,
                _dir: dir,
                _lock: lock,
            }
        }
    }

    impl Drop for PathOverride {
        fn drop(&mut self) {
            match &self.original {
                Some(p) => std::env::set_var("PATH", p),
                None => std::env::remove_var("PATH"),
            }
        }
    }

    #[test]
    fn terminal_exists_finds_known_binary() {
        let _g = PathOverride::with_fake_binary("myterm");
        assert!(Terminal::terminal_exists("myterm"));
    }

    #[test]
    fn terminal_exists_returns_false_for_missing() {
        let _g = PathOverride::with_fake_binary("myterm");
        assert!(!Terminal::terminal_exists("not-a-real-terminal-xyz"));
    }

    #[test]
    fn build_shell_command_quotes_safely() {
        let s = build_shell_command(&["ssh", "-p", "2222", "alice@example.com"]);
        assert!(s.contains("'ssh'"));
        assert!(s.contains("'-p'"));
        assert!(s.contains("'2222'"));
        assert!(s.contains("'alice@example.com'"));
        assert!(s.contains("Press Enter to close"));
    }

    #[test]
    fn build_shell_command_handles_single_quote_in_arg() {
        // Pathological: an arg containing a single quote. POSIX-safe escape.
        let s = build_shell_command(&["echo", "it's"]);
        assert!(s.contains("'\\''"));
    }

    #[test]
    fn spawn_terminal_rejects_empty_argv() {
        let res = Terminal::spawn_terminal("xterm", &[], &[]);
        assert!(matches!(res, Err(SshManagerError::SshLaunchFailed(_))));
    }
}
