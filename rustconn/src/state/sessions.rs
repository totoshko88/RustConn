//! Session management, snippets, clusters, templates, history, statistics, clipboard.
//!
//! Extracted from `state.rs` as part of ARCH-5 decomposition.

use std::collections::HashMap;

use rustconn_core::cluster::Cluster;
use rustconn_core::models::{Connection, ConnectionHistoryEntry, ConnectionStatistics, Snippet};
use rustconn_core::session::Session;
use uuid::Uuid;

use super::AppState;
use crate::vault_ops::copy_vault_credential;

impl AppState {
    /// Terminates a session
    pub fn terminate_session(&mut self, session_id: Uuid) -> Result<(), String> {
        self.session_manager
            .terminate_session(session_id)
            .map_err(|e| format!("Failed to terminate session: {e}"))
    }

    /// Gets active sessions
    pub fn active_sessions(&self) -> Vec<&Session> {
        self.session_manager.active_sessions()
    }

    // ========== Snippet Operations ==========

    /// Creates a new snippet
    pub fn create_snippet(&mut self, snippet: Snippet) -> Result<Uuid, String> {
        self.snippet_manager
            .create_snippet_from(snippet)
            .map_err(|e| format!("Failed to create snippet: {e}"))
            .inspect(|_| self.mark_simple_sync_dirty())
    }

    /// Updates a snippet
    pub fn update_snippet(&mut self, id: Uuid, snippet: Snippet) -> Result<(), String> {
        self.snippet_manager
            .update_snippet(id, snippet)
            .map_err(|e| format!("Failed to update snippet: {e}"))?;
        self.mark_simple_sync_dirty();
        Ok(())
    }

    /// Deletes a snippet
    pub fn delete_snippet(&mut self, id: Uuid) -> Result<(), String> {
        self.snippet_manager
            .delete_snippet(id)
            .map_err(|e| format!("Failed to delete snippet: {e}"))?;
        self.record_tombstone(rustconn_core::sync::SyncEntityType::Snippet, id);
        Ok(())
    }

    /// Gets a snippet by ID
    pub fn get_snippet(&self, id: Uuid) -> Option<&Snippet> {
        self.snippet_manager.get_snippet(id)
    }

    /// Lists all snippets
    pub fn list_snippets(&self) -> Vec<&Snippet> {
        self.snippet_manager.list_snippets()
    }

    /// Searches snippets
    pub fn search_snippets(&self, query: &str) -> Vec<&Snippet> {
        self.snippet_manager.search(query)
    }

    // ========== Secret/Credential Operations ==========

    /// Creates a new cluster
    pub fn create_cluster(&mut self, cluster: Cluster) -> Result<Uuid, String> {
        let id = cluster.id;
        self.cluster_manager.add_cluster(cluster);
        self.save_clusters()?;
        Ok(id)
    }

    /// Updates an existing cluster
    pub fn update_cluster(&mut self, cluster: Cluster) -> Result<(), String> {
        self.cluster_manager
            .update_cluster(cluster.id, cluster)
            .map_err(|e| format!("Failed to update cluster: {e}"))?;
        self.save_clusters()
    }

    /// Deletes a cluster
    pub fn delete_cluster(&mut self, cluster_id: Uuid) -> Result<(), String> {
        self.cluster_manager.remove_cluster(cluster_id);
        self.save_clusters()
    }

    /// Gets a cluster by ID
    pub fn get_cluster(&self, cluster_id: Uuid) -> Option<&Cluster> {
        self.cluster_manager.get_cluster(cluster_id)
    }

    /// Gets all clusters
    pub fn get_all_clusters(&self) -> Vec<&Cluster> {
        self.cluster_manager.get_all_clusters()
    }

    /// Starts a cluster session for tracking connection states
    pub fn start_cluster_session(&mut self, cluster_id: Uuid) -> Result<(), String> {
        self.cluster_manager
            .start_session(cluster_id)
            .map(|_| ())
            .map_err(|e| format!("Failed to start cluster session: {e}"))
    }

    /// Ends a cluster session
    pub fn end_cluster_session(&mut self, cluster_id: Uuid) {
        self.cluster_manager.end_session(cluster_id);
    }

    /// Saves clusters to disk
    fn save_clusters(&self) -> Result<(), String> {
        let clusters = self.cluster_manager.clusters_to_vec();
        self.config_manager
            .save_clusters(&clusters)
            .map_err(|e| format!("Failed to save clusters: {e}"))
    }

