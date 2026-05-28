//! SecureCRT session file importer.
//!
//! Parses SecureCRT session `.ini` files from the `Config/Sessions/` directory.
//! Supports SSH2, Telnet, RDP, VNC, and Serial session types.
//!
//! SecureCRT stores each session as a separate `.ini` file with key-value pairs.
//! Keys are prefixed with a type indicator:
//! - `S:` — String value
//! - `D:` — DWORD (hex-encoded u32)
//! - `B:` — Binary data (hex dump)
//!
//! The directory hierarchy under `Config/Sessions/` maps to connection groups.
//!
//! ## Supported formats
//!
//! 1. **Session directory** — a folder containing individual `.ini` files
//!    (e.g., `~/.vandyke/Config/Sessions/` on Linux)
//! 2. **Single `.ini` file** — import a single session file directly

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::error::ImportError;
use crate::models::{
    Connection, ConnectionGroup, ProtocolConfig, RdpConfig, SshAuthMethod, SshConfig, SshKeySource,
    TelnetConfig, VncConfig,
};

use super::traits::{ImportResult, ImportSource, SkippedEntry, read_import_file};

/// SecureCRT protocol identifiers as stored in `S:"Protocol Name"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScrtProtocol {
    Ssh2,
    Ssh1,
    Telnet,
    Rdp,
    Vnc,
    Serial,
    Rlogin,
    Raw,
}

impl ScrtProtocol {
    fn from_str(s: &str) -> Option<Self> {
        match s.trim() {
            "SSH2" => Some(Self::Ssh2),
            "SSH1" => Some(Self::Ssh1),
            "Telnet" => Some(Self::Telnet),
            "RDP" => Some(Self::Rdp),
            "VNC" => Some(Self::Vnc),
            "Serial" => Some(Self::Serial),
            "Rlogin" => Some(Self::Rlogin),
            "Raw" => Some(Self::Raw),
            _ => None,
        }
    }
}

/// Parsed key-value entries from a SecureCRT `.ini` file.
#[derive(Debug, Default)]
struct ScrtSession {
    protocol: Option<String>,
    hostname: Option<String>,
    port: Option<u32>,
    username: Option<String>,
    identity_file: Option<String>,
    auth_methods: Option<String>,
    x11_forwarding: bool,
    agent_forwarding: bool,
    compression: bool,
    description: Option<String>,
    emulation: Option<String>,
}

/// Importer for SecureCRT session files.
///
/// SecureCRT stores sessions as individual `.ini` files in a directory tree.
/// The directory structure represents the session folder hierarchy.
pub struct SecureCrtImporter {
    /// Custom path to import from (directory or single file)
    custom_path: Option<PathBuf>,
}

impl SecureCrtImporter {
    /// Creates a new SecureCRT importer.
    #[must_use]
    pub const fn new() -> Self {
        Self { custom_path: None }
    }

    /// Creates a new importer with a custom path.
    #[must_use]
    pub const fn with_path(path: PathBuf) -> Self {
        Self {
            custom_path: Some(path),
        }
    }

    /// Parses a DWORD value from SecureCRT hex format (e.g., "00000016" → 22).
    fn parse_dword(s: &str) -> Option<u32> {
        u32::from_str_radix(s.trim(), 16).ok()
    }

    /// Extracts the value from a SecureCRT INI line.
    ///
    /// Lines have the format: `TYPE:"Key Name"=value`
    /// where TYPE is S, D, or B.
    fn parse_ini_line(line: &str) -> Option<(&str, &str, &str)> {
        // Format: X:"Key"=Value
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }

        // Get type prefix (S, D, B)
        let type_prefix = line.get(..2)?;
        if !matches!(type_prefix, "S:" | "D:" | "B:" | "Z:") {
            return None;
        }
        let value_type = &type_prefix[..1];

