//! Standalone SSH tunnel manager
//!
//! Manages headless `ssh -N` processes for port forwarding without
//! terminal sessions. Each tunnel references an existing SSH connection
//! for host/key/password configuration.

use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

use secrecy::ExposeSecret;
use thiserror::Error;
use uuid::Uuid;

use crate::models::{Connection, ProtocolConfig, StandaloneTunnel, TunnelStatus};

/// Errors from tunnel operations
#[derive(Debug, Error)]
pub enum TunnelManagerError {
    /// The referenced SSH connection was not found
    #[error("SSH connection not found: {0}")]
    ConnectionNotFound(Uuid),
    /// The referenced connection is not an SSH connection
    #[error("Connection {0} is not SSH")]
    NotSshConnection(Uuid),
    /// The tunnel is already running
    #[error("Tunnel {0} is already running")]
    AlreadyRunning(Uuid),
    /// The tunnel was not found
    #[error("Tunnel not found: {0}")]
    TunnelNotFound(Uuid),
    /// Failed to spawn the SSH process
    #[error("Failed to spawn SSH tunnel: {0}")]
    SpawnFailed(#[from] std::io::Error),
}

/// Result type for tunnel manager operations
pub type TunnelManagerResult<T> = Result<T, TunnelManagerError>;

/// A running tunnel process with its metadata
struct RunningTunnel {
    child: Child,
    stderr_output: Arc<Mutex<String>>,
    status: TunnelStatus,
}

/// Maximum number of automatic reconnect attempts before giving up
const MAX_RECONNECT_ATTEMPTS: u32 = 5;

/// Manages standalone SSH tunnels (headless `ssh -N` processes)
///
/// The manager holds references to running tunnel processes and provides
/// start/stop/status operations. It does NOT own the tunnel definitions —
/// those live in `AppSettings.standalone_tunnels`.
pub struct TunnelManager {
    /// Running tunnel processes indexed by tunnel ID
    running: HashMap<Uuid, RunningTunnel>,
    /// Consecutive reconnect failure count per tunnel (reset on manual start/stop)
    reconnect_failures: HashMap<Uuid, u32>,
}

impl TunnelManager {
    /// Creates a new empty tunnel manager
    #[must_use]
    pub fn new() -> Self {
        Self {
            running: HashMap::new(),
            reconnect_failures: HashMap::new(),
        }
    }

