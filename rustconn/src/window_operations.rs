//! Connection operations for main window
//!
//! This module contains functions for connection operations like delete,
//! duplicate, copy, paste, and reload sidebar.

use crate::sidebar::{ConnectionItem, ConnectionSidebar};
use crate::state::SharedAppState;
use crate::window::MainWindow;
use crate::window_types::get_protocol_string;
use gtk4::gio;
use gtk4::ApplicationWindow;
use std::rc::Rc;
use uuid::Uuid;

/// Type alias for shared sidebar reference
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Deletes the selected connection or group
pub fn delete_selected_connection(
    window: &ApplicationWindow,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
    // Get selected item using sidebar's method (works in both single and multi-selection modes)
    let Some(conn_item) = sidebar.get_selected_item() else {
        return;
    };

    let id_str = conn_item.id();
    let Ok(id) = Uuid::parse_str(&id_str) else {
        return;
    };
    let name = conn_item.name();
    let is_group = conn_item.is_group();

    // Show confirmation dialog with connection count for groups
    let item_type = if is_group { "group" } else { "connection" };
    let detail = if is_group {
        let state_ref = state.borrow();
        let connection_count = state_ref.count_connections_in_group(id);
        drop(state_ref);

        if connection_count > 0 {
            format!(
                "Are you sure you want to delete the group '{name}'?\n\n\
                 This will also delete {connection_count} connection(s) in this group."
            )
        } else {
            format!("Are you sure you want to delete the empty group '{name}'?")
        }
    } else {
        format!("Are you sure you want to delete the connection '{name}'?")
    };

    let alert = gtk4::AlertDialog::builder()
        .message(format!("Delete {item_type}?"))
        .detail(&detail)
        .buttons(["Cancel", "Delete"])
        .default_button(0)
        .cancel_button(0)
        .modal(true)
        .build();

    let state_clone = state.clone();
    let sidebar_clone = sidebar.clone();
    let window_clone = window.clone();
    alert.choose(Some(window), gio::Cancellable::NONE, move |result| {
        if result == Ok(1) {
            // "Delete" button
            if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                let delete_result = if is_group {
                    // Use cascade delete to remove group and all its connections
                    state_mut.delete_group_cascade(id)
                } else {
                    state_mut.delete_connection(id)
                };

                match delete_result {
                    Ok(()) => {
                        drop(state_mut);
                        // Preserve tree state when deleting (scroll position, other expanded groups)
                        MainWindow::reload_sidebar_preserving_state(&state_clone, &sidebar_clone);
                    }
                    Err(e) => {
                        let error_alert = gtk4::AlertDialog::builder()
                            .message("Error Deleting")
                            .detail(&e)
                            .modal(true)
                            .build();
                        error_alert.show(Some(&window_clone));
                    }
                }
            }
        }
    });
}

/// Duplicates the selected connection
pub fn duplicate_selected_connection(state: &SharedAppState, sidebar: &SharedSidebar) {
    // Get selected item using sidebar's method (works in both single and multi-selection modes)
    let Some(conn_item) = sidebar.get_selected_item() else {
        return;
    };

    // Can only duplicate connections, not groups
    if conn_item.is_group() {
        return;
    }

    let id_str = conn_item.id();
    let Ok(id) = Uuid::parse_str(&id_str) else {
        return;
    };

    let state_ref = state.borrow();
    let Some(conn) = state_ref.get_connection(id).cloned() else {
        return;
    };

    // Generate unique name for duplicate
    let new_name =
        state_ref.generate_unique_connection_name(&format!("{} (copy)", conn.name), conn.protocol);
    drop(state_ref);

    // Create duplicate with new ID and name
    let mut duplicate = conn;
    duplicate.id = Uuid::new_v4();
    duplicate.name = new_name;
    duplicate.created_at = chrono::Utc::now();
    duplicate.updated_at = chrono::Utc::now();

    if let Ok(mut state_mut) = state.try_borrow_mut() {
        match state_mut
            .connection_manager()
            .create_connection_from(duplicate)
        {
            Ok(_) => {
                drop(state_mut);
                // Preserve tree state when duplicating
                MainWindow::reload_sidebar_preserving_state(state, sidebar);
            }
            Err(e) => {
                eprintln!("Failed to duplicate connection: {e}");
            }
        }
    }
}

