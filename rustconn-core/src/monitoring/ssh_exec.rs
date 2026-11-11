//! SSH command execution for monitoring
//!
//! Runs monitoring commands on remote hosts via `ssh` with `SSH_ASKPASS`
//! for password-authenticated connections. This uses a separate SSH process
//! (not the VTE terminal session) to avoid interfering with the user's
//! interactive shell.
//!
//! Password authentication uses the `SSH_ASKPASS` mechanism instead of
//! `sshpass`: a temporary script reads the password from an owner-only file
//! whose path is passed in the environment, and `SSH_ASKPASS_REQUIRE=force`
//! tells OpenSSH to use it even without a TTY. The password value never enters
//! the SSH process environment. This eliminates the `sshpass` external
//! dependency.

use std::sync::Arc;
use std::time::Duration;

use futures::stream::StreamExt;
use secrecy::SecretString;
use tokio::process::Command;

/// Default timeout for SSH monitoring commands (seconds)
const SSH_EXEC_TIMEOUT_SECS: u64 = 10;

/// Directory that holds RustConn's SSH ControlMaster sockets.
///
/// Prefers `XDG_RUNTIME_DIR` (tmpfs, user-private, short path). On macOS the
/// fallback is `/tmp` rather than `$TMPDIR`: the latter is ~52 chars under
/// `/var/folders/...`, which alone eats half of the 104-byte socket path limit.
fn control_socket_dir() -> String {
    std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| {
        if cfg!(target_os = "macos") {
            "/tmp".to_string()
        } else {
            std::env::temp_dir().to_string_lossy().to_string()
        }
    })
}

/// Usable length of `sockaddr_un.sun_path`: 104 bytes on macOS, 108 on Linux,
/// minus the NUL terminator. The smaller value keeps paths portable.
const SOCKET_PATH_MAX: usize = 103;

/// Extra bytes OpenSSH appends while the multiplex master is being created.
///
/// `muxserver_listen()` binds `"{ControlPath}.{16 random chars}"` first and
/// only then renames it into place, so the path that actually reaches
/// `bind()` is a dot plus 16 characters longer than what we pass in.
const MUX_TEMP_SUFFIX_LEN: usize = 17;

/// Budget reserved for the `%r` (remote username) expansion.
///
/// SSH expands `%r` itself, so the length of the real username is unknown here
/// and a fixed budget is the only defence. Linux caps local usernames at 32
/// chars, cloud/IdP setups hand out UUIDs (36) and Entra ID hands out UPNs, so
/// the budget covers 48.
///
/// Raising it further is not free: the whole path must fit
/// [`SOCKET_PATH_MAX`], so a larger reserve pushes more hosts from the readable
/// form onto the digest form and eventually out of `XDG_RUNTIME_DIR` into
/// `/tmp` — where the socket is no longer user-private and the cleanup helpers
/// (which scan [`control_socket_dir`]) would stop finding it. 48 keeps the
/// digest form comfortably inside a runtime directory of up to 21 chars.
const USERNAME_RESERVE: usize = 48;

/// Length of the digest used when the readable form does not fit.
///
/// 12 hex chars = 48 bits of SHA-256: collision-free in practice for the
/// handful of hosts a single user connects to.
const HOST_DIGEST_LEN: usize = 12;

/// Worst-case length of a `ControlPath` template once SSH is done with it.
///
/// `%r` (2 chars) becomes a username of up to [`USERNAME_RESERVE`] chars, and
/// the master adds [`MUX_TEMP_SUFFIX_LEN`] bytes before renaming.
fn expanded_path_len(template: &str) -> usize {
    template.len() - "%r".len() + USERNAME_RESERVE + MUX_TEMP_SUFFIX_LEN
}

/// Returns a short, stable digest of `host:port` for use in a socket name.
///
/// Hostnames are hashed rather than truncated: two hosts sharing a long
/// prefix (`<uuid>-a.example.com` / `<uuid>-b.example.com`) must never end up
/// multiplexing over the same master connection.
fn host_digest(host: &str, port: u16) -> String {
    let key = format!("{host}\u{0}{port}");
    let digest = ring::digest::digest(&ring::digest::SHA256, key.as_bytes());
    let mut hex = hex::encode(digest.as_ref());
    hex.truncate(HOST_DIGEST_LEN);
    hex
}

