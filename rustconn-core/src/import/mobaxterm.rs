//! MobaXterm session file importer.
//!
//! Parses `.mxtsessions` files exported from MobaXterm.
//! Supports SSH, RDP, VNC, and SFTP session types.
//!
//! File format: INI-style with Windows-1252 encoding and CRLF line endings.
//! Sessions are stored as key=value pairs where the value contains
//! `#`-separated fields with `%`-separated sub-fields.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::ImportError;
use crate::models::{
    Connection, ConnectionGroup, ProtocolConfig, RdpConfig, Resolution, SshAuthMethod, SshConfig,
    SshKeySource, TelnetConfig, VncConfig,
};

use super::traits::{ImportResult, ImportSource, SkippedEntry};

/// MobaXterm session type identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MobaSessionType {
    Ssh = 0,
    Telnet = 1,
    Rsh = 2,
    Xdmcp = 3,
    Rdp = 4,
    Vnc = 5,
    Ftp = 6,
    Sftp = 7,
    Serial = 8,
    File = 9,
    Shell = 10,
    Browser = 11,
    Mosh = 12,
    AwsS3 = 13,
    Wsl = 14,
}

impl MobaSessionType {
    fn from_id(id: u8) -> Option<Self> {
        match id {
            0 => Some(Self::Ssh),
            1 => Some(Self::Telnet),
            2 => Some(Self::Rsh),
            3 => Some(Self::Xdmcp),
            4 => Some(Self::Rdp),
            5 => Some(Self::Vnc),
            6 => Some(Self::Ftp),
            7 => Some(Self::Sftp),
            8 => Some(Self::Serial),
            9 => Some(Self::File),
            10 => Some(Self::Shell),
            11 => Some(Self::Browser),
            12 => Some(Self::Mosh),
            13 => Some(Self::AwsS3),
            14 => Some(Self::Wsl),
            _ => None,
        }
    }
}

/// Importer for MobaXterm `.mxtsessions` files.
///
/// MobaXterm stores sessions in INI format with a custom encoding scheme.
/// Each session is a single line with fields separated by `#` and sub-fields by `%`.
pub struct MobaXtermImporter {
    /// Custom path to import from
    custom_path: Option<PathBuf>,
}

impl MobaXtermImporter {
    /// Creates a new MobaXterm importer.
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

    /// Decodes MobaXterm escape sequences in a string.
    fn decode_escapes(s: &str) -> String {
        s.replace("__PERCENT__", "%")
            .replace("__DIEZE__", "#")
            .replace("__PTVIRG__", ";")
            .replace("__DBLQUO__", "\"")
            .replace("__PIPE__", "|")
            .replace("_CurrentDrive_", "C:")
            .replace("_ProfileDir_", "~")
    }

    /// Parses the content of a `.mxtsessions` file.
    #[must_use]
    pub fn parse_content(&self, content: &str, source_path: &str) -> ImportResult {
        let mut result = ImportResult::new();
        let mut groups: HashMap<String, ConnectionGroup> = HashMap::new();
        let mut current_section: Option<String> = None;
        let mut current_subrep: Option<String> = None;

        for line in content.lines() {
            let line = line.trim();

            // Skip empty lines
            if line.is_empty() {
                continue;
            }

            // Check for section header
            if line.starts_with('[') && line.ends_with(']') {
                current_section = Some(line[1..line.len() - 1].to_string());
                current_subrep = None;
                continue;
            }

            // Parse key=value
            let Some(eq_pos) = line.find('=') else {
                continue;
            };

            let key = line[..eq_pos].trim();
            let value = line[eq_pos + 1..].trim();

            // Handle folder metadata
            if key == "SubRep" {
                current_subrep = if value.is_empty() {
                    None
                } else {
                    Some(Self::decode_escapes(value))
                };
                continue;
            }

            if key == "ImgNum" {
                continue; // Skip icon number
            }

            // This is a session line
            if let Some(ref section) = current_section
                && section.starts_with("Bookmarks")
            {
                // Determine group path
                let group_path = current_subrep.clone();

                // Create or get group
                let group_id = if let Some(ref path) = group_path {
                    if let std::collections::hash_map::Entry::Vacant(e) = groups.entry(path.clone())
                    {
                        let group = ConnectionGroup::new(path.clone());
                        let id = group.id;
                        e.insert(group.clone());
                        result.add_group(group);
                        Some(id)
                    } else {
                        groups.get(path).map(|g| g.id)
                    }
                } else {
                    None
                };

                // Parse session
                match self.parse_session(key, value, source_path) {
                    Ok(Some(mut connection)) => {
                        connection.group_id = group_id;
                        result.add_connection(connection);
                    }
                    Ok(None) => {
                        // Unsupported session type, already logged
                    }
                    Err(reason) => {
                        result.add_skipped(SkippedEntry::with_location(key, reason, source_path));
                    }
                }
            }
        }

        result
    }

