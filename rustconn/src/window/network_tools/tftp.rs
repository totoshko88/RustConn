//! TFTP server: pick a directory, then expose it read-only.

use std::cell::RefCell;
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use adw::prelude::*;
use gtk4::gio;
use gtk4::prelude::*;
use libadwaita as adw;
use rustconn_core::transfer::tftp::{TftpError, TftpServer};

use super::{Access, access_widgets, show_error};
use crate::async_utils::with_runtime;
use crate::i18n::{i18n, i18n_f};

thread_local! {
    static ACTIVE_TFTP: RefCell<Option<TftpServer>> = const { RefCell::new(None) };
}

/// Lets the user pick a directory, then the port and interface to expose.
pub(super) fn choose_tftp_directory(window: &adw::ApplicationWindow) {
    let win = window.downgrade();
    gtk4::FileDialog::new().select_folder(Some(window), None::<&gio::Cancellable>, move |result| {
        let Ok(file) = result else { return };
        let Some(path) = file.path() else { return };
        let Some(win) = win.upgrade() else { return };
        if is_sensitive_folder(&path) {
            confirm_sensitive_folder(&win, path);
        } else {
            choose_tftp_bind(&win, path);
        }
    });
}

/// True for the home folder itself or a folder that holds or sits inside `.ssh`.
fn is_sensitive_folder(path: &Path) -> bool {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let is_home = dirs::home_dir()
        .and_then(|home| home.canonicalize().ok())
        .is_some_and(|home| home == canonical);
    is_home
        || canonical.join(".ssh").exists()
        || canonical.components().any(|c| c.as_os_str() == ".ssh")
}

/// Warns that the chosen folder may hold private keys before it is shared.
fn confirm_sensitive_folder(window: &adw::ApplicationWindow, root: PathBuf) {
    let dialog = adw::AlertDialog::builder()
        .heading(i18n("Share this folder?"))
        .body(&i18n_f(
            "{} is your home folder or holds your SSH keys. Anyone who can reach the server could read private files in it. Choose a folder that has only the files you want to share.",
            &[&root.display().to_string()],
        ))
        .default_response("cancel")
        .close_response("cancel")
        .build();
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("share", &i18n("Share anyway"));
    dialog.set_response_appearance("share", adw::ResponseAppearance::Destructive);
    let win = window.downgrade();
    dialog.connect_response(None, move |_, response| {
        if response == "share"
            && let Some(win) = win.upgrade()
        {
            choose_tftp_bind(&win, root.clone());
        }
    });
    dialog.present(Some(window));
}

/// Asks for the port and interface, then starts the read-only server.
fn choose_tftp_bind(window: &adw::ApplicationWindow, root: PathBuf) {
    let ports = gtk4::StringList::new(&[
        &i18n("6969 — unprivileged"),
        &i18n("69 — standard TFTP (needs privileges)"),
    ]);
    let port = gtk4::DropDown::new(Some(ports), gtk4::Expression::NONE);
    port.set_selected(0);
    port.set_hexpand(true);
    port.update_property(&[gtk4::accessible::Property::Label(&i18n("Port"))]);
    let port_column = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    let port_title = gtk4::Label::builder()
        .label(i18n("Port"))
        .xalign(0.0)
        .build();
    port_title.add_css_class("heading");
    let port_hint = gtk4::Label::builder()
        .label(i18n(
            "Port 69 is the standard; Linux, macOS and FreeBSD require elevated rights for it.",
        ))
        .xalign(0.0)
        .wrap(true)
        .build();
    port_hint.add_css_class("dim-label");
    port_column.append(&port_title);
    port_column.append(&port_hint);
    port_column.append(&port);

    let (access_box, access, addresses) = access_widgets(
        &i18n("Bind loopback or the interface a device connects to."),
        false,
    );
    let rows = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    rows.append(&access_box);
    rows.append(&port_column);

    let dialog = adw::AlertDialog::builder()
        .heading(i18n("TFTP server"))
        .body(&i18n_f(
            "Serve {} read-only. Choose the port and the interface a device will reach.",
            &[&root.display().to_string()],
        ))
        .extra_child(&rows)
        .default_response("cancel")
        .close_response("cancel")
        .build();
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("start", &i18n("Start server"));
    dialog.set_response_appearance("start", adw::ResponseAppearance::Suggested);

    let win = window.downgrade();
    let access_row = access.clone();
    let port_row = port.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "start" {
            return;
        }
        let Some(win) = win.upgrade() else { return };
        let index = usize::try_from(access_row.selected()).unwrap_or(0);
        let port = if port_row.selected() == 1 { 69 } else { 6969 };
        let bind = match addresses.get(index).copied() {
            Some(Access::Interface(ip)) => SocketAddr::new(ip, port),
            _ => SocketAddr::from(([127, 0, 0, 1], port)),
        };
        start_tftp(&win, root.clone(), bind);
    });
    dialog.present(Some(window));
}

/// Starts the server and shows its address, or explains why port 69 failed.
fn start_tftp(window: &adw::ApplicationWindow, root: PathBuf, bind: SocketAddr) {
    let server = match with_runtime(|rt| rt.block_on(TftpServer::start(bind, root))) {
        Ok(Ok(server)) => server,
        Ok(Err(e)) => {
            let body = tftp_bind_error(&e, bind.port());
            show_error(window, &i18n("Could not start TFTP server"), &body);
            return;
        }
        Err(e) => {
            show_error(window, &i18n("Could not start TFTP server"), &e);
            return;
        }
    };
    let address = server.local_addr().to_string();
    ACTIVE_TFTP.with(|cell| {
        if let Some(old) = cell.borrow_mut().replace(server) {
            let _ = with_runtime(|rt| rt.block_on(old.stop()));
        }
    });
    let dialog = adw::AlertDialog::builder()
        .heading(i18n("TFTP server running"))
        .body(&i18n_f(
            "Read-only TFTP is listening on {} for the chosen folder.",
            &[&address],
        ))
        .default_response("close")
        .close_response("close")
        .build();
    dialog.add_response("copy", &i18n("Copy address"));
    dialog.add_response("stop", &i18n("Stop server"));
    dialog.add_response("close", &i18n("Close"));
    let win2 = window.downgrade();
    let address_for_copy = address.clone();
    dialog.connect_response(None, move |_, response| match response {
        "copy" => {
            if let Some(win) = win2.upgrade() {
                let display = gtk4::prelude::WidgetExt::display(&win);
                display.clipboard().set_text(&address_for_copy);
            }
        }
        "stop" => {
            if let Some(server) = ACTIVE_TFTP.with(|cell| cell.borrow_mut().take()) {
                let _ = with_runtime(|rt| rt.block_on(server.stop()));
            }
        }
        _ => {}
    });
    dialog.present(Some(window));
}

/// Turns a bind failure on the privileged port into something actionable.
fn tftp_bind_error(error: &TftpError, port: u16) -> String {
    if port == 69
        && let TftpError::Bind(err) = error
        && err.kind() == ErrorKind::PermissionDenied
    {
        return i18n(
            "Port 69 needs elevated rights. Start RustConn as root/administrator, grant CAP_NET_BIND_SERVICE on Linux, or choose port 6969.",
        );
    }
    error.to_string()
}
