//! Document management actions for main window
//!
//! This module contains functions for setting up document-related actions:
//! new, open, save, close, export, and import documents.

use crate::sidebar::ConnectionSidebar;
use crate::state::SharedAppState;
use crate::window_groups as groups;
use gtk4::prelude::*;
use gtk4::{gio, ApplicationWindow};
use std::rc::Rc;

/// Type alias for shared sidebar reference
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Sets up document management actions on the window
pub fn setup_document_actions(
    window: &ApplicationWindow,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
    use crate::dialogs::{DocumentDialogResult, NewDocumentDialog};

    // New document action
    let new_doc_action = gio::SimpleAction::new("new-document", None);
    let window_weak = window.downgrade();
    let state_clone = state.clone();
    let sidebar_clone = sidebar.clone();
    new_doc_action.connect_activate(move |_, _| {
        if let Some(win) = window_weak.upgrade() {
            let dialog = NewDocumentDialog::new(Some(&win.clone().upcast()));
            let state_for_cb = state_clone.clone();
            let _sidebar_for_cb = sidebar_clone.clone();
            dialog.set_callback(move |result| {
                if let Some(DocumentDialogResult::Create { name, password: _ }) = result {
                    let mut state_ref = state_for_cb.borrow_mut();
                    let _doc_id = state_ref.create_document(name);
                    drop(state_ref);
                }
            });
            dialog.present();
        }
    });
    window.add_action(&new_doc_action);

    // Open document action
    setup_open_document_action(window, state, sidebar);

    // Save document action
    setup_save_document_action(window, state);

    // Close document action
    setup_close_document_action(window, state, sidebar);

    // Export document action
    setup_export_document_action(window, state);

    // Import document action
    setup_import_document_action(window, state, sidebar);
}

fn setup_open_document_action(
    window: &ApplicationWindow,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
    use crate::dialogs::{DocumentDialogResult, OpenDocumentDialog};

    let open_doc_action = gio::SimpleAction::new("open-document", None);
    let window_weak = window.downgrade();
    let state_clone = state.clone();
    let sidebar_clone = sidebar.clone();
    open_doc_action.connect_activate(move |_, _| {
        if let Some(win) = window_weak.upgrade() {
            let dialog = OpenDocumentDialog::new();
            let state_for_cb = state_clone.clone();
            let _sidebar_for_cb = sidebar_clone.clone();
            let win_for_cb = win.clone();
            dialog.set_callback(move |result| {
                if let Some(DocumentDialogResult::Open { path, password }) = result {
                    let mut state_ref = state_for_cb.borrow_mut();
                    match state_ref.open_document(&path, password.as_deref()) {
                        Ok(_doc_id) => {
                            drop(state_ref);
                        }
                        Err(e) => {
                            drop(state_ref);
                            groups::show_error_toast(
                                &win_for_cb,
                                &format!("Failed to open document: {e}"),
                            );
                        }
                    }
                }
            });
            dialog.present(Some(&win.clone().upcast()));
        }
    });
    window.add_action(&open_doc_action);
}

fn setup_save_document_action(window: &ApplicationWindow, state: &SharedAppState) {
    use crate::dialogs::{DocumentDialogResult, SaveDocumentDialog};

    let save_doc_action = gio::SimpleAction::new("save-document", None);
    let window_weak = window.downgrade();
    let state_clone = state.clone();
    save_doc_action.connect_activate(move |_, _| {
        if let Some(win) = window_weak.upgrade() {
            let state_ref = state_clone.borrow();
            if let Some(doc_id) = state_ref.active_document_id() {
                if let Some(doc) = state_ref.get_document(doc_id) {
                    let doc_name = doc.name.clone();
                    let existing_path =
                        state_ref.get_document_path(doc_id).map(|p| p.to_path_buf());
                    drop(state_ref);

                    if let Some(path) = existing_path {
                        let mut state_ref = state_clone.borrow_mut();
                        if let Err(e) = state_ref.save_document(doc_id, &path, None) {
                            drop(state_ref);
                            groups::show_error_toast(
                                &win,
                                &format!("Failed to save document: {e}"),
                            );
                        }
                    } else {
                        let dialog = SaveDocumentDialog::new();
                        let state_for_cb = state_clone.clone();
                        let win_for_cb = win.clone();
                        dialog.set_callback(move |result| {
                            if let Some(DocumentDialogResult::Save { id, path, password }) = result
                            {
                                let mut state_ref = state_for_cb.borrow_mut();
                                if let Err(e) =
                                    state_ref.save_document(id, &path, password.as_deref())
                                {
                                    drop(state_ref);
                                    groups::show_error_toast(
                                        &win_for_cb,
                                        &format!("Failed to save document: {e}"),
                                    );
                                }
                            }
                        });
                        dialog.present(Some(&win.clone().upcast()), doc_id, &doc_name);
                    }
                }
            }
        }
    });
    window.add_action(&save_doc_action);
}

