//! Pure Rust SPICE client for embedded SPICE sessions
//!
//! This module provides a SPICE client implementation for embedded SPICE sessions
//! in GTK4 without external processes.
//!
//! # Architecture
//!
//! The SPICE client runs in a background thread with its own Tokio runtime and
//! communicates with the GUI through channels:
//! - `SpiceClientEvent` channel: framebuffer updates, resolution changes, etc.
//! - `SpiceClientCommand` channel: keyboard/mouse input, disconnect requests
//!
//! This follows the same pattern as the VNC and RDP clients.
//!
//! # Feature Flag
//!
//! The embedded SPICE client requires the `spice-embedded` feature flag:
//!
//! ```toml
//! [dependencies]
//! rustconn-core = { version = "0.1", features = ["spice-embedded"] }
//! ```
//!
//! When the feature is disabled, the module still provides the types and
//! configuration, but the `SpiceClient` struct is not available. In this case,
//! the GUI falls back to virt-viewer subprocess.
//!
//! # Requirements Coverage
//!
//! - Requirement 9.1: Native SPICE embedding as GTK widget
//! - Requirement 9.2: Display rendering in embedded mode
//! - Requirement 9.3: Keyboard and mouse input forwarding
//! - Requirement 9.4: Fallback to virt-viewer

#[cfg(feature = "spice-embedded")]
mod client;
mod config;
mod error;
mod event;

#[cfg(feature = "spice-embedded")]
pub use client::{SpiceClient, SpiceClientState, SpiceCommandSender, SpiceEventReceiver};
pub use config::{
    SpiceClientConfig, SpiceImageCompression as SpiceCompression, SpiceSecurityProtocol,
    SpiceSharedFolder,
};
pub use error::SpiceClientError;
pub use event::{SpiceClientCommand, SpiceClientEvent, SpiceRect};

/// Check if embedded SPICE support is available
///
/// Returns true if the `spice-embedded` feature is enabled, which means
/// the native SPICE client can be used. When false, the GUI should
/// fall back to virt-viewer subprocess.
#[must_use]
pub const fn is_embedded_spice_available() -> bool {
    cfg!(feature = "spice-embedded")
}

/// Detects available SPICE viewer applications for fallback mode
///
/// Returns the path to the first available SPICE viewer, or None if none found.
/// Checks for: remote-viewer, virt-viewer, spicy
#[must_use]
pub fn detect_spice_viewer() -> Option<String> {
    let candidates = ["remote-viewer", "virt-viewer", "spicy"];

    for candidate in &candidates {
        if std::process::Command::new("which")
            .arg(candidate)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Some((*candidate).to_string());
        }
    }

    None
}

/// Builds command-line arguments for virt-viewer/remote-viewer fallback
///
/// This function generates the appropriate command-line arguments for
/// launching an external SPICE viewer when native embedding is not available.
///
/// # Arguments
///
/// * `config` - The SPICE client configuration
///
/// # Returns
///
/// A vector of command-line arguments for the SPICE viewer
#[must_use]
pub fn build_spice_viewer_args(config: &SpiceClientConfig) -> Vec<String> {
    let mut args = Vec::new();

    // Connection URI: spice://host:port
    let uri = if config.tls_enabled {
        format!("spice+tls://{}:{}", config.host, config.port)
    } else {
        format!("spice://{}:{}", config.host, config.port)
    };
    args.push(uri);

    // Full screen option (not enabled by default for embedded-like behavior)
    // args.push("--full-screen".to_string());

    // Title
    args.push("--title".to_string());
    args.push(format!("SPICE: {}", config.host));

    // USB redirection
    if config.usb_redirection {
        args.push("--spice-usbredir-auto-redirect-filter".to_string());
        args.push("0x03,-1,-1,-1,0|-1,-1,-1,-1,1".to_string());
    }

    // Shared folders (webdav)
    for folder in &config.shared_folders {
        args.push("--spice-shared-dir".to_string());
        args.push(folder.local_path.to_string_lossy().to_string());
    }

    // TLS options
    if config.tls_enabled {
        if let Some(ref ca_path) = config.ca_cert_path {
            args.push("--spice-ca-file".to_string());
            args.push(ca_path.to_string_lossy().to_string());
        }

        if config.skip_cert_verify {
            // Note: remote-viewer doesn't have a direct skip-verify flag
            // but we can set host-subject to empty to be more permissive
            args.push("--spice-host-subject".to_string());
            args.push(String::new());
        }
    }

    // Disable audio if not wanted
    if !config.audio_playback {
        args.push("--spice-disable-audio".to_string());
    }

    // SPICE proxy for tunnelled connections (e.g. Proxmox VE)
    if let Some(ref proxy) = config.proxy {
        args.push("--spice-proxy".to_string());
        args.push(proxy.clone());
    }

    args
}

