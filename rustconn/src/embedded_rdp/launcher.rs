//! Safe FreeRDP launcher with Qt error suppression
//!
//! This module provides the `SafeFreeRdpLauncher` struct for launching FreeRDP
//! with environment variables set to suppress Qt/Wayland warnings.

use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use secrecy::{ExposeSecret, SecretString};

use super::types::{EmbeddedRdpError, RdpConfig};

/// Shared buffer collecting stderr lines from the external FreeRDP process.
///
/// Used by [`arm_external_exit_watchdog`](super::connection) to produce
/// user-friendly error messages (e.g. "authentication failed") instead of
/// the generic "client exited unexpectedly" toast.
pub(crate) type StderrLines = Arc<Mutex<Vec<String>>>;

/// Shared buffer collecting stdout lines from the external FreeRDP process.
///
/// Used to detect interactive certificate prompts that FreeRDP prints to stdout
/// when using `/cert:tofu` and the server certificate has changed. Without this,
/// the process hangs waiting for stdin input that never comes.
pub(crate) type StdoutLines = Arc<Mutex<Vec<String>>>;

/// Shared result returned by a background FreeRDP launch.
pub(crate) type FreeRdpLaunchResult = Result<(Child, StderrLines, StdoutLines), EmbeddedRdpError>;

/// Maximum time to reap a FreeRDP process after requesting termination.
const CANCELLED_CHILD_REAP_TIMEOUT: Duration = Duration::from_millis(500);
/// Poll interval keeps child cleanup bounded without busy-waiting.
const CANCELLED_CHILD_POLL_INTERVAL: Duration = Duration::from_millis(10);

fn kill_and_reap_child(mut child: Child) {
    if let Err(error) = child.kill() {
        tracing::debug!(protocol = "rdp", %error, "FreeRDP process exited before cancellation");
    }
    let deadline = Instant::now() + CANCELLED_CHILD_REAP_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(CANCELLED_CHILD_POLL_INTERVAL);
            }
            Ok(None) => {
                tracing::warn!(
                    protocol = "rdp",
                    "FreeRDP did not exit promptly after kill; reaping in background"
                );
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return;
            }
            Err(error) => {
                tracing::warn!(protocol = "rdp", %error, "Failed to poll cancelled FreeRDP process");
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return;
            }
        }
    }
}

pub(crate) fn cleanup_child_without_blocking(child: Child) {
    std::thread::spawn(move || kill_and_reap_child(child));
}

struct PendingFreeRdpLaunch {
    result: Option<FreeRdpLaunchResult>,
}

impl PendingFreeRdpLaunch {
    fn new(result: FreeRdpLaunchResult) -> Self {
        Self {
            result: Some(result),
        }
    }

    fn into_result(mut self) -> FreeRdpLaunchResult {
        self.result
            .take()
            .expect("pending launch result must exist until delivery")
    }
}

impl Drop for PendingFreeRdpLaunch {
    fn drop(&mut self) {
        if let Some(Ok((child, _, _))) = self.result.take() {
            cleanup_child_without_blocking(child);
        }
    }
}

/// Drop-cancelling handle for a background FreeRDP launch.
pub(crate) struct FreeRdpLaunchHandle {
    receiver: std::sync::mpsc::Receiver<PendingFreeRdpLaunch>,
    cancellation: Arc<AtomicBool>,
}

impl FreeRdpLaunchHandle {
    pub(crate) fn try_recv(&self) -> Result<FreeRdpLaunchResult, std::sync::mpsc::TryRecvError> {
        self.receiver
            .try_recv()
            .map(PendingFreeRdpLaunch::into_result)
    }
}

impl Drop for FreeRdpLaunchHandle {
    fn drop(&mut self) {
        self.cancellation.store(true, Ordering::Release);
    }
}

// FreeRDP variants may open `/args-from:` asynchronously after spawn, so keep
// the credential file alive briefly while still guaranteeing prompt cleanup.
const ARGS_FILE_CLEANUP_DELAY: std::time::Duration = std::time::Duration::from_secs(1);

/// Prepared `/args-from:` switch plus the cleanup guard backing its file.
pub(crate) struct PreparedFreeRdpArgs {
    argument: String,
    _guard: super::ephemeral_args::EphemeralRdpArgs,
}

