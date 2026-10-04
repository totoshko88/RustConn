//! Per-connection command-macro keybind dispatcher.
//!
//! Command macros (see [`rustconn_core::models::CommandMacro`]) belong to a
//! single connection and may carry an optional GTK accelerator. Because the
//! accelerator is only meaningful while that connection's session is the active
//! tab, the binding cannot be a static entry in the keybindings table — it has
//! to follow the active tab.
//!
//! The dispatcher does this with one parameterized window action,
//! `win.fire-connection-macro` (the string parameter is the macro's index in the
//! active connection's `command_macros`), and a `selected-page` hook that
//! re-registers the active connection's macro accelerators on every tab switch.
//! Registering them dynamically means a macro accel is live ONLY while its tab is
//! active, so it naturally wins over a global binding for the duration — and is
//! gone the moment the user switches away.
//!
//! Scope: the main window's notebook. Detached windows own a separate notebook
//! and are not covered by this pass (a follow-up can apply the same hook to a
//! detached notebook's `tab_view`).

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;

use crate::state::SharedAppState;
use crate::window::types::SharedNotebook;

/// Window action name (string parameter = macro index in the active connection).
const FIRE_MACRO_ACTION: &str = "fire-connection-macro";
/// Fully-qualified action name for accelerator registration.
const FIRE_MACRO_ACTION_FULL: &str = "win.fire-connection-macro";

/// Upper bound on macro accelerators cleared per tab switch. Clearing is by
/// detailed action name (`win.fire-connection-macro::<index>`) over a fixed
/// range, so a connection that previously had more macros than the current one
/// cannot leave a stale binding live. 64 is far above any realistic macro count.
const MAX_CLEARED_MACRO_ACCELS: usize = 64;

/// Registers the command-macro dispatcher on `window` and wires the tab-switch
/// hook that keeps the active connection's macro accelerators current.
pub(crate) fn setup_macro_dispatch(
    window: &adw::ApplicationWindow,
    notebook: &SharedNotebook,
    state: &SharedAppState,
) {
    // The action fires a macro by index into the ACTIVE connection's macro list.
    let fire_action = gio::SimpleAction::new(FIRE_MACRO_ACTION, Some(glib::VariantTy::STRING));
    {
        let notebook = notebook.clone();
        let state = state.clone();
        fire_action.connect_activate(move |_, param| {
            let Some(index_str) = param.and_then(glib::Variant::str) else {
                return;
            };
            let Ok(index) = index_str.parse::<usize>() else {
                return;
            };
            fire_macro_by_index(&notebook, &state, index);
        });
    }
    window.add_action(&fire_action);

    // Keep the active connection's macro accelerators registered as the user
    // switches tabs. Registration is on the application (accels live there), the
    // action lives on the window.
    let window_weak = window.downgrade();
    let notebook_for_switch = notebook.clone();
    let state_for_switch = state.clone();
    notebook.tab_view().connect_selected_page_notify(move |_| {
        let Some(window) = window_weak.upgrade() else {
            return;
        };
        reregister_macro_accels(&window, &notebook_for_switch, &state_for_switch);
    });

    // Register once for whatever tab is active at setup time.
    reregister_macro_accels(window, notebook, state);
}

/// Renders and sends the macro at `index` of the active connection into the
/// active session's terminal. A no-op if there is no active session, the
/// connection is gone, or the index is out of range.
fn fire_macro_by_index(notebook: &SharedNotebook, state: &SharedAppState, index: usize) {
    let Some(session_id) = notebook.get_active_session_id() else {
        return;
    };
    let Some(info) = notebook.get_session_info(session_id) else {
        return;
    };
    let connection_id = info.connection_id;

    let Ok(state_ref) = state.try_borrow() else {
        return;
    };
    let Some(connection) = state_ref.get_connection(connection_id) else {
        return;
    };
    let Some(macro_) = connection.automation.command_macros.get(index) else {
        return;
    };

    // Build the connection's variable scope the same way the launch/snippet
    // paths do (globals + synthetic host/port/username/name + local vars), then
    // render through the shared terminal-input substitution. mask_secrets=false:
    // the real value must reach the terminal.
    let global_variables = crate::state::resolve_global_variables(state_ref.settings());
    let has_password = connection.password_source != rustconn_core::models::PasswordSource::None;
    let variables = super::protocols::connection_variable_manager(
        connection,
        &global_variables,
        has_password,
        false,
    );
    let scope = rustconn_core::variables::VariableScope::Connection(connection_id);
    match macro_.render_for_terminal(&variables, scope) {
        Ok(sub) => {
            // Sent even with unresolved placeholders left verbatim — matching how
            // the rest of the terminal-input paths behave; the editor is where a
            // user fixes a bad reference.
            notebook.send_text_to_session(session_id, &sub.text);
        }
        Err(e) => {
            tracing::warn!(
                macro_name = %macro_.name,
                error = %e,
                "command macro not sent: variable substitution rejected the value"
            );
        }
    }
}

/// Clears any previously-registered macro accelerators and registers the active
/// connection's macro accels, each targeting the action with its macro index as
/// the string parameter.
fn reregister_macro_accels(
    window: &adw::ApplicationWindow,
    notebook: &SharedNotebook,
    state: &SharedAppState,
) {
    let Some(app) = window.application().and_downcast::<adw::Application>() else {
        return;
    };

    // Clearing is done by setting an empty accel list for the detailed action
    // names we might have registered last time. GTK keys accels by the full
    // detailed name (action + target), so we clear by rebuilding from scratch:
    // first drop the plain action, then add the active connection's entries.
    // `set_accels_for_action` replaces whatever was there for each detailed name.
    //
    // We cannot enumerate previously-set detailed names cheaply, so we clear a
    // bounded range of indices (0..MAX_CLEARED) to drop stale bindings from a
    // connection that had more macros than the current one.
    for i in 0..MAX_CLEARED_MACRO_ACCELS {
        let detailed = format!("{FIRE_MACRO_ACTION_FULL}::{i}");
        app.set_accels_for_action(&detailed, &[]);
    }

    let Some(session_id) = notebook.get_active_session_id() else {
        return;
    };
    let Some(info) = notebook.get_session_info(session_id) else {
        return;
    };
    let connection_id = info.connection_id;
    let Ok(state_ref) = state.try_borrow() else {
        return;
    };
    let Some(connection) = state_ref.get_connection(connection_id) else {
        return;
    };

    for (i, macro_) in connection.automation.command_macros.iter().enumerate() {
        let Some(accel) = macro_.keybind.as_deref() else {
            continue;
        };
        let accel = accel.trim();
        if accel.is_empty() {
            continue;
        }
        // Validate the accelerator before registering; a bad string would make
        // GTK warn at runtime. accelerator_parse yields None when the string does
        // not parse, and a key whose name() is None for an invalid keyval.
        let valid = gtk4::accelerator_parse(accel).is_some_and(|(key, _mods)| key.name().is_some());
        if !valid {
            tracing::warn!(
                macro_name = %macro_.name,
                accel,
                "command macro keybind is not a valid accelerator; skipping"
            );
            continue;
        }
        let detailed = format!("{FIRE_MACRO_ACTION_FULL}::{i}");
        app.set_accels_for_action(&detailed, &[accel]);
    }
}
