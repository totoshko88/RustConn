//! Protocol layer for `RustConn`
//!
//! This module provides the Protocol trait and implementations for
//! SSH, RDP, VNC, SPICE, Telnet, Serial, SFTP, and Kubernetes protocols.
//! Each protocol handler is responsible for validation and protocol metadata.

mod cli;
mod detection;
pub mod freerdp;
pub mod icons;
mod kubernetes;
mod mosh;
mod rdp;
mod registry;
mod serial;
mod sftp;
mod spice;
mod ssh;
mod telnet;
mod vnc;
mod web;

pub use cli::{format_command_message, format_connection_message};
pub use detection::{
    ClientDetectionResult, ClientInfo, ZeroTrustDetectionResult, detect_aws_cli, detect_azure_cli,
    detect_boundary, detect_cloudflared, detect_gcloud_cli, detect_hoop, detect_kubectl,
    detect_mosh, detect_oci_cli, detect_picocom, detect_rdp_client, detect_spice_client,
    detect_ssh_client, detect_tailscale, detect_teleport, detect_telnet_client, detect_vnc_client,
    detect_vnc_viewer_name, detect_vnc_viewer_path, detect_waypipe,
};
pub use freerdp::{
    FreeRdpConfig, build_freerdp_args, extract_geometry_from_args, has_decorations_flag,
};
pub use icons::{
    CloudProvider, PROTOCOL_TAB_CSS_CLASSES, ProviderIconCache, all_protocol_icons,
    detect_provider, get_protocol_color_rgb, get_protocol_icon, get_protocol_icon_by_name,
    get_protocol_tab_css_class, get_zero_trust_provider_icon,
};
pub use kubernetes::KubernetesProtocol;
pub use mosh::MoshProtocol;
pub use rdp::RdpProtocol;
pub use registry::ProtocolRegistry;
pub use serial::SerialProtocol;
pub use sftp::SftpProtocol;
pub use spice::SpiceProtocol;
pub use ssh::SshProtocol;
pub use telnet::TelnetProtocol;
pub use vnc::VncProtocol;
pub use web::WebProtocol;

pub use crate::error::ProtocolResult;
use crate::models::Connection;

/// Describes what a protocol supports at a feature level.
///
/// Used by the GUI and CLI to decide which UI elements to show
/// (e.g., split-view button, audio controls, clipboard toggle,
/// multi-monitor options, port forwarding settings).
// Allow many bools — these are distinct, independent capability flags
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolCapabilities {
    /// Has a built-in embedded viewer (VTE, IronRDP, vnc-rs)
    pub embedded: bool,
    /// Can fall back to an external CLI client
    pub external_fallback: bool,
    /// Supports file transfer / shared folders
    pub file_transfer: bool,
    /// Supports audio redirection
    pub audio: bool,
    /// Supports clipboard sharing
    pub clipboard: bool,
    /// Can be used inside a split-view panel
    pub split_view: bool,
    /// Runs inside a VTE terminal (SSH, Telnet)
    pub terminal_based: bool,
    // --- New capability flags (v0.12.8) ---
    /// Supports multi-monitor / multi-display (RDP MS-RDPEDISP, SPICE)
    pub multi_monitor: bool,
    /// Supports USB device redirection (SPICE)
    pub usb_redirection: bool,
    /// Supports local/remote/dynamic port forwarding (SSH)
    pub port_forwarding: bool,
    /// Supports Wayland forwarding (SSH + waypipe)
    pub wayland_forwarding: bool,
    /// Supports X11 forwarding (SSH -X/-Y)
    pub x11_forwarding: bool,
    /// Supports session recording / logging
    pub session_recording: bool,
    /// Supports remote system monitoring (CPU, RAM, disk)
    pub remote_monitoring: bool,
    /// Supports command snippets / automation
    pub command_snippets: bool,
    /// Supports Wake-on-LAN
    pub wake_on_lan: bool,
}

impl ProtocolCapabilities {
    /// Shorthand for a terminal-based protocol (SSH, Telnet).
    ///
    /// Terminal protocols support split view, session recording,
    /// remote monitoring, command snippets, and Wake-on-LAN.
    const fn terminal() -> Self {
        Self {
            embedded: true,
            external_fallback: false,
            file_transfer: false,
            audio: false,
            clipboard: false,
            split_view: true,
            terminal_based: true,
            multi_monitor: false,
            usb_redirection: false,
            port_forwarding: false,
            wayland_forwarding: false,
            x11_forwarding: false,
            session_recording: true,
            remote_monitoring: true,
            command_snippets: true,
            wake_on_lan: true,
        }
    }

    /// Shorthand for a graphical protocol with embedded + external fallback.
    ///
    /// Graphical protocols support Wake-on-LAN but not terminal-specific
    /// features like snippets or monitoring.
    const fn graphical(file_transfer: bool, audio: bool, clipboard: bool) -> Self {
        Self {
            embedded: true,
            external_fallback: true,
            file_transfer,
            audio,
            clipboard,
            split_view: false,
            terminal_based: false,
            multi_monitor: false,
            usb_redirection: false,
            port_forwarding: false,
            wayland_forwarding: false,
            x11_forwarding: false,
            session_recording: false,
            remote_monitoring: false,
            command_snippets: false,
            wake_on_lan: true,
        }
    }

    /// Shorthand for an external-only protocol.
    const fn external_only(clipboard: bool) -> Self {
        Self {
            embedded: false,
            external_fallback: true,
            file_transfer: false,
            audio: false,
            clipboard,
            split_view: false,
            terminal_based: false,
            multi_monitor: false,
            usb_redirection: false,
            port_forwarding: false,
            wayland_forwarding: false,
            x11_forwarding: false,
            session_recording: false,
            remote_monitoring: false,
            command_snippets: false,
            wake_on_lan: true,
        }
    }
}

/// Core trait for all connection protocols
///
/// This trait defines the interface that all protocol handlers must implement.
/// It provides methods for validation, protocol metadata, capability queries,
/// and optional CLI command building.
pub trait Protocol: Send + Sync {
    /// Returns the protocol identifier (e.g., "ssh", "rdp", "vnc")
    fn protocol_id(&self) -> &'static str;

    /// Returns human-readable protocol name
    fn display_name(&self) -> &'static str;

    /// Returns default port for this protocol
    fn default_port(&self) -> u16;

    /// Validates connection configuration for this protocol
    ///
    /// # Arguments
    /// * `connection` - The connection to validate
    ///
    /// # Returns
    /// `Ok(())` if valid, or a `ProtocolError` describing the validation failure
    ///
    /// # Errors
    /// Returns `ProtocolError` if the connection configuration is invalid
    fn validate_connection(&self, connection: &Connection) -> ProtocolResult<()>;

    /// Returns the set of features this protocol supports.
    ///
    /// The default implementation returns a terminal-based capability set.
    /// Override in each protocol handler to reflect actual capabilities.
    fn capabilities(&self) -> ProtocolCapabilities {
        ProtocolCapabilities::terminal()
    }

    /// Builds the CLI command arguments for launching this protocol.
    ///
    /// Returns `None` for protocols that don't use an external CLI command
    /// (e.g., embedded-only graphical protocols). The first element of the
    /// returned `Vec` is the program name, followed by arguments.
    ///
    /// Jump-host resolution requires access to the connection store, which
    /// lives in the GUI layer. Pass pre-resolved jump hosts via
    /// `SshConfig.proxy_jump` before calling this method.
    fn build_command(&self, _connection: &Connection) -> Option<Vec<String>> {
        None
    }
}
