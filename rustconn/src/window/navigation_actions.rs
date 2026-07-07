//! Navigation and group operations window actions
//!
//! Extracted from `window/mod.rs` to reduce module complexity.

use super::*;

impl MainWindow {
    pub(crate) fn setup_navigation_actions(
        &self,
        window: &adw::ApplicationWindow,
        terminal_notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
        state: &SharedAppState,
        session_split_bridges: &SessionSplitBridges,
    ) {
        // Focus sidebar action
        let focus_sidebar_action = gio::SimpleAction::new("focus-sidebar", None);
        let sidebar_clone = sidebar.clone();
        focus_sidebar_action.connect_activate(move |_, _| {
            sidebar_clone.list_view().grab_focus();
        });
        window.add_action(&focus_sidebar_action);

        // Focus terminal action
        let focus_terminal_action = gio::SimpleAction::new("focus-terminal", None);
        let notebook_clone = terminal_notebook.clone();
        focus_terminal_action.connect_activate(move |_, _| {
            if let Some(terminal) = notebook_clone.get_active_terminal() {
                terminal.grab_focus();
            }
        });
        window.add_action(&focus_terminal_action);

        // Next tab action
        let next_tab_action = gio::SimpleAction::new("next-tab", None);
        let notebook_clone = terminal_notebook.clone();
        next_tab_action.connect_activate(move |_, _| {
            let tab_view = notebook_clone.tab_view();
            let n_pages = tab_view.n_pages();
            if n_pages > 0
                && let Some(selected) = tab_view.selected_page()
            {
                let current_pos = tab_view.page_position(&selected);
                let next_pos = (current_pos + 1) % n_pages;
                let next_page = tab_view.nth_page(next_pos);
                tab_view.set_selected_page(&next_page);
            }
        });
        window.add_action(&next_tab_action);

        // Previous tab action
        let prev_tab_action = gio::SimpleAction::new("prev-tab", None);
        let notebook_clone = terminal_notebook.clone();
        prev_tab_action.connect_activate(move |_, _| {
            let tab_view = notebook_clone.tab_view();
            let n_pages = tab_view.n_pages();
            if n_pages > 0
                && let Some(selected) = tab_view.selected_page()
            {
                let current_pos = tab_view.page_position(&selected);
                let prev_pos = if current_pos == 0 {
                    n_pages - 1
                } else {
                    current_pos - 1
                };
                let prev_page = tab_view.nth_page(prev_pos);
                tab_view.set_selected_page(&prev_page);
            }
        });
        window.add_action(&prev_tab_action);

        // Tab overview action — opens the grid view of all tabs
        let tab_overview_action = gio::SimpleAction::new("tab-overview", None);
        let notebook_clone = terminal_notebook.clone();
        tab_overview_action.connect_activate(move |_, _| {
            notebook_clone.open_tab_overview();
        });
        window.add_action(&tab_overview_action);

        // Switch tab via command palette (% prefix)
        let switch_tab_action = gio::SimpleAction::new("switch-tab-palette", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let notebook_clone = terminal_notebook.clone();
        let monitoring_clone = self.monitoring.clone();
        switch_tab_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_command_palette(
                    &win,
                    &state_clone,
                    &sidebar_clone,
                    &notebook_clone,
                    &monitoring_clone,
                    "%",
                );
            }
        });
        window.add_action(&switch_tab_action);

        // Toggle fullscreen action (stateful per GNOME HIG — menu shows checkmark)
        let toggle_fullscreen_action =
            gio::SimpleAction::new_stateful("toggle-fullscreen", None, &false.to_variant());
        let window_weak = window.downgrade();
        toggle_fullscreen_action.connect_activate(move |action, _| {
            if let Some(win) = window_weak.upgrade() {
                let is_fullscreen = win.is_fullscreen();
                if is_fullscreen {
                    win.unfullscreen();
                } else {
                    win.fullscreen();
                }
                action.set_state(&(!is_fullscreen).to_variant());
            }
        });
        window.add_action(&toggle_fullscreen_action);

        // Toggle compact interface (stateful per GNOME HIG — menu shows a
        // checkmark). Reflects and persists the manual compact setting; the
        // automatic-on-small-windows preference is preserved untouched.
        let initial_compact = state.borrow().settings().ui.compact_ui;
        let toggle_compact_action =
            gio::SimpleAction::new_stateful("toggle-compact", None, &initial_compact.to_variant());
        let state_for_compact = state.clone();
        toggle_compact_action.connect_activate(move |action, _| {
            let (manual, auto) = {
                let mut st = state_for_compact.borrow_mut();
                let ui = &mut st.settings_mut().ui;
                ui.compact_ui = !ui.compact_ui;
                (ui.compact_ui, ui.compact_auto)
            };
            if let Err(e) = state_for_compact.borrow().save_settings() {
                tracing::warn!(error = %e, "Failed to persist compact interface toggle");
            }
            crate::app::set_compact_prefs(manual, auto);
            action.set_state(&manual.to_variant());
        });
        window.add_action(&toggle_compact_action);

        // Toggle keyboard passthrough mode (stateful)
        // When enabled, all keybindings except quit/fullscreen/passthrough-toggle
        // are disabled so keys pass through to VTE terminal or embedded viewer.
        let toggle_passthrough_action =
            gio::SimpleAction::new_stateful("toggle-passthrough", None, &false.to_variant());
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let toast_overlay_clone = self.toast_overlay.clone();
        let passthrough_indicator_clone = self.passthrough_indicator.clone();
        let menu_button_clone = self.menu_button.clone();
        toggle_passthrough_action.connect_activate(move |action, _| {
            if let Some(win) = window_weak.upgrade() {
                let is_passthrough = action
                    .state()
                    .and_then(|v| v.get::<bool>())
                    .unwrap_or(false);
                let new_state = !is_passthrough;
                action.set_state(&new_state.to_variant());

                if let Some(app) = win.application().and_downcast::<adw::Application>() {
                    crate::app::set_passthrough(&app, &state_clone, new_state);
                }

                // Toggle passthrough indicator visibility in header bar
                passthrough_indicator_clone.set_visible(new_state);

                // The F10 primary-menu binding is GTK-internal (triggered by
                // the menu button's `primary` property), not an application
                // accelerator, so `set_passthrough` cannot remove it. Drop
                // the `primary` flag while passthrough is active so F10 also
                // reaches the remote session.
                menu_button_clone.set_primary(!new_state);

                // Show toast notification about the mode change
                let message = if new_state {
                    crate::i18n::i18n("Keyboard passthrough enabled — shortcuts disabled")
                } else {
                    crate::i18n::i18n("Keyboard passthrough disabled — shortcuts restored")
                };
                toast_overlay_clone.show_toast(&message);
            }
        });
        window.add_action(&toggle_passthrough_action);

        // Toggle cluster broadcast mode (stateful, per active tab's cluster).
        // Toggle split-view broadcast mode (stateful).
        // The action is enabled only when the active tab has a split layout with ≥2 sessions.
        // Activating mirrors keystrokes from any panel to all other panels in the same split.
        let toggle_broadcast_action =
            gio::SimpleAction::new_stateful("toggle-broadcast", None, &false.to_variant());
        toggle_broadcast_action.set_enabled(false);
        let notebook_for_action = terminal_notebook.clone();
        let bridges_for_action = session_split_bridges.clone();
        let toast_for_action = self.toast_overlay.clone();
        let broadcast_toggle_widget = self.broadcast_toggle.clone();
        toggle_broadcast_action.connect_activate(move |action, _| {
            let Some(session_id) = notebook_for_action.get_active_session_id() else {
                return;
            };
            let Some(bridge) = bridges_for_action.borrow().get(&session_id).cloned() else {
                return;
            };
            let new_state = !bridge.broadcast_active.get();
            bridge.broadcast_active.set(new_state);
            action.set_state(&new_state.to_variant());
            broadcast_toggle_widget.set_active(new_state);

            // When enabling broadcast, wire commit handlers for any sessions in this
            // split that don't have one yet.
            if new_state {
                for &sid in &bridge.active_sessions() {
                    wire_broadcast_for_session(&bridge, &notebook_for_action, sid);
                }
            }

            let message = if new_state {
                crate::i18n::i18n("Broadcast enabled — keystrokes mirrored to all split panels")
            } else {
                crate::i18n::i18n("Broadcast disabled — keystrokes go to focused panel only")
            };
            toast_for_action.show_toast(&message);
        });
        window.add_action(&toggle_broadcast_action);

        // Track active tab → update broadcast toggle visibility/state.
        // The toggle is visible only when the active tab has a split with ≥2 sessions.
        {
            let notebook_for_signal = terminal_notebook.clone();
            let bridges_for_signal = session_split_bridges.clone();
            let toggle_widget = self.broadcast_toggle.clone();
            let action_for_signal = toggle_broadcast_action.clone();
            terminal_notebook
                .tab_view()
                .connect_selected_page_notify(move |_| {
                    update_broadcast_toggle_state(
                        &notebook_for_signal,
                        &bridges_for_signal,
                        &toggle_widget,
                        &action_for_signal,
                    );
                });
        }
    }

    /// Sets up group operations actions (select all, delete selected, etc.)
    pub(crate) fn setup_group_operations_actions(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        terminal_notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
    ) {
        // Group operations action (toggle mode)
        let group_ops_action =
            gio::SimpleAction::new_stateful("group-operations", None, &false.to_variant());
        let sidebar_clone = sidebar.clone();
        group_ops_action.connect_activate(move |action, _| {
            let current = action
                .state()
                .and_then(|v| v.get::<bool>())
                .unwrap_or(false);
            action.set_state(&(!current).to_variant());
            Self::toggle_group_operations_mode(&sidebar_clone, !current);
        });
        window.add_action(&group_ops_action);

        // Select all action
        let select_all_action = gio::SimpleAction::new("select-all", None);
        let sidebar_clone = sidebar.clone();
        select_all_action.connect_activate(move |_, _| {
            if sidebar_clone.is_group_operations_mode() {
                sidebar_clone.select_all();
            }
        });
        window.add_action(&select_all_action);

        // Clear selection action
        let clear_selection_action = gio::SimpleAction::new("clear-selection", None);
        let sidebar_clone = sidebar.clone();
        clear_selection_action.connect_activate(move |_, _| {
            sidebar_clone.clear_selection();
        });
        window.add_action(&clear_selection_action);

        // Delete selected action
        let delete_selected_action = gio::SimpleAction::new("delete-selected", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        delete_selected_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::delete_selected_connections(win.upcast_ref(), &state_clone, &sidebar_clone);
            }
        });
        window.add_action(&delete_selected_action);

        // Batch edit selected connections action
        let batch_edit_action = gio::SimpleAction::new("batch-edit-selected", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let toast_clone = self.toast_overlay.widget().clone();
        batch_edit_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                super::batch_edit::show_batch_edit_dialog(
                    win.upcast_ref(),
                    &state_clone,
                    &sidebar_clone,
                    &toast_clone,
                );
            }
        });
        window.add_action(&batch_edit_action);

        // Move selected to group action
        let move_selected_action = gio::SimpleAction::new("move-selected-to-group", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        move_selected_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_move_selected_to_group_dialog(
                    win.upcast_ref(),
                    &state_clone,
                    &sidebar_clone,
                );
            }
        });
        window.add_action(&move_selected_action);

        // Sort connections action
        let sort_action = gio::SimpleAction::new("sort-connections", None);
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        sort_action.connect_activate(move |_, _| {
            Self::sort_connections(&state_clone, &sidebar_clone);
        });
        window.add_action(&sort_action);

        // Sort recent action
        let sort_recent_action = gio::SimpleAction::new("sort-recent", None);
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        sort_recent_action.connect_activate(move |_, _| {
            Self::sort_recent(&state_clone, &sidebar_clone);
        });
        window.add_action(&sort_recent_action);

        // Create cluster from sidebar selection
        let cluster_from_selection_action = gio::SimpleAction::new("cluster-from-selection", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = terminal_notebook.clone();
        let sidebar_clone = sidebar.clone();
        let toast_clone = self.toast_overlay.clone();
        cluster_from_selection_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                let selected_ids = sidebar_clone.get_selected_ids();
                if selected_ids.is_empty() {
                    return;
                }
                clusters::show_new_cluster_dialog_with_selection(
                    win.upcast_ref(),
                    state_clone.clone(),
                    notebook_clone.clone(),
                    selected_ids,
                    toast_clone.clone(),
                );
            }
        });
        window.add_action(&cluster_from_selection_action);
    }
}

