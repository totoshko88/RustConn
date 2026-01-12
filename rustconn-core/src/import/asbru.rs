//! Asbru-CM configuration importer.
//!
//! Parses Asbru-CM YAML configuration files from ~/.config/pac/ or ~/.config/asbru/

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use uuid::Uuid;

use crate::error::ImportError;
use crate::models::{
    Connection, ConnectionGroup, ProtocolConfig, RdpConfig, SshAuthMethod, SshConfig, SshKeySource,
    VncConfig,
};

use super::traits::{ImportResult, ImportSource, SkippedEntry};

/// Importer for Asbru-CM configuration files.
///
/// Asbru-CM stores connections in YAML format, typically in
/// ~/.config/pac/ (legacy) or ~/.config/asbru/
pub struct AsbruImporter {
    /// Custom paths to search for Asbru config
    custom_paths: Vec<PathBuf>,
}

/// Asbru-CM entry from YAML (flat format with UUID keys)
/// This handles the actual Asbru-CM export format where entries are flat
/// with `_is_group` field to distinguish groups from connections
#[derive(Debug, Deserialize)]
struct AsbruEntry {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    ip: Option<String>,
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default, rename = "type")]
    protocol_type: Option<String>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    auth_type: Option<String>,
    #[serde(default, rename = "public key")]
    public_key: Option<String>,
    #[serde(default)]
    options: Option<String>,
    #[serde(default)]
    description: Option<String>,
    /// 0 = connection, 1 = group
    #[serde(default, rename = "_is_group")]
    is_group: Option<i32>,
    /// Parent UUID for hierarchy
    #[serde(default)]
    parent: Option<String>,
    /// Children (usually empty `HashMap` for connections)
    ///
    /// Parsed from YAML for structural completeness but not currently used
    /// for nested group import. Asbru-CM stores children as a flat structure
    /// with parent references rather than nested objects. This field must be
    /// present to correctly deserialize the YAML structure.
    #[serde(default)]
    #[allow(dead_code)] // Required for YAML deserialization completeness
    children: Option<HashMap<String, serde_yaml::Value>>,
}

impl AsbruImporter {
    /// Creates a new Asbru-CM importer with default paths
    #[must_use]
    pub const fn new() -> Self {
        Self {
            custom_paths: Vec::new(),
        }
    }

    /// Creates a new Asbru-CM importer with custom paths
    #[must_use]
    pub const fn with_paths(paths: Vec<PathBuf>) -> Self {
        Self {
            custom_paths: paths,
        }
    }

    /// Extracts hostname from an Asbru entry using fallback chain.
    ///
    /// Tries fields in order: ip → host → name → title
    /// Filters out "tmp", empty values, and placeholder names.
    /// For name/title fields, only extracts if they look like hostnames
    /// (contain dots, are IP addresses, or contain dynamic variables).
    #[allow(clippy::unused_self)]
    fn extract_hostname(&self, entry: &AsbruEntry) -> Option<String> {
        // Try ip field first
        if let Some(ip) = &entry.ip {
            if Self::is_valid_hostname(ip) {
                return Some(ip.clone());
            }
        }

        // Try host field
        if let Some(host) = &entry.host {
            if Self::is_valid_hostname(host) {
                return Some(host.clone());
            }
        }

        // Try extracting from name field if it looks like a hostname
        if let Some(name) = &entry.name {
            if Self::looks_like_hostname(name) {
                return Some(name.clone());
            }
        }

        // Try extracting from title field if it looks like a hostname
        if let Some(title) = &entry.title {
            if Self::looks_like_hostname(title) {
                return Some(title.clone());
            }
        }

        None
    }

    /// Checks if a string is a valid hostname (not empty, not "tmp", not placeholder)
    fn is_valid_hostname(s: &str) -> bool {
        let trimmed = s.trim();
        !trimmed.is_empty()
            && trimmed.to_lowercase() != "tmp"
            && !trimmed.eq_ignore_ascii_case("placeholder")
            && !trimmed.eq_ignore_ascii_case("none")
    }

