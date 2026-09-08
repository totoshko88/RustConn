//! SSH tunnel for forwarding connections through a jump host.
//!
//! Used by RDP, VNC, SPICE, and Telnet connections that have a
//! `jump_host_id` configured. Creates an `ssh -L` local port forward
//! in the background and returns the local port for the client to
//! connect to.

use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

use crate::models::{SshAuthMethod, SshConfig};

/// Errors that can occur when creating an SSH tunnel.
#[derive(Debug, Error)]
pub enum SshTunnelError {
    /// No free local port could be found.
    #[error("Could not find a free local port")]
    NoFreePort,
    /// Failed to spawn the SSH process.
    #[error("Failed to spawn SSH tunnel: {0}")]
    SpawnFailed(#[from] std::io::Error),
}

/// Result type for SSH tunnel operations.
pub type SshTunnelResult<T> = Result<T, SshTunnelError>;

/// A running SSH tunnel (`ssh -N -L ...`).
///
/// The tunnel process is killed when this struct is dropped.
/// If a temporary askpass script was created, it is zeroized and deleted.
pub struct SshTunnel {
    /// The child SSH process.
    child: Child,
    /// The local port that forwards to the remote destination.
    local_port: u16,
    /// Captured stderr output from the SSH process (populated by background reader).
    stderr_output: Arc<Mutex<String>>,
    /// Path to the temporary askpass script (cleaned up on drop).
    askpass_script: Option<std::path::PathBuf>,
}

impl SshTunnel {
    /// Returns the local port to connect to.
    #[must_use]
    pub const fn local_port(&self) -> u16 {
        self.local_port
    }

    /// Checks whether the SSH tunnel process is still running.
    ///
    /// Returns `true` if the process is alive, `false` if it has exited.
    /// When the process has exited, any captured stderr is logged.
    pub fn is_alive(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(None) => true,
            Ok(Some(status)) => {
                let stderr = self
                    .stderr_output
                    .lock()
                    .map(|s| s.clone())
                    .unwrap_or_default();
                if stderr.is_empty() {
                    tracing::error!(
                        local_port = self.local_port,
                        %status,
                        "SSH tunnel process exited"
                    );
                } else {
                    tracing::error!(
                        local_port = self.local_port,
                        %status,
                        stderr = %stderr.trim(),
                        "SSH tunnel process exited"
                    );
                }
                false
            }
            Err(e) => {
                tracing::error!(
                    local_port = self.local_port,
                    %e,
                    "Failed to check SSH tunnel process status"
                );
                false
            }
        }
    }

    /// Returns any captured stderr output from the SSH process.
    #[must_use]
    pub fn stderr(&self) -> String {
        self.stderr_output
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    /// Stops the tunnel by killing the SSH process.
    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        self.stop();
        if let Some(ref path) = self.askpass_script {
            cleanup_askpass_script(path);
        }
    }
}

/// Parameters for creating an SSH tunnel.
#[derive(Debug, Clone)]
pub struct SshTunnelParams {
    /// Jump host address (e.g. `user@bastion.example.com`).
    pub jump_host: String,
    /// Jump host SSH port (default 22).
    pub jump_port: u16,
    /// Remote destination host (the actual RDP/VNC/SPICE server).
    pub remote_host: String,
    /// Remote destination port.
    pub remote_port: u16,
    /// Optional SSH identity file for the jump host.
    pub identity_file: Option<String>,
    /// Optional password for the jump host (used via `SSH_ASKPASS`).
    ///
    /// When set, a temporary askpass helper script is created and
    /// `SSH_ASKPASS_REQUIRE=force` is used so OpenSSH calls the script
    /// instead of prompting on a TTY. `BatchMode` is NOT set in this case.
    pub password: Option<SecretString>,
    /// Optional extra SSH args (e.g. `-o StrictHostKeyChecking=no`).
    pub extra_args: Vec<String>,
}

/// Environment variable name used to pass the password to the askpass script.
/// Intentionally obscure to reduce exposure in `/proc/PID/environ`.
const TUNNEL_ASKPASS_ENV_VAR: &str = "_RC_TUN_PW";

/// Creates a temporary `SSH_ASKPASS` helper script that echoes the password
/// from [`TUNNEL_ASKPASS_ENV_VAR`]. The script is created with mode 0700.
///
/// # Errors
///
/// Returns a human-readable error string on failure.
fn create_tunnel_askpass_script() -> Result<std::path::PathBuf, String> {
    use std::io::Write;

    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "rc-tun-askpass-{}",
        uuid::Uuid::new_v4().as_hyphenated()
    ));

    let script = format!("#!/bin/sh\necho \"${TUNNEL_ASKPASS_ENV_VAR}\"\n");

    let mut file = std::fs::File::create(&path)
        .map_err(|e| format!("Failed to create tunnel askpass script: {e}"))?;
    file.write_all(script.as_bytes())
        .map_err(|e| format!("Failed to write tunnel askpass script: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("Failed to set tunnel askpass script permissions: {e}"))?;
    }

    Ok(path)
}

/// Cleans up a temporary askpass script, zeroizing its content first.
fn cleanup_askpass_script(path: &std::path::Path) {
    // Overwrite with zeros before deletion to prevent recovery
    if let Ok(metadata) = std::fs::metadata(path) {
        let size = metadata.len() as usize;
        if size > 0 {
            let _ = std::fs::write(path, vec![0u8; size]);
        }
    }
    let _ = std::fs::remove_file(path);
}

/// Finds a free TCP port by binding to port 0 and reading the assigned port.
///
/// # Errors
///
/// Returns `SshTunnelError::NoFreePort` if binding fails.
pub fn find_free_port() -> SshTunnelResult<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| SshTunnelError::NoFreePort)?;
    let port = listener
        .local_addr()
        .map_err(|_| SshTunnelError::NoFreePort)?
        .port();
    // Drop the listener so the port is released before SSH binds to it.
    // There is a small TOCTOU window, but it is acceptable for this use case.
    drop(listener);
    Ok(port)
}

/// Creates an SSH tunnel by spawning `ssh -N -L local_port:remote:remote_port`.
///
/// The tunnel runs in the background. The caller must keep the returned
/// [`SshTunnel`] alive for the duration of the connection — dropping it
/// kills the SSH process.
///
/// # Errors
///
/// Returns an error if no free port is found or the SSH process fails to spawn.
pub fn create_tunnel(params: &SshTunnelParams) -> SshTunnelResult<SshTunnel> {
    let local_port = find_free_port()?;
    let forward_spec = format!(
        "{}:{}:{}",
        local_port, params.remote_host, params.remote_port
    );
    let remote_desc = format!("{}:{}", params.remote_host, params.remote_port);
    spawn_tunnel(params, local_port, &["-L", &forward_spec], &remote_desc)
}

/// Creates a dynamic SOCKS tunnel by spawning `ssh -N -D <local_port> <jump_host>`.
///
/// Unlike [`create_tunnel`], which forwards a single destination, this opens a
/// local SOCKS5 proxy on a free port: any client pointed at
/// `socks5://127.0.0.1:<local_port>` reaches the network the jump host sits on.
/// It is what an embedded Web connection uses to browse through an SSH host
/// (issue: Ásbrú-style incognito browser). `remote_host`/`remote_port` on
/// `params` are ignored — a SOCKS proxy has no single destination.
///
/// The tunnel runs in the background; keep the returned [`SshTunnel`] alive for
/// the life of the browsing session — dropping it kills the SSH process and
/// closes the proxy.
///
/// # Errors
///
/// Returns an error if no free port is found or the SSH process fails to spawn.
pub fn create_socks_tunnel(params: &SshTunnelParams) -> SshTunnelResult<SshTunnel> {
    let local_port = find_free_port()?;
    let port_str = local_port.to_string();
    spawn_tunnel(params, local_port, &["-D", &port_str], "SOCKS proxy")
}

