//! SSH tunnel for forwarding connections through a jump host.
//!
//! Used by RDP, VNC, SPICE, and Telnet connections that have a
//! `jump_host_id` configured. Creates an `ssh -L` local port forward
//! in the background and returns the local port for the client to
//! connect to. The local end is bound to `127.0.0.1` explicitly, so
//! `GatewayPorts yes` cannot expose it on other interfaces.

use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use secrecy::SecretString;
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
    /// A Tailscale SSH host or user name that could be read as an ssh option or
    /// a shell word. Rejected before anything is spawned.
    #[error(
        "Refusing a Tailscale SSH host or user name that is not a plain name (letters, digits, '.', '_', '-', ':')"
    )]
    UnsafeDestination,
    /// The connection's type cannot host a SOCKS tunnel (see
    /// [`supports_browser_tunnel`]).
    #[error("This connection type cannot host a SOCKS tunnel")]
    UnsupportedConnection,
}

/// Result type for SSH tunnel operations.
pub type SshTunnelResult<T> = Result<T, SshTunnelError>;

/// A running SSH tunnel (`ssh -N -L ...`).
///
/// The tunnel process is killed when this struct is dropped.
/// If a temporary askpass script or secret file was created, it is zeroized
/// and deleted.
pub struct SshTunnel {
    /// The child SSH process.
    child: Child,
    /// The local port that forwards to the remote destination.
    local_port: u16,
    /// Captured stderr output from the SSH process (populated by background reader).
    stderr_output: Arc<Mutex<String>>,
    /// Path to the temporary askpass script (cleaned up on drop).
    askpass_script: Option<std::path::PathBuf>,
    /// Path to the owner-only file holding the jump-host password (cleaned up
    /// on drop). The password lives here, not in the child's environment; the
    /// env carries only this path, matching the multi-hop askpass convention.
    secret_file: Option<std::path::PathBuf>,
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
        if let Some(ref path) = self.secret_file {
            // Same zeroize-then-unlink the askpass script gets: the password
            // byte s lived here in plaintext.
            cleanup_askpass_script(path);
        }
    }
}

/// Which client starts the tunnel process.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TunnelLauncher {
    /// `ssh … <jump_host>` — the destination goes last.
    #[default]
    OpenSsh,
    /// `tailscale ssh <jump_host> …` — the same wrapper the Tailscale SSH
    /// session runs, so auth, ACLs and host-key checks match it. The
    /// destination goes first (tailscale reads `args[0]` as the host and
    /// passes the rest to the system `ssh`, which parses options after the
    /// destination). `jump_port` and the Flatpak known_hosts are not used.
    TailscaleSsh,
}

/// How a SOCKS browser tunnel reaches a connection, or `None` from
/// [`socks_tunnel_route`] when the connection type cannot host one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocksTunnelRoute {
    /// Plain OpenSSH to the connection's own host.
    OpenSsh,
    /// `tailscale ssh <destination>`.
    TailscaleSsh {
        /// The `[user@]host` the Tailscale SSH session itself is given.
        destination: String,
    },
}

/// How a SOCKS browser tunnel reaches `conn`, or `None` when the connection
/// type cannot host one.
///
/// The single gate behind "Open Browser via Tunnel" (issue #397): SSH/SFTP go
/// through OpenSSH, Tailscale SSH through `tailscale ssh`; every other Zero
/// Trust provider stays unsupported.
///
/// A Zero Trust connection is accepted only when its `provider` and its
/// `provider_config` both say Tailscale SSH. The two can disagree on imported or
/// hand-edited data; the session itself runs from `provider_config`, so trusting
/// either field alone would offer a tunnel the other part of the code refuses.
#[must_use]
pub fn socks_tunnel_route(conn: &crate::models::Connection) -> Option<SocksTunnelRoute> {
    use crate::models::{ProtocolConfig, ZeroTrustProvider, ZeroTrustProviderConfig};

    match &conn.protocol_config {
        ProtocolConfig::Ssh(_) | ProtocolConfig::Sftp(_) => Some(SocksTunnelRoute::OpenSsh),
        ProtocolConfig::ZeroTrust(zt) => match (zt.provider, &zt.provider_config) {
            (ZeroTrustProvider::TailscaleSsh, ZeroTrustProviderConfig::TailscaleSsh(cfg)) => {
                Some(SocksTunnelRoute::TailscaleSsh {
                    destination: cfg.destination(conn.username.as_deref()),
                })
            }
            _ => None,
        },
        _ => None,
    }
}

/// Whether "Open Browser via Tunnel" can work for `conn`.
///
/// The one question both the sidebar menu and the action handler ask, so the
/// item is shown exactly when [`socks_tunnel_route`] has a route.
#[must_use]
pub fn supports_browser_tunnel(conn: &crate::models::Connection) -> bool {
    socks_tunnel_route(conn).is_some()
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
    /// Which client starts the tunnel (OpenSSH by default).
    pub launcher: TunnelLauncher,
}

