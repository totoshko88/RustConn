//! Royal TS configuration importer.
//!
//! Parses Royal TS export files (.rtsz XML format).
//! Supports importing SSH, RDP, and VNC connections with folder hierarchy.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use quick_xml::events::Event;
use quick_xml::Reader;
use uuid::Uuid;

use crate::error::ImportError;
use crate::models::{
    Connection, ConnectionGroup, PasswordSource, ProtocolConfig, RdpConfig, SshConfig, VncConfig,
};

use super::traits::{ImportResult, ImportSource, SkippedEntry};

/// Royal TS SSH connection data
#[derive(Debug, Clone, Default)]
struct SshConnectionData {
    id: String,
    name: String,
    uri: Option<String>,
    port: Option<u16>,
    parent_id: Option<String>,
    credential_id: Option<String>,
}

/// Royal TS RDP connection data
#[derive(Debug, Clone, Default)]
struct RdpConnectionData {
    id: String,
    name: String,
    uri: Option<String>,
    port: Option<u16>,
    parent_id: Option<String>,
    credential_id: Option<String>,
}

/// Royal TS VNC connection data
#[derive(Debug, Clone, Default)]
struct VncConnectionData {
    id: String,
    name: String,
    uri: Option<String>,
    port: Option<u16>,
    parent_id: Option<String>,
    credential_id: Option<String>,
}

/// Royal TS folder data
#[derive(Debug, Clone, Default)]
struct FolderData {
    id: String,
    name: String,
    parent_id: Option<String>,
}

/// Royal TS credential data
#[derive(Debug, Clone, Default)]
struct CredentialData {
    id: String,
    username: Option<String>,
    domain: Option<String>,
}

/// Importer for Royal TS .rtsz files (XML format).
pub struct RoyalTsImporter {
    custom_paths: Vec<PathBuf>,
}

impl RoyalTsImporter {
    /// Creates a new Royal TS importer
    #[must_use]
    pub const fn new() -> Self {
        Self {
            custom_paths: Vec::new(),
        }
    }

    /// Creates a new Royal TS importer with custom paths
    #[must_use]
    pub const fn with_paths(paths: Vec<PathBuf>) -> Self {
        Self {
            custom_paths: paths,
        }
    }

