//! Terminal types and data structures
//!
//! This module contains type definitions for terminal sessions.

use std::cell::RefCell;
use std::path::PathBuf;
use std::process::Child;
use std::rc::Rc;

use uuid::Uuid;

use crate::embedded_rdp::EmbeddedRdpWidget;
#[cfg(feature = "web-embedded")]
use crate::embedded_web::EmbeddedWebWidget;
use crate::session::VncSessionWidget;

/// Terminal session information
#[derive(Debug, Clone)]
pub struct TerminalSession {
    /// Session UUID for session management
    pub id: Uuid,
    /// Connection ID this session is for
    pub connection_id: Uuid,
    /// Connection name for display
    pub name: String,
    /// Protocol type (ssh, rdp, vnc, spice)
    pub protocol: String,
    /// Whether this is an embedded terminal or external window
    pub is_embedded: bool,
    /// Log file path if logging is enabled
    pub log_file: Option<PathBuf>,
    /// History entry ID for tracking connection history
    pub history_entry_id: Option<Uuid>,
    /// Tab group name (e.g., "Production", "Staging")
    pub tab_group: Option<String>,
    /// Color index from palette for visual grouping
    pub tab_color_index: Option<usize>,
    /// Timestamp when the session was created/connected
    pub connected_at: chrono::DateTime<chrono::Utc>,
}

impl TerminalSession {
    /// Formats the session duration as a human-readable string.
    pub fn format_duration(&self) -> String {
        let elapsed = chrono::Utc::now()
            .signed_duration_since(self.connected_at)
            .num_seconds()
            .max(0);
        let hours = elapsed / 3600;
        let minutes = (elapsed % 3600) / 60;
        let seconds = elapsed % 60;
        if hours > 0 {
            format!("{hours}h {minutes:02}m")
        } else if minutes > 0 {
            format!("{minutes}m {seconds:02}s")
        } else {
            format!("{seconds}s")
        }
    }
}

/// Session widget storage for non-SSH sessions
pub enum SessionWidgetStorage {
    /// VNC session widget
    Vnc(Rc<VncSessionWidget>),
    /// Embedded RDP widget (with dynamic resolution)
    EmbeddedRdp(Rc<EmbeddedRdpWidget>),
    /// Embedded Web browser widget (WebKitGTK 6.0)
    #[cfg(feature = "web-embedded")]
    EmbeddedWeb(Rc<EmbeddedWebWidget>),
    /// External process (xfreerdp, vncviewer, etc.) — killed on tab close
    ExternalProcess(Rc<RefCell<Option<Child>>>),
}
