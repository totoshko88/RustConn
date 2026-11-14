//! PuTTY session importer.
//!
//! Imports saved sessions from a Windows Registry export (`.reg` file) of the
//! `HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions` key. PuTTY (and the
//! compatible KiTTY fork) store each session as a registry subkey whose values
//! describe the host, port, protocol and a handful of SSH options.
//!
//! On Windows a user exports that key with
//! `reg export "HKCU\Software\SimonTatham\PuTTY\Sessions" putty.reg`; on Linux
//! the same file can come from a colleague's Windows box. RustConn does not read
//! the live registry — only the exported text file, which makes the importer
//! cross-platform and testable.
//!
//! Passwords are never present in PuTTY's registry export, so none are imported.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::traits::{ImportResult, ImportSource, SkippedEntry, read_import_bytes};
use crate::error::ImportError;
use crate::models::{Connection, ProtocolConfig, SshConfig, TelnetConfig};

const SOURCE_NAME: &str = "PuTTY";

/// The registry path prefix that identifies a PuTTY session subkey.
///
/// KiTTY reuses the same `SimonTatham\PuTTY` path, so its exports import too.
const SESSION_KEY_PREFIX: &str = r"Software\SimonTatham\PuTTY\Sessions\";

/// A single registry value, typed as it appears in a `.reg` file.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RegValue {
    /// A `"key"="value"` string entry.
    String(String),
    /// A `"key"=dword:0000abcd` entry, decoded from hexadecimal.
    Dword(u32),
}

/// Importer for PuTTY / KiTTY sessions from a Registry export file.
pub struct PuttyImporter {
    /// Custom `.reg` paths to import from. When empty, [`Self::default_paths`]
    /// returns nothing — a Registry export has no canonical location on Linux,
    /// so the user always picks the file explicitly.
    custom_paths: Vec<PathBuf>,
}

