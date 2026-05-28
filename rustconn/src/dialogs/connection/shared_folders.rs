//! Shared folders UI components for RDP and SPICE protocols
//!
//! This module provides reusable UI components for managing shared folders
//! that can be used by both RDP and SPICE connection dialogs.

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, FileDialog, Label, Orientation};
use rustconn_core::models::SharedFolder;
use std::cell::RefCell;
use std::rc::Rc;

use crate::i18n::i18n;

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
            .title(i18n("Select Folder to Share"))
            .modal(true)
            .build();

        let folders_list = folders_list_clone.clone();
        let shared_folders = shared_folders_clone.clone();
        let parent = btn.root().and_then(|r| r.downcast::<gtk4::Window>().ok());

        file_dialog.select_folder(
            parent.as_ref(),
            gtk4::gio::Cancellable::NONE,
            move |result| {
                if let Ok(file) = result
                    && let Some(path) = file.path()
                {
                    let share_name = path
                        .file_name()
                        .map_or_else(|| "Share".to_string(), |n| n.to_string_lossy().to_string());

                    let folder = SharedFolder {
                        local_path: path.clone(),
                        share_name: share_name.clone(),
                    };

                    shared_folders.borrow_mut().push(folder);
                    add_folder_row_to_list(&folders_list, &path, &share_name);
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
    row_box.set_margin_top(6);
    row_box.set_margin_bottom(6);
    row_box.set_margin_start(12);
    row_box.set_margin_end(12);

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
        if let Some(selected_row) = folders_list_clone.selected_row()
            && let Ok(index) = usize::try_from(selected_row.index())
            && index < shared_folders_clone.borrow().len()
        {
            shared_folders_clone.borrow_mut().remove(index);
            folders_list_clone.remove(&selected_row);
        }
    });
}