        // Find key between quotes
        let rest = &line[2..];
        if !rest.starts_with('"') {
            return None;
        }
        let end_quote = rest[1..].find('"')?;
        let key = &rest[1..=end_quote];

        // Get value after '='
        let after_key = &rest[end_quote + 2..];
        if !after_key.starts_with('=') {
            return None;
        }
        let value = &after_key[1..];

        Some((value_type, key, value))
    }

    /// Parses a single SecureCRT `.ini` file into a session struct.
    fn parse_ini_content(content: &str) -> ScrtSession {
        let mut session = ScrtSession::default();

        for line in content.lines() {
            let Some((value_type, key, value)) = Self::parse_ini_line(line) else {
                continue;
            };

            match (value_type, key) {
                ("S", "Protocol Name") => {
                    session.protocol = Some(value.to_string());
                }
                ("S", "Hostname") if !value.is_empty() => {
                    session.hostname = Some(value.to_string());
                }
                ("S", "Username") if !value.is_empty() => {
                    session.username = Some(value.to_string());
                }
                ("D", "[SSH2] Port") => {
                    session.port = Self::parse_dword(value);
                }
                ("D", "[SSH1] Port") if session.port.is_none() => {
                    session.port = Self::parse_dword(value);
                }
                ("D", "Port") if session.port.is_none() => {
                    session.port = Self::parse_dword(value);
                }
                ("S", "Identity Filename" | "Identity Filename V2") if !value.is_empty() => {
                    session.identity_file = Some(value.to_string());
                }
                ("S", "SSH2 Authentications V2") if !value.is_empty() => {
                    session.auth_methods = Some(value.to_string());
                }
                ("D", "Forward X11") => {
                    session.x11_forwarding = Self::parse_dword(value) == Some(1);
                }
                ("D", "Enable Agent Forwarding") => {
                    // Value 2 means "use global setting", 1 means enabled
                    session.agent_forwarding = Self::parse_dword(value) == Some(1);
                }
                ("S", "Compression List") => {
                    // If compression list is not "none", compression is enabled
                    session.compression = !value.is_empty() && value != "none";
                }
                ("S", "Description") if !value.is_empty() => {
                    session.description = Some(value.replace("\\r", "\n"));
                }
                ("S", "Emulation") if !value.is_empty() => {
                    session.emulation = Some(value.to_string());
                }
                _ => {}
            }
        }

        session
    }

    /// Converts a parsed session into a Connection.
    #[expect(
        clippy::too_many_lines,
        reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
    )]
    fn session_to_connection(
        session: &ScrtSession,
        name: &str,
    ) -> Result<Option<Connection>, String> {
        let protocol_str = session.protocol.as_deref().unwrap_or("SSH2");

        let Some(protocol) = ScrtProtocol::from_str(protocol_str) else {
            return Err(format!("Unknown protocol: {protocol_str}"));
        };

        // Skip unsupported protocols
        match protocol {
            ScrtProtocol::Rlogin | ScrtProtocol::Raw => return Ok(None),
            _ => {}
        }

        let hostname = session.hostname.as_deref().unwrap_or_default();

        if hostname.is_empty() {
            return Err("No hostname specified".to_string());
        }

        let connection = match protocol {
            ScrtProtocol::Ssh2 | ScrtProtocol::Ssh1 => {
                let port = session
                    .port
                    .and_then(|p| u16::try_from(p).ok())
                    .unwrap_or(22);

                // Determine auth method
                let has_key = session.identity_file.is_some();
                let auth_methods = session.auth_methods.as_deref().unwrap_or("");
                let auth_method = if has_key
                    || (auth_methods.contains("publickey") && !auth_methods.contains("password"))
                {
                    SshAuthMethod::PublicKey
                } else {
                    SshAuthMethod::Password
                };

                let key_path = session
                    .identity_file
                    .as_ref()
                    .map(|p| PathBuf::from(p.trim()));

                let ssh_config = SshConfig {
                    auth_method,
                    key_path,
                    key_source: SshKeySource::Default,
                    agent_key_fingerprint: None,
                    identities_only: false,
                    jump_host_id: None,
                    proxy_jump: None,
                    proxy_command: None,
                    use_control_master: false,
                    agent_forwarding: session.agent_forwarding,
                    x11_forwarding: session.x11_forwarding,
                    compression: session.compression,
                    custom_options: HashMap::new(),
                    startup_command: None,
                    sftp_enabled: false,
                    port_forwards: Vec::new(),
                    waypipe: false,
                    ssh_agent_socket: None,
                    keep_alive_interval: None,
                    keep_alive_count_max: None,
                    verbose: false,
                };

                let mut conn = Connection::new(
                    name.to_string(),
                    hostname.to_string(),
                    port,
                    ProtocolConfig::Ssh(ssh_config),
                );
                conn.username = session.username.clone();
                conn.description = session.description.clone();
                conn
            }
            ScrtProtocol::Telnet => {
                let port = session
                    .port
                    .and_then(|p| u16::try_from(p).ok())
                    .unwrap_or(23);
                let mut conn = Connection::new(
                    name.to_string(),
                    hostname.to_string(),
                    port,
                    ProtocolConfig::Telnet(TelnetConfig::default()),
                );
                conn.username = session.username.clone();
                conn.description = session.description.clone();
                conn
            }
            ScrtProtocol::Rdp => {
                let port = session
                    .port
                    .and_then(|p| u16::try_from(p).ok())
                    .unwrap_or(3389);
                let mut conn = Connection::new(
                    name.to_string(),
                    hostname.to_string(),
                    port,
                    ProtocolConfig::Rdp(RdpConfig::default()),
                );
                conn.username = session.username.clone();
                conn.description = session.description.clone();
                conn
            }
            ScrtProtocol::Vnc => {
                let port = session
                    .port
                    .and_then(|p| u16::try_from(p).ok())
                    .unwrap_or(5900);
                let mut conn = Connection::new(
                    name.to_string(),
                    hostname.to_string(),
                    port,
                    ProtocolConfig::Vnc(VncConfig::default()),
                );
                conn.description = session.description.clone();
                conn
            }
            ScrtProtocol::Serial => {
                // Serial connections don't have a meaningful host/port for RustConn
                return Err("Serial connections are not supported".to_string());
            }
            ScrtProtocol::Rlogin | ScrtProtocol::Raw => unreachable!(),
        };

        Ok(Some(connection))
    }

    /// Recursively imports sessions from a directory tree.
    fn import_directory(
        &self,
        dir: &Path,
        group_id: Option<Uuid>,
        result: &mut ImportResult,
        groups: &mut HashMap<PathBuf, Uuid>,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return,
        };

        let mut entries: Vec<_> = entries.filter_map(std::result::Result::ok).collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);

        for entry in entries {
            let path = entry.path();
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();

            if path.is_dir() {
                // Skip __FolderData__ directories (SecureCRT metadata)
                if name == "__FolderData__" || name.starts_with('.') {
                    continue;
                }

                // Create a group for this directory
                let mut group = ConnectionGroup::new(name.to_string());
                let new_group_id = group.id;
                group.parent_id = group_id;
                groups.insert(path.clone(), new_group_id);
                result.add_group(group);

                // Recurse into subdirectory
                self.import_directory(&path, Some(new_group_id), result, groups);
            } else if path.extension().is_some_and(|ext| ext == "ini") {
                // Skip Default.ini if it has no hostname (it's a template)
                let session_name = name.trim_end_matches(".ini");

                // Skip the "Default" session template
                if session_name == "Default" {
                    continue;
                }

                match self.import_single_ini(&path, session_name, group_id) {
                    Ok(Some(connection)) => {
                        result.add_connection(connection);
                    }
                    Ok(None) => {
                        // Unsupported protocol, silently skip
                    }
                    Err(reason) => {
                        result.add_skipped(SkippedEntry::with_location(
                            session_name,
                            reason,
                            path.display().to_string(),
                        ));
                    }
                }
            }
        }
    }

    /// Imports a single `.ini` file.
    #[expect(
        clippy::unused_self,
        reason = "method is part of a uniform helper API where most operations need &self; keeping &self preserves the consistent signature"
    )]
    fn import_single_ini(
        &self,
        path: &Path,
        name: &str,
        group_id: Option<Uuid>,
    ) -> Result<Option<Connection>, String> {
        let content = read_import_file(path, "SecureCRT").map_err(|e| e.to_string())?;
        let session = Self::parse_ini_content(&content);
        let mut connection = Self::session_to_connection(&session, name)?;

        if let Some(ref mut conn) = connection {
            conn.group_id = group_id;
            conn.tags.push("imported:securecrt".to_string());
        }

        Ok(connection)
    }
}

