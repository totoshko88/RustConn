//! Quick terminal theme picker.
//!
//! `win.pick-terminal-theme` (command palette) targets the window's current
//! session — the active tab's focused pane in the main window, the selected tab
//! in a group window, the one session of a detached window.
//! `win.pick-terminal-theme-for` (tab menu) targets the named tab, as
//! `record-script-for` does, and outside the main window only a tab that window
//! hosts. Both open one list; choosing a row repaints the terminal at once with
//! the same colour path Settings uses (`TerminalNotebook::set_session_theme`).
//!
//! shortcut: the choice is per live session and is not saved, because the
//! per-connection store holds fg/bg/cursor hex values, not a theme name. It is
//! lost on reconnect, and Settings > Terminal > Save repaints over it. Upgrade
//! by saving a named theme on the connection.

use super::*;
use crate::i18n::i18n;
use rustconn_core::terminal_themes::TerminalTheme;
use std::collections::HashMap;
use std::rc::Weak;

/// What the picker reads, held weakly so a window's actions never keep the
/// notebook, the split layouts or the state alive.
#[derive(Clone)]
pub(crate) struct PickerSource {
    pub(crate) notebook: Weak<TerminalNotebook>,
    pub(crate) bridges: Weak<RefCell<HashMap<Uuid, Rc<SplitViewBridge>>>>,
    pub(crate) state: Weak<RefCell<crate::state::AppState>>,
}

/// The names the picker offers, in the order Settings offers them.
fn theme_choices() -> Vec<String> {
    TerminalTheme::theme_names()
}

/// The pane to repaint: the split's focused pane when it has one, else the tab's own session.
fn pane_or_owner(focused_pane: Option<Uuid>, owner: Uuid) -> Uuid {
    focused_pane.unwrap_or(owner)
}

/// The theme a session shows: the one the picker painted on it, else the
/// configured one. `None` when a connection colour override is layered over
/// the configured theme, because then no named theme is what the user sees.
fn current_theme(picked: Option<String>, configured: &str, has_override: bool) -> Option<String> {
    picked.or_else(|| (!has_override).then(|| configured.to_string()))
}

/// Registers the two picker actions of the main window.
pub(crate) fn setup_theme_actions(
    window: &adw::ApplicationWindow,
    notebook: &SharedNotebook,
    bridges: &SessionSplitBridges,
    state: &SharedAppState,
) {
    let source = PickerSource {
        notebook: Rc::downgrade(notebook),
        bridges: Rc::downgrade(bridges),
        state: Rc::downgrade(state),
    };
    let nb = Rc::downgrade(notebook);
    install_picker_actions(
        window,
        &source,
        move || nb.upgrade()?.get_active_session_id(),
        |_| true,
    );
}

/// Registers the two picker actions on `window`.
///
/// `active` names the session `win.pick-terminal-theme` acts on; `hosts` says
/// whether `win.pick-terminal-theme-for` may act on the session it names, so a
/// group or detached window only ever repaints its own sessions.
pub(crate) fn install_picker_actions(
    window: &adw::ApplicationWindow,
    source: &PickerSource,
    active: impl Fn() -> Option<Uuid> + 'static,
    hosts: impl Fn(Uuid) -> bool + 'static,
) {
    let action = gio::SimpleAction::new("pick-terminal-theme", None);
    let win = window.downgrade();
    let src = source.clone();
    action.connect_activate(move |_, _| {
        if let (Some(win), Some(owner)) = (win.upgrade(), active()) {
            present_picker(&win, &src, owner);
        }
    });
    window.add_action(&action);

    let for_tab = gio::SimpleAction::new("pick-terminal-theme-for", Some(glib::VariantTy::STRING));
    let win = window.downgrade();
    let src = source.clone();
    for_tab.connect_activate(move |_, target| {
        let Some(owner) = target
            .and_then(glib::Variant::get::<String>)
            .and_then(|id| Uuid::parse_str(&id).ok())
        else {
            return;
        };
        if !hosts(owner) {
            tracing::debug!(session = %owner, "theme picker ignored a session this window does not host");
            return;
        }
        if let Some(win) = win.upgrade() {
            present_picker(&win, &src, owner);
        }
    });
    window.add_action(&for_tab);
}

/// The session in the focused pane of `owner`'s split, if `owner` hosts one.
///
/// `None` also while a layout is being rebuilt (its map is borrowed); callers
/// fall back to the owner.
pub(crate) fn focused_pane(
    bridges: &Weak<RefCell<HashMap<Uuid, Rc<SplitViewBridge>>>>,
    owner: Uuid,
) -> Option<Uuid> {
    let bridges = bridges.upgrade()?;
    let map = bridges.try_borrow().ok()?;
    map.get(&owner)
        .and_then(|bridge| bridge.get_focused_session())
}

