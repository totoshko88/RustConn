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

    let mut cmd = Command::new("ssh");
    cmd.arg("-N") // No remote command — just forward
        .arg("-L")
        .arg(&forward_spec);

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
        remote = %format!("{}:{}", params.remote_host, params.remote_port),
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
    build_nested_proxy_command_with_askpass(
        hops,
        identity_file,
        known_hosts,
        accept_new_host_keys,
        &[],
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
///
/// The env-var carrying each hop's password is `_RC_JH_PW_<depth>` where
/// `depth` is the hop's absolute index in the original chain (passed through
/// from the caller). The scripts themselves only contain the var name — the
/// password value rides in the spawned process's environment.
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
) -> String {
    debug_assert!(!hops.is_empty(), "build_nested_proxy_command needs >=1 hop");

    let mut parts: Vec<String> = Vec::new();

    // Askpass prefix for THIS hop (hops[0]).
    if let Some(Some(script)) = askpass_scripts.first() {
        parts.extend(askpass_proxy_prefix(script));
    }

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
        let inner = build_nested_proxy_command_with_askpass(
            &hops[1..],
            identity_file,
            known_hosts,
            accept_new_host_keys,
            inner_askpass,
        );
        parts.push("-o".to_string());
        parts.push(format!("ProxyCommand={}", shell_single_quote(&inner)));
    }

    append_proxy_command_destination(&mut parts, hops[0]);
    parts.join(" ")
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
/// Returns the prefix tokens (`["env", "SSH_ASKPASS=<script>",
/// "SSH_ASKPASS_REQUIRE=force"]`) to prepend to the bastion `ssh -W %h:%p`
/// invocation. The password VALUE itself is delivered through a separate
/// out-of-band environment variable read by the helper script; it never appears
/// on the command line.
#[must_use]
pub fn askpass_proxy_prefix(askpass_script: &std::path::Path) -> Vec<String> {
    vec![
        "env".to_string(),
        format!("SSH_ASKPASS={}", askpass_script.display()),
        "SSH_ASKPASS_REQUIRE=force".to_string(),
    ]
}

/// Returns the env-var name carrying the `hop_index`-th bastion password.
///
/// In a multi-hop chain (issue #203), index 0 uses the legacy `_RC_JH_PW` name
/// for backward compatibility with single-bastion setups; deeper hops use
/// `_RC_JH_PW_1`, `_RC_JH_PW_2`, etc.
#[must_use]
pub fn jump_host_pw_env_name(hop_index: usize) -> String {
    if hop_index == 0 {
        "_RC_JH_PW".to_string()
    } else {
        format!("_RC_JH_PW_{hop_index}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_free_port() {
        let port = find_free_port().expect("should find a free port");
        assert!(port > 0);
        // Verify the port is actually free by binding to it
        let listener = TcpListener::bind(format!("127.0.0.1:{port}"));
        assert!(listener.is_ok(), "port {port} should be bindable");
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
        let prefix = askpass_proxy_prefix(std::path::Path::new(
            "/run/user/1000/rustconn-jh-askpass.sh",
        ));
        assert_eq!(
            prefix,
            vec![
                "env".to_string(),
                "SSH_ASKPASS=/run/user/1000/rustconn-jh-askpass.sh".to_string(),
                "SSH_ASKPASS_REQUIRE=force".to_string(),
            ]
        );
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
        let mut proxy_parts = askpass_proxy_prefix(script);
        proxy_parts.push("ssh".to_string());
        proxy_parts.push("-W".to_string());
        proxy_parts.push("%h:%p".to_string());
        append_proxy_command_destination(&mut proxy_parts, "admin@bastion.example.com:2222");
        let proxy_cmd = proxy_parts.join(" ");

        assert_eq!(
            proxy_cmd,
            "env SSH_ASKPASS=/run/user/1000/rustconn-jh-askpass.sh \
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
        assert_eq!(jump_host_pw_env_name(0), "_RC_JH_PW");
        assert_eq!(jump_host_pw_env_name(1), "_RC_JH_PW_1");
        assert_eq!(jump_host_pw_env_name(2), "_RC_JH_PW_2");
    }

    #[test]
    fn test_nested_askpass_empty_scripts_matches_plain() {
        // With no askpass scripts, the with_askpass variant must be byte-for-byte
        // identical to the plain builder (backward compatibility).
        let plain = build_nested_proxy_command(&["near", "far"], None, None, false);
        let with =
            build_nested_proxy_command_with_askpass(&["near", "far"], None, None, false, &[]);
        assert_eq!(plain, with);
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
        );
        // hops[0]=near uses forced askpass → must get accept-new so an unknown
        // host key never routes the yes/no prompt to the password helper (#203).
        // far has no askpass and accept_new=false → no StrictHostKeyChecking.
        assert_eq!(
            cmd,
            "env SSH_ASKPASS=/run/user/1000/rustconn-jh-askpass.sh SSH_ASKPASS_REQUIRE=force \
             ssh -W %h:%p -o StrictHostKeyChecking=accept-new \
             -o ProxyCommand='ssh -W %h:%p far' near"
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
        );
        // near has no askpass → no accept-new; far (inner) uses forced askpass
        // → gets accept-new inside the nested ProxyCommand (#203).
        assert_eq!(
            cmd,
            "ssh -W %h:%p -o ProxyCommand='env SSH_ASKPASS=/run/user/1000/rustconn-jh-askpass-1.sh \
             SSH_ASKPASS_REQUIRE=force ssh -W %h:%p -o StrictHostKeyChecking=accept-new far' near"
        );
    }
}
