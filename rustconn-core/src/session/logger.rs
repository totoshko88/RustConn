//! Session logging functionality
//!
//! This module provides session logging capabilities for recording
//! terminal output to timestamped log files with configurable rotation
//! and retention policies.

use chrono::{Local, Utc};
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::variables::{VariableManager, VariableScope};

/// Errors that can occur during logging operations
#[derive(Debug, Error)]
pub enum LogError {
    /// Failed to create log directory
    #[error("Failed to create log directory: {0}")]
    DirectoryCreation(String),

    /// Failed to create or open log file
    #[error("Failed to create/open log file: {0}")]
    FileCreation(String),

    /// Failed to write to log file
    #[error("Failed to write to log: {0}")]
    WriteError(String),

    /// Failed to flush log file
    #[error("Failed to flush log: {0}")]
    FlushError(String),

    /// Failed to rotate log file
    #[error("Failed to rotate log: {0}")]
    RotationError(String),

    /// Invalid path template
    #[error("Invalid path template: {0}")]
    InvalidTemplate(String),

    /// Failed to expand path template
    #[error("Failed to expand path template: {0}")]
    TemplateExpansion(String),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type for logging operations
pub type LogResult<T> = std::result::Result<T, LogError>;

/// Log configuration for session logging
///
/// Defines how session output should be logged, including file paths,
/// timestamp formatting, and retention policies.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // Logging modes are independent boolean flags
pub struct LogConfig {
    /// Whether logging is enabled
    pub enabled: bool,
    /// Path template for log files (supports variables like `${connection_name}`, `${date}`, `${time}`, `${protocol}`)
    pub path_template: String,
    /// Timestamp format string (strftime format)
    pub timestamp_format: String,
    /// Maximum log file size in megabytes (0 = no limit)
    pub max_size_mb: u32,
    /// Number of days to retain log files (0 = no limit)
    pub retention_days: u32,
    /// Log terminal activity (change counts) - default mode
    pub log_activity: bool,
    /// Log user input (commands typed)
    pub log_input: bool,
    /// Log full terminal output (transcript)
    pub log_output: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path_template: String::from(
                "${HOME}/.local/share/rustconn/logs/${connection_name}_${date}.log",
            ),
            timestamp_format: String::from("%Y-%m-%d %H:%M:%S"),
            max_size_mb: 10,
            retention_days: 30,
            log_activity: true,
            log_input: false,
            log_output: false,
        }
    }
}

impl LogConfig {
    /// Creates a new `LogConfig` with the specified path template
    #[must_use]
    pub fn new(path_template: impl Into<String>) -> Self {
        Self {
            enabled: true,
            path_template: path_template.into(),
            ..Default::default()
        }
    }

    /// Sets whether logging is enabled
    #[must_use]
    pub const fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Sets the timestamp format
    #[must_use]
    pub fn with_timestamp_format(mut self, format: impl Into<String>) -> Self {
        self.timestamp_format = format.into();
        self
    }

    /// Sets the maximum log file size in megabytes
    #[must_use]
    pub const fn with_max_size_mb(mut self, max_size_mb: u32) -> Self {
        self.max_size_mb = max_size_mb;
        self
    }

    /// Sets the retention period in days
    #[must_use]
    pub const fn with_retention_days(mut self, retention_days: u32) -> Self {
        self.retention_days = retention_days;
        self
    }

    /// Sets whether to log terminal activity (change counts)
    #[must_use]
    pub const fn with_log_activity(mut self, enabled: bool) -> Self {
        self.log_activity = enabled;
        self
    }

    /// Sets whether to log user input (commands)
    #[must_use]
    pub const fn with_log_input(mut self, enabled: bool) -> Self {
        self.log_input = enabled;
        self
    }

    /// Sets whether to log full terminal output (transcript)
    #[must_use]
    pub const fn with_log_output(mut self, enabled: bool) -> Self {
        self.log_output = enabled;
        self
    }

