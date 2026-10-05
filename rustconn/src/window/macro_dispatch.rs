//! Per-connection command-macro keybind dispatcher.
//!
//! Command macros (see [`rustconn_core::models::CommandMacro`]) belong to a
//! single connection and may carry an optional GTK accelerator. A macro accel is
//! registered only while a terminal session's VTE has keyboard focus, and only
//! for the macros of *that* session's connection — in a split tab, the focused
//! pane, resolved through the tab's split bridge exactly as the clipboard and
//! snippet actions do. It is cleared when focus leaves the terminal, while
//! keyboard passthrough is on, and for sessions without a terminal (RDP, VNC,
//! SPICE, web), where it could only swallow keys.
//!
//! Registration is one parameterized window action, `win.fire-connection-macro`,
//! whose string target is the macro's **keybind**, not its position. At fire
//! time the dispatcher looks the keybind up again in the focused session's
//! connection, so a registration that is momentarily stale (a macro edited,
//! deleted or reordered, focus moved) can at worst do nothing — it can never
//! fire a different macro. Registrations are refreshed through the internal
//! `win.refresh-connection-macros` action, coalesced to one idle callback, on
//! terminal focus changes, tab switches, passthrough toggles and every sidebar
//! reload (which follows each connection save, import and sync).
//!
//! Macro accels are application accelerators like the configured shortcuts; a
//! macro keybind that collides with one of those is refused by
//! [`crate::command_macros`] rather than relying on GTK's resolution order.
//!
//! Scope: the main window's notebook. Detached windows own no
//! `win.fire-connection-macro` action, so a macro accel does nothing there.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use rustconn_core::Variable;
use rustconn_core::models::CommandMacro;
use rustconn_core::variables::{VariableManager, VariableScope};
use uuid::Uuid;

use crate::i18n::{i18n, i18n_f};
use crate::state::SharedAppState;
use crate::terminal::TerminalNotebook;
use crate::toast::ToastType;
use crate::window::types::{SessionSplitBridges, SharedNotebook};

/// Window action name (string parameter = the macro's keybind).
const FIRE_MACRO_ACTION: &str = "fire-connection-macro";
/// Fully-qualified fire action name, for accelerator registration.
const FIRE_MACRO_ACTION_FULL: &str = "win.fire-connection-macro";
/// Window action that re-registers the macro accels on the next idle.
const REFRESH_ACTION: &str = "refresh-connection-macros";
/// Fully-qualified refresh action name, for [`request_refresh`].
const REFRESH_ACTION_FULL: &str = "win.refresh-connection-macros";

/// Everything the dispatcher's handlers need. Widgets and the notebook are held
/// weakly: the actions holding this live on the window itself.
struct Dispatcher {
    window: glib::WeakRef<adw::ApplicationWindow>,
    notebook: Weak<TerminalNotebook>,
    bridges: SessionSplitBridges,
    state: SharedAppState,
    /// Detailed action names registered by the last refresh, so the next one
    /// clears exactly those — however many there were.
    registered: RefCell<Vec<String>>,
    /// Set while a refresh is queued, so a burst of focus events costs one.
    refresh_pending: Cell<bool>,
}

/// Registers the command-macro dispatcher on `window`.
pub(crate) fn setup_macro_dispatch(
    window: &adw::ApplicationWindow,
    notebook: &SharedNotebook,
    bridges: &SessionSplitBridges,
    state: &SharedAppState,
) {
    let dispatcher = Rc::new(Dispatcher {
        window: window.downgrade(),
        notebook: Rc::downgrade(notebook),
        bridges: bridges.clone(),
        state: state.clone(),
        registered: RefCell::new(Vec::new()),
        refresh_pending: Cell::new(false),
    });

    let fire_action = gio::SimpleAction::new(FIRE_MACRO_ACTION, Some(glib::VariantTy::STRING));
    {
        let dispatcher = Rc::clone(&dispatcher);
        fire_action.connect_activate(move |_, param| {
            if let Some(accel) = param.and_then(glib::Variant::str) {
                dispatcher.fire(accel);
            }
        });
    }
    window.add_action(&fire_action);

    let refresh_action = gio::SimpleAction::new(REFRESH_ACTION, None);
    refresh_action.connect_activate(move |_, _| {
        Rc::clone(&dispatcher).schedule_refresh();
    });
    window.add_action(&refresh_action);

    // Only the window is captured: the handler lives on the notebook's own
    // tab view, so capturing the notebook would be a reference cycle.
    let window_weak = window.downgrade();
    notebook.tab_view().connect_selected_page_notify(move |_| {
        if let Some(window) = window_weak.upgrade() {
            request_refresh(&window);
        }
    });

    request_refresh(window);
}

