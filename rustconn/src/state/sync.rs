//! Cloud Sync operations: sync_now, startup sync, import with credentials.
//!
//! Extracted from `state.rs` as part of ARCH-5 decomposition.

use rustconn_core::import::ImportResult;
use rustconn_core::models::{Connection, ConnectionGroup};
use rustconn_core::sync::{
    FullMergeResult, FullSyncExport, LocalState, SyncEntityType, SyncManager, SyncReport, Tombstone,
};
use uuid::Uuid;

use super::AppState;

/// Summary of a Simple Sync import: how many entities changed locally.
#[derive(Debug, Default, Clone, Copy)]
pub struct SimpleSyncReport {
    /// Entities created from the remote export.
    pub created: usize,
    /// Entities updated from the remote export.
    pub updated: usize,
    /// Entities deleted via remote tombstones.
    pub deleted: usize,
}

impl SimpleSyncReport {
    /// Returns `true` if the import changed nothing locally.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.created == 0 && self.updated == 0 && self.deleted == 0
    }
}

impl AppState {
    /// Gets the sync manager
    pub fn sync_manager(&self) -> &SyncManager {
        &self.sync_manager
    }

    /// Gets mutable reference to the sync manager
    pub fn sync_manager_mut(&mut self) -> &mut SyncManager {
        &mut self.sync_manager
    }

    // ========== Cloud Sync Operations ==========

    /// Performs a sync-now operation for a specific group.
    ///
    /// For Master groups: exports to the `.rcn` file.
    /// For Import groups: imports from the `.rcn` file and applies merge results.
    ///
    /// # Errors
    ///
    /// Returns a human-readable error string on failure.
    pub fn sync_now_group(&mut self, group_id: Uuid) -> Result<SyncReport, String> {
        use rustconn_core::sync::settings::SyncMode;

        let group = self
            .connection_manager
            .list_groups()
            .into_iter()
            .find(|g| g.id == group_id)
            .cloned()
            .ok_or_else(|| "Group not found".to_string())?;

        match group.sync_mode {
            SyncMode::Master => {
                let groups: Vec<ConnectionGroup> = self
                    .connection_manager
                    .list_groups()
                    .into_iter()
                    .cloned()
                    .collect();
                let connections: Vec<Connection> = self
                    .connection_manager
                    .list_connections()
                    .into_iter()
                    .cloned()
                    .collect();
                let variables = self.settings.global_variables.clone();
                let app_version = env!("CARGO_PKG_VERSION").to_string();

                let report = self
                    .sync_manager
                    .export_group(group_id, &groups, &connections, &variables, &app_version)
                    .map_err(|e| format!("{e}"))?;

                // Update group sync_file (if not set) and last_synced_at
                let mut updated = group;
                if updated.sync_file.is_none() {
                    updated.sync_file = Some(
                        rustconn_core::sync::group_export::group_name_to_filename(&updated.name),
                    );
                }
                updated.last_synced_at = Some(chrono::Utc::now());
                if let Err(e) = self.connection_manager.update_group(group_id, updated) {
                    tracing::warn!(?e, "Failed to update group after export");
                }

                Ok(report)
            }
            SyncMode::Import => {
                let groups: Vec<ConnectionGroup> = self
                    .connection_manager
                    .list_groups()
                    .into_iter()
                    .cloned()
                    .collect();
                let connections: Vec<Connection> = self
                    .connection_manager
                    .list_connections()
                    .into_iter()
                    .cloned()
                    .collect();
                let local_var_names: std::collections::HashSet<String> = self
                    .settings
                    .global_variables
                    .iter()
                    .map(|v| v.name.clone())
                    .collect();

                let (merge_result, report) = self
                    .sync_manager
                    .import_group(group_id, &groups, &connections, &local_var_names)
                    .map_err(|e| format!("{e}"))?;

                // Apply merge results to local data
                self.apply_group_merge_result(group_id, &merge_result);

                // Update last_synced_at on the group
                if let Some(group) = self
                    .connection_manager
                    .list_groups()
                    .into_iter()
                    .find(|g| g.id == group_id)
                {
                    let mut updated = group.clone();
                    updated.last_synced_at = Some(chrono::Utc::now());
                    let _ = self.connection_manager.update_group(group_id, updated);
                }

                Ok(report)
            }
            SyncMode::None => Err("Group is not configured for sync".to_string()),
        }
    }

