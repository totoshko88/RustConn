//! Standalone SSH tunnel manager
//!
//! Manages headless `ssh -N` processes for port forwarding without
//! terminal sessions. Each tunnel references an existing SSH connection
//! for host/key/password configuration.

use std::collections::HashMap;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

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
    /// The program that carries the tunnel is not installed.
    ///
    /// Separate from `SpawnFailed` because it is the one spawn failure with an
    /// obvious remedy, and because the caller cannot otherwise tell *which*
    /// program is missing: an MPTCP-enabled connection runs `mptcpize`, not
    /// `ssh`, so a message naming `ssh` would send the user to install something
    /// they already have.
    #[error("{program} was not found")]
    ProgramNotFound {
        /// Name of the executable that could not be found on `PATH`.
        program: String,
    },
}

/// A tunnel that exited on its own, with the diagnosis of why.
///
/// `health_check` used to build this message, assign it to the tunnel's status
/// and then drop the whole record in the same call, so the captured stderr never
/// reached a caller. Returning it is what makes the failure reportable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelFailure {
    /// The tunnel that exited.
    pub id: Uuid,
    /// Exit status, and the process's stderr when it wrote any.
    pub reason: String,
}

/// Result type for tunnel manager operations
pub type TunnelManagerResult<T> = Result<T, TunnelManagerError>;

/// A running tunnel process with its metadata
struct RunningTunnel {
    child: Child,
    stderr_output: Arc<Mutex<String>>,
    status: TunnelStatus,
    /// Path to the temporary askpass script, removed when the tunnel is dropped.
    askpass_script: Option<std::path::PathBuf>,
    /// Path to the owner-only file holding the jump-host password, removed when
    /// the tunnel is dropped. The password lives here, not in the child's
    /// environment — the env carries only this path (matching `ssh_tunnel`).
    secret_file: Option<std::path::PathBuf>,
}

impl Drop for RunningTunnel {
    fn drop(&mut self) {
        // Every path that stops a tunnel removes it from `running`, dropping this
        // value — so the askpass script and the plaintext secret file are cleaned
        // up here once, zeroized first, rather than at each call site.
        if let Some(ref path) = self.askpass_script {
            crate::ssh_tunnel::cleanup_askpass_script(path);
        }
        if let Some(ref path) = self.secret_file {
            crate::ssh_tunnel::cleanup_askpass_script(path);
        }
    }
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
    /// Why each tunnel last exited on its own, kept after the process record is
    /// gone so `status` can still answer `Failed`.
    ///
    /// Without this, a tunnel that died reported `Stopped` — indistinguishable
    /// from one the user stopped on purpose — and the `TunnelStatus::Failed`
    /// branch that `tunnel_builder::path_diagram` already draws was unreachable.
    last_failure: HashMap<Uuid, String>,
}

