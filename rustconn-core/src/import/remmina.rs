//! Remmina configuration importer.
//!
//! Parses .remmina files from ~/.local/share/remmina/
//! Supports importing passwords from GNOME Keyring via secret-tool.
//! Creates proper group hierarchy from Remmina's group field.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use uuid::Uuid;

use crate::error::ImportError;
use crate::models::{
    Connection, ConnectionGroup, Credentials, PasswordSource, ProtocolConfig, RdpConfig,
    Resolution, SpiceConfig, SshAuthMethod, SshConfig, SshKeySource, TelnetConfig, VncConfig,
};

use super::normalize::parse_host_port;
use super::traits::{read_import_file, ImportResult, ImportSource, SkippedEntry};

/// Importer for Remmina connection files.
///
/// Remmina stores each connection in a separate .remmina file
/// in ~/.local/share/remmina/
pub struct RemminaImporter {
    /// Custom paths to search for Remmina files
    custom_paths: Vec<PathBuf>,
    /// Whether to import passwords from GNOME Keyring
    import_passwords: bool,
}

impl RemminaImporter {
    /// Creates a new Remmina importer with default paths
    #[must_use]
    pub const fn new() -> Self {
        Self {
            custom_paths: Vec::new(),
            import_passwords: true,
        }
    }

    /// Creates a new Remmina importer with custom paths
    #[must_use]
    pub const fn with_paths(paths: Vec<PathBuf>) -> Self {
        Self {
            custom_paths: paths,
            import_passwords: true,
        }
    }

    /// Sets whether to import passwords from GNOME Keyring
    #[must_use]
    pub const fn with_password_import(mut self, import: bool) -> Self {
        self.import_passwords = import;
        self
    }

