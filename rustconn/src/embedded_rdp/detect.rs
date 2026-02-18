//! FreeRDP detection utilities
//!
//! This module provides functions for detecting available FreeRDP clients.
//! Detection follows a Wayland-first strategy: on Wayland sessions,
//! `wlfreerdp` variants are preferred over `xfreerdp`.

use std::process::{Command, Stdio};

/// Ordered candidate list for FreeRDP detection.
///
/// Wayland-native variants come first, followed by X11 fallbacks.
const WAYLAND_FIRST_CANDIDATES: &[&str] = &[
    "wlfreerdp3", // FreeRDP 3.x Wayland
    "wlfreerdp",  // FreeRDP 2.x Wayland
    "xfreerdp3",  // FreeRDP 3.x X11
    "xfreerdp",   // FreeRDP 2.x X11
];

/// X11-first candidate order (used when not running under Wayland).
const X11_FIRST_CANDIDATES: &[&str] = &[
    "xfreerdp3",  // FreeRDP 3.x X11
    "xfreerdp",   // FreeRDP 2.x X11
    "wlfreerdp3", // FreeRDP 3.x Wayland (still usable as fallback)
    "wlfreerdp",  // FreeRDP 2.x Wayland
];

/// Returns `true` if the current session is Wayland.
fn is_wayland_session() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|v| v == "wayland")
        .unwrap_or(false)
        || std::env::var("WAYLAND_DISPLAY").is_ok()
}

/// Checks whether a binary is available on `PATH`.
fn binary_exists(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Detects the best available FreeRDP binary.
///
/// On Wayland sessions, Wayland-native variants (`wlfreerdp3`, `wlfreerdp`)
/// are tried first. On X11 sessions, `xfreerdp` variants take priority.
/// Returns `None` if no FreeRDP client is found.
#[must_use]
pub fn detect_best_freerdp() -> Option<String> {
    let candidates = if is_wayland_session() {
        WAYLAND_FIRST_CANDIDATES
    } else {
        X11_FIRST_CANDIDATES
    };

    for candidate in candidates {
        if binary_exists(candidate) {
            return Some((*candidate).to_string());
        }
    }
    None
}

/// Detects if a Wayland-native FreeRDP variant is available
#[must_use]
pub fn detect_wlfreerdp() -> bool {
    detect_best_freerdp().is_some_and(|b| b.starts_with("wl"))
}

/// Detects if any FreeRDP client is available for external mode
///
/// Returns the name of the best available FreeRDP client.
#[must_use]
pub fn detect_xfreerdp() -> Option<String> {
    detect_best_freerdp()
}

/// Checks if IronRDP native client is available
///
/// This is determined at compile time via the rdp-embedded feature flag.
#[must_use]
pub fn is_ironrdp_available() -> bool {
    rustconn_core::is_embedded_rdp_available()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wayland_candidates_include_wlfreerdp() {
        assert!(WAYLAND_FIRST_CANDIDATES.contains(&"wlfreerdp3"));
        assert!(WAYLAND_FIRST_CANDIDATES.contains(&"wlfreerdp"));
        // Wayland variants should come before X11 variants
        let wl_pos = WAYLAND_FIRST_CANDIDATES
            .iter()
            .position(|c| *c == "wlfreerdp3")
            .unwrap();
        let x11_pos = WAYLAND_FIRST_CANDIDATES
            .iter()
            .position(|c| *c == "xfreerdp3")
            .unwrap();
        assert!(wl_pos < x11_pos);
    }

    #[test]
    fn test_x11_candidates_prefer_xfreerdp() {
        let x11_pos = X11_FIRST_CANDIDATES
            .iter()
            .position(|c| *c == "xfreerdp3")
            .unwrap();
        let wl_pos = X11_FIRST_CANDIDATES
            .iter()
            .position(|c| *c == "wlfreerdp3")
            .unwrap();
        assert!(x11_pos < wl_pos);
    }

    #[test]
    fn test_binary_exists_returns_false_for_nonexistent() {
        assert!(!binary_exists("this_binary_does_not_exist_12345"));
    }
}