    /// Applies a `GroupMergeResult` to the local connection manager.
    fn apply_group_merge_result(
        &mut self,
        root_group_id: Uuid,
        merge_result: &rustconn_core::sync::GroupMergeResult,
    ) {
        // Create new groups
        for sync_group in &merge_result.groups_to_create {
            if let Err(e) = self
                .connection_manager
                .create_group_with_parent(sync_group.name.clone(), root_group_id)
            {
                tracing::warn!(name = %sync_group.name, ?e, "Failed to create synced group");
            }
        }

        // Create new connections
        for sync_conn in &merge_result.connections_to_create {
            let conn = rustconn_core::sync::group_export::sync_connection_to_connection(
                sync_conn,
                root_group_id,
            );
            if let Err(e) = self.connection_manager.create_connection_from(conn) {
                tracing::warn!(name = %sync_conn.name, ?e, "Failed to create synced connection");
            }
        }

        // Update existing connections
        for (conn_id, sync_conn) in &merge_result.connections_to_update {
            if let Some(existing) = self.connection_manager.get_connection(*conn_id) {
                let mut updated = existing.clone();
                rustconn_core::sync::group_export::apply_sync_connection_update(
                    &mut updated,
                    sync_conn,
                );
                if let Err(e) = self.connection_manager.update_connection(*conn_id, updated) {
                    tracing::warn!(id = %conn_id, ?e, "Failed to update synced connection");
                }
            }
        }

        // Delete connections
        for conn_id in &merge_result.connections_to_delete {
            if let Err(e) = self.connection_manager.delete_connection(*conn_id) {
                tracing::warn!(id = %conn_id, ?e, "Failed to delete synced connection");
            }
        }

        // Delete groups
        for group_id in &merge_result.groups_to_delete {
            if let Err(e) = self.connection_manager.delete_group(*group_id) {
                tracing::warn!(id = %group_id, ?e, "Failed to delete synced group");
            }
        }
    }

    /// Runs startup import for all Import groups.
    ///
    /// Returns a list of sync reports for groups that were imported.
    pub fn run_startup_sync(&mut self) -> Vec<SyncReport> {
        let groups: Vec<ConnectionGroup> = self
            .connection_manager
            .list_groups()
            .into_iter()
            .cloned()
            .collect();
        let connections: Vec<Connection> = self
            .connection_manager
            .list_connections()
            .into_iter()
            .cloned()
            .collect();
        let local_var_names: std::collections::HashSet<String> = self
            .settings
            .global_variables
            .iter()
            .map(|v| v.name.clone())
            .collect();

        let results =
            self.sync_manager
                .import_all_on_start(&groups, &connections, &local_var_names);

        let mut reports = Vec::new();
        for (merge_result, report) in &results {
            // Find the group_id from the report name
            if let Some(group) = groups.iter().find(|g| g.name == report.group_name) {
                self.apply_group_merge_result(group.id, merge_result);

                // Update last_synced_at
                let mut updated = group.clone();
                updated.last_synced_at = Some(report.timestamp);
                let _ = self.connection_manager.update_group(group.id, updated);
            }
            reports.push(report.clone());
        }

        reports
    }

    // ========== Simple Sync (bidirectional full sync) ==========

    /// Records a deletion tombstone when Simple Sync is enabled, then persists.
    ///
    /// No-op when Simple Sync is disabled, so users who never enable it do not
    /// accumulate tombstones. Call this from local user-initiated deletes only —
    /// not when applying a remote deletion (that would re-broadcast it).
    pub fn record_tombstone(&mut self, entity_type: SyncEntityType, id: Uuid) {
        self.record_tombstones(std::iter::once((entity_type, id)));
    }

    /// Records several deletion tombstones at once (one disk write), when
    /// Simple Sync is enabled. Used by cascade deletes.
    pub fn record_tombstones(&mut self, items: impl IntoIterator<Item = (SyncEntityType, Uuid)>) {
        if !self.settings.sync.simple_sync_enabled {
            return;
        }
        let mut any = false;
        for (entity_type, id) in items {
            self.tombstones.push(Tombstone::new(entity_type, id));
            any = true;
        }
        if any {
            self.persist_tombstones();
            self.simple_sync_dirty.set(true);
        }
    }