impl PreparedFreeRdpArgs {
    /// Returns the sole command-line argument FreeRDP should receive.
    pub(crate) fn argument(&self) -> &str {
        &self.argument
    }

    /// Retains the args file while FreeRDP asynchronously opens it after spawn.
    pub(crate) fn retain_for_post_spawn_parse(self) {
        std::thread::spawn(move || {
            std::thread::sleep(ARGS_FILE_CLEANUP_DELAY);
            drop(self);
        });
    }
}

/// Safe FreeRDP launcher with Qt error suppression
///
/// This struct provides methods to launch FreeRDP with environment variables
/// set to suppress Qt/Wayland warnings that can cause issues when mixing
/// Qt-based FreeRDP with GTK4 applications.
pub struct SafeFreeRdpLauncher {
    /// Whether to suppress Qt warnings
    pub(crate) suppress_qt_warnings: bool,
    /// Whether to force X11 backend
    pub(crate) force_x11: bool,
}

impl SafeFreeRdpLauncher {
    /// Creates a new launcher with Wayland-first defaults
    ///
    /// By default, uses native Wayland backend. Use `with_x11_fallback()`
    /// if you need X11 compatibility for older FreeRDP versions.
    #[must_use]
    pub fn new() -> Self {
        Self {
            suppress_qt_warnings: true,
            force_x11: false, // Wayland-first approach
        }
    }

    /// Creates a launcher that forces X11 backend (for compatibility)
    ///
    /// Use this when Wayland backend causes issues with specific FreeRDP versions.
    #[must_use]
    pub fn with_x11_fallback() -> Self {
        Self {
            suppress_qt_warnings: true,
            force_x11: true,
        }
    }

    /// Sets whether to suppress Qt warnings
    #[must_use]
    pub const fn with_suppress_warnings(mut self, suppress: bool) -> Self {
        self.suppress_qt_warnings = suppress;
        self
    }

    /// Sets whether to force X11 backend for FreeRDP
    #[must_use]
    pub const fn with_force_x11(mut self, force: bool) -> Self {
        self.force_x11 = force;
        self
    }

    /// Builds the environment variables for Qt suppression
    pub(crate) fn build_env(&self) -> Vec<(&'static str, &'static str)> {
        let mut env = Vec::new();

        if self.suppress_qt_warnings {
            // Suppress Qt/Wayland warnings
            env.push(("QT_LOGGING_RULES", "qt.qpa.wayland=false;qt.qpa.*=false"));
        }

        if self.force_x11 {
            // Force X11 backend to avoid Wayland-specific issues
            env.push(("QT_QPA_PLATFORM", "xcb"));
        }

        env
    }

    /// Resolves args-file syntax, then creates a guarded credential file.
    ///
    /// `binary` must be the original target, including any `host:` marker.
    ///
    /// # Errors
    /// Returns an initialization error if the args file cannot be prepared.
    pub(crate) fn prepare_args_file(
        binary: &str,
        plain_args: &[String],
        secret_args: &[(&str, &SecretString)],
    ) -> Result<PreparedFreeRdpArgs, EmbeddedRdpError> {
        Self::prepare_args_file_with_cancel(binary, plain_args, secret_args, None)
    }

    fn prepare_args_file_with_cancel(
        binary: &str,
        plain_args: &[String],
        secret_args: &[(&str, &SecretString)],
        cancellation: Option<&AtomicBool>,
    ) -> Result<PreparedFreeRdpArgs, EmbeddedRdpError> {
        // Probe before opening the credential file so a hung version command
        // cannot extend the on-disk secret lifetime.
        let form = super::detect::resolve_args_from_form_with_cancel(binary, cancellation);
        Self::ensure_not_cancelled(cancellation)?;
        let guard = super::ephemeral_args::EphemeralRdpArgs::write_all(plain_args, secret_args)
            .map_err(|error| {
                EmbeddedRdpError::FreeRdpInit(format!("could not prepare RDP args file: {error}"))
            })?;
        Self::ensure_not_cancelled(cancellation)?;
        let argument = super::detect::args_from_argument_for_form(form, guard.path());
        Ok(PreparedFreeRdpArgs {
            argument,
            _guard: guard,
        })
    }