/// Shared spawn path for [`create_tunnel`] and [`create_socks_tunnel`].
///
/// `forward_args` is the forwarding flag and its value (`["-L", spec]` or
/// `["-D", port]`); `remote_desc` is a human-readable destination for the log.
/// Everything else — jump-host port, identity, askpass, Flatpak known_hosts,
/// `ExitOnForwardFailure`, stderr capture — is identical between the two.
fn spawn_tunnel(
    params: &SshTunnelParams,
    local_port: u16,
    forward_args: &[&str; 2],
    remote_desc: &str,
) -> SshTunnelResult<SshTunnel> {
    let mut cmd = Command::new("ssh");
    cmd.arg("-N") // No remote command — just forward
        .arg(forward_args[0])
        .arg(forward_args[1]);

    // Jump host port
    if params.jump_port != 22 {
        cmd.arg("-p").arg(params.jump_port.to_string());
    }

    // Identity file
    if let Some(ref key) = params.identity_file {
        cmd.arg("-i").arg(key);
    }

    // Extra args
    for arg in &params.extra_args {
        cmd.arg(arg);
    }

    // Flatpak writable known_hosts
    if let Some(kh_path) = crate::get_flatpak_known_hosts_path() {
        cmd.arg("-o")
            .arg(format!("UserKnownHostsFile={}", kh_path.display()));
    }

    // SSH_ASKPASS for password-authenticated jump hosts, or BatchMode
    // when no password is available (prevents SSH from hanging on a
    // TTY prompt that nobody can answer).
    let askpass_script_path = if let Some(ref pw) = params.password {
        match create_tunnel_askpass_script() {
            Ok(script_path) => {
                cmd.env("SSH_ASKPASS", &script_path);
                cmd.env("SSH_ASKPASS_REQUIRE", "force");
                cmd.env(TUNNEL_ASKPASS_ENV_VAR, pw.expose_secret());
                // Ensure DISPLAY is set so SSH considers ASKPASS
                if std::env::var("DISPLAY").is_err() {
                    cmd.env("DISPLAY", "");
                }
                Some(script_path)
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "Failed to create SSH_ASKPASS script for tunnel; \
                     falling back to BatchMode (password auth will fail)"
                );
                cmd.arg("-o").arg("BatchMode=yes");
                None
            }
        }
    } else {
        // No password — prevent SSH from reading stdin
        cmd.arg("-o").arg("BatchMode=yes");
        None
    };

    // Exit if the forwarding fails (e.g. port already in use)
    cmd.arg("-o").arg("ExitOnForwardFailure=yes");

    // The jump host destination
    cmd.arg(&params.jump_host);

    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    tracing::info!(
        local_port,
        remote = %remote_desc,
        jump_host = %params.jump_host,
        "Starting SSH tunnel"
    );

    let mut child = cmd.spawn()?;

    // Capture SSH stderr in a background thread so diagnostic messages
    // (auth failures, port unreachable, etc.) are available for logging.
    let stderr_output = Arc::new(Mutex::new(String::new()));
    if let Some(stderr_handle) = child.stderr.take() {
        let stderr_buf = Arc::clone(&stderr_output);
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(stderr_handle);
            for line in reader.lines() {
                match line {
                    Ok(line) => {
                        tracing::warn!(target: "ssh_tunnel", "{}", line);
                        if let Ok(mut buf) = stderr_buf.lock() {
                            if !buf.is_empty() {
                                buf.push('\n');
                            }
                            buf.push_str(&line);
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }

    Ok(SshTunnel {
        child,
        local_port,
        stderr_output,
        askpass_script: askpass_script_path,
    })
}

/// Waits for the SSH tunnel to become ready by polling the local port.
///
/// Tries to connect to `127.0.0.1:local_port` up to `max_attempts` times
/// with `interval` between attempts. Also checks that the SSH process is
/// still alive between attempts. Returns `Ok(())` when the port
/// accepts connections, or `Err` if all attempts fail or the process exits.
///
/// # Errors
///
/// Returns `SshTunnelError::SpawnFailed` if the tunnel never becomes ready
/// or the SSH process exits prematurely.
pub fn wait_for_tunnel_ready(
    tunnel: &mut SshTunnel,
    max_attempts: u32,
    interval: std::time::Duration,
) -> SshTunnelResult<()> {
    use std::net::TcpStream;

    let local_port = tunnel.local_port;

    for attempt in 1..=max_attempts {
        // Check if SSH process is still alive before trying to connect
        if !tunnel.is_alive() {
            let stderr = tunnel.stderr();
            let detail = if stderr.is_empty() {
                "SSH process exited unexpectedly".to_string()
            } else {
                format!("SSH process exited: {}", stderr.trim())
            };
            return Err(SshTunnelError::SpawnFailed(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                detail,
            )));
        }

        match TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], local_port)),
            std::time::Duration::from_secs(1),
        ) {
            Ok(_) => {
                tracing::debug!(local_port, attempt, "SSH tunnel is ready");
                return Ok(());
            }
            Err(_) => {
                if attempt < max_attempts {
                    std::thread::sleep(interval);
                }
            }
        }
    }

    Err(SshTunnelError::SpawnFailed(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        format!("SSH tunnel on port {local_port} not ready after {max_attempts} attempts"),
    )))
}

/// Probes the remote endpoint through an established SSH tunnel.
///
/// Connects to the tunnel's local port and waits for the remote end to
/// respond within `timeout`. If the remote host/port is unreachable
/// (firewall, service down), the connection will either be refused or
/// time out.
///
/// Returns `Ok(())` if the remote end accepts the connection, or an
/// error describing why it failed.
///
/// # Errors
///
/// Returns `SshTunnelError::SpawnFailed` if the remote port is unreachable
/// or the SSH tunnel process has exited.
pub fn probe_tunnel_remote(
    tunnel: &mut SshTunnel,
    timeout: std::time::Duration,
) -> SshTunnelResult<()> {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    // First check the tunnel process is still alive
    if !tunnel.is_alive() {
        let stderr = tunnel.stderr();
        let detail = if stderr.is_empty() {
            "SSH tunnel process exited before probe".to_string()
        } else {
            format!("SSH tunnel exited: {}", stderr.trim())
        };
        return Err(SshTunnelError::SpawnFailed(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            detail,
        )));
    }

    let local_port = tunnel.local_port;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], local_port));

    // Connect to the tunnel's local port
    let mut stream = TcpStream::connect_timeout(&addr, timeout).map_err(|e| {
        SshTunnelError::SpawnFailed(std::io::Error::new(
            e.kind(),
            format!("Cannot connect to tunnel port {local_port}: {e}"),
        ))
    })?;

    // Set read/write timeouts for the probe
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    // Send a minimal probe byte and wait for any response or error.
    // For RDP (port 3389), the server responds to any data with an
    // X.224 Connection Confirm or rejects the connection. If the
    // remote port is unreachable, SSH will close the forwarded
    // channel and we'll get a connection reset or EOF.
    //
    // We send a single zero byte — this is enough to trigger SSH
    // channel forwarding to the remote host. If the remote host is
    // unreachable, SSH will close the local socket.
    let _ = stream.write_all(&[0]);
    let _ = stream.flush();

    // Give SSH time to forward and detect unreachable remote
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Try to read — if the remote is unreachable, SSH will have
    // closed the connection and we'll get an error or EOF.
    let mut buf = [0u8; 1];
    match stream.read(&mut buf) {
        Ok(0) => {
            // EOF — SSH closed the forwarded channel (remote unreachable)
            Err(SshTunnelError::SpawnFailed(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                format!(
                    "Remote port unreachable through SSH tunnel (port {local_port}): \
                     connection closed by tunnel"
                ),
            )))
        }
        Ok(_) => {
            // Got data back — remote is alive and responding
            tracing::debug!(local_port, "Remote endpoint is reachable through tunnel");
            Ok(())
        }
        Err(ref e)
            if e.kind() == std::io::ErrorKind::WouldBlock
                || e.kind() == std::io::ErrorKind::TimedOut =>
        {
            // Read timed out — the remote accepted the connection but
            // hasn't sent data yet. This is normal for many protocols
            // (they wait for a proper handshake). The important thing
            // is that SSH didn't close the channel, so the remote is
            // reachable.
            tracing::debug!(
                local_port,
                "Remote endpoint accepted connection through tunnel (read timed out, which is OK)"
            );
            Ok(())
        }
        Err(e) => {
            // Connection reset, broken pipe, etc. — remote unreachable
            Err(SshTunnelError::SpawnFailed(std::io::Error::new(
                e.kind(),
                format!("Remote port unreachable through SSH tunnel (port {local_port}): {e}"),
            )))
        }
    }
}

