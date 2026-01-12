//! Connection operations for main window
//!
//! This module contains functions for connection operations like delete,
//! duplicate, copy, paste, and reload sidebar.

use crate::alert;
use crate::sidebar::{ConnectionItem, ConnectionSidebar};
use crate::state::SharedAppState;
use crate::window::MainWindow;
use crate::window_types::get_protocol_string;
use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;

use std::rc::Rc;
use uuid::Uuid;

/// Type alias for shared sidebar reference
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Deletes the selected connection or group
pub fn delete_selected_connection(
    window: &gtk4::Window,
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
        let connection_count = state
            .try_borrow()
            .map(|s| s.count_connections_in_group(id))
            .unwrap_or(0);

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

    let state_clone = state.clone();
    let sidebar_clone = sidebar.clone();
    let window_clone = window.clone();
    alert::show_confirm(
        window,
        &format!("Delete {item_type}?"),
        &detail,
        "Delete",
        true,
        move |confirmed| {
            if confirmed {
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
                            // Defer sidebar reload to prevent UI freeze
                            let state = state_clone.clone();
                            let sidebar = sidebar_clone.clone();
                            glib::idle_add_local_once(move || {
                                MainWindow::reload_sidebar_preserving_state(&state, &sidebar);
                            });
                        }
                        Err(e) => {
                            alert::show_error(&window_clone, "Error Deleting", &e);
                        }
                    }
                }
            }
        },
    );
}

/// Duplicates the selected connection
pub fn duplicate_selected_connection(
    window: &gtk4::Window,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
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

    let (conn, new_name) = if let Ok(state_ref) = state.try_borrow() {
        let Some(conn) = state_ref.get_connection(id).cloned() else {
            return;
        };
        // Generate unique name for duplicate
        let new_name = state_ref
            .generate_unique_connection_name(&format!("{} (copy)", conn.name), conn.protocol);
        (conn, new_name)
    } else {
        return;
    };

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
                // Defer sidebar reload to prevent UI freeze
                let state = state.clone();
                let sidebar = sidebar.clone();
                let window = window.clone();
                glib::idle_add_local_once(move || {
                    MainWindow::reload_sidebar_preserving_state(&state, &sidebar);
                    crate::toast::show_toast_on_window(
                        &window,
                        "Connection duplicated",
                        crate::toast::ToastType::Success,
                    );
                });
            }
            Err(e) => {
                tracing::error!("Failed to duplicate connection: {e}");
                crate::toast::show_toast_on_window(
                    window,
                    "Failed to duplicate connection",
                    crate::toast::ToastType::Error,
                );
            }
        }
    }
}

/// Copies the selected connection to the internal clipboard
pub fn copy_selected_connection(
    window: &gtk4::Window,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
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
        match state_mut.copy_connection(id) {
            Ok(()) => {
                crate::toast::show_toast_on_window(
                    window,
                    "Connection copied to clipboard",
                    crate::toast::ToastType::Info,
                );
            }
            Err(e) => {
                tracing::error!("Failed to copy connection: {e}");
                crate::toast::show_toast_on_window(
                    window,
                    "Failed to copy connection",
                    crate::toast::ToastType::Error,
                );
            }
        }
    }
}

/// Pastes a connection from the internal clipboard
pub fn paste_connection(window: &gtk4::Window, state: &SharedAppState, sidebar: &SharedSidebar) {
    // Check if clipboard has content
    let has_content = state
        .try_borrow()
        .map(|s| s.has_clipboard_content())
        .unwrap_or(false);

    if !has_content {
        crate::toast::show_toast_on_window(
            window,
            "Nothing to paste - copy a connection first",
            crate::toast::ToastType::Warning,
        );
        return;
    }

    if let Ok(mut state_mut) = state.try_borrow_mut() {
        match state_mut.paste_connection() {
            Ok(_) => {
                drop(state_mut);
                // Defer sidebar reload to prevent UI freeze
                let state = state.clone();
                let sidebar = sidebar.clone();
                let window = window.clone();
                glib::idle_add_local_once(move || {
                    MainWindow::reload_sidebar_preserving_state(&state, &sidebar);
                    crate::toast::show_toast_on_window(
                        &window,
                        "Connection pasted",
                        crate::toast::ToastType::Success,
                    );
                });
            }
            Err(e) => {
                tracing::error!("Failed to paste connection: {e}");
                crate::toast::show_toast_on_window(
                    window,
                    "Failed to paste connection",
                    crate::toast::ToastType::Error,
                );
            }
        }
    }
}

