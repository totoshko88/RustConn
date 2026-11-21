//! Cross-platform network tools: session sharing, TFTP server, remote file editing.
//!
//! These are window actions rather than `MainWindow` methods: a `SimpleAction`
//! closure cannot hold a `MainWindow` (it is a plain struct, not a `glib::Object`).
//! The same shape is used by `script_actions.rs`.
//!
//! All three act on what the user is looking at — the focused terminal or its
//! connection — so they are reachable from the command palette, not the app menu.

mod remote_edit;
mod share;
mod tftp;

use std::cell::RefCell;
use std::net::IpAddr;
use std::rc::Rc;

use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use rustconn_core::network_scan::local_interfaces;
use uuid::Uuid;

use self::remote_edit::{begin_remote_edit, begin_remote_edit_connection};
use self::share::{join_shared_session, start_sharing, stop_all_sharing};
use self::tftp::choose_tftp_directory;
use super::types::SharedNotebook;
use crate::i18n::i18n;
use crate::state::AppState;
use crate::toast::{ToastType, show_toast_on_window};

/// Where the viewer of a share connects.
#[derive(Clone, Copy)]
pub(super) enum Access {
    /// This computer, over loopback.
    Loopback,
    /// One local interface, over the LAN.
    Interface(IpAddr),
    /// The internet, over WebTransport through STUN-discovered NAT mappings.
    Internet,
}

/// Registers the three window actions.
pub(crate) fn setup_network_tools(
    window: &adw::ApplicationWindow,
    state: &Rc<RefCell<AppState>>,
    notebook: &SharedNotebook,
) {
    let share = gio::SimpleAction::new("share-session", None);
    let win = window.downgrade();
    let nb = notebook.clone();
    share.connect_activate(move |_, _| {
        if let Some(win) = win.upgrade() {
            start_sharing(&win, &nb, None);
        }
    });
    window.add_action(&share);

    // The tab and connection menus know which session they were opened on;
    // without a target they would act on the active tab and could share or
    // edit a different host.
    let share_for = gio::SimpleAction::new("share-session-for", Some(glib::VariantTy::STRING));
    let win = window.downgrade();
    let nb = notebook.clone();
    share_for.connect_activate(move |_, target| {
        let Some(id) = target.and_then(|v| v.get::<String>()) else {
            return;
        };
        let Ok(id) = Uuid::parse_str(&id) else { return };
        if let Some(win) = win.upgrade() {
            start_sharing(&win, &nb, Some(id));
        }
    });
    window.add_action(&share_for);

    let stop_share = gio::SimpleAction::new("stop-sharing", None);
    let win = window.downgrade();
    stop_share.connect_activate(move |_, _| {
        let stopped = stop_all_sharing();
        if let Some(win) = win.upgrade() {
            // Reachable from the command palette at any time, so "nothing to
            // stop" gets an answer instead of silence.
            let (message, kind) = if stopped {
                (i18n("Sharing stopped"), ToastType::Success)
            } else {
                (i18n("Nothing is being shared"), ToastType::Info)
            };
            show_toast_on_window(&win, &message, kind);
        }
    });
    window.add_action(&stop_share);

    let share_connection =
        gio::SimpleAction::new("share-connection-session", Some(glib::VariantTy::STRING));
    let win = window.downgrade();
    let nb = notebook.clone();
    share_connection.connect_activate(move |_, target| {
        let Some(id) = target
            .and_then(glib::Variant::get::<String>)
            .and_then(|id| Uuid::parse_str(&id).ok())
        else {
            return;
        };
        let Some(win) = win.upgrade() else { return };
        let Some(session) = nb.session_for_connection(id) else {
            show_toast_on_window(
                &win,
                &i18n("That connection has no open session"),
                ToastType::Warning,
            );
            return;
        };
        start_sharing(&win, &nb, Some(session));
    });
    window.add_action(&share_connection);

    let join = gio::SimpleAction::new("join-shared-session", None);
    let win = window.downgrade();
    join.connect_activate(move |_, _| {
        if let Some(win) = win.upgrade() {
            join_shared_session(&win);
        }
    });
    window.add_action(&join);

    let tftp = gio::SimpleAction::new("start-tftp", None);
    let win = window.downgrade();
    tftp.connect_activate(move |_, _| {
        if let Some(win) = win.upgrade() {
            choose_tftp_directory(&win);
        }
    });
    window.add_action(&tftp);

    let edit = gio::SimpleAction::new("edit-remote-file", None);
    let win = window.downgrade();
    let st = state.clone();
    let nb = notebook.clone();
    edit.connect_activate(move |_, _| {
        if let Some(win) = win.upgrade() {
            begin_remote_edit(&win, &st, &nb, None);
        }
    });
    window.add_action(&edit);

    let edit_for = gio::SimpleAction::new("edit-remote-file-for", Some(glib::VariantTy::STRING));
    let win = window.downgrade();
    let st = state.clone();
    let nb = notebook.clone();
    edit_for.connect_activate(move |_, target| {
        let Some(id) = target.and_then(|v| v.get::<String>()) else {
            return;
        };
        let Ok(id) = Uuid::parse_str(&id) else { return };
        if let Some(win) = win.upgrade() {
            begin_remote_edit(&win, &st, &nb, Some(id));
        }
    });
    window.add_action(&edit_for);

    let edit_connection =
        gio::SimpleAction::new("edit-connection-file", Some(glib::VariantTy::STRING));
    let win = window.downgrade();
    let st = state.clone();
    edit_connection.connect_activate(move |_, target| {
        let Some(id) = target
            .and_then(|v| v.get::<String>())
            .and_then(|id| Uuid::parse_str(&id).ok())
        else {
            return;
        };
        let Some(win) = win.upgrade() else { return };
        let connection = st
            .borrow_mut()
            .connection_manager()
            .get_connection(id)
            .cloned();
        let Some(connection) = connection else {
            show_error(
                &win,
                &i18n("Remote file editor"),
                &i18n("Connection not found"),
            );
            return;
        };
        begin_remote_edit_connection(&win, &st, &connection);
    });
    window.add_action(&edit_connection);
}