    /// Persists tombstones to disk (best-effort; logs on failure).
    fn persist_tombstones(&self) {
        if let Err(e) = self.config_manager.save_tombstones(&self.tombstones) {
            tracing::warn!(error = %e, "Failed to save Simple Sync tombstones");
        }
    }

    /// Returns `true` if Simple Sync is enabled in settings.
    #[must_use]
    pub fn simple_sync_enabled(&self) -> bool {
        self.settings.sync.simple_sync_enabled
    }

    /// Marks local data as changed so the poll loop re-exports. No-op when
    /// Simple Sync is disabled.
    pub fn mark_simple_sync_dirty(&self) {
        if self.settings.sync.simple_sync_enabled {
            self.simple_sync_dirty.set(true);
        }
    }

    /// Returns whether a Simple Sync export is pending, clearing the flag.
    pub fn take_simple_sync_dirty(&self) -> bool {
        self.simple_sync_dirty.replace(false)
    }

    /// Writes the full Simple Sync export (`full-sync.rcn`) from current state.
    ///
    /// Clusters are intentionally excluded: the [`Cluster`](rustconn_core::cluster::Cluster)
    /// model has no `updated_at`, so the UUID merge cannot resolve cluster
    /// conflicts reliably. Connections, groups, templates, snippets, and
    /// non-secret variables are synced.
    ///
    /// # Errors
    ///
    /// Returns an error string if the export fails (sync directory missing or
    /// not writable).
    pub fn simple_sync_export(&self) -> Result<(), String> {
        let connections: Vec<Connection> = self
            .connection_manager
            .list_connections()
            .into_iter()
            .cloned()
            .collect();
        let groups: Vec<ConnectionGroup> = self
            .connection_manager
            .list_groups()
            .into_iter()
            .cloned()
            .collect();
        let templates: Vec<_> = self
            .template_manager
            .list_templates()
            .into_iter()
            .cloned()
            .collect();
        let snippets: Vec<_> = self
            .snippet_manager
            .list_snippets()
            .into_iter()
            .cloned()
            .collect();
        let variables = self.settings.global_variables.clone();
        let app_version = env!("CARGO_PKG_VERSION");

        self.sync_manager
            .export_simple_sync(
                connections,
                groups,
                templates,
                snippets,
                Vec::new(), // clusters excluded (no updated_at)
                &variables,
                self.tombstones.clone(),
                app_version,
            )
            .map_err(|e| format!("Simple Sync export failed: {e}"))
    }

    /// Builds the [`LocalState`] snapshot used as merge input.
    fn build_simple_sync_local_state(&self) -> LocalState {
        let connections = self
            .connection_manager
            .list_connections()
            .into_iter()
            .map(|c| (c.id, c.updated_at))
            .collect();
        let groups = self
            .connection_manager
            .list_groups()
            .into_iter()
            .map(|g| (g.id, g.updated_at))
            .collect();
        let templates = self
            .template_manager
            .list_templates()
            .into_iter()
            .map(|t| (t.id, t.updated_at))
            .collect();
        let snippets = self
            .snippet_manager
            .list_snippets()
            .into_iter()
            .map(|s| (s.id, s.updated_at))
            .collect();

        LocalState {
            device_id: self.settings.sync.device_id,
            connections,
            groups,
            templates,
            snippets,
            clusters: std::collections::HashMap::new(), // excluded
            tombstones: self.tombstones.clone(),
            retention_days: self.settings.sync.tombstone_retention_days,
        }
    }

