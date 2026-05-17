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

use crate::activity_coordinator::ActivityCoordinator;
use crate::external_window::ExternalWindowManager;
use crate::monitoring::MonitoringCoordinator;
use crate::sidebar::ConnectionSidebar;
use crate::split_view::SplitViewBridge;
use crate::terminal::TerminalNotebook;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use uuid::Uuid;

/// Maximum number of quick connect history entries to keep (LIFO)
const QUICK_CONNECT_HISTORY_MAX: usize = 15;

/// A runtime-only quick connect history entry (not serialized to disk)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickConnectHistoryEntry {
    /// Protocol index: 0=SSH, 1=RDP, 2=VNC, 3=Telnet
    pub protocol_index: u32,
    /// Protocol display name
    pub protocol_name: String,
    /// Host or IP
    pub host: String,
    /// Port number
    pub port: u16,
    /// Username (if any)
    pub username: Option<String>,
}

impl QuickConnectHistoryEntry {
    /// Creates a new quick connect history entry
    #[must_use]
    pub fn new(protocol_index: u32, host: String, port: u16, username: Option<String>) -> Self {
        let protocol_name = match protocol_index {
            0 => "SSH".to_string(),
            1 => "RDP".to_string(),
            2 => "VNC".to_string(),
            3 => "Telnet".to_string(),
            _ => "SSH".to_string(),
        };
        Self {
            protocol_index,
            protocol_name,
            host,
            port,
            username,
        }
    }

    /// Returns a display string for the history entry
    #[must_use]
    pub fn display_string(&self) -> String {
        let user_part = self
            .username
            .as_ref()
            .map_or(String::new(), |u| format!("{u}@"));
        format!(
            "{} — {user_part}{}:{}",
            self.protocol_name, self.host, self.port
        )
    }
}

/// Shared quick connect history (runtime only, max 15 entries, LIFO)
pub type SharedQuickConnectHistory = Rc<RefCell<Vec<QuickConnectHistoryEntry>>>;

/// Adds an entry to the quick connect history (LIFO, deduplicates, max 15)
pub fn add_to_quick_connect_history(
    history: &SharedQuickConnectHistory,
    entry: QuickConnectHistoryEntry,
) {
    let mut hist = history.borrow_mut();
    // Remove duplicate if exists
    hist.retain(|e| e != &entry);
    // Insert at front (most recent first)
    hist.insert(0, entry);
    // Trim to max
    hist.truncate(QUICK_CONNECT_HISTORY_MAX);
}

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

/// Shared monitoring coordinator type
///
/// Uses `Rc` because GTK is single-threaded; no need for `Arc`.
pub type SharedMonitoring = Rc<MonitoringCoordinator>;

/// Shared activity coordinator type for terminal activity/silence detection
///
/// Uses `Rc` because GTK is single-threaded; no need for `Arc`.
pub type SharedActivityCoordinator = Rc<ActivityCoordinator>;

/// Result of starting a connection
///
/// Distinguishes between a synchronously started session, an asynchronous
/// connection attempt (e.g. port check in progress), and a real failure.
/// This prevents the sidebar from flashing "failed" when a port check is
/// still running in the background.
pub enum ConnectionStartResult {
    /// Session was created synchronously — contains the session UUID.
    Started(Uuid),
    /// Connection is being established asynchronously (e.g. port check).
    /// The caller should keep the "connecting" status and not set "failed".
    Pending,
    /// Connection failed to start.
    Failed,
}

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
        rustconn_core::ProtocolConfig::Serial(_) => "serial".to_string(),
        rustconn_core::ProtocolConfig::Sftp(_) => "sftp".to_string(),
        rustconn_core::ProtocolConfig::Kubernetes(_) => "kubernetes".to_string(),
        rustconn_core::ProtocolConfig::Mosh(_) => "mosh".to_string(),
        rustconn_core::ProtocolConfig::Web(_) => "web".to_string(),
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
                rustconn_core::models::ZeroTrustProvider::HoopDev => "hoop",
                rustconn_core::models::ZeroTrustProvider::Generic => "generic",
            };
            format!("zerotrust:{provider}")
        }
    }
}