fn setup_close_document_action(
    window: &ApplicationWindow,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
    use crate::dialogs::{CloseDocumentDialog, DocumentDialogResult};

    let close_doc_action = gio::SimpleAction::new("close-document", None);
    let window_weak = window.downgrade();
    let state_clone = state.clone();
    let sidebar_clone = sidebar.clone();
    close_doc_action.connect_activate(move |_, _| {
        if let Some(win) = window_weak.upgrade() {
            let state_ref = state_clone.borrow();
            if let Some(doc_id) = state_ref.active_document_id() {
                let is_dirty = state_ref.is_document_dirty(doc_id);
                let doc_name = state_ref
                    .get_document(doc_id)
                    .map(|d| d.name.clone())
                    .unwrap_or_else(|| "Untitled".to_string());
                drop(state_ref);

                if is_dirty {
                    let dialog = CloseDocumentDialog::new();
                    let state_for_cb = state_clone.clone();
                    let _sidebar_for_cb = sidebar_clone.clone();
                    let _win_for_cb = win.clone();
                    dialog.set_callback(move |result| match result {
                        Some(DocumentDialogResult::Close { id, save: true }) => {
                            let state_ref = state_for_cb.borrow();
                            let existing_path =
                                state_ref.get_document_path(id).map(|p| p.to_path_buf());
                            drop(state_ref);

                            if let Some(path) = existing_path {
                                let mut state_ref = state_for_cb.borrow_mut();
                                let _ = state_ref.save_document(id, &path, None);
                                let _ = state_ref.close_document(id);
                            }
                        }
                        Some(DocumentDialogResult::Close { id, save: false }) => {
                            let mut state_ref = state_for_cb.borrow_mut();
                            let _ = state_ref.close_document(id);
                        }
                        _ => {}
                    });
                    dialog.present(Some(&win.clone().upcast()), doc_id, &doc_name);
                } else {
                    let mut state_ref = state_clone.borrow_mut();
                    let _ = state_ref.close_document(doc_id);
                }
            }
        }
    });
    window.add_action(&close_doc_action);
}

fn setup_export_document_action(window: &ApplicationWindow, state: &SharedAppState) {
    let export_doc_action = gio::SimpleAction::new("export-document", None);
    let window_weak = window.downgrade();
    let state_clone = state.clone();
    export_doc_action.connect_activate(move |_, _| {
        if let Some(win) = window_weak.upgrade() {
            let state_ref = state_clone.borrow();
            if let Some(doc_id) = state_ref.active_document_id() {
                if let Some(doc) = state_ref.get_document(doc_id) {
                    let doc_name = doc.name.clone();
                    drop(state_ref);

                    let filter = gtk4::FileFilter::new();
                    filter.add_pattern("*.json");
                    filter.add_pattern("*.yaml");
                    filter.set_name(Some("Document Files"));

                    let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
                    filters.append(&filter);

                    let dialog = gtk4::FileDialog::builder()
                        .title("Export Document")
                        .filters(&filters)
                        .initial_name(format!("{doc_name}.json"))
                        .modal(true)
                        .build();

                    let state_for_cb = state_clone.clone();
                    let win_for_cb = win.clone();

                    dialog.save(
                        Some(&win.clone().upcast::<gtk4::Window>()),
                        gtk4::gio::Cancellable::NONE,
                        move |result| {
                            if let Ok(file) = result {
                                if let Some(path) = file.path() {
                                    let state_ref = state_for_cb.borrow();
                                    if let Err(e) = state_ref.export_document(doc_id, &path) {
                                        drop(state_ref);
                                        groups::show_error_toast(
                                            &win_for_cb,
                                            &format!("Failed to export document: {e}"),
                                        );
                                    }
                                }
                            }
                        },
                    );
                }
            }
        }
    });
    window.add_action(&export_doc_action);
}

fn setup_import_document_action(
    window: &ApplicationWindow,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
    let import_doc_action = gio::SimpleAction::new("import-document", None);
    let window_weak = window.downgrade();
    let state_clone = state.clone();
    let sidebar_clone = sidebar.clone();
    import_doc_action.connect_activate(move |_, _| {
        if let Some(win) = window_weak.upgrade() {
            let filter = gtk4::FileFilter::new();
            filter.add_pattern("*.json");
            filter.add_pattern("*.yaml");
            filter.add_pattern("*.yml");
            filter.add_pattern("*.rcdb");
            filter.set_name(Some("Document Files"));

            let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
            filters.append(&filter);

            let dialog = gtk4::FileDialog::builder()
                .title("Import Document")
                .filters(&filters)
                .modal(true)
                .build();

            let state_for_cb = state_clone.clone();
            let _sidebar_for_cb = sidebar_clone.clone();
            let win_for_cb = win.clone();

            dialog.open(
                Some(&win.clone().upcast::<gtk4::Window>()),
                gtk4::gio::Cancellable::NONE,
                move |result| {
                    if let Ok(file) = result {
                        if let Some(path) = file.path() {
                            let mut state_ref = state_for_cb.borrow_mut();
                            match state_ref.import_document(&path) {
                                Ok(_doc_id) => {
                                    drop(state_ref);
                                }
                                Err(e) => {
                                    drop(state_ref);
                                    groups::show_error_toast(
                                        &win_for_cb,
                                        &format!("Failed to import document: {e}"),
                                    );
                                }
                            }
                        }
                    }
                },
            );
        }
    });
    window.add_action(&import_doc_action);
}
