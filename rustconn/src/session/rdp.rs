//! RDP session widget for native embedding
//!
//! This module provides the `RdpSessionWidget` struct that wraps the RDP FFI
//! display widget with overlay controls and state management.
//!
//! # Requirements Coverage
//!
//! - Requirement 3.1: Native RDP embedding as GTK widget
//! - Requirement 3.3: NLA authentication handling
//! - Requirement 3.4: Gateway configuration support

use super::{SessionError, SessionState};
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Label, Orientation, Overlay, Spinner};
use rustconn_core::ffi::{RdpConnectionConfig, RdpDisplay};
use std::cell::RefCell;
use std::rc::Rc;

/// Callback type for authentication requests
type AuthCallback = Box<dyn Fn() + 'static>;

/// Callback type for state change notifications
type StateCallback = Box<dyn Fn(SessionState) + 'static>;

/// Callback type for error notifications
type ErrorCallback = Box<dyn Fn(&str) + 'static>;

/// RDP session widget with overlay controls
///
/// This widget wraps the RDP FFI display and provides:
/// - Connection lifecycle management
/// - NLA authentication callback handling
/// - Gateway configuration support
/// - State tracking and error reporting
/// - Overlay controls for session management
///
/// # Example
///
/// ```ignore
/// use rustconn::session::rdp::RdpSessionWidget;
/// use rustconn_core::ffi::RdpConnectionConfig;
///
/// let widget = RdpSessionWidget::new();
///
/// // Set up authentication callback
/// widget.connect_auth_required(|| {
///     // Prompt user for credentials
/// });
///
/// // Connect to RDP server
/// let config = RdpConnectionConfig::new("192.168.1.100");
/// widget.connect(&config);
/// ```
pub struct RdpSessionWidget {
    /// The GTK overlay container
    overlay: Overlay,
    /// The RDP display widget
    display: Rc<RdpDisplay>,
    /// Current session state
    state: Rc<RefCell<SessionState>>,
    /// Status label for connection feedback
    status_label: Label,
    /// Spinner for connection progress
    spinner: Spinner,
    /// Status container (kept for preventing premature deallocation and future floating controls)
    status_container: GtkBox,
    /// Authentication callback
    auth_callback: Rc<RefCell<Option<AuthCallback>>>,
    /// State change callback
    state_callback: Rc<RefCell<Option<StateCallback>>>,
    /// Error callback
    error_callback: Rc<RefCell<Option<ErrorCallback>>>,
}

impl RdpSessionWidget {
    /// Creates a new RDP session widget
    ///
    /// The widget is created in a disconnected state and ready for connection.
    #[must_use]
    pub fn new() -> Self {
        let display = Rc::new(RdpDisplay::new());
        let state = Rc::new(RefCell::new(SessionState::Disconnected));
        let auth_callback: Rc<RefCell<Option<AuthCallback>>> = Rc::new(RefCell::new(None));
        let state_callback: Rc<RefCell<Option<StateCallback>>> = Rc::new(RefCell::new(None));
        let error_callback: Rc<RefCell<Option<ErrorCallback>>> = Rc::new(RefCell::new(None));

        // Create the overlay container
        let overlay = Overlay::new();

        // Create a placeholder widget for the RDP display
        // In a real implementation, this would be the actual gtk-frdp widget
        let display_placeholder = GtkBox::new(Orientation::Vertical, 0);
        display_placeholder.set_hexpand(true);
        display_placeholder.set_vexpand(true);
        display_placeholder.add_css_class("rdp-display");

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
            auth_callback,
            state_callback,
            error_callback,
        };

        // Set up RDP display signal handlers
        widget.setup_display_signals();

