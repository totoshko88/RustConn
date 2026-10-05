//! Remote Desktop Manager (RDM) JSON importer.
//!
//! Parses RDM export files in JSON format.
//! Supports importing SSH, RDP, VNC, and Telnet connections with folder hierarchy.
//!
//! Two dialects of the same format are handled:
//!
//! * The documented import format, where `ConnectionType` is a token name
//!   (`"RDPConfigured"`, `"SSHShell"`, ...) and the destination folder is a
//!   backslash separated path in `Group`.
//! * Real RDM exports, where `ConnectionType` is the numeric value of the
//!   Devolutions `ConnectionType` enum (`1` = `RDPConfigured`, `25` = `Group`,
//!   `77` = `SSHShell`, ...).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use secrecy::ExposeSecret;
use serde::{Deserialize, Deserializer};
use uuid::Uuid;

use super::normalize::parse_host_port;
use super::traits::{ImportResult, ImportSource, SkippedEntry};
use crate::error::ImportError;
use crate::models::{
    AutomationConfig, Connection, ConnectionGroup, Credentials, PasswordSource, ProtocolConfig,
    ProtocolType, RdpConfig, SshAuthMethod, SshConfig, TelnetConfig, VncConfig, WindowMode,
};
use crate::progress::ProgressReporter;

/// Maps the numeric Devolutions `ConnectionType` enum to its token name.
///
/// RDM serialises the enum as an integer in real exports while the documented
/// import format uses the token name, so both have to be understood. Only the
/// values RustConn can act on are listed; everything else is reported as an
/// unsupported entry type.
fn connection_type_token(value: u64) -> Option<&'static str> {
    match value {
        1 => Some("rdpconfigured"),
        2 => Some("rdpfilename"),
        4 => Some("vnc"),
        8 => Some("putty"),
        25 => Some("group"),
        26 => Some("credential"),
        28 => Some("ftp"),
        32 => Some("website"),
        38 => Some("scp"),
        62 => Some("terminalconsole"),
        67 => Some("securecrt"),
        68 => Some("iterm"),
        74 => Some("telnet"),
        77 => Some("sshshell"),
        92 => Some("root"),
        100 => Some("sftp"),
        104 => Some("novnc"),
        _ => None,
    }
}

/// Deserializes `ConnectionType`, which may be a token name or an enum number.
///
/// Unknown numbers are kept as their decimal representation so the skipped
/// entry reason names the actual value found in the file. A present but blank
/// token is kept as an empty token, which [`RdmImporter::parse_protocol`]
/// reports; only an absent field yields `None` and with it RDM's own default
/// of RDP.
fn deserialize_connection_type<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(raw.and_then(|value| match value {
        // A token may itself be the enum number in quotes.
        serde_json::Value::String(token) => {
            let token = token.trim().to_lowercase();
            // A blank token is a value the file names and the parser cannot
            // read, so it stays distinguishable from an absent field: the
            // entry is reported instead of being guessed into an RDP one.
            if token.is_empty() {
                return Some(String::new());
            }
            Some(
                token
                    .parse::<u64>()
                    .ok()
                    .and_then(connection_type_token)
                    .map_or(token, ToString::to_string),
            )
        }
        serde_json::Value::Number(number) => Some(
            number
                .as_u64()
                .and_then(connection_type_token)
                .map_or_else(|| number.to_string(), ToString::to_string),
        ),
        _ => None,
    }))
}

/// Parses a port from the scalar forms RDM's serializers write.
///
/// .NET serializers occasionally write an integral value in floating point
/// form (`3389.0`), so a zero fraction is accepted. A negative, fractional or
/// out-of-range value is not a port, and neither is `0`: it means "any free
/// port" to a listener and nothing at all to a client, so the caller's
/// protocol default is the better answer.
fn parse_port_text(text: &str) -> Option<u16> {
    let text = text.trim();
    // A value without a fraction is treated as one with a zero fraction, so
    // `3389` and `3389.0` take the same path.
    let (integral, fraction) = text.split_once('.').unwrap_or((text, "0"));
    if !fraction.chars().all(|digit| digit == '0') {
        return None;
    }
    // Port 0 is rejected like an out-of-range value, see the doc comment.
    integral.parse::<u16>().ok().filter(|port| *port != 0)
}

/// Converts a JSON number to a port.
///
/// Shares [`parse_port_text`] with the quoted form so the two cannot drift
/// apart: a zero fraction is accepted, while a negative, fractional,
/// out-of-range or zero value yields `None` and leaves the protocol default in
/// place.
fn number_to_port(number: &serde_json::Number) -> Option<u16> {
    parse_port_text(&number.to_string())
}

/// A `Port` field as it was written in the export.
///
/// The raw scalar is kept next to the parsed value so a port the parser had to
/// reject can be named in a warning instead of silently becoming the protocol
/// default.
#[derive(Debug, Clone, Default)]
struct PortField {
    /// The usable port, when the value is one.
    value: Option<u16>,
    /// The scalar as written, present only when it named a value.
    raw: Option<String>,
}

impl PortField {
    /// Returns the written value when it was present but not a usable port.
    fn rejected(&self) -> Option<&str> {
        if self.value.is_some() {
            return None;
        }
        self.raw.as_deref()
    }
}

/// Deserializes a port that may be encoded as a number or as a string.
///
/// Both forms go through [`parse_port_text`]; the raw scalar is retained so a
/// rejected value can be reported. See [`PortField`].
fn deserialize_optional_port<'de, D>(deserializer: D) -> Result<PortField, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(value) = Option::<serde_json::Value>::deserialize(deserializer)? else {
        return Ok(PortField::default());
    };

    let (port, written) = match value {
        serde_json::Value::Number(number) => (number_to_port(&number), Some(number.to_string())),
        serde_json::Value::String(text) => {
            let text = text.trim();
            // An empty string is how RDM writes "not set", so it counts as an
            // absent field rather than a value that was rejected.
            let written = (!text.is_empty()).then(|| text.to_string());
            (parse_port_text(text), written)
        }
        // Any other shape names no scalar worth quoting back to the user.
        _ => (None, None),
    };

    Ok(PortField {
        value: port,
        raw: written,
    })
}

