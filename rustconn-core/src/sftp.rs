//! SFTP URI and command builder
//!
//! Provides utilities for building SFTP URIs and CLI commands
//! for SSH connections with SFTP enabled.

use crate::models::Connection;
use crate::models::SshKeySource;
use std::path::PathBuf;
use std::sync::OnceLock;

/// Information about a running ssh-agent instance.
///
/// Stored globally via [`set_agent_info`] so that any code spawning
/// child processes can inject `SSH_AUTH_SOCK` (and optionally
/// `SSH_AGENT_PID`) via [`apply_agent_env`].
#[derive(Debug, Clone)]
pub struct SshAgentInfo {
    /// Path to the agent socket (value for `SSH_AUTH_SOCK`).
    pub socket_path: String,
    /// Agent process ID (value for `SSH_AGENT_PID`), if known.
    pub pid: Option<String>,
}

/// Global storage for the ssh-agent info discovered or started at
/// application startup. Initialised once from `main()`.
static AGENT_INFO: OnceLock<SshAgentInfo> = OnceLock::new();

/// Stores ssh-agent information globally.
///
/// Call this once from `main()` after [`ensure_ssh_agent`] returns
/// agent info. Subsequent calls are silently ignored (first write wins).
pub fn set_agent_info(info: SshAgentInfo) {
    let _ = AGENT_INFO.set(info);
}

/// Returns the globally stored ssh-agent info, if any.
#[must_use]
pub fn get_agent_info() -> Option<&'static SshAgentInfo> {
    AGENT_INFO.get()
}

/// Applies `SSH_AUTH_SOCK` (and `SSH_AGENT_PID` if available) to a
/// [`std::process::Command`] so the child process can reach the agent.
///
/// Sources (in priority order):
/// 1. Global [`SshAgentInfo`] stored via [`set_agent_info`]
/// 2. Inherited process environment (no-op — already inherited)
///
/// This replaces the former `std::env::set_var("SSH_AUTH_SOCK", …)`
/// pattern, which became `unsafe` in Rust 2024 edition.
pub fn apply_agent_env(cmd: &mut std::process::Command) {
    if let Some(info) = AGENT_INFO.get() {
        cmd.env("SSH_AUTH_SOCK", &info.socket_path);
        if let Some(ref pid) = info.pid {
            cmd.env("SSH_AGENT_PID", pid);
        }
    }
}

/// Result of validating a socket path.
///
/// Used by the GUI to provide real-time feedback on socket path entries.
/// Validation is advisory — only `NotAbsolute` is a hard warning; `NotFound`
/// is informational because the socket may be created later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketPathValidation {
    /// Path is valid and the socket file exists.
    Valid,
    /// Path is absolute but the socket file doesn't exist (non-blocking warning).
    NotFound,
    /// Path is not absolute (does not start with `/`).
    NotAbsolute,
    /// Path is empty (no validation needed).
    Empty,
}

/// Validates a socket path and returns a [`SocketPathValidation`] result.
///
/// - Empty string → `Empty`
/// - Non-absolute (doesn't start with `/`) → `NotAbsolute`
/// - Absolute but file doesn't exist → `NotFound`
/// - Absolute and file exists → `Valid`
#[must_use]
pub fn validate_socket_path(path: &str) -> SocketPathValidation {
    if path.is_empty() {
        return SocketPathValidation::Empty;
    }
    if !path.starts_with('/') {
        return SocketPathValidation::NotAbsolute;
    }
    if std::path::Path::new(path).exists() {
        SocketPathValidation::Valid
    } else {
        SocketPathValidation::NotFound
    }
}

