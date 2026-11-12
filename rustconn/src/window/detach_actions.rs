//! Detach and attach window actions.
//!
//! Registers the main-window half of the feature (`win.detach-session`,
//! `win.detach-session-to-monitor`, `win.attach-session`, `win.toggle-detach`)
//! and the session-scoped half installed on every detached window, so a
//! shortcut pressed there acts on that window's own session and never on the
//! main window's selection.
//!
//! `win.toggle-detach` is deliberately one action name registered on both
//! windows with different handlers: GTK resolves `win.*` against the focused
//! window, so a single accelerator detaches from the main window and attaches
//! from a detached one.
//!
//! Every closure holds `Weak` handles to the notebook, the registry, and the
//! main window, so nothing registered here can keep a session alive past its
//! teardown.
//!
//! The free helpers below are `pub` rather than `pub(crate)` even though this
//! module is private: `clippy::redundant_pub_crate` is part of the nursery set
//! this crate enables and it rejects `pub(crate)` on an item in a private
//! module. `MainWindow::setup_detach_actions` can be `pub(crate)` because it is
//! a method on a type that is public elsewhere.

use std::rc::Weak;

use gtk4::gdk;
use rustconn_core::DetachVerdict;

use super::*;
use crate::detached_window::{DetachedSessionWindow, DetachedWindowParams, DetachedWindowRegistry};
use crate::i18n::i18n;
use crate::toast::ToastType;

/// Variant type of `win.detach-session-to-monitor`: session id plus monitor index.
const DETACH_TO_MONITOR_TYPE: &str = "(su)";

/// Weak handles the detach and attach helpers need.
///
/// The toast overlay is held strongly: it is a plain widget wrapper that owns
/// nothing pointing back here, so it cannot form a cycle.
#[derive(Clone)]
struct DetachHandles {
    notebook: Weak<TerminalNotebook>,
    registry: Weak<DetachedWindowRegistry>,
    main_window: glib::WeakRef<adw::ApplicationWindow>,
    toasts: SharedToastOverlay,
}

/// The upgraded counterparts of [`DetachHandles`].
struct DetachTargets {
    notebook: Rc<TerminalNotebook>,
    registry: Rc<DetachedWindowRegistry>,
    main_window: adw::ApplicationWindow,
}

impl DetachHandles {
    /// Upgrades every handle, or reports `None` once any of them is gone.
    fn upgrade(&self) -> Option<DetachTargets> {
        Some(DetachTargets {
            notebook: self.notebook.upgrade()?,
            registry: self.registry.upgrade()?,
            main_window: self.main_window.upgrade()?,
        })
    }
}

/// Returns the translated explanation for a verdict that blocks a detach.
///
/// Mirrors `DetachVerdict::reason_key()` one-to-one — `detach-already-detached`,
/// `detach-external-viewer`, `detach-split-owner`, `detach-split-guest` — while
/// `detach-allowed` stays unexplained, because nothing is shown for it. Matched
/// on the enum so a future verdict cannot silently fall into a generic message.
/// Sentence case, per the project rules for explanations.
fn verdict_explanation(verdict: DetachVerdict) -> Option<String> {
    match verdict {
        DetachVerdict::Allowed => None,
        DetachVerdict::AlreadyDetached => Some(i18n("This session already has its own window.")),
        DetachVerdict::ExternalViewer => Some(i18n(
            "This session is displayed by a separate viewer, which already runs in its own window.",
        )),
        DetachVerdict::SplitOwner => Some(i18n(
            "Remove the split layout from this tab before moving the session to its own window.",
        )),
        DetachVerdict::SplitGuest => Some(i18n(
            "This session is displayed in a split layout. Return it to its own tab first.",
        )),
    }
}

/// Parses a session id from an action parameter.
fn session_id_from(text: &str) -> Option<Uuid> {
    match Uuid::parse_str(text) {
        Ok(session_id) => Some(session_id),
        Err(error) => {
            tracing::warn!(%error, text, "detach action received an invalid session id");
            None
        }
    }
}

/// Returns the monitor at `index` in the default display's monitor list.
///
/// A monitor that disappeared between building the menu and activating it
/// yields `None`, and the caller falls back to a normal window.
fn monitor_at(index: u32) -> Option<gdk::Monitor> {
    let monitor = gdk::Display::default()?
        .monitors()
        .item(index)
        .and_downcast::<gdk::Monitor>();
    if monitor.is_none() {
        tracing::debug!(index, "requested monitor is no longer connected");
    }
    monitor
}

