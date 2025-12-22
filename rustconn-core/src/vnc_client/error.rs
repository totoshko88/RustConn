//! VNC client error types

use thiserror::Error;

/// Error type for VNC client operations
#[derive(Debug, Error, Clone)]
pub enum VncClientError {
    /// Connection to VNC server failed
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Authentication failed
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    /// Protocol error during VNC communication
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
}

impl From<std::io::Error> for VncClientError {
    fn from(err: std::io::Error) -> Self {
        Self::IoError(err.to_string())
    }
}
