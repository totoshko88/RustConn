//! Sorting and drag-drop operations for main window
//!
//! This module contains functions for sorting connections and handling
//! drag-drop reordering operations.

use std::rc::Rc;

use uuid::Uuid;

use super::types::get_protocol_string;
use crate::i18n::i18n;
use crate::sidebar::{ConnectionItem, ConnectionSidebar, PINNED_GROUP_ID};
use crate::state::SharedAppState;

/// Type alias for shared sidebar reference
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Builds a sidebar row for `conn`, carrying over the live state the sidebar holds.
///
/// A rebuild throws every `ConnectionItem` away and makes new ones, so state that
/// only ever arrived through a property setter is lost with the old objects. The
/// status was already read back from the sidebar's own map; the recording dot,
/// the external-viewer emblem and the split marker were not restored at all, so
/// any reload — a rename, a duplicate, a pin toggle, a re-sort, a drag-drop —
/// silently cleared all three for every connection.
///
/// `pinned` drives the star emblem and is passed in rather than read from
/// `conn.is_pinned`: the copy shown inside the Favorites group passes `false` so
/// the emblem does not repeat on rows that are all favorites by definition,
/// while the copy at the connection's real place passes `conn.is_pinned` to mark
/// it there.
fn build_connection_item(
    sidebar: &SharedSidebar,
    conn: &rustconn_core::models::Connection,
    pinned: bool,
) -> ConnectionItem {
    let id = conn.id.to_string();
    let protocol = get_protocol_string(&conn.protocol_config);
    let status = sidebar
        .get_connection_status(&id)
        .unwrap_or_else(|| "disconnected".to_string());
    let item = ConnectionItem::new_connection_full_with_icon(
        &id,
        &conn.name,
        &protocol,
        &conn.host,
        &status,
        pinned,
        conn.icon.as_deref().unwrap_or(""),
    );
    item.set_description(conn.description.as_deref().unwrap_or(""));
    item.set_browser_tunnel(rustconn_core::ssh_tunnel::supports_browser_tunnel(conn));
    if let Some(indicators) = sidebar.row_indicators(&id) {
        item.set_is_recording(indicators.recording);
        item.set_external_session(indicators.external_session);
        item.set_split_color(indicators.split_color);
    }
    item
}

/// Toggles group operations mode for multi-select
pub fn toggle_group_operations_mode(sidebar: &SharedSidebar, enabled: bool) {
    sidebar.set_group_operations_mode(enabled);
}

/// Sorts connections alphabetically and updates `sort_order`
///
/// If a group is selected, only sorts connections within that group.
/// Otherwise, sorts all groups and connections globally.
pub fn sort_connections(state: &SharedAppState, sidebar: &SharedSidebar) {
    // Check if a group is selected
    let selected_group_id = sidebar.get_selected_item().and_then(|item| {
        if item.is_group() {
            Uuid::parse_str(&item.id()).ok()
        } else {
            None
        }
    });

    // Perform the appropriate sort operation
    if let Some(group_id) = selected_group_id {
        // Sort only the selected group
        if let Ok(mut state_mut) = state.try_borrow_mut()
            && let Err(e) = state_mut.sort_group(group_id)
        {
            tracing::error!(%e, "Failed to sort group");
        }
    } else {
        // Sort all groups and connections
        if let Ok(mut state_mut) = state.try_borrow_mut()
            && let Err(e) = state_mut.sort_all()
        {
            tracing::error!(%e, "Failed to sort all");
        }
    }

    // Save expanded groups before rebuild so open groups stay open
    let expanded = sidebar.get_expanded_groups();

    // Rebuild the sidebar to reflect the new sort order
    rebuild_sidebar_sorted(state, sidebar);

    // Restore expanded groups
    sidebar.apply_expanded_groups(&expanded);
}

