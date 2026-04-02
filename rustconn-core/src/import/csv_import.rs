//! CSV file importer for connections.
//!
//! Parses CSV files with configurable delimiter and column mapping,
//! supporting automatic header detection and RFC 4180 compliance
//! via the `csv` crate.

use std::collections::HashMap;
use std::path::Path;

use uuid::Uuid;

use crate::error::ImportError;
use crate::models::{
    Connection, ConnectionGroup, KubernetesConfig, ProtocolConfig, ProtocolType, RdpConfig,
    SerialConfig, SpiceConfig, SshConfig, TelnetConfig, VncConfig, ZeroTrustConfig,
};

use super::traits::{ImportResult, ImportSource, SkippedEntry, read_import_file};

/// CSV column mapping configuration.
///
/// Maps CSV column indices to `Connection` fields.
#[derive(Debug, Clone)]
pub struct CsvColumnMapping {
    /// Column index for connection name (required)
    pub name_col: usize,
    /// Column index for host address (required)
    pub host_col: usize,
    /// Column index for port number
    pub port_col: Option<usize>,
    /// Column index for protocol type
    pub protocol_col: Option<usize>,
    /// Column index for username
    pub username_col: Option<usize>,
    /// Column index for group path
    pub group_col: Option<usize>,
    /// Column index for tags (semicolon-separated)
    pub tags_col: Option<usize>,
    /// Column index for description
    pub description_col: Option<usize>,
}

/// CSV parsing options.
#[derive(Debug, Clone)]
pub struct CsvParseOptions {
    /// Field delimiter byte (default: `,`)
    pub delimiter: u8,
    /// Whether the first row is a header
    pub has_header: bool,
    /// Explicit column mapping (auto-detected from header if `None`)
    pub mapping: Option<CsvColumnMapping>,
}

impl Default for CsvParseOptions {
    fn default() -> Self {
        Self {
            delimiter: b',',
            has_header: true,
            mapping: None,
        }
    }
}

/// Importer for CSV connection files.
pub struct CsvImporter {
    options: CsvParseOptions,
}

impl CsvImporter {
    /// Creates a new CSV importer with default options.
    #[must_use]
    pub fn new() -> Self {
        Self {
            options: CsvParseOptions::default(),
        }
    }

    /// Creates a new CSV importer with the given options.
    #[must_use]
    pub fn with_options(options: CsvParseOptions) -> Self {
        Self { options }
    }

    /// Parses CSV content into an `ImportResult`.
    pub fn parse_csv(&self, content: &str) -> ImportResult {
        let mut result = ImportResult::new();
        let mut groups: HashMap<String, Uuid> = HashMap::new();

        let mut rdr = csv::ReaderBuilder::new()
            .delimiter(self.options.delimiter)
            .has_headers(self.options.has_header)
            .flexible(true)
            .from_reader(content.as_bytes());

        let mapping = match self.resolve_mapping(&mut rdr, &mut result) {
            Some(m) => m,
            None => return result,
        };

        for (row_idx, record) in rdr.records().enumerate() {
            Self::process_record(record, row_idx, &mapping, &mut groups, &mut result);
        }

        result
    }

    /// Resolves column mapping from options or auto-detects from headers.
    fn resolve_mapping(
        &self,
        rdr: &mut csv::Reader<&[u8]>,
        result: &mut ImportResult,
    ) -> Option<CsvColumnMapping> {
        if let Some(ref m) = self.options.mapping {
            return Some(m.clone());
        }

        if self.options.has_header {
            let headers = rdr.headers().cloned().unwrap_or_default();
            if let Some(m) = auto_map_headers(&headers) {
                return Some(m);
            }
            result.add_skipped(SkippedEntry::new(
                "header",
                "Could not detect 'name' and 'host' columns from header row",
            ));
            return None;
        }

        // No header, no explicit mapping — assume positional
        Some(CsvColumnMapping {
            name_col: 0,
            host_col: 1,
            port_col: Some(2),
            protocol_col: Some(3),
            username_col: Some(4),
            group_col: Some(5),
            tags_col: Some(6),
            description_col: Some(7),
        })
    }