/// Copies the selected connection to the internal clipboard
pub fn copy_selected_connection(state: &SharedAppState, sidebar: &SharedSidebar) {
    // Get selected item using sidebar's method
    let Some(conn_item) = sidebar.get_selected_item() else {
        return;
    };

    // Can only copy connections, not groups
    if conn_item.is_group() {
        return;
    }

    let id_str = conn_item.id();
    let Ok(id) = Uuid::parse_str(&id_str) else {
        return;
    };

    if let Ok(mut state_mut) = state.try_borrow_mut() {
        if let Err(e) = state_mut.copy_connection(id) {
            eprintln!("Failed to copy connection: {e}");
        }
    }
}

/// Pastes a connection from the internal clipboard
pub fn paste_connection(state: &SharedAppState, sidebar: &SharedSidebar) {
    // Check if clipboard has content
    {
        let state_ref = state.borrow();
        if !state_ref.has_clipboard_content() {
            return;
        }
    }

    if let Ok(mut state_mut) = state.try_borrow_mut() {
        match state_mut.paste_connection() {
            Ok(_) => {
                drop(state_mut);
                // Preserve tree state when pasting
                MainWindow::reload_sidebar_preserving_state(state, sidebar);
            }
            Err(e) => {
                eprintln!("Failed to paste connection: {e}");
            }
        }
    }
}

/// Reloads the sidebar with current data (preserving hierarchy)
pub fn reload_sidebar(state: &SharedAppState, sidebar: &SharedSidebar) {
    let store = sidebar.store();
    store.remove_all();

    let state_ref = state.borrow();

    // Add root groups with their children
    for group in state_ref.get_root_groups() {
        let group_item = ConnectionItem::new_group(&group.id.to_string(), &group.name);
        add_group_children_static(&state_ref, sidebar, &group_item, group.id);
        store.append(&group_item);
    }

    // Add ungrouped connections
    for conn in state_ref.get_ungrouped_connections() {
        let protocol = get_protocol_string(&conn.protocol_config);
        let status = sidebar
            .get_connection_status(&conn.id.to_string())
            .unwrap_or_else(|| "disconnected".to_string());
        let item = ConnectionItem::new_connection_with_status(
            &conn.id.to_string(),
            &conn.name,
            &protocol,
            &conn.host,
            &status,
        );
        store.append(&item);
    }
}

/// Recursively adds children to a group item (static version)
pub fn add_group_children_static(
    state: &std::cell::Ref<crate::state::AppState>,
    sidebar: &SharedSidebar,
    parent_item: &ConnectionItem,
    group_id: Uuid,
) {
    // Add child groups first
    for child_group in state.get_child_groups(group_id) {
        let child_item = ConnectionItem::new_group(&child_group.id.to_string(), &child_group.name);
        add_group_children_static(state, sidebar, &child_item, child_group.id);
        parent_item.add_child(&child_item);
    }

    // Add connections in this group
    for conn in state.get_connections_by_group(group_id) {
        let protocol = get_protocol_string(&conn.protocol_config);
        let status = sidebar
            .get_connection_status(&conn.id.to_string())
            .unwrap_or_else(|| "disconnected".to_string());
        let item = ConnectionItem::new_connection_with_status(
            &conn.id.to_string(),
            &conn.name,
            &protocol,
            &conn.host,
            &status,
        );
        parent_item.add_child(&item);
    }
}