/// Deserializes a flag that may arrive as a bool, a number or a string.
///
/// RDM's add-on system feeds boolean fields from data sources that serialize
/// them as `0`/`1` or as `"true"`/`"false"`.
fn deserialize_tolerant_bool<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(raw.and_then(|value| match value {
        serde_json::Value::Bool(flag) => Some(flag),
        // Compared as text to keep the check exact for every numeric form.
        serde_json::Value::Number(number) => {
            Some(!matches!(number.to_string().as_str(), "0" | "-0" | "0.0"))
        }
        serde_json::Value::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" => Some(true),
            "false" | "no" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }))
}

/// Deserializes a display name, accepting the scalar forms RDM may write.
///
/// An entry named after a number or a missing name must not abort the parse,
/// so anything that is not a usable scalar becomes an empty name.
fn deserialize_tolerant_name<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let name = crate::secret::serde_helpers::deserialize_tolerant_string(deserializer)?;
    Ok(name.unwrap_or_default())
}

/// Deserializes the nested `Credentials` object, ignoring any other shape.
///
/// Restricted to a JSON object on purpose: serde would otherwise map an array
/// onto the fields by position, so `["bob"]` would silently become a username.
fn deserialize_tolerant_credentials<'de, D>(
    deserializer: D,
) -> Result<Option<RdmCredentials>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(raw
        .filter(serde_json::Value::is_object)
        .and_then(|value| serde_json::from_value(value).ok()))
}

/// Nested `Credentials` structure RDM uses for entry credentials.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RdmCredentials {
    #[serde(
        default,
        deserialize_with = "crate::secret::serde_helpers::deserialize_tolerant_string"
    )]
    user_name: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::secret::serde_helpers::deserialize_tolerant_string"
    )]
    domain: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::secret::serde_helpers::deserialize_tolerant_secret"
    )]
    password: Option<secrecy::SecretString>,
}

/// RDM JSON connection entry
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RdmConnection {
    #[serde(default, deserialize_with = "deserialize_tolerant_name")]
    name: String,
    #[serde(
        default,
        rename = "ID",
        deserialize_with = "crate::secret::serde_helpers::deserialize_tolerant_string"
    )]
    id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_connection_type")]
    connection_type: Option<String>,
    /// Target host. RDM stores it in `Host` for terminal entries and in `Url`
    /// for RDP and web based entries.
    #[serde(
        default,
        alias = "Url",
        alias = "HostName",
        alias = "Server",
        deserialize_with = "crate::secret::serde_helpers::deserialize_tolerant_string"
    )]
    host: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_port")]
    port: PortField,
    #[serde(
        default,
        alias = "UserName",
        deserialize_with = "crate::secret::serde_helpers::deserialize_tolerant_string"
    )]
    username: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::secret::serde_helpers::deserialize_tolerant_secret"
    )]
    password: Option<secrecy::SecretString>,
    #[serde(
        default,
        deserialize_with = "crate::secret::serde_helpers::deserialize_tolerant_string"
    )]
    domain: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::secret::serde_helpers::deserialize_tolerant_string"
    )]
    description: Option<String>,
    /// Backslash separated destination folder, e.g. `Customers\ACME`.
    #[serde(
        default,
        deserialize_with = "crate::secret::serde_helpers::deserialize_tolerant_string"
    )]
    group: Option<String>,
    #[serde(
        default,
        rename = "ParentID",
        deserialize_with = "crate::secret::serde_helpers::deserialize_tolerant_string"
    )]
    parent_id: Option<String>,
    /// Credentials embedded in the entry.
    #[serde(default, deserialize_with = "deserialize_tolerant_credentials")]
    credentials: Option<RdmCredentials>,
    /// Reference to a separate `Credential` entry.
    #[serde(
        default,
        rename = "CredentialConnectionID",
        deserialize_with = "crate::secret::serde_helpers::deserialize_tolerant_string"
    )]
    credential_connection_id: Option<String>,
    // SSH specific
    #[serde(
        default,
        rename = "PrivateKeyPath",
        deserialize_with = "crate::secret::serde_helpers::deserialize_tolerant_string"
    )]
    private_key_path: Option<String>,
    // VNC specific
    #[serde(
        default,
        rename = "ViewOnly",
        deserialize_with = "deserialize_tolerant_bool"
    )]
    view_only: Option<bool>,
}

/// RDM JSON folder entry
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RdmFolder {
    #[serde(
        default,
        rename = "ID",
        deserialize_with = "crate::secret::serde_helpers::deserialize_tolerant_string"
    )]
    id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_tolerant_name")]
    name: String,
    #[serde(
        default,
        rename = "ParentID",
        deserialize_with = "crate::secret::serde_helpers::deserialize_tolerant_string"
    )]
    parent_id: Option<String>,
}

/// Top-level keys that mark a document as an RDM export envelope.
///
/// Both spellings serde accepts for [`RdmExport`] are listed, so the check that
/// rejects an unrelated JSON file cannot disagree with the deserializer.
const ENVELOPE_KEYS: [&str; 4] = ["Connections", "connections", "Folders", "folders"];

/// RDM JSON export envelope.
///
/// Entries stay as raw values here and are decoded one by one in
/// [`RdmImporter::decode_entries`]. RDM has an open architecture in which
/// add-ons contribute their own entry fields, so no schema can cover every
/// export; decoding per entry keeps one unreadable entry from aborting the
/// whole import.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RdmExport {
    #[serde(default, alias = "connections")]
    connections: Vec<serde_json::Value>,
    #[serde(default, alias = "folders")]
    folders: Vec<serde_json::Value>,
}

/// Credential fields collected from an entry or a linked `Credential` entry.
#[derive(Debug, Clone, Default)]
struct EntryCredential {
    username: Option<String>,
    domain: Option<String>,
    password: Option<secrecy::SecretString>,
}

/// Remote Desktop Manager JSON importer
pub struct RdmImporter;

