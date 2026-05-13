//! Group Sync export format and file I/O.
//!
//! Defines the [`GroupSyncExport`] file format used by Group Sync (Master/Import
//! model). Each root group exports to a single `.rcn` JSON file containing the
//! group hierarchy, connections (without local-only fields), and variable
//! templates.
//!
//! File I/O uses atomic writes (temp file + rename) so readers never see
//! partial or corrupt JSON.

use std::io::{BufReader, BufWriter, Write};
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use std::collections::HashSet;

use crate::automation::ConnectionTask;
use crate::models::{
    AutomationConfig, Connection, ConnectionGroup, CustomProperty, HighlightRule, PasswordSource,
    ProtocolConfig, ProtocolType, SshAuthMethod,
};
use crate::variables::Variable;
use crate::wol::WolConfig;

use super::variable_template::VariableTemplate;

/// Errors that can occur during sync file operations.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// File I/O error (read, write, rename).
    #[error("Sync file I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization or deserialization error.
    #[error("Sync file format error: {0}")]
    InvalidFormat(#[from] serde_json::Error),

    /// The sync file has an unsupported version.
    #[error("Unsupported sync version {version}, expected {expected}")]
    UnsupportedVersion {
        /// Version found in the file.
        version: u32,
        /// Version this build supports.
        expected: u32,
    },

    /// The sync file has an unexpected `sync_type`.
    #[error("Unexpected sync type \"{found}\", expected \"{expected}\"")]
    UnexpectedSyncType {
        /// Type found in the file.
        found: String,
        /// Type expected by the caller.
        expected: String,
    },

    /// The sync directory is not configured.
    #[error("Sync directory is not configured")]
    SyncDirNotConfigured,

    /// The requested group was not found.
    #[error("Group not found: {0}")]
    GroupNotFound(Uuid),

    /// The group is not in Master sync mode.
    #[error("Group is not in Master sync mode: {0}")]
    NotMasterGroup(Uuid),

    /// The group is not in Import sync mode.
    #[error("Group is not in Import sync mode: {0}")]
    NotImportGroup(Uuid),

    /// The group is not a root group (has a parent).
    #[error("Group is not a root group: {0}")]
    NotRootGroup(Uuid),

    /// The sync directory does not exist or is not writable.
    #[error("Sync directory is not writable: {0}")]
    SyncDirNotWritable(std::path::PathBuf),

    /// The sync filename contains path traversal or invalid characters.
    #[error("Invalid sync filename: {0}")]
    InvalidSyncFilename(String),
}

/// Current sync format version.
const SYNC_VERSION: u32 = 1;

/// Expected `sync_type` value for group exports.
const SYNC_TYPE_GROUP: &str = "group";

/// A complete Group Sync export — the on-disk `.rcn` file format.
///
/// Contains the root group, all subgroups (path-based), connections (without
/// local-only fields), and variable templates referenced by those connections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupSyncExport {
    /// Format version (currently 1).
    pub sync_version: u32,

    /// Always `"group"` for Group Sync files.
    pub sync_type: String,

    /// Timestamp when this export was created.
    pub exported_at: DateTime<Utc>,

    /// Application version that produced this export.
    pub app_version: String,

    /// Device ID of the Master that exported this file.
    pub master_device_id: Uuid,

    /// Human-readable name of the Master device.
    pub master_device_name: String,

    /// The root group being exported.
    pub root_group: SyncGroup,

    /// Subgroups within the root group (path-based hierarchy).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<SyncGroup>,

    /// Connections belonging to the root group and its subgroups.
    /// Local-only fields are excluded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<SyncConnection>,

    /// Variable templates referenced by the exported connections.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variable_templates: Vec<VariableTemplate>,
}

/// A group in the sync export, identified by its hierarchical path.
///
/// Local-only fields (`ssh_key_path`, `ssh_agent_socket`, `expanded`,
/// `sort_order`) are excluded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncGroup {
    /// Group display name.
    pub name: String,

    /// Hierarchical path, e.g. `"Production/Web"`.
    /// Empty string for the root group.
    #[serde(default)]
    pub path: String,

    /// Optional description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Custom icon (emoji or GTK icon name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    /// Inherited username.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,

    /// Inherited domain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,

    /// SSH auth method for inheritance (synced).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_auth_method: Option<SshAuthMethod>,

    /// SSH ProxyJump for inheritance (synced).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_proxy_jump: Option<String>,
}

/// A connection in the sync export.
///
/// Contains only synced fields — local-only fields (`last_connected`,
/// `sort_order`, `is_pinned`, `pin_order`, `window_geometry`, `window_mode`,
/// `remember_window_position`, `skip_port_check`, `ssh_key_path`) are excluded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncConnection {
    /// Connection name — primary key for merge within a group.
    pub name: String,

    /// Hierarchical group path, e.g. `"Production Servers/Web"`.
    pub group_path: String,

    /// Remote host address.
    pub host: String,

    /// Remote port number.
    pub port: u16,

    /// Protocol type.
    pub protocol: ProtocolType,

    /// Username for authentication.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,

    /// Optional description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Tags for organization.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// Protocol-specific configuration.
    pub protocol_config: ProtocolConfig,

    /// Credential source (variable names only, no secret values).
    #[serde(default)]
    pub password_source: PasswordSource,

    /// Automation configuration (expect rules, post-login scripts).
    #[serde(default)]
    pub automation: AutomationConfig,

    /// Custom metadata properties.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_properties: Vec<CustomProperty>,

    /// Task to run before connecting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_connect_task: Option<ConnectionTask>,

    /// Task to run after disconnecting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_disconnect_task: Option<ConnectionTask>,

    /// Wake-on-LAN configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wol_config: Option<WolConfig>,

    /// Custom icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,

    /// Per-connection highlight rules.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub highlight_rules: Vec<HighlightRule>,

    /// Last modification timestamp (used for conflict resolution).
    pub updated_at: DateTime<Utc>,
}

