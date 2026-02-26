//! SSH command execution for monitoring
//!
//! Runs monitoring commands on remote hosts via `ssh` (or `sshpass -e ssh`
//! for password-authenticated connections). This uses a separate SSH process
//! (not the VTE terminal session) to avoid interfering with the user's
//! interactive shell.

use std::time::Duration;
use tokio::process::Command;

/// Default timeout for SSH monitoring commands (seconds)
const SSH_EXEC_TIMEOUT_SECS: u64 = 10;

/// Builds an SSH exec closure for use with [`super::start_collector`].
///
/// The returned closure spawns `ssh` (or `sshpass -e ssh` when a password
/// is provided and `sshpass` is available) with the given host/port/user
/// and executes the provided shell command, returning stdout as a `String`.
///
/// # Arguments
/// * `host` - Remote hostname or IP
/// * `port` - SSH port
/// * `username` - Optional SSH username
/// * `identity_file` - Optional path to SSH private key
/// * `password` - Optional password for `sshpass` authentication
pub fn ssh_exec_factory(
    host: String,
    port: u16,
    username: Option<String>,
    identity_file: Option<String>,
    password: Option<String>,
) -> impl Fn(
    String,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
+ Send
+ 'static {
    // Check sshpass availability once at factory creation time
    let use_sshpass = password.is_some()
        && std::process::Command::new("sshpass")
            .arg("-V")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok();

    move |command: String| {
        let host = host.clone();
        let username = username.clone();
        let identity_file = identity_file.clone();
        let password = password.clone();
        let use_sshpass = use_sshpass;

        Box::pin(async move {
            let mut cmd;

            if use_sshpass {
                // Use sshpass for password auth
                cmd = Command::new("sshpass");
                cmd.arg("-e").arg("ssh");
                // Set SSHPASS env var (sshpass reads it with -e flag)
                if let Some(ref pw) = password {
                    cmd.env("SSHPASS", pw);
                }
            } else {
                cmd = Command::new("ssh");
                // Batch mode only when NOT using password auth
                cmd.arg("-o").arg("BatchMode=yes");
            }

            // Suppress known_hosts warnings
            cmd.arg("-o").arg("StrictHostKeyChecking=no");
            // Short connection timeout
            cmd.arg("-o").arg("ConnectTimeout=5");

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
