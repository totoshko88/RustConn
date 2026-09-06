//! Common trait for embedded protocol widgets
//!
//! This module provides a common interface for embedded protocol widgets (RDP, VNC, SPICE).
//! It reduces code duplication by defining shared behavior and types.

use gtk4::Box as GtkBox;
use gtk4::prelude::*;

/// Shows a brief status message on a toolbar label, auto-hiding it afterwards.
///
/// Shared by the RDP and VNC toolbars so that a clipboard action reports the
/// same way in both. It lived in `embedded_rdp::clipboard` first, which is why
/// the VNC Copy button had no feedback at all: there was nothing to call.
pub fn show_status_briefly(label: &gtk4::Label, text: &str, duration_secs: u64) {
    label.set_text(text);
    label.set_visible(true);
    let hide = label.clone();
    gtk4::glib::timeout_add_local_once(std::time::Duration::from_secs(duration_secs), move || {
        hide.set_visible(false);
    });
}

/// Common connection state for all embedded protocols
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddedConnectionState {
    /// Not connected
    Disconnected,
    /// Connection in progress
    Connecting,
    /// Successfully connected
    Connected,
    /// Connection error occurred
    Error,
}

impl std::fmt::Display for EmbeddedConnectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => write!(f, "Disconnected"),
            Self::Connecting => write!(f, "Connecting..."),
            Self::Connected => write!(f, "Connected"),
            Self::Error => write!(f, "Error"),
        }
    }
}

/// Common error type for embedded protocol operations
#[derive(Debug, Clone, thiserror::Error)]
pub enum EmbeddedError {
    /// Connection failed
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    /// Protocol not available
    #[error("Protocol not available: {0}")]
    ProtocolNotAvailable(String),
    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigurationError(String),
    /// Already connected
    #[error("Already connected")]
    AlreadyConnected,
    /// Not connected
    #[error("Not connected")]
    NotConnected,
    /// Input/output error
    #[error("I/O error: {0}")]
    IoError(String),
}

/// Type alias for state change callback
pub type StateCallback = Box<dyn Fn(EmbeddedConnectionState) + 'static>;

/// Type alias for error callback
pub type ErrorCallback = Box<dyn Fn(&EmbeddedError) + 'static>;

/// Type alias for reconnect callback
pub type ReconnectCallback = Box<dyn Fn() + 'static>;

/// Common trait for embedded protocol widgets
///
/// This trait defines the shared interface for all embedded protocol widgets,
/// enabling polymorphic handling of RDP, VNC, and SPICE sessions.
pub trait EmbeddedWidget {
    /// Returns the main container widget
    fn widget(&self) -> &GtkBox;

    /// Returns the current connection state
    fn state(&self) -> EmbeddedConnectionState;

    /// Returns whether the widget is using embedded mode (vs external window)
    fn is_embedded(&self) -> bool;

    /// Disconnects the current session
    ///
    /// # Errors
    /// Returns error if disconnect fails
    fn disconnect(&self) -> Result<(), EmbeddedError>;

    /// Reconnects to the last configured session
    ///
    /// # Errors
    /// Returns error if reconnect fails
    fn reconnect(&self) -> Result<(), EmbeddedError>;

    /// Sends Ctrl+Alt+Del to the remote session
    fn send_ctrl_alt_del(&self);

    /// Returns the protocol name (e.g., "RDP", "VNC", "SPICE")
    fn protocol_name(&self) -> &'static str;
}

// `EmbeddedWidgetState` used to live here: a state-plus-callbacks helper meant
// to be shared by the embedded widgets. Nothing ever used it — RDP, VNC and web
// each keep their own `Rc<RefCell<…>>` fields — so it was a spare copy of a
// pattern that had already been written three times, and its only test asserted
// that its constructor produced its own defaults.

// Two more unused helpers used to live here, and both were spare copies rather
// than shared code:
//
// `create_embedded_toolbar` built a Copy/Paste/Ctrl+Alt+Del/Reconnect toolbar
// that no widget asked for — RDP, VNC and web each assemble their own, because
// each needs a different set of buttons. Its existence was actively misleading:
// it set accessible labels on its buttons, so the VNC toolbar looked like it had
// simply forgotten to, when in truth it had never been built from here at all.
//
// `draw_status_overlay` drew the pre-framebuffer placeholder, duplicating
// `embedded_rdp::ui::draw_status_overlay` (which is what RDP actually calls) and
// the inline drawing in `embedded_vnc::ui`. Three copies, one of them unreachable
// — and the unreachable one was the only place some of this was translated.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_state_display() {
        assert_eq!(
            EmbeddedConnectionState::Disconnected.to_string(),
            "Disconnected"
        );
        assert_eq!(
            EmbeddedConnectionState::Connecting.to_string(),
            "Connecting..."
        );
        assert_eq!(EmbeddedConnectionState::Connected.to_string(), "Connected");
        assert_eq!(EmbeddedConnectionState::Error.to_string(), "Error");
    }

    #[test]
    fn test_embedded_error_display() {
        let err = EmbeddedError::ConnectionFailed("timeout".to_string());
        assert!(err.to_string().contains("timeout"));

        let err = EmbeddedError::AlreadyConnected;
        assert_eq!(err.to_string(), "Already connected");
    }
}