/// Captures a monitor identity that survives display-list reordering.
fn monitor_preference(index: u32) -> crate::terminal::DetachMonitor {
    let monitor = monitor_at(index);
    crate::terminal::DetachMonitor::from_monitor(index, monitor.as_ref())
}

/// Resolves a saved monitor by stable identity, using its index only when no
/// identity was available from the display backend.
fn monitor_for_preference(preference: &crate::terminal::DetachMonitor) -> Option<gdk::Monitor> {
    if !preference.has_identity() {
        return monitor_at(preference.index);
    }

    let monitors = gdk::Display::default()?.monitors();
    for index in 0..monitors.n_items() {
        let Some(monitor) = monitors.item(index).and_downcast::<gdk::Monitor>() else {
            continue;
        };
        let connector = monitor.connector();
        let manufacturer = monitor.manufacturer();
        let model = monitor.model();
        if preference.matches_identity(
            connector.as_ref().map(|value| value.as_str()),
            manufacturer.as_ref().map(|value| value.as_str()),
            model.as_ref().map(|value| value.as_str()),
        ) {
            return Some(monitor);
        }
    }

    tracing::debug!(
        connector = ?preference.connector,
        manufacturer = ?preference.manufacturer,
        model = ?preference.model,
        "preferred monitor is no longer connected"
    );
    None
}

/// Returns the window hosting a session, when that session is detached.
///
/// Session-scoped feedback belongs on the window the user is looking at, so
/// anything addressed to a session asks here first and falls back to the main
/// window. The window is cloned out of the registry, so no
/// borrow is held while it is used.
pub fn detached_host_window(session_id: Uuid) -> Option<adw::ApplicationWindow> {
    crate::window::detached_window_registry()?
        .with_window(session_id, |window| window.window().clone())
}

/// Renames every open session of a connection, in tabs and in detached windows.
///
/// A rename used to reach only the sidebar: an open tab kept the old name in its
/// title and tooltip, and a detached window kept it in its own title bar. Called
/// from the rename dialog once the new name is stored (issue #236).
pub fn rename_open_sessions(notebook: &SharedNotebook, connection_id: Uuid, new_name: &str) {
    let renamed = notebook.rename_connection_sessions(connection_id, new_name);
    let retitled = crate::window::detached_window_registry().map_or(0, |registry| {
        registry.rename_connection(connection_id, new_name)
    });
    if !renamed.is_empty() || retitled > 0 {
        tracing::debug!(
            connection = %connection_id,
            sessions = renamed.len(),
            windows = retitled,
            "connection rename applied to open sessions"
        );
    }
}

/// Shows a high-priority error toast inside a detached window.
fn show_error_in_window(window: &DetachedSessionWindow, message: &str) {
    let toast = crate::toast::plain_toast(message);
    toast.set_priority(adw::ToastPriority::High);
    window.toast_overlay().add_toast(toast);
}

/// Moves a session out of its tab into a new detached window.
///
/// A rejected verdict is explained as a toast and changes nothing. Every other
/// failure leaves the session in its tab, because `take_session_content` rolls
/// itself back.
fn detach_session(
    handles: &DetachHandles,
    session_id: Uuid,
    presentation: crate::terminal::DetachPresentation,
) -> bool {
    let Some(targets) = handles.upgrade() else {
        return false;
    };

    let verdict = targets.notebook.detach_verdict(session_id);
    if let Some(explanation) = verdict_explanation(verdict) {
        tracing::info!(
            session = %session_id,
            reason = verdict.reason_key(),
            "detach rejected"
        );
        handles
            .toasts
            .show_toast_with_type(&explanation, ToastType::Warning);
        return false;
    }

    let Some(info) = targets.notebook.get_session_info(session_id) else {
        tracing::warn!(session = %session_id, "detach failed: no session metadata");
        handles
            .toasts
            .show_error(&i18n("Could not move the session to its own window."));
        return false;
    };
    let Some(app) = targets
        .main_window
        .application()
        .and_downcast::<adw::Application>()
    else {
        tracing::warn!(session = %session_id, "detach failed: no application on the main window");
        handles
            .toasts
            .show_error(&i18n("Could not move the session to its own window."));
        return false;
    };

    let Some(content) = targets.notebook.take_session_content(session_id) else {
        handles
            .toasts
            .show_error(&i18n("Could not move the session to its own window."));
        return false;
    };

    let params = DetachedWindowParams {
        session_id,
        connection_id: info.connection_id,
        title: &info.name,
        protocol: &info.protocol.to_uppercase(),
    };
    let detached = DetachedSessionWindow::new(&app, &params, &content);
    install_detached_window_actions(&detached, handles);
    wire_detached_window_callbacks(&detached, handles);

    if presentation.fullscreen {
        if let Some(preference) = presentation.monitor {
            if let Some(monitor) = monitor_for_preference(&preference) {
                detached.present_fullscreen_on(&monitor, preference);
            } else {
                // Keep the identity so a later reconnect can target the same
                // display if it returns; do not silently use a reordered index.
                detached.present_fullscreen_preferred(preference);
            }
        } else {
            detached.present_fullscreen();
        }
    } else {
        detached.present();
    }
    targets.registry.insert(detached);

    targets.notebook.is_detached(session_id) && targets.registry.contains(session_id)
}