    // ========== Workspace Profile Operations ==========

    /// Creates a new workspace profile
    pub fn create_workspace_profile(
        &mut self,
        profile: rustconn_core::models::WorkspaceProfile,
    ) -> Result<Uuid, String> {
        self.workspace_manager
            .create(profile)
            .map_err(|e| format!("Failed to create workspace profile: {e}"))
    }

    /// Updates an existing workspace profile
    pub fn update_workspace_profile(
        &mut self,
        id: Uuid,
        profile: rustconn_core::models::WorkspaceProfile,
    ) -> Result<(), String> {
        self.workspace_manager
            .update(id, profile)
            .map_err(|e| format!("Failed to update workspace profile: {e}"))
    }

    /// Deletes a workspace profile
    pub fn delete_workspace_profile(&mut self, id: Uuid) -> Result<(), String> {
        self.workspace_manager
            .delete(id)
            .map_err(|e| format!("Failed to delete workspace profile: {e}"))
    }

    /// Renames a workspace profile
    pub fn rename_workspace_profile(&mut self, id: Uuid, new_name: String) -> Result<(), String> {
        self.workspace_manager
            .rename(id, new_name)
            .map_err(|e| format!("Failed to rename workspace profile: {e}"))
    }

    /// Gets a workspace profile by ID
    pub fn get_workspace_profile(
        &self,
        id: Uuid,
    ) -> Option<&rustconn_core::models::WorkspaceProfile> {
        self.workspace_manager.get(id)
    }

    /// Lists all workspace profiles (sorted)
    pub fn list_workspace_profiles(&self) -> Vec<&rustconn_core::models::WorkspaceProfile> {
        self.workspace_manager.list()
    }

    /// Notifies workspace manager that a connection was deleted
    pub fn workspace_on_connection_deleted(&mut self, connection_id: Uuid) {
        if let Err(e) = self.workspace_manager.on_connection_deleted(connection_id) {
            tracing::warn!("Failed to update workspaces after connection deletion: {e}");
        }
    }

    // ========== Template Operations ==========

    /// Adds a template and persists via `TemplateManager`
    pub fn add_template(
        &mut self,
        template: rustconn_core::ConnectionTemplate,
    ) -> Result<(), String> {
        // Add to active document if one exists
        if let Some(doc_id) = self.active_document_id
            && let Some(doc) = self.document_manager.get_mut(doc_id)
        {
            doc.add_template(template.clone());
        }

        // Persist via template manager
        self.template_manager
            .create_template(template)
            .map(|_| ())
            .map_err(|e| format!("Failed to add template: {e}"))
            .inspect(|()| self.mark_simple_sync_dirty())
    }

    /// Updates a template and persists via `TemplateManager`
    pub fn update_template(
        &mut self,
        template: rustconn_core::ConnectionTemplate,
    ) -> Result<(), String> {
        let id = template.id;

        // Update in active document if one exists
        if let Some(doc_id) = self.active_document_id
            && let Some(doc) = self.document_manager.get_mut(doc_id)
        {
            doc.remove_template(id);
            doc.add_template(template.clone());
        }

        // Persist via template manager (create if not found, update if exists)
        let result = if self.template_manager.get_template(id).is_some() {
            self.template_manager
                .update_template(id, template)
                .map_err(|e| format!("Failed to update template: {e}"))
        } else {
            self.template_manager
                .create_template(template)
                .map(|_| ())
                .map_err(|e| format!("Failed to add template: {e}"))
        };
        result.inspect(|()| self.mark_simple_sync_dirty())
    }

    /// Deletes a template and persists via `TemplateManager`
    pub fn delete_template(&mut self, template_id: uuid::Uuid) -> Result<(), String> {
        // Remove from active document if one exists
        if let Some(doc_id) = self.active_document_id
            && let Some(doc) = self.document_manager.get_mut(doc_id)
        {
            doc.remove_template(template_id);
        }

        // Remove via template manager (ignore not-found — may only exist in document)
        if self.template_manager.get_template(template_id).is_some() {
            self.template_manager
                .delete_template(template_id)
                .map_err(|e| format!("Failed to delete template: {e}"))?;
            self.record_tombstone(rustconn_core::sync::SyncEntityType::Template, template_id);
            Ok(())
        } else {
            Ok(())
        }
    }

