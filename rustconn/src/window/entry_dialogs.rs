//! Entry points for the import, export, palette and quick-connect dialogs

use super::*;

impl MainWindow {
    /// Shows the new connection dialog with optional template selection
    pub(super) fn show_new_connection_dialog(
        window: &adw::ApplicationWindow,
        state: SharedAppState,
        sidebar: SharedSidebar,
    ) {
        connection_dialogs::show_new_connection_dialog(window.upcast_ref(), state, sidebar);
    }

    /// Shows the new group dialog with optional parent selection
    pub(super) fn show_new_group_dialog(
        window: &adw::ApplicationWindow,
        state: SharedAppState,
        sidebar: SharedSidebar,
    ) {
        connection_dialogs::show_new_group_dialog(window.upcast_ref(), state, sidebar);
    }

    /// Shows the command palette dialog
    pub(super) fn show_command_palette(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
        notebook: &SharedNotebook,
        monitoring: &types::SharedMonitoring,
        session_bridges: &SessionSplitBridges,
        prefix: &str,
    ) {
        let gtk_window: &gtk4::Window = window.upcast_ref();
        let palette = crate::dialogs::CommandPaletteDialog::new(Some(gtk_window));

        // Populate with current connections and groups
        {
            let state_ref = state.borrow();
            let connections: Vec<_> = state_ref.list_connections_owned();
            let groups: Vec<_> = state_ref.list_groups().into_iter().cloned().collect();
            palette.set_connections(connections);
            palette.set_groups(groups);
            // Keys shown beside commands: the user's own, not the defaults.
            let keybindings = state_ref.settings().keybindings.clone();
            palette.set_shortcuts(
                rustconn_core::config::keybindings::default_keybindings()
                    .iter()
                    .filter_map(|def| {
                        let accel = keybindings.registered_accels(def).first().copied()?;
                        let (key, mods) = gtk4::accelerator_parse(accel)?;
                        Some((
                            def.action.clone(),
                            gtk4::accelerator_get_label(key, mods).to_string(),
                        ))
                    })
                    .collect(),
            );
        }

        // Populate open tabs for % mode
        {
            let open_tabs: Vec<crate::dialogs::OpenTabInfo> = notebook
                .get_all_sessions()
                .into_iter()
                .map(|s| crate::dialogs::OpenTabInfo {
                    session_id: s.id,
                    title: s.name,
                    protocol: s.protocol,
                    group: s.tab_group,
                })
                .collect();
            palette.set_open_tabs(open_tabs);
        }

        // Populate saved snippets for ! mode (W5). The palette holds only the
        // id/name/preview summary; running one goes back through the existing
        // execute_snippet path so variables and "confirm before run" apply.
        {
            let state_ref = state.borrow();
            let snippets: Vec<crate::dialogs::SnippetSummary> = state_ref
                .list_snippets()
                .into_iter()
                .map(|s| crate::dialogs::SnippetSummary {
                    id: s.id,
                    name: s.name.clone(),
                    // First non-empty line, so a multi-line snippet gives a
                    // one-line hint rather than wrapping the row.
                    preview: s
                        .command
                        .lines()
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or("")
                        .trim()
                        .to_string(),
                })
                .collect();
            palette.set_snippets(snippets);
        }

        // Wire action callback
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let notebook_clone = notebook.clone();
        let monitoring_clone = monitoring.clone();
        let session_bridges_clone = session_bridges.clone();
        let window_weak = window.downgrade();
        palette.connect_on_action(move |action| match action {
            rustconn_core::search::command_palette::CommandPaletteAction::Connect(uuid) => {
                // Same as the sidebar's Connect / Show Session: show a running
                // session instead of opening a duplicate.
                if Self::show_running_session(&notebook_clone, &session_bridges_clone, uuid) {
                    return;
                }
                Self::start_connection(
                    &state_clone,
                    &notebook_clone,
                    &sidebar_clone,
                    &monitoring_clone,
                    uuid,
                );
            }
            rustconn_core::search::command_palette::CommandPaletteAction::SwitchTab(session_id) => {
                notebook_clone.switch_to_tab(session_id);
            }
            rustconn_core::search::command_palette::CommandPaletteAction::RunSnippet(id) => {
                // Resolve the snippet by id and run it through the existing
                // path, so variables and "confirm before run" still apply.
                let snippet = state_clone
                    .borrow()
                    .list_snippets()
                    .into_iter()
                    .find(|s| s.id == id)
                    .cloned();
                if let Some(snippet) = snippet
                    && let Some(win) = window_weak.upgrade()
                {
                    // Always confirm: the palette hangs off the main window, so
                    // the send could reach the wrong focused terminal (#315).
                    crate::window::snippets::execute_snippet_confirmed(
                        &win,
                        &notebook_clone,
                        &session_bridges_clone,
                        &snippet,
                        &state_clone,
                    );
                }
            }
            rustconn_core::search::command_palette::CommandPaletteAction::GtkAction(name) => {
                // The palette names actions with their prefix (`win.unsplit`,
                // `app.shortcuts`). Only the widget lookup resolves a prefix;
                // the window's own `gio::ActionGroup` knows bare names, so a
                // prefixed name sent there found no action and did nothing. A
                // disabled action (Remove Split on a tab without a split) is
                // found and ignored.
                if let Some(win) = window_weak.upgrade()
                    && let Err(err) = gtk4::prelude::WidgetExt::activate_action(&win, &name, None)
                {
                    tracing::warn!(action = %name, %err, "command palette: no such action");
                }
            }
            other => {
                let action_name = match other {
                    rustconn_core::search::command_palette::CommandPaletteAction::OpenSettings => {
                        "settings"
                    }
                    rustconn_core::search::command_palette::CommandPaletteAction::NewConnection => {
                        "new-connection"
                    }
                    rustconn_core::search::command_palette::CommandPaletteAction::NewGroup => {
                        "new-group"
                    }
                    rustconn_core::search::command_palette::CommandPaletteAction::Import => {
                        "import"
                    }
                    rustconn_core::search::command_palette::CommandPaletteAction::Export => {
                        "export"
                    }
                    rustconn_core::search::command_palette::CommandPaletteAction::LocalShell => {
                        "local-shell"
                    }
                    rustconn_core::search::command_palette::CommandPaletteAction::QuickConnect => {
                        "quick-connect"
                    }
                    _ => return,
                };
                if let Some(win) = window_weak.upgrade() {
                    gio::ActionGroup::activate_action(
                        win.upcast_ref::<gio::ActionGroup>(),
                        action_name,
                        None,
                    );
                }
            }
        });

        palette.present_with_prefix(prefix);
    }

