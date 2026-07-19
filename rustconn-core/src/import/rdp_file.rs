//! Microsoft `.rdp` file parser.
//!
//! Parses Remote Desktop Protocol connection files (`.rdp` format).
//! The format uses `key:type:value` lines where type is `s` (string)
//! or `i` (integer).
//!
//! # Supported Fields
//!
//! - `full address` — host\[:port\]
//! - `username` — login name
//! - `domain` — Windows domain
//! - `gatewayhostname` — RD Gateway server
//! - `gatewayusagemethod` — 1 = always use gateway
//! - `desktopwidth` / `desktopheight` — resolution
//! - `screen mode id` — 1 = windowed, 2 = fullscreen
//! - `audiomode` — 0 = local, 1 = remote, 2 = none
//! - `redirectclipboard` — 0/1
//! - `redirectprinters` — 0/1

use std::collections::HashMap;
use std::path::Path;

use super::traits::{ImportResult, ImportSource, read_import_file};
use crate::error::ImportError;
use crate::models::{Connection, ProtocolConfig, RdpConfig, RdpGateway, Resolution};

/// Parsed contents of an `.rdp` file.
#[derive(Debug, Default)]
struct RdpFileFields {
    fields: HashMap<String, String>,
}

impl RdpFileFields {
    fn parse(content: &str) -> Self {
        let mut fields = HashMap::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // Format: key:type:value  (type = s|i|b)
            let parts: Vec<&str> = line.splitn(3, ':').collect();
            if parts.len() == 3 {
                let key = parts[0].trim().to_lowercase();
                let value = parts[2].trim().to_string();
                fields.insert(key, value);
            }
        }
        Self { fields }
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }

    fn get_u16(&self, key: &str) -> Option<u16> {
        self.get(key).and_then(|v| v.parse().ok())
    }

    fn get_u32(&self, key: &str) -> Option<u32> {
        self.get(key).and_then(|v| v.parse().ok())
    }

    fn get_bool(&self, key: &str) -> Option<bool> {
        self.get(key).map(|v| v == "1")
    }
}

/// Importer for Microsoft `.rdp` connection files.
pub struct RdpFileImporter;

impl RdpFileImporter {
    /// Creates a new `.rdp` file importer.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Parses a single `.rdp` file into a `Connection`.
    ///
    /// # Errors
    ///
    /// Returns `ImportError` if the file cannot be read or has no
    /// `full address` field.
    pub fn parse_rdp_file(path: &Path) -> Result<Connection, ImportError> {
        let content = read_import_file(path, "RDP file")?;
        let fields = RdpFileFields::parse(&content);

        let full_address = fields
            .get("full address")
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ImportError::ParseError {
                source_name: "RDP file".to_string(),
                reason: format!("Missing 'full address' in {}", path.display()),
            })?;

        let (host, port) = parse_rdp_address(full_address);

        let username = fields
            .get("username")
            .filter(|s| !s.is_empty())
            .map(String::from);
        let domain = fields
            .get("domain")
            .filter(|s| !s.is_empty())
            .map(String::from);

        // Resolution
        let resolution = match (
            fields.get_u32("desktopwidth"),
            fields.get_u32("desktopheight"),
        ) {
            (Some(w), Some(h)) if w > 0 && h > 0 => Some(Resolution {
                width: w,
                height: h,
            }),
            _ => None,
        };

        // Audio
        let audio_redirect = fields.get("audiomode").is_some_and(|v| v == "0");

        // Clipboard
        let clipboard = fields.get_bool("redirectclipboard").unwrap_or(true);

        // Printer redirection
        let printer_enabled = fields.get_bool("redirectprinters").unwrap_or(false);

        // Gateway
        let gateway = fields
            .get("gatewayhostname")
            .filter(|s| !s.is_empty())
            .map(|gw_host| {
                let gw_port = fields.get_u16("gatewayport").unwrap_or(443);
                RdpGateway {
                    hostname: gw_host.to_string(),
                    port: gw_port,
                    username: fields
                        .get("gatewaycredentialssource")
                        .and_then(|_| username.clone()),
                }
            });

        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("RDP Connection")
            .to_string();

        let rdp_config = ProtocolConfig::Rdp(RdpConfig {
            resolution,
            audio_redirect,
            gateway,
            clipboard_enabled: clipboard,
            printer_enabled,
            remote_app_program: fields
                .get("remoteapplicationprogram")
                .filter(|s| !s.is_empty())
                .map(String::from),
            remote_app_args: fields
                .get("remoteapplicationcmdline")
                .filter(|s| !s.is_empty())
                .map(String::from),
            remote_app_name: fields
                .get("remoteapplicationname")
                .filter(|s| !s.is_empty())
                .map(String::from),
            ..Default::default()
        });

        let mut connection = Connection::new(name, host, port, rdp_config);
        connection.domain = domain;

        if let Some(user) = username {
            connection.username = Some(user);
        }

        // Tag with import source
        connection.tags.push("imported:rdp-file".to_string());

        Ok(connection)
    }
}

impl Default for RdpFileImporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportSource for RdpFileImporter {
    fn source_id(&self) -> &'static str {
        "rdp-file"
    }

    fn display_name(&self) -> &'static str {
        "RDP File (.rdp)"
    }

    fn is_available(&self) -> bool {
        // Always available — user provides the file path
        true
    }

    fn default_paths(&self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }

    fn import(&self) -> Result<ImportResult, ImportError> {
        Err(ImportError::FileNotFound(std::path::PathBuf::from(
            "RDP file importer requires a specific file path",
        )))
    }

    fn import_from_path(&self, path: &Path) -> Result<ImportResult, ImportError> {
        let connection = Self::parse_rdp_file(path)?;
        let mut result = ImportResult::new();
        result.connections.push(connection);
        Ok(result)
    }
}