    /// Imports `full-sync.rcn`, merges it into local state, applies the result,
    /// prunes expired tombstones, and re-exports so the merged state and our
    /// tombstones are published back.
    ///
    /// Skips silently (returns an empty report) when the remote file was
    /// written by this same device.
    ///
    /// # Errors
    ///
    /// Returns an error string if reading/parsing the sync file fails.
    pub fn simple_sync_import_and_apply(&mut self) -> Result<SimpleSyncReport, String> {
        let local_state = self.build_simple_sync_local_state();

        let (result, remote) = self
            .sync_manager
            .import_simple_sync(&local_state)
            .map_err(|e| format!("Simple Sync import failed: {e}"))?;

        let report = self.apply_simple_sync_result(&result, &remote);

        // Adopt the remote tombstones into our own set. The Simple Sync file is
        // a single shared `full-sync.rcn` (last-writer-wins): if we re-exported
        // without the deletions another device recorded, a third device would
        // never learn of them. Carrying the union forward guarantees
        // propagation; retention cleanup bounds the set.
        let tomb_before = self.tombstones.len();
        self.merge_remote_tombstones(&remote.tombstones);
        let added_tombstones = self.tombstones.len() != tomb_before;

        let pruned = rustconn_core::sync::cleanup_tombstones(
            &self.tombstones,
            self.retention_days_for_sync(),
        );
        let pruned_tombstones = pruned.len() != self.tombstones.len();
        self.tombstones = pruned;

        let tombstones_changed = added_tombstones || pruned_tombstones;
        if tombstones_changed {
            self.persist_tombstones();
        }

        // Re-export only when this import actually changed something locally —
        // otherwise two devices would ping-pong import↔export forever. Once the
        // data and tombstone sets converge, the import is a no-op and we stop.
        if (!report.is_empty() || tombstones_changed)
            && let Err(e) = self.simple_sync_export()
        {
            tracing::warn!(error = %e, "Simple Sync re-export after import failed");
        }

        Ok(report)
    }

    /// Tombstone retention window (days) from sync settings.
    fn retention_days_for_sync(&self) -> u32 {
        self.settings.sync.tombstone_retention_days
    }

    /// Merges remote tombstones into the local set, de-duplicating by
    /// `(entity_type, id)` and keeping the latest `deleted_at`.
    fn merge_remote_tombstones(&mut self, remote: &[Tombstone]) {
        for rt in remote {
            if let Some(existing) = self
                .tombstones
                .iter_mut()
                .find(|t| t.entity_type == rt.entity_type && t.id == rt.id)
            {
                if rt.deleted_at > existing.deleted_at {
                    existing.deleted_at = rt.deleted_at;
                }
            } else {
                self.tombstones.push(rt.clone());
            }
        }
    }

