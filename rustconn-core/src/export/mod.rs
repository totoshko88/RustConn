//! Export module for converting `RustConn` connections to external formats.
//!
//! This module provides functionality to export connections to various formats
//! including Ansible inventory, SSH config, Remmina, Asbru-CM, and `RustConn` native format.
//!
//! For large exports (more than 10 connections), use `BatchExporter` for
//! efficient batch processing with progress reporting and cancellation support.

pub mod ansible;
pub mod asbru;
pub mod batch;
pub mod native;
pub mod remmina;
pub mod royalts;
pub mod ssh_config;

use std::path::PathBuf;

pub use ansible::AnsibleExporter;
pub use asbru::AsbruExporter;
pub use batch::{
    BatchExportCancelHandle, BatchExportResult, BatchExporter, BATCH_EXPORT_THRESHOLD,
    DEFAULT_EXPORT_BATCH_SIZE,
};
pub use native::{NativeExport, NativeImportError, NATIVE_FILE_EXTENSION, NATIVE_FORMAT_VERSION};
pub use remmina::RemminaExporter;
pub use royalts::RoyalTsExporter;
pub use ssh_config::SshConfigExporter;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::models::{Connection, ConnectionGroup};
use crate::progress::ProgressReporter;

/// Export format types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    /// Ansible inventory format (INI or YAML)
    Ansible,
    /// OpenSSH config format (~/.ssh/config)
    SshConfig,
    /// Remmina connection files (.remmina)
    Remmina,
    /// Asbru-CM YAML configuration
    Asbru,
    /// `RustConn` native format (.rcn)
    Native,
    /// Royal TS XML format (.rtsz)
    RoyalTs,
}

impl ExportFormat {
    /// Returns all available export formats
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Ansible,
            Self::SshConfig,
            Self::Remmina,
            Self::Asbru,
            Self::Native,
            Self::RoyalTs,
        ]
    }

    /// Returns the display name for this export format
    #[must_use]
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::Ansible => "Ansible Inventory",
            Self::SshConfig => "SSH Config",
            Self::Remmina => "Remmina",
            Self::Asbru => "Asbru-CM",
            Self::Native => "RustConn Native",
            Self::RoyalTs => "Royal TS",
        }
    }

    /// Returns the file extension for this export format
    #[must_use]
    pub const fn file_extension(&self) -> &'static str {
        match self {
            Self::Ansible => "ini",
            Self::SshConfig => "config",
            Self::Remmina => "remmina",
            Self::Asbru => "yml",
            Self::Native => NATIVE_FILE_EXTENSION,
            Self::RoyalTs => "rtsz",
        }
    }

    /// Returns true if this format exports to a directory (multiple files)
    #[must_use]
    pub const fn exports_to_directory(&self) -> bool {
        matches!(self, Self::Remmina)
    }
}

impl std::fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Options for export operations
#[derive(Debug, Clone)]
pub struct ExportOptions {
    /// The export format to use
    pub format: ExportFormat,
    /// Whether to include passwords in the export (if supported)
    pub include_passwords: bool,
    /// Whether to include group hierarchy in the export
    pub include_groups: bool,
    /// Output path (file or directory depending on format)
    pub output_path: PathBuf,
}

impl ExportOptions {
    /// Creates new export options with the specified format and output path
    #[must_use]
    pub const fn new(format: ExportFormat, output_path: PathBuf) -> Self {
        Self {
            format,
            include_passwords: false,
            include_groups: true,
            output_path,
        }
    }

    /// Sets whether to include passwords
    #[must_use]
    pub const fn with_passwords(mut self, include: bool) -> Self {
        self.include_passwords = include;
        self
    }

    /// Sets whether to include groups
    #[must_use]
    pub const fn with_groups(mut self, include: bool) -> Self {
        self.include_groups = include;
        self
    }
}

/// Result of an export operation
#[derive(Debug, Default)]
pub struct ExportResult {
    /// Number of connections successfully exported
    pub exported_count: usize,
    /// Number of connections skipped (e.g., unsupported protocol)
    pub skipped_count: usize,
    /// Warnings generated during export
    pub warnings: Vec<String>,
    /// Output files created
    pub output_files: Vec<PathBuf>,
}

impl ExportResult {
    /// Creates a new empty export result
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the total number of connections processed
    #[must_use]
    pub const fn total_processed(&self) -> usize {
        self.exported_count + self.skipped_count
    }

    /// Returns true if any connections were skipped
    #[must_use]
    pub const fn has_skipped(&self) -> bool {
        self.skipped_count > 0
    }

