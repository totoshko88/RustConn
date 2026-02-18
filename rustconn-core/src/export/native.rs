//! Native `RustConn` export/import format (.rcn)
//!
//! This module provides functionality to export and import connections in `RustConn`'s
//! native JSON format, preserving all data including connections, groups, templates,
//! clusters, and variables.

use std::collections::HashMap;
use std::fs;
use std::io::{BufReader, BufWriter};
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::cluster::Cluster;
use crate::models::{Connection, ConnectionGroup, ConnectionTemplate, Snippet};
use crate::variables::Variable;

use super::ExportError;

/// Current version of the native export format
pub const NATIVE_FORMAT_VERSION: u32 = 2;

/// File extension for native export files
pub const NATIVE_FILE_EXTENSION: &str = "rcn";

/// Errors specific to native format import operations
#[derive(Debug, Clone, thiserror::Error)]
pub enum NativeImportError {
    /// Failed to parse JSON
    #[error("Failed to parse JSON: {0}")]
    Parse(String),

    /// Unsupported format version
    #[error("Unsupported format version: {0} (current: {NATIVE_FORMAT_VERSION})")]
    UnsupportedVersion(u32),

    /// Failed to read file
    #[error("Failed to read file: {0}")]
    FileRead(String),

    /// Migration failed
    #[error("Migration failed: {0}")]
    Migration(String),
}

