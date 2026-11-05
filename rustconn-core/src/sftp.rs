//! SFTP URI and command builder
//!
//! Provides utilities for building SFTP URIs and CLI commands
//! for SSH connections with SFTP enabled.

use std::path::PathBuf;
use std::sync::OnceLock;

use crate::models::{Connection, ConnectionGroup, SshKeySource};

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
/// Format: `sftp://[user@]host[:port][/path]`
///
/// Used by GUI (`nautilus`) and CLI (`xdg-open`) to open the host file
/// manager's SFTP browser. When `path` is `Some`, it is normalised to an
/// absolute path and appended so the file manager opens there instead of the
/// server root (issue #212).
#[must_use]
pub fn build_sftp_uri(username: Option<&str>, host: &str, port: u16, path: Option<&str>) -> String {
    let user_part = username.map_or_else(String::new, |u| format!("{u}@"));
    let base = if port == 22 {
        format!("sftp://{user_part}{host}")
    } else {
        format!("sftp://{user_part}{host}:{port}")
    };
    match normalize_remote_path(path) {
        Some(p) => format!("{base}{p}"),
        None => base,
    }
}

/// Normalises an optional remote path into an absolute URI suffix.
///
/// Returns `None` for an empty/whitespace path. Otherwise trims trailing
/// slashes and guarantees a single leading slash, so both `home/user` and
/// `/home/user/` become `/home/user`. A literal `/` stays `/` (server root).
///
// ponytail: no percent-encoding — home paths from `pwd` are plain POSIX paths,
// consistent with the already-unencoded host/user in the URI. Add encoding if
// remote paths containing spaces or `#` ever appear in the wild.
fn normalize_remote_path(path: Option<&str>) -> Option<String> {
    let trimmed = path?.trim();
    if trimmed.is_empty() {
        return None;
    }
    let no_trailing = trimmed.trim_end_matches('/');
    Some(if no_trailing.starts_with('/') {
        no_trailing.to_string()
    } else {
        format!("/{no_trailing}")
    })
}

/// Returns the connection's configured SFTP remote path, if any (SSH/SFTP only).
fn connection_remote_path(connection: &Connection) -> Option<&str> {
    match &connection.protocol_config {
        crate::models::ProtocolConfig::Ssh(cfg) | crate::models::ProtocolConfig::Sftp(cfg) => {
            cfg.remote_path.as_deref()
        }
        _ => None,
    }
}

/// Builds an SFTP URI from a `Connection`, honouring its configured remote path.
///
/// Returns `None` if the connection is not SSH. Does no network I/O: when no
/// remote path is configured the URI points at the server root. For the
/// home-directory auto-resolution used by the file browser, see
/// [`build_sftp_browser_uri`].
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
        connection_remote_path(connection),
    ))
}

/// Session cache of remote-home probe outcomes, keyed by `user@host:port`.
///
/// `Some(path)` is a resolved home directory; `None` records a probe that
/// failed this session (e.g. a password-only host that `BatchMode` cannot
/// authenticate), so a second SFTP open does not repeat the round-trip.
static HOME_CACHE: OnceLock<std::sync::Mutex<std::collections::HashMap<String, Option<String>>>> =
    OnceLock::new();

