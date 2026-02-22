//! Common trait for embedded protocol widgets
//!
//! This module provides a common interface for embedded protocol widgets (RDP, VNC, SPICE).
//! It reduces code duplication by defining shared behavior and types.

use crate::i18n::i18n;
use gtk4::Box as GtkBox;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

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

/// Helper struct for managing common widget state
pub struct EmbeddedWidgetState {
    /// Current connection state
    pub state: Rc<RefCell<EmbeddedConnectionState>>,
    /// Whether using embedded mode
    pub is_embedded: Rc<RefCell<bool>>,
    /// State change callback
    pub on_state_changed: Rc<RefCell<Option<StateCallback>>>,
    /// Error callback
    pub on_error: Rc<RefCell<Option<ErrorCallback>>>,
    /// Reconnect callback
    pub on_reconnect: Rc<RefCell<Option<ReconnectCallback>>>,
}

impl EmbeddedWidgetState {
    /// Creates a new widget state manager
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(EmbeddedConnectionState::Disconnected)),
            is_embedded: Rc::new(RefCell::new(false)),
            on_state_changed: Rc::new(RefCell::new(None)),
            on_error: Rc::new(RefCell::new(None)),
            on_reconnect: Rc::new(RefCell::new(None)),
        }
    }

    /// Sets the connection state and notifies callback
    pub fn set_state(&self, new_state: EmbeddedConnectionState) {
        *self.state.borrow_mut() = new_state;
        if let Some(ref callback) = *self.on_state_changed.borrow() {
            callback(new_state);
        }
    }

    /// Reports an error and notifies callback
    pub fn report_error(&self, error: &EmbeddedError) {
        self.set_state(EmbeddedConnectionState::Error);
        if let Some(ref callback) = *self.on_error.borrow() {
            callback(error);
        }
    }

    /// Connects a state change callback
    pub fn connect_state_changed<F>(&self, callback: F)
    where
        F: Fn(EmbeddedConnectionState) + 'static,
    {
        *self.on_state_changed.borrow_mut() = Some(Box::new(callback));
    }

    /// Connects an error callback
    pub fn connect_error<F>(&self, callback: F)
    where
        F: Fn(&EmbeddedError) + 'static,
    {
        *self.on_error.borrow_mut() = Some(Box::new(callback));
    }

    /// Connects a reconnect callback
    pub fn connect_reconnect<F>(&self, callback: F)
    where
        F: Fn() + 'static,
    {
        *self.on_reconnect.borrow_mut() = Some(Box::new(callback));
    }
}

impl Default for EmbeddedWidgetState {
    fn default() -> Self {
        Self::new()
    }
}

/// Creates a standard toolbar for embedded widgets
///
/// Returns a tuple of (toolbar_box, copy_button, paste_button, ctrl_alt_del_button, reconnect_button, status_label)
#[must_use]
pub fn create_embedded_toolbar() -> (
    GtkBox,
    gtk4::Button,
    gtk4::Button,
    gtk4::Button,
    gtk4::Button,
    gtk4::Label,
) {
    let toolbar = GtkBox::new(gtk4::Orientation::Horizontal, 4);
    toolbar.set_margin_start(4);
    toolbar.set_margin_end(4);
    toolbar.set_margin_top(4);
    toolbar.set_margin_bottom(4);
    toolbar.set_halign(gtk4::Align::End);

    // Status label (hidden by default)
    let status_label = gtk4::Label::new(None);
    status_label.set_visible(false);
    status_label.set_margin_end(8);
    status_label.add_css_class("dim-label");
    toolbar.append(&status_label);

    // Copy button
    let copy_button = gtk4::Button::with_label(&i18n("Copy"));
    copy_button.set_tooltip_text(Some(&i18n("Copy from remote session to local clipboard")));
    toolbar.append(&copy_button);

    // Paste button
    let paste_button = gtk4::Button::with_label(&i18n("Paste"));
    paste_button.set_tooltip_text(Some(&i18n("Paste from local clipboard to remote session")));
    toolbar.append(&paste_button);

    // Separator
    let separator = gtk4::Separator::new(gtk4::Orientation::Vertical);
    separator.set_margin_start(4);
    separator.set_margin_end(4);
    toolbar.append(&separator);

    // Ctrl+Alt+Del button
    let ctrl_alt_del_button = gtk4::Button::with_label("Ctrl+Alt+Del");
    ctrl_alt_del_button.add_css_class("suggested-action");
    ctrl_alt_del_button.set_tooltip_text(Some(&i18n("Send Ctrl+Alt+Del to remote session")));
    toolbar.append(&ctrl_alt_del_button);

    // Reconnect button (hidden by default)
    let reconnect_button = gtk4::Button::with_label(&i18n("Reconnect"));
    reconnect_button.add_css_class("suggested-action");
    reconnect_button.set_tooltip_text(Some(&i18n("Reconnect to the remote session")));
    reconnect_button.set_visible(false);
    toolbar.append(&reconnect_button);

    // Hide toolbar initially
    toolbar.set_visible(false);

    (
        toolbar,
        copy_button,
        paste_button,
        ctrl_alt_del_button,
        reconnect_button,
        status_label,
    )
}