/// Environment variable name carrying the *path* to the owner-only file that
/// holds the jump-host password — never the password itself.
///
/// This matches the multi-hop askpass convention (`_RC_JH_PW_FILE`, issue
/// #203): the env is visible in `/proc/<pid>/environ` to the same user, so it
/// must not carry a secret value. The helper script opens the file this names
/// and prints its contents, so the plaintext stays in an owner-only file the
/// tunnel removes on drop, not in the SSH process environment.
pub(crate) const TUNNEL_ASKPASS_ENV_VAR: &str = "_RC_TUN_PW_FILE";

/// Creates a temporary `SSH_ASKPASS` helper script that prints the contents of
/// the file named by [`TUNNEL_ASKPASS_ENV_VAR`]. The script is created with
/// mode 0700.
///
/// The script reads the secret from a file rather than straight from the
/// environment so the password value never enters `/proc/<pid>/environ`; the
/// env carries only the file path. It deliberately does not unlink the file —
/// cleanup is owned by [`SshTunnel`]'s `Drop` — so a second askpass invocation
/// (OpenSSH may call the helper more than once) still finds it.
///
/// # Errors
///
/// Returns a human-readable error string on failure.
pub(crate) fn create_tunnel_askpass_script() -> Result<std::path::PathBuf, String> {
    // Read the secret from the owner-only file whose path the env var carries,
    // rather than echoing the env value directly — the env is visible in
    // `/proc/<pid>/environ`, the file is not. Mirrors the target/jump-host
    // helper in `crate::ssh_askpass::script_contents`.
    write_askpass_script(&format!(
        "#!/bin/sh\n\
         secret_file=\"${{{TUNNEL_ASKPASS_ENV_VAR}}}\"\n\
         [ -n \"$secret_file\" ] || exit 1\n\
         exec 3<\"$secret_file\" || exit 1\n\
         cat <&3\n"
    ))
}

/// Like [`create_tunnel_askpass_script`], but the helper answers once.
///
/// It prints the secret file and then empties it, so a second invocation finds
/// an empty file and exits 1 without printing anything. For `ssh-add`, which
/// re-asks after a wrong passphrase until askpass gives it an empty answer: the
/// tunnel helper prints the same file every time, so with it a wrong passphrase
/// made `ssh-add` loop forever. Emptying the file also takes the plaintext off
/// it as soon as it has been read; the caller still removes the file.
///
/// # Errors
///
/// Returns a human-readable error string on failure.
pub(crate) fn create_one_shot_askpass_script() -> Result<std::path::PathBuf, String> {
    write_askpass_script(&format!(
        "#!/bin/sh\n\
         secret_file=\"${{{TUNNEL_ASKPASS_ENV_VAR}}}\"\n\
         [ -n \"$secret_file\" ] || exit 1\n\
         [ -s \"$secret_file\" ] || exit 1\n\
         exec 3<\"$secret_file\" || exit 1\n\
         cat <&3\n\
         : > \"$secret_file\"\n"
    ))
}

/// Writes `script` to a fresh mode-0700 file in the temp dir and returns its
/// path. The script holds no secret, only the name of the env var to read.
///
/// The file is created with `create_new` (O_EXCL) and, on Unix, mode 0700 set
/// at open time, so it never exists world-readable for a window and a symlink
/// at the random path cannot redirect the write (SEC-4 hardening).
pub(crate) fn write_askpass_script(script: &str) -> Result<std::path::PathBuf, String> {
    use std::io::Write;

    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "rc-tun-askpass-{}",
        uuid::Uuid::new_v4().as_hyphenated()
    ));

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o700);
    }
    let mut file = options
        .open(&path)
        .map_err(|e| format!("Failed to create tunnel askpass script: {e}"))?;
    file.write_all(script.as_bytes())
        .map_err(|e| format!("Failed to write tunnel askpass script: {e}"))?;

    Ok(path)
}

/// Writes `password` to a fresh owner-only (mode 0600) file and returns its path.
///
/// The file lives in `$XDG_RUNTIME_DIR` (tmpfs) when available, falling back to
/// the system temp dir only when no runtime dir exists — it is written by the
/// same function as the GUI's askpass secret files
/// ([`crate::ssh_askpass`]). The caller owns the path and must remove it
/// (`SshTunnel`'s `Drop` does, zeroizing first). The password reaches the SSH
/// askpass helper through this file, never through the process environment.
///
/// # Errors
///
/// Returns a human-readable error string if the file cannot be created or written.
pub(crate) fn create_tunnel_secret_file(
    password: &SecretString,
) -> Result<std::path::PathBuf, String> {
    use crate::ssh_askpass::AskpassError;

    crate::ssh_askpass::create_prefixed_secret_file("rc-tun-secret-", password).map_err(|error| {
        match error {
            AskpassError::CreateSecretFile(e) => {
                format!("Failed to create tunnel secret file: {e}")
            }
            AskpassError::WriteSecretFile(e) => format!("Failed to write tunnel secret file: {e}"),
            other => other.to_string(),
        }
    })
}

/// Cleans up a temporary askpass script or secret file, zeroizing its content
/// first.
pub(crate) fn cleanup_askpass_script(path: &std::path::Path) {
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

/// Creates an SSH tunnel by spawning `ssh -N -L 127.0.0.1:local_port:remote:remote_port`.
///
/// The local end listens on IPv4 loopback only, whatever `GatewayPorts` says
/// (see [`loopback_bind_spec`]); every consumer dials `127.0.0.1`.
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
    let [flag, spec] = local_forward_args(local_port, &params.remote_host, params.remote_port);
    let remote_desc = format!("{}:{}", params.remote_host, params.remote_port);
    spawn_tunnel(params, local_port, &[&flag, &spec], &remote_desc)
}