    /// Retrieves a password from GNOME Keyring for a Remmina connection
    ///
    /// Remmina stores passwords using the `org.remmina.Password` schema with
    /// attributes: `filename` (full path to .remmina file) and `key` (field name,
    /// e.g. "password"). We try the full path first, then fall back to just the
    /// filename for compatibility with older Remmina versions.
    fn get_password_from_keyring(file_path: &str) -> Option<String> {
        // Primary lookup: full path + key="password" (matches Remmina's schema)
        let output = Command::new("secret-tool")
            .args(["lookup", "filename", file_path, "key", "password"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .ok()?;

        if output.status.success() {
            let password = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !password.is_empty() {
                return Some(password);
            }
        }

        // Fallback: try with just the basename (older Remmina versions)
        let basename = Path::new(file_path)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or(file_path);

        if basename != file_path {
            let output = Command::new("secret-tool")
                .args(["lookup", "filename", basename, "key", "password"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output()
                .ok()?;

            if output.status.success() {
                let password = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !password.is_empty() {
                    return Some(password);
                }
            }
        }

        None
    }

    /// Parses a single .remmina file content
    #[must_use]
    pub fn parse_remmina_file(
        &self,
        content: &str,
        source_path: &str,
        group_map: &mut HashMap<String, Uuid>,
    ) -> ImportResult {
        let mut result = ImportResult::new();

        // Parse INI-style format
        let config = Self::parse_ini(content);

        // Get the remmina section
        let Some(remmina_section) = config.get("remmina") else {
            result.add_skipped(SkippedEntry::with_location(
                source_path,
                "No [remmina] section found",
                source_path,
            ));
            return result;
        };

        // Extract connection details
        if let Some((mut connection, group_path)) =
            self.convert_to_connection(remmina_section, source_path, &mut result)
        {
            // Assign group if present
            if let Some(ref path) = group_path {
                if !path.is_empty() {
                    let group_id = Self::get_or_create_group(path, group_map, &mut result);
                    connection.group_id = Some(group_id);
                }
            }
            result.add_connection(connection);
        }

        result
    }

    /// Parses INI-style content into sections
    fn parse_ini(content: &str) -> HashMap<String, HashMap<String, String>> {
        let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut current_section: Option<String> = None;

        for line in content.lines() {
            let line = line.trim();

            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }

            // Check for section header
            if line.starts_with('[') && line.ends_with(']') {
                let section_name = line[1..line.len() - 1].to_lowercase();
                current_section = Some(section_name.clone());
                sections.entry(section_name).or_default();
                continue;
            }

            // Parse key=value
            if let Some(eq_pos) = line.find('=') {
                let key = line[..eq_pos].trim().to_lowercase();
                // Strip trailing literal escape sequences (\n, \r, \t)
                // that Remmina INI files sometimes contain at end of values
                let value = super::normalize::sanitize_imported_value(line[eq_pos + 1..].trim());

                if let Some(ref section) = current_section {
                    sections
                        .entry(section.clone())
                        .or_default()
                        .insert(key, value);
                }
            }
        }

        sections
    }

    /// Converts Remmina config section to a Connection
    /// Returns (Connection, Option<group_path>) for group assignment
    #[allow(clippy::too_many_lines)]
    fn convert_to_connection(
        &self,
        config: &HashMap<String, String>,
        source_path: &str,
        result: &mut ImportResult,
    ) -> Option<(Connection, Option<String>)> {
        // Get protocol
        let protocol = config.get("protocol").map(|s| s.to_uppercase());

        // Get server/host
        let server = config.get("server").or_else(|| config.get("ssh_server"));
        let (host, parsed_port) = match server {
            Some(s) if !s.is_empty() => {
                // Use shared utility for host:port parsing
                parse_host_port(s)
            }
            _ => {
                result.add_skipped(SkippedEntry::with_location(
                    source_path,
                    "No server specified",
                    source_path,
                ));
                return None;
            }
        };

        // Get name
        let name = config.get("name").cloned().unwrap_or_else(|| host.clone());

        // Parse port from dedicated field or use parsed port from server string
        let port = config
            .get("ssh_server_port")
            .and_then(|p| p.parse().ok())
            .or(parsed_port);

        // Create protocol-specific config
        let (protocol_config, default_port) = match protocol.as_deref() {
            Some("SSH" | "SFTP") => {
                let auth_method = match config.get("ssh_auth").map(std::string::String::as_str) {
                    Some("2" | "publickey") => SshAuthMethod::PublicKey,
                    Some("3" | "agent") => SshAuthMethod::Agent,
                    _ => SshAuthMethod::Password,
                };

                let key_path = config
                    .get("ssh_privatekey")
                    .filter(|s| !s.is_empty())
                    .map(|p| PathBuf::from(shellexpand::tilde(p).into_owned()));

                // Check for SSH tunnel options
                let x11_forwarding = config
                    .get("ssh_tunnel_x11")
                    .is_some_and(|v| v == "1" || v.to_lowercase() == "yes");

                let compression = config
                    .get("ssh_compression")
                    .is_some_and(|v| v == "1" || v.to_lowercase() == "yes");

                let agent_forwarding = config
                    .get("ssh_tunnel_agent")
                    .is_some_and(|v| v == "1" || v.to_lowercase() == "yes");

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
                        custom_options: HashMap::new(),
                        startup_command: None,
                        sftp_enabled: false,
                    }),
                    22u16,
                )
            }
            Some("RDP") => {
                let resolution = config.get("resolution").and_then(|r| {
                    let parts: Vec<&str> = r.split('x').collect();
                    if parts.len() == 2 {
                        let width = parts[0].parse().ok()?;
                        let height = parts[1].parse().ok()?;
                        Some(Resolution::new(width, height))
                    } else {
                        None
                    }
                });

                let color_depth = config.get("colordepth").and_then(|d| d.parse().ok());

                (
                    ProtocolConfig::Rdp(RdpConfig {
                        resolution,
                        color_depth,
                        audio_redirect: config.get("sound").is_some_and(|s| s != "off"),
                        ..Default::default()
                    }),
                    3389u16,
                )
            }
            Some("VNC") => (ProtocolConfig::Vnc(VncConfig::default()), 5900u16),
            Some("SPICE") => (ProtocolConfig::Spice(SpiceConfig::default()), 5900u16),
            Some("TELNET") => (ProtocolConfig::Telnet(TelnetConfig::default()), 23u16),
            Some(p) => {
                result.add_skipped(SkippedEntry::with_location(
                    &name,
                    format!("Unsupported protocol: {p}"),
                    source_path,
                ));
                return None;
            }
            None => {
                result.add_skipped(SkippedEntry::with_location(
                    &name,
                    "No protocol specified",
                    source_path,
                ));
                return None;
            }
        };

        let port = port.unwrap_or(default_port);

        let mut connection = Connection::new(name, host, port, protocol_config);

        // Set username
        if let Some(username) = config
            .get("username")
            .or_else(|| config.get("ssh_username"))
        {
            if !username.is_empty() {
                connection.username = Some(username.clone());
            }
        }

        // Try to import password from GNOME Keyring if enabled
        if self.import_passwords {
            if let Some(password) = Self::get_password_from_keyring(source_path) {
                // Store credentials in the result for later persistence
                let creds = Credentials::with_password(
                    connection.username.clone().unwrap_or_default(),
                    password,
                );
                result.add_credentials(connection.id, creds);
                connection.password_source = PasswordSource::Vault;
            }
        }

        // Return connection and group name for later processing
        Some((connection, config.get("group").cloned()))
    }

