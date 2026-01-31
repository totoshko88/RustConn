//! RDP backend selection and detection
//!
//! This module provides centralized logic for detecting available RDP backends
//! and selecting the most appropriate one based on system capabilities.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::process::{Command, Stdio};

/// Available RDP backend implementations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RdpBackend {
    /// Native IronRDP implementation (embedded, Rust-native)
    IronRdp,
    /// FreeRDP wlfreerdp for Wayland embedded mode
    WlFreeRdp,
    /// FreeRDP xfreerdp3 for X11/external mode
    XFreeRdp3,
    /// FreeRDP xfreerdp (legacy) for X11/external mode
    XFreeRdp,
    /// Generic freerdp command
    FreeRdp,
}

impl RdpBackend {
    /// Returns the command name for this backend
    #[must_use]
    pub const fn command_name(&self) -> &'static str {
        match self {
            Self::IronRdp => "ironrdp",
            Self::WlFreeRdp => "wlfreerdp",
            Self::XFreeRdp3 => "xfreerdp3",
            Self::XFreeRdp => "xfreerdp",
            Self::FreeRdp => "freerdp",
        }
    }

    /// Returns whether this backend supports embedded mode
    #[must_use]
    pub const fn supports_embedded(&self) -> bool {
        matches!(self, Self::IronRdp | Self::WlFreeRdp)
    }

    /// Returns whether this backend is native Rust (no external process)
    #[must_use]
    pub const fn is_native(&self) -> bool {
        matches!(self, Self::IronRdp)
    }

    /// Returns the display name for UI
    #[must_use]
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::IronRdp => "IronRDP (Native)",
            Self::WlFreeRdp => "FreeRDP (Wayland)",
            Self::XFreeRdp3 => "FreeRDP 3.x",
            Self::XFreeRdp => "FreeRDP 2.x",
            Self::FreeRdp => "FreeRDP",
        }
    }
}

impl fmt::Display for RdpBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Result of backend detection
#[derive(Debug, Clone)]
pub struct BackendDetectionResult {
    /// The detected backend
    pub backend: RdpBackend,
    /// Whether the backend is available
    pub available: bool,
    /// Optional version string
    pub version: Option<String>,
}

/// RDP backend selector for choosing the best available backend
#[derive(Debug, Clone, Default)]
pub struct RdpBackendSelector {
    /// Cached detection results
    cache: Option<Vec<BackendDetectionResult>>,
    /// Whether IronRDP is compiled in
    ironrdp_compiled: bool,
}

impl RdpBackendSelector {
    /// Creates a new backend selector
    #[must_use]
    pub fn new() -> Self {
        Self {
            cache: None,
            ironrdp_compiled: crate::is_embedded_rdp_available(),
        }
    }

    /// Creates a selector with IronRDP availability override (for testing)
    #[must_use]
    pub fn with_ironrdp_available(available: bool) -> Self {
        Self {
            cache: None,
            ironrdp_compiled: available,
        }
    }

    /// Detects all available backends
    pub fn detect_all(&mut self) -> &[BackendDetectionResult] {
        if self.cache.is_none() {
            let mut results = Vec::new();

            // Check IronRDP (compile-time)
            results.push(BackendDetectionResult {
                backend: RdpBackend::IronRdp,
                available: self.ironrdp_compiled,
                version: if self.ironrdp_compiled {
                    Some("native".to_string())
                } else {
                    None
                },
            });

            // Check wlfreerdp
            results.push(BackendDetectionResult {
                backend: RdpBackend::WlFreeRdp,
                available: Self::check_command("wlfreerdp"),
                version: Self::get_freerdp_version("wlfreerdp"),
            });

            // Check xfreerdp3
            results.push(BackendDetectionResult {
                backend: RdpBackend::XFreeRdp3,
                available: Self::check_command("xfreerdp3"),
                version: Self::get_freerdp_version("xfreerdp3"),
            });

            // Check xfreerdp
            results.push(BackendDetectionResult {
                backend: RdpBackend::XFreeRdp,
                available: Self::check_command("xfreerdp"),
                version: Self::get_freerdp_version("xfreerdp"),
            });

            // Check freerdp
            results.push(BackendDetectionResult {
                backend: RdpBackend::FreeRdp,
                available: Self::check_command("freerdp"),
                version: Self::get_freerdp_version("freerdp"),
            });

            self.cache = Some(results);
        }

        self.cache.as_ref().expect("cache should be populated")
    }

    /// Returns all available backends
    #[must_use]
    pub fn available_backends(&mut self) -> Vec<RdpBackend> {
        self.detect_all()
            .iter()
            .filter(|r| r.available)
            .map(|r| r.backend)
            .collect()
    }

    /// Selects the best backend for embedded mode
    ///
    /// Priority: IronRDP > wlfreerdp
    #[must_use]
    pub fn select_embedded(&mut self) -> Option<RdpBackend> {
        let available = self.available_backends();

        // Prefer IronRDP for native embedded
        if available.contains(&RdpBackend::IronRdp) {
            return Some(RdpBackend::IronRdp);
        }

        // Fall back to wlfreerdp for Wayland embedded
        if available.contains(&RdpBackend::WlFreeRdp) {
            return Some(RdpBackend::WlFreeRdp);
        }

        None
    }

