//! Tracing integration for structured logging and performance profiling
//!
//! This module provides utilities for integrating the `tracing` crate into `RustConn`,
//! enabling structured logging with spans for key operations like connection establishment,
//! search execution, import/export, and credential resolution.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use thiserror::Error;
use tracing::Level;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Global flag indicating whether tracing has been initialized
static TRACING_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Global tracing configuration
static TRACING_CONFIG: OnceLock<TracingConfig> = OnceLock::new();

/// Errors that can occur during tracing initialization
#[derive(Debug, Error)]
pub enum TracingError {
    /// Failed to initialize tracing subscriber
    #[error("Failed to initialize tracing: {0}")]
    InitializationFailed(String),

    /// Invalid output configuration
    #[error("Invalid output configuration: {0}")]
    InvalidOutput(String),

    /// Tracing already initialized
    #[error("Tracing has already been initialized")]
    AlreadyInitialized,

    /// Failed to create log file
    #[error("Failed to create log file: {0}")]
    FileCreationFailed(String),
}

/// Result type for tracing operations
pub type TracingResult<T> = Result<T, TracingError>;

/// Tracing log level configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TracingLevel {
    /// Error level - only errors
    Error,
    /// Warn level - errors and warnings
    Warn,
    /// Info level - errors, warnings, and info (default)
    #[default]
    Info,
    /// Debug level - all above plus debug messages
    Debug,
    /// Trace level - all messages including trace
    Trace,
}

impl TracingLevel {
    /// Converts to tracing crate's Level
    #[must_use]
    pub const fn to_tracing_level(self) -> Level {
        match self {
            Self::Error => Level::ERROR,
            Self::Warn => Level::WARN,
            Self::Info => Level::INFO,
            Self::Debug => Level::DEBUG,
            Self::Trace => Level::TRACE,
        }
    }
}

impl std::str::FromStr for TracingLevel {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "error" => Ok(Self::Error),
            "warn" | "warning" => Ok(Self::Warn),
            "info" => Ok(Self::Info),
            "debug" => Ok(Self::Debug),
            "trace" => Ok(Self::Trace),
            _ => Err(()),
        }
    }
}

impl std::fmt::Display for TracingLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Error => write!(f, "error"),
            Self::Warn => write!(f, "warn"),
            Self::Info => write!(f, "info"),
            Self::Debug => write!(f, "debug"),
            Self::Trace => write!(f, "trace"),
        }
    }
}

/// Output destination for tracing logs
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TracingOutput {
    /// Output to stdout
    Stdout,
    /// Output to stderr
    #[default]
    Stderr,
    /// Output to a file
    File {
        /// Path to the log file
        path: PathBuf,
        /// Whether to rotate logs
        rotate: bool,
    },
    /// Output to OpenTelemetry collector (placeholder for future implementation)
    OpenTelemetry {
        /// Endpoint URL for the collector
        endpoint: String,
    },
}

/// Configuration for tracing initialization
#[derive(Debug, Clone)]
pub struct TracingConfig {
    /// Log level
    pub level: TracingLevel,
    /// Output destination
    pub output: TracingOutput,
    /// Whether profiling/timing is enabled
    pub profiling_enabled: bool,
    /// Whether to include connection IDs in spans
    pub include_connection_ids: bool,
    /// Whether to include timing information
    pub include_timing: bool,
    /// Custom filter string (overrides level if set)
    pub filter: Option<String>,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            level: TracingLevel::Info,
            output: TracingOutput::Stderr,
            profiling_enabled: cfg!(debug_assertions),
            include_connection_ids: true,
            include_timing: true,
            filter: None,
        }
    }
}

impl TracingConfig {
    /// Creates a new tracing configuration with default values
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the log level
    #[must_use]
    pub const fn with_level(mut self, level: TracingLevel) -> Self {
        self.level = level;
        self
    }

    /// Sets the output destination
    #[must_use]
    pub fn with_output(mut self, output: TracingOutput) -> Self {
        self.output = output;
        self
    }

    /// Enables or disables profiling
    #[must_use]
    pub const fn with_profiling(mut self, enabled: bool) -> Self {
        self.profiling_enabled = enabled;
        self
    }

    /// Sets whether to include connection IDs
    #[must_use]
    pub const fn with_connection_ids(mut self, include: bool) -> Self {
        self.include_connection_ids = include;
        self
    }

    /// Sets whether to include timing information
    #[must_use]
    pub const fn with_timing(mut self, include: bool) -> Self {
        self.include_timing = include;
        self
    }

    /// Sets a custom filter string
    #[must_use]
    pub fn with_filter(mut self, filter: impl Into<String>) -> Self {
        self.filter = Some(filter.into());
        self
    }

    /// Creates a configuration for development (debug level, stdout)
    #[must_use]
    pub const fn development() -> Self {
        Self {
            level: TracingLevel::Debug,
            output: TracingOutput::Stdout,
            profiling_enabled: true,
            include_connection_ids: true,
            include_timing: true,
            filter: None,
        }
    }