    /// Shows the import dialog
    pub(super) fn show_import_dialog(
        window: &adw::ApplicationWindow,
        state: SharedAppState,
        sidebar: SharedSidebar,
    ) {
        connection_dialogs::show_import_dialog(window.upcast_ref(), state, sidebar);
    }

    /// Shows the quick connect dialog with protocol selection
    pub(super) fn show_quick_connect_dialog(
        window: &adw::ApplicationWindow,
        notebook: SharedNotebook,
        split_view: SharedSplitView,
        sidebar: SharedSidebar,
        state: &SharedAppState,
        history: types::SharedQuickConnectHistory,
    ) {
        edit_dialogs::show_quick_connect_dialog(
            window.upcast_ref(),
            notebook,
            split_view,
            sidebar,
            state,
            history,
        );
    }

    /// Shows the export dialog
    ///
    /// Displays a dialog for exporting connections to various formats:
    /// - Ansible Inventory (INI/YAML)
    /// - SSH Config
    /// - Remmina (.remmina files)
    /// - Asbru-CM (YAML)
    pub(super) fn show_export_dialog(window: &adw::ApplicationWindow, state: SharedAppState) {
        let dialog = ExportDialog::new(Some(&window.clone().upcast()));

        // Get connections and groups from state
        let state_ref = state.borrow();
        let connections: Vec<_> = state_ref
            .list_connections()
            .iter()
            .map(|c| (*c).clone())
            .collect();
        let groups: Vec<_> = state_ref
            .list_groups()
            .iter()
            .map(|g| (*g).clone())
            .collect();
        let snippets: Vec<_> = state_ref.list_snippets().into_iter().cloned().collect();
        let smart_folders = state_ref.settings().smart_folders.clone();
        let templates = state_ref.get_all_templates();
        let clusters: Vec<_> = state_ref.get_all_clusters().into_iter().cloned().collect();
        let variables = state_ref.settings().global_variables.clone();
        drop(state_ref);

        // Set data for export
        dialog.set_connections(connections);
        dialog.set_groups(groups);
        dialog.set_snippets(snippets);
        dialog.set_smart_folders(smart_folders);
        dialog.set_templates(templates);
        dialog.set_clusters(clusters);
        dialog.set_variables(variables);

        let window_clone = window.clone();
        dialog.run(move |result| {
            if let Some(export_result) = result {
                // Optionally open the output location on success
                if !export_result.output_files.is_empty()
                    && let Some(first_file) = export_result.output_files.first()
                {
                    ExportDialog::open_output_location(first_file);
                }

                // Non-blocking success feedback (GNOME HIG: toast, not dialog).
                crate::toast::show_toast_on_window(
                    &window_clone,
                    &crate::i18n::i18n_f(
                        "Successfully exported {} connection(s). {} skipped.",
                        &[
                            &export_result.exported_count.to_string(),
                            &export_result.skipped_count.to_string(),
                        ],
                    ),
                    crate::toast::ToastType::Success,
                );
            }
        });
    }

    /// Shows the terminal search dialog
    pub(super) fn show_terminal_search_dialog(
        window: &adw::ApplicationWindow,
        notebook: &SharedNotebook,
    ) {
        if let Some(terminal) = notebook.get_active_terminal() {
            let dialog =
                crate::dialogs::TerminalSearchDialog::new(Some(&window.clone().upcast()), terminal);
            dialog.show();
        }
    }
}
