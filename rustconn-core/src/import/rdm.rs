//! Remote Desktop Manager (RDM) JSON importer.
//!
//! Parses RDM export files in JSON format.
//! Supports importing SSH, RDP, VNC, and Telnet connections with folder hierarchy.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ImportError;
use crate::models::{
    AutomationConfig, Connection, ConnectionGroup, PasswordSource, ProtocolConfig, ProtocolType,
    RdpConfig, SshAuthMethod, SshConfig, VncConfig, WindowMode,
};
use crate::progress::ProgressReporter;

use super::normalize::parse_host_port;
use super::traits::{ImportResult, ImportSource, SkippedEntry};

/// RDM JSON connection entry
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
struct RdmConnection {
    #[serde(rename = "ID")]
    id: String,
    name: String,
    #[serde(rename = "ConnectionType")]
    connection_type: String,
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    password: Option<String>,
    domain: Option<String>,
    description: Option<String>,
    #[serde(rename = "ParentID")]
    parent_id: Option<String>,
    #[serde(rename = "GroupName")]
    group_name: Option<String>,
    // SSH specific
    #[serde(rename = "PrivateKeyPath")]
    private_key_path: Option<String>,
    // RDP specific
    #[serde(rename = "ColorDepth")]
    color_depth: Option<u16>,
    #[serde(rename = "ScreenSize")]
    screen_size: Option<String>,
    // VNC specific
    #[serde(rename = "ViewOnly")]
    view_only: Option<bool>,
}

/// RDM JSON folder entry
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
struct RdmFolder {
    #[serde(rename = "ID")]
    id: String,
    name: String,
    #[serde(rename = "ParentID")]
    parent_id: Option<String>,
    description: Option<String>,
}

/// RDM JSON export structure
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
struct RdmExport {
    connections: Option<Vec<RdmConnection>>,
    folders: Option<Vec<RdmFolder>>,
    #[serde(rename = "ExportVersion")]
    export_version: Option<String>,
    #[serde(rename = "ApplicationVersion")]
    application_version: Option<String>,
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
    /// Parses the provided JSON string as an RDM export file and converts
    /// the connections and folders to RustConn format.
    ///
    /// # Arguments
    ///
    /// * `content` - The JSON content to parse
    ///
    /// # Returns
    ///
    /// An `ImportResult` containing the converted connections and groups.
    ///
    /// # Errors
    ///
    /// Returns `ImportError::ParseError` if:
    /// - The JSON is malformed or cannot be parsed
    /// - Required fields are missing from connection entries
    /// - Connection types are not supported (only SSH, RDP, VNC, Telnet)
    pub fn import_from_content(&self, content: &str) -> Result<ImportResult, ImportError> {
        let rdm_data: RdmExport =
            serde_json::from_str(content).map_err(|e| ImportError::ParseError {
                source_name: "RDM JSON".to_string(),
                reason: format!("Failed to parse JSON: {e}"),
            })?;

        let mut result = ImportResult::new();
        // Maps RDM folder ID -> RustConn group UUID
        let mut group_map: HashMap<String, Uuid> = HashMap::new();

        // First pass: create groups/folders and build the ID mapping
        // We need to process folders in order to handle parent references
        if let Some(folders) = &rdm_data.folders {
            // First, create all groups without parent references
            for folder in folders {
                let group_id = Uuid::new_v4();
                group_map.insert(folder.id.clone(), group_id);
            }

            // Second, create the actual groups with resolved parent IDs
            for folder in folders {
                let group = Self::create_group_from_folder(folder, &group_map);
                result.add_group(group);
            }
        }

        // Second pass: create connections
        if let Some(connections) = &rdm_data.connections {
            for conn in connections {
                match Self::create_connection_from_rdm(conn, &group_map) {
                    Ok(connection) => result.add_connection(connection),
                    Err(e) => {
                        result.add_skipped(SkippedEntry::new(
                            &conn.name,
                            format!("Failed to convert connection: {e}"),
                        ));
                    }
                }
            }
        }

        Ok(result)
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

        ConnectionGroup {
            // Use the pre-allocated UUID from the group_map
            id: group_map
                .get(&folder.id)
                .copied()
                .unwrap_or_else(Uuid::new_v4),
            name: folder.name.clone(),
            parent_id,
            expanded: true,
            created_at: chrono::Utc::now(),
            sort_order: 0,
            username: None,
            domain: None,
            password_source: None,
        }
    }

    /// Parses protocol configuration from RDM connection type
    fn parse_protocol(conn: &RdmConnection) -> Result<(ProtocolType, ProtocolConfig, u16), String> {
        match conn.connection_type.to_lowercase().as_str() {
            "ssh" | "ssh2" => {
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
            "rdp" | "rdp2" => Ok((
                ProtocolType::Rdp,
                ProtocolConfig::Rdp(RdpConfig::default()),
                3389,
            )),
            "vnc" => {
                let vnc_config = VncConfig {
                    view_only: conn.view_only.unwrap_or(false),
                    ..Default::default()
                };
                Ok((ProtocolType::Vnc, ProtocolConfig::Vnc(vnc_config), 5900))
            }
            "telnet" => Ok((
                ProtocolType::Ssh,
                ProtocolConfig::Ssh(SshConfig::default()),
                23,
            )),
            other => Err(format!("Unsupported connection type: {other}")),
        }
    }

    /// Creates a connection from RDM connection data
    fn create_connection_from_rdm(
        conn: &RdmConnection,
        group_map: &HashMap<String, Uuid>,
    ) -> Result<Connection, ImportError> {
        let host_raw = conn.host.as_ref().ok_or_else(|| ImportError::ParseError {
            source_name: "RDM JSON".to_string(),
            reason: format!("Connection '{}' missing host", conn.name),
        })?;

        // Use shared utility for host:port parsing
        let (host, parsed_port) = parse_host_port(host_raw);

        let (protocol, protocol_config, default_port) =
            Self::parse_protocol(conn).map_err(|reason| ImportError::ParseError {
                source_name: "RDM JSON".to_string(),
                reason,
            })?;

        // Use parsed port from host string, or connection port, or default
        let port = parsed_port.or(conn.port).unwrap_or(default_port);

        let password_source = conn.password.as_ref().map_or(PasswordSource::None, |p| {
            if p.is_empty() {
                PasswordSource::None
            } else {
                PasswordSource::Stored
            }
        });

        let group_id = conn
            .parent_id
            .as_ref()
            .and_then(|pid| group_map.get(pid).copied());
        let now = chrono::Utc::now();

        Ok(Connection {
            id: Uuid::new_v4(),
            name: conn.name.clone(),
            description: conn.description.clone(),
            protocol,
            host,
            port,
            username: conn.username.clone(),
            group_id,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            protocol_config,
            automation: AutomationConfig::default(),
            sort_order: 0,
            last_connected: None,
            password_source,
            domain: conn.domain.clone(),
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
        })
    }
}

impl Default for RdmImporter {
    fn default() -> Self {
        Self::new()
    }
}