/// Returns the SSH `ControlPath` for a given host/port combination.
///
/// This path is shared between the main VTE terminal SSH connection and the
/// monitoring SSH process. By using the same `ControlPath`, monitoring can
/// multiplex over the already-authenticated master connection, avoiding a
/// second key/passphrase prompt.
///
/// The result is guaranteed to stay within the Unix domain socket path limit
/// even after SSH expands `%r` and appends its temporary master suffix. Hosts
/// whose readable form would overflow are identified by a digest instead
/// (issue #239).
#[must_use]
pub fn ssh_control_path(host: &str, port: u16) -> String {
    control_path_in(&control_socket_dir(), host, port)
}

/// Builds the `ControlPath` template inside `dir` (see [`ssh_control_path`]).
fn control_path_in(dir: &str, host: &str, port: u16) -> String {
    // Preferred, human-readable form: {dir}/rc-{host}-{port}-%r
    let readable = format!("{dir}/rc-{host}-{port}-%r");
    if expanded_path_len(&readable) <= SOCKET_PATH_MAX {
        return readable;
    }

    // Too long — fall back to a digest of host+port (also covers the port,
    // so it can be dropped from the name).
    let digest = host_digest(host, port);
    let hashed = format!("{dir}/rc-{digest}-%r");
    if expanded_path_len(&hashed) <= SOCKET_PATH_MAX {
        return hashed;
    }

    // Even the digest form does not fit, so the directory itself is the
    // problem (an unusually long XDG_RUNTIME_DIR). /tmp is the last resort.
    let fallback = format!("/tmp/rc-{digest}-%r");
    tracing::warn!(
        %dir,
        %fallback,
        "Runtime directory too long for an SSH ControlMaster socket, using /tmp"
    );
    fallback
}

/// Reports whether a ControlMaster socket for `host:port` is already present.
///
/// Used as a connect-time fast path (P4): when the master socket is already
/// there, a new SSH session multiplexes over it, so the pre-connect port check
/// is redundant (the TCP connection is already up) and no target-password
/// askpass file need be written (the master is already authenticated).
///
/// `ControlPath` contains `%r` (the remote user), which OpenSSH expands at
/// runtime, so the exact file name is not known here — this matches any socket
/// file sharing the path's prefix. It additionally requires the match to be a
/// Unix **socket**, not a stale regular file, so a leftover non-socket does not
/// trigger the fast path. It does **not** run `ssh -O check`: that would spawn
/// the very process the fast path exists to avoid, and a socket that is present
/// but dead is simply re-established by OpenSSH's own `ControlMaster=auto`
/// fallback — the only cost of a false positive is a skipped port check, which
/// `bypasses_direct_probe` connections already skip safely.
#[must_use]
pub fn control_socket_present(host: &str, port: u16) -> bool {
    let control_path = ssh_control_path(host, port);
    let prefix = control_path.replace("-%r", "-");
    socket_with_prefix_exists(&prefix)
}

/// Like [`glob_socket_exists`] but requires the match to be a Unix socket.
fn socket_with_prefix_exists(prefix: &str) -> bool {
    let Some(dir) = std::path::Path::new(prefix).parent() else {
        return false;
    };
    let Some(file_prefix) = std::path::Path::new(prefix).file_name() else {
        return false;
    };
    let file_prefix_str = file_prefix.to_string_lossy();

    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };

    entries.filter_map(Result::ok).any(|entry| {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(file_prefix_str.as_ref())
        {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::FileTypeExt as _;
            entry.file_type().map(|ft| ft.is_socket()).unwrap_or(false)
        }
        #[cfg(not(unix))]
        {
            true
        }
    })
}

