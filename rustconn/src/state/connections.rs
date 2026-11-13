//! Connection and group CRUD operations, sorting, reordering.
//!
//! Extracted from `state.rs` as part of ARCH-5 decomposition.

use rustconn_core::error::ConfigResult;
use rustconn_core::models::{Connection, ConnectionGroup};
use uuid::Uuid;

use super::AppState;
use crate::async_utils::with_runtime;
use crate::vault_ops::{
    delete_group_vault_credential, delete_trashed_vault_credential,
    migrate_vault_entries_on_group_change, rename_vault_credential_for_move,
};

impl AppState {
    /// Creates a new connection
    ///
    /// If a connection with the same name already exists, automatically generates
    /// a unique name by appending the protocol suffix (e.g., "server (RDP)").
    pub fn create_connection(&mut self, mut connection: Connection) -> Result<Uuid, String> {
        // Auto-generate unique name if duplicate exists (Bug 4 fix)
        if self.connection_exists_by_name(&connection.name) {
            let protocol_type = connection.protocol_config.protocol_type();
            connection.name = self.generate_unique_connection_name(&connection.name, protocol_type);
        }

        self.connection_manager
            .create_connection_from(connection)
            .map_err(|e| format!("Failed to create connection: {e}"))
            .inspect(|_| self.mark_simple_sync_dirty())
    }

