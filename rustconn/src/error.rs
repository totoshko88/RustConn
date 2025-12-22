//! Error types for the `RustConn` GUI application
//!
//! This module defines typed errors for application state operations,
//! replacing generic `String` errors with structured error types.

use std::path::PathBuf;
use thiserror::Error;
use uuid::Uuid;

/// Errors that can occur during application state operations
#[derive(Debug, Error)]
pub enum AppStateError {
    /// Failed to initialize a manager component
    #[error("Failed to initialize {component}: {reason}")]
    InitializationFailed {
        /// The component that failed to initialize
        component: &'static str,
        /// The reason for failure
        reason: String,
    },

    /// Connection not found
    #[error("Connection not found: {0}")]
    ConnectionNotFound(Uuid),

    /// Group not found
    #[error("Group not found: {0}")]
    GroupNotFound(Uuid),

    /// Session not found
    #[error("Session not found: {0}")]
    SessionNotFound(Uuid),

    /// Snippet not found
    #[error("Snippet not found: {0}")]
    SnippetNotFound(Uuid),

    /// Document not found
    #[error("Document not found: {0}")]
    DocumentNotFound(Uuid),

    /// Cluster not found
    #[error("Cluster not found: {0}")]
    ClusterNotFound(Uuid),

    /// Template not found
    #[error("Template not found: {0}")]
    TemplateNotFound(Uuid),

    /// Duplicate name error
    #[error("{entity_type} with name '{name}' already exists")]
    DuplicateName {
        /// The type of entity (Connection, Group, etc.)
        entity_type: &'static str,
        /// The duplicate name
        name: String,
    },

    /// Failed to create entity
    #[error("Failed to create {entity_type}: {reason}")]
    CreateFailed {
        /// The type of entity
        entity_type: &'static str,
        /// The reason for failure
        reason: String,
    },

    /// Failed to update entity
    #[error("Failed to update {entity_type}: {reason}")]
    UpdateFailed {
        /// The type of entity
        entity_type: &'static str,
        /// The reason for failure
        reason: String,
    },

    /// Failed to delete entity
    #[error("Failed to delete {entity_type}: {reason}")]
    DeleteFailed {
        /// The type of entity
        entity_type: &'static str,
        /// The reason for failure
        reason: String,
    },

    /// Credential operation failed
    #[error("Credential operation failed: {0}")]
    CredentialError(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// Import error
    #[error("Import failed: {0}")]
    ImportError(String),

    /// Export error
    #[error("Export failed: {0}")]
    ExportError(String),

    /// Document I/O error
    #[error("Document I/O error for {path}: {reason}")]
    DocumentIoError {
        /// The path involved
        path: PathBuf,
        /// The reason for failure
        reason: String,
    },

    /// Session operation failed
    #[error("Session operation failed: {0}")]
    SessionError(String),

    /// Clipboard is empty
    #[error("Clipboard is empty")]
    ClipboardEmpty,

    /// Runtime creation failed
    #[error("Failed to create async runtime: {0}")]
    RuntimeError(String),
}

/// Result type alias for application state operations
pub type AppStateResult<T> = Result<T, AppStateError>;

impl From<std::io::Error> for AppStateError {
    fn from(err: std::io::Error) -> Self {
        Self::ConfigError(err.to_string())
    }
}