/// Checks if any file exists with the given prefix (for socket detection).
///
/// SSH expands `%r` in `ControlPath` to the remote username, so we can't
/// predict the exact filename. Instead we check if any file starting with
/// the prefix (everything before `-%r`) exists in the directory.
fn glob_socket_exists(prefix: &str) -> bool {
    let Some(dir) = std::path::Path::new(prefix).parent() else {
        return false;
    };
    let Some(file_prefix) = std::path::Path::new(prefix).file_name() else {
        return false;
    };
    let file_prefix_str = file_prefix.to_string_lossy();

    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };

    entries.filter_map(Result::ok).any(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .starts_with(file_prefix_str.as_ref())
    })
}

/// Closes the SSH ControlMaster socket for a given host/port.
///
/// Sends `ssh -O exit` to gracefully terminate the master connection.
/// Called when the last session to a host is closed or on application exit.
/// Errors are logged but not propagated (best-effort cleanup).
pub async fn close_control_socket(host: &str, port: u16, username: Option<&str>) {
    let control_path = ssh_control_path(host, port);

    let mut cmd = Command::new("ssh");
    cmd.arg("-O").arg("exit");
    cmd.arg("-o").arg(format!("ControlPath={control_path}"));

    if port != 22 {
        cmd.arg("-p").arg(port.to_string());
    }

    let destination = if let Some(user) = username {
        format!("{user}@{host}")
    } else {
        host.to_string()
    };
    cmd.arg(&destination);

    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());

    match tokio::time::timeout(Duration::from_secs(3), cmd.output()).await {
        Ok(Ok(output)) => {
            if output.status.success() {
                tracing::debug!(%host, port, "ControlMaster socket closed");
            } else {
                tracing::debug!(%host, port, "ControlMaster socket already closed or not found");
            }
        }
        Ok(Err(e)) => {
            tracing::debug!(%host, port, error = %e, "Failed to close ControlMaster socket");
        }
        Err(_) => {
            tracing::debug!(%host, port, "Timeout closing ControlMaster socket");
        }
    }
}

/// Finds and closes all RustConn SSH ControlMaster sockets.
///
/// Scans the runtime directory (`XDG_RUNTIME_DIR` or system temp) for socket
/// files matching the `rc-*` naming pattern and sends `ssh -O exit` to each.
/// This is called on application exit to ensure no stale sockets linger,
/// regardless of session state (which may already be `Terminated` by the time
/// the GTK shutdown handler runs).
///
/// Errors are logged but not propagated (best-effort cleanup).
pub async fn close_all_control_sockets() {
    let dir = control_socket_dir();

    let Ok(entries) = std::fs::read_dir(&dir) else {
        tracing::debug!(dir, "Cannot read runtime directory for socket cleanup");
        return;
    };

    let socket_files: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            // Match RustConn SSH sockets: rc-{host}-{port}-{username}
            // Exclude askpass scripts: rc-askpass-*
            if !name_str.starts_with("rc-") || name_str.starts_with("rc-askpass-") {
                return false;
            }
            // Only include actual Unix sockets (skip regular files that happen
            // to match the naming pattern).
            #[cfg(unix)]
            {
                use std::os::unix::fs::FileTypeExt;
                entry.file_type().map(|ft| ft.is_socket()).unwrap_or(false)
            }
            #[cfg(not(unix))]
            {
                true
            }
        })
        .collect();

    if socket_files.is_empty() {
        return;
    }

    tracing::debug!(
        count = socket_files.len(),
        "Closing RustConn SSH ControlMaster sockets on exit"
    );

    futures::stream::iter(socket_files.iter().map(|entry| {
        let path = entry.path();
        let path_str = path.to_string_lossy().to_string();
        async move {
            let mut cmd = Command::new("ssh");
            cmd.arg("-O").arg("exit");
            cmd.arg("-o").arg(format!("ControlPath={path_str}"));
            // Use a dummy destination — ssh -O exit only needs the socket path
            // to identify the master, but requires a destination argument.
            // We use "_" as a placeholder (less likely to collide with a real
            // Host entry in ~/.ssh/config than "none" or "localhost").
            cmd.arg("_");
            cmd.stdout(std::process::Stdio::null());
            cmd.stderr(std::process::Stdio::null());

            match tokio::time::timeout(Duration::from_secs(3), cmd.output()).await {
                Ok(Ok(output)) => {
                    if output.status.success() {
                        tracing::debug!(socket = %path_str, "ControlMaster socket closed");
                    } else {
                        // Socket may already be stale — remove the file directly
                        let _ = std::fs::remove_file(&path);
                        tracing::debug!(
                            socket = %path_str,
                            "ControlMaster socket not responding, removed file"
                        );
                    }
                }
                Ok(Err(e)) => {
                    tracing::debug!(
                        socket = %path_str, error = %e,
                        "Failed to close ControlMaster socket"
                    );
                }
                Err(_) => {
                    // Timeout — force remove the stale socket file
                    let _ = std::fs::remove_file(&path);
                    tracing::debug!(
                        socket = %path_str,
                        "Timeout closing ControlMaster socket, removed file"
                    );
                }
            }
        }
    }))
    // Limit concurrency to avoid spawning too many ssh processes at once
    .buffer_unordered(10)
    .collect::<Vec<()>>()
    .await;
}