    /// Returns true if any warnings were generated
    #[must_use]
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }

    /// Returns a summary string of the export result
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "Exported: {}, Skipped: {}, Warnings: {}",
            self.exported_count,
            self.skipped_count,
            self.warnings.len()
        )
    }

    /// Adds a warning message
    pub fn add_warning(&mut self, warning: impl Into<String>) {
        self.warnings.push(warning.into());
    }

    /// Adds an output file to the result
    pub fn add_output_file(&mut self, path: PathBuf) {
        self.output_files.push(path);
    }

    /// Increments the exported count
    pub const fn increment_exported(&mut self) {
        self.exported_count += 1;
    }

    /// Increments the skipped count
    pub const fn increment_skipped(&mut self) {
        self.skipped_count += 1;
    }
}

/// Errors that can occur during export operations
#[derive(Debug, Error)]
pub enum ExportError {
    /// The protocol is not supported for this export format
    #[error("Unsupported protocol for export: {0}")]
    UnsupportedProtocol(String),

    /// Failed to write output file
    #[error("Failed to write output: {0}")]
    WriteError(String),

    /// Invalid connection data for export
    #[error("Invalid connection data: {0}")]
    InvalidData(String),

    /// Output path is invalid
    #[error("Invalid output path: {0}")]
    InvalidPath(String),

    /// I/O error during export
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Export operation was cancelled
    #[error("Export cancelled")]
    Cancelled,
}

/// Result type alias for export operations
pub type ExportResult2<T> = std::result::Result<T, ExportError>;

/// Trait for export implementations.
///
/// Each export format (Ansible, SSH config, Remmina, Asbru) implements
/// this trait to provide a uniform interface for exporting connections.
pub trait ExportTarget: Send + Sync {
    /// Returns the export format identifier
    fn format_id(&self) -> ExportFormat;

    /// Returns a human-readable name for this export format
    fn display_name(&self) -> &'static str;

    /// Exports connections to the target format
    ///
    /// # Arguments
    ///
    /// * `connections` - The connections to export
    /// * `groups` - The connection groups (for hierarchy)
    /// * `options` - Export options
    ///
    /// # Errors
    ///
    /// Returns an error if the export fails.
    fn export(
        &self,
        connections: &[Connection],
        groups: &[ConnectionGroup],
        options: &ExportOptions,
    ) -> ExportResult2<ExportResult>;

    /// Exports a single connection to a string representation
    ///
    /// # Arguments
    ///
    /// * `connection` - The connection to export
    ///
    /// # Errors
    ///
    /// Returns an error if the connection cannot be exported.
    fn export_connection(&self, connection: &Connection) -> ExportResult2<String>;

    /// Exports connections with progress reporting
    ///
    /// # Arguments
    ///
    /// * `connections` - The connections to export
    /// * `groups` - The connection groups (for hierarchy)
    /// * `options` - Export options
    /// * `progress` - Optional progress reporter
    ///
    /// # Errors
    ///
    /// Returns an error if the export fails or is cancelled.
    fn export_with_progress(
        &self,
        connections: &[Connection],
        groups: &[ConnectionGroup],
        options: &ExportOptions,
        progress: Option<&dyn ProgressReporter>,
    ) -> ExportResult2<ExportResult> {
        // Default implementation delegates to export
        if let Some(reporter) = progress {
            reporter.report(0, 1, "Starting export...");
            if reporter.is_cancelled() {
                return Err(ExportError::Cancelled);
            }
        }

        let result = self.export(connections, groups, options)?;

        if let Some(reporter) = progress {
            reporter.report(1, 1, "Export complete");
        }

        Ok(result)
    }

    /// Returns true if this exporter supports the given protocol
    fn supports_protocol(&self, protocol: &crate::models::ProtocolType) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_format_all() {
        let formats = ExportFormat::all();
        assert_eq!(formats.len(), 6);
        assert!(formats.contains(&ExportFormat::Ansible));
        assert!(formats.contains(&ExportFormat::SshConfig));
        assert!(formats.contains(&ExportFormat::Remmina));
        assert!(formats.contains(&ExportFormat::Asbru));
        assert!(formats.contains(&ExportFormat::Native));
        assert!(formats.contains(&ExportFormat::RoyalTs));
    }