    /// Creates a configuration for production (info level, stderr)
    #[must_use]
    pub const fn production() -> Self {
        Self {
            level: TracingLevel::Info,
            output: TracingOutput::Stderr,
            profiling_enabled: false,
            include_connection_ids: true,
            include_timing: false,
            filter: None,
        }
    }
}

/// Initializes the tracing subscriber with the given configuration
///
/// This function should be called once at application startup.
/// Subsequent calls will return an error.
///
/// # Errors
///
/// Returns an error if:
/// - Tracing has already been initialized
/// - The subscriber fails to initialize
/// - File output is configured but the file cannot be created
pub fn init_tracing(config: &TracingConfig) -> TracingResult<()> {
    // Check if already initialized
    if TRACING_INITIALIZED.swap(true, Ordering::SeqCst) {
        return Err(TracingError::AlreadyInitialized);
    }

    // Store the configuration
    let _ = TRACING_CONFIG.set(config.clone());

    // Build the filter
    let filter = if let Some(ref custom_filter) = config.filter {
        EnvFilter::try_new(custom_filter)
            .map_err(|e| TracingError::InitializationFailed(e.to_string()))?
    } else {
        EnvFilter::try_new(format!("rustconn={}", config.level))
            .unwrap_or_else(|_| EnvFilter::new("info"))
    };

    // Initialize based on output type
    match &config.output {
        TracingOutput::Stdout => {
            tracing_subscriber::registry()
                .with(filter)
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_target(true)
                        .with_level(true)
                        .with_thread_ids(config.profiling_enabled)
                        .with_writer(std::io::stdout),
                )
                .try_init()
                .map_err(|e| TracingError::InitializationFailed(e.to_string()))?;
        }
        TracingOutput::Stderr => {
            tracing_subscriber::registry()
                .with(filter)
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_target(true)
                        .with_level(true)
                        .with_thread_ids(config.profiling_enabled)
                        .with_writer(std::io::stderr),
                )
                .try_init()
                .map_err(|e| TracingError::InitializationFailed(e.to_string()))?;
        }
        TracingOutput::File { path, .. } => {
            let file = std::fs::File::create(path)
                .map_err(|e| TracingError::FileCreationFailed(e.to_string()))?;

            tracing_subscriber::registry()
                .with(filter)
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_target(true)
                        .with_level(true)
                        .with_thread_ids(config.profiling_enabled)
                        .with_ansi(false)
                        .with_writer(file),
                )
                .try_init()
                .map_err(|e| TracingError::InitializationFailed(e.to_string()))?;
        }
        TracingOutput::OpenTelemetry { endpoint } => {
            // OpenTelemetry support is a placeholder for future implementation
            // For now, fall back to stderr with a warning
            tracing_subscriber::registry()
                .with(filter)
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_target(true)
                        .with_level(true)
                        .with_writer(std::io::stderr),
                )
                .try_init()
                .map_err(|e| TracingError::InitializationFailed(e.to_string()))?;

            tracing::warn!(
                endpoint = %endpoint,
                "OpenTelemetry output is not yet implemented, falling back to stderr"
            );
        }
    }

    tracing::info!(
        level = %config.level,
        profiling = config.profiling_enabled,
        "Tracing initialized"
    );

    Ok(())
}

/// Checks if tracing has been initialized
#[must_use]
pub fn is_tracing_initialized() -> bool {
    TRACING_INITIALIZED.load(Ordering::SeqCst)
}

/// Gets the current tracing configuration (if initialized)
#[must_use]
pub fn get_tracing_config() -> Option<&'static TracingConfig> {
    TRACING_CONFIG.get()
}

/// Resets the tracing initialization state (for testing only)
///
/// # Safety
///
/// This function is only intended for use in tests. Using it in production
/// code may lead to undefined behavior with the tracing subscriber.
#[cfg(test)]
pub fn reset_tracing_for_tests() {
    TRACING_INITIALIZED.store(false, Ordering::SeqCst);
}

/// Macro for creating operation spans with standard fields
///
/// This macro creates a tracing span with consistent field naming
/// for `RustConn` operations.
///
/// # Examples
///
/// ```ignore
/// use rustconn_core::trace_operation;
///
/// // Create a span for connection establishment
/// let _span = trace_operation!("connection.establish",
///     connection_id = %conn.id,
///     protocol = %conn.protocol
/// );
///
/// // Create a span for search execution
/// let _span = trace_operation!("search.execute",
///     query = %query,
///     filter_count = filters.len()
/// );
/// ```
#[macro_export]
macro_rules! trace_operation {
    ($name:expr) => {
        tracing::info_span!($name)
    };
    ($name:expr, $($field:tt)*) => {
        tracing::info_span!($name, $($field)*)
    };
}

