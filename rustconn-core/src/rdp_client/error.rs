//! RDP client error types

use thiserror::Error;

/// Error type for RDP client operations
#[derive(Debug, Error, Clone)]
pub enum RdpClientError {
    /// Connection to RDP server failed
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Authentication failed
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    /// Protocol error during RDP communication
    #[error("Protocol error: {0}")]
    ProtocolError(String),

    /// IO error during network operations
    #[error("IO error: {0}")]
    IoError(String),

    /// TLS/SSL error
    #[error("TLS error: {0}")]
    TlsError(String),

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
}

impl From<std::io::Error> for RdpClientError {
    fn from(err: std::io::Error) -> Self {
        Self::IoError(err.to_string())
    }
}
