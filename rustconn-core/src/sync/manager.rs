//! Sync manager — coordinates all sync operations.
//!
//! [`SyncManager`] is the central coordinator for Cloud Sync. It handles
//! export scheduling, import triggering, file listing, and per-group sync
//! state tracking.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use tokio::sync::mpsc;
use uuid::Uuid;

use super::group_export::{
    GroupSyncExport, SyncConnection, SyncError, SyncGroup, collect_variable_templates,
    compute_group_path, group_name_to_filename, validate_sync_filename,
};
use super::group_merge::{GroupMergeEngine, GroupMergeResult};
use super::settings::{SyncMode, SyncSettings};
use crate::models::{Connection, ConnectionGroup};
use crate::variables::Variable;

/// Per-group sync state tracking.
#[derive(Debug, Clone, Default)]
pub struct GroupSyncState {
    /// Timestamp of the last successful sync operation for this group.
    pub last_synced_at: Option<DateTime<Utc>>,
    /// Last error message if the most recent sync operation failed.
    pub last_error: Option<String>,
}

/// Summary of a sync operation (export or import).
#[derive(Debug, Clone)]
pub struct SyncReport {
    /// Name of the group that was synced.
    pub group_name: String,
    /// Number of connections added during this sync.
    pub connections_added: usize,
    /// Number of connections updated during this sync.
    pub connections_updated: usize,
    /// Number of connections removed during this sync.
    pub connections_removed: usize,
    /// Number of groups added during this sync.
    pub groups_added: usize,
    /// Number of groups removed during this sync.
    pub groups_removed: usize,
    /// Number of variable templates created during this sync.
    pub variables_created: usize,
    /// Timestamp when this sync operation completed.
    pub timestamp: DateTime<Utc>,
}

impl SyncReport {
    /// Creates a [`SyncReport`] from a [`GroupMergeResult`].
    ///
    /// Counts the items in each category of the merge result to produce
    /// an accurate summary of the import operation.
    #[must_use]
    pub fn from_merge_result(
        group_name: &str,
        result: &GroupMergeResult,
        timestamp: DateTime<Utc>,
    ) -> Self {
        Self {
            group_name: group_name.to_owned(),
            connections_added: result.connections_to_create.len(),
            connections_updated: result.connections_to_update.len(),
            connections_removed: result.connections_to_delete.len(),
            groups_added: result.groups_to_create.len(),
            groups_removed: result.groups_to_delete.len(),
            variables_created: result.variables_to_create.len(),
            timestamp,
        }
    }
}

/// Coordinates all sync operations for Cloud Sync.
///
/// Manages export scheduling, import triggering, sync file discovery,
/// and per-group sync state tracking.
///
/// # File Watcher Integration
///
/// The GUI layer should create a [`SyncFileWatcher`](super::watcher::SyncFileWatcher)
/// when a sync directory is configured, and route its callbacks through
/// `import_group()` for Import-mode groups. Master group files must be
/// registered via [`SyncFileWatcher::add_master_file`](super::watcher::SyncFileWatcher::add_master_file)
/// to prevent circular export→import loops.
pub struct SyncManager {
    /// Global sync settings (sync directory, device identity, timing).
    settings: SyncSettings,
    /// Per-group sync state (last synced timestamp, last error).
    state: HashMap<Uuid, GroupSyncState>,
    /// Channel sender for debounced export scheduling.
    /// Created by [`SyncManager::create_export_channel`]. When `Some`,
    /// [`SyncManager::schedule_export`] sends group IDs through this channel.
    export_tx: Option<mpsc::UnboundedSender<Uuid>>,
    /// Channel receiver for debounced export scheduling.
    /// Created by [`SyncManager::create_export_channel`].
    export_rx: Option<mpsc::UnboundedReceiver<Uuid>>,
}

impl SyncManager {
    /// Creates a new `SyncManager` with the given settings.
    #[must_use]
    pub fn new(settings: SyncSettings) -> Self {
        Self {
            settings,
            state: HashMap::new(),
            export_tx: None,
            export_rx: None,
        }
    }

    /// Returns a reference to the sync settings.
    #[must_use]
    pub fn settings(&self) -> &SyncSettings {
        &self.settings
    }

    /// Replaces the sync settings in place, preserving the export channel and
    /// per-group state. Call this when the user changes sync settings so the
    /// manager picks up a new `sync_dir`, device identity, or retention.
    pub fn set_settings(&mut self, settings: SyncSettings) {
        self.settings = settings;
    }

    /// Returns a reference to the per-group sync state map.
    #[must_use]
    pub fn state(&self) -> &HashMap<Uuid, GroupSyncState> {
        &self.state
    }

    /// Returns a mutable reference to the state for a specific group,
    /// inserting a default entry if none exists.
    pub fn state_mut(&mut self, group_id: Uuid) -> &mut GroupSyncState {
        self.state.entry(group_id).or_default()
    }