/// Sorts connections by recent usage (most recently used first)
pub fn sort_recent(state: &SharedAppState, sidebar: &SharedSidebar) {
    // Sort all connections by last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.sort_by_recent()
    {
        tracing::error!(%e, "Failed to sort by recent");
    }

    // Save expanded groups before rebuild so open groups stay open
    let expanded = sidebar.get_expanded_groups();

    // Rebuild the sidebar to reflect the new sort order
    rebuild_sidebar_sorted(state, sidebar);

    // Restore expanded groups
    sidebar.apply_expanded_groups(&expanded);
}

/// Rebuilds the sidebar with sorted items
pub fn rebuild_sidebar_sorted(state: &SharedAppState, sidebar: &SharedSidebar) {
    // Every rebuild path funnels through here, so clear the search cache once at
    // this choke point; otherwise direct callers (`sort_connections`,
    // `sort_recent`, `handle_drag_drop`) would leave a stale filter result.
    sidebar.invalidate_search_cache();

    let store = sidebar.store();
    let state_ref = state.borrow();

    // Get and sort groups by sort_order, then by name. Owned clones are
    // required: state_ref is dropped before the store is rebuilt below.
    let mut groups: Vec<_> = state_ref
        .get_root_groups()
        .iter()
        .map(|g| (*g).clone())
        .collect();
    groups.sort_by(|a, b| match a.sort_order.cmp(&b.sort_order) {
        std::cmp::Ordering::Equal => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        other => other,
    });

    // Get and sort ungrouped connections by sort_order, then by name
    let mut ungrouped: Vec<_> = state_ref
        .get_ungrouped_connections()
        .iter()
        .map(|c| (*c).clone())
        .collect();
    ungrouped.sort_by(|a, b| match a.sort_order.cmp(&b.sort_order) {
        std::cmp::Ordering::Equal => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        other => other,
    });

    // Check if there are any connections at all
    let total_connections = state_ref.list_connections().len();

    drop(state_ref);

    // Rebuild store with sorted items (groups first, then ungrouped)
    let trash_expanded = sidebar.trash_expanded();
    store.remove_all();

    // If no connections exist, the list will be empty
    // The empty state is shown via CSS/placeholder in the list view
    if total_connections == 0 && groups.is_empty() {
        // Store is empty - list view will show nothing
        // Empty state is handled by the main window layout. The Trash row still
        // shows: deleting the last connection is when Undo is most needed.
        let trash = state.borrow().trash_entries();
        sidebar.set_trash_row(&trash, trash_expanded);
        return;
    }

    let state_ref = state.borrow();

    // Collect pinned connections from ALL connections (including grouped)
    let mut pinned: Vec<_> = state_ref
        .list_connections()
        .iter()
        .filter(|c| c.is_pinned)
        .map(|c| (*c).clone())
        .collect();
    pinned.sort_by(|a, b| match a.pin_order.cmp(&b.pin_order) {
        std::cmp::Ordering::Equal => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        other => other,
    });

    // Add pinned connections as a virtual "Favorites" group at the top.
    // The same connection is also added below at its real place, so a pinned row
    // exists twice in the store — see `ConnectionSidebar::update_items_with_id`.
    //
    // The star emoji sets this synthetic group apart from ordinary folders, which
    // carry the plain folder icon. Its children are built with `pinned = false`
    // so the star emblem does not repeat on every row inside a group that is, by
    // definition, entirely favorites — the emblem's job is to mark a favorite at
    // its real place elsewhere in the tree.
    if !pinned.is_empty() {
        let favorites_item =
            ConnectionItem::new_group_with_icon(PINNED_GROUP_ID, &i18n("Favorites"), "⭐");
        for conn in &pinned {
            favorites_item.add_child(&build_connection_item(sidebar, conn, false));
        }
        store.append(&favorites_item);
    }

    // Add sorted groups with their sorted children
    for group in &groups {
        let icon = group.icon.as_deref().unwrap_or("");
        let sync_mode_str = match group.sync_mode {
            rustconn_core::sync::SyncMode::None => "none",
            rustconn_core::sync::SyncMode::Master => "master",
            rustconn_core::sync::SyncMode::Import => "import",
        };
        // Look up sync error from SyncManager state
        let sync_error = state_ref
            .sync_manager()
            .state()
            .get(&group.id)
            .and_then(|s| s.last_error.as_deref())
            .unwrap_or("");
        let group_item = ConnectionItem::new_group_full_with_dynamic(
            &group.id.to_string(),
            &group.name,
            icon,
            sync_mode_str,
            sync_error,
            true, // root groups
            group.dynamic_folder.is_some(),
        );
        add_sorted_group_children(&state_ref, sidebar, &group_item, group.id);
        store.append(&group_item);
    }

    // Add sorted ungrouped connections
    for conn in &ungrouped {
        store.append(&build_connection_item(sidebar, conn, conn.is_pinned));
    }

    // The virtual Trash row closes the tree, and only exists while the trash
    // holds something.
    sidebar.set_trash_row(&state_ref.trash_entries(), trash_expanded);

    // Refresh Smart Folders section with current filter evaluation
    let smart_folders = state_ref.settings().smart_folders.clone();
    let all_connections: Vec<_> = state_ref.list_connections_owned();
    drop(state_ref);
    sidebar.refresh_smart_folders(&smart_folders, &all_connections);
}

