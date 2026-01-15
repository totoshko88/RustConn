//! Error types for `RustConn`
//!
//! This module defines all error types used throughout the `RustConn` application,
//! providing descriptive error messages for configuration, protocol, import,
//! secret storage, and session management operations.

use std::path::PathBuf;
use thiserror::Error;

/// Top-level error type for `RustConn` operations
#[derive(Debug, Error)]
pub enum RustConnError {
    /// Configuration-related errors
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    /// Protocol-related errors (SSH, RDP, VNC)
    #[error("Protocol error: {0}")]
    Protocol(#[from] ProtocolError),

    /// Secret storage errors (`KeePassXC`, libsecret)
    #[error("Secret storage error: {0}")]
    Secret(#[from] SecretError),

    /// Import errors (Asbru-CM, Remmina, SSH config, Ansible)
    #[error("Import error: {0}")]
    Import(#[from] ImportError),

    /// Session management errors
    #[error("Session error: {0}")]
    Session(#[from] SessionError),

    /// I/O errors
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Errors related to configuration file operations
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Failed to parse configuration file
    #[error("Failed to parse configuration: {0}")]
    Parse(String),

    /// Invalid configuration value
    #[error("Invalid configuration value for {field}: {reason}")]
    Validation {
        /// The field that failed validation
        field: String,
        /// The reason for validation failure
        reason: String,
    },

    /// Configuration file not found
    #[error("Configuration file not found: {0}")]
    NotFound(PathBuf),

    /// Failed to write configuration file
    #[error("Failed to write configuration: {0}")]
    Write(String),

    /// Failed to serialize configuration
    #[error("Failed to serialize configuration: {0}")]
    Serialize(String),

    /// Failed to deserialize configuration
    #[error("Failed to deserialize configuration: {0}")]
    Deserialize(String),
}

/// Errors related to protocol operations (SSH, RDP, VNC)
#[derive(Debug, Error)]
pub enum ProtocolError {
    /// Connection to remote host failed
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Authentication with remote host failed
    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    /// Required client binary not found
    #[error("Client not found: {0}")]
    ClientNotFound(PathBuf),

    /// Invalid protocol configuration
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    /// Command execution failed
    #[error("Command execution failed: {0}")]
    CommandFailed(String),

    /// Unsupported protocol feature
    #[error("Unsupported feature: {0}")]
    UnsupportedFeature(String),
}

/// Errors related to secret storage operations
#[derive(Debug, Error)]
pub enum SecretError {
    /// Failed to connect to secret backend
    #[error("Failed to connect to secret backend: {0}")]
    ConnectionFailed(String),

    /// Failed to store credentials
    #[error("Failed to store credentials: {0}")]
    StoreFailed(String),

    /// Failed to retrieve credentials
    #[error("Failed to retrieve credentials: {0}")]
    RetrieveFailed(String),

    /// Failed to delete credentials
    #[error("Failed to delete credentials: {0}")]
    DeleteFailed(String),

    /// Secret backend not available
    #[error("Secret backend not available: {0}")]
    BackendUnavailable(String),

    /// KeePassXC-specific error
    #[error("KeePassXC error: {0}")]
    KeePassXC(String),

    /// libsecret-specific error
    #[error("libsecret error: {0}")]
    LibSecret(String),

    /// Bitwarden-specific error
    #[error("Bitwarden error: {0}")]
    Bitwarden(String),
}

/// Errors related to configuration import operations
#[derive(Debug, Error)]
pub enum ImportError {
    /// Failed to parse import source
    #[error("Failed to parse {source_name}: {reason}")]
    ParseError {
        /// The import source (e.g., "SSH config", "Asbru-CM")
        source_name: String,
        /// The reason for parse failure
        reason: String,
    },

    /// Unsupported import format
    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),

    /// Import source file not found
    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    /// Invalid entry in import source
    #[error("Invalid entry in {source_name}: {reason}")]
    InvalidEntry {
        /// The import source
        source_name: String,
        /// The reason the entry is invalid
        reason: String,
    },

    /// I/O error during import
    #[error("IO error during import: {0}")]
    Io(#[from] std::io::Error),

    /// Import operation was cancelled by user
    #[error("Import cancelled")]
    Cancelled,
}

/// Errors related to session management
#[derive(Debug, Error)]
pub enum SessionError {
    /// Failed to start session
    #[error("Failed to start session: {0}")]
    StartFailed(String),

    /// Failed to terminate session
    #[error("Failed to terminate session: {0}")]
    TerminateFailed(String),

    /// Session not found
    #[error("Session not found: {0}")]
    NotFound(String),

    /// Session already exists
    #[error("Session already exists: {0}")]
    AlreadyExists(String),

    /// Process management error
    #[error("Process error: {0}")]
    ProcessError(String),

    /// Terminal error
    #[error("Terminal error: {0}")]
    TerminalError(String),

    /// Logging error
    #[error("Logging error: {0}")]
    LoggingError(String),
}

/// Result type alias for `RustConn` operations
pub type Result<T> = std::result::Result<T, RustConnError>;

/// Result type alias for configuration operations
pub type ConfigResult<T> = std::result::Result<T, ConfigError>;

/// Result type alias for protocol operations
pub type ProtocolResult<T> = std::result::Result<T, ProtocolError>;

/// Result type alias for secret operations
pub type SecretResult<T> = std::result::Result<T, SecretError>;

/// Result type alias for import operations
pub type ImportResult<T> = std::result::Result<T, ImportError>;

/// Result type alias for session operations
pub type SessionResult<T> = std::result::Result<T, SessionError>;