    /// Exports a single Master group to its `.rcn` sync file.
    ///
    /// Validates preconditions, collects all subgroups and connections
    /// recursively, converts them to sync format, writes the file atomically,
    /// and updates sync state.
    ///
    /// # Arguments
    ///
    /// * `group_id` — ID of the root group to export
    /// * `groups` — all groups in the application
    /// * `connections` — all connections in the application
    /// * `variables` — all global variables (for template collection)
    /// * `app_version` — current application version string
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] if:
    /// - The sync directory is not configured ([`SyncError::SyncDirNotConfigured`])
    /// - The group is not found ([`SyncError::GroupNotFound`])
    /// - The group is not a root group ([`SyncError::NotRootGroup`])
    /// - The group is not in Master mode ([`SyncError::NotMasterGroup`])
    /// - File I/O fails ([`SyncError::Io`])
    pub fn export_group(
        &mut self,
        group_id: Uuid,
        groups: &[ConnectionGroup],
        connections: &[Connection],
        variables: &[Variable],
        app_version: &str,
    ) -> Result<SyncReport, SyncError> {
        // 1. Validate sync_dir is configured
        let sync_dir = self
            .settings
            .sync_dir
            .as_ref()
            .ok_or(SyncError::SyncDirNotConfigured)?
            .clone();

        // 2. Find the root group and validate preconditions
        let root_group = groups
            .iter()
            .find(|g| g.id == group_id)
            .ok_or(SyncError::GroupNotFound(group_id))?;

        if !root_group.is_root() {
            return Err(SyncError::NotRootGroup(group_id));
        }

        if root_group.sync_mode != SyncMode::Master {
            return Err(SyncError::NotMasterGroup(group_id));
        }

        // Determine sync filename (use existing or generate from name)
        let sync_file = root_group
            .sync_file
            .clone()
            .unwrap_or_else(|| group_name_to_filename(&root_group.name));

        // Validate filename against path traversal
        validate_sync_filename(&sync_file)?;

        // 3. Collect all subgroups recursively
        let subgroups = collect_subgroups(group_id, groups);

        // 4. Collect connections belonging to root group and subgroups
        let group_ids: std::collections::HashSet<Uuid> = std::iter::once(group_id)
            .chain(subgroups.iter().map(|g| g.id))
            .collect();
        let group_connections: Vec<&Connection> = connections
            .iter()
            .filter(|c| c.group_id.is_some_and(|gid| group_ids.contains(&gid)))
            .collect();

        // 5. Convert groups to SyncGroup
        let root_sync_group = SyncGroup::from_group(root_group, &root_group.name);
        let sync_groups: Vec<SyncGroup> = subgroups
            .iter()
            .map(|g| {
                let path = compute_group_path(g.id, groups);
                SyncGroup::from_group(g, &path)
            })
            .collect();

        // 6. Convert connections to SyncConnection
        let sync_connections: Vec<SyncConnection> = group_connections
            .iter()
            .map(|c| {
                let group_path = c
                    .group_id
                    .map(|gid| compute_group_path(gid, groups))
                    .unwrap_or_default();
                SyncConnection::from_connection(c, &group_path)
            })
            .collect();

        // 7. Collect variable templates
        let conn_refs: Vec<Connection> = group_connections.iter().map(|c| (*c).clone()).collect();
        let variable_templates = collect_variable_templates(&conn_refs, variables);

        // 8. Build GroupSyncExport
        let export = GroupSyncExport::from_group_tree(
            app_version.to_owned(),
            self.settings.device_id,
            self.settings.device_name.clone(),
            root_sync_group,
            sync_groups.clone(),
            sync_connections.clone(),
            variable_templates.clone(),
        );

        // 9. Write to file atomically (temp file + rename).
        //
        // Concurrent Master exports (misconfiguration: two devices both set as
        // Master for the same group) are handled via last-write-wins semantics.
        // Because `to_file` writes to a `.rcn.tmp` temp file and then performs
        // an atomic `rename`, readers never see partial JSON. Whichever rename
        // completes last becomes the current version of the file. No file-level
        // locking is used — this is intentional per the design doc (Error
        // Scenario 7). Documentation recommends a single Master per group.
        let file_path = sync_dir.join(&sync_file);
        tracing::debug!(
            %group_id,
            file = %file_path.display(),
            "Writing sync file atomically (last-write-wins for concurrent exports)"
        );
        export.to_file(&file_path)?;

        // 10. Update per-group sync state
        let now = Utc::now();
        let state = self.state.entry(group_id).or_default();
        state.last_synced_at = Some(now);
        state.last_error = None;

        // 11. Build and return SyncReport
        Ok(SyncReport {
            group_name: root_group.name.clone(),
            connections_added: sync_connections.len(),
            connections_updated: 0,
            connections_removed: 0,
            groups_added: sync_groups.len(),
            groups_removed: 0,
            variables_created: variable_templates.len(),
            timestamp: now,
        })
    }

    /// Schedules a debounced export for the given group.
    ///
    /// Sends the `group_id` through the internal channel. The consumer
    /// (spawned via [`SyncManager::create_export_channel`]) coalesces
    /// multiple calls within the debounce window into a single export.
    ///
    /// If the export channel has not been created yet (via
    /// [`SyncManager::create_export_channel`]), the call is a no-op and
    /// a warning is logged.
    pub fn schedule_export(&self, group_id: Uuid) {
        if let Some(ref tx) = self.export_tx {
            if let Err(e) = tx.send(group_id) {
                tracing::warn!(
                    %group_id,
                    "Export scheduler channel closed, cannot schedule export: {e}"
                );
            }
        } else {
            tracing::warn!(
                %group_id,
                "Export channel not created, call create_export_channel() first"
            );
        }
    }

    /// Creates the export scheduling channel.
    ///
    /// Returns the receiving end of an unbounded MPSC channel. The caller
    /// (typically the GUI layer) should spawn a task that reads from this
    /// receiver and applies debounce logic using
    /// [`SyncManager::export_debounce_secs`].
    ///
    /// After this call, [`SyncManager::schedule_export`] will send group
    /// IDs through the channel instead of being a no-op.
    ///
    /// # Panics
    ///
    /// Does not panic. If called multiple times, the previous channel is
    /// replaced and its receiver becomes inert.
    pub fn create_export_channel(&mut self) -> mpsc::UnboundedReceiver<Uuid> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.export_tx = Some(tx);
        rx
    }

    /// Creates the export channel and stores both ends internally.
    ///
    /// Returns the sender that should be passed to
    /// [`ConnectionManager::set_export_sender`](crate::connection::ConnectionManager::set_export_sender).
    /// The receiver is stored internally and polled via [`try_recv_export`](Self::try_recv_export).
    pub fn setup_export_channel(&mut self) -> mpsc::UnboundedSender<Uuid> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.export_tx = Some(tx.clone());
        self.export_rx = Some(rx);
        tx
    }

    /// Tries to receive a pending export group ID from the channel.
    ///
    /// Returns `Some(group_id)` if a group ID is available, or `None` if
    /// the channel is empty, disconnected, or never created. Mirrors
    /// `tokio::sync::mpsc::Receiver::try_recv` semantics — the absence of
    /// a value is normal control flow, not an error condition.
    #[must_use]
    pub fn try_recv_export(&mut self) -> Option<Uuid> {
        self.export_rx.as_mut().and_then(|rx| rx.try_recv().ok())
    }

    /// Returns the configured export debounce interval in seconds.
    ///
    /// This value comes from [`SyncSettings::export_debounce_secs`] and
    /// defaults to 5. The consumer of the export channel should wait this
    /// long after the last received group ID before triggering the actual
    /// export.
    #[must_use]
    pub fn export_debounce_secs(&self) -> u32 {
        self.settings.export_debounce_secs
    }

    /// Lists `.rcn` files available in the sync directory.
    ///
    /// Returns file paths for all `.rcn` files found in `sync_dir`.
    /// Returns an empty `Vec` if `sync_dir` is not configured.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Io`] if the sync directory cannot be read.
    pub fn list_available_sync_files(&self) -> Result<Vec<PathBuf>, SyncError> {
        let Some(ref sync_dir) = self.settings.sync_dir else {
            return Ok(Vec::new());
        };

        let mut files = Vec::new();
        for entry in std::fs::read_dir(sync_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file()
                && let Some(ext) = path.extension()
                && ext == "rcn"
            {
                files.push(path);
            }
        }
        files.sort();
        Ok(files)
    }

    /// Validates that the sync directory exists and is writable.
    ///
    /// Returns the validated path on success.
    ///
    /// # Errors
    ///
    /// - [`SyncError::SyncDirNotConfigured`] if `sync_dir` is `None`.
    /// - [`SyncError::SyncDirNotWritable`] if the directory does not exist
    ///   or is not writable.
    pub fn validate_sync_dir(&self) -> Result<PathBuf, SyncError> {
        let sync_dir = self
            .settings
            .sync_dir
            .as_ref()
            .ok_or(SyncError::SyncDirNotConfigured)?
            .clone();

        // Check that the directory exists and is a directory
        if !sync_dir.is_dir() {
            return Err(SyncError::SyncDirNotWritable(sync_dir));
        }

        // Check writability by attempting to create and remove a temp file
        let probe = sync_dir.join(".rustconn-write-probe");
        match std::fs::File::create(&probe) {
            Ok(_) => {
                // Clean up the probe file
                let _ = std::fs::remove_file(&probe);
                Ok(sync_dir)
            }
            Err(_) => Err(SyncError::SyncDirNotWritable(sync_dir)),
        }
    }

    /// Imports a single Import group from its sync file.
    ///
    /// Reads the `.rcn` file from the sync directory, runs the merge engine
    /// against the local group tree, and returns the merge result along with
    /// a [`SyncReport`] summarising the changes.
    ///
    /// The caller is responsible for applying the [`GroupMergeResult`] to the
    /// local data store (e.g. via `ConnectionManager`).
    ///
    /// # Arguments
    ///
    /// * `group_id` — ID of the Import root group
    /// * `groups` — all local groups
    /// * `connections` — all local connections
    /// * `local_variable_names` — names of variables that already exist locally
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] if the sync directory is not configured, the
    /// group is not found, the group is not in Import mode, the sync file
    /// is missing, or the file cannot be parsed.
    pub fn import_group(
        &mut self,
        group_id: Uuid,
        groups: &[ConnectionGroup],
        connections: &[Connection],
        local_variable_names: &HashSet<String>,
    ) -> Result<(GroupMergeResult, SyncReport), SyncError> {
        // 1. Validate sync_dir
        let sync_dir = self
            .settings
            .sync_dir
            .as_ref()
            .ok_or(SyncError::SyncDirNotConfigured)?
            .clone();

        // 2. Find the group
        let group = groups
            .iter()
            .find(|g| g.id == group_id)
            .ok_or(SyncError::GroupNotFound(group_id))?;

        if group.sync_mode != SyncMode::Import {
            return Err(SyncError::NotImportGroup(group_id));
        }

        // 3. Determine sync file path
        let sync_file = group
            .sync_file
            .as_ref()
            .ok_or(SyncError::GroupNotFound(group_id))?;
        let file_path = sync_dir.join(sync_file);

        // 4. Read and parse the export file
        let export = GroupSyncExport::from_file(&file_path)?;

        // 5. Collect local groups and connections belonging to this root group
        let local_groups = collect_group_tree(group_id, groups);
        let group_ids: HashSet<Uuid> = local_groups.iter().map(|g| g.id).collect();
        let local_connections: Vec<Connection> = connections
            .iter()
            .filter(|c| c.group_id.is_some_and(|gid| group_ids.contains(&gid)))
            .cloned()
            .collect();

        // 6. Run merge
        let merge_result = GroupMergeEngine::merge(
            &local_groups,
            &local_connections,
            &export,
            local_variable_names,
        );

        // 7. Build SyncReport
        let now = Utc::now();
        let report = SyncReport::from_merge_result(&group.name, &merge_result, now);

        // 8. Update sync state
        let state = self.state.entry(group_id).or_default();
        state.last_synced_at = Some(now);
        state.last_error = None;

        tracing::info!(
            group = %group.name,
            added = report.connections_added,
            updated = report.connections_updated,
            removed = report.connections_removed,
            groups_added = report.groups_added,
            groups_removed = report.groups_removed,
            variables = report.variables_created,
            "Import sync completed"
        );

        Ok((merge_result, report))
    }

    /// Imports all Import groups on application startup.
    ///
    /// Iterates all groups with `sync_mode == Import` and `sync_file.is_some()`,
    /// reads each `.rcn` file from the sync directory, compares `exported_at`
    /// with `last_synced_at`, and triggers a merge for groups that have been
    /// updated since the last import.
    ///
    /// Files that don't exist or are corrupt are skipped with a warning log.
    ///
    /// # Arguments
    ///
    /// * `groups` — all local groups
    /// * `connections` — all local connections
    /// * `local_variable_names` — names of variables that already exist locally
    ///
    /// # Returns
    ///
    /// A `Vec` of `(GroupMergeResult, SyncReport)` tuples for each group that
    /// was successfully imported. The caller should apply each `GroupMergeResult`
    /// to the local data store.
    ///
    /// # Panics
    ///
    /// Does not panic. The internal `expect` is guarded by a preceding filter
    /// that ensures `sync_file.is_some()`.
    #[expect(
        clippy::too_many_lines,
        reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
    )]
    pub fn import_all_on_start(
        &mut self,
        groups: &[ConnectionGroup],
        connections: &[Connection],
        local_variable_names: &HashSet<String>,
    ) -> Vec<(GroupMergeResult, SyncReport)> {
        let sync_dir = if let Some(dir) = self.settings.sync_dir.as_ref() {
            dir.clone()
        } else {
            tracing::debug!("Sync directory not configured, skipping startup import");
            return Vec::new();
        };

        if !self.settings.auto_import_on_start {
            tracing::debug!("Auto-import on start is disabled, skipping");
            return Vec::new();
        }

        // Collect Import groups with sync files
        let import_groups: Vec<&ConnectionGroup> = groups
            .iter()
            .filter(|g| g.sync_mode == SyncMode::Import && g.sync_file.is_some())
            .collect();

        if import_groups.is_empty() {
            tracing::debug!("No Import groups found, skipping startup import");
            return Vec::new();
        }

        tracing::info!(
            count = import_groups.len(),
            "Checking Import groups for startup sync"
        );

        let mut reports = Vec::new();

        for group in import_groups {
            let sync_file = group.sync_file.as_ref().expect("filtered above");

            // Validate filename against path traversal
            if let Err(e) = validate_sync_filename(sync_file) {
                tracing::warn!(
                    group = %group.name,
                    sync_file,
                    "Invalid sync filename, skipping import: {e}"
                );
                let state = self.state.entry(group.id).or_default();
                state.last_error = Some(format!("Invalid sync filename: {sync_file}"));
                continue;
            }

            let file_path = sync_dir.join(sync_file);

            // Skip files that don't exist
            if !file_path.exists() {
                tracing::warn!(
                    group = %group.name,
                    file = %file_path.display(),
                    "Sync file not found, skipping import"
                );
                let state = self.state.entry(group.id).or_default();
                state.last_error = Some(format!("Sync file not found: {}", file_path.display()));
                continue;
            }

            // Try to parse the file; skip corrupt files
            let export = match GroupSyncExport::from_file(&file_path) {
                Ok(export) => export,
                Err(e) => {
                    tracing::warn!(
                        group = %group.name,
                        file = %file_path.display(),
                        error = %e,
                        "Failed to parse sync file, skipping import"
                    );
                    let state = self.state.entry(group.id).or_default();
                    state.last_error = Some(format!("Parse error: {e}"));
                    continue;
                }
            };

            // Compare exported_at with last_synced_at
            let needs_import = match group.last_synced_at {
                None => true, // Never synced — always import
                Some(last_synced) => export.exported_at > last_synced,
            };

            if !needs_import {
                tracing::debug!(
                    group = %group.name,
                    exported_at = %export.exported_at,
                    last_synced_at = ?group.last_synced_at,
                    "Sync file not updated since last import, skipping"
                );
                continue;
            }

            // Collect local groups and connections for this import group
            let local_groups = collect_group_tree(group.id, groups);
            let group_ids: HashSet<Uuid> = local_groups.iter().map(|g| g.id).collect();
            let local_connections: Vec<Connection> = connections
                .iter()
                .filter(|c| c.group_id.is_some_and(|gid| group_ids.contains(&gid)))
                .cloned()
                .collect();

            // Run merge
            let merge_result = GroupMergeEngine::merge(
                &local_groups,
                &local_connections,
                &export,
                local_variable_names,
            );

            let now = Utc::now();
            let report = SyncReport::from_merge_result(&group.name, &merge_result, now);

            // Update sync state
            let state = self.state.entry(group.id).or_default();
            state.last_synced_at = Some(now);
            state.last_error = None;

            tracing::info!(
                group = %group.name,
                added = report.connections_added,
                updated = report.connections_updated,
                removed = report.connections_removed,
                groups_added = report.groups_added,
                groups_removed = report.groups_removed,
                variables = report.variables_created,
                "Startup import completed"
            );

            reports.push((merge_result, report));
        }

        if reports.is_empty() {
            tracing::info!("No Import groups needed updating on startup");
        } else {
            tracing::info!(
                count = reports.len(),
                "Startup import completed for {} group(s)",
                reports.len()
            );
        }

        reports
    }

    /// Enables Master sync for a root group: validates `sync_dir`,
    /// generates `sync_file` if not already set, and performs the first
    /// export.
    ///
    /// The `sync_file` is fixed at first export and never changes, even
    /// if the group is later renamed.
    ///
    /// # Arguments
    ///
    /// * `group` — the root group to enable Master sync on (mutated to
    ///   set `sync_file` if not already set)
    /// * `groups` — all groups in the application
    /// * `connections` — all connections in the application
    /// * `variables` — all global variables (for template collection)
    /// * `app_version` — current application version string
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] if sync_dir validation fails, or if the
    /// first export fails.
    pub fn enable_master(
        &mut self,
        group: &mut ConnectionGroup,
        groups: &[ConnectionGroup],
        connections: &[Connection],
        variables: &[Variable],
        app_version: &str,
    ) -> Result<SyncReport, SyncError> {
        // 1. Validate sync_dir exists and is writable
        let _sync_dir = self.validate_sync_dir()?;

        // 2. Generate sync_file from group name if not already set
        if group.sync_file.is_none() {
            group.sync_file = Some(group_name_to_filename(&group.name));
        }

        // 3. Ensure group is in Master mode
        group.sync_mode = SyncMode::Master;

        // 4. Build the groups slice with the updated group for export.
        //    Replace the matching group in the slice so export_group sees
        //    the updated sync_mode and sync_file.
        let mut updated_groups: Vec<ConnectionGroup> = groups
            .iter()
            .map(|g| {
                if g.id == group.id {
                    group.clone()
                } else {
                    g.clone()
                }
            })
            .collect();

        // If the group wasn't in the original slice, add it
        if !updated_groups.iter().any(|g| g.id == group.id) {
            updated_groups.push(group.clone());
        }

        // 5. Perform the first export
        self.export_group(
            group.id,
            &updated_groups,
            connections,
            variables,
            app_version,
        )
    }

    // =========================================================================
    // Simple Sync methods (tasks 8.7–8.9)
    // =========================================================================

    /// Writes the full Simple Sync export to `<sync_dir>/full-sync.rcn`.
    ///
    /// Builds a [`FullSyncExport`](super::full_export::FullSyncExport) from all
    /// app data plus deletion tombstones, stamped with this device's identity
    /// so other devices skip self-sync. Secret variable values and local-only
    /// connection fields are stripped by the builder.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] if the sync directory is not configured/writable
    /// or the file cannot be written.
    #[expect(
        clippy::too_many_arguments,
        reason = "parameters mirror the FullSyncExport entity set 1:1; a struct would only restate the field list"
    )]
    pub fn export_simple_sync(
        &self,
        connections: Vec<Connection>,
        groups: Vec<ConnectionGroup>,
        templates: Vec<crate::models::ConnectionTemplate>,
        snippets: Vec<crate::models::Snippet>,
        clusters: Vec<crate::cluster::Cluster>,
        variables: &[Variable],
        tombstones: Vec<super::tombstone::Tombstone>,
        app_version: &str,
    ) -> Result<(), SyncError> {
        let sync_dir = self.validate_sync_dir()?;

        let export = super::full_export::FullSyncExport::build(
            app_version.to_owned(),
            self.settings.device_id,
            self.settings.device_name.clone(),
            connections,
            groups,
            templates,
            snippets,
            clusters,
            variables,
            tombstones,
        );

        let file_path = sync_dir.join("full-sync.rcn");
        export.to_file(&file_path)
    }

    /// Imports Simple Sync data: runs UUID-based merge with tombstone support.
    ///
    /// Reads the `full-sync.rcn` file from the sync directory, checks the
    /// `device_id` to prevent self-sync, and returns the merge result.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError`] if sync_dir is not configured, the file doesn't
    /// exist, or parsing fails.
    pub fn import_simple_sync(
        &mut self,
        local_state: &super::full_merge::LocalState,
    ) -> Result<
        (
            super::full_merge::FullMergeResult,
            super::full_export::FullSyncExport,
        ),
        SyncError,
    > {
        let sync_dir = self
            .settings
            .sync_dir
            .as_ref()
            .ok_or(SyncError::SyncDirNotConfigured)?
            .clone();

        let file_path = sync_dir.join("full-sync.rcn");
        let remote = super::full_export::FullSyncExport::from_file(&file_path)?;

        // Device ID check — prevent self-sync
        if remote.device_id == local_state.device_id {
            tracing::debug!("Skipping Simple Sync import: same device_id");
            return Ok((super::full_merge::FullMergeResult::default(), remote));
        }

        let result = super::full_merge::FullMergeEngine::merge(local_state, &remote);

        tracing::info!(
            created = result.to_create.len(),
            updated = result.to_update.len(),
            deleted = result.to_delete.len(),
            tombstones_cleaned = result.tombstones_to_remove.len(),
            "Simple Sync import completed"
        );

        Ok((result, remote))
    }

    /// Disables sync on a group, restoring it to a regular editable group.
    ///
    /// Resets `sync_mode` to `None`, clears `sync_file` and `last_synced_at`,
    /// and removes internal sync state for the group. After this call the group
    /// behaves like any other group: all fields are editable, the full context
    /// menu is available, and drag-and-drop works normally.
    pub fn disable_sync(&mut self, group: &mut ConnectionGroup) {
        group.sync_mode = SyncMode::None;
        group.sync_file = None;
        group.last_synced_at = None;
        self.state.remove(&group.id);

        tracing::info!(group = %group.name, "Sync disabled — group is now a regular editable group");
    }

    /// Checks whether a Simple Sync import should be triggered.
    ///
    /// Returns `true` if the `full-sync.rcn` file exists in the sync directory
    /// and was exported by a different device.
    #[must_use]
    pub fn should_import_simple_sync(&self, local_device_id: Uuid) -> bool {
        let Some(ref sync_dir) = self.settings.sync_dir else {
            return false;
        };

        let file_path = sync_dir.join("full-sync.rcn");
        if !file_path.exists() {
            return false;
        }

        // Try to read just enough to check device_id
        match super::full_export::FullSyncExport::from_file(&file_path) {
            Ok(export) => export.device_id != local_device_id,
            Err(_) => false,
        }
    }
}

