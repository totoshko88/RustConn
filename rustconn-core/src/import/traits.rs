//! Import source trait and result types.
//!
//! This module defines the core abstractions for the import engine,
//! allowing different import sources to be implemented uniformly.

use std::fs;
use std::path::Path;

use crate::error::ImportError;
use crate::models::{Connection, ConnectionGroup};
use crate::progress::ProgressReporter;

/// Reads a file for import operations with consistent error handling.
///
/// This helper consolidates the duplicated file I/O pattern across all importers,
/// providing uniform error messages and reducing code duplication.
///
/// # Arguments
/// * `path` - Path to the file to read
/// * `source_name` - Human-readable name of the import source (e.g., "SSH config")
///
/// # Errors
/// Returns `ImportError::ParseError` if the file cannot be read.
///
/// # Example
/// ```ignore
/// let content = read_import_file(path, "SSH config")?;
/// ```
pub fn read_import_file(path: &Path, source_name: &str) -> Result<String, ImportError> {
    fs::read_to_string(path).map_err(|e| ImportError::ParseError {
        source_name: source_name.to_string(),
        reason: format!("Failed to read {}: {}", path.display(), e),
    })
}

/// Async variant of `read_import_file` for future use.
///
/// Uses tokio's async file I/O for non-blocking reads.
/// Currently unused but prepared for async import migration.
///
/// # Arguments
/// * `path` - Path to the file to read
/// * `source_name` - Human-readable name of the import source
///
/// # Errors
/// Returns `ImportError::ParseError` if the file cannot be read.
#[allow(dead_code)] // Prepared for future async import implementation
pub async fn read_import_file_async(path: &Path, source_name: &str) -> Result<String, ImportError> {
    tokio::fs::read_to_string(path)
        .await
        .map_err(|e| ImportError::ParseError {
            source_name: source_name.to_string(),
            reason: format!("Failed to read {}: {}", path.display(), e),
        })
}

/// Result of an import operation containing successful imports and any issues encountered.
#[derive(Debug, Default)]
pub struct ImportResult {
    /// Successfully imported connections
    pub connections: Vec<Connection>,
    /// Successfully imported or created groups
    pub groups: Vec<ConnectionGroup>,
    /// Entries that were skipped (invalid but non-fatal)
    pub skipped: Vec<SkippedEntry>,
    /// Errors encountered during import
    pub errors: Vec<ImportError>,
}

impl ImportResult {
    /// Creates a new empty import result
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the total number of entries processed
    #[must_use]
    pub fn total_processed(&self) -> usize {
        self.connections.len() + self.skipped.len() + self.errors.len()
    }

    /// Returns true if the import had any errors
    #[must_use]
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Returns true if any entries were skipped
    #[must_use]
    pub fn has_skipped(&self) -> bool {
        !self.skipped.is_empty()
    }

    /// Returns a summary string of the import result
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "Imported: {}, Groups: {}, Skipped: {}, Errors: {}",
            self.connections.len(),
            self.groups.len(),
            self.skipped.len(),
            self.errors.len()
        )
    }

    /// Adds a connection to the result
    pub fn add_connection(&mut self, connection: Connection) {
        self.connections.push(connection);
    }

    /// Adds a group to the result
    pub fn add_group(&mut self, group: ConnectionGroup) {
        self.groups.push(group);
    }

    /// Adds a skipped entry to the result
    pub fn add_skipped(&mut self, entry: SkippedEntry) {
        self.skipped.push(entry);
    }

    /// Adds an error to the result
    pub fn add_error(&mut self, error: ImportError) {
        self.errors.push(error);
    }

    /// Merges another import result into this one
    pub fn merge(&mut self, other: Self) {
        self.connections.extend(other.connections);
        self.groups.extend(other.groups);
        self.skipped.extend(other.skipped);
        self.errors.extend(other.errors);
    }
}

/// An entry that was skipped during import
#[derive(Debug, Clone)]
pub struct SkippedEntry {
    /// Identifier or name of the skipped entry
    pub identifier: String,
    /// Reason why the entry was skipped
    pub reason: String,
    /// Source location (file path, line number, etc.)
    pub location: Option<String>,
}

impl SkippedEntry {
    /// Creates a new skipped entry
    #[must_use]
    pub fn new(identifier: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            identifier: identifier.into(),
            reason: reason.into(),
            location: None,
        }
    }

    /// Creates a new skipped entry with location information
    #[must_use]
    pub fn with_location(
        identifier: impl Into<String>,
        reason: impl Into<String>,
        location: impl Into<String>,
    ) -> Self {
        Self {
            identifier: identifier.into(),
            reason: reason.into(),
            location: Some(location.into()),
        }
    }
}

