//! Sorting and drag-drop operations for main window
//!
//! This module contains functions for sorting connections and handling
//! drag-drop reordering operations.

use crate::sidebar::{ConnectionItem, ConnectionSidebar};
use crate::state::SharedAppState;
use crate::window_types::get_protocol_string;
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
        if let Ok(mut state_mut) = state.try_borrow_mut() {
            if let Err(e) = state_mut.sort_group(group_id) {
                eprintln!("Failed to sort group: {e}");
            }
        }
    } else {
        // Sort all groups and connections
        if let Ok(mut state_mut) = state.try_borrow_mut() {
            if let Err(e) = state_mut.sort_all() {
                eprintln!("Failed to sort all: {e}");
            }
        }
    }

    // Rebuild the sidebar to reflect the new sort order
    rebuild_sidebar_sorted(state, sidebar);
}

/// Sorts connections by recent usage (most recently used first)
pub fn sort_recent(state: &SharedAppState, sidebar: &SharedSidebar) {
    // Sort all connections by last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut() {
        if let Err(e) = state_mut.sort_by_recent() {
            eprintln!("Failed to sort by recent: {e}");
        }
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

    drop(state_ref);

    // Rebuild store with sorted items (groups first, then ungrouped)
    store.remove_all();

    let state_ref = state.borrow();

    // Add sorted groups with their sorted children
    for group in &groups {
        let group_item = ConnectionItem::new_group(&group.id.to_string(), &group.name);
        add_sorted_group_children(&state_ref, &group_item, group.id);
        store.append(&group_item);
    }

    // Add sorted ungrouped connections
    for conn in &ungrouped {
        let protocol = get_protocol_string(&conn.protocol_config);
        let item =
            ConnectionItem::new_connection(&conn.id.to_string(), &conn.name, &protocol, &conn.host);
        store.append(&item);
    }
}

/// Recursively adds sorted group children
pub fn add_sorted_group_children(
    state: &std::cell::Ref<crate::state::AppState>,
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
        let child_item = ConnectionItem::new_group(&child_group.id.to_string(), &child_group.name);
        add_sorted_group_children(state, &child_item, child_group.id);
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
        let item =
            ConnectionItem::new_connection(&conn.id.to_string(), &conn.name, &protocol, &conn.host);
        parent_item.add_child(&item);
    }
}

/// Handles drag-drop operations for reordering connections
///
/// Data format: "`item_type:item_id:target_id:target_is_group`"
pub fn handle_drag_drop(state: &SharedAppState, sidebar: &SharedSidebar, data: &str) {
    let parts: Vec<&str> = data.split(':').collect();
    if parts.len() != 4 {
        return;
    }

    let item_type = parts[0];
    let item_id = parts[1];
    let target_id = parts[2];
    let target_is_group = parts[3] == "true";

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
                // Move connection to the target group
                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    if let Err(e) = state_mut.move_connection_to_group(item_uuid, Some(target_uuid))
                    {
                        eprintln!("Failed to move connection to group: {e}");
                        return;
                    }
                }
            } else {
                // Reorder connection relative to target connection
                // Get the target connection's group and position
                let target_group_id = {
                    let state_ref = state.borrow();
                    state_ref
                        .get_connection(target_uuid)
                        .and_then(|c| c.group_id)
                };

                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    // First move to the same group as target
                    if let Err(e) = state_mut.move_connection_to_group(item_uuid, target_group_id) {
                        eprintln!("Failed to move connection: {e}");
                        return;
                    }

                    // Then reorder within the group
                    if let Err(e) = state_mut.reorder_connection(item_uuid, target_uuid) {
                        eprintln!("Failed to reorder connection: {e}");
                        return;
                    }
                }
            }
        }
        "group" => {
            // Moving a group - reorder among groups
            if let Ok(mut state_mut) = state.try_borrow_mut() {
                if let Err(e) = state_mut.reorder_group(item_uuid, target_uuid) {
                    eprintln!("Failed to reorder group: {e}");
                    return;
                }
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