/// Asks the dispatcher of the window `widget` belongs to to re-register the
/// macro accelerators on the next idle. A no-op outside the main window.
pub(crate) fn request_refresh(widget: &impl IsA<gtk4::Widget>) {
    if let Err(e) = widget.activate_action(REFRESH_ACTION_FULL, None) {
        tracing::debug!(error = %e, "macro accel refresh not available here");
    }
}

impl Dispatcher {
    /// Queues one refresh for the next main-loop idle, so it runs after every
    /// handler of the current event (pane click, focus change) has updated the
    /// state it reads.
    fn schedule_refresh(self: Rc<Self>) {
        if self.refresh_pending.replace(true) {
            return;
        }
        glib::idle_add_local_once(move || {
            self.refresh_pending.set(false);
            self.refresh();
        });
    }

    /// Clears the previous registrations and registers the focused terminal
    /// session's macro accels, if there is one and passthrough is off.
    fn refresh(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let Some(app) = window.application().and_downcast::<adw::Application>() else {
            return;
        };

        for detailed in self.registered.take() {
            app.set_accels_for_action(&detailed, &[]);
        }

        if passthrough_active(&window) {
            return;
        }
        let Some(notebook) = self.notebook.upgrade() else {
            return;
        };
        let Some(session_id) = focused_terminal_session(&window, &notebook, &self.bridges) else {
            return;
        };
        let Some(info) = notebook.get_session_info(session_id) else {
            return;
        };
        let accels = {
            let Ok(state_ref) = self.state.try_borrow() else {
                return;
            };
            let Some(connection) = state_ref.get_connection(info.connection_id) else {
                return;
            };
            if !crate::command_macros::protocol_takes_macros(
                connection.protocol_config.protocol_type(),
            ) {
                return;
            }
            crate::command_macros::registrable_keybinds(
                &connection.automation.command_macros,
                &state_ref.settings().keybindings,
            )
        };

        let mut registered = Vec::with_capacity(accels.len());
        for accel in accels {
            let detailed =
                gio::Action::print_detailed_name(FIRE_MACRO_ACTION_FULL, Some(&accel.to_variant()));
            app.set_accels_for_action(&detailed, &[accel.as_str()]);
            registered.push(detailed.to_string());
        }
        *self.registered.borrow_mut() = registered;
    }

    /// Fires the macro bound to `accel` in the focused terminal session's
    /// connection. Looks the keybind up afresh and re-validates the macro, so a
    /// stale registration sends nothing rather than the wrong command.
    fn fire(&self, accel: &str) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let Some(notebook) = self.notebook.upgrade() else {
            return;
        };
        if passthrough_active(&window) {
            return;
        }
        let Some(session_id) = focused_terminal_session(&window, &notebook, &self.bridges) else {
            return;
        };
        let Some(info) = notebook.get_session_info(session_id) else {
            return;
        };

        let (macro_, connection, global_variables, problem) = {
            let Ok(state_ref) = self.state.try_borrow() else {
                return;
            };
            let Some(connection) = state_ref.get_connection(info.connection_id) else {
                return;
            };
            if !crate::command_macros::protocol_takes_macros(
                connection.protocol_config.protocol_type(),
            ) {
                return;
            }
            let connection_macros = &connection.automation.command_macros;
            let Some(index) = connection_macros.iter().position(|m| {
                crate::command_macros::bound_keybind(m).is_some_and(|k| {
                    rustconn_core::config::keybindings::accelerators_equivalent(k, accel)
                })
            }) else {
                // Registered for a macro that has since been edited away.
                request_refresh(&window);
                return;
            };
            let problem = crate::command_macros::macro_problem(
                connection_macros,
                index,
                &state_ref.settings().keybindings,
            );
            (
                connection_macros[index].clone(),
                connection.clone(),
                crate::state::resolve_global_variables(state_ref.settings()),
                problem,
            )
        };

        if let Some(problem) = problem {
            tracing::warn!(
                macro_name = %macro_.name,
                reason = %problem,
                "command macro not sent: it fails validation"
            );
            show_not_sent(&window, &macro_, &problem);
            return;
        }

        let mut variables = VariableManager::new();
        for var in super::protocols::automation_variables_base(&connection, &global_variables) {
            variables.set_global(var);
        }
        // Same rule as an Expect response: the credential is copied out of the
        // cache only when the command asks for it, into a secret variable that
        // scrubs itself on drop. Never the child-process password token.
        if references(&macro_.command, "password")
            && let Some(password) =
                super::protocols::cached_connection_password(&self.state, connection.id)
        {
            use secrecy::ExposeSecret;
            variables.set_global(Variable::new_secret("password", password.expose_secret()));
        }

        let asks = match variables.collect_ask_requests(&macro_.command, VariableScope::Global) {
            Ok(asks) => asks,
            Err(e) => {
                tracing::warn!(macro_name = %macro_.name, error = %e, "command macro not sent");
                show_not_sent(
                    &window,
                    &macro_,
                    &i18n("The command has an invalid ${…} reference"),
                );
                return;
            }
        };
        if asks.is_empty() {
            send_rendered(&window, &notebook, session_id, &macro_, &variables);
            return;
        }

