//! SPICE session widget for native embedding
//!
//! This module provides the `SpiceSessionWidget` struct that wraps the SPICE FFI
//! display widget with overlay controls and state management.
//!
//! # Requirements Coverage
//!
//! - Requirement 4.2: Native SPICE embedding as GTK widget
//! - Requirement 4.5: SPICE agent features (clipboard, resize)

use super::{SessionError, SessionState};
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Label, Orientation, Overlay, Spinner};
use rustconn_core::ffi::{SpiceChannelEvent, SpiceConnectionConfig, SpiceDisplay};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

/// Callback type for state change notifications
type StateCallback = Box<dyn Fn(SessionState) + 'static>;

/// Callback type for error notifications
type ErrorCallback = Box<dyn Fn(&str) + 'static>;

/// Callback type for channel event notifications
type ChannelEventCallback = Box<dyn Fn(SpiceChannelEvent) + 'static>;

/// SPICE session widget with overlay controls
///
/// This widget wraps the SPICE FFI display and provides:
/// - Connection lifecycle management
/// - SPICE agent feature handling (clipboard, resize)
/// - USB redirection support
/// - Shared folder support
/// - State tracking and error reporting
/// - Overlay controls for session management
///
/// # Example
///
/// ```ignore
/// use rustconn::session::spice::SpiceSessionWidget;
/// use rustconn_core::ffi::SpiceConnectionConfig;
///
/// let widget = SpiceSessionWidget::new();
///
/// // Set up state change callback
/// widget.connect_state_changed(|state| {
///     println!("State changed: {:?}", state);
/// });
///
/// // Connect to SPICE server
/// let config = SpiceConnectionConfig::new("192.168.1.100", 5900);
/// widget.connect(&config);
/// ```
pub struct SpiceSessionWidget {
    /// The GTK overlay container
    overlay: Overlay,
    /// The SPICE display widget
    display: Rc<SpiceDisplay>,
    /// Current session state
    state: Rc<RefCell<SessionState>>,
    /// Status label for connection feedback
    status_label: Label,
    /// Spinner for connection progress
    spinner: Spinner,
    /// Status container (kept for preventing premature deallocation and future floating controls)
    status_container: GtkBox,
    /// State change callback
    state_callback: Rc<RefCell<Option<StateCallback>>>,
    /// Error callback
    error_callback: Rc<RefCell<Option<ErrorCallback>>>,
    /// Channel event callback
    channel_event_callback: Rc<RefCell<Option<ChannelEventCallback>>>,
}

impl SpiceSessionWidget {
    /// Creates a new SPICE session widget
    ///
    /// The widget is created in a disconnected state and ready for connection.
    #[must_use]
    pub fn new() -> Self {
        let display = Rc::new(SpiceDisplay::new());
        let state = Rc::new(RefCell::new(SessionState::Disconnected));
        let state_callback: Rc<RefCell<Option<StateCallback>>> = Rc::new(RefCell::new(None));
        let error_callback: Rc<RefCell<Option<ErrorCallback>>> = Rc::new(RefCell::new(None));
        let channel_event_callback: Rc<RefCell<Option<ChannelEventCallback>>> =
            Rc::new(RefCell::new(None));

        // Create the overlay container
        let overlay = Overlay::new();

        // Create a placeholder widget for the SPICE display
        // In a real implementation, this would be the actual spice-gtk widget
        let display_placeholder = GtkBox::new(Orientation::Vertical, 0);
        display_placeholder.set_hexpand(true);
        display_placeholder.set_vexpand(true);
        display_placeholder.add_css_class("spice-display");

        // Create status container for connection feedback
        let status_container = GtkBox::new(Orientation::Vertical, 12);
        status_container.set_halign(gtk4::Align::Center);
        status_container.set_valign(gtk4::Align::Center);

        let spinner = Spinner::new();
        spinner.set_spinning(false);
        spinner.set_visible(false);

        let status_label = Label::new(Some("Disconnected"));
        status_label.add_css_class("dim-label");

        status_container.append(&spinner);
        status_container.append(&status_label);

        // Set up the overlay
        overlay.set_child(Some(&display_placeholder));
        overlay.add_overlay(&status_container);

        let widget = Self {
            overlay,
            display,
            state,
            status_label,
            spinner,
            status_container,
            state_callback,
            error_callback,
            channel_event_callback,
        };

        // Set up SPICE display signal handlers
        widget.setup_display_signals();

        widget
    }

