//! Quick terminal theme picker.
//!
//! `win.pick-terminal-theme` (command palette) targets the active tab's focused
//! pane; `win.pick-terminal-theme-for` (tab menu) targets the right-clicked tab,
//! as `record-script-for` does. Both open one list; choosing a row repaints the
//! terminal at once with the same colour path Settings uses
//! (`TerminalNotebook::set_session_theme`).
//!
//! shortcut: the choice is per live session and is not saved, because the
//! per-connection store holds fg/bg/cursor hex values, not a theme name. It is
//! lost on reconnect, and Settings > Terminal > Save repaints over it. Upgrade
//! by saving a named theme on the connection.

use super::*;
use crate::i18n::i18n;
use rustconn_core::terminal_themes::TerminalTheme;

/// The names the picker offers, in the order Settings offers them.
fn theme_choices() -> Vec<String> {
    TerminalTheme::theme_names()
}

/// The pane to repaint: the split's focused pane when it has one, else the tab's own session.
fn pane_or_owner(focused_pane: Option<Uuid>, owner: Uuid) -> Uuid {
    focused_pane.unwrap_or(owner)
}

/// Registers the two picker actions.
pub(crate) fn setup_theme_actions(
    window: &adw::ApplicationWindow,
    notebook: &SharedNotebook,
    bridges: &SessionSplitBridges,
) {
    let active = gio::SimpleAction::new("pick-terminal-theme", None);
    let win = window.downgrade();
    let nb = notebook.clone();
    let br = bridges.clone();
    active.connect_activate(move |_, _| {
        if let (Some(win), Some(owner)) = (win.upgrade(), nb.get_active_session_id()) {
            present_picker(&win, &nb, &br, owner);
        }
    });
    window.add_action(&active);

    let for_tab = gio::SimpleAction::new("pick-terminal-theme-for", Some(glib::VariantTy::STRING));
    let win = window.downgrade();
    let nb = notebook.clone();
    let br = bridges.clone();
    for_tab.connect_activate(move |_, target| {
        let Some(owner) = target
            .and_then(glib::Variant::get::<String>)
            .and_then(|id| Uuid::parse_str(&id).ok())
        else {
            return;
        };
        if let Some(win) = win.upgrade() {
            present_picker(&win, &nb, &br, owner);
        }
    });
    window.add_action(&for_tab);
}

fn present_picker(
    window: &adw::ApplicationWindow,
    notebook: &SharedNotebook,
    bridges: &SessionSplitBridges,
    owner: Uuid,
) {
    // A failed borrow means a split is being rebuilt; fall back to the owner.
    let focused = bridges.try_borrow().ok().and_then(|b| {
        b.get(&owner)
            .and_then(|bridge| bridge.get_focused_session())
    });
    let session = pane_or_owner(focused, owner);
    if notebook.get_terminal(session).is_none() {
        crate::toast::show_toast_on_window(
            window,
            &i18n("Select a terminal first"),
            crate::toast::ToastType::Warning,
        );
        return;
    }

    let names = theme_choices();
    let list = gtk4::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk4::SelectionMode::None);
    for name in &names {
        let row = adw::ActionRow::new();
        row.set_title(&glib::markup_escape_text(name));
        row.set_activatable(true);
        list.append(&row);
    }
    let notebook = notebook.clone();
    list.connect_row_activated(move |_, row| {
        if let Some(name) = usize::try_from(row.index()).ok().and_then(|i| names.get(i)) {
            notebook.set_session_theme(session, name);
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
    fn a_split_acts_on_its_focused_pane() {
        let (owner, pane) = (Uuid::new_v4(), Uuid::new_v4());
        assert_eq!(pane_or_owner(Some(pane), owner), pane);
        assert_eq!(pane_or_owner(None, owner), owner);
    }
}