/// Moves a detached session back into a tab of the main window.
///
/// On failure the session keeps running in its detached window and the error is
/// reported there.
fn attach_session(handles: &DetachHandles, session_id: Uuid) {
    let Some(targets) = handles.upgrade() else {
        return;
    };
    if !targets.notebook.is_detached(session_id) {
        tracing::debug!(session = %session_id, "attach ignored: session is not detached");
        return;
    }

    if !targets.notebook.attach_session(session_id) {
        let message = i18n("Could not move the session back to the main window.");
        let shown = targets
            .registry
            .with_window(session_id, |window| {
                show_error_in_window(window, &message);
            })
            .is_some();
        if !shown {
            handles.toasts.show_error(&message);
        }
        return;
    }

    if let Some(window) = targets.registry.take(session_id) {
        // Marked first, so the close handler treats this as a move rather than
        // as the user ending the session.
        window.begin_attach();
        window.close();
    }
    targets.main_window.present();
}

/// Ends a detached session and closes its window.
///
/// `close_session` runs the standard tab-close teardown, so the window must not
/// run it a second time: it is marked as "closing without teardown" first.
fn close_detached_session(handles: &DetachHandles, session_id: Uuid) {
    let Some(targets) = handles.upgrade() else {
        return;
    };
    let window = targets.registry.take(session_id);
    targets.notebook.close_session(session_id);
    if let Some(window) = window {
        window.begin_attach();
        window.close();
    }
}

/// Drops a session's detached window on idle, without a second teardown.
///
/// Deferred to idle so a window is never closed from inside its own close
/// handler, and idempotent: whichever path reaches the registry first takes the
/// entry, the others find nothing. `begin_attach` marks the close as a move so
/// the window's close handler does not run `close_session` again.
fn close_detached_window_later(registry: &Weak<DetachedWindowRegistry>, session_id: Uuid) {
    // Most sessions that end live in a tab; scheduling an idle for each of them
    // would be work with nothing to do. Asking first also keeps the log quiet.
    if !registry
        .upgrade()
        .is_some_and(|registry| registry.contains(session_id))
    {
        return;
    }
    let registry = registry.clone();
    glib::idle_add_local_once(move || {
        if let Some(registry) = registry.upgrade()
            && let Some(window) = registry.take(session_id)
        {
            window.begin_attach();
            window.close();
            tracing::debug!(session = %session_id, "detached window dropped after session end");
        }
    });
}

/// Wires the attach button and the close handler of a fresh detached window.
fn wire_detached_window_callbacks(detached: &DetachedSessionWindow, handles: &DetachHandles) {
    let handles_attach = handles.clone();
    detached.set_on_attach(move |session_id| {
        attach_session(&handles_attach, session_id);
    });

    // The user closing the window ends the session, exactly as closing its tab
    // would. The registry entry goes away on idle, so it is not removed from
    // inside the window's own close handler.
    let notebook = handles.notebook.clone();
    let registry = handles.registry.clone();
    detached.set_on_close(move |session_id| {
        if let Some(notebook) = notebook.upgrade() {
            notebook.close_session(session_id);
        }
        close_detached_window_later(&registry, session_id);
    });
}

