//! Split view window actions (split, unsplit, resize, navigate panes)
//!
//! Extracted from `window/mod.rs` to reduce module complexity.

use super::*;

impl MainWindow {
    pub(crate) fn setup_split_view_actions(&self, window: &adw::ApplicationWindow) {
        // Helper function to get or create a split bridge for a session
        // Each tab maintains its own independent split layout
        // A session gets its own bridge when it initiates a split.
        // If the session is already displayed in another bridge, we still create
        // a new bridge for it (the session will be moved to the new bridge).
        fn get_or_create_session_bridge(
            session_id: Uuid,
            session_split_bridges: &SessionSplitBridges,
            color_pool: &SharedColorPool,
        ) -> Rc<SplitViewBridge> {
            let mut bridges = session_split_bridges.borrow_mut();
            // Check if this session already owns a bridge
            if let Some(bridge) = bridges.get(&session_id) {
                // Session already has its own bridge - use it
                tracing::debug!(
                    "get_or_create_session_bridge: REUSING existing bridge for session {:?}, \
                     pool_ptr={:p}, pool_allocated={}",
                    session_id,
                    &*color_pool.borrow(),
                    color_pool.borrow().allocated_count()
                );
                bridge.clone()
            } else {
                // Create a new bridge for this session with the shared color pool
                // This ensures different split containers get different colors
                tracing::debug!(
                    "get_or_create_session_bridge: CREATING new bridge for session {:?}, \
                     pool_ptr={:p}, pool_allocated={}",
                    session_id,
                    &*color_pool.borrow(),
                    color_pool.borrow().allocated_count()
                );
                let new_bridge = Rc::new(SplitViewBridge::with_color_pool(Rc::clone(color_pool)));
                bridges.insert(session_id, new_bridge.clone());
                new_bridge
            }
        }

        // Helper closure that refreshes the broadcast toggle button state.
        // Cloned into every callback that may change a split's session count
        // (split, unsplit, close pane, move to panel) so the toggle accurately
        // reflects the active tab's split layout.
        //
        // It also drives a one-shot discoverability toast: the very first
        // time a split has ≥2 active panels in this app session we tell the
        // user that broadcast is now an option. The flag lives on
        // `MainWindow::broadcast_hint_shown` and is not persisted.
        let refresh_broadcast: Rc<dyn Fn()> = {
            let window_weak = window.downgrade();
            let notebook = self.terminal_notebook.clone();
            let bridges = self.session_split_bridges.clone();
            let toggle = self.broadcast_toggle.clone();
            let toast_overlay = self.toast_overlay.clone();
            let hint_shown = self.broadcast_hint_shown.clone();
            Rc::new(move || {
                if let Some(win) = window_weak.upgrade() {
                    super::navigation_actions::refresh_broadcast_toggle(
                        &win, &notebook, &bridges, &toggle,
                    );

                    // First-time hint: only when the toggle just became
                    // visible (i.e. a split with ≥2 active panels exists)
                    // and we haven't shown the hint yet this session.
                    if !hint_shown.get() && toggle.is_visible() {
                        hint_shown.set(true);
                        let hint = crate::i18n::i18n(
                            "Tip: press Ctrl+Shift+B or use the Broadcast button to mirror keystrokes across split panels",
                        );
                        toast_overlay
                            .widget()
                            .add_toast(adw::Toast::builder().title(&hint).timeout(6).build());
                    }
                }
            })
        };

        // Split horizontal action
        let split_horizontal_action = gio::SimpleAction::new("split-horizontal", None);
        let session_bridges = self.session_split_bridges.clone();
        let notebook_for_split_h = self.terminal_notebook.clone();
        let split_container_h = self.split_container.clone();
        let global_split_view_h = self.split_view.clone();
        let color_pool_h = self.global_color_pool.clone();
        let window_weak_h = window.downgrade();
        let monitoring_h = self.monitoring.clone();
        let refresh_broadcast_h = refresh_broadcast.clone();
        split_horizontal_action.connect_activate(move |_, _| {
            // Get current active session before splitting
            let Some(current_session) = notebook_for_split_h.get_active_session_id() else {
                return; // No active session to split
            };

            // Gate split on the session's eligibility rather than a hardcoded
            // protocol allowlist: VTE terminals and in-process embedded viewers
            // (RDP/VNC/SPICE) are Embeddable; external-process viewers are declined.
            match notebook_for_split_h.split_eligibility(current_session) {
                crate::terminal::SplitEligibility::Embeddable => {}
                crate::terminal::SplitEligibility::ExternalViewer => {
                    tracing::debug!(
                        "split-horizontal: session {:?} uses an external viewer, declining split",
                        current_session
                    );
                    if let Some(win) = window_weak_h.upgrade() {
                        crate::toast::show_toast_on_window(
                            &win,
                            &crate::i18n::i18n("Split view is not available for external-viewer sessions. Switch this connection to embedded mode to use split."),
                            crate::toast::ToastType::Warning,
                        );
                    }
                    return;
                }
                crate::terminal::SplitEligibility::None => return,
            }

            tracing::debug!("split-horizontal: splitting session {:?}", current_session);

            // Get or create a split bridge for this session (with shared color pool)
            let split_view =
                get_or_create_session_bridge(current_session, &session_bridges, &color_pool_h);

            // Wire the content provider so the bridge can place any session's
            // display widget (VTE terminal or embedded RDP/VNC/SPICE viewer)
            // through the uniform content-widget path.
            let notebook_for_content_provider_h = notebook_for_split_h.clone();
            split_view.set_content_provider(Rc::new(move |sid| {
                notebook_for_content_provider_h.get_session_display_widget(sid)
            }));

            // Check if this is the first split (bridge has only 1 panel)
            // If bridge already has multiple panels, we don't need to show the current session
            // because restore_panel_contents() already restored all terminals
            let is_first_split = split_view.pane_count() == 1;

            // Clone for close callback
            let sv_for_close = split_view.clone();
            if let Some((new_pane_id, new_color_index, original_color_index)) = split_view
                .split_with_close_callback(SplitDirection::Horizontal, move || {
                    let _ = sv_for_close.close_pane();
                })
            {
                tracing::debug!(
                    "split-horizontal: session {:?} got original_color={}, new_color={}, \
                     is_first_split={}",
                    current_session,
                    original_color_index,
                    new_color_index,
                    is_first_split
                );

                let notebook = notebook_for_split_h.clone();
                let notebook_for_drop = notebook_for_split_h.clone();
                let sv_for_click = split_view.clone();

                // Per spec: Split transforms current tab into Container Tab
                // Only show current session in the original pane if this is the FIRST split
                // For subsequent splits, restore_panel_contents() already restored all terminals
                if is_first_split {
                    // Ensure session is registered in split_view
                    if let Some(info) = notebook_for_split_h.get_session_info(current_session) {
                        split_view.add_session(info);
                    }
                    // Show in the focused (original) pane
                    let _ = split_view.show_session(current_session);

                    // Use the original pane's color (properly allocated during split)
                    split_view.set_session_color(current_session, original_color_index);
                    notebook_for_split_h.set_tab_split_color(current_session, original_color_index);
                    tracing::debug!(
                        "split-horizontal: applied color {} to tab for session {:?}",
                        original_color_index,
                        current_session
                    );

                    // Suspend monitoring — bar is not visible in split view
                    monitoring_h.suspend_monitoring(current_session);
                }

                // Place split view widget inside the TabPage via TabPageContainer
                split_view.widget().set_vexpand(true);
                split_view.widget().set_hexpand(true);
                notebook_for_split_h.switch_tab_to_split(current_session, split_view.widget());

                // Also hide global split view (we're using per-tab now)
                global_split_view_h.widget().set_visible(false);
                split_container_h.set_visible(false);

                // Setup drop target for the new (empty) pane
                let sv_for_drop = split_view.clone();
                split_view.setup_pane_drop_target_with_callbacks(
                    new_pane_id,
                    move |session_id| {
                        let info = notebook.get_session_info(session_id)?;
                        let terminal = notebook.get_terminal(session_id);
                        Some((info, terminal))
                    },
                    move |session_id, color_index| {
                        // Store session color in split_view for tracking
                        sv_for_drop.set_session_color(session_id, color_index);
                        // Set tab color indicator when session is dropped into pane
                        notebook_for_drop.set_tab_split_color(session_id, color_index);
                    },
                );

                // Setup click handlers for ALL panes (both original and new)
                // This ensures focus rectangle moves correctly when clicking any pane
                let sv_for_focus = sv_for_click.clone();
                let sv_for_session = sv_for_click.clone();
                let sv_for_terminal = sv_for_click.clone();
                sv_for_click.setup_all_panel_click_handlers(move |clicked_pane_uuid| {
                    // Update the bridge's focused pane state (handles all focus styling)
                    sv_for_focus.set_focused_pane(Some(clicked_pane_uuid));
                    // Get session_id from the clicked pane via adapter
                    let session_to_focus = sv_for_session.get_pane_session(clicked_pane_uuid);
                    // Grab focus on the terminal in the clicked pane.
                    // Do NOT call switch_to_tab() here — the split widget lives on the
                    // split-owner's TabPage. Switching to another session's tab would
                    // navigate away from the split widget and make the content disappear.
                    if let Some(session_id) = session_to_focus
                        && let Some(widget) = sv_for_terminal.content_widget(session_id)
                    {
                        widget.grab_focus();
                    }
                });

                // Setup select tab callback for this per-session bridge
                let split_view_for_select = split_view.clone();
                let notebook_for_select = notebook_for_split_h.clone();
                let notebook_for_provider = notebook_for_split_h.clone();
                let notebook_for_terminal = notebook_for_split_h.clone();
                let notebook_for_placeholder_h = notebook_for_split_h.clone();
                // Clone session_bridges so we can register the new session in the map
                let session_bridges_for_select = session_bridges.clone();
                // Clone for clearing from previous split
                let session_bridges_for_clear = session_bridges.clone();
                // Clone for provider closure
                let split_view_for_provider = split_view.clone();
                let monitoring_for_select_h = monitoring_h.clone();
                let split_colors_h = Rc::clone(notebook_for_split_h.split_colors());
                // Refresh broadcast toggle once a new session is placed via Select Tab —
                // until this point the bridge has 1 active session and the toggle is hidden.
                let refresh_broadcast_select_h = refresh_broadcast_h.clone();
                // Notebook clone for wiring broadcast on the freshly-placed session
                // (needed because the wired commit handler also calls send_text_to_session).
                let notebook_for_broadcast_h = notebook_for_split_h.clone();
                split_view.setup_select_tab_callback_with_provider(
                    move || {
                        // Get all sessions from the notebook, excluding those already in THIS split.
                        // Include VTE terminals and in-process embedded viewers (Embeddable);
                        // external-process viewers are excluded via eligibility (R4.3).
                        notebook_for_provider
                            .get_all_sessions()
                            .into_iter()
                            .filter(|s| {
                                matches!(
                                    notebook_for_provider.split_eligibility(s.id),
                                    crate::terminal::SplitEligibility::Embeddable
                                )
                            })
                            .map(|s| (s.id, s.name, s.protocol))
                            .filter(|(id, _, _)| !split_view_for_provider.is_session_displayed(*id))
                            .collect()
                    },
                    move |panel_uuid, session_id| {
                        tracing::debug!(
                            "Select Tab callback (horizontal): moving session {} to panel {}",
                            session_id,
                            panel_uuid
                        );

                        // First, clear this session from any previous split view
                        {
                            let bridges = session_bridges_for_clear.borrow();
                            for (other_session_id, other_bridge) in bridges.iter() {
                                // Skip if this is the same bridge we're adding to
                                if Rc::ptr_eq(other_bridge, &split_view_for_select) {
                                    continue;
                                }
                                // Check if this session is displayed in another bridge
                                if other_bridge.is_session_displayed(session_id) {
                                    tracing::debug!(
                                        "Select Tab callback (horizontal): clearing session {} \
                                         from previous split (owner: {})",
                                        session_id,
                                        other_session_id
                                    );
                                    other_bridge.clear_session_from_panes(session_id);
                                    // Clear the old tab color
                                    notebook_for_select.clear_tab_split_color(session_id);
                                    break;
                                }
                            }
                        }

                        // Resolve the session's display widget from the notebook
                        // (terminal or embedded viewer) — not from the bridge's
                        // internal map.
                        let Some(content) =
                            notebook_for_terminal.get_session_display_widget(session_id)
                        else {
                            tracing::warn!(
                                "Select Tab callback (horizontal): no content widget for session {}",
                                session_id
                            );
                            return;
                        };

                        // Move the session to the panel with its content widget.
                        // This returns the color index on success
                        match split_view_for_select
                            .move_session_to_panel(panel_uuid, session_id, &content)
                        {
                            Ok(color_index) => {
                                // Register this session in session_split_bridges
                                session_bridges_for_select
                                    .borrow_mut()
                                    .insert(session_id, split_view_for_select.clone());

                                // Set tab color indicator using the color from the panel
                                notebook_for_select.set_tab_split_color(session_id, color_index);

                                // Remove the moved session's standalone tab — it
                                // now lives in this split, so a placeholder tab
                                // would only clutter the tab bar and Tab Overview.
                                notebook_for_placeholder_h.park_session_tab(session_id);

                                // Suspend monitoring — session is now in split view
                                monitoring_for_select_h.suspend_monitoring(session_id);

                                tracing::debug!(
                                    "Select Tab callback (horizontal): moved session {} to panel {} with color {}",
                                    session_id,
                                    panel_uuid,
                                    color_index
                                );

                                // Layout changed — the bridge now has ≥2 active panels,
                                // so the broadcast toggle should appear in the header bar.
                                refresh_broadcast_select_h();

                                // If broadcast is already on, the freshly-placed session
                                // must be wired immediately — otherwise it would silently
                                // miss keystroke mirroring until broadcast is toggled off
                                // and back on.
                                if split_view_for_select.broadcast_active.get() {
                                    super::navigation_actions::wire_broadcast_for_session(
                                        &split_view_for_select,
                                        &notebook_for_broadcast_h,
                                        session_id,
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Failed to move session to panel: {}", e);
                            }
                        }

                        // Note: Do NOT call switch_to_tab() here - the terminal should be
                        // displayed in the split panel, not switched to as the active tab
                    },
                    split_colors_h,
                );

                // Setup close panel callback for empty panel close buttons
                let split_view_for_close = split_view.clone();
                split_view.setup_close_panel_callback(move |pane_uuid| {
                    // Focus the pane first so close_pane() closes the correct one
                    split_view_for_close.set_focused_pane(Some(pane_uuid));

                    // Update focus styling via the adapter
                    if let Some(panel_id) = split_view_for_close.get_panel_id_for_uuid(pane_uuid)
                        && let Err(e) = split_view_for_close.adapter_set_focus(panel_id) {
                            tracing::warn!("Failed to set focus on panel: {}", e);
                        }
                });
            }

            // Layout changed — refresh the broadcast toggle so it reflects the
            // new session count of the active tab's split.
            refresh_broadcast_h();
        });
        window.add_action(&split_horizontal_action);

        // Split vertical action
        let split_vertical_action = gio::SimpleAction::new("split-vertical", None);
        let session_bridges_v = self.session_split_bridges.clone();
        let notebook_for_split_v = self.terminal_notebook.clone();
        let split_container_v = self.split_container.clone();
        let global_split_view_v = self.split_view.clone();
        let color_pool_v = self.global_color_pool.clone();
        let window_weak_v = window.downgrade();
        let monitoring_v = self.monitoring.clone();
        let refresh_broadcast_v = refresh_broadcast.clone();
        split_vertical_action.connect_activate(move |_, _| {
            // Get current active session before splitting
            let Some(current_session) = notebook_for_split_v.get_active_session_id() else {
                return; // No active session to split
            };

            // Gate split on the session's eligibility rather than a hardcoded
            // protocol allowlist: VTE terminals and in-process embedded viewers
            // (RDP/VNC/SPICE) are Embeddable; external-process viewers are declined.
            match notebook_for_split_v.split_eligibility(current_session) {
                crate::terminal::SplitEligibility::Embeddable => {}
                crate::terminal::SplitEligibility::ExternalViewer => {
                    tracing::debug!(
                        "split-vertical: session {:?} uses an external viewer, declining split",
                        current_session
                    );
                    if let Some(win) = window_weak_v.upgrade() {
                        crate::toast::show_toast_on_window(
                            &win,
                            &crate::i18n::i18n("Split view is not available for external-viewer sessions. Switch this connection to embedded mode to use split."),
                            crate::toast::ToastType::Warning,
                        );
                    }
                    return;
                }
                crate::terminal::SplitEligibility::None => return,
            }

            tracing::debug!("split-vertical: splitting session {:?}", current_session);

            // Get or create a split bridge for this session (with shared color pool)
            let split_view =
                get_or_create_session_bridge(current_session, &session_bridges_v, &color_pool_v);

            // Wire the content provider so the bridge can place any session's
            // display widget (VTE terminal or embedded RDP/VNC/SPICE viewer)
            // through the uniform content-widget path.
            let notebook_for_content_provider_v = notebook_for_split_v.clone();
            split_view.set_content_provider(Rc::new(move |sid| {
                notebook_for_content_provider_v.get_session_display_widget(sid)
            }));

            // Check if this is the first split (bridge has only 1 panel)
            // If bridge already has multiple panels, we don't need to show the current session
            // because restore_panel_contents() already restored all terminals
            let is_first_split = split_view.pane_count() == 1;

            // Clone for close callback
            let sv_for_close = split_view.clone();
            if let Some((new_pane_id, new_color_index, original_color_index)) = split_view
                .split_with_close_callback(SplitDirection::Vertical, move || {
                    let _ = sv_for_close.close_pane();
                })
            {
                tracing::debug!(
                    "split-vertical: session {:?} got original_color={}, new_color={}, \
                     is_first_split={}",
                    current_session,
                    original_color_index,
                    new_color_index,
                    is_first_split
                );

                let notebook = notebook_for_split_v.clone();
                let notebook_for_drop = notebook_for_split_v.clone();
                let sv_for_click = split_view.clone();

                // Per spec: Split transforms current tab into Container Tab
                // Only show current session in the original pane if this is the FIRST split
                // For subsequent splits, restore_panel_contents() already restored all terminals
                if is_first_split {
                    // Ensure session is registered in split_view
                    if let Some(info) = notebook_for_split_v.get_session_info(current_session) {
                        split_view.add_session(info);
                    }
                    // Show in the focused (original) pane
                    let _ = split_view.show_session(current_session);

                    // Use the original pane's color (properly allocated during split)
                    split_view.set_session_color(current_session, original_color_index);
                    notebook_for_split_v.set_tab_split_color(current_session, original_color_index);
                    tracing::debug!(
                        "split-vertical: applied color {} to tab for session {:?}",
                        original_color_index,
                        current_session
                    );

                    // Suspend monitoring — bar is not visible in split view
                    monitoring_v.suspend_monitoring(current_session);
                }

                // Place split view widget inside the TabPage via TabPageContainer
                split_view.widget().set_vexpand(true);
                split_view.widget().set_hexpand(true);
                notebook_for_split_v.switch_tab_to_split(current_session, split_view.widget());

                // Also hide global split view (we're using per-tab now)
                global_split_view_v.widget().set_visible(false);
                split_container_v.set_visible(false);

                // Setup drop target for the new (empty) pane
                let sv_for_drop = split_view.clone();
                split_view.setup_pane_drop_target_with_callbacks(
                    new_pane_id,
                    move |session_id| {
                        let info = notebook.get_session_info(session_id)?;
                        let terminal = notebook.get_terminal(session_id);
                        Some((info, terminal))
                    },
                    move |session_id, color_index| {
                        // Store session color in split_view for tracking
                        sv_for_drop.set_session_color(session_id, color_index);
                        // Set tab color indicator when session is dropped into pane
                        notebook_for_drop.set_tab_split_color(session_id, color_index);
                    },
                );

                // Setup click handlers for ALL panes (both original and new)
                // This ensures focus rectangle moves correctly when clicking any pane
                let sv_for_focus = sv_for_click.clone();
                let sv_for_session = sv_for_click.clone();
                let sv_for_terminal = sv_for_click.clone();
                sv_for_click.setup_all_panel_click_handlers(move |clicked_pane_uuid| {
                    // Update the bridge's focused pane state (handles all focus styling)
                    sv_for_focus.set_focused_pane(Some(clicked_pane_uuid));
                    // Get session_id from the clicked pane via adapter
                    let session_to_focus = sv_for_session.get_pane_session(clicked_pane_uuid);
                    // Grab focus on the terminal in the clicked pane.
                    // Do NOT call switch_to_tab() here — the split widget lives on the
                    // split-owner's TabPage. Switching to another session's tab would
                    // navigate away from the split widget and make the content disappear.
                    if let Some(session_id) = session_to_focus
                        && let Some(widget) = sv_for_terminal.content_widget(session_id)
                    {
                        widget.grab_focus();
                    }
                });

                // Setup select tab callback for this per-session bridge
                let split_view_for_select = split_view.clone();
                let notebook_for_select = notebook_for_split_v.clone();
                let notebook_for_provider = notebook_for_split_v.clone();
                let notebook_for_terminal = notebook_for_split_v.clone();
                // Clone session_bridges so we can register the new session in the map
                let session_bridges_for_select = session_bridges_v.clone();
                // Clone for clearing from previous split
                let session_bridges_for_clear = session_bridges_v.clone();
                // Clone for provider closure
                let split_view_for_provider = split_view.clone();
                let monitoring_for_select_v = monitoring_v.clone();
                let split_colors_v = Rc::clone(notebook_for_split_v.split_colors());
                let notebook_for_placeholder_v = notebook_for_split_v.clone();
                // Refresh broadcast toggle once a new session is placed via Select Tab —
                // until this point the bridge has 1 active session and the toggle is hidden.
                let refresh_broadcast_select_v = refresh_broadcast_v.clone();
                // Notebook clone for wiring broadcast on the freshly-placed session
                // (needed because the wired commit handler also calls send_text_to_session).
                let notebook_for_broadcast_v = notebook_for_split_v.clone();
                split_view.setup_select_tab_callback_with_provider(
                    move || {
                        // Get all sessions from the notebook, excluding those already in THIS split.
                        // Include VTE terminals and in-process embedded viewers (Embeddable);
                        // external-process viewers are excluded via eligibility (R4.3).
                        notebook_for_provider
                            .get_all_sessions()
                            .into_iter()
                            .filter(|s| {
                                matches!(
                                    notebook_for_provider.split_eligibility(s.id),
                                    crate::terminal::SplitEligibility::Embeddable
                                )
                            })
                            .map(|s| (s.id, s.name, s.protocol))
                            .filter(|(id, _, _)| !split_view_for_provider.is_session_displayed(*id))
                            .collect()
                    },
                    move |panel_uuid, session_id| {
                        tracing::debug!(
                            "Select Tab callback (vertical): moving session {} to panel {}",
                            session_id,
                            panel_uuid
                        );

                        // First, clear this session from any previous split view
                        {
                            let bridges = session_bridges_for_clear.borrow();
                            for (other_session_id, other_bridge) in bridges.iter() {
                                // Skip if this is the same bridge we're adding to
                                if Rc::ptr_eq(other_bridge, &split_view_for_select) {
                                    continue;
                                }
                                // Check if this session is displayed in another bridge
                                if other_bridge.is_session_displayed(session_id) {
                                    tracing::debug!(
                                        "Select Tab callback (vertical): clearing session {} \
                                         from previous split (owner: {})",
                                        session_id,
                                        other_session_id
                                    );
                                    other_bridge.clear_session_from_panes(session_id);
                                    // Clear the old tab color
                                    notebook_for_select.clear_tab_split_color(session_id);
                                    break;
                                }
                            }
                        }

                        // Resolve the session's display widget from the notebook
                        // (terminal or embedded viewer) — not from the bridge's
                        // internal map.
                        let Some(content) =
                            notebook_for_terminal.get_session_display_widget(session_id)
                        else {
                            tracing::warn!(
                                "Select Tab callback (vertical): no content widget for session {}",
                                session_id
                            );
                            return;
                        };

                        // Move the session to the panel with its content widget.
                        // This returns the color index on success
                        match split_view_for_select
                            .move_session_to_panel(panel_uuid, session_id, &content)
                        {
                            Ok(color_index) => {
                                // Register this session in session_split_bridges
                                session_bridges_for_select
                                    .borrow_mut()
                                    .insert(session_id, split_view_for_select.clone());

                                // Set tab color indicator using the color from the panel
                                notebook_for_select.set_tab_split_color(session_id, color_index);

                                // Remove the moved session's standalone tab — it
                                // now lives in this split, so a placeholder tab
                                // would only clutter the tab bar and Tab Overview.
                                notebook_for_placeholder_v.park_session_tab(session_id);

                                // Suspend monitoring — session is now in split view
                                monitoring_for_select_v.suspend_monitoring(session_id);

                                tracing::debug!(
                                    "Select Tab callback (vertical): moved session {} to panel {} with color {}",
                                    session_id,
                                    panel_uuid,
                                    color_index
                                );

                                // Layout changed — the bridge now has ≥2 active panels,
                                // so the broadcast toggle should appear in the header bar.
                                refresh_broadcast_select_v();

                                // If broadcast is already on, the freshly-placed session
                                // must be wired immediately — otherwise it would silently
                                // miss keystroke mirroring until broadcast is toggled off
                                // and back on.
                                if split_view_for_select.broadcast_active.get() {
                                    super::navigation_actions::wire_broadcast_for_session(
                                        &split_view_for_select,
                                        &notebook_for_broadcast_v,
                                        session_id,
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Failed to move session to panel: {}", e);
                            }
                        }

                        // Note: Do NOT call switch_to_tab() here - the terminal should be
                        // displayed in the split panel, not switched to as the active tab
                    },
                    split_colors_v,
                );

                // Setup close panel callback for empty panel close buttons
                let split_view_for_close = split_view.clone();
                split_view.setup_close_panel_callback(move |pane_uuid| {
                    // Focus the pane first so close_pane() closes the correct one
                    split_view_for_close.set_focused_pane(Some(pane_uuid));

                    // Update focus styling via the adapter
                    if let Some(panel_id) = split_view_for_close.get_panel_id_for_uuid(pane_uuid)
                        && let Err(e) = split_view_for_close.adapter_set_focus(panel_id) {
                            tracing::warn!("Failed to set focus on panel: {}", e);
                        }
                });
            }

            // Layout changed — refresh broadcast toggle.
            refresh_broadcast_v();
        });
        window.add_action(&split_vertical_action);

        // Close pane action
        let close_pane_action = gio::SimpleAction::new("close-pane", None);
        let session_bridges_close = self.session_split_bridges.clone();
        let notebook_for_close = self.terminal_notebook.clone();
        let split_view_for_close = self.split_view.clone();
        let split_container_close = self.split_container.clone();
        let monitoring_close = self.monitoring.clone();
        let refresh_broadcast_close = refresh_broadcast.clone();
        close_pane_action.connect_activate(move |_, _| {
            // Find the bridge for the current session and close its focused pane
            if let Some(session_id) = notebook_for_close.get_active_session_id() {
                let bridges = session_bridges_close.borrow();
                if let Some(bridge) = bridges.get(&session_id) {
                    // Get the session in the focused pane before closing
                    let focused_session = bridge.get_focused_session();

                    tracing::debug!(
                        "close-pane: closing focused pane, focused_session={:?}, \
                         pane_count_before={}",
                        focused_session,
                        bridge.pane_count()
                    );

                    match bridge.close_pane() {
                        Ok(should_close_split) => {
                            // Clear tab color for the session that was in the closed pane
                            if let Some(sess_id) = focused_session {
                                notebook_for_close.clear_tab_split_color(sess_id);
                            }

                            // Check if we should close the split view
                            // This happens when: no panels remain, no sessions remain,
                            // or only one panel with one session remains
                            let remaining_sessions: Vec<Uuid> = bridge
                                .pane_ids()
                                .iter()
                                .filter_map(|&pane_id| bridge.get_pane_session(pane_id))
                                .collect();

                            let pane_count = bridge.pane_count();
                            let is_empty = bridge.is_empty();

                            tracing::debug!(
                                "close-pane: after close - should_close_split={}, pane_count={}, \
                                 is_empty={}, remaining_sessions={:?}",
                                should_close_split,
                                pane_count,
                                is_empty,
                                remaining_sessions
                            );

                            let should_unsplit = should_close_split
                                || is_empty
                                || (pane_count == 1 && remaining_sessions.len() == 1);

                            tracing::debug!(
                                "close-pane: should_unsplit={} (should_close_split={} || \
                                 is_empty={} || (pane_count==1 && remaining==1)={})",
                                should_unsplit,
                                should_close_split,
                                is_empty,
                                pane_count == 1 && remaining_sessions.len() == 1
                            );

                            if should_unsplit {
                                // Close split view and show remaining session as regular tab
                                tracing::debug!(
                                    "close-pane: closing split view for session {}, \
                                     remaining_sessions={:?}",
                                    session_id,
                                    remaining_sessions
                                );

                                // Clear tab colors for all remaining sessions
                                for sess_id in &remaining_sessions {
                                    notebook_for_close.clear_tab_split_color(*sess_id);
                                    // Reparent terminal back to TabView
                                    notebook_for_close.reparent_terminal_to_tab(*sess_id);
                                    // Resume monitoring if it was suspended
                                    if monitoring_close.is_suspended(*sess_id)
                                        && let Some(container) =
                                            notebook_for_close.get_session_container(*sess_id)
                                    {
                                        monitoring_close.resume_monitoring(*sess_id, &container);
                                    }
                                }

                                // Hide split view and show TabView
                                bridge.widget().set_visible(false);
                                split_view_for_close.widget().set_visible(false);
                                split_container_close.set_visible(false);
                                notebook_for_close.show_tab_view_content();

                                // Clear tab color for the main session too
                                notebook_for_close.clear_tab_split_color(session_id);
                            } else {
                                // Multiple panels remain - restore terminal content
                                bridge.restore_panel_contents();
                            }

                            // Terminate the session whose pane was just closed —
                            // with Option B it has no standalone tab to fall back
                            // to, so closing the pane closes the session. Drop the
                            // `bridges` borrow first: `close_session` → close-page
                            // → `on_split_cleanup` takes a mutable borrow of the
                            // same map and would otherwise panic (BorrowMutError).
                            drop(bridges);
                            if let Some(sess_id) = focused_session {
                                notebook_for_close.close_session(sess_id);
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to close pane: {}", e);
                        }
                    }
                }
            }

            // Layout changed — refresh broadcast toggle.
            refresh_broadcast_close();
        });
        window.add_action(&close_pane_action);

        // Focus next pane action
        let focus_next_pane_action = gio::SimpleAction::new("focus-next-pane", None);
        let session_bridges_focus = self.session_split_bridges.clone();
        let notebook_for_focus = self.terminal_notebook.clone();
        focus_next_pane_action.connect_activate(move |_, _| {
            if let Some(session_id) = notebook_for_focus.get_active_session_id() {
                let bridges = session_bridges_focus.borrow();
                if let Some(bridge) = bridges.get(&session_id) {
                    let _ = bridge.focus_next_pane();
                }
            }
        });
        window.add_action(&focus_next_pane_action);

        // Unsplit session action - moves session from split pane to its own tab
        let unsplit_session_action =
            gio::SimpleAction::new("unsplit-session", Some(glib::VariantTy::STRING));
        let session_bridges_unsplit = self.session_split_bridges.clone();
        let notebook_for_unsplit = self.terminal_notebook.clone();
        let split_container_unsplit = self.split_container.clone();
        let monitoring_unsplit = self.monitoring.clone();
        unsplit_session_action.connect_activate(move |_, param| {
            if let Some(param) = param
                && let Some(session_id_str) = param.get::<String>()
                && let Ok(session_id) = Uuid::parse_str(&session_id_str)
            {
                // Find the bridge containing this session
                let bridges = session_bridges_unsplit.borrow();
                for bridge in bridges.values() {
                    if bridge.is_session_displayed(session_id) {
                        // Clear session from split pane
                        bridge.clear_session_from_panes(session_id);

                        // Move terminal back to TabView
                        notebook_for_unsplit.reparent_terminal_to_tab(session_id);

                        // Clear tab color indicator
                        notebook_for_unsplit.clear_tab_split_color(session_id);

                        // Resume monitoring if it was suspended
                        if monitoring_unsplit.is_suspended(session_id)
                            && let Some(container) =
                                notebook_for_unsplit.get_session_container(session_id)
                        {
                            monitoring_unsplit.resume_monitoring(session_id, &container);
                        }

                        // Check if any sessions remain in this split view
                        let has_sessions_in_split = bridge
                            .pane_ids()
                            .iter()
                            .any(|&pane_id| bridge.get_pane_session(pane_id).is_some());

                        if !has_sessions_in_split {
                            // No sessions in split view - hide it
                            bridge.widget().set_visible(false);
                            split_container_unsplit.set_visible(false);
                            notebook_for_unsplit.show_tab_view_content();
                        }
                        break;
                    }
                }
            }
        });
        window.add_action(&unsplit_session_action);
    }
}