/// Checks all RustConn SSH ControlMaster sockets and removes only dead ones.
///
/// Unlike [`close_all_control_sockets`] (which unconditionally kills every
/// master), this function first probes each socket with `ssh -O check`. If the
/// master is still alive and responsive, the socket is left untouched. Only
/// sockets where `check` fails (master exited, TCP connection dead) are cleaned
/// up by removing the stale socket file.
///
/// This is the appropriate function to call on network-change events where the
/// route to the SSH host may not have changed (e.g. VPN connect/disconnect that
/// only adds/removes specific routes without affecting the default gateway).
///
/// # Returns
/// The number of stale sockets that were removed.
pub async fn close_dead_control_sockets() -> u32 {
    let dir = control_socket_dir();

    let Ok(entries) = std::fs::read_dir(&dir) else {
        tracing::debug!(dir, "Cannot read runtime directory for socket health check");
        return 0;
    };

    let socket_files: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.starts_with("rc-") || name_str.starts_with("rc-askpass-") {
                return false;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::FileTypeExt;
                entry.file_type().map(|ft| ft.is_socket()).unwrap_or(false)
            }
            #[cfg(not(unix))]
            {
                true
            }
        })
        .collect();

    if socket_files.is_empty() {
        return 0;
    }

    tracing::debug!(
        count = socket_files.len(),
        "Checking health of RustConn SSH ControlMaster sockets"
    );

    let removed = std::sync::atomic::AtomicU32::new(0);

    futures::stream::iter(socket_files.iter().map(|entry| {
        let path = entry.path();
        let path_str = path.to_string_lossy().to_string();
        let removed_ref = &removed;
        async move {
            // Probe the socket with `ssh -O check` — if the master is alive
            // and the underlying TCP connection is healthy, this returns exit 0.
            let mut cmd = Command::new("ssh");
            cmd.arg("-O").arg("check");
            cmd.arg("-o").arg(format!("ControlPath={path_str}"));
            cmd.arg("_");
            cmd.stdout(std::process::Stdio::null());
            cmd.stderr(std::process::Stdio::null());

            match tokio::time::timeout(Duration::from_secs(3), cmd.output()).await {
                Ok(Ok(output)) if output.status.success() => {
                    // Master is alive — do not touch this socket
                    tracing::debug!(
                        socket = %path_str,
                        "ControlMaster socket is healthy, keeping alive"
                    );
                }
                Ok(Ok(_output)) => {
                    // `ssh -O check` failed → master is dead, remove stale file
                    let _ = std::fs::remove_file(&path);
                    removed_ref.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    tracing::debug!(
                        socket = %path_str,
                        "ControlMaster socket is dead, removed stale file"
                    );
                }
                Ok(Err(e)) => {
                    tracing::debug!(
                        socket = %path_str, error = %e,
                        "Failed to check ControlMaster socket health"
                    );
                }
                Err(_) => {
                    // Timeout on check — master is likely stuck, remove socket
                    let _ = std::fs::remove_file(&path);
                    removed_ref.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    tracing::debug!(
                        socket = %path_str,
                        "Timeout checking ControlMaster socket, removed stale file"
                    );
                }
            }
        }
    }))
    .buffer_unordered(10)
    .collect::<Vec<()>>()
    .await;

    let count = removed.load(std::sync::atomic::Ordering::Relaxed);
    if count > 0 {
        tracing::info!(
            removed = count,
            total = socket_files.len(),
            "Removed dead ControlMaster sockets after network change"
        );
    } else {
        tracing::debug!(
            total = socket_files.len(),
            "All ControlMaster sockets are healthy — no cleanup needed"
        );
    }
    count
}

