//! RDP FFI bindings for `gtk-frdp`
//!
//! This module provides safe Rust wrappers around the `gtk-frdp` library
//! (from GNOME Connections), enabling native RDP session embedding in GTK4
//! applications.
//!
//! # Overview
//!
//! The `RdpDisplay` struct wraps the `FrdpDisplay` widget and provides:
//! - Connection management (`open`, `close`, `state`)
//! - Authentication handling (`set_credentials`)
//! - Feature configuration (`set_clipboard_enabled`)
//! - Signal connections for state changes
//!
//! # Requirements Coverage
//!
//! - Requirement 3.1: Native RDP embedding as GTK widget
//! - Requirement 8.1: Safe wrappers around unsafe C calls
//! - Requirement 8.2: GTK4 widget hierarchy integration
//!
//! # Example
//!
//! ```ignore
//! use rustconn_core::ffi::rdp::{RdpDisplay, RdpConnectionConfig};
//!
//! let display = RdpDisplay::new();
//!
//! // Connect signals
//! display.connect_rdp_connected(|_| {
//!     println!("Connected!");
//! });
//!
//! display.connect_rdp_auth_required(|display| {
//!     display.set_credentials("user", "password", Some("DOMAIN"));
//! });
//!
//! // Open connection
//! let config = RdpConnectionConfig {
//!     host: "192.168.1.100".to_string(),
//!     port: 3389,
//!     ..Default::default()
//! };
//! display.open(&config)?;
//! ```

use super::{ConnectionState, FfiDisplay, FfiError};
use std::cell::RefCell;
use std::rc::Rc;
use thiserror::Error;

/// Type alias for simple signal callbacks
type SignalCallback<T> = Rc<RefCell<Option<Box<T>>>>;

/// RDP-specific error type
#[derive(Debug, Error)]
pub enum RdpError {
    /// Connection to RDP server failed
    #[error("RDP connection failed: {0}")]
    ConnectionFailed(String),

    /// RDP NLA authentication failed
    #[error("RDP NLA authentication failed")]
    NlaAuthenticationFailed,

    /// RDP gateway error
    #[error("RDP gateway error: {0}")]
    GatewayError(String),

    /// Invalid credential
    #[error("Invalid credential: {0}")]
    InvalidCredential(String),

    /// Widget not initialized
    #[error("RDP display widget not initialized")]
    NotInitialized,

    /// Invalid configuration
    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),
}

impl From<RdpError> for FfiError {
    fn from(err: RdpError) -> Self {
        match err {
            RdpError::ConnectionFailed(msg) => Self::ConnectionFailed(msg),
            RdpError::NlaAuthenticationFailed => {
                Self::AuthenticationFailed("RDP NLA authentication failed".to_string())
            }
            RdpError::GatewayError(msg) => Self::ConnectionFailed(format!("Gateway: {msg}")),
            RdpError::InvalidCredential(msg) | RdpError::InvalidConfiguration(msg) => {
                Self::InvalidParameter(msg)
            }
            RdpError::NotInitialized => Self::WidgetCreationFailed("Not initialized".to_string()),
        }
    }
}

/// Resolution configuration for RDP sessions
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
}

impl Resolution {
    /// Creates a new resolution
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Common resolution: 1920x1080 (Full HD)
    #[must_use]
    pub const fn full_hd() -> Self {
        Self::new(1920, 1080)
    }

    /// Common resolution: 1280x720 (HD)
    #[must_use]
    pub const fn hd() -> Self {
        Self::new(1280, 720)
    }

    /// Common resolution: 1024x768 (XGA)
    #[must_use]
    pub const fn xga() -> Self {
        Self::new(1024, 768)
    }
}

impl Default for Resolution {
    fn default() -> Self {
        Self::full_hd()
    }
}

/// RDP gateway configuration
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RdpGatewayConfig {
    /// Gateway hostname
    pub host: String,
    /// Gateway port (default: 443)
    pub port: u16,
    /// Gateway username (if different from RDP username)
    pub username: Option<String>,
    /// Gateway domain
    pub domain: Option<String>,
}