    /// Parses Royal TS XML content using event-based parsing
    #[allow(clippy::too_many_lines)]
    #[must_use]
    pub fn parse_xml(&self, content: &str, source_path: &str) -> ImportResult {
        let mut result = ImportResult::new();

        // Remove BOM if present
        let content = content.trim_start_matches('\u{feff}');

        let mut reader = Reader::from_str(content);
        reader.config_mut().trim_text(true);

        let mut ssh_connections: Vec<SshConnectionData> = Vec::new();
        let mut rdp_connections: Vec<RdpConnectionData> = Vec::new();
        let mut vnc_connections: Vec<VncConnectionData> = Vec::new();
        let mut folders: Vec<FolderData> = Vec::new();
        let mut credentials: Vec<CredentialData> = Vec::new();
        let mut trash_id: Option<String> = None;

        let mut current_field = String::new();
        let mut in_ssh = false;
        let mut in_rdp = false;
        let mut in_vnc = false;
        let mut in_folder = false;
        let mut in_credential = false;
        let mut in_trash = false;
        let mut current_ssh = SshConnectionData::default();
        let mut current_rdp = RdpConnectionData::default();
        let mut current_vnc = VncConnectionData::default();
        let mut current_folder = FolderData::default();
        let mut current_credential = CredentialData::default();
        let mut current_trash_id = String::new();

        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    match name.as_str() {
                        "RoyalSSHConnection" => {
                            in_ssh = true;
                            current_ssh = SshConnectionData::default();
                        }
                        "RoyalRDPConnection" => {
                            in_rdp = true;
                            current_rdp = RdpConnectionData::default();
                        }
                        "RoyalVNCConnection" => {
                            in_vnc = true;
                            current_vnc = VncConnectionData::default();
                        }
                        "RoyalFolder" => {
                            in_folder = true;
                            current_folder = FolderData::default();
                        }
                        "RoyalCredential" => {
                            in_credential = true;
                            current_credential = CredentialData::default();
                        }
                        "RoyalTrash" => {
                            in_trash = true;
                            current_trash_id.clear();
                        }
                        _ => {
                            current_field = name;
                        }
                    }
                }
                Ok(Event::End(e)) => {
                    let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                    match name.as_str() {
                        "RoyalSSHConnection" => {
                            ssh_connections.push(current_ssh.clone());
                            in_ssh = false;
                        }
                        "RoyalRDPConnection" => {
                            rdp_connections.push(current_rdp.clone());
                            in_rdp = false;
                        }
                        "RoyalVNCConnection" => {
                            vnc_connections.push(current_vnc.clone());
                            in_vnc = false;
                        }
                        "RoyalFolder" => {
                            folders.push(current_folder.clone());
                            in_folder = false;
                        }
                        "RoyalCredential" => {
                            credentials.push(current_credential.clone());
                            in_credential = false;
                        }
                        "RoyalTrash" => {
                            if !current_trash_id.is_empty() {
                                trash_id = Some(current_trash_id.clone());
                            }
                            in_trash = false;
                        }
                        _ => {}
                    }
                    current_field.clear();
                }
                Ok(Event::Text(e)) => {
                    let text = std::str::from_utf8(&e).unwrap_or_default().to_string();
                    if text.is_empty() {
                        continue;
                    }

                    if in_ssh {
                        Self::set_ssh_field(&mut current_ssh, &current_field, &text);
                    } else if in_rdp {
                        Self::set_rdp_field(&mut current_rdp, &current_field, &text);
                    } else if in_vnc {
                        Self::set_vnc_field(&mut current_vnc, &current_field, &text);
                    } else if in_folder {
                        Self::set_folder_field(&mut current_folder, &current_field, &text);
                    } else if in_credential {
                        Self::set_credential_field(&mut current_credential, &current_field, &text);
                    } else if in_trash && current_field == "ID" {
                        current_trash_id = text;
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => {
                    result.add_error(ImportError::ParseError {
                        source_name: "Royal TS".to_string(),
                        reason: format!("XML parse error: {e}"),
                    });
                    return result;
                }
                _ => {}
            }
        }

        // Build credential map
        let cred_map: HashMap<String, CredentialData> =
            credentials.into_iter().map(|c| (c.id.clone(), c)).collect();

        // Build folder map and create groups
        let (folder_map, groups) = Self::build_folder_hierarchy(&folders);
        for group in groups {
            result.add_group(group);
        }

        // Convert SSH connections (skip trashed)
        for conn in &ssh_connections {
            // Skip connections in trash
            if let Some(ref tid) = trash_id {
                if conn.parent_id.as_ref() == Some(tid) {
                    continue;
                }
            }
            if let Some(c) = Self::convert_ssh(conn, &cred_map, &folder_map) {
                result.add_connection(c);
            } else {
                result.add_skipped(SkippedEntry::with_location(
                    &conn.name,
                    "Missing host",
                    source_path,
                ));
            }
        }

        // Convert RDP connections (skip trashed)
        for conn in &rdp_connections {
            // Skip connections in trash
            if let Some(ref tid) = trash_id {
                if conn.parent_id.as_ref() == Some(tid) {
                    continue;
                }
            }
            if let Some(c) = Self::convert_rdp(conn, &cred_map, &folder_map) {
                result.add_connection(c);
            } else {
                result.add_skipped(SkippedEntry::with_location(
                    &conn.name,
                    "Missing host",
                    source_path,
                ));
            }
        }

        // Convert VNC connections (skip trashed)
        for conn in &vnc_connections {
            // Skip connections in trash
            if let Some(ref tid) = trash_id {
                if conn.parent_id.as_ref() == Some(tid) {
                    continue;
                }
            }
            if let Some(c) = Self::convert_vnc(conn, &cred_map, &folder_map) {
                result.add_connection(c);
            } else {
                result.add_skipped(SkippedEntry::with_location(
                    &conn.name,
                    "Missing host",
                    source_path,
                ));
            }
        }