/// Resolves the SSH agent socket path using the priority chain:
/// per-connection → global setting → OnceLock agent info → None (inherit env).
///
/// Returns `Some(path)` if an override should be applied, `None` if the
/// child process should inherit `SSH_AUTH_SOCK` from the parent environment.
///
/// Empty strings are filtered out internally, so callers don't need to
/// distinguish `Some("")` from `None`.
#[must_use]
pub fn resolve_ssh_agent_socket(
    per_connection: Option<&str>,
    global_setting: Option<&str>,
) -> Option<String> {
    // Per-connection override (highest priority)
    if let Some(path) = per_connection
        && !path.is_empty()
    {
        return Some(path.to_string());
    }

    // Global setting (second priority)
    if let Some(path) = global_setting
        && !path.is_empty()
    {
        return Some(path.to_string());
    }

    // OnceLock agent info (third priority)
    if let Some(info) = AGENT_INFO.get()
        && !info.socket_path.is_empty()
    {
        return Some(info.socket_path.clone());
    }

    // None — inherit SSH_AUTH_SOCK from parent environment
    None
}

/// Applies `SSH_AUTH_SOCK` to a [`std::process::Command`] using the priority chain.
///
/// Uses [`resolve_ssh_agent_socket`] to determine the socket path. If an
/// override is found, sets `SSH_AUTH_SOCK` on the command. Otherwise falls
/// back to the existing [`apply_agent_env`] behavior (OnceLock only).
pub fn apply_agent_env_with_overrides(
    cmd: &mut std::process::Command,
    per_connection: Option<&str>,
    global_setting: Option<&str>,
) {
    if let Some(socket_path) = resolve_ssh_agent_socket(per_connection, global_setting) {
        cmd.env("SSH_AUTH_SOCK", &socket_path);
        // Also set SSH_AGENT_PID if available from OnceLock
        if let Some(info) = AGENT_INFO.get()
            && let Some(ref pid) = info.pid
        {
            cmd.env("SSH_AGENT_PID", pid);
        }
    } else {
        // No overrides — fall back to existing behavior
        apply_agent_env(cmd);
    }
}

/// Builds an SFTP URI for the given connection.
///
/// Format: `sftp://[user@]host[:port]`
///
/// Used by GUI (`nautilus`) and CLI (`xdg-open`) to open
/// the host file manager's SFTP browser.
#[must_use]
pub fn build_sftp_uri(username: Option<&str>, host: &str, port: u16) -> String {
    let user_part = username.map_or_else(String::new, |u| format!("{u}@"));
    if port == 22 {
        format!("sftp://{user_part}{host}")
    } else {
        format!("sftp://{user_part}{host}:{port}")
    }
}

/// Builds an SFTP URI from a `Connection`.
///
/// Returns `None` if the connection is not SSH.
#[must_use]
pub fn build_sftp_uri_from_connection(connection: &Connection) -> Option<String> {
    if !matches!(
        connection.protocol_config,
        crate::models::ProtocolConfig::Ssh(_) | crate::models::ProtocolConfig::Sftp(_)
    ) {
        return None;
    }

    Some(build_sftp_uri(
        connection.username.as_deref(),
        &connection.host,
        connection.port,
    ))
}

/// Builds an `sftp` CLI command for the given connection.
///
/// Returns `None` if the connection is not SSH.
///
/// The returned `Vec` has the program name as the first element,
/// followed by arguments: `["sftp", "-P", "port", "user@host"]`.
#[must_use]
pub fn build_sftp_command(connection: &Connection) -> Option<Vec<String>> {
    if !matches!(
        connection.protocol_config,
        crate::models::ProtocolConfig::Ssh(_) | crate::models::ProtocolConfig::Sftp(_)
    ) {
        return None;
    }

    let mut cmd = vec!["sftp".to_string()];

    if connection.port != 22 {
        cmd.push("-P".to_string());
        cmd.push(connection.port.to_string());
    }

    let target = if let Some(ref user) = connection.username {
        format!("{user}@{}", connection.host)
    } else {
        connection.host.clone()
    };
    cmd.push(target);

    Some(cmd)
}

