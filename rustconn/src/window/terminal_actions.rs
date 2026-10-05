//! Terminal-related window actions (copy, paste, close tab, zoom, search)
//!
//! Extracted from `window/mod.rs` to reduce module complexity.

use super::*;

/// Returns the terminal of the focused pane, when the active tab holds a split.
///
/// A tab that hosts a split layout still reports its own session as the active
/// one, so a clipboard action resolved through the notebook alone always lands on
/// the pane the layout was created from — the first one — no matter which pane
/// the user is typing into. The pane actually in focus is known only to the
/// layout's bridge, and those live in `session_split_bridges` keyed by the
/// hosting session. (The window's `split_view` field is a separate bridge that
/// never receives these sessions, so asking it always answered `None` and the
/// clipboard silently fell back to the host pane.)
///
/// Returns `None` when the active tab has no split, when the focused pane holds
/// no session, or when that session is an embedded viewer rather than a
/// terminal — in each case the caller's notebook-level fallback is correct.
fn focused_pane_terminal(
    notebook: &SharedNotebook,
    bridges: &SessionSplitBridges,
) -> Option<vte4::Terminal> {
    let host_session = notebook.get_active_session_id()?;
    // A failed borrow would mean a split is being rebuilt at this very moment;
    // falling back is better than panicking on a keystroke.
    let focused_session = bridges
        .try_borrow()
        .ok()?
        .get(&host_session)?
        .get_focused_session()?;
    notebook.get_terminal(focused_session)
}