    /// Checks if a string looks like a hostname (contains dots, is IP-like, or has variables)
    fn looks_like_hostname(s: &str) -> bool {
        let trimmed = s.trim();
        if trimmed.is_empty() || trimmed.to_lowercase() == "tmp" {
            return false;
        }

        // Contains dynamic variable syntax - preserve as-is
        if Self::contains_dynamic_variable(trimmed) {
            return true;
        }

        // Contains dots (like a FQDN)
        if trimmed.contains('.') {
            return true;
        }

        // Looks like an IP address
        if trimmed.parse::<std::net::IpAddr>().is_ok() {
            return true;
        }

        false
    }

    /// Checks if a string contains dynamic variable syntax (${VAR} or $VAR)
    fn contains_dynamic_variable(s: &str) -> bool {
        s.contains("${") || (s.contains('$') && s.chars().any(|c| c.is_ascii_alphabetic()))
    }

    /// Parses Asbru YAML content and returns an import result
    #[must_use]
    pub fn parse_config(&self, content: &str, source_path: &str) -> ImportResult {
        let mut result = ImportResult::new();

        // First parse as generic YAML to handle special keys like __PAC__EXPORTED__FULL__
        let raw_config: HashMap<String, serde_yaml::Value> = match serde_yaml::from_str(content) {
            Ok(c) => c,
            Err(e) => {
                result.add_error(ImportError::ParseError {
                    source_name: "Asbru-CM".to_string(),
                    reason: format!("Failed to parse YAML: {e}"),
                });
                return result;
            }
        };

        // Filter out special keys and parse entries
        // Asbru-CM stores connections in two possible locations:
        // 1. Top-level (exported files) - entries are at root level
        // 2. Inside "environments" key (installed Asbru config)
        let mut config: HashMap<String, AsbruEntry> = HashMap::new();

        // Check if this is an installed Asbru config with "environments" key
        if let Some(environments) = raw_config.get("environments") {
            if let Some(env_map) = environments.as_mapping() {
                for (key, value) in env_map {
                    if let Some(key_str) = key.as_str() {
                        // Skip special keys
                        if key_str.starts_with("__") {
                            continue;
                        }
                        // Try to deserialize as AsbruEntry
                        if let Ok(entry) = serde_yaml::from_value(value.clone()) {
                            config.insert(key_str.to_string(), entry);
                        }
                    }
                }
            }
        }

        // Also check top-level entries (for exported files)
        for (key, value) in &raw_config {
            // Skip special Asbru metadata keys
            if key.starts_with("__")
                || key == "defaults"
                || key == "environments"
                || key.starts_with("config ")
            {
                continue;
            }

            // Try to deserialize as AsbruEntry
            if let Ok(entry) = serde_yaml::from_value(value.clone()) {
                config.insert(key.clone(), entry);
            }
            // Skip entries that don't match the expected structure
        }

        // Build parent-child relationships
        // First pass: create groups and map original UUIDs to new UUIDs
        let mut uuid_map: HashMap<String, Uuid> = HashMap::new();

        // Process groups first - collect them to set parent_id after all are created
        let mut groups_to_add: Vec<(String, ConnectionGroup, Option<String>)> = Vec::new();

        for (key, entry) in &config {
            if entry.is_group == Some(1) {
                let group_name = entry
                    .name
                    .as_ref()
                    .or(entry.title.as_ref())
                    .cloned()
                    .unwrap_or_else(|| key.clone());

                let group = ConnectionGroup::new(group_name);
                uuid_map.insert(key.clone(), group.id);
                groups_to_add.push((key.clone(), group, entry.parent.clone()));
            }
        }

        // Second pass: set parent_id for groups and add them to result
        for (_key, mut group, parent_key) in groups_to_add {
            if let Some(parent_key) = parent_key {
                if let Some(&parent_uuid) = uuid_map.get(&parent_key) {
                    group.parent_id = Some(parent_uuid);
                }
            }
            result.add_group(group);
        }

        // Second pass: process connections
        for (key, entry) in &config {
            if entry.is_group != Some(1) {
                if let Some(connection) =
                    self.convert_entry(key, entry, &uuid_map, source_path, &mut result)
                {
                    result.add_connection(connection);
                }
            }
        }

        result
    }