/// Extracts the SSH key file path from a connection's config.
///
/// Checks `key_source` (preferred) then falls back to legacy `key_path`.
/// Returns `None` if no key is configured or the connection is not SSH.
#[must_use]
pub fn get_ssh_key_path(connection: &Connection) -> Option<PathBuf> {
    let ssh = match &connection.protocol_config {
        crate::models::ProtocolConfig::Ssh(cfg) | crate::models::ProtocolConfig::Sftp(cfg) => cfg,
        _ => return None,
    };

    match &ssh.key_source {
        SshKeySource::File { path } if !path.as_os_str().is_empty() => Some(path.clone()),
        SshKeySource::Agent { comment, .. } => {
            // Agent key identified by comment — if comment looks like
            // a file path, return it so we can ssh-add it.
            let p = std::path::Path::new(comment);
            if comment.starts_with('/') || comment.starts_with('~') {
                if comment.starts_with('~') {
                    dirs::home_dir()
                        .map(|home| home.join(comment.strip_prefix("~/").unwrap_or(comment)))
                } else {
                    Some(p.to_path_buf())
                }
            } else {
                None
            }
        }
        _ => {
            // Legacy key_path fallback
            ssh.key_path
                .as_ref()
                .filter(|p| !p.as_os_str().is_empty())
                .cloned()
        }
    }
}

/// Checks whether ssh-agent is reachable.
///
/// Looks for a valid socket in two places:
/// 1. The global [`SshAgentInfo`] (set by [`set_agent_info`])
/// 2. The inherited `SSH_AUTH_SOCK` environment variable
#[must_use]
pub fn is_ssh_agent_available() -> bool {
    if let Some(info) = AGENT_INFO.get()
        && std::path::Path::new(&info.socket_path).exists()
    {
        return true;
    }
    std::env::var("SSH_AUTH_SOCK")
        .ok()
        .filter(|s| !s.is_empty())
        .is_some_and(|sock| std::path::Path::new(&sock).exists())
}

/// Ensures an ssh-agent is running and returns its connection info.
///
/// On some desktop environments (notably KDE on openSUSE Tumbleweed)
/// ssh-agent is not started by default. This function:
///
/// 1. Checks if `SSH_AUTH_SOCK` is already set and the socket exists
/// 2. If not, starts `ssh-agent` and parses its output
/// 3. Returns [`SshAgentInfo`] so the caller can store it globally
///    via [`set_agent_info`]
///
/// The caller is responsible for calling [`set_agent_info`] with the
/// returned value. Child processes then receive the agent socket via
/// [`apply_agent_env`] or inherit it from the process environment.
///
/// # Returns
///
/// `Some(SshAgentInfo)` if an agent is available (either pre-existing
/// or freshly started), `None` if we failed to find or start one.
pub fn ensure_ssh_agent() -> Option<SshAgentInfo> {
    if let Ok(sock) = std::env::var("SSH_AUTH_SOCK")
        && !sock.is_empty()
        && std::path::Path::new(&sock).exists()
    {
        tracing::debug!(%sock, "ssh-agent already available");
        let pid = std::env::var("SSH_AGENT_PID").ok();
        return Some(SshAgentInfo {
            socket_path: sock,
            pid,
        });
    }

    tracing::info!("SSH_AUTH_SOCK not set or socket missing; starting ssh-agent");

    // In Flatpak, ~/.ssh is mounted read-only so ssh-agent cannot create its
    // socket there. Use `-a <path>` to place the socket in a writable directory.
    let explicit_socket = if crate::flatpak::is_flatpak() {
        let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
        let path = format!("{dir}/rustconn-ssh-agent.sock");
        // Remove stale socket from a previous run
        let _ = std::fs::remove_file(&path);
        tracing::debug!(%path, "Using explicit ssh-agent socket path (Flatpak)");
        Some(path)
    } else {
        None
    };

    let mut cmd = std::process::Command::new("ssh-agent");
    if let Some(ref sock_path) = explicit_socket {
        cmd.arg("-a").arg(sock_path);
    }
    cmd.arg("-s");

    let output = match cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(?e, "Failed to run ssh-agent");
            return None;
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(
            %stderr,
            "ssh-agent exited with non-zero status"
        );
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut sock = None;
    let mut pid = None;

    for line in stdout.lines() {
        if let Some(val) = line
            .strip_prefix("SSH_AUTH_SOCK=")
            .and_then(|s| s.split(';').next())
        {
            sock = Some(val.to_string());
        }
        if let Some(val) = line
            .strip_prefix("SSH_AGENT_PID=")
            .and_then(|s| s.split(';').next())
        {
            pid = Some(val.to_string());
        }
    }

    let Some(sock_val) = sock else {
        tracing::warn!(
            %stdout,
            "Could not parse SSH_AUTH_SOCK from ssh-agent output"
        );
        return None;
    };

    tracing::info!(
        %sock_val,
        pid = pid.as_deref().unwrap_or("unknown"),
        "Started ssh-agent"
    );

    Some(SshAgentInfo {
        socket_path: sock_val,
        pid,
    })
}

