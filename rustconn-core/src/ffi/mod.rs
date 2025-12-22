//! FFI bindings for native protocol embedding
//!
//! This module provides safe Rust wrappers around C libraries for native
//! protocol embedding in GTK4. The bindings follow these principles:
//!
//! - Safe wrappers around unsafe C calls (Requirement 8.1)
//! - Proper integration with GTK4-rs widget hierarchy (Requirement 8.2)
//! - Memory cleanup through Drop implementations (Requirement 8.3)
//! - Correct handling of Rust closure lifetimes for callbacks (Requirement 8.4)
//!
//! # Supported Libraries
//!
//! - `gtk-vnc`: VNC client widget for GTK
//! - `gtk-frdp`: RDP client widget (GNOME Connections)
//! - `spice-gtk`: SPICE client widget
//!
//! # Architecture
//!
//! Each protocol has its own submodule with:
//! - A safe wrapper struct (e.g., `VncDisplay`)
//! - Connection/disconnection methods
//! - Signal connection helpers
//! - Widget accessor for GTK integration

pub mod rdp;
pub mod spice;
pub mod vnc;

pub use rdp::{RdpConnectionConfig, RdpDisplay, RdpError, RdpGatewayConfig, Resolution};
pub use spice::{
    SpiceChannelEvent, SpiceConnectionConfig, SpiceDisplay, SpiceError, SpiceSharedFolder,
    SpiceTlsConfig,
};
pub use vnc::{VncCredentialType, VncDisplay, VncError};

use thiserror::Error;

/// Common error type for FFI operations
#[derive(Debug, Error)]
pub enum FfiError {
    /// Failed to initialize the FFI library
    #[error("FFI initialization failed: {0}")]
    InitializationFailed(String),

    /// Failed to create a widget
    #[error("Widget creation failed: {0}")]
    WidgetCreationFailed(String),

    /// Connection operation failed
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Authentication failed
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    /// The underlying C library returned an error
    #[error("Library error: {0}")]
    LibraryError(String),

    /// Invalid parameter passed to FFI function
    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    /// Resource cleanup failed
    #[error("Cleanup failed: {0}")]
    CleanupFailed(String),
}

/// Result type for FFI operations
pub type FfiResult<T> = Result<T, FfiError>;

/// Connection state for FFI-wrapped displays
///
/// This enum represents the lifecycle states of a remote display connection.
/// It is used by all protocol implementations (VNC, RDP, SPICE).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionState {
    /// Not connected to any remote host
    #[default]
    Disconnected,

    /// Connection attempt in progress
    Connecting,

    /// Waiting for authentication credentials
    Authenticating,

    /// Successfully connected and displaying remote content
    Connected,

    /// Connection failed with an error
    Error,
}

impl std::fmt::Display for ConnectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => write!(f, "Disconnected"),
            Self::Connecting => write!(f, "Connecting"),
            Self::Authenticating => write!(f, "Authenticating"),
            Self::Connected => write!(f, "Connected"),
            Self::Error => write!(f, "Error"),
        }
    }
}

/// Trait for FFI display widgets
///
/// This trait defines the common interface for all FFI-wrapped display widgets.
/// It ensures consistent behavior across VNC, RDP, and SPICE implementations.
pub trait FfiDisplay {
    /// Returns the current connection state
    fn state(&self) -> ConnectionState;

    /// Returns whether the display is currently connected
    fn is_connected(&self) -> bool {
        self.state() == ConnectionState::Connected
    }

    /// Closes the current connection
    fn close(&self);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_state_display() {
        assert_eq!(ConnectionState::Disconnected.to_string(), "Disconnected");
        assert_eq!(ConnectionState::Connecting.to_string(), "Connecting");
        assert_eq!(
            ConnectionState::Authenticating.to_string(),
            "Authenticating"
        );
        assert_eq!(ConnectionState::Connected.to_string(), "Connected");
        assert_eq!(ConnectionState::Error.to_string(), "Error");
    }

    #[test]
    fn test_connection_state_default() {
        let state: ConnectionState = ConnectionState::default();
        assert_eq!(state, ConnectionState::Disconnected);
    }

    #[test]
    fn test_ffi_error_display() {
        let err = FfiError::ConnectionFailed("timeout".to_string());
        assert_eq!(err.to_string(), "Connection failed: timeout");

        let err = FfiError::AuthenticationFailed("invalid password".to_string());
        assert_eq!(err.to_string(), "Authentication failed: invalid password");
    }
}