impl SyncConnection {
    /// Converts a [`Connection`] into a [`SyncConnection`], filtering out
    /// local-only fields.
    ///
    /// Excluded fields: `last_connected`, `sort_order`, `is_pinned`,
    /// `pin_order`, `window_geometry`, `window_mode`,
    /// `remember_window_position`, `skip_port_check`, and `ssh_key_path`
    /// (stripped from SSH-based protocol configs).
    #[must_use]
    pub fn from_connection(conn: &Connection, group_path: &str) -> Self {
        Self {
            name: conn.name.clone(),
            group_path: group_path.to_owned(),
            host: conn.host.clone(),
            port: conn.port,
            protocol: conn.protocol,
            username: conn.username.clone(),
            description: conn.description.clone(),
            tags: conn.tags.clone(),
            protocol_config: strip_local_only_ssh_fields(&conn.protocol_config),
            password_source: conn.password_source.clone(),
            automation: conn.automation.clone(),
            custom_properties: conn.custom_properties.clone(),
            pre_connect_task: conn.pre_connect_task.clone(),
            post_disconnect_task: conn.post_disconnect_task.clone(),
            wol_config: conn.wol_config.clone(),
            icon: conn.icon.clone(),
            highlight_rules: conn.highlight_rules.clone(),
            updated_at: conn.updated_at,
        }
    }
}

impl SyncGroup {
    /// Converts a [`ConnectionGroup`] into a [`SyncGroup`].
    ///
    /// `path` is the pre-computed hierarchical path for this group
    /// (see [`compute_group_path`]).
    ///
    /// Local-only fields are excluded: `ssh_key_path`, `ssh_agent_socket`,
    /// `expanded`, `sort_order`, `sync_mode`, `sync_file`, `last_synced_at`.
    #[must_use]
    pub fn from_group(group: &ConnectionGroup, path: &str) -> Self {
        Self {
            name: group.name.clone(),
            path: path.to_owned(),
            description: group.description.clone(),
            icon: group.icon.clone(),
            username: group.username.clone(),
            domain: group.domain.clone(),
            ssh_auth_method: group.ssh_auth_method.clone(),
            ssh_proxy_jump: group.ssh_proxy_jump.clone(),
        }
    }
}

/// Computes the hierarchical path for a group by walking up the parent chain.
///
/// Returns a `"/"` separated path like `"Production Servers/Web/Backend"`.
/// The root group's path is its own name. Cycles are handled gracefully via a
/// visited set.
///
/// # Examples
///
/// ```text
/// Root "Production" (parent_id=None) → path = "Production"
/// Child "Web" (parent_id=Production) → path = "Production/Web"
/// Grandchild "Backend" (parent_id=Web) → path = "Production/Web/Backend"
/// ```
#[must_use]
pub fn compute_group_path(group_id: Uuid, groups: &[ConnectionGroup]) -> String {
    // Collect ancestors from the target group up to the root.
    let mut segments: Vec<&str> = Vec::new();
    let mut visited = HashSet::new();
    let mut current_id = Some(group_id);

    while let Some(gid) = current_id {
        if !visited.insert(gid) {
            // Cycle detected — stop walking.
            break;
        }
        let Some(group) = groups.iter().find(|g| g.id == gid) else {
            // Orphaned reference — stop walking.
            break;
        };
        segments.push(&group.name);
        current_id = group.parent_id;
    }

    // Segments are leaf-to-root; reverse to get root-to-leaf.
    segments.reverse();
    segments.join("/")
}

/// Collects variable templates from connections that reference variables.
///
/// Scans each connection's `password_source` for [`PasswordSource::Variable`]
/// references and also scans string fields (`host`, `username`, `description`)
/// for `${var_name}` patterns. Returns a deduplicated list of
/// [`VariableTemplate`] entries.
///
/// For each unique variable name found:
/// - Looks up the variable in `existing_variables` to get description and
///   `is_secret`.
/// - For non-secret variables with a non-empty value, includes `default_value`.
/// - For secret variables, `default_value` is always `None`.
/// - If the variable is not found in `existing_variables`, creates a template
///   with `is_secret = true` and no description (safe default).
#[must_use]
pub fn collect_variable_templates(
    connections: &[Connection],
    existing_variables: &[Variable],
) -> Vec<VariableTemplate> {
    use crate::variables::VARIABLE_REGEX;

    let mut seen = HashSet::new();
    let mut templates = Vec::new();

    for conn in connections {
        // 1. Check password_source for Variable(name)
        if let PasswordSource::Variable(ref name) = conn.password_source
            && seen.insert(name.clone())
        {
            templates.push(build_template(name, existing_variables));
        }

        // 2. Scan string fields for ${var_name} patterns
        for field in variable_bearing_fields(conn) {
            for caps in VARIABLE_REGEX.captures_iter(field) {
                if let Some(m) = caps.get(1) {
                    let name = m.as_str().to_owned();
                    if seen.insert(name.clone()) {
                        templates.push(build_template(&name, existing_variables));
                    }
                }
            }
        }
    }

    templates
}

