//! Full Sync export format for Simple Sync (bidirectional).
//!
//! [`FullSyncExport`] is the on-disk file format for Simple Sync. It contains
//! all connections, groups, templates, snippets, clusters, non-secret variables,
//! and tombstones. Secret variable values are filtered out before export.
//!
//! File I/O uses atomic writes (temp file + rename) so readers never see
//! partial or corrupt JSON.

use std::io::{BufReader, BufWriter, Write};
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::group_export::SyncError;
use super::tombstone::Tombstone;
use crate::cluster::Cluster;
use crate::models::{Connection, ConnectionGroup, ConnectionTemplate, Snippet};
use crate::variables::Variable;

/// Current sync format version.
const SYNC_VERSION: u32 = 1;

/// Expected `sync_type` value for full exports.
const SYNC_TYPE_FULL: &str = "full";

/// A complete Simple Sync export — the on-disk `full-sync.rcn` file format.
///
/// Contains all application data (minus secret variable values and local-only
/// fields) plus tombstones for deletion tracking across devices.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullSyncExport {
    /// Format version (currently 1).
    pub sync_version: u32,

    /// Always `"full"` for Simple Sync files.
    pub sync_type: String,

    /// Timestamp when this export was created.
    pub exported_at: DateTime<Utc>,

    /// Application version that produced this export.
    pub app_version: String,

    /// Device ID of the exporting device (used to prevent self-sync).
    pub device_id: Uuid,

    /// Human-readable name of the exporting device.
    pub device_name: String,

    /// All connections (local-only fields included — filtered at merge time).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<Connection>,

    /// All connection groups.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<ConnectionGroup>,

    /// All connection templates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub templates: Vec<ConnectionTemplate>,

    /// All snippets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub snippets: Vec<Snippet>,

    /// All clusters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clusters: Vec<Cluster>,

    /// Non-secret variables only. Secret variable values are stripped.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<Variable>,

    /// Tombstones for deletion tracking across devices.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tombstones: Vec<Tombstone>,
}

impl FullSyncExport {
    /// Builds a `FullSyncExport` from application data.
    ///
    /// Filters out secret variable values — only non-secret variables are
    /// included. Secret variables have their value cleared to prevent leakage.
    ///
    /// Local-only connection fields (`last_connected`, `sort_order`,
    /// `is_pinned`, `pin_order`, `window_geometry`, `window_mode`,
    /// `remember_window_position`, `skip_port_check`) are cleared to
    /// defaults to avoid leaking device-specific state.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
    pub fn build(
        app_version: String,
        device_id: Uuid,
        device_name: String,
        connections: Vec<Connection>,
        groups: Vec<ConnectionGroup>,
        templates: Vec<ConnectionTemplate>,
        snippets: Vec<Snippet>,
        clusters: Vec<Cluster>,
        variables: &[Variable],
        tombstones: Vec<Tombstone>,
    ) -> Self {
        let filtered_variables = filter_secret_variables(variables);
        let cleaned_connections = strip_local_only_connection_fields(connections);
        let cleaned_groups = strip_local_only_group_fields(groups);

        Self {
            sync_version: SYNC_VERSION,
            sync_type: SYNC_TYPE_FULL.to_owned(),
            exported_at: Utc::now(),
            app_version,
            device_id,
            device_name,
            connections: cleaned_connections,
            groups: cleaned_groups,
            templates,
            snippets,
            clusters,
            variables: filtered_variables,
            tombstones,
        }
    }

    /// Writes this export to a JSON file using atomic write (temp file + rename).
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

        std::fs::rename(&temp_path, path)?;

        // Restrict file permissions to owner-only (0600) — sync files may
        // contain hostnames, usernames, and variable references.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }

        Ok(())
    }

    /// Reads and parses a `FullSyncExport` from a JSON file.
    ///
    /// Validates `sync_version` and `sync_type` after parsing.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] on read/parse errors or version/type mismatch.
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

        if export.sync_type != SYNC_TYPE_FULL {
            return Err(SyncError::UnexpectedSyncType {
                found: export.sync_type,
                expected: SYNC_TYPE_FULL.to_owned(),
            });
        }

        Ok(export)
    }
}

/// Filters variables for export: only non-secret variables are included.
fn filter_secret_variables(variables: &[Variable]) -> Vec<Variable> {
    variables.iter().filter(|v| !v.is_secret).cloned().collect()
}

/// Strips local-only fields from connections before export.
///
/// Clears `last_connected`, `sort_order`, `is_pinned`, `pin_order`,
/// `window_geometry`, `window_mode`, `remember_window_position`, and
/// `skip_port_check` to their default values. These fields are
/// device-specific and should not be synced between devices.
///
/// Web connections: `zoom_level` is reset to 1.0 (device-dependent screen size).
fn strip_local_only_connection_fields(connections: Vec<Connection>) -> Vec<Connection> {
    connections
        .into_iter()
        .map(|mut c| {
            c.last_connected = None;
            c.sort_order = 0;
            c.is_pinned = false;
            c.pin_order = 0;
            c.window_geometry = None;
            c.window_mode = crate::models::WindowMode::default();
            c.remember_window_position = false;
            c.skip_port_check = false;
            // Web: zoom level is device-local (a 13" laptop and a 32" monitor
            // need different zoom). Reset to default so per-device preferences
            // do not propagate between machines.
            if let crate::models::ProtocolConfig::Web(ref mut web) = c.protocol_config {
                web.zoom_level = 1.0;
            }
            c
        })
        .collect()
}