/// The local bind (`127.0.0.1:<port>`) of a `-D` or `-L` forward RustConn creates itself.
///
/// A bare `-D <port>` or `-L <port>:host:port` follows the user's `GatewayPorts`
/// setting, so with `GatewayPorts yes` in `~/.ssh/config` the forwarded port would
/// listen on every interface and anyone on the network could use the tunnel. An
/// explicit bind address overrides that setting. It is the literal `127.0.0.1`,
/// not `localhost`, so it is the address [`wait_for_tunnel_ready`] probes and the
/// clients are pointed at.
///
/// Forwards the user writes themselves (`PortForward`) are not routed through
/// here on purpose: they keep following `GatewayPorts`.
fn loopback_bind_spec(port: u16) -> String {
    format!("127.0.0.1:{port}")
}

/// The forwarding flag pair [`create_socks_tunnel`] hands to [`spawn_tunnel`].
///
/// One helper for the spawn and for the tests that pin its shape, so a change
/// back to a bare `-D <port>` is seen by the tests instead of only by a reviewer.
fn socks_forward_args(port: u16) -> [String; 2] {
    ["-D".to_string(), loopback_bind_spec(port)]
}

/// The forwarding flag pair [`create_tunnel`] hands to [`spawn_tunnel`].
///
/// Same reasoning as [`socks_forward_args`]: pinned by tests so a change back to
/// a bare `-L <port>:host:port` is caught.
fn local_forward_args(local_port: u16, remote_host: &str, remote_port: u16) -> [String; 2] {
    [
        "-L".to_string(),
        format!(
            "{}:{remote_host}:{remote_port}",
            loopback_bind_spec(local_port)
        ),
    ]
}

/// Creates a dynamic SOCKS tunnel by spawning `ssh -N -D 127.0.0.1:<local_port> <jump_host>`.
///
/// Unlike [`create_tunnel`], which forwards a single destination, this opens a
/// local SOCKS5 proxy on a free port, bound to loopback only whatever
/// `GatewayPorts` says: any client pointed at
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
    let [flag, spec] = socks_forward_args(local_port);
    spawn_tunnel(params, local_port, &[&flag, &spec], "SOCKS proxy")
}

/// Checks the `[user@]host` destination handed to `tailscale ssh`.
///
/// `tailscale ssh` places our forwarding flags after the destination and
/// without a `--`, so a host or user starting with `-` would be read by the
/// system `ssh` as an option (`-oProxyCommand=…` runs a local command). A host
/// tailscale cannot resolve is passed through to `ProxyCommand … nc %h %p`,
/// which runs under `sh -c`, so shell metacharacters matter too. Only plain
/// names pass: user `[A-Za-z0-9._-]`, host `[A-Za-z0-9._:-]` (names, IPv4 and
/// IPv6 literals), neither empty, neither starting with `-`, at most one `@`.
/// The value is deliberately not echoed in the error.
fn validate_tailscale_destination(destination: &str) -> SshTunnelResult<()> {
    let (user, host) = match destination.split_once('@') {
        Some((user, host)) => (Some(user), host),
        None => (None, destination),
    };
    let plain = |part: &str, extra: &[char]| {
        !part.is_empty()
            && !part.starts_with('-')
            && part.chars().all(|c| {
                c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') || extra.contains(&c)
            })
    };
    if user.is_none_or(|u| plain(u, &[])) && plain(host, &[':']) {
        Ok(())
    } else {
        Err(SshTunnelError::UnsafeDestination)
    }
}

