//! Safe FreeRDP launcher with Qt error suppression
//!
//! This module provides the `SafeFreeRdpLauncher` struct for launching FreeRDP
//! with environment variables set to suppress Qt/Wayland warnings.
//!
//! # Requirements Coverage
//!
//! - Requirement 6.1: QSocketNotifier error handling
//! - Requirement 6.2: Wayland requestActivate warning suppression

use crate::embedded_rdp_types::{EmbeddedRdpError, RdpConfig};
use std::process::{Child, Command, Stdio};

/// Safe FreeRDP launcher with Qt error suppression
///
/// This struct provides methods to launch FreeRDP with environment variables
/// set to suppress Qt/Wayland warnings that can cause issues when mixing
/// Qt-based FreeRDP with GTK4 applications.
///
/// # Requirements Coverage
///
/// - Requirement 6.1: QSocketNotifier error handling
/// - Requirement 6.2: Wayland requestActivate warning suppression
pub struct SafeFreeRdpLauncher {
    /// Whether to suppress Qt warnings
    pub(crate) suppress_qt_warnings: bool,
    /// Whether to force X11 backend
    pub(crate) force_x11: bool,
}

impl SafeFreeRdpLauncher {
    /// Creates a new launcher with default settings (warnings suppressed)
    #[must_use]
    pub fn new() -> Self {
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
            // Suppress Qt/Wayland warnings (Requirement 6.1, 6.2)
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
    pub fn launch(&self, config: &RdpConfig) -> Result<Child, EmbeddedRdpError> {
        let binary = Self::detect_freerdp().ok_or_else(|| {
            EmbeddedRdpError::FreeRdpInit(
                "No FreeRDP client found. Install xfreerdp or wlfreerdp.".to_string(),
            )
        })?;

        let mut cmd = Command::new(&binary);

        // Set environment to suppress Qt warnings (Requirement 6.1, 6.2)
        for (key, value) in self.build_env() {
            cmd.env(key, value);
        }

        // Build connection arguments
        Self::add_connection_args(&mut cmd, config);

        // Redirect stderr to suppress warnings
        cmd.stderr(Stdio::null());

        cmd.spawn()
            .map_err(|e| EmbeddedRdpError::FreeRdpInit(e.to_string()))
    }

    /// Detects available FreeRDP binary
    pub fn detect_freerdp() -> Option<String> {
        let candidates = ["xfreerdp3", "xfreerdp", "freerdp"];
        for candidate in candidates {
            if Command::new("which")
                .arg(candidate)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
            {
                return Some(candidate.to_string());
            }
        }
        None
    }

    /// Adds connection arguments to the command
    pub fn add_connection_args(cmd: &mut Command, config: &RdpConfig) {
        if let Some(ref domain) = config.domain {
            if !domain.is_empty() {
                cmd.arg(format!("/d:{domain}"));
            }
        }

        if let Some(ref username) = config.username {
            cmd.arg(format!("/u:{username}"));
        }

        if let Some(ref password) = config.password {
            if !password.is_empty() {
                cmd.arg(format!("/p:{password}"));
            }
        }

        cmd.arg(format!("/w:{}", config.width));
        cmd.arg(format!("/h:{}", config.height));
        cmd.arg("/cert:ignore");
        cmd.arg("/dynamic-resolution");

        // Add decorations flag for window controls (Requirement 6.1)
        cmd.arg("/decorations");

        // Add window geometry if saved and remember_window_position is enabled
        if config.remember_window_position {
            if let Some((x, y, _width, _height)) = config.window_geometry {
                cmd.arg(format!("/x:{x}"));
                cmd.arg(format!("/y:{y}"));
            }
        }

        if config.clipboard_enabled {
            cmd.arg("+clipboard");
        }

        // Add shared folders for drive redirection
        for folder in &config.shared_folders {
            let path = folder.local_path.display();
            cmd.arg(format!("/drive:{},{}", folder.share_name, path));
        }

        for arg in &config.extra_args {
            cmd.arg(arg);
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
    fn test_safe_freerdp_launcher_default() {
        let launcher = SafeFreeRdpLauncher::new();
        assert!(launcher.suppress_qt_warnings);
        assert!(launcher.force_x11);
    }

    #[test]
    fn test_safe_freerdp_launcher_builder() {
        let launcher = SafeFreeRdpLauncher::new()
            .with_suppress_warnings(false)
            .with_force_x11(false);
        assert!(!launcher.suppress_qt_warnings);
        assert!(!launcher.force_x11);
    }

    #[test]
    fn test_safe_freerdp_launcher_env() {
        let launcher = SafeFreeRdpLauncher::new();
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
}