/// Parses a jump host string in `[user@]host[:port]` format and appends
/// the correct SSH arguments for use inside a `ProxyCommand`.
///
/// Inside `ProxyCommand`, the port must be specified via `-p port` and the
/// destination is `user@host` (without the `:port` suffix). The standard
/// `-J` format `user@host:port` is invalid inside `ProxyCommand`.
///
/// Handles IPv6 addresses in brackets: `[::1]:2222`.
pub fn append_proxy_command_destination(proxy_parts: &mut Vec<String>, jump_host: &str) {
    let (user_part, host_port) = if let Some(at_pos) = jump_host.rfind('@') {
        (Some(&jump_host[..at_pos]), &jump_host[at_pos + 1..])
    } else {
        (None, jump_host)
    };

    let (host, port) = if host_port.starts_with('[') {
        // IPv6: [addr]:port
        if let Some(bracket_end) = host_port.find(']') {
            let after_bracket = &host_port[bracket_end + 1..];
            if let Some(colon_port) = after_bracket.strip_prefix(':') {
                (&host_port[..=bracket_end], Some(colon_port))
            } else {
                (host_port, None)
            }
        } else {
            (host_port, None)
        }
    } else if let Some(colon_pos) = host_port.rfind(':') {
        let maybe_port = &host_port[colon_pos + 1..];
        if maybe_port.chars().all(|c| c.is_ascii_digit()) && !maybe_port.is_empty() {
            (&host_port[..colon_pos], Some(maybe_port))
        } else {
            (host_port, None)
        }
    } else {
        (host_port, None)
    };

    if let Some(p) = port
        && p != "22"
    {
        proxy_parts.push("-p".to_string());
        proxy_parts.push(p.to_string());
    }

    let destination = if let Some(user) = user_part {
        format!("{user}@{host}")
    } else {
        host.to_string()
    };
    proxy_parts.push(destination);
}