/// Parses `host:port` or `host` from the `full address` field.
fn parse_rdp_address(address: &str) -> (String, u16) {
    if let Some((host, port_str)) = address.rsplit_once(':')
        && let Ok(port) = port_str.parse::<u16>()
    {
        return (host.to_string(), port);
    }
    (address.to_string(), 3389)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn test_parse_rdp_address_with_port() {
        let (host, port) = parse_rdp_address("server.example.com:3390");
        assert_eq!(host, "server.example.com");
        assert_eq!(port, 3390);
    }

    #[test]
    fn test_parse_rdp_address_default_port() {
        let (host, port) = parse_rdp_address("server.example.com");
        assert_eq!(host, "server.example.com");
        assert_eq!(port, 3389);
    }

    #[test]
    fn test_parse_rdp_fields() {
        let content = "\
full address:s:myserver.example.com:3390
username:s:admin
domain:s:CORP
desktopwidth:i:1920
desktopheight:i:1080
audiomode:i:0
redirectclipboard:i:1
gatewayhostname:s:gw.example.com
";
        let fields = RdpFileFields::parse(content);
        assert_eq!(
            fields.get("full address"),
            Some("myserver.example.com:3390")
        );
        assert_eq!(fields.get("username"), Some("admin"));
        assert_eq!(fields.get("domain"), Some("CORP"));
        assert_eq!(fields.get_u32("desktopwidth"), Some(1920));
        assert_eq!(fields.get_bool("redirectclipboard"), Some(true));
        assert_eq!(fields.get("gatewayhostname"), Some("gw.example.com"));
    }

    #[test]
    fn test_parse_rdp_file_minimal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rdp");
        fs::write(&path, "full address:s:server.example.com\n").unwrap();

        let conn = RdpFileImporter::parse_rdp_file(&path).unwrap();
        assert_eq!(conn.host, "server.example.com");
        assert_eq!(conn.port, 3389);
        assert_eq!(conn.name, "test");
    }

    #[test]
    fn test_parse_rdp_file_with_gateway() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corp.rdp");
        let content = "\
full address:s:internal.corp.com:3390
username:s:jdoe
domain:s:CORP
gatewayhostname:s:gateway.corp.com
gatewayusagemethod:i:1
desktopwidth:i:1920
desktopheight:i:1080
";
        fs::write(&path, content).unwrap();

        let conn = RdpFileImporter::parse_rdp_file(&path).unwrap();
        assert_eq!(conn.host, "internal.corp.com");
        assert_eq!(conn.port, 3390);
        assert_eq!(conn.domain, Some("CORP".to_string()));

        if let ProtocolConfig::Rdp(ref rdp) = conn.protocol_config {
            assert!(rdp.gateway.is_some());
            let gw = rdp.gateway.as_ref().unwrap();
            assert_eq!(gw.hostname, "gateway.corp.com");
            assert_eq!(gw.port, 443);
            assert_eq!(rdp.resolution.as_ref().unwrap().width, 1920);
        } else {
            panic!("Expected RDP protocol config");
        }
    }

    #[test]
    fn test_parse_rdp_file_missing_address() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.rdp");
        fs::write(&path, "username:s:admin\n").unwrap();

        let result = RdpFileImporter::parse_rdp_file(&path);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_rdp_file_with_remoteapp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("remoteapp.rdp");
        let content = "\
full address:s:rdserver.corp.com
username:s:user1
remoteapplicationprogram:s:||notepad
remoteapplicationcmdline:s:/p C:\\docs\\readme.txt
remoteapplicationname:s:Notepad Editor
";
        fs::write(&path, content).unwrap();

        let conn = RdpFileImporter::parse_rdp_file(&path).unwrap();
        assert_eq!(conn.host, "rdserver.corp.com");
        assert_eq!(conn.port, 3389);

        if let ProtocolConfig::Rdp(ref rdp) = conn.protocol_config {
            assert_eq!(rdp.remote_app_program.as_deref(), Some("||notepad"));
            assert_eq!(
                rdp.remote_app_args.as_deref(),
                Some("/p C:\\docs\\readme.txt")
            );
            assert_eq!(rdp.remote_app_name.as_deref(), Some("Notepad Editor"));
            assert!(rdp.is_remote_app());
            assert!(rdp.requires_freerdp_fallback());
        } else {
            panic!("Expected RDP protocol config");
        }
    }

    #[test]
    fn test_parse_rdp_file_remoteapp_empty_fields_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty_app.rdp");
        let content = "\
full address:s:server.example.com
remoteapplicationprogram:s:
remoteapplicationcmdline:s:
remoteapplicationname:s:
";
        fs::write(&path, content).unwrap();

        let conn = RdpFileImporter::parse_rdp_file(&path).unwrap();

        if let ProtocolConfig::Rdp(ref rdp) = conn.protocol_config {
            assert!(rdp.remote_app_program.is_none());
            assert!(rdp.remote_app_args.is_none());
            assert!(rdp.remote_app_name.is_none());
            assert!(!rdp.is_remote_app());
        } else {
            panic!("Expected RDP protocol config");
        }
    }
}
