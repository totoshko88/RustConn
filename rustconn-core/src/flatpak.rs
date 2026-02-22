//! Flatpak sandbox detection
//!
//! This module provides utilities for detecting if the application is running
//! inside a Flatpak sandbox.
//!
//! **Note:** As of version 0.7.7, the `--talk-name=org.freedesktop.Flatpak`
//! permission was removed from the Flatpak manifest per Flathub reviewer feedback.
//! The deprecated `flatpak-spawn --host` wrapper functions were removed in 0.9.0.
//! Use embedded clients (IronRDP, vnc-rs) instead of external host commands.

use std::sync::OnceLock;

/// Cached result of Flatpak detection
static IS_FLATPAK: OnceLock<bool> = OnceLock::new();

/// Checks if the application is running inside a Flatpak sandbox.
///
/// This function caches the result for performance.
///
/// Detection is based on:
/// 1. Presence of `/.flatpak-info` file (most reliable)
/// 2. `FLATPAK_ID` environment variable
#[must_use]
pub fn is_flatpak() -> bool {
    *IS_FLATPAK.get_or_init(|| {
        // Primary check: /.flatpak-info exists in Flatpak sandbox
        if std::path::Path::new("/.flatpak-info").exists() {
            tracing::debug!("Detected Flatpak sandbox via /.flatpak-info");
            return true;
        }

        // Secondary check: FLATPAK_ID environment variable
        if std::env::var("FLATPAK_ID").is_ok() {
            tracing::debug!("Detected Flatpak sandbox via FLATPAK_ID env var");
            return true;
        }

        false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_flatpak_detection() {
        // This test will return false in normal test environment
        // and true only when actually running in Flatpak
        let result = is_flatpak();
        // Just verify it doesn't panic and returns a boolean
        // The result depends on the environment
        let _ = result;
    }
}