/// Returns an iterator over connection string fields that may contain
/// `${var_name}` references.
fn variable_bearing_fields(conn: &Connection) -> Vec<&str> {
    let mut fields = vec![conn.host.as_str()];
    if let Some(ref u) = conn.username {
        fields.push(u.as_str());
    }
    if let Some(ref d) = conn.description {
        fields.push(d.as_str());
    }
    fields
}

/// Builds a [`VariableTemplate`] for the given variable name by looking it up
/// in `existing_variables`.
fn build_template(name: &str, existing_variables: &[Variable]) -> VariableTemplate {
    if let Some(var) = existing_variables.iter().find(|v| v.name == name) {
        let default_value = if var.is_secret || var.value.is_empty() {
            None
        } else {
            Some(var.value.clone())
        };
        VariableTemplate {
            name: var.name.clone(),
            description: var.description.clone(),
            is_secret: var.is_secret,
            default_value,
        }
    } else {
        // Variable not found locally — assume secret (safe default).
        VariableTemplate {
            name: name.to_owned(),
            description: None,
            is_secret: true,
            default_value: None,
        }
    }
}

/// Returns a copy of `config` with local-only SSH fields removed.
///
/// For SSH and SFTP configs, `key_path` is set to `None` because it is a
/// local filesystem path that differs between devices.  All other protocol
/// configs are returned unchanged.
fn strip_local_only_ssh_fields(config: &ProtocolConfig) -> ProtocolConfig {
    match config {
        ProtocolConfig::Ssh(ssh) => ProtocolConfig::Ssh(strip_ssh_key_path(ssh)),
        ProtocolConfig::Sftp(ssh) => ProtocolConfig::Sftp(strip_ssh_key_path(ssh)),
        other => other.clone(),
    }
}

/// Returns a copy of `ssh` with `key_path` cleared (local-only field).
fn strip_ssh_key_path(ssh: &crate::models::SshConfig) -> crate::models::SshConfig {
    let mut cleaned = ssh.clone();
    cleaned.key_path = None;
    cleaned
}

impl GroupSyncExport {
    /// Writes this export to a JSON file using atomic write (temp file + rename).
    ///
    /// The file is first written to a `.tmp` sibling, then renamed into place
    /// so that concurrent readers never see partial content. When two Master
    /// instances export concurrently (a misconfiguration), the last rename to
    /// complete wins — this is the intended last-write-wins conflict resolution
    /// strategy (see design doc, Error Scenario 7).
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Io`] on file system errors or [`SyncError::InvalidFormat`]
    /// if serialization fails.
    pub fn to_file(&self, path: &Path) -> Result<(), SyncError> {
        let temp_path = path.with_extension("rcn.tmp");

        let file = std::fs::File::create(&temp_path)?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, self)?;
        writer.flush()?;

        // Atomic rename into final location.
        std::fs::rename(&temp_path, path)?;

        // Restrict file permissions to owner-only (0600) — sync files may
        // contain hostnames, usernames, automation scripts, and variable
        // references that should not be world-readable.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }

        Ok(())
    }

    /// Reads and parses a `GroupSyncExport` from a JSON file.
    ///
    /// Validates `sync_version` and `sync_type` after parsing.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Io`] on read errors, [`SyncError::InvalidFormat`] on
    /// parse errors, [`SyncError::UnsupportedVersion`] if the version is not 1,
    /// or [`SyncError::UnexpectedSyncType`] if `sync_type` is not `"group"`.
    pub fn from_file(path: &Path) -> Result<Self, SyncError> {
        let file = std::fs::File::open(path)?;
        let reader = BufReader::new(file);
        let export: Self = serde_json::from_reader(reader)?;

        if export.sync_version != SYNC_VERSION {
            return Err(SyncError::UnsupportedVersion {
                version: export.sync_version,
                expected: SYNC_VERSION,
            });
        }

        if export.sync_type != SYNC_TYPE_GROUP {
            return Err(SyncError::UnexpectedSyncType {
                found: export.sync_type,
                expected: SYNC_TYPE_GROUP.to_owned(),
            });
        }

        Ok(export)
    }

    /// Builds a `GroupSyncExport` from pre-collected data.
    ///
    /// This is a convenience constructor; the actual tree-walking logic that
    /// collects groups, connections, and variable templates from
    /// `ConnectionManager` will be implemented in tasks 2.9–2.11.
    #[must_use]
    pub fn from_group_tree(
        app_version: String,
        device_id: Uuid,
        device_name: String,
        root_group: SyncGroup,
        groups: Vec<SyncGroup>,
        connections: Vec<SyncConnection>,
        variable_templates: Vec<VariableTemplate>,
    ) -> Self {
        Self {
            sync_version: SYNC_VERSION,
            sync_type: SYNC_TYPE_GROUP.to_owned(),
            exported_at: Utc::now(),
            app_version,
            master_device_id: device_id,
            master_device_name: device_name,
            root_group,
            groups,
            connections,
            variable_templates,
        }
    }
}