impl ImportSource for RdmImporter {
    fn source_id(&self) -> &'static str {
        "rdm"
    }

    fn display_name(&self) -> &'static str {
        "Remote Desktop Manager (JSON)"
    }

    fn is_available(&self) -> bool {
        // RDM is file-based, so always available for file import
        true
    }

    fn default_paths(&self) -> Vec<PathBuf> {
        // RDM doesn't have standard config paths, return empty
        Vec::new()
    }

    fn import(&self) -> Result<ImportResult, ImportError> {
        // No default paths for RDM, return empty result
        Ok(ImportResult::new())
    }

    fn import_from_path(&self, path: &Path) -> Result<ImportResult, ImportError> {
        let content = fs::read_to_string(path).map_err(ImportError::Io)?;

        self.import_from_content(&content)
    }

    fn import_from_path_with_progress(
        &self,
        path: &Path,
        progress: Option<&dyn ProgressReporter>,
    ) -> Result<ImportResult, ImportError> {
        if let Some(reporter) = progress {
            reporter.report(0, 3, "Reading RDM file...");
            if reporter.is_cancelled() {
                return Err(ImportError::Cancelled);
            }
        }

        let content = fs::read_to_string(path).map_err(ImportError::Io)?;

        if let Some(reporter) = progress {
            reporter.report(1, 3, "Parsing RDM data...");
            if reporter.is_cancelled() {
                return Err(ImportError::Cancelled);
            }
        }

        let result = self.import_from_content(&content)?;

        if let Some(reporter) = progress {
            reporter.report(3, 3, "Import completed");
        }

        Ok(result)
    }
}

impl RdmImporter {
    /// Creates a new RDM importer
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Imports connections from RDM JSON content
    ///
    /// Parses the provided JSON string as an RDM export file and converts the
    /// connections and folders to RustConn format. Folder hierarchy is taken
    /// from the `Folders` array when present and from the `Group` path of each
    /// entry otherwise.
    ///
    /// # Arguments
    ///
    /// * `content` - The JSON content to parse
    ///
    /// # Returns
    ///
    /// An `ImportResult` containing the converted connections, groups and
    /// credentials. An entry of an unsupported type, and an entry the parser
    /// cannot read at all, are both reported in `skipped` so that neither
    /// stops the rest of the export from being imported. A single field that
    /// had to be dropped, such as an out-of-range `Port`, leaves the entry
    /// importable and is reported in `warnings`.
    ///
    /// # Errors
    ///
    /// Returns `ImportError::ParseError` if the JSON is malformed or does not
    /// describe an RDM export.
    pub fn import_from_content(&self, content: &str) -> Result<ImportResult, ImportError> {
        let rdm_data = Self::parse_envelope(content)?;

        let mut result = ImportResult::new();
        // Entries are decoded individually so that one entry carrying a field
        // shape the parser does not know still leaves the rest importable.
        let folders: Vec<RdmFolder> = Self::decode_entries(rdm_data.folders, "Folder", &mut result);
        let connections: Vec<RdmConnection> =
            Self::decode_entries(rdm_data.connections, "Connection", &mut result);

        // Maps RDM folder ID -> RustConn group UUID
        let mut group_map: HashMap<String, Uuid> = HashMap::new();
        // Maps a normalized folder path -> RustConn group UUID
        let mut path_map: HashMap<String, Uuid> = HashMap::new();

        // Explicit folder list (documented format and older exports)
        for folder in &folders {
            if let Some(id) = folder.id.as_ref().filter(|id| !id.is_empty()) {
                group_map.insert(id.clone(), Uuid::new_v4());
            }
        }

        for folder in &folders {
            let group = Self::create_group_from_folder(folder, &group_map);
            result.add_group(group);
        }

        // First pass: folder paths and credential entries. Group entries carry
        // the folder tree in real exports; other entries reference their folder
        // through the same `Group` path, which may not have its own entry.
        let mut credentials: HashMap<String, EntryCredential> = HashMap::new();
        for conn in &connections {
            let is_group = conn.connection_type.as_deref() == Some("group");
            let path = if is_group {
                Self::group_entry_path(conn)
            } else {
                conn.group.as_deref().unwrap_or_default().trim().to_string()
            };
            if !path.is_empty() {
                Self::ensure_group_path(&path, &mut path_map, &mut result);
            }

            if conn.connection_type.as_deref() == Some("credential")
                && let Some(id) = conn.id.as_ref().filter(|id| !id.is_empty())
            {
                credentials.insert(id.clone(), Self::entry_credential(conn));
            }
        }

        // Second pass: connections.
        for conn in &connections {
            // Folders and credential entries were handled above, the document
            // root is not a connection either.
            if matches!(
                conn.connection_type.as_deref(),
                Some("group" | "credential" | "root")
            ) {
                continue;
            }

            match Self::create_connection_from_rdm(
                conn,
                &credentials,
                &group_map,
                &path_map,
                &mut result,
            ) {
                Ok((connection, creds)) => {
                    if let Some(creds) = creds {
                        result.add_credentials(connection.id, creds);
                    }
                    result.add_connection(connection);
                }
                Err(reason) => {
                    result.add_skipped(SkippedEntry::new(&conn.name, reason));
                }
            }
        }

        Ok(result)
    }

    /// Reads the export envelope, accepting the shapes RDM actually writes.
    ///
    /// `File - Export` produces `{"Connections": [...]}`; a bare array and a
    /// single entry object are accepted too, because RDM's
    /// `Clipboard - Copy` yields one entry and PowerShell pipelines yield an
    /// array.
    fn parse_envelope(content: &str) -> Result<RdmExport, ImportError> {
        let parse_error = |reason: String| ImportError::ParseError {
            source_name: "RDM JSON".to_string(),
            reason,
        };

        let value: serde_json::Value = serde_json::from_str(content)
            .map_err(|e| parse_error(format!("Failed to parse JSON: {e}")))?;

        if let serde_json::Value::Array(entries) = value {
            return Ok(RdmExport {
                connections: entries,
                ..RdmExport::default()
            });
        }

        let Some(members) = value.as_object() else {
            return Err(parse_error(format!(
                "Expected a JSON object or array, found {}",
                match value {
                    serde_json::Value::Null => "null",
                    serde_json::Value::Bool(_) => "a boolean",
                    serde_json::Value::Number(_) => "a number",
                    _ => "a string",
                }
            )));
        };

        if Self::looks_like_entry(&value) {
            return Ok(RdmExport {
                connections: vec![value],
                ..RdmExport::default()
            });
        }

        // Without either array there is nothing to import, and silently
        // reporting "0 connections" hides that the wrong file was picked.
        if !members
            .keys()
            .any(|key| ENVELOPE_KEYS.contains(&key.as_str()))
        {
            return Err(parse_error(
                "No 'Connections' or 'Folders' array found. In RDM use \
                 File - Export - Export All to write a .json export."
                    .to_string(),
            ));
        }

        serde_json::from_value(value)
            .map_err(|e| parse_error(format!("Not a Remote Desktop Manager export: {e}")))
    }

