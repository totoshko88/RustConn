//! Dynamic inventory synchronization engine.
//!
//! Synchronizes connections from external inventory sources (scripts, APIs,
//! CMDBs) into RustConn. Connections are matched by a source tag and
//! name+host, supporting add/update/remove operations.

use std::collections::HashMap;
use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ConfigError;
use crate::models::{
    Connection, ConnectionGroup, KubernetesConfig, ProtocolConfig, RdpConfig, SerialConfig,
    SpiceConfig, SshConfig, TelnetConfig, VncConfig,
};

/// Tag prefix for sync-managed connections (e.g. `sync:netbox`).
pub const SYNC_TAG_PREFIX: &str = "sync:";

/// A simplified connection entry from an external inventory source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryEntry {
    /// Connection name (required)
    pub name: String,
    /// Host address or IP (required)
    pub host: String,
    /// Protocol: ssh, rdp, vnc, spice, telnet, serial, kubernetes
    #[serde(default = "default_protocol")]
    pub protocol: String,
    /// Port number (defaults to protocol default)
    #[serde(default)]
    pub port: Option<u16>,
    /// Username for authentication
    #[serde(default)]
    pub username: Option<String>,
    /// Group name (created if it doesn't exist)
    #[serde(default)]
    pub group: Option<String>,
    /// Additional tags
    #[serde(default)]
    pub tags: Vec<String>,
    /// Optional description
    #[serde(default)]
    pub description: Option<String>,
    /// Custom icon (emoji or GTK icon name)
    #[serde(default)]
    pub icon: Option<String>,
}

fn default_protocol() -> String {
    "ssh".to_string()
}

/// The top-level inventory document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    /// List of connection entries
    pub connections: Vec<InventoryEntry>,
}

/// Result of a sync operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncResult {
    /// Number of new connections added
    pub added: usize,
    /// Number of existing connections updated
    pub updated: usize,
    /// Number of stale connections removed
    pub removed: usize,
    /// Number of entries skipped due to errors
    pub skipped: usize,
    /// Descriptions of skipped entries
    pub skip_reasons: Vec<String>,
}

/// Parses an inventory from a JSON string.
///
/// # Errors
///
/// Returns `ConfigError::Parse` if the JSON is invalid.
pub fn parse_inventory_json(json: &str) -> Result<Inventory, ConfigError> {
    serde_json::from_str(json).map_err(|e| ConfigError::Parse(e.to_string()))
}

/// Parses an inventory from a YAML string.
///
/// # Errors
///
/// Returns `ConfigError::Parse` if the YAML is invalid.
pub fn parse_inventory_yaml(yaml: &str) -> Result<Inventory, ConfigError> {
    serde_yaml::from_str(yaml).map_err(|e: serde_yaml::Error| ConfigError::Parse(e.to_string()))
}

/// Loads an inventory from a file, detecting format by extension.
///
/// Supported extensions: `.json`, `.yaml`, `.yml`.
///
/// # Errors
///
/// Returns `ConfigError::NotFound` if the file doesn't exist, or
/// `ConfigError::Parse` if the content is invalid.
pub fn load_inventory(path: &Path) -> Result<Inventory, ConfigError> {
    if !path.exists() {
        return Err(ConfigError::NotFound(path.to_path_buf()));
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| ConfigError::Parse(format!("Failed to read {}: {e}", path.display())))?;

    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml") => {
            parse_inventory_yaml(&content)
        }
        _ => parse_inventory_json(&content),
    }
}

/// Resolves a protocol string to a default port number.
#[must_use]
pub fn default_port_for_protocol(protocol: &str) -> u16 {
    match protocol.to_lowercase().as_str() {
        "ssh" | "sftp" => 22,
        "rdp" => 3389,
        "vnc" => 5900,
        "spice" => 5900,
        "telnet" => 23,
        _ => 22,
    }
}

/// Builds a [`ProtocolConfig`] from a protocol name string.
fn protocol_config_from_str(protocol: &str) -> Option<ProtocolConfig> {
    match protocol.to_lowercase().as_str() {
        "ssh" => Some(ProtocolConfig::Ssh(SshConfig::default())),
        "rdp" => Some(ProtocolConfig::Rdp(RdpConfig::default())),
        "vnc" => Some(ProtocolConfig::Vnc(VncConfig::default())),
        "spice" => Some(ProtocolConfig::Spice(SpiceConfig::default())),
        "telnet" => Some(ProtocolConfig::Telnet(TelnetConfig::default())),
        "serial" => Some(ProtocolConfig::Serial(SerialConfig::default())),
        "kubernetes" | "k8s" => Some(ProtocolConfig::Kubernetes(KubernetesConfig::default())),
        _ => None,
    }
}

/// Builds the sync tag for a given source name.
#[must_use]
pub fn sync_tag(source: &str) -> String {
    format!("{SYNC_TAG_PREFIX}{source}")
}

