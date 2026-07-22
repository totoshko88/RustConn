//! SSH command execution for monitoring
//!
//! Runs monitoring commands on remote hosts via `ssh` with `SSH_ASKPASS`
//! for password-authenticated connections. This uses a separate SSH process
//! (not the VTE terminal session) to avoid interfering with the user's
//! interactive shell.
//!
//! Password authentication uses the `SSH_ASKPASS` mechanism instead of
//! `sshpass`: a temporary script echoes the password from an environment
//! variable, and `SSH_ASKPASS_REQUIRE=force` tells OpenSSH to use it
//! even without a TTY. This eliminates the `sshpass` external dependency.

use std::sync::Arc;
use std::time::Duration;

use futures::stream::StreamExt;
use secrecy::{ExposeSecret, SecretString};
use tokio::process::Command;

/// Default timeout for SSH monitoring commands (seconds)
const SSH_EXEC_TIMEOUT_SECS: u64 = 10;

/// Returns the SSH `ControlPath` for a given host/port combination.
///
/// This path is shared between the main VTE terminal SSH connection and the
/// monitoring SSH process. By using the same `ControlPath`, monitoring can
/// multiplex over the already-authenticated master connection, avoiding a
/// second key/passphrase prompt.
///
/// The path uses `XDG_RUNTIME_DIR` (tmpfs, user-private) when available,
/// falling back to the system temp directory.
///
/// On macOS, uses `/tmp` instead of `$TMPDIR` (which is a long path under
/// `/var/folders/...`) to stay within the 104-byte Unix socket path limit.
/// Long hostnames are truncated to keep the total path under the limit.
#[must_use]
pub fn ssh_control_path(host: &str, port: u16) -> String {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| {
        // macOS $TMPDIR is ~52 chars (/var/folders/xx/.../T/), which leaves
        // very little room for the socket name within the 104-byte limit.
        // Use /tmp instead (symlinks to /private/tmp on macOS, only 4 chars).
        if cfg!(target_os = "macos") {
            "/tmp".to_string()
        } else {
            std::env::temp_dir().to_string_lossy().to_string()
        }
    });

    // Unix socket path limit: 104 bytes on macOS, 108 on Linux.
    // Format: {dir}/rc-{host}-{port}-%r
    // Reserve ~20 chars for /%r expansion and null terminator.
    let max_host_len = 40;
    let short_host = if host.len() > max_host_len {
        // Truncate at a valid char boundary to avoid panic on IDN hostnames.
        &host[..host.floor_char_boundary(max_host_len)]
    } else {
        host
    };
    format!("{dir}/rc-{short_host}-{port}-%r")
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

/// Environment variable name used to pass the password to the askpass script.
/// Intentionally obscure to reduce exposure in `/proc/PID/environ`.
const ASKPASS_ENV_VAR: &str = "_RC_MON_PW";

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
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| {
        if cfg!(target_os = "macos") {
            "/tmp".to_string()
        } else {
            std::env::temp_dir().to_string_lossy().to_string()
        }
    });

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
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| {
        if cfg!(target_os = "macos") {
            "/tmp".to_string()
        } else {
            std::env::temp_dir().to_string_lossy().to_string()
        }
    });

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
/// Deletes the script file when the last `Arc<AskpassScript>` reference is dropped
/// (i.e. when the monitoring session ends and the factory closure is freed).
struct AskpassScript(std::path::PathBuf);

impl Drop for AskpassScript {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.0) {
            tracing::debug!(
                path = %self.0.display(),
                error = %e,
                "Failed to clean up askpass script"
            );
        }
    }
}

/// Creates a temporary `SSH_ASKPASS` helper script that echoes the password
/// from `ASKPASS_ENV_VAR`. The script is created with mode 0700 and lives
/// in the system temp directory.
///
/// Returns the path to the script on success.
fn create_askpass_script() -> Result<std::path::PathBuf, String> {
    use std::io::Write;

    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "rc-askpass-{}",
        uuid::Uuid::new_v4().as_hyphenated()
    ));

    let script = format!("#!/bin/sh\necho \"${ASKPASS_ENV_VAR}\"\n");

    let mut file = std::fs::File::create(&path)
        .map_err(|e| format!("Failed to create askpass script: {e}"))?;
    file.write_all(script.as_bytes())
        .map_err(|e| format!("Failed to write askpass script: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("Failed to set askpass script permissions: {e}"))?;
    }

    Ok(path)
}