        result
    }

    fn set_ssh_field(conn: &mut SshConnectionData, field: &str, value: &str) {
        match field {
            "ID" => conn.id = value.to_string(),
            "Name" => conn.name = value.to_string(),
            "URI" => conn.uri = Some(value.to_string()),
            "Port" => conn.port = value.parse().ok(),
            "ParentID" => conn.parent_id = Some(value.to_string()),
            "CredentialId" => conn.credential_id = Some(value.to_string()),
            _ => {}
        }
    }

    fn set_rdp_field(conn: &mut RdpConnectionData, field: &str, value: &str) {
        match field {
            "ID" => conn.id = value.to_string(),
            "Name" => conn.name = value.to_string(),
            "URI" => conn.uri = Some(value.to_string()),
            "Port" => conn.port = value.parse().ok(),
            "ParentID" => conn.parent_id = Some(value.to_string()),
            "CredentialId" => conn.credential_id = Some(value.to_string()),
            _ => {}
        }
    }

    fn set_vnc_field(conn: &mut VncConnectionData, field: &str, value: &str) {
        match field {
            "ID" => conn.id = value.to_string(),
            "Name" => conn.name = value.to_string(),
            "URI" => conn.uri = Some(value.to_string()),
            "Port" | "VNCPort" => conn.port = value.parse().ok(),
            "ParentID" => conn.parent_id = Some(value.to_string()),
            "CredentialId" => conn.credential_id = Some(value.to_string()),
            _ => {}
        }
    }

    fn set_folder_field(folder: &mut FolderData, field: &str, value: &str) {
        match field {
            "ID" => folder.id = value.to_string(),
            "Name" => folder.name = value.to_string(),
            "ParentID" => folder.parent_id = Some(value.to_string()),
            _ => {}
        }
    }

    fn set_credential_field(cred: &mut CredentialData, field: &str, value: &str) {
        match field {
            "ID" => cred.id = value.to_string(),
            "UserName" => cred.username = Some(value.to_string()),
            "Domain" => cred.domain = Some(value.to_string()),
            _ => {}
        }
    }

    fn build_folder_hierarchy(
        folders: &[FolderData],
    ) -> (HashMap<String, Uuid>, Vec<ConnectionGroup>) {
        let mut id_map: HashMap<String, Uuid> = HashMap::new();
        let mut groups = Vec::new();

        // First pass: create UUIDs
        for folder in folders {
            if !folder.id.is_empty() {
                id_map.insert(folder.id.clone(), Uuid::new_v4());
            }
        }

        // Second pass: create groups
        for folder in folders {
            if folder.id.is_empty() || folder.name.is_empty() {
                continue;
            }
            let new_id = id_map.get(&folder.id).copied().unwrap_or_else(Uuid::new_v4);
            let parent_uuid = folder
                .parent_id
                .as_ref()
                .and_then(|pid| id_map.get(pid).copied());

            let group = parent_uuid.map_or_else(
                || {
                    let mut g = ConnectionGroup::new(folder.name.clone());
                    g.id = new_id;
                    g
                },
                |parent_id| {
                    let mut g = ConnectionGroup::with_parent(folder.name.clone(), parent_id);
                    g.id = new_id;
                    g
                },
            );
            groups.push(group);
        }

        (id_map, groups)
    }

    fn convert_ssh(
        conn: &SshConnectionData,
        credentials: &HashMap<String, CredentialData>,
        folder_map: &HashMap<String, Uuid>,
    ) -> Option<Connection> {
        let host = conn.uri.as_ref().filter(|h| !h.is_empty())?;
        let port = conn.port.unwrap_or(22);

        let mut connection = Connection::new(
            conn.name.clone(),
            host.clone(),
            port,
            ProtocolConfig::Ssh(SshConfig::default()),
        );

        if let Some(cred_id) = &conn.credential_id {
            if let Some(cred) = credentials.get(cred_id) {
                connection.username.clone_from(&cred.username);
                connection.password_source = PasswordSource::Prompt;
            }
        }

        if let Some(parent_id) = &conn.parent_id {
            if let Some(group_id) = folder_map.get(parent_id) {
                connection.group_id = Some(*group_id);
            }
        }

        Some(connection)
    }

    fn convert_rdp(
        conn: &RdpConnectionData,
        credentials: &HashMap<String, CredentialData>,
        folder_map: &HashMap<String, Uuid>,
    ) -> Option<Connection> {
        let host = conn.uri.as_ref().filter(|h| !h.is_empty())?;
        let port = conn.port.unwrap_or(3389);

        let mut connection = Connection::new(
            conn.name.clone(),
            host.clone(),
            port,
            ProtocolConfig::Rdp(RdpConfig::default()),
        );

        if let Some(cred_id) = &conn.credential_id {
            if let Some(cred) = credentials.get(cred_id) {
                connection.username.clone_from(&cred.username);
                connection.domain.clone_from(&cred.domain);
                connection.password_source = PasswordSource::Prompt;
            }
        }

        if let Some(parent_id) = &conn.parent_id {
            if let Some(group_id) = folder_map.get(parent_id) {
                connection.group_id = Some(*group_id);
            }
        }

        Some(connection)
    }

    fn convert_vnc(
        conn: &VncConnectionData,
        credentials: &HashMap<String, CredentialData>,
        folder_map: &HashMap<String, Uuid>,
    ) -> Option<Connection> {
        let host = conn.uri.as_ref().filter(|h| !h.is_empty())?;
        let port = conn.port.unwrap_or(5900);

        let mut connection = Connection::new(
            conn.name.clone(),
            host.clone(),
            port,
            ProtocolConfig::Vnc(VncConfig::default()),
        );

        if let Some(cred_id) = &conn.credential_id {
            if credentials.contains_key(cred_id) {
                connection.password_source = PasswordSource::Prompt;
            }
        }

        if let Some(parent_id) = &conn.parent_id {
            if let Some(group_id) = folder_map.get(parent_id) {
                connection.group_id = Some(*group_id);
            }
        }

        Some(connection)
    }
}