fn home_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, Option<String>>> {
    HOME_CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Best-effort resolution of the remote login home directory via `ssh … pwd`.
///
/// The GVFS sftp backend mounts at the server root `/`, so a file manager
/// opening a bare `sftp://host` URI lands in `/` — inaccessible on shared
/// hosting (issue #212). Resolving the home directory lets the caller build a
/// URI that opens where the user actually has access, matching `ssh` (a login
/// shell starts in `$HOME`).
///
/// Reuses the connection's jump-host chain, port and identity file. Runs a
/// non-interactive `pwd` with `BatchMode=yes` (so it never prompts and cannot
/// hang waiting for a password), a short `ConnectTimeout`, and keepalives to
/// bound a stall after connect. The outcome — success *or* failure — is
/// memoised per host, so a second SFTP open in the same session skips the
/// round-trip even when the probe cannot authenticate (password-only hosts).
/// A per-connection/group ssh-agent socket override is honoured.
///
/// This performs blocking network I/O — call it off the GTK main thread.
///
/// Returns `None` if the connection is not SSH, `ssh` is unavailable,
/// non-interactive auth is not possible, or the output is not an absolute path.
#[must_use]
pub fn resolve_remote_home(
    connection: &Connection,
    connections: &[Connection],
    groups: &[ConnectionGroup],
    network: &crate::config::NetworkSettings,
) -> Option<String> {
    if !matches!(
        connection.protocol_config,
        crate::models::ProtocolConfig::Ssh(_) | crate::models::ProtocolConfig::Sftp(_)
    ) {
        return None;
    }

    let user = connection.username.as_deref().unwrap_or_default();
    let cache_key = format!("{user}@{}:{}", connection.host, connection.port);
    if let Ok(cache) = home_cache().lock()
        && let Some(outcome) = cache.get(&cache_key)
    {
        // A cached `None` means a prior probe failed this session — return it
        // without retrying (avoids repeating the round-trip on every open).
        return outcome.clone();
    }

    let mut cmd = std::process::Command::new("ssh");
    // BatchMode=yes → never prompts, so it cannot hang on auth; ConnectTimeout
    // bounds the TCP connect. ServerAliveInterval/CountMax bound a channel that
    // stalls *after* connect (3s × 2 missed keepalives ≈ 6s) so a dead link
    // mid-`pwd` cannot pin the blocking thread. `pwd` runs instantly otherwise.
    // StrictHostKeyChecking=yes → a background probe never records a host key
    // the user has not accepted, even when their ssh config says `accept-new`
    // or `no`; an unknown key fails the probe like any other failure. The
    // `-J` hops do not inherit `-o` and keep the user's own setting; this
    // probe hands them no askpass, so a key they accept is one the user
    // configured or answered for.
    cmd.arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("StrictHostKeyChecking=yes")
        .arg("-o")
        .arg("ConnectTimeout=8")
        .arg("-o")
        .arg("ServerAliveInterval=3")
        .arg("-o")
        .arg("ServerAliveCountMax=2");

    // Covers both bastion forms: the free-text `proxy_jump` and a jump host
    // picked from the connection list (`jump_host_id`). Only the former used to
    // be honoured here, so the probe could not reach a host behind a
    // picker-selected bastion and the browser fell back to the server root.
    if let Some(proxy_jump) =
        crate::connection::resolve_proxy_jump_value(connection, connections, groups, network)
    {
        cmd.arg("-J").arg(proxy_jump);
    }
    if connection.port != 22 {
        cmd.arg("-p").arg(connection.port.to_string());
    }
    if let Some(key_path) =
        crate::connection::ssh_inheritance::resolve_ssh_key_path(connection, groups)
    {
        cmd.arg("-i").arg(key_path);
    }

    let target = if user.is_empty() {
        connection.host.clone()
    } else {
        format!("{user}@{}", connection.host)
    };
    cmd.arg(&target)
        .arg("pwd")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        // Capture stderr so a failing probe can be diagnosed (issue #212): on
        // Ubuntu 26.04 the probe silently failed and there was no way to see
        // why. Logged at debug level below; run with `RUST_LOG=debug`.
        .stderr(std::process::Stdio::piped())
        // Strip host SSH_ASKPASS so BatchMode is not defeated by a GUI prompt.
        .env_remove("SSH_ASKPASS");

    tracing::debug!(
        %target,
        port = connection.port,
        "Probing remote home directory via `ssh … pwd` (issue #212)"
    );
    // Honour a per-connection/group ssh-agent socket override so the probe
    // authenticates with the same agent the real connection uses; otherwise
    // BatchMode would fail and reintroduce the server-root landing (#212).
    apply_agent_env_with_overrides(
        &mut cmd,
        crate::connection::ssh_inheritance::resolve_ssh_agent_socket(connection, groups).as_deref(),
        None,
    );

    // Take the last absolute-looking line, ignoring any stray banner output.
    // Each failure mode is logged distinctly so `RUST_LOG=debug` reveals why a
    // probe fell back to the server root (issue #212): spawn error, non-zero
    // exit (usually BatchMode auth failure — stderr carries the reason), or
    // output with no absolute path.
    let resolved = match cmd.output() {
        Err(e) => {
            tracing::warn!(%target, error = %e, "`ssh … pwd` probe could not run");
            None
        }
        Ok(o) if !o.status.success() => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            tracing::debug!(
                %target,
                status = ?o.status.code(),
                stderr = %stderr.trim(),
                "`ssh … pwd` probe failed (likely BatchMode auth); \
                 falling back to server root"
            );
            None
        }
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let path = stdout
                .lines()
                .rev()
                .map(str::trim)
                .find(|line| line.starts_with('/'))
                .map(str::to_string);
            if let Some(home) = &path {
                tracing::debug!(%target, %home, "Resolved remote home directory");
            } else {
                tracing::debug!(
                    %target,
                    stdout = %stdout.trim(),
                    "`ssh … pwd` probe returned no absolute path; \
                     falling back to server root"
                );
            }
            path
        }
    };

    // Cache the outcome, including failure (`None`), for the session.
    if let Ok(mut cache) = home_cache().lock() {
        cache.insert(cache_key, resolved.clone());
    }
    resolved
}