/// Builds jump host arguments for the SSH monitoring command.
///
/// In Flatpak, `-J` (ProxyJump) spawns a nested SSH process that does NOT
/// inherit `-o` flags from the outer command. The jump host SSH tries to
/// write to `~/.ssh/known_hosts` (read-only in Flatpak) and prompts for
/// host key verification. This function replaces `-J` with a `ProxyCommand`
/// that passes `StrictHostKeyChecking` and `UserKnownHostsFile` to the
/// jump host SSH process.
///
/// Outside Flatpak, standard `-J` is used.
fn build_jump_host_args(cmd: &mut Command, jump_host: &str, identity_file: Option<&str>) {
    let flatpak_kh = crate::flatpak::get_flatpak_known_hosts_path();
    if flatpak_kh.is_some() {
        // The chain may carry several hops (`a,b,c`). Each must inherit the
        // identity and Flatpak known_hosts, so nest a ProxyCommand per hop —
        // `-J` alone drops them and the deeper hops fail (issue #191 follow-up).
        let hops: Vec<&str> = jump_host
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if hops.is_empty() {
            cmd.arg("-J").arg(jump_host);
            return;
        }
        // Monitoring probes have no controlling TTY, so unknown host keys are
        // accepted automatically (accept-new) — they cannot be prompted for.
        let proxy_cmd = crate::ssh_tunnel::build_nested_proxy_command(
            &hops,
            identity_file,
            flatpak_kh.as_deref(),
            true,
        );
        tracing::debug!(
            protocol = "ssh",
            proxy_command = %proxy_cmd,
            "Monitoring: using ProxyCommand instead of -J for Flatpak compatibility"
        );
        cmd.arg("-o").arg(format!("ProxyCommand={proxy_cmd}"));
    } else {
        // Non-Flatpak: standard -J. `jump_host` is target-first (RustConn's
        // internal order); OpenSSH `-J` visits hops client-first, so reverse.
        cmd.arg("-J")
            .arg(crate::ssh_tunnel::proxy_jump_arg(jump_host));
    }
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
    // Create the askpass script once at factory creation time.
    // It is reused for every monitoring command invocation.
    // Wrapped in Arc<AskpassScript> so the file is deleted when the factory is dropped.
    let askpass_script = if password.is_some() {
        match create_askpass_script() {
            Ok(p) => Some(Arc::new(AskpassScript(p))),
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "Failed to create SSH_ASKPASS script; \
                     password auth will not work for monitoring"
                );
                None
            }
        }
    } else {
        None
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

            if let (Some(pw), Some(script)) = (&password, &askpass_script) {
                // SSH_ASKPASS mechanism: OpenSSH calls the script to get
                // the password. DISPLAY must be set (even empty) and
                // SSH_ASKPASS_REQUIRE=force skips the TTY check.
                cmd.env("SSH_ASKPASS", &script.0);
                cmd.env("SSH_ASKPASS_REQUIRE", "force");
                cmd.env(ASKPASS_ENV_VAR, pw.expose_secret());
                // Ensure DISPLAY is set so SSH considers ASKPASS
                if std::env::var("DISPLAY").is_err() {
                    cmd.env("DISPLAY", "");
                }
            } else if password.is_none() {
                // Batch mode only when NOT using password auth
                cmd.arg("-o").arg("BatchMode=yes");
            }

            // Accept new host keys but reject changed ones (OpenSSH 7.6+).
            // Using `accept-new` instead of `no` prevents MITM attacks on
            // hosts whose key has changed while still allowing first-time
            // connections without manual intervention.
            cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");

            // In Flatpak, ~/.ssh is read-only — use writable known_hosts path
            if let Some(kh_path) = crate::flatpak::get_flatpak_known_hosts_path() {
                let kh_opt = format!("UserKnownHostsFile={}", kh_path.display());
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
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        Err(format!(
                            "SSH command failed (exit {}): {}",
                            output.status,
                            stderr.trim()
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