impl PuttyImporter {
    /// Creates a new PuTTY importer.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            custom_paths: Vec::new(),
        }
    }

    /// Creates a PuTTY importer bound to specific `.reg` files.
    #[must_use]
    pub const fn with_paths(paths: Vec<PathBuf>) -> Self {
        Self {
            custom_paths: paths,
        }
    }

    /// Parses the text of a `.reg` file into connections.
    ///
    /// Each `[...\Sessions\<name>]` block becomes one connection; the values in
    /// between are collected until the next section header. Session names are
    /// percent-decoded (`My%20Server` → `My Server`), matching how PuTTY escapes
    /// them in the key path.
    #[must_use]
    pub fn parse_reg(&self, content: &str) -> ImportResult {
        let mut result = ImportResult::new();

        let mut current_name: Option<String> = None;
        let mut current_values: HashMap<String, RegValue> = HashMap::new();

        for line in content.lines() {
            let line = line.trim();

            if let Some(session_name) = parse_session_header(line) {
                // A new section starts: flush the session accumulated so far.
                if let Some(name) = current_name.take() {
                    Self::finish_session(&name, &current_values, &mut result);
                }
                current_values = HashMap::new();
                current_name = Some(session_name);
            } else if line.starts_with('[') {
                // A registry section that is not a PuTTY session (some other
                // key present in the same export). Flush and stop accumulating
                // until the next session header.
                if let Some(name) = current_name.take() {
                    Self::finish_session(&name, &current_values, &mut result);
                }
                current_values = HashMap::new();
            } else if current_name.is_some()
                && let Some((key, value)) = parse_value_line(line)
            {
                current_values.insert(key, value);
            }
        }

        // Flush the final session (no trailing header closes it).
        if let Some(name) = current_name.take() {
            Self::finish_session(&name, &current_values, &mut result);
        }

        result
    }

    /// Builds a connection from one session's values and records it, or records
    /// a skip when the session cannot be represented.
    fn finish_session(name: &str, values: &HashMap<String, RegValue>, result: &mut ImportResult) {
        let host = match values.get("HostName") {
            Some(RegValue::String(h)) if !h.trim().is_empty() => h.trim().to_string(),
            _ => {
                result.add_skipped(SkippedEntry::new(
                    name.to_string(),
                    "No HostName — likely a template session (e.g. \"Default Settings\")",
                ));
                return;
            }
        };

        // PuTTY's "Protocol" is lowercase: ssh, telnet, raw, rlogin, serial.
        let protocol = match values.get("Protocol") {
            Some(RegValue::String(p)) => p.to_ascii_lowercase(),
            _ => "ssh".to_string(),
        };

        let port = match values.get("PortNumber") {
            Some(RegValue::Dword(p)) => u16::try_from(*p).ok(),
            _ => None,
        };

        let (protocol_config, default_port) = match protocol.as_str() {
            "ssh" => (ProtocolConfig::Ssh(Self::build_ssh_config(values)), 22u16),
            // Telnet, raw and rlogin are all line protocols RustConn serves
            // through its Telnet client; raw/rlogin keep their own default port.
            "telnet" => (ProtocolConfig::Telnet(TelnetConfig::default()), 23u16),
            "raw" => (ProtocolConfig::Telnet(TelnetConfig::default()), 23u16),
            "rlogin" => (ProtocolConfig::Telnet(TelnetConfig::default()), 513u16),
            other => {
                // Serial has no host in the registry sense and SUPDUP/other
                // exotic protocols have no RustConn equivalent.
                result.add_skipped(SkippedEntry::new(
                    name.to_string(),
                    format!("Unsupported PuTTY protocol: {other}"),
                ));
                return;
            }
        };

        let port = port.unwrap_or(default_port);
        let mut connection = Connection::new(name.to_string(), host, port, protocol_config);

        if let Some(RegValue::String(user)) = values.get("UserName")
            && !user.trim().is_empty()
        {
            connection.username = Some(user.trim().to_string());
        }

        result.add_connection(connection);
    }

    /// Builds an [`SshConfig`] from the SSH-relevant registry values.
    ///
    /// Only the options PuTTY exposes as simple flags are carried: a private key
    /// file, agent forwarding, X11 forwarding and compression. Everything else
    /// (cipher order, terminal modes, proxy) is left at RustConn's defaults.
    fn build_ssh_config(values: &HashMap<String, RegValue>) -> SshConfig {
        use crate::models::SshAuthMethod;

        let key_path = match values.get("PublicKeyFile") {
            Some(RegValue::String(p)) if !p.trim().is_empty() => Some(PathBuf::from(p.trim())),
            _ => None,
        };

        // A configured key file means public-key auth; otherwise leave the
        // default (password), since PuTTY does not record "password auth" as a
        // positive value.
        let auth_method = if key_path.is_some() {
            SshAuthMethod::PublicKey
        } else {
            SshAuthMethod::Password
        };

        let dword_is_set = |key: &str| matches!(values.get(key), Some(RegValue::Dword(1)));

        SshConfig {
            auth_method,
            key_path,
            agent_forwarding: dword_is_set("AgentFwd"),
            x11_forwarding: dword_is_set("X11Forward"),
            compression: dword_is_set("Compression"),
            ..Default::default()
        }
    }
}

/// Extracts the session name from a `[...\Sessions\<name>]` header line.
///
/// Returns `None` for any other line, including registry sections outside the
/// PuTTY sessions key. The name is percent-decoded.
fn parse_session_header(line: &str) -> Option<String> {
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    let idx = inner.find(SESSION_KEY_PREFIX)?;
    let raw_name = &inner[idx + SESSION_KEY_PREFIX.len()..];
    if raw_name.is_empty() {
        return None;
    }
    Some(percent_decode(raw_name))
}