    /// Gets or creates a group from the group map, handling nested paths like "Folder/Subfolder"
    ///
    /// # Preconditions
    ///
    /// `group_path` must not be empty. The caller is responsible for checking this.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if `group_path` is empty.
    fn get_or_create_group(
        group_path: &str,
        group_map: &mut HashMap<String, Uuid>,
        result: &mut ImportResult,
    ) -> Uuid {
        debug_assert!(!group_path.is_empty(), "group_path must not be empty");

        // Check if already exists
        if let Some(&id) = group_map.get(group_path) {
            return id;
        }

        // Handle nested paths (e.g., "Production/Web Servers")
        let parts: Vec<&str> = group_path.split('/').collect();
        let mut parent_id: Option<Uuid> = None;
        let mut current_path = String::new();

        for (idx, part) in parts.iter().enumerate() {
            if idx > 0 {
                current_path.push('/');
            }
            current_path.push_str(part);

            if let Some(&existing_id) = group_map.get(&current_path) {
                parent_id = Some(existing_id);
            } else {
                // Create new group
                let group = if let Some(pid) = parent_id {
                    ConnectionGroup::with_parent((*part).to_string(), pid)
                } else {
                    ConnectionGroup::new((*part).to_string())
                };
                let group_id = group.id;
                group_map.insert(current_path.clone(), group_id);
                result.add_group(group);
                parent_id = Some(group_id);
            }
        }

        // SAFETY: parent_id is always Some after the loop because:
        // 1. group_path is non-empty (precondition)
        // 2. split('/') on non-empty string always yields at least one part
        // 3. Each part either finds an existing group or creates a new one
        parent_id.expect("parent_id is always Some for non-empty group_path")
    }
}

impl Default for RemminaImporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportSource for RemminaImporter {
    fn source_id(&self) -> &'static str {
        "remmina"
    }

    fn display_name(&self) -> &'static str {
        "Remmina"
    }

    fn is_available(&self) -> bool {
        self.default_paths().iter().any(|p| p.exists())
    }

    fn default_paths(&self) -> Vec<PathBuf> {
        if !self.custom_paths.is_empty() {
            return self.custom_paths.clone();
        }

        let mut paths = Vec::new();

        if let Some(data_dir) = dirs::data_local_dir() {
            let remmina_dir = data_dir.join("remmina");
            if remmina_dir.is_dir() {
                if let Ok(entries) = fs::read_dir(&remmina_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().is_some_and(|ext| ext == "remmina") {
                            paths.push(path);
                        }
                    }
                }
            }
        }

        paths
    }

    fn import(&self) -> Result<ImportResult, ImportError> {
        let paths = self.default_paths();

        if paths.is_empty() {
            return Err(ImportError::FileNotFound(PathBuf::from(
                "~/.local/share/remmina/",
            )));
        }

        let mut combined_result = ImportResult::new();
        let mut group_map: HashMap<String, Uuid> = HashMap::new();

        for path in paths {
            match self.import_single_file(&path, &mut group_map) {
                Ok(result) => combined_result.merge(result),
                Err(e) => combined_result.add_error(e),
            }
        }

        Ok(combined_result)
    }

    fn import_from_path(&self, path: &Path) -> Result<ImportResult, ImportError> {
        let mut group_map: HashMap<String, Uuid> = HashMap::new();
        self.import_single_file(path, &mut group_map)
    }
}

impl RemminaImporter {
    /// Imports a single .remmina file with shared group map
    fn import_single_file(
        &self,
        path: &Path,
        group_map: &mut HashMap<String, Uuid>,
    ) -> Result<ImportResult, ImportError> {
        let content = read_import_file(path, "Remmina")?;

        Ok(self.parse_remmina_file(&content, &path.display().to_string(), group_map))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ssh_connection() {
        let importer = RemminaImporter::new();
        let mut group_map = HashMap::new();
        let content = r"
[remmina]
name=My SSH Server
protocol=SSH
server=192.168.1.100:22
username=admin
";

        let result = importer.parse_remmina_file(content, "test.remmina", &mut group_map);
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert_eq!(conn.name, "My SSH Server");
        assert_eq!(conn.host, "192.168.1.100");
        assert_eq!(conn.port, 22);
        assert_eq!(conn.username, Some("admin".to_string()));
    }

    #[test]
    fn test_parse_rdp_connection() {
        let importer = RemminaImporter::new();
        let mut group_map = HashMap::new();
        let content = r"
[remmina]
name=Windows Server
protocol=RDP
server=192.168.1.50
username=Administrator
resolution=1920x1080
colordepth=32
";

        let result = importer.parse_remmina_file(content, "test.remmina", &mut group_map);
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert_eq!(conn.name, "Windows Server");
        assert!(matches!(conn.protocol_config, ProtocolConfig::Rdp(_)));

        if let ProtocolConfig::Rdp(rdp_config) = &conn.protocol_config {
            assert!(rdp_config.resolution.is_some());
            assert_eq!(rdp_config.color_depth, Some(32));
        }
    }

    #[test]
    fn test_parse_vnc_connection() {
        let importer = RemminaImporter::new();
        let mut group_map = HashMap::new();
        let content = r"
[remmina]
name=VNC Desktop
protocol=VNC
server=192.168.1.75:5901
";

        let result = importer.parse_remmina_file(content, "test.remmina", &mut group_map);
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert!(matches!(conn.protocol_config, ProtocolConfig::Vnc(_)));
        assert_eq!(conn.port, 5901);
    }

    #[test]
    fn test_parse_spice_connection() {
        let importer = RemminaImporter::new();
        let mut group_map = HashMap::new();
        let content = r"
[remmina]
name=SPICE VM
protocol=SPICE
server=192.168.1.100:5900
";

        let result = importer.parse_remmina_file(content, "test.remmina", &mut group_map);
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert!(matches!(conn.protocol_config, ProtocolConfig::Spice(_)));
        assert_eq!(conn.port, 5900);
    }

    #[test]
    fn test_parse_with_group() {
        let importer = RemminaImporter::new();
        let mut group_map = HashMap::new();
        let content = r"
[remmina]
name=Production Server
protocol=SSH
server=10.0.0.1
group=Production
";

        let result = importer.parse_remmina_file(content, "test.remmina", &mut group_map);
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.groups.len(), 1);

        let conn = &result.connections[0];
        assert!(conn.group_id.is_some());
        assert_eq!(result.groups[0].name, "Production");
    }