/// `RustConn` native export format (.rcn)
///
/// This struct represents the complete export of a `RustConn` configuration,
/// including all connections, groups, templates, clusters, and variables.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeExport {
    /// Format version for migrations
    pub version: u32,
    /// Export timestamp
    pub exported_at: DateTime<Utc>,
    /// Application version that created export
    pub app_version: String,
    /// All connections
    pub connections: Vec<Connection>,
    /// All groups with hierarchy
    pub groups: Vec<ConnectionGroup>,
    /// All templates
    pub templates: Vec<ConnectionTemplate>,
    /// All clusters
    pub clusters: Vec<Cluster>,
    /// Global variables
    pub variables: Vec<Variable>,
    /// Snippets (added in format version 2)
    #[serde(default)]
    pub snippets: Vec<Snippet>,
    /// Custom metadata
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl NativeExport {
    /// Creates a new empty native export with current version and timestamp
    #[must_use]
    pub fn new() -> Self {
        Self {
            version: NATIVE_FORMAT_VERSION,
            exported_at: Utc::now(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            connections: Vec::new(),
            groups: Vec::new(),
            templates: Vec::new(),
            clusters: Vec::new(),
            variables: Vec::new(),
            snippets: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    /// Creates a native export with the provided data
    #[must_use]
    pub fn with_data(
        connections: Vec<Connection>,
        groups: Vec<ConnectionGroup>,
        templates: Vec<ConnectionTemplate>,
        clusters: Vec<Cluster>,
        variables: Vec<Variable>,
        snippets: Vec<Snippet>,
    ) -> Self {
        Self {
            version: NATIVE_FORMAT_VERSION,
            exported_at: Utc::now(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            connections,
            groups,
            templates,
            clusters,
            variables,
            snippets,
            metadata: HashMap::new(),
        }
    }

    /// Sets custom metadata
    #[must_use]
    pub fn with_metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.metadata = metadata;
        self
    }

    /// Adds a single metadata entry
    pub fn add_metadata(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.metadata.insert(key.into(), value.into());
    }

    /// Export to JSON string
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn to_json(&self) -> Result<String, ExportError> {
        serde_json::to_string_pretty(self).map_err(|e| ExportError::Serialization(e.to_string()))
    }

    /// Export to JSON string (compact, no pretty printing)
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn to_json_compact(&self) -> Result<String, ExportError> {
        serde_json::to_string(self).map_err(|e| ExportError::Serialization(e.to_string()))
    }

    /// Export to a file using buffered I/O
    ///
    /// Streams JSON directly to a `BufWriter` instead of building an intermediate
    /// `String`, reducing peak memory usage for large exports.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or file writing fails.
    pub fn to_file(&self, path: &Path) -> Result<(), ExportError> {
        let file = fs::File::create(path).map_err(|e| {
            ExportError::WriteError(format!("Failed to create {}: {}", path.display(), e))
        })?;
        let writer = BufWriter::new(file);
        serde_json::to_writer_pretty(writer, self)
            .map_err(|e| ExportError::Serialization(e.to_string()))
    }

    /// Import from JSON string with version validation
    ///
    /// Performs a lightweight version check before full deserialization to avoid
    /// parsing large files with unsupported format versions.
    ///
    /// # Errors
    ///
    /// Returns an error if parsing fails or the version is unsupported.
    pub fn from_json(json: &str) -> Result<Self, NativeImportError> {
        // Quick version pre-check before full deserialization
        #[derive(Deserialize)]
        struct VersionCheck {
            version: u32,
        }
        let check: VersionCheck =
            serde_json::from_str(json).map_err(|e| NativeImportError::Parse(e.to_string()))?;
        if check.version > NATIVE_FORMAT_VERSION {
            return Err(NativeImportError::UnsupportedVersion(check.version));
        }

        let export: Self =
            serde_json::from_str(json).map_err(|e| NativeImportError::Parse(e.to_string()))?;

        // Apply migrations if needed
        Ok(Self::migrate(export))
    }

    /// Import from a file using buffered I/O
    ///
    /// Streams JSON from a `BufReader` instead of reading the entire file into
    /// a `String`, reducing peak memory usage by ~50% for large imports.
    ///
    /// # Errors
    ///
    /// Returns an error if file reading, parsing, or version validation fails.
    pub fn from_file(path: &Path) -> Result<Self, NativeImportError> {
        let file = fs::File::open(path)
            .map_err(|e| NativeImportError::FileRead(format!("{}: {}", path.display(), e)))?;
        let reader = BufReader::new(file);
        let export: Self =
            serde_json::from_reader(reader).map_err(|e| NativeImportError::Parse(e.to_string()))?;

        if export.version > NATIVE_FORMAT_VERSION {
            return Err(NativeImportError::UnsupportedVersion(export.version));
        }

        Ok(Self::migrate(export))
    }

    /// Migrate export data from older versions to current version
    ///
    /// This function handles forward compatibility by applying necessary
    /// transformations to data from older format versions.
    const fn migrate(mut export: Self) -> Self {
        // v1 → v2: snippets field added with #[serde(default)], no data migration needed

        // Update version to current after migration
        export.version = NATIVE_FORMAT_VERSION;
        export
    }

    /// Returns the total number of items in this export
    #[must_use]
    pub fn total_items(&self) -> usize {
        self.connections.len()
            + self.groups.len()
            + self.templates.len()
            + self.clusters.len()
            + self.variables.len()
            + self.snippets.len()
    }

    /// Returns true if this export contains no data
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.connections.is_empty()
            && self.groups.is_empty()
            && self.templates.is_empty()
            && self.clusters.is_empty()
            && self.variables.is_empty()
            && self.snippets.is_empty()
    }

    /// Returns a summary of the export contents
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "Connections: {}, Groups: {}, Templates: {}, Clusters: {}, \
             Variables: {}, Snippets: {}",
            self.connections.len(),
            self.groups.len(),
            self.templates.len(),
            self.clusters.len(),
            self.variables.len(),
            self.snippets.len()
        )
    }
}

impl Default for NativeExport {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_native_export_new() {
        let export = NativeExport::new();
        assert_eq!(export.version, NATIVE_FORMAT_VERSION);
        assert!(export.connections.is_empty());
        assert!(export.groups.is_empty());
        assert!(export.templates.is_empty());
        assert!(export.clusters.is_empty());
        assert!(export.variables.is_empty());
        assert!(export.metadata.is_empty());
        assert!(export.is_empty());
    }