    fn ensure_not_cancelled(cancellation: Option<&AtomicBool>) -> Result<(), EmbeddedRdpError> {
        if cancellation.is_some_and(|flag| flag.load(Ordering::Acquire)) {
            Err(EmbeddedRdpError::FreeRdpInit(
                "FreeRDP launch cancelled".to_string(),
            ))
        } else {
            Ok(())
        }
    }

    /// Launches xfreerdp with Qt error suppression
    ///
    /// # Arguments
    ///
    /// * `config` - The RDP connection configuration
    ///
    /// # Returns
    ///
    /// The spawned child process.
    ///
    /// # Errors
    ///
    /// Returns error if FreeRDP cannot be launched.
    pub fn launch(
        &self,
        config: &RdpConfig,
    ) -> Result<(Child, StderrLines, StdoutLines), EmbeddedRdpError> {
        self.launch_with_cancel(config, None)
    }

    fn launch_with_cancel(
        &self,
        config: &RdpConfig,
        cancellation: Option<&AtomicBool>,
    ) -> Result<(Child, StderrLines, StdoutLines), EmbeddedRdpError> {
        Self::ensure_not_cancelled(cancellation)?;
        super::ephemeral_args::EphemeralRdpArgs::validate_plain_args(&config.extra_args).map_err(
            |error| {
                EmbeddedRdpError::FreeRdpInit(format!(
                    "could not validate additional RDP arguments: {error}"
                ))
            },
        )?;
        // RemoteApp (RAIL) is not supported by wlfreerdp — it requires a window
        // manager that can create individual app windows. Use xfreerdp/sdl-freerdp.
        let is_remote_app = config
            .remote_app_program
            .as_ref()
            .is_some_and(|p| !p.is_empty());

        let binary = if is_remote_app {
            super::detect::detect_best_freerdp_for_remoteapp_with_cancel(cancellation)
        } else {
            super::detect::detect_best_freerdp_with_cancel(cancellation)
        };
        Self::ensure_not_cancelled(cancellation)?;
        let binary = binary.ok_or_else(|| {
            EmbeddedRdpError::FreeRdpInit(
                "No FreeRDP client found. Install sdl-freerdp3, xfreerdp, or wlfreerdp."
                    .to_string(),
            )
        })?;

        // Keep the original target (including a `host:` marker) for version
        // probing and cache identity. The stripped name is only for spawning.
        let (actual_binary, via_host) = if let Some(host_bin) = binary.strip_prefix("host:") {
            (host_bin.to_string(), true)
        } else {
            (binary.clone(), false)
        };

        let mut cmd = if via_host {
            let mut c = Command::new("flatpak-spawn");
            c.args(["--host", "--watch-bus"]);
            // Pass environment variables via flatpak-spawn --env
            for (key, value) in self.build_env() {
                c.arg(format!("--env={key}={value}"));
            }
            // Pass display environment so xfreerdp can open a window on host
            if let Ok(display) = std::env::var("DISPLAY") {
                c.arg(format!("--env=DISPLAY={display}"));
            }
            if let Ok(wayland) = std::env::var("WAYLAND_DISPLAY") {
                c.arg(format!("--env=WAYLAND_DISPLAY={wayland}"));
            }
            if let Ok(xdg_runtime) = std::env::var("XDG_RUNTIME_DIR") {
                c.arg(format!("--env=XDG_RUNTIME_DIR={xdg_runtime}"));
            }
            c.arg(&actual_binary);
            c
        } else {
            Command::new(&actual_binary)
        };

        // Set environment to suppress Qt warnings
        // (only for non-host mode; host mode passes env via flatpak-spawn --env above)
        if !via_host {
            for (key, value) in self.build_env() {
                cmd.env(key, value);
            }
        }

        // Session password is always passed via a single-use args file (mode
        // 0600) consumed by `/args-from:`, placed in the shared user-private
        // runtime directory (`$XDG_RUNTIME_DIR` on Linux, `$TMPDIR` on macOS —
        // see `rustconn_core::secret_file_dir`).
        //
        // FreeRDP (PR #12697) requires `/args-from:` to be the ONLY argument on
        // the command line — it cannot be combined with other CLI arguments.
        // All connection parameters (including secrets) are therefore written
        // into the file, one per line. This also improves security: nothing is
        // visible in `/proc/<pid>/cmdline`.
        //
        // The exact spelling of the switch depends on the installed FreeRDP
        // version, see [`super::detect::args_from_argument`].
        //
        // The RD Gateway no longer needs a separate secret here: FreeRDP
        // reuses the session credentials (`/u:`, `/d:` and the `/p:` from
        // the args file) for the gateway, matching the working manual command
        // `xfreerdp /gateway:g:HOST /u:NAME /d:DOMAIN`. The previous `/gp:`
        // path used a FreeRDP 2.x alias that FreeRDP 3.x rejects (issue #187).
        let session_password = config
            .password
            .as_ref()
            .filter(|p| !p.expose_secret().is_empty());

        let mut secret_args: Vec<(&str, &SecretString)> = Vec::new();
        if let Some(p) = session_password {
            secret_args.push(("p", p));
        }

        // Collect all plain-text connection arguments into a Vec<String>
        let plain_args = Self::build_connection_args(config);

        let prepared_args =
            Self::prepare_args_file_with_cancel(&binary, &plain_args, &secret_args, cancellation)?;
        cmd.arg(prepared_args.argument());

        // Capture stderr instead of discarding it. The real FreeRDP failure
        // reason (authentication failure, rejected certificate, missing codec,
        // wrong display backend) is printed to stderr — silencing it made
        // blank-screen / auto-close reports impossible to diagnose remotely.
        // Qt/Wayland noise is already filtered via QT_LOGGING_RULES. (See #177)
        cmd.stderr(Stdio::piped());

        // Capture stdout to detect interactive certificate prompts. FreeRDP with
        // `/cert:tofu` prints "Do you trust the above certificate? (Y/T/N)" to
        // stdout when a server certificate changes. Without stdin connected to a
        // TTY, the process hangs waiting for input. By capturing stdout we can
        // detect this prompt, terminate the process, and show a GUI dialog. (#324)
        cmd.stdout(Stdio::piped());

        // Log the chosen binary and full argument vector (the password is sent
        // via stdin / args-file, never on argv, so this is safe to log).
        tracing::debug!(
            protocol = "rdp",
            binary = %actual_binary,
            via_host,
            host = %config.host,
            port = config.port,
            command = ?cmd,
            "[FreeRDP] Launching external client"
        );

        Self::ensure_not_cancelled(cancellation)?;
        let mut child = cmd
            .spawn()
            .map_err(|e| EmbeddedRdpError::FreeRdpInit(e.to_string()))?;
        if let Err(error) = Self::ensure_not_cancelled(cancellation) {
            kill_and_reap_child(child);
            return Err(error);
        }

        // Drain the client's stderr on a background thread and forward every
        // non-empty line to `tracing`. Also accumulate lines in a shared buffer
        // so that the exit watchdog can classify the failure (auth vs cert vs
        // generic) and present a meaningful message to the user.
        let stderr_lines: StderrLines = Arc::new(Mutex::new(Vec::new()));
        if let Some(stderr) = child.stderr.take() {
            let client = actual_binary.clone();
            let lines_clone = Arc::clone(&stderr_lines);
            std::thread::spawn(move || {
                use std::io::{BufRead, BufReader};
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        tracing::warn!(protocol = "rdp", client = %client, "[FreeRDP] {trimmed}");
                        if let Ok(mut buf) = lines_clone.lock() {
                            buf.push(trimmed.to_owned());
                        }
                    }
                }
            });
        }

        // Drain stdout similarly. FreeRDP prints certificate prompts and other
        // interactive messages to stdout. We accumulate them so the watchdog can
        // detect when the process is waiting for a certificate confirmation.
        let stdout_lines: StdoutLines = Arc::new(Mutex::new(Vec::new()));
        if let Some(stdout) = child.stdout.take() {
            let client = actual_binary.clone();
            let lines_clone = Arc::clone(&stdout_lines);
            std::thread::spawn(move || {
                use std::io::{BufRead, BufReader};
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        tracing::debug!(protocol = "rdp", client = %client, "[FreeRDP stdout] {trimmed}");
                        if let Ok(mut buf) = lines_clone.lock() {
                            buf.push(trimmed.to_owned());
                        }
                    }
                }
            });
        }

        if let Err(error) = Self::ensure_not_cancelled(cancellation) {
            kill_and_reap_child(child);
            return Err(error);
        }

        // Keep the credential file alive briefly after spawn. Some FreeRDP
        // variants open `/args-from:` asynchronously, so unlinking immediately
        // can race their argument parser. The guard still guarantees cleanup.
        prepared_args.retain_for_post_spawn_parse();

        Ok((child, stderr_lines, stdout_lines))
    }

    /// Runs FreeRDP detection, version probing, args-file creation, and spawn
    /// away from the GTK main thread.
    pub(crate) fn launch_background(self, config: RdpConfig) -> FreeRdpLaunchHandle {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let cancellation = Arc::new(AtomicBool::new(false));
        let worker_cancellation = Arc::clone(&cancellation);
        std::thread::spawn(move || {
            let result = self.launch_with_cancel(&config, Some(&worker_cancellation));
            let pending = PendingFreeRdpLaunch::new(result);
            if worker_cancellation.load(Ordering::Acquire) {
                drop(pending);
                return;
            }
            // Failed delivery drops the pending guard, which kills and reaps
            // any process that the stale GTK continuation no longer owns.
            let _ = sender.send(pending);
        });
        FreeRdpLaunchHandle {
            receiver,
            cancellation,
        }
    }

    /// Detects the best available FreeRDP binary (Wayland-first)
    ///
    /// Delegates to the unified detection in [`super::detect::detect_best_freerdp`].
    pub fn detect_freerdp() -> Option<String> {
        super::detect::detect_best_freerdp()
    }

    /// Detects the best FreeRDP binary for RemoteApp (RAIL) sessions.
    ///
    /// `wlfreerdp` does not support RAIL — it renders a full desktop into a
    /// Wayland subsurface and cannot create individual application windows.
    /// This method skips `wl*` variants and prefers `xfreerdp3`/`sdl-freerdp3`.
    pub fn detect_freerdp_for_remoteapp() -> Option<String> {
        super::detect::detect_best_freerdp_for_remoteapp()
    }

    /// Translates the embedded widget's configuration into the shared
    /// [`FreeRdpConfig`](rustconn_core::protocol::FreeRdpConfig).
    ///
    /// Note what is *not* carried over: `width`/`height`. Those are the embedded
    /// viewer's own geometry, measured from its `DrawingArea` in logical pixels,
    /// and a standalone FreeRDP window is not that widget. Feeding them to the
    /// external client is what opened a FullHD-sized window on a 4K display.
    /// The window is sized by `external_display_mode` instead, which defaults to
    /// filling the monitor.
    pub(crate) fn external_config(config: &RdpConfig) -> rustconn_core::protocol::FreeRdpConfig {
        rustconn_core::protocol::FreeRdpConfig {
            host: config.host.clone(),
            port: config.port,
            username: config.username.clone(),
            // The password is passed as a secret arg via EphemeralRdpArgs, never
            // in the plain-text argument vector, so the builder ignores it.
            password: None,
            domain: config.domain.clone(),
            display_mode: config.external_display_mode,
            resolution: config.external_resolution.clone(),
            scale_override: config.scale_override,
            system_scale_percent: config.system_scale_percent,
            color_depth: config.color_depth,
            clipboard_enabled: config.clipboard_enabled,
            shared_folders: config
                .shared_folders
                .iter()
                .map(|folder| rustconn_core::protocol::freerdp::SharedFolder {
                    local_path: folder.local_path.clone(),
                    share_name: folder.share_name.clone(),
                })
                .collect(),
            printer_enabled: config.printer_enabled,
            audio_mode: config.audio_mode,
            gateway: config.gateway_hostname.as_ref().and_then(|hostname| {
                (!hostname.is_empty()).then(|| rustconn_core::models::RdpGateway {
                    hostname: hostname.clone(),
                    port: config.gateway_port,
                    username: config.gateway_username.clone(),
                })
            }),
            remote_app_program: config.remote_app_program.clone(),
            remote_app_args: config.remote_app_args.clone(),
            remote_app_name: config.remote_app_name.clone(),
            security_layer: config.security_layer,
            tls_security_level: config.tls_security_level,
            disable_nla: config.disable_nla,
            extra_args: config.extra_args.clone(),
            window_geometry: config.window_geometry.map(|(x, y, width, height)| {
                rustconn_core::models::WindowGeometry::new(x, y, width, height)
            }),
            remember_window_position: config.remember_window_position,
            ignore_certificate: config.ignore_certificate,
        }
    }

    /// Builds the full list of connection arguments as owned strings.
    ///
    /// FreeRDP requires all arguments to be in the `/args-from:` file. This
    /// method collects them into a `Vec<String>` so they can be written to the
    /// ephemeral args file by [`EphemeralRdpArgs::write_all`].
    ///
    /// The argument list itself is built by
    /// [`rustconn_core::protocol::build_freerdp_args`], shared with the
    /// `External` client mode. This method used to hand-write its own list; the
    /// two drifted, and only this one emitted `/gateway:` or sanitised the
    /// user's extra arguments.
    pub fn build_connection_args(config: &RdpConfig) -> Vec<String> {
        rustconn_core::protocol::build_freerdp_args(&Self::external_config(config))
    }
}

