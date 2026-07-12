//! SPICE external-viewer integration boundary.
//!
//! SPICE sessions are handled by launching an external viewer
//! (`remote-viewer`, `virt-viewer`, or `spicy`). This module provides the
//! connection configuration ([`SpiceClientConfig`]), error type
//! ([`SpiceClientError`]), and the helpers that detect a viewer and build its
//! command line ([`detect_spice_viewer`], [`build_spice_viewer_args`]).
//! Callers that need a strict domain-only build should avoid invoking the
//! detection/launch helpers and treat [`SpiceClientConfig`] as data.
//!
//! # History
//!
//! A native embedded SPICE client (behind a `spice-embedded` feature) was
//! removed in 0.18.0: the bundled `spice-client` 0.2 exposes neither an inputs
//! channel nor raw display frames through its public API, so embedded rendering
//! and input forwarding were impossible without forking the crate. The external
//! viewer is the supported path.

mod config;
mod error;

pub use config::{
    SpiceClientConfig, SpiceImageCompression as SpiceCompression, SpiceSecurityProtocol,
    SpiceSharedFolder,
};
pub use error::SpiceClientError;

use std::path::Path;

/// USB auto-redirect filter for `remote-viewer`: auto-redirect HID-class
/// (`0x03`) devices on connect. The value is a `|`-separated list of
/// `class,vendor,product,version,allow` rules.
pub const SPICE_USB_AUTO_REDIRECT_FILTER: &str = "0x03,-1,-1,-1,0|-1,-1,-1,-1,1";

/// Builds the SPICE connection URI for an external viewer.
///
/// Returns `spice+unix://<path>` when `unix_socket_path` is set (host/port are
/// ignored), `spice+tls://host:port` when TLS is enabled, otherwise
/// `spice://host:port`. Shared by [`build_spice_viewer_args`] and the CLI's
/// `SpiceProtocol::build_command` so both paths stay in sync.
#[must_use]
pub fn build_spice_uri(
    unix_socket_path: Option<&Path>,
    tls_enabled: bool,
    host: &str,
    port: u16,
) -> String {
    match unix_socket_path {
        Some(path) => format!("spice+unix://{}", path.display()),
        None if tls_enabled => format!("spice+tls://{host}:{port}"),
        None => format!("spice://{host}:{port}"),
    }
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

    // Connection URI: spice+unix:///path or spice://host:port
    args.push(build_spice_uri(
        config.unix_socket_path.as_deref(),
        config.tls_enabled,
        &config.host,
        config.port,
    ));

    // Full screen option (not enabled by default for embedded-like behavior)

    // Title
    args.push("--title".to_string());
    if config.unix_socket_path.is_some() {
        args.push(format!(
            "SPICE: {}",
            config
                .unix_socket_path
                .as_ref()
                .map_or("socket", |p| p.to_str().unwrap_or("socket"))
        ));
    } else {
        args.push(format!("SPICE: {}", config.host));
    }

    // USB redirection
    if config.usb_redirection {
        args.push("--spice-usbredir-auto-redirect-filter".to_string());
        args.push(SPICE_USB_AUTO_REDIRECT_FILTER.to_string());
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_build_spice_viewer_args_unix_socket() {
        // Unix-socket mode uses the spice+unix:// scheme and ignores host:port.
        let config = SpiceClientConfig::new("ignored-host")
            .with_port(5900)
            .with_tls(true) // must not produce spice+tls:// in socket mode
            .with_unix_socket("/run/libvirt/qemu/vm-spice.sock");
        let args = build_spice_viewer_args(&config);

        assert!(args.contains(&"spice+unix:///run/libvirt/qemu/vm-spice.sock".to_string()));
        assert!(!args.iter().any(|a| a.starts_with("spice://")));
        assert!(!args.iter().any(|a| a.starts_with("spice+tls://")));
    }
}