/// Strips local-only and sync-bookkeeping fields from groups before export.
///
/// Clears UI/order state (`expanded`, `sort_order`), device-specific SSH
/// fields (`ssh_key_path`, `ssh_jump_host_id`, `ssh_agent_socket`), and
/// Group-Sync bookkeeping (`sync_mode`, `sync_file`, `last_synced_at`) to their
/// defaults. The importing device preserves its own copies of these fields, so
/// they must never travel in the shared file. `updated_at` is left intact — it
/// is the merge clock.
fn strip_local_only_group_fields(groups: Vec<ConnectionGroup>) -> Vec<ConnectionGroup> {
    groups
        .into_iter()
        .map(|mut g| {
            g.expanded = true;
            g.sort_order = 0;
            g.ssh_key_path = None;
            g.ssh_jump_host_id = None;
            g.ssh_agent_socket = None;
            g.sync_mode = crate::sync::SyncMode::default();
            g.sync_file = None;
            g.last_synced_at = None;
            g
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_export() -> FullSyncExport {
        FullSyncExport::build(
            "0.12.0".to_owned(),
            Uuid::new_v4(),
            "test-laptop".to_owned(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            &[],
            Vec::new(),
        )
    }

    #[test]
    fn build_sets_metadata() {
        let device_id = Uuid::new_v4();
        let export = FullSyncExport::build(
            "0.12.0".to_owned(),
            device_id,
            "laptop".to_owned(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            &[],
            Vec::new(),
        );
        assert_eq!(export.sync_version, 1);
        assert_eq!(export.sync_type, "full");
        assert_eq!(export.device_id, device_id);
        assert_eq!(export.device_name, "laptop");
    }

    #[test]
    fn build_strips_local_only_group_fields() {
        use crate::sync::SyncMode;

        let mut group = ConnectionGroup::new("prod".to_owned());
        let edited_at = group.updated_at;
        group.ssh_key_path = Some(std::path::PathBuf::from("/home/me/.ssh/id_ed25519"));
        group.ssh_jump_host_id = Some(Uuid::new_v4());
        group.ssh_agent_socket = Some("/run/agent.sock".to_owned());
        group.sync_mode = SyncMode::Master;
        group.sync_file = Some("prod.rcn".to_owned());
        group.sort_order = 7;

        let export = FullSyncExport::build(
            "0.12.0".to_owned(),
            Uuid::new_v4(),
            "laptop".to_owned(),
            Vec::new(),
            vec![group],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            &[],
            Vec::new(),
        );

        let g = &export.groups[0];
        // Device-specific / Group-Sync fields are stripped.
        assert!(g.ssh_key_path.is_none());
        assert!(g.ssh_jump_host_id.is_none());
        assert!(g.ssh_agent_socket.is_none());
        assert_eq!(g.sync_mode, SyncMode::None);
        assert!(g.sync_file.is_none());
        assert_eq!(g.sort_order, 0);
        // Synced content and the merge clock are preserved.
        assert_eq!(g.name, "prod");
        assert_eq!(g.updated_at, edited_at);
    }

    #[test]
    fn build_filters_secret_variables() {
        let variables = vec![
            Variable::new("host", "example.com"),
            Variable::new_secret("password", "s3cret"),
            Variable::new("port", "8080"),
        ];

        let export = FullSyncExport::build(
            "0.12.0".to_owned(),
            Uuid::new_v4(),
            "laptop".to_owned(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            &variables,
            Vec::new(),
        );

        assert_eq!(export.variables.len(), 2);
        assert!(export.variables.iter().all(|v| !v.is_secret));
        assert!(export.variables.iter().any(|v| v.name == "host"));
        assert!(export.variables.iter().any(|v| v.name == "port"));
    }

    #[test]
    fn serialization_round_trip() {
        let export = sample_export();
        let json = serde_json::to_string_pretty(&export).unwrap();
        let deserialized: FullSyncExport = serde_json::from_str(&json).unwrap();
        assert_eq!(export.sync_version, deserialized.sync_version);
        assert_eq!(export.sync_type, deserialized.sync_type);
        assert_eq!(export.device_id, deserialized.device_id);
        assert_eq!(export.device_name, deserialized.device_name);
        assert_eq!(export.app_version, deserialized.app_version);
    }

    #[test]
    fn file_round_trip() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("full-sync.rcn");

        let export = sample_export();
        export.to_file(&path).unwrap();

        let loaded = FullSyncExport::from_file(&path).unwrap();
        assert_eq!(export.sync_version, loaded.sync_version);
        assert_eq!(export.sync_type, loaded.sync_type);
        assert_eq!(export.device_id, loaded.device_id);
    }

    #[test]
    fn from_file_rejects_wrong_version() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("bad.rcn");

        let mut export = sample_export();
        export.sync_version = 99;
        let json = serde_json::to_string(&export).unwrap();
        std::fs::write(&path, json).unwrap();

        let err = FullSyncExport::from_file(&path).unwrap_err();
        assert!(matches!(
            err,
            SyncError::UnsupportedVersion {
                version: 99,
                expected: 1
            }
        ));
    }

    #[test]
    fn from_file_rejects_wrong_type() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("bad.rcn");

        let mut export = sample_export();
        export.sync_type = "group".to_owned();
        let json = serde_json::to_string(&export).unwrap();
        std::fs::write(&path, json).unwrap();

        let err = FullSyncExport::from_file(&path).unwrap_err();
        assert!(matches!(err, SyncError::UnexpectedSyncType { .. }));
    }

    #[test]
    fn no_secret_values_in_serialized_output() {
        let variables = vec![
            Variable::new("host", "example.com"),
            Variable::new_secret("api_key", "super-secret-key-12345"),
        ];

        let export = FullSyncExport::build(
            "0.12.0".to_owned(),
            Uuid::new_v4(),
            "laptop".to_owned(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            &variables,
            Vec::new(),
        );

        let json = serde_json::to_string(&export).unwrap();
        assert!(!json.contains("super-secret-key-12345"));
        assert!(json.contains("example.com"));
    }
}