    #[test]
    fn test_export_format_display_name() {
        assert_eq!(ExportFormat::Ansible.display_name(), "Ansible Inventory");
        assert_eq!(ExportFormat::SshConfig.display_name(), "SSH Config");
        assert_eq!(ExportFormat::Remmina.display_name(), "Remmina");
        assert_eq!(ExportFormat::Asbru.display_name(), "Asbru-CM");
        assert_eq!(ExportFormat::Native.display_name(), "RustConn Native");
        assert_eq!(ExportFormat::RoyalTs.display_name(), "Royal TS");
    }

    #[test]
    fn test_export_format_file_extension() {
        assert_eq!(ExportFormat::Ansible.file_extension(), "ini");
        assert_eq!(ExportFormat::SshConfig.file_extension(), "config");
        assert_eq!(ExportFormat::Remmina.file_extension(), "remmina");
        assert_eq!(ExportFormat::Asbru.file_extension(), "yml");
        assert_eq!(ExportFormat::Native.file_extension(), "rcn");
        assert_eq!(ExportFormat::RoyalTs.file_extension(), "rtsz");
    }

    #[test]
    fn test_export_format_exports_to_directory() {
        assert!(!ExportFormat::Ansible.exports_to_directory());
        assert!(!ExportFormat::SshConfig.exports_to_directory());
        assert!(ExportFormat::Remmina.exports_to_directory());
        assert!(!ExportFormat::Asbru.exports_to_directory());
        assert!(!ExportFormat::Native.exports_to_directory());
        assert!(!ExportFormat::RoyalTs.exports_to_directory());
    }

    #[test]
    fn test_export_options_new() {
        let options = ExportOptions::new(ExportFormat::Ansible, PathBuf::from("/tmp/test.ini"));
        assert_eq!(options.format, ExportFormat::Ansible);
        assert!(!options.include_passwords);
        assert!(options.include_groups);
        assert_eq!(options.output_path, PathBuf::from("/tmp/test.ini"));
    }

    #[test]
    fn test_export_options_builder() {
        let options = ExportOptions::new(ExportFormat::SshConfig, PathBuf::from("/tmp/config"))
            .with_passwords(true)
            .with_groups(false);

        assert!(options.include_passwords);
        assert!(!options.include_groups);
    }

    #[test]
    fn test_export_result_new() {
        let result = ExportResult::new();
        assert_eq!(result.exported_count, 0);
        assert_eq!(result.skipped_count, 0);
        assert!(result.warnings.is_empty());
        assert!(result.output_files.is_empty());
    }

    #[test]
    fn test_export_result_total_processed() {
        let mut result = ExportResult::new();
        result.exported_count = 5;
        result.skipped_count = 2;
        assert_eq!(result.total_processed(), 7);
    }

    #[test]
    fn test_export_result_has_skipped() {
        let mut result = ExportResult::new();
        assert!(!result.has_skipped());
        result.skipped_count = 1;
        assert!(result.has_skipped());
    }

    #[test]
    fn test_export_result_has_warnings() {
        let mut result = ExportResult::new();
        assert!(!result.has_warnings());
        result.add_warning("Test warning");
        assert!(result.has_warnings());
    }

    #[test]
    fn test_export_result_summary() {
        let mut result = ExportResult::new();
        result.exported_count = 10;
        result.skipped_count = 2;
        result.add_warning("Warning 1");
        result.add_warning("Warning 2");

        let summary = result.summary();
        assert!(summary.contains("Exported: 10"));
        assert!(summary.contains("Skipped: 2"));
        assert!(summary.contains("Warnings: 2"));
    }

    #[test]
    fn test_export_result_increment() {
        let mut result = ExportResult::new();
        result.increment_exported();
        result.increment_exported();
        result.increment_skipped();

        assert_eq!(result.exported_count, 2);
        assert_eq!(result.skipped_count, 1);
    }

    #[test]
    fn test_export_result_add_output_file() {
        let mut result = ExportResult::new();
        result.add_output_file(PathBuf::from("/tmp/file1.ini"));
        result.add_output_file(PathBuf::from("/tmp/file2.ini"));

        assert_eq!(result.output_files.len(), 2);
    }

    #[test]
    fn test_export_error_display() {
        let err = ExportError::UnsupportedProtocol("SPICE".to_string());
        assert_eq!(err.to_string(), "Unsupported protocol for export: SPICE");

        let err = ExportError::WriteError("Permission denied".to_string());
        assert_eq!(err.to_string(), "Failed to write output: Permission denied");

        let err = ExportError::InvalidData("Missing hostname".to_string());
        assert_eq!(err.to_string(), "Invalid connection data: Missing hostname");

        let err = ExportError::Cancelled;
        assert_eq!(err.to_string(), "Export cancelled");
    }
}
