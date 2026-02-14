//! FreeRDP detection utilities
//!
//! This module provides functions for detecting available FreeRDP clients.

use std::process::{Command, Stdio};

/// Detects if wlfreerdp is available for embedded mode
#[must_use]
pub fn detect_wlfreerdp() -> bool {
    Command::new("which")
        .arg("wlfreerdp")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Detects if xfreerdp is available for external mode
///
/// Returns the name of the first available FreeRDP client found.
#[must_use]
pub fn detect_xfreerdp() -> Option<String> {
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

/// Checks if IronRDP native client is available
///
/// This is determined at compile time via the rdp-embedded feature flag.
#[must_use]
pub fn is_ironrdp_available() -> bool {
    rustconn_core::is_embedded_rdp_available()
}