/// Updates the broadcast toggle button's visibility and state based on the
/// active tab. Hidden if the tab has no split layout (or only 1 active pane),
/// otherwise shown with the toggle reflecting the split's current broadcast
/// flag. When broadcast is on, also adds a `.broadcasting` CSS class so the
/// button gets a visible accent (defined in `assets/style.css`).
pub(super) fn update_broadcast_toggle_state(
    notebook: &SharedNotebook,
    bridges: &SessionSplitBridges,
    toggle: &gtk4::ToggleButton,
    action: &gio::SimpleAction,
) {
    let Some(session_id) = notebook.get_active_session_id() else {
        tracing::debug!("update_broadcast_toggle_state: no active session, hiding toggle");
        toggle.set_visible(false);
        toggle.remove_css_class("broadcasting");
        action.set_enabled(false);
        return;
    };

    let bridge = bridges.borrow().get(&session_id).cloned();
    // Broadcast mirrors VTE `commit` signals, so the toggle is meaningful only
    // when the split holds ≥2 terminal sessions (R8.3) and the focused panel is
    // itself a terminal — an embedded viewer in focus hides it (R8.2).
    let show_toggle = bridge.as_ref().is_some_and(|b| {
        let focused_is_embedded = b
            .get_focused_session()
            .is_some_and(|sid| b.get_terminal(sid).is_none());
        b.terminal_sessions().len() >= 2 && !focused_is_embedded
    });
    match bridge {
        Some(b) if show_toggle => {
            let active = b.broadcast_active.get();
            tracing::debug!(
                "update_broadcast_toggle_state: showing toggle for session {} (terminal_sessions={}, broadcast_active={})",
                session_id,
                b.terminal_sessions().len(),
                active
            );
            toggle.set_visible(true);
            toggle.set_active(active);
            if active {
                toggle.add_css_class("broadcasting");
            } else {
                toggle.remove_css_class("broadcasting");
            }
            action.set_state(&active.to_variant());
            action.set_enabled(true);
        }
        Some(b) => {
            tracing::debug!(
                "update_broadcast_toggle_state: hiding toggle — session {} has {} terminal session(s) / embedded_panel={}",
                session_id,
                b.terminal_sessions().len(),
                b.has_embedded_panel()
            );
            toggle.set_visible(false);
            toggle.remove_css_class("broadcasting");
            action.set_enabled(false);
        }
        None => {
            tracing::debug!(
                "update_broadcast_toggle_state: hiding toggle — no bridge for session {} (bridges count={})",
                session_id,
                bridges.borrow().len()
            );
            toggle.set_visible(false);
            toggle.remove_css_class("broadcasting");
            action.set_enabled(false);
        }
    }
}