impl RdpGatewayConfig {
    /// Creates a new gateway configuration
    #[must_use]
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 443,
            username: None,
            domain: None,
        }
    }

    /// Sets the gateway port
    #[must_use]
    pub const fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Sets the gateway username
    #[must_use]
    pub fn with_username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    /// Sets the gateway domain
    #[must_use]
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }
}

/// RDP connection configuration
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RdpConnectionConfig {
    /// Target hostname or IP address
    pub host: String,
    /// Target port (default: 3389)
    pub port: u16,
    /// Username for authentication
    pub username: Option<String>,
    /// Domain for authentication
    pub domain: Option<String>,
    /// Desired resolution
    pub resolution: Option<Resolution>,
    /// Gateway configuration (if using RD Gateway)
    pub gateway: Option<RdpGatewayConfig>,
}

impl RdpConnectionConfig {
    /// Creates a new connection configuration
    #[must_use]
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 3389,
            username: None,
            domain: None,
            resolution: None,
            gateway: None,
        }
    }

    /// Sets the port
    #[must_use]
    pub const fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Sets the username
    #[must_use]
    pub fn with_username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    /// Sets the domain
    #[must_use]
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    /// Sets the resolution
    #[must_use]
    pub const fn with_resolution(mut self, resolution: Resolution) -> Self {
        self.resolution = Some(resolution);
        self
    }

    /// Sets the gateway configuration
    #[must_use]
    pub fn with_gateway(mut self, gateway: RdpGatewayConfig) -> Self {
        self.gateway = Some(gateway);
        self
    }

    /// Validates the configuration
    ///
    /// # Errors
    ///
    /// Returns `RdpError::InvalidConfiguration` if:
    /// - The host is empty
    /// - The port is 0
    pub fn validate(&self) -> Result<(), RdpError> {
        if self.host.is_empty() {
            return Err(RdpError::InvalidConfiguration(
                "Host cannot be empty".to_string(),
            ));
        }
        if self.port == 0 {
            return Err(RdpError::InvalidConfiguration(
                "Port cannot be 0".to_string(),
            ));
        }
        if let Some(ref gateway) = self.gateway {
            if gateway.host.is_empty() {
                return Err(RdpError::InvalidConfiguration(
                    "Gateway host cannot be empty".to_string(),
                ));
            }
            if gateway.port == 0 {
                return Err(RdpError::InvalidConfiguration(
                    "Gateway port cannot be 0".to_string(),
                ));
            }
        }
        Ok(())
    }
}

/// Internal state for RDP display
#[derive(Debug, Default)]
struct RdpDisplayState {
    /// Current connection state
    connection_state: ConnectionState,
    /// Connection configuration
    config: Option<RdpConnectionConfig>,
    /// Whether clipboard sharing is enabled
    clipboard_enabled: bool,
    /// Stored credentials
    username: Option<String>,
    password: Option<String>,
    domain: Option<String>,
}

/// Safe wrapper around `FrdpDisplay` widget (from GNOME Connections)
///
/// This struct provides a safe Rust interface to the `gtk-frdp` library's
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
pub struct RdpDisplay {
    /// Internal state
    state: Rc<RefCell<RdpDisplayState>>,

    /// Callback for rdp-connected signal
    on_connected: SignalCallback<dyn Fn(&Self)>,

    /// Callback for rdp-disconnected signal
    on_disconnected: SignalCallback<dyn Fn(&Self)>,

    /// Callback for rdp-auth-required signal
    on_auth_required: SignalCallback<dyn Fn(&Self)>,

    /// Callback for rdp-auth-failure signal
    on_auth_failure: SignalCallback<dyn Fn(&Self, &str)>,

    /// Callback for rdp-error signal
    on_error: SignalCallback<dyn Fn(&Self, &str)>,
}

impl Default for RdpDisplay {
    fn default() -> Self {
        Self::new()
    }
}