/// Installs the session-scoped actions of a detached window.
///
/// Every handler targets `detached.session_id()`, so a shortcut pressed here
/// never reaches the main window's selected tab.
fn install_detached_window_actions(detached: &DetachedSessionWindow, handles: &DetachHandles) {
    let window = detached.window().clone();
    let session_id = detached.session_id();

    // Same accelerator as detach in the main window, opposite direction.
    let toggle_action = gio::SimpleAction::new("toggle-detach", None);
    let handles_toggle = handles.clone();
    toggle_action.connect_activate(move |_, _| {
        attach_session(&handles_toggle, session_id);
    });
    window.add_action(&toggle_action);

    // Scoped to this window's session: a parameter naming another session is
    // ignored, so a stray target cannot act on a different window.
    let attach_action = gio::SimpleAction::new("attach-session", Some(glib::VariantTy::STRING));
    let handles_attach = handles.clone();
    attach_action.connect_activate(move |_, param| {
        if let Some(requested) = param
            .and_then(glib::Variant::get::<String>)
            .as_deref()
            .and_then(session_id_from)
            && requested != session_id
        {
            tracing::debug!(
                session = %session_id,
                requested = %requested,
                "detached window ignored an attach request for another session"
            );
            return;
        }
        attach_session(&handles_attach, session_id);
    });
    window.add_action(&attach_action);

    let copy_action = gio::SimpleAction::new("copy", None);
    let notebook_copy = handles.notebook.clone();
    copy_action.connect_activate(move |_, _| {
        // Embedded viewers keep their own copy handling on the widget, which
        // travelled into this window with the session.
        if let Some(notebook) = notebook_copy.upgrade()
            && let Some(terminal) = notebook.get_terminal(session_id)
            && let Some(text) = terminal.text_selected(vte4::Format::Text)
        {
            terminal.display().clipboard().set_text(&text);
        }
    });
    window.add_action(&copy_action);

    let paste_action = gio::SimpleAction::new("paste", None);
    let notebook_paste = handles.notebook.clone();
    paste_action.connect_activate(move |_, _| {
        if let Some(notebook) = notebook_paste.upgrade()
            && let Some(terminal) = notebook.get_terminal(session_id)
        {
            crate::terminal::safe_paste::paste_into_terminal(&terminal);
        }
    });
    window.add_action(&paste_action);

    let search_action = gio::SimpleAction::new("terminal-search", None);
    let notebook_search = handles.notebook.clone();
    let window_search = window.downgrade();
    search_action.connect_activate(move |_, _| {
        if let Some(notebook) = notebook_search.upgrade()
            && let Some(parent) = window_search.upgrade()
            && let Some(terminal) = notebook.get_terminal(session_id)
        {
            let dialog =
                crate::dialogs::TerminalSearchDialog::new(Some(&parent.clone().upcast()), terminal);
            dialog.show();
        }
    });
    window.add_action(&search_action);

    // Ctrl+W in a detached window ends only this session — or, while a dialog
    // is open in the window, closes just the dialog.
    let close_action = gio::SimpleAction::new("close-tab", None);
    let handles_close = handles.clone();
    let window_close = window.downgrade();
    close_action.connect_activate(move |_, _| {
        if let Some(window) = window_close.upgrade()
            && super::terminal_actions::close_visible_dialog(&window)
        {
            return;
        }
        close_detached_session(&handles_close, session_id);
    });
    window.add_action(&close_action);

    // Stateful, like the main window's, so the checkmark semantics stay
    // identical if the action is ever surfaced in a menu. The state follows the
    // window's `fullscreened` property rather than the action, mirroring
    // `window/navigation_actions.rs`: F11 runs this action, but the window
    // manager or `present_fullscreen_on` can also change full screen without
    // it, and the checkmark would otherwise go stale (#388).
    let fullscreen_action = gio::SimpleAction::new_stateful(
        "toggle-fullscreen",
        None,
        &window.is_fullscreen().to_variant(),
    );
    let window_fullscreen = window.downgrade();
    let presentation = detached.presentation_handle();
    fullscreen_action.connect_activate(move |_, _| {
        if let Some(window) = window_fullscreen.upgrade() {
            let is_fullscreen = window.is_fullscreen();
            if is_fullscreen {
                window.unfullscreen();
            } else {
                window.fullscreen();
            }
            *presentation.borrow_mut() = crate::terminal::DetachPresentation {
                fullscreen: !is_fullscreen,
                monitor: None,
            };
        }
    });
    {
        let action = fullscreen_action.clone();
        window.connect_fullscreened_notify(move |win| {
            action.set_state(&win.is_fullscreen().to_variant());
        });
    }
    window.add_action(&fullscreen_action);

    // Passthrough is application-wide (it drops accelerators from the whole
    // application), so this forwards to the main window's stateful action
    // instead of keeping a second copy of the state and the indicator.
    let passthrough_action = gio::SimpleAction::new("toggle-passthrough", None);
    let main_window = handles.main_window.clone();
    passthrough_action.connect_activate(move |_, _| {
        if let Some(main_window) = main_window.upgrade() {
            let _ = WidgetExt::activate_action(&main_window, "win.toggle-passthrough", None);
        }
    });
    window.add_action(&passthrough_action);

    // Ctrl+, and the menu's Settings… open Settings, which are application-wide
    // and whose dialog lives on the main window: raise the main window (it may
    // be behind this one, or hidden in the tray) and run its action there.
    let settings_action = gio::SimpleAction::new("settings", None);
    let main_window = handles.main_window.clone();
    settings_action.connect_activate(move |_, _| {
        if let Some(main_window) = main_window.upgrade() {
            main_window.present();
            let _ = WidgetExt::activate_action(&main_window, "win.settings", None);
        }
    });
    window.add_action(&settings_action);

    follow_passthrough_for_primary_menu(detached, handles);
}