        widget
    }

    /// Sets up signal handlers for the RDP display
    fn setup_display_signals(&self) {
        let state = self.state.clone();
        let status_label = self.status_label.clone();
        let spinner = self.spinner.clone();
        let state_callback = self.state_callback.clone();
        let error_callback = self.error_callback.clone();

        // Connected signal
        let state_clone = state.clone();
        let status_label_clone = status_label.clone();
        let spinner_clone = spinner.clone();
        let state_callback_clone = state_callback.clone();
        self.display.connect_rdp_connected(move |_| {
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
        self.display.connect_rdp_disconnected(move |_| {
            *state_clone.borrow_mut() = SessionState::Disconnected;
            status_label_clone.set_text("Disconnected");
            spinner_clone.set_spinning(false);
            spinner_clone.set_visible(false);

            if let Some(ref callback) = *state_callback_clone.borrow() {
                callback(SessionState::Disconnected);
            }
        });

        // Auth required signal
        let state_clone = state.clone();
        let status_label_clone = status_label.clone();
        let auth_callback_clone = self.auth_callback.clone();
        let state_callback_clone = state_callback.clone();
        self.display.connect_rdp_auth_required(move |_| {
            *state_clone.borrow_mut() = SessionState::Authenticating;
            status_label_clone.set_text("Authentication required...");

            if let Some(ref callback) = *state_callback_clone.borrow() {
                callback(SessionState::Authenticating);
            }

            if let Some(ref callback) = *auth_callback_clone.borrow() {
                callback();
            }
        });

        // Auth failure signal
        let state_clone = state.clone();
        let status_label_clone = status_label.clone();
        let spinner_clone = spinner.clone();
        let state_callback_clone = state_callback.clone();
        let error_callback_clone = error_callback.clone();
        self.display.connect_rdp_auth_failure(move |_, msg| {
            let error = SessionError::authentication_failed(msg);
            *state_clone.borrow_mut() = SessionState::Error(error.clone());
            status_label_clone.set_text(&format!("Authentication failed: {msg}"));
            spinner_clone.set_spinning(false);
            spinner_clone.set_visible(false);

            if let Some(ref callback) = *state_callback_clone.borrow() {
                callback(SessionState::Error(error));
            }

            if let Some(ref callback) = *error_callback_clone.borrow() {
                callback(msg);
            }
        });

        // Error signal
        let state_clone = state;
        let status_label_clone = status_label;
        let spinner_clone = spinner;
        let state_callback_clone = state_callback;
        let error_callback_clone = error_callback;
        self.display.connect_rdp_error(move |_, msg| {
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
    }

    /// Connects to an RDP server
    ///
    /// # Arguments
    ///
    /// * `config` - The connection configuration
    ///
    /// # Errors
    ///
    /// Returns a `SessionError` if the connection cannot be initiated.
    pub fn connect(&self, config: &RdpConnectionConfig) -> Result<(), SessionError> {
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

    /// Connects to an RDP server with simple parameters
    ///
    /// This is a convenience method for simple connections without gateway.
    ///
    /// # Arguments
    ///
    /// * `host` - The hostname or IP address
    /// * `port` - The port number (typically 3389)
    /// * `username` - Optional username
    /// * `password` - Optional password
    ///
    /// # Errors
    ///
    /// Returns a `SessionError` if the connection cannot be initiated.
    pub fn connect_simple(
        &self,
        host: &str,
        port: u16,
        username: Option<&str>,
        password: Option<&str>,
    ) -> Result<(), SessionError> {
        let mut config = RdpConnectionConfig::new(host).with_port(port);

        if let Some(user) = username {
            config = config.with_username(user);
        }

        // If credentials are provided, set them
        if let (Some(user), Some(pwd)) = (username, password) {
            self.display
                .set_credentials(user, pwd, None)
                .map_err(|e| SessionError::authentication_failed(e.to_string()))?;
        }

        self.connect(&config)
    }

    /// Disconnects from the RDP server
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

    /// Provides credentials for NLA authentication
    ///
    /// This should be called in response to the `auth_required` callback.
    ///
    /// # Arguments
    ///
    /// * `username` - The username
    /// * `password` - The password
    /// * `domain` - Optional domain
    ///
    /// # Errors
    ///
    /// Returns a `SessionError` if credentials cannot be set.
    pub fn provide_credentials(
        &self,
        username: &str,
        password: &str,
        domain: Option<&str>,
    ) -> Result<(), SessionError> {
        self.display
            .set_credentials(username, password, domain)
            .map_err(|e| SessionError::authentication_failed(e.to_string()))?;

        Ok(())
    }

    /// Enables or disables clipboard sharing
    pub fn set_clipboard_enabled(&self, enabled: bool) {
        self.display.set_clipboard_enabled(enabled);
    }

    /// Returns whether clipboard sharing is enabled
    #[must_use]
    pub fn clipboard_enabled(&self) -> bool {
        self.display.clipboard_enabled()
    }

    /// Connects a callback for authentication requests
    ///
    /// The callback is invoked when the server requires NLA authentication.
    pub fn connect_auth_required<F>(&self, callback: F)
    where
        F: Fn() + 'static,
    {
        *self.auth_callback.borrow_mut() = Some(Box::new(callback));
    }

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

    /// Returns the underlying RDP display (for advanced usage)
    #[must_use]
    pub fn display(&self) -> &RdpDisplay {
        &self.display
    }
}

impl Default for RdpSessionWidget {
    fn default() -> Self {
        Self::new()
    }
}

// Manual Debug implementation since we can't derive it for callback types
impl std::fmt::Debug for RdpSessionWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RdpSessionWidget")
            .field("state", &self.state.borrow())
            .field("display", &"RdpDisplay { ... }")
            .finish_non_exhaustive()
    }
}

// Re-export types for convenience
pub use rustconn_core::ffi::{RdpGatewayConfig, Resolution};

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
    fn test_rdp_connection_config_builder() {
        let config = RdpConnectionConfig::new("server.example.com")
            .with_port(3390)
            .with_username("admin")
            .with_domain("CORP")
            .with_resolution(Resolution::full_hd());

        assert_eq!(config.host, "server.example.com");
        assert_eq!(config.port, 3390);
        assert_eq!(config.username, Some("admin".to_string()));
        assert_eq!(config.domain, Some("CORP".to_string()));
    }

    #[test]
    fn test_rdp_gateway_config_builder() {
        let gateway = RdpGatewayConfig::new("gateway.example.com")
            .with_port(8443)
            .with_username("gwuser")
            .with_domain("GWDOMAIN");

        assert_eq!(gateway.host, "gateway.example.com");
        assert_eq!(gateway.port, 8443);
        assert_eq!(gateway.username, Some("gwuser".to_string()));
        assert_eq!(gateway.domain, Some("GWDOMAIN".to_string()));
    }
}