    /// Parses a single session line.
    fn parse_session(
        &self,
        name: &str,
        value: &str,
        source_path: &str,
    ) -> Result<Option<Connection>, String> {
        let name = Self::decode_escapes(name);

        // Split by '#' to get main fields
        let fields: Vec<&str> = value.split('#').collect();
        if fields.len() < 3 {
            return Err("Invalid session format: too few fields".to_string());
        }

        // fields[0] = reconnect flag (";  logout" or empty)
        // fields[1] = icon number
        // fields[2] = connection parameters (%-separated)
        // fields[3] = terminal parameters (%-separated) - optional
        // fields[4] = start mode - optional
        // fields[5] = comment - optional
        // fields[6] = tab color - optional

        let conn_params: Vec<&str> = fields[2].split('%').collect();
        if conn_params.is_empty() {
            return Err("Invalid session format: no connection parameters".to_string());
        }

        // First param is session type
        let session_type_id: u8 = conn_params[0]
            .parse()
            .map_err(|_| format!("Invalid session type: {}", conn_params[0]))?;

        let Some(session_type) = MobaSessionType::from_id(session_type_id) else {
            return Err(format!("Unknown session type ID: {session_type_id}"));
        };

        match session_type {
            MobaSessionType::Ssh | MobaSessionType::Sftp => {
                self.parse_ssh_session(&name, &conn_params, source_path)
            }
            MobaSessionType::Rdp => self.parse_rdp_session(&name, &conn_params, source_path),
            MobaSessionType::Vnc => self.parse_vnc_session(&name, &conn_params, source_path),
            MobaSessionType::Telnet => self.parse_telnet_session(&name, &conn_params, source_path),
            _ => {
                // Unsupported session type - return None without error
                Ok(None)
            }
        }
    }

    /// Parses an SSH/SFTP session.
    fn parse_ssh_session(
        &self,
        name: &str,
        params: &[&str],
        _source_path: &str,
    ) -> Result<Option<Connection>, String> {
        // SSH params layout:
        // [0] = session type (0 for SSH, 7 for SFTP)
        // [1] = host
        // [2] = port
        // [3] = username
        // [5] = X11 forwarding (-1=on, 0=off)
        // [6] = compression (-1=on, 0=off)
        // [7] = startup command
        // [14] = private key path
        // [33] = SSH agent (-1=on, 0=off)
        // [34] = agent forwarding (-1=on, 0=off)

        let host = params
            .get(1)
            .map(|s| Self::decode_escapes(s))
            .unwrap_or_default();
        if host.is_empty() {
            return Err("No host specified".to_string());
        }

        let port: u16 = params.get(2).and_then(|s| s.parse().ok()).unwrap_or(22);

        let username = params
            .get(3)
            .filter(|s| !s.is_empty() && **s != "<default>")
            .map(|s| Self::decode_escapes(s));

        // Parse SSH-specific options
        let key_path = params
            .get(14)
            .filter(|s| !s.is_empty())
            .map(|s| PathBuf::from(Self::decode_escapes(s)));

        let use_agent = params.get(33).map(|s| *s == "-1").unwrap_or(false);

        let agent_forwarding = params.get(34).map(|s| *s == "-1").unwrap_or(false);

        let startup_command = params
            .get(7)
            .filter(|s| !s.is_empty())
            .map(|s| Self::decode_escapes(s));

        // Determine auth method
        let auth_method = if key_path.is_some() {
            SshAuthMethod::PublicKey
        } else if use_agent {
            SshAuthMethod::Agent
        } else {
            SshAuthMethod::Password
        };

        // Parse X11 forwarding and compression from MobaXterm params
        let x11_forwarding = params.get(5).map(|s| *s == "-1").unwrap_or(false);
        let compression = params.get(6).map(|s| *s == "-1").unwrap_or(false);

        let ssh_config = SshConfig {
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
            startup_command,
            sftp_enabled: false,
            port_forwards: Vec::new(),
        };

        let mut connection = Connection::new(
            name.to_string(),
            host,
            port,
            ProtocolConfig::Ssh(ssh_config),
        );
        connection.username = username;

        Ok(Some(connection))
    }