        // Ask first, then send to the session that was focused when the key was
        // pressed. Everything the callback holds is weak or owned by it.
        let pending = RefCell::new(Some(variables));
        let window_weak = window.downgrade();
        let notebook_weak = Rc::downgrade(&notebook);
        let macro_for_answer = macro_.clone();
        crate::dialogs::show_macro_ask_dialog(&window, &macro_.name, &asks, move |answers| {
            let Some(answers) = answers else {
                return;
            };
            let (Some(window), Some(notebook), Some(mut variables)) = (
                window_weak.upgrade(),
                notebook_weak.upgrade(),
                pending.borrow_mut().take(),
            ) else {
                return;
            };
            for answer in answers {
                variables.set_global(answer);
            }
            send_rendered(
                &window,
                &notebook,
                session_id,
                &macro_for_answer,
                &variables,
            );
        });
    }
}

/// Whether `win.toggle-passthrough` is currently on.
fn passthrough_active(window: &adw::ApplicationWindow) -> bool {
    window
        .lookup_action("toggle-passthrough")
        .and_then(|a| a.state())
        .and_then(|v| v.get::<bool>())
        .unwrap_or(false)
}

/// The session whose terminal has keyboard focus in the active tab: the split
/// bridge's focused pane when the tab is split, the tab's own session
/// otherwise — and only if that session has a VTE and the VTE (or a child of
/// it) is the window's focus widget.
fn focused_terminal_session(
    window: &adw::ApplicationWindow,
    notebook: &TerminalNotebook,
    bridges: &SessionSplitBridges,
) -> Option<Uuid> {
    let host = notebook.get_active_session_id()?;
    // A failed borrow means a split is being rebuilt right now; the focus check
    // below keeps the fallback to the host from typing into the wrong pane.
    let focused_pane = bridges
        .try_borrow()
        .ok()
        .and_then(|b| b.get(&host).and_then(|bridge| bridge.get_focused_session()));
    let session = focused_pane.unwrap_or(host);
    let terminal = notebook.get_terminal(session)?;
    let focus = gtk4::prelude::GtkWindowExt::focus(window)?;
    (focus == *terminal.upcast_ref::<gtk4::Widget>() || focus.is_ancestor(&terminal))
        .then_some(session)
}

/// Whether `command` contains a `${name}` reference.
fn references(command: &str, name: &str) -> bool {
    VariableManager::parse_references(command).is_ok_and(|refs| refs.iter().any(|r| r == name))
}

/// Renders `macro_` against `variables` and types it into `session_id`, or
/// explains in a toast why it was not sent. Unresolved references are never
/// typed literally.
fn send_rendered(
    window: &adw::ApplicationWindow,
    notebook: &TerminalNotebook,
    session_id: Uuid,
    macro_: &CommandMacro,
    variables: &VariableManager,
) {
    match macro_.render_for_terminal(variables, VariableScope::Global) {
        Ok(sub) if sub.unresolved.is_empty() => {
            notebook.send_text_to_session(session_id, &sub.text);
        }
        Ok(sub) => {
            // Names only, never values.
            tracing::warn!(
                macro_name = %macro_.name,
                unresolved = %sub.unresolved.join(", "),
                "command macro not sent: undefined variables"
            );
            let reason = if sub.unresolved.iter().any(|n| n == "password") {
                i18n(
                    "No password is available for this connection yet. Connect with a saved or entered password first.",
                )
            } else {
                let names = sub
                    .unresolved
                    .iter()
                    .map(|n| format!("${{{n}}}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                i18n_f("Undefined variables: {}", &[&names])
            };
            show_not_sent(window, macro_, &reason);
        }
        Err(e) => {
            tracing::warn!(
                macro_name = %macro_.name,
                error = %e,
                "command macro not sent: variable substitution rejected the value"
            );
            show_not_sent(
                window,
                macro_,
                &i18n("A variable value contains characters that cannot be typed into a terminal"),
            );
        }
    }
}

/// Shows the "macro not sent" warning toast with `reason`.
fn show_not_sent(window: &adw::ApplicationWindow, macro_: &CommandMacro, reason: &str) {
    let name = if macro_.name.trim().is_empty() {
        i18n("Command macro")
    } else {
        macro_.name.clone()
    };
    crate::toast::show_toast_on_window(
        window,
        &i18n_f("“{}” was not sent. {}", &[&name, reason]),
        ToastType::Warning,
    );
}

#[cfg(test)]
mod tests {
    use super::references;

    #[test]
    fn macro_password_reference_is_detected_by_name() {
        assert!(references("sudo -S <<< ${password}", "password"));
        assert!(!references("echo ${password_hint}", "password"));
        assert!(!references("echo $password", "password"));
    }
}