/// Builds the SFTP file-browser URI, resolving the home directory when needed.
///
/// Hybrid of the configured remote path and auto-detection (issue #212):
/// 1. an explicit `remote_path` on the connection wins (covers chroot and
///    non-standard layouts, and skips the network round-trip);
/// 2. otherwise the login home directory is resolved via [`resolve_remote_home`];
/// 3. if resolution fails, falls back to the bare URI (server root).
///
/// Returns `None` if the connection is not SSH. Performs blocking network I/O
/// in case 2 — call it off the GTK main thread.
#[must_use]
pub fn build_sftp_browser_uri(
    connection: &Connection,
    connections: &[Connection],
    groups: &[ConnectionGroup],
    network: &crate::config::NetworkSettings,
) -> Option<String> {
    if !matches!(
        connection.protocol_config,
        crate::models::ProtocolConfig::Ssh(_) | crate::models::ProtocolConfig::Sftp(_)
    ) {
        return None;
    }

    let path = if let Some(explicit) = connection_remote_path(connection)
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        Some(explicit.to_string())
    } else {
        resolve_remote_home(connection, connections, groups, network)
    };

    Some(build_sftp_uri(
        connection.username.as_deref(),
        &connection.host,
        connection.port,
        path.as_deref(),
    ))
}

/// Builds an `sftp` CLI command for the given connection.
///
/// Returns `None` if the connection is not SSH.
///
/// Resolves the full jump-host chain — a free-text `proxy_jump`, one inherited
/// from a group, and a jump host referenced by `jump_host_id` — into `-J`.
///
/// The returned `Vec` has the program name as the first element,
/// followed by arguments: `["sftp", "-P", "port", "user@host"]`.
#[must_use]
pub fn build_sftp_command(
    connection: &Connection,
    connections: &[Connection],
    groups: &[ConnectionGroup],
    network: &crate::config::NetworkSettings,
) -> Option<Vec<String>> {
    if !matches!(
        connection.protocol_config,
        crate::models::ProtocolConfig::Ssh(_) | crate::models::ProtocolConfig::Sftp(_)
    ) {
        return None;
    }

    let mut cmd = vec!["sftp".to_string()];

    // Full bastion chain: the free-text `proxy_jump`, a group-inherited one, and
    // a jump host picked from the connection list.
    if let Some(proxy_jump) =
        crate::connection::resolve_proxy_jump_value(connection, connections, groups, network)
    {
        cmd.push("-J".to_string());
        cmd.push(proxy_jump);
    }

    if connection.port != 22 {
        cmd.push("-P".to_string());
        cmd.push(connection.port.to_string());
    }

    // Add identity file from inheritance chain if available
    if let Some(key_path) =
        crate::connection::ssh_inheritance::resolve_ssh_key_path(connection, groups)
    {
        cmd.push("-i".to_string());
        cmd.push(key_path.to_string_lossy().into_owned());
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
/// Uses SSH inheritance resolution: checks the connection-level setting
/// first, then walks the group hierarchy via [`crate::connection::ssh_inheritance::resolve_ssh_key_path`].
///
/// # Arguments
/// * `connection` — the connection to resolve the key for
/// * `groups` — the full group hierarchy for inheritance resolution
///
/// Returns `None` if no key is configured or the connection is not SSH.
#[must_use]
pub fn get_ssh_key_path(connection: &Connection, groups: &[ConnectionGroup]) -> Option<PathBuf> {
    // Delegate to the inheritance resolver which handles:
    // 1. Connection-level File { path } → return path
    // 2. Agent with file-like comment → handled below as fallback
    // 3. Inherit / no config → walk group chain
    //
    // First try the inheritance resolver for File and Inherit cases
    if let Some(path) = crate::connection::ssh_inheritance::resolve_ssh_key_path(connection, groups)
    {
        return Some(path);
    }

    // Handle Agent key_source with file-like comment (not covered by inheritance resolver)
    let ssh = match &connection.protocol_config {
        crate::models::ProtocolConfig::Ssh(cfg) | crate::models::ProtocolConfig::Sftp(cfg) => cfg,
        _ => return None,
    };

    if let SshKeySource::Agent { comment, .. } = &ssh.key_source {
        let p = std::path::Path::new(comment);
        if comment.starts_with('/') || comment.starts_with('~') {
            if comment.starts_with('~') {
                return dirs::home_dir()
                    .map(|home| home.join(comment.strip_prefix("~/").unwrap_or(comment)));
            }
            return Some(p.to_path_buf());
        }
    }

    // Legacy key_path fallback (only when inheritance resolver returned None
    // and key_source is not Agent with file path)
    ssh.key_path
        .as_ref()
        .filter(|p| !p.as_os_str().is_empty())
        .cloned()
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

    // Bounded: this runs at startup, before the main window, so an
    // `ssh-agent` that never exits must not hold the whole program.
    let output = match crate::ssh_agent::run_agent_command(
        cmd,
        crate::ssh_agent::AGENT_COMMAND_TIMEOUT,
        "ssh-agent -s",
    ) {
        Ok(Some(o)) => o,
        Ok(None) => {
            tracing::warn!(
                timeout_s = crate::ssh_agent::AGENT_COMMAND_TIMEOUT.as_secs(),
                "ssh-agent did not finish in time; killed it"
            );
            return None;
        }
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

/// The agent socket a child spawned through [`apply_agent_env`] talks to: the
/// agent RustConn found or started at startup (`agent_info`), else the
/// inherited `SSH_AUTH_SOCK` (`env_socket`). Empty values count as unset.
fn agent_socket_from(agent_info: Option<&str>, env_socket: Option<String>) -> Option<String> {
    agent_info
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| env_socket.filter(|s| !s.is_empty()))
}

/// The agent socket SFTP, mc and an SSH session without a per-connection
/// agent socket use: the agent RustConn found or started at startup, else the
/// inherited `SSH_AUTH_SOCK`. `None` when neither is set.
#[must_use]
pub fn default_agent_socket() -> Option<String> {
    agent_socket_from(
        AGENT_INFO.get().map(|info| info.socket_path.as_str()),
        std::env::var("SSH_AUTH_SOCK").ok(),
    )
}

/// Loads one SSH private key into the ssh-agent that SFTP and mc will use, the
/// add path the GUI's launchers and the CLI share (A3).
///
/// The agent is the one a child spawned through [`apply_agent_env`] reaches:
/// the agent RustConn found or started at startup, else the inherited
/// `SSH_AUTH_SOCK`. That socket is resolved once and used for the whole add,
/// through [`crate::ssh_agent::SshAgentManager::add_own_key`]: asking the agent
/// which keys it holds, the `ssh-add -t 43200`
/// ([`crate::ssh_agent::OWN_KEY_LIFETIME`]) and recording the key when its own
/// file carries its public blob, which the GUI removes again on exit, screen
/// lock and suspend ([`crate::ssh_agent::evict_own_keys`]). The CLI removes it
/// once a `--cli` or `--mc` session's child exits; for the detached file
/// manager it launches, the lifetime alone bounds the key. A key the agent already
/// holds is left as it is, because it may be the user's, and when that cannot
/// be told the key is not added. The `ssh-add` has no stdin, no host
/// `SSH_ASKPASS` (it may name a helper absent in the Flatpak sandbox) and
/// `SSH_ASKPASS_REQUIRE=never`, so no passphrase prompt opens, and it is
/// bounded to 30 s.
///
/// The caller decides what a failure means, so this returns the reason rather
/// than a bool: a path that shows the user a message needs it, and a path
/// that only logs can ignore it. The text is untranslated English; a GUI
/// caller translates its own message.
///
/// # Errors
///
/// Returns [`IdentityLoadError::Unverified`] when the agent's answer cannot
/// settle whether it already holds this key (the key was left out, but the
/// agent may hold it). Returns [`IdentityLoadError::Failed`] with the trimmed
/// `ssh-add` stderr when the agent rejects the key, or a message when no
/// agent socket is known, the agent cannot be asked which keys it holds, or
/// `ssh-add` could not be run or did not finish in time.
pub fn ensure_identity_loaded(key_path: &std::path::Path) -> Result<(), IdentityLoadError> {
    let Some(socket) = default_agent_socket() else {
        tracing::warn!(
            ?key_path,
            "no SSH agent socket is known; not adding the key"
        );
        return Err(IdentityLoadError::Failed(
            "no SSH agent is running (SSH_AUTH_SOCK is not set)".to_owned(),
        ));
    };
    crate::ssh_agent::SshAgentManager::new(Some(socket))
        .add_own_key(key_path)
        .map_err(|e| {
            tracing::warn!(?key_path, error = %e, "could not add the SSH key to the agent");
            IdentityLoadError::from(e)
        })
}

/// Why [`ensure_identity_loaded`] did not put the key in the agent. The text
/// is untranslated English.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdentityLoadError {
    /// RustConn could not tell whether the agent already holds the key, so it
    /// left the agent alone. The agent may hold it anyway: a launch that
    /// needs the key can go ahead with a warning.
    #[error("{0}")]
    Unverified(String),
    /// The key is not in the agent through RustConn: no agent, the agent
    /// could not be asked, or `ssh-add` failed or did not finish in time.
    #[error("{0}")]
    Failed(String),
}

impl From<crate::ssh_agent::AgentError> for IdentityLoadError {
    fn from(error: crate::ssh_agent::AgentError) -> Self {
        use crate::ssh_agent::AgentError;
        match error {
            AgentError::KeyPresenceUnknown(message) => Self::Unverified(message),
            AgentError::AddKeyFailed(message) => Self::Failed(message),
            other => Self::Failed(other.to_string()),
        }
    }
}

/// Ensures the connection's SSH key is loaded in ssh-agent.
///
/// Resolves the key path through SSH inheritance and, when one is configured
/// and present, loads it via [`ensure_identity_loaded`]. This is the SFTP/mc
/// entry point (and the CLI's): neither mc nor a file manager can pass an
/// identity file directly, so the key must be in the agent first.
///
/// Returns `true` if the key was added or none is needed, `false` if `ssh-add`
/// failed. The bool form is kept for the existing callers; a caller that needs
/// the failure text calls [`ensure_identity_loaded`] directly.
pub fn ensure_key_in_agent(connection: &Connection, groups: &[ConnectionGroup]) -> bool {
    let Some(key_path) = get_ssh_key_path(connection, groups) else {
        // No key configured — ssh-agent may already have the
        // right key, or password auth is used. Proceed anyway.
        return true;
    };

    if !key_path.exists() {
        tracing::warn!(?key_path, "SSH key file not found, skipping ssh-add");
        return true; // Don't block SFTP — agent may have it
    }

    ensure_identity_loaded(&key_path).is_ok()
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
/// Uses mc's shell VFS: `["mc", "-g", "<downloads>", "sh://user@host:port/~"]`.
/// Left panel shows the XDG Downloads directory, right panel the remote host.
/// Requires the SSH key to be loaded in ssh-agent beforehand.
///
/// The URI carries only user, host and port because that is all mc's `sh://`
/// syntax accepts. Everything else the connection configures — jump host,
/// identity file, `HostKeyAlias`, custom options — reaches the `ssh` that mc
/// spawns through [`crate::mc_ssh::prepare_mc_ssh_env`], which the caller must
/// apply to the mc process environment.
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

// The Flatpak-only `ensure_flatpak_mc_ssh_wrapper` used to live here. It wrote a
// PATH-shadowing `ssh` wrapper that injected nothing but a writable
// `known_hosts`, which left every other SSH option — a jump host above all
// (issue #255) — unreachable from mc. `crate::mc_ssh` generalises the same trick:
// it generates a full `ssh_config` per session and applies on every platform.

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
        let uri = build_sftp_uri(Some("admin"), "server.example.com", 22, None);
        assert_eq!(uri, "sftp://admin@server.example.com");
    }

    #[test]
    fn test_build_sftp_uri_without_user() {
        let uri = build_sftp_uri(None, "10.0.0.1", 22, None);
        assert_eq!(uri, "sftp://10.0.0.1");
    }

    #[test]
    fn test_build_sftp_uri_custom_port() {
        let uri = build_sftp_uri(Some("root"), "host.local", 2222, None);
        assert_eq!(uri, "sftp://root@host.local:2222");
    }

    #[test]
    fn test_build_sftp_uri_with_path_appends_absolute() {
        // Relative and trailing-slash forms both normalise to one absolute path.
        assert_eq!(
            build_sftp_uri(Some("u"), "host", 22, Some("home/u")),
            "sftp://u@host/home/u"
        );
        assert_eq!(
            build_sftp_uri(Some("u"), "host", 2222, Some("/home/u/")),
            "sftp://u@host:2222/home/u"
        );
    }

    #[test]
    fn test_build_sftp_uri_empty_path_is_ignored() {
        assert_eq!(
            build_sftp_uri(Some("u"), "host", 22, Some("   ")),
            "sftp://u@host"
        );
    }

    #[test]
    fn test_normalize_remote_path() {
        assert_eq!(normalize_remote_path(None), None);
        assert_eq!(normalize_remote_path(Some("")), None);
        assert_eq!(normalize_remote_path(Some("/")), Some("/".to_string()));
        assert_eq!(
            normalize_remote_path(Some("var/www")),
            Some("/var/www".to_string())
        );
    }

    #[test]
    fn test_build_sftp_uri_from_connection_uses_remote_path() {
        use crate::models::ProtocolConfig;
        let mut conn = Connection::new_ssh("Test".to_string(), "host".to_string(), 22);
        conn.username = Some("u".to_string());
        if let ProtocolConfig::Ssh(cfg) = &mut conn.protocol_config {
            cfg.remote_path = Some("/var/www/vhosts/example".to_string());
        }
        assert_eq!(
            build_sftp_uri_from_connection(&conn).unwrap(),
            "sftp://u@host/var/www/vhosts/example"
        );
    }

    #[test]
    fn test_build_sftp_command_default_port() {
        let mut conn =
            Connection::new_ssh("Test".to_string(), "server.example.com".to_string(), 22);
        conn.username = Some("admin".to_string());

        let cmd = build_sftp_command(&conn, &[], &[], &crate::config::NetworkSettings::default())
            .unwrap();
        assert_eq!(cmd, vec!["sftp", "admin@server.example.com"]);
    }

    #[test]
    fn test_build_sftp_command_custom_port() {
        let mut conn = Connection::new_ssh("Test".to_string(), "host.local".to_string(), 2222);
        conn.username = Some("root".to_string());

        let cmd = build_sftp_command(&conn, &[], &[], &crate::config::NetworkSettings::default())
            .unwrap();
        assert_eq!(cmd, vec!["sftp", "-P", "2222", "root@host.local"]);
    }

    #[test]
    fn test_build_sftp_command_non_ssh() {
        let conn = Connection::new_rdp("Test".to_string(), "server.example.com".to_string(), 3389);
        assert!(
            build_sftp_command(&conn, &[], &[], &crate::config::NetworkSettings::default())
                .is_none()
        );
    }

    #[test]
    fn test_build_sftp_command_includes_referenced_jump_host() {
        // A jump host chosen in the connection editor is stored as a reference,
        // not as a `proxy_jump` string; `sftp` used to miss it entirely.
        let mut bastion =
            Connection::new_ssh("Bastion".to_string(), "jump.example.com".to_string(), 22);
        bastion.username = Some("ops".to_string());

        let mut conn =
            Connection::new_ssh("Test".to_string(), "target.example.com".to_string(), 22);
        conn.username = Some("me".to_string());
        if let crate::models::ProtocolConfig::Ssh(ref mut cfg) = conn.protocol_config {
            cfg.jump_host_id = Some(bastion.id);
        }

        let cmd = build_sftp_command(
            &conn,
            std::slice::from_ref(&bastion),
            &[],
            &crate::config::NetworkSettings::default(),
        )
        .unwrap();
        assert_eq!(
            cmd,
            vec![
                "sftp",
                "-J",
                "ops@jump.example.com",
                "me@target.example.com"
            ]
        );
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

    // ── A3: ensure_key_in_agent invariants kept during ssh-add consolidation ──

    /// "Cannot tell" stays its own outcome, so the mc launcher can warn and
    /// go on instead of refusing.
    #[test]
    fn a_key_presence_rustconn_cannot_tell_is_unverified_not_failed() {
        use crate::ssh_agent::AgentError;
        assert_eq!(
            IdentityLoadError::from(AgentError::KeyPresenceUnknown("why".to_owned())),
            IdentityLoadError::Unverified("why".to_owned())
        );
        assert_eq!(
            IdentityLoadError::from(AgentError::AddKeyFailed("bad key".to_owned())),
            IdentityLoadError::Failed("bad key".to_owned())
        );
        assert!(matches!(
            IdentityLoadError::from(AgentError::NotRunning),
            IdentityLoadError::Failed(_)
        ));
    }

    #[test]
    fn ensure_key_in_agent_proceeds_when_no_key_is_configured() {
        // SHALL CONTINUE TO: a connection with no SSH key must not block the
        // SFTP/mc launch — the agent may already hold the right key, or
        // password auth is used. No `ssh-add` runs.
        let conn = Connection::new_ssh("Test".to_string(), "host".to_string(), 22);
        assert!(ensure_key_in_agent(&conn, &[]));
    }

    #[test]
    fn ensure_key_in_agent_proceeds_when_key_file_is_absent() {
        // SHALL CONTINUE TO: a configured key whose file does not exist must
        // not block the launch (the agent may still have it loaded).
        let mut conn = Connection::new_ssh("Test".to_string(), "host".to_string(), 22);
        if let crate::models::ProtocolConfig::Ssh(ref mut cfg) = conn.protocol_config {
            cfg.key_source = crate::models::SshKeySource::File {
                path: PathBuf::from("/nonexistent/rustconn-test-key-does-not-exist"),
            };
        }
        assert!(ensure_key_in_agent(&conn, &[]));
    }

    /// The SFTP add asks, adds to and records against the agent mc and sftp
    /// reach through `apply_agent_env`: the one RustConn started wins over
    /// the inherited `SSH_AUTH_SOCK`, which may be unset or stale then.
    #[test]
    fn the_sftp_add_uses_the_agent_rustconn_started_over_the_inherited_one() {
        assert_eq!(
            agent_socket_from(
                Some("/run/user/1000/rustconn-ssh-agent.sock"),
                Some("/tmp/ssh-stale/agent.1".to_owned())
            ),
            Some("/run/user/1000/rustconn-ssh-agent.sock".to_owned())
        );
        assert_eq!(
            agent_socket_from(None, Some("/tmp/ssh-x/agent.1".to_owned())),
            Some("/tmp/ssh-x/agent.1".to_owned())
        );
        assert_eq!(
            agent_socket_from(Some(""), Some("/tmp/ssh-x/agent.1".to_owned())),
            Some("/tmp/ssh-x/agent.1".to_owned())
        );
        assert_eq!(agent_socket_from(None, Some(String::new())), None);
        assert_eq!(agent_socket_from(None, None), None);
    }
}
