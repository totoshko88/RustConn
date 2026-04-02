//! CSV exporter for connections.
//!
//! Serializes connections into RFC 4180-compliant CSV with configurable
//! delimiter and field selection.

use crate::models::{Connection, ConnectionGroup, ProtocolType};

use super::{
    ExportFormat, ExportOperationResult, ExportOptions, ExportResult, ExportTarget,
    write_export_file,
};

/// Fields that can be included in CSV export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsvExportField {
    /// Connection name
    Name,
    /// Host address
    Host,
    /// Port number
    Port,
    /// Protocol type
    Protocol,
    /// Username
    Username,
    /// Group name (resolved from group hierarchy)
    GroupName,
    /// Tags (joined with `;`)
    Tags,
    /// Description
    Description,
}

impl CsvExportField {
    /// Returns all default export fields.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Name,
            Self::Host,
            Self::Port,
            Self::Protocol,
            Self::Username,
            Self::GroupName,
            Self::Tags,
            Self::Description,
        ]
    }

    /// Returns the CSV header name for this field.
    #[must_use]
    pub const fn header(&self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Host => "host",
            Self::Port => "port",
            Self::Protocol => "protocol",
            Self::Username => "username",
            Self::GroupName => "group",
            Self::Tags => "tags",
            Self::Description => "description",
        }
    }
}

/// Options for CSV export.
#[derive(Debug, Clone)]
pub struct CsvExportOptions {
    /// Field delimiter byte (default: `,`)
    pub delimiter: u8,
    /// Fields to include in the export
    pub fields: Vec<CsvExportField>,
}

impl Default for CsvExportOptions {
    fn default() -> Self {
        Self {
            delimiter: b',',
            fields: CsvExportField::all().to_vec(),
        }
    }
}

/// CSV exporter.
pub struct CsvExporter {
    options: CsvExportOptions,
}

impl CsvExporter {
    /// Creates a new CSV exporter with default options.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            options: CsvExportOptions {
                delimiter: b',',
                fields: Vec::new(), // will use all() at export time
            },
        }
    }

    /// Creates a CSV exporter with the given options.
    #[must_use]
    pub const fn with_options(options: CsvExportOptions) -> Self {
        Self { options }
    }

    /// Returns the effective fields list (all fields if empty).
    fn effective_fields(&self) -> &[CsvExportField] {
        if self.options.fields.is_empty() {
            CsvExportField::all()
        } else {
            &self.options.fields
        }
    }

    /// Builds the full group path for a connection by walking the group hierarchy.
    fn resolve_group_name(conn: &Connection, groups: &[ConnectionGroup]) -> String {
        let Some(group_id) = conn.group_id else {
            return String::new();
        };

        let mut path_parts = Vec::new();
        let mut current_id = Some(group_id);

        while let Some(id) = current_id {
            if let Some(group) = groups.iter().find(|g| g.id == id) {
                path_parts.push(group.name.clone());
                current_id = group.parent_id;
            } else {
                break;
            }
        }

        path_parts.reverse();
        path_parts.join("/")
    }

    /// Exports connections to a CSV string.
    pub fn export_to_string(
        &self,
        connections: &[Connection],
        groups: &[ConnectionGroup],
    ) -> String {
        let delimiter = self.options.delimiter as char;
        let fields = self.effective_fields();
        let mut output = String::new();

        // Header row
        let headers: Vec<&str> = fields.iter().map(CsvExportField::header).collect();
        output.push_str(&headers.join(&delimiter.to_string()));
        output.push('\n');

        // Data rows
        for conn in connections {
            let values: Vec<String> = fields
                .iter()
                .map(|field| {
                    let raw = match field {
                        CsvExportField::Name => conn.name.clone(),
                        CsvExportField::Host => conn.host.clone(),
                        CsvExportField::Port => conn.port.to_string(),
                        CsvExportField::Protocol => conn.protocol.as_str().to_string(),
                        CsvExportField::Username => conn.username.clone().unwrap_or_default(),
                        CsvExportField::GroupName => Self::resolve_group_name(conn, groups),
                        CsvExportField::Tags => {
                            // Filter out the "imported:csv" tag for clean round-trip
                            let user_tags: Vec<&str> = conn
                                .tags
                                .iter()
                                .filter(|t| !t.starts_with("imported:"))
                                .map(String::as_str)
                                .collect();
                            user_tags.join(";")
                        }
                        CsvExportField::Description => conn.description.clone().unwrap_or_default(),
                    };
                    csv_quote(&raw, self.options.delimiter)
                })
                .collect();
            output.push_str(&values.join(&delimiter.to_string()));
            output.push('\n');
        }

        output
    }
}

impl Default for CsvExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ExportTarget for CsvExporter {
    fn format_id(&self) -> ExportFormat {
        ExportFormat::Csv
    }

    fn display_name(&self) -> &'static str {
        "CSV"
    }

    fn export(
        &self,
        connections: &[Connection],
        groups: &[ConnectionGroup],
        options: &ExportOptions,
    ) -> ExportOperationResult<ExportResult> {
        let mut result = ExportResult::new();
        let content = self.export_to_string(connections, groups);
        write_export_file(&options.output_path, &content)?;
        result.exported_count = connections.len();
        result.add_output_file(options.output_path.clone());
        Ok(result)
    }

    fn export_connection(&self, connection: &Connection) -> ExportOperationResult<String> {
        let fields = self.effective_fields();
        let delimiter = self.options.delimiter as char;
        let values: Vec<String> = fields
            .iter()
            .map(|field| {
                let raw = match field {
                    CsvExportField::Name => connection.name.clone(),
                    CsvExportField::Host => connection.host.clone(),
                    CsvExportField::Port => connection.port.to_string(),
                    CsvExportField::Protocol => connection.protocol.as_str().to_string(),
                    CsvExportField::Username => connection.username.clone().unwrap_or_default(),
                    CsvExportField::GroupName => String::new(),
                    CsvExportField::Tags => {
                        let user_tags: Vec<&str> = connection
                            .tags
                            .iter()
                            .filter(|t| !t.starts_with("imported:"))
                            .map(String::as_str)
                            .collect();
                        user_tags.join(";")
                    }
                    CsvExportField::Description => {
                        connection.description.clone().unwrap_or_default()
                    }
                };
                csv_quote(&raw, self.options.delimiter)
            })
            .collect();
        Ok(values.join(&delimiter.to_string()))
    }

    fn supports_protocol(&self, _protocol: &ProtocolType) -> bool {
        true // CSV supports all protocols
    }
}

/// Quotes a CSV field value according to RFC 4180 if it contains the delimiter,
/// double-quotes, or newlines.
fn csv_quote(value: &str, delimiter: u8) -> String {
    let delim_char = delimiter as char;
    if value.contains(delim_char)
        || value.contains('"')
        || value.contains('\n')
        || value.contains('\r')
    {
        let escaped = value.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        value.to_string()
    }
}
