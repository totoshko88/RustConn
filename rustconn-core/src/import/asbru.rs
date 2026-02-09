//! Asbru-CM configuration importer.
//!
//! Parses Asbru-CM YAML configuration files from ~/.config/pac/ or ~/.config/asbru/
//!
//! # Password Import
//!
//! Asbru-CM stores passwords in YAML files. By default, password import is disabled
//! for security reasons. Enable with `with_password_import(true)`.
//!
//! **Security Warning:** Imported passwords are stored in plain text in the YAML file.
//! Consider using a secure credential backend after import.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use uuid::Uuid;

use crate::error::ImportError;
use crate::models::{
    Connection, ConnectionGroup, PasswordSource, ProtocolConfig, RdpConfig, SshAuthMethod,
    SshConfig, SshKeySource, TelnetConfig, VncConfig,
};

use super::traits::{read_import_file, ImportResult, ImportSource, SkippedEntry};

/// Importer for Asbru-CM configuration files.
///
/// Asbru-CM stores connections in YAML format, typically in
/// ~/.config/pac/ (legacy) or ~/.config/asbru/
pub struct AsbruImporter {
    /// Custom paths to search for Asbru config
    custom_paths: Vec<PathBuf>,
    /// Whether to import passwords from YAML (disabled by default for security)
    import_passwords: bool,
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
    /// Password field - may be plain text or encrypted
    /// Asbru-CM stores passwords as "pass" in YAML
    /// Currently parsed for YAML completeness but not imported by default for security
    #[serde(default)]
    #[allow(dead_code)] // Reserved for future password import feature
    pass: Option<String>,
    /// Alternative password field name used in some Asbru versions
    #[serde(default)]
    #[allow(dead_code)] // Reserved for future password import feature
    password: Option<String>,
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
            import_passwords: false,
        }
    }

    /// Creates a new Asbru-CM importer with custom paths
    #[must_use]
    pub const fn with_paths(paths: Vec<PathBuf>) -> Self {
        Self {
            custom_paths: paths,
            import_passwords: false,
        }
    }

    /// Enables or disables password import from Asbru YAML files.
    ///
    /// **Security Warning:** Asbru-CM stores passwords in YAML files, which may be
    /// plain text or weakly encrypted. Enabling this option will mark connections
    /// as having stored passwords, but the actual password storage depends on your
    /// configured secret backend.
    ///
    /// After import, consider:
    /// - Moving passwords to a secure backend (libsecret, KeePassXC)
    /// - Deleting the original Asbru config file
    /// - Using SSH keys instead of passwords
    #[must_use]
    pub const fn with_password_import(mut self, import: bool) -> Self {
        self.import_passwords = import;
        self
    }

    /// Returns whether password import is enabled
    #[must_use]
    pub const fn imports_passwords(&self) -> bool {
        self.import_passwords
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

    /// Converts Asbru global variable syntax `<GV:VAR_NAME>` to RustConn syntax `${VAR_NAME}`
    ///
    /// Asbru-CM uses `<GV:variable_name>` for global variables.
    /// RustConn uses `${variable_name}` syntax.
    ///
    /// # Examples
    ///
    /// - `<GV:US_Parrallels_User>` → `${US_Parrallels_User}`
    /// - `<GV:dp_SSH_username>` → `${dp_SSH_username}`
    /// - `admin` → `admin` (unchanged)
    fn convert_asbru_variables(s: &str) -> String {
        // Match <GV:variable_name> pattern and replace with ${variable_name}
        // Variable names can contain letters, numbers, and underscores
        let re = regex::Regex::new(r"<GV:([a-zA-Z_][a-zA-Z0-9_]*)>")
            .expect("ASBRU_GV_REGEX is a valid regex pattern");
        // Use $$ to escape the literal $ in the replacement string
        re.replace_all(s, "$${$1}").into_owned()
    }

    /// Checks if a string contains dynamic variable syntax (${VAR} or $VAR)
    fn contains_dynamic_variable(s: &str) -> bool {
        s.contains("${")
            || s.contains("<GV:")
            || (s.contains('$') && s.chars().any(|c| c.is_ascii_alphabetic()))
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
        // First pass: create ALL groups and map original UUIDs to new UUIDs
        // This ensures all groups exist in uuid_map before we try to resolve parent references
        let mut uuid_map: HashMap<String, Uuid> = HashMap::new();
        let mut groups_data: HashMap<String, (ConnectionGroup, Option<String>, Option<String>)> =
            HashMap::new();

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
                groups_data.insert(
                    key.clone(),
                    (group, entry.parent.clone(), entry.description.clone()),
                );
            }
        }

        // Second pass: set parent_id and description for groups using the complete uuid_map
        // Now all groups are in uuid_map, so parent lookups will succeed
        for (_key, (mut group, parent_key, description)) in groups_data {
            if let Some(ref parent_key) = parent_key {
                // Skip special Asbru parent keys that don't map to real groups
                if !parent_key.starts_with("__") {
                    if let Some(&parent_uuid) = uuid_map.get(parent_key) {
                        group.parent_id = Some(parent_uuid);
                    }
                }
            }
            // Set description if present and not empty
            if let Some(desc) = description {
                if !desc.is_empty() {
                    group.description = Some(desc);
                }
            }
            result.add_group(group);
        }

        // Third pass: process connections
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

                // Parse SSH options from the options field
                // Asbru stores options like "-X -C -A -o \"Option=value\""
                let mut x11_forwarding = false;
                let mut compression = false;
                let mut agent_forwarding = false;
                let mut custom_options = HashMap::new();

                if let Some(opts) = &entry.options {
                    let parts: Vec<&str> = opts.split_whitespace().collect();
                    let mut i = 0;
                    while i < parts.len() {
                        let part = parts[i];
                        match part {
                            "-X" | "-x" => x11_forwarding = true,
                            "-C" => compression = true,
                            "-A" => agent_forwarding = true,
                            "-o" => {
                                // Next part is the option value
                                if i + 1 < parts.len() {
                                    i += 1;
                                    let opt = parts[i].trim_matches('"');
                                    if let Some((k, v)) = opt.split_once('=') {
                                        custom_options.insert(k.to_string(), v.to_string());
                                    }
                                }
                            }
                            _ if part.contains('=') => {
                                // Standalone option like "Option=value"
                                let clean = part.trim_matches('"');
                                if let Some((k, v)) = clean.split_once('=') {
                                    custom_options.insert(k.to_string(), v.to_string());
                                }
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                }

                (
                    ProtocolConfig::Ssh(SshConfig {
                        auth_method,
                        key_path,
                        key_source: SshKeySource::Default,
                        agent_key_fingerprint: None,
                        identities_only: false,
                        jump_host_id: None,
                        proxy_jump: None,
                        use_control_master: false,
                        agent_forwarding,
                        x11_forwarding,
                        compression,
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
            "telnet" => (ProtocolConfig::Telnet(TelnetConfig::default()), 23u16),
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
            // Convert Asbru global variable syntax <GV:VAR> to RustConn syntax ${VAR}
            connection.username = Some(Self::convert_asbru_variables(user));
        }

        // Handle password import if enabled
        // Check both 'pass' and 'password' fields (different Asbru versions use different names)
        if self.import_passwords {
            let has_password = entry
                .pass
                .as_ref()
                .or(entry.password.as_ref())
                .is_some_and(|p| !p.is_empty());

            if has_password {
                // Mark as having a password that needs to be entered
                // The actual password will be handled by the secret backend during connection
                // We don't store the plain text password here for security
                connection.password_source = PasswordSource::Prompt;
            }
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

    /// Extracts the password from an Asbru entry if password import is enabled.
    ///
    /// Returns `None` if password import is disabled or no password is present.
    ///
    /// **Security Note:** The returned password should be immediately stored in a
    /// secure backend and not kept in memory longer than necessary.
    #[must_use]
    pub fn extract_password(&self, entry_key: &str, content: &str) -> Option<String> {
        if !self.import_passwords {
            return None;
        }

        // Parse the YAML to find the specific entry
        let raw_config: HashMap<String, serde_yaml::Value> = serde_yaml::from_str(content).ok()?;

        // Check environments first
        if let Some(environments) = raw_config.get("environments") {
            if let Some(env_map) = environments.as_mapping() {
                if let Some(entry_value) =
                    env_map.get(&serde_yaml::Value::String(entry_key.to_string()))
                {
                    if let Ok(entry) = serde_yaml::from_value::<AsbruEntry>(entry_value.clone()) {
                        return entry.pass.or(entry.password).filter(|p| !p.is_empty());
                    }
                }
            }
        }

        // Check top-level
        if let Some(entry_value) = raw_config.get(entry_key) {
            if let Ok(entry) = serde_yaml::from_value::<AsbruEntry>(entry_value.clone()) {
                return entry.pass.or(entry.password).filter(|p| !p.is_empty());
            }
        }

        None
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
        let content = read_import_file(path, "Asbru-CM")?;

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
    fn test_parse_nested_groups() {
        let importer = AsbruImporter::new();
        // Test nested group hierarchy like:
        // Root Group
        // └── Child Group
        //     └── Grandchild Group
        //         └── Connection
        // Note: HashMap iteration order is not guaranteed, so this tests
        // that parent_id is correctly set regardless of processing order
        let yaml = r#"
grandchild-group:
  _is_group: 1
  name: "Grandchild"
  parent: "child-group"
  children: {}

child-group:
  _is_group: 1
  name: "Child"
  parent: "root-group"
  children:
    grandchild-group: 1

root-group:
  _is_group: 1
  name: "Root"
  parent: "__PAC__EXPORTED__"
  children:
    child-group: 1

connection-uuid:
  _is_group: 0
  name: "Server"
  ip: "10.0.0.1"
  method: "SSH"
  parent: "grandchild-group"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.groups.len(), 3, "Should have 3 groups");
        assert_eq!(result.connections.len(), 1, "Should have 1 connection");

        // Find groups by name
        let root = result.groups.iter().find(|g| g.name == "Root").unwrap();
        let child = result.groups.iter().find(|g| g.name == "Child").unwrap();
        let grandchild = result
            .groups
            .iter()
            .find(|g| g.name == "Grandchild")
            .unwrap();

        // Root should have no parent (special __PAC__ keys are skipped)
        assert!(root.parent_id.is_none(), "Root should have no parent");

        // Child should have Root as parent
        assert_eq!(
            child.parent_id,
            Some(root.id),
            "Child should have Root as parent"
        );

        // Grandchild should have Child as parent
        assert_eq!(
            grandchild.parent_id,
            Some(child.id),
            "Grandchild should have Child as parent"
        );

        // Connection should be in Grandchild group
        let conn = &result.connections[0];
        assert_eq!(
            conn.group_id,
            Some(grandchild.id),
            "Connection should be in Grandchild group"
        );
    }

    #[test]
    fn test_parse_group_description() {
        let importer = AsbruImporter::new();
        // Test that group description is imported from Asbru
        let yaml = r#"
group-with-desc:
  _is_group: 1
  name: "Project Group"
  description: |-
    Connection group 'Project'
    
    notify-project@example.com
    
    TimeReport: 24/7 cover
    
    AWS
    US West (Oregon) - us-west-2
  children: {}

group-no-desc:
  _is_group: 1
  name: "Empty Description Group"
  description: ""
  children: {}

group-missing-desc:
  _is_group: 1
  name: "No Description Field"
  children: {}
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.groups.len(), 3, "Should have 3 groups");

        // Find groups by name
        let with_desc = result
            .groups
            .iter()
            .find(|g| g.name == "Project Group")
            .unwrap();
        let empty_desc = result
            .groups
            .iter()
            .find(|g| g.name == "Empty Description Group")
            .unwrap();
        let no_desc = result
            .groups
            .iter()
            .find(|g| g.name == "No Description Field")
            .unwrap();

        // Group with description should have it set
        assert!(
            with_desc.description.is_some(),
            "Group should have description"
        );
        assert!(
            with_desc
                .description
                .as_ref()
                .unwrap()
                .contains("notify-project@example.com"),
            "Description should contain email"
        );

        // Empty description should be None
        assert!(
            empty_desc.description.is_none(),
            "Empty description should be None"
        );

        // Missing description field should be None
        assert!(
            no_desc.description.is_none(),
            "Missing description should be None"
        );
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

    #[test]
    fn test_password_import_disabled_by_default() {
        let importer = AsbruImporter::new();
        assert!(!importer.imports_passwords());

        let yaml = r#"
server-uuid:
  _is_group: 0
  name: "Server with password"
  ip: "192.168.1.1"
  user: "admin"
  pass: "secret123"
  method: "SSH"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.connections.len(), 1);
        // Password should NOT be marked as stored when import is disabled
        assert_eq!(result.connections[0].password_source, PasswordSource::None);
    }

    #[test]
    fn test_password_import_enabled() {
        let importer = AsbruImporter::new().with_password_import(true);
        assert!(importer.imports_passwords());

        let yaml = r#"
server-uuid:
  _is_group: 0
  name: "Server with password"
  ip: "192.168.1.1"
  user: "admin"
  pass: "secret123"
  method: "SSH"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.connections.len(), 1);
        // Password should be marked as prompt when import is enabled
        assert_eq!(
            result.connections[0].password_source,
            PasswordSource::Prompt
        );
    }

    #[test]
    fn test_password_import_alternative_field_name() {
        let importer = AsbruImporter::new().with_password_import(true);

        // Some Asbru versions use "password" instead of "pass"
        let yaml = r#"
server-uuid:
  _is_group: 0
  name: "Server with password"
  ip: "192.168.1.1"
  user: "admin"
  password: "secret456"
  method: "SSH"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.connections.len(), 1);
        assert_eq!(
            result.connections[0].password_source,
            PasswordSource::Prompt
        );
    }

    #[test]
    fn test_password_import_empty_password() {
        let importer = AsbruImporter::new().with_password_import(true);

        let yaml = r#"
server-uuid:
  _is_group: 0
  name: "Server without password"
  ip: "192.168.1.1"
  user: "admin"
  pass: ""
  method: "SSH"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.connections.len(), 1);
        // Empty password should not be marked as stored
        assert_eq!(result.connections[0].password_source, PasswordSource::None);
    }

    #[test]
    fn test_extract_password() {
        let importer = AsbruImporter::new().with_password_import(true);

        let yaml = r#"
server-uuid:
  _is_group: 0
  name: "Server"
  ip: "192.168.1.1"
  pass: "my_secret_password"
  method: "SSH"
"#;

        let password = importer.extract_password("server-uuid", yaml);
        assert_eq!(password, Some("my_secret_password".to_string()));
    }

    #[test]
    fn test_extract_password_from_environments() {
        let importer = AsbruImporter::new().with_password_import(true);

        let yaml = r#"
environments:
  server-uuid:
    _is_group: 0
    name: "Server"
    ip: "192.168.1.1"
    pass: "env_password"
    method: "SSH"
"#;

        let password = importer.extract_password("server-uuid", yaml);
        assert_eq!(password, Some("env_password".to_string()));
    }

    #[test]
    fn test_extract_password_disabled() {
        let importer = AsbruImporter::new(); // Password import disabled

        let yaml = r#"
server-uuid:
  _is_group: 0
  name: "Server"
  ip: "192.168.1.1"
  pass: "should_not_extract"
  method: "SSH"
"#;

        let password = importer.extract_password("server-uuid", yaml);
        assert_eq!(password, None);
    }

    #[test]
    fn test_convert_asbru_global_variables() {
        // Test the static conversion function
        assert_eq!(
            AsbruImporter::convert_asbru_variables("<GV:US_Parrallels_User>"),
            "${US_Parrallels_User}"
        );
        assert_eq!(
            AsbruImporter::convert_asbru_variables("<GV:dp_SSH_username>"),
            "${dp_SSH_username}"
        );
        assert_eq!(
            AsbruImporter::convert_asbru_variables("<GV:C2S_User>"),
            "${C2S_User}"
        );
        // Plain text should remain unchanged
        assert_eq!(AsbruImporter::convert_asbru_variables("admin"), "admin");
        // Already RustConn syntax should remain unchanged
        assert_eq!(
            AsbruImporter::convert_asbru_variables("${MY_VAR}"),
            "${MY_VAR}"
        );
        // Mixed content
        assert_eq!(
            AsbruImporter::convert_asbru_variables("prefix_<GV:VAR>_suffix"),
            "prefix_${VAR}_suffix"
        );
    }

    #[test]
    fn test_import_converts_asbru_global_variables_in_username() {
        let importer = AsbruImporter::new();
        // Entry with Asbru global variable in user field
        let yaml = r#"
server-uuid:
  _is_group: 0
  name: "Server with GV"
  ip: "192.168.1.1"
  user: "<GV:US_Parrallels_User>"
  method: "SSH"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.connections.len(), 1);
        // Username should be converted from <GV:...> to ${...}
        assert_eq!(
            result.connections[0].username,
            Some("${US_Parrallels_User}".to_string())
        );
    }

    #[test]
    fn test_import_preserves_plain_username() {
        let importer = AsbruImporter::new();
        let yaml = r#"
server-uuid:
  _is_group: 0
  name: "Server"
  ip: "192.168.1.1"
  user: "admin"
  method: "SSH"
"#;

        let result = importer.parse_config(yaml, "test");
        assert_eq!(result.connections.len(), 1);
        // Plain username should remain unchanged
        assert_eq!(result.connections[0].username, Some("admin".to_string()));
    }
}
