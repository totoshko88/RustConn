//! Remote file editor: download over SSH, edit locally, upload on Save.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk4::gio;
use gtk4::prelude::*;
use libadwaita as adw;
use rustconn_core::models::Connection;
use rustconn_core::remote_editor::{RemoteEditor, RemoteEditorError, RemoteTarget, SaveOutcome};
use uuid::Uuid;

use super::show_error;
use crate::i18n::{i18n, i18n_f};
use crate::state::AppState;
use crate::toast::{ToastType, show_toast_on_window};
use crate::utils::spawn_blocking_with_callback;
use crate::window::types::SharedNotebook;

thread_local! {
    /// Editors whose upload failed and whose dialog was dismissed with Escape.
    /// Keeping them keeps the temporary directory, so the edit is not lost.
    static KEPT_EDITORS: RefCell<Vec<Arc<RemoteEditor>>> = const { RefCell::new(Vec::new()) };
}

type Editor = Arc<RemoteEditor>;

/// Runs a core future to completion on the calling worker thread. Only for use
/// inside `spawn_blocking_with_callback`, never on the GTK main thread.
fn block_on_worker<F: std::future::Future>(future: F) -> Result<F::Output, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map(|rt| rt.block_on(future))
        .map_err(|e| e.to_string())
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
    begin_remote_edit_connection(window, state, &connection);
}

/// Opens the editor for a saved connection; no live session is required.
pub(super) fn begin_remote_edit_connection(
    window: &adw::ApplicationWindow,
    state: &Rc<RefCell<AppState>>,
    connection: &Connection,
) {
    let entry = gtk4::Entry::builder()
        .placeholder_text("/etc/hosts")
        .activates_default(true)
        .build();
    let dialog = adw::AlertDialog::builder()
        .heading(i18n("Edit remote file"))
        .body(&i18n(
            "The file is downloaded to a temporary copy and uploaded when you choose Upload.",
        ))
        .extra_child(&entry)
        .default_response("edit")
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
    let st = state.clone();
    let connection = connection.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "edit" {
            return;
        }
        let Some(win) = win.upgrade() else { return };
        let remote_path = entry.text().to_string();

        // Everything that needs `AppState` is read here, on the main thread.
        let target = {
            let state = st.borrow();
            RemoteTarget::from_connection(
                &connection,
                &state.list_connections_owned(),
                &state.list_groups_owned(),
                &state.settings().network,
                state.settings().ssh_agent_socket.as_deref(),
            )
        };
        let target = match target {
            Ok(target) => target,
            Err(e) => {
                show_error(
                    &win,
                    &i18n("Could not open the remote file"),
                    &e.to_string(),
                );
                return;
            }
        };

        show_toast_on_window(&win, &i18n("Downloading…"), ToastType::Info);
        let win = win.downgrade();
        spawn_blocking_with_callback(
            move || block_on_worker(RemoteEditor::open(target, &remote_path)),
            move |result| {
                let Some(win) = win.upgrade() else { return };
                match flatten(result) {
                    Ok(editor) => {
                        let editor: Editor = Arc::new(editor);
                        open_in_text_editor(&win, &editor);
                        show_upload_dialog(&win, &editor);
                    }
                    Err(e) => show_error(&win, &i18n("Could not open the remote file"), &e),
                }
            },
        );
    });
    dialog.present(Some(window));
}

/// Joins the runtime error and the editor error into one message.
fn flatten<T>(result: Result<Result<T, RemoteEditorError>, String>) -> Result<T, String> {
    result.and_then(|r| r.map_err(|e| e.to_string()))
}

/// Opens the temporary copy with the default `text/plain` handler, so the
/// application is not chosen by the MIME type of attacker-controlled content.
/// Falls back to the portal launcher when no text handler is installed.
fn open_in_text_editor(win: &adw::ApplicationWindow, editor: &Editor) {
    let file = gio::File::for_path(editor.local_path());
    if let Some(app) = gio::AppInfo::default_for_type("text/plain", false) {
        match app.launch(std::slice::from_ref(&file), None::<&gio::AppLaunchContext>) {
            Ok(()) => return,
            Err(e) => tracing::warn!(%e, "Text editor failed to start; using the portal"),
        }
    }
    // `UriLauncher` is portal-aware on Linux and works on macOS/WSLg too.
    gtk4::UriLauncher::new(file.uri().as_str()).launch(
        Some(win),
        None::<&gio::Cancellable>,
        |result| {
            if let Err(e) = result {
                tracing::warn!(%e, "Could not open the system text editor");
            }
        },
    );
}