    /// Validates the configuration
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid (e.g., empty path template when enabled).
    pub fn validate(&self) -> LogResult<()> {
        if self.enabled && self.path_template.is_empty() {
            return Err(LogError::InvalidTemplate(
                "Path template cannot be empty".to_string(),
            ));
        }
        Ok(())
    }
}

// Implement serde traits manually to support serialization
impl serde::Serialize for LogConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("LogConfig", 8)?;
        state.serialize_field("enabled", &self.enabled)?;
        state.serialize_field("path_template", &self.path_template)?;
        state.serialize_field("timestamp_format", &self.timestamp_format)?;
        state.serialize_field("max_size_mb", &self.max_size_mb)?;
        state.serialize_field("retention_days", &self.retention_days)?;
        state.serialize_field("log_activity", &self.log_activity)?;
        state.serialize_field("log_input", &self.log_input)?;
        state.serialize_field("log_output", &self.log_output)?;
        state.end()
    }
}

impl<'de> serde::Deserialize<'de> for LogConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[allow(clippy::struct_excessive_bools)]
        struct LogConfigHelper {
            enabled: bool,
            path_template: String,
            timestamp_format: String,
            max_size_mb: u32,
            retention_days: u32,
            #[serde(default = "default_log_activity")]
            log_activity: bool,
            #[serde(default)]
            log_input: bool,
            #[serde(default)]
            log_output: bool,
        }

        fn default_log_activity() -> bool {
            true
        }

        let helper = LogConfigHelper::deserialize(deserializer)?;
        Ok(Self {
            enabled: helper.enabled,
            path_template: helper.path_template,
            timestamp_format: helper.timestamp_format,
            max_size_mb: helper.max_size_mb,
            retention_days: helper.retention_days,
            log_activity: helper.log_activity,
            log_input: helper.log_input,
            log_output: helper.log_output,
        })
    }
}

/// Context for path template expansion
///
/// Contains the variables that can be used in log path templates.
#[derive(Debug, Clone, Default)]
pub struct LogContext {
    /// Connection name
    pub connection_name: String,
    /// Protocol type (ssh, rdp, vnc, spice)
    pub protocol: String,
    /// Additional custom variables
    pub custom_vars: std::collections::HashMap<String, String>,
}

impl LogContext {
    /// Creates a new `LogContext` with the given connection name and protocol
    #[must_use]
    pub fn new(connection_name: impl Into<String>, protocol: impl Into<String>) -> Self {
        Self {
            connection_name: connection_name.into(),
            protocol: protocol.into(),
            custom_vars: std::collections::HashMap::new(),
        }
    }

    /// Adds a custom variable to the context
    #[must_use]
    pub fn with_var(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.custom_vars.insert(name.into(), value.into());
        self
    }
}

/// Session logger for writing terminal output to files
///
/// Handles log file creation, writing with timestamps, rotation,
/// and cleanup based on retention policies.
pub struct SessionLogger {
    /// Log configuration
    config: LogConfig,
    /// Current log file path
    log_path: PathBuf,
    /// Buffered file writer
    writer: Option<BufWriter<File>>,
    /// Bytes written to current log file
    bytes_written: u64,
    /// Rotation counter for current session
    rotation_count: u32,
}