/// Ensures the connection's SSH key is loaded in ssh-agent.
///
/// Runs `ssh-add <key_path>` if a key file is configured.
/// This is needed before opening SFTP via mc or file managers,
/// because neither can pass an identity file directly.
///
/// Returns `true` if the key was added (or no key is needed),
/// `false` if `ssh-add` failed.
pub fn ensure_key_in_agent(connection: &Connection) -> bool {
    let Some(key_path) = get_ssh_key_path(connection) else {
        // No key configured — ssh-agent may already have the
        // right key, or password auth is used. Proceed anyway.
        return true;
    };

    if !key_path.exists() {
        tracing::warn!(?key_path, "SSH key file not found, skipping ssh-add");
        return true; // Don't block SFTP — agent may have it
    }

    if !is_ssh_agent_available() {
        tracing::warn!(
            "SSH_AUTH_SOCK not set or agent not running; \
             ssh-add will likely fail"
        );
        // Continue anyway — ssh-add may still work if the
        // agent socket is at a non-standard path.
    }

    tracing::info!(?key_path, "Adding SSH key to agent for SFTP");
    let mut cmd = std::process::Command::new("ssh-add");
    cmd.arg(&key_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    apply_agent_env(&mut cmd);
    match cmd.output() {
        Ok(output) if output.status.success() => {
            tracing::info!(?key_path, "SSH key added to agent");
            true
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(
                ?key_path,
                status = ?output.status,
                %stderr,
                "ssh-add failed"
            );
            false
        }
        Err(e) => {
            tracing::error!(?e, "Failed to run ssh-add");
            false
        }
    }
}

/// Returns the XDG Downloads directory path as a string.
///
/// Uses `$XDG_DOWNLOAD_DIR` (via `dirs::download_dir()`) with
/// fallback to `~/Downloads`. Creates the directory if it does
/// not exist.
#[must_use]
pub fn get_downloads_dir() -> String {
    let path = dirs::download_dir().unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("Downloads")
    });
    if !path.exists() {
        let _ = std::fs::create_dir_all(&path);
    }
    path.to_string_lossy().into_owned()
}

/// Builds a Midnight Commander command to open an SFTP panel.
///
/// Returns `None` if the connection is not SSH.
///
/// Uses mc's FISH VFS: `["mc", "<downloads>", "sh://user@host:port"]`.
/// Left panel shows XDG Downloads directory, right panel shows
/// remote via FISH. Requires the SSH key to be loaded in
/// ssh-agent beforehand.
#[must_use]
pub fn build_mc_sftp_command(connection: &Connection) -> Option<Vec<String>> {
    if !matches!(
        connection.protocol_config,
        crate::models::ProtocolConfig::Ssh(_) | crate::models::ProtocolConfig::Sftp(_)
    ) {
        return None;
    }

    // Append /~ so mc opens the remote user's home directory
    // instead of the filesystem root.
    let target = if let Some(ref user) = connection.username {
        if connection.port == 22 {
            format!("sh://{user}@{}/~", connection.host)
        } else {
            format!("sh://{user}@{}:{}/~", connection.host, connection.port)
        }
    } else if connection.port == 22 {
        format!("sh://{}/~", connection.host)
    } else {
        format!("sh://{}:{}/~", connection.host, connection.port)
    };

    let local_dir = get_downloads_dir();

    // In Flatpak, /app/bin/mc is a shell wrapper script that sources
    // mc-wrapper.sh for directory-change-on-exit. Use mc.bin (the real
    // binary) directly to avoid the extra shell layer — the wrapper's
    // directory-change-on-exit feature is irrelevant since mc runs in
    // a dedicated VTE tab, not the user's interactive shell.
    let mc_binary = if crate::flatpak::is_flatpak() {
        "/app/bin/mc.bin".to_string()
    } else {
        "mc".to_string()
    };

    // Return mc arguments directly (no sh -c wrapper) so VTE spawns
    // mc as the direct child of the PTY. This ensures mc receives
    // mouse events from VTE without a shell intercepting them.
    //
    // `-g` (--oldmouse) forces "normal tracking" mouse mode (X10/1000)
    // instead of SGR mode (1006). VTE 0.80 negotiates SGR mouse
    // encoding which mc's ncurses may not parse correctly, causing
    // raw escape sequences to leak as text artifacts. Normal tracking
    // mode is simpler and universally supported.
    Some(vec![mc_binary, "-g".to_string(), local_dir, target])
}

