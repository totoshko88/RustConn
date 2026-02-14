//! Type definitions and utilities for the main window
//!
//! # Type Aliases
//!
//! This module defines shared type aliases used throughout the GUI crate.
//! These aliases use `Rc` (Reference Counted) instead of `Arc` (Atomic Reference Counted)
//! because GTK4 is single-threaded and all GUI operations happen on the main thread.
//!
//! Using `Rc` provides:
//! - Lower overhead (no atomic operations)
//! - Simpler debugging (no Send/Sync bounds)
//! - Explicit single-thread semantics matching GTK's model
//!
//! For interior mutability, `RefCell` is used instead of `Mutex` for the same reasons.

use crate::external_window::ExternalWindowManager;
use crate::sidebar::ConnectionSidebar;
use crate::split_view::{SplitViewBridge, TabSplitManager};
use crate::terminal::TerminalNotebook;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use uuid::Uuid;

/// Shared sidebar type
///
/// Uses `Rc` because GTK is single-threaded; no need for `Arc`.
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Shared terminal notebook type
///
/// Uses `Rc` because GTK is single-threaded; no need for `Arc`.
pub type SharedNotebook = Rc<TerminalNotebook>;

/// Shared split view type (uses new SplitViewBridge implementation)
///
/// Uses `Rc` because GTK is single-threaded; no need for `Arc`.
pub type SharedSplitView = Rc<SplitViewBridge>;

/// Shared tab split manager type (new implementation)
///
/// Uses `Rc<RefCell<_>>` for single-threaded interior mutability.
pub type SharedTabSplitManager = Rc<RefCell<TabSplitManager>>;

/// Map of session IDs to their split view bridges
///
/// Each session that has been split gets its own independent `SplitViewBridge`.
/// Uses `Rc<RefCell<_>>` for single-threaded interior mutability.
///
/// Requirement 3: Each tab maintains its own independent split layout
pub type SessionSplitBridges = Rc<RefCell<HashMap<Uuid, Rc<SplitViewBridge>>>>;

/// Shared external window manager type
///
/// Uses `Rc` because GTK is single-threaded; no need for `Arc`.
pub type SharedExternalWindowManager = Rc<ExternalWindowManager>;

/// Returns the protocol string for a connection, including provider info for ZeroTrust
///
/// For ZeroTrust connections, returns "zerotrust:provider" format to enable
/// provider-specific icons in the sidebar.
///
/// Uses the provider enum to determine the provider type for icon display.
#[must_use]
pub fn get_protocol_string(config: &rustconn_core::ProtocolConfig) -> String {
    match config {
        rustconn_core::ProtocolConfig::Ssh(_) => "ssh".to_string(),
        rustconn_core::ProtocolConfig::Rdp(_) => "rdp".to_string(),
        rustconn_core::ProtocolConfig::Vnc(_) => "vnc".to_string(),
        rustconn_core::ProtocolConfig::Spice(_) => "spice".to_string(),
        rustconn_core::ProtocolConfig::Telnet(_) => "telnet".to_string(),
        rustconn_core::ProtocolConfig::ZeroTrust(zt) => {
            // Use provider enum to determine the provider type
            let provider = match zt.provider {
                rustconn_core::models::ZeroTrustProvider::AwsSsm => "aws",
                rustconn_core::models::ZeroTrustProvider::GcpIap => "gcloud",
                rustconn_core::models::ZeroTrustProvider::AzureBastion => "azure",
                rustconn_core::models::ZeroTrustProvider::AzureSsh => "azure_ssh",
                rustconn_core::models::ZeroTrustProvider::OciBastion => "oci",
                rustconn_core::models::ZeroTrustProvider::CloudflareAccess => "cloudflare",
                rustconn_core::models::ZeroTrustProvider::Teleport => "teleport",
                rustconn_core::models::ZeroTrustProvider::TailscaleSsh => "tailscale",
                rustconn_core::models::ZeroTrustProvider::Boundary => "boundary",
                rustconn_core::models::ZeroTrustProvider::Generic => "generic",
            };
            format!("zerotrust:{provider}")
        }
    }
}