    /// Whether a top-level object is a single entry rather than an envelope.
    ///
    /// `ConnectionType` is required, because `Clipboard - Copy` always writes
    /// it and because a name plus a host is the shape of countless unrelated
    /// inventory files; those are read as an envelope instead, so picking the
    /// wrong file reports a parse error rather than importing one connection.
    fn looks_like_entry(value: &serde_json::Value) -> bool {
        if ENVELOPE_KEYS.iter().any(|key| value.get(key).is_some()) {
            return false;
        }
        // A copied entry also identifies itself, so requiring one of the two
        // keeps a lone `ConnectionType` fragment from passing as an entry.
        value.get("ConnectionType").is_some()
            && ["ID", "Name"].iter().any(|key| value.get(key).is_some())
    }

    /// Decodes raw entries, reporting the ones that cannot be read.
    ///
    /// A decode failure is recorded as a skipped entry naming the offending
    /// entry, instead of aborting the import the way a single
    /// `serde_json::from_str` over the whole document would.
    fn decode_entries<T: serde::de::DeserializeOwned>(
        raw: Vec<serde_json::Value>,
        kind: &str,
        result: &mut ImportResult,
    ) -> Vec<T> {
        let mut decoded = Vec::with_capacity(raw.len());
        for (index, value) in raw.into_iter().enumerate() {
            let label = Self::entry_label(&value, kind, index);
            match serde_json::from_value(value) {
                Ok(entry) => decoded.push(entry),
                Err(e) => result.add_skipped(SkippedEntry::new(
                    label,
                    format!("Entry could not be read: {e}"),
                )),
            }
        }
        decoded
    }

    /// Best-effort label for an entry, used when it cannot be decoded.
    ///
    /// Accepts the same scalars as [`deserialize_tolerant_name`], so the label
    /// cannot fall back to a positional name for a value the deserializer would
    /// have read.
    fn entry_label(value: &serde_json::Value, kind: &str, index: usize) -> String {
        value
            .get("Name")
            .and_then(|name| match name {
                serde_json::Value::String(text) if !text.trim().is_empty() => {
                    Some(text.trim().to_string())
                }
                serde_json::Value::Number(number) => Some(number.to_string()),
                serde_json::Value::Bool(flag) => Some(flag.to_string()),
                _ => None,
            })
            .unwrap_or_else(|| format!("{kind} #{}", index + 1))
    }

    /// Collects the credential fields of an entry, flat or nested.
    fn entry_credential(conn: &RdmConnection) -> EntryCredential {
        let nested = conn.credentials.clone().unwrap_or_default();
        EntryCredential {
            username: conn
                .username
                .clone()
                .or(nested.user_name)
                .filter(|u| !u.is_empty()),
            domain: conn
                .domain
                .clone()
                .or(nested.domain)
                .filter(|d| !d.is_empty()),
            password: conn
                .password
                .clone()
                .or(nested.password)
                .filter(|p| !p.expose_secret().is_empty()),
        }
    }

    /// Returns the full folder path of a `Group` entry (`Group` path + `Name`).
    fn group_entry_path(conn: &RdmConnection) -> String {
        let parent = conn.group.as_deref().unwrap_or_default().trim();
        let name = conn.name.trim();
        if name.is_empty() {
            parent.to_string()
        } else if parent.is_empty() {
            name.to_string()
        } else {
            format!("{parent}\\{name}")
        }
    }

    /// Creates (or reuses) the group chain for a backslash separated path.
    ///
    /// Returns the UUID of the deepest group in the path, or `None` when the
    /// path holds no usable segment.
    fn ensure_group_path(
        path: &str,
        path_map: &mut HashMap<String, Uuid>,
        result: &mut ImportResult,
    ) -> Option<Uuid> {
        let mut current_path = String::new();
        let mut parent: Option<Uuid> = None;

        for segment in path.split(['\\', '/']) {
            let segment = segment.trim();
            if segment.is_empty() {
                continue;
            }
            if !current_path.is_empty() {
                current_path.push('\\');
            }
            current_path.push_str(segment);

            if let Some(existing) = path_map.get(&current_path) {
                parent = Some(*existing);
                continue;
            }

            let group = parent.map_or_else(
                || ConnectionGroup::new(segment.to_string()),
                |parent_id| ConnectionGroup::with_parent(segment.to_string(), parent_id),
            );
            let group_id = group.id;
            result.add_group(group);
            path_map.insert(current_path.clone(), group_id);
            parent = Some(group_id);
        }

        parent
    }

    /// Normalizes a `Group` path the same way [`Self::ensure_group_path`] does.
    fn normalize_group_path(path: &str) -> String {
        path.split(['\\', '/'])
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>()
            .join("\\")
    }

    /// Creates a connection group from RDM folder with parent resolution
    ///
    /// # Arguments
    ///
    /// * `folder` - The RDM folder to convert
    /// * `group_map` - Mapping from RDM folder IDs to RustConn group UUIDs
    fn create_group_from_folder(
        folder: &RdmFolder,
        group_map: &HashMap<String, Uuid>,
    ) -> ConnectionGroup {
        // Resolve parent_id: look up the RDM parent ID in the group map
        let parent_id = folder
            .parent_id
            .as_ref()
            .and_then(|pid| group_map.get(pid).copied());

        let mut group = parent_id.map_or_else(
            || ConnectionGroup::new(folder.name.clone()),
            |parent| ConnectionGroup::with_parent(folder.name.clone(), parent),
        );
        // Use the pre-allocated UUID from the group_map
        if let Some(id) = folder.id.as_ref().and_then(|id| group_map.get(id)).copied() {
            group.id = id;
        }
        group
    }