/// Keeps the detached window's F10 binding in step with keyboard passthrough.
///
/// F10 comes from the menu button's GTK-internal `primary` flag, not from the
/// accelerator table, so `set_passthrough` cannot drop it. The main window
/// clears its own button's flag in the `win.toggle-passthrough` handler; this
/// window follows that action's state the same way, so F10 reaches the session
/// (Midnight Commander, for one) while passthrough is on. The handler is
/// removed when the window is destroyed.
fn follow_passthrough_for_primary_menu(detached: &DetachedSessionWindow, handles: &DetachHandles) {
    let Some(action) = handles
        .main_window
        .upgrade()
        .and_then(|main_window| main_window.lookup_action("toggle-passthrough"))
        .and_downcast::<gio::SimpleAction>()
    else {
        tracing::warn!(
            session = %detached.session_id(),
            "detached window: no passthrough action to follow, F10 stays bound"
        );
        return;
    };
    let passthrough_on =
        |action: &gio::SimpleAction| action.state().and_then(|v| v.get::<bool>()) == Some(true);

    let button = detached.menu_button();
    button.set_primary(!passthrough_on(&action));
    let button_weak = button.downgrade();
    let state_watch = action.connect_state_notify(move |action| {
        if let Some(button) = button_weak.upgrade() {
            button.set_primary(!passthrough_on(action));
        }
    });

    let action_weak = action.downgrade();
    let state_watch = std::cell::RefCell::new(Some(state_watch));
    detached.window().connect_destroy(move |_| {
        if let Some(action) = action_weak.upgrade()
            && let Some(id) = state_watch.take()
        {
            action.disconnect(id);
        }
    });
}