/// Converts a group name to a sync filename.
///
/// Uses the `slug` crate to transliterate Unicode to ASCII, replace special
/// characters with hyphens, collapse consecutive hyphens, and strip
/// leading/trailing hyphens. The result is appended with `.rcn`.
///
/// If the slug is empty after processing (e.g. input was all special
/// characters), the fallback `"group"` is used.
///
/// # Examples
///
/// ```
/// # use rustconn_core::sync::group_export::group_name_to_filename;
/// assert_eq!(group_name_to_filename("Production Servers"), "production-servers.rcn");
/// assert_eq!(group_name_to_filename("---"), "group.rcn");
/// ```
#[must_use]
pub fn group_name_to_filename(name: &str) -> String {
    let slugified = slug::slugify(name);
    let base = if slugified.is_empty() {
        "group".to_owned()
    } else {
        slugified
    };
    format!("{base}.rcn")
}

/// Validates that a sync filename is safe (no path traversal, no absolute paths).
///
/// Returns `Ok(())` if the filename is a plain filename without directory
/// separators or `..` components. Returns `Err(SyncError::InvalidSyncFilename)`
/// otherwise.
///
/// # Errors
///
/// Returns [`SyncError::InvalidSyncFilename`] if the filename contains
/// path traversal components (`..`), directory separators, or is an absolute path.
pub fn validate_sync_filename(filename: &str) -> Result<(), SyncError> {
    use std::path::Path;

    let path = Path::new(filename);

    // Reject absolute paths
    if path.is_absolute() {
        return Err(SyncError::InvalidSyncFilename(filename.to_owned()));
    }

    // Reject any path with parent directory traversal or directory separators
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => return Err(SyncError::InvalidSyncFilename(filename.to_owned())),
        }
    }

    // Reject if it contains more than one component (subdirectory)
    if path.components().count() != 1 {
        return Err(SyncError::InvalidSyncFilename(filename.to_owned()));
    }

    Ok(())
}

/// Creates a [`Connection`] from a [`SyncConnection`], assigning it to the
/// given `root_group_id`.
///
/// Local-only fields are set to defaults (e.g. `sort_order = 0`,
/// `is_pinned = false`, `last_connected = None`).
#[must_use]
pub fn sync_connection_to_connection(sync_conn: &SyncConnection, group_id: Uuid) -> Connection {
    let mut conn = Connection::new(
        sync_conn.name.clone(),
        sync_conn.host.clone(),
        sync_conn.port,
        sync_conn.protocol_config.clone(),
    );
    conn.port = sync_conn.port;
    conn.protocol = sync_conn.protocol;
    conn.username.clone_from(&sync_conn.username);
    conn.description.clone_from(&sync_conn.description);
    conn.tags.clone_from(&sync_conn.tags);
    conn.protocol_config = sync_conn.protocol_config.clone();
    conn.password_source = sync_conn.password_source.clone();
    conn.automation = sync_conn.automation.clone();
    conn.custom_properties
        .clone_from(&sync_conn.custom_properties);
    conn.pre_connect_task
        .clone_from(&sync_conn.pre_connect_task);
    conn.post_disconnect_task
        .clone_from(&sync_conn.post_disconnect_task);
    conn.wol_config.clone_from(&sync_conn.wol_config);
    conn.icon.clone_from(&sync_conn.icon);
    conn.highlight_rules.clone_from(&sync_conn.highlight_rules);
    conn.updated_at = sync_conn.updated_at;
    conn.group_id = Some(group_id);
    conn
}