/// RAII wrapper for the temporary `SSH_ASKPASS` script.
///
/// Deletes the script and the owner-only secret file when the last
/// `Arc<AskpassScript>` reference is dropped (i.e. when the monitoring session
/// ends and the factory closure is freed).
///
/// `script` is the helper OpenSSH runs; `secret_file` holds the password bytes.
/// The password lives in `secret_file`, not in the SSH process environment —
/// the env carries only its path. Both are torn down together, zeroizing the
/// secret file first.
struct AskpassScript {
    script: std::path::PathBuf,
    secret_file: std::path::PathBuf,
}

impl Drop for AskpassScript {
    fn drop(&mut self) {
        // Zeroize-then-unlink the secret file (it held the plaintext); the
        // script carries no secret, so a plain remove is enough.
        crate::ssh_tunnel::cleanup_askpass_script(&self.secret_file);
        if let Err(e) = std::fs::remove_file(&self.script) {
            tracing::debug!(
                path = %self.script.display(),
                error = %e,
                "Failed to clean up askpass script"
            );
        }
    }
}

/// Builds jump host arguments for the SSH monitoring command.
///
/// `-J` (ProxyJump) spawns a nested SSH process per hop that does NOT inherit
/// `-o` flags from the outer command, so a hop would fall back to the user's
/// `StrictHostKeyChecking` (usually `ask`) and could put the host-key question
/// to a desktop askpass, or to the monitoring password helper. This replaces
/// `-J` with a nested `ProxyCommand` that gives every hop
/// `StrictHostKeyChecking=yes` ([`HopHostKeys::KnownOnly`]). In Flatpak each
/// hop also gets the identity and the writable known_hosts file, which `-J`
/// would drop as well (issue #191 follow-up); outside it, as with `-J`, the
/// user's ssh config supplies them.
///
/// [`HopHostKeys::KnownOnly`]: crate::ssh_tunnel::HopHostKeys::KnownOnly
fn build_jump_host_args(cmd: &mut Command, jump_host: &str, identity_file: Option<&str>) {
    let flatpak_kh = crate::flatpak::get_flatpak_known_hosts_path();
    // The chain may carry several hops (`a,b,c`), target-first (RustConn's
    // internal order), which is the order the nested command takes.
    let hops: Vec<&str> = jump_host
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if hops.is_empty() {
        return;
    }
    let proxy_cmd = crate::ssh_tunnel::build_nested_proxy_command(
        &hops,
        identity_file.filter(|_| flatpak_kh.is_some()),
        flatpak_kh.as_deref(),
        crate::ssh_tunnel::HopHostKeys::KnownOnly,
    );
    tracing::debug!(
        protocol = "ssh",
        proxy_command = %proxy_cmd,
        "Monitoring: using a ProxyCommand that refuses unknown host keys on every hop"
    );
    cmd.arg("-o").arg(format!("ProxyCommand={proxy_cmd}"));
}