    /// Parses an RDP session.
    fn parse_rdp_session(
        &self,
        name: &str,
        params: &[&str],
        _source_path: &str,
    ) -> Result<Option<Connection>, String> {
        // RDP params layout:
        // [0] = session type (4)
        // [1] = host
        // [2] = port
        // [3] = username
        // [10] = resolution (0=Fit, 1=Screen, 2=640x480, ..., 11=1920x1080, ...)
        // [16] = audio (0=none, 1=redirect, 2=remote)
        // [19] = clipboard (-1=on, 0=off)
        // [28] = color depth (0=auto, 1=8bit, 2=16bit, 3=24bit, 4=32bit)

        let host = params
            .get(1)
            .map(|s| Self::decode_escapes(s))
            .unwrap_or_default();
        if host.is_empty() {
            return Err("No host specified".to_string());
        }

        let port: u16 = params.get(2).and_then(|s| s.parse().ok()).unwrap_or(3389);

        let username = params
            .get(3)
            .filter(|s| !s.is_empty())
            .map(|s| Self::decode_escapes(s));

        // Parse resolution
        let resolution = params.get(10).and_then(|s| {
            match *s {
                "2" => Some(Resolution::new(640, 480)),
                "3" => Some(Resolution::new(800, 600)),
                "4" => Some(Resolution::new(1024, 768)),
                "5" => Some(Resolution::new(1152, 864)),
                "6" => Some(Resolution::new(1280, 720)),
                "7" => Some(Resolution::new(1280, 968)),
                "8" => Some(Resolution::new(1280, 1024)),
                "9" => Some(Resolution::new(1400, 1050)),
                "10" => Some(Resolution::new(1600, 1200)),
                "11" => Some(Resolution::new(1920, 1080)),
                "14" => Some(Resolution::new(1920, 1200)),
                "24" => Some(Resolution::new(2560, 1440)),
                "26" => Some(Resolution::new(3840, 2160)),
                _ => None, // Fit to terminal/screen - no fixed resolution
            }
        });

        // Parse color depth
        let color_depth = params.get(28).and_then(|s| {
            match *s {
                "1" => Some(8),
                "2" => Some(16),
                "3" => Some(24),
                "4" => Some(32),
                _ => None, // Auto
            }
        });

        // Parse audio redirect
        let audio_redirect = params.get(16).map(|s| *s == "1").unwrap_or(false);

        let rdp_config = RdpConfig {
            resolution,
            color_depth,
            audio_redirect,
            ..Default::default()
        };

        let mut connection = Connection::new(
            name.to_string(),
            host,
            port,
            ProtocolConfig::Rdp(rdp_config),
        );
        connection.username = username;

        Ok(Some(connection))
    }

    /// Parses a Telnet session.
    fn parse_telnet_session(
        &self,
        name: &str,
        params: &[&str],
        _source_path: &str,
    ) -> Result<Option<Connection>, String> {
        // Telnet params layout:
        // [0] = session type (1)
        // [1] = host
        // [2] = port

        let host = params
            .get(1)
            .map(|s| Self::decode_escapes(s))
            .unwrap_or_default();
        if host.is_empty() {
            return Err("No host specified".to_string());
        }

        let port: u16 = params.get(2).and_then(|s| s.parse().ok()).unwrap_or(23);

        let connection = Connection::new(
            name.to_string(),
            host,
            port,
            ProtocolConfig::Telnet(TelnetConfig::default()),
        );

        Ok(Some(connection))
    }