impl Default for RoyalTsImporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportSource for RoyalTsImporter {
    fn source_id(&self) -> &'static str {
        "royalts"
    }

    fn display_name(&self) -> &'static str {
        "Royal TS"
    }

    fn is_available(&self) -> bool {
        !self.custom_paths.is_empty() && self.custom_paths.iter().any(|p| p.exists())
    }

    fn default_paths(&self) -> Vec<PathBuf> {
        if !self.custom_paths.is_empty() {
            return self.custom_paths.clone();
        }
        Vec::new()
    }

    fn import(&self) -> Result<ImportResult, ImportError> {
        let paths = self.default_paths();
        if paths.is_empty() {
            return Err(ImportError::FileNotFound(PathBuf::from(
                "No Royal TS file specified",
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
            source_name: "Royal TS".to_string(),
            reason: format!("Failed to read {}: {e}", path.display()),
        })?;

        Ok(self.parse_xml(&content, &path.display().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ssh_connection() {
        let importer = RoyalTsImporter::new();
        let content = r#"<?xml version="1.0" encoding="utf-8"?>
<RTSZDocument>
  <RoyalSSHConnection>
    <ID>conn1</ID>
    <Name>My SSH Server</Name>
    <URI>192.168.1.100</URI>
    <Port>22</Port>
  </RoyalSSHConnection>
</RTSZDocument>"#;

        let result = importer.parse_xml(content, "test.rtsz");
        assert_eq!(result.connections.len(), 1);
        assert!(result.errors.is_empty());

        let conn = &result.connections[0];
        assert_eq!(conn.name, "My SSH Server");
        assert_eq!(conn.host, "192.168.1.100");
        assert_eq!(conn.port, 22);
    }

    #[test]
    fn test_parse_multiple_ssh_connections() {
        let importer = RoyalTsImporter::new();
        let content = r#"<?xml version="1.0" encoding="utf-8"?>
<RTSZDocument>
  <RoyalSSHConnection>
    <ID>conn1</ID>
    <Name>Server 1</Name>
    <URI>server1.example.com</URI>
    <Port>22</Port>
  </RoyalSSHConnection>
  <RoyalSSHConnection>
    <ID>conn2</ID>
    <Name>Server 2</Name>
    <URI>server2.example.com</URI>
    <Port>2222</Port>
  </RoyalSSHConnection>
  <RoyalSSHConnection>
    <ID>conn3</ID>
    <Name>Server 3</Name>
    <URI>server3.example.com</URI>
  </RoyalSSHConnection>
</RTSZDocument>"#;

        let result = importer.parse_xml(content, "test.rtsz");
        assert_eq!(result.connections.len(), 3);
        assert!(result.errors.is_empty());

        assert_eq!(result.connections[0].name, "Server 1");
        assert_eq!(result.connections[1].name, "Server 2");
        assert_eq!(result.connections[1].port, 2222);
        assert_eq!(result.connections[2].name, "Server 3");
        assert_eq!(result.connections[2].port, 22); // default
    }

    #[test]
    fn test_parse_with_credential() {
        let importer = RoyalTsImporter::new();
        let content = r#"<?xml version="1.0" encoding="utf-8"?>
<RTSZDocument>
  <RoyalCredential>
    <ID>cred1</ID>
    <Name>Root</Name>
    <UserName>root</UserName>
  </RoyalCredential>
  <RoyalSSHConnection>
    <ID>conn1</ID>
    <Name>Server</Name>
    <URI>server.example.com</URI>
    <CredentialId>cred1</CredentialId>
  </RoyalSSHConnection>
</RTSZDocument>"#;

        let result = importer.parse_xml(content, "test.rtsz");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert_eq!(conn.username, Some("root".to_string()));
        assert_eq!(conn.password_source, PasswordSource::Prompt);
    }

    #[test]
    fn test_parse_folder_hierarchy() {
        let importer = RoyalTsImporter::new();
        let content = r#"<?xml version="1.0" encoding="utf-8"?>
<RTSZDocument>
  <RoyalFolder>
    <ID>folder1</ID>
    <Name>Production</Name>
  </RoyalFolder>
  <RoyalFolder>
    <ID>folder2</ID>
    <Name>Web Servers</Name>
    <ParentID>folder1</ParentID>
  </RoyalFolder>
  <RoyalSSHConnection>
    <ID>conn1</ID>
    <Name>Web01</Name>
    <URI>web01.example.com</URI>
    <ParentID>folder2</ParentID>
  </RoyalSSHConnection>
</RTSZDocument>"#;

        let result = importer.parse_xml(content, "test.rtsz");
        assert_eq!(result.groups.len(), 2);
        assert_eq!(result.connections.len(), 1);

        let production = result
            .groups
            .iter()
            .find(|g| g.name == "Production")
            .unwrap();
        let web_servers = result
            .groups
            .iter()
            .find(|g| g.name == "Web Servers")
            .unwrap();
        assert!(production.parent_id.is_none());
        assert_eq!(web_servers.parent_id, Some(production.id));

        let conn = &result.connections[0];
        assert_eq!(conn.group_id, Some(web_servers.id));
    }

    #[test]
    fn test_skip_no_host() {
        let importer = RoyalTsImporter::new();
        let content = r#"<?xml version="1.0" encoding="utf-8"?>
<RTSZDocument>
  <RoyalSSHConnection>
    <ID>conn1</ID>
    <Name>No Host</Name>
  </RoyalSSHConnection>
</RTSZDocument>"#;

        let result = importer.parse_xml(content, "test.rtsz");
        assert_eq!(result.connections.len(), 0);
        assert_eq!(result.skipped.len(), 1);
    }

    #[test]
    fn test_skip_trashed_connections() {
        let importer = RoyalTsImporter::new();
        let content = r#"<?xml version="1.0" encoding="utf-8"?>
<RTSZDocument>
  <RoyalTrash>
    <ID>trash-folder-id</ID>
    <Name>Trash</Name>
  </RoyalTrash>
  <RoyalSSHConnection>
    <ID>conn1</ID>
    <Name>Active Server</Name>
    <URI>active.example.com</URI>
  </RoyalSSHConnection>
  <RoyalSSHConnection>
    <ID>conn2</ID>
    <Name>Deleted Server</Name>
    <URI>deleted.example.com</URI>
    <ParentID>trash-folder-id</ParentID>
  </RoyalSSHConnection>
  <RoyalRDPConnection>
    <ID>rdp1</ID>
    <Name>Deleted RDP</Name>
    <URI>deleted-rdp.example.com</URI>
    <ParentID>trash-folder-id</ParentID>
  </RoyalRDPConnection>
</RTSZDocument>"#;

        let result = importer.parse_xml(content, "test.rtsz");
        // Only the active server should be imported, trashed ones skipped
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.connections[0].name, "Active Server");
    }
}
