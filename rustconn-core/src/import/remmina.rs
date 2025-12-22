//! Remmina configuration importer.
//!
//! Parses .remmina files from ~/.local/share/remmina/
//! Supports importing passwords from GNOME Keyring via secret-tool.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::ImportError;
use crate::models::{
    Connection, PasswordSource, ProtocolConfig, RdpConfig, Resolution, SshAuthMethod, SshConfig,
    SshKeySource, VncConfig,
};

use super::traits::{ImportResult, ImportSource, SkippedEntry};

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
    /// Remmina stores passwords in the keyring with specific attributes.
    fn get_password_from_keyring(filename: &str, protocol: &str) -> Option<String> {
        // Remmina uses the filename (without path) as the key
        // Format: secret-tool lookup filename <filename> protocol <protocol>
        let output = Command::new("secret-tool")
            .args(["lookup", "filename", filename, "protocol", protocol])
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

        // Try alternative lookup with just filename
        let output = Command::new("secret-tool")
            .args(["lookup", "filename", filename])
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

        // Try with Remmina-specific attributes
        let output = Command::new("secret-tool")
            .args([
                "lookup",
                "application",
                "org.remmina.Remmina",
                "filename",
                filename,
            ])
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

        None
    }

    /// Parses a single .remmina file content
    #[must_use]
    pub fn parse_remmina_file(&self, content: &str, source_path: &str) -> ImportResult {
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
        if let Some(connection) =
            self.convert_to_connection(remmina_section, source_path, &mut result)
        {
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
                let value = line[eq_pos + 1..].trim().to_string();

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
    #[allow(clippy::unused_self, clippy::too_many_lines)]
    fn convert_to_connection(
        &self,
        config: &HashMap<String, String>,
        source_path: &str,
        result: &mut ImportResult,
    ) -> Option<Connection> {
        // Get protocol
        let protocol = config.get("protocol").map(|s| s.to_uppercase());

        // Get server/host
        let server = config.get("server").or_else(|| config.get("ssh_server"));
        let host = match server {
            Some(s) if !s.is_empty() => {
                // Server might include port (host:port)
                s.rfind(':')
                    .map_or_else(|| s.clone(), |colon_pos| s[..colon_pos].to_string())
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

        // Parse port from server string or dedicated field
        let port = config
            .get("server")
            .and_then(|s| {
                s.rfind(':')
                    .and_then(|pos| s[pos + 1..].parse::<u16>().ok())
            })
            .or_else(|| config.get("ssh_server_port").and_then(|p| p.parse().ok()));

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

                (
                    ProtocolConfig::Ssh(SshConfig {
                        auth_method,
                        key_path,
                        key_source: SshKeySource::Default,
                        agent_key_fingerprint: None,
                        identities_only: false,
                        proxy_jump: None,
                        use_control_master: false,
                        custom_options: HashMap::new(),
                        startup_command: None,
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
            // Extract filename from source_path for keyring lookup
            let filename = Path::new(source_path)
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or(source_path);

            let protocol_str = protocol.as_deref().unwrap_or("SSH");

            if let Some(_password) = Self::get_password_from_keyring(filename, protocol_str) {
                // Mark that password is available in keyring
                // The actual password will be retrieved at connection time
                connection.password_source = PasswordSource::Keyring;
            }
        }

        // Add group as tag if present
        if let Some(group) = config.get("group") {
            if !group.is_empty() {
                connection.tags.push(format!("remmina:{group}"));
            }
        }

        Some(connection)
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
            source_name: "Remmina".to_string(),
            reason: format!("Failed to read {}: {}", path.display(), e),
        })?;

        Ok(self.parse_remmina_file(&content, &path.display().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ssh_connection() {
        let importer = RemminaImporter::new();
        let content = r"
[remmina]
name=My SSH Server
protocol=SSH
server=192.168.1.100:22
username=admin
";

        let result = importer.parse_remmina_file(content, "test.remmina");
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
        let content = r"
[remmina]
name=Windows Server
protocol=RDP
server=192.168.1.50
username=Administrator
resolution=1920x1080
colordepth=32
";

        let result = importer.parse_remmina_file(content, "test.remmina");
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
        let content = r"
[remmina]
name=VNC Desktop
protocol=VNC
server=192.168.1.75:5901
";

        let result = importer.parse_remmina_file(content, "test.remmina");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert!(matches!(conn.protocol_config, ProtocolConfig::Vnc(_)));
        assert_eq!(conn.port, 5901);
    }

    #[test]
    fn test_skip_unsupported_protocol() {
        let importer = RemminaImporter::new();
        let content = r"
[remmina]
name=Unknown
protocol=SPICE
server=192.168.1.100
";

        let result = importer.parse_remmina_file(content, "test.remmina");
        assert_eq!(result.connections.len(), 0);
        assert_eq!(result.skipped.len(), 1);
    }

    #[test]
    fn test_skip_no_server() {
        let importer = RemminaImporter::new();
        let content = r"
[remmina]
name=No Server
protocol=SSH
";

        let result = importer.parse_remmina_file(content, "test.remmina");
        assert_eq!(result.connections.len(), 0);
        assert_eq!(result.skipped.len(), 1);
    }
}