    /// Converts an Asbru entry to a Connection
    #[allow(clippy::too_many_lines)]
    fn convert_entry(
        &self,
        key: &str,
        entry: &AsbruEntry,
        uuid_map: &HashMap<String, Uuid>,
        source_path: &str,
        result: &mut ImportResult,
    ) -> Option<Connection> {
        // Get connection name
        let name = entry
            .name
            .as_ref()
            .or(entry.title.as_ref())
            .cloned()
            .unwrap_or_else(|| key.to_string());

        // Get hostname using fallback chain: ip → host → name → title
        let Some(host) = self.extract_hostname(entry) else {
            result.add_skipped(SkippedEntry::with_location(
                &name,
                "No hostname specified in ip, host, name, or title fields",
                source_path,
            ));
            return None;
        };

        // Determine protocol and create config
        // Asbru uses 'method' field primarily, 'type' as fallback
        let protocol_type = entry
            .method
            .as_ref()
            .or(entry.protocol_type.as_ref())
            .map_or_else(|| "ssh".to_string(), |s| s.to_lowercase());

        // Log protocol detection for debugging
        #[cfg(debug_assertions)]
        eprintln!(
            "Asbru import: {} - method={:?}, type={:?}, resolved={}",
            name, entry.method, entry.protocol_type, protocol_type
        );

        let (protocol_config, default_port) = match protocol_type.as_str() {
            "ssh" | "sftp" | "scp" => {
                let auth_method = match entry.auth_type.as_deref() {
                    Some("publickey" | "key") => SshAuthMethod::PublicKey,
                    Some("keyboard-interactive") => SshAuthMethod::KeyboardInteractive,
                    Some("agent") => SshAuthMethod::Agent,
                    _ => SshAuthMethod::Password,
                };

                let key_path = entry
                    .public_key
                    .as_ref()
                    .filter(|p| !p.is_empty())
                    .map(|p| PathBuf::from(shellexpand::tilde(p).into_owned()));

                // Parse custom options from the options field
                let mut custom_options = HashMap::new();
                if let Some(opts) = &entry.options {
                    // Parse options like "-x -C -o \"PubkeyAuthentication=no\""
                    for part in opts.split_whitespace() {
                        if part.starts_with("-o") {
                            continue;
                        }
                        if part.contains('=') {
                            let clean = part.trim_matches('"');
                            if let Some((k, v)) = clean.split_once('=') {
                                custom_options.insert(k.to_string(), v.to_string());
                            }
                        }
                    }
                }

                (
                    ProtocolConfig::Ssh(SshConfig {
                        auth_method,
                        key_path,
                        key_source: SshKeySource::Default,
                        agent_key_fingerprint: None,
                        identities_only: false,
                        proxy_jump: None,
                        use_control_master: false,
                        agent_forwarding: false,
                        custom_options,
                        startup_command: None,
                    }),
                    22u16,
                )
            }
            // Match RDP protocols - Asbru stores as "rdp (rdesktop)", "rdp (xfreerdp)", etc.
            s if s == "rdp"
                || s.starts_with("rdp ")
                || s.starts_with("rdp(")
                || s == "rdesktop"
                || s == "xfreerdp"
                || s == "freerdp"
                || s == "rdesktop3" =>
            {
                (ProtocolConfig::Rdp(RdpConfig::default()), 3389u16)
            }
            // Match VNC protocols - Asbru stores as "vnc (vncviewer)", "vnc (tigervnc)", etc.
            s if s == "vnc"
                || s.starts_with("vnc ")
                || s.starts_with("vnc(")
                || s == "vncviewer"
                || s == "tigervnc"
                || s == "realvnc" =>
            {
                (ProtocolConfig::Vnc(VncConfig::default()), 5900u16)
            }
            _ => {
                result.add_skipped(SkippedEntry::with_location(
                    &name,
                    format!("Unsupported protocol: {protocol_type}"),
                    source_path,
                ));
                return None;
            }
        };

        let port = entry.port.unwrap_or(default_port);

        let mut connection = Connection::new(name, host, port, protocol_config);

        if let Some(user) = &entry.user {
            connection.username = Some(user.clone());
        }

        // Set parent group if exists
        if let Some(parent_uuid) = &entry.parent {
            if let Some(&group_id) = uuid_map.get(parent_uuid) {
                connection.group_id = Some(group_id);
            }
        }

        // Set description if present
        if let Some(desc) = &entry.description {
            if !desc.is_empty() {
                connection.description = Some(desc.clone());
            }
        }

        Some(connection)
    }