/// Result of attempting to launch a SPICE viewer
#[derive(Debug)]
pub enum SpiceViewerLaunchResult {
    /// Successfully launched the viewer
    Launched {
        /// The viewer command that was launched
        viewer: String,
        /// Process ID if available
        pid: Option<u32>,
    },
    /// No SPICE viewer found on the system
    NoViewerFound,
    /// Failed to launch the viewer
    LaunchFailed(String),
}

/// Launches an external SPICE viewer as fallback
///
/// This function attempts to launch an external SPICE viewer (remote-viewer,
/// virt-viewer, or spicy) when native embedding is not available.
///
/// # Arguments
///
/// * `config` - The SPICE client configuration
///
/// # Returns
///
/// A `SpiceViewerLaunchResult` indicating success or failure
#[must_use]
pub fn launch_spice_viewer(config: &SpiceClientConfig) -> SpiceViewerLaunchResult {
    let Some(viewer) = detect_spice_viewer() else {
        return SpiceViewerLaunchResult::NoViewerFound;
    };

    let args = build_spice_viewer_args(config);

    match std::process::Command::new(&viewer).args(&args).spawn() {
        Ok(child) => SpiceViewerLaunchResult::Launched {
            viewer,
            pid: Some(child.id()),
        },
        Err(e) => SpiceViewerLaunchResult::LaunchFailed(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_embedded_spice_available() {
        // This test just verifies the function compiles and returns a bool
        let _available = is_embedded_spice_available();
    }

    #[test]
    fn test_build_spice_viewer_args_basic() {
        let config = SpiceClientConfig::new("192.168.1.100").with_port(5900);
        let args = build_spice_viewer_args(&config);

        assert!(args.contains(&"spice://192.168.1.100:5900".to_string()));
        assert!(args.contains(&"--title".to_string()));
    }

    #[test]
    fn test_build_spice_viewer_args_with_tls() {
        let config = SpiceClientConfig::new("secure.example.com")
            .with_port(5901)
            .with_tls(true)
            .with_skip_cert_verify(true);
        let args = build_spice_viewer_args(&config);

        assert!(args.contains(&"spice+tls://secure.example.com:5901".to_string()));
    }

    #[test]
    fn test_build_spice_viewer_args_with_usb() {
        let config = SpiceClientConfig::new("localhost").with_usb_redirection(true);
        let args = build_spice_viewer_args(&config);

        assert!(args.contains(&"--spice-usbredir-auto-redirect-filter".to_string()));
    }

    #[test]
    fn test_build_spice_viewer_args_with_shared_folder() {
        let folder = SpiceSharedFolder::new("/home/user/share", "MyShare");
        let config = SpiceClientConfig::new("localhost").with_shared_folder(folder);
        let args = build_spice_viewer_args(&config);

        assert!(args.contains(&"--spice-shared-dir".to_string()));
        assert!(args.contains(&"/home/user/share".to_string()));
    }

    #[test]
    fn test_build_spice_viewer_args_no_audio() {
        let config = SpiceClientConfig::new("localhost").with_audio_playback(false);
        let args = build_spice_viewer_args(&config);

        assert!(args.contains(&"--spice-disable-audio".to_string()));
    }

    #[test]
    fn test_build_spice_viewer_args_with_ca_cert() {
        let config = SpiceClientConfig::new("localhost")
            .with_tls(true)
            .with_ca_cert("/etc/ssl/certs/ca.crt");
        let args = build_spice_viewer_args(&config);

        assert!(args.contains(&"--spice-ca-file".to_string()));
        assert!(args.contains(&"/etc/ssl/certs/ca.crt".to_string()));
    }

    #[test]
    fn test_build_spice_viewer_args_with_proxy() {
        let config = SpiceClientConfig::new("localhost").with_proxy("http://192.168.1.100:3128");
        let args = build_spice_viewer_args(&config);

        assert!(args.contains(&"--spice-proxy".to_string()));
        assert!(args.contains(&"http://192.168.1.100:3128".to_string()));
    }
}