    /// Parses a VNC session.
    fn parse_vnc_session(
        &self,
        name: &str,
        params: &[&str],
        _source_path: &str,
    ) -> Result<Option<Connection>, String> {
        // VNC params layout:
        // [0] = session type (5)
        // [1] = host
        // [2] = port
        // [3] = auto scale (-1=on, 0=off)
        // [4] = view only (-1=on, 0=off)

        let host = params
            .get(1)
            .map(|s| Self::decode_escapes(s))
            .unwrap_or_default();
        if host.is_empty() {
            return Err("No host specified".to_string());
        }

        let port: u16 = params.get(2).and_then(|s| s.parse().ok()).unwrap_or(5900);

        // Parse VNC options
        let view_only = params.get(4).map(|s| *s == "-1").unwrap_or(false);

        let vnc_config = VncConfig {
            view_only,
            ..Default::default()
        };

        let connection = Connection::new(
            name.to_string(),
            host,
            port,
            ProtocolConfig::Vnc(vnc_config),
        );

        Ok(Some(connection))
    }
}

impl Default for MobaXtermImporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportSource for MobaXtermImporter {
    fn source_id(&self) -> &'static str {
        "mobaxterm"
    }

    fn display_name(&self) -> &'static str {
        "MobaXterm"
    }

    fn is_available(&self) -> bool {
        self.custom_path.as_ref().is_some_and(|p| p.exists())
    }

    fn default_paths(&self) -> Vec<PathBuf> {
        self.custom_path
            .as_ref()
            .map(|p| vec![p.clone()])
            .unwrap_or_default()
    }

    fn import(&self) -> Result<ImportResult, ImportError> {
        let Some(ref path) = self.custom_path else {
            return Err(ImportError::FileNotFound(PathBuf::from(
                "No MobaXterm session file specified",
            )));
        };

        self.import_from_path(path)
    }

    fn import_from_path(&self, path: &Path) -> Result<ImportResult, ImportError> {
        // Read file as bytes first to handle Windows-1252 encoding
        let bytes = fs::read(path).map_err(|e| ImportError::ParseError {
            source_name: "MobaXterm".to_string(),
            reason: format!("Failed to read {}: {}", path.display(), e),
        })?;

        // Try UTF-8 first, fall back to lossy conversion for Windows-1252
        let content = match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => {
                let bytes = e.into_bytes();
                String::from_utf8_lossy(&bytes).into_owned()
            }
        };

        Ok(self.parse_content(&content, &path.display().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_escapes() {
        assert_eq!(
            MobaXtermImporter::decode_escapes("test__PERCENT__value"),
            "test%value"
        );
        assert_eq!(
            MobaXtermImporter::decode_escapes("test__DIEZE__value"),
            "test#value"
        );
        assert_eq!(
            MobaXtermImporter::decode_escapes("test__PIPE__value"),
            "test|value"
        );
        assert_eq!(
            MobaXtermImporter::decode_escapes("_CurrentDrive_\\path"),
            "C:\\path"
        );
    }

    #[test]
    fn test_parse_ssh_session() {
        let importer = MobaXtermImporter::new();
        let content = "[Bookmarks]
SubRep=
ImgNum=42
My SSH Server=#109#0%192.168.1.100%22%admin%%-1%-1%%%%%0%0%0%%%-1%0%0%0%%1080%%0%0%1%#MobaFont%10%0%0%-1%15%236,236,236%30,30,30%180,180,192%0%-1%0%%xterm%-1%0%_Std_Colors_0_%80%24%0%0%-1%<none>%%0%0%-1%-1#0# #-1";

        let result = importer.parse_content(content, "test.mxtsessions");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert_eq!(conn.name, "My SSH Server");
        assert_eq!(conn.host, "192.168.1.100");
        assert_eq!(conn.port, 22);
        assert_eq!(conn.username, Some("admin".to_string()));
        assert!(matches!(conn.protocol_config, ProtocolConfig::Ssh(_)));
    }

    #[test]
    fn test_parse_rdp_session() {
        let importer = MobaXtermImporter::new();
        // RDP session with resolution at index 10 (value "11" = 1920x1080) and color depth at index 28 (value "4" = 32bit)
        // Format: type%host%port%user%...%resolution(10)%...%audio(16)%...%clipboard(19)%...%colordepth(28)%...
        let content = "[Bookmarks]
SubRep=
ImgNum=42
Windows Server=#91#4%192.168.1.50%3389%Administrator%0%0%0%0%-1%0%11%-1%%%%%-1%0%0%0%0%0%0%0%0%0%0%0%4%0%0%0%#MobaFont%10#0# #-1";

        let result = importer.parse_content(content, "test.mxtsessions");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert_eq!(conn.name, "Windows Server");
        assert_eq!(conn.host, "192.168.1.50");
        assert_eq!(conn.port, 3389);
        assert!(matches!(conn.protocol_config, ProtocolConfig::Rdp(_)));

        if let ProtocolConfig::Rdp(rdp_config) = &conn.protocol_config {
            // Resolution 11 = 1920x1080
            assert!(rdp_config.resolution.is_some());
            let res = rdp_config.resolution.as_ref().unwrap();
            assert_eq!(res.width, 1920);
            assert_eq!(res.height, 1080);
            // Color depth 4 = 32bit
            assert_eq!(rdp_config.color_depth, Some(32));
        }
    }

    #[test]
    fn test_parse_vnc_session() {
        let importer = MobaXtermImporter::new();
        let content = "[Bookmarks]
SubRep=
ImgNum=42
VNC Desktop=#128#5%192.168.1.75%5901%-1%0%%%%%0%0%0%%%%%#MobaFont%10%0%0%-1%15%236,236,236%30,30,30%180,180,192%0%-1%0%%xterm%-1%0%_Std_Colors_0_%80%24%0%0%-1%<none>%%0%0%-1%-1#0# #-1";

        let result = importer.parse_content(content, "test.mxtsessions");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert_eq!(conn.name, "VNC Desktop");
        assert_eq!(conn.host, "192.168.1.75");
        assert_eq!(conn.port, 5901);
        assert!(matches!(conn.protocol_config, ProtocolConfig::Vnc(_)));
    }

    #[test]
    fn test_parse_with_folders() {
        let importer = MobaXtermImporter::new();
        let content = "[Bookmarks]
SubRep=
ImgNum=42
Root Server=#109#0%10.0.0.1%22%root%%-1%-1%%%%%0%0%0%%%-1%0%0%0%%1080%%0%0%1%#MobaFont%10#0# #-1
[Bookmarks_1]
SubRep=Production
ImgNum=41
Prod Server=#109#0%10.0.1.1%22%admin%%-1%-1%%%%%0%0%0%%%-1%0%0%0%%1080%%0%0%1%#MobaFont%10#0# #-1
[Bookmarks_2]
SubRep=Production\\Web
ImgNum=41
Web Server=#109#0%10.0.1.2%22%www%%-1%-1%%%%%0%0%0%%%-1%0%0%0%%1080%%0%0%1%#MobaFont%10#0# #-1";

        let result = importer.parse_content(content, "test.mxtsessions");
        assert_eq!(result.connections.len(), 3);
        assert_eq!(result.groups.len(), 2);

        // Check root connection has no group
        let root_conn = result
            .connections
            .iter()
            .find(|c| c.name == "Root Server")
            .unwrap();
        assert!(root_conn.group_id.is_none());

        // Check Production connection has group
        let prod_conn = result
            .connections
            .iter()
            .find(|c| c.name == "Prod Server")
            .unwrap();
        assert!(prod_conn.group_id.is_some());
    }

    #[test]
    fn test_skip_unsupported_session_type() {
        let importer = MobaXtermImporter::new();
        // Rsh session (type 2) - not supported
        let content = "[Bookmarks]
SubRep=
ImgNum=42
Rsh Server=#98#2%192.168.1.100%514%%%#MobaFont%10#0# #-1";

        let result = importer.parse_content(content, "test.mxtsessions");
        assert_eq!(result.connections.len(), 0);
        // Should not be in skipped either - just silently ignored
    }

    #[test]
    fn test_session_with_private_key() {
        let importer = MobaXtermImporter::new();
        // SSH session with private key at index 14
        // Indices: 0=type, 1=host, 2=port, 3=user, 4, 5=x11, 6=compress, 7=cmd, 8-10=gateway, 11, 12, 13, 14=key
        let content = "[Bookmarks]
SubRep=
ImgNum=42
Key Auth Server=#109#0%192.168.1.100%22%admin%%0%0%%%%%0%0%0%~/.ssh/id_rsa%#MobaFont%10#0# #-1";

        let result = importer.parse_content(content, "test.mxtsessions");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        if let ProtocolConfig::Ssh(ssh_config) = &conn.protocol_config {
            assert!(ssh_config.key_path.is_some(), "key_path should be Some");
            assert_eq!(ssh_config.auth_method, SshAuthMethod::PublicKey);
        } else {
            panic!("Expected SSH config");
        }
    }
}