impl Default for SecureCrtImporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportSource for SecureCrtImporter {
    fn source_id(&self) -> &'static str {
        "securecrt"
    }

    fn display_name(&self) -> &'static str {
        "SecureCRT"
    }

    fn is_available(&self) -> bool {
        if let Some(ref path) = self.custom_path {
            return path.exists();
        }
        self.default_paths().iter().any(|p| p.exists())
    }

    fn default_paths(&self) -> Vec<PathBuf> {
        if let Some(ref path) = self.custom_path {
            return vec![path.clone()];
        }

        let mut paths = Vec::new();

        // Linux: ~/.vandyke/Config/Sessions
        if let Some(home) = dirs::home_dir() {
            let linux_path = home.join(".vandyke").join("Config").join("Sessions");
            if linux_path.exists() {
                paths.push(linux_path);
            }

            // Alternative Linux path
            let alt_path = home
                .join(".config")
                .join("VanDyke")
                .join("Config")
                .join("Sessions");
            if alt_path.exists() {
                paths.push(alt_path);
            }
        }

        paths
    }

    fn import(&self) -> Result<ImportResult, ImportError> {
        let paths = self.default_paths();

        if paths.is_empty() {
            return Err(ImportError::FileNotFound(PathBuf::from(
                "No SecureCRT configuration found",
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
        if path.is_dir() {
            let mut result = ImportResult::new();
            let mut groups = HashMap::new();
            self.import_directory(path, None, &mut result, &mut groups);
            Ok(result)
        } else if path.is_file() {
            // Single .ini file import
            let file_name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "Unknown".to_string());

            let mut result = ImportResult::new();
            match self.import_single_ini(path, &file_name, None) {
                Ok(Some(connection)) => {
                    result.add_connection(connection);
                }
                Ok(None) => {}
                Err(reason) => {
                    result.add_skipped(SkippedEntry::with_location(
                        &file_name,
                        reason,
                        path.display().to_string(),
                    ));
                }
            }
            Ok(result)
        } else {
            Err(ImportError::FileNotFound(path.to_path_buf()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_SSH_SESSION: &str = r#"D:"Is Session"=00000001
S:"Protocol Name"=SSH2
S:"Hostname"=192.168.1.100
D:"[SSH2] Port"=00000016
S:"Username"=admin
S:"Identity Filename"=/home/user/.ssh/id_rsa
S:"SSH2 Authentications V2"=publickey,password,keyboard-interactive
D:"Forward X11"=00000001
D:"Enable Agent Forwarding"=00000001
S:"Emulation"=XTerm
S:"Description"=Production server\rMain datacenter
"#;

    const SAMPLE_TELNET_SESSION: &str = r#"D:"Is Session"=00000001
S:"Protocol Name"=Telnet
S:"Hostname"=10.0.0.1
D:"Port"=00000017
S:"Username"=
S:"Emulation"=VT100
"#;

    const SAMPLE_RDP_SESSION: &str = r#"D:"Is Session"=00000001
S:"Protocol Name"=RDP
S:"Hostname"=windows-server.local
D:"Port"=00000d3d
S:"Username"=Administrator
"#;

    const SAMPLE_NO_HOST: &str = r#"D:"Is Session"=00000001
S:"Protocol Name"=SSH2
S:"Hostname"=
D:"[SSH2] Port"=00000016
S:"Username"=
"#;

    #[test]
    fn test_parse_dword() {
        assert_eq!(SecureCrtImporter::parse_dword("00000016"), Some(22));
        assert_eq!(SecureCrtImporter::parse_dword("00000d3d"), Some(3389));
        assert_eq!(SecureCrtImporter::parse_dword("00000017"), Some(23));
        assert_eq!(SecureCrtImporter::parse_dword("0000168c"), Some(5772));
        assert_eq!(SecureCrtImporter::parse_dword("00000001"), Some(1));
        assert_eq!(SecureCrtImporter::parse_dword("00000000"), Some(0));
    }

    #[test]
    fn test_parse_ini_line() {
        let (t, k, v) = SecureCrtImporter::parse_ini_line(r#"S:"Protocol Name"=SSH2"#).unwrap();
        assert_eq!(t, "S");
        assert_eq!(k, "Protocol Name");
        assert_eq!(v, "SSH2");

        let (t, k, v) = SecureCrtImporter::parse_ini_line(r#"D:"[SSH2] Port"=00000016"#).unwrap();
        assert_eq!(t, "D");
        assert_eq!(k, "[SSH2] Port");
        assert_eq!(v, "00000016");

        let (t, k, v) = SecureCrtImporter::parse_ini_line(r#"S:"Hostname"=192.168.1.100"#).unwrap();
        assert_eq!(t, "S");
        assert_eq!(k, "Hostname");
        assert_eq!(v, "192.168.1.100");

        // Empty lines and comments
        assert!(SecureCrtImporter::parse_ini_line("").is_none());
        assert!(SecureCrtImporter::parse_ini_line("# comment").is_none());
    }

    #[test]
    fn test_parse_ssh_session() {
        let session = SecureCrtImporter::parse_ini_content(SAMPLE_SSH_SESSION);
        assert_eq!(session.protocol.as_deref(), Some("SSH2"));
        assert_eq!(session.hostname.as_deref(), Some("192.168.1.100"));
        assert_eq!(session.port, Some(22));
        assert_eq!(session.username.as_deref(), Some("admin"));
        assert_eq!(
            session.identity_file.as_deref(),
            Some("/home/user/.ssh/id_rsa")
        );
        assert!(session.x11_forwarding);
        assert!(session.agent_forwarding);
    }

    #[test]
    fn test_session_to_connection_ssh() {
        let session = SecureCrtImporter::parse_ini_content(SAMPLE_SSH_SESSION);
        let conn = SecureCrtImporter::session_to_connection(&session, "Prod Server")
            .unwrap()
            .unwrap();

        assert_eq!(conn.name, "Prod Server");
        assert_eq!(conn.host, "192.168.1.100");
        assert_eq!(conn.port, 22);
        assert_eq!(conn.username, Some("admin".to_string()));

        if let ProtocolConfig::Ssh(ssh) = &conn.protocol_config {
            assert_eq!(ssh.auth_method, SshAuthMethod::PublicKey);
            assert!(ssh.key_path.is_some());
            assert!(ssh.x11_forwarding);
            assert!(ssh.agent_forwarding);
        } else {
            panic!("Expected SSH config");
        }
    }

    #[test]
    fn test_session_to_connection_telnet() {
        let session = SecureCrtImporter::parse_ini_content(SAMPLE_TELNET_SESSION);
        let conn = SecureCrtImporter::session_to_connection(&session, "Switch")
            .unwrap()
            .unwrap();

        assert_eq!(conn.name, "Switch");
        assert_eq!(conn.host, "10.0.0.1");
        assert_eq!(conn.port, 23);
        assert!(matches!(conn.protocol_config, ProtocolConfig::Telnet(_)));
    }

    #[test]
    fn test_session_to_connection_rdp() {
        let session = SecureCrtImporter::parse_ini_content(SAMPLE_RDP_SESSION);
        let conn = SecureCrtImporter::session_to_connection(&session, "Windows")
            .unwrap()
            .unwrap();

        assert_eq!(conn.name, "Windows");
        assert_eq!(conn.host, "windows-server.local");
        assert_eq!(conn.port, 3389);
        assert_eq!(conn.username, Some("Administrator".to_string()));
        assert!(matches!(conn.protocol_config, ProtocolConfig::Rdp(_)));
    }

    #[test]
    fn test_session_no_hostname_skipped() {
        let session = SecureCrtImporter::parse_ini_content(SAMPLE_NO_HOST);
        let result = SecureCrtImporter::session_to_connection(&session, "Empty");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No hostname"));
    }

    #[test]
    fn test_unsupported_protocol_returns_none() {
        let content = r#"D:"Is Session"=00000001
S:"Protocol Name"=Rlogin
S:"Hostname"=legacy.host
D:"Port"=00000201
"#;
        let session = SecureCrtImporter::parse_ini_content(content);
        let result = SecureCrtImporter::session_to_connection(&session, "Legacy");
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_source_metadata() {
        let importer = SecureCrtImporter::new();
        assert_eq!(importer.source_id(), "securecrt");
        assert_eq!(importer.display_name(), "SecureCRT");
    }

    #[test]
    fn test_import_from_path_single_file() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("MyServer.ini");
        let mut file = std::fs::File::create(&file_path).unwrap();
        write!(file, "{SAMPLE_SSH_SESSION}").unwrap();

        let importer = SecureCrtImporter::with_path(file_path.clone());
        let result = importer.import_from_path(&file_path).unwrap();

        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.connections[0].name, "MyServer");
        assert_eq!(result.connections[0].host, "192.168.1.100");
    }

    #[test]
    fn test_import_from_directory() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let sessions_dir = dir.path().join("Sessions");
        std::fs::create_dir(&sessions_dir).unwrap();

        // Create a session file
        let mut f = std::fs::File::create(sessions_dir.join("Server1.ini")).unwrap();
        write!(f, "{SAMPLE_SSH_SESSION}").unwrap();

        // Create a subfolder with a session
        let sub_dir = sessions_dir.join("Production");
        std::fs::create_dir(&sub_dir).unwrap();
        let mut f = std::fs::File::create(sub_dir.join("WebServer.ini")).unwrap();
        write!(f, "{SAMPLE_TELNET_SESSION}").unwrap();

        // Create Default.ini (should be skipped)
        let mut f = std::fs::File::create(sessions_dir.join("Default.ini")).unwrap();
        write!(f, "{SAMPLE_NO_HOST}").unwrap();

        let importer = SecureCrtImporter::with_path(sessions_dir.clone());
        let result = importer.import_from_path(&sessions_dir).unwrap();

        assert_eq!(result.connections.len(), 2);
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].name, "Production");

        // Check that the subfolder connection has the group assigned
        let web_conn = result
            .connections
            .iter()
            .find(|c| c.name == "WebServer")
            .unwrap();
        assert_eq!(web_conn.group_id, Some(result.groups[0].id));
    }

    #[test]
    fn test_description_multiline() {
        let content = r#"D:"Is Session"=00000001
S:"Protocol Name"=SSH2
S:"Hostname"=test.host
D:"[SSH2] Port"=00000016
S:"Description"=Line one\rLine two\rLine three
"#;
        let session = SecureCrtImporter::parse_ini_content(content);
        assert_eq!(
            session.description.as_deref(),
            Some("Line one\nLine two\nLine three")
        );
    }
}