/// The program and arguments [`spawn_tunnel`] runs. Pure apart from the
/// Flatpak known_hosts lookup, so the argv shape is unit-testable.
///
/// `uses_askpass` is true when a password helper is wired up; otherwise
/// `BatchMode=yes` keeps `ssh` from waiting on a prompt nobody can answer.
fn tunnel_command_line(
    params: &SshTunnelParams,
    forward_args: &[&str; 2],
    uses_askpass: bool,
) -> (&'static str, Vec<String>) {
    let tailscale = params.launcher == TunnelLauncher::TailscaleSsh;
    let mut args: Vec<String> = Vec::new();

    // `tailscale ssh` reads args[0] as the host; the rest goes to `ssh`.
    if tailscale {
        args.push("ssh".to_string());
        args.push(params.jump_host.clone());
    }

    // No remote command — just forward
    args.extend([
        "-N".to_string(),
        forward_args[0].to_string(),
        forward_args[1].to_string(),
    ]);

    // Jump host port. `tailscale ssh` takes none: the session passes none either.
    if !tailscale && params.jump_port != 22 {
        args.push("-p".to_string());
        args.push(params.jump_port.to_string());
    }

    if let Some(ref key) = params.identity_file {
        args.push("-i".to_string());
        args.push(key.clone());
    }

    args.extend(params.extra_args.iter().cloned());

    // Bound the initial connect so a dead jump host fails in seconds instead of
    // hanging on the OS TCP timeout. Without this a SOCKS/Web tunnel (or an
    // RDP/VNC/SPICE `-L` tunnel) to an unreachable bastion left `ssh` blocked
    // for minutes.
    //
    // The value MUST stay below the readiness poll's own budget, or it never
    // helps: the Web/SOCKS callers poll `wait_for_tunnel_ready` 40 × 250 ms =
    // 10 s and then fail with "not ready after 40 attempts". If `ssh` only gives
    // up *after* that — which a 15 s ConnectTimeout did — the poll returns its
    // generic timeout while the process is still alive, so the liveness check
    // never fires and the real reason (the bastion is down) is lost. Eight
    // seconds is the value the SFTP
    // reachability probe already uses for the same reason: `ssh` dies at ~8 s,
    // the next poll attempt sees `is_alive() == false`, and the failure is
    // reported at once carrying the bastion's own stderr. Skipped if the caller
    // already supplied a ConnectTimeout via `extra_args`.
    let has_connect_timeout = params
        .extra_args
        .iter()
        .any(|a| a.eq_ignore_ascii_case("ConnectTimeout") || a.starts_with("ConnectTimeout="));
    if !has_connect_timeout {
        args.push("-o".to_string());
        args.push("ConnectTimeout=8".to_string());
    }

    // Flatpak writable known_hosts. Not for Tailscale: it sets its own
    // UserKnownHostsFile first, and the first value wins in OpenSSH.
    if !tailscale && let Some(kh_path) = crate::get_flatpak_known_hosts_path() {
        args.push("-o".to_string());
        args.push(format!("UserKnownHostsFile={}", kh_path.display()));
    }

    // No password helper — prevent SSH from reading stdin
    if !uses_askpass {
        args.push("-o".to_string());
        args.push("BatchMode=yes".to_string());
    }

    // Exit if the forwarding fails (e.g. port already in use)
    args.push("-o".to_string());
    args.push("ExitOnForwardFailure=yes".to_string());

    // The jump host destination goes last for OpenSSH (first for tailscale, above)
    if !tailscale {
        args.push(params.jump_host.clone());
    }

    (if tailscale { "tailscale" } else { "ssh" }, args)
}