/// Reloads the sidebar with current data (preserving hierarchy)
pub fn reload_sidebar(state: &SharedAppState, sidebar: &SharedSidebar) {
    let store = sidebar.store();
    store.remove_all();

    let Ok(state_ref) = state.try_borrow() else {
        tracing::warn!("Could not borrow state for sidebar reload");
        return;
    };

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

/// Builds a list of items to delete with their display names
fn build_delete_item_list(
    state: &SharedAppState,
    selected_ids: &[Uuid],
) -> Option<(Vec<String>, usize, usize)> {
    let state_ref = state.try_borrow().ok()?;
    let mut names: Vec<String> = Vec::new();
    let mut conn_count = 0;
    let mut grp_count = 0;

    for id in selected_ids {
        if let Some(conn) = state_ref.get_connection(*id) {
            names.push(format!("• {} (connection)", conn.name));
            conn_count += 1;
        } else if let Some(group) = state_ref.get_group(*id) {
            names.push(format!("• {} (group)", group.name));
            grp_count += 1;
        }
    }
    Some((names, conn_count, grp_count))
}

/// Creates the bulk delete confirmation dialog UI
fn create_bulk_delete_dialog(
    window: &gtk4::Window,
    item_names: &[String],
    summary: &str,
) -> (adw::Window, gtk4::Button, gtk4::Button) {
    use gtk4::prelude::*;
    use gtk4::Label;

    let dialog = adw::Window::builder()
        .title("Delete Selected Items?")
        .transient_for(window)
        .modal(true)
        .default_width(500)
        .default_height(if item_names.len() > 10 { 400 } else { 300 })
        .build();

    let header = adw::HeaderBar::new();
    let cancel_btn = gtk4::Button::builder().label("Cancel").build();
    let delete_btn = gtk4::Button::builder()
        .label("Delete All")
        .css_classes(["destructive-action"])
        .build();
    header.pack_start(&cancel_btn);
    header.pack_end(&delete_btn);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
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

    // Use ToolbarView for proper adw::Window layout
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content));
    dialog.set_content(Some(&toolbar_view));

    (dialog, cancel_btn, delete_btn)
}

/// Performs bulk deletion and shows results
fn perform_bulk_delete(
    state: &SharedAppState,
    sidebar: &SharedSidebar,
    window: &gtk4::Window,
    selected_ids: Vec<Uuid>,
) {
    let mut success_count = 0;
    let mut failures: Vec<String> = Vec::new();

    if let Ok(mut state_mut) = state.try_borrow_mut() {
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

    // Defer sidebar reload to prevent UI freeze
    let state = state.clone();
    let sidebar = sidebar.clone();
    let window = window.clone();
    glib::idle_add_local_once(move || {
        MainWindow::reload_sidebar_preserving_state(&state, &sidebar);

        // Show results
        if failures.is_empty() {
            alert::show_success(
                &window,
                "Deletion Complete",
                &format!("Successfully deleted {success_count} item(s)."),
            );
        } else {
            alert::show_error(
                &window,
                "Deletion Partially Complete",
                &format!(
                    "Deleted {} item(s).\n\nFailed to delete {} item(s):\n{}",
                    success_count,
                    failures.len(),
                    failures.join("\n")
                ),
            );
        }
    });
}

/// Deletes all selected connections (bulk delete for group operations mode)
pub fn delete_selected_connections(
    window: &gtk4::Window,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
    use gtk4::prelude::*;

    let selected_ids = sidebar.get_selected_ids();

    if selected_ids.is_empty() {
        alert::show_alert(
            window,
            "No Selection",
            "Please select one or more items to delete.",
        );
        return;
    }

    // Build list of items to delete for confirmation
    let Some((item_names, connection_count, group_count)) =
        build_delete_item_list(state, &selected_ids)
    else {
        return;
    };

    let summary = match (connection_count, group_count) {
        (c, 0) => format!("{c} connection(s)"),
        (0, g) => format!("{g} group(s)"),
        (c, g) => format!("{c} connection(s) and {g} group(s)"),
    };

    // Create dialog
    let (dialog, cancel_btn, delete_btn) = create_bulk_delete_dialog(window, &item_names, &summary);

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
        perform_bulk_delete(
            &state_clone,
            &sidebar_clone,
            &window_clone,
            selected_ids.clone(),
        );
    });

    dialog.present();
}