/// Recursively adds sorted group children
pub fn add_sorted_group_children(
    state: &std::cell::Ref<crate::state::AppState>,
    sidebar: &SharedSidebar,
    parent_item: &ConnectionItem,
    group_id: Uuid,
) {
    // Get and sort child groups by sort_order, then by name
    let mut child_groups: Vec<_> = state
        .get_child_groups(group_id)
        .iter()
        .map(|g| (*g).clone())
        .collect();
    child_groups.sort_by(|a, b| match a.sort_order.cmp(&b.sort_order) {
        std::cmp::Ordering::Equal => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        other => other,
    });

    for child_group in &child_groups {
        let icon = child_group.icon.as_deref().unwrap_or("");
        let child_item = ConnectionItem::new_group_full_with_dynamic(
            &child_group.id.to_string(),
            &child_group.name,
            icon,
            "none",
            "",
            false,
            child_group.dynamic_folder.is_some(),
        );
        add_sorted_group_children(state, sidebar, &child_item, child_group.id);
        parent_item.add_child(&child_item);
    }

    // Get and sort connections in this group by sort_order, then by name
    let mut connections: Vec<_> = state
        .get_connections_by_group(group_id)
        .iter()
        .map(|c| (*c).clone())
        .collect();
    connections.sort_by(|a, b| match a.sort_order.cmp(&b.sort_order) {
        std::cmp::Ordering::Equal => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        other => other,
    });

    for conn in &connections {
        parent_item.add_child(&build_connection_item(sidebar, conn, conn.is_pinned));
    }
}