    /// Selects the best backend for external mode
    ///
    /// Priority: xfreerdp3 > xfreerdp > freerdp
    #[must_use]
    pub fn select_external(&mut self) -> Option<RdpBackend> {
        let available = self.available_backends();

        // Prefer xfreerdp3 (newest)
        if available.contains(&RdpBackend::XFreeRdp3) {
            return Some(RdpBackend::XFreeRdp3);
        }

        // Fall back to xfreerdp
        if available.contains(&RdpBackend::XFreeRdp) {
            return Some(RdpBackend::XFreeRdp);
        }

        // Last resort: generic freerdp
        if available.contains(&RdpBackend::FreeRdp) {
            return Some(RdpBackend::FreeRdp);
        }

        None
    }

    /// Selects the best available backend (embedded preferred)
    #[must_use]
    pub fn select_best(&mut self) -> Option<RdpBackend> {
        self.select_embedded().or_else(|| self.select_external())
    }

    /// Checks if any RDP backend is available
    #[must_use]
    pub fn has_any_backend(&mut self) -> bool {
        !self.available_backends().is_empty()
    }

    /// Checks if embedded mode is available
    #[must_use]
    pub fn has_embedded_support(&mut self) -> bool {
        self.select_embedded().is_some()
    }

    /// Clears the detection cache (useful after system changes)
    pub fn clear_cache(&mut self) {
        self.cache = None;
    }

    /// Checks if a command is available in PATH
    fn check_command(cmd: &str) -> bool {
        Command::new("which")
            .arg(cmd)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// Gets FreeRDP version string
    fn get_freerdp_version(cmd: &str) -> Option<String> {
        let output = Command::new(cmd)
            .arg("--version")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .ok()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // FreeRDP outputs version to stderr or stdout depending on version
        let version_text = if stdout.contains("freerdp") || stdout.contains("FreeRDP") {
            stdout.to_string()
        } else {
            stderr.to_string()
        };

        // Extract version number (e.g., "3.0.0" from "This is FreeRDP version 3.0.0")
        version_text
            .lines()
            .next()
            .map(|line| line.trim().to_string())
            .filter(|s| !s.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_command_names() {
        assert_eq!(RdpBackend::IronRdp.command_name(), "ironrdp");
        assert_eq!(RdpBackend::WlFreeRdp.command_name(), "wlfreerdp");
        assert_eq!(RdpBackend::XFreeRdp3.command_name(), "xfreerdp3");
        assert_eq!(RdpBackend::XFreeRdp.command_name(), "xfreerdp");
    }

    #[test]
    fn test_backend_embedded_support() {
        assert!(RdpBackend::IronRdp.supports_embedded());
        assert!(RdpBackend::WlFreeRdp.supports_embedded());
        assert!(!RdpBackend::XFreeRdp3.supports_embedded());
        assert!(!RdpBackend::XFreeRdp.supports_embedded());
    }

    #[test]
    fn test_backend_is_native() {
        assert!(RdpBackend::IronRdp.is_native());
        assert!(!RdpBackend::WlFreeRdp.is_native());
        assert!(!RdpBackend::XFreeRdp3.is_native());
    }

    #[test]
    fn test_selector_creation() {
        let selector = RdpBackendSelector::new();
        assert!(selector.cache.is_none());
    }

    #[test]
    fn test_selector_with_ironrdp_override() {
        let selector = RdpBackendSelector::with_ironrdp_available(true);
        assert!(selector.ironrdp_compiled);

        let selector = RdpBackendSelector::with_ironrdp_available(false);
        assert!(!selector.ironrdp_compiled);
    }

    #[test]
    fn test_selector_detect_all_populates_cache() {
        let mut selector = RdpBackendSelector::new();
        assert!(selector.cache.is_none());

        let results = selector.detect_all();
        assert!(!results.is_empty());
        assert!(selector.cache.is_some());
    }

    #[test]
    fn test_selector_clear_cache() {
        let mut selector = RdpBackendSelector::new();
        let _ = selector.detect_all();
        assert!(selector.cache.is_some());

        selector.clear_cache();
        assert!(selector.cache.is_none());
    }

    #[test]
    fn test_backend_display() {
        assert_eq!(format!("{}", RdpBackend::IronRdp), "IronRDP (Native)");
        assert_eq!(format!("{}", RdpBackend::WlFreeRdp), "FreeRDP (Wayland)");
    }

    #[test]
    fn test_selector_embedded_priority() {
        // With IronRDP available, it should be preferred
        let mut selector = RdpBackendSelector::with_ironrdp_available(true);
        // Note: This test depends on system state, so we just verify the logic
        if let Some(backend) = selector.select_embedded() {
            assert!(backend.supports_embedded());
        }
    }
}
