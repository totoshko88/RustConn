//! Snippet, cluster, and template window actions
//!
//! Extracted from `window/mod.rs` to reduce module complexity.

use super::*;

impl MainWindow {
    pub(crate) fn setup_snippet_actions(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        terminal_notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
    ) {
        // New snippet action
        let new_snippet_action = gio::SimpleAction::new("new-snippet", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let toast_clone = self.toast_overlay.clone();
        let notebook_clone = terminal_notebook.clone();
        new_snippet_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                snippets::show_new_snippet_dialog(
                    win.upcast_ref(),
                    state_clone.clone(),
                    toast_clone.clone(),
                    notebook_clone.clone(),
                );
            }
        });
        window.add_action(&new_snippet_action);

        // Manage snippets action
        let manage_snippets_action = gio::SimpleAction::new("manage-snippets", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = terminal_notebook.clone();
        let bridges_clone = self.session_split_bridges.clone();
        manage_snippets_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                snippets::show_snippets_manager(
                    win.upcast_ref(),
                    state_clone.clone(),
                    notebook_clone.clone(),
                    bridges_clone.clone(),
                );
            }
        });
        window.add_action(&manage_snippets_action);

        // Execute snippet action
        let execute_snippet_action = gio::SimpleAction::new("execute-snippet", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = terminal_notebook.clone();
        let bridges_clone = self.session_split_bridges.clone();
        execute_snippet_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                snippets::show_snippet_picker(
                    win.upcast_ref(),
                    state_clone.clone(),
                    notebook_clone.clone(),
                    bridges_clone.clone(),
                );
            }
        });
        window.add_action(&execute_snippet_action);

        // Run snippet directly by ID (from inline context menu items)
        let run_snippet_direct_action =
            gio::SimpleAction::new("run-snippet-direct", Some(glib::VariantTy::STRING));
        let state_clone = state.clone();
        let notebook_clone = terminal_notebook.clone();
        let bridges_clone = self.session_split_bridges.clone();
        run_snippet_direct_action.connect_activate(move |_, param| {
            if let Some(param) = param
                && let Some(id_str) = param.get::<String>()
                && let Ok(id) = Uuid::parse_str(&id_str)
            {
                let state_ref = state_clone.borrow();
                if let Some(snippet) = state_ref.get_snippet(id).cloned() {
                    drop(state_ref);
                    crate::window::snippets::execute_snippet_direct(
                        &notebook_clone,
                        &bridges_clone,
                        &snippet,
                        &state_clone,
                    );
                }
            }
        });
        window.add_action(&run_snippet_direct_action);

        // Run snippet for selected connection (from context menu)
        // First connects to the selected connection, then shows snippet picker
        let run_snippet_for_conn_action =
            gio::SimpleAction::new("run-snippet-for-connection", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = terminal_notebook.clone();
        let sidebar_clone = sidebar.clone();
        let split_view_clone = self.split_view.clone();
        let monitoring_clone = self.monitoring.clone();
        let activity_clone = self.activity_coordinator.clone();
        let bridges_clone = self.session_split_bridges.clone();
        run_snippet_for_conn_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                // Get selected connection from sidebar
                if let Some(item) = sidebar_clone.get_selected_item() {
                    if item.is_group() {
                        return; // Can't run snippet on a group
                    }

                    // Parse UUID from item id string
                    let id_str = item.id();
                    let Ok(id) = Uuid::parse_str(&id_str) else {
                        return;
                    };

                    // Check if connection is already connected (has active session)
                    let has_active_session = notebook_clone
                        .get_all_sessions()
                        .iter()
                        .any(|s| s.connection_id == id);

                    if has_active_session {
                        // Already connected, just show snippet picker
                        snippets::show_snippet_picker(
                            win.upcast_ref(),
                            state_clone.clone(),
                            notebook_clone.clone(),
                            bridges_clone.clone(),
                        );
                    } else {
                        // Need to connect first, then show snippet picker
                        // Start connection
                        Self::start_connection_with_split(
                            &state_clone,
                            &notebook_clone,
                            &split_view_clone,
                            &sidebar_clone,
                            &monitoring_clone,
                            id,
                            Some(&activity_clone),
                        );

                        // Show snippet picker after a short delay to allow connection to establish
                        let win_for_timeout = win.clone();
                        let state_for_timeout = state_clone.clone();
                        let notebook_for_timeout = notebook_clone.clone();
                        let bridges_for_timeout = bridges_clone.clone();
                        glib::timeout_add_local_once(
                            std::time::Duration::from_millis(500),
                            move || {
                                snippets::show_snippet_picker(
                                    win_for_timeout.upcast_ref(),
                                    state_for_timeout,
                                    notebook_for_timeout,
                                    bridges_for_timeout,
                                );
                            },
                        );
                    }
                }
            }
        });
        window.add_action(&run_snippet_for_conn_action);

        // Show sessions action
        let show_sessions_action = gio::SimpleAction::new("show-sessions", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = terminal_notebook.clone();
        let sidebar_clone = sidebar.clone();
        show_sessions_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                sessions::show_sessions_manager(
                    win.upcast_ref(),
                    state_clone.clone(),
                    notebook_clone.clone(),
                    sidebar_clone.clone(),
                );
            }
        });
        window.add_action(&show_sessions_action);

        // Initialize snippet context menu with current snippets
        terminal_notebook.rebuild_snippet_menu(state);
    }

    /// Sets up cluster-related actions
    pub(crate) fn setup_cluster_actions(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        terminal_notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
    ) {
        // New cluster action
        let new_cluster_action = gio::SimpleAction::new("new-cluster", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = terminal_notebook.clone();
        let toast_clone = self.toast_overlay.clone();
        new_cluster_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                clusters::show_new_cluster_dialog(
                    win.upcast_ref(),
                    state_clone.clone(),
                    notebook_clone.clone(),
                    toast_clone.clone(),
                );
            }
        });
        window.add_action(&new_cluster_action);

        // Manage clusters action
        let manage_clusters_action = gio::SimpleAction::new("manage-clusters", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = terminal_notebook.clone();
        let sidebar_clone = sidebar.clone();
        let monitoring_clone = self.monitoring.clone();
        manage_clusters_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                clusters::show_clusters_manager(
                    win.upcast_ref(),
                    state_clone.clone(),
                    notebook_clone.clone(),
                    sidebar_clone.clone(),
                    monitoring_clone.clone(),
                );
            }
        });
        window.add_action(&manage_clusters_action);
    }

    /// Sets up template-related actions
    pub(crate) fn setup_template_actions(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        // Manage templates action
        let manage_templates_action = gio::SimpleAction::new("manage-templates", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        manage_templates_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                templates::show_templates_manager(
                    win.upcast_ref(),
                    state_clone.clone(),
                    sidebar_clone.clone(),
                );
            }
        });
        window.add_action(&manage_templates_action);
    }

    /// Sets up workspace profile management action
    pub(crate) fn setup_workspace_actions(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        terminal_notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
    ) {
        let manage_workspaces_action = gio::SimpleAction::new("manage-workspaces", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = terminal_notebook.clone();
        let sidebar_clone = sidebar.clone();
        let monitoring_clone = self.monitoring.clone();
        let bridges_clone = self.session_split_bridges.clone();
        manage_workspaces_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                super::workspaces::show_workspace_manager(
                    win.upcast_ref(),
                    state_clone.clone(),
                    notebook_clone.clone(),
                    sidebar_clone.clone(),
                    monitoring_clone.clone(),
                    bridges_clone.clone(),
                );
            }
        });
        window.add_action(&manage_workspaces_action);
    }
}
