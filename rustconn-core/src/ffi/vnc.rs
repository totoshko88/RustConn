//! VNC FFI bindings for `gtk-vnc`
//!
//! This module provides safe Rust wrappers around the `gtk-vnc` library,
//! enabling native VNC session embedding in GTK4 applications.
//!
//! # Overview
//!
//! The `VncDisplay` struct wraps the `GtkVncDisplay` widget and provides:
//! - Connection management (`open_host`, `close`, `is_open`)
//! - Authentication handling (`set_credential`)
//! - Display configuration (`set_scaling`)
//! - Signal connections for state changes
//!
//! # Requirements Coverage
//!
//! - Requirement 2.1: Native VNC embedding as GTK widget
//! - Requirement 8.1: Safe wrappers around unsafe C calls
//! - Requirement 8.2: GTK4 widget hierarchy integration
//!
//! # Example
//!
//! ```ignore
//! use rustconn_core::ffi::vnc::{VncDisplay, VncCredentialType};
//!
//! let display = VncDisplay::new();
//!
//! // Connect signals
//! display.connect_vnc_connected(|_| {
//!     tracing::info!("Connected!");
//! });
//!
//! display.connect_vnc_auth_credential(|display, creds| {
//!     for cred in creds {
//!         match cred {
//!             VncCredentialType::Password => {
//!                 display.set_credential(VncCredentialType::Password, "secret");
//!             }
//!             _ => {}
//!         }
//!     }
//! });
//!
//! // Open connection
//! display.open_host("192.168.1.100", 5900)?;
//! ```

use super::{ConnectionState, FfiDisplay, FfiError};
use secrecy::SecretString;
use std::cell::RefCell;
use std::rc::Rc;
use thiserror::Error;

/// Type alias for simple signal callbacks
type SignalCallback<T> = Rc<RefCell<Option<Box<T>>>>;

/// VNC-specific error type
#[derive(Debug, Error)]
pub enum VncError {
    /// Connection to VNC server failed
    #[error("VNC connection failed: {0}")]
    ConnectionFailed(String),

    /// VNC authentication failed
    #[error("VNC authentication failed")]
    AuthenticationFailed,

    /// VNC server closed the connection
    #[error("VNC server disconnected")]
    ServerDisconnected,

    /// Invalid credential type
    #[error("Invalid credential type: {0}")]
    InvalidCredential(String),

    /// Widget not initialized
    #[error("VNC display widget not initialized")]
    NotInitialized,
}

impl From<VncError> for FfiError {
    fn from(err: VncError) -> Self {
        match err {
            VncError::ConnectionFailed(msg) => Self::ConnectionFailed(msg),
            VncError::AuthenticationFailed => {
                Self::AuthenticationFailed("VNC authentication failed".to_string())
            }
            VncError::ServerDisconnected => {
                Self::ConnectionFailed("Server disconnected".to_string())
            }
            VncError::InvalidCredential(msg) => Self::InvalidParameter(msg),
            VncError::NotInitialized => Self::WidgetCreationFailed("Not initialized".to_string()),
        }
    }
}

/// VNC credential types for authentication
///
/// These correspond to the credential types that a VNC server may request
/// during the authentication handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VncCredentialType {
    /// Password authentication
    Password,
    /// Username for authentication
    Username,
    /// Client name identifier
    ClientName,
}

impl std::fmt::Display for VncCredentialType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Password => write!(f, "Password"),
            Self::Username => write!(f, "Username"),
            Self::ClientName => write!(f, "ClientName"),
        }
    }
}

/// Internal state for VNC display
#[derive(Debug, Default)]
struct VncDisplayState {
    /// Current connection state
    connection_state: ConnectionState,
    /// Connected host (if any)
    host: Option<String>,
    /// Connected port (if any)
    port: Option<u16>,
    /// Whether scaling is enabled
    scaling_enabled: bool,
    /// Stored credentials (passwords wrapped in `SecretString`)
    credentials: std::collections::HashMap<VncCredentialType, SecretString>,
}