    /// Sets up signal handlers for the SPICE display
    fn setup_display_signals(&self) {
        let state = self.state.clone();
        let status_label = self.status_label.clone();
        let spinner = self.spinner.clone();
        let state_callback = self.state_callback.clone();
        let error_callback = self.error_callback.clone();
        let channel_event_callback = self.channel_event_callback.clone();

        // Connected signal
        let state_clone = state.clone();
        let status_label_clone = status_label.clone();
        let spinner_clone = spinner.clone();
        let state_callback_clone = state_callback.clone();
        self.display.connect_spice_connected(move |_| {
            *state_clone.borrow_mut() = SessionState::Connected;
            status_label_clone.set_text("Connected");
            spinner_clone.set_spinning(false);
            spinner_clone.set_visible(false);

            if let Some(ref callback) = *state_callback_clone.borrow() {
                callback(SessionState::Connected);
            }
        });

        // Disconnected signal
        let state_clone = state.clone();
        let status_label_clone = status_label.clone();
        let spinner_clone = spinner.clone();
        let state_callback_clone = state_callback.clone();
        self.display.connect_spice_disconnected(move |_| {
            *state_clone.borrow_mut() = SessionState::Disconnected;
            status_label_clone.set_text("Disconnected");
            spinner_clone.set_spinning(false);
            spinner_clone.set_visible(false);

            if let Some(ref callback) = *state_callback_clone.borrow() {
                callback(SessionState::Disconnected);
            }
        });

        // Error signal
        let state_clone = state.clone();
        let status_label_clone = status_label.clone();
        let spinner_clone = spinner.clone();
        let state_callback_clone = state_callback.clone();
        let error_callback_clone = error_callback;
        self.display.connect_spice_error(move |_, msg| {
            let error = SessionError::connection_failed(msg);
            *state_clone.borrow_mut() = SessionState::Error(error.clone());
            status_label_clone.set_text(&format!("Error: {msg}"));
            spinner_clone.set_spinning(false);
            spinner_clone.set_visible(false);

            if let Some(ref callback) = *state_callback_clone.borrow() {
                callback(SessionState::Error(error));
            }

            if let Some(ref callback) = *error_callback_clone.borrow() {
                callback(msg);
            }
        });

        // Channel event signal
        let state_clone = state;
        let status_label_clone = status_label;
        let spinner_clone = spinner;
        let state_callback_clone = state_callback;
        self.display.connect_spice_channel_event(move |_, event| {
            match event {
                SpiceChannelEvent::Opened => {
                    // Channel opened - update status
                    status_label_clone.set_text("Channel opened");
                }
                SpiceChannelEvent::Closed => {
                    // Channel closed - may indicate disconnection
                    *state_clone.borrow_mut() = SessionState::Disconnected;
                    status_label_clone.set_text("Channel closed");
                    spinner_clone.set_spinning(false);
                    spinner_clone.set_visible(false);

                    if let Some(ref callback) = *state_callback_clone.borrow() {
                        callback(SessionState::Disconnected);
                    }
                }
                SpiceChannelEvent::Error => {
                    // Channel error
                    let error = SessionError::protocol_error("Channel error");
                    *state_clone.borrow_mut() = SessionState::Error(error.clone());
                    status_label_clone.set_text("Channel error");
                    spinner_clone.set_spinning(false);
                    spinner_clone.set_visible(false);

                    if let Some(ref callback) = *state_callback_clone.borrow() {
                        callback(SessionState::Error(error));
                    }
                }
            }

            if let Some(ref callback) = *channel_event_callback.borrow() {
                callback(event);
            }
        });
    }

