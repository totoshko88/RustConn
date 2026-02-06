//! Terminal types and data structures
//!
//! This module contains type definitions for terminal sessions.

use std::path::PathBuf;
use std::rc::Rc;
use uuid::Uuid;

use crate::embedded_rdp::EmbeddedRdpWidget;
use crate::embedded_spice::EmbeddedSpiceWidget;
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
}

/// Session widget storage for non-SSH sessions
#[allow(dead_code)] // Enum variants store widgets for GTK lifecycle
pub enum SessionWidgetStorage {
    /// VNC session widget
    Vnc(Rc<VncSessionWidget>),
    /// Embedded RDP widget (with dynamic resolution)
    EmbeddedRdp(Rc<EmbeddedRdpWidget>),
    /// Embedded SPICE widget (native spice-client)
    EmbeddedSpice(Rc<EmbeddedSpiceWidget>),
}