/// Waits for the main SSH session's ControlMaster socket to appear.
///
/// Polls for up to 5 seconds (50 × 100ms) checking if any socket file
/// matching the control path pattern exists. Returns `true` if found.
async fn wait_for_control_socket(control_path: &str) -> bool {
    let socket_prefix = control_path.replace("-%r", "-");
    for _ in 0..50 {
        if std::path::Path::new(control_path).exists() || glob_socket_exists(&socket_prefix) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

/// Builds an SSH exec closure for use with [`super::start_collector`].
///
/// The returned closure spawns `ssh` with the given host/port/user and
/// executes the provided shell command, returning stdout as a `String`.
///
/// When a password is provided, the `SSH_ASKPASS` mechanism is used:
/// a temporary script echoes the password from an environment variable,
/// and `SSH_ASKPASS_REQUIRE=force` tells OpenSSH to invoke it. This
/// replaces the previous `sshpass` dependency.
///
/// # Arguments
/// * `host` - Remote hostname or IP
/// * `port` - SSH port
/// * `username` - Optional SSH username
/// * `identity_file` - Optional path to SSH private key
/// * `password` - Optional password (as `SecretString`) for SSH_ASKPASS auth
/// * `jump_host` - Optional jump host chain for `-J` flag (e.g. `"user@bastion:22"`)
#[expect(
    clippy::too_many_lines,
    reason = "factory sets up the file-based askpass helper once, then returns one closure that assembles the full ssh command; both halves read linearly and splitting them would hide the per-invocation command shape"
)]
pub fn ssh_exec_factory(
    host: String,
    port: u16,
    username: Option<String>,
    identity_file: Option<String>,
    password: Option<SecretString>,
    jump_host: Option<String>,
) -> impl Fn(
    String,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
+ Send
+ 'static {
    // Create the askpass script and the owner-only secret file once at factory
    // creation time; both are reused for every monitoring command invocation
    // and removed together when the factory (and its last Arc) is dropped. The
    // password reaches OpenSSH through the file, never the environment. Shared
    // with the embedded tunnel path in `ssh_tunnel`.
    let askpass_script = match password.as_ref() {
        Some(pw) => match (
            crate::ssh_tunnel::create_tunnel_askpass_script(),
            crate::ssh_tunnel::create_tunnel_secret_file(pw),
        ) {
            (Ok(script), Ok(secret_file)) => Some(Arc::new(AskpassScript {
                script,
                secret_file,
            })),
            (script_result, secret_result) => {
                if let Ok(ref path) = script_result {
                    crate::ssh_tunnel::cleanup_askpass_script(path);
                }
                if let Ok(ref path) = secret_result {
                    crate::ssh_tunnel::cleanup_askpass_script(path);
                }
                let error = script_result
                    .err()
                    .or_else(|| secret_result.err())
                    .unwrap_or_default();
                tracing::error!(
                    %error,
                    "Failed to create SSH_ASKPASS helper; \
                     password auth will not work for monitoring"
                );
                None
            }
        },
        None => None,
    };

    move |command: String| {
        let host = host.clone();
        let username = username.clone();
        let identity_file = identity_file.clone();
        let password = password.clone();
        let jump_host = jump_host.clone();
        let askpass_script = askpass_script.clone();
        let control_path = ssh_control_path(&host, port);

        Box::pin(async move {
            let mut cmd = Command::new("ssh");

            // Wait for the main SSH session's ControlMaster socket to appear.
            let socket_ready = wait_for_control_socket(&control_path).await;

            if socket_ready {
                // Socket exists — connect as slave only (no new auth needed).
                cmd.arg("-o").arg("ControlMaster=no");
            } else {
                // Socket not found after timeout — fall back to creating our own
                // master. This handles edge cases where the main session doesn't
                // use ControlMaster (e.g., user disabled it in extra_args).
                tracing::debug!(
                    %control_path,
                    "Monitoring: ControlMaster socket not found, creating own master"
                );
                cmd.arg("-o").arg("ControlMaster=auto");
                cmd.arg("-o").arg("ControlPersist=30");
            }
            cmd.arg("-o").arg(format!("ControlPath={control_path}"));

            if let Some(script) = &askpass_script {
                // SSH_ASKPASS mechanism: OpenSSH calls the script, which reads
                // the password from the owner-only secret file. The env carries
                // only that file's path, never the password. DISPLAY must be set
                // (even empty) and SSH_ASKPASS_REQUIRE=force skips the TTY check.
                cmd.env("SSH_ASKPASS", &script.script);
                cmd.env("SSH_ASKPASS_REQUIRE", "force");
                cmd.env(
                    crate::ssh_tunnel::TUNNEL_ASKPASS_ENV_VAR,
                    &script.secret_file,
                );
                // Ensure DISPLAY is set so SSH considers ASKPASS
                if std::env::var("DISPLAY").is_err() {
                    cmd.env("DISPLAY", "");
                }
            } else if password.is_none() {
                // Batch mode only when NOT using password auth
                cmd.arg("-o").arg("BatchMode=yes");
            }

            // Only a host key already in known_hosts: the probe runs in the
            // background, so it must never accept a key the user has not
            // accepted in the session's terminal, nor ask about one. A probe
            // that meets an unknown key fails like any other probe (logged at
            // debug by the collector).
            cmd.arg("-o").arg("StrictHostKeyChecking=yes");

            // In Flatpak, ~/.ssh is read-only — use writable known_hosts path
            if let Some(kh_opt) = crate::flatpak::get_flatpak_known_hosts_path()
                .and_then(|kh_path| crate::ssh_tunnel::user_known_hosts_option(&kh_path))
            {
                cmd.arg("-o").arg(kh_opt);
            }

            // Short connection timeout
            cmd.arg("-o").arg("ConnectTimeout=5");

            // Jump host chain for tunneled connections
            if let Some(ref jh) = jump_host {
                build_jump_host_args(&mut cmd, jh, identity_file.as_deref());
            }

            if port != 22 {
                cmd.arg("-p").arg(port.to_string());
            }

            if let Some(ref key) = identity_file {
                cmd.arg("-i").arg(key);
            }

            let destination = if let Some(ref user) = username {
                format!("{user}@{host}")
            } else {
                host.clone()
            };
            cmd.arg(&destination);
            cmd.arg(&command);

            // Suppress stderr to avoid noise
            cmd.stderr(std::process::Stdio::piped());
            cmd.stdout(std::process::Stdio::piped());

            let timeout = Duration::from_secs(SSH_EXEC_TIMEOUT_SECS);

            match tokio::time::timeout(timeout, cmd.output()).await {
                Ok(Ok(output)) => {
                    if output.status.success() {
                        String::from_utf8(output.stdout)
                            .map_err(|e| format!("Invalid UTF-8 in SSH output: {e}"))
                    } else {
                        // A non-zero exit still carries diagnostic output, and for
                        // RouterOS the signal the collector keys on ("bad command
                        // name") is printed to STDOUT, not stderr — its console
                        // exits non-zero for every Linux command. Include both so
                        // the collector can detect RouterOS from the error, and a
                        // real Linux failure is easier to read. (See
                        // `looks_like_routeros` and the collector's OS detection.)
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        Err(format!(
                            "SSH command failed (exit {}): {} {}",
                            output.status,
                            stderr.trim(),
                            stdout.trim()
                        ))
                    }
                }
                Ok(Err(e)) => Err(format!("Failed to spawn SSH process: {e}")),
                Err(_) => Err(format!(
                    "SSH monitoring command timed out after {SSH_EXEC_TIMEOUT_SECS}s"
                )),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MUX_TEMP_SUFFIX_LEN, SOCKET_PATH_MAX, USERNAME_RESERVE, control_path_in,
        socket_with_prefix_exists, ssh_control_path,
    };

    /// Simulates what OpenSSH actually binds: `%r` expanded to a username of
    /// `username_len` chars plus the temporary master suffix.
    fn bound_socket_len(template: &str, username_len: usize) -> usize {
        template.len() - "%r".len() + username_len + MUX_TEMP_SUFFIX_LEN
    }

    #[test]
    fn short_host_keeps_readable_form() {
        let path = control_path_in("/run/user/1000", "192.168.1.10", 22);
        assert_eq!(path, "/run/user/1000/rc-192.168.1.10-22-%r");
    }

    #[test]
    fn long_host_with_uuid_username_fits_socket_limit() {
        // Reproduces issue #239: UUID subdomain + UUID remote username.
        let host = "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0-server.example.com";
        let path = control_path_in("/run/user/1000", host, 5902);
        let uuid_username_len = 36;

        assert!(
            bound_socket_len(&path, uuid_username_len) <= SOCKET_PATH_MAX,
            "path {path} would overflow sun_path once expanded"
        );
        assert!(
            bound_socket_len(&path, USERNAME_RESERVE) <= SOCKET_PATH_MAX,
            "path {path} must also fit the reserved username budget"
        );
        assert!(path.ends_with("-%r"), "path must keep the %r token: {path}");
    }

    #[test]
    fn hosts_sharing_a_long_prefix_get_distinct_paths() {
        let prefix = "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0-server";
        let a = control_path_in("/run/user/1000", &format!("{prefix}-a.example.com"), 22);
        let b = control_path_in("/run/user/1000", &format!("{prefix}-b.example.com"), 22);
        assert_ne!(a, b, "truncation must not merge two different hosts");
    }

    #[test]
    fn digest_form_is_stable_and_port_specific() {
        let host = "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0-server.example.com";
        let first = control_path_in("/run/user/1000", host, 22);
        let second = control_path_in("/run/user/1000", host, 22);
        assert_eq!(first, second, "same host/port must map to the same socket");
        assert_ne!(
            first,
            control_path_in("/run/user/1000", host, 2222),
            "different ports must not share a master socket"
        );
    }

    /// The digest form must still fit inside `XDG_RUNTIME_DIR`: `/tmp` is not
    /// user-private and the cleanup helpers only scan the runtime directory, so
    /// a larger `USERNAME_RESERVE` must not silently push sockets out of it.
    #[test]
    fn digest_form_stays_in_runtime_dir() {
        let host = "x".repeat(200);
        for dir in ["/run/user/1000", "/run/user/1000000"] {
            let path = control_path_in(dir, &host, 22);
            assert!(
                path.starts_with(dir),
                "socket left the runtime directory {dir}: {path}"
            );
            assert!(bound_socket_len(&path, USERNAME_RESERVE) <= SOCKET_PATH_MAX);
        }
    }

    #[test]
    fn overlong_directory_falls_back_to_tmp() {
        let dir = format!("/run/user/1000/{}", "d".repeat(80));
        let path = control_path_in(&dir, "example.com", 22);
        assert!(path.starts_with("/tmp/rc-"), "unexpected fallback: {path}");
        assert!(bound_socket_len(&path, USERNAME_RESERVE) <= SOCKET_PATH_MAX);
    }

    #[test]
    fn public_path_fits_for_pathological_host() {
        let host = "x".repeat(255);
        let path = ssh_control_path(&host, 65535);
        assert!(
            bound_socket_len(&path, USERNAME_RESERVE) <= SOCKET_PATH_MAX,
            "ssh_control_path returned an unusable path: {path}"
        );
    }

    // ── P4: ControlMaster socket detection ────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn a_real_socket_sharing_the_prefix_is_detected() {
        use std::os::unix::net::UnixListener;

        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("rc-host-22-");
        // OpenSSH binds `<prefix><user>`; emulate it with a real socket.
        let socket_path = dir.path().join("rc-host-22-alice");
        let _listener = UnixListener::bind(&socket_path).unwrap();

        assert!(socket_with_prefix_exists(prefix.to_str().unwrap()));
    }

    #[cfg(unix)]
    #[test]
    fn a_regular_file_sharing_the_prefix_is_not_a_socket() {
        // A stale non-socket left with the prefix must NOT trigger the fast
        // path — only a live socket does.
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("rc-host-22-");
        std::fs::write(dir.path().join("rc-host-22-alice"), b"stale").unwrap();

        assert!(!socket_with_prefix_exists(prefix.to_str().unwrap()));
    }

    #[test]
    fn an_empty_directory_has_no_socket() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("rc-host-22-");
        assert!(!socket_with_prefix_exists(prefix.to_str().unwrap()));
    }
}