    /// Connects to a SPICE server
    ///
    /// # Arguments
    ///
    /// * `config` - The connection configuration
    ///
    /// # Errors
    ///
    /// Returns a `SessionError` if the connection cannot be initiated.
    pub fn connect(&self, config: &SpiceConnectionConfig) -> Result<(), SessionError> {
        // Check current state
        let current_state = self.state.borrow().clone();
        if !current_state.can_transition_to(&SessionState::Connecting) {
            return Err(SessionError::connection_failed(format!(
                "Cannot connect from state: {current_state}"
            )));
        }

        // Update state to connecting
        *self.state.borrow_mut() = SessionState::Connecting;
        self.status_label
            .set_text(&format!("Connecting to {}:{}...", config.host, config.port));
        self.spinner.set_visible(true);
        self.spinner.set_spinning(true);

        // Notify state change
        if let Some(ref callback) = *self.state_callback.borrow() {
            callback(SessionState::Connecting);
        }

        // Initiate connection
        self.display
            .open(config)
            .map_err(|e| SessionError::connection_failed(e.to_string()))?;

        Ok(())
    }

    /// Connects to a SPICE server with simple parameters
    ///
    /// This is a convenience method for simple connections.
    ///
    /// # Arguments
    ///
    /// * `host` - The hostname or IP address
    /// * `port` - The port number (typically 5900)
    ///
    /// # Errors
    ///
    /// Returns a `SessionError` if the connection cannot be initiated.
    pub fn connect_simple(&self, host: &str, port: u16) -> Result<(), SessionError> {
        let config = SpiceConnectionConfig::new(host, port);
        self.connect(&config)
    }

    /// Disconnects from the SPICE server
    pub fn disconnect(&self) {
        self.display.close();
        *self.state.borrow_mut() = SessionState::Disconnected;
        self.status_label.set_text("Disconnected");
        self.spinner.set_spinning(false);
        self.spinner.set_visible(false);

        if let Some(ref callback) = *self.state_callback.borrow() {
            callback(SessionState::Disconnected);
        }
    }

    /// Returns the GTK widget for embedding in containers
    #[must_use]
    pub fn widget(&self) -> &gtk4::Widget {
        self.overlay.upcast_ref()
    }

    /// Returns the current session state
    #[must_use]
    pub fn state(&self) -> SessionState {
        self.state.borrow().clone()
    }

