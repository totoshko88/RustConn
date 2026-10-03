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
//! - `gatewayhostname` — RD Gateway server as `host` or `host:port`
//! - `gatewayusagemethod` — 0 = never use the gateway, anything else enables it
//! - `desktopwidth` / `desktopheight` — resolution
//! - `screen mode id` — 1 = windowed, 2 = fullscreen
//! - `audiomode` — 0 = local, 1 = remote, 2 = none
//! - `redirectclipboard` — 0/1
//! - `redirectprinters` — 0/1
//! - `smart sizing` — 0/1, scale the session to the window
//! - `dynamic resolution` — 0/1, resize the remote desktop with the window

use std::collections::HashMap;
use std::path::Path;

use super::traits::{ImportResult, ImportSource, read_import_file};
use crate::error::ImportError;
use crate::models::{Connection, ProtocolConfig, RdpAudioMode, RdpConfig, RdpGateway, Resolution};

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

        // Audio. The .rdp `audiomode` is three-way: 0 = play on this computer
        // (Local), 1 = leave on the remote (Remote), 2 = do not play (None).
        // Map it to the three-state `audio_mode` so Remote and None round-trip
        // (the legacy `audio_redirect` boolean cannot tell them apart — both are
        // "not local" — and the export writes all three modes back out). Keep
        // `audio_redirect` as the back-compat mirror (true only for Local). A
        // missing or unrecognised value leaves both at their defaults.
        let audio_mode = match fields.get("audiomode") {
            Some("0") => Some(RdpAudioMode::Local),
            Some("1") => Some(RdpAudioMode::Remote),
            Some("2") => Some(RdpAudioMode::None),
            _ => None,
        };
        let audio_redirect = audio_mode == Some(RdpAudioMode::Local);

        // Clipboard
        let clipboard = fields.get_bool("redirectclipboard").unwrap_or(true);

        // Printer redirection
        let printer_enabled = fields.get_bool("redirectprinters").unwrap_or(false);

        // Sizing. A profile for a legacy server (e.g. Windows 2008 R2) carries
        // `smart sizing:i:1` and often `dynamic resolution:i:0`; without these
        // it imported with the defaults and opened unreadably small on a HiDPI
        // display (issue #341). Absent keys keep the defaults.
        let smart_sizing = fields.get_bool("smart sizing").unwrap_or(false);
        let dynamic_resolution = fields.get_bool("dynamic resolution").unwrap_or(true);

        let gateway = parse_gateway(&fields);

        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("RDP Connection")
            .to_string();

        let rdp_config = ProtocolConfig::Rdp(RdpConfig {
            resolution,
            audio_redirect,
            audio_mode,
            gateway,
            clipboard_enabled: clipboard,
            printer_enabled,
            dynamic_resolution,
            smart_sizing,
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

/// Default RD Gateway port (HTTPS).
const DEFAULT_GATEWAY_PORT: u16 = 443;

/// `gatewayusagemethod` value that disables the gateway entirely
/// (`TSC_PROXY_MODE_NONE_DIRECT`).
const GATEWAY_USAGE_NONE: u32 = 0;

/// Builds the RD Gateway configuration from parsed `.rdp` fields.
///
/// Returns `None` when no gateway is named or when `gatewayusagemethod` is 0,
/// which means the profile explicitly connects direct. Every other usage method
/// (always, on-failure, deployment default, bypass-local) results in a gateway
/// being configured, matching how FreeRDP treats a named gateway.
fn parse_gateway(fields: &RdpFileFields) -> Option<RdpGateway> {
    if fields.get_u32("gatewayusagemethod") == Some(GATEWAY_USAGE_NONE) {
        return None;
    }

    let raw_host = fields
        .get("gatewayhostname")
        .map(str::trim)
        .filter(|s| !s.is_empty())?;

    // MSTSC stores the port inside `gatewayhostname` (`gw.example.com:444`).
    // `gatewayport` is not part of the documented format but some third-party
    // writers emit it, so it supplies the default when no port is embedded.
    let default_port = fields
        .get_u16("gatewayport")
        .filter(|port| *port > 0)
        .unwrap_or(DEFAULT_GATEWAY_PORT);
    let (hostname, port) = split_gateway_host(raw_host, default_port);

    Some(RdpGateway {
        hostname,
        port,
        // `.rdp` has no field for a separate gateway account — MSTSC reuses the
        // session credentials, which `gatewaycredentialssource` only describes
        // the prompt style for. `None` means "same user as the session".
        // `gatewayusername` is a third-party extension and honoured when set.
        username: fields
            .get("gatewayusername")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from),
    })
}

/// Splits `host[:port]` from a `gatewayhostname` value.
///
/// Bare IPv6 literals are left intact — RD Gateway endpoints are DNS names, and
/// `::1` would otherwise be read as host `::` on port 1 — while the bracketed
/// `[addr]:port` form is split.
fn split_gateway_host(value: &str, default_port: u16) -> (String, u16) {
    if let Some(rest) = value.strip_prefix('[')
        && let Some((addr, tail)) = rest.split_once(']')
    {
        let port = tail
            .strip_prefix(':')
            .and_then(|p| p.parse().ok())
            .filter(|port| *port > 0)
            .unwrap_or(default_port);
        return (addr.to_string(), port);
    }

    if let Some((host, port_str)) = value.rsplit_once(':')
        && !host.contains(':')
        && !host.is_empty()
        && let Ok(port) = port_str.parse::<u16>()
        && port > 0
    {
        return (host.to_string(), port);
    }

    (value.to_string(), default_port)
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
    fn test_parse_rdp_file_audiomode_three_way() {
        use crate::models::RdpAudioMode;

        // The .rdp `audiomode` is three-way (0=local, 1=remote, 2=none). The
        // import used to collapse it to a bool, so Remote and None both became
        // "not redirected" and were indistinguishable on re-export. Each value
        // must now map to a distinct `effective_audio_mode()`.
        let cases = [
            ("0", RdpAudioMode::Local),
            ("1", RdpAudioMode::Remote),
            ("2", RdpAudioMode::None),
        ];
        let dir = tempfile::tempdir().unwrap();
        for (value, expected) in cases {
            let path = dir.path().join(format!("audio-{value}.rdp"));
            fs::write(
                &path,
                format!("full address:s:server.example.com\naudiomode:i:{value}\n"),
            )
            .unwrap();

            let conn = RdpFileImporter::parse_rdp_file(&path).unwrap();
            let ProtocolConfig::Rdp(ref rdp) = conn.protocol_config else {
                panic!("expected RDP protocol config for audiomode {value}");
            };
            assert_eq!(
                rdp.effective_audio_mode(),
                expected,
                "audiomode:i:{value} should import as {expected:?}"
            );
            // Back-compat mirror: the legacy boolean is true only for Local.
            assert_eq!(rdp.audio_redirect, expected == RdpAudioMode::Local);
        }
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

    /// Extracts the RDP protocol config, panicking on any other variant.
    fn rdp_config(conn: &Connection) -> &RdpConfig {
        match conn.protocol_config {
            ProtocolConfig::Rdp(ref rdp) => rdp,
            _ => panic!("Expected RDP protocol config"),
        }
    }

    fn import_rdp(dir: &std::path::Path, name: &str, content: &str) -> Connection {
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        RdpFileImporter::parse_rdp_file(&path).unwrap()
    }

    #[test]
    fn gateway_port_comes_from_the_hostname_field() {
        // MSTSC writes the gateway port inside `gatewayhostname`.
        let dir = tempfile::tempdir().unwrap();
        let conn = import_rdp(
            dir.path(),
            "corp.rdp",
            "full address:s:host.internal\ngatewayhostname:s:gw.example.com:444\n",
        );
        let gw = rdp_config(&conn).gateway.as_ref().unwrap();
        assert_eq!(gw.hostname, "gw.example.com");
        assert_eq!(gw.port, 444);
    }

    #[test]
    fn embedded_gateway_port_wins_over_gatewayport_field() {
        let dir = tempfile::tempdir().unwrap();
        let conn = import_rdp(
            dir.path(),
            "corp.rdp",
            "full address:s:host.internal\n\
             gatewayhostname:s:gw.example.com:444\n\
             gatewayport:i:8443\n",
        );
        let gw = rdp_config(&conn).gateway.as_ref().unwrap();
        assert_eq!(gw.port, 444);
    }

    #[test]
    fn gatewayport_field_is_used_without_an_embedded_port() {
        let dir = tempfile::tempdir().unwrap();
        let conn = import_rdp(
            dir.path(),
            "corp.rdp",
            "full address:s:host.internal\n\
             gatewayhostname:s:gw.example.com\n\
             gatewayport:i:8443\n",
        );
        let gw = rdp_config(&conn).gateway.as_ref().unwrap();
        assert_eq!(gw.hostname, "gw.example.com");
        assert_eq!(gw.port, 8443);
    }

    #[test]
    fn gateway_usage_method_zero_disables_the_gateway() {
        let dir = tempfile::tempdir().unwrap();
        let conn = import_rdp(
            dir.path(),
            "direct.rdp",
            "full address:s:host.internal\n\
             gatewayhostname:s:gw.example.com\n\
             gatewayusagemethod:i:0\n",
        );
        assert!(rdp_config(&conn).gateway.is_none());
    }

    #[test]
    fn gateway_username_is_not_inferred_from_credentials_source() {
        // `gatewaycredentialssource` describes the prompt style, not a separate
        // account; `None` keeps "same user as the session".
        let dir = tempfile::tempdir().unwrap();
        let conn = import_rdp(
            dir.path(),
            "corp.rdp",
            "full address:s:host.internal\n\
             username:s:jdoe\n\
             gatewayhostname:s:gw.example.com\n\
             gatewaycredentialssource:i:4\n",
        );
        let gw = rdp_config(&conn).gateway.as_ref().unwrap();
        assert_eq!(gw.username, None);
        assert_eq!(conn.username, Some("jdoe".to_string()));
    }

    #[test]
    fn explicit_gateway_username_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let conn = import_rdp(
            dir.path(),
            "corp.rdp",
            "full address:s:host.internal\n\
             username:s:jdoe\n\
             gatewayhostname:s:gw.example.com\n\
             gatewayusername:s:gwadmin\n",
        );
        let gw = rdp_config(&conn).gateway.as_ref().unwrap();
        assert_eq!(gw.username, Some("gwadmin".to_string()));
    }

    #[test]
    fn split_gateway_host_handles_ipv6_and_defaults() {
        assert_eq!(
            split_gateway_host("gw.example.com", 443),
            ("gw.example.com".to_string(), 443)
        );
        assert_eq!(
            split_gateway_host("gw.example.com:0", 443),
            ("gw.example.com:0".to_string(), 443)
        );
        // Bare IPv6 must not be split into host `::` on port 1.
        assert_eq!(split_gateway_host("::1", 443), ("::1".to_string(), 443));
        assert_eq!(
            split_gateway_host("[2001:db8::1]:444", 443),
            ("2001:db8::1".to_string(), 444)
        );
        assert_eq!(
            split_gateway_host("[2001:db8::1]", 443),
            ("2001:db8::1".to_string(), 443)
        );
    }

    /// The profile a legacy server needs: smart sizing on, dynamic resolution
    /// off (issue #341).
    #[test]
    fn sizing_keys_are_imported() {
        let dir = tempfile::tempdir().unwrap();
        let conn = import_rdp(
            dir.path(),
            "legacy.rdp",
            "full address:s:w2k8r2.corp\n\
             smart sizing:i:1\n\
             dynamic resolution:i:0\n",
        );
        let rdp = rdp_config(&conn);
        assert!(rdp.smart_sizing);
        assert!(!rdp.dynamic_resolution);
    }

    #[test]
    fn missing_sizing_keys_keep_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let conn = import_rdp(dir.path(), "plain.rdp", "full address:s:host.internal\n");
        let rdp = rdp_config(&conn);
        assert!(!rdp.smart_sizing);
        assert!(rdp.dynamic_resolution);
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
