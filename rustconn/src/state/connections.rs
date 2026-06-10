//! Connection and group CRUD operations, sorting, reordering.
//!
//! Extracted from `state.rs` as part of ARCH-5 decomposition.

use crate::async_utils::with_runtime;
use crate::vault_ops::{
    delete_group_vault_credential, delete_vault_credential, migrate_vault_entries_on_group_change,
    rename_vault_credential_for_move,
};
use rustconn_core::error::ConfigResult;
use rustconn_core::models::{Connection, ConnectionGroup};
use uuid::Uuid;

use super::AppState;

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
    }

    /// Checks if a connection with the given name exists
    pub fn connection_exists_by_name(&self, name: &str) -> bool {
        self.connection_manager
            .list_connections()
            .iter()
            .any(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// Checks if a group with the given name exists
    pub fn group_exists_by_name(&self, name: &str) -> bool {
        self.connection_manager
            .list_groups()
            .iter()
            .any(|g| g.name.eq_ignore_ascii_case(name))
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

    /// Permanently empties the trash, cleaning up vault credentials first.
    ///
    /// Connections and groups with `PasswordSource::Vault` have their
    /// credentials deleted from the configured backend before the trash
    /// entries are removed. Credential cleanup failures are logged but
    /// do not prevent the trash from being emptied.
    pub fn empty_trash(&mut self) -> ConfigResult<()> {
        use rustconn_core::models::PasswordSource;

        let settings = self.settings.clone();
        let groups: Vec<rustconn_core::models::ConnectionGroup> = self
            .connection_manager
            .list_groups()
            .into_iter()
            .cloned()
            .collect();

        // Collect vault connections from trash for credential cleanup
        let vault_connections: Vec<rustconn_core::models::Connection> = self
            .connection_manager
            .list_trash_connections()
            .into_iter()
            .filter(|c| c.password_source == PasswordSource::Vault)
            .cloned()
            .collect();

        // Collect vault groups from trash for credential cleanup
        let vault_groups: Vec<rustconn_core::models::ConnectionGroup> = self
            .connection_manager
            .list_trash_groups()
            .into_iter()
            .filter(|g| g.password_source.as_ref() == Some(&PasswordSource::Vault))
            .cloned()
            .collect();

        // Clean up credentials (best-effort, log failures)
        for conn in &vault_connections {
            if let Err(e) = delete_vault_credential(&settings, &groups, conn) {
                tracing::warn!(
                    connection_name = %conn.name,
                    error = %e,
                    "Failed to clean up vault credential on permanent delete"
                );
            }
        }
        for group in &vault_groups {
            if let Err(e) = delete_group_vault_credential(&settings, &groups, group) {
                tracing::warn!(
                    group_name = %group.name,
                    error = %e,
                    "Failed to clean up group vault credential on permanent delete"
                );
            }
        }

        self.connection_manager.empty_trash()
    }

    /// Generates a unique group name by appending a number if needed
    pub fn generate_unique_group_name(&self, base_name: &str) -> String {
        if !self.group_exists_by_name(base_name) {
            return base_name.to_string();
        }

        let mut counter = 1;
        loop {
            let new_name = format!("{base_name} ({counter})");
            if !self.group_exists_by_name(&new_name) {
                return new_name;
            }
            counter += 1;
        }
    }

    /// Updates an existing connection
    pub fn update_connection(&mut self, id: Uuid, connection: Connection) -> Result<(), String> {
        self.connection_manager
            .update_connection(id, connection)
            .map_err(|e| format!("Failed to update connection: {e}"))
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
            .map_err(|e| format!("Failed to delete connection: {e}"))
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
        // Check for duplicate name
        if self.group_exists_by_name(&name) {
            return Err(format!("Group with name '{name}' already exists"));
        }

        self.connection_manager
            .create_group(name)
            .map_err(|e| format!("Failed to create group: {e}"))
    }

    /// Creates a group with a parent
    pub fn create_group_with_parent(
        &mut self,
        name: String,
        parent_id: Uuid,
    ) -> Result<Uuid, String> {
        self.connection_manager
            .create_group_with_parent(name, parent_id)
            .map_err(|e| format!("Failed to create group: {e}"))
    }

    /// Deletes a group (connections become ungrouped)
    pub fn delete_group(&mut self, id: Uuid) -> Result<(), String> {
        self.connection_manager
            .delete_group(id)
            .map_err(|e| format!("Failed to delete group: {e}"))
    }

    /// Deletes a group and all connections within it (cascade delete)
    pub fn delete_group_cascade(&mut self, id: Uuid) -> Result<(), String> {
        self.connection_manager
            .delete_group_cascade(id)
            .map_err(|e| format!("Failed to delete group: {e}"))
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
            self.list_groups().into_iter().cloned().collect()
        } else {
            Vec::new()
        };

        self.connection_manager
            .move_group(group_id, new_parent_id)
            .map_err(|e| format!("Failed to move group: {e}"))?;

        // Migrate vault entries if parent changed (KeePass paths affected)
        if parent_changed {
            let new_groups: Vec<_> = self.list_groups().into_iter().cloned().collect();
            let connections: Vec<_> = self.list_connections().into_iter().cloned().collect();
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