    /// Gets all templates (from `TemplateManager` and active document)
    pub fn get_all_templates(&self) -> Vec<rustconn_core::ConnectionTemplate> {
        let mut templates: Vec<rustconn_core::ConnectionTemplate> = self
            .template_manager
            .list_templates()
            .into_iter()
            .cloned()
            .collect();

        // Also include templates from active document
        if let Some(doc) = self.active_document() {
            for doc_template in &doc.templates {
                if !templates.iter().any(|t| t.id == doc_template.id) {
                    templates.push(doc_template.clone());
                }
            }
        }

        templates
    }

    // ========== Connection History Operations ==========

    /// Gets all history entries
    #[must_use]
    pub fn history_entries(&self) -> &[ConnectionHistoryEntry] {
        &self.history_entries
    }

    /// Adds a new history entry for a connection start
    pub fn record_connection_start(
        &mut self,
        connection: &Connection,
        username: Option<&str>,
    ) -> Uuid {
        let entry = ConnectionHistoryEntry::new(
            connection.id,
            connection.name.clone(),
            connection.host.clone(),
            connection.port,
            format!("{:?}", connection.protocol).to_lowercase(),
            username.map(String::from),
        );
        let entry_id = entry.id;
        self.history_entries.push(entry);
        self.trim_history();
        self.mark_history_dirty();
        entry_id
    }

    /// Marks a history entry as ended (successful)
    pub fn record_connection_end(&mut self, entry_id: Uuid) {
        if let Some(entry) = self.history_entries.iter_mut().find(|e| e.id == entry_id) {
            entry.end();
            self.mark_history_dirty();
        }
    }

    /// Marks a history entry as failed
    pub fn record_connection_failed(&mut self, entry_id: Uuid, error: &str) {
        if let Some(entry) = self.history_entries.iter_mut().find(|e| e.id == entry_id) {
            entry.fail(error);
            self.mark_history_dirty();
        }
    }

    /// Records a connection attempt that failed before a session was created
    /// (e.g. a pre-connection port check that timed out).
    ///
    /// Creates a history entry and immediately marks it failed with `error`, so
    /// the attempt is visible in the History dialog instead of being lost.
    pub fn record_connection_attempt_failed(
        &mut self,
        connection: &Connection,
        username: Option<&str>,
        error: &str,
    ) {
        let entry_id = self.record_connection_start(connection, username);
        self.record_connection_failed(entry_id, error);
    }

    /// Installs the sender that wakes the debounced history flusher in
    /// `app.rs`. Until installed, history changes are saved immediately.
    pub fn set_history_dirty_sender(&mut self, sender: async_channel::Sender<()>) {
        self.history_dirty_tx = Some(sender);
    }

    /// Marks history as needing a save.
    ///
    /// With a flusher installed this only wakes it (the actual disk write is
    /// debounced and runs off the main thread); without one it saves
    /// immediately, preserving the old behavior for tests.
    fn mark_history_dirty(&self) {
        self.history_dirty.set(true);
        if let Some(tx) = &self.history_dirty_tx {
            let _ = tx.try_send(());
        } else {
            let _ = self.save_history();
            self.history_dirty.set(false);
        }
    }

    /// Synchronously saves history if it has unsaved changes (shutdown path)
    pub(crate) fn flush_history_if_dirty(&self) {
        if self.history_dirty.get() {
            if let Err(e) = self.save_history() {
                tracing::error!(%e, "Failed to flush connection history");
            }
            self.history_dirty.set(false);
        }
    }

    /// Takes a snapshot for an off-main-thread save and clears the dirty
    /// flag. Returns `None` when there is nothing to save.
    pub(crate) fn take_history_snapshot_if_dirty(
        &self,
    ) -> Option<(
        rustconn_core::config::ConfigManager,
        Vec<ConnectionHistoryEntry>,
    )> {
        if self.history_dirty.get() {
            self.history_dirty.set(false);
            Some((self.config_manager.clone(), self.history_entries.clone()))
        } else {
            None
        }
    }

