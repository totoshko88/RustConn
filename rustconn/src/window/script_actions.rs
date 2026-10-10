//! Script recording window actions
//!
//! Extracted from `window/mod.rs` to reduce module complexity.
//!
//! # Why these actions are not in the app menu
//!
//! Both actions act on "the session the user is looking at", and the reason
//! `execute-snippet` is kept out of the app menu (`snippet_actions.rs`) applies
//! verbatim: the app menu hangs off the main window, so by the time an item in
//! it is activated the focused terminal may have changed, and recording the
//! wrong session produces a script that types one host's commands into another.
//! Recording is also not idempotent — the damage is a file on disk, not a
//! retried command.
//!
//! The reachable route is therefore the terminal's own right-click menu, where
//! the target is chosen in the same gesture. See `terminal/config.rs`.
//!
//! These are free functions taking `&adw::ApplicationWindow` rather than
//! `MainWindow` methods: `MainWindow` is a plain struct, not a `glib::Object`,
//! so a `SimpleAction` closure holding only the window cannot reach back to it.
//! Same shape as `window/snippets.rs`.

use super::*;
use crate::i18n::{i18n, i18n_f};
use crate::terminal::script_recorder::ScriptSaveError;

/// Registers the script recording actions.
pub(crate) fn setup_script_actions(
    window: &adw::ApplicationWindow,
    terminal_notebook: &SharedNotebook,
    session_bridges: &SessionSplitBridges,
) {
    let record_action = gio::SimpleAction::new("record-script", None);
    let window_weak = window.downgrade();
    let notebook = terminal_notebook.clone();
    let bridges = session_bridges.clone();
    record_action.connect_activate(move |_, _| {
        if let Some(win) = window_weak.upgrade() {
            toggle_script_recording(&win, &notebook, &bridges);
        }
    });
    window.add_action(&record_action);

    let stop_action = gio::SimpleAction::new("stop-record-script", None);
    let window_weak = window.downgrade();
    let notebook = terminal_notebook.clone();
    stop_action.connect_activate(move |_, _| {
        if let Some(win) = window_weak.upgrade() {
            finish_script_recording(&win, &notebook);
        }
    });
    window.add_action(&stop_action);
}

/// Starts recording, or stops the recording already in progress.
///
/// One action rather than two because only one of the two is ever meaningful;
/// the context menu label says which.
fn toggle_script_recording(
    window: &adw::ApplicationWindow,
    notebook: &SharedNotebook,
    bridges: &SessionSplitBridges,
) {
    if notebook.is_script_recording() {
        finish_script_recording(window, notebook);
    } else {
        begin_script_recording(window, notebook, bridges);
    }
}

/// Begins recording on the focused session.
fn begin_script_recording(
    window: &adw::ApplicationWindow,
    notebook: &SharedNotebook,
    bridges: &SessionSplitBridges,
) {
    let Some(session_id) = focused_session_for_scripting(notebook, bridges) else {
        crate::toast::show_toast_on_window(
            window,
            &i18n("Select a terminal before recording a script"),
            crate::toast::ToastType::Warning,
        );
        return;
    };

    let name = notebook
        .get_session_info(session_id)
        .map_or_else(|| i18n("session"), |s| s.name.clone());

    if !notebook.start_script_recording(session_id, &name) {
        crate::toast::show_toast_on_window(
            window,
            &i18n("Could not start recording"),
            crate::toast::ToastType::Error,
        );
        return;
    }
    notebook.rebuild_script_menu_section();

    tracing::info!(%session_id, "Script recording started from terminal menu");
    crate::toast::show_toast_on_window(
        window,
        &i18n("Recording — open the terminal menu to stop"),
        crate::toast::ToastType::Info,
    );
}

/// Stops recording and reports where the file went.
fn finish_script_recording(window: &adw::ApplicationWindow, notebook: &SharedNotebook) {
    match notebook.stop_script_recording() {
        Ok(recorded) => {
            notebook.rebuild_script_menu_section();
            let path = recorded.path.display().to_string();
            let steps = recorded.step_count.to_string();
            let message = if recorded.step_count == 0 {
                i18n_f("Recording saved with no steps to {}", &[&path])
            } else {
                i18n_f("Saved {} steps to {}", &[&steps, &path])
            };
            crate::toast::show_toast_on_window(window, &message, crate::toast::ToastType::Success);
        }
        // Nothing was running: the menu item cannot be reached in this state,
        // so there is nothing to tell the user.
        Err(ScriptSaveError::NoRecording) => {}
        // The keystrokes only ever existed in memory. A save failure loses
        // them, so it gets a modal dialog rather than a log line.
        Err(e) => {
            notebook.rebuild_script_menu_section();
            tracing::error!(%e, "Failed to save recorded script");
            let reason = e.to_string();
            crate::alert::show_error(
                window,
                &i18n("Could Not Save Script"),
                &i18n_f(
                    "The recording was lost and the file was not written. {}",
                    &[&reason],
                ),
            );
        }
    }
}

/// The session a script action should target.
///
/// Resolves through the active tab's split bridge, so a split whose focused
/// pane is not the tab owner still records where the user is typing — the
/// same chain `send_text_to_focused` uses, and the reason a bare
/// `get_active_session_id()` is wrong here.
fn focused_session_for_scripting(
    notebook: &SharedNotebook,
    bridges: &SessionSplitBridges,
) -> Option<Uuid> {
    if let Some(active_session_id) = notebook.get_active_session_id() {
        let borrowed = bridges.borrow();
        if let Some(bridge) = borrowed.get(&active_session_id)
            && let Some(focused) = bridge.get_focused_session()
        {
            return Some(focused);
        }
    }
    notebook.get_active_session_id()
}