/// Builds the labelled full-width "Access" dropdown shared by the share and
/// TFTP dialogs. Session sharing adds an internet option; TFTP stops at the
/// local network because it has no encryption or authentication.
///
/// `AdwComboRow` squeezes its value into a few characters inside an
/// `AlertDialog`, which also makes the click target tiny; a plain
/// `GtkDropDown` under its own label keeps the whole row usable.
pub(super) fn access_widgets(
    hint: &str,
    include_internet: bool,
) -> (gtk4::Box, gtk4::DropDown, Vec<Access>) {
    let interfaces = local_interfaces();
    let mut labels: Vec<String> = vec![i18n("This computer only")];
    let mut access = vec![Access::Loopback];
    for interface in &interfaces {
        labels.push(if interface.name.is_empty() {
            interface.address.to_string()
        } else {
            format!("{} — {}", interface.name, interface.address)
        });
        access.push(Access::Interface(interface.address));
    }
    if include_internet {
        labels.push(i18n("Internet — WebTransport in a browser"));
        access.push(Access::Internet);
    }
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let dropdown = gtk4::DropDown::new(
        Some(gtk4::StringList::new(&label_refs)),
        gtk4::Expression::NONE,
    );
    dropdown.set_selected(0);
    dropdown.set_hexpand(true);
    dropdown.update_property(&[gtk4::accessible::Property::Label(&i18n("Access"))]);

    let column = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    let title = gtk4::Label::builder()
        .label(i18n("Access"))
        .xalign(0.0)
        .build();
    title.add_css_class("heading");
    let subtitle = gtk4::Label::builder()
        .label(hint)
        .xalign(0.0)
        .wrap(true)
        .build();
    subtitle.add_css_class("dim-label");
    column.append(&title);
    column.append(&subtitle);
    column.append(&dropdown);
    (column, dropdown, access)
}

pub(super) fn show_error(window: &adw::ApplicationWindow, heading: &str, body: &str) {
    crate::alert::show_error(window, heading, body);
}