/// Parses a `"key"="value"` or `"key"=dword:hex` line into a typed entry.
fn parse_value_line(line: &str) -> Option<(String, RegValue)> {
    let rest = line.strip_prefix('"')?;
    let end_quote = rest.find('"')?;
    let key = rest[..end_quote].to_string();
    let after_key = rest[end_quote + 1..].strip_prefix('=')?;

    if let Some(hex) = after_key.strip_prefix("dword:") {
        let value = u32::from_str_radix(hex.trim(), 16).ok()?;
        return Some((key, RegValue::Dword(value)));
    }

    let quoted = after_key.strip_prefix('"')?;
    let close = quoted.rfind('"')?;
    let value = unescape_reg_string(&quoted[..close]);
    Some((key, RegValue::String(value)))
}

/// Unescapes a `.reg` string value: `\\` → `\` and `\"` → `"`.
fn unescape_reg_string(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Decodes `%XX` escapes in a PuTTY session-key name.
///
/// PuTTY escapes bytes it cannot put in a registry key name as `%` followed by
/// two hexadecimal digits (so a space becomes `%20`). A `%` not followed by two
/// hex digits is left as-is. Decoding is byte-wise then re-interpreted as UTF-8,
/// falling back to the raw text if the result is not valid UTF-8.
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| raw.to_string())
}

impl Default for PuttyImporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportSource for PuttyImporter {
    fn source_id(&self) -> &'static str {
        "putty"
    }

    fn display_name(&self) -> &'static str {
        "PuTTY"
    }

    fn is_available(&self) -> bool {
        self.default_paths().iter().any(|p| p.exists())
    }

    fn default_paths(&self) -> Vec<PathBuf> {
        // A Registry export has no fixed location on Linux; the user selects the
        // `.reg` file explicitly, so there are no default paths to scan.
        self.custom_paths.clone()
    }

    fn import(&self) -> Result<ImportResult, ImportError> {
        let paths = self.default_paths();
        if paths.is_empty() {
            return Err(ImportError::FileNotFound(PathBuf::from(
                "PuTTY .reg export (select a file)",
            )));
        }
        let mut combined = ImportResult::new();
        for path in paths {
            match self.import_from_path(&path) {
                Ok(result) => combined.merge(result),
                Err(e) => combined.add_error(e),
            }
        }
        Ok(combined)
    }

    fn import_from_path(&self, path: &Path) -> Result<ImportResult, ImportError> {
        let bytes = read_import_bytes(path, SOURCE_NAME)?;
        let content = decode_reg_text(&bytes).ok_or_else(|| ImportError::ParseError {
            source_name: SOURCE_NAME.to_string(),
            reason: format!("{} is not UTF-8 or UTF-16 text", path.display()),
        })?;
        Ok(self.parse_reg(&content))
    }
}