/// Macro for creating debug-level operation spans
///
/// Similar to `trace_operation!` but at debug level for less important operations.
#[macro_export]
macro_rules! trace_operation_debug {
    ($name:expr) => {
        tracing::debug_span!($name)
    };
    ($name:expr, $($field:tt)*) => {
        tracing::debug_span!($name, $($field)*)
    };
}

/// Standard span names for `RustConn` operations
pub mod span_names {
    /// Connection establishment span
    pub const CONNECTION_ESTABLISH: &str = "connection.establish";
    /// Connection disconnect span
    pub const CONNECTION_DISCONNECT: &str = "connection.disconnect";
    /// Search execution span
    pub const SEARCH_EXECUTE: &str = "search.execute";
    /// Search cache lookup span
    pub const SEARCH_CACHE_LOOKUP: &str = "search.cache_lookup";
    /// Import operation span
    pub const IMPORT_EXECUTE: &str = "import.execute";
    /// Export operation span
    pub const EXPORT_EXECUTE: &str = "export.execute";
    /// Credential resolution span
    pub const CREDENTIAL_RESOLVE: &str = "credential.resolve";
    /// Credential store span
    pub const CREDENTIAL_STORE: &str = "credential.store";
    /// Configuration load span
    pub const CONFIG_LOAD: &str = "config.load";
    /// Configuration save span
    pub const CONFIG_SAVE: &str = "config.save";
    /// Session start span
    pub const SESSION_START: &str = "session.start";
    /// Session end span
    pub const SESSION_END: &str = "session.end";
}

/// Standard field names for tracing spans
pub mod field_names {
    /// Connection ID field
    pub const CONNECTION_ID: &str = "connection_id";
    /// Protocol type field
    pub const PROTOCOL: &str = "protocol";
    /// Host field
    pub const HOST: &str = "host";
    /// Port field
    pub const PORT: &str = "port";
    /// Username field.
    /// WARNING: May contain PII. Use only at debug/trace level.
    pub const USERNAME: &str = "username";
    /// Query field (for search)
    pub const QUERY: &str = "query";
    /// Result count field
    pub const RESULT_COUNT: &str = "result_count";
    /// Duration field (in milliseconds)
    pub const DURATION_MS: &str = "duration_ms";
    /// Success field
    pub const SUCCESS: &str = "success";
    /// Error message field
    pub const ERROR: &str = "error";
    /// Format field (for import/export)
    pub const FORMAT: &str = "format";
    /// Item count field
    pub const ITEM_COUNT: &str = "item_count";
    /// Cache hit field
    pub const CACHE_HIT: &str = "cache_hit";
    /// Session ID field
    pub const SESSION_ID: &str = "session_id";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracing_level_from_str() {
        assert_eq!("error".parse::<TracingLevel>(), Ok(TracingLevel::Error));
        assert_eq!("WARN".parse::<TracingLevel>(), Ok(TracingLevel::Warn));
        assert_eq!("Info".parse::<TracingLevel>(), Ok(TracingLevel::Info));
        assert_eq!("debug".parse::<TracingLevel>(), Ok(TracingLevel::Debug));
        assert_eq!("trace".parse::<TracingLevel>(), Ok(TracingLevel::Trace));
        assert!("invalid".parse::<TracingLevel>().is_err());
    }

    #[test]
    fn test_tracing_level_display() {
        assert_eq!(TracingLevel::Error.to_string(), "error");
        assert_eq!(TracingLevel::Warn.to_string(), "warn");
        assert_eq!(TracingLevel::Info.to_string(), "info");
        assert_eq!(TracingLevel::Debug.to_string(), "debug");
        assert_eq!(TracingLevel::Trace.to_string(), "trace");
    }

    #[test]
    fn test_tracing_config_builder() {
        let config = TracingConfig::new()
            .with_level(TracingLevel::Debug)
            .with_output(TracingOutput::Stdout)
            .with_profiling(true)
            .with_connection_ids(false)
            .with_timing(true)
            .with_filter("rustconn=debug,tokio=warn");

        assert_eq!(config.level, TracingLevel::Debug);
        assert_eq!(config.output, TracingOutput::Stdout);
        assert!(config.profiling_enabled);
        assert!(!config.include_connection_ids);
        assert!(config.include_timing);
        assert_eq!(config.filter, Some("rustconn=debug,tokio=warn".to_string()));
    }

    #[test]
    fn test_development_config() {
        let config = TracingConfig::development();
        assert_eq!(config.level, TracingLevel::Debug);
        assert_eq!(config.output, TracingOutput::Stdout);
        assert!(config.profiling_enabled);
    }

    #[test]
    fn test_production_config() {
        let config = TracingConfig::production();
        assert_eq!(config.level, TracingLevel::Info);
        assert_eq!(config.output, TracingOutput::Stderr);
        assert!(!config.profiling_enabled);
    }

    #[test]
    fn test_tracing_output_default() {
        let output = TracingOutput::default();
        assert_eq!(output, TracingOutput::Stderr);
    }
}