/// Shows dialog to move selected items to a group (supports both connections and groups)
pub fn show_move_selected_to_group_dialog(
    window: &gtk4::Window,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
    let selected_ids = sidebar.get_selected_ids();

    if selected_ids.is_empty() {
        alert::show_alert(
            window,
            "No Selection",
            "Please select one or more items to move.",
        );
        return;
    }

    // Separate connections and groups
    let (connection_ids, group_ids_to_move) = if let Ok(state_ref) = state.try_borrow() {
        let conn_ids: Vec<Uuid> = selected_ids
            .iter()
            .filter(|id| state_ref.get_connection(**id).is_some())
            .copied()
            .collect();
        let grp_ids: Vec<Uuid> = selected_ids
            .iter()
            .filter(|id| state_ref.get_group(**id).is_some())
            .copied()
            .collect();
        (conn_ids, grp_ids)
    } else {
        return;
    };

    let total_items = connection_ids.len() + group_ids_to_move.len();
    if total_items == 0 {
        return;
    }

    // Create dialog
    let move_window = adw::Window::builder()
        .title("Move")
        .transient_for(window)
        .modal(true)
        .default_width(750)
        .build();

    let header = adw::HeaderBar::new();
    let cancel_btn = gtk4::Button::builder().label("Cancel").build();
    let move_btn = gtk4::Button::builder()
        .label("Move")
        .css_classes(["suggested-action"])
        .build();
    header.pack_start(&cancel_btn);
    header.pack_end(&move_btn);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let info_text = if !connection_ids.is_empty() && !group_ids_to_move.is_empty() {
        format!(
            "Select destination for {} connection(s) and {} group(s):",
            connection_ids.len(),
            group_ids_to_move.len()
        )
    } else if !connection_ids.is_empty() {
        format!("Select a group for {} connection(s):", connection_ids.len())
    } else {
        format!("Select parent for {} group(s):", group_ids_to_move.len())
    };

    let info_label = gtk4::Label::builder()
        .label(&info_text)
        .halign(gtk4::Align::Start)
        .build();
    content.append(&info_label);

    // Build group dropdown with hierarchical sorting
    let mut group_paths: Vec<(Uuid, String)> = if let Ok(state_ref) = state.try_borrow() {
        let groups: Vec<_> = state_ref
            .list_groups()
            .iter()
            .map(|g| (*g).clone())
            .collect();

        // Build paths for all groups, excluding groups being moved and their descendants
        groups
            .iter()
            .filter(|g| {
                // Exclude groups being moved
                if group_ids_to_move.contains(&g.id) {
                    return false;
                }
                // Exclude descendants of groups being moved
                for &moving_id in &group_ids_to_move {
                    if is_descendant_of_group(&state_ref, g.id, moving_id) {
                        return false;
                    }
                }
                true
            })
            .map(|g| {
                let path = state_ref
                    .get_group_path(g.id)
                    .unwrap_or_else(|| g.name.clone());
                (g.id, path)
            })
            .collect()
    } else {
        Vec::new()
    };

    // Sort by path (hierarchical + alphabetical)
    group_paths.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));

    let mut group_ids: Vec<Option<Uuid>> = vec![None];
    let first_option = if group_ids_to_move.is_empty() {
        "(No Group)"
    } else {
        "(Root Level)"
    };
    let mut strings: Vec<String> = vec![first_option.to_string()];

    for (id, path) in &group_paths {
        strings.push(path.clone());
        group_ids.push(Some(*id));
    }

    let string_list = gtk4::StringList::new(
        &strings
            .iter()
            .map(std::string::String::as_str)
            .collect::<Vec<_>>(),
    );
    let group_dropdown = gtk4::DropDown::builder()
        .model(&string_list)
        .selected(0)
        .hexpand(true)
        .build();

    content.append(&group_dropdown);

    // Use ToolbarView for proper adw::Window layout
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content));
    move_window.set_content(Some(&toolbar_view));

    // Connect cancel
    let window_clone = move_window.clone();
    cancel_btn.connect_clicked(move |_| {
        window_clone.close();
    });

    // Connect move
    let state_clone = state.clone();
    let sidebar_clone = sidebar.clone();
    let window_clone = move_window.clone();
    let parent_window = window.clone();
    move_btn.connect_clicked(move |_| {
        let choice_idx = group_dropdown.selected() as usize;
        let target_group = if choice_idx < group_ids.len() {
            group_ids[choice_idx]
        } else {
            None
        };

        let mut success_count = 0;
        let mut failures: Vec<String> = Vec::new();

        if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
            // Move connections
            for conn_id in &connection_ids {
                match state_mut.move_connection_to_group(*conn_id, target_group) {
                    Ok(()) => success_count += 1,
                    Err(e) => failures.push(format!("{conn_id}: {e}")),
                }
            }
            // Move groups
            for group_id in &group_ids_to_move {
                match state_mut.move_group_to_parent(*group_id, target_group) {
                    Ok(()) => success_count += 1,
                    Err(e) => failures.push(format!("{group_id}: {e}")),
                }
            }
        }

        // Defer sidebar reload to prevent UI freeze
        let state = state_clone.clone();
        let sidebar = sidebar_clone.clone();
        let window = window_clone.clone();
        let parent = parent_window.clone();
        let failures_clone = failures.clone();
        glib::idle_add_local_once(move || {
            MainWindow::reload_sidebar_preserving_state(&state, &sidebar);
            window.close();

            // Show results if there were failures
            if !failures_clone.is_empty() {
                alert::show_error(
                    &parent,
                    "Move Partially Complete",
                    &format!(
                        "Moved {} item(s).\n\nFailed to move {} item(s):\n{}",
                        success_count,
                        failures_clone.len(),
                        failures_clone.join("\n")
                    ),
                );
            }
        });
    });

    move_window.present();
}

/// Checks if a group is a descendant of another group
fn is_descendant_of_group(
    state: &std::cell::Ref<crate::state::AppState>,
    group_id: Uuid,
    potential_ancestor: Uuid,
) -> bool {
    let mut current_id = state.get_group(group_id).and_then(|g| g.parent_id);
    while let Some(parent_id) = current_id {
        if parent_id == potential_ancestor {
            return true;
        }
        current_id = state.get_group(parent_id).and_then(|g| g.parent_id);
    }
    false
}