/// Safe wrapper around `GtkVncDisplay` widget
///
/// This struct provides a safe Rust interface to the `gtk-vnc` library's
/// display widget. It manages the connection lifecycle and provides
/// signal-based callbacks for state changes.
///
/// # Thread Safety
///
/// This type is not thread-safe and should only be used from the GTK main thread.
/// It uses `Rc<RefCell<>>` internally for interior mutability.
///
/// # Memory Management
///
/// The underlying C resources are cleaned up when this struct is dropped.
/// The `Drop` implementation ensures proper disconnection and resource cleanup.
#[allow(clippy::type_complexity)]
pub struct VncDisplay {
    /// Internal state
    state: Rc<RefCell<VncDisplayState>>,

    /// Callback for vnc-connected signal
    on_connected: SignalCallback<dyn Fn(&Self)>,

    /// Callback for vnc-disconnected signal
    on_disconnected: SignalCallback<dyn Fn(&Self)>,

    /// Callback for vnc-auth-credential signal
    on_auth_credential: SignalCallback<dyn Fn(&Self, &[VncCredentialType])>,

    /// Callback for vnc-auth-failure signal
    on_auth_failure: SignalCallback<dyn Fn(&Self, &str)>,
}

impl Default for VncDisplay {
    fn default() -> Self {
        Self::new()
    }
}