/// Handles drag-drop operations for reordering connections
///
/// Data format: "`item_type:item_id:target_id:target_is_group:position`"
/// where position is "before", "after", or "into"
pub fn handle_drag_drop(state: &SharedAppState, sidebar: &SharedSidebar, data: &str) {
    let parts: Vec<&str> = data.split(':').collect();
    if parts.len() != 5 {
        return;
    }

    let item_type = parts[0];
    let item_id = parts[1];
    let target_id = parts[2];
    let target_is_group = parts[3] == "true";
    let position = parts[4]; // "before", "after", or "into"

    // Parse UUIDs
    let Ok(item_uuid) = Uuid::parse_str(item_id) else {
        tracing::debug!(item_id, "Drag-drop ignored: source ID is not a UUID");
        return;
    };
    let Ok(target_uuid) = Uuid::parse_str(target_id) else {
        // Favorites is a virtual group with no UUID and nothing to move into:
        // pinning is what puts a connection there. Said out loud because the
        // drop otherwise looked accepted and did nothing.
        if target_id == PINNED_GROUP_ID {
            tracing::debug!(
                "Drag-drop into Favorites ignored: it is a virtual group — pin the \
                 connection instead"
            );
        } else {
            tracing::debug!(target_id, "Drag-drop ignored: target ID is not a UUID");
        }
        return;
    };

    // Handle based on item type
    match item_type {
        "conn" => {
            // Moving a connection
            if target_is_group {
                // Target is a group - move connection into it
                if position == "into" {
                    // Block drops INTO Import groups (sync-managed)
                    {
                        let state_ref = state.borrow();
                        if let Some(group) = state_ref.get_group(target_uuid)
                            && group.sync_mode == rustconn_core::sync::SyncMode::Import
                        {
                            tracing::warn!("Rejected drag-drop into Import group '{}'", group.name);
                            return;
                        }
                    }
                    // Drop INTO group - move to the group
                    if let Ok(mut state_mut) = state.try_borrow_mut()
                        && let Err(e) =
                            state_mut.move_connection_to_group(item_uuid, Some(target_uuid))
                    {
                        tracing::error!(%e, "Failed to move connection to group");
                        return;
                    }
                } else {
                    // Drop BEFORE/AFTER group - move to parent of that group (or root)
                    let parent_group_id = {
                        let state_ref = state.borrow();
                        state_ref.get_group(target_uuid).and_then(|g| g.parent_id)
                    };

                    if let Ok(mut state_mut) = state.try_borrow_mut()
                        && let Err(e) =
                            state_mut.move_connection_to_group(item_uuid, parent_group_id)
                    {
                        tracing::error!(%e, "Failed to move connection");
                        return;
                    }
                }
            } else {
                // Target is a connection - reorder relative to it
                // Get the target connection's group
                let target_group_id = {
                    let state_ref = state.borrow();
                    state_ref
                        .get_connection(target_uuid)
                        .and_then(|c| c.group_id)
                };

                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    // First move to the same group as target
                    if let Err(e) = state_mut.move_connection_to_group(item_uuid, target_group_id) {
                        tracing::error!(%e, "Failed to move connection");
                        return;
                    }

                    // Then reorder within the group
                    if let Err(e) = state_mut.reorder_connection(item_uuid, target_uuid) {
                        tracing::error!(%e, "Failed to reorder connection");
                        return;
                    }
                }
            }
        }
        "group" => {
            // Moving a group — may reparent it, reorder it, or both
            if !move_dropped_group(state, item_uuid, target_uuid, target_is_group, position) {
                return;
            }
        }
        _ => return,
    }

    // Save tree state before rebuild
    let tree_state = sidebar.save_state();

    // Rebuild sidebar to reflect changes
    rebuild_sidebar_sorted(state, sidebar);

    // Restore tree state after rebuild
    sidebar.restore_state(&tree_state);
}

/// Returns `true` when `group_id` is an Import folder or sits inside one.
///
/// Import folders and everything under them are recreated from the synced
/// source on the next sync run, so a manual move there would silently revert.
fn is_in_import_subtree(
    state_ref: &std::cell::Ref<crate::state::AppState>,
    group_id: Uuid,
) -> bool {
    let mut current = Some(group_id);
    // A corrupted parent chain must not spin forever; no real tree is this deep.
    for _ in 0..64 {
        let Some(id) = current else { return false };
        let Some(group) = state_ref.get_group(id) else {
            return false;
        };
        if group.sync_mode == rustconn_core::sync::SyncMode::Import {
            return true;
        }
        current = group.parent_id;
    }
    tracing::warn!(%group_id, "Folder parent chain too deep, treating as unmanaged");
    false
}

