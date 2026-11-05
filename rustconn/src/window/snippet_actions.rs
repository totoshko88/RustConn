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

        // Execute snippet action — opens the picker for the focused session.
        //
        // Deliberately absent from the Tools menu, and it should stay absent. The
        // picker sends to whatever terminal currently has focus, resolved through
        // `send_text_to_focused` and the active tab's split bridge. That target is
        // only unambiguous while the user is looking at it. The app menu hangs off
        // the main window, so activating from there against a detached session
        // window — or against a split whose focused pane changed while the menu was
        // open — sends the command to a session the user did not mean. A snippet is
        // not an idempotent request; delivering it to the wrong host is the whole
        // class of accident issue #315 exists to reduce.
        //
        // The reachable routes are all ones where the target is chosen in the same
        // gesture. This action itself is reached from the terminal's own
        // right-click menu, as the "Execute Snippet…" entry that replaces the
        // inline list once there are more than five snippets — see
        // `terminal/config.rs`. Alongside it: `run-snippet-direct` for the inline
        // entries, `run-snippet-for-connection` from the connection context menu,
        // and the snippets manager, which checks for an active terminal first.
        //
        // Not the command palette, which is worth stating because the palette
        // hangs off the main window the same way the app menu does and would carry
        // the same ambiguity.
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
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = terminal_notebook.clone();
        let bridges_clone = self.session_split_bridges.clone();
        run_snippet_direct_action.connect_activate(move |_, param| {
            if let Some(win) = window_weak.upgrade()
                && let Some(param) = param
                && let Some(id_str) = param.get::<String>()
                && let Ok(id) = Uuid::parse_str(&id_str)
            {
                let state_ref = state_clone.borrow();
                if let Some(snippet) = state_ref.get_snippet(id).cloned() {
                    drop(state_ref);
                    crate::window::snippets::execute_snippet_direct(
                        win.upcast_ref::<gtk4::Window>(),
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

        // Save Selection as Snippet… (from the terminal right-click menu, W5).
        // Reads the active terminal's current selection and opens the snippet
        // editor in create mode with the command pre-filled. The user names and
        // trims it before saving, so a selection that happens to contain a
        // secret is never written to the snippet store without their action.
        let save_selection_snippet_action = gio::SimpleAction::new("save-selection-snippet", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = terminal_notebook.clone();
        save_selection_snippet_action.connect_activate(move |_, _| {
            let Some(win) = window_weak.upgrade() else {
                return;
            };
            let selection = notebook_clone.get_active_terminal().and_then(|term| {
                term.has_selection()
                    .then(|| term.text_selected(vte4::Format::Text))
                    .flatten()
                    .map(|s| s.to_string())
            });
            let Some(selection) = selection.filter(|s| !s.trim().is_empty()) else {
                crate::toast::show_toast_on_window(
                    &win,
                    &crate::i18n::i18n("Select some text in the terminal first"),
                    crate::toast::ToastType::Info,
                );
                return;
            };
            let dialog = crate::dialogs::SnippetDialog::new(Some(win.upcast_ref()));
            dialog.set_command(&selection);
            let state_inner = state_clone.clone();
            let notebook_inner = notebook_clone.clone();
            dialog.run(move |result| {
                if let Some(snippet) = result
                    && let Ok(mut state_mut) = state_inner.try_borrow_mut()
                {
                    if let Err(e) = state_mut.create_snippet(snippet) {
                        tracing::warn!(?e, "Failed to create snippet from selection");
                    }
                    drop(state_mut);
                    notebook_inner.rebuild_snippet_menu(&state_inner);
                }
            });
        });
        window.add_action(&save_selection_snippet_action);

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
        let split_view_clone = self.split_view.clone();
        let activity_clone = self.activity_coordinator.clone();
        manage_workspaces_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                super::workspaces::show_workspace_manager(
                    win.upcast_ref(),
                    state_clone.clone(),
                    notebook_clone.clone(),
                    sidebar_clone.clone(),
                    monitoring_clone.clone(),
                    bridges_clone.clone(),
                    split_view_clone.clone(),
                    activity_clone.clone(),
                );
            }
        });
        window.add_action(&manage_workspaces_action);
    }
}