/// Recursively collects all subgroups of a given root group.
///
/// Performs a breadth-first traversal of the group hierarchy starting from
/// `root_id`, returning all descendant groups (not including the root itself).
fn collect_subgroups(root_id: Uuid, groups: &[ConnectionGroup]) -> Vec<ConnectionGroup> {
    let mut result = Vec::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(root_id);

    while let Some(parent_id) = queue.pop_front() {
        for group in groups {
            if group.parent_id == Some(parent_id) {
                result.push(group.clone());
                queue.push_back(group.id);
            }
        }
    }

    result
}

/// Collects the root group and all its subgroups into a single `Vec`.
///
/// Unlike [`collect_subgroups`], this includes the root group itself,
/// which is needed by [`GroupMergeEngine::merge`] to compute group paths.
fn collect_group_tree(root_id: Uuid, groups: &[ConnectionGroup]) -> Vec<ConnectionGroup> {
    let mut result = Vec::new();
    if let Some(root) = groups.iter().find(|g| g.id == root_id) {
        result.push(root.clone());
    }
    result.extend(collect_subgroups(root_id, groups));
    result
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;

    fn test_settings(sync_dir: Option<PathBuf>) -> SyncSettings {
        SyncSettings {
            sync_dir,
            device_id: Uuid::new_v4(),
            device_name: "test-device".to_owned(),
            auto_import_on_start: true,
            export_debounce_secs: 5,
            tombstone_retention_days: 30,
            simple_sync_enabled: false,
        }
    }

    #[test]
    fn new_creates_empty_state() {
        let mgr = SyncManager::new(test_settings(None));
        assert!(mgr.state().is_empty());
    }

    #[test]
    fn export_simple_sync_writes_readable_full_sync_file() {
        use crate::models::Connection;

        let dir = TempDir::new().unwrap();
        let settings = test_settings(Some(dir.path().to_owned()));
        let device_id = settings.device_id;
        let mgr = SyncManager::new(settings);

        let conn = Connection::new_ssh("web".to_owned(), "host".to_owned(), 22);
        let conn_id = conn.id;

        mgr.export_simple_sync(
            vec![conn],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            &[],
            Vec::new(),
            "0.17.2",
        )
        .expect("export should succeed");

        let path = dir.path().join("full-sync.rcn");
        assert!(path.exists(), "full-sync.rcn should be written");

        let loaded = super::super::full_export::FullSyncExport::from_file(&path).unwrap();
        assert_eq!(loaded.sync_type, "full");
        assert_eq!(loaded.device_id, device_id);
        assert_eq!(loaded.connections.len(), 1);
        assert_eq!(loaded.connections[0].id, conn_id);
    }

    #[test]
    fn should_import_simple_sync_false_for_same_device() {
        use crate::models::Connection;

        let dir = TempDir::new().unwrap();
        let settings = test_settings(Some(dir.path().to_owned()));
        let device_id = settings.device_id;
        let mgr = SyncManager::new(settings);

        mgr.export_simple_sync(
            vec![Connection::new_ssh("a".to_owned(), "h".to_owned(), 22)],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            &[],
            Vec::new(),
            "0.17.2",
        )
        .unwrap();

        // The file we just wrote carries our own device_id, so we must not
        // re-import it.
        assert!(!mgr.should_import_simple_sync(device_id));
        // A different device would import it.
        assert!(mgr.should_import_simple_sync(Uuid::new_v4()));
    }

    #[test]
    fn settings_returns_configured_settings() {
        let settings = test_settings(Some(PathBuf::from("/tmp/sync")));
        let mgr = SyncManager::new(settings);
        assert_eq!(mgr.settings().sync_dir, Some(PathBuf::from("/tmp/sync")));
        assert_eq!(mgr.settings().device_name, "test-device");
    }

    #[test]
    fn state_mut_inserts_default() {
        let mut mgr = SyncManager::new(test_settings(None));
        let group_id = Uuid::new_v4();
        let state = mgr.state_mut(group_id);
        assert!(state.last_synced_at.is_none());
        assert!(state.last_error.is_none());
    }

    #[test]
    fn state_mut_returns_existing() {
        let mut mgr = SyncManager::new(test_settings(None));
        let group_id = Uuid::new_v4();
        let now = Utc::now();
        mgr.state_mut(group_id).last_synced_at = Some(now);
        assert_eq!(mgr.state_mut(group_id).last_synced_at, Some(now));
    }

    #[test]
    fn list_available_sync_files_returns_empty_when_no_sync_dir() {
        let mgr = SyncManager::new(test_settings(None));
        let files = mgr.list_available_sync_files().unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn list_available_sync_files_finds_rcn_files() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("production.rcn"), "{}").unwrap();
        std::fs::write(dir.path().join("staging.rcn"), "{}").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not a sync file").unwrap();
        std::fs::create_dir(dir.path().join("subdir.rcn")).unwrap();

        let mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));
        let files = mgr.list_available_sync_files().unwrap();

        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.ends_with("production.rcn")));
        assert!(files.iter().any(|f| f.ends_with("staging.rcn")));
    }

    #[test]
    fn list_available_sync_files_returns_sorted() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("zebra.rcn"), "{}").unwrap();
        std::fs::write(dir.path().join("alpha.rcn"), "{}").unwrap();
        std::fs::write(dir.path().join("middle.rcn"), "{}").unwrap();

        let mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));
        let files = mgr.list_available_sync_files().unwrap();

        assert_eq!(files.len(), 3);
        assert!(files[0].ends_with("alpha.rcn"));
        assert!(files[1].ends_with("middle.rcn"));
        assert!(files[2].ends_with("zebra.rcn"));
    }

    #[test]
    fn list_available_sync_files_error_on_missing_dir() {
        let mgr = SyncManager::new(test_settings(Some(PathBuf::from(
            "/nonexistent/path/that/does/not/exist",
        ))));
        let result = mgr.list_available_sync_files();
        assert!(result.is_err());
    }

    #[test]
    fn list_available_sync_files_empty_dir() {
        let dir = TempDir::new().unwrap();
        let mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));
        let files = mgr.list_available_sync_files().unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn schedule_export_sends_to_channel() {
        let mut mgr = SyncManager::new(test_settings(None));
        let mut rx = mgr.create_export_channel();
        let group_id = Uuid::new_v4();

        mgr.schedule_export(group_id);

        // Channel should have the group_id
        let received = rx.try_recv().unwrap();
        assert_eq!(received, group_id);
    }

    #[test]
    fn schedule_export_without_channel_does_not_panic() {
        let mgr = SyncManager::new(test_settings(None));
        mgr.schedule_export(Uuid::new_v4());
        // No channel created — should log warning but not panic
    }

    #[test]
    fn schedule_export_multiple_sends_all() {
        let mut mgr = SyncManager::new(test_settings(None));
        let mut rx = mgr.create_export_channel();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        mgr.schedule_export(id1);
        mgr.schedule_export(id2);
        mgr.schedule_export(id3);

        assert_eq!(rx.try_recv().unwrap(), id1);
        assert_eq!(rx.try_recv().unwrap(), id2);
        assert_eq!(rx.try_recv().unwrap(), id3);
    }

    #[test]
    fn create_export_channel_replaces_previous() {
        let mut mgr = SyncManager::new(test_settings(None));
        let mut rx1 = mgr.create_export_channel();
        let mut rx2 = mgr.create_export_channel();

        let group_id = Uuid::new_v4();
        mgr.schedule_export(group_id);

        // Old receiver should get nothing (sender was replaced)
        assert!(rx1.try_recv().is_err());
        // New receiver should get the message
        assert_eq!(rx2.try_recv().unwrap(), group_id);
    }

    #[test]
    fn export_debounce_secs_returns_settings_value() {
        let mut settings = test_settings(None);
        settings.export_debounce_secs = 10;
        let mgr = SyncManager::new(settings);
        assert_eq!(mgr.export_debounce_secs(), 10);
    }

    #[test]
    fn export_debounce_secs_default_is_five() {
        let mgr = SyncManager::new(test_settings(None));
        assert_eq!(mgr.export_debounce_secs(), 5);
    }

    #[test]
    fn group_sync_state_default() {
        let state = GroupSyncState::default();
        assert!(state.last_synced_at.is_none());
        assert!(state.last_error.is_none());
    }

    // --- export_group tests ---

    fn make_master_root_group(name: &str) -> ConnectionGroup {
        let mut g = ConnectionGroup::new(name.to_owned());
        g.sync_mode = SyncMode::Master;
        g.sync_file = Some(format!("{}.rcn", name.to_lowercase().replace(' ', "-")));
        g
    }

    fn make_connection(name: &str, host: &str, group_id: Uuid) -> Connection {
        Connection::new_ssh(name.to_owned(), host.to_owned(), 22).with_group(group_id)
    }

    #[test]
    fn export_group_succeeds_with_valid_master_group() {
        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));

        let root = make_master_root_group("Production");
        let conn = make_connection("nginx-1", "10.0.1.10", root.id);

        let report = mgr
            .export_group(root.id, std::slice::from_ref(&root), &[conn], &[], "0.12.0")
            .unwrap();

        assert_eq!(report.group_name, "Production");
        assert_eq!(report.connections_added, 1);
        assert_eq!(report.groups_added, 0);

        // Verify file was written
        let file_path = dir.path().join("production.rcn");
        assert!(file_path.exists());

        // Verify sync state was updated
        let state = mgr.state().get(&root.id).unwrap();
        assert!(state.last_synced_at.is_some());
        assert!(state.last_error.is_none());
    }

    #[test]
    fn export_group_fails_when_sync_dir_not_configured() {
        let mut mgr = SyncManager::new(test_settings(None));
        let root = make_master_root_group("Test");

        let err = mgr
            .export_group(root.id, &[root], &[], &[], "0.12.0")
            .unwrap_err();
        assert!(matches!(err, SyncError::SyncDirNotConfigured));
    }

    #[test]
    fn export_group_fails_when_group_not_found() {
        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));
        let missing_id = Uuid::new_v4();

        let err = mgr
            .export_group(missing_id, &[], &[], &[], "0.12.0")
            .unwrap_err();
        assert!(matches!(err, SyncError::GroupNotFound(id) if id == missing_id));
    }

    #[test]
    fn export_group_fails_when_not_root_group() {
        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));

        let parent = make_master_root_group("Parent");
        let mut child = ConnectionGroup::with_parent("Child".to_owned(), parent.id);
        child.sync_mode = SyncMode::Master;

        let err = mgr
            .export_group(child.id, &[parent, child.clone()], &[], &[], "0.12.0")
            .unwrap_err();
        assert!(matches!(err, SyncError::NotRootGroup(id) if id == child.id));
    }

    #[test]
    fn export_group_fails_when_not_master_mode() {
        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));

        let mut root = ConnectionGroup::new("Import Group".to_owned());
        root.sync_mode = SyncMode::Import;

        let err = mgr
            .export_group(root.id, std::slice::from_ref(&root), &[], &[], "0.12.0")
            .unwrap_err();
        assert!(matches!(err, SyncError::NotMasterGroup(id) if id == root.id));
    }

    #[test]
    fn export_group_includes_subgroups_and_their_connections() {
        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));

        let root = make_master_root_group("Production");
        let child = ConnectionGroup::with_parent("Web".to_owned(), root.id);
        let grandchild = ConnectionGroup::with_parent("Backend".to_owned(), child.id);

        let conn_root = make_connection("bastion", "10.0.0.1", root.id);
        let conn_child = make_connection("nginx-1", "10.0.1.10", child.id);
        let conn_grandchild = make_connection("api-1", "10.0.2.10", grandchild.id);

        let groups = vec![root.clone(), child, grandchild];
        let connections = vec![conn_root, conn_child, conn_grandchild];

        let report = mgr
            .export_group(root.id, &groups, &connections, &[], "0.12.0")
            .unwrap();

        assert_eq!(report.connections_added, 3);
        assert_eq!(report.groups_added, 2); // Web + Backend (not root)
    }

    #[test]
    fn export_group_excludes_connections_from_other_groups() {
        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));

        let root = make_master_root_group("Production");
        let other = ConnectionGroup::new("Other Group".to_owned());

        let conn_ours = make_connection("nginx-1", "10.0.1.10", root.id);
        let conn_other = make_connection("other-server", "10.0.2.10", other.id);

        let groups = vec![root.clone(), other];
        let connections = vec![conn_ours, conn_other];

        let report = mgr
            .export_group(root.id, &groups, &connections, &[], "0.12.0")
            .unwrap();

        assert_eq!(report.connections_added, 1);
    }

    #[test]
    fn export_group_empty_group_produces_valid_file() {
        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));

        let root = make_master_root_group("Empty");

        let report = mgr
            .export_group(root.id, &[root], &[], &[], "0.12.0")
            .unwrap();

        assert_eq!(report.connections_added, 0);
        assert_eq!(report.groups_added, 0);

        // Verify the file is valid JSON
        let file_path = dir.path().join("empty.rcn");
        assert!(file_path.exists());
        let export = super::super::group_export::GroupSyncExport::from_file(&file_path).unwrap();
        assert_eq!(export.sync_version, 1);
        assert_eq!(export.sync_type, "group");
        assert!(export.connections.is_empty());
        assert!(export.groups.is_empty());
    }

    #[test]
    fn export_group_generates_filename_when_sync_file_is_none() {
        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));

        let mut root = ConnectionGroup::new("My Servers".to_owned());
        root.sync_mode = SyncMode::Master;
        // sync_file is None — should auto-generate from name

        let _report = mgr
            .export_group(root.id, &[root], &[], &[], "0.12.0")
            .unwrap();

        let file_path = dir.path().join("my-servers.rcn");
        assert!(file_path.exists());
    }

    #[test]
    fn export_group_collects_variable_templates() {
        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));

        let root = make_master_root_group("Production");
        let mut conn = make_connection("nginx-1", "10.0.1.10", root.id);
        conn.password_source = crate::models::PasswordSource::Variable("web_key".to_owned());

        let vars = vec![
            crate::variables::Variable::new_secret("web_key", "secret")
                .with_description("Web deploy key"),
        ];

        let report = mgr
            .export_group(root.id, &[root], &[conn], &vars, "0.12.0")
            .unwrap();

        assert_eq!(report.variables_created, 1);

        // Verify the file contains the variable template
        let file_path = dir.path().join("production.rcn");
        let export = super::super::group_export::GroupSyncExport::from_file(&file_path).unwrap();
        assert_eq!(export.variable_templates.len(), 1);
        assert_eq!(export.variable_templates[0].name, "web_key");
        assert!(export.variable_templates[0].is_secret);
        // Secret value must NOT be in the template
        assert_eq!(export.variable_templates[0].default_value, None);
    }

    // --- collect_subgroups tests ---

    #[test]
    fn collect_subgroups_returns_empty_for_leaf_group() {
        let root = ConnectionGroup::new("Root".to_owned());
        let result = collect_subgroups(root.id, &[root]);
        assert!(result.is_empty());
    }

    #[test]
    fn collect_subgroups_finds_direct_children() {
        let root = ConnectionGroup::new("Root".to_owned());
        let child1 = ConnectionGroup::with_parent("A".to_owned(), root.id);
        let child2 = ConnectionGroup::with_parent("B".to_owned(), root.id);
        let groups = vec![root.clone(), child1.clone(), child2.clone()];

        let result = collect_subgroups(root.id, &groups);
        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|g| g.id == child1.id));
        assert!(result.iter().any(|g| g.id == child2.id));
    }

    #[test]
    fn collect_subgroups_finds_nested_children() {
        let root = ConnectionGroup::new("Root".to_owned());
        let child = ConnectionGroup::with_parent("Child".to_owned(), root.id);
        let grandchild = ConnectionGroup::with_parent("Grandchild".to_owned(), child.id);
        let groups = vec![root.clone(), child.clone(), grandchild.clone()];

        let result = collect_subgroups(root.id, &groups);
        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|g| g.id == child.id));
        assert!(result.iter().any(|g| g.id == grandchild.id));
    }

    #[test]
    fn collect_subgroups_excludes_unrelated_groups() {
        let root = ConnectionGroup::new("Root".to_owned());
        let child = ConnectionGroup::with_parent("Child".to_owned(), root.id);
        let other = ConnectionGroup::new("Other".to_owned());
        let groups = vec![root.clone(), child.clone(), other];

        let result = collect_subgroups(root.id, &groups);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, child.id);
    }

    // --- validate_sync_dir tests ---

    #[test]
    fn validate_sync_dir_succeeds_for_writable_dir() {
        let dir = TempDir::new().unwrap();
        let mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));
        let result = mgr.validate_sync_dir();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), dir.path());
    }

    #[test]
    fn validate_sync_dir_fails_when_not_configured() {
        let mgr = SyncManager::new(test_settings(None));
        let err = mgr.validate_sync_dir().unwrap_err();
        assert!(matches!(err, SyncError::SyncDirNotConfigured));
    }

    #[test]
    fn validate_sync_dir_fails_for_nonexistent_dir() {
        let mgr = SyncManager::new(test_settings(Some(PathBuf::from(
            "/nonexistent/path/that/does/not/exist",
        ))));
        let err = mgr.validate_sync_dir().unwrap_err();
        assert!(matches!(err, SyncError::SyncDirNotWritable(_)));
    }

    #[test]
    fn validate_sync_dir_fails_when_path_is_a_file() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("not-a-dir");
        std::fs::write(&file_path, "content").unwrap();

        let mgr = SyncManager::new(test_settings(Some(file_path.clone())));
        let err = mgr.validate_sync_dir().unwrap_err();
        assert!(matches!(err, SyncError::SyncDirNotWritable(p) if p == file_path));
    }

    // --- enable_master tests ---

    #[test]
    fn enable_master_succeeds_and_generates_sync_file() {
        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));

        let mut group = ConnectionGroup::new("Production Servers".to_owned());
        let conn = make_connection("nginx-1", "10.0.1.10", group.id);
        let groups = vec![group.clone()];

        let report = mgr
            .enable_master(&mut group, &groups, &[conn], &[], "0.12.0")
            .unwrap();

        // sync_file should be generated from group name
        assert_eq!(group.sync_file, Some("production-servers.rcn".to_owned()));
        // sync_mode should be set to Master
        assert_eq!(group.sync_mode, SyncMode::Master);
        // Report should reflect the export
        assert_eq!(report.group_name, "Production Servers");
        assert_eq!(report.connections_added, 1);
        // File should exist
        assert!(dir.path().join("production-servers.rcn").exists());
    }

    #[test]
    fn enable_master_preserves_existing_sync_file() {
        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));

        let mut group = ConnectionGroup::new("Production Servers".to_owned());
        group.sync_file = Some("custom-name.rcn".to_owned());
        let groups = vec![group.clone()];

        let _report = mgr
            .enable_master(&mut group, &groups, &[], &[], "0.12.0")
            .unwrap();

        // sync_file should NOT be overwritten
        assert_eq!(group.sync_file, Some("custom-name.rcn".to_owned()));
        // File should be written with the custom name
        assert!(dir.path().join("custom-name.rcn").exists());
    }

    #[test]
    fn enable_master_fails_when_sync_dir_not_configured() {
        let mut mgr = SyncManager::new(test_settings(None));
        let mut group = ConnectionGroup::new("Test".to_owned());

        let err = mgr
            .enable_master(&mut group, &[], &[], &[], "0.12.0")
            .unwrap_err();
        assert!(matches!(err, SyncError::SyncDirNotConfigured));
        // Group should not be modified on failure
        assert_eq!(group.sync_mode, SyncMode::None);
    }

    #[test]
    fn enable_master_fails_when_sync_dir_not_writable() {
        let mgr_settings = test_settings(Some(PathBuf::from("/nonexistent/path")));
        let mut mgr = SyncManager::new(mgr_settings);
        let mut group = ConnectionGroup::new("Test".to_owned());

        let err = mgr
            .enable_master(&mut group, &[], &[], &[], "0.12.0")
            .unwrap_err();
        assert!(matches!(err, SyncError::SyncDirNotWritable(_)));
    }

    #[test]
    fn enable_master_updates_sync_state() {
        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));

        let mut group = ConnectionGroup::new("Staging".to_owned());
        let groups = vec![group.clone()];

        let _report = mgr
            .enable_master(&mut group, &groups, &[], &[], "0.12.0")
            .unwrap();

        // Sync state should be updated
        let state = mgr.state().get(&group.id).unwrap();
        assert!(state.last_synced_at.is_some());
        assert!(state.last_error.is_none());
    }

    #[test]
    fn enable_master_with_connections_and_subgroups() {
        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));

        let mut root = ConnectionGroup::new("Production".to_owned());
        let child = ConnectionGroup::with_parent("Web".to_owned(), root.id);
        let conn1 = make_connection("bastion", "10.0.0.1", root.id);
        let conn2 = make_connection("nginx-1", "10.0.1.10", child.id);

        let groups = vec![root.clone(), child];
        let connections = vec![conn1, conn2];

        let report = mgr
            .enable_master(&mut root, &groups, &connections, &[], "0.12.0")
            .unwrap();

        assert_eq!(report.connections_added, 2);
        assert_eq!(report.groups_added, 1); // Web subgroup
        assert!(dir.path().join("production.rcn").exists());
    }

    // --- SyncReport::from_merge_result tests ---

    #[test]
    fn sync_report_from_empty_merge_result() {
        let result = super::GroupMergeResult::default();
        let report = SyncReport::from_merge_result("Test", &result, Utc::now());
        assert_eq!(report.group_name, "Test");
        assert_eq!(report.connections_added, 0);
        assert_eq!(report.connections_updated, 0);
        assert_eq!(report.connections_removed, 0);
        assert_eq!(report.groups_added, 0);
        assert_eq!(report.groups_removed, 0);
        assert_eq!(report.variables_created, 0);
    }

    #[test]
    fn sync_report_counts_merge_result_items() {
        use crate::models::{
            AutomationConfig, PasswordSource, ProtocolConfig, ProtocolType, SshConfig,
        };
        use crate::sync::group_export::{SyncConnection, SyncGroup};
        use crate::sync::variable_template::VariableTemplate;

        let make_sc = |name: &str| SyncConnection {
            name: name.to_owned(),
            group_path: "Root".to_owned(),
            host: "h".to_owned(),
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

        let result = super::GroupMergeResult {
            connections_to_create: vec![make_sc("a"), make_sc("b")],
            connections_to_update: vec![(Uuid::new_v4(), make_sc("c"))],
            connections_to_delete: vec![Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()],
            groups_to_create: vec![SyncGroup {
                name: "G".to_owned(),
                path: "Root/G".to_owned(),
                description: None,
                icon: None,
                username: None,
                domain: None,
                ssh_auth_method: None,
                ssh_proxy_jump: None,
            }],
            groups_to_delete: vec![],
            variables_to_create: vec![VariableTemplate {
                name: "v".to_owned(),
                description: None,
                is_secret: false,
                default_value: None,
            }],
        };

        let report = SyncReport::from_merge_result("Prod", &result, Utc::now());
        assert_eq!(report.connections_added, 2);
        assert_eq!(report.connections_updated, 1);
        assert_eq!(report.connections_removed, 3);
        assert_eq!(report.groups_added, 1);
        assert_eq!(report.groups_removed, 0);
        assert_eq!(report.variables_created, 1);
    }

    // --- collect_group_tree tests ---

    #[test]
    fn collect_group_tree_includes_root() {
        let root = ConnectionGroup::new("Root".to_owned());
        let result = collect_group_tree(root.id, std::slice::from_ref(&root));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, root.id);
    }

    #[test]
    fn collect_group_tree_includes_root_and_children() {
        let root = ConnectionGroup::new("Root".to_owned());
        let child = ConnectionGroup::with_parent("Child".to_owned(), root.id);
        let result = collect_group_tree(root.id, &[root.clone(), child.clone()]);
        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|g| g.id == root.id));
        assert!(result.iter().any(|g| g.id == child.id));
    }

    // --- import_group tests ---

    /// Helper: create an Import group with a sync file, export a file for it.
    fn setup_import_scenario(dir: &TempDir) -> (ConnectionGroup, GroupSyncExport) {
        use crate::models::{
            AutomationConfig, PasswordSource, ProtocolConfig, ProtocolType, SshConfig,
        };
        use crate::sync::group_export::{SyncConnection, SyncGroup};

        let mut group = ConnectionGroup::new("Imported".to_owned());
        group.sync_mode = SyncMode::Import;
        group.sync_file = Some("imported.rcn".to_owned());

        let export = GroupSyncExport::from_group_tree(
            "0.12.0".to_owned(),
            Uuid::new_v4(),
            "master-device".to_owned(),
            SyncGroup {
                name: "Imported".to_owned(),
                path: "Imported".to_owned(),
                description: None,
                icon: None,
                username: None,
                domain: None,
                ssh_auth_method: None,
                ssh_proxy_jump: None,
            },
            vec![],
            vec![SyncConnection {
                name: "server-1".to_owned(),
                group_path: "Imported".to_owned(),
                host: "10.0.0.1".to_owned(),
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
            }],
            vec![],
        );

        let file_path = dir.path().join("imported.rcn");
        export.to_file(&file_path).unwrap();

        (group, export)
    }

    #[test]
    fn import_group_succeeds_with_valid_import_group() {
        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));
        let (group, _export) = setup_import_scenario(&dir);

        let (merge_result, report) = mgr
            .import_group(group.id, std::slice::from_ref(&group), &[], &HashSet::new())
            .unwrap();

        assert_eq!(report.group_name, "Imported");
        assert_eq!(report.connections_added, 1);
        assert_eq!(merge_result.connections_to_create.len(), 1);
        assert_eq!(merge_result.connections_to_create[0].name, "server-1");

        // Sync state updated
        let state = mgr.state().get(&group.id).unwrap();
        assert!(state.last_synced_at.is_some());
        assert!(state.last_error.is_none());
    }

    #[test]
    fn import_group_fails_when_sync_dir_not_configured() {
        let mut mgr = SyncManager::new(test_settings(None));
        let mut group = ConnectionGroup::new("Test".to_owned());
        group.sync_mode = SyncMode::Import;
        group.sync_file = Some("test.rcn".to_owned());

        let err = mgr
            .import_group(group.id, &[group], &[], &HashSet::new())
            .unwrap_err();
        assert!(matches!(err, SyncError::SyncDirNotConfigured));
    }

    #[test]
    fn import_group_fails_when_not_import_mode() {
        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));
        let group = make_master_root_group("Master");

        let err = mgr
            .import_group(group.id, &[group], &[], &HashSet::new())
            .unwrap_err();
        assert!(matches!(err, SyncError::NotImportGroup(_)));
    }

    // --- import_all_on_start tests ---

    #[test]
    fn import_all_on_start_returns_empty_when_no_sync_dir() {
        let mut mgr = SyncManager::new(test_settings(None));
        let reports = mgr.import_all_on_start(&[], &[], &HashSet::new());
        assert!(reports.is_empty());
    }

    #[test]
    fn import_all_on_start_returns_empty_when_no_import_groups() {
        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));
        let root = make_master_root_group("Master");

        let reports = mgr.import_all_on_start(&[root], &[], &HashSet::new());
        assert!(reports.is_empty());
    }

    #[test]
    fn import_all_on_start_imports_group_with_no_last_synced() {
        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));
        let (group, _) = setup_import_scenario(&dir);
        // group.last_synced_at is None → should always import

        let reports = mgr.import_all_on_start(&[group], &[], &HashSet::new());
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].1.group_name, "Imported");
        assert_eq!(reports[0].1.connections_added, 1);
    }

    #[test]
    fn import_all_on_start_skips_when_not_updated() {
        use chrono::Duration;

        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));
        let (mut group, _) = setup_import_scenario(&dir);
        // Set last_synced_at to the future so exported_at < last_synced_at
        group.last_synced_at = Some(Utc::now() + Duration::hours(1));

        let reports = mgr.import_all_on_start(&[group], &[], &HashSet::new());
        assert!(reports.is_empty());
    }

    #[test]
    fn import_all_on_start_skips_missing_files() {
        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));

        let mut group = ConnectionGroup::new("Missing".to_owned());
        group.sync_mode = SyncMode::Import;
        group.sync_file = Some("nonexistent.rcn".to_owned());

        let reports = mgr.import_all_on_start(std::slice::from_ref(&group), &[], &HashSet::new());
        assert!(reports.is_empty());

        // Error state should be recorded
        let state = mgr.state().get(&group.id).unwrap();
        assert!(state.last_error.is_some());
    }

    #[test]
    fn import_all_on_start_skips_corrupt_files() {
        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));

        // Write corrupt JSON
        std::fs::write(dir.path().join("corrupt.rcn"), "not valid json {{{").unwrap();

        let mut group = ConnectionGroup::new("Corrupt".to_owned());
        group.sync_mode = SyncMode::Import;
        group.sync_file = Some("corrupt.rcn".to_owned());

        let reports = mgr.import_all_on_start(std::slice::from_ref(&group), &[], &HashSet::new());
        assert!(reports.is_empty());

        // Error state should be recorded
        let state = mgr.state().get(&group.id).unwrap();
        assert!(state.last_error.is_some());
        assert!(state.last_error.as_ref().unwrap().contains("Parse error"));
    }

    #[test]
    fn import_all_on_start_skips_when_auto_import_disabled() {
        let dir = TempDir::new().unwrap();
        let mut settings = test_settings(Some(dir.path().to_owned()));
        settings.auto_import_on_start = false;
        let mut mgr = SyncManager::new(settings);
        let (group, _) = setup_import_scenario(&dir);

        let reports = mgr.import_all_on_start(&[group], &[], &HashSet::new());
        assert!(reports.is_empty());
    }

    #[test]
    fn disable_sync_on_import_group_restores_editability() {
        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));

        // Create an Import group with sync fields set
        let mut group = ConnectionGroup::new("Imported Servers".to_owned());
        group.sync_mode = SyncMode::Import;
        group.sync_file = Some("imported-servers.rcn".to_owned());
        group.last_synced_at = Some(Utc::now());

        // Seed internal sync state so we can verify it gets removed
        let state = mgr.state_mut(group.id);
        state.last_synced_at = Some(Utc::now());

        // Disable sync
        mgr.disable_sync(&mut group);

        // Group fields should be reset
        assert_eq!(group.sync_mode, SyncMode::None);
        assert!(group.sync_file.is_none());
        assert!(group.last_synced_at.is_none());

        // Internal sync state should be removed
        assert!(!mgr.state().contains_key(&group.id));
    }

    #[test]
    fn disable_sync_on_master_group_restores_editability() {
        let dir = TempDir::new().unwrap();
        let mut mgr = SyncManager::new(test_settings(Some(dir.path().to_owned())));

        // Create a Master group
        let mut group = make_master_root_group("Production");
        group.last_synced_at = Some(Utc::now());

        let state = mgr.state_mut(group.id);
        state.last_synced_at = Some(Utc::now());

        mgr.disable_sync(&mut group);

        assert_eq!(group.sync_mode, SyncMode::None);
        assert!(group.sync_file.is_none());
        assert!(group.last_synced_at.is_none());
        assert!(!mgr.state().contains_key(&group.id));
    }
}