    /// Parses protocol configuration from RDM connection type
    fn parse_protocol(conn: &RdmConnection) -> Result<(ProtocolType, ProtocolConfig, u16), String> {
        // RDM falls back to RDP when no connection type is serialised.
        let connection_type = conn
            .connection_type
            .as_deref()
            .unwrap_or("rdpconfigured")
            .trim();

        match connection_type {
            "ssh" | "ssh2" | "sshshell" | "putty" | "iterm" | "terminalconsole" | "securecrt" => {
                let key_path = conn
                    .private_key_path
                    .as_ref()
                    .filter(|p| !p.is_empty())
                    .map(|p| std::path::PathBuf::from(shellexpand::tilde(p).into_owned()));
                let auth_method = if key_path.is_some() {
                    SshAuthMethod::PublicKey
                } else {
                    SshAuthMethod::Password
                };
                let ssh_config = SshConfig {
                    auth_method,
                    key_path,
                    x11_forwarding: false,
                    compression: false,
                    ..Default::default()
                };
                Ok((ProtocolType::Ssh, ProtocolConfig::Ssh(ssh_config), 22))
            }
            "rdp" | "rdp2" | "rdpconfigured" | "rdpfilename" => Ok((
                ProtocolType::Rdp,
                ProtocolConfig::Rdp(RdpConfig::default()),
                3389,
            )),
            "vnc" | "novnc" => {
                let vnc_config = VncConfig {
                    view_only: conn.view_only.unwrap_or(false),
                    ..Default::default()
                };
                Ok((ProtocolType::Vnc, ProtocolConfig::Vnc(vnc_config), 5900))
            }
            "telnet" => Ok((
                ProtocolType::Telnet,
                ProtocolConfig::Telnet(TelnetConfig::default()),
                23,
            )),
            // A `ConnectionType` that is present but blank names no protocol.
            // Reporting the entry loses less than falling through to the RDP
            // default, which would invent a connection the file never named;
            // an absent field still gets that default above.
            "" => Err("Blank connection type: the entry names no protocol".to_string()),
            other => Err(format!("Unsupported connection type: {other}")),
        }
    }

    /// Creates a connection from RDM connection data
    ///
    /// Returns the connection together with the credentials that have to be
    /// stored in the secret backend, or the reason the entry was skipped. A
    /// value that had to be dropped, such as an unusable `Port`, is recorded as
    /// a warning on `result`.
    fn create_connection_from_rdm(
        conn: &RdmConnection,
        credentials: &HashMap<String, EntryCredential>,
        group_map: &HashMap<String, Uuid>,
        path_map: &HashMap<String, Uuid>,
        result: &mut ImportResult,
    ) -> Result<(Connection, Option<Credentials>), String> {
        let (protocol, protocol_config, default_port) = Self::parse_protocol(conn)?;

        let host_raw = conn
            .host
            .as_deref()
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .ok_or_else(|| format!("Connection '{}' has no host", conn.name))?;

        // Use shared utility for host:port parsing
        let (host, parsed_port) = parse_host_port(host_raw);

        // Use parsed port from host string, or connection port, or default
        let port = parsed_port.or(conn.port.value).unwrap_or(default_port);

        // A `Port` the parser had to reject would otherwise vanish. The entry
        // is still worth importing on the default port, so the dropped value is
        // named in a warning rather than turned into a skipped entry.
        if let Some(rejected) = conn.port.rejected() {
            result.add_warning(crate::import::ImportWarning::PortIgnored {
                connection_name: conn.name.clone(),
                rejected_port: rejected.to_string(),
                port,
            });
        }

        // Credentials on the entry win over the linked Credential entry.
        let own = Self::entry_credential(conn);
        let linked = conn
            .credential_connection_id
            .as_ref()
            .and_then(|id| credentials.get(id))
            .cloned()
            .unwrap_or_default();
        let username = own.username.or(linked.username);
        let domain = own.domain.or(linked.domain);
        let password = own.password.or(linked.password);

        let group_id = conn
            .parent_id
            .as_ref()
            .and_then(|pid| group_map.get(pid).copied())
            .or_else(|| {
                conn.group
                    .as_deref()
                    .map(Self::normalize_group_path)
                    .filter(|path| !path.is_empty())
                    .and_then(|path| path_map.get(&path).copied())
            });

        // A password is only useful when it can be handed to the secret
        // backend, so the connection is switched to the vault source.
        let (password_source, creds) = password.map_or((PasswordSource::None, None), |password| {
            let creds = Credentials {
                username: username.clone(),
                password: Some(password),
                key_passphrase: None,
                domain: domain.clone(),
            };
            (PasswordSource::Vault, Some(creds))
        });

        let now = chrono::Utc::now();

        let connection = Connection {
            id: Uuid::new_v4(),
            name: conn.name.clone(),
            description: conn.description.clone(),
            protocol,
            host,
            port,
            username,
            group_id,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            protocol_config,
            automation: AutomationConfig::default(),
            sort_order: 0,
            last_connected: None,
            password_source,
            network_mode: crate::models::NetworkMode::default(),
            domain,
            custom_properties: Vec::new(),
            pre_connect_task: None,
            post_disconnect_task: None,
            wol_config: None,
            local_variables: HashMap::new(),
            log_config: None,
            key_sequence: None,
            window_mode: WindowMode::default(),
            remember_window_position: false,
            window_geometry: None,
            skip_port_check: false,
            is_pinned: false,
            pin_order: 0,
            icon: None,
            monitoring_config: None,
            activity_monitor_config: None,
            theme_override: None,
            session_recording_enabled: false,
            highlight_rules: Vec::new(),
            is_dynamic: false,
            retry_config: None,
            knock_sequence: None,
            spa_config: None,
            postpend: None,
            sync_origin_id: None,
        };

        Ok((connection, creds))
    }
}