    #[test]
    fn test_native_export_with_data() {
        let conn = Connection::new_ssh("Test".to_string(), "host.com".to_string(), 22);
        let group = ConnectionGroup::new("Group".to_string());
        let template = ConnectionTemplate::new_ssh("Template".to_string());
        let cluster = Cluster::new("Cluster".to_string());
        let variable = Variable::new("var", "value");

        let export = NativeExport::with_data(
            vec![conn],
            vec![group],
            vec![template],
            vec![cluster],
            vec![variable],
            Vec::new(),
        );

        assert_eq!(export.connections.len(), 1);
        assert_eq!(export.groups.len(), 1);
        assert_eq!(export.templates.len(), 1);
        assert_eq!(export.clusters.len(), 1);
        assert_eq!(export.variables.len(), 1);
        assert_eq!(export.total_items(), 5);
        assert!(!export.is_empty());
    }

    #[test]
    fn test_native_export_metadata() {
        let mut export = NativeExport::new();
        export.add_metadata("author", "test_user");
        export.add_metadata("description", "Test export");

        assert_eq!(
            export.metadata.get("author"),
            Some(&"test_user".to_string())
        );
        assert_eq!(
            export.metadata.get("description"),
            Some(&"Test export".to_string())
        );
    }

    #[test]
    fn test_native_export_with_metadata_builder() {
        let mut metadata = HashMap::new();
        metadata.insert("key".to_string(), "value".to_string());

        let export = NativeExport::new().with_metadata(metadata);
        assert_eq!(export.metadata.get("key"), Some(&"value".to_string()));
    }

    #[test]
    fn test_native_export_to_json() {
        let export = NativeExport::new();
        let json = export.to_json().unwrap();

        assert!(json.contains("\"version\""));
        assert!(json.contains(&format!("{NATIVE_FORMAT_VERSION}")));
        assert!(json.contains("\"exported_at\""));
        assert!(json.contains("\"connections\""));
        assert!(json.contains("\"groups\""));
        assert!(json.contains("\"templates\""));
        assert!(json.contains("\"clusters\""));
        assert!(json.contains("\"variables\""));
    }

    #[test]
    fn test_native_export_from_json() {
        let export = NativeExport::new();
        let json = export.to_json().unwrap();
        let imported = NativeExport::from_json(&json).unwrap();

        assert_eq!(imported.version, export.version);
        assert_eq!(imported.connections.len(), export.connections.len());
        assert_eq!(imported.groups.len(), export.groups.len());
    }

    #[test]
    fn test_native_export_unsupported_version() {
        let json = r#"{
            "version": 999,
            "exported_at": "2024-01-01T00:00:00Z",
            "app_version": "0.1.0",
            "connections": [],
            "groups": [],
            "templates": [],
            "clusters": [],
            "variables": []
        }"#;

        let result = NativeExport::from_json(json);
        assert!(matches!(
            result,
            Err(NativeImportError::UnsupportedVersion(999))
        ));
    }

    #[test]
    fn test_native_export_invalid_json() {
        let result = NativeExport::from_json("not valid json");
        assert!(matches!(result, Err(NativeImportError::Parse(_))));
    }

    #[test]
    fn test_native_export_summary() {
        let export = NativeExport::with_data(
            vec![Connection::new_ssh(
                "Test".to_string(),
                "host.com".to_string(),
                22,
            )],
            vec![ConnectionGroup::new("Group".to_string())],
            vec![],
            vec![],
            vec![],
            vec![],
        );

        let summary = export.summary();
        assert!(summary.contains("Connections: 1"));
        assert!(summary.contains("Groups: 1"));
        assert!(summary.contains("Templates: 0"));
    }

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn test_native_format_version_constant() {
        assert!(NATIVE_FORMAT_VERSION >= 1);
    }

    #[test]
    fn test_native_file_extension() {
        assert_eq!(NATIVE_FILE_EXTENSION, "rcn");
    }
}