    /// Processes a single CSV record into a connection or skipped entry.
    fn process_record(
        record: Result<csv::StringRecord, csv::Error>,
        row_idx: usize,
        mapping: &CsvColumnMapping,
        groups: &mut HashMap<String, Uuid>,
        result: &mut ImportResult,
    ) {
        let record = match record {
            Ok(r) => r,
            Err(e) => {
                result.add_skipped(SkippedEntry::with_location(
                    format!("row {}", row_idx + 1),
                    format!("CSV parse error: {e}"),
                    format!("row {}", row_idx + 1),
                ));
                return;
            }
        };

        let get = |idx: usize| -> Option<&str> {
            record.get(idx).map(str::trim).filter(|s| !s.is_empty())
        };

        let Some(name) = get(mapping.name_col).map(String::from) else {
            result.add_skipped(SkippedEntry::with_location(
                format!("row {}", row_idx + 1),
                "Missing required field: name",
                format!("row {}", row_idx + 1),
            ));
            return;
        };

        let Some(host) = get(mapping.host_col).map(String::from) else {
            result.add_skipped(SkippedEntry::with_location(
                name,
                "Missing required field: host",
                format!("row {}", row_idx + 1),
            ));
            return;
        };

        let protocol_type = match mapping.protocol_col.and_then(&get) {
            Some(proto_str) => {
                if let Some(pt) = parse_protocol(proto_str) {
                    pt
                } else {
                    result.add_skipped(SkippedEntry::with_location(
                        name,
                        format!("Unrecognized protocol: {proto_str}"),
                        format!("row {}", row_idx + 1),
                    ));
                    return;
                }
            }
            None => ProtocolType::Ssh,
        };

        let port = mapping
            .port_col
            .and_then(&get)
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or_else(|| protocol_type.default_port());

        let mut conn = Connection::new(name, host, port, default_protocol_config(protocol_type));

        if let Some(username) = mapping.username_col.and_then(&get) {
            conn.username = Some(username.to_string());
        }
        if let Some(desc) = mapping.description_col.and_then(&get) {
            conn.description = Some(desc.to_string());
        }
        if let Some(tags_str) = mapping.tags_col.and_then(&get) {
            conn.tags = tags_str
                .split(';')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
        }
        if let Some(group_path) = mapping.group_col.and_then(get) {
            conn.group_id = Some(resolve_group_path(group_path, groups, result));
        }

        conn.tags.push("imported:csv".to_string());
        result.add_connection(conn);
    }
}

impl Default for CsvImporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportSource for CsvImporter {
    fn source_id(&self) -> &'static str {
        "csv"
    }

    fn display_name(&self) -> &'static str {
        "CSV File (.csv)"
    }

    fn is_available(&self) -> bool {
        true
    }

    fn default_paths(&self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }

    fn import(&self) -> Result<ImportResult, ImportError> {
        Err(ImportError::FileNotFound(std::path::PathBuf::from(
            "CSV importer requires a specific file path",
        )))
    }

    fn import_from_path(&self, path: &Path) -> Result<ImportResult, ImportError> {
        let content = read_import_file(path, "CSV file")?;
        Ok(self.parse_csv(&content))
    }
}