/// Wraps a string in single quotes for safe use in `sh -c` commands.
///
/// Single quotes inside the value are escaped as `'\''` (end quote,
/// escaped literal quote, start quote).
#[cfg(test)]
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_sftp_uri_with_user_default_port() {
        let uri = build_sftp_uri(Some("admin"), "server.example.com", 22);
        assert_eq!(uri, "sftp://admin@server.example.com");
    }

    #[test]
    fn test_build_sftp_uri_without_user() {
        let uri = build_sftp_uri(None, "10.0.0.1", 22);
        assert_eq!(uri, "sftp://10.0.0.1");
    }

    #[test]
    fn test_build_sftp_uri_custom_port() {
        let uri = build_sftp_uri(Some("root"), "host.local", 2222);
        assert_eq!(uri, "sftp://root@host.local:2222");
    }

    #[test]
    fn test_build_sftp_command_default_port() {
        let mut conn =
            Connection::new_ssh("Test".to_string(), "server.example.com".to_string(), 22);
        conn.username = Some("admin".to_string());

        let cmd = build_sftp_command(&conn).unwrap();
        assert_eq!(cmd, vec!["sftp", "admin@server.example.com"]);
    }

    #[test]
    fn test_build_sftp_command_custom_port() {
        let mut conn = Connection::new_ssh("Test".to_string(), "host.local".to_string(), 2222);
        conn.username = Some("root".to_string());

        let cmd = build_sftp_command(&conn).unwrap();
        assert_eq!(cmd, vec!["sftp", "-P", "2222", "root@host.local"]);
    }

    #[test]
    fn test_build_sftp_command_non_ssh() {
        let conn = Connection::new_rdp("Test".to_string(), "server.example.com".to_string(), 3389);
        assert!(build_sftp_command(&conn).is_none());
    }

    #[test]
    fn test_build_sftp_uri_from_ssh_connection() {
        let mut conn =
            Connection::new_ssh("Test".to_string(), "server.example.com".to_string(), 22);
        conn.username = Some("admin".to_string());

        let uri = build_sftp_uri_from_connection(&conn).unwrap();
        assert_eq!(uri, "sftp://admin@server.example.com");
    }

    #[test]
    fn test_build_sftp_uri_from_non_ssh() {
        let conn = Connection::new_rdp("Test".to_string(), "server.example.com".to_string(), 3389);
        assert!(build_sftp_uri_from_connection(&conn).is_none());
    }

    #[test]
    fn test_build_mc_sftp_command_default_port() {
        let mut conn =
            Connection::new_ssh("Test".to_string(), "server.example.com".to_string(), 22);
        conn.username = Some("admin".to_string());

        let cmd = build_mc_sftp_command(&conn).unwrap();
        // Direct argv: ["mc", "-g", <downloads>, "sh://user@host/~"]
        assert_eq!(cmd.len(), 4);
        assert_eq!(cmd[0], "mc");
        assert_eq!(cmd[1], "-g");
        assert_eq!(cmd[3], "sh://admin@server.example.com/~");
    }

    #[test]
    fn test_build_mc_sftp_command_custom_port() {
        let mut conn = Connection::new_ssh("Test".to_string(), "host.local".to_string(), 2222);
        conn.username = Some("root".to_string());

        let cmd = build_mc_sftp_command(&conn).unwrap();
        assert_eq!(cmd.len(), 4);
        assert_eq!(cmd[0], "mc");
        assert_eq!(cmd[1], "-g");
        assert_eq!(cmd[3], "sh://root@host.local:2222/~");
    }

    #[test]
    fn test_build_mc_sftp_command_non_ssh() {
        let conn = Connection::new_rdp("Test".to_string(), "server.example.com".to_string(), 3389);
        assert!(build_mc_sftp_command(&conn).is_none());
    }

    #[test]
    fn test_shell_escape_simple() {
        assert_eq!(shell_escape("hello"), "'hello'");
    }

    #[test]
    fn test_shell_escape_with_single_quote() {
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }

    // --- Task 2.4: validate_socket_path tests ---

    #[test]
    fn test_validate_socket_path_empty() {
        assert_eq!(validate_socket_path(""), SocketPathValidation::Empty);
    }

    #[test]
    fn test_validate_socket_path_not_absolute() {
        assert_eq!(
            validate_socket_path("relative/path"),
            SocketPathValidation::NotAbsolute
        );
    }

    #[test]
    fn test_validate_socket_path_not_found() {
        assert_eq!(
            validate_socket_path("/nonexistent/socket.sock"),
            SocketPathValidation::NotFound
        );
    }

    #[test]
    fn test_validate_socket_path_valid() {
        let dir = tempfile::tempdir().unwrap();
        let sock_path = dir.path().join("test.sock");
        std::fs::write(&sock_path, b"").unwrap();
        let path_str = sock_path.to_str().unwrap();
        assert_eq!(validate_socket_path(path_str), SocketPathValidation::Valid);
    }

    // --- Task 2.5: resolve_ssh_agent_socket tests ---

    #[test]
    fn test_resolve_per_connection_wins() {
        let result = resolve_ssh_agent_socket(Some("/per/conn.sock"), Some("/global.sock"));
        assert_eq!(result, Some("/per/conn.sock".to_string()));
    }

    #[test]
    fn test_resolve_global_when_no_per_connection() {
        let result = resolve_ssh_agent_socket(None, Some("/global.sock"));
        assert_eq!(result, Some("/global.sock".to_string()));
    }

    #[test]
    fn test_resolve_empty_per_connection_falls_through() {
        let result = resolve_ssh_agent_socket(Some(""), Some("/global.sock"));
        assert_eq!(result, Some("/global.sock".to_string()));
    }

    #[test]
    fn test_resolve_empty_strings_treated_as_none() {
        // Both empty — should fall through to OnceLock or None
        let result = resolve_ssh_agent_socket(Some(""), Some(""));
        // OnceLock may or may not be set in test env; just verify
        // empty strings don't produce a result themselves
        if let Some(ref path) = result {
            // If we got something, it must be from OnceLock (non-empty)
            assert!(!path.is_empty());
        }
    }

    #[test]
    fn test_resolve_all_none_without_oncelock() {
        // When OnceLock is not set and no overrides, result depends on
        // whether AGENT_INFO was set by another test. We verify the
        // function doesn't panic and returns a consistent result.
        let result = resolve_ssh_agent_socket(None, None);
        // Result is either None (no OnceLock) or Some(path) from OnceLock
        if let Some(ref path) = result {
            assert!(!path.is_empty());
        }
    }

    #[test]
    fn test_resolve_per_connection_overrides_everything() {
        // Per-connection should win regardless of global
        let result =
            resolve_ssh_agent_socket(Some("/custom/agent.sock"), Some("/global/agent.sock"));
        assert_eq!(result, Some("/custom/agent.sock".to_string()));
    }

    #[test]
    fn test_resolve_empty_global_falls_through() {
        let result = resolve_ssh_agent_socket(None, Some(""));
        // Empty global should be treated as None, falling through to OnceLock
        if let Some(ref path) = result {
            assert!(!path.is_empty());
        }
    }
}
