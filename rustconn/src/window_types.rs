//! Type definitions and utilities for the main window

use crate::external_window::ExternalWindowManager;
use crate::sidebar::ConnectionSidebar;
use crate::split_view::{SplitViewBridge, TabSplitManager};
use crate::terminal::TerminalNotebook;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use uuid::Uuid;

/// Shared sidebar type
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Shared terminal notebook type
pub type SharedNotebook = Rc<TerminalNotebook>;

/// Shared split view type (uses new SplitViewBridge implementation)
pub type SharedSplitView = Rc<SplitViewBridge>;

/// Shared tab split manager type (new implementation)
pub type SharedTabSplitManager = Rc<RefCell<TabSplitManager>>;

/// Map of session IDs to their split view bridges
/// Each session that has been split gets its own independent SplitViewBridge
/// Requirement 3: Each tab maintains its own independent split layout
pub type SessionSplitBridges = Rc<RefCell<HashMap<Uuid, Rc<SplitViewBridge>>>>;

/// Shared external window manager type
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