/// Information about a field that was not imported
#[derive(Debug, Clone)]
pub struct SkippedField {
    /// Name of the connection this field belongs to
    pub connection_name: String,
    /// Name of the field that was skipped
    pub field_name: String,
    /// Original value that couldn't be imported
    pub original_value: Option<String>,
    /// Reason why the field was skipped
    pub reason: SkippedFieldReason,
}

/// Reason why a field was skipped during import
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkippedFieldReason {
    /// Field is not supported by the target protocol
    NotSupported,
    /// Field value is invalid or malformed
    InvalidValue,
    /// Field is deprecated in the source format
    Deprecated,
    /// Field requires a feature that is not available
    FeatureUnavailable,
    /// Field is intentionally ignored (e.g., UI-only settings)
    Ignored,
    /// Unknown field in source format
    Unknown,
}

impl SkippedFieldReason {
    /// Returns a human-readable description of the reason
    #[must_use]
    pub const fn description(&self) -> &'static str {
        match self {
            Self::NotSupported => "Field not supported by target protocol",
            Self::InvalidValue => "Invalid or malformed value",
            Self::Deprecated => "Deprecated field in source format",
            Self::FeatureUnavailable => "Required feature not available",
            Self::Ignored => "Field intentionally ignored",
            Self::Unknown => "Unknown field in source format",
        }
    }
}

impl SkippedField {
    /// Creates a new skipped field entry
    #[must_use]
    pub fn new(
        connection_name: impl Into<String>,
        field_name: impl Into<String>,
        reason: SkippedFieldReason,
    ) -> Self {
        Self {
            connection_name: connection_name.into(),
            field_name: field_name.into(),
            original_value: None,
            reason,
        }
    }

    /// Creates a skipped field with the original value
    #[must_use]
    pub fn with_value(
        connection_name: impl Into<String>,
        field_name: impl Into<String>,
        value: impl Into<String>,
        reason: SkippedFieldReason,
    ) -> Self {
        Self {
            connection_name: connection_name.into(),
            field_name: field_name.into(),
            original_value: Some(value.into()),
            reason,
        }
    }
}

/// Detailed import statistics with field-level tracking
#[derive(Debug, Clone, Default)]
pub struct ImportStatistics {
    /// Total connections processed
    pub total_connections: usize,
    /// Successfully imported connections
    pub imported_connections: usize,
    /// Connections that failed to import
    pub failed_connections: usize,
    /// Total groups processed
    pub total_groups: usize,
    /// Successfully imported groups
    pub imported_groups: usize,
    /// Fields that were skipped during import
    pub skipped_fields: Vec<SkippedField>,
    /// Warnings generated during import
    pub warnings: Vec<String>,
}

impl ImportStatistics {
    /// Creates new empty statistics
    #[must_use]
    pub const fn new() -> Self {
        Self {
            total_connections: 0,
            imported_connections: 0,
            failed_connections: 0,
            total_groups: 0,
            imported_groups: 0,
            skipped_fields: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Records a successful connection import
    pub fn record_connection_success(&mut self) {
        self.total_connections += 1;
        self.imported_connections += 1;
    }

    /// Records a failed connection import
    pub fn record_connection_failure(&mut self) {
        self.total_connections += 1;
        self.failed_connections += 1;
    }

    /// Records a successful group import
    pub fn record_group_success(&mut self) {
        self.total_groups += 1;
        self.imported_groups += 1;
    }

    /// Records a skipped field
    pub fn record_skipped_field(&mut self, field: SkippedField) {
        self.skipped_fields.push(field);
    }

    /// Records a warning
    pub fn record_warning(&mut self, warning: impl Into<String>) {
        self.warnings.push(warning.into());
    }

    /// Returns true if any fields were skipped
    #[must_use]
    pub fn has_skipped_fields(&self) -> bool {
        !self.skipped_fields.is_empty()
    }

    /// Returns true if any warnings were generated
    #[must_use]
    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }

    /// Returns the success rate as a percentage
    #[must_use]
    pub fn success_rate(&self) -> f64 {
        if self.total_connections == 0 {
            100.0
        } else {
            (self.imported_connections as f64 / self.total_connections as f64) * 100.0
        }
    }

    /// Returns a summary of skipped fields grouped by reason
    #[must_use]
    pub fn skipped_fields_summary(&self) -> std::collections::HashMap<SkippedFieldReason, usize> {
        let mut summary = std::collections::HashMap::new();
        for field in &self.skipped_fields {
            *summary.entry(field.reason).or_insert(0) += 1;
        }
        summary
    }

    /// Returns skipped fields for a specific connection
    #[must_use]
    pub fn skipped_fields_for_connection(&self, connection_name: &str) -> Vec<&SkippedField> {
        self.skipped_fields
            .iter()
            .filter(|f| f.connection_name == connection_name)
            .collect()
    }

    /// Generates a detailed report of the import
    #[must_use]
    pub fn detailed_report(&self) -> String {
        use std::fmt::Write;

        let mut report = String::new();

        let _ = write!(
            report,
            "Import Statistics:\n\
             - Connections: {}/{} imported ({:.1}% success)\n\
             - Groups: {}/{} imported\n\
             - Skipped fields: {}\n\
             - Warnings: {}\n",
            self.imported_connections,
            self.total_connections,
            self.success_rate(),
            self.imported_groups,
            self.total_groups,
            self.skipped_fields.len(),
            self.warnings.len()
        );

        if !self.skipped_fields.is_empty() {
            report.push_str("\nSkipped Fields:\n");
            for (reason, count) in self.skipped_fields_summary() {
                let _ = writeln!(report, "  - {}: {count}", reason.description());
            }
        }

        if !self.warnings.is_empty() {
            report.push_str("\nWarnings:\n");
            for warning in &self.warnings {
                let _ = writeln!(report, "  - {warning}");
            }
        }

        report
    }
}

/// Trait for import source implementations.
///
/// Each import source (SSH config, Asbru-CM, Remmina, Ansible) implements
/// this trait to provide a uniform interface for importing connections.
pub trait ImportSource: Send + Sync {
    /// Returns the unique identifier for this import source
    fn source_id(&self) -> &'static str;