    /// Applies a [`FullMergeResult`] to local data using `remote` as the source
    /// of entity content. Uses the managers directly (not the tombstone-aware
    /// wrappers) so remote-driven deletes do not generate new tombstones.
    fn apply_simple_sync_result(
        &mut self,
        result: &FullMergeResult,
        remote: &FullSyncExport,
    ) -> SimpleSyncReport {
        let mut report = SimpleSyncReport::default();

        // Creates: groups first (so connections have their parent present),
        // then connections, templates, snippets.
        for action in &result.to_create {
            match action.entity_type {
                SyncEntityType::Group => {
                    if let Some(g) = remote.groups.iter().find(|g| g.id == action.id) {
                        match self.connection_manager.create_group_from(g.clone()) {
                            Ok(_) => report.created += 1,
                            Err(e) => {
                                tracing::warn!(id = %action.id, error = %e, "Simple Sync: create group failed");
                            }
                        }
                    }
                }
                SyncEntityType::Connection => {
                    if let Some(c) = remote.connections.iter().find(|c| c.id == action.id) {
                        match self.connection_manager.create_connection_from(c.clone()) {
                            Ok(_) => report.created += 1,
                            Err(e) => {
                                tracing::warn!(id = %action.id, error = %e, "Simple Sync: create connection failed");
                            }
                        }
                    }
                }
                SyncEntityType::Template => {
                    if let Some(t) = remote.templates.iter().find(|t| t.id == action.id) {
                        match self.template_manager.create_template(t.clone()) {
                            Ok(_) => report.created += 1,
                            Err(e) => {
                                tracing::warn!(id = %action.id, error = %e, "Simple Sync: create template failed");
                            }
                        }
                    }
                }
                SyncEntityType::Snippet => {
                    if let Some(s) = remote.snippets.iter().find(|s| s.id == action.id) {
                        match self.snippet_manager.create_snippet_from(s.clone()) {
                            Ok(_) => report.created += 1,
                            Err(e) => {
                                tracing::warn!(id = %action.id, error = %e, "Simple Sync: create snippet failed");
                            }
                        }
                    }
                }
                SyncEntityType::Cluster | SyncEntityType::Variable => {}
            }
        }

        // Updates: replace local entity with the newer remote version.
        for action in &result.to_update {
            match action.entity_type {
                SyncEntityType::Group => {
                    if let Some(g) = remote.groups.iter().find(|g| g.id == action.id) {
                        // The remote copy has local-only and Group-Sync fields
                        // stripped. Preserve this device's own values for them
                        // so a remote content edit never clobbers our SSH paths,
                        // UI state, or Master/Import configuration.
                        let mut merged = g.clone();
                        if let Some(local) = self.connection_manager.get_group(action.id) {
                            merged.expanded = local.expanded;
                            merged.sort_order = local.sort_order;
                            merged.ssh_key_path = local.ssh_key_path.clone();
                            merged.ssh_jump_host_id = local.ssh_jump_host_id;
                            merged.ssh_agent_socket = local.ssh_agent_socket.clone();
                            merged.sync_mode = local.sync_mode;
                            merged.sync_file = local.sync_file.clone();
                            merged.last_synced_at = local.last_synced_at;
                        }
                        match self.connection_manager.update_group(action.id, merged) {
                            Ok(()) => report.updated += 1,
                            Err(e) => {
                                tracing::warn!(id = %action.id, error = %e, "Simple Sync: update group failed");
                            }
                        }
                    }
                }
                SyncEntityType::Connection => {
                    if let Some(c) = remote.connections.iter().find(|c| c.id == action.id) {
                        match self
                            .connection_manager
                            .update_connection(action.id, c.clone())
                        {
                            Ok(()) => report.updated += 1,
                            Err(e) => {
                                tracing::warn!(id = %action.id, error = %e, "Simple Sync: update connection failed");
                            }
                        }
                    }
                }
                SyncEntityType::Template => {
                    if let Some(t) = remote.templates.iter().find(|t| t.id == action.id) {
                        match self.template_manager.update_template(action.id, t.clone()) {
                            Ok(()) => report.updated += 1,
                            Err(e) => {
                                tracing::warn!(id = %action.id, error = %e, "Simple Sync: update template failed");
                            }
                        }
                    }
                }
                SyncEntityType::Snippet => {
                    if let Some(s) = remote.snippets.iter().find(|s| s.id == action.id) {
                        match self.snippet_manager.update_snippet(action.id, s.clone()) {
                            Ok(()) => report.updated += 1,
                            Err(e) => {
                                tracing::warn!(id = %action.id, error = %e, "Simple Sync: update snippet failed");
                            }
                        }
                    }
                }
                SyncEntityType::Cluster | SyncEntityType::Variable => {}
            }
        }

        // Deletes: remote tombstone wins. Use managers directly so no new local
        // tombstone is created for a deletion that originated elsewhere.
        for action in &result.to_delete {
            let deleted = match action.entity_type {
                SyncEntityType::Connection => {
                    self.connection_manager.delete_connection(action.id).is_ok()
                }
                SyncEntityType::Group => self.connection_manager.delete_group(action.id).is_ok(),
                SyncEntityType::Template => {
                    self.template_manager.delete_template(action.id).is_ok()
                }
                SyncEntityType::Snippet => self.snippet_manager.delete_snippet(action.id).is_ok(),
                SyncEntityType::Cluster | SyncEntityType::Variable => false,
            };
            if deleted {
                report.deleted += 1;
            }
        }

        // Variables: name-keyed union. Variables carry no UUID or timestamp, so
        // the UUID merge engine cannot resolve them; instead we additively adopt
        // any non-secret remote variable we do not already have by name. Edits
        // and deletions of variables do not propagate (no per-variable clock).
        report.created += self.merge_remote_variables(remote);

        report
    }

