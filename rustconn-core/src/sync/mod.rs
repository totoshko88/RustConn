//! Synchronization engine for RustConn.
//!
//! This module provides two synchronization systems:
//!
//! ## Inventory Sync
//!
//! Dynamic inventory synchronization from external sources (scripts, APIs,
//! CMDBs). Connections are matched by a source tag and name+host, supporting
//! add/update/remove operations. See [`inventory`].
//!
//! ## Cloud Sync (planned)
//!
//! Cloud-based configuration synchronization between devices and team members
//! through a shared directory (Google Drive, Syncthing, Nextcloud, Dropbox, etc.).
//!
//! Two modes are supported:
//!
//! - **Group Sync** — per-group `.rcn` files with Master/Import access model
//!   and name-based merge for team collaboration.
//! - **Simple Sync** — single-file bidirectional sync with UUID-based merge
//!   and tombstones for personal multi-device use.

// --- Inventory sync (existing) ---
pub mod inventory;

// --- Cloud Sync submodules (added by subsequent tasks) ---
pub mod settings;

pub mod group_export;
pub mod variable_template;

pub mod credential_check;
pub mod full_export;
pub mod full_merge;
pub mod group_merge;
pub mod manager;
pub mod tombstone;
pub mod watcher;

// Re-export inventory types for backward compatibility.
// Re-export Cloud Sync types.
pub use credential_check::CredentialResolutionResult;
// Re-export Simple Sync types.
pub use full_export::FullSyncExport;
pub use full_merge::{FullMergeEngine, FullMergeResult, LocalState, MergeAction};
pub use group_export::{
    GroupSyncExport, SyncConnection, SyncError, SyncGroup, collect_variable_templates,
    compute_group_path, group_name_to_filename, validate_sync_filename,
};
pub use group_merge::{GroupMergeEngine, GroupMergeResult};
pub use inventory::{
    Inventory, InventoryEntry, SYNC_TAG_PREFIX, SyncResult, default_port_for_protocol,
    load_inventory, parse_inventory_json, parse_inventory_yaml, sync_inventory, sync_tag,
};
pub use manager::{GroupSyncState, SyncManager, SyncReport};
pub use settings::{SyncMode, SyncSettings};
pub use tombstone::{SyncEntityType, Tombstone, cleanup_tombstones};
pub use variable_template::VariableTemplate;
pub use watcher::SyncFileWatcher;
