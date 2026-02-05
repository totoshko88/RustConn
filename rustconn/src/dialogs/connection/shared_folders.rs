//! Shared folders UI components for RDP and SPICE protocols
//!
//! This module provides reusable UI components for managing shared folders
//! that can be used by both RDP and SPICE connection dialogs.

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, FileDialog, Label, Orientation, ScrolledWindow};
use libadwaita as adw;
use rustconn_core::models::SharedFolder;
use std::cell::RefCell;
use std::rc::Rc;

/// Creates the Shared Folders preferences group
///
/// Returns a tuple containing:
/// - The preferences group widget
/// - A reference-counted vector of shared folders
/// - The list box widget for displaying folders
#[must_use]
pub fn create_shared_folders_group() -> (
    adw::PreferencesGroup,
    Rc<RefCell<Vec<SharedFolder>>>,
    gtk4::ListBox,
) {
    let folders_group = adw::PreferencesGroup::builder()
        .title("Shared Folders")
        .description("Local folders accessible from remote session")
        .build();

    let folders_list = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::Single)
        .css_classes(["boxed-list"])
        .build();
    folders_list.set_placeholder(Some(&Label::new(Some("No shared folders"))));

    let folders_scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .min_content_height(80)
        .max_content_height(120)
        .child(&folders_list)
        .build();
    folders_group.add(&folders_scrolled);

    let folders_buttons = GtkBox::new(Orientation::Horizontal, 8);
    folders_buttons.set_halign(gtk4::Align::End);
    folders_buttons.set_margin_top(8);
    let add_folder_btn = Button::builder()
        .label("Add")
        .css_classes(["suggested-action"])
        .build();
    let remove_folder_btn = Button::builder().label("Remove").sensitive(false).build();
    folders_buttons.append(&add_folder_btn);
    folders_buttons.append(&remove_folder_btn);
    folders_group.add(&folders_buttons);

    let shared_folders: Rc<RefCell<Vec<SharedFolder>>> = Rc::new(RefCell::new(Vec::new()));

    // Connect add folder button
    connect_add_folder_button(&add_folder_btn, &folders_list, &shared_folders);

    // Connect remove folder button
    connect_remove_folder_button(&remove_folder_btn, &folders_list, &shared_folders);

    // Enable/disable remove button based on selection
    let remove_btn_for_selection = remove_folder_btn;
    folders_list.connect_row_selected(move |_, row| {
        remove_btn_for_selection.set_sensitive(row.is_some());
    });

    (folders_group, shared_folders, folders_list)
}

/// Connects the add folder button to show file dialog and add folder
pub fn connect_add_folder_button(
    add_btn: &Button,
    folders_list: &gtk4::ListBox,
    shared_folders: &Rc<RefCell<Vec<SharedFolder>>>,
) {
    let folders_list_clone = folders_list.clone();
    let shared_folders_clone = shared_folders.clone();
    add_btn.connect_clicked(move |btn| {
        let file_dialog = FileDialog::builder()
            .title("Select Folder to Share")
            .modal(true)
            .build();

        let folders_list = folders_list_clone.clone();
        let shared_folders = shared_folders_clone.clone();
        let parent = btn.root().and_then(|r| r.downcast::<gtk4::Window>().ok());

        file_dialog.select_folder(
            parent.as_ref(),
            gtk4::gio::Cancellable::NONE,
            move |result| {
                if let Ok(file) = result {
                    if let Some(path) = file.path() {
                        let share_name = path.file_name().map_or_else(
                            || "Share".to_string(),
                            |n| n.to_string_lossy().to_string(),
                        );

                        let folder = SharedFolder {
                            local_path: path.clone(),
                            share_name: share_name.clone(),
                        };

                        shared_folders.borrow_mut().push(folder);
                        add_folder_row_to_list(&folders_list, &path, &share_name);
                    }
                }
            },
        );
    });
}

/// Adds a folder row to the list UI
pub fn add_folder_row_to_list(
    folders_list: &gtk4::ListBox,
    path: &std::path::Path,
    share_name: &str,
) {
    let row_box = GtkBox::new(Orientation::Horizontal, 8);
    row_box.set_margin_top(4);
    row_box.set_margin_bottom(4);
    row_box.set_margin_start(8);
    row_box.set_margin_end(8);

    let path_label = Label::builder()
        .label(path.to_string_lossy().as_ref())
        .hexpand(true)
        .halign(gtk4::Align::Start)
        .ellipsize(gtk4::pango::EllipsizeMode::Middle)
        .build();
    let name_label = Label::builder()
        .label(format!("→ {share_name}"))
        .halign(gtk4::Align::End)
        .build();

    row_box.append(&path_label);
    row_box.append(&name_label);
    folders_list.append(&row_box);
}

/// Connects the remove folder button
pub fn connect_remove_folder_button(
    remove_btn: &Button,
    folders_list: &gtk4::ListBox,
    shared_folders: &Rc<RefCell<Vec<SharedFolder>>>,
) {
    let folders_list_clone = folders_list.clone();
    let shared_folders_clone = shared_folders.clone();
    remove_btn.connect_clicked(move |_| {
        if let Some(selected_row) = folders_list_clone.selected_row() {
            if let Ok(index) = usize::try_from(selected_row.index()) {
                if index < shared_folders_clone.borrow().len() {
                    shared_folders_clone.borrow_mut().remove(index);
                    folders_list_clone.remove(&selected_row);
                }
            }
        }
    });
}