impl MainWindow {
    pub(crate) fn setup_terminal_actions(
        &self,
        window: &adw::ApplicationWindow,
        terminal_notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
        state: &SharedAppState,
    ) {
        // Search action
        let search_action = gio::SimpleAction::new("search", None);
        let sidebar_clone = sidebar.clone();
        let split_view_weak = self.overlay_split_view.downgrade();
        search_action.connect_activate(move |_, _| {
            // Ctrl+F with the sidebar hidden would focus an entry nobody can
            // see; reveal the sidebar first.
            if let Some(split_view) = split_view_weak.upgrade()
                && !split_view.shows_sidebar()
            {
                split_view.set_show_sidebar(true);
            }
            sidebar_clone.focus_search();
        });
        window.add_action(&search_action);

        // Command palette action (Ctrl+P)
        let command_palette_action = gio::SimpleAction::new("command-palette", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let notebook_clone = terminal_notebook.clone();
        let monitoring_clone = self.monitoring.clone();
        command_palette_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_command_palette(
                    &win,
                    &state_clone,
                    &sidebar_clone,
                    &notebook_clone,
                    &monitoring_clone,
                    "",
                );
            }
        });
        window.add_action(&command_palette_action);

        // Command palette commands mode action (Ctrl+Shift+P)
        let command_palette_commands_action =
            gio::SimpleAction::new("command-palette-commands", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let notebook_clone = terminal_notebook.clone();
        let monitoring_clone = self.monitoring.clone();
        command_palette_commands_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_command_palette(
                    &win,
                    &state_clone,
                    &sidebar_clone,
                    &notebook_clone,
                    &monitoring_clone,
                    ">",
                );
            }
        });
        window.add_action(&command_palette_commands_action);

        // Copy action - targets the focused pane when the tab holds a split
        let copy_action = gio::SimpleAction::new("copy", None);
        let notebook_clone = terminal_notebook.clone();
        let session_bridges_copy = self.session_split_bridges.clone();
        copy_action.connect_activate(move |_, _| {
            if let Some(terminal) = focused_pane_terminal(&notebook_clone, &session_bridges_copy) {
                if let Some(text) = terminal.text_selected(vte4::Format::Text) {
                    terminal.display().clipboard().set_text(&text);
                }
                return;
            }
            // No split: the active tab is the session (also the RDP/VNC path).
            notebook_clone.copy_to_clipboard();
        });
        window.add_action(&copy_action);

        // Paste action - targets the focused pane when the tab holds a split
        let paste_action = gio::SimpleAction::new("paste", None);
        let notebook_clone = terminal_notebook.clone();
        let session_bridges_paste = self.session_split_bridges.clone();
        paste_action.connect_activate(move |_, _| {
            if let Some(terminal) = focused_pane_terminal(&notebook_clone, &session_bridges_paste) {
                crate::terminal::safe_paste::paste_into_terminal(&terminal);
                return;
            }
            // No split: the active tab is the session (also the RDP/VNC path).
            notebook_clone.paste_from_clipboard();
        });
        window.add_action(&paste_action);

        // Terminal search action
        let terminal_search_action = gio::SimpleAction::new("terminal-search", None);
        let notebook_clone = terminal_notebook.clone();
        let window_weak = window.downgrade();
        terminal_search_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_terminal_search_dialog(win.upcast_ref(), &notebook_clone);
            }
        });
        window.add_action(&terminal_search_action);

        // Close tab action - closes the currently active session tab
        let close_tab_action = gio::SimpleAction::new("close-tab", None);
        let notebook_clone = terminal_notebook.clone();
        let split_view_clone = self.split_view.clone();
        let sidebar_clone = self.sidebar.clone();
        let session_bridges_close_tab = self.session_split_bridges.clone();
        close_tab_action.connect_activate(move |_, _| {
            if let Some(session_id) = notebook_clone.get_active_session_id() {
                // Get connection ID before closing
                let connection_id = notebook_clone
                    .get_session_info(session_id)
                    .map(|info| info.connection_id);

                // Clear from ALL per-session split bridges first
                {
                    let bridges = session_bridges_close_tab.borrow();
                    for bridge in bridges.values() {
                        if bridge.is_session_displayed(session_id) {
                            tracing::debug!(
                                "close-tab: clearing session {} from per-session bridge",
                                session_id
                            );
                            bridge.clear_session_from_panes(session_id);
                        }
                    }
                }

                // Clear from global split view
                split_view_clone.clear_session_from_panes(session_id);
                // Then close the tab
                notebook_clone.close_tab(session_id);
                // Decrement session count in sidebar if we have a connection ID
                if let Some(conn_id) = connection_id {
                    sidebar_clone.decrement_session_count(&conn_id.to_string(), false);
                }

                // After closing, the selected-page handler will take care of
                // showing the correct content for the new active session.
                // We only need to handle RDP redraw here since it's not handled
                // by the selected-page handler.
                if let Some(new_session_id) = notebook_clone.get_active_session_id()
                    && let Some(info) = notebook_clone.get_session_info(new_session_id)
                    && info.protocol == "rdp"
                {
                    // Trigger redraw for RDP widget
                    notebook_clone.queue_rdp_redraw(new_session_id);
                }
            }
        });
        window.add_action(&close_tab_action);

        // Close tab by ID action - closes a specific session tab without switching first
        let close_tab_by_id_action =
            gio::SimpleAction::new("close-tab-by-id", Some(glib::VariantTy::STRING));
        let notebook_clone = terminal_notebook.clone();
        let split_view_clone = self.split_view.clone();
        let sidebar_clone = self.sidebar.clone();
        let session_bridges_close = self.session_split_bridges.clone();
        let split_container_close = self.split_container.clone();
        close_tab_by_id_action.connect_activate(move |_, param| {
            if let Some(param) = param
                && let Some(session_id_str) = param.get::<String>()
                && let Ok(session_id) = uuid::Uuid::parse_str(&session_id_str)
            {
                // Get the currently active session BEFORE closing
                // This is important because page numbers will shift after removal
                let current_active_session = notebook_clone.get_active_session_id();
                let is_closing_active = current_active_session == Some(session_id);

                // Get connection ID before closing
                let connection_id = notebook_clone
                    .get_session_info(session_id)
                    .map(|info| info.connection_id);

                // Clear tab color indicator
                notebook_clone.clear_tab_split_color(session_id);

                // Clear session from ALL per-session split bridges
                // This ensures the panel shows "Empty Panel" placeholder
                {
                    let bridges = session_bridges_close.borrow();
                    for bridge in bridges.values() {
                        if bridge.is_session_displayed(session_id) {
                            tracing::debug!(
                                "close-tab-by-id: clearing session {} from per-session bridge",
                                session_id
                            );
                            bridge.clear_session_from_panes(session_id);
                        }
                    }
                }

                // Close session from global split view with auto-cleanup
                // Returns true if split view should be hidden
                let should_hide_split = split_view_clone.close_session_from_panes(session_id);

                // Then close the tab
                notebook_clone.close_tab(session_id);

                // Decrement session count in sidebar if we have a connection ID
                if let Some(conn_id) = connection_id {
                    sidebar_clone.decrement_session_count(&conn_id.to_string(), false);
                }

                // Hide split view if no sessions remain in panes
                if should_hide_split {
                    split_view_clone.widget().set_visible(false);
                    split_view_clone.widget().set_vexpand(false);
                    split_container_close.set_visible(false);
                    notebook_clone.widget().set_vexpand(true);
                    notebook_clone.show_tab_view_content();
                }

                // The selected-page handler will take care of showing the correct
                // content for the new active session. We only need to handle
                // special cases here.
                let notebook_for_idle = notebook_clone.clone();
                if is_closing_active {
                    // We closed the active tab - selected-page handler will fire
                    // Just handle RDP redraw
                    glib::idle_add_local_once(move || {
                        if let Some(new_session_id) = notebook_for_idle.get_active_session_id()
                            && let Some(info) = notebook_for_idle.get_session_info(new_session_id)
                            && info.protocol == "rdp"
                        {
                            notebook_for_idle.queue_rdp_redraw(new_session_id);
                        }
                    });
                } else if let Some(active_id) = current_active_session {
                    // We closed a non-active tab, ensure we stay on the active tab
                    // Defer to next main loop iteration to override switch-page effects
                    glib::idle_add_local_once(move || {
                        notebook_for_idle.switch_to_tab(active_id);
                        if let Some(info) = notebook_for_idle.get_session_info(active_id) {
                            if info.protocol == "rdp" {
                                notebook_for_idle.queue_rdp_redraw(active_id);
                            } else if info.protocol == "vnc"
                                && let Some(vnc_widget) =
                                    notebook_for_idle.get_vnc_widget(active_id)
                            {
                                vnc_widget.widget().queue_draw();
                            }
                        }
                    });
                }
            }
        });
        window.add_action(&close_tab_by_id_action);

        // Local shell action
        let local_shell_action = gio::SimpleAction::new("local-shell", None);
        let notebook_clone = terminal_notebook.clone();
        let split_view_clone = self.split_view.clone();
        let state_clone = state.clone();
        local_shell_action.connect_activate(move |_, _| {
            Self::open_local_shell_with_split(
                &notebook_clone,
                &split_view_clone,
                Some(&state_clone),
            );
        });
        window.add_action(&local_shell_action);

        // Quick connect action
        let quick_connect_action = gio::SimpleAction::new("quick-connect", None);
        let window_weak = window.downgrade();
        let notebook_clone = terminal_notebook.clone();
        let split_view_clone = self.split_view.clone();
        let sidebar_clone = self.sidebar.clone();
        let history_clone = self.quick_connect_history.clone();
        let state_clone = state.clone();
        quick_connect_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_quick_connect_dialog(
                    win.upcast_ref(),
                    notebook_clone.clone(),
                    split_view_clone.clone(),
                    sidebar_clone.clone(),
                    &state_clone,
                    history_clone.clone(),
                );
            }
        });
        window.add_action(&quick_connect_action);

        // Start recording action - starts recording for the selected sidebar connection's session
        let start_recording_action = gio::SimpleAction::new("start-recording", None);
        let notebook_clone = terminal_notebook.clone();
        let sidebar_clone = sidebar.clone();
        let state_clone = state.clone();
        start_recording_action.connect_activate(move |_, _| {
            if let Some(item) = sidebar_clone.get_selected_item() {
                let id_str = item.id();
                if let Ok(conn_id) = Uuid::parse_str(&id_str) {
                    // Find the active session for this connection
                    if let Some(session) = notebook_clone
                        .get_all_sessions()
                        .into_iter()
                        .find(|s| s.connection_id == conn_id)
                    {
                        let (conn_name, ssh_params) = {
                            let state_ref = state_clone.borrow();
                            let conn = state_ref.get_connection(conn_id);
                            let name = conn.map(|c| c.name.clone()).unwrap_or_else(|| item.name());
                            let params = conn.and_then(|c| {
                                // Resolve key path via inheritance (connection → group → parent group → root)
                                let groups: Vec<rustconn_core::models::ConnectionGroup> =
                                    state_ref.list_groups_owned();
                                let key_path = rustconn_core::connection::ssh_inheritance::resolve_ssh_key_path(c, &groups)
                                    .and_then(|p| rustconn_core::resolve_key_path(&p))
                                    .map(|p| p.to_string_lossy().to_string());
                                // Only build params for SSH-like protocols
                                if matches!(
                                    c.protocol.as_str(),
                                    "ssh" | "sftp" | "telnet" | "mosh" | "serial"
                                ) {
                                    Some(crate::terminal::SshRecordingParams {
                                        host: c.host.clone(),
                                        port: c.port,
                                        username: c.username.clone(),
                                        identity_file: key_path,
                                    })
                                } else {
                                    None
                                }
                            });
                            (name, params)
                        };
                        notebook_clone.start_recording(
                            session.id,
                            &conn_name,
                            ssh_params,
                        );
                    }
                }
            }
        });
        window.add_action(&start_recording_action);

        // Stop recording action - stops recording for the selected sidebar connection's session
        let stop_recording_action = gio::SimpleAction::new("stop-recording", None);
        let notebook_clone = terminal_notebook.clone();
        let sidebar_clone = sidebar.clone();
        stop_recording_action.connect_activate(move |_, _| {
            if let Some(item) = sidebar_clone.get_selected_item() {
                let id_str = item.id();
                if let Ok(conn_id) = Uuid::parse_str(&id_str) {
                    // Find the active session for this connection
                    if let Some(session) = notebook_clone
                        .get_all_sessions()
                        .into_iter()
                        .find(|s| s.connection_id == conn_id)
                    {
                        notebook_clone.stop_recording(session.id);
                    }
                }
            }
        });
        window.add_action(&stop_recording_action);

        // External viewer: Disconnect (issue #209) — gracefully terminates the
        // owned viewer child of the selected connection's external session(s).
        // For a detaching viewer RustConn does not own (`disconnect` returns
        // false), inform the user via a toast and leave the session unchanged
        // (R5.5). The registry is reached through the thread-local accessor so
        // the launch path and these actions share one instance without a cycle.
        let external_disconnect_action = gio::SimpleAction::new("external-disconnect", None);
        let sidebar_clone = sidebar.clone();
        let toast_clone = self.toast_overlay.clone();
        external_disconnect_action.connect_activate(move |_, _| {
            let Some(item) = sidebar_clone.get_selected_item() else {
                return;
            };
            let Ok(conn_id) = Uuid::parse_str(&item.id()) else {
                return;
            };
            let Some(registry) = super::external_session_registry() else {
                return;
            };
            let session_ids = registry.active_session_ids(conn_id);
            if session_ids.is_empty() {
                return;
            }
            let mut any_not_owned = false;
            for session_id in session_ids {
                if !registry.disconnect(session_id) {
                    any_not_owned = true;
                }
            }
            if any_not_owned {
                toast_clone.show_toast(&crate::i18n::i18n(
                    "This viewer runs independently and cannot be closed from RustConn",
                ));
            }
        });
        window.add_action(&external_disconnect_action);

        // External viewer: Stop tracking (issue #209) — deregisters the selected
        // connection's external session(s) without terminating the viewer (R5.4).
        let external_stop_tracking_action = gio::SimpleAction::new("external-stop-tracking", None);
        let sidebar_clone = sidebar.clone();
        external_stop_tracking_action.connect_activate(move |_, _| {
            let Some(item) = sidebar_clone.get_selected_item() else {
                return;
            };
            let Ok(conn_id) = Uuid::parse_str(&item.id()) else {
                return;
            };
            let Some(registry) = super::external_session_registry() else {
                return;
            };
            for session_id in registry.active_session_ids(conn_id) {
                registry.stop_tracking(session_id);
            }
        });
        window.add_action(&external_stop_tracking_action);

        // Open new session (issue #209 / R7.5) — force-launches a brand-new
        // session for the selected connection, bypassing the smart double-click
        // focus behavior. Offered in the context menu for external-only sessions
        // where a plain double-click would only show the "already running" toast.
        let open_new_session_action = gio::SimpleAction::new("open-new-session", None);
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let notebook_clone = terminal_notebook.clone();
        let split_view_clone = self.split_view.clone();
        let monitoring_clone = self.monitoring.clone();
        let activity_clone = self.activity_coordinator.clone();
        open_new_session_action.connect_activate(move |_, _| {
            let Some(item) = sidebar_clone.get_selected_item() else {
                return;
            };
            let Ok(conn_id) = Uuid::parse_str(&item.id()) else {
                return;
            };
            sidebar_clone.update_connection_status(&conn_id.to_string(), "connecting");
            Self::start_connection_with_credential_resolution(
                state_clone.clone(),
                notebook_clone.clone(),
                split_view_clone.clone(),
                sidebar_clone.clone(),
                monitoring_clone.clone(),
                conn_id,
                Some(activity_clone.clone()),
            );
        });
        window.add_action(&open_new_session_action);
    }
}