/// Deletes all selected connections (bulk delete for group operations mode)
#[allow(clippy::too_many_lines)]
pub fn delete_selected_connections(
    window: &ApplicationWindow,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
    use gtk4::prelude::*;
    use gtk4::{Button, HeaderBar, Label, Orientation};

    let selected_ids = sidebar.get_selected_ids();

    if selected_ids.is_empty() {
        let alert = gtk4::AlertDialog::builder()
            .message("No Selection")
            .detail("Please select one or more items to delete.")
            .modal(true)
            .build();
        alert.show(Some(window));
        return;
    }

    // Build list of items to delete for confirmation
    let state_ref = state.borrow();
    let mut item_names: Vec<String> = Vec::new();
    let mut connection_count = 0;
    let mut group_count = 0;

    for id in &selected_ids {
        if let Some(conn) = state_ref.get_connection(*id) {
            item_names.push(format!("• {} (connection)", conn.name));
            connection_count += 1;
        } else if let Some(group) = state_ref.get_group(*id) {
            item_names.push(format!("• {} (group)", group.name));
            group_count += 1;
        }
    }
    drop(state_ref);

    let summary = match (connection_count, group_count) {
        (c, 0) => format!("{c} connection(s)"),
        (0, g) => format!("{g} group(s)"),
        (c, g) => format!("{c} connection(s) and {g} group(s)"),
    };

    // Create custom dialog with scrolling for large lists
    let dialog = gtk4::Window::builder()
        .title("Delete Selected Items?")
        .transient_for(window)
        .modal(true)
        .default_width(400)
        .default_height(if item_names.len() > 10 { 400 } else { 250 })
        .build();

    let header = HeaderBar::new();
    let cancel_btn = Button::builder().label("Cancel").build();
    let delete_btn = Button::builder()
        .label("Delete All")
        .css_classes(["destructive-action"])
        .build();
    header.pack_start(&cancel_btn);
    header.pack_end(&delete_btn);
    dialog.set_titlebar(Some(&header));

    let content = gtk4::Box::new(Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // Summary label
    let summary_label = Label::builder()
        .label(format!("Are you sure you want to delete {summary}?"))
        .halign(gtk4::Align::Start)
        .wrap(true)
        .build();
    content.append(&summary_label);

    // Scrolled list of items
    let scrolled = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .min_content_height(100)
        .max_content_height(250)
        .vexpand(true)
        .build();

    let items_label = Label::builder()
        .label(item_names.join("\n"))
        .halign(gtk4::Align::Start)
        .valign(gtk4::Align::Start)
        .wrap(true)
        .selectable(true)
        .build();
    scrolled.set_child(Some(&items_label));
    content.append(&scrolled);

    // Warning label
    let warning_label = Label::builder()
        .label("Connections in deleted groups will become ungrouped.")
        .halign(gtk4::Align::Start)
        .wrap(true)
        .css_classes(["dim-label"])
        .build();
    content.append(&warning_label);

    dialog.set_child(Some(&content));

    // Connect cancel button
    let dialog_weak = dialog.downgrade();
    cancel_btn.connect_clicked(move |_| {
        if let Some(d) = dialog_weak.upgrade() {
            d.close();
        }
    });

    // Connect delete button
    let dialog_weak = dialog.downgrade();
    let state_clone = state.clone();
    let sidebar_clone = sidebar.clone();
    let window_clone = window.clone();
    delete_btn.connect_clicked(move |_| {
        if let Some(d) = dialog_weak.upgrade() {
            d.close();
        }

        let mut success_count = 0;
        let mut failures: Vec<String> = Vec::new();

        if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
            for id in &selected_ids {
                // Try to delete as connection first, then as group
                let delete_result = state_mut
                    .delete_connection(*id)
                    .or_else(|_| state_mut.delete_group(*id));

                match delete_result {
                    Ok(()) => success_count += 1,
                    Err(e) => failures.push(format!("{id}: {e}")),
                }
            }
        }

        // Reload sidebar preserving state
        MainWindow::reload_sidebar_preserving_state(&state_clone, &sidebar_clone);

        // Show results
        if failures.is_empty() {
            let success_alert = gtk4::AlertDialog::builder()
                .message("Deletion Complete")
                .detail(format!("Successfully deleted {success_count} item(s)."))
                .modal(true)
                .build();
            success_alert.show(Some(&window_clone));
        } else {
            let error_alert = gtk4::AlertDialog::builder()
                .message("Deletion Partially Complete")
                .detail(format!(
                    "Deleted {} item(s).\n\nFailed to delete {} item(s):\n{}",
                    success_count,
                    failures.len(),
                    failures.join("\n")
                ))
                .modal(true)
                .build();
            error_alert.show(Some(&window_clone));
        }
    });

    dialog.present();
}