impl MainWindow {
    /// Registers the main-window detach and attach actions and the notebook hooks.
    pub(crate) fn setup_detach_actions(&self, window: &adw::ApplicationWindow) {
        let handles = DetachHandles {
            notebook: Rc::downgrade(&self.terminal_notebook),
            registry: Rc::downgrade(&self.detached_windows),
            main_window: window.downgrade(),
            toasts: Rc::clone(&self.toast_overlay),
        };

        let detach_action = gio::SimpleAction::new("detach-session", Some(glib::VariantTy::STRING));
        let handles_detach = handles.clone();
        detach_action.connect_activate(move |_, param| {
            if let Some(session_id) = param
                .and_then(glib::Variant::get::<String>)
                .as_deref()
                .and_then(session_id_from)
            {
                let _ = detach_session(
                    &handles_detach,
                    session_id,
                    crate::terminal::DetachPresentation::default(),
                );
            }
        });
        window.add_action(&detach_action);

        match glib::VariantTy::new(DETACH_TO_MONITOR_TYPE) {
            Ok(variant_type) => {
                let monitor_action =
                    gio::SimpleAction::new("detach-session-to-monitor", Some(variant_type));
                let handles_monitor = handles.clone();
                monitor_action.connect_activate(move |_, param| {
                    if let Some((session, monitor)) =
                        param.and_then(glib::Variant::get::<(String, u32)>)
                        && let Some(session_id) = session_id_from(&session)
                    {
                        let _ = detach_session(
                            &handles_monitor,
                            session_id,
                            crate::terminal::DetachPresentation {
                                fullscreen: true,
                                monitor: Some(monitor_preference(monitor)),
                            },
                        );
                    }
                });
                window.add_action(&monitor_action);
            }
            Err(error) => {
                tracing::error!(%error, "could not register win.detach-session-to-monitor");
            }
        }

        let attach_action = gio::SimpleAction::new("attach-session", Some(glib::VariantTy::STRING));
        let handles_attach = handles.clone();
        attach_action.connect_activate(move |_, param| {
            if let Some(session_id) = param
                .and_then(glib::Variant::get::<String>)
                .as_deref()
                .and_then(session_id_from)
            {
                attach_session(&handles_attach, session_id);
            }
        });
        window.add_action(&attach_action);

        // Same accelerator as attach in a detached window; GTK routes it to
        // whichever window has focus.
        let toggle_action = gio::SimpleAction::new("toggle-detach", None);
        let handles_toggle = handles.clone();
        toggle_action.connect_activate(move |_, _| {
            let Some(notebook) = handles_toggle.notebook.upgrade() else {
                return;
            };
            let Some(session_id) = notebook.get_active_session_id() else {
                tracing::debug!("toggle-detach ignored: no active session");
                return;
            };
            let _ = detach_session(
                &handles_toggle,
                session_id,
                crate::terminal::DetachPresentation::default(),
            );
        });
        window.add_action(&toggle_action);

        // Focusing a detached session presents its window instead of selecting a
        // tab, so sidebar activation, the session manager, and workspace restore
        // need no changes of their own.
        let registry_focus = handles.registry.clone();
        self.terminal_notebook
            .set_on_focus_detached(move |session_id| {
                if let Some(registry) = registry_focus.upgrade()
                    && !registry.present(session_id)
                {
                    tracing::warn!(
                        session = %session_id,
                        "detached session has no window to present"
                    );
                }
            });

        // A detached session can also end without the window being involved: a
        // remote disconnect, its child exiting, or a terminate from the session
        // manager all tear it down inside the notebook. Closing the window from
        // here is what keeps no empty detached window behind.
        let registry_ended = handles.registry.clone();
        self.terminal_notebook
            .set_on_session_ended(move |session_id| {
                close_detached_window_later(&registry_ended, session_id);
            });

        let handles_request = handles;
        self.terminal_notebook
            .set_on_detach_request(move |session_id, presentation| {
                detach_session(&handles_request, session_id, presentation)
            });
    }
}

#[cfg(test)]
mod tests {
    use rustconn_core::DetachVerdict;

    use super::{DETACH_TO_MONITOR_TYPE, session_id_from, verdict_explanation};

    #[test]
    fn every_blocking_verdict_has_an_explanation() {
        for verdict in [
            DetachVerdict::AlreadyDetached,
            DetachVerdict::ExternalViewer,
            DetachVerdict::SplitOwner,
            DetachVerdict::SplitGuest,
        ] {
            let explanation = verdict_explanation(verdict)
                .unwrap_or_else(|| panic!("{} needs an explanation", verdict.reason_key()));
            assert_ne!(explanation, "");
        }
    }

    #[test]
    fn an_allowed_verdict_explains_nothing() {
        assert!(verdict_explanation(DetachVerdict::Allowed).is_none());
    }

    #[test]
    fn session_ids_round_trip_and_garbage_is_rejected() {
        let id = uuid::Uuid::new_v4();
        assert_eq!(session_id_from(&id.to_string()), Some(id));
        assert!(session_id_from("not-a-uuid").is_none());
        assert!(session_id_from("").is_none());
    }

    #[test]
    fn monitor_action_type_is_a_valid_variant_type() {
        assert!(gtk4::glib::VariantTy::new(DETACH_TO_MONITOR_TYPE).is_ok());
    }
}
