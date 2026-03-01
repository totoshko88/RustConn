//! CLI error types and exit codes.

/// Exit codes for CLI operations
pub mod exit_codes {
    /// General error - configuration, validation, or other non-connection errors
    pub const GENERAL_ERROR: i32 = 1;
    /// Connection failure - connection test failed or connection could not be
    /// established
    pub const CONNECTION_FAILURE: i32 = 2;
}

/// CLI error type
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Connection not found
    #[error("Connection not found: {0}")]
    ConnectionNotFound(String),

    /// Export error
    #[error("Export error: {0}")]
    Export(String),

    /// Import error
    #[error("Import error: {0}")]
    Import(String),

    /// Connection test failed
    #[error("Connection test failed: {0}")]
    TestFailed(String),

    /// Wake-on-LAN error
    #[error("Wake-on-LAN error: {0}")]
    Wol(String),

    /// Snippet error
    #[error("Snippet error: {0}")]
    Snippet(String),

    /// Group error
    #[error("Group error: {0}")]
    Group(String),

    /// Template error
    #[error("Template error: {0}")]
    Template(String),

    /// Cluster error
    #[error("Cluster error: {0}")]
    Cluster(String),

    /// Variable error
    #[error("Variable error: {0}")]
    Variable(String),

    /// Secret backend error
    #[error("Secret error: {0}")]
    Secret(String),

    /// Protocol error
    #[error("Protocol error: {0}")]
    Protocol(String),

    /// Connection error
    #[error("Connection error: {0}")]
    Connection(String),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<rustconn_core::error::RustConnError> for CliError {
    fn from(err: rustconn_core::error::RustConnError) -> Self {
        use rustconn_core::error::RustConnError;
        match err {
            RustConnError::Config(e) => Self::Config(e.to_string()),
            RustConnError::Protocol(e) => Self::Protocol(e.to_string()),
            RustConnError::Secret(e) => Self::Secret(e.to_string()),
            RustConnError::Import(e) => Self::Import(e.to_string()),
            RustConnError::Session(e) => Self::Connection(e.to_string()),
            RustConnError::Io(e) => Self::Io(e),
        }
    }
}

impl CliError {
    /// Returns the appropriate exit code for this error type.
    ///
    /// Exit codes:
    /// - 0: Success (not an error)
    /// - 1: General error (configuration, validation, export, import, IO)
    /// - 2: Connection failure (test failed, connection not found)
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        match self {
            // Connection-related failures use exit code 2
            Self::TestFailed(_) | Self::ConnectionNotFound(_) | Self::Connection(_) => {
                exit_codes::CONNECTION_FAILURE
            }
            // All other errors use exit code 1
            Self::Config(_)
            | Self::Export(_)
            | Self::Import(_)
            | Self::Io(_)
            | Self::Wol(_)
            | Self::Snippet(_)
            | Self::Group(_)
            | Self::Template(_)
            | Self::Cluster(_)
            | Self::Variable(_)
            | Self::Secret(_)
            | Self::Protocol(_) => exit_codes::GENERAL_ERROR,
        }
    }
}