    /// Checks if a connection with the given name exists
    pub fn connection_exists_by_name(&self, name: &str) -> bool {
        self.connection_manager
            .list_connections()
            .iter()
            .any(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// Checks if a sibling group with the given name exists under the same parent.
    ///
    /// Two groups are siblings when they share the same `parent_id`.
    /// `exclude_id` allows ignoring a specific group (useful during rename).
    pub fn sibling_group_exists(
        &self,
        name: &str,
        parent_id: Option<Uuid>,
        exclude_id: Option<Uuid>,
    ) -> bool {
        self.connection_manager
            .sibling_group_name_exists(name, parent_id, exclude_id)
    }

    /// Generates a unique name by appending a protocol suffix and/or number if needed
    ///
    /// Uses the `ConnectionManager::generate_unique_name` method which follows the pattern:
    /// 1. If base name is unique, return it as-is
    /// 2. If duplicate, append protocol suffix (e.g., "server (RDP)")
    /// 3. If still duplicate, append numeric suffix (e.g., "server (RDP) 2")
    pub fn generate_unique_connection_name(
        &self,
        base_name: &str,
        protocol: rustconn_core::ProtocolType,
    ) -> String {
        self.connection_manager
            .generate_unique_name(base_name, protocol)
    }

    /// Restores a deleted connection from trash.
    ///
    /// Vault credentials are intentionally preserved during soft-delete (trash),
    /// so restoring a connection makes its credentials accessible again without
    /// any additional work. Credentials are only cleaned up during permanent
    /// deletion via [`empty_trash`](Self::empty_trash).
    pub fn restore_connection(&mut self, id: Uuid) -> ConfigResult<()> {
        self.connection_manager.restore_connection(id)
    }

    /// Restores a deleted group
    pub fn restore_group(&mut self, id: Uuid) -> ConfigResult<()> {
        self.connection_manager.restore_group(id)
    }

    /// Deletes vault credentials for trashed items on a background thread.
    ///
    /// Vault deletion spawns `keepassxc-cli` or makes D-Bus round-trips, so it
    /// must not run on the GTK main thread. Failures are logged only: the trash
    /// entry is going away regardless, and a stale vault entry is recoverable by
    /// hand while a blocked purge is not.
    ///
    /// `on_done` runs on the main thread once the worker has finished, or at
    /// once when there is nothing to delete.
    ///
    /// `keepers` are the connections whose credential must survive: a doomed
    /// connection that resolves to the same vault key as one of them keeps its
    /// entry (see [`crate::vault_ops::live_connection_sharing_vault_key`]).
    fn spawn_trashed_credential_cleanup(
        &self,
        connections: &[Connection],
        groups: &[ConnectionGroup],
        keepers: Vec<Connection>,
        on_done: impl FnOnce() + 'static,
    ) {
        use rustconn_core::models::PasswordSource;

        let vault_conns: Vec<Connection> = connections
            .iter()
            .filter(|c| c.password_source == PasswordSource::Vault)
            .cloned()
            .collect();
        let vault_groups: Vec<ConnectionGroup> = groups
            .iter()
            .filter(|g| g.password_source.as_ref() == Some(&PasswordSource::Vault))
            .cloned()
            .collect();

        if vault_conns.is_empty() && vault_groups.is_empty() {
            on_done();
            return;
        }

        // The hierarchy used to rebuild lookup keys has to include the trashed
        // groups: a cascade delete moves the parent group into the trash too, and
        // resolving the path against live groups alone would yield an empty group
        // path and therefore the wrong key.
        let mut hierarchy = self.connection_manager.list_groups_owned();
        hierarchy.extend(
            self.connection_manager
                .list_trash_groups()
                .into_iter()
                .cloned(),
        );
        let settings = self.settings.clone();

        crate::utils::spawn_blocking_with_callback(
            move || {
                for conn in &vault_conns {
                    if let Err(e) =
                        delete_trashed_vault_credential(&settings, &hierarchy, conn, &keepers)
                    {
                        tracing::warn!(
                            connection_name = %conn.name,
                            error = %e,
                            "Failed to clean up vault credential on permanent delete"
                        );
                    }
                }
                for group in &vault_groups {
                    if let Err(e) = delete_group_vault_credential(&settings, &hierarchy, group) {
                        tracing::warn!(
                            group_name = %group.name,
                            error = %e,
                            "Failed to clean up group vault credential on permanent delete"
                        );
                    }
                }
            },
            move |()| on_done(),
        );
    }

    /// Permanently empties the trash and deletes its vault credentials.
    ///
    /// Connections and groups with `PasswordSource::Vault` have their
    /// credentials deleted from the configured backend on a worker thread, so
    /// the window stays responsive; `on_credentials_done` runs on the main
    /// thread when that ends. Credential cleanup failures are logged but
    /// do not prevent the trash from being emptied.
    ///
    /// Also removes webkit session data directories for Web connections
    /// (`~/.local/share/rustconn/webkit/<uuid>/` and `~/.cache/rustconn/webkit/<uuid>/`).
    pub fn empty_trash(
        &mut self,
        on_credentials_done: impl FnOnce() + 'static,
    ) -> ConfigResult<()> {
        let trashed_conns: Vec<Connection> = self
            .connection_manager
            .list_trash_connections()
            .into_iter()
            .cloned()
            .collect();
        let trashed_groups: Vec<ConnectionGroup> = self
            .connection_manager
            .list_trash_groups()
            .into_iter()
            .cloned()
            .collect();
        let trashed_ids: Vec<uuid::Uuid> = trashed_conns.iter().map(|c| c.id).collect();

        // Each vault deletion spawns `keepassxc-cli` or waits on D-Bus (about
        // 0.7 s apiece with a KeePass file), so a full trash used to freeze the
        // window for the sum of them. The cleanup runs on a worker thread and
        // `on_credentials_done` runs on the main thread when it ends. The lists
        // and the group hierarchy are copied here, before the entries leave the
        // trash.
        //
        // A live connection can resolve to the same key as a trashed one — Group
        // Sync leaves exactly that pair behind — and its entry must survive.
        let live = self.connection_manager.list_connections_owned();
        self.spawn_trashed_credential_cleanup(
            &trashed_conns,
            &trashed_groups,
            live,
            on_credentials_done,
        );

        remove_webkit_session_dirs(&trashed_ids);

        let result = self.connection_manager.empty_trash();
        if result.is_ok() {
            for id in trashed_ids {
                self.workspace_on_connection_deleted(id);
            }
        }
        result
    }

    /// Permanently removes one entry from the trash and deletes its vault
    /// credential, the single-item counterpart of
    /// [`empty_trash`](Self::empty_trash).
    ///
    /// Only that entry goes: a trashed group's trashed connections stay in the
    /// trash. The credential is kept when a live connection — or another
    /// connection still in the trash, which can be restored — resolves to the
    /// same vault key ([`single_purge_credential_keepers`]). The deletion runs on
    /// a worker thread; `on_credentials_done` runs on the main thread when it
    /// ends, or at once when the entry is not in the trash.
    ///
    /// Returns `true` when the entry was removed, `false` when it was no longer
    /// in the trash (restored or purged meanwhile).
    ///
    /// # Errors
    ///
    /// Returns an error if persisting the trash fails.
    pub fn purge_trash_item(
        &mut self,
        kind: crate::sidebar::trash::TrashKind,
        id: Uuid,
        on_credentials_done: impl FnOnce() + 'static,
    ) -> ConfigResult<bool> {
        use crate::sidebar::trash::TrashKind;

        match kind {
            TrashKind::Connection => {
                let Some(conn) = self.connection_manager.get_trash_connection(id).cloned() else {
                    on_credentials_done();
                    return Ok(false);
                };
                let keepers = single_purge_credential_keepers(
                    self.connection_manager.list_connections_owned(),
                    &self.connection_manager.list_trash_connections(),
                    id,
                );
                self.spawn_trashed_credential_cleanup(
                    std::slice::from_ref(&conn),
                    &[],
                    keepers,
                    on_credentials_done,
                );
                remove_webkit_session_dirs(&[id]);
                let removed = self.connection_manager.purge_trash_connection(id)?;
                if removed {
                    self.workspace_on_connection_deleted(id);
                }
                Ok(removed)
            }
            TrashKind::Group => {
                let Some(group) = self.connection_manager.get_trash_group(id).cloned() else {
                    on_credentials_done();
                    return Ok(false);
                };
                self.spawn_trashed_credential_cleanup(
                    &[],
                    std::slice::from_ref(&group),
                    Vec::new(),
                    on_credentials_done,
                );
                self.connection_manager.purge_trash_group(id)
            }
        }
    }

    /// Generates a unique group name among siblings by appending a number if needed
    pub fn generate_unique_group_name(&self, base_name: &str, parent_id: Option<Uuid>) -> String {
        if !self.sibling_group_exists(base_name, parent_id, None) {
            return base_name.to_string();
        }

        let mut counter = 1;
        loop {
            let new_name = format!("{base_name} ({counter})");
            if !self.sibling_group_exists(&new_name, parent_id, None) {
                return new_name;
            }
            counter += 1;
        }
    }

    /// Updates an existing connection
    pub fn update_connection(&mut self, id: Uuid, connection: Connection) -> Result<(), String> {
        self.connection_manager
            .update_connection(id, connection)
            .map_err(|e| format!("Failed to update connection: {e}"))?;
        // An edit can change the password, its source or the user name; a
        // cached copy from before it must not be replayed by the next connect.
        self.password_cache.forget(id);
        self.mark_simple_sync_dirty();
        Ok(())
    }

    /// Toggles the pinned (favorite) state of a connection.
    ///
    /// Routes through the manager's dedicated pin path rather than
    /// `update_connection`, which preserves the old pin state and would discard
    /// the toggle. Pinning is local-only, so it does not mark sync dirty.
    ///
    /// # Errors
    ///
    /// Returns an error string if the connection does not exist or persistence
    /// fails.
    pub fn toggle_pin(&mut self, id: Uuid) -> Result<(), String> {
        self.connection_manager
            .toggle_pin(id)
            .map_err(|e| format!("Failed to toggle pin state: {e}"))
    }

    /// Soft-deletes a connection (moves to trash).
    ///
    /// Vault credentials are intentionally kept so that
    /// [`restore_connection`](Self::restore_connection) works without
    /// re-entering passwords. Credentials are cleaned up only when
    /// [`empty_trash`](Self::empty_trash) permanently removes the connection.
    pub fn delete_connection(&mut self, id: Uuid) -> Result<(), String> {
        self.connection_manager
            .delete_connection(id)
            .map_err(|e| format!("Failed to delete connection: {e}"))?;
        self.password_cache.forget(id);
        // Simple Sync: record the deletion so it propagates to other devices.
        self.record_tombstone(rustconn_core::sync::SyncEntityType::Connection, id);
        Ok(())
    }

    /// Gets a connection by ID
    pub fn get_connection(&self, id: Uuid) -> Option<&Connection> {
        self.connection_manager.get_connection(id)
    }

    /// Finds a connection by name (case-insensitive)
    ///
    /// Returns the first match. Used by CLI `--connect <name>` resolution.
    pub fn find_connection_by_name(&self, name: &str) -> Option<&Connection> {
        let lower = name.to_lowercase();
        self.connection_manager
            .list_connections()
            .into_iter()
            .find(|c| c.name.to_lowercase() == lower)
    }

    /// Lists all connections
    pub fn list_connections(&self) -> Vec<&Connection> {
        self.connection_manager.list_connections()
    }

    /// Returns an owned copy of all connections in a single allocation
    pub fn list_connections_owned(&self) -> Vec<Connection> {
        self.connection_manager.list_connections_owned()
    }

    /// Everything in the trash, for the sidebar's Trash row.
    #[must_use]
    pub fn trash_entries(&self) -> Vec<crate::sidebar::trash::TrashEntry> {
        use crate::sidebar::trash::{TrashEntry, TrashKind, sort_entries};
        let mut entries: Vec<TrashEntry> = self
            .connection_manager
            .list_trash_groups()
            .into_iter()
            .map(|g| TrashEntry {
                kind: TrashKind::Group,
                id: g.id,
                name: g.name.clone(),
                protocol: String::new(),
                host: String::new(),
            })
            .collect();
        entries.extend(
            self.connection_manager
                .list_trash_connections()
                .into_iter()
                .map(|c| TrashEntry {
                    kind: TrashKind::Connection,
                    id: c.id,
                    name: c.name.clone(),
                    protocol: crate::window::types::get_protocol_string(&c.protocol_config),
                    host: c.host.clone(),
                }),
        );
        sort_entries(&mut entries);
        entries
    }

    /// Gets connections by group
    pub fn get_connections_by_group(&self, group_id: Uuid) -> Vec<&Connection> {
        self.connection_manager.get_by_group(group_id)
    }

    /// Gets ungrouped connections
    pub fn get_ungrouped_connections(&self) -> Vec<&Connection> {
        self.connection_manager.get_ungrouped()
    }

    // ========== Group Operations ==========

    /// Creates a new group
    pub fn create_group(&mut self, name: String) -> Result<Uuid, String> {
        // Check for duplicate name among root-level siblings
        if self.sibling_group_exists(&name, None, None) {
            return Err(format!("Group with name '{name}' already exists"));
        }

        self.connection_manager
            .create_group(name)
            .map_err(|e| format!("Failed to create group: {e}"))
            .inspect(|_| self.mark_simple_sync_dirty())
    }

    /// Creates a group with a parent
    pub fn create_group_with_parent(
        &mut self,
        name: String,
        parent_id: Uuid,
    ) -> Result<Uuid, String> {
        // Check for duplicate name among siblings under the same parent
        if self.sibling_group_exists(&name, Some(parent_id), None) {
            return Err(format!(
                "Group with name '{name}' already exists in this folder"
            ));
        }

        self.connection_manager
            .create_group_with_parent(name, parent_id)
            .map_err(|e| format!("Failed to create group: {e}"))
            .inspect(|_| self.mark_simple_sync_dirty())
    }

    /// Updates a group's content and marks Simple Sync dirty.
    ///
    /// Bumps the group's `updated_at` (via [`ConnectionGroup::touch`]) so the
    /// edit wins the Simple Sync merge on other devices. Use this for genuine
    /// user edits; sync bookkeeping must call `ConnectionManager::update_group`
    /// directly to avoid churning the merge clock.
    ///
    /// # Errors
    ///
    /// Returns an error string if validation or persistence fails.
    pub fn update_group(
        &mut self,
        id: Uuid,
        mut group: rustconn_core::models::ConnectionGroup,
    ) -> Result<(), String> {
        group.touch();
        self.connection_manager
            .update_group(id, group)
            .map_err(|e| format!("Failed to update group: {e}"))?;
        self.mark_simple_sync_dirty();
        Ok(())
    }

    /// Deletes a group (connections become ungrouped)
    pub fn delete_group(&mut self, id: Uuid) -> Result<(), String> {
        self.connection_manager
            .delete_group(id)
            .map_err(|e| format!("Failed to delete group: {e}"))?;
        // Simple Sync: only the group is removed (connections survive ungrouped),
        // so a single group tombstone is enough.
        self.record_tombstone(rustconn_core::sync::SyncEntityType::Group, id);
        Ok(())
    }

    /// Deletes a group and all connections within it (cascade delete)
    pub fn delete_group_cascade(&mut self, id: Uuid) -> Result<(), String> {
        use rustconn_core::sync::SyncEntityType;

        // Snapshot IDs before the cascade so we can tombstone everything it
        // removes (the group, its descendant groups, and their connections).
        // Only needed when Simple Sync is on.
        let track = self.settings.sync.simple_sync_enabled;
        let (before_conns, before_groups) = if track {
            (
                self.connection_manager
                    .list_connections()
                    .iter()
                    .map(|c| c.id)
                    .collect::<std::collections::HashSet<Uuid>>(),
                self.connection_manager
                    .list_groups()
                    .iter()
                    .map(|g| g.id)
                    .collect::<std::collections::HashSet<Uuid>>(),
            )
        } else {
            (
                std::collections::HashSet::new(),
                std::collections::HashSet::new(),
            )
        };

        self.connection_manager
            .delete_group_cascade(id)
            .map_err(|e| format!("Failed to delete group: {e}"))?;

        if track {
            let after_conns: std::collections::HashSet<Uuid> = self
                .connection_manager
                .list_connections()
                .iter()
                .map(|c| c.id)
                .collect();
            let after_groups: std::collections::HashSet<Uuid> = self
                .connection_manager
                .list_groups()
                .iter()
                .map(|g| g.id)
                .collect();

            let removed = before_conns
                .difference(&after_conns)
                .map(|id| (SyncEntityType::Connection, *id))
                .chain(
                    before_groups
                        .difference(&after_groups)
                        .map(|id| (SyncEntityType::Group, *id)),
                )
                .collect::<Vec<_>>();
            self.record_tombstones(removed);
        }

        Ok(())
    }

    /// Moves a group to a new parent group
    ///
    /// When the group uses KeePass backend, vault entries for the group and all
    /// its descendant connections are automatically migrated to the new paths.
    pub fn move_group_to_parent(
        &mut self,
        group_id: Uuid,
        new_parent_id: Option<Uuid>,
    ) -> Result<(), String> {
        // Capture old groups snapshot before the move for vault migration
        let old_parent_id = self.get_group(group_id).map(|g| g.parent_id);
        let parent_changed = old_parent_id.is_some_and(|old| old != new_parent_id);

        let old_groups_snapshot: Vec<rustconn_core::models::ConnectionGroup> = if parent_changed {
            self.list_groups_owned()
        } else {
            Vec::new()
        };

        self.connection_manager
            .move_group(group_id, new_parent_id)
            .map_err(|e| format!("Failed to move group: {e}"))?;
        // Reparenting bumps the group's `updated_at` in the manager; flag a
        // Simple Sync re-export so the move propagates.
        self.mark_simple_sync_dirty();

        // Migrate vault entries if parent changed (KeePass paths affected)
        if parent_changed {
            let new_groups: Vec<_> = self.list_groups_owned();
            let connections: Vec<_> = self.list_connections_owned();
            let settings = self.settings.clone();
            migrate_vault_entries_on_group_change(
                &settings,
                &old_groups_snapshot,
                &new_groups,
                &connections,
                group_id,
            );
        }

        Ok(())
    }

    /// Counts connections in a group (including child groups)
    pub fn count_connections_in_group(&self, group_id: Uuid) -> usize {
        self.connection_manager.count_connections_in_group(group_id)
    }

    /// Gets a group by ID
    pub fn get_group(&self, id: Uuid) -> Option<&ConnectionGroup> {
        self.connection_manager.get_group(id)
    }

    /// Lists all groups
    pub fn list_groups(&self) -> Vec<&ConnectionGroup> {
        self.connection_manager.list_groups()
    }

    /// Returns an owned copy of all groups in a single allocation
    pub fn list_groups_owned(&self) -> Vec<ConnectionGroup> {
        self.connection_manager.list_groups_owned()
    }

    /// Gets root-level groups
    pub fn get_root_groups(&self) -> Vec<&ConnectionGroup> {
        self.connection_manager.get_root_groups()
    }

    /// Gets child groups
    pub fn get_child_groups(&self, parent_id: Uuid) -> Vec<&ConnectionGroup> {
        self.connection_manager.get_child_groups(parent_id)
    }

    /// Moves a connection to a group
    ///
    /// When the connection uses `PasswordSource::Vault` with a KeePass backend,
    /// the vault entry is automatically renamed to match the new group hierarchy.
    ///
    /// NOTE: Only connections with `PasswordSource::Vault` trigger credential
    /// migration. Connections with `PasswordSource::None` that happen to have
    /// legacy credentials in a backend will not have those entries migrated.
    /// This is acceptable because `PasswordSource::None` means the user has
    /// not explicitly configured vault storage for this connection.
    pub fn move_connection_to_group(
        &mut self,
        connection_id: Uuid,
        group_id: Option<Uuid>,
    ) -> Result<(), String> {
        // Capture old group_id and entry path before the move (for vault credential migration)
        let old_conn = self
            .connection_manager
            .get_connection(connection_id)
            .cloned();

        self.connection_manager
            .move_connection_to_group(connection_id, group_id)
            .map_err(|e| format!("Failed to move connection: {e}"))?;
        // The manager bumps the connection's `updated_at`; flag a Simple Sync
        // re-export so the regrouping propagates.
        self.mark_simple_sync_dirty();

        // Migrate vault credential if the group changed and password source is Vault
        if let Some(old_conn) = old_conn
            && old_conn.group_id != group_id
            && old_conn.password_source == rustconn_core::models::PasswordSource::Vault
        {
            let new_conn = self
                .connection_manager
                .get_connection(connection_id)
                .cloned();
            if let Some(new_conn) = new_conn {
                let groups: Vec<rustconn_core::models::ConnectionGroup> = self
                    .connection_manager
                    .list_groups()
                    .iter()
                    .cloned()
                    .cloned()
                    .collect();
                let settings = self.settings.clone();
                let protocol_str = old_conn
                    .protocol_config
                    .protocol_type()
                    .as_str()
                    .to_lowercase();

                // Spawn background task to rename the vault entry
                crate::utils::spawn_blocking_with_callback(
                    move || {
                        rename_vault_credential_for_move(
                            &settings,
                            &groups,
                            &old_conn,
                            &new_conn,
                            &protocol_str,
                        )
                    },
                    |result| {
                        if let Err(e) = result {
                            tracing::error!(error = %e, "Failed to migrate vault credential after group move");
                        }
                    },
                );
            }
        }

        Ok(())
    }

    /// Gets the group path
    pub fn get_group_path(&self, group_id: Uuid) -> Option<String> {
        self.connection_manager.get_group_path(group_id)
    }

    /// Sorts connections within a specific group alphabetically
    pub fn sort_group(&mut self, group_id: Uuid) -> Result<(), String> {
        self.connection_manager
            .sort_group(group_id)
            .map_err(|e| format!("Failed to sort group: {e}"))
    }

    /// Sorts all groups and connections alphabetically
    pub fn sort_all(&mut self) -> Result<(), String> {
        self.connection_manager
            .sort_all()
            .map_err(|e| format!("Failed to sort all: {e}"))
    }

    /// Reorders a connection to be positioned after another connection
    pub fn reorder_connection(
        &mut self,
        connection_id: Uuid,
        target_id: Uuid,
    ) -> Result<(), String> {
        self.connection_manager
            .reorder_connection(connection_id, target_id)
            .map_err(|e| format!("Failed to reorder connection: {e}"))
    }

    /// Reorders a group to be positioned after another group
    pub fn reorder_group(&mut self, group_id: Uuid, target_id: Uuid) -> Result<(), String> {
        self.connection_manager
            .reorder_group(group_id, target_id)
            .map_err(|e| format!("Failed to reorder group: {e}"))
    }

    /// Updates the `last_connected` timestamp for a connection
    pub fn update_last_connected(&mut self, connection_id: Uuid) -> Result<(), String> {
        self.connection_manager
            .update_last_connected(connection_id)
            .map_err(|e| format!("Failed to update last connected: {e}"))
    }

    /// Updates the persisted zoom level for a Web connection.
    ///
    /// Called by the debounced zoom-change handler in the embedded web widget.
    /// Only updates when the connection uses `ProtocolConfig::Web`.
    pub fn update_web_zoom_level(&mut self, connection_id: Uuid, zoom_level: f64) {
        use rustconn_core::models::ProtocolConfig;

        let Some(conn) = self.connection_manager.get_connection(connection_id) else {
            tracing::debug!(%connection_id, "Connection not found for zoom persist");
            return;
        };

        let mut updated = conn.clone();
        if let ProtocolConfig::Web(ref mut web_config) = updated.protocol_config {
            web_config.zoom_level = zoom_level.clamp(0.3, 3.0);
        } else {
            return;
        }

        if let Err(e) = self
            .connection_manager
            .update_connection(connection_id, updated)
        {
            tracing::debug!(%connection_id, %e, "Failed to persist zoom level");
        }
    }

    /// Sorts all connections by `last_connected` timestamp (most recent first)
    pub fn sort_by_recent(&mut self) -> Result<(), String> {
        self.connection_manager
            .sort_by_recent()
            .map_err(|e| format!("Failed to sort by recent: {e}"))
    }

    /// Flushes any pending persistence operations immediately
    ///
    /// This ensures that debounced saves are written to disk before application exit.
    /// Includes a 5-second timeout to prevent hanging on shutdown if the
    /// persistence layer is unresponsive.
    pub fn flush_persistence(&self) -> Result<(), String> {
        // History saves are debounced; write any pending changes before exit.
        self.flush_history_if_dirty();
        with_runtime(|rt| {
            rt.block_on(async {
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    self.connection_manager.flush_persistence(),
                )
                .await
                .map_err(|_| "Flush persistence timed out after 5s".to_string())?
                .map_err(|e| format!("Failed to flush persistence: {e}"))
            })
        })?
    }

    // ========== Session Operations ==========
}

/// The connections whose vault credential must survive when the trashed
/// connection `purged` alone is deleted for good.
///
/// Emptying the whole trash only has to spare live connections. Deleting one
/// entry must also spare the connections left in the trash: they can still be
/// restored, and one that shares the purged entry's name, group and protocol
/// resolves to the same vault entry.
fn single_purge_credential_keepers(
    mut live: Vec<Connection>,
    trashed: &[&Connection],
    purged: Uuid,
) -> Vec<Connection> {
    live.extend(
        trashed
            .iter()
            .filter(|c| c.id != purged)
            .map(|c| (*c).clone()),
    );
    live
}

/// Removes the webkit session data and cache directories of connections that
/// are deleted for good, on a background thread.
#[cfg(feature = "web-embedded")]
fn remove_webkit_session_dirs(connection_ids: &[Uuid]) {
    let dirs_to_remove: Vec<(std::path::PathBuf, std::path::PathBuf)> = connection_ids
        .iter()
        .map(|conn_id| {
            (
                crate::embedded_web::session_data_dir(conn_id),
                crate::embedded_web::session_cache_dir(conn_id),
            )
        })
        .collect();
    // Move directory removal off the main thread to avoid blocking UI
    std::thread::spawn(move || {
        for (data_dir, cache_dir) in dirs_to_remove {
            if data_dir.exists()
                && let Err(e) = std::fs::remove_dir_all(&data_dir)
            {
                tracing::warn!(
                    path = %data_dir.display(),
                    error = %e,
                    "Failed to remove webkit session data directory"
                );
            }
            if cache_dir.exists()
                && let Err(e) = std::fs::remove_dir_all(&cache_dir)
            {
                tracing::warn!(
                    path = %cache_dir.display(),
                    error = %e,
                    "Failed to remove webkit session cache directory"
                );
            }
        }
    });
}

/// Without the embedded browser there are no webkit session directories.
#[cfg(not(feature = "web-embedded"))]
const fn remove_webkit_session_dirs(_connection_ids: &[Uuid]) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh(name: &str) -> Connection {
        Connection::new(
            name.to_string(),
            "10.0.0.1".to_string(),
            22,
            rustconn_core::models::ProtocolConfig::Ssh(rustconn_core::models::SshConfig::default()),
        )
    }

    #[test]
    fn single_purge_spares_live_and_the_rest_of_the_trash() {
        let live = ssh("live");
        let purged = ssh("web");
        // Same name, group and protocol as `purged`: the same vault entry.
        let trashed_twin = ssh("web");
        let trash = [&purged, &trashed_twin];

        let keepers = single_purge_credential_keepers(vec![live.clone()], &trash, purged.id);
        let ids: Vec<Uuid> = keepers.iter().map(|c| c.id).collect();

        assert_eq!(ids, vec![live.id, trashed_twin.id]);
    }

    #[test]
    fn single_purge_never_keeps_the_purged_connection_itself() {
        let purged = ssh("web");
        let keepers = single_purge_credential_keepers(Vec::new(), &[&purged], purged.id);
        assert!(keepers.is_empty());
    }
}