    /// Gets statistics for all connections
    #[must_use]
    pub fn get_all_statistics(&self) -> Vec<(String, ConnectionStatistics, String)> {
        let mut stats_map: HashMap<Uuid, (String, ConnectionStatistics, String)> = HashMap::new();

        for entry in &self.history_entries {
            let stat_entry = stats_map.entry(entry.connection_id).or_insert_with(|| {
                (
                    entry.connection_name.clone(),
                    ConnectionStatistics::new(entry.connection_id),
                    entry.protocol.clone(),
                )
            });
            stat_entry.1.update_from_entry(entry);
        }

        stats_map.into_values().collect()
    }

    /// Clears all connection statistics by clearing history
    pub fn clear_all_statistics(&mut self) {
        self.history_entries.clear();
        if let Err(e) = self.save_history() {
            tracing::error!("Failed to save cleared history: {e}");
        }
    }

    /// Trims history to max entries and retention period
    fn trim_history(&mut self) {
        let max_entries = self.settings.history.max_entries;
        let retention_days = self.settings.history.retention_days;

        // Remove old entries
        let cutoff = chrono::Utc::now() - chrono::Duration::days(i64::from(retention_days));
        self.history_entries.retain(|e| e.started_at > cutoff);

        // Trim to max entries (keep most recent)
        if self.history_entries.len() > max_entries {
            self.history_entries
                .sort_by_key(|b| std::cmp::Reverse(b.started_at));
            self.history_entries.truncate(max_entries);
        }
    }

    /// Saves history to disk
    fn save_history(&self) -> Result<(), String> {
        self.config_manager
            .save_history(&self.history_entries)
            .map_err(|e| format!("Failed to save history: {e}"))
    }

    // ========== Clipboard Operations ==========

    /// Copies a connection to the clipboard
    ///
    /// # Arguments
    /// * `connection_id` - The ID of the connection to copy
    ///
    /// # Returns
    /// `Ok(())` if the connection was copied, `Err` if not found
    pub fn copy_connection(&mut self, connection_id: Uuid) -> Result<(), String> {
        let connection = self
            .get_connection(connection_id)
            .ok_or_else(|| format!("Connection not found: {connection_id}"))?
            .clone();
        let group_id = connection.group_id;
        self.clipboard.copy(&connection, group_id);
        Ok(())
    }

    /// Pastes a connection from the clipboard
    ///
    /// Creates a duplicate connection with a new ID and "(Copy)" suffix.
    /// The connection is added to the same group as the original.
    /// If the original had `PasswordSource::Vault`, credentials are copied
    /// to the new connection's key in the background.
    ///
    /// # Returns
    /// `Ok(Uuid)` with the new connection's ID, or `Err` if clipboard is empty
    pub fn paste_connection(&mut self) -> Result<Uuid, String> {
        let new_conn = self
            .clipboard
            .paste()
            .ok_or_else(|| "Clipboard is empty".to_string())?;

        // Capture original connection info for credential copy
        let original_conn = self.clipboard.original_connection().cloned();

        // Get the source group from clipboard
        let target_group = self.clipboard.source_group();

        // Create the connection with the target group
        let mut conn_with_group = new_conn;
        conn_with_group.group_id = target_group;

        // Generate unique name if needed using protocol-aware naming
        if self.connection_exists_by_name(&conn_with_group.name) {
            conn_with_group.name = self
                .generate_unique_connection_name(&conn_with_group.name, conn_with_group.protocol);
        }

        // Copy vault credentials from original to new connection
        if let Some(ref orig) = original_conn
            && orig.password_source == rustconn_core::models::PasswordSource::Vault
        {
            let settings = self.settings.clone();
            let groups: Vec<rustconn_core::models::ConnectionGroup> = self
                .connection_manager
                .list_groups()
                .into_iter()
                .cloned()
                .collect();
            let old_conn = orig.clone();
            let new_conn_copy = conn_with_group.clone();
            crate::utils::spawn_blocking_with_callback(
                move || copy_vault_credential(&settings, &groups, &old_conn, &new_conn_copy),
                |result| {
                    if let Err(e) = result {
                        tracing::warn!(error = %e, "Failed to copy vault credential on paste");
                    }
                },
            );
        }

        self.connection_manager
            .create_connection_from(conn_with_group)
            .map_err(|e| format!("Failed to paste connection: {e}"))
    }

    /// Checks if the clipboard has content
    #[must_use]
    pub const fn has_clipboard_content(&self) -> bool {
        self.clipboard.has_content()
    }
}
