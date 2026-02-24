//! Sorting and drag-drop operations for main window
//!
//! This module contains functions for sorting connections and handling
//! drag-drop reordering operations.

use super::types::get_protocol_string;
use crate::i18n::i18n;
use crate::sidebar::{ConnectionItem, ConnectionSidebar};
use crate::state::SharedAppState;
use std::rc::Rc;
use uuid::Uuid;

/// Type alias for shared sidebar reference
pub type SharedSidebar = Rc<ConnectionSidebar>;

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

    // Rebuild the sidebar to reflect the new sort order
    rebuild_sidebar_sorted(state, sidebar);
}

/// Sorts connections by recent usage (most recently used first)
pub fn sort_recent(state: &SharedAppState, sidebar: &SharedSidebar) {
    // Sort all connections by last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.sort_by_recent()
    {
        tracing::error!(%e, "Failed to sort by recent");
    }

    // Rebuild the sidebar to reflect the new sort order
    rebuild_sidebar_sorted(state, sidebar);
}

/// Rebuilds the sidebar with sorted items
pub fn rebuild_sidebar_sorted(state: &SharedAppState, sidebar: &SharedSidebar) {
    let store = sidebar.store();
    let state_ref = state.borrow();

    // Get and sort groups by sort_order, then by name
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
    store.remove_all();

    // If no connections exist, the list will be empty
    // The empty state is shown via CSS/placeholder in the list view
    if total_connections == 0 && groups.is_empty() {
        // Store is empty - list view will show nothing
        // Empty state is handled by the main window layout
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

    // Add pinned connections as a virtual "Favorites" group at the top
    if !pinned.is_empty() {
        let favorites_item = ConnectionItem::new_group("__pinned__", &i18n("Favorites"));
        for conn in &pinned {
            let protocol = get_protocol_string(&conn.protocol_config);
            let status = sidebar
                .get_connection_status(&conn.id.to_string())
                .unwrap_or_else(|| "disconnected".to_string());
            let icon = conn.icon.as_deref().unwrap_or("");
            let item = ConnectionItem::new_connection_full_with_icon(
                &conn.id.to_string(),
                &conn.name,
                &protocol,
                &conn.host,
                &status,
                true,
                icon,
            );
            favorites_item.add_child(&item);
        }
        store.append(&favorites_item);
    }

    // Add sorted groups with their sorted children
    for group in &groups {
        let icon = group.icon.as_deref().unwrap_or("");
        let group_item =
            ConnectionItem::new_group_with_icon(&group.id.to_string(), &group.name, icon);
        add_sorted_group_children(&state_ref, sidebar, &group_item, group.id);
        store.append(&group_item);
    }

    // Add sorted ungrouped connections
    for conn in &ungrouped {
        let protocol = get_protocol_string(&conn.protocol_config);
        let status = sidebar
            .get_connection_status(&conn.id.to_string())
            .unwrap_or_else(|| "disconnected".to_string());
        let icon = conn.icon.as_deref().unwrap_or("");
        let item = ConnectionItem::new_connection_full_with_icon(
            &conn.id.to_string(),
            &conn.name,
            &protocol,
            &conn.host,
            &status,
            conn.is_pinned,
            icon,
        );
        store.append(&item);
    }
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
        let child_item = ConnectionItem::new_group_with_icon(
            &child_group.id.to_string(),
            &child_group.name,
            icon,
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
        let protocol = get_protocol_string(&conn.protocol_config);
        let status = sidebar
            .get_connection_status(&conn.id.to_string())
            .unwrap_or_else(|| "disconnected".to_string());
        let icon = conn.icon.as_deref().unwrap_or("");
        let item = ConnectionItem::new_connection_full_with_icon(
            &conn.id.to_string(),
            &conn.name,
            &protocol,
            &conn.host,
            &status,
            conn.is_pinned,
            icon,
        );
        parent_item.add_child(&item);
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
        return;
    };
    let Ok(target_uuid) = Uuid::parse_str(target_id) else {
        return;
    };

    // Handle based on item type
    match item_type {
        "conn" => {
            // Moving a connection
            if target_is_group {
                // Target is a group - move connection into it
                if position == "into" {
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
            // Moving a group - reorder among groups
            if let Ok(mut state_mut) = state.try_borrow_mut()
                && let Err(e) = state_mut.reorder_group(item_uuid, target_uuid)
            {
                tracing::error!(%e, "Failed to reorder group");
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