/// Shows dialog to move selected items to a group
pub fn show_move_selected_to_group_dialog(
    window: &ApplicationWindow,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
    let selected_ids = sidebar.get_selected_ids();

    if selected_ids.is_empty() {
        let alert = gtk4::AlertDialog::builder()
            .message("No Selection")
            .detail("Please select one or more connections to move.")
            .modal(true)
            .build();
        alert.show(Some(window));
        return;
    }

    // Filter to only connections (not groups)
    let state_ref = state.borrow();
    let connection_ids: Vec<Uuid> = selected_ids
        .iter()
        .filter(|id| state_ref.get_connection(**id).is_some())
        .copied()
        .collect();
    drop(state_ref);

    if connection_ids.is_empty() {
        let alert = gtk4::AlertDialog::builder()
            .message("No Connections Selected")
            .detail(
                "Only connections can be moved to groups. Please select at least one connection.",
            )
            .modal(true)
            .build();
        alert.show(Some(window));
        return;
    }

    // Build group selection dialog
    let state_ref = state.borrow();
    let groups = state_ref.list_groups();
    let mut group_names: Vec<String> = vec!["(No Group)".to_string()];
    let mut group_ids: Vec<Option<Uuid>> = vec![None];

    for group in groups {
        group_names.push(group.name.clone());
        group_ids.push(Some(group.id));
    }
    drop(state_ref);

    let alert = gtk4::AlertDialog::builder()
        .message("Move to Group")
        .detail(format!(
            "Select a group for {} connection(s):",
            connection_ids.len()
        ))
        .buttons(group_names)
        .default_button(0)
        .cancel_button(-1)
        .modal(true)
        .build();

    let state_clone = state.clone();
    let sidebar_clone = sidebar.clone();
    let window_clone = window.clone();
    alert.choose(Some(window), gio::Cancellable::NONE, move |result| {
        if let Ok(choice) = result {
            #[allow(clippy::cast_sign_loss)]
            let choice_idx = choice as usize;
            if choice_idx < group_ids.len() {
                let target_group = group_ids[choice_idx];
                let mut success_count = 0;
                let mut failures: Vec<String> = Vec::new();

                if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                    for conn_id in &connection_ids {
                        match state_mut.move_connection_to_group(*conn_id, target_group) {
                            Ok(()) => success_count += 1,
                            Err(e) => failures.push(format!("{conn_id}: {e}")),
                        }
                    }
                }

                // Reload sidebar preserving state
                MainWindow::reload_sidebar_preserving_state(&state_clone, &sidebar_clone);

                // Show results if there were failures
                if !failures.is_empty() {
                    let error_alert = gtk4::AlertDialog::builder()
                        .message("Move Partially Complete")
                        .detail(format!(
                            "Moved {} connection(s).\n\nFailed to move {} connection(s):\n{}",
                            success_count,
                            failures.len(),
                            failures.join("\n")
                        ))
                        .modal(true)
                        .build();
                    error_alert.show(Some(&window_clone));
                }
            }
        }
    });
}