    /// Returns whether the session is connected
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.state.borrow().is_connected()
    }

    // ========================================================================
    // SPICE Agent Features
    // ========================================================================

    /// Enables or disables USB redirection
    ///
    /// When enabled, USB devices can be redirected to the remote VM.
    ///
    /// # Arguments
    ///
    /// * `enabled` - Whether to enable USB redirection
    ///
    /// # Errors
    ///
    /// Returns a `SessionError` if USB redirection cannot be configured.
    pub fn set_usb_redirection(&self, enabled: bool) -> Result<(), SessionError> {
        self.display
            .set_usb_redirection(enabled)
            .map_err(|e| SessionError::protocol_error(e.to_string()))
    }

    /// Returns whether USB redirection is active
    #[must_use]
    pub fn usb_redirection_active(&self) -> bool {
        self.display.usb_redirection_active()
    }

    /// Adds a shared folder
    ///
    /// # Arguments
    ///
    /// * `path` - Local path to share
    /// * `name` - Name visible to the remote VM
    ///
    /// # Errors
    ///
    /// Returns a `SessionError` if the folder cannot be added.
    pub fn add_shared_folder(&self, path: &Path, name: &str) -> Result<(), SessionError> {
        self.display
            .add_shared_folder(path, name)
            .map_err(|e| SessionError::protocol_error(e.to_string()))
    }

    /// Removes a shared folder by name
    ///
    /// # Arguments
    ///
    /// * `name` - Name of the folder to remove
    ///
    /// # Returns
    ///
    /// `true` if the folder was found and removed, `false` otherwise.
    #[must_use]
    pub fn remove_shared_folder(&self, name: &str) -> bool {
        self.display.remove_shared_folder(name)
    }

    /// Returns the list of shared folder names
    #[must_use]
    pub fn shared_folder_names(&self) -> Vec<String> {
        self.display
            .shared_folders()
            .iter()
            .map(|f| f.name.clone())
            .collect()
    }

    /// Enables or disables clipboard sharing
    ///
    /// # Arguments
    ///
    /// * `enabled` - Whether to enable clipboard sharing
    pub fn set_clipboard_enabled(&self, enabled: bool) {
        self.display.set_clipboard_enabled(enabled);
    }

    /// Returns whether clipboard sharing is enabled
    #[must_use]
    pub fn clipboard_enabled(&self) -> bool {
        self.display.clipboard_enabled()
    }

    // ========================================================================
    // Signal Connections
    // ========================================================================

    /// Connects a callback for state changes
    ///
    /// The callback is invoked whenever the session state changes.
    pub fn connect_state_changed<F>(&self, callback: F)
    where
        F: Fn(SessionState) + 'static,
    {
        *self.state_callback.borrow_mut() = Some(Box::new(callback));
    }

    /// Connects a callback for error notifications
    ///
    /// The callback is invoked when an error occurs.
    pub fn connect_error<F>(&self, callback: F)
    where
        F: Fn(&str) + 'static,
    {
        *self.error_callback.borrow_mut() = Some(Box::new(callback));
    }

    /// Connects a callback for channel events
    ///
    /// The callback is invoked when a SPICE channel event occurs.
    pub fn connect_channel_event<F>(&self, callback: F)
    where
        F: Fn(SpiceChannelEvent) + 'static,
    {
        *self.channel_event_callback.borrow_mut() = Some(Box::new(callback));
    }

    /// Returns the underlying SPICE display (for advanced usage)
    #[must_use]
    pub fn display(&self) -> &SpiceDisplay {
        &self.display
    }
}

impl Default for SpiceSessionWidget {
    fn default() -> Self {
        Self::new()
    }
}

// Manual Debug implementation since we can't derive it for callback types
impl std::fmt::Debug for SpiceSessionWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpiceSessionWidget")
            .field("state", &self.state.borrow())
            .field("display", &"SpiceDisplay { ... }")
            .finish_non_exhaustive()
    }
}

// Re-export types for convenience
pub use rustconn_core::ffi::{SpiceSharedFolder, SpiceTlsConfig};

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require GTK to be initialized, which may not be available
    // in all test environments. The property tests in rustconn-core handle
    // the core logic testing without GTK dependencies.

    #[test]
    fn test_session_state_transitions() {
        // Test that state transitions are properly validated
        let disconnected = SessionState::Disconnected;
        assert!(disconnected.can_transition_to(&SessionState::Connecting));
        assert!(!disconnected.can_transition_to(&SessionState::Connected));

        let connecting = SessionState::Connecting;
        assert!(connecting.can_transition_to(&SessionState::Connected));
        assert!(connecting.can_transition_to(&SessionState::Authenticating));
        assert!(connecting.can_transition_to(&SessionState::Disconnected));

        let connected = SessionState::Connected;
        assert!(connected.can_transition_to(&SessionState::Disconnected));
        assert!(!connected.can_transition_to(&SessionState::Connecting));
    }

    #[test]
    fn test_spice_connection_config_builder() {
        let config = SpiceConnectionConfig::new("server.example.com", 5900)
            .with_usb_redirection(true)
            .with_clipboard(true);

        assert_eq!(config.host, "server.example.com");
        assert_eq!(config.port, 5900);
        assert!(config.usb_redirection);
        assert!(config.clipboard_enabled);
    }
}