    /// Finds the Asbru config file in a directory
    #[allow(clippy::unused_self)]
    fn find_config_file(&self, dir: &Path) -> Option<PathBuf> {
        // Asbru stores connections in various files
        let possible_files = [
            "pac.yml",
            "pac.yaml",
            "asbru.yml",
            "asbru.yaml",
            "connections.yml",
        ];

        for filename in &possible_files {
            let path = dir.join(filename);
            if path.exists() {
                return Some(path);
            }
        }

        None
    }
}

impl Default for AsbruImporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportSource for AsbruImporter {
    fn source_id(&self) -> &'static str {
        "asbru"
    }

    fn display_name(&self) -> &'static str {
        "Asbru-CM"
    }

    fn is_available(&self) -> bool {
        self.default_paths().iter().any(|p| p.exists())
    }

    fn default_paths(&self) -> Vec<PathBuf> {
        if !self.custom_paths.is_empty() {
            return self.custom_paths.clone();
        }

        let mut paths = Vec::new();

        if let Some(config_dir) = dirs::config_dir() {
            // Check ~/.config/asbru/
            let asbru_dir = config_dir.join("asbru");
            if let Some(config_file) = self.find_config_file(&asbru_dir) {
                paths.push(config_file);
            }

            // Check ~/.config/pac/ (legacy)
            let pac_dir = config_dir.join("pac");
            if let Some(config_file) = self.find_config_file(&pac_dir) {
                paths.push(config_file);
            }
        }

        paths
    }

    fn import(&self) -> Result<ImportResult, ImportError> {
        let paths = self.default_paths();

        if paths.is_empty() {
            return Err(ImportError::FileNotFound(PathBuf::from("~/.config/asbru/")));
        }

        let mut combined_result = ImportResult::new();

        for path in paths {
            match self.import_from_path(&path) {
                Ok(result) => combined_result.merge(result),
                Err(e) => combined_result.add_error(e),
            }
        }

        Ok(combined_result)
    }

    fn import_from_path(&self, path: &Path) -> Result<ImportResult, ImportError> {
        if !path.exists() {
            return Err(ImportError::FileNotFound(path.to_path_buf()));
        }

        let content = fs::read_to_string(path).map_err(|e| ImportError::ParseError {
            source_name: "Asbru-CM".to_string(),
            reason: format!("Failed to read {}: {}", path.display(), e),
        })?;

        Ok(self.parse_config(&content, &path.display().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_connection() {
        let importer = AsbruImporter::new();
        // Real Asbru format with UUID keys
        let yaml = r#"
00c67275-e7bb-4e65-98a3-14e29b0e4258:
  _is_group: 0
  name: "My Server"
  ip: "192.168.1.100"
  port: 22
  user: "admin"
  method: "SSH"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert_eq!(conn.name, "My Server");
        assert_eq!(conn.host, "192.168.1.100");
        assert_eq!(conn.port, 22);
        assert_eq!(conn.username, Some("admin".to_string()));
    }

    #[test]
    fn test_parse_with_groups() {
        let importer = AsbruImporter::new();
        // Real Asbru format with groups
        let yaml = r#"
group-uuid-1234:
  _is_group: 1
  name: "Production"
  children: {}

conn-uuid-5678:
  _is_group: 0
  name: "Web Server 1"
  ip: "10.0.0.1"
  method: "SSH"
  parent: "group-uuid-1234"

conn-uuid-9012:
  _is_group: 0
  name: "Web Server 2"
  ip: "10.0.0.2"
  method: "SSH"
  parent: "group-uuid-1234"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.connections.len(), 2);

        // Connections should have group_id set
        for conn in &result.connections {
            assert!(conn.group_id.is_some());
        }
    }

    #[test]
    fn test_parse_rdp_connection() {
        let importer = AsbruImporter::new();
        let yaml = r#"
windows-uuid:
  _is_group: 0
  name: "Windows Server"
  ip: "192.168.1.50"
  port: 3389
  user: "Administrator"
  method: "RDP"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert!(matches!(conn.protocol_config, ProtocolConfig::Rdp(_)));
    }

    #[test]
    fn test_parse_rdp_rdesktop_format() {
        let importer = AsbruImporter::new();
        // Asbru-CM stores RDP with client info like "rdp (rdesktop)"
        let yaml = r#"
rdp-rdesktop-uuid:
  _is_group: 0
  name: "Windows via rdesktop"
  ip: "192.168.1.50"
  port: 3389
  user: "Administrator"
  method: "rdp (rdesktop)"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.skipped.len(), 0);

        let conn = &result.connections[0];
        assert!(matches!(conn.protocol_config, ProtocolConfig::Rdp(_)));
    }

    #[test]
    fn test_parse_rdp_xfreerdp_format() {
        let importer = AsbruImporter::new();
        // Asbru-CM stores RDP with client info like "rdp (xfreerdp)"
        let yaml = r#"
rdp-xfreerdp-uuid:
  _is_group: 0
  name: "Windows via xfreerdp"
  ip: "192.168.1.50"
  port: 3389
  user: "Administrator"
  method: "rdp (xfreerdp)"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.skipped.len(), 0);

        let conn = &result.connections[0];
        assert!(matches!(conn.protocol_config, ProtocolConfig::Rdp(_)));
    }

    #[test]
    fn test_parse_vnc_vncviewer_format() {
        let importer = AsbruImporter::new();
        // Asbru-CM stores VNC with client info like "vnc (vncviewer)"
        let yaml = r#"
vnc-viewer-uuid:
  _is_group: 0
  name: "Linux via VNC"
  ip: "192.168.1.60"
  port: 5900
  method: "vnc (vncviewer)"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.skipped.len(), 0);

        let conn = &result.connections[0];
        assert!(matches!(conn.protocol_config, ProtocolConfig::Vnc(_)));
    }

    #[test]
    fn test_skip_invalid_entries() {
        let importer = AsbruImporter::new();
        let yaml = r#"
valid-uuid:
  _is_group: 0
  name: "Valid Server"
  ip: "192.168.1.1"
  method: "SSH"
invalid-uuid:
  _is_group: 0
  name: "No Host"
  method: "SSH"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.skipped.len(), 1);
    }

    #[test]
    fn test_hostname_extraction_fallback_from_name() {
        let importer = AsbruImporter::new();
        // Entry with "tmp" in ip field but valid hostname in name
        let yaml = r#"
fallback-uuid:
  _is_group: 0
  name: "server.example.com"
  ip: "tmp"
  method: "SSH"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.skipped.len(), 0);
        assert_eq!(result.connections[0].host, "server.example.com");
    }

    #[test]
    fn test_hostname_extraction_fallback_from_title() {
        let importer = AsbruImporter::new();
        // Entry with empty ip/host but valid hostname in title
        let yaml = r#"
fallback-uuid:
  _is_group: 0
  name: "My Server"
  title: "192.168.1.50"
  ip: ""
  method: "SSH"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.skipped.len(), 0);
        assert_eq!(result.connections[0].host, "192.168.1.50");
    }

    #[test]
    fn test_hostname_extraction_with_dynamic_variable() {
        let importer = AsbruImporter::new();
        // Entry with dynamic variable in name
        let yaml = r#"
dynamic-uuid:
  _is_group: 0
  name: "${SERVER_HOST}"
  ip: ""
  method: "SSH"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.skipped.len(), 0);
        assert_eq!(result.connections[0].host, "${SERVER_HOST}");
    }

    #[test]
    fn test_hostname_extraction_skips_tmp_and_empty() {
        let importer = AsbruImporter::new();
        // Entry with only "tmp" and empty values - should be skipped
        let yaml = r#"
skip-uuid:
  _is_group: 0
  name: "tmp"
  title: ""
  ip: "tmp"
  host: ""
  method: "SSH"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.connections.len(), 0);
        assert_eq!(result.skipped.len(), 1);
    }

    #[test]
    fn test_parse_with_options() {
        let importer = AsbruImporter::new();
        let yaml = r#"
server-uuid:
  _is_group: 0
  name: "Server with options"
  ip: "192.168.1.1"
  method: "SSH"
  options: ' -x -C -o "PubkeyAuthentication=no"'
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        if let ProtocolConfig::Ssh(ssh) = &conn.protocol_config {
            assert!(ssh.custom_options.contains_key("PubkeyAuthentication"));
        }
    }

    #[test]
    fn test_dynamic_variables_preserved_in_all_fields() {
        let importer = AsbruImporter::new();
        // Entry with dynamic variables in multiple fields
        let yaml = r#"
dynamic-uuid:
  _is_group: 0
  name: "Dynamic Server"
  ip: "${DB_HOST}"
  user: "${DB_USER}"
  port: 5432
  method: "SSH"
  description: "Connect to ${ENV} database"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.skipped.len(), 0);

        let conn = &result.connections[0];
        // Dynamic variables should be preserved as-is
        assert_eq!(conn.host, "${DB_HOST}");
        assert_eq!(conn.username, Some("${DB_USER}".to_string()));
        // Description with variable should be in description field (not tags)
        assert!(conn
            .description
            .as_ref()
            .is_some_and(|d| d.contains("${ENV}")));
    }

    #[test]
    fn test_dynamic_variable_in_host_field() {
        let importer = AsbruImporter::new();
        // Entry with dynamic variable in host field (not ip)
        let yaml = r#"
host-var-uuid:
  _is_group: 0
  name: "Host Variable Server"
  host: "${REMOTE_HOST}"
  method: "SSH"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.connections[0].host, "${REMOTE_HOST}");
    }

    #[test]
    fn test_dollar_sign_variable_syntax() {
        let importer = AsbruImporter::new();
        // Entry with $VAR syntax (without braces)
        let yaml = r#"
dollar-uuid:
  _is_group: 0
  name: "$HOSTNAME"
  ip: ""
  method: "SSH"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.connections[0].host, "$HOSTNAME");
    }

    #[test]
    fn test_parse_installed_asbru_format() {
        let importer = AsbruImporter::new();
        // Real installed Asbru-CM format with environments key
        let yaml = r#"
---
__PAC__EXPORTED__FULL__: 1
config version: 2
defaults:
  auto save: 1
environments:
  group-uuid-1234:
    _is_group: 1
    name: "Production"
    children: {}
  conn-uuid-5678:
    _is_group: 0
    name: "Web Server"
    ip: "10.0.0.1"
    port: 22
    user: "ubuntu"
    method: "SSH"
    parent: "group-uuid-1234"
  conn-uuid-9012:
    _is_group: 0
    name: "Database"
    ip: "10.0.0.2"
    port: 22
    user: "admin"
    method: "SSH"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.groups.len(), 1, "Should have 1 group");
        assert_eq!(result.connections.len(), 2, "Should have 2 connections");

        // Check group name
        assert_eq!(result.groups[0].name, "Production");

        // Check that one connection has parent group
        let with_parent = result
            .connections
            .iter()
            .filter(|c| c.group_id.is_some())
            .count();
        assert_eq!(with_parent, 1, "One connection should have parent group");
    }

    #[test]
    fn test_parse_mixed_format() {
        let importer = AsbruImporter::new();
        // Test that both top-level and environments entries are parsed
        let yaml = r#"
---
environments:
  env-conn-uuid:
    _is_group: 0
    name: "Env Server"
    ip: "10.0.0.1"
    method: "SSH"
top-level-uuid:
  _is_group: 0
  name: "Top Level Server"
  ip: "10.0.0.2"
  method: "SSH"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.connections.len(), 2, "Should parse both formats");
    }
}