    /// Starts a tunnel by spawning an `ssh -N` process with the configured forwards.
    ///
    /// The `connection` must be an SSH connection that provides host, port,
    /// username, identity file, and other SSH options.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection is not SSH, the tunnel is already
    /// running, or the SSH process fails to spawn.
    #[expect(
        clippy::too_many_lines,
        reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
    )]
    pub fn start(
        &mut self,
        tunnel: &StandaloneTunnel,
        connection: &Connection,
        password: Option<&secrecy::SecretString>,
        extra_ssh_args: &[String],
    ) -> TunnelManagerResult<()> {
        if self.is_running(tunnel.id) {
            return Err(TunnelManagerError::AlreadyRunning(tunnel.id));
        }

        // Reset reconnect failure counter on manual start
        self.reconnect_failures.remove(&tunnel.id);

        let ProtocolConfig::Ssh(ref ssh_config) = connection.protocol_config else {
            return Err(TunnelManagerError::NotSshConnection(tunnel.connection_id));
        };

        // Build SSH command: ssh -N [-L ...] [-R ...] [-D ...] [options] user@host
        let mut cmd = Command::new("ssh");
        cmd.arg("-N"); // No remote command — just forward

        // Add port forwarding rules
        for pf in &tunnel.forwards {
            let args = pf.to_ssh_arg();
            for arg in &args {
                cmd.arg(arg);
            }
        }

        // Port
        if connection.port != 22 {
            cmd.arg("-p").arg(connection.port.to_string());
        }

        // SSH config args (identity, IdentitiesOnly, proxy, compression, etc.)
        let config_args = ssh_config.build_command_args();
        for arg in &config_args {
            cmd.arg(arg);
        }

        // Extra args from caller (e.g. Flatpak known_hosts)
        for arg in extra_ssh_args {
            cmd.arg(arg);
        }

        // Exit if forwarding fails (e.g. port already in use)
        cmd.arg("-o").arg("ExitOnForwardFailure=yes");

        // Flatpak writable known_hosts
        if let Some(kh_path) = crate::get_flatpak_known_hosts_path() {
            let already_set = config_args.iter().any(|a| a.contains("UserKnownHostsFile"));
            if !already_set {
                cmd.arg("-o")
                    .arg(format!("UserKnownHostsFile={}", kh_path.display()));
            }
        }

        // Password via SSH_ASKPASS or BatchMode
        if let Some(pw) = password {
            if let Ok(script_path) = create_askpass_script() {
                cmd.env("SSH_ASKPASS", &script_path);
                cmd.env("SSH_ASKPASS_REQUIRE", "force");
                cmd.env(ASKPASS_ENV_VAR, pw.expose_secret());
                if std::env::var("DISPLAY").is_err() {
                    cmd.env("DISPLAY", "");
                }
            } else {
                tracing::error!(
                    tunnel = %tunnel.name,
                    "Failed to create SSH_ASKPASS script; falling back to BatchMode"
                );
                cmd.arg("-o").arg("BatchMode=yes");
            }
        } else {
            cmd.arg("-o").arg("BatchMode=yes");
        }

        // Destination: user@host
        let destination = if let Some(ref user) = connection.username {
            format!("{user}@{}", connection.host)
        } else {
            connection.host.clone()
        };
        cmd.arg(&destination);

        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());

        let forwards_desc = tunnel.forwards_summary();
        tracing::info!(
            tunnel_name = %tunnel.name,
            tunnel_id = %tunnel.id,
            destination = %destination,
            forwards = %forwards_desc,
            "Starting standalone SSH tunnel"
        );

        let mut child = cmd.spawn()?;

        // Capture stderr in background thread
        let stderr_output = Arc::new(Mutex::new(String::new()));
        if let Some(stderr_handle) = child.stderr.take() {
            let stderr_buf = stderr_output.clone();
            let tunnel_name = tunnel.name.clone();
            std::thread::spawn(move || {
                use std::io::{BufRead, BufReader};
                let reader = BufReader::new(stderr_handle);
                for line in reader.lines() {
                    match line {
                        Ok(line) => {
                            tracing::warn!(
                                target: "tunnel_manager",
                                tunnel = %tunnel_name,
                                "{}", line
                            );
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

        self.running.insert(
            tunnel.id,
            RunningTunnel {
                child,
                stderr_output,
                status: TunnelStatus::Starting,
            },
        );

        Ok(())
    }

    /// Stops a running tunnel by killing its SSH process
    ///
    /// # Errors
    ///
    /// Returns an error if the tunnel is not found in the running set.
    pub fn stop(&mut self, tunnel_id: Uuid) -> TunnelManagerResult<()> {
        if let Some(mut running) = self.running.remove(&tunnel_id) {
            let _ = running.child.kill();
            let _ = running.child.wait();
            // Reset reconnect failure counter on manual stop
            self.reconnect_failures.remove(&tunnel_id);
            tracing::info!(tunnel_id = %tunnel_id, "Stopped standalone SSH tunnel");
            Ok(())
        } else {
            Err(TunnelManagerError::TunnelNotFound(tunnel_id))
        }
    }

    /// Stops all running tunnels
    pub fn stop_all(&mut self) {
        let ids: Vec<Uuid> = self.running.keys().copied().collect();
        for id in ids {
            let _ = self.stop(id);
        }
    }

    /// Returns the status of a tunnel
    #[must_use]
    pub fn status(&self, tunnel_id: Uuid) -> TunnelStatus {
        self.running
            .get(&tunnel_id)
            .map_or(TunnelStatus::Stopped, |r| r.status.clone())
    }

    /// Returns true if the tunnel is currently running
    #[must_use]
    pub fn is_running(&self, tunnel_id: Uuid) -> bool {
        self.running.contains_key(&tunnel_id)
    }

    /// Returns the number of currently running tunnels
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.running.len()
    }

    /// Returns stderr output from a tunnel (for error diagnostics)
    #[must_use]
    pub fn stderr(&self, tunnel_id: Uuid) -> Option<String> {
        self.running.get(&tunnel_id).map(|r| {
            r.stderr_output
                .lock()
                .map(|s| s.clone())
                .unwrap_or_default()
        })
    }

    /// Performs a health check on all running tunnels.
    ///
    /// Returns a list of tunnel IDs that have exited unexpectedly.
    /// Updates internal status to `Failed` for crashed tunnels and
    /// increments the reconnect failure counter for each failed tunnel.
    pub fn health_check(&mut self) -> Vec<Uuid> {
        let mut failed = Vec::new();

        for (id, running) in &mut self.running {
            match running.child.try_wait() {
                Ok(Some(status)) => {
                    let stderr = running
                        .stderr_output
                        .lock()
                        .map(|s| s.clone())
                        .unwrap_or_default();
                    let msg = if stderr.is_empty() {
                        format!("Process exited with {status}")
                    } else {
                        format!("Process exited with {status}: {}", stderr.trim())
                    };
                    tracing::warn!(
                        tunnel_id = %id,
                        %status,
                        "Standalone tunnel exited unexpectedly"
                    );
                    running.status = TunnelStatus::Failed(msg);
                    // Increment reconnect failure counter
                    let count = self.reconnect_failures.entry(*id).or_insert(0);
                    *count += 1;
                    failed.push(*id);
                }
                Ok(None) => {
                    // Still running — mark as Running if it was Starting
                    if matches!(running.status, TunnelStatus::Starting) {
                        running.status = TunnelStatus::Running;
                    }
                }
                Err(e) => {
                    tracing::error!(tunnel_id = %id, %e, "Failed to check tunnel status");
                }
            }
        }

        // Remove failed tunnels from the running set
        for id in &failed {
            self.running.remove(id);
        }

        failed
    }

    /// Returns the number of consecutive reconnect failures for a tunnel
    #[must_use]
    pub fn reconnect_failure_count(&self, tunnel_id: Uuid) -> u32 {
        self.reconnect_failures
            .get(&tunnel_id)
            .copied()
            .unwrap_or(0)
    }

    /// Returns true if the tunnel has exceeded the maximum reconnect attempts
    #[must_use]
    pub fn exceeded_max_reconnects(&self, tunnel_id: Uuid) -> bool {
        self.reconnect_failure_count(tunnel_id) >= MAX_RECONNECT_ATTEMPTS
    }
}

impl Default for TunnelManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TunnelManager {
    fn drop(&mut self) {
        self.stop_all();
    }
}

/// Environment variable name used to pass the password to the askpass script.
const ASKPASS_ENV_VAR: &str = "_RC_TUN_PW";

/// Creates a temporary `SSH_ASKPASS` helper script that echoes the password.
///
/// # Errors
///
/// Returns a human-readable error string on failure.
fn create_askpass_script() -> Result<std::path::PathBuf, String> {
    use std::io::Write;

    let dir = std::env::temp_dir();
    let path = dir.join(format!("rc_tun_askpass_{}", std::process::id()));

    let script = format!("#!/bin/sh\necho \"${ASKPASS_ENV_VAR}\"\n");

    let mut file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    file.write_all(script.as_bytes())
        .map_err(|e| e.to_string())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }

    Ok(path)
}