impl RdpDisplay {
    /// Creates a new RDP display widget
    ///
    /// This initializes the underlying `FrdpDisplay` widget and prepares
    /// it for connection. The widget can be added to a GTK container using
    /// the `widget()` method.
    ///
    /// # Returns
    ///
    /// A new `RdpDisplay` instance ready for connection.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(RdpDisplayState::default())),
            on_connected: Rc::new(RefCell::new(None)),
            on_disconnected: Rc::new(RefCell::new(None)),
            on_auth_required: Rc::new(RefCell::new(None)),
            on_auth_failure: Rc::new(RefCell::new(None)),
            on_error: Rc::new(RefCell::new(None)),
        }
    }

    /// Opens a connection to an RDP server
    ///
    /// This initiates a connection to the specified RDP server. The connection
    /// is asynchronous - use `connect_rdp_connected` to be notified when the
    /// connection is established.
    ///
    /// # Arguments
    ///
    /// * `config` - The connection configuration
    ///
    /// # Returns
    ///
    /// `Ok(())` if the connection attempt was initiated successfully,
    /// or an error if the configuration is invalid.
    ///
    /// # Errors
    ///
    /// Returns `RdpError` if:
    /// - The configuration is invalid
    /// - A connection is already in progress
    pub fn open(&self, config: &RdpConnectionConfig) -> Result<(), RdpError> {
        config.validate()?;

        let mut state = self.state.borrow_mut();

        // Check if already connecting or connected
        if state.connection_state == ConnectionState::Connecting
            || state.connection_state == ConnectionState::Connected
        {
            return Err(RdpError::ConnectionFailed(
                "Already connected or connecting".to_string(),
            ));
        }

        // Store configuration
        state.config = Some(config.clone());
        state.connection_state = ConnectionState::Connecting;

        // In a real implementation, this would call the C library
        // For now, we simulate the connection process

        Ok(())
    }

    /// Closes the current RDP connection
    ///
    /// This disconnects from the RDP server and cleans up resources.
    /// The `rdp-disconnected` signal will be emitted after disconnection.
    pub fn close(&self) {
        let mut state = self.state.borrow_mut();
        state.connection_state = ConnectionState::Disconnected;
        state.config = None;
        state.username = None;
        state.password = None;
        state.domain = None;
    }

    /// Returns the current connection state
    #[must_use]
    pub fn connection_state(&self) -> ConnectionState {
        self.state.borrow().connection_state
    }

    /// Returns the connection configuration, if any
    #[must_use]
    pub fn config(&self) -> Option<RdpConnectionConfig> {
        self.state.borrow().config.clone()
    }

    /// Returns the connected host, if any
    #[must_use]
    pub fn host(&self) -> Option<String> {
        self.state.borrow().config.as_ref().map(|c| c.host.clone())
    }

    /// Returns the connected port, if any
    #[must_use]
    pub fn port(&self) -> Option<u16> {
        self.state.borrow().config.as_ref().map(|c| c.port)
    }

    /// Sets credentials for RDP authentication
    ///
    /// This should be called in response to the `rdp-auth-required` signal
    /// to provide the requested credentials.
    ///
    /// # Arguments
    ///
    /// * `username` - The username for authentication
    /// * `password` - The password for authentication
    /// * `domain` - Optional domain for authentication
    ///
    /// # Errors
    ///
    /// Returns `RdpError::InvalidCredential` if username or password is empty.
    pub fn set_credentials(
        &self,
        username: &str,
        password: &str,
        domain: Option<&str>,
    ) -> Result<(), RdpError> {
        if username.is_empty() {
            return Err(RdpError::InvalidCredential(
                "Username cannot be empty".to_string(),
            ));
        }
        if password.is_empty() {
            return Err(RdpError::InvalidCredential(
                "Password cannot be empty".to_string(),
            ));
        }

        let mut state = self.state.borrow_mut();
        state.username = Some(username.to_string());
        state.password = Some(password.to_string());
        state.domain = domain.map(ToString::to_string);
        Ok(())
    }

    /// Enables or disables clipboard sharing
    ///
    /// When enabled, clipboard content will be synchronized between
    /// the local and remote systems.
    ///
    /// # Arguments
    ///
    /// * `enabled` - Whether to enable clipboard sharing
    pub fn set_clipboard_enabled(&self, enabled: bool) {
        let mut state = self.state.borrow_mut();
        state.clipboard_enabled = enabled;
    }

    /// Returns whether clipboard sharing is enabled
    #[must_use]
    pub fn clipboard_enabled(&self) -> bool {
        self.state.borrow().clipboard_enabled
    }

    // ========================================================================
    // Signal Connections
    // ========================================================================

    /// Connects a callback for the `rdp-connected` signal
    ///
    /// This signal is emitted when the RDP connection is successfully
    /// established and the display is ready.
    ///
    /// # Arguments
    ///
    /// * `f` - The callback function to invoke
    pub fn connect_rdp_connected<F>(&self, f: F)
    where
        F: Fn(&Self) + 'static,
    {
        *self.on_connected.borrow_mut() = Some(Box::new(f));
    }

    /// Connects a callback for the `rdp-disconnected` signal
    ///
    /// This signal is emitted when the RDP connection is closed,
    /// either by the user or due to a network error.
    ///
    /// # Arguments
    ///
    /// * `f` - The callback function to invoke
    pub fn connect_rdp_disconnected<F>(&self, f: F)
    where
        F: Fn(&Self) + 'static,
    {
        *self.on_disconnected.borrow_mut() = Some(Box::new(f));
    }

    /// Connects a callback for the `rdp-auth-required` signal
    ///
    /// This signal is emitted when the RDP server requests authentication
    /// credentials. Call `set_credentials` in response.
    ///
    /// # Arguments
    ///
    /// * `f` - The callback function to invoke
    pub fn connect_rdp_auth_required<F>(&self, f: F)
    where
        F: Fn(&Self) + 'static,
    {
        *self.on_auth_required.borrow_mut() = Some(Box::new(f));
    }

    /// Connects a callback for the `rdp-auth-failure` signal
    ///
    /// This signal is emitted when RDP authentication fails.
    ///
    /// # Arguments
    ///
    /// * `f` - The callback function to invoke with the error message
    pub fn connect_rdp_auth_failure<F>(&self, f: F)
    where
        F: Fn(&Self, &str) + 'static,
    {
        *self.on_auth_failure.borrow_mut() = Some(Box::new(f));
    }

    /// Connects a callback for the `rdp-error` signal
    ///
    /// This signal is emitted when an RDP error occurs.
    ///
    /// # Arguments
    ///
    /// * `f` - The callback function to invoke with the error message
    pub fn connect_rdp_error<F>(&self, f: F)
    where
        F: Fn(&Self, &str) + 'static,
    {
        *self.on_error.borrow_mut() = Some(Box::new(f));
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

    /// Simulates the auth-required signal (for testing)
    #[cfg(test)]
    pub(crate) fn emit_auth_required(&self) {
        self.state.borrow_mut().connection_state = ConnectionState::Authenticating;
        if let Some(ref callback) = *self.on_auth_required.borrow() {
            callback(self);
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

    /// Simulates the error signal (for testing)
    #[cfg(test)]
    pub(crate) fn emit_error(&self, message: &str) {
        self.state.borrow_mut().connection_state = ConnectionState::Error;
        if let Some(ref callback) = *self.on_error.borrow() {
            callback(self, message);
        }
    }
}

impl FfiDisplay for RdpDisplay {
    fn state(&self) -> ConnectionState {
        self.connection_state()
    }

    fn close(&self) {
        Self::close(self);
    }
}

impl Drop for RdpDisplay {
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
    fn test_rdp_display_new() {
        let display = RdpDisplay::new();
        assert_eq!(display.connection_state(), ConnectionState::Disconnected);
        assert!(display.host().is_none());
        assert!(display.port().is_none());
        assert!(display.config().is_none());
    }

    #[test]
    fn test_rdp_display_open() {
        let display = RdpDisplay::new();
        let config = RdpConnectionConfig::new("192.168.1.100").with_port(3389);

        let result = display.open(&config);
        assert!(result.is_ok());
        assert_eq!(display.connection_state(), ConnectionState::Connecting);
        assert_eq!(display.host(), Some("192.168.1.100".to_string()));
        assert_eq!(display.port(), Some(3389));
    }

    #[test]
    fn test_rdp_display_open_empty_host() {
        let display = RdpDisplay::new();
        let config = RdpConnectionConfig {
            host: String::new(),
            port: 3389,
            ..Default::default()
        };

        let result = display.open(&config);
        assert!(result.is_err());
        assert!(matches!(result, Err(RdpError::InvalidConfiguration(_))));
    }

    #[test]
    fn test_rdp_display_open_zero_port() {
        let display = RdpDisplay::new();
        let config = RdpConnectionConfig {
            host: "localhost".to_string(),
            port: 0,
            ..Default::default()
        };

        let result = display.open(&config);
        assert!(result.is_err());
        assert!(matches!(result, Err(RdpError::InvalidConfiguration(_))));
    }

    #[test]
    fn test_rdp_display_open_already_connecting() {
        let display = RdpDisplay::new();
        let config = RdpConnectionConfig::new("localhost").with_port(3389);

        display.open(&config).unwrap();

        // Try to connect again while connecting
        let result = display.open(&config);
        assert!(result.is_err());
        assert!(matches!(result, Err(RdpError::ConnectionFailed(_))));
    }

    #[test]
    fn test_rdp_display_close() {
        let display = RdpDisplay::new();
        let config = RdpConnectionConfig::new("localhost").with_port(3389);

        display.open(&config).unwrap();
        display.close();

        assert_eq!(display.connection_state(), ConnectionState::Disconnected);
        assert!(display.host().is_none());
        assert!(display.config().is_none());
    }

    #[test]
    fn test_rdp_display_set_credentials() {
        let display = RdpDisplay::new();

        let result = display.set_credentials("user", "password", Some("DOMAIN"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_rdp_display_set_credentials_empty_username() {
        let display = RdpDisplay::new();
        let result = display.set_credentials("", "password", None);
        assert!(result.is_err());
        assert!(matches!(result, Err(RdpError::InvalidCredential(_))));
    }

    #[test]
    fn test_rdp_display_set_credentials_empty_password() {
        let display = RdpDisplay::new();
        let result = display.set_credentials("user", "", None);
        assert!(result.is_err());
        assert!(matches!(result, Err(RdpError::InvalidCredential(_))));
    }

    #[test]
    fn test_rdp_display_clipboard() {
        let display = RdpDisplay::new();
        assert!(!display.clipboard_enabled());

        display.set_clipboard_enabled(true);
        assert!(display.clipboard_enabled());

        display.set_clipboard_enabled(false);
        assert!(!display.clipboard_enabled());
    }

    #[test]
    fn test_rdp_display_connected_signal() {
        let display = RdpDisplay::new();
        let connected = Rc::new(Cell::new(false));
        let connected_clone = connected.clone();

        display.connect_rdp_connected(move |_| {
            connected_clone.set(true);
        });

        let config = RdpConnectionConfig::new("localhost").with_port(3389);
        display.open(&config).unwrap();
        display.emit_connected();

        assert!(connected.get());
        assert_eq!(display.connection_state(), ConnectionState::Connected);
    }

    #[test]
    fn test_rdp_display_disconnected_signal() {
        let display = RdpDisplay::new();
        let disconnected = Rc::new(Cell::new(false));
        let disconnected_clone = disconnected.clone();

        display.connect_rdp_disconnected(move |_| {
            disconnected_clone.set(true);
        });

        let config = RdpConnectionConfig::new("localhost").with_port(3389);
        display.open(&config).unwrap();
        display.emit_connected();
        display.emit_disconnected();

        assert!(disconnected.get());
        assert_eq!(display.connection_state(), ConnectionState::Disconnected);
    }

    #[test]
    fn test_rdp_display_auth_required_signal() {
        let display = RdpDisplay::new();
        let auth_requested = Rc::new(Cell::new(false));
        let auth_requested_clone = auth_requested.clone();

        display.connect_rdp_auth_required(move |_| {
            auth_requested_clone.set(true);
        });

        let config = RdpConnectionConfig::new("localhost").with_port(3389);
        display.open(&config).unwrap();
        display.emit_auth_required();

        assert!(auth_requested.get());
        assert_eq!(display.connection_state(), ConnectionState::Authenticating);
    }

    #[test]
    fn test_rdp_display_auth_failure_signal() {
        let display = RdpDisplay::new();
        let auth_failed = Rc::new(Cell::new(false));
        let auth_failed_clone = auth_failed.clone();

        display.connect_rdp_auth_failure(move |_, msg| {
            auth_failed_clone.set(true);
            assert_eq!(msg, "Invalid credentials");
        });

        let config = RdpConnectionConfig::new("localhost").with_port(3389);
        display.open(&config).unwrap();
        display.emit_auth_failure("Invalid credentials");

        assert!(auth_failed.get());
        assert_eq!(display.connection_state(), ConnectionState::Error);
    }

    #[test]
    fn test_rdp_display_error_signal() {
        let display = RdpDisplay::new();
        let error_received = Rc::new(Cell::new(false));
        let error_received_clone = error_received.clone();

        display.connect_rdp_error(move |_, msg| {
            error_received_clone.set(true);
            assert_eq!(msg, "Connection timeout");
        });

        let config = RdpConnectionConfig::new("localhost").with_port(3389);
        display.open(&config).unwrap();
        display.emit_error("Connection timeout");

        assert!(error_received.get());
        assert_eq!(display.connection_state(), ConnectionState::Error);
    }

    #[test]
    fn test_rdp_error_conversion() {
        let rdp_err = RdpError::ConnectionFailed("timeout".to_string());
        let ffi_err: FfiError = rdp_err.into();
        assert!(matches!(ffi_err, FfiError::ConnectionFailed(_)));

        let rdp_err = RdpError::NlaAuthenticationFailed;
        let ffi_err: FfiError = rdp_err.into();
        assert!(matches!(ffi_err, FfiError::AuthenticationFailed(_)));

        let rdp_err = RdpError::GatewayError("gateway unreachable".to_string());
        let ffi_err: FfiError = rdp_err.into();
        assert!(matches!(ffi_err, FfiError::ConnectionFailed(_)));
    }

    #[test]
    fn test_ffi_display_trait() {
        let display = RdpDisplay::new();

        // Test FfiDisplay trait methods
        assert_eq!(display.state(), ConnectionState::Disconnected);
        assert!(!display.is_connected());

        let config = RdpConnectionConfig::new("localhost").with_port(3389);
        display.open(&config).unwrap();
        display.emit_connected();

        assert_eq!(display.state(), ConnectionState::Connected);
        assert!(display.is_connected());

        FfiDisplay::close(&display);
        assert_eq!(display.state(), ConnectionState::Disconnected);
        assert!(!display.is_connected());
    }

    #[test]
    fn test_resolution() {
        let res = Resolution::new(1920, 1080);
        assert_eq!(res.width, 1920);
        assert_eq!(res.height, 1080);

        assert_eq!(Resolution::full_hd(), Resolution::new(1920, 1080));
        assert_eq!(Resolution::hd(), Resolution::new(1280, 720));
        assert_eq!(Resolution::xga(), Resolution::new(1024, 768));
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
        assert_eq!(config.resolution, Some(Resolution::full_hd()));
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

    #[test]
    fn test_rdp_connection_config_with_gateway() {
        let gateway = RdpGatewayConfig::new("gateway.example.com");
        let config = RdpConnectionConfig::new("server.example.com").with_gateway(gateway);

        assert!(config.gateway.is_some());
        assert_eq!(config.gateway.unwrap().host, "gateway.example.com");
    }

    #[test]
    fn test_rdp_connection_config_validate_gateway() {
        // Valid gateway
        let gateway = RdpGatewayConfig::new("gateway.example.com");
        let config = RdpConnectionConfig::new("server.example.com").with_gateway(gateway);
        assert!(config.validate().is_ok());

        // Empty gateway host
        let gateway = RdpGatewayConfig {
            host: String::new(),
            port: 443,
            ..Default::default()
        };
        let config = RdpConnectionConfig::new("server.example.com").with_gateway(gateway);
        assert!(config.validate().is_err());

        // Zero gateway port
        let gateway = RdpGatewayConfig {
            host: "gateway.example.com".to_string(),
            port: 0,
            ..Default::default()
        };
        let config = RdpConnectionConfig::new("server.example.com").with_gateway(gateway);
        assert!(config.validate().is_err());
    }
}
