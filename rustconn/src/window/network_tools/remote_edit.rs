//! Remote file editor: download over SSH, edit locally, upload on Save.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::gio;
use gtk4::prelude::*;
use libadwaita as adw;
use rustconn_core::models::Connection;
use rustconn_core::remote_editor::{RemoteEditor, SaveOutcome};
use uuid::Uuid;

use super::show_error;
use crate::async_utils::with_runtime;
use crate::i18n::{i18n, i18n_f};
use crate::state::AppState;
use crate::toast::{ToastType, show_toast_on_window};
use crate::window::types::SharedNotebook;

thread_local! {
    static ACTIVE_EDITOR: RefCell<Option<RemoteEditor>> = const { RefCell::new(None) };
}

/// Downloads a remote file, opens it in the system editor, then uploads on Save.
pub(super) fn begin_remote_edit(
    window: &adw::ApplicationWindow,
    state: &Rc<RefCell<AppState>>,
    notebook: &SharedNotebook,
    session_id: Option<Uuid>,
) {
    let Some(session) = session_id
        .or_else(|| notebook.get_active_session_id())
        .and_then(|id| notebook.get_session_info(id))
    else {
        show_toast_on_window(
            window,
            &i18n("Select an SSH terminal before editing a remote file"),
            ToastType::Warning,
        );
        return;
    };
    let connection = {
        let mut state = state.borrow_mut();
        state
            .connection_manager()
            .get_connection(session.connection_id)
            .cloned()
    };
    let Some(connection) = connection else {
        show_error(
            window,
            &i18n("Remote file editor"),
            &i18n("Connection not found"),
        );
        return;
    };
    begin_remote_edit_connection(window, &connection);
}

/// Opens the editor for a saved connection; no live session is required.
pub(super) fn begin_remote_edit_connection(
    window: &adw::ApplicationWindow,
    connection: &Connection,
) {
    let entry = gtk4::Entry::builder()
        .placeholder_text("/etc/hosts")
        .build();
    let dialog = adw::AlertDialog::builder()
        .heading(i18n("Edit remote file"))
        .body(&i18n(
            "The file is downloaded to a temporary copy and uploaded when you press Save.",
        ))
        .extra_child(&entry)
        .default_response("cancel")
        .close_response("cancel")
        .build();
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("edit", &i18n("Edit"));
    dialog.set_response_enabled("edit", false);
    let dlg = dialog.clone();
    entry.connect_changed(move |entry| {
        dlg.set_response_enabled("edit", !entry.text().is_empty());
    });

    let win = window.downgrade();
    let st_connection = connection.to_owned();
    dialog.connect_response(None, move |_, response| {
        if response != "edit" {
            return;
        }
        let Some(win) = win.upgrade() else { return };
        let remote_path = entry.text().to_string();
        let conn = st_connection.clone();
        let downloaded =
            match with_runtime(|rt| rt.block_on(RemoteEditor::open(&conn, &remote_path))) {
                Ok(Ok(editor)) => editor,
                Ok(Err(e)) => {
                    show_error(
                        &win,
                        &i18n("Could not open the remote file"),
                        &e.to_string(),
                    );
                    return;
                }
                Err(e) => {
                    show_error(&win, &i18n("Could not open the remote file"), &e);
                    return;
                }
            };

        // `UriLauncher` is portal-aware on Linux and works on macOS/WSLg too.
        let local_uri = gio::File::for_path(downloaded.local_path())
            .uri()
            .to_string();
        {
            gtk4::UriLauncher::new(&local_uri).launch(
                Some(&win),
                None::<&gio::Cancellable>,
                move |result| {
                    if let Err(e) = result {
                        tracing::warn!(%e, "Could not open the system text editor");
                    }
                },
            );
        }

        let path = downloaded.local_path().display().to_string();
        ACTIVE_EDITOR.with(|cell| {
            let _ = cell.borrow_mut().replace(downloaded);
        });
        let save_dialog = adw::AlertDialog::builder()
            .heading(i18n("Remote file opened"))
            .body(&i18n_f(
                "Editing {}. Press Upload when the file is saved.",
                &[&path],
            ))
            .default_response("upload")
            .close_response("cancel")
            .build();
        save_dialog.add_response("cancel", &i18n("Cancel"));
        save_dialog.add_response("upload", &i18n("Upload"));
        let win2 = win.downgrade();
        save_dialog.connect_response(None, move |_, response| {
            if response != "upload" {
                ACTIVE_EDITOR.with(|cell| {
                    cell.borrow_mut().take();
                });
                return;
            }
            let editor = ACTIVE_EDITOR.with(|cell| cell.borrow_mut().take());
            let Some(editor) = editor else { return };
            match with_runtime(|rt| rt.block_on(editor.save())) {
                Ok(Ok(SaveOutcome::Unchanged)) => {
                    if let Some(win) = win2.upgrade() {
                        show_toast_on_window(
                            &win,
                            &i18n("The file was not changed"),
                            ToastType::Info,
                        );
                    }
                }
                Ok(Ok(SaveOutcome::Uploaded)) => {
                    if let Some(win) = win2.upgrade() {
                        show_toast_on_window(
                            &win,
                            &i18n("Remote file uploaded"),
                            ToastType::Success,
                        );
                    }
                }
                Ok(Err(e)) => {
                    if let Some(win) = win2.upgrade() {
                        show_error(
                            &win,
                            &i18n("Could not upload the remote file"),
                            &e.to_string(),
                        );
                    }
                }
                Err(e) => {
                    if let Some(win) = win2.upgrade() {
                        show_error(&win, &i18n("Could not upload the remote file"), &e);
                    }
                }
            }
        });
        save_dialog.present(Some(&win));
    });
    dialog.present(Some(window));
}