/// Updates synced fields on an existing [`Connection`] from a [`SyncConnection`].
///
/// Preserves local-only fields (`sort_order`, `is_pinned`, `pin_order`,
/// `window_geometry`, `window_mode`, `last_connected`, `ssh_key_path`,
/// `skip_port_check`).
pub fn apply_sync_connection_update(conn: &mut Connection, sync_conn: &SyncConnection) {
    conn.name.clone_from(&sync_conn.name);
    conn.host.clone_from(&sync_conn.host);
    conn.port = sync_conn.port;
    conn.protocol = sync_conn.protocol;
    conn.username.clone_from(&sync_conn.username);
    conn.description.clone_from(&sync_conn.description);
    conn.tags.clone_from(&sync_conn.tags);
    conn.protocol_config = sync_conn.protocol_config.clone();
    conn.password_source = sync_conn.password_source.clone();
    conn.automation = sync_conn.automation.clone();
    conn.custom_properties
        .clone_from(&sync_conn.custom_properties);
    conn.pre_connect_task
        .clone_from(&sync_conn.pre_connect_task);
    conn.post_disconnect_task
        .clone_from(&sync_conn.post_disconnect_task);
    conn.wol_config.clone_from(&sync_conn.wol_config);
    conn.icon.clone_from(&sync_conn.icon);
    conn.highlight_rules.clone_from(&sync_conn.highlight_rules);
    conn.updated_at = sync_conn.updated_at;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SshConfig;

    fn sample_root_group() -> SyncGroup {
        SyncGroup {
            name: "Production Servers".to_owned(),
            path: String::new(),
            description: Some("Production infrastructure".to_owned()),
            icon: Some("🏭".to_owned()),
            username: Some("deploy".to_owned()),
            domain: None,
            ssh_auth_method: Some(SshAuthMethod::PublicKey),
            ssh_proxy_jump: Some("bastion.example.com".to_owned()),
        }
    }

    fn sample_connection() -> SyncConnection {
        SyncConnection {
            name: "nginx-1".to_owned(),
            group_path: "Production Servers/Web".to_owned(),
            host: "10.0.1.10".to_owned(),
            port: 22,
            protocol: ProtocolType::Ssh,
            username: Some("deploy".to_owned()),
            description: Some("Primary web server".to_owned()),
            tags: vec!["web".to_owned(), "nginx".to_owned()],
            protocol_config: ProtocolConfig::Ssh(SshConfig::default()),
            password_source: PasswordSource::Variable("web_deploy_key".to_owned()),
            automation: AutomationConfig::default(),
            custom_properties: Vec::new(),
            pre_connect_task: None,
            post_disconnect_task: None,
            wol_config: None,
            icon: None,
            highlight_rules: Vec::new(),
            updated_at: Utc::now(),
        }
    }

    fn sample_export() -> GroupSyncExport {
        GroupSyncExport::from_group_tree(
            "0.12.0".to_owned(),
            Uuid::new_v4(),
            "admin-laptop".to_owned(),
            sample_root_group(),
            vec![SyncGroup {
                name: "Web".to_owned(),
                path: "Production Servers/Web".to_owned(),
                description: None,
                icon: None,
                username: Some("deploy".to_owned()),
                domain: None,
                ssh_auth_method: None,
                ssh_proxy_jump: None,
            }],
            vec![sample_connection()],
            vec![VariableTemplate {
                name: "web_deploy_key".to_owned(),
                description: Some("SSH key passphrase for web deployment".to_owned()),
                is_secret: true,
                default_value: None,
            }],
        )
    }

    #[test]
    fn serialization_round_trip() {
        let export = sample_export();
        let json = serde_json::to_string_pretty(&export).unwrap();
        let deserialized: GroupSyncExport = serde_json::from_str(&json).unwrap();
        assert_eq!(export, deserialized);
    }

    #[test]
    fn file_round_trip() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.rcn");

        let export = sample_export();
        export.to_file(&path).unwrap();

        let loaded = GroupSyncExport::from_file(&path).unwrap();
        assert_eq!(export, loaded);
    }

    #[test]
    fn from_file_rejects_unsupported_version() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("bad_version.rcn");

        let mut export = sample_export();
        export.sync_version = 99;
        let json = serde_json::to_string(&export).unwrap();
        std::fs::write(&path, json).unwrap();

        let err = GroupSyncExport::from_file(&path).unwrap_err();
        assert!(
            matches!(
                err,
                SyncError::UnsupportedVersion {
                    version: 99,
                    expected: 1
                }
            ),
            "expected UnsupportedVersion, got: {err}"
        );
    }

    #[test]
    fn from_file_rejects_wrong_sync_type() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("bad_type.rcn");

        let mut export = sample_export();
        export.sync_type = "full".to_owned();
        let json = serde_json::to_string(&export).unwrap();
        std::fs::write(&path, json).unwrap();

        let err = GroupSyncExport::from_file(&path).unwrap_err();
        assert!(
            matches!(err, SyncError::UnexpectedSyncType { .. }),
            "expected UnexpectedSyncType, got: {err}"
        );
    }

    #[test]
    fn from_file_rejects_corrupt_json() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("corrupt.rcn");
        std::fs::write(&path, "not valid json {{{").unwrap();

        let err = GroupSyncExport::from_file(&path).unwrap_err();
        assert!(matches!(err, SyncError::InvalidFormat(_)));
    }

    #[test]
    fn from_group_tree_sets_metadata() {
        let device_id = Uuid::new_v4();
        let export = GroupSyncExport::from_group_tree(
            "0.12.0".to_owned(),
            device_id,
            "my-laptop".to_owned(),
            sample_root_group(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(export.sync_version, 1);
        assert_eq!(export.sync_type, "group");
        assert_eq!(export.app_version, "0.12.0");
        assert_eq!(export.master_device_id, device_id);
        assert_eq!(export.master_device_name, "my-laptop");
    }

    #[test]
    fn sync_group_optional_fields_skipped_when_none() {
        let group = SyncGroup {
            name: "Minimal".to_owned(),
            path: "Root/Minimal".to_owned(),
            description: None,
            icon: None,
            username: None,
            domain: None,
            ssh_auth_method: None,
            ssh_proxy_jump: None,
        };
        let json = serde_json::to_string(&group).unwrap();
        assert!(!json.contains("description"));
        assert!(!json.contains("icon"));
        assert!(!json.contains("username"));
        assert!(!json.contains("domain"));
        assert!(!json.contains("ssh_auth_method"));
        assert!(!json.contains("ssh_proxy_jump"));
    }

    #[test]
    fn sync_connection_optional_fields_skipped_when_empty() {
        let conn = SyncConnection {
            name: "test".to_owned(),
            group_path: "Root".to_owned(),
            host: "localhost".to_owned(),
            port: 22,
            protocol: ProtocolType::Ssh,
            username: None,
            description: None,
            tags: Vec::new(),
            protocol_config: ProtocolConfig::Ssh(SshConfig::default()),
            password_source: PasswordSource::None,
            automation: AutomationConfig::default(),
            custom_properties: Vec::new(),
            pre_connect_task: None,
            post_disconnect_task: None,
            wol_config: None,
            icon: None,
            highlight_rules: Vec::new(),
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&conn).unwrap();
        assert!(!json.contains("\"tags\""));
        assert!(!json.contains("\"custom_properties\""));
        assert!(!json.contains("\"highlight_rules\""));
        assert!(!json.contains("\"wol_config\""));
        assert!(!json.contains("\"icon\""));
    }

    #[test]
    fn atomic_write_leaves_no_temp_file_on_success() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("clean.rcn");
        let temp_path = dir.path().join("clean.rcn.tmp");

        let export = sample_export();
        export.to_file(&path).unwrap();

        assert!(path.exists());
        assert!(!temp_path.exists());
    }

    #[test]
    fn from_connection_copies_synced_fields() {
        let mut conn = Connection::new_ssh("nginx-1".to_owned(), "10.0.1.10".to_owned(), 22);
        conn.username = Some("deploy".to_owned());
        conn.description = Some("Primary web server".to_owned());
        conn.tags = vec!["web".to_owned(), "nginx".to_owned()];
        conn.password_source = PasswordSource::Variable("web_key".to_owned());
        conn.icon = Some("🌐".to_owned());

        let sync_conn = SyncConnection::from_connection(&conn, "Production/Web");

        assert_eq!(sync_conn.name, "nginx-1");
        assert_eq!(sync_conn.group_path, "Production/Web");
        assert_eq!(sync_conn.host, "10.0.1.10");
        assert_eq!(sync_conn.port, 22);
        assert_eq!(sync_conn.protocol, ProtocolType::Ssh);
        assert_eq!(sync_conn.username, Some("deploy".to_owned()));
        assert_eq!(sync_conn.description, Some("Primary web server".to_owned()));
        assert_eq!(sync_conn.tags, vec!["web".to_owned(), "nginx".to_owned()]);
        assert_eq!(
            sync_conn.password_source,
            PasswordSource::Variable("web_key".to_owned())
        );
        assert_eq!(sync_conn.icon, Some("🌐".to_owned()));
        assert_eq!(sync_conn.updated_at, conn.updated_at);
    }

    #[test]
    fn from_connection_excludes_local_only_fields() {
        use crate::models::WindowGeometry;

        let mut conn = Connection::new_ssh("test".to_owned(), "host".to_owned(), 22);
        conn.last_connected = Some(Utc::now());
        conn.sort_order = 42;
        conn.is_pinned = true;
        conn.pin_order = 5;
        conn.window_geometry = Some(WindowGeometry::new(100, 200, 800, 600));
        conn.window_mode = crate::models::WindowMode::External;
        conn.remember_window_position = true;
        conn.skip_port_check = true;

        let sync_conn = SyncConnection::from_connection(&conn, "Root");

        // SyncConnection doesn't have these fields at all — the struct
        // definition itself excludes them. We verify the conversion compiles
        // and produces a valid SyncConnection without panicking.
        assert_eq!(sync_conn.name, "test");
        assert_eq!(sync_conn.host, "host");
    }

    #[test]
    fn from_connection_strips_ssh_key_path() {
        use std::path::PathBuf;

        let ssh_config = SshConfig {
            key_path: Some(PathBuf::from("/home/user/.ssh/id_ed25519")),
            proxy_jump: Some("bastion.example.com".to_owned()),
            ..Default::default()
        };

        let conn = Connection::new(
            "test".to_owned(),
            "host".to_owned(),
            22,
            ProtocolConfig::Ssh(ssh_config),
        );

        let sync_conn = SyncConnection::from_connection(&conn, "Root");

        // key_path must be stripped (local-only)
        match &sync_conn.protocol_config {
            ProtocolConfig::Ssh(ssh) => {
                assert_eq!(ssh.key_path, None, "ssh key_path should be stripped");
                // proxy_jump is synced, should be preserved
                assert_eq!(ssh.proxy_jump, Some("bastion.example.com".to_owned()));
            }
            other => panic!("expected Ssh config, got: {other:?}"),
        }
    }

    #[test]
    fn from_connection_strips_sftp_key_path() {
        use std::path::PathBuf;

        let ssh_config = SshConfig {
            key_path: Some(PathBuf::from("/home/user/.ssh/sftp_key")),
            ..Default::default()
        };

        let conn = Connection::new(
            "sftp-test".to_owned(),
            "host".to_owned(),
            22,
            ProtocolConfig::Sftp(ssh_config),
        );

        let sync_conn = SyncConnection::from_connection(&conn, "Root");

        match &sync_conn.protocol_config {
            ProtocolConfig::Sftp(ssh) => {
                assert_eq!(ssh.key_path, None, "sftp key_path should be stripped");
            }
            other => panic!("expected Sftp config, got: {other:?}"),
        }
    }

    #[test]
    fn from_connection_preserves_non_ssh_protocol_config() {
        let conn = Connection::new_rdp("rdp-test".to_owned(), "host".to_owned(), 3389);

        let sync_conn = SyncConnection::from_connection(&conn, "Root");

        assert_eq!(sync_conn.protocol, ProtocolType::Rdp);
        assert!(matches!(sync_conn.protocol_config, ProtocolConfig::Rdp(_)));
    }

    // --- SyncGroup::from_group and compute_group_path tests ---

    fn make_group(name: &str, parent_id: Option<Uuid>) -> ConnectionGroup {
        let mut g = ConnectionGroup::new(name.to_owned());
        g.parent_id = parent_id;
        g
    }

    #[test]
    fn from_group_copies_synced_fields() {
        let mut group = ConnectionGroup::new("Production".to_owned());
        group.description = Some("Prod infra".to_owned());
        group.icon = Some("🏭".to_owned());
        group.username = Some("deploy".to_owned());
        group.domain = Some("example.com".to_owned());
        group.ssh_auth_method = Some(SshAuthMethod::PublicKey);
        group.ssh_proxy_jump = Some("bastion.example.com".to_owned());

        let sync_group = SyncGroup::from_group(&group, "Production");

        assert_eq!(sync_group.name, "Production");
        assert_eq!(sync_group.path, "Production");
        assert_eq!(sync_group.description, Some("Prod infra".to_owned()));
        assert_eq!(sync_group.icon, Some("🏭".to_owned()));
        assert_eq!(sync_group.username, Some("deploy".to_owned()));
        assert_eq!(sync_group.domain, Some("example.com".to_owned()));
        assert_eq!(sync_group.ssh_auth_method, Some(SshAuthMethod::PublicKey));
        assert_eq!(
            sync_group.ssh_proxy_jump,
            Some("bastion.example.com".to_owned())
        );
    }

    #[test]
    fn from_group_excludes_local_only_fields() {
        use std::path::PathBuf;

        let mut group = ConnectionGroup::new("Test".to_owned());
        // Set local-only fields that should NOT appear in SyncGroup
        group.ssh_key_path = Some(PathBuf::from("/home/user/.ssh/key"));
        group.ssh_agent_socket = Some("/tmp/agent.sock".to_owned());
        group.expanded = true;
        group.sort_order = 42;
        group.sync_mode = crate::sync::SyncMode::Master;
        group.sync_file = Some("test.rcn".to_owned());
        group.last_synced_at = Some(Utc::now());

        let sync_group = SyncGroup::from_group(&group, "Test");

        // SyncGroup struct doesn't have these fields at all — the conversion
        // compiles and produces a valid SyncGroup without local-only data.
        assert_eq!(sync_group.name, "Test");
        assert_eq!(sync_group.path, "Test");
    }

    #[test]
    fn compute_group_path_root_group() {
        let root = make_group("Production", None);
        let groups = vec![root.clone()];

        let path = compute_group_path(root.id, &groups);
        assert_eq!(path, "Production");
    }

    #[test]
    fn compute_group_path_child() {
        let root = make_group("Production", None);
        let child = make_group("Web", Some(root.id));
        let groups = vec![root, child.clone()];

        let path = compute_group_path(child.id, &groups);
        assert_eq!(path, "Production/Web");
    }

    #[test]
    fn compute_group_path_grandchild() {
        let root = make_group("Production", None);
        let child = make_group("Web", Some(root.id));
        let grandchild = make_group("Backend", Some(child.id));
        let groups = vec![root, child, grandchild.clone()];

        let path = compute_group_path(grandchild.id, &groups);
        assert_eq!(path, "Production/Web/Backend");
    }

    #[test]
    fn compute_group_path_handles_cycle() {
        let mut group_a = make_group("A", None);
        let group_b = make_group("B", Some(group_a.id));
        // Create cycle: A → B → A
        group_a.parent_id = Some(group_b.id);
        let groups = vec![group_a.clone(), group_b];

        // Should terminate without panic
        let path = compute_group_path(group_a.id, &groups);
        // The path will contain the groups visited before the cycle is detected.
        // Starting from A: A → parent B → parent A (cycle!) → stop.
        // Segments collected: ["A", "B"], reversed: ["B", "A"].
        assert_eq!(path, "B/A");
    }

    #[test]
    fn compute_group_path_orphaned_parent() {
        let orphan_parent_id = Uuid::new_v4();
        let child = make_group("Orphan", Some(orphan_parent_id));
        let groups = vec![child.clone()];

        // Parent not in groups slice — should return just the child name.
        let path = compute_group_path(child.id, &groups);
        assert_eq!(path, "Orphan");
    }

    #[test]
    fn compute_group_path_unknown_group_id() {
        let root = make_group("Root", None);
        let groups = vec![root];

        // Unknown ID — returns empty string.
        let path = compute_group_path(Uuid::new_v4(), &groups);
        assert_eq!(path, "");
    }

    // --- collect_variable_templates tests ---

    #[test]
    fn collect_templates_from_password_source_variable() {
        let mut conn = Connection::new_ssh("test".to_owned(), "host".to_owned(), 22);
        conn.password_source = PasswordSource::Variable("web_key".to_owned());

        let vars =
            vec![Variable::new_secret("web_key", "secret123").with_description("Web deploy key")];

        let templates = collect_variable_templates(&[conn], &vars);
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].name, "web_key");
        assert_eq!(templates[0].description, Some("Web deploy key".to_owned()));
        assert!(templates[0].is_secret);
        assert_eq!(templates[0].default_value, None);
    }

    #[test]
    fn collect_templates_deduplicates_same_variable() {
        let mut conn1 = Connection::new_ssh("a".to_owned(), "host1".to_owned(), 22);
        conn1.password_source = PasswordSource::Variable("shared_key".to_owned());

        let mut conn2 = Connection::new_ssh("b".to_owned(), "host2".to_owned(), 22);
        conn2.password_source = PasswordSource::Variable("shared_key".to_owned());

        let vars = vec![Variable::new_secret("shared_key", "val")];

        let templates = collect_variable_templates(&[conn1, conn2], &vars);
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].name, "shared_key");
    }

    #[test]
    fn collect_templates_secret_vs_non_secret() {
        let mut conn_secret = Connection::new_ssh("s".to_owned(), "host".to_owned(), 22);
        conn_secret.password_source = PasswordSource::Variable("secret_var".to_owned());

        let mut conn_plain = Connection::new_ssh("p".to_owned(), "host".to_owned(), 22);
        conn_plain.password_source = PasswordSource::Variable("plain_var".to_owned());

        let vars = vec![
            Variable::new_secret("secret_var", "hidden"),
            Variable::new("plain_var", "visible").with_description("A plain variable"),
        ];

        let templates = collect_variable_templates(&[conn_secret, conn_plain], &vars);
        assert_eq!(templates.len(), 2);

        let secret_t = templates.iter().find(|t| t.name == "secret_var").unwrap();
        assert!(secret_t.is_secret);
        assert_eq!(secret_t.default_value, None);

        let plain_t = templates.iter().find(|t| t.name == "plain_var").unwrap();
        assert!(!plain_t.is_secret);
        assert_eq!(plain_t.default_value, Some("visible".to_owned()));
    }

    #[test]
    fn collect_templates_empty_connections() {
        let templates = collect_variable_templates(&[], &[]);
        assert!(templates.is_empty());
    }

    #[test]
    fn collect_templates_from_host_variable_reference() {
        let conn = Connection::new_ssh("test".to_owned(), "${db_host}".to_owned(), 5432);

        let vars =
            vec![Variable::new("db_host", "db.example.com").with_description("Database host")];

        let templates = collect_variable_templates(&[conn], &vars);
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].name, "db_host");
        assert!(!templates[0].is_secret);
        assert_eq!(
            templates[0].default_value,
            Some("db.example.com".to_owned())
        );
    }

    #[test]
    fn collect_templates_from_username_variable_reference() {
        let mut conn = Connection::new_ssh("test".to_owned(), "host".to_owned(), 22);
        conn.username = Some("${deploy_user}".to_owned());

        let vars = vec![Variable::new("deploy_user", "admin")];

        let templates = collect_variable_templates(&[conn], &vars);
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].name, "deploy_user");
    }

    #[test]
    fn collect_templates_unknown_variable_defaults_to_secret() {
        let mut conn = Connection::new_ssh("test".to_owned(), "host".to_owned(), 22);
        conn.password_source = PasswordSource::Variable("unknown_var".to_owned());

        let templates = collect_variable_templates(&[conn], &[]);
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].name, "unknown_var");
        assert!(templates[0].is_secret);
        assert_eq!(templates[0].default_value, None);
        assert_eq!(templates[0].description, None);
    }

    #[test]
    fn collect_templates_deduplicates_across_sources() {
        // Same variable referenced both via PasswordSource and ${} in host
        let mut conn = Connection::new_ssh("test".to_owned(), "${my_var}".to_owned(), 22);
        conn.password_source = PasswordSource::Variable("my_var".to_owned());

        let vars = vec![Variable::new_secret("my_var", "val")];

        let templates = collect_variable_templates(&[conn], &vars);
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].name, "my_var");
    }

    #[test]
    fn collect_templates_non_secret_empty_value_no_default() {
        let mut conn = Connection::new_ssh("test".to_owned(), "host".to_owned(), 22);
        conn.password_source = PasswordSource::Variable("empty_var".to_owned());

        let vars = vec![Variable::new("empty_var", "")];

        let templates = collect_variable_templates(&[conn], &vars);
        assert_eq!(templates.len(), 1);
        assert!(!templates[0].is_secret);
        // Empty value → no default_value
        assert_eq!(templates[0].default_value, None);
    }

    // --- group_name_to_filename (slug generation) tests ---

    #[test]
    fn slug_basic_ascii() {
        assert_eq!(
            group_name_to_filename("Production Servers"),
            "production-servers.rcn"
        );
    }

    #[test]
    fn slug_unicode_cyrillic() {
        let result = group_name_to_filename("Сервери");
        assert!(
            std::path::Path::new(&result)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rcn"))
        );
        // Cyrillic should be transliterated to ASCII
        assert!(result.is_ascii());
        assert!(!result.is_empty());
    }

    #[test]
    fn slug_unicode_emoji_transliterated() {
        // The slug crate transliterates emoji (🏭 → "factory")
        assert_eq!(group_name_to_filename("Servers 🏭"), "servers-factory.rcn");
    }

    #[test]
    fn slug_special_characters() {
        assert_eq!(group_name_to_filename("My@Group#1!"), "my-group-1.rcn");
    }

    #[test]
    fn slug_consecutive_hyphens_collapsed() {
        assert_eq!(group_name_to_filename("a---b"), "a-b.rcn");
    }

    #[test]
    fn slug_leading_trailing_hyphens_stripped() {
        assert_eq!(group_name_to_filename("---test---"), "test.rcn");
    }

    #[test]
    fn slug_all_special_chars_fallback() {
        assert_eq!(group_name_to_filename("!!!"), "group.rcn");
    }

    #[test]
    fn slug_empty_string_fallback() {
        assert_eq!(group_name_to_filename(""), "group.rcn");
    }

    #[test]
    fn slug_determinism() {
        let input = "My Test Group 🚀";
        let first = group_name_to_filename(input);
        let second = group_name_to_filename(input);
        assert_eq!(first, second);
    }

    #[test]
    fn slug_mixed_case() {
        assert_eq!(group_name_to_filename("MyGroup"), "mygroup.rcn");
    }

    #[test]
    fn slug_numbers() {
        assert_eq!(group_name_to_filename("Group 42"), "group-42.rcn");
    }
}