impl Default for RdmImporter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_numeric_connection_types() {
        // Real RDM exports serialise ConnectionType as the enum number:
        // 25 = Group, 77 = SSHShell, 1 = RDPConfigured, 4 = VNC.
        let json = r#"{"Connections":[
            {"ConnectionType":25,"ID":"g1","Name":"Prod"},
            {"ConnectionType":77,"Name":"shell","Host":"ssh.example.com","Group":"Prod"},
            {"ConnectionType":1,"Name":"desk","Url":"rdp.example.com","Group":"Prod"},
            {"ConnectionType":4,"Name":"screen","Host":"vnc.example.com","Port":"5901"}
        ]}"#;

        let result = RdmImporter::new()
            .import_from_content(json)
            .expect("import should succeed");

        assert_eq!(result.connections.len(), 3, "{:?}", result.skipped);
        assert_eq!(result.groups.len(), 1);

        let ssh = &result.connections[0];
        assert_eq!(ssh.protocol, ProtocolType::Ssh);
        assert_eq!(ssh.host, "ssh.example.com");
        assert_eq!(ssh.port, 22);
        assert_eq!(ssh.group_id, Some(result.groups[0].id));

        let rdp = &result.connections[1];
        assert_eq!(rdp.protocol, ProtocolType::Rdp);
        assert_eq!(rdp.host, "rdp.example.com");
        assert_eq!(rdp.group_id, Some(result.groups[0].id));

        let vnc = &result.connections[2];
        assert_eq!(vnc.protocol, ProtocolType::Vnc);
        assert_eq!(vnc.port, 5901, "string port must be accepted");
    }

    #[test]
    fn creates_nested_groups_from_group_path() {
        let json = r#"{"Connections":[
            {"ConnectionType":"SSHShell","Name":"web","Host":"web.example.com",
             "Group":"Customers\\ACME\\Web"}
        ]}"#;

        let result = RdmImporter::new()
            .import_from_content(json)
            .expect("import should succeed");

        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.groups.len(), 3);

        let customers = result
            .groups
            .iter()
            .find(|g| g.name == "Customers")
            .expect("Customers group");
        let acme = result
            .groups
            .iter()
            .find(|g| g.name == "ACME")
            .expect("ACME group");
        let web = result
            .groups
            .iter()
            .find(|g| g.name == "Web")
            .expect("Web group");

        assert!(customers.parent_id.is_none());
        assert_eq!(acme.parent_id, Some(customers.id));
        assert_eq!(web.parent_id, Some(acme.id));
        assert_eq!(result.connections[0].group_id, Some(web.id));
    }

    #[test]
    fn imports_nested_and_linked_credentials() {
        let json = r#"{"Connections":[
            {"ConnectionType":"Credential","ID":"cred-1","Name":"Bob",
             "Credentials":{"UserName":"bob","Domain":"corp","Password":"s3cret"}},
            {"ConnectionType":"RDPConfigured","Name":"desk","Url":"rdp.example.com",
             "CredentialConnectionID":"cred-1"},
            {"ConnectionType":"SSHShell","Name":"shell","Host":"ssh.example.com",
             "Credentials":{"UserName":"root","Password":"hunter2"}}
        ]}"#;

        let result = RdmImporter::new()
            .import_from_content(json)
            .expect("import should succeed");

        assert_eq!(result.connections.len(), 2, "{:?}", result.skipped);

        let rdp = &result.connections[0];
        assert_eq!(rdp.username.as_deref(), Some("bob"));
        assert_eq!(rdp.domain.as_deref(), Some("corp"));
        assert_eq!(rdp.password_source, PasswordSource::Vault);
        let rdp_creds = result
            .credentials
            .get(&rdp.id)
            .expect("linked credential stored");
        assert_eq!(
            rdp_creds
                .password
                .as_ref()
                .map(|p| p.expose_secret().to_string()),
            Some("s3cret".to_string())
        );

        let ssh = &result.connections[1];
        assert_eq!(ssh.username.as_deref(), Some("root"));
        assert_eq!(ssh.password_source, PasswordSource::Vault);
        assert!(result.credentials.contains_key(&ssh.id));
    }

    #[test]
    fn reports_unsupported_and_hostless_entries() {
        let json = r#"{"Connections":[
            {"ConnectionType":5,"Name":"portal","Url":"https://example.com"},
            {"ConnectionType":"SSHShell","Name":"broken"}
        ]}"#;

        let result = RdmImporter::new()
            .import_from_content(json)
            .expect("import should succeed");

        assert!(result.connections.is_empty());
        assert_eq!(result.skipped.len(), 2);
        assert!(
            result.skipped[0].reason.contains('5'),
            "unknown numeric type must be named: {}",
            result.skipped[0].reason
        );
        assert!(result.skipped[1].reason.contains("no host"));
    }

    /// Regression test for issue #234: real RDM exports may contain integer
    /// values in fields the documented format encodes as strings (Port inside
    /// a URL field, numeric IDs, numeric passwords for PIN-based devices).
    #[test]
    fn tolerant_parsing_accepts_numeric_field_values() {
        // Simulates a real Devolutions export where:
        // - ConnectionType is numeric (77 = SSHShell)
        // - Folders[].ID and ParentID are bare integers
        // - Port is a bare integer
        // - Description is absent (null)
        // - A connection ParentID is numeric
        // - Password is a numeric PIN
        let json = r#"{"Folders":[
            {"ID":100,"Name":"Servers"},
            {"ID":101,"Name":"Network","ParentID":100}
        ],"Connections":[
            {"ConnectionType":77,"Name":"Router","Host":"192.168.1.1",
             "Port":22,"Username":"admin","Password":1234,
             "ParentID":101,"Description":null},
            {"ConnectionType":1,"Name":"Win2022","Url":"10.0.0.5",
             "Port":3389,"Domain":"CORP","Username":"sysadmin",
             "Credentials":{"UserName":"nested-user","Domain":"NESTED","Password":"s3cret"}}
        ]}"#;

        let result = RdmImporter::new()
            .import_from_content(json)
            .expect("import must not fail on numeric fields");

        // Two explicit folders + 2 connections
        assert_eq!(result.groups.len(), 2);
        assert_eq!(result.connections.len(), 2, "skipped: {:?}", result.skipped);

        let servers = result
            .groups
            .iter()
            .find(|group| group.name == "Servers")
            .expect("parent folder");
        let network = result
            .groups
            .iter()
            .find(|group| group.name == "Network")
            .expect("numeric child folder");
        assert_eq!(network.parent_id, Some(servers.id));

        let router = &result.connections[0];
        assert_eq!(router.group_id, Some(network.id));
        assert_eq!(router.host, "192.168.1.1");
        assert_eq!(router.port, 22);
        assert_eq!(router.username.as_deref(), Some("admin"));
        assert_eq!(router.password_source, PasswordSource::Vault);

        // Verify the numeric password was captured as "1234"
        let router_creds = result.credentials.get(&router.id).expect("credentials");
        assert_eq!(
            router_creds
                .password
                .as_ref()
                .map(|p| p.expose_secret().to_string()),
            Some("1234".to_string())
        );

        let win = &result.connections[1];
        assert_eq!(win.host, "10.0.0.5");
        assert_eq!(win.port, 3389);
        // Flat username wins over nested
        assert_eq!(win.username.as_deref(), Some("sysadmin"));
        assert_eq!(win.domain.as_deref(), Some("CORP"));
    }

    /// Regression test for issue #234: the reported failure was a bare
    /// `ConnectionType` of 25 (`Group`) together with an integer port, in the
    /// single-line layout a real export uses.
    #[test]
    fn accepts_integer_connection_type_and_port() {
        let json = concat!(
            r#"{"Connections":[{"ID":"f1","Group":"","ConnectionType":25,"Name":"Prod"},"#,
            r#"{"ConnectionType":77,"Name":"mail","Host":"mail.example.com","Port":25},"#,
            r#"{"ConnectionType":1,"Name":"desk","Url":"desk.example.com","Port":3389},"#,
            r#"{"ConnectionType":4,"Name":"kvm","Host":"kvm.example.com","Port":5901}]}"#
        );

        let result = RdmImporter::new()
            .import_from_content(json)
            .expect("integer ConnectionType and Port must be accepted");

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(result.connections.len(), 3, "skipped: {:?}", result.skipped);
        assert_eq!(result.groups.len(), 1, "type 25 must become a group");
        assert_eq!(result.groups[0].name, "Prod");

        // Port 25 is a legitimate integer port and must survive as such.
        let mail = &result.connections[0];
        assert_eq!(mail.protocol, ProtocolType::Ssh);
        assert_eq!(mail.port, 25);
        assert_eq!(result.connections[1].port, 3389);
        assert_eq!(result.connections[2].port, 5901);
    }

    /// A port may also arrive as a quoted number or in the `3389.0` form .NET
    /// serializers produce for an integral value.
    #[test]
    fn accepts_every_scalar_port_form() {
        let json = r#"{"Connections":[
            {"ConnectionType":77,"Name":"a","Host":"a.example.com","Port":"2222"},
            {"ConnectionType":77,"Name":"b","Host":"b.example.com","Port":2200.0},
            {"ConnectionType":77,"Name":"c","Host":"c.example.com","Port":-1},
            {"ConnectionType":77,"Name":"d","Host":"d.example.com","Port":70000},
            {"ConnectionType":77,"Name":"e","Host":"e.example.com","Port":22.5}
        ]}"#;

        let result = RdmImporter::new()
            .import_from_content(json)
            .expect("import should succeed");

        assert_eq!(result.connections.len(), 5, "skipped: {:?}", result.skipped);
        assert_eq!(result.connections[0].port, 2222);
        assert_eq!(result.connections[1].port, 2200);
        // Out-of-range and fractional values are not ports; the SSH default wins.
        assert_eq!(result.connections[2].port, 22);
        assert_eq!(result.connections[3].port, 22);
        assert_eq!(result.connections[4].port, 22);
    }

    /// `Name` was the last field that still demanded a JSON string, so an
    /// entry named after a number aborted the whole import.
    #[test]
    fn accepts_numeric_names_and_flags() {
        let json = r#"{"Folders":[{"ID":"f1","Name":404}],"Connections":[
            {"ConnectionType":4,"Name":25,"Host":"vnc.example.com","ViewOnly":1,"ParentID":"f1"},
            {"ConnectionType":4,"Name":"editable","Host":"rw.example.com","ViewOnly":"false"},
            {"ConnectionType":4,"Name":"watched","Host":"ro.example.com","ViewOnly":"yes"}
        ]}"#;

        let result = RdmImporter::new()
            .import_from_content(json)
            .expect("numeric names must not abort the parse");

        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].name, "404");
        assert_eq!(result.connections.len(), 3, "skipped: {:?}", result.skipped);
        assert_eq!(result.connections[0].name, "25");

        let view_only = |index: usize| match &result.connections[index].protocol_config {
            ProtocolConfig::Vnc(config) => config.view_only,
            other => panic!("expected VNC config, got {other:?}"),
        };
        assert!(view_only(0), "ViewOnly 1 means true");
        assert!(!view_only(1), "ViewOnly \"false\" means false");
        assert!(view_only(2), "ViewOnly \"yes\" means true");
    }

    /// One entry the parser cannot read must not cost the whole file.
    #[test]
    fn one_unreadable_entry_does_not_abort_the_import() {
        // `Credentials` is an array rather than the documented object, and
        // `Connections` holds a bare string where an entry belongs.
        let json = r#"{"Connections":[
            {"ConnectionType":77,"Name":"good","Host":"good.example.com"},
            {"ConnectionType":77,"Name":"odd","Host":"odd.example.com","Credentials":["x"]},
            "not-an-entry",
            {"ConnectionType":77,"Name":"also-good","Host":"also.example.com"}
        ]}"#;

        let result = RdmImporter::new()
            .import_from_content(json)
            .expect("a single bad entry must not fail the import");

        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(result.connections.len(), 3, "skipped: {:?}", result.skipped);
        // The unusable Credentials array is dropped, the entry itself survives.
        assert_eq!(result.connections[1].name, "odd");
        assert!(result.connections[1].username.is_none());

        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].identifier, "Connection #3");
        assert!(
            result.skipped[0].reason.contains("could not be read"),
            "reason should explain the entry was unreadable: {}",
            result.skipped[0].reason
        );
    }

    /// RDM's `Clipboard - Copy` yields one entry object and PowerShell
    /// pipelines yield a bare array; both are accepted.
    #[test]
    fn accepts_bare_array_and_single_entry() {
        let array = r#"[{"ConnectionType":77,"Name":"shell","Host":"shell.example.com"}]"#;
        let single = r#"{"ConnectionType":1,"Name":"desk","Url":"desk.example.com","Port":3390}"#;

        let importer = RdmImporter::new();

        let from_array = importer
            .import_from_content(array)
            .expect("a bare array is a valid export shape");
        assert_eq!(from_array.connections.len(), 1);
        assert_eq!(from_array.connections[0].host, "shell.example.com");

        let from_single = importer
            .import_from_content(single)
            .expect("a single copied entry is a valid shape");
        assert_eq!(from_single.connections.len(), 1);
        assert_eq!(from_single.connections[0].port, 3390);
    }

    /// An unrelated JSON document must still be reported as a parse error
    /// rather than silently importing nothing.
    #[test]
    fn rejects_json_that_is_not_an_rdm_export() {
        let error = RdmImporter::new()
            .import_from_content(r#"{"servers":{"web":{"addr":"1.2.3.4"}}}"#)
            .expect_err("an unrelated object is not an export");

        assert!(
            matches!(error, ImportError::ParseError { .. }),
            "expected a parse error, got {error:?}"
        );

        let error = RdmImporter::new()
            .import_from_content("42")
            .expect_err("a bare number is not an export");
        assert!(matches!(error, ImportError::ParseError { .. }));
    }

    /// A name plus a host is the shape of countless unrelated inventory files,
    /// so it must not pass as one copied RDM entry.
    #[test]
    fn rejects_name_and_host_object_that_is_not_an_entry() {
        let error = RdmImporter::new()
            .import_from_content(r#"{"Name":"router","Host":"10.0.0.1"}"#)
            .expect_err("a bare Name plus Host object is not an RDM export");

        assert!(
            matches!(error, ImportError::ParseError { .. }),
            "expected a parse error, got {error:?}"
        );
    }

    /// Every port form that is not a usable port falls back to the protocol
    /// default; a quoted integral float is a usable port.
    #[test]
    fn unusable_port_forms_fall_back_to_the_protocol_default() {
        let json = r#"{"Connections":[
            {"ConnectionType":77,"Name":"zero","Host":"zero.example.com","Port":0},
            {"ConnectionType":77,"Name":"zero-text","Host":"zero-text.example.com","Port":"0"},
            {"ConnectionType":77,"Name":"over","Host":"over.example.com","Port":65536},
            {"ConnectionType":77,"Name":"blank","Host":"blank.example.com","Port":""},
            {"ConnectionType":77,"Name":"absent","Host":"absent.example.com","Port":null},
            {"ConnectionType":77,"Name":"quoted-float","Host":"qf.example.com","Port":"3389.0"}
        ]}"#;

        let result = RdmImporter::new()
            .import_from_content(json)
            .expect("import should succeed");

        assert_eq!(result.connections.len(), 6, "skipped: {:?}", result.skipped);
        let port = |name: &str| {
            result
                .connections
                .iter()
                .find(|conn| conn.name == name)
                .map(|conn| conn.port)
                .expect("every entry must be imported")
        };

        // Port 0 is not a reachable endpoint, in either encoding.
        assert_eq!(port("zero"), 22);
        assert_eq!(port("zero-text"), 22);
        assert_eq!(port("over"), 22, "65536 is outside the u16 range");
        assert_eq!(port("blank"), 22, "an empty string sets no port");
        assert_eq!(port("absent"), 22, "null sets no port");
        assert_eq!(
            port("quoted-float"),
            3389,
            "a quoted integral float is the same port as an unquoted one"
        );
    }

    /// A rejected `Port` must be visible instead of silently becoming the
    /// protocol default.
    #[test]
    fn rejected_port_is_reported_as_a_warning() {
        let json = r#"{"Connections":[
            {"ConnectionType":77,"Name":"typo","Host":"typo.example.com","Port":70000}
        ]}"#;

        let result = RdmImporter::new()
            .import_from_content(json)
            .expect("import should succeed");

        assert_eq!(result.connections.len(), 1, "skipped: {:?}", result.skipped);
        assert_eq!(result.connections[0].port, 22);
        assert_eq!(
            result.warnings,
            vec![crate::import::ImportWarning::PortIgnored {
                connection_name: "typo".to_string(),
                rejected_port: "70000".to_string(),
                port: 22,
            }],
            "the warning must name the entry and the ignored value"
        );
    }

    /// A blank `ConnectionType` names no protocol, an absent one means RDP.
    #[test]
    fn blank_connection_type_is_reported_while_an_absent_one_means_rdp() {
        let json = r#"{"Connections":[
            {"ConnectionType":"","Name":"blank","Host":"blank.example.com"},
            {"ConnectionType":"   ","Name":"spaces","Host":"spaces.example.com"},
            {"Name":"absent","Host":"absent.example.com"}
        ]}"#;

        let result = RdmImporter::new()
            .import_from_content(json)
            .expect("import should succeed");

        assert_eq!(result.connections.len(), 1, "skipped: {:?}", result.skipped);
        assert_eq!(result.connections[0].name, "absent");
        assert_eq!(
            result.connections[0].protocol,
            ProtocolType::Rdp,
            "an absent ConnectionType keeps RDM's documented default"
        );

        assert_eq!(result.skipped.len(), 2);
        assert_eq!(result.skipped[0].identifier, "blank");
        assert_eq!(result.skipped[1].identifier, "spaces");
        for entry in &result.skipped {
            assert!(
                entry.reason.contains("connection type"),
                "a blank type must be reported, not guessed: {}",
                entry.reason
            );
        }
    }

    /// The fallback label accepts every scalar `Name` the deserializer does.
    #[test]
    fn entry_label_accepts_the_same_scalars_as_the_name_field() {
        let label = |json: &str| {
            let value: serde_json::Value =
                serde_json::from_str(json).expect("fixture must be valid JSON");
            RdmImporter::entry_label(&value, "Connection", 0)
        };

        assert_eq!(label(r#"{"Name":true}"#), "true", "Name may be a bool");
        assert_eq!(label(r#"{"Name":404}"#), "404");
        assert_eq!(label(r#"{"Name":"  spaced  "}"#), "spaced");
        assert_eq!(
            label(r#"{"Host":"h.example.com"}"#),
            "Connection #1",
            "a nameless entry keeps its position"
        );
    }
}