/// Shared spawn path for [`create_tunnel`] and [`create_socks_tunnel`].
///
/// `forward_args` is the forwarding flag and its value (`["-L", spec]` or
/// `["-D", "127.0.0.1:port"]`); `remote_desc` is a human-readable destination for the log.
/// Everything else — jump-host port, identity, askpass, Flatpak known_hosts,
/// `ExitOnForwardFailure`, stderr capture — is identical between the two.
fn spawn_tunnel(
    params: &SshTunnelParams,
    local_port: u16,
    forward_args: &[&str; 2],
    remote_desc: &str,
) -> SshTunnelResult<SshTunnel> {
    // Before any askpass file exists, so a refusal leaves nothing behind.
    if params.launcher == TunnelLauncher::TailscaleSsh {
        validate_tailscale_destination(&params.jump_host)?;
    }

    // SSH_ASKPASS for password-authenticated jump hosts, or BatchMode
    // when no password is available (prevents SSH from hanging on a
    // TTY prompt that nobody can answer).
    // Writes the password to an owner-only file and passes its *path* in the
    // environment, so the secret never lands in `/proc/<pid>/environ`. Both the
    // script and the secret file are torn down by `SshTunnel`'s `Drop`.
    let (askpass_script_path, secret_file_path) = if let Some(ref pw) = params.password {
        match (
            create_tunnel_askpass_script(),
            create_tunnel_secret_file(pw),
        ) {
            (Ok(script_path), Ok(secret_path)) => (Some(script_path), Some(secret_path)),
            (script_result, secret_result) => {
                // Either helper failed. Clean up whichever one succeeded so a
                // stray script or plaintext file is not left behind, then fall
                // back to BatchMode (password auth will fail, but nothing hangs).
                if let Ok(ref path) = script_result {
                    cleanup_askpass_script(path);
                }
                if let Ok(ref path) = secret_result {
                    cleanup_askpass_script(path);
                }
                let error = script_result
                    .err()
                    .or_else(|| secret_result.err())
                    .unwrap_or_default();
                tracing::error!(
                    %error,
                    "Failed to set up SSH_ASKPASS for tunnel; \
                     falling back to BatchMode (password auth will fail)"
                );
                (None, None)
            }
        }
    } else {
        (None, None)
    };

    let (program, args) = tunnel_command_line(params, forward_args, askpass_script_path.is_some());
    let mut cmd = match params.launcher {
        TunnelLauncher::OpenSsh => Command::new(program),
        TunnelLauncher::TailscaleSsh => {
            // Sandbox-aware lookup (Flatpak CLI dir, Snap); tailscale then needs
            // the same extended PATH to find `ssh` and, for its ProxyCommand,
            // itself.
            let mut cmd = Command::new(
                crate::which::find_in_path(program)
                    .unwrap_or_else(|| std::path::PathBuf::from(program)),
            );
            cmd.env("PATH", crate::cli_download::get_extended_path());
            cmd
        }
    };
    cmd.args(&args);

    if let (Some(script_path), Some(secret_path)) = (&askpass_script_path, &secret_file_path) {
        cmd.env("SSH_ASKPASS", script_path);
        cmd.env("SSH_ASKPASS_REQUIRE", "force");
        cmd.env(TUNNEL_ASKPASS_ENV_VAR, secret_path);
        // Ensure DISPLAY is set so SSH considers ASKPASS
        if std::env::var("DISPLAY").is_err() {
            cmd.env("DISPLAY", "");
        }
    }

    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    // Log the assembled flags (not the env — the askpass password lives there).
    // Without this the "not ready" failures could not be told apart from a
    // missing ConnectTimeout, which cost a round of diagnosis.
    tracing::info!(
        local_port,
        remote = %remote_desc,
        jump_host = %params.jump_host,
        program,
        args = ?args,
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
        secret_file: secret_file_path,
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

/// `ProxyCommand` of the [moul/quicssh](https://github.com/moul/quicssh)
/// tunnel (SSH over QUIC), offered as a preset on the editor's ProxyCommand
/// row.
///
/// `%h` and `%p` are expanded by OpenSSH itself; the command is a pure byte
/// relay, so `proxy_command_can_prompt_for_password` keeps target password
/// auto-fill enabled for it.
pub const QUIC_SSH_PROXY_COMMAND: &str = "quicssh client --addr %h:%p";

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
        "quicssh",
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
    let mut prefix = vec![
        "env".to_string(),
        format!("{}=", crate::ssh_askpass::TARGET_PASSWORD_ENV),
    ];
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
    let mut prefix = vec![
        "env".to_string(),
        format!("{}=", crate::ssh_askpass::TARGET_PASSWORD_ENV),
    ];
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
        // The quicssh preset (SSH over QUIC, moul/quicssh) is a pure byte
        // relay: its server has no auth of its own, so target auto-fill
        // stays enabled for connections using the preset.
        assert!(!proxy_command_can_prompt_for_password(
            "quicssh client --addr %h:%p"
        ));
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

    /// The single-hop tunnel askpass helper must read the password from the
    /// file named by its env var, not carry the secret in the environment
    /// (issue: align single-hop with the multi-hop file-based convention).
    #[test]
    fn tunnel_askpass_script_reads_secret_from_a_file_not_the_env() {
        let script_path = create_tunnel_askpass_script().expect("askpass script must be creatable");
        let body = std::fs::read_to_string(&script_path).expect("script readable");
        cleanup_askpass_script(&script_path);

        // The env var carries a file path; the script dereferences it.
        assert!(
            body.contains(TUNNEL_ASKPASS_ENV_VAR),
            "script must reference the path env var: {body}"
        );
        assert!(
            body.contains("secret_file=") && body.contains("cat <&3"),
            "script must read the secret from the file it names, not echo the env: {body}"
        );
        // The old implementation printed the env value directly; guard against
        // a regression to that shape.
        assert!(
            !body.contains("echo \"$_RC_TUN_PW\""),
            "script must not echo the password straight from the environment: {body}"
        );
    }

    /// `create_tunnel_secret_file` writes the password to an owner-only file;
    /// the password value must live there, never in the env the child sees.
    #[cfg(unix)]
    #[test]
    fn tunnel_secret_file_is_owner_only_and_holds_the_password() {
        use std::os::unix::fs::PermissionsExt;

        let password = secrecy::SecretString::from("hunter2-tunnel-fsecret");
        let path = create_tunnel_secret_file(&password).expect("secret file must be creatable");
        let mode = std::fs::metadata(&path)
            .expect("file was just created")
            .permissions()
            .mode()
            & 0o777;
        let body = std::fs::read_to_string(&path).expect("secret file readable");
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("secret file name is UTF-8")
            .to_string();
        cleanup_askpass_script(&path);

        assert!(
            name.starts_with("rc-tun-secret-"),
            "tunnel secret files keep their prefix: {name}"
        );
        assert_eq!(
            mode, 0o600,
            "secret file must be readable only by its owner"
        );
        assert_eq!(
            body, "hunter2-tunnel-fsecret",
            "the secret file holds exactly the password bytes"
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

    /// `SshTunnelParams` derives `Debug` while holding a jump-host password
    /// (`SecretString`); the rendered Debug must never contain the plaintext
    /// (M-PUBLIC-DEBUG). `secrecy` redacts itself today, but this guards a
    /// future field or a switch to a manual impl from silently leaking.
    #[test]
    fn ssh_tunnel_params_debug_does_not_leak_password() {
        let params = crate::ssh_tunnel::SshTunnelParams {
            jump_host: "user@bastion.example.com".to_string(),
            jump_port: 22,
            remote_host: "10.0.0.5".to_string(),
            remote_port: 3389,
            identity_file: None,
            password: Some(secrecy::SecretString::from("hunter2-tunnel-secret")),
            extra_args: Vec::new(),
            launcher: crate::ssh_tunnel::TunnelLauncher::OpenSsh,
        };
        let rendered = format!("{params:?}");
        assert!(
            !rendered.contains("hunter2-tunnel-secret"),
            "Debug output must not contain the plaintext jump-host password: {rendered}"
        );
    }
}

#[cfg(test)]
mod tailscale_tunnel_tests {
    use super::*;
    use crate::models::{
        Connection, ProtocolConfig, TailscaleSshConfig, ZeroTrustConfig, ZeroTrustProvider,
        ZeroTrustProviderConfig,
    };

    fn params(launcher: TunnelLauncher, jump_host: &str) -> SshTunnelParams {
        SshTunnelParams {
            jump_host: jump_host.to_string(),
            jump_port: 22,
            remote_host: String::new(),
            remote_port: 0,
            identity_file: None,
            password: None,
            extra_args: Vec::new(),
            launcher,
        }
    }

    /// The command line `create_socks_tunnel` would run for `port`, built from
    /// the same forwarding pair it passes to `spawn_tunnel`.
    fn socks_command_line(
        p: &SshTunnelParams,
        port: u16,
        uses_askpass: bool,
    ) -> (&'static str, Vec<String>) {
        let [flag, spec] = socks_forward_args(port);
        tunnel_command_line(p, &[&flag, &spec], uses_askpass)
    }

    /// The command line `create_tunnel` would run for `port`, built from the
    /// same forwarding pair it passes to `spawn_tunnel`.
    fn local_forward_command_line(
        p: &SshTunnelParams,
        port: u16,
        uses_askpass: bool,
    ) -> (&'static str, Vec<String>) {
        let [flag, spec] = local_forward_args(port, &p.remote_host, p.remote_port);
        tunnel_command_line(p, &[&flag, &spec], uses_askpass)
    }

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(ToString::to_string).collect()
    }

    fn tailscale_conn(
        conn_user: Option<&str>,
        cfg_user: Option<&str>,
        custom_args: &[&str],
    ) -> Connection {
        let mut conn = Connection::new(
            "ts".to_string(),
            String::new(),
            0,
            ProtocolConfig::ZeroTrust(ZeroTrustConfig {
                provider: ZeroTrustProvider::TailscaleSsh,
                provider_config: ZeroTrustProviderConfig::TailscaleSsh(TailscaleSshConfig {
                    host: "my-server".to_string(),
                    username: cfg_user.map(String::from),
                }),
                custom_args: strs(custom_args),
            }),
        );
        conn.username = conn_user.map(String::from);
        conn
    }

    #[test]
    fn openssh_socks_command_line_is_unchanged() {
        // Outside Flatpak there is no known_hosts override to add.
        if crate::flatpak::is_flatpak() {
            return;
        }
        let mut p = params(TunnelLauncher::OpenSsh, "admin@bastion.example.com");
        p.jump_port = 2222;
        p.identity_file = Some("/k".to_string());
        let (program, args) = socks_command_line(&p, 1080, false);
        assert_eq!(program, "ssh");
        assert_eq!(
            args,
            strs(&[
                "-N",
                "-D",
                "127.0.0.1:1080",
                "-p",
                "2222",
                "-i",
                "/k",
                "-o",
                "ConnectTimeout=8",
                "-o",
                "BatchMode=yes",
                "-o",
                "ExitOnForwardFailure=yes",
                "admin@bastion.example.com",
            ])
        );
    }

    #[test]
    fn tailscale_socks_command_line_puts_destination_first() {
        let mut p = params(TunnelLauncher::TailscaleSsh, "alice@my-server");
        p.jump_port = 2222; // ignored: the session passes no port either
        let (program, args) = socks_command_line(&p, 1080, false);
        assert_eq!(program, "tailscale");
        assert_eq!(
            args,
            strs(&[
                "ssh",
                "alice@my-server",
                "-N",
                "-D",
                "127.0.0.1:1080",
                "-o",
                "ConnectTimeout=8",
                "-o",
                "BatchMode=yes",
                "-o",
                "ExitOnForwardFailure=yes",
            ])
        );
        assert!(!args.iter().any(|a| a.starts_with("UserKnownHostsFile")));
    }

    #[test]
    fn tailscale_command_line_with_askpass_omits_batchmode() {
        let p = params(TunnelLauncher::TailscaleSsh, "my-server");
        let (_, args) = socks_command_line(&p, 1080, true);
        assert!(!args.contains(&"BatchMode=yes".to_string()), "{args:?}");
    }

    #[test]
    fn tailscale_command_line_keeps_caller_connect_timeout() {
        let mut p = params(TunnelLauncher::TailscaleSsh, "my-server");
        p.extra_args = strs(&["-o", "ConnectTimeout=3"]);
        let (_, args) = socks_command_line(&p, 1080, false);
        assert!(!args.contains(&"ConnectTimeout=8".to_string()), "{args:?}");
        let timeout_at = args
            .iter()
            .position(|a| a == "ConnectTimeout=3")
            .expect("caller timeout kept");
        let forward_at = args.iter().position(|a| a == "-D").expect("forward flag");
        assert!(timeout_at > forward_at, "{args:?}");
    }

    #[test]
    fn socks_tunnel_binds_to_loopback_for_both_launchers() {
        for (launcher, jump_host) in [
            (TunnelLauncher::OpenSsh, "admin@bastion.example.com"),
            (TunnelLauncher::TailscaleSsh, "alice@my-server"),
        ] {
            for uses_askpass in [false, true] {
                let p = params(launcher, jump_host);
                let (_, args) = socks_command_line(&p, 1080, uses_askpass);
                assert!(
                    args.windows(2)
                        .any(|w| w[0] == "-D" && w[1] == "127.0.0.1:1080"),
                    "{launcher:?} (askpass {uses_askpass}) must bind -D to loopback: {args:?}"
                );
                assert!(
                    !args.contains(&"1080".to_string()),
                    "{launcher:?} (askpass {uses_askpass}) must not pass a bare port: {args:?}"
                );
            }
        }
    }

    #[test]
    fn openssh_local_forward_command_line_binds_loopback() {
        // Outside Flatpak there is no known_hosts override to add.
        if crate::flatpak::is_flatpak() {
            return;
        }
        let mut p = params(TunnelLauncher::OpenSsh, "admin@bastion.example.com");
        p.remote_host = "rdp.internal".to_string();
        p.remote_port = 3389;
        let (program, args) = local_forward_command_line(&p, 40000, false);
        assert_eq!(program, "ssh");
        assert_eq!(
            args,
            strs(&[
                "-N",
                "-L",
                "127.0.0.1:40000:rdp.internal:3389",
                "-o",
                "ConnectTimeout=8",
                "-o",
                "BatchMode=yes",
                "-o",
                "ExitOnForwardFailure=yes",
                "admin@bastion.example.com",
            ])
        );
    }

    #[test]
    fn local_forward_binds_to_loopback_for_both_launchers() {
        for (launcher, jump_host) in [
            (TunnelLauncher::OpenSsh, "admin@bastion.example.com"),
            (TunnelLauncher::TailscaleSsh, "alice@my-server"),
        ] {
            for uses_askpass in [false, true] {
                let mut p = params(launcher, jump_host);
                p.remote_host = "10.0.0.5".to_string();
                p.remote_port = 5900;
                let (_, args) = local_forward_command_line(&p, 40001, uses_askpass);
                assert!(
                    args.windows(2)
                        .any(|w| w[0] == "-L" && w[1] == "127.0.0.1:40001:10.0.0.5:5900"),
                    "{launcher:?} (askpass {uses_askpass}) must bind -L to loopback: {args:?}"
                );
                assert!(
                    !args.iter().any(|a| a.starts_with("40001:")),
                    "{launcher:?} (askpass {uses_askpass}) must not pass a bare port: {args:?}"
                );
            }
        }
    }

    #[test]
    fn loopback_bind_spec_is_ipv4_loopback_literal() {
        let spec = loopback_bind_spec(1080);
        assert_eq!(spec, "127.0.0.1:1080");
        // Must stay the address `wait_for_tunnel_ready` probes: a `localhost` or
        // `::1` here would let the proxy and the probe disagree.
        let addr: std::net::SocketAddr = spec.parse().expect("a socket address");
        assert!(addr.is_ipv4() && addr.ip().is_loopback());
        assert_eq!(addr.port(), 1080);
    }

    #[test]
    fn tailscale_destination_accepts_plain_names() {
        for input in [
            "my-server",
            "alice@my-server.tail1234.ts.net",
            "root@my_server",
            "100.101.102.103",
            "fd7a:115c:a1e0::1",
            "j.doe@host-1",
        ] {
            assert!(
                validate_tailscale_destination(input).is_ok(),
                "{input:?} should be accepted"
            );
        }
    }

    #[test]
    fn tailscale_destination_rejects_option_and_shell_metacharacters() {
        for input in [
            "",
            "@host",
            "alice@",
            "-oProxyCommand=touch /tmp/pwn",
            "-oProxyCommand=x@host",
            "alice@-oProxyCommand=x",
            "host; touch /tmp/pwn",
            "host$(id)",
            "host`id`",
            "$(id)@host",
            "a@b@c",
            "ho st",
            "host\nx",
            "'host'",
            "host|nc",
        ] {
            assert!(
                matches!(
                    validate_tailscale_destination(input),
                    Err(SshTunnelError::UnsafeDestination)
                ),
                "{input:?} should be refused"
            );
        }
    }

    #[test]
    fn tailscale_socks_tunnel_refuses_unsafe_destination_without_spawning() {
        let p = params(
            TunnelLauncher::TailscaleSsh,
            "-oProxyCommand=touch /tmp/pwn",
        );
        assert!(matches!(
            create_socks_tunnel(&p),
            Err(SshTunnelError::UnsafeDestination)
        ));
    }

    #[test]
    fn socks_tunnel_route_gates_ssh_sftp_and_tailscale_only() {
        let route = socks_tunnel_route;
        assert_eq!(
            route(&Connection::new_ssh("a".into(), "h".into(), 22)),
            Some(SocksTunnelRoute::OpenSsh)
        );
        assert_eq!(
            route(&Connection::new_sftp("a".into(), "h".into(), 22)),
            Some(SocksTunnelRoute::OpenSsh)
        );
        assert_eq!(
            route(&Connection::new_rdp("a".into(), "h".into(), 3389)),
            None
        );
        assert_eq!(
            route(&Connection::new_vnc("a".into(), "h".into(), 5900)),
            None
        );
        assert_eq!(
            route(&Connection::new_telnet("a".into(), "h".into(), 23)),
            None
        );

        // Connection-level user is the fallback; the config's own user wins.
        assert_eq!(
            route(&tailscale_conn(Some("bob"), None, &["tmux"])),
            Some(SocksTunnelRoute::TailscaleSsh {
                destination: "bob@my-server".to_string()
            })
        );
        assert_eq!(
            route(&tailscale_conn(Some("bob"), Some("alice"), &[])),
            Some(SocksTunnelRoute::TailscaleSsh {
                destination: "alice@my-server".to_string()
            })
        );

        // Every other Zero Trust provider stays unsupported.
        for &provider in ZeroTrustProvider::all() {
            if provider == ZeroTrustProvider::TailscaleSsh {
                continue;
            }
            let conn = Connection::new(
                "zt".to_string(),
                String::new(),
                0,
                ProtocolConfig::ZeroTrust(ZeroTrustConfig {
                    provider,
                    provider_config: ZeroTrustProviderConfig::from_wizard_fields(
                        provider,
                        Some("echo hi"),
                        Some("x"),
                        Some("y"),
                        Some("z"),
                    ),
                    custom_args: Vec::new(),
                }),
            );
            assert_eq!(route(&conn), None, "{provider:?} must stay unsupported");
        }
    }

    fn zero_trust_conn(
        provider: ZeroTrustProvider,
        provider_config: ZeroTrustProviderConfig,
    ) -> Connection {
        Connection::new(
            "zt".to_string(),
            String::new(),
            0,
            ProtocolConfig::ZeroTrust(ZeroTrustConfig {
                provider,
                provider_config,
                custom_args: Vec::new(),
            }),
        )
    }

    fn tailscale_config() -> ZeroTrustProviderConfig {
        ZeroTrustProviderConfig::TailscaleSsh(TailscaleSshConfig {
            host: "my-server".to_string(),
            username: None,
        })
    }

    fn other_config(provider: ZeroTrustProvider) -> ZeroTrustProviderConfig {
        ZeroTrustProviderConfig::from_wizard_fields(
            provider,
            Some("echo hi"),
            Some("x"),
            Some("y"),
            Some("z"),
        )
    }

    #[test]
    fn socks_tunnel_route_allows_tailscale_when_provider_and_config_agree() {
        let conn = zero_trust_conn(ZeroTrustProvider::TailscaleSsh, tailscale_config());
        assert_eq!(
            socks_tunnel_route(&conn),
            Some(SocksTunnelRoute::TailscaleSsh {
                destination: "my-server".to_string()
            })
        );
        assert!(supports_browser_tunnel(&conn));
    }

    #[test]
    fn socks_tunnel_route_refuses_provider_config_mismatch() {
        // Provider says Tailscale SSH, the config belongs to another provider.
        for &other in ZeroTrustProvider::all() {
            if other == ZeroTrustProvider::TailscaleSsh {
                continue;
            }
            let conn = zero_trust_conn(ZeroTrustProvider::TailscaleSsh, other_config(other));
            assert_eq!(socks_tunnel_route(&conn), None, "config of {other:?}");
            assert!(!supports_browser_tunnel(&conn), "config of {other:?}");
        }
        // Config says Tailscale SSH, the provider is another one.
        for &other in ZeroTrustProvider::all() {
            if other == ZeroTrustProvider::TailscaleSsh {
                continue;
            }
            let conn = zero_trust_conn(other, tailscale_config());
            assert_eq!(socks_tunnel_route(&conn), None, "provider {other:?}");
            assert!(!supports_browser_tunnel(&conn), "provider {other:?}");
        }
    }

    #[test]
    fn supports_browser_tunnel_matches_route_for_all_protocols() {
        let supported = [
            Connection::new_ssh("a".into(), "h".into(), 22),
            Connection::new_sftp("a".into(), "h".into(), 22),
            tailscale_conn(None, None, &[]),
        ];
        for conn in &supported {
            assert!(supports_browser_tunnel(conn), "{:?}", conn.protocol_config);
            assert!(socks_tunnel_route(conn).is_some());
        }
        let unsupported = [
            Connection::new_rdp("a".into(), "h".into(), 3389),
            Connection::new_vnc("a".into(), "h".into(), 5900),
            Connection::new_telnet("a".into(), "h".into(), 23),
            zero_trust_conn(
                ZeroTrustProvider::AwsSsm,
                other_config(ZeroTrustProvider::AwsSsm),
            ),
        ];
        for conn in &unsupported {
            assert!(!supports_browser_tunnel(conn), "{:?}", conn.protocol_config);
            assert!(socks_tunnel_route(conn).is_none());
        }
    }

    #[test]
    fn socks_tunnel_route_destination_matches_session_command() {
        let conn = tailscale_conn(Some("bob"), None, &[]);
        let Some(SocksTunnelRoute::TailscaleSsh { destination }) = socks_tunnel_route(&conn) else {
            panic!("a Tailscale SSH connection must route through tailscale");
        };
        let ProtocolConfig::ZeroTrust(zt) = &conn.protocol_config else {
            panic!("expected a Zero Trust connection");
        };
        let (_, session_args) = zt.build_command(conn.username.as_deref());
        assert_eq!(session_args[1], destination);
    }
}