    /// Returns a human-readable name for this import source
    fn display_name(&self) -> &'static str;

    /// Checks if this import source is available (e.g., config files exist)
    fn is_available(&self) -> bool;

    /// Returns the default paths where this source looks for configuration
    fn default_paths(&self) -> Vec<std::path::PathBuf>;

    /// Imports connections from the source
    ///
    /// # Errors
    ///
    /// Returns an error if the import fails completely (e.g., file not found).
    /// Partial failures (invalid entries) are recorded in the `ImportResult`.
    fn import(&self) -> Result<ImportResult, ImportError>;

    /// Imports connections from a specific path
    ///
    /// # Errors
    ///
    /// Returns an error if the import fails completely.
    fn import_from_path(&self, path: &std::path::Path) -> Result<ImportResult, ImportError>;

    /// Imports connections from a specific path with progress reporting.
    ///
    /// This method allows callers to receive progress updates during the import
    /// and optionally cancel the operation.
    ///
    /// # Arguments
    ///
    /// * `path` - The path to import from
    /// * `progress` - Optional progress reporter for receiving updates
    ///
    /// # Errors
    ///
    /// Returns an error if the import fails completely or is cancelled.
    fn import_from_path_with_progress(
        &self,
        path: &std::path::Path,
        progress: Option<&dyn ProgressReporter>,
    ) -> Result<ImportResult, ImportError> {
        // Default implementation delegates to import_from_path
        // Subclasses can override for actual progress reporting
        if let Some(reporter) = progress {
            reporter.report(0, 1, "Starting import...");
            if reporter.is_cancelled() {
                return Err(ImportError::Cancelled);
            }
        }

        let result = self.import_from_path(path)?;

        if let Some(reporter) = progress {
            reporter.report(1, 1, "Import complete");
        }

        Ok(result)
    }

    /// Imports connections from the source with progress reporting.
    ///
    /// # Arguments
    ///
    /// * `progress` - Optional progress reporter for receiving updates
    ///
    /// # Errors
    ///
    /// Returns an error if the import fails completely or is cancelled.
    fn import_with_progress(
        &self,
        progress: Option<&dyn ProgressReporter>,
    ) -> Result<ImportResult, ImportError> {
        let paths = self.default_paths();

        if paths.is_empty() {
            return Err(ImportError::FileNotFound(std::path::PathBuf::from(
                "No default paths found",
            )));
        }

        let total = paths.len();
        let mut combined_result = ImportResult::new();

        for (index, path) in paths.iter().enumerate() {
            if let Some(reporter) = progress {
                reporter.report(index, total, &format!("Importing from {}", path.display()));
                if reporter.is_cancelled() {
                    return Err(ImportError::Cancelled);
                }
            }

            match self.import_from_path(path) {
                Ok(result) => combined_result.merge(result),
                Err(e) => combined_result.add_error(e),
            }
        }

        if let Some(reporter) = progress {
            reporter.report(total, total, "Import complete");
        }

        Ok(combined_result)
    }
}