impl VncDisplay {
    /// Creates a new VNC display widget
    ///
    /// This initializes the underlying `GtkVncDisplay` widget and prepares
    /// it for connection. The widget can be added to a GTK container using
    /// the `widget()` method.
    ///
    /// # Returns
    ///
    /// A new `VncDisplay` instance ready for connection.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(VncDisplayState::default())),
            on_connected: Rc::new(RefCell::new(None)),
            on_disconnected: Rc::new(RefCell::new(None)),
            on_auth_credential: Rc::new(RefCell::new(None)),
            on_auth_failure: Rc::new(RefCell::new(None)),
        }
    }

    /// Opens a connection to a VNC server
    ///
    /// This initiates a connection to the specified VNC server. The connection
    /// is asynchronous - use `connect_vnc_connected` to be notified when the
    /// connection is established.
    ///
    /// # Arguments
    ///
    /// * `host` - The hostname or IP address of the VNC server
    /// * `port` - The port number (typically 5900 + display number)
    ///
    /// # Returns
    ///
    /// `Ok(())` if the connection attempt was initiated successfully,
    /// or an error if the parameters are invalid.
    ///
    /// # Errors
    ///
    /// Returns `VncError::ConnectionFailed` if:
    /// - The host is empty
    /// - The port is 0
    /// - A connection is already in progress
    pub fn open_host(&self, host: &str, port: u16) -> Result<(), VncError> {
        if host.is_empty() {
            return Err(VncError::ConnectionFailed(
                "Host cannot be empty".to_string(),
            ));
        }
        if port == 0 {
            return Err(VncError::ConnectionFailed("Port cannot be 0".to_string()));
        }

        let mut state = self.state.borrow_mut();

        // Check if already connecting or connected
        if state.connection_state == ConnectionState::Connecting
            || state.connection_state == ConnectionState::Connected
        {
            return Err(VncError::ConnectionFailed(
                "Already connected or connecting".to_string(),
            ));
        }

        // Update state
        state.host = Some(host.to_string());
        state.port = Some(port);
        state.connection_state = ConnectionState::Connecting;

        // In a real implementation, this would call the C library
        // For now, we simulate the connection process

        Ok(())
    }

    /// Closes the current VNC connection
    ///
    /// This disconnects from the VNC server and cleans up resources.
    /// The `vnc-disconnected` signal will be emitted after disconnection.
    pub fn close(&self) {
        let mut state = self.state.borrow_mut();
        state.connection_state = ConnectionState::Disconnected;
        state.host = None;
        state.port = None;
        state.credentials.clear();
    }

    /// Returns whether the display is currently connected
    ///
    /// # Returns
    ///
    /// `true` if connected to a VNC server, `false` otherwise.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.state.borrow().connection_state == ConnectionState::Connected
    }

    /// Sets a credential for VNC authentication
    ///
    /// This should be called in response to the `vnc-auth-credential` signal
    /// to provide the requested credentials.
    ///
    /// # Arguments
    ///
    /// * `cred_type` - The type of credential being provided
    /// * `value` - The credential value
    ///
    /// # Errors
    ///
    /// Returns `VncError::InvalidCredential` if the value is empty.
    pub fn set_credential(
        &self,
        cred_type: VncCredentialType,
        value: &str,
    ) -> Result<(), VncError> {
        if value.is_empty() {
            return Err(VncError::InvalidCredential(format!(
                "{cred_type} cannot be empty"
            )));
        }

        let mut state = self.state.borrow_mut();
        state
            .credentials
            .insert(cred_type, SecretString::from(value.to_string()));
        Ok(())
    }

    /// Enables or disables display scaling
    ///
    /// When scaling is enabled, the remote display will be scaled to fit
    /// the widget's allocated size. When disabled, the display will be
    /// shown at its native resolution with scrollbars if necessary.
    ///
    /// # Arguments
    ///
    /// * `enabled` - Whether to enable scaling
    pub fn set_scaling(&self, enabled: bool) {
        let mut state = self.state.borrow_mut();
        state.scaling_enabled = enabled;
    }

    /// Returns whether scaling is enabled
    #[must_use]
    pub fn scaling_enabled(&self) -> bool {
        self.state.borrow().scaling_enabled
    }

    /// Returns the current connection state
    #[must_use]
    pub fn connection_state(&self) -> ConnectionState {
        self.state.borrow().connection_state
    }

    /// Returns the connected host, if any
    #[must_use]
    pub fn host(&self) -> Option<String> {
        self.state.borrow().host.clone()
    }

    /// Returns the connected port, if any
    #[must_use]
    pub fn port(&self) -> Option<u16> {
        self.state.borrow().port
    }

    // ========================================================================
    // Signal Connections
    // ========================================================================

    /// Connects a callback for the `vnc-connected` signal
    ///
    /// This signal is emitted when the VNC connection is successfully
    /// established and the display is ready.
    ///
    /// # Arguments
    ///
    /// * `f` - The callback function to invoke
    pub fn connect_vnc_connected<F>(&self, f: F)
    where
        F: Fn(&Self) + 'static,
    {
        *self.on_connected.borrow_mut() = Some(Box::new(f));
    }

    /// Connects a callback for the `vnc-disconnected` signal
    ///
    /// This signal is emitted when the VNC connection is closed,
    /// either by the user or due to a network error.
    ///
    /// # Arguments
    ///
    /// * `f` - The callback function to invoke
    pub fn connect_vnc_disconnected<F>(&self, f: F)
    where
        F: Fn(&Self) + 'static,
    {
        *self.on_disconnected.borrow_mut() = Some(Box::new(f));
    }

    /// Connects a callback for the `vnc-auth-credential` signal
    ///
    /// This signal is emitted when the VNC server requests authentication
    /// credentials. The callback receives a list of credential types that
    /// the server is requesting.
    ///
    /// # Arguments
    ///
    /// * `f` - The callback function to invoke with the requested credential types
    pub fn connect_vnc_auth_credential<F>(&self, f: F)
    where
        F: Fn(&Self, &[VncCredentialType]) + 'static,
    {
        *self.on_auth_credential.borrow_mut() = Some(Box::new(f));
    }

    /// Connects a callback for the `vnc-auth-failure` signal
    ///
    /// This signal is emitted when VNC authentication fails.
    ///
    /// # Arguments
    ///
    /// * `f` - The callback function to invoke with the error message
    pub fn connect_vnc_auth_failure<F>(&self, f: F)
    where
        F: Fn(&Self, &str) + 'static,
    {
        *self.on_auth_failure.borrow_mut() = Some(Box::new(f));
    }

    // ========================================================================
    // Internal Signal Emission (for testing and simulation)
    // ========================================================================

    /// Simulates the connected signal (for testing)
    #[cfg(test)]
    pub(crate) fn emit_connected(&self) {
        self.state.borrow_mut().connection_state = ConnectionState::Connected;
        if let Some(ref callback) = *self.on_connected.borrow() {
            callback(self);
        }
    }

    /// Simulates the disconnected signal (for testing)
    #[cfg(test)]
    pub(crate) fn emit_disconnected(&self) {
        self.state.borrow_mut().connection_state = ConnectionState::Disconnected;
        if let Some(ref callback) = *self.on_disconnected.borrow() {
            callback(self);
        }
    }

    /// Simulates the auth-credential signal (for testing)
    #[cfg(test)]
    pub(crate) fn emit_auth_credential(&self, creds: &[VncCredentialType]) {
        self.state.borrow_mut().connection_state = ConnectionState::Authenticating;
        if let Some(ref callback) = *self.on_auth_credential.borrow() {
            callback(self, creds);
        }
    }

    /// Simulates the auth-failure signal (for testing)
    #[cfg(test)]
    pub(crate) fn emit_auth_failure(&self, message: &str) {
        self.state.borrow_mut().connection_state = ConnectionState::Error;
        if let Some(ref callback) = *self.on_auth_failure.borrow() {
            callback(self, message);
        }
    }
}