    /// Adds non-secret remote variables not already present locally (by name).
    ///
    /// Returns the number of variables added; persists settings when it changed.
    /// Existing local variables — including a secret sharing a name — are never
    /// overwritten, as there is no per-variable timestamp to resolve conflicts.
    fn merge_remote_variables(&mut self, remote: &FullSyncExport) -> usize {
        let mut added = 0usize;
        for rv in &remote.variables {
            // Never adopt secret values from the shared file (defensive — the
            // exporter already strips them).
            if rv.is_secret {
                continue;
            }
            let exists = self
                .settings
                .global_variables
                .iter()
                .any(|v| v.name == rv.name);
            if !exists {
                self.settings.global_variables.push(rv.clone());
                added += 1;
            }
        }
        if added > 0
            && let Err(e) = self.save_settings()
        {
            tracing::warn!(error = %e, "Failed to persist variables after Simple Sync import");
        }
        added
    }

    // ========== Import Operations ==========

    /// Imports connections from an import result with automatic group creation
    ///
    /// Creates a parent group for the import source (e.g., "Remmina Import", "SSH Config Import")
    /// and organizes connections into subgroups based on their original grouping.
    pub fn import_connections_with_source(
        &mut self,
        result: &ImportResult,
        source_name: &str,
    ) -> Result<usize, String> {
        let mut imported = 0;

        // Create parent group for this import source
        // Use generate_unique_group_name to handle duplicate names
        let base_group_name = format!("{source_name} Import");
        let parent_group_name = self.generate_unique_group_name(&base_group_name);
        let parent_group_id = match self.connection_manager.create_group(parent_group_name) {
            Ok(id) => Some(id),
            Err(_) => {
                // Group might already exist, try to find it
                self.connection_manager
                    .list_groups()
                    .iter()
                    .find(|g| g.name == base_group_name)
                    .map(|g| g.id)
            }
        };

        // Create a map for subgroups - maps OLD group UUID to NEW group UUID
        let mut group_uuid_map: std::collections::HashMap<Uuid, Uuid> =
            std::collections::HashMap::new();
        // Also keep name-based map for Remmina groups
        let mut subgroup_map: std::collections::HashMap<String, Uuid> =
            std::collections::HashMap::new();

        // Import groups from result preserving hierarchy
        // First pass: identify root groups (no parent or parent not in import)
        let imported_group_ids: std::collections::HashSet<Uuid> =
            result.groups.iter().map(|g| g.id).collect();

        // Topological sort: process groups level by level so parents are always
        // created before their children. This prevents hierarchy flattening.
        // Track already-sorted group IDs here (not group_uuid_map, which is only
        // populated later in the creation loop and would be empty during the sort,
        // degrading the sort so deep hierarchies get appended in arbitrary order).
        let mut sorted_ids: std::collections::HashSet<Uuid> =
            std::collections::HashSet::with_capacity(result.groups.len());
        let mut sorted_groups: Vec<&ConnectionGroup> = Vec::with_capacity(result.groups.len());
        let mut remaining: Vec<&ConnectionGroup> = result.groups.iter().collect();
        while !remaining.is_empty() {
            let before_len = remaining.len();
            remaining.retain(|g| {
                let ready = if let Some(pid) = g.parent_id {
                    // Parent not in import → root-level, ready immediately
                    // Parent in import → ready only when parent already sorted
                    !imported_group_ids.contains(&pid) || sorted_ids.contains(&pid)
                } else {
                    true
                };
                if ready {
                    sorted_ids.insert(g.id);
                    sorted_groups.push(g);
                    false // remove from remaining
                } else {
                    true // keep in remaining
                }
            });
            // Safety: break if no progress (circular parent references)
            if remaining.len() == before_len {
                // Append remaining groups as root-level to avoid infinite loop
                for g in &remaining {
                    sorted_ids.insert(g.id);
                }
                sorted_groups.append(&mut remaining);
            }
        }

        // Create groups preserving hierarchy and all fields
        for group in &sorted_groups {
            // Determine the actual parent for this group
            let actual_parent_id = if let Some(orig_parent_id) = group.parent_id {
                // Check if original parent is in the import
                if let Some(&new_parent_id) = group_uuid_map.get(&orig_parent_id) {
                    // Parent was already created, use its new ID
                    Some(new_parent_id)
                } else {
                    // Parent not in import, use import root group
                    parent_group_id
                }
            } else {
                // Root group in import, make it child of import root
                parent_group_id
            };

            let new_group_id = if let Some(parent_id) = actual_parent_id {
                match self
                    .connection_manager
                    .create_group_with_parent(group.name.clone(), parent_id)
                {
                    Ok(id) => Some(id),
                    Err(_) => {
                        // Try to find existing
                        self.connection_manager
                            .get_child_groups(parent_id)
                            .iter()
                            .find(|g| g.name == group.name)
                            .map(|g| g.id)
                    }
                }
            } else {
                self.connection_manager
                    .create_group(group.name.clone())
                    .ok()
            };

            if let Some(new_id) = new_group_id {
                // Copy all fields from the imported group to the newly created one
                if let Some(existing) = self.connection_manager.get_group(new_id) {
                    let mut updated = existing.clone();
                    updated.icon = group.icon.clone();
                    updated.description = group.description.clone();
                    updated.username = group.username.clone();
                    updated.domain = group.domain.clone();
                    updated.password_source = group.password_source.clone();
                    updated.ssh_auth_method = group.ssh_auth_method.clone();
                    updated.ssh_key_path = group.ssh_key_path.clone();
                    updated.ssh_proxy_jump = group.ssh_proxy_jump.clone();
                    updated.ssh_agent_socket = group.ssh_agent_socket.clone();
                    updated.sort_order = group.sort_order;
                    updated.expect_rules = group.expect_rules.clone();
                    updated.post_login_scripts = group.post_login_scripts.clone();
                    updated.dynamic_folder = group.dynamic_folder.clone();
                    if let Err(e) = self.connection_manager.update_group(new_id, updated) {
                        tracing::warn!(group = %group.name, %e, "Failed to update imported group fields");
                    }
                }

                // Map old group UUID to new group UUID
                group_uuid_map.insert(group.id, new_id);
                subgroup_map.insert(group.name.clone(), new_id);
            }
        }

        // Import connections with automatic conflict resolution
        for conn in &result.connections {
            let mut connection = conn.clone();

            // Sanitize imported values — strip trailing escape sequences
            // (e.g. literal \n from Remmina INI files)
            connection.name = rustconn_core::import::sanitize_imported_value(&connection.name);
            connection.host = rustconn_core::import::sanitize_imported_value(&connection.host);
            if let Some(ref username) = connection.username {
                let clean = rustconn_core::import::sanitize_imported_value(username);
                connection.username = if clean.is_empty() { None } else { Some(clean) };
            }

            // Check for Remmina group tag (format: "remmina:group_name")
            let remmina_group = connection
                .tags
                .iter()
                .find(|t| t.starts_with("remmina:"))
                .map(|t| t.strip_prefix("remmina:").unwrap_or("").to_string());

            // Remove the remmina group tag from tags
            connection.tags.retain(|t| !t.starts_with("remmina:"));

            // Determine target group
            let target_group_id = if let Some(group_name) = remmina_group {
                // Create subgroup for Remmina group if not exists
                if !subgroup_map.contains_key(&group_name)
                    && let Some(parent_id) = parent_group_id
                    && let Ok(id) = self
                        .connection_manager
                        .create_group_with_parent(group_name.clone(), parent_id)
                {
                    subgroup_map.insert(group_name.clone(), id);
                }
                subgroup_map.get(&group_name).copied()
            } else if let Some(existing_group_id) = connection.group_id {
                // Connection has a group from import - map to new UUID
                group_uuid_map
                    .get(&existing_group_id)
                    .copied()
                    .or(parent_group_id)
            } else {
                // Use parent import group
                parent_group_id
            };

            // Set the group
            connection.group_id = target_group_id;

            // Auto-resolve name conflicts using protocol-aware naming
            if self.connection_exists_by_name(&connection.name) {
                connection.name =
                    self.generate_unique_connection_name(&connection.name, connection.protocol);
            }

            match self.connection_manager.create_connection_from(connection) {
                Ok(_) => imported += 1,
                Err(e) => tracing::warn!(name = %conn.name, %e, "Failed to import connection"),
            }
        }

        // Store imported credentials using synchronous secret-tool calls.
        // We avoid the async LibSecretBackend here because block_on inside
        // the GTK main thread can deadlock with the D-Bus/GLib main loop
        // that secret-tool relies on.
        if result.has_credentials() {
            let mut stored = 0usize;
            let total = result.credentials.len();

            for (conn_id, creds) in &result.credentials {
                // Build the lookup key in the same "{name} ({protocol})" format
                // that resolve_from_keyring uses for retrieval
                let Some(conn) = self.connection_manager.get_connection(*conn_id) else {
                    tracing::warn!(
                        connection_id = %conn_id,
                        "Skipping credential store: connection not found"
                    );
                    continue;
                };
                let protocol = conn.protocol_config.protocol_type();
                let name = rustconn_core::import::sanitize_imported_value(
                    &conn.name.trim().replace('/', "-"),
                );
                let lookup_key = format!("{} ({})", name, protocol.as_str().to_lowercase());

                match Self::store_credential_sync(&lookup_key, creds) {
                    Ok(()) => {
                        stored += 1;
                        tracing::debug!(lookup_key, "Stored imported credential");
                    }
                    Err(e) => {
                        tracing::warn!(
                            lookup_key,
                            error = %e,
                            "Failed to store imported credential"
                        );
                    }
                }
            }

            if stored == total {
                tracing::info!("Stored {stored} imported credential(s)");
            } else {
                tracing::warn!("Stored {stored}/{total} imported credential(s)");
            }
        }

        // Import smart folders with remapped group IDs
        if !result.smart_folders.is_empty() {
            let mut settings = self.settings().clone();
            for sf in &result.smart_folders {
                let mut folder = sf.clone();
                // Generate new ID to avoid collisions
                folder.id = Uuid::new_v4();
                // Remap filter_group_id to the new group UUID
                if let Some(old_gid) = folder.filter_group_id {
                    folder.filter_group_id = group_uuid_map.get(&old_gid).copied();
                }
                // Skip duplicates by name
                if !settings.smart_folders.iter().any(|f| f.name == folder.name) {
                    settings.smart_folders.push(folder);
                }
            }
            let _ = self.update_settings(settings);
            tracing::info!(count = result.smart_folders.len(), "Imported smart folders");
        }

        Ok(imported)
    }