impl SessionLogger {
    /// Creates a new session logger with the given configuration and context
    ///
    /// # Arguments
    ///
    /// * `config` - Log configuration
    /// * `context` - Context for path template expansion
    /// * `variable_manager` - Optional variable manager for additional substitution
    ///
    /// # Errors
    ///
    /// Returns an error if the log file cannot be created.
    pub fn new(
        config: LogConfig,
        context: &LogContext,
        variable_manager: Option<&VariableManager>,
    ) -> LogResult<Self> {
        config.validate()?;

        if !config.enabled {
            return Ok(Self {
                config,
                log_path: PathBuf::new(),
                writer: None,
                bytes_written: 0,
                rotation_count: 0,
            });
        }

        // Expand the path template
        let log_path =
            Self::expand_path_template(&config.path_template, context, variable_manager)?;

        // Create parent directories if needed
        if let Some(parent) = log_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                LogError::DirectoryCreation(format!("Failed to create {}: {}", parent.display(), e))
            })?;
        }

        // Create the log file
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .map_err(|e| {
                LogError::FileCreation(format!("Failed to open {}: {}", log_path.display(), e))
            })?;

        let writer = BufWriter::new(file);

        // Get current file size
        let bytes_written = fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);

        Ok(Self {
            config,
            log_path,
            writer: Some(writer),
            bytes_written,
            rotation_count: 0,
        })
    }

    /// Expands a path template with context variables
    ///
    /// Supports the following variables:
    /// - `${connection_name}` - The connection name
    /// - `${protocol}` - The protocol type
    /// - `${date}` - Current date (YYYY-MM-DD)
    /// - `${time}` - Current time (HH-MM-SS)
    /// - `${datetime}` - Current datetime (YYYY-MM-DD_HH-MM-SS)
    /// - `${HOME}` - User's home directory
    /// - Any custom variables from the context
    /// - Any variables from the `VariableManager`
    ///
    /// # Errors
    ///
    /// Returns an error if a variable cannot be expanded or is undefined.
    pub fn expand_path_template(
        template: &str,
        context: &LogContext,
        variable_manager: Option<&VariableManager>,
    ) -> LogResult<PathBuf> {
        let now = Local::now();
        let mut result = template.to_string();

        // Built-in variables
        let builtins = [
            (
                "connection_name",
                sanitize_filename(&context.connection_name),
            ),
            ("protocol", context.protocol.clone()),
            ("date", now.format("%Y-%m-%d").to_string()),
            ("time", now.format("%H-%M-%S").to_string()),
            ("datetime", now.format("%Y-%m-%d_%H-%M-%S").to_string()),
            (
                "HOME",
                dirs::home_dir()
                    .map_or_else(|| ".".to_string(), |p| p.to_string_lossy().to_string()),
            ),
        ];

        for (name, value) in &builtins {
            let pattern = format!("${{{name}}}");
            result = result.replace(&pattern, value);
        }

        // Custom context variables
        for (name, value) in &context.custom_vars {
            let pattern = format!("${{{name}}}");
            result = result.replace(&pattern, value);
        }

        // Variable manager substitution (if provided)
        if let Some(vm) = variable_manager {
            // Try to substitute remaining variables using the variable manager
            result = vm
                .substitute(&result, VariableScope::Global)
                .map_err(|e| LogError::TemplateExpansion(e.to_string()))?;
        }

        // Check for any remaining unsubstituted variables
        if result.contains("${") {
            // Extract the first unsubstituted variable for error message
            if let Some(start) = result.find("${") {
                if let Some(end) = result[start..].find('}') {
                    let var_name = &result[start + 2..start + end];
                    return Err(LogError::TemplateExpansion(format!(
                        "Undefined variable: {var_name}"
                    )));
                }
            }
        }

        Ok(PathBuf::from(result))
    }

    /// Formats a timestamp according to the configured format
    #[must_use]
    pub fn format_timestamp(&self, format: &str) -> String {
        Local::now().format(format).to_string()
    }

    /// Returns the current timestamp formatted according to config
    #[must_use]
    pub fn current_timestamp(&self) -> String {
        self.format_timestamp(&self.config.timestamp_format)
    }

    /// Returns the log file path
    #[must_use]
    pub fn log_path(&self) -> &Path {
        &self.log_path
    }

    /// Returns the number of bytes written to the current log file
    #[must_use]
    pub const fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Returns whether logging is enabled
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Returns the log configuration
    #[must_use]
    pub const fn config(&self) -> &LogConfig {
        &self.config
    }

    /// Writes data to the log file with a timestamp prefix
    ///
    /// # Errors
    ///
    /// Returns an error if writing fails or rotation fails.
    pub fn write(&mut self, data: &[u8]) -> LogResult<()> {
        if !self.config.enabled {
            return Ok(());
        }

        // Check that writer is available
        if self.writer.is_none() {
            return Err(LogError::WriteError("Log file not open".to_string()));
        }

        // Check if rotation is needed before writing
        self.rotate_if_needed()?;

        // Write timestamp prefix for each line
        let timestamp = self.current_timestamp();
        let data_str = String::from_utf8_lossy(data);

        for line in data_str.lines() {
            let formatted = format!("[{timestamp}] {line}\n");
            let bytes = formatted.as_bytes();

            // Get writer (may have changed after rotation)
            let writer = self.writer.as_mut().ok_or_else(|| {
                LogError::WriteError("Log file not open after rotation".to_string())
            })?;

            writer
                .write_all(bytes)
                .map_err(|e| LogError::WriteError(format!("Failed to write: {e}")))?;

            self.bytes_written += bytes.len() as u64;
        }

        Ok(())
    }

    /// Writes raw data to the log file without timestamp prefix
    ///
    /// # Errors
    ///
    /// Returns an error if writing fails.
    pub fn write_raw(&mut self, data: &[u8]) -> LogResult<()> {
        if !self.config.enabled {
            return Ok(());
        }

        // Check if rotation is needed before writing
        self.rotate_if_needed()?;

        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| LogError::WriteError("Log file not open".to_string()))?;

        writer
            .write_all(data)
            .map_err(|e| LogError::WriteError(format!("Failed to write: {e}")))?;

        self.bytes_written += data.len() as u64;
        Ok(())
    }

    /// Flushes the log buffer to disk
    ///
    /// # Errors
    ///
    /// Returns an error if flushing fails.
    pub fn flush(&mut self) -> LogResult<()> {
        if let Some(writer) = self.writer.as_mut() {
            writer
                .flush()
                .map_err(|e| LogError::FlushError(format!("Failed to flush: {e}")))?;
        }
        Ok(())
    }

    /// Checks if log rotation is needed and performs it if necessary
    fn rotate_if_needed(&mut self) -> LogResult<()> {
        if self.config.max_size_mb == 0 {
            return Ok(()); // No size limit
        }

        let max_bytes = u64::from(self.config.max_size_mb) * 1024 * 1024;

        if self.bytes_written >= max_bytes {
            self.rotate()?;
        }

        Ok(())
    }

    /// Rotates the log file
    ///
    /// Creates a new log file with a rotation suffix and updates the writer.
    ///
    /// # Errors
    ///
    /// Returns an error if rotation fails.
    pub fn rotate(&mut self) -> LogResult<()> {
        if !self.config.enabled {
            return Ok(());
        }

        // Flush and close current file
        self.flush()?;
        self.writer = None;

        // Generate rotated filename
        self.rotation_count += 1;
        let rotated_path = self.generate_rotated_path();

        // Rename current log to rotated name
        if self.log_path.exists() {
            fs::rename(&self.log_path, &rotated_path).map_err(|e| {
                LogError::RotationError(format!(
                    "Failed to rename {} to {}: {}",
                    self.log_path.display(),
                    rotated_path.display(),
                    e
                ))
            })?;
        }

        // Create new log file
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)
            .map_err(|e| {
                LogError::FileCreation(format!(
                    "Failed to create new log file {}: {}",
                    self.log_path.display(),
                    e
                ))
            })?;

        self.writer = Some(BufWriter::new(file));
        self.bytes_written = 0;

        // Clean up old rotated files based on retention policy
        self.cleanup_old_logs()?;

        Ok(())
    }

    /// Generates a path for a rotated log file
    fn generate_rotated_path(&self) -> PathBuf {
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let stem = self
            .log_path
            .file_stem()
            .map_or_else(|| "log".to_string(), |s| s.to_string_lossy().to_string());
        let ext = self
            .log_path
            .extension()
            .map(|s| format!(".{}", s.to_string_lossy()))
            .unwrap_or_default();

        let rotated_name = format!("{stem}.{timestamp}.{}{ext}", self.rotation_count);

        self.log_path.with_file_name(rotated_name)
    }

    /// Cleans up old log files based on retention policy
    #[allow(clippy::unnecessary_wraps)]
    fn cleanup_old_logs(&self) -> LogResult<()> {
        if self.config.retention_days == 0 {
            return Ok(()); // No retention limit
        }

        let Some(parent) = self.log_path.parent() else {
            return Ok(());
        };

        let cutoff = std::time::SystemTime::now()
            - std::time::Duration::from_secs(u64::from(self.config.retention_days) * 24 * 60 * 60);

        let Ok(entries) = fs::read_dir(parent) else {
            return Ok(()); // Directory might not exist yet
        };

        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();

            // Only process log files
            if path.extension().is_some_and(|ext| ext == "log") {
                if let Ok(metadata) = fs::metadata(&path) {
                    if let Ok(modified) = metadata.modified() {
                        if modified < cutoff {
                            let _ = fs::remove_file(&path);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Closes the log file, flushing any buffered data
    ///
    /// # Errors
    ///
    /// Returns an error if flushing fails.
    pub fn close(&mut self) -> LogResult<()> {
        if let Some(mut writer) = self.writer.take() {
            // Write session end marker
            let timestamp = self.current_timestamp();
            let end_marker = format!("\n[{timestamp}] === Session ended ===\n");
            let _ = writer.write_all(end_marker.as_bytes());

            writer
                .flush()
                .map_err(|e| LogError::FlushError(format!("Failed to flush on close: {e}")))?;
        }
        Ok(())
    }
}

impl Drop for SessionLogger {
    fn drop(&mut self) {
        // Attempt to close gracefully, ignoring errors
        let _ = self.close();
    }
}

/// Sanitizes a filename by removing or replacing invalid characters
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .chars()
        .take(64) // Limit length
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_log_config_default() {
        let config = LogConfig::default();
        assert!(!config.enabled);
        assert!(!config.path_template.is_empty());
        assert_eq!(config.max_size_mb, 10);
        assert_eq!(config.retention_days, 30);
    }

    #[test]
    fn test_log_config_builder() {
        let config = LogConfig::new("/tmp/test.log")
            .with_enabled(true)
            .with_timestamp_format("%H:%M:%S")
            .with_max_size_mb(5)
            .with_retention_days(7);

        assert!(config.enabled);
        assert_eq!(config.path_template, "/tmp/test.log");
        assert_eq!(config.timestamp_format, "%H:%M:%S");
        assert_eq!(config.max_size_mb, 5);
        assert_eq!(config.retention_days, 7);
    }

    #[test]
    fn test_log_config_validation() {
        let valid_config = LogConfig::new("/tmp/test.log").with_enabled(true);
        assert!(valid_config.validate().is_ok());

        let invalid_config = LogConfig::new("").with_enabled(true);
        assert!(invalid_config.validate().is_err());

        let disabled_config = LogConfig::new("").with_enabled(false);
        assert!(disabled_config.validate().is_ok());
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("test-server"), "test-server");
        assert_eq!(sanitize_filename("test server"), "test_server");
        assert_eq!(sanitize_filename("test/server"), "test_server");
        assert_eq!(sanitize_filename("test:server"), "test_server");
        assert_eq!(sanitize_filename("test@server.com"), "test_server.com");
    }

    #[test]
    fn test_log_context() {
        let context = LogContext::new("my-server", "ssh").with_var("custom", "value");

        assert_eq!(context.connection_name, "my-server");
        assert_eq!(context.protocol, "ssh");
        assert_eq!(
            context.custom_vars.get("custom"),
            Some(&"value".to_string())
        );
    }

    #[test]
    fn test_expand_path_template_basic() {
        let context = LogContext::new("test-server", "ssh");
        let template = "/tmp/${connection_name}_${protocol}.log";

        let result = SessionLogger::expand_path_template(template, &context, None).unwrap();
        assert_eq!(result, PathBuf::from("/tmp/test-server_ssh.log"));
    }

    #[test]
    fn test_expand_path_template_with_date() {
        let context = LogContext::new("server", "vnc");
        let template = "/tmp/${connection_name}_${date}.log";

        let result = SessionLogger::expand_path_template(template, &context, None).unwrap();
        let result_str = result.to_string_lossy();

        assert!(result_str.starts_with("/tmp/server_"));
        assert!(result_str.ends_with(".log"));
        // Date format: YYYY-MM-DD
        assert!(result_str.contains('-'));
    }

    #[test]
    fn test_expand_path_template_undefined_var() {
        let context = LogContext::new("server", "ssh");
        let template = "/tmp/${undefined_var}.log";

        let result = SessionLogger::expand_path_template(template, &context, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_session_logger_disabled() {
        let config = LogConfig::default(); // disabled by default
        let context = LogContext::new("test", "ssh");

        let logger = SessionLogger::new(config, &context, None).unwrap();
        assert!(!logger.is_enabled());
        assert!(logger.writer.is_none());
    }

    #[test]
    fn test_session_logger_creation() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("test.log");

        let config = LogConfig::new(log_path.to_string_lossy().to_string()).with_enabled(true);
        let context = LogContext::new("test", "ssh");

        let logger = SessionLogger::new(config, &context, None).unwrap();
        assert!(logger.is_enabled());
        assert!(logger.writer.is_some());
        assert!(log_path.exists());
    }

    #[test]
    fn test_session_logger_write() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("test.log");

        let config = LogConfig::new(log_path.to_string_lossy().to_string()).with_enabled(true);
        let log_ctx = LogContext::new("test", "ssh");

        let mut logger = SessionLogger::new(config, &log_ctx, None).unwrap();
        logger.write(b"Hello, World!").unwrap();
        logger.flush().unwrap();

        let log_content = fs::read_to_string(&log_path).unwrap();
        assert!(log_content.contains("Hello, World!"));
        assert!(log_content.contains('[') && log_content.contains(']')); // Has timestamp
    }

    #[test]
    fn test_session_logger_close() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("test.log");

        let config = LogConfig::new(log_path.to_string_lossy().to_string()).with_enabled(true);
        let log_ctx = LogContext::new("test", "ssh");

        let mut logger = SessionLogger::new(config, &log_ctx, None).unwrap();
        logger.write(b"Test data").unwrap();
        logger.close().unwrap();

        let log_content = fs::read_to_string(&log_path).unwrap();
        assert!(log_content.contains("Session ended"));
    }

    #[test]
    fn test_log_config_serialization() {
        let config = LogConfig::new("/tmp/test.log")
            .with_enabled(true)
            .with_timestamp_format("%H:%M:%S")
            .with_max_size_mb(5)
            .with_retention_days(7);

        let json = serde_json::to_string(&config).unwrap();
        let parsed: LogConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(config, parsed);
    }

    #[test]
    fn test_format_timestamp() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("test.log");

        let config = LogConfig::new(log_path.to_string_lossy().to_string())
            .with_enabled(true)
            .with_timestamp_format("%Y-%m-%d");
        let context = LogContext::new("test", "ssh");

        let logger = SessionLogger::new(config, &context, None).unwrap();
        let timestamp = logger.current_timestamp();

        // Should be in YYYY-MM-DD format
        assert_eq!(timestamp.len(), 10);
        assert!(timestamp.chars().nth(4) == Some('-'));
        assert!(timestamp.chars().nth(7) == Some('-'));
    }
}
