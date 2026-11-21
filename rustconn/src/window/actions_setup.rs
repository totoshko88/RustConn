//! Window action registration and signal wiring

use super::*;

impl MainWindow {
    /// Sets up window actions
    pub(super) fn setup_actions(&self) {
        let window = &self.window;
        let state = self.state.clone();
        let sidebar = self.sidebar.clone();
        let terminal_notebook = self.terminal_notebook.clone();

        // Set up action groups
        self.setup_connection_actions(window, &state, &sidebar, &terminal_notebook);
        self.setup_edit_actions(window, &state, &sidebar);
        self.setup_terminal_actions(window, &terminal_notebook, &sidebar, &state);
        self.setup_navigation_actions(
            window,
            &terminal_notebook,
            &sidebar,
            &state,
            &self.session_split_bridges,
        );
        // Per-connection command-macro keybinds: dynamic accels that follow the
        // focused terminal session, split panes included (asbru-borrow #1).
        macro_dispatch::setup_macro_dispatch(
            window,
            &terminal_notebook,
            &self.session_split_bridges,
            &state,
        );
        self.setup_group_operations_actions(window, &state, &sidebar);
        self.setup_group_broadcast_actions(window, &terminal_notebook);
        self.setup_snippet_actions(window, &state, &terminal_notebook, &sidebar);
        script_actions::setup_script_actions(
            window,
            &terminal_notebook,
            &self.session_split_bridges,
        );
        network_tools::setup_network_tools(window, &state, &terminal_notebook);
        self.setup_cluster_actions(window, &state, &terminal_notebook, &sidebar);
        self.setup_template_actions(window, &state, &sidebar);
        self.setup_workspace_actions(window, &state, &terminal_notebook, &sidebar);
        self.setup_split_view_actions(window);
        self.setup_variables_actions(window, &state);
        self.setup_history_actions(window, &state);
        self.setup_misc_actions(window, &state, &sidebar, &terminal_notebook);
        self.setup_detach_actions(window);
        Self::setup_smart_folder_actions(window, &state, &sidebar);
    }