/// Applies a dropped folder, returning `true` when the sidebar needs a rebuild.
///
/// Dropping a folder on the middle band of another folder nests it inside that
/// folder; dropping it on a folder's top/bottom edge, or on a connection, makes
/// it a sibling at that level instead. A single drop can require both a
/// reparent and a reorder, and they must run in that order because
/// `reorder_group` only accepts folders that already share a parent.
fn move_dropped_group(
    state: &SharedAppState,
    group_id: Uuid,
    target_uuid: Uuid,
    target_is_group: bool,
    position: &str,
) -> bool {
    let nest_into_target = target_is_group && position == "into";

    let (current_parent, new_parent) = {
        // A drop arrives from a GTK callback, where another handler may still
        // hold the state — try_borrow keeps that a no-op instead of a panic.
        let Ok(state_ref) = state.try_borrow() else {
            tracing::warn!("State busy, folder drop ignored");
            return false;
        };
        let Some(dragged) = state_ref.get_group(group_id) else {
            tracing::warn!(%group_id, "Dragged folder no longer exists");
            return false;
        };
        let current_parent = dragged.parent_id;
        let dragged_name = dragged.name.clone();

        // Import folders mirror a synced source: the next sync run recreates
        // their place in the tree, so neither the folder itself nor anything
        // inside it may be reorganised by hand.
        if is_in_import_subtree(&state_ref, group_id) {
            tracing::warn!(group = %dragged_name, "Rejected move of an Import folder");
            drop(state_ref);
            crate::toast::show_error_toast_on_active_window(&i18n(
                "Imported folders are managed by sync and their structure cannot be changed",
            ));
            return false;
        }

        let new_parent = if nest_into_target {
            Some(target_uuid)
        } else if target_is_group {
            state_ref.get_group(target_uuid).and_then(|g| g.parent_id)
        } else {
            // Dropped on a connection — join the folder that holds it.
            state_ref
                .get_connection(target_uuid)
                .and_then(|c| c.group_id)
        };
        // The destination must not belong to an Import tree either — a folder
        // nested there would be dropped again by the next sync run.
        if new_parent.is_some_and(|p| is_in_import_subtree(&state_ref, p)) {
            tracing::warn!(group = %dragged_name, "Rejected folder drop into an Import subtree");
            drop(state_ref);
            crate::toast::show_error_toast_on_active_window(&i18n(
                "Imported folders are managed by sync and their structure cannot be changed",
            ));
            return false;
        }
        // A folder cannot land inside itself, and nesting it under one of its own
        // descendants would cut the whole subtree out of the tree.
        if new_parent == Some(group_id)
            || new_parent.is_some_and(|p| super::groups::is_descendant_of(&state_ref, p, group_id))
        {
            drop(state_ref);
            crate::toast::show_error_toast_on_active_window(&i18n(
                "Cannot move a folder into one of its own subfolders",
            ));
            return false;
        }
        (current_parent, new_parent)
    };

    if current_parent != new_parent {
        let Ok(mut state_mut) = state.try_borrow_mut() else {
            tracing::warn!("State busy, folder drop ignored");
            return false;
        };
        if let Err(e) = state_mut.move_group_to_parent(group_id, new_parent) {
            tracing::error!(%e, "Failed to reparent folder");
            return false;
        }
    }

    // Edge drops also carry a position among the siblings.
    if target_is_group
        && !nest_into_target
        && let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.reorder_group(group_id, target_uuid)
    {
        // Non-fatal: the folder already sits at the requested level, only the
        // order among its siblings stays as it was.
        tracing::warn!(%e, "Failed to reorder folder after drop");
    }

    true
}

/// Refreshes only the virtual Trash row, keeping the rest of the tree (and the
/// selection) as it is. For changes that touch the trash alone, such as
/// emptying it.
pub fn refresh_trash_row(state: &SharedAppState, sidebar: &SharedSidebar) {
    let entries = state.borrow().trash_entries();
    sidebar.set_trash_row(&entries, sidebar.trash_expanded());
}