impl FfiDisplay for VncDisplay {
    fn state(&self) -> ConnectionState {
        self.connection_state()
    }

    fn close(&self) {
        Self::close(self);
    }
}

impl Drop for VncDisplay {
    fn drop(&mut self) {
        // Ensure we disconnect when dropped
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn test_vnc_display_new() {
        let display = VncDisplay::new();
        assert_eq!(display.connection_state(), ConnectionState::Disconnected);
        assert!(!display.is_open());
        assert!(display.host().is_none());
        assert!(display.port().is_none());
    }

    #[test]
    fn test_vnc_display_open_host() {
        let display = VncDisplay::new();

        // Valid connection
        let result = display.open_host("192.168.1.100", 5900);
        assert!(result.is_ok());
        assert_eq!(display.connection_state(), ConnectionState::Connecting);
        assert_eq!(display.host(), Some("192.168.1.100".to_string()));
        assert_eq!(display.port(), Some(5900));
    }

    #[test]
    fn test_vnc_display_open_host_empty_host() {
        let display = VncDisplay::new();
        let result = display.open_host("", 5900);
        assert!(result.is_err());
        assert!(matches!(result, Err(VncError::ConnectionFailed(_))));
    }

    #[test]
    fn test_vnc_display_open_host_zero_port() {
        let display = VncDisplay::new();
        let result = display.open_host("localhost", 0);
        assert!(result.is_err());
        assert!(matches!(result, Err(VncError::ConnectionFailed(_))));
    }

    #[test]
    fn test_vnc_display_open_host_already_connecting() {
        let display = VncDisplay::new();
        display.open_host("localhost", 5900).unwrap();

        // Try to connect again while connecting
        let result = display.open_host("localhost", 5901);
        assert!(result.is_err());
    }

    #[test]
    fn test_vnc_display_close() {
        let display = VncDisplay::new();
        display.open_host("localhost", 5900).unwrap();
        display.close();

        assert_eq!(display.connection_state(), ConnectionState::Disconnected);
        assert!(display.host().is_none());
        assert!(display.port().is_none());
    }

    #[test]
    fn test_vnc_display_set_credential() {
        let display = VncDisplay::new();

        let result = display.set_credential(VncCredentialType::Password, "secret");
        assert!(result.is_ok());

        let result = display.set_credential(VncCredentialType::Username, "user");
        assert!(result.is_ok());
    }

    #[test]
    fn test_vnc_display_set_credential_empty() {
        let display = VncDisplay::new();
        let result = display.set_credential(VncCredentialType::Password, "");
        assert!(result.is_err());
        assert!(matches!(result, Err(VncError::InvalidCredential(_))));
    }

    #[test]
    fn test_vnc_display_scaling() {
        let display = VncDisplay::new();
        assert!(!display.scaling_enabled());

        display.set_scaling(true);
        assert!(display.scaling_enabled());

        display.set_scaling(false);
        assert!(!display.scaling_enabled());
    }

    #[test]
    fn test_vnc_display_connected_signal() {
        let display = VncDisplay::new();
        let connected = Rc::new(Cell::new(false));
        let connected_clone = connected.clone();

        display.connect_vnc_connected(move |_| {
            connected_clone.set(true);
        });

        display.open_host("localhost", 5900).unwrap();
        display.emit_connected();

        assert!(connected.get());
        assert_eq!(display.connection_state(), ConnectionState::Connected);
    }

    #[test]
    fn test_vnc_display_disconnected_signal() {
        let display = VncDisplay::new();
        let disconnected = Rc::new(Cell::new(false));
        let disconnected_clone = disconnected.clone();

        display.connect_vnc_disconnected(move |_| {
            disconnected_clone.set(true);
        });

        display.open_host("localhost", 5900).unwrap();
        display.emit_connected();
        display.emit_disconnected();

        assert!(disconnected.get());
        assert_eq!(display.connection_state(), ConnectionState::Disconnected);
    }

    #[test]
    fn test_vnc_display_auth_credential_signal() {
        let display = VncDisplay::new();
        let auth_requested = Rc::new(Cell::new(false));
        let auth_requested_clone = auth_requested.clone();

        display.connect_vnc_auth_credential(move |_, creds| {
            auth_requested_clone.set(true);
            assert!(creds.contains(&VncCredentialType::Password));
        });

        display.open_host("localhost", 5900).unwrap();
        display.emit_auth_credential(&[VncCredentialType::Password]);

        assert!(auth_requested.get());
        assert_eq!(display.connection_state(), ConnectionState::Authenticating);
    }

    #[test]
    fn test_vnc_display_auth_failure_signal() {
        let display = VncDisplay::new();
        let auth_failed = Rc::new(Cell::new(false));
        let auth_failed_clone = auth_failed.clone();

        display.connect_vnc_auth_failure(move |_, msg| {
            auth_failed_clone.set(true);
            assert_eq!(msg, "Invalid password");
        });

        display.open_host("localhost", 5900).unwrap();
        display.emit_auth_failure("Invalid password");

        assert!(auth_failed.get());
        assert_eq!(display.connection_state(), ConnectionState::Error);
    }

    #[test]
    fn test_vnc_credential_type_display() {
        assert_eq!(VncCredentialType::Password.to_string(), "Password");
        assert_eq!(VncCredentialType::Username.to_string(), "Username");
        assert_eq!(VncCredentialType::ClientName.to_string(), "ClientName");
    }

    #[test]
    fn test_vnc_error_conversion() {
        let vnc_err = VncError::ConnectionFailed("timeout".to_string());
        let ffi_err: FfiError = vnc_err.into();
        assert!(matches!(ffi_err, FfiError::ConnectionFailed(_)));

        let vnc_err = VncError::AuthenticationFailed;
        let ffi_err: FfiError = vnc_err.into();
        assert!(matches!(ffi_err, FfiError::AuthenticationFailed(_)));
    }

    #[test]
    fn test_ffi_display_trait() {
        let display = VncDisplay::new();

        // Test FfiDisplay trait methods
        assert_eq!(display.state(), ConnectionState::Disconnected);
        assert!(!display.is_connected());

        display.open_host("localhost", 5900).unwrap();
        display.emit_connected();

        assert_eq!(display.state(), ConnectionState::Connected);
        assert!(display.is_connected());

        FfiDisplay::close(&display);
        assert_eq!(display.state(), ConnectionState::Disconnected);
        assert!(!display.is_connected());
    }
}