/// Attempts to auto-detect column mapping from a CSV header row.
fn auto_map_headers(headers: &csv::StringRecord) -> Option<CsvColumnMapping> {
    let mut name_col = None;
    let mut host_col = None;
    let mut port_col = None;
    let mut protocol_col = None;
    let mut username_col = None;
    let mut group_col = None;
    let mut tags_col = None;
    let mut description_col = None;

    for (i, header) in headers.iter().enumerate() {
        match header.trim().to_lowercase().as_str() {
            "name" | "connection_name" | "connection name" => name_col = Some(i),
            "host" | "hostname" | "address" | "server" | "ip" => host_col = Some(i),
            "port" => port_col = Some(i),
            "protocol" | "type" | "proto" => protocol_col = Some(i),
            "username" | "user" | "login" => username_col = Some(i),
            "group" | "folder" | "category" | "group_name" | "group name" => {
                group_col = Some(i);
            }
            "tags" | "labels" => tags_col = Some(i),
            "description" | "notes" | "comment" | "comments" => description_col = Some(i),
            _ => {}
        }
    }

    Some(CsvColumnMapping {
        name_col: name_col?,
        host_col: host_col?,
        port_col,
        protocol_col,
        username_col,
        group_col,
        tags_col,
        description_col,
    })
}

/// Parses a protocol string (case-insensitive) into a `ProtocolType`.
fn parse_protocol(s: &str) -> Option<ProtocolType> {
    match s.trim().to_lowercase().as_str() {
        "ssh" => Some(ProtocolType::Ssh),
        "rdp" => Some(ProtocolType::Rdp),
        "vnc" => Some(ProtocolType::Vnc),
        "spice" => Some(ProtocolType::Spice),
        "telnet" => Some(ProtocolType::Telnet),
        "zerotrust" | "zero trust" | "zero_trust" => Some(ProtocolType::ZeroTrust),
        "serial" => Some(ProtocolType::Serial),
        "sftp" => Some(ProtocolType::Sftp),
        "kubernetes" | "k8s" => Some(ProtocolType::Kubernetes),
        "mosh" => Some(ProtocolType::Mosh),
        _ => None,
    }
}

/// Creates a default `ProtocolConfig` for the given protocol type.
fn default_protocol_config(pt: ProtocolType) -> ProtocolConfig {
    match pt {
        ProtocolType::Ssh => ProtocolConfig::Ssh(SshConfig::default()),
        ProtocolType::Rdp => ProtocolConfig::Rdp(RdpConfig::default()),
        ProtocolType::Vnc => ProtocolConfig::Vnc(VncConfig::default()),
        ProtocolType::Spice => ProtocolConfig::Spice(SpiceConfig::default()),
        ProtocolType::Telnet => ProtocolConfig::Telnet(TelnetConfig::default()),
        ProtocolType::ZeroTrust => ProtocolConfig::ZeroTrust(ZeroTrustConfig::default()),
        ProtocolType::Serial => ProtocolConfig::Serial(SerialConfig::default()),
        ProtocolType::Sftp => ProtocolConfig::Sftp(SshConfig::default()),
        ProtocolType::Kubernetes => ProtocolConfig::Kubernetes(KubernetesConfig::default()),
        ProtocolType::Mosh => ProtocolConfig::Mosh(crate::models::MoshConfig::default()),
    }
}

/// Resolves a group path like `"Production/Web Servers"` into a group UUID,
/// creating intermediate `ConnectionGroup` entries as needed.
fn resolve_group_path(
    path: &str,
    groups: &mut HashMap<String, Uuid>,
    result: &mut ImportResult,
) -> Uuid {
    let segments: Vec<&str> = path
        .split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let mut parent_id: Option<Uuid> = None;
    let mut accumulated = String::new();

    for segment in segments {
        if !accumulated.is_empty() {
            accumulated.push('/');
        }
        accumulated.push_str(segment);

        let group_id = *groups.entry(accumulated.clone()).or_insert_with(|| {
            let group = if let Some(pid) = parent_id {
                ConnectionGroup::with_parent(segment.to_string(), pid)
            } else {
                ConnectionGroup::new(segment.to_string())
            };
            let id = group.id;
            result.add_group(group);
            id
        });

        parent_id = Some(group_id);
    }

    parent_id.expect("group path should have at least one segment")
}
