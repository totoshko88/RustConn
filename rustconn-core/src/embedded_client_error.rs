//! Unified error types for embedded protocol clients (RDP, VNC, SPICE)
//!
//! This module provides a single error enum that covers all embedded client
//! operations, reducing code duplication across protocol implementations.
//!
//! # Backward Compatibility
//!
//! Type aliases are provided for backward compatibility:
//! - `RdpClientError` → `EmbeddedClientError`
//! - `VncClientError` → `EmbeddedClientError`
//! - `SpiceClientError` → `EmbeddedClientError`

use thiserror::Error;

/// Generic error type for embedded protocol clients.
///
/// This enum consolidates error variants from RDP, VNC, and SPICE clients
/// into a single type. Protocol-specific variants are included for cases
/// that only apply to certain protocols.
#[derive(Debug, Error, Clone)]
pub enum EmbeddedClientError {
    // === Common variants (all protocols) ===
    /// Connection to server failed
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Authentication failed
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    /// Protocol error during communication
    #[error("Protocol error: {0}")]
    ProtocolError(String),

    /// IO error during network operations
    #[error("IO error: {0}")]
    IoError(String),

    /// Client is not connected
    #[error("Not connected")]
    NotConnected,

    /// Client is already connected
    #[error("Already connected")]
    AlreadyConnected,

    /// Invalid configuration
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    /// Channel communication error
    #[error("Channel error: {0}")]
    ChannelError(String),

    /// Timeout waiting for operation
    #[error("Operation timed out")]
    Timeout,

    /// Server disconnected
    #[error("Server disconnected: {0}")]
    ServerDisconnected(String),

    /// Unsupported feature
    #[error("Unsupported feature: {0}")]
    Unsupported(String),

    /// Unsupported VNC security type — auto-fallback to external viewer recommended
    #[error("Unsupported security type: {0}")]
    UnsupportedSecurityType(String),

    // === TLS (RDP, SPICE) ===
    /// TLS/SSL error
    #[error("TLS error: {0}")]
    TlsError(String),

    // === SPICE-specific ===
    /// USB redirection error
    #[error("USB redirection error: {0}")]
    UsbRedirectionError(String),

    /// Shared folder error
    #[error("Shared folder error: {0}")]
    SharedFolderError(String),

    /// Native SPICE client not available, fallback required
    #[error("Native SPICE client not available, falling back to virt-viewer")]
    NativeClientNotAvailable,
}

impl From<std::io::Error> for EmbeddedClientError {
    fn from(err: std::io::Error) -> Self {
        Self::IoError(err.to_string())
    }
}

// Type aliases for backward compatibility
/// RDP client error type (alias for `EmbeddedClientError`)
pub type RdpClientError = EmbeddedClientError;

/// VNC client error type (alias for `EmbeddedClientError`)
pub type VncClientError = EmbeddedClientError;

/// SPICE client error type (alias for `EmbeddedClientError`)
pub type SpiceClientError = EmbeddedClientError;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = EmbeddedClientError::ConnectionFailed("timeout".to_string());
        assert_eq!(err.to_string(), "Connection failed: timeout");
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: EmbeddedClientError = io_err.into();
        assert!(matches!(err, EmbeddedClientError::IoError(_)));
    }

    #[test]
    fn test_type_aliases() {
        // Verify type aliases work correctly
        let rdp_err: RdpClientError = EmbeddedClientError::TlsError("cert invalid".to_string());
        let vnc_err: VncClientError = EmbeddedClientError::Timeout;
        let spice_err: SpiceClientError =
            EmbeddedClientError::UsbRedirectionError("device busy".to_string());

        assert!(matches!(rdp_err, EmbeddedClientError::TlsError(_)));
        assert!(matches!(vnc_err, EmbeddedClientError::Timeout));
        assert!(matches!(
            spice_err,
            EmbeddedClientError::UsbRedirectionError(_)
        ));
    }
}