impl TunnelManager {
    /// Creates a new empty tunnel manager
    #[must_use]
    pub fn new() -> Self {
        Self {
            running: HashMap::new(),
            reconnect_failures: HashMap::new(),
            last_failure: HashMap::new(),
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

        // Reset reconnect failure counter on manual start, and clear any
        // recorded exit reason so a restarted tunnel does not keep reporting the
        // failure it recovered from.
        self.reconnect_failures.remove(&tunnel.id);
        self.last_failure.remove(&tunnel.id);

        let ProtocolConfig::Ssh(ref ssh_config) = connection.protocol_config else {
            return Err(TunnelManagerError::NotSshConnection(tunnel.connection_id));
        };

        // Build SSH command: ssh -N [-L ...] [-R ...] [-D ...] [options] user@host
        // Wrap with mptcpize if MPTCP is enabled for this connection.
        let program = tunnel_program(ssh_config.mptcp);
        let mut cmd = if ssh_config.mptcp {
            let mut c = Command::new("mptcpize");
            c.args(["run", "ssh"]);
            c
        } else {
            Command::new("ssh")
        };
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
        if let Some(option) = crate::get_flatpak_known_hosts_path()
            .and_then(|kh| crate::ssh_tunnel::user_known_hosts_option(&kh))
        {
            let already_set = config_args.iter().any(|a| a.contains("UserKnownHostsFile"));
            if !already_set {
                cmd.arg("-o").arg(option);
            }
        }

        // Password via SSH_ASKPASS or BatchMode. The password is written to an
        // owner-only file and only its *path* goes in the environment, so the
        // secret never lands in `/proc/<pid>/environ`. Shared with the embedded
        // tunnel path in `ssh_tunnel`.
        let (askpass_script, secret_file) = if let Some(pw) = password {
            match (
                crate::ssh_tunnel::create_tunnel_askpass_script(),
                crate::ssh_tunnel::create_tunnel_secret_file(pw),
            ) {
                (Ok(script_path), Ok(secret_path)) => {
                    cmd.env("SSH_ASKPASS", &script_path);
                    cmd.env("SSH_ASKPASS_REQUIRE", "force");
                    cmd.env(crate::ssh_tunnel::TUNNEL_ASKPASS_ENV_VAR, &secret_path);
                    if std::env::var("DISPLAY").is_err() {
                        cmd.env("DISPLAY", "");
                    }
                    (Some(script_path), Some(secret_path))
                }
                (script_result, secret_result) => {
                    if let Ok(ref path) = script_result {
                        crate::ssh_tunnel::cleanup_askpass_script(path);
                    }
                    if let Ok(ref path) = secret_result {
                        crate::ssh_tunnel::cleanup_askpass_script(path);
                    }
                    tracing::error!(
                        tunnel = %tunnel.name,
                        "Failed to set up SSH_ASKPASS; falling back to BatchMode"
                    );
                    cmd.arg("-o").arg("BatchMode=yes");
                    (None, None)
                }
            }
        } else {
            cmd.arg("-o").arg("BatchMode=yes");
            (None, None)
        };

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

        let mut child = cmd.spawn().map_err(|e| classify_spawn_error(e, program))?;

        // Capture stderr in background thread
        let stderr_output = Arc::new(Mutex::new(String::new()));
        if let Some(stderr_handle) = child.stderr.take() {
            let stderr_buf = Arc::clone(&stderr_output);
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
                askpass_script,
                secret_file,
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
            // Reset reconnect failure counter on manual stop, and forget any
            // recorded exit reason: a tunnel the user stopped is `Stopped`, not
            // `Failed`, whatever it did before.
            self.reconnect_failures.remove(&tunnel_id);
            self.last_failure.remove(&tunnel_id);
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

    /// Returns the status of a tunnel.
    ///
    /// A tunnel that is not running reports `Failed` when it exited on its own
    /// and `health_check` recorded why, and `Stopped` only when it was never
    /// started or the user stopped it. Both used to report `Stopped`.
    #[must_use]
    pub fn status(&self, tunnel_id: Uuid) -> TunnelStatus {
        if let Some(running) = self.running.get(&tunnel_id) {
            return running.status.clone();
        }
        self.last_failure
            .get(&tunnel_id)
            .map_or(TunnelStatus::Stopped, |reason| {
                TunnelStatus::Failed(reason.clone())
            })
    }

    /// Returns why a tunnel last exited on its own, if it did.
    ///
    /// Cleared when the tunnel is started or stopped deliberately.
    #[must_use]
    pub fn last_failure(&self, tunnel_id: Uuid) -> Option<&str> {
        self.last_failure.get(&tunnel_id).map(String::as_str)
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
    /// Returns the tunnels that have exited unexpectedly, each with the exit
    /// status and whatever the process wrote to stderr. Increments the reconnect
    /// failure counter for each, and records the reason so a later `status` call
    /// still reports `Failed` rather than `Stopped`.
    ///
    /// The reason is returned rather than only stored because the caller is the
    /// only thing that can put it in front of the user; an earlier version built
    /// this message, assigned it to the process record and then removed that
    /// record in the same call, which discarded it.
    pub fn health_check(&mut self) -> Vec<TunnelFailure> {
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
                    running.status = TunnelStatus::Failed(msg.clone());
                    // Increment reconnect failure counter
                    let count = self.reconnect_failures.entry(*id).or_insert(0);
                    *count += 1;
                    failed.push(TunnelFailure {
                        id: *id,
                        reason: msg,
                    });
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

        // Remove failed tunnels from the running set, keeping the reason behind
        // so the tunnel does not silently read as "Stopped" afterwards.
        for failure in &failed {
            self.running.remove(&failure.id);
            self.last_failure.insert(failure.id, failure.reason.clone());
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

/// Returns the program that carries the tunnel.
///
/// `mptcpize run ssh` rather than plain `ssh` when the connection asks for
/// Multipath TCP. Split out from `start` so the choice can be asserted without
/// spawning anything, and because `ProgramNotFound` reports this name.
const fn tunnel_program(mptcp: bool) -> &'static str {
    if mptcp { "mptcpize" } else { "ssh" }
}

/// Turns a spawn failure into the most specific error variant available.
///
/// `NotFound` means the executable is absent, which is the one spawn failure
/// with an obvious remedy, so it does not get lumped in with the rest.
fn classify_spawn_error(error: std::io::Error, program: &str) -> TunnelManagerError {
    if error.kind() == std::io::ErrorKind::NotFound {
        TunnelManagerError::ProgramNotFound {
            program: program.to_string(),
        }
    } else {
        TunnelManagerError::SpawnFailed(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Connection;

    /// The obvious way to test the spawn paths — drop a stub `ssh` in a temp
    /// directory and prepend it to `PATH` — is not available here: the workspace
    /// forbids `std::env::set_var`, which is `unsafe` in Rust 2024 and permitted
    /// only in `rustconn-env-sys` from `main()`. So the spawn paths are covered
    /// by making the real `ssh` fail immediately and offline instead (see
    /// `a_tunnel_that_exits_on_its_own_...`), and everything that does not need a
    /// process is asserted directly.
    fn ssh_is_installed() -> bool {
        // Not `which` — the workspace deliberately does not spawn it (issue #303).
        std::env::var_os("PATH")
            .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join("ssh").is_file()))
    }

    fn ssh_connection() -> Connection {
        Connection::new_ssh("jump".to_string(), "127.0.0.1".to_string(), 22)
    }

    fn tunnel_for(connection: &Connection) -> StandaloneTunnel {
        StandaloneTunnel::new("mysql prod", connection.id)
    }

    #[test]
    fn a_fresh_manager_reports_nothing_running_and_no_failures() {
        let manager = TunnelManager::new();
        let id = Uuid::new_v4();

        assert_eq!(manager.active_count(), 0);
        assert!(!manager.is_running(id));
        assert_eq!(manager.status(id), TunnelStatus::Stopped);
        assert_eq!(manager.last_failure(id), None);
        assert_eq!(manager.reconnect_failure_count(id), 0);
        assert!(!manager.exceeded_max_reconnects(id));
    }

    #[test]
    fn starting_a_non_ssh_connection_is_refused_before_anything_is_spawned() {
        let mut manager = TunnelManager::new();
        let connection = Connection::new_rdp("desktop".to_string(), "10.0.0.5".to_string(), 3389);
        let tunnel = tunnel_for(&connection);

        let error = manager
            .start(&tunnel, &connection, None, &[])
            .expect_err("an RDP connection cannot carry a port forward");

        assert!(matches!(error, TunnelManagerError::NotSshConnection(_)));
        // The point of checking first is that no process was created.
        assert_eq!(manager.active_count(), 0);
        assert!(!manager.is_running(tunnel.id));
    }

    #[test]
    fn stopping_a_tunnel_that_is_not_running_is_an_error() {
        let mut manager = TunnelManager::new();
        let id = Uuid::new_v4();

        let error = manager.stop(id).expect_err("nothing is running");
        assert!(matches!(error, TunnelManagerError::TunnelNotFound(_)));
    }

    #[test]
    fn stop_all_is_harmless_when_nothing_is_running() {
        let mut manager = TunnelManager::new();
        manager.stop_all();
        assert_eq!(manager.active_count(), 0);
    }

    #[test]
    fn the_program_is_mptcpize_only_when_the_connection_asks_for_it() {
        // ProgramNotFound reports this name, so getting it wrong would tell the
        // user to install the wrong package.
        assert_eq!(tunnel_program(false), "ssh");
        assert_eq!(tunnel_program(true), "mptcpize");
    }

    #[test]
    fn a_missing_program_is_classified_apart_from_other_spawn_failures() {
        let missing = classify_spawn_error(
            std::io::Error::from(std::io::ErrorKind::NotFound),
            "mptcpize",
        );
        match missing {
            TunnelManagerError::ProgramNotFound { ref program } => {
                assert_eq!(program, "mptcpize");
                // The name has to reach the message, not just the variant.
                assert!(missing.to_string().contains("mptcpize"));
            }
            other => panic!("expected ProgramNotFound, got {other:?}"),
        }

        let denied = classify_spawn_error(
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            "ssh",
        );
        assert!(matches!(denied, TunnelManagerError::SpawnFailed(_)));
    }

    // The askpass script and secret-file helpers now live in `ssh_tunnel` and
    // are covered by its tests (`tunnel_askpass_script_reads_secret_from_a_file_not_the_env`,
    // `tunnel_secret_file_is_owner_only_and_holds_the_password`). This module
    // reuses them, so the duplicate local tests were removed with the duplicate
    // local function.

    /// Exercises the path the visibility fix is about: a tunnel that starts and
    /// then exits on its own must be reported, and must keep saying so.
    ///
    /// `ssh` is made to fail offline and immediately by handing it a
    /// configuration option that does not exist, so this touches no network and
    /// does not wait for a connect timeout.
    #[test]
    fn a_tunnel_that_exits_on_its_own_is_reported_and_keeps_saying_so() {
        if !ssh_is_installed() {
            return;
        }

        let mut manager = TunnelManager::new();
        let connection = ssh_connection();
        let tunnel = tunnel_for(&connection);

        manager
            .start(
                &tunnel,
                &connection,
                None,
                &["-o".to_string(), "ThisOptionDoesNotExist=yes".to_string()],
            )
            .expect("ssh exists, so the spawn itself succeeds");
        assert!(
            manager.is_running(tunnel.id),
            "the process was just spawned"
        );

        // Poll rather than assume: the child has to be scheduled and exit first.
        let mut failures = Vec::new();
        for _ in 0..100 {
            failures = manager.health_check();
            if !failures.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        let failure = failures
            .first()
            .expect("ssh rejects an unknown option and exits");
        assert_eq!(failure.id, tunnel.id);
        // The reason is what reaches the user, so it must carry ssh's own words
        // rather than only an exit code.
        assert!(
            failure.reason.to_lowercase().contains("option"),
            "reason lost ssh's stderr: {}",
            failure.reason
        );

        // This is the regression the fix is for: the record is gone from the
        // running set, but the tunnel must not read as a deliberate stop.
        assert!(!manager.is_running(tunnel.id));
        assert_eq!(
            manager.status(tunnel.id),
            TunnelStatus::Failed(failure.reason.clone())
        );
        assert_eq!(
            manager.last_failure(tunnel.id),
            Some(failure.reason.as_str())
        );
        assert_eq!(manager.reconnect_failure_count(tunnel.id), 1);
    }

    #[test]
    fn a_recorded_reason_is_what_turns_status_from_stopped_into_failed() {
        // The distinction the fix introduced: not running plus a recorded reason
        // is Failed, not running with no reason is Stopped. Driven through the
        // private map because producing it otherwise needs a live child, which
        // the ssh-backed tests above cover end to end.
        let mut manager = TunnelManager::new();
        let id = Uuid::new_v4();

        assert_eq!(manager.status(id), TunnelStatus::Stopped);

        manager
            .last_failure
            .insert(id, "Process exited with 255".to_string());

        assert_eq!(
            manager.status(id),
            TunnelStatus::Failed("Process exited with 255".to_string())
        );
        // Still not running: Failed describes how it ended, not a live process.
        assert!(!manager.is_running(id));
        assert_eq!(manager.active_count(), 0);
    }

    #[test]
    fn a_deliberate_stop_clears_the_recorded_reason() {
        if !ssh_is_installed() {
            return;
        }

        // A tunnel the user stopped is Stopped whatever it did before, so the
        // warning icon must not survive a deliberate stop. This goes through the
        // real `stop`, which only reaches its clearing code when something was
        // actually running.
        let mut manager = TunnelManager::new();
        let connection = ssh_connection();
        let tunnel = tunnel_for(&connection);

        manager
            .start(&tunnel, &connection, None, &[])
            .expect("ssh exists, so the spawn itself succeeds");
        manager
            .last_failure
            .insert(tunnel.id, "Process exited with 255".to_string());

        manager.stop(tunnel.id).expect("it is running");

        assert_eq!(manager.last_failure(tunnel.id), None);
        assert_eq!(manager.status(tunnel.id), TunnelStatus::Stopped);
    }

    #[test]
    fn restarting_clears_a_recorded_failure_before_it_can_be_shown_again() {
        if !ssh_is_installed() {
            return;
        }

        let mut manager = TunnelManager::new();
        let connection = ssh_connection();
        let tunnel = tunnel_for(&connection);
        manager
            .last_failure
            .insert(tunnel.id, "Process exited with 255".to_string());
        manager.reconnect_failures.insert(tunnel.id, 3);

        manager
            .start(&tunnel, &connection, None, &[])
            .expect("ssh exists, so the spawn itself succeeds");

        assert_eq!(manager.last_failure(tunnel.id), None);
        assert_eq!(manager.reconnect_failure_count(tunnel.id), 0);

        let _ = manager.stop(tunnel.id);
    }

    #[test]
    fn a_running_tunnel_refuses_a_second_start() {
        if !ssh_is_installed() {
            return;
        }

        let mut manager = TunnelManager::new();
        let connection = ssh_connection();
        let tunnel = tunnel_for(&connection);

        manager
            .start(&tunnel, &connection, None, &[])
            .expect("ssh exists, so the spawn itself succeeds");

        let error = manager
            .start(&tunnel, &connection, None, &[])
            .expect_err("one tunnel is one process");
        assert!(matches!(error, TunnelManagerError::AlreadyRunning(_)));
        assert_eq!(manager.active_count(), 1);

        manager.stop(tunnel.id).expect("it is running");
        assert_eq!(manager.active_count(), 0);
        assert_eq!(manager.status(tunnel.id), TunnelStatus::Stopped);
    }
}