    #[test]
    fn test_parse_with_nested_group() {
        let importer = RemminaImporter::new();
        let mut group_map = HashMap::new();
        let content = r"
[remmina]
name=Web Server
protocol=SSH
server=10.0.0.1
group=Production/Web Servers
";

        let result = importer.parse_remmina_file(content, "test.remmina", &mut group_map);
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.groups.len(), 2);

        // Check hierarchy
        let production = result.groups.iter().find(|g| g.name == "Production");
        let web_servers = result.groups.iter().find(|g| g.name == "Web Servers");
        assert!(production.is_some());
        assert!(web_servers.is_some());
        assert!(production
            .as_ref()
            .map(|g| g.parent_id.is_none())
            .unwrap_or(false));
        assert_eq!(
            web_servers.as_ref().and_then(|g| g.parent_id),
            production.map(|g| g.id)
        );
    }

    #[test]
    fn test_skip_unsupported_protocol() {
        let importer = RemminaImporter::new();
        let mut group_map = HashMap::new();
        let content = r"
[remmina]
name=Unknown
protocol=X2GO
server=192.168.1.100
";

        let result = importer.parse_remmina_file(content, "test.remmina", &mut group_map);
        assert_eq!(result.connections.len(), 0);
        assert_eq!(result.skipped.len(), 1);
    }

    #[test]
    fn test_skip_no_server() {
        let importer = RemminaImporter::new();
        let mut group_map = HashMap::new();
        let content = r"
[remmina]
name=No Server
protocol=SSH
";

        let result = importer.parse_remmina_file(content, "test.remmina", &mut group_map);
        assert_eq!(result.connections.len(), 0);
        assert_eq!(result.skipped.len(), 1);
    }

    #[test]
    fn test_parse_name_with_trailing_escape() {
        let importer = RemminaImporter::new();
        let mut group_map = HashMap::new();
        // Remmina files can have literal \n at end of name values
        let content = r"
[remmina]
name=ec2-3-250-166-202.eu-west-1.compute.amazonaws.com\n
protocol=RDP
server=ec2-3-250-166-202.eu-west-1.compute.amazonaws.com
username=Administrator
";

        let result = importer.parse_remmina_file(content, "test.remmina", &mut group_map);
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        // parse_ini now strips trailing \n at the source
        assert_eq!(
            conn.name,
            "ec2-3-250-166-202.eu-west-1.compute.amazonaws.com"
        );
    }

    #[test]
    fn test_shared_group_map() {
        let importer = RemminaImporter::new();
        let mut group_map = HashMap::new();

        // First file
        let content1 = r"
[remmina]
name=Server 1
protocol=SSH
server=10.0.0.1
group=Production
";
        let result1 = importer.parse_remmina_file(content1, "test1.remmina", &mut group_map);

        // Second file with same group
        let content2 = r"
[remmina]
name=Server 2
protocol=SSH
server=10.0.0.2
group=Production
";
        let result2 = importer.parse_remmina_file(content2, "test2.remmina", &mut group_map);

        // Group should be reused, not duplicated
        assert_eq!(result1.groups.len(), 1);
        assert_eq!(result2.groups.len(), 0); // No new group created
        assert_eq!(
            result1.connections[0].group_id,
            result2.connections[0].group_id
        );
    }
}