impl Default for SafeFreeRdpLauncher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sleeping_child() -> Child {
        Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn test child")
    }

    fn assert_process_reaped(pid: u32) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let proc_path = std::path::PathBuf::from(format!("/proc/{pid}"));
        while proc_path.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!proc_path.exists(), "cancelled child {pid} was not reaped");
    }

    fn pending_child(child: Child) -> PendingFreeRdpLaunch {
        PendingFreeRdpLaunch::new(Ok((
            child,
            Arc::new(Mutex::new(Vec::new())),
            Arc::new(Mutex::new(Vec::new())),
        )))
    }

    #[test]
    fn background_handle_drop_requests_cancellation() {
        let (_sender, receiver) = std::sync::mpsc::sync_channel(1);
        let cancellation = Arc::new(AtomicBool::new(false));
        let handle = FreeRdpLaunchHandle {
            receiver,
            cancellation: Arc::clone(&cancellation),
        };
        drop(handle);
        assert!(cancellation.load(Ordering::Acquire));
    }

    #[test]
    fn dropped_pending_delivery_kills_and_reaps_child() {
        let child = sleeping_child();
        let pid = child.id();
        drop(pending_child(child));
        assert_process_reaped(pid);
    }

    #[test]
    fn failed_result_delivery_kills_and_reaps_child() {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        drop(receiver);
        let child = sleeping_child();
        let pid = child.id();
        let error = sender
            .send(pending_child(child))
            .expect_err("delivery must fail after receiver drop");
        drop(error);
        assert_process_reaped(pid);
    }

    #[test]
    fn test_safe_freerdp_launcher_default_wayland_first() {
        let launcher = SafeFreeRdpLauncher::new();
        assert!(launcher.suppress_qt_warnings);
        assert!(!launcher.force_x11); // Wayland-first by default
    }

    #[test]
    fn test_safe_freerdp_launcher_x11_fallback() {
        let launcher = SafeFreeRdpLauncher::with_x11_fallback();
        assert!(launcher.suppress_qt_warnings);
        assert!(launcher.force_x11);
    }

    #[test]
    fn test_safe_freerdp_launcher_builder() {
        let launcher = SafeFreeRdpLauncher::new()
            .with_suppress_warnings(false)
            .with_force_x11(true);
        assert!(!launcher.suppress_qt_warnings);
        assert!(launcher.force_x11);
    }

    #[test]
    fn test_safe_freerdp_launcher_env_wayland() {
        let launcher = SafeFreeRdpLauncher::new();
        let env = launcher.build_env();

        // Should have QT_LOGGING_RULES but NOT QT_QPA_PLATFORM (Wayland-first)
        assert!(env.iter().any(|(k, _)| *k == "QT_LOGGING_RULES"));
        assert!(!env.iter().any(|(k, _)| *k == "QT_QPA_PLATFORM"));
    }

    #[test]
    fn test_safe_freerdp_launcher_env_x11_fallback() {
        let launcher = SafeFreeRdpLauncher::with_x11_fallback();
        let env = launcher.build_env();

        // Should have both QT_LOGGING_RULES and QT_QPA_PLATFORM
        assert!(env.iter().any(|(k, _)| *k == "QT_LOGGING_RULES"));
        assert!(env.iter().any(|(k, _)| *k == "QT_QPA_PLATFORM"));
    }

    #[test]
    fn test_safe_freerdp_launcher_env_disabled() {
        let launcher = SafeFreeRdpLauncher::new()
            .with_suppress_warnings(false)
            .with_force_x11(false);
        let env = launcher.build_env();

        // Should be empty when both are disabled
        assert!(env.is_empty());
    }

    /// Collects connection arguments from the production builder.
    fn connection_args(config: &RdpConfig) -> Vec<String> {
        SafeFreeRdpLauncher::build_connection_args(config)
    }

    #[test]
    fn connection_args_filter_secret_aliases_and_execution_options() {
        let config = RdpConfig {
            extra_args: vec![
                "/gateway:g:host,p:session-secret".to_string(),
                "/gateway:g:host,GATEWAY-PASSWORD:gateway-secret".to_string(),
                "--pth".to_string(),
                "hash-secret".to_string(),
                "--shell".to_string(),
                "command-secret".to_string(),
                "/gateway:g:host,u:user".to_string(),
            ],
            ..RdpConfig::default()
        };

        let args = connection_args(&config);

        assert!(args.iter().any(|arg| arg == "/gateway:g:host,u:user"));
        for secret in [
            "session-secret",
            "gateway-secret",
            "hash-secret",
            "command-secret",
        ] {
            assert!(args.iter().all(|arg| !arg.contains(secret)));
        }
    }

    #[test]
    fn test_gateway_uses_freerdp3_unified_syntax() {
        // Regression for #187: FreeRDP 3.x rejects the old `/g:` / `/gu:`
        // aliases with "Unexpected keyword", aborting the connection.
        let config = RdpConfig {
            host: "vm1.example.com".to_string(),
            gateway_hostname: Some("gw.example.com".to_string()),
            gateway_port: 443,
            ..RdpConfig::default()
        };

        let args = connection_args(&config);
        assert!(
            args.iter().any(|a| a == "/gateway:g:gw.example.com:443"),
            "expected unified /gateway: option, got {args:?}"
        );
        assert!(
            !args.iter().any(|a| a.starts_with("/g:")),
            "the removed /g: alias must not be emitted: {args:?}"
        );
        assert!(
            !args.iter().any(|a| a.starts_with("/gu:")),
            "the removed /gu: alias must not be emitted: {args:?}"
        );
    }

    #[test]
    fn test_gateway_omits_user_when_same_as_session() {
        // The reported connection had gateway user == session user; FreeRDP
        // then reuses the session credentials, so no explicit gateway user is
        // needed (matching the working `xfreerdp /gateway:g:HOST` command).
        let config = RdpConfig {
            host: "vm1.example.com".to_string(),
            username: Some("alice".to_string()),
            gateway_hostname: Some("gw.example.com".to_string()),
            gateway_username: Some("alice".to_string()),
            ..RdpConfig::default()
        };

        let args = connection_args(&config);
        assert!(
            args.iter().any(|a| a == "/gateway:g:gw.example.com:443"),
            "expected bare gateway option, got {args:?}"
        );
    }

    #[test]
    fn test_gateway_adds_user_when_distinct() {
        let config = RdpConfig {
            host: "vm1.example.com".to_string(),
            username: Some("alice".to_string()),
            gateway_hostname: Some("gw.example.com".to_string()),
            gateway_username: Some("gwadmin".to_string()),
            ..RdpConfig::default()
        };

        let args = connection_args(&config);
        assert!(
            args.iter()
                .any(|a| a == "/gateway:g:gw.example.com:443,u:gwadmin"),
            "expected gateway option with distinct user, got {args:?}"
        );
    }
}