    /// Sets up miscellaneous actions (drag-drop)
    pub(super) fn setup_misc_actions(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
        terminal_notebook: &SharedNotebook,
    ) {
        // Drag-drop item action for reordering connections
        let drag_drop_action =
            gio::SimpleAction::new("drag-drop-item", Some(glib::VariantTy::STRING));
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        drag_drop_action.connect_activate(move |_, param| {
            if let Some(data) = param.and_then(gtk4::glib::Variant::get::<String>) {
                Self::handle_drag_drop(&state_clone, &sidebar_clone, &data);
            }
        });
        window.add_action(&drag_drop_action);

        // Hide drop indicator action - called when drag ends or drop completes
        let hide_drop_indicator_action = gio::SimpleAction::new("hide-drop-indicator", None);
        let sidebar_clone = sidebar.clone();
        hide_drop_indicator_action.connect_activate(move |_, _| {
            sidebar_clone.hide_drop_indicator();
        });
        window.add_action(&hide_drop_indicator_action);

        // Toggle sidebar visibility
        let toggle_sidebar_action = gio::SimpleAction::new("toggle-sidebar", None);
        let split_view_clone = self.overlay_split_view.clone();
        toggle_sidebar_action.connect_activate(move |_, _| {
            let visible = split_view_clone.shows_sidebar();
            split_view_clone.set_show_sidebar(!visible);
        });
        window.add_action(&toggle_sidebar_action);

        // Scripts manager — opens the script library dialog.
        let manage_scripts_action = gio::SimpleAction::new("manage-scripts", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = terminal_notebook.clone();
        manage_scripts_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                crate::dialogs::script::show_scripts_manager(
                    win.upcast_ref::<gtk4::ApplicationWindow>(),
                    state_clone.clone(),
                    notebook_clone.clone(),
                );
            }
        });
        window.add_action(&manage_scripts_action);
    }

    /// Connects UI signals
    #[expect(
        clippy::too_many_lines,
        reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
    )]
    pub(super) fn connect_signals(&self) {
        let state = self.state.clone();
        let sidebar = self.sidebar.clone();
        let terminal_notebook = self.terminal_notebook.clone();
        let split_view = self.split_view.clone();
        let split_view_for_close = self.overlay_split_view.clone();
        let window = self.window.clone();

        // Set up split view cleanup callback for when tabs are closed via TabView
        // This ensures panels show "Empty Pane" placeholder when their session is closed
        {
            let session_bridges_for_cleanup = self.session_split_bridges.clone();
            let split_view_for_cleanup = split_view.clone();
            // Weak ref: the closure is stored *inside* terminal_notebook, so a
            // strong clone here would form an Rc cycle (notebook → closure →
            // notebook) and leak the whole notebook.
            let notebook_for_cleanup = Rc::downgrade(&terminal_notebook);
            let monitoring_for_cleanup = self.monitoring.clone();
            let split_container_for_cleanup = self.split_container.clone();

            // Tells the notebook which sessions a tab's split shows, so a user
            // closing that tab is asked first (item 11). Same owner test and
            // pane walk as the cleanup hook below, which is what a confirmed
            // close then runs. `try_borrow`: a close fired while this map is
            // borrowed closes as before instead of panicking.
            let session_bridges_for_members = self.session_split_bridges.clone();
            terminal_notebook.set_split_members_provider(move |session_id| {
                let Ok(bridges) = session_bridges_for_members.try_borrow() else {
                    tracing::warn!(
                        session = %session_id,
                        "split members: bridge map busy; closing without asking"
                    );
                    return Vec::new();
                };
                let Some(bridge) = bridges
                    .get(&session_id)
                    .filter(|bridge| bridge.is_owned_by(session_id))
                else {
                    return Vec::new();
                };
                let mut members = vec![session_id];
                for pane_id in bridge.pane_ids() {
                    if let Some(sid) = bridge.get_pane_session(pane_id)
                        && !sid.is_nil()
                        && !members.contains(&sid)
                    {
                        members.push(sid);
                    }
                }
                members
            });

            terminal_notebook.set_on_split_cleanup(move |session_id| {
                // If the closing tab OWNS a split bridge (key == owner session_id),
                // its guest (Select-Tabbed) sessions live inside this tab's split
                // widget and their home tabs only show the "Displayed in Split View"
                // placeholder. Return every guest to its home tab before the owner
                // tab — and its bridge widget — is torn down, otherwise the guest
                // is stranded on that placeholder.
                if let Some(notebook) = notebook_for_cleanup.upgrade() {
                    let guests: Vec<Uuid> = {
                        let bridges = session_bridges_for_cleanup.borrow();
                        bridges
                            .get(&session_id)
                            // The map holds an entry for *every* session a layout
                            // displays, so finding one here does not mean this
                            // session owns it. Asking the bridge is what tells
                            // owner from guest; without this check, closing a
                            // single pane evacuated the whole layout, because the
                            // departing guest looked like the owner.
                            .filter(|bridge| bridge.is_owned_by(session_id))
                            .map_or_else(Vec::new, |bridge| {
                                bridge
                                    .pane_ids()
                                    .iter()
                                    .filter_map(|&pane_id| bridge.get_pane_session(pane_id))
                                    .filter(|&sid| sid != session_id)
                                    .collect()
                            })
                    };
                    for guest_id in guests {
                        tracing::debug!(
                            "on_split_cleanup: returning guest session {} to its home tab \
                             (split owner {} closing)",
                            guest_id,
                            session_id
                        );
                        notebook.clear_tab_split_color(guest_id);
                        notebook.reparent_terminal_to_tab(guest_id);
                        // Resume monitoring suspended when the guest entered the split.
                        if monitoring_for_cleanup.is_suspended(guest_id)
                            && let Some(container) = notebook.get_session_container(guest_id)
                        {
                            monitoring_for_cleanup.resume_monitoring(guest_id, &container);
                        }
                        // Drop the guest's bridge entry too (issue #374 follow-up).
                        // The entry is the SAME Rc as the owner's torn-down
                        // layout, whose model still lists this guest's old panels
                        // AND the now-killed owner session. Left in the map, a
                        // later split on this guest REUSED that stale bridge and
                        // restore_panel_contents walked a panel whose session has
                        // no content widget, leaving a dead pane with no header,
                        // no hover actions and no empty-placeholder X. Removing it
                        // forces get_or_create_session_bridge down the CREATE path
                        // with a clean single-panel model.
                        session_bridges_for_cleanup.borrow_mut().remove(&guest_id);
                    }
                }

                // Take the session out of every per-session split bridge. The map
                // holds one entry per displayed session, so the same layout shows
                // up under several keys; collect each one once, and drop the
                // borrow before a guest's layout collapses (that takes
                // `borrow_mut` on the same map).
                let displaying: Vec<Rc<SplitViewBridge>> = {
                    let bridges = session_bridges_for_cleanup.borrow();
                    let mut unique: Vec<Rc<SplitViewBridge>> = Vec::new();
                    for bridge in bridges.values() {
                        if bridge.is_session_displayed(session_id)
                            && !unique.iter().any(|seen| Rc::ptr_eq(seen, bridge))
                        {
                            unique.push(Rc::clone(bridge));
                        }
                    }
                    unique
                };
                for bridge in displaying {
                    // A guest's pane goes away, as `win.close-pane` does; leaving
                    // an empty slot behind made a guest closed by id (sidebar
                    // Close, session manager, script, clean exit) look like a
                    // pane still waiting for a session.
                    let teardown = (!bridge.is_owned_by(session_id))
                        .then(|| notebook_for_cleanup.upgrade())
                        .flatten()
                        .map(|notebook| {
                            split_view_actions::SplitTeardown::new(
                                notebook,
                                session_bridges_for_cleanup.clone(),
                                Rc::clone(&monitoring_for_cleanup),
                                split_view_for_cleanup.clone(),
                                split_container_for_cleanup.clone(),
                            )
                        });
                    if let Some(teardown) = teardown {
                        tracing::debug!(
                            session = %session_id,
                            "on_split_cleanup: removing the closing guest's pane"
                        );
                        teardown.remove_guest_pane(&bridge, session_id);
                    } else {
                        tracing::debug!(
                            "on_split_cleanup: clearing session {} from per-session bridge",
                            session_id
                        );
                        bridge.clear_session_from_panes(session_id);
                    }
                }
                // Drop the owner's now-emptied bridge so its stale entry does not
                // linger in the map keyed by a closed session.
                session_bridges_for_cleanup.borrow_mut().remove(&session_id);
                // Clear from global split view
                split_view_for_cleanup.clear_session_from_panes(session_id);
            });
        }

        // Set up "Select Tab" callback for empty panel placeholders
        // This provides an alternative to drag-and-drop for moving sessions to split panels
        {
            let split_view_for_select = split_view.clone();
            let notebook_for_select = terminal_notebook.clone();
            let notebook_for_provider = terminal_notebook.clone();
            let notebook_for_terminal = terminal_notebook.clone();
            split_view.setup_select_tab_callback_with_provider(
                move || {
                    // Get all sessions from the notebook
                    // Only show VTE-based sessions (SSH, ZeroTrust, Local Shell)
                    // RDP/VNC/SPICE not supported in split view
                    notebook_for_provider
                        .get_all_sessions()
                        .into_iter()
                        .filter(|s| {
                            s.protocol == "ssh"
                                || s.protocol == "local"
                                || s.protocol.starts_with("zerotrust")
                        })
                        .map(|s| (s.id, s.name, s.protocol))
                        .collect()
                },
                move |panel_uuid, session_id| {
                    tracing::debug!(
                        "Select Tab callback: moving session {} to panel {}",
                        session_id,
                        panel_uuid
                    );

                    // Get terminal from notebook (not from bridge's internal map)
                    let Some(terminal) = notebook_for_terminal.get_terminal(session_id) else {
                        tracing::warn!(
                            "Select Tab callback (global): no terminal found for session {}",
                            session_id
                        );
                        return;
                    };

                    // Move the session to the panel with the terminal
                    if let Err(e) = split_view_for_select
                        .move_session_to_panel_with_terminal(panel_uuid, session_id, &terminal)
                    {
                        tracing::warn!("Failed to move session to panel: {}", e);
                        return;
                    }

                    // Get color for this pane using the new method
                    let color_index = split_view_for_select.get_pane_color(panel_uuid);

                    tracing::debug!(
                        "Select Tab callback (global): panel {} has color {:?}",
                        panel_uuid,
                        color_index
                    );

                    // Set tab color indicator
                    if let Some(color) = color_index {
                        notebook_for_select.set_tab_split_color(session_id, color);
                        split_view_for_select.set_session_color(session_id, color);
                        tracing::debug!(
                            "Select Tab callback (global): applied color {} to session {}",
                            color,
                            session_id
                        );
                    } else {
                        tracing::warn!(
                            "Select Tab callback (global): no color found for panel {}",
                            panel_uuid
                        );
                    }

                    // Note: Do NOT call switch_to_tab() here - the terminal should be
                    // displayed in the split panel, not switched to as the active tab
                },
                Rc::clone(terminal_notebook.split_colors()),
            );

            // Wire the panel buttons and panel context menu
            split_view_actions::wire_panel_action_callbacks(&split_view, &terminal_notebook);
        }

        // Set up drag-and-drop for initial pane with notebook lookup
        if let Some(initial_pane_id) = split_view.pane_ids().first().copied() {
            let notebook_for_drop = terminal_notebook.clone();
            let notebook_for_color = terminal_notebook.clone();
            split_view.setup_pane_drop_target_with_callbacks(
                initial_pane_id,
                move |session_id| {
                    let info = notebook_for_drop.get_session_info(session_id)?;
                    let terminal = notebook_for_drop.get_terminal(session_id);
                    Some((info, terminal))
                },
                move |session_id, color_index| {
                    // Set tab color indicator when session is dropped into pane
                    notebook_for_color.set_tab_split_color(session_id, color_index);
                },
            );
        }

        // Set up click handlers for focus management on global split view
        // Note: This is for the global split view; per-session bridges set up their own handlers
        {
            let split_view_for_click = split_view.clone();
            let notebook_for_click = terminal_notebook.clone();
            let sv_for_focus = split_view_for_click.clone();
            let sv_for_session = split_view_for_click.clone();
            let notebook_clone = notebook_for_click.clone();
            let sv_for_terminal = split_view_for_click.clone();

            split_view_for_click.setup_all_panel_click_handlers(move |clicked_pane_uuid| {
                // Update the bridge's focused pane state (handles all focus styling)
                sv_for_focus.set_focused_pane(Some(clicked_pane_uuid));
                // Get session_id from the clicked pane via adapter
                let session_to_switch = sv_for_session.get_pane_session(clicked_pane_uuid);
                // Switch to the tab if there's a session in this pane
                if let Some(session_id) = session_to_switch {
                    notebook_clone.switch_to_tab(session_id);
                    // Grab focus on the terminal: a press on the pane header or
                    // the scrollbar does not focus it by itself.
                    if let Some(terminal) = sv_for_terminal.get_terminal(session_id) {
                        terminal.grab_focus();
                    }
                }
            });
        }

        // Connect sidebar search with debouncing
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        sidebar.search_entry().connect_search_changed(move |entry| {
            let query = entry.text().to_string();

            // Save pre-search state on first keystroke
            if !query.is_empty() {
                sidebar_clone.save_pre_search_state();
            }

            // Check if we should debounce
            let debouncer = sidebar_clone.search_debouncer();
            if debouncer.should_proceed() {
                // Immediate search - hide spinner and filter
                sidebar_clone.hide_search_pending();
                Self::filter_connections(&state_clone, &sidebar_clone, &query);

                // Restore state if search cleared
                if query.is_empty() {
                    sidebar_clone.restore_pre_search_state();
                }
            } else {
                // Debounced - show spinner and schedule search
                sidebar_clone.show_search_pending();
                sidebar_clone.set_pending_search_query(Some(query.clone()));

                // Schedule delayed search using glib timeout
                let state_for_timeout = state_clone.clone();
                let sidebar_for_timeout = sidebar_clone.clone();
                let delay_ms = debouncer.delay().as_millis() as u32;

                glib::timeout_add_local_once(
                    std::time::Duration::from_millis(u64::from(delay_ms)),
                    move || {
                        // Only proceed if this is still the pending query
                        if let Some(pending) = sidebar_for_timeout.pending_search_query()
                            && pending == query
                        {
                            sidebar_for_timeout.hide_search_pending();
                            sidebar_for_timeout.set_pending_search_query(None);
                            Self::filter_connections(
                                &state_for_timeout,
                                &sidebar_for_timeout,
                                &pending,
                            );

                            // Restore state if search cleared
                            if pending.is_empty() {
                                sidebar_for_timeout.restore_pre_search_state();
                            }
                        }
                    },
                );
            }
        });

        // Enter in the search entry records the query in the history and opens
        // the connection the results point at: the selected one, or the first.
        let sidebar_for_history = sidebar.clone();
        let state_for_history = state.clone();
        sidebar.search_entry().connect_activate(move |entry| {
            let query = entry.text().to_string();
            if query.is_empty() {
                return;
            }
            sidebar_for_history.add_to_search_history(&query);
            // Persist to settings
            if let Ok(mut state_mut) = state_for_history.try_borrow_mut() {
                state_mut.settings_mut().ui.add_search_history(&query);
                if let Err(e) = state_mut.save_settings() {
                    tracing::warn!(?e, "Failed to save settings");
                }
            }

            // A fast typist presses Enter before the list has caught up with the
            // text: `search-changed` is itself delayed by GtkSearchEntry, and
            // the debounce above adds to that. Filter with what the entry holds
            // right now so Enter opens what was typed, not a row of the
            // previous list; clearing the pending query turns the scheduled
            // debounce timeout into a no-op. Rebuilding the list resets the
            // selection to its first row, so the connection selected before is
            // taken first and opened if the new results still hold it.
            let preferred_id = sidebar_for_history
                .get_selected_item()
                .filter(|item| !item.is_group())
                .map(|item| item.id());
            sidebar_for_history.hide_search_pending();
            sidebar_for_history.set_pending_search_query(None);
            Self::filter_connections(&state_for_history, &sidebar_for_history, &query);
            sidebar_for_history.activate_search_result(preferred_id.as_deref());
        });

        // Also add to history when search entry loses focus with non-empty query
        let sidebar_for_focus = sidebar.clone();
        let state_for_focus = state.clone();
        sidebar
            .search_entry()
            .connect_has_focus_notify(move |entry| {
                if !entry.has_focus() {
                    let query = entry.text().to_string();
                    if !query.is_empty() {
                        sidebar_for_focus.add_to_search_history(&query);
                        // Persist to settings
                        if let Ok(mut state_mut) = state_for_focus.try_borrow_mut() {
                            state_mut.settings_mut().ui.add_search_history(&query);
                            if let Err(e) = state_mut.save_settings() {
                                tracing::warn!(?e, "Failed to save settings");
                            }
                        }
                    }
                }
            });

        // Connect sidebar double-click to connect
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let notebook_clone = terminal_notebook.clone();
        let split_view_clone = split_view.clone();
        let monitoring_clone = self.monitoring.clone();
        let activity_clone_sidebar = self.activity_coordinator.clone();
        let session_bridges_for_activate = self.session_split_bridges.clone();

        // R7.5: `connect_activate` does not carry keyboard-modifier state, so a
        // capture-phase primary-button gesture records whether Shift/Ctrl was
        // held on the press that precedes activation. Capture phase without
        // claiming the sequence means it never interferes with the ListView's
        // own selection/activation or the secondary-button context-menu gesture
        // (mirrors the existing primary-button dismiss gesture in `sidebar/mod`).
        let force_new_modifier = Rc::new(std::cell::Cell::new(false));
        let modifier_gesture = gtk4::GestureClick::new();
        modifier_gesture.set_button(gtk4::gdk::BUTTON_PRIMARY);
        modifier_gesture.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let force_new_for_gesture = force_new_modifier.clone();
        modifier_gesture.connect_pressed(move |gesture, _n_press, _x, _y| {
            let modifiers = gesture.current_event_state();
            force_new_for_gesture.set(
                modifiers.contains(gtk4::gdk::ModifierType::SHIFT_MASK)
                    || modifiers.contains(gtk4::gdk::ModifierType::CONTROL_MASK),
            );
            // Observe-only: deny the sequence so the gesture never claims the
            // press and the ListView's own selection/activation handling (and
            // the TreeExpander's primary-button expand/collapse) proceed intact.
            gesture.set_state(gtk4::EventSequenceState::Denied);
        });
        sidebar.list_view().add_controller(modifier_gesture);

        sidebar
            .list_view()
            .connect_activate(move |list_view, position| {
                // Get the item at position from the tree model
                let tree_model = sidebar_clone.tree_model();
                if let Some(item) = tree_model.item(position)
                    && let Some(row) = item.downcast_ref::<gtk4::TreeListRow>()
                    && let Some(conn_item) = row
                        .item()
                        .and_then(|i| i.downcast::<crate::sidebar::ConnectionItem>().ok())
                    && conn_item.is_group()
                {
                    // Toggle expand/collapse for groups on double-click
                    row.set_expanded(!row.is_expanded());
                    // Re-select the row after toggle so it stays highlighted
                    if let Some(model) = list_view.model()
                        && let Some(sel) = model.downcast_ref::<gtk4::SingleSelection>()
                    {
                        sel.set_selected(position);
                    }
                    return;
                }
                // Read (and reset) the modifier captured on the preceding press.
                // Reset guards against a stale value on a later keyboard
                // (Enter) activation, which fires no primary-button press.
                let force_new = force_new_modifier.get();
                force_new_modifier.set(false);

                Self::connect_at_position_with_split(
                    &state_clone,
                    &sidebar_clone,
                    &notebook_clone,
                    &split_view_clone,
                    &monitoring_clone,
                    &session_bridges_for_activate,
                    position,
                    force_new,
                    Some(&activity_clone_sidebar),
                );
            });

        // Connect TabView page selection
        // With the new per-tab split architecture, GTK handles content switching
        // automatically — split views live inside TabPages, not in a global container.
        let session_bridges_for_tab = self.session_split_bridges.clone();
        let global_split_view = split_view.clone();
        let split_container_for_tab = self.split_container.clone();
        let notebook_clone = terminal_notebook.clone();
        let activity_for_tab = self.activity_coordinator.clone();
        let sessions_for_tab = terminal_notebook.sessions_map();
        terminal_notebook.tab_view().connect_notify_local(
            Some("selected-page"),
            move |tab_view, _| {
                let Some(selected_page) = tab_view.selected_page() else {
                    return;
                };
                let page_num = tab_view.page_position(&selected_page) as u32;

                // Hide legacy global containers (no longer used for content)
                global_split_view.widget().set_visible(false);
                split_container_for_tab.set_visible(false);

                // Get session ID for this page
                if let Some(session_id) = notebook_clone.get_session_id_for_page(page_num) {
                    // Clear activity monitor indicator and reset notification state
                    // but preserve split color indicators
                    activity_for_tab.on_tab_switched(session_id);
                    if let Some(page) = sessions_for_tab.borrow().get(&session_id) {
                        // libadwaita does not clear `needs-attention` on selection,
                        // and looking at the tab is the acknowledgement — so it is
                        // cleared here, unconditionally. Unlike `indicator-icon`
                        // below there is nothing else competing for it, so there is
                        // no state to preserve.
                        page.set_needs_attention(false);

                        if !notebook_clone
                            .split_colors()
                            .borrow()
                            .contains_key(&session_id)
                        {
                            page.set_indicator_icon(gio::Icon::NONE);
                            page.set_indicator_tooltip("");
                        }
                    }

                    // If session has a split bridge, focus the correct pane
                    let bridges = session_bridges_for_tab.borrow();
                    if let Some(bridge) = bridges.get(&session_id) {
                        // Focus the pane containing the selected session
                        for pane_id in bridge.pane_ids() {
                            if bridge.get_pane_session(pane_id) == Some(session_id) {
                                let _ = bridge.focus_pane(pane_id);
                                if let Some(terminal) = bridge.get_terminal(session_id) {
                                    terminal.grab_focus();
                                }
                                break;
                            }
                        }
                    } else {
                        // Regular tab — focus the terminal directly
                        if let Some(terminal) = notebook_clone.get_terminal(session_id) {
                            terminal.grab_focus();
                        }
                    }
                }
                // Welcome tab — nothing extra to do, GTK shows the content
            },
        );

        // Save window state on close and handle minimize to tray
        let state_clone = state.clone();
        let split_view_clone = split_view_for_close;
        let sidebar_clone = sidebar.clone();
        let notebook_for_close = terminal_notebook.clone();
        let tunnel_manager_for_close = self.tunnel_manager.clone();
        let monitoring_for_exit = self.monitoring.clone();
        // One-shot flag: set after the user confirms closing with open
        // sessions, so the second close() pass proceeds without re-asking.
        let force_close = Rc::new(std::cell::Cell::new(false));
        window.connect_close_request(move |win| {
            // When minimize-to-tray is enabled the window only hides and the
            // app keeps running — no confirmation needed.
            let minimize_to_tray = crate::app::tray_is_active()
                && state_clone.try_borrow().is_ok_and(|s| {
                    s.settings().ui.minimize_to_tray && s.settings().ui.enable_tray_icon
                });

            // Confirm before quitting with open sessions (GNOME HIG: protect
            // against accidental loss of active connections). Count tabless
            // external-viewer sessions too (issue #209), so a window holding
            // only external VNC/RDP/SPICE sessions still warns before quitting.
            // Detached sessions (issue #236) have no tab either, so they are
            // counted separately — closing the main window ends them too.
            let external_open = external_session_registry().map_or(0, |reg| reg.active_count());
            let open_sessions = open_session_count(
                notebook_for_close.session_count(),
                notebook_for_close.detached_count(),
                external_open,
            );
            if !minimize_to_tray && !force_close.get() && open_sessions > 0 {
                // `None` means a confirmation raised elsewhere — the quit action,
                // which is where Ctrl+Q and the tray's Quit item arrive — is
                // already asking. Either way the close is stopped here and the
                // window goes only once that question is answered.
                if let Some(dialog) = present_close_confirmation(win, open_sessions) {
                    let force_close_confirm = force_close.clone();
                    let win_weak = win.downgrade();
                    dialog.connect_response(Some("close"), move |_, _| {
                        force_close_confirm.set(true);
                        if let Some(w) = win_weak.upgrade() {
                            w.close();
                        }
                    });
                }
                return glib::Propagation::Stop;
            }

            // Save window geometry and expanded groups state. Done before the
            // tray branch below so both outcomes — hiding and really quitting —
            // persist the same thing.
            let (width, height) = win.default_size();
            let is_maximized = win.is_maximized();
            let sidebar_width = rustconn_core::config::clamp_sidebar_width(
                split_view_clone.max_sidebar_width().round() as i32,
            );

            // Save expanded groups state
            let expanded = sidebar_clone.get_expanded_groups();

            if let Ok(mut state) = state_clone.try_borrow_mut() {
                // Update expanded groups
                if let Err(e) = state.update_expanded_groups(expanded) {
                    tracing::warn!(?e, "Failed to update expanded groups");
                }

                let mut settings = state.settings().clone();
                if settings.ui.remember_window_geometry {
                    settings.ui.window_width = Some(width);
                    settings.ui.window_height = Some(height);
                    settings.ui.window_maximized = is_maximized;
                    settings.ui.sidebar_width = Some(sidebar_width);
                }

                // Persist the keyboard passthrough toggle so it survives a
                // restart (issue #274 follow-up). Read the action state from
                // the window — the action is stateful and always present.
                settings.ui.keyboard_passthrough = win
                    .lookup_action("toggle-passthrough")
                    .and_then(|a| a.state())
                    .and_then(|v| v.get::<bool>())
                    .unwrap_or(false);

                if let Err(e) = state.update_settings(settings) {
                    tracing::warn!(?e, "Failed to update settings");
                }
            }

            // Minimizing to tray only hides the window; the app keeps running,
            // so nothing below may be torn down. The decision has to happen
            // *before* that teardown: it stops every standalone SSH tunnel,
            // kills every tracked external viewer and stops every active
            // recording, and doing so on a mere hide leaves the user restoring
            // the window to find their port forwards dead and their viewer
            // windows gone, with no message. The detached-window registry
            // already carried this guard (issue #236) and states the contract
            // the rest of the teardown has to honour too: on a tray-minimize
            // every session, detached or not, keeps running.
            //
            // Deciding here also means the hide no longer depends on
            // `try_borrow_mut()` above succeeding — a failed borrow used to
            // fall through and really close the window.
            if minimize_to_tray {
                // Snapshot the session list so a subsequent force-kill from the
                // tray (or OOM, power loss) does not lose which tabs were open.
                session_restore::save_snapshot(&state_clone, &notebook_for_close);
                win.set_visible(false);
                return glib::Propagation::Stop;
            }

            // Snapshot the open sessions for the next start (issue #243). Done
            // before any teardown, while the notebook still knows what is live.
            session_restore::save_snapshot(&state_clone, &notebook_for_close);

            // Drop the portable store's passphrase and derived key. Below the
            // tray decision on purpose: a tray-minimize leaves the application
            // running, and locking the store there would make every connection
            // opened from the tray prompt for a passphrase the user had already
            // given. On a real close it is the synchronous half of the lock that
            // matters — see `AppState::lock_portable_store`.
            if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                state_mut.lock_portable_store();
            }

            // Flush all active session recordings before shutdown. Ahead of the
            // teardown below, which drops the PTY relays a recorder writes through.
            notebook_for_close.flush_active_recordings();

            // Stop all standalone SSH tunnels. Separate from the session teardown
            // because a standalone forward belongs to the window, not to a session.
            tunnel_manager_for_close.borrow_mut().stop_all();

            // Stop every monitoring collector so none outlives the window.
            monitoring_for_exit.stop_all();

            // End every live session: external viewers, session children, detached
            // windows (issues #209, #304, #236). One call, so this path cannot
            // drift from the other two the way it did before.
            shutdown_sessions_for_exit(&notebook_for_close);

            glib::Propagation::Proceed
        });
    }
}