/// Synchronizes an inventory against existing connections and groups.
///
/// Connections are matched by the sync source tag (`sync:<source>`) and
/// the combination of `name` + `host`. New entries are added, changed
/// entries are updated, and entries absent from the inventory are
/// optionally removed (`remove_stale`).
///
/// # Arguments
///
/// * `inventory` — parsed inventory from an external source
/// * `source` — source identifier (e.g. `"netbox"`, `"ansible"`)
/// * `existing_connections` — current connections (mutated in place)
/// * `existing_groups` — current groups (mutated in place)
/// * `remove_stale` — if `true`, remove connections tagged with this
///   source that are no longer in the inventory
///
/// # Returns
///
/// A [`SyncResult`] summarizing what changed.
#[expect(
    clippy::too_many_lines,
    reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
)]
pub fn sync_inventory(
    inventory: &Inventory,
    source: &str,
    existing_connections: &mut Vec<Connection>,
    existing_groups: &mut Vec<ConnectionGroup>,
    remove_stale: bool,
) -> SyncResult {
    let tag = sync_tag(source);
    let mut result = SyncResult {
        added: 0,
        updated: 0,
        removed: 0,
        skipped: 0,
        skip_reasons: Vec::new(),
    };

    // Build group name → ID map for quick lookup
    let mut group_map: HashMap<String, Uuid> = existing_groups
        .iter()
        .map(|g| (g.name.clone(), g.id))
        .collect();

    // Track which sync-tagged connections are still present
    let mut seen_ids: Vec<Uuid> = Vec::new();

    for entry in &inventory.connections {
        // Validate entry
        if entry.name.trim().is_empty() || entry.host.trim().is_empty() {
            result.skipped += 1;
            result.skip_reasons.push(format!(
                "Skipped entry with empty name or host: name={:?}, host={:?}",
                entry.name, entry.host
            ));
            continue;
        }

        let Some(proto_config) = protocol_config_from_str(&entry.protocol) else {
            result.skipped += 1;
            result.skip_reasons.push(format!(
                "Skipped '{}': unknown protocol '{}'",
                entry.name, entry.protocol
            ));
            continue;
        };

        let port = entry
            .port
            .unwrap_or_else(|| default_port_for_protocol(&entry.protocol));

        // Resolve group
        let group_id = entry.group.as_ref().map(|group_name| {
            *group_map.entry(group_name.clone()).or_insert_with(|| {
                let group = ConnectionGroup::new(group_name.clone());
                let id = group.id;
                existing_groups.push(group);
                id
            })
        });

        // Build full tag list: sync tag + user tags
        let mut tags = vec![tag.clone()];
        tags.extend(entry.tags.iter().cloned());

        // Find existing connection by sync tag + name + host
        let existing = existing_connections
            .iter_mut()
            .find(|c| c.tags.contains(&tag) && c.name == entry.name && c.host == entry.host);

        if let Some(conn) = existing {
            // Update existing connection fields that changed
            let port_changed = conn.port != port;
            if port_changed {
                conn.port = port;
            }
            let proto_changed = conn.protocol != proto_config.protocol_type();
            if proto_changed {
                conn.protocol = proto_config.protocol_type();
                conn.protocol_config = proto_config;
            }
            let user_changed = conn.username != entry.username;
            if user_changed {
                conn.username = entry.username.clone();
            }
            let group_changed = conn.group_id != group_id;
            if group_changed {
                conn.group_id = group_id;
            }
            let desc_changed = conn.description != entry.description;
            if desc_changed {
                conn.description = entry.description.clone();
            }
            let icon_changed = conn.icon != entry.icon;
            if icon_changed {
                conn.icon = entry.icon.clone();
            }
            let tags_changed = conn.tags != tags;
            if tags_changed {
                conn.tags = tags;
            }
            if port_changed
                || proto_changed
                || user_changed
                || group_changed
                || desc_changed
                || icon_changed
                || tags_changed
            {
                conn.updated_at = Utc::now();
                result.updated += 1;
            }
            seen_ids.push(conn.id);
        } else {
            // Create new connection
            let mut conn =
                Connection::new(entry.name.clone(), entry.host.clone(), port, proto_config);
            conn.username = entry.username.clone();
            conn.group_id = group_id;
            conn.tags = tags;
            conn.description = entry.description.clone();
            conn.icon = entry.icon.clone();
            seen_ids.push(conn.id);
            existing_connections.push(conn);
            result.added += 1;
        }
    }

    // Remove stale connections (tagged with this source but not in inventory)
    if remove_stale {
        let before = existing_connections.len();
        existing_connections.retain(|c| !c.tags.contains(&tag) || seen_ids.contains(&c.id));
        result.removed = before - existing_connections.len();
    }

    result
}
