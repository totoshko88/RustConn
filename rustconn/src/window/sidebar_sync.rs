//! Sidebar loading, filtering and selection helpers

use super::*;

impl MainWindow {
    /// Loads connections into the sidebar
    pub(super) fn load_connections(&self) {
        let expanded_groups = self.state.borrow().expanded_groups().clone();

        // Use sorted rebuild to ensure alphabetical order by default
        sorting::rebuild_sidebar_sorted(&self.state, &self.sidebar);

        // Apply expanded state after populating
        self.sidebar.apply_expanded_groups(&expanded_groups);
    }

    /// Updates the password vault button status in the sidebar based on current settings
    pub(super) fn update_keepass_button_status(&self) {
        let state_ref = self.state.borrow();
        let settings = state_ref.settings();
        let backend = settings.secrets.preferred_backend;

        // For libsecret, Bitwarden, 1Password, Pass, and Passbolt, always enabled (no database file needed)
        // For KeePassXC/KdbxFile, check if enabled and database exists
        let (enabled, database_exists) = match backend {
            rustconn_core::config::SecretBackendType::LibSecret
            | rustconn_core::config::SecretBackendType::MacOsKeychain
            | rustconn_core::config::SecretBackendType::Bitwarden
            | rustconn_core::config::SecretBackendType::OnePassword
            | rustconn_core::config::SecretBackendType::Passbolt
            | rustconn_core::config::SecretBackendType::Pass
            | rustconn_core::config::SecretBackendType::ProtonPass
            | rustconn_core::config::SecretBackendType::EncryptedFile => (true, true),
            // Configured but unusable while locked — see the matching arm in
            // `app::refresh_sidebar_secret_status`. The backend's own
            // `is_available()` is its unlock state, so hardcoding "ready" here
            // made the indicator disagree with the resolution chain.
            rustconn_core::config::SecretBackendType::PortableEncryptedFile => {
                (true, state_ref.portable_store_unlocked())
            }
            rustconn_core::config::SecretBackendType::KeePassXc
            | rustconn_core::config::SecretBackendType::KdbxFile => {
                let kdbx_enabled = settings.secrets.kdbx_enabled;
                let db_exists = settings
                    .secrets
                    .kdbx_path
                    .as_ref()
                    .is_some_and(|p| p.exists());
                (kdbx_enabled, db_exists)
            }
        };
        drop(state_ref);

        self.sidebar.update_keepass_status(enabled, database_exists);
    }