/// The theme `session` shows now, for the check mark.
fn session_current_theme(
    source: &PickerSource,
    notebook: &TerminalNotebook,
    session: Uuid,
) -> Option<String> {
    let picked = notebook.session_theme(session);
    if picked.is_some() {
        return picked;
    }
    let state = source.state.upgrade()?;
    let state = state.try_borrow().ok()?;
    let has_override = notebook
        .get_session_info(session)
        .and_then(|info| state.get_connection(info.connection_id))
        .is_some_and(|connection| connection.theme_override.is_some());
    current_theme(None, &state.settings().terminal.color_theme, has_override)
}

fn present_picker(window: &adw::ApplicationWindow, source: &PickerSource, owner: Uuid) {
    let Some(notebook) = source.notebook.upgrade() else {
        return;
    };
    let session = pane_or_owner(focused_pane(&source.bridges, owner), owner);
    if notebook.get_terminal(session).is_none() {
        crate::toast::show_toast_on_window(
            window,
            &i18n("Select a terminal first"),
            crate::toast::ToastType::Warning,
        );
        return;
    }
    let current = session_current_theme(source, &notebook, session);

    let names = theme_choices();
    // Settings' labels: the follow-system entry translated, palette names as is.
    let labels = crate::dialogs::settings::theme_labels(&names);
    let list = gtk4::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk4::SelectionMode::None);
    let mut marks = Vec::with_capacity(names.len());
    for (name, label) in names.iter().zip(&labels) {
        let row = adw::ActionRow::new();
        row.set_title(&glib::markup_escape_text(label));
        row.set_activatable(true);
        let mark = gtk4::Image::from_icon_name("object-select-symbolic");
        mark.update_property(&[gtk4::accessible::Property::Label(&i18n("Current theme"))]);
        mark.set_visible(current.as_deref() == Some(name.as_str()));
        row.add_suffix(&mark);
        list.append(&row);
        marks.push(mark);
    }
    let notebook = Rc::downgrade(&notebook);
    list.connect_row_activated(move |_, row| {
        let Some(index) = usize::try_from(row.index()).ok() else {
            return;
        };
        if let (Some(name), Some(notebook)) = (names.get(index), notebook.upgrade()) {
            notebook.set_session_theme(session, name);
            for (i, mark) in marks.iter().enumerate() {
                mark.set_visible(i == index);
            }
        }
    });
    let scrolled = gtk4::ScrolledWindow::builder()
        .child(&list)
        .min_content_height(320)
        .propagate_natural_height(true)
        .build();

    let dialog = adw::AlertDialog::builder()
        .heading(i18n("Terminal Theme"))
        .extra_child(&scrolled)
        .close_response("close")
        .build();
    dialog.add_response("close", &i18n("Close"));
    dialog.present(Some(window));
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustconn_core::terminal_themes::FOLLOW_SYSTEM_THEME;

    #[test]
    fn choices_start_with_follow_system_and_all_resolve() {
        let names = theme_choices();
        assert_eq!(names.first().map(String::as_str), Some(FOLLOW_SYSTEM_THEME));
        assert!(names.len() > 1);
        for name in names.iter().skip(1) {
            assert!(TerminalTheme::by_name(name).is_some(), "{name}");
        }
    }

    #[test]
    fn labels_keep_every_name_but_the_follow_system_one() {
        let names = theme_choices();
        let labels = crate::dialogs::settings::theme_labels(&names);
        assert_eq!(labels.len(), names.len());
        // The follow-system label goes through i18n; palette names never do.
        assert_eq!(
            labels.iter().skip(1).collect::<Vec<_>>(),
            names.iter().skip(1).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_split_acts_on_its_focused_pane() {
        let (owner, pane) = (Uuid::new_v4(), Uuid::new_v4());
        assert_eq!(pane_or_owner(Some(pane), owner), pane);
        assert_eq!(pane_or_owner(None, owner), owner);
    }

    #[test]
    fn the_mark_follows_the_picked_then_the_configured_theme() {
        let picked = Some("Dracula".to_string());
        assert_eq!(current_theme(picked.clone(), "Nord", false), picked);
        assert_eq!(current_theme(picked.clone(), "Nord", true), picked);
        assert_eq!(current_theme(None, "Nord", false).as_deref(), Some("Nord"));
        // A colour override over the configured theme matches no row.
        assert_eq!(current_theme(None, "Nord", true), None);
    }
}
