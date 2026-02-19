//! Variables system for `RustConn`
//!
//! This module provides a hierarchical variable system with support for:
//! - Global, document-level, and connection-level variables
//! - Variable substitution in strings using `${variable_name}` syntax
//! - Nested variable resolution with cycle detection
//! - Secure storage for secret variables

mod manager;

pub use manager::VariableManager;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Maximum depth for nested variable resolution
pub const MAX_NESTING_DEPTH: usize = 10;

/// A variable definition with optional secret flag
///
/// Variables can be defined at different scopes (global, document, connection)
/// and can be marked as secret for secure storage and masked display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Variable {
    /// The variable name (used in `${name}` references)
    pub name: String,
    /// The variable value.
    /// TODO: For `is_secret == true`, consider using `SecretString` or zeroize on Drop.
    pub value: String,
    /// Whether this variable contains sensitive data
    pub is_secret: bool,
    /// Optional description for documentation
    pub description: Option<String>,
}

impl Variable {
    /// Creates a new variable with the given name and value
    #[must_use]
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            is_secret: false,
            description: None,
        }
    }

    /// Creates a new secret variable
    #[must_use]
    pub fn new_secret(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
            is_secret: true,
            description: None,
        }
    }

    /// Sets the description for this variable
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Sets whether this variable is secret
    #[must_use]
    pub const fn with_secret(mut self, is_secret: bool) -> Self {
        self.is_secret = is_secret;
        self
    }

    /// Returns the value for display, masking secret values
    #[must_use]
    pub fn display_value(&self) -> &str {
        if self.is_secret {
            "********"
        } else {
            &self.value
        }
    }

    /// Returns true if this variable is marked as secret
    #[must_use]
    pub const fn is_secret(&self) -> bool {
        self.is_secret
    }
}

/// Generates the secret backend lookup key for a secret variable.
///
/// Format: `rustconn/var/{name}`
#[must_use]
pub fn variable_secret_key(name: &str) -> String {
    format!("rustconn/var/{name}")
}

/// Variable scope for resolution
///
/// Variables are resolved in order from most specific to least specific:
/// Connection -> Document -> Global
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VariableScope {
    /// Global variables available to all connections
    Global,
    /// Document-level variables available to connections within a document
    Document(Uuid),
    /// Connection-level variables specific to a single connection
    Connection(Uuid),
}

impl VariableScope {
    /// Returns the parent scope for resolution chain
    ///
    /// Connection -> Document -> Global -> None
    #[must_use]
    pub const fn parent(&self) -> Option<Self> {
        match self {
            Self::Document(_) => Some(Self::Global),
            // Connection scope needs document ID to get parent, Global has no parent
            Self::Global | Self::Connection(_) => None,
        }
    }
}

/// Errors that can occur during variable operations
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum VariableError {
    /// Variable reference not found in any scope
    #[error("Undefined variable: {0}")]
    Undefined(String),

    /// Circular reference detected during resolution
    #[error("Circular reference detected: {0}")]
    CircularReference(String),

    /// Invalid variable syntax
    #[error("Invalid syntax: {0}")]
    InvalidSyntax(String),

    /// Maximum nesting depth exceeded during resolution
    #[error("Maximum nesting depth ({0}) exceeded")]
    MaxDepthExceeded(usize),

    /// Empty variable name
    #[error("Empty variable name")]
    EmptyName,

    /// Resolved value contains characters unsafe for command arguments
    #[error("Variable '{name}' contains unsafe characters for command use: {reason}")]
    UnsafeValue {
        /// Variable name
        name: String,
        /// Reason the value is unsafe
        reason: String,
    },
}

/// Result type for variable operations
pub type VariableResult<T> = std::result::Result<T, VariableError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_variable_new() {
        let var = Variable::new("test", "value");
        assert_eq!(var.name, "test");
        assert_eq!(var.value, "value");
        assert!(!var.is_secret);
        assert!(var.description.is_none());
    }

    #[test]
    fn test_variable_new_secret() {
        let var = Variable::new_secret("password", "secret123");
        assert_eq!(var.name, "password");
        assert_eq!(var.value, "secret123");
        assert!(var.is_secret);
    }

    #[test]
    fn test_variable_with_description() {
        let var = Variable::new("host", "example.com").with_description("The target host");
        assert_eq!(var.description, Some("The target host".to_string()));
    }

    #[test]
    fn test_variable_scope_parent() {
        assert_eq!(VariableScope::Global.parent(), None);
        assert_eq!(
            VariableScope::Document(Uuid::nil()).parent(),
            Some(VariableScope::Global)
        );
        // Connection scope parent depends on document context
        assert_eq!(VariableScope::Connection(Uuid::nil()).parent(), None);
    }

    #[test]
    fn test_variable_serialization() {
        let var = Variable::new("test", "value")
            .with_description("A test variable")
            .with_secret(true);

        let json = serde_json::to_string(&var).unwrap();
        let parsed: Variable = serde_json::from_str(&json).unwrap();

        assert_eq!(var, parsed);
    }

    #[test]
    fn test_display_value_masks_secrets() {
        let secret_var = Variable::new_secret("password", "super_secret_123");
        assert_eq!(secret_var.display_value(), "********");

        let normal_var = Variable::new("host", "example.com");
        assert_eq!(normal_var.display_value(), "example.com");
    }

    #[test]
    fn test_is_secret_method() {
        let secret_var = Variable::new_secret("password", "secret");
        assert!(secret_var.is_secret());

        let normal_var = Variable::new("host", "example.com");
        assert!(!normal_var.is_secret());
    }
}