/// Decodes the text of a `.reg` file.
///
/// `regedit` writes UTF-16 little-endian with a byte-order mark; a hand-edited
/// or converted file is usually UTF-8, with or without one. `None` when the
/// bytes are neither.
fn decode_reg_text(bytes: &[u8]) -> Option<String> {
    let utf16 = |rest: &[u8], from: fn([u8; 2]) -> u16| -> Option<String> {
        let (pairs, tail) = rest.as_chunks::<2>();
        if !tail.is_empty() {
            return None;
        }
        let units: Vec<u16> = pairs.iter().map(|pair| from(*pair)).collect();
        String::from_utf16(&units).ok()
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(rest, u16::from_le_bytes),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, u16::from_be_bytes),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8(rest.to_vec()).ok(),
        _ => String::from_utf8(bytes.to_vec()).ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_reg_text_reads_regedit_utf16_le_with_bom() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "Windows Registry Editor Version 5.00\r\n".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(
            decode_reg_text(&bytes).as_deref(),
            Some("Windows Registry Editor Version 5.00\r\n")
        );
    }

    #[test]
    fn decode_reg_text_reads_utf8_with_and_without_bom() {
        assert_eq!(decode_reg_text(b"abc").as_deref(), Some("abc"));
        assert_eq!(decode_reg_text(b"\xEF\xBB\xBFabc").as_deref(), Some("abc"));
    }

    #[test]
    fn decode_reg_text_refuses_other_bytes() {
        assert_eq!(decode_reg_text(&[0xFF, 0xFE, 0x41]), None);
        assert_eq!(decode_reg_text(&[0xC3, 0x28]), None);
    }

    #[test]
    fn parses_ssh_session_with_key_and_forwarding() {
        let reg = r#"Windows Registry Editor Version 5.00

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\My%20Server]
"HostName"="server.example.com"
"PortNumber"=dword:00000016
"Protocol"="ssh"
"UserName"="admin"
"PublicKeyFile"="C:\\Users\\admin\\.ssh\\id_rsa.ppk"
"AgentFwd"=dword:00000001
"Compression"=dword:00000001
"#;

        let result = PuttyImporter::new().parse_reg(reg);
        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.name, "My Server");
        assert_eq!(conn.host, "server.example.com");
        assert_eq!(conn.port, 22);
        assert_eq!(conn.username, Some("admin".to_string()));

        if let ProtocolConfig::Ssh(ssh) = &conn.protocol_config {
            assert!(ssh.key_path.is_some());
            assert!(ssh.agent_forwarding);
            assert!(ssh.compression);
            assert!(!ssh.x11_forwarding);
        } else {
            panic!("expected SSH protocol config");
        }
    }

    #[test]
    fn telnet_session_uses_default_port_when_absent() {
        let reg = r#"[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\Router]
"HostName"="10.0.0.1"
"Protocol"="telnet"
"#;

        let result = PuttyImporter::new().parse_reg(reg);
        assert_eq!(result.connections.len(), 1);
        let conn = &result.connections[0];
        assert_eq!(conn.port, 23);
        assert!(matches!(conn.protocol_config, ProtocolConfig::Telnet(_)));
    }

    #[test]
    fn default_settings_template_is_skipped() {
        let reg = r#"[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\Default%20Settings]
"Protocol"="ssh"
"PortNumber"=dword:00000016
"#;

        let result = PuttyImporter::new().parse_reg(reg);
        assert!(result.connections.is_empty());
        assert_eq!(result.skipped.len(), 1);
    }

    #[test]
    fn multiple_sessions_are_all_parsed() {
        let reg = r#"[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\Alpha]
"HostName"="a.example.com"
"Protocol"="ssh"

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\Beta]
"HostName"="b.example.com"
"Protocol"="ssh"
"PortNumber"=dword:00000FA0
"#;

        let result = PuttyImporter::new().parse_reg(reg);
        assert_eq!(result.connections.len(), 2);
        let beta = result
            .connections
            .iter()
            .find(|c| c.name == "Beta")
            .expect("Beta session present");
        assert_eq!(beta.port, 4000);
    }

    #[test]
    fn unsupported_protocol_is_skipped_not_imported() {
        let reg = r#"[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\Console]
"HostName"="COM1"
"Protocol"="serial"
"#;

        let result = PuttyImporter::new().parse_reg(reg);
        assert!(result.connections.is_empty());
        assert_eq!(result.skipped.len(), 1);
    }

    #[test]
    fn non_putty_registry_section_is_ignored() {
        let reg = r#"[HKEY_CURRENT_USER\Software\SomethingElse\Config]
"HostName"="should.not.import"
"Protocol"="ssh"

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\Real]
"HostName"="real.example.com"
"Protocol"="ssh"
"#;

        let result = PuttyImporter::new().parse_reg(reg);
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.connections[0].name, "Real");
    }

    #[test]
    fn percent_decode_handles_plain_and_escaped() {
        assert_eq!(percent_decode("Plain"), "Plain");
        assert_eq!(percent_decode("My%20Server"), "My Server");
        assert_eq!(percent_decode("100%done"), "100%done");
    }
}