fn show_upload_dialog(win: &adw::ApplicationWindow, editor: &Editor) {
    let dialog = adw::AlertDialog::builder()
        .heading(i18n("Remote file opened"))
        .body(&i18n_f(
            "Editing {}. Press Upload when the file is saved.",
            &[&editor.local_path().display().to_string()],
        ))
        .default_response("upload")
        .close_response("cancel")
        .build();
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("upload", &i18n("Upload"));
    let win_weak = win.downgrade();
    let editor = editor.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "upload" {
            return;
        }
        if let Some(win) = win_weak.upgrade() {
            upload(&win, &editor);
        }
    });
    dialog.present(Some(win));
}

/// Uploads off the main thread. On failure the editor stays alive in the
/// failure dialog, so the edited copy is never dropped.
fn upload(win: &adw::ApplicationWindow, editor: &Editor) {
    show_toast_on_window(win, &i18n("Uploading…"), ToastType::Info);
    let win_weak = win.downgrade();
    let worker_editor = editor.clone();
    let editor = editor.clone();
    spawn_blocking_with_callback(
        move || block_on_worker(worker_editor.save()),
        move |result| {
            let Some(win) = win_weak.upgrade() else {
                return;
            };
            match result {
                Ok(Ok(SaveOutcome::Unchanged)) => {
                    show_toast_on_window(&win, &i18n("The file was not changed"), ToastType::Info);
                }
                Ok(Ok(SaveOutcome::Uploaded)) => {
                    show_toast_on_window(&win, &i18n("Remote file uploaded"), ToastType::Success);
                }
                Ok(Err(e)) => show_upload_failed(&win, &editor, &e.to_string()),
                Err(e) => show_upload_failed(&win, &editor, &e),
            }
        },
    );
}

/// Tells the user why the upload failed and offers Retry, Save a Copy… and
/// Discard. Escape keeps the editor, so nothing is lost by accident.
fn show_upload_failed(win: &adw::ApplicationWindow, editor: &Editor, reason: &str) {
    let path = editor.local_path().display().to_string();
    let dialog = adw::AlertDialog::builder()
        .heading(i18n("Could not upload the remote file"))
        .body(format!(
            "{reason}\n\n{}",
            i18n_f("Your edited copy is kept at {}.", &[&path])
        ))
        .default_response("retry")
        .close_response("keep")
        .build();
    dialog.add_response("discard", &i18n("Discard"));
    dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
    dialog.add_response("copy", &i18n("Save a Copy…"));
    dialog.add_response("retry", &i18n("Retry"));
    dialog.set_response_appearance("retry", adw::ResponseAppearance::Suggested);
    let win_weak = win.downgrade();
    let editor = editor.clone();
    let reason = reason.to_string();
    dialog.connect_response(None, move |_, response| {
        let Some(win) = win_weak.upgrade() else {
            return;
        };
        match response {
            "retry" => upload(&win, &editor),
            "copy" => save_copy(&win, &editor, &reason),
            "discard" => {}
            _ => {
                show_toast_on_window(
                    &win,
                    &i18n_f(
                        "Edited copy kept at {}",
                        &[&editor.local_path().display().to_string()],
                    ),
                    ToastType::Warning,
                );
                KEPT_EDITORS.with(|kept| kept.borrow_mut().push(editor.clone()));
            }
        }
    });
    dialog.present(Some(win));
}

/// Asks where to save a copy of the edited file, then returns to the failure
/// dialog so the user can still retry the upload.
fn save_copy(win: &adw::ApplicationWindow, editor: &Editor, reason: &str) {
    let name = editor
        .local_path()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let chooser = gtk4::FileDialog::builder()
        .title(i18n("Save a Copy…"))
        .initial_name(name)
        .build();
    let win_weak = win.downgrade();
    let editor = editor.clone();
    let reason = reason.to_string();
    chooser.save(Some(win), None::<&gio::Cancellable>, move |chosen| {
        let Some(win) = win_weak.upgrade() else {
            return;
        };
        let dest: Option<PathBuf> = chosen.ok().and_then(|f| f.path());
        let Some(dest) = dest else {
            // Cancelled: back to the choices.
            show_upload_failed(&win, &editor, &reason);
            return;
        };
        let worker_editor = editor.clone();
        let dest_for_worker = dest.clone();
        spawn_blocking_with_callback(
            move || worker_editor.save_copy(&dest_for_worker),
            move |result| {
                match result {
                    Ok(()) => show_toast_on_window(
                        &win,
                        &i18n_f("Copy saved to {}", &[&dest.display().to_string()]),
                        ToastType::Success,
                    ),
                    Err(e) => show_error(&win, &i18n("Could not save the copy"), &e.to_string()),
                }
                show_upload_failed(&win, &editor, &reason);
            },
        );
    });
}