/// Draws a status overlay on a Cairo context
///
/// This is used when the embedded widget is not connected or in external mode.
#[allow(clippy::too_many_arguments)]
pub fn draw_status_overlay(
    cr: &gtk4::cairo::Context,
    width: i32,
    height: i32,
    protocol_letter: &str,
    protocol_color: (f64, f64, f64),
    host: &str,
    state: EmbeddedConnectionState,
    is_embedded: bool,
) {
    // Dark background
    cr.set_source_rgb(0.12, 0.12, 0.14);
    let _ = cr.paint();

    cr.select_font_face(
        "Sans",
        gtk4::cairo::FontSlant::Normal,
        gtk4::cairo::FontWeight::Normal,
    );

    let center_y = f64::from(height) / 2.0 - 40.0;

    // Protocol icon (circle with letter)
    cr.set_source_rgb(protocol_color.0, protocol_color.1, protocol_color.2);
    cr.arc(
        f64::from(width) / 2.0,
        center_y,
        40.0,
        0.0,
        2.0 * std::f64::consts::PI,
    );
    let _ = cr.fill();

    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.set_font_size(32.0);
    if let Ok(extents) = cr.text_extents(protocol_letter) {
        cr.move_to(
            f64::from(width) / 2.0 - extents.width() / 2.0,
            center_y + extents.height() / 2.0,
        );
        let _ = cr.show_text(protocol_letter);
    }

    // Host name
    cr.set_source_rgb(0.9, 0.9, 0.9);
    cr.set_font_size(18.0);
    if let Ok(extents) = cr.text_extents(host) {
        cr.move_to((f64::from(width) - extents.width()) / 2.0, center_y + 70.0);
        let _ = cr.show_text(host);
    }

    // Status message
    cr.set_font_size(13.0);
    let status_text = match state {
        EmbeddedConnectionState::Disconnected => "Disconnected",
        EmbeddedConnectionState::Connecting => "Connecting...",
        EmbeddedConnectionState::Connected if !is_embedded => "Session running in external window",
        EmbeddedConnectionState::Connected => "Connected",
        EmbeddedConnectionState::Error => "Connection error",
    };

    let color = match state {
        EmbeddedConnectionState::Connected => (0.6, 0.8, 0.6),
        EmbeddedConnectionState::Connecting => (0.8, 0.8, 0.6),
        EmbeddedConnectionState::Error => (0.8, 0.4, 0.4),
        EmbeddedConnectionState::Disconnected => (0.5, 0.5, 0.5),
    };
    cr.set_source_rgb(color.0, color.1, color.2);

    if let Ok(extents) = cr.text_extents(status_text) {
        cr.move_to((f64::from(width) - extents.width()) / 2.0, center_y + 100.0);
        let _ = cr.show_text(status_text);
    }
}

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

    #[test]
    fn test_widget_state_default() {
        let state = EmbeddedWidgetState::new();
        assert_eq!(*state.state.borrow(), EmbeddedConnectionState::Disconnected);
        assert!(!*state.is_embedded.borrow());
    }
}