    /// Public method to refresh KeePass button status (called after settings change)
    #[expect(
        dead_code,
        reason = "Part of KeePass integration API, called from settings dialog"
    )]
    pub fn refresh_keepass_status(&self) {
        self.update_keepass_button_status();
    }

    /// Filters connections based on search query
    pub(super) fn filter_connections(state: &SharedAppState, sidebar: &SharedSidebar, query: &str) {
        if query.is_empty() {
            // Restore full hierarchy when search is cleared
            Self::reload_sidebar(state, sidebar);
            // Restore the tree state that was saved before search started
            sidebar.restore_pre_search_state();
            return;
        }

        // Save tree state before first search keystroke
        sidebar.save_pre_search_state();

        let store = sidebar.store();
        store.remove_all();

        let state_ref = state.borrow();

        // Get connections and groups for search. `list_connections()` already
        // returns borrowed `&Connection`s, so the search runs on references —
        // no per-keystroke deep clone of every connection.
        let connections: Vec<&rustconn_core::models::Connection> = state_ref.list_connections();
        let groups = state_ref.list_groups();

        // Protocol filter syntax: a single name (protocol:rdp, proto:ssh, p:vnc)
        // or an OR-list (protocols:ssh,mosh). Both go straight to a direct
        // filter without scoring.
        let protocol_filter: Option<Vec<&str>> = query
            .strip_prefix("protocol:")
            .or_else(|| query.strip_prefix("proto:"))
            .or_else(|| query.strip_prefix("p:"))
            .map(|name| vec![name])
            .or_else(|| {
                query
                    .strip_prefix("protocols:")
                    .map(|names| names.split(',').collect())
            });

        if let Some(protocol_names) = protocol_filter {
            for conn in &connections {
                let protocol = get_protocol_string(&conn.protocol_config);
                if !types::protocol_matches_filter(&protocol, &protocol_names) {
                    continue;
                }
                let item = sorting::build_connection_item(sidebar, conn, false);
                store.append(&item);
            }
        } else {
            // Use cached search engine from sidebar (avoids per-keystroke allocation
            // and provides result caching with TTL for repeated queries)
            let search_engine = sidebar.search_engine();
            let parsed_query = match rustconn_core::search::SearchEngine::parse_query(query) {
                Ok(q) => q,
                Err(_) => {
                    // Fall back to simple text search on parse error
                    rustconn_core::search::SearchQuery::with_text(query)
                }
            };

            // Use search_debounced which checks/populates the result cache.
            // GTK-level debouncing (sidebar.search_debouncer) has already passed,
            // so the internal debouncer will proceed — giving us automatic caching
            // for repeated queries (e.g. type-delete-retype same text).
            let results = search_engine
                .search_debounced(&parsed_query, &connections, &groups)
                .unwrap_or_default();

            // Index by id once so result lookup is O(1) instead of O(n) per hit.
            let conn_by_id: std::collections::HashMap<_, _> =
                connections.iter().map(|c| (c.id, *c)).collect();

            // Display results sorted by relevance
            for result in results {
                if let Some(conn) = conn_by_id.get(&result.connection_id) {
                    // Same builder as the tree rows, so the context menu sees the
                    // real state. `result.score` already orders the rows; the name
                    // stays untouched so Rename and Delete never see a prefix.
                    let item = sorting::build_connection_item(sidebar, conn, false);
                    store.append(&item);
                }
            }
        }
    }

    /// Edits the selected connection or group
    pub(super) fn edit_selected_connection(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
        key_setup: &Rc<key_setup::KeySetupContext>,
    ) {
        edit_dialogs::edit_selected_connection(window.upcast_ref(), state, sidebar, key_setup);
    }

    /// Renames the selected connection or group
    pub(super) fn rename_selected_item(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
        notebook: &SharedNotebook,
    ) {
        edit_dialogs::rename_selected_item(window.upcast_ref(), state, sidebar, notebook);
    }

    /// Deletes the selected connection or group
    pub(super) fn delete_selected_connection(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        operations::delete_selected_connection(window.upcast_ref(), state, sidebar);
    }

    /// Deletes all selected connections (bulk delete for group operations mode)
    pub(super) fn delete_selected_connections(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        operations::delete_selected_connections(window.upcast_ref(), state, sidebar);
    }

    /// Shows dialog to move selected items to a group
    pub(super) fn show_move_selected_to_group_dialog(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        operations::show_move_selected_to_group_dialog(window.upcast_ref(), state, sidebar);
    }

    /// Duplicates the selected connection
    pub(super) fn duplicate_selected_connection(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        operations::duplicate_selected_connection(window.upcast_ref(), state, sidebar);
    }

    /// Toggles pin state of the selected connection
    pub(super) fn toggle_pin_selected(state: &SharedAppState, sidebar: &SharedSidebar) {
        operations::toggle_pin_selected(state, sidebar);
    }

    /// Copies the selected connection to the internal clipboard
    pub(super) fn copy_selected_connection(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        operations::copy_selected_connection(window.upcast_ref(), state, sidebar);
    }

    /// Pastes a connection from the internal clipboard
    pub(super) fn paste_connection(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        operations::paste_connection(window.upcast_ref(), state, sidebar);
    }

    /// Reloads the sidebar with current data (preserving hierarchy)
    pub(super) fn reload_sidebar(state: &SharedAppState, sidebar: &SharedSidebar) {
        sidebar.invalidate_search_cache();
        sorting::rebuild_sidebar_sorted(state, sidebar);
        // Every connection save, import and sync ends in a sidebar reload, so
        // this is where an edited macro list reaches the registered accels.
        macro_dispatch::request_refresh(sidebar.widget());
    }

    /// Reloads the sidebar while preserving tree state
    ///
    /// This method saves the current expanded groups, scroll position, and selection,
    /// reloads the sidebar data, and then restores the state. Use this when editing
    /// connections to maintain the user's view.
    pub fn reload_sidebar_preserving_state(state: &SharedAppState, sidebar: &SharedSidebar) {
        sidebar.invalidate_search_cache();
        // Save current tree state
        let tree_state = sidebar.save_state();

        // Perform the reload
        Self::reload_sidebar(state, sidebar);

        // Restore tree state
        sidebar.restore_state(&tree_state);
    }

    /// Toggles group operations mode for multi-select
    pub(super) fn toggle_group_operations_mode(sidebar: &SharedSidebar, enabled: bool) {
        sorting::toggle_group_operations_mode(sidebar, enabled);
    }

    /// Sorts connections alphabetically and updates `sort_order`
    pub(super) fn sort_connections(state: &SharedAppState, sidebar: &SharedSidebar) {
        sorting::sort_connections(state, sidebar);
    }

    /// Sorts connections by recent usage (most recently used first)
    pub(super) fn sort_recent(state: &SharedAppState, sidebar: &SharedSidebar) {
        sorting::sort_recent(state, sidebar);
    }

    /// Handles drag-drop operations for reordering connections
    pub(super) fn handle_drag_drop(state: &SharedAppState, sidebar: &SharedSidebar, data: &str) {
        sorting::handle_drag_drop(state, sidebar, data);
    }
}
