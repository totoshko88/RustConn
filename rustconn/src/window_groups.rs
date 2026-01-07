//! Group hierarchy methods for the main window
//!
//! This module contains methods for managing connection groups,
//! including move to group dialog and related functionality.

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Button, HeaderBar, Label, Orientation};
use std::rc::Rc;
use uuid::Uuid;

use crate::sidebar::ConnectionSidebar;
use crate::state::SharedAppState;
use crate::window::MainWindow;

/// Type alias for shared sidebar
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Shows the move to group dialog for the selected item (connection or group)
#[allow(clippy::too_many_lines)]
pub fn show_move_to_group_dialog(
    window: &gtk4::Window,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
    // Get selected item using sidebar's method
    let Some(conn_item) = sidebar.get_selected_item() else {
        return;
    };

    let id_str = conn_item.id();
    let Ok(item_id) = Uuid::parse_str(&id_str) else {
        return;
    };
    let item_name = conn_item.name();
    let is_group = conn_item.is_group();

    // Get current parent group
    let state_ref = state.borrow();
    let current_parent_id = if is_group {
        state_ref.get_group(item_id).and_then(|g| g.parent_id)
    } else {
        state_ref.get_connection(item_id).and_then(|c| c.group_id)
    };
    drop(state_ref);

    // Create dialog
    let move_window = gtk4::Window::builder()
        .title("Move")
        .transient_for(window)
        .modal(true)
        .default_width(750)
        .build();

    let header = HeaderBar::new();
    header.set_show_title_buttons(false);
    let cancel_btn = Button::builder().label("Cancel").build();
    let move_btn = Button::builder()
        .label("Move")
        .css_classes(["suggested-action"])
        .build();
    header.pack_start(&cancel_btn);
    header.pack_end(&move_btn);
    move_window.set_titlebar(Some(&header));

    let content = gtk4::Box::new(Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let info_label = Label::builder()
        .label(format!("Move '{item_name}' to:"))
        .halign(gtk4::Align::Start)
        .build();
    content.append(&info_label);

    // Group dropdown with hierarchical sorting
    let state_ref = state.borrow();
    let groups: Vec<_> = state_ref
        .list_groups()
        .iter()
        .map(|g| (*g).clone())
        .collect();

    // Build paths for all groups, excluding the item itself and its descendants if it's a group
    let mut group_paths: Vec<(Uuid, String)> = groups
        .iter()
        .filter(|g| {
            if is_group {
                // Exclude the group itself and its descendants
                g.id != item_id && !is_descendant_of(&state_ref, g.id, item_id)
            } else {
                true
            }
        })
        .map(|g| {
            let path = state_ref
                .get_group_path(g.id)
                .unwrap_or_else(|| g.name.clone());
            (g.id, path)
        })
        .collect();
    drop(state_ref);

    // Sort by path (hierarchical + alphabetical)
    group_paths.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));

    let mut group_ids: Vec<Option<Uuid>> = vec![None];
    let first_option = if is_group {
        "(Root Level)"
    } else {
        "(Ungrouped)"
    };
    let mut strings: Vec<String> = vec![first_option.to_string()];
    let mut current_index = 0u32;

    for (id, path) in &group_paths {
        strings.push(path.clone());
        group_ids.push(Some(*id));

        if current_parent_id == Some(*id) {
            #[allow(clippy::cast_possible_truncation)]
            {
                current_index = (group_ids.len() - 1) as u32;
            }
        }
    }

    let string_list = gtk4::StringList::new(
        &strings
            .iter()
            .map(std::string::String::as_str)
            .collect::<Vec<_>>(),
    );
    let group_dropdown = gtk4::DropDown::builder()
        .model(&string_list)
        .selected(current_index)
        .hexpand(true)
        .build();

    content.append(&group_dropdown);
    move_window.set_child(Some(&content));

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
        let selected_idx = group_dropdown.selected() as usize;
        let target_parent_id = if selected_idx < group_ids.len() {
            group_ids[selected_idx]
        } else {
            None
        };

        let result = if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
            if is_group {
                state_mut.move_group_to_parent(item_id, target_parent_id)
            } else {
                state_mut.move_connection_to_group(item_id, target_parent_id)
            }
        } else {
            Err("Cannot access state".to_string())
        };

        match result {
            Ok(()) => {
                // Defer sidebar reload to prevent UI freeze
                let state = state_clone.clone();
                let sidebar = sidebar_clone.clone();
                let window = window_clone.clone();
                glib::idle_add_local_once(move || {
                    MainWindow::reload_sidebar_preserving_state(&state, &sidebar);
                    window.close();
                });
            }
            Err(e) => {
                let alert = gtk4::AlertDialog::builder()
                    .message("Error Moving Item")
                    .detail(&e)
                    .modal(true)
                    .build();
                alert.show(Some(&parent_window));
            }
        }
    });

    move_window.present();
}

/// Checks if a group is a descendant of another group
fn is_descendant_of(
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

/// Shows an error toast/notification
pub fn show_error_toast(window: &impl gtk4::prelude::IsA<gtk4::Window>, message: &str) {
    let alert = gtk4::AlertDialog::builder()
        .message("Error")
        .detail(message)
        .modal(true)
        .build();
    alert.show(Some(window));
}
