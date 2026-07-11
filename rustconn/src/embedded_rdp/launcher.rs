//! Safe FreeRDP launcher with Qt error suppression
//!
//! This module provides the `SafeFreeRdpLauncher` struct for launching FreeRDP
//! with environment variables set to suppress Qt/Wayland warnings.

use super::types::{EmbeddedRdpError, RdpConfig};
use secrecy::{ExposeSecret, SecretString};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

/// Shared buffer collecting stderr lines from the external FreeRDP process.
///
/// Used by [`arm_external_exit_watchdog`](super::connection) to produce
/// user-friendly error messages (e.g. "authentication failed") instead of
/// the generic "client exited unexpectedly" toast.
pub(crate) type StderrLines = Arc<Mutex<Vec<String>>>;

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
    pub fn launch(&self, config: &RdpConfig) -> Result<(Child, StderrLines), EmbeddedRdpError> {
        // RemoteApp (RAIL) is not supported by wlfreerdp — it requires a window
        // manager that can create individual app windows. Use xfreerdp/sdl-freerdp.
        let is_remote_app = config
            .remote_app_program
            .as_ref()
            .is_some_and(|p| !p.is_empty());

        let binary = if is_remote_app {
            Self::detect_freerdp_for_remoteapp()
        } else {
            Self::detect_freerdp()
        }
        .ok_or_else(|| {
            EmbeddedRdpError::FreeRdpInit(
                "No FreeRDP client found. Install sdl-freerdp3, xfreerdp, or wlfreerdp."
                    .to_string(),
            )
        })?;

        // Check if we need to launch via flatpak-spawn --host
        let (actual_binary, via_host) = if let Some(host_bin) = binary.strip_prefix("host:") {
            (host_bin.to_string(), true)
        } else {
            (binary, false)
        };

        let mut cmd = if via_host {
            let mut c = Command::new("flatpak-spawn");
            c.arg("--host");
            // Forward stdin for password passing via /from-stdin
            c.arg("--forward-fd=0");
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

        // The RemoteApp session password must never appear on argv
        // (`/proc/<pid>/cmdline`): `/from-stdin` does not work for RAIL
        // sessions, so it is written into a single-use args file in
        // $XDG_RUNTIME_DIR (mode 0600) and consumed via `/args-from:file:`.
        // The guard removes the file when this function returns, even on the
        // error path.
        //
        // The RD Gateway no longer needs a separate secret here: FreeRDP reuses
        // the session credentials (the `/u:`, `/d:` and `/from-stdin` password)
        // for the gateway, matching the working manual command
        // `xfreerdp /gateway:g:HOST /u:NAME /d:DOMAIN`. The previous `/gp:` path
        // used a FreeRDP 2.x alias that FreeRDP 3.x rejects (issue #187).
        let session_password = config
            .password
            .as_ref()
            .filter(|p| !p.expose_secret().is_empty());

        let mut secret_args: Vec<(&str, &SecretString)> = Vec::new();
        if is_remote_app && let Some(p) = session_password {
            secret_args.push(("p", p));
        }

        let _password_guard = if secret_args.is_empty() {
            None
        } else {
            match super::ephemeral_args::EphemeralRdpArgs::write_args(&secret_args) {
                Ok(guard) => {
                    cmd.arg(format!("/args-from:file:{}", guard.path().display()));
                    Some(guard)
                }
                Err(e) => {
                    return Err(EmbeddedRdpError::FreeRdpInit(format!(
                        "could not prepare RDP credentials file: {e}"
                    )));
                }
            }
        };

        // Build connection arguments
        Self::add_connection_args(&mut cmd, config);

        // Capture stderr instead of discarding it. The real FreeRDP failure
        // reason (authentication failure, rejected certificate, missing codec,
        // wrong display backend) is printed to stderr — silencing it made
        // blank-screen / auto-close reports impossible to diagnose remotely.
        // Qt/Wayland noise is already filtered via QT_LOGGING_RULES. (See #177)
        cmd.stderr(Stdio::piped());

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

        let mut child = cmd
            .spawn()
            .map_err(|e| EmbeddedRdpError::FreeRdpInit(e.to_string()))?;

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

        // Write password via stdin when /from-stdin is used (i.e. for
        // non-RemoteApp sessions; RemoteApp gets the password via the
        // ephemeral args file above).
        if !is_remote_app
            && let Some(ref password) = config.password
            && !password.expose_secret().is_empty()
            && let Some(mut stdin) = child.stdin.take()
        {
            use std::io::Write;
            // FreeRDP 3.x /from-stdin expects: domain\npassword\n
            // Send empty domain line first, then password
            let domain = config.domain.as_deref().unwrap_or("");
            let _ = writeln!(stdin, "{domain}");
            let _ = writeln!(stdin, "{}", password.expose_secret());
        }

        // _password_guard is dropped here, removing the temp args file.
        // FreeRDP keeps the file mapped only briefly during argument
        // parsing, so removing it shortly after spawn is safe.
        Ok((child, stderr_lines))
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

    /// Adds connection arguments to the command
    pub fn add_connection_args(cmd: &mut Command, config: &RdpConfig) {
        if let Some(ref domain) = config.domain
            && !domain.is_empty()
        {
            cmd.arg(format!("/d:{domain}"));
        }

        if let Some(ref username) = config.username {
            cmd.arg(format!("/u:{username}"));
        }

        if let Some(ref password) = config.password
            && !password.expose_secret().is_empty()
        {
            // For RemoteApp (external xfreerdp3 process), the caller writes
            // the password into a single-use args file in $XDG_RUNTIME_DIR
            // and passes `/args-from:file:<path>` instead — so the password
            // never appears in `/proc/<pid>/cmdline`. See
            // `super::ephemeral_args::EphemeralRdpArgs` for details and
            // `launch()` for the wiring.
            //
            // For embedded mode (wlfreerdp) we still use `/from-stdin` —
            // it works fine for non-RAIL sessions and reuses the existing
            // pipe-based password injection.
            let is_remote_app = config
                .remote_app_program
                .as_ref()
                .is_some_and(|p| !p.is_empty());

            if !is_remote_app {
                cmd.arg("/from-stdin");
                cmd.stdin(Stdio::piped());
            }
        }

        cmd.arg(format!("/w:{}", config.width));
        cmd.arg(format!("/h:{}", config.height));
        if config.ignore_certificate {
            cmd.arg("/cert:ignore");
        } else {
            cmd.arg("/cert:tofu");
        }
        cmd.arg("/dynamic-resolution");

        // Add decorations flag for window controls
        cmd.arg("/decorations");

        // Add window geometry if saved and remember_window_position is enabled
        if config.remember_window_position
            && let Some((x, y, _width, _height)) = config.window_geometry
        {
            cmd.arg(format!("/x:{x}"));
            cmd.arg(format!("/y:{y}"));
        }

        if config.clipboard_enabled {
            cmd.arg("+clipboard");
        }

        // Add shared folders for drive redirection
        for folder in &config.shared_folders {
            let path = folder.local_path.display();
            // FreeRDP `/drive:<name>,<path>` is comma-delimited; a comma in the
            // share name would split the argument and corrupt the path.
            let safe_name = folder.share_name.replace(',', "_");
            cmd.arg(format!("/drive:{safe_name},{path}"));
        }

        // Map the local default printer into the session via CUPS.
        if config.printer_enabled {
            cmd.arg("/printer");
        }

        for arg in &config.extra_args {
            cmd.arg(arg);
        }

        // Add gateway configuration for RD Gateway connections.
        //
        // FreeRDP 3.x removed the short `/g:` / `/gu:` / `/gp:` aliases in
        // favour of the unified `/gateway:` option (see xfreerdp3(1)); the old
        // aliases are rejected as "Unexpected keyword" and the client exits
        // before connecting (issue #187). FreeRDP reuses the session
        // credentials (`/u:`, `/d:` and the `/from-stdin` password) for the
        // gateway, exactly like the working manual command
        // `xfreerdp /gateway:g:HOST /u:NAME /d:DOMAIN`. We only add an explicit
        // gateway user when it differs from the session user; a distinct
        // gateway account would also need its own password, which RustConn does
        // not store yet (future work).
        if let Some(ref gw_host) = config.gateway_hostname
            && !gw_host.is_empty()
        {
            let mut gateway = format!("g:{gw_host}:{}", config.gateway_port);
            if let Some(ref gw_user) = config.gateway_username
                && !gw_user.is_empty()
                && config.username.as_deref() != Some(gw_user.as_str())
            {
                gateway.push_str(",u:");
                gateway.push_str(gw_user);
            }
            cmd.arg(format!("/gateway:{gateway}"));
        }

        // Add RemoteApp arguments for launching individual applications
        for arg in config.remote_app_freerdp_args() {
            cmd.arg(arg);
        }

        // When RemoteApp is used with xfreerdp3, force NTLM authentication.
        // xfreerdp3 on the host often lacks Kerberos realm configuration,
        // causing NLA to fail even with correct credentials. NTLM works
        // reliably for standalone (non-domain) Windows servers.
        if config
            .remote_app_program
            .as_ref()
            .is_some_and(|p| !p.is_empty())
        {
            cmd.arg("/auth-pkg-list:ntlm");
        }

        if config.port == 3389 {
            cmd.arg(format!("/v:{}", config.host));
        } else {
            cmd.arg(format!("/v:{}:{}", config.host, config.port));
        }
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

    /// Collects the argument vector that `add_connection_args` would pass to
    /// FreeRDP, as owned `String`s for easy assertions.
    fn connection_args(config: &RdpConfig) -> Vec<String> {
        let mut cmd = Command::new("xfreerdp3");
        SafeFreeRdpLauncher::add_connection_args(&mut cmd, config);
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
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