/// Refreshes the broadcast toggle by looking up the action on the given window.
/// Used by other modules (e.g. split_view_actions) that don't have direct
/// access to the action handle but do have the window reference.
pub(super) fn refresh_broadcast_toggle(
    window: &adw::ApplicationWindow,
    notebook: &SharedNotebook,
    bridges: &SessionSplitBridges,
    toggle: &gtk4::ToggleButton,
) {
    use gtk4::prelude::*;
    let Some(action) = window
        .lookup_action("toggle-broadcast")
        .and_then(|a| a.downcast::<gio::SimpleAction>().ok())
    else {
        return;
    };
    update_broadcast_toggle_state(notebook, bridges, toggle, &action);
}

/// Wires a single split-view session into the broadcast mirroring chain.
///
/// Idempotent: if the session is already in `broadcast_wired_sessions`, this
/// is a no-op. Safe to call from any code path that introduces a new session
/// into the split (initial enable, Select Tab placement, drop-target).
///
/// The handler uses `bridge.broadcast_active` as a guard so it is a no-op
/// while broadcast is off, and a SHARED `broadcast_busy` re-entrancy guard
/// on the bridge prevents the `feed_child → commit → feed_child` cascade
/// across all wired sessions (otherwise each commit handler would have its
/// own per-instance flag and characters would be doubled when text is fed
/// back into the source terminal's neighbours).
pub(super) fn wire_broadcast_for_session(
    bridge: &std::rc::Rc<crate::split_view::SplitViewBridge>,
    notebook: &SharedNotebook,
    sid: uuid::Uuid,
) {
    // Broadcast mirrors VTE `commit` signals; embedded RDP/VNC/SPICE sessions
    // have no terminal, so never wire them — mirroring stays terminal-only
    // (R8.1, R8.4, R8.5).
    if bridge.get_terminal(sid).is_none() {
        tracing::debug!("wire_broadcast_for_session: skipping non-terminal session {sid}");
        return;
    }
    if bridge.broadcast_wired_sessions.borrow().contains(&sid) {
        return;
    }
    bridge.broadcast_wired_sessions.borrow_mut().insert(sid);

    let bridge_for_cb = bridge.clone();
    let notebook_for_cb = notebook.clone();

    notebook.connect_commit(sid, move |text| {
        if !bridge_for_cb.broadcast_active.get() {
            return;
        }
        if bridge_for_cb.broadcast_busy.get() {
            return;
        }
        bridge_for_cb.broadcast_busy.set(true);
        for target_id in bridge_for_cb.active_sessions() {
            if target_id == sid {
                continue;
            }
            notebook_for_cb.send_text_to_session(target_id, text);
        }
        bridge_for_cb.broadcast_busy.set(false);
    });
}