/// Single-quotes `s` for safe embedding inside a `sh -c` `ProxyCommand`,
/// escaping any embedded single quote as `'\''`.
///
/// Needed because OpenSSH runs a `ProxyCommand` through `/bin/sh -c`, so a
/// nested `ProxyCommand` value (which itself contains spaces) must be a single
/// shell word.
#[must_use]
pub fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Joins an `argv` array into one shell-safe line for a human-readable echo.
///
/// The command is spawned as a real `argv` array (`Command::args`), so an
/// argument that contains spaces — such as `ProxyCommand=nc -X 5 %h %p` — is a
/// single element and reaches `ssh` intact. A naive `join(" ")` for the
/// "Executing:" echo loses that word boundary, making the value look split
/// across several arguments and suggesting (wrongly) that the option was
/// dropped (issue #322). Each element that is not already a bare shell word is
/// single-quoted so the printed line is both accurate and copy-paste safe;
/// simple tokens like `ssh`, `-o` or `user@host` are left unquoted for
/// readability.
#[must_use]
pub fn format_argv_for_display(argv: &[String]) -> String {
    argv.iter()
        .map(|arg| {
            if arg.is_empty() || arg.chars().any(is_shell_unsafe_char) {
                shell_single_quote(arg)
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Reports whether a character forces an argument to be quoted for display.
///
/// Anything outside the conservative "bare word" set — letters, digits and the
/// punctuation that routinely appears unquoted in an `ssh` invocation
/// (`@`, `:`, `.`, `/`, `-`, `_`, `%`, `,`, `=`, `+`) — triggers quoting. A
/// space is the common trigger (`ProxyCommand=nc -X 5 %h %p`); the rest keeps
/// values with shell metacharacters safe to paste.
fn is_shell_unsafe_char(c: char) -> bool {
    let is_bare_word_char = c.is_ascii_alphanumeric()
        || matches!(c, '@' | ':' | '.' | '/' | '-' | '_' | '%' | ',' | '=' | '+');
    !is_bare_word_char
}

/// Converts a RustConn jump-host chain into the value for OpenSSH's `-J`
/// (`ProxyJump`) option, fixing the hop direction.
///
/// RustConn resolves chains target-first (`chain[0]` is the bastion closest to
/// the target, walking outward to the client), which is the order
/// [`build_nested_proxy_command`] consumes directly. OpenSSH's `-J`, however,
/// visits hops left-to-right starting from the client, so the comma-separated
/// list must be reversed. Without this, a two-bastion chain `J_near,J_far`
/// would be contacted as client→J_near→J_far→target instead of
/// client→J_far→J_near→target (#191 — multi-hop direction).
///
/// Accepts a comma-separated chain; entries are trimmed and empty ones dropped.
/// A single-hop chain is returned unchanged (the common case), so existing
/// single-bastion connections are unaffected.
#[must_use]
pub fn proxy_jump_arg(chain_target_first: &str) -> String {
    chain_target_first
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .rev()
        .collect::<Vec<_>>()
        .join(",")
}

/// Returns whether SSH routing is opaque to RustConn's sanitized proxy builder.
///
/// Dedicated `proxy_command` and raw routing options keep their existing manual
/// behavior, but must never be combined with RustConn-managed hop credentials.
#[must_use]
pub fn has_unmanaged_proxy_route(config: &SshConfig) -> bool {
    config.proxy_command.is_some()
        || config.custom_options.keys().any(|key| {
            key.eq_ignore_ascii_case("ProxyCommand") || key.eq_ignore_ascii_case("ProxyJump")
        })
}

/// Reports whether a `ProxyCommand` string can itself prompt for a password.
///
/// A `ProxyCommand` set under Connection settings is not necessarily an SSH
/// bastion. The common case (issue #322) is a plain TCP relay —
/// `nc -X 5 -x 127.0.0.1:1080 %h %p`, `ncat`, `socat`, `connect`, `corkscrew`,
/// `proxytunnel` — which forwards bytes and never
/// speaks SSH, so it opens no interactive prompt of its own. The target host
/// then asks for the password directly, and answering it hands the credential
/// to no one but the target: there is nothing to leak to (contrast a nested
/// `ssh` hop, which #191 protects against).
///
/// Returns `false` for a recognised relay so the caller can keep target
/// password auto-fill enabled. Returns `true` — the conservative, credential-safe
/// answer — when the relay is unrecognised or the command itself invokes `ssh`
/// (a hand-written bastion `ProxyCommand`), since that nested `ssh` can prompt
/// and auto-fill must stay suppressed to avoid the #191 leak.
#[must_use]
pub fn proxy_command_can_prompt_for_password(proxy_command: &str) -> bool {
    // Recognised byte-relay front commands never prompt. Compare the command's
    // basename (first shell word, path stripped) case-insensitively.
    const RELAYS: &[&str] = &[
        "nc",
        "ncat",
        "netcat",
        "socat",
        "connect",
        "connect-proxy",
        "corkscrew",
        "proxytunnel",
        "cloudflared",
        "boringproxy",
        "gost",
    ];

    // A nested `ssh` invocation is a bastion hop that can prompt — always
    // conservative. Matched as a whole word so `sshpass` or a path containing
    // "ssh" in a relay name does not trip it, and case-insensitively.
    let mentions_ssh = proxy_command
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| word.eq_ignore_ascii_case("ssh") || word.eq_ignore_ascii_case("sshpass"));
    if mentions_ssh {
        return true;
    }

    let front = proxy_command
        .split_whitespace()
        .next()
        .unwrap_or("")
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("");
    if !front.is_empty() && RELAYS.iter().any(|r| front.eq_ignore_ascii_case(r)) {
        return false;
    }

    // Unknown command: assume it could prompt and keep auto-fill suppressed.
    true
}

/// Parses `ssh -G` output and reports whether it declares an effective proxy route.
///
/// Split out from [`ssh_config_declares_proxy`] so the parsing is testable without running
/// `ssh`. `ssh -G` prints resolved keywords lowercased, one per line, and omits `proxycommand`
/// and `proxyjump` entirely when neither applies; an explicit `none` counts as no route, which
/// is how OpenSSH itself spells "disabled".
#[must_use]
pub fn ssh_g_output_declares_proxy(output: &str) -> bool {
    output.lines().any(|line| {
        let mut parts = line.split_whitespace();
        let keyword = parts.next().unwrap_or_default();
        if !keyword.eq_ignore_ascii_case("proxycommand")
            && !keyword.eq_ignore_ascii_case("proxyjump")
        {
            return false;
        }
        let value = parts.next().unwrap_or_default();
        !value.is_empty() && !value.eq_ignore_ascii_case("none")
    })
}

/// Returns whether OpenSSH's own configuration would route `host` through a proxy.
///
/// Asks `ssh -G`, which resolves `~/.ssh/config`, `/etc/ssh/ssh_config` and every `Match`/`Host`
/// block exactly as a real connection would, and prints the effective value of every keyword.
/// That is the only honest way to answer the question: RustConn cannot see a bastion the user
/// declared in their own config, and a stored setting is not a description of what OpenSSH will
/// do — the same reasoning that issue #307 applied to Password Source.
///
/// This exists because automatic target-password delivery must not be combined with a proxy
/// RustConn did not build. The nested `ssh` that a config-declared `ProxyJump` spawns inherits the
/// outer process's `SSH_ASKPASS` and credential path, and its bastion prompt has the same
/// `<user>@<host>'s password: ` shape the helper answers — so the target's password would be typed
/// at the bastion, which is issue #191 in a new costume. Until the 0.21.2 review this was handled
/// by appending `ProxyJump=none`/`ProxyCommand=none`, which closed the leak by silently discarding
/// the user's routing and breaking the connection instead. Detecting it and declining to deliver
/// the password is the same protection without the collateral damage: the connection simply
/// prompts, which is the documented fallback.
///
/// Cost is a single short-lived `ssh -G`, measured at ~4 ms against OpenSSH 10.2 — it performs no
/// network I/O. Returns `true` on any failure to run or parse, because "cannot tell" must not
/// enable automatic delivery.
#[must_use]
pub fn ssh_config_declares_proxy(host: &str, port: u16, username: Option<&str>) -> bool {
    let mut command = std::process::Command::new("ssh");
    command.arg("-G").arg("-p").arg(port.to_string());
    if let Some(user) = username.map(str::trim).filter(|user| !user.is_empty()) {
        command.arg("-l").arg(user);
    }
    command
        .arg(host)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    match command.output() {
        Ok(output) if output.status.success() => {
            let declares = ssh_g_output_declares_proxy(&String::from_utf8_lossy(&output.stdout));
            if declares {
                tracing::debug!(
                    host,
                    "ssh -G reports a proxy route; automatic password delivery stays off"
                );
            }
            declares
        }
        Ok(output) => {
            tracing::warn!(
                host,
                status = ?output.status.code(),
                "ssh -G failed; assuming a proxy route and leaving automatic password delivery off"
            );
            true
        }
        Err(error) => {
            tracing::warn!(
                host,
                %error,
                "could not run ssh -G; assuming a proxy route and leaving automatic password delivery off"
            );
            true
        }
    }
}

/// Returns whether the outer SSH process may receive the cached target password via askpass.
///
/// What makes automatic delivery safe is the helper's prompt discrimination, not a narrow set
/// of connections: the helper answers only the prompt OpenSSH *generates itself* for the
/// `password` method (`<user>@<host>'s password: `, verified against OpenSSH 10.2) and exits
/// without printing for a key passphrase, a host-key question, a token touch, an OTP or a
/// password change. A method the helper refuses simply fails and OpenSSH moves on to the next
/// one, so an identity being available alongside a stored password is not a hazard — if the key
/// authenticates, the helper is never consulted at all.
///
/// So this gate covers the cases where RustConn cannot reason about the launch, not the question
/// of which credential wins:
///
/// * a custom option that redirects authentication or routing — the user has taken over, and an
///   `-o` of ours on the command line would silently outrank what they wrote;
/// * a PKCS#11 token or a legacy agent fingerprint, where the interactive PIN/touch step is the
///   point and a forced helper would sit in front of it;
/// * `SecurityKey` or `KeyboardInteractive` authentication, for the same reason;
/// * an opaque proxy route ([`has_unmanaged_proxy_route`]), because the nested `ssh` it spawns is
///   outside the env-sanitised chain this module builds.
///
/// A connection the gate refuses is not left without its password: the caller falls back to the
/// terminal watcher, which is what handled every SSH connection before 0.21.2.
///
/// `has_resolved_identity` is accepted and deliberately *not* treated as a veto. It was one until
/// the review of 0.21.2: a key inherited from a group turned automatic delivery off for a
/// password-authenticated connection, which is the same conflation
/// [`crate::Connection::expects_password_prompt`] was written to avoid. The parameter stays in
/// the signature because callers have the value and because a reader who does not find this note
/// will re-add the veto.
#[must_use]
pub fn target_password_askpass_allowed(config: &SshConfig, has_resolved_identity: bool) -> bool {
    const AUTH_ROUTING_OPTIONS: &[&str] = &[
        "batchmode",
        "certfile",
        "certificatefile",
        "challengeresponseauthentication",
        "identityagent",
        "identityfile",
        "kbdinteractiveauthentication",
        "numberofpasswordprompts",
        "passwordauthentication",
        "pkcs11provider",
        "preferredauthentications",
        "proxycommand",
        "proxyjump",
        "pubkeyauthentication",
        "securitykeyprovider",
    ];

    // Read only so the parameter is not silently dropped from the signature; see the doc note
    // above for why an available identity is not a reason to refuse.
    let _ = has_resolved_identity;

    let has_legacy_agent_identity = config
        .agent_key_fingerprint
        .as_deref()
        .is_some_and(|fingerprint| !fingerprint.trim().is_empty());
    let has_pkcs11_identity = config.pkcs11_provider.as_deref().is_some_and(|provider| {
        let provider = provider.trim();
        !provider.is_empty() && !provider.eq_ignore_ascii_case("none")
    });
    let has_auth_override = config.custom_options.keys().any(|key| {
        AUTH_ROUTING_OPTIONS
            .iter()
            .any(|option| key.eq_ignore_ascii_case(option))
    });
    // An interactive second factor is the one thing a forced helper must never stand in front of.
    let wants_interactive_factor = matches!(
        config.auth_method,
        SshAuthMethod::SecurityKey | SshAuthMethod::KeyboardInteractive
    );

    !wants_interactive_factor
        && !has_legacy_agent_identity
        && !has_pkcs11_identity
        && !has_auth_override
        && !has_unmanaged_proxy_route(config)
}

/// Builds a (possibly nested) SSH `ProxyCommand` value that reaches `hops[0]`
/// (the hop closest to the target) through every deeper hop in `hops[1..]`.
///
/// Each hop receives `identity_file`, `known_hosts`, and — when
/// `accept_new_host_keys` — `StrictHostKeyChecking=accept-new`. This is
/// required because ProxyJump (`-J`) children do NOT inherit `-i`/`-o` from the
/// parent command: in Flatpak that breaks multi-hop chains, where deeper hops
/// fail key auth and host-key verification (issue #191 follow-up — double jump).
///
/// Returns the bare command string (no `ProxyCommand=` prefix, no surrounding
/// quotes). Nested levels are single-quoted so each `sh -c` re-parse keeps the
/// inner command as one word.
///
/// # Panics
/// Panics in debug builds if `hops` is empty (a programming bug — callers must
/// only invoke this with at least one hop).
#[must_use]
pub fn build_nested_proxy_command(
    hops: &[&str],
    identity_file: Option<&str>,
    known_hosts: Option<&std::path::Path>,
    accept_new_host_keys: bool,
) -> String {
    build_nested_proxy_command_inner(
        hops,
        identity_file,
        known_hosts,
        accept_new_host_keys,
        &[],
        false,
        (0, hops.len()),
    )
}

/// Multi-hop variant of [`build_nested_proxy_command`] with per-hop askpass.
///
/// Each hop in `hops` can optionally receive its own `SSH_ASKPASS` helper for
/// password authentication without a controlling TTY (issue #203 — multi-bastion
/// password chains).
///
/// `askpass_scripts` is index-aligned with `hops`: `askpass_scripts[i]` is the
/// askpass helper path for `hops[i]`. If the slice is shorter than `hops` or
/// the entry is `None`, that hop gets no askpass wiring (key/agent auth).
/// `hop_index_offset` identifies `hops[0]` in the complete chain and
/// `total_hop_count` lets each recursive level clear every sibling credential.
///
/// The env var carrying each hop's owner-only secret-file path is
/// `_RC_JH_PW_FILE_<depth>`, where `depth` is the hop's absolute index in the
/// original chain. The helper opens and unlinks that file before returning its
/// contents, so no password value enters the SSH process environment.
///
/// Any hop that gets an askpass helper is forced to
/// `StrictHostKeyChecking=accept-new` regardless of `accept_new_host_keys`,
/// because a forced-askpass hop with an unknown host key would otherwise feed
/// its password into the host-key confirmation prompt and loop (issue #203).
///
/// # Panics
/// Panics in debug builds if `hops` is empty.
#[must_use]
pub fn build_nested_proxy_command_with_askpass(
    hops: &[&str],
    identity_file: Option<&str>,
    known_hosts: Option<&std::path::Path>,
    accept_new_host_keys: bool,
    askpass_scripts: &[Option<&std::path::Path>],
    hop_index_offset: usize,
    total_hop_count: usize,
) -> String {
    build_nested_proxy_command_inner(
        hops,
        identity_file,
        known_hosts,
        accept_new_host_keys,
        askpass_scripts,
        true,
        (hop_index_offset, total_hop_count),
    )
}

fn build_nested_proxy_command_inner(
    hops: &[&str],
    identity_file: Option<&str>,
    known_hosts: Option<&std::path::Path>,
    accept_new_host_keys: bool,
    askpass_scripts: &[Option<&std::path::Path>],
    isolate_outer_askpass: bool,
    hop_scope: (usize, usize),
) -> String {
    debug_assert!(!hops.is_empty(), "build_nested_proxy_command needs >=1 hop");

    let mut parts = match askpass_scripts.first() {
        Some(Some(script)) => askpass_proxy_prefix(script, hop_scope.0, hop_scope.1),
        _ if isolate_outer_askpass => askpass_disabled_proxy_prefix(hop_scope.1),
        _ => Vec::new(),
    };

    parts.extend(["ssh".to_string(), "-W".to_string(), "%h:%p".to_string()]);

    // A hop authenticated via forced SSH_ASKPASS (password, no controlling TTY)
    // must NOT let an unknown host key raise the "yes/no/[fingerprint]" prompt:
    // OpenSSH routes that prompt to the askpass helper too, which answers with
    // the PASSWORD. SSH rejects it, re-prompts, and loops until the bastion
    // drops the connection — surfacing as "Connection closed by UNKNOWN port
    // 65535" (issue #203). accept-new auto-accepts a first-seen key while still
    // rejecting a CHANGED one, so MITM protection is preserved.
    let this_hop_uses_askpass = matches!(askpass_scripts.first(), Some(Some(_)));
    if accept_new_host_keys || this_hop_uses_askpass {
        parts.push("-o".to_string());
        parts.push("StrictHostKeyChecking=accept-new".to_string());
    }
    if let Some(kh) = known_hosts {
        parts.push("-o".to_string());
        parts.push(format!("UserKnownHostsFile={}", kh.display()));
    }
    if let Some(key) = identity_file {
        parts.push("-i".to_string());
        parts.push(key.to_string());
        parts.push("-o".to_string());
        parts.push("IdentitiesOnly=yes".to_string());
    }

    // Reach the deeper hops via a nested ProxyCommand so they inherit the same
    // identity/known_hosts. `-J` here would silently drop them.
    if hops.len() > 1 {
        let inner_askpass = if askpass_scripts.len() > 1 {
            &askpass_scripts[1..]
        } else {
            &[]
        };
        let inner = build_nested_proxy_command_inner(
            &hops[1..],
            identity_file,
            known_hosts,
            accept_new_host_keys,
            inner_askpass,
            isolate_outer_askpass,
            (hop_scope.0 + 1, hop_scope.1),
        );
        parts.push("-o".to_string());
        parts.push(format!("ProxyCommand={}", shell_single_quote(&inner)));
    }

    append_proxy_command_destination(&mut parts, hops[0]);
    parts.join(" ")
}

/// Builds an `env` prefix that keeps RustConn's credentials out of a proxy hop.
///
/// The target credential and every indexed bastion credential are cleared, so a hop RustConn
/// holds no password for cannot reach one it was not given. The shell that starts a
/// `ProxyCommand` necessarily inherits the outer environment long enough to execute `env`, but
/// the nested SSH process sees these overrides.
///
/// It also detaches the hop from RustConn's own `SSH_ASKPASS`, which the hop would otherwise
/// inherit from the outer `ssh` whenever target delivery is active. Both variables are set to the
/// *empty string* rather than to a value, and the distinction is the whole point:
///
/// * `SSH_ASKPASS=` — OpenSSH treats an unset **or empty** value as "use the compiled-in default"
///   (`/usr/bin/ssh-askpass` on Debian and Ubuntu, an alternatives symlink that points at the
///   desktop helper), verified against OpenSSH 10.2, which reports `ssh_askpass: exec()` against
///   that path when the default is not installed. So the hop keeps a working passphrase dialog
///   while no longer pointing at a RustConn script.
/// * `SSH_ASKPASS_REQUIRE=` — back to OpenSSH's own rule, askpass only when there is no TTY,
///   rather than to a forcing value.
///
/// Until the review of 0.21.2 the second one was `never`, which forbids askpass outright. A
/// `ProxyCommand` has no controlling TTY, so `never` left the hop with no way to ask for anything:
/// a bastion whose key carries a passphrase, with no agent loaded, went from a passphrase dialog
/// to `Permission denied`. Nothing about RustConn's isolation needed that — with the credential
/// variables cleared, RustConn's helper finds an empty path and exits without printing, which is
/// the same refusal by a cheaper route.
#[must_use]
pub fn askpass_disabled_proxy_prefix(hop_count: usize) -> Vec<String> {
    let mut prefix = vec!["env".to_string(), "_RC_TGT_PW_FILE=".to_string()];
    prefix.extend((0..hop_count).map(|index| format!("{}=", jump_host_pw_env_name(index))));
    prefix.push("SSH_ASKPASS=".to_string());
    prefix.push("SSH_ASKPASS_REQUIRE=".to_string());
    prefix
}

/// Builds the `env`-assignment prefix for a bastion's `SSH_ASKPASS` helper.
///
/// Scopes the helper to a single nested bastion `ProxyCommand` (issue #191 —
/// the bastion authenticates with its OWN password out-of-band, never via the
/// target's VTE prompt).
///
/// OpenSSH ≥10 prepends `exec` to a `ProxyCommand`, and `exec VAR=val cmd` is
/// not valid POSIX `sh` (the shell treats `VAR=val` as a command path), so the
/// assignments ride on the `env` command (`env VAR=val cmd`), which works in
/// every shell. `SSH_ASKPASS_REQUIRE=force` makes OpenSSH call the helper even
/// without a controlling TTY.
///
/// Returns tokens beginning with `env _RC_TGT_PW_FILE=` to prepend to the
/// bastion `ssh -W %h:%p` invocation. Every sibling hop's file path is cleared,
/// leaving only `active_hop_index` available to this helper. Password values
/// never appear on the command line or in the SSH process environment.
#[must_use]
pub fn askpass_proxy_prefix(
    askpass_script: &std::path::Path,
    active_hop_index: usize,
    hop_count: usize,
) -> Vec<String> {
    let mut prefix = vec!["env".to_string(), "_RC_TGT_PW_FILE=".to_string()];
    prefix.extend(
        (0..hop_count)
            .filter(|index| *index != active_hop_index)
            .map(|index| format!("{}=", jump_host_pw_env_name(index))),
    );
    prefix.push(format!("SSH_ASKPASS={}", askpass_script.display()));
    prefix.push("SSH_ASKPASS_REQUIRE=force".to_string());
    prefix
}

/// Returns the env-var name carrying the `hop_index`-th bastion password.
///
/// In a multi-hop chain (issue #203), index 0 uses the legacy `_RC_JH_PW_FILE` name
/// for backward compatibility with single-bastion setups; deeper hops use
/// `_RC_JH_PW_FILE_1`, `_RC_JH_PW_FILE_2`, etc.
#[must_use]
pub fn jump_host_pw_env_name(hop_index: usize) -> String {
    if hop_index == 0 {
        "_RC_JH_PW_FILE".to_string()
    } else {
        format!("_RC_JH_PW_FILE_{hop_index}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_free_port() {
        let port = find_free_port().expect("should find a free port");
        assert!(port > 0);

        // Deliberately not asserting that the port is still bindable. It is a
        // port the kernel had free a moment ago, and nothing holds a claim on it
        // between the two calls — so anything else on the machine, including
        // another test in this binary, can take it first. The test failed that
        // way once and passed on re-run, which is the signature of a race rather
        // than a bug in `find_free_port`. Binding is attempted so a hard failure
        // (a malformed address, no loopback) still shows up.
        match TcpListener::bind(format!("127.0.0.1:{port}")) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                eprintln!("port {port} was taken between the probe and the bind — expected race");
            }
            Err(e) => panic!("binding 127.0.0.1:{port} failed for an unexpected reason: {e}"),
        }
    }

    #[test]
    fn test_find_free_port_unique() {
        let p1 = find_free_port().expect("port 1");
        let p2 = find_free_port().expect("port 2");
        // Ports should be different (extremely likely, not guaranteed)
        // This is a probabilistic test — skip assertion if they happen to match
        if p1 == p2 {
            eprintln!("Warning: two consecutive find_free_port() returned the same port {p1}");
        }
    }

    #[test]
    fn test_proxy_destination_simple_host() {
        let mut parts = Vec::new();
        append_proxy_command_destination(&mut parts, "bastion.example.com");
        assert_eq!(parts, vec!["bastion.example.com"]);
    }

    #[test]
    fn test_proxy_destination_user_at_host() {
        let mut parts = Vec::new();
        append_proxy_command_destination(&mut parts, "admin@bastion.example.com");
        assert_eq!(parts, vec!["admin@bastion.example.com"]);
    }

    #[test]
    fn test_proxy_destination_host_with_port() {
        let mut parts = Vec::new();
        append_proxy_command_destination(&mut parts, "bastion.example.com:2222");
        assert_eq!(parts, vec!["-p", "2222", "bastion.example.com"]);
    }

    #[test]
    fn test_proxy_destination_user_host_port() {
        let mut parts = Vec::new();
        append_proxy_command_destination(&mut parts, "admin@bastion.example.com:2222");
        assert_eq!(parts, vec!["-p", "2222", "admin@bastion.example.com"]);
    }

    #[test]
    fn test_proxy_destination_port_22_omitted() {
        let mut parts = Vec::new();
        append_proxy_command_destination(&mut parts, "admin@bastion.example.com:22");
        assert_eq!(parts, vec!["admin@bastion.example.com"]);
    }

    #[test]
    fn test_proxy_destination_ipv6_with_port() {
        let mut parts = Vec::new();
        append_proxy_command_destination(&mut parts, "user@[::1]:2222");
        assert_eq!(parts, vec!["-p", "2222", "user@[::1]"]);
    }

    #[test]
    fn test_proxy_destination_ipv6_no_port() {
        let mut parts = Vec::new();
        append_proxy_command_destination(&mut parts, "[fe80::1]");
        assert_eq!(parts, vec!["[fe80::1]"]);
    }

    #[test]
    fn test_shell_single_quote_plain() {
        assert_eq!(shell_single_quote("ssh -W %h:%p b"), "'ssh -W %h:%p b'");
    }

    #[test]
    fn test_shell_single_quote_escapes_embedded_quote() {
        // The classic close-quote / escaped-quote / reopen-quote dance, so a
        // nested ProxyCommand survives the `sh -c` re-parse intact.
        assert_eq!(shell_single_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn test_format_argv_display_quotes_only_args_with_spaces() {
        // issue #322: the ProxyCommand value has spaces and must be quoted so
        // the echoed line does not look like several separate ssh arguments.
        let argv = vec![
            "ssh".to_string(),
            "-o".to_string(),
            "ProxyCommand=nc -X 5 -x 127.0.0.1:1080 %h %p".to_string(),
            "user@host".to_string(),
        ];
        assert_eq!(
            format_argv_for_display(&argv),
            "ssh -o 'ProxyCommand=nc -X 5 -x 127.0.0.1:1080 %h %p' user@host"
        );
    }

    #[test]
    fn test_format_argv_display_leaves_bare_words_unquoted() {
        let argv = vec![
            "ssh".to_string(),
            "-p".to_string(),
            "2222".to_string(),
            "-i".to_string(),
            "/home/u/.ssh/id_ed25519".to_string(),
            "admin@host".to_string(),
        ];
        assert_eq!(
            format_argv_for_display(&argv),
            "ssh -p 2222 -i /home/u/.ssh/id_ed25519 admin@host"
        );
    }

    #[test]
    fn test_format_argv_display_quotes_empty_arg() {
        let argv = vec!["ssh".to_string(), String::new(), "host".to_string()];
        assert_eq!(format_argv_for_display(&argv), "ssh '' host");
    }

    #[test]
    fn test_proxy_command_relay_does_not_prompt() {
        // issue #322: a SOCKS5 TCP relay never speaks SSH, so it cannot prompt —
        // the target's own password auto-fill must stay enabled.
        assert!(!proxy_command_can_prompt_for_password(
            "nc -X 5 -x 127.0.0.1:1080 %h %p"
        ));
        assert!(!proxy_command_can_prompt_for_password(
            "ncat --proxy 127.0.0.1:9050 %h %p"
        ));
        assert!(!proxy_command_can_prompt_for_password(
            "socat - PROXY:127.0.0.1:%h:%p"
        ));
        assert!(!proxy_command_can_prompt_for_password(
            "corkscrew p 8080 %h %p"
        ));
        // Absolute path to the relay binary is stripped to its basename.
        assert!(!proxy_command_can_prompt_for_password("/usr/bin/nc %h %p"));
    }

    #[test]
    fn test_proxy_command_ssh_subcommand_stays_conservative() {
        // A relay whose command line contains the word `ssh` (e.g. a
        // `cloudflared access ssh` tunnel) is treated conservatively: the word
        // match cannot tell an ssh *subcommand* from a nested ssh *hop*, and
        // suppressing auto-fill on a false positive only means a manual password
        // entry, never a leak.
        assert!(proxy_command_can_prompt_for_password(
            "cloudflared access ssh --hostname %h"
        ));
    }

    #[test]
    fn test_proxy_command_nested_ssh_stays_conservative() {
        // A hand-written bastion ProxyCommand invokes ssh, which can prompt —
        // auto-fill must stay suppressed (issue #191 leak guard).
        assert!(proxy_command_can_prompt_for_password(
            "ssh -W %h:%p bastion.example.com"
        ));
        assert!(proxy_command_can_prompt_for_password(
            "ssh bastion nc %h %p"
        ));
        // sshpass is credential machinery, not a bare relay — stay conservative.
        assert!(proxy_command_can_prompt_for_password("sshpass -p x ssh %h"));
    }

    #[test]
    fn test_proxy_command_unknown_stays_conservative() {
        // An unrecognised command could prompt; the safe default is to suppress.
        assert!(proxy_command_can_prompt_for_password(
            "my-custom-proxy %h %p"
        ));
        assert!(proxy_command_can_prompt_for_password(""));
    }

    #[test]
    fn test_nested_proxy_single_hop_accept_new() {
        let cmd = build_nested_proxy_command(&["bastion.example.com"], None, None, true);
        assert_eq!(
            cmd,
            "ssh -W %h:%p -o StrictHostKeyChecking=accept-new bastion.example.com"
        );
        // A single hop must not wrap itself in a ProxyCommand.
        assert!(!cmd.contains("ProxyCommand"));
    }

    #[test]
    fn test_nested_proxy_single_hop_identity_and_known_hosts() {
        let cmd = build_nested_proxy_command(
            &["admin@bastion:2222"],
            Some("/home/me/.ssh/id_ed25519"),
            Some(std::path::Path::new("/run/kh")),
            false,
        );
        // accept_new=false must not emit StrictHostKeyChecking.
        assert!(!cmd.contains("StrictHostKeyChecking"));
        assert_eq!(
            cmd,
            "ssh -W %h:%p -o UserKnownHostsFile=/run/kh -i /home/me/.ssh/id_ed25519 \
             -o IdentitiesOnly=yes -p 2222 admin@bastion"
        );
    }

    #[test]
    fn test_nested_proxy_two_hops_nests_inner_command() {
        // `hops[0]` (closest to the target) is the destination of the OUTER ssh;
        // it is reached THROUGH `hops[1]` via the nested, single-quoted
        // ProxyCommand. Reversing this would break double-jump chains (#191).
        let cmd = build_nested_proxy_command(&["near", "far"], None, None, false);
        assert_eq!(cmd, "ssh -W %h:%p -o ProxyCommand='ssh -W %h:%p far' near");
    }

    #[test]
    fn test_nested_proxy_three_hops_orders_target_to_client() {
        // chain[0] reached via chain[1] reached via chain[2]: the innermost
        // command targets the hop closest to the client.
        let cmd = build_nested_proxy_command(&["h0", "h1", "h2"], None, None, false);
        assert_eq!(
            cmd,
            "ssh -W %h:%p -o ProxyCommand='ssh -W %h:%p -o ProxyCommand='\\''ssh -W %h:%p h2'\\'' h1' h0"
        );
    }

    #[test]
    fn test_proxy_jump_arg_single_hop_unchanged() {
        // The common single-bastion case must be a no-op.
        assert_eq!(proxy_jump_arg("bastion.example.com"), "bastion.example.com");
    }

    #[test]
    fn test_proxy_jump_arg_reverses_multi_hop() {
        // Target-first internal order -> client-first OpenSSH order.
        assert_eq!(proxy_jump_arg("near,far"), "far,near");
        assert_eq!(proxy_jump_arg("j0,j1,j2"), "j2,j1,j0");
    }

    #[test]
    fn test_proxy_jump_arg_trims_and_drops_empty() {
        assert_eq!(proxy_jump_arg(" a , , b "), "b,a");
        assert_eq!(proxy_jump_arg(""), "");
    }

    #[test]
    fn test_askpass_proxy_prefix_shape() {
        // Issue #191: the env-assignment prefix carries the askpass wiring, not
        // the password. Lock in the exact tokens and order OpenSSH needs.
        let prefix = askpass_proxy_prefix(
            std::path::Path::new("/run/user/1000/rustconn-jh-askpass.sh"),
            0,
            3,
        );
        assert_eq!(
            prefix,
            vec![
                "env".to_string(),
                "_RC_TGT_PW_FILE=".to_string(),
                "_RC_JH_PW_FILE_1=".to_string(),
                "_RC_JH_PW_FILE_2=".to_string(),
                "SSH_ASKPASS=/run/user/1000/rustconn-jh-askpass.sh".to_string(),
                "SSH_ASKPASS_REQUIRE=force".to_string(),
            ]
        );
    }

    #[test]
    fn test_askpass_disabled_proxy_prefix_clears_target_credential() {
        assert_eq!(
            askpass_disabled_proxy_prefix(3),
            vec![
                "env".to_string(),
                "_RC_TGT_PW_FILE=".to_string(),
                "_RC_JH_PW_FILE=".to_string(),
                "_RC_JH_PW_FILE_1=".to_string(),
                "_RC_JH_PW_FILE_2=".to_string(),
                "SSH_ASKPASS=".to_string(),
                "SSH_ASKPASS_REQUIRE=".to_string(),
            ]
        );
    }

    #[test]
    fn test_target_password_askpass_refuses_only_interactive_second_factors() {
        let config = SshConfig::default();
        assert!(target_password_askpass_allowed(&config, false));

        // An available identity is not a veto: if the key authenticates, the helper is never
        // consulted; if it does not, OpenSSH falls through to the password prompt the helper does
        // answer. Treating this as a refusal is what stopped a group-inherited key path from ever
        // getting its stored password (0.21.2 review).
        assert!(
            target_password_askpass_allowed(&config, true),
            "a resolved identity must not disable automatic delivery"
        );

        // A key or agent key source is likewise not a veto, for the same reason.
        for key_source in [
            crate::models::SshKeySource::File {
                path: std::path::PathBuf::from("/home/me/.ssh/id_ed25519"),
            },
            crate::models::SshKeySource::Agent {
                fingerprint: "SHA256:test".to_string(),
                comment: "test".to_string(),
            },
        ] {
            let config = SshConfig {
                key_source,
                ..SshConfig::default()
            };
            assert!(target_password_askpass_allowed(&config, true));
        }

        // PublicKey and Agent negotiate normally and fall through to a password prompt when the
        // key is rejected, so they stay covered too.
        for auth_method in [SshAuthMethod::PublicKey, SshAuthMethod::Agent] {
            let config = SshConfig {
                auth_method,
                ..SshConfig::default()
            };
            assert!(target_password_askpass_allowed(&config, false));
        }

        // The two that must stay interactive: a forced helper would sit in front of the touch or
        // the challenge that is the whole point of the method.
        for auth_method in [
            SshAuthMethod::SecurityKey,
            SshAuthMethod::KeyboardInteractive,
        ] {
            let config = SshConfig {
                auth_method,
                ..SshConfig::default()
            };
            assert!(!target_password_askpass_allowed(&config, false));
        }
    }

    #[test]
    fn test_target_password_askpass_rejects_unreasonable_launches() {
        // A token's PIN/touch step is the point of the method, so a forced helper must not stand
        // in front of it.
        let token_config = SshConfig {
            pkcs11_provider: Some("/usr/lib/pkcs11.so".to_string()),
            ..SshConfig::default()
        };
        assert!(!target_password_askpass_allowed(&token_config, false));

        let legacy_agent_config = SshConfig {
            agent_key_fingerprint: Some("SHA256:legacy".to_string()),
            ..SshConfig::default()
        };
        assert!(!target_password_askpass_allowed(
            &legacy_agent_config,
            false
        ));

        // An opaque route spawns a nested ssh outside the env-sanitised chain this module builds.
        let proxy_config = SshConfig {
            proxy_command: Some("ncat %h %p".to_string()),
            ..SshConfig::default()
        };
        assert!(has_unmanaged_proxy_route(&proxy_config));
        assert!(!target_password_askpass_allowed(&proxy_config, false));

        // The user has taken authentication over by hand; our own -o would outrank what they wrote.
        let mut override_config = SshConfig::default();
        override_config.custom_options.insert(
            "PreferredAuthentications".to_string(),
            "keyboard-interactive".to_string(),
        );
        assert!(!target_password_askpass_allowed(&override_config, false));

        let mut proxy_jump_override = SshConfig::default();
        proxy_jump_override.custom_options.insert(
            "ProxyJump".to_string(),
            "unmanaged-bastion.example.com".to_string(),
        );
        assert!(has_unmanaged_proxy_route(&proxy_jump_override));
        assert!(!target_password_askpass_allowed(
            &proxy_jump_override,
            false
        ));
    }

    #[test]
    fn test_variable_bastion_proxy_command_uses_askpass() {
        // Issue #191, Req 2.1/2.3: a bastion whose password comes from a Variable
        // (or Vault) source is authenticated OUT-OF-BAND. The assembled first-hop
        // ProxyCommand is prefixed with `env SSH_ASKPASS=... SSH_ASKPASS_REQUIRE=
        // force` and reaches the bastion via `ssh -W %h:%p`, so the target's
        // password is never fed to the bastion prompt. This mirrors the
        // assembly in `protocols_ssh.rs::build_ssh_command_args`.
        let script = std::path::Path::new("/run/user/1000/rustconn-jh-askpass.sh");
        let mut proxy_parts = askpass_proxy_prefix(script, 0, 1);
        proxy_parts.push("ssh".to_string());
        proxy_parts.push("-W".to_string());
        proxy_parts.push("%h:%p".to_string());
        append_proxy_command_destination(&mut proxy_parts, "admin@bastion.example.com:2222");
        let proxy_cmd = proxy_parts.join(" ");

        assert_eq!(
            proxy_cmd,
            "env _RC_TGT_PW_FILE= SSH_ASKPASS=/run/user/1000/rustconn-jh-askpass.sh \
             SSH_ASKPASS_REQUIRE=force ssh -W %h:%p -p 2222 admin@bastion.example.com"
        );
        // The askpass prefix must precede the `ssh` invocation it scopes.
        let askpass_pos = proxy_cmd
            .find("SSH_ASKPASS=")
            .expect("ProxyCommand must carry SSH_ASKPASS");
        let ssh_pos = proxy_cmd
            .find("ssh -W")
            .expect("ProxyCommand must run ssh -W");
        assert!(
            askpass_pos < ssh_pos,
            "askpass env prefix must come before `ssh -W`"
        );
    }

    #[test]
    fn test_jump_host_pw_env_name_indices() {
        // Issue #203: hop 0 keeps the legacy name for single-bastion backward
        // compatibility; deeper hops get an indexed suffix.
        assert_eq!(jump_host_pw_env_name(0), "_RC_JH_PW_FILE");
        assert_eq!(jump_host_pw_env_name(1), "_RC_JH_PW_FILE_1");
        assert_eq!(jump_host_pw_env_name(2), "_RC_JH_PW_FILE_2");
    }

    #[test]
    fn test_nested_askpass_empty_scripts_isolates_every_hop() {
        let cmd =
            build_nested_proxy_command_with_askpass(&["near", "far"], None, None, false, &[], 0, 2);
        assert_eq!(
            cmd,
            "env _RC_TGT_PW_FILE= _RC_JH_PW_FILE= _RC_JH_PW_FILE_1= SSH_ASKPASS= \
             SSH_ASKPASS_REQUIRE= ssh -W %h:%p \
             -o ProxyCommand='env _RC_TGT_PW_FILE= _RC_JH_PW_FILE= _RC_JH_PW_FILE_1= SSH_ASKPASS= \
             SSH_ASKPASS_REQUIRE= ssh -W %h:%p far' near"
        );
    }

    #[test]
    fn test_nested_askpass_wires_outer_hop() {
        // Issue #203: a single askpass script for hops[0] prefixes the OUTER ssh
        // with the env-assignment wiring, without touching the inner hop.
        let script = std::path::Path::new("/run/user/1000/rustconn-jh-askpass.sh");
        let cmd = build_nested_proxy_command_with_askpass(
            &["near", "far"],
            None,
            None,
            false,
            &[Some(script), None],
            0,
            2,
        );
        // hops[0]=near uses forced askpass → must get accept-new so an unknown
        // host key never routes the yes/no prompt to the password helper (#203).
        // far has no askpass and accept_new=false → no StrictHostKeyChecking.
        assert_eq!(
            cmd,
            "env _RC_TGT_PW_FILE= _RC_JH_PW_FILE_1= \
             SSH_ASKPASS=/run/user/1000/rustconn-jh-askpass.sh \
             SSH_ASKPASS_REQUIRE=force ssh -W %h:%p -o StrictHostKeyChecking=accept-new \
             -o ProxyCommand='env _RC_TGT_PW_FILE= _RC_JH_PW_FILE= _RC_JH_PW_FILE_1= SSH_ASKPASS= \
             SSH_ASKPASS_REQUIRE= ssh -W %h:%p far' near"
        );
    }

    #[test]
    fn test_nested_askpass_wires_inner_hop() {
        // The deeper hop's askpass must land inside the nested ProxyCommand only,
        // so JUMP1 (the entry bastion) authenticates with its own password
        // (issue #203: only-one-bastion-gets-a-password regression).
        let inner = std::path::Path::new("/run/user/1000/rustconn-jh-askpass-1.sh");
        let cmd = build_nested_proxy_command_with_askpass(
            &["near", "far"],
            None,
            None,
            false,
            &[None, Some(inner)],
            0,
            2,
        );
        // near has no askpass → no accept-new; far (inner) uses forced askpass
        // → gets accept-new inside the nested ProxyCommand (#203).
        assert_eq!(
            cmd,
            "env _RC_TGT_PW_FILE= _RC_JH_PW_FILE= _RC_JH_PW_FILE_1= SSH_ASKPASS= \
             SSH_ASKPASS_REQUIRE= ssh -W %h:%p \
             -o ProxyCommand='env _RC_TGT_PW_FILE= _RC_JH_PW_FILE= \
             SSH_ASKPASS=/run/user/1000/rustconn-jh-askpass-1.sh \
             SSH_ASKPASS_REQUIRE=force ssh -W %h:%p -o StrictHostKeyChecking=accept-new far' near"
        );
    }
}

#[cfg(test)]
mod proxy_detection_tests {
    use super::ssh_g_output_declares_proxy;

    #[test]
    fn ssh_g_reports_no_proxy_when_the_keywords_are_absent() {
        // `ssh -G` omits both keywords entirely for a host with no routing, which is the common
        // case and the one that must enable automatic delivery.
        let output = "user me\nhostname target.example.com\nport 22\naddressfamily any\n";
        assert!(!ssh_g_output_declares_proxy(output));
    }

    #[test]
    fn ssh_g_reports_a_proxy_for_proxyjump_and_proxycommand() {
        assert!(ssh_g_output_declares_proxy(
            "hostname target.example.com\nproxyjump bastion.example.com\n"
        ));
        assert!(ssh_g_output_declares_proxy(
            "hostname target.example.com\nproxycommand ncat %h %p\n"
        ));
    }

    #[test]
    fn ssh_g_treats_none_as_no_proxy() {
        // `none` is how OpenSSH itself spells "disabled", so it must not read as a route.
        assert!(!ssh_g_output_declares_proxy("proxyjump none\n"));
        assert!(!ssh_g_output_declares_proxy("proxycommand none\n"));
        // A bare keyword with no value is not a route either.
        assert!(!ssh_g_output_declares_proxy("proxycommand\n"));
    }

    #[test]
    fn ssh_g_parsing_ignores_unrelated_keywords_that_merely_contain_the_name() {
        // Prefix matching would fire on this; the parser compares the whole first field.
        assert!(!ssh_g_output_declares_proxy(
            "proxyusepdisc no\nproxyjumpsomething value\n"
        ));
    }
}