    /// Stores a single credential field via synchronous `secret-tool store`.
    ///
    /// Uses `std::process::Command` instead of the async `LibSecretBackend`
    /// to avoid deadlocks when `block_on` is called on the GTK main thread
    /// (the D-Bus calls that `secret-tool` makes can re-enter the GLib main
    /// loop, which is blocked by the tokio runtime).
    fn store_secret_tool_sync(
        lookup_key: &str,
        key: &str,
        value: &str,
        label: &str,
    ) -> Result<(), String> {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let mut child = Command::new("secret-tool")
            .args([
                "store",
                "--label",
                label,
                "application",
                "rustconn",
                "connection_id",
                lookup_key,
                "key",
                key,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn secret-tool: {e}"))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(value.as_bytes())
                .map_err(|e| format!("Failed to write secret: {e}"))?;
        }
        // stdin is closed here (dropped), signalling EOF to secret-tool

        let output = child
            .wait_with_output()
            .map_err(|e| format!("Failed to wait for secret-tool: {e}"))?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("secret-tool store failed: {stderr}"))
        }
    }

    /// Stores credentials for an imported connection using synchronous I/O.
    fn store_credential_sync(
        lookup_key: &str,
        creds: &rustconn_core::models::Credentials,
    ) -> Result<(), String> {
        let label = format!("RustConn: {lookup_key}");

        if let Some(username) = &creds.username {
            Self::store_secret_tool_sync(lookup_key, "username", username, &label)?;
        }

        if let Some(password) = creds.expose_password() {
            Self::store_secret_tool_sync(lookup_key, "password", password, &label)?;
        }

        if let Some(passphrase) = creds.expose_key_passphrase() {
            Self::store_secret_tool_sync(lookup_key, "key_passphrase", passphrase, &label)?;
        }

        if let Some(domain) = &creds.domain {
            Self::store_secret_tool_sync(lookup_key, "domain", domain, &label)?;
        }

        Ok(())
    }

    // ========== Document Operations ==========
}
