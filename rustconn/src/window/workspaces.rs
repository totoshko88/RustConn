//! Workspace profile management UI integration
//!
//! Connects the workspace manager dialog to the application state and
//! provides "Save current" and "Open workspace" functionality.

use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use rustconn_core::models::{WorkspaceEntry, WorkspaceProfile, WorkspaceSplitLayout};

use crate::dialogs::WorkspaceManagerDialog;
use crate::i18n::{i18n, i18n_f};
use crate::state::SharedAppState;
use crate::toast::{ToastType, show_toast_on_window};
use crate::window::types::{SessionSplitBridges, SharedMonitoring, SharedNotebook, SharedSidebar};

/// Shows the workspace manager dialog
#[expect(
    clippy::too_many_arguments,
    reason = "orchestration entry point — each dependency is distinct and required"
)]
pub fn show_workspace_manager(
    window: &gtk4::Window,
    state: SharedAppState,
    notebook: SharedNotebook,
    sidebar: SharedSidebar,
    monitoring: SharedMonitoring,
    session_split_bridges: SessionSplitBridges,
    split_view: super::types::SharedSplitView,
    activity: super::types::SharedActivityCoordinator,
) {
    let dialog = WorkspaceManagerDialog::new(None);

    // Provider: fetch workspace profiles from state
    let state_for_provider = state.clone();
    dialog.set_provider(move || {
        if let Ok(state_ref) = state_for_provider.try_borrow() {
            state_ref
                .list_workspace_profiles()
                .iter()
                .map(|ws| (ws.id, ws.name.clone(), ws.entry_count()))
                .collect()
        } else {
            Vec::new()
        }
    });

    // Open callback: connect all entries in the workspace, then restore the
    // saved split layout (if any) via the active window's split machinery.
    let state_for_open = state.clone();
    let notebook_for_open = notebook.clone();
    let sidebar_for_open = sidebar.clone();
    let monitoring_for_open = monitoring.clone();
    let split_view_for_open = split_view.clone();
    let activity_for_open = activity.clone();
    let session_bridges_for_open = session_split_bridges.clone();
    let window_for_open = window.downgrade();
    dialog.set_on_open(move |workspace_id| {
        let profile = if let Ok(state_ref) = state_for_open.try_borrow() {
            state_ref.get_workspace_profile(workspace_id).cloned()
        } else {
            None
        };
        if let Some(profile) = profile {
            // Determine the guest connection_ids before starting connections.
            // Use `split_guests` if available (multi-panel), fall back to
            // `split_guest_entry_index` for backward compat with old profiles.
            let guest_connection_ids: Vec<uuid::Uuid> = if !profile.split_layout.split_guests.is_empty() {
                profile.split_layout.split_guests.iter()
                    .filter_map(|&idx| profile.entries.get(idx))
                    .map(|e| e.connection_id)
                    .collect()
            } else if let Some(idx) = profile.split_layout.split_guest_entry_index {
                profile.entries.get(idx).map(|e| vec![e.connection_id]).unwrap_or_default()
            } else {
                Vec::new()
            };

            // Determine if we need to defer apply_layout until the owner
            // session's tab is created. For multi-panel splits (3+) the owner
            // might not be the last-created tab, so firing split actions on the
            // currently-active tab would target the wrong session.
            let owner_connection_id: Option<uuid::Uuid> = profile
                .split_layout
                .split_owner_entry_index
                .and_then(|idx| profile.entries.get(idx))
                .map(|e| e.connection_id);

            // Flag: should we wait for the owner before applying layout?
            // If no owner index is saved (old profile format) — fall back to
            // immediate apply_layout (existing 2-panel behavior).
            // Also: if owner is a Local Shell (nil UUID), it appears synchronously
            // as the first tab — no need to defer, immediate apply works.
            let owner_is_sync = owner_connection_id.is_some_and(|id| id.is_nil());
            let needs_deferred_layout = profile.split_layout.is_split
                && owner_connection_id.is_some()
                && profile.split_layout.extra_splits > 0
                && !owner_is_sync;

            if !needs_deferred_layout {
                // Sync path: owner is Local Shell or legacy 2-panel.
                // apply_layout fires splits on idle; for sync guests we schedule
                // placement on a SECOND idle (runs after splits create panels).
                if let Some(win) = window_for_open.upgrade() {
                    crate::split_view::apply_layout(
                        &win,
                        &profile.split_layout,
                        &notebook_for_open,
                        &session_bridges_for_open,
                    );
                }
                // For sync owner with guests: schedule deferred placement
                // on idle AFTER the split idle creates the panels.
                if owner_is_sync && !guest_connection_ids.is_empty() {
                    let notebook_for_sync = notebook_for_open.clone();
                    let bridges_for_sync = session_bridges_for_open.clone();
                    let monitoring_for_sync = monitoring_for_open.clone();
                    let guest_cids_sync = guest_connection_ids.clone();
                    let total_sync = guest_connection_ids.len();
                    gtk4::glib::idle_add_local_once(move || {
                        let mut placed = 0usize;
                        // Find sessions matching guest connection_ids
                        for sid in notebook_for_sync.ordered_session_ids() {
                            if placed >= total_sync {
                                break;
                            }
                            let Some(info) = notebook_for_sync.get_session_info(sid) else {
                                continue;
                            };
                            if !guest_cids_sync.contains(&info.connection_id) {
                                continue;
                            }
                            // Skip if already in a bridge (could be owner)
                            if bridges_for_sync.borrow().contains_key(&sid) {
                                continue;
                            }
                            let bridges = bridges_for_sync.borrow();
                            let bridge = bridges.values().find(|b| {
                                b.pane_count() >= 2
                                    && b.active_sessions().len() < b.pane_count()
                            });
                            if let Some(bridge) = bridge.cloned() {
                                drop(bridges);
                                if let Some(empty_pane) = bridge.first_empty_pane_uuid() {
                                    if let Some(info) = notebook_for_sync.get_session_info(sid) {
                                        bridge.add_session(info);
                                    }
                                    if let Some(content) =
                                        notebook_for_sync.get_session_display_widget(sid)
                                        && let Ok(color_index) =
                                            bridge.move_session_to_panel(empty_pane, sid, &content)
                                    {
                                        bridges_for_sync
                                            .borrow_mut()
                                            .insert(sid, bridge.clone());
                                        notebook_for_sync.park_session_tab(sid);
                                        notebook_for_sync
                                            .set_tab_split_color(sid, color_index);
                                        monitoring_for_sync.suspend_monitoring(sid);
                                        placed += 1;
                                        tracing::debug!(
                                            "Workspace restore: placed sync guest {sid} ({placed}/{total_sync})"
                                        );
                                    }
                                }
                            }
                        }
                    });
                }
            }

            // Deferred split restore: register on_tab_added BEFORE starting
            // connections so that synchronous Local Shell tabs are captured.
            if needs_deferred_layout || !guest_connection_ids.is_empty() {
                let notebook_for_guest = notebook_for_open.clone();
                let bridges_for_guest = session_bridges_for_open.clone();
                let monitoring_for_guest = monitoring_for_open.clone();
                let placed_count = std::rc::Rc::new(std::cell::Cell::new(0usize));
                let total_guests = guest_connection_ids.len();
                let guest_cids = std::rc::Rc::new(guest_connection_ids);
                let layout_applied = std::rc::Rc::new(std::cell::Cell::new(!needs_deferred_layout));
                let layout_for_deferred = profile.split_layout.clone();
                let window_for_deferred = window_for_open.clone();
                let owner_cid = owner_connection_id;
                let early_guests: std::rc::Rc<std::cell::RefCell<Vec<(uuid::Uuid, uuid::Uuid)>>> =
                    std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
                notebook_for_open.set_on_tab_added(move |session_id, connection_id| {
                    // Helper closure: attempt to place a guest in the next empty panel.
                    let try_place_guest = |sid: uuid::Uuid, cid: uuid::Uuid| {
                        if placed_count.get() >= total_guests {
                            return;
                        }
                        if !guest_cids.contains(&cid) {
                            return;
                        }
                        let bridges = bridges_for_guest.borrow();
                        let bridge = bridges.values().find(|b| {
                            b.pane_count() >= 2
                                && b.active_sessions().len() < b.pane_count()
                        });
                        if let Some(bridge) = bridge.cloned() {
                            drop(bridges);
                            if let Some(empty_pane) = bridge.first_empty_pane_uuid() {
                                if let Some(info) = notebook_for_guest.get_session_info(sid) {
                                    bridge.add_session(info);
                                }
                                if let Some(content) =
                                    notebook_for_guest.get_session_display_widget(sid)
                                {
                                    match bridge.move_session_to_panel(
                                        empty_pane,
                                        sid,
                                        &content,
                                    ) {
                                        Ok(color_index) => {
                                            bridges_for_guest
                                                .borrow_mut()
                                                .insert(sid, bridge.clone());
                                            notebook_for_guest.park_session_tab(sid);
                                            notebook_for_guest
                                                .set_tab_split_color(sid, color_index);
                                            monitoring_for_guest.suspend_monitoring(sid);
                                            tracing::debug!(
                                                "Workspace restore: placed guest session \
                                                 {sid} in split panel ({}/{})",
                                                placed_count.get() + 1,
                                                total_guests
                                            );
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                "Workspace restore: failed to place guest \
                                                 in split panel: {e}"
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        placed_count.set(placed_count.get() + 1);
                    };

                    // Phase 1: wait for the owner tab, switch to it, apply layout.
                    if !layout_applied.get() {
                        if Some(connection_id) == owner_cid {
                            // Owner session appeared — switch to its tab so
                            // win.split-* targets the correct session.
                            notebook_for_guest.switch_to_tab(session_id);
                            if let Some(win) = window_for_deferred.upgrade() {
                                crate::split_view::apply_layout(
                                    &win,
                                    &layout_for_deferred,
                                    &notebook_for_guest,
                                    &bridges_for_guest,
                                );
                            }
                            layout_applied.set(true);
                            tracing::debug!(
                                "Workspace restore: owner session {session_id} appeared, \
                                 applied multi-panel layout (extra_splits={})",
                                layout_for_deferred.extra_splits
                            );
                            // Drain early guests that arrived before the owner.
                            // Schedule on idle AFTER the split actions (which
                            // also run on idle) have created the empty panels.
                            let early = early_guests.borrow_mut().drain(..).collect::<Vec<_>>();
                            if !early.is_empty() {
                                let try_place_deferred = {
                                    let placed_count = placed_count.clone();
                                    let total_guests = total_guests;
                                    let guest_cids = guest_cids.clone();
                                    let bridges_for_guest = bridges_for_guest.clone();
                                    let notebook_for_guest = notebook_for_guest.clone();
                                    let monitoring_for_guest = monitoring_for_guest.clone();
                                    move || {
                                        for (sid, cid) in early {
                                            if placed_count.get() >= total_guests {
                                                break;
                                            }
                                            if !guest_cids.contains(&cid) {
                                                continue;
                                            }
                                            let bridges = bridges_for_guest.borrow();
                                            let bridge = bridges.values().find(|b| {
                                                b.pane_count() >= 2
                                                    && b.active_sessions().len() < b.pane_count()
                                            });
                                            if let Some(bridge) = bridge.cloned() {
                                                drop(bridges);
                                                if let Some(empty_pane) = bridge.first_empty_pane_uuid() {
                                                    if let Some(info) = notebook_for_guest.get_session_info(sid) {
                                                        bridge.add_session(info);
                                                    }
                                                    if let Some(content) =
                                                        notebook_for_guest.get_session_display_widget(sid)
                                                    {
                                                        match bridge.move_session_to_panel(empty_pane, sid, &content) {
                                                            Ok(color_index) => {
                                                                bridges_for_guest.borrow_mut().insert(sid, bridge.clone());
                                                                notebook_for_guest.park_session_tab(sid);
                                                                notebook_for_guest.set_tab_split_color(sid, color_index);
                                                                monitoring_for_guest.suspend_monitoring(sid);
                                                                tracing::debug!(
                                                                    "Workspace restore: placed buffered guest {sid} ({}/{})",
                                                                    placed_count.get() + 1, total_guests
                                                                );
                                                            }
                                                            Err(e) => {
                                                                tracing::warn!("Workspace restore: buffered guest placement failed: {e}");
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            placed_count.set(placed_count.get() + 1);
                                        }
                                    }
                                };
                                // Run AFTER the current idle (split actions)
                                gtk4::glib::idle_add_local_once(try_place_deferred);
                            }
                        } else {
                            // Guest arrived before owner — buffer it for later.
                            early_guests.borrow_mut().push((session_id, connection_id));
                        }
                        return;
                    }

                    // Phase 2: deferred guest placement.
                    try_place_guest(session_id, connection_id);
                });
            }

            // Now start connections — on_tab_added is already registered to
            // capture both sync (Local Shell) and async (SSH/RDP) sessions.
            for entry in &profile.entries {
                if entry.connection_id.is_nil() && entry.protocol == "local" {
                    super::MainWindow::open_local_shell_with_split(
                        &notebook_for_open,
                        &split_view_for_open,
                        Some(&state_for_open),
                    );
                } else {
                    super::MainWindow::start_connection_with_credential_resolution(
                        state_for_open.clone(),
                        notebook_for_open.clone(),
                        split_view_for_open.clone(),
                        sidebar_for_open.clone(),
                        monitoring_for_open.clone(),
                        entry.connection_id,
                        Some(activity_for_open.clone()),
                    );
                }
            }
        }
    });

    // Delete callback
    let state_for_delete = state.clone();
    let dialog_rc = std::rc::Rc::new(dialog);
    let dialog_for_delete = dialog_rc.clone();
    dialog_rc.set_on_delete(move |workspace_id| {
        if let Ok(mut state_ref) = state_for_delete.try_borrow_mut()
            && let Err(e) = state_ref.delete_workspace_profile(workspace_id)
        {
            tracing::warn!("Failed to delete workspace: {e}");
        }
        dialog_for_delete.refresh_list();
    });

    // Rename callback
    let state_for_rename = state.clone();
    let dialog_for_rename = dialog_rc.clone();
    dialog_rc.set_on_rename(move |workspace_id, new_name| {
        if let Ok(mut state_ref) = state_for_rename.try_borrow_mut()
            && let Err(e) = state_ref.rename_workspace_profile(workspace_id, new_name)
        {
            tracing::warn!("Failed to rename workspace: {e}");
        }
        dialog_for_rename.refresh_list();
    });

    // Save current callback
    let state_for_save = state.clone();
    let notebook_for_save = notebook.clone();
    let bridges_for_save = session_split_bridges.clone();
    let dialog_for_save = dialog_rc.clone();
    let window_weak = window.downgrade();
    dialog_rc.set_on_save_current(move || {
        if let Some(win) = window_weak.upgrade() {
            save_current_workspace(
                &state_for_save,
                &notebook_for_save,
                &bridges_for_save,
                &dialog_for_save,
                &win,
            );
        }
    });

    dialog_rc.refresh_list();
    dialog_rc.show(window.upcast_ref::<gtk4::Widget>());
}

/// Saves currently open sessions as a new workspace profile
fn save_current_workspace(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    session_split_bridges: &SessionSplitBridges,
    dialog: &std::rc::Rc<WorkspaceManagerDialog>,
    window: &gtk4::Window,
) {
    use rustconn_core::session::SessionType;
    use rustconn_core::split::SplitDirection;

    // Collect open sessions from the terminal notebook (the live source of
    // truth for the GUI — the core SessionManager is not populated here).
    let entries: Vec<WorkspaceEntry> = notebook
        .ordered_session_ids()
        .iter()
        .filter_map(|id| notebook.get_session_info(*id).map(|s| (*id, s)))
        .enumerate()
        .map(|(i, (id, session))| {
            let session_type = if session.is_embedded {
                SessionType::Embedded
            } else {
                SessionType::External
            };
            let mut entry = WorkspaceEntry::new(
                session.connection_id,
                session.name.clone(),
                session.protocol.clone(),
                session_type,
                i,
            );
            // Preserve the tab group so it is restored when the workspace reopens.
            if let Some(group) = notebook.get_tab_group(id) {
                entry = entry.with_tab_group(group);
            }
            entry
        })
        .collect();

    if entries.is_empty() {
        show_toast_on_window(window, &i18n("No active sessions to save"), ToastType::Info);
        return;
    }

    // Capture the split layout of the currently active tab so it can be
    // restored on open. Only the active session's bridge is consulted —
    // WorkspaceSplitLayout stores the primary split direction/ratio plus
    // ALL guest entry indices for multi-panel restore.
    let split_layout = notebook
        .get_active_session_id()
        .and_then(|active| {
            let bridge = session_split_bridges.borrow().get(&active).cloned()?;
            let (direction, ratio) = bridge.root_split()?;
            let active_sessions = bridge.active_sessions();
            let pane_count = bridge.pane_count();

            tracing::debug!(
                "Workspace save: capturing split layout, active={active}, \
                 pane_count={pane_count}, active_sessions={active_sessions:?}"
            );

            // The owner is the first session in the bridge's active list (the
            // one that initiated the split). Identify its entry index so
            // restore can target the correct tab.
            let owner_entry_index = active_sessions
                .first()
                .and_then(|&owner_sid| notebook.get_session_info(owner_sid))
                .and_then(|info| {
                    entries
                        .iter()
                        .position(|e| e.connection_id == info.connection_id)
                });

            // Collect ALL guest sessions (everything after the first/owner).
            // Track used entry indices to handle duplicate connection_ids
            // (multiple Local Shells all have Uuid::nil).
            let mut used_indices: Vec<usize> = Vec::new();
            if let Some(owner_idx) = owner_entry_index {
                used_indices.push(owner_idx);
            }
            let split_guests: Vec<usize> = active_sessions
                .iter()
                .skip(1) // skip the owner
                .filter_map(|&guest_sid| notebook.get_session_info(guest_sid))
                .filter_map(|info| {
                    let idx = entries.iter().enumerate().position(|(i, e)| {
                        e.connection_id == info.connection_id && !used_indices.contains(&i)
                    });
                    if let Some(i) = idx {
                        used_indices.push(i);
                    }
                    idx
                })
                .collect();

            // Backward compat: also set single guest index
            let guest_entry_index = split_guests.first().copied();

            // extra_splits = total panels - 2 (the initial split creates 2)
            let extra_splits = pane_count.saturating_sub(2);

            // Capture per-split directions so restore uses the correct action
            // for each split (horizontal vs vertical).
            let split_directions: Vec<bool> = bridge
                .all_split_directions()
                .into_iter()
                .map(|d| d == SplitDirection::Horizontal)
                .collect();

            tracing::debug!(
                "Workspace save: owner_entry_index={owner_entry_index:?}, \
                 split_guests={split_guests:?}, extra_splits={extra_splits}, \
                 split_directions={split_directions:?}"
            );

            Some(WorkspaceSplitLayout {
                is_split: true,
                horizontal: direction == SplitDirection::Horizontal,
                split_ratio: ratio,
                split_guest_entry_index: guest_entry_index,
                split_guests,
                extra_splits,
                split_directions,
                split_owner_entry_index: owner_entry_index,
            })
        })
        .unwrap_or_default();

    // Prompt for name
    let state_clone = state.clone();
    let entries_clone = entries;
    let dialog_clone = dialog.clone();
    let window_weak = window.downgrade();

    let alert = adw::AlertDialog::new(
        Some(&i18n("Save Workspace")),
        Some(&i18n("Enter a name for this workspace profile:")),
    );
    alert.add_response("cancel", &i18n("Cancel"));
    alert.add_response("save", &i18n("Save"));
    alert.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    alert.set_default_response(Some("save"));
    alert.set_close_response("cancel");

    let entry = gtk4::Entry::builder()
        .placeholder_text(i18n("Workspace name"))
        .activates_default(true)
        .build();
    alert.set_extra_child(Some(&entry));

    let entry_clone = entry.clone();
    alert.connect_response(None, move |_, response| {
        if response != "save" {
            return;
        }
        let name = entry_clone.text().to_string();
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }

        let mut profile = WorkspaceProfile::new(&name);
        for e in &entries_clone {
            profile.add_entry(e.clone());
        }
        profile.set_split_layout(split_layout.clone());

        // Create the profile, then release the state borrow *before* refreshing
        // the list — refresh_list's provider re-borrows the same state.
        let result = state_clone
            .try_borrow_mut()
            .ok()
            .map(|mut state_ref| state_ref.create_workspace_profile(profile));

        match result {
            Some(Ok(_)) => {
                if let Some(win) = window_weak.upgrade() {
                    let msg = i18n_f("Workspace '{}' saved", &[&name]);
                    show_toast_on_window(&win, &msg, ToastType::Success);
                }
                // Refresh now that the profile exists and the state borrow is
                // released.
                dialog_clone.refresh_list();
            }
            Some(Err(e)) => {
                tracing::warn!("Failed to save workspace: {e}");
                if let Some(win) = window_weak.upgrade() {
                    // Persistence failure is not transient — blocking dialog (GNOME HIG).
                    crate::alert::show_error(&win, &i18n("Failed to save workspace"), &e.clone());
                }
            }
            None => {}
        }
    });

    alert.present(Some(window));
}
