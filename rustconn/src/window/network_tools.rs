//! Cross-platform network tools: session sharing, TFTP server, remote file editing.
//!
//! These are window actions rather than `MainWindow` methods: a `SimpleAction`
//! closure cannot hold a `MainWindow` (it is a plain struct, not a `glib::Object`).
//! The same shape is used by `script_actions.rs`.
//!
//! All three act on what the user is looking at — the focused terminal or its
//! connection — so they are reachable from the command palette, not the app menu.

use std::cell::RefCell;
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use rustconn_core::models::Connection;
use rustconn_core::network_scan::local_interfaces;
use rustconn_core::remote_editor::{RemoteEditor, SaveOutcome};
use rustconn_core::session::share::ShareServer;
use rustconn_core::transfer::tftp::{TftpError, TftpServer};
use secrecy::SecretString;
use uuid::Uuid;

use super::types::SharedNotebook;
use crate::async_utils::with_runtime;
use crate::i18n::{i18n, i18n_f};
use crate::state::AppState;
use crate::toast::{ToastType, show_toast_on_window};

thread_local! {
    /// At most one relay and one TFTP server per process: both are local,
    /// read-only convenience services, not multi-tenant daemons.
    static ACTIVE_SHARE: RefCell<Option<ShareServer>> = const { RefCell::new(None) };
    static ACTIVE_TFTP: RefCell<Option<TftpServer>> = const { RefCell::new(None) };
    static ACTIVE_EDITOR: RefCell<Option<RemoteEditor>> = const { RefCell::new(None) };
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
        begin_remote_edit_connection(&win, &connection);
    });
    window.add_action(&edit_connection);
}

/// Builds the labelled full-width "Access" dropdown shared by the share and
/// TFTP dialogs.
///
/// `AdwComboRow` squeezes its value into a few characters inside an
/// `AlertDialog`, which also makes the click target tiny; a plain
/// `GtkDropDown` under its own label keeps the whole row usable.
fn access_widgets(hint: &str) -> (gtk4::Box, gtk4::DropDown, Vec<Option<IpAddr>>) {
    let interfaces = local_interfaces();
    let mut labels: Vec<String> = vec![i18n("This computer only")];
    let mut addresses: Vec<Option<IpAddr>> = vec![None];
    for interface in &interfaces {
        labels.push(if interface.name.is_empty() {
            interface.address.to_string()
        } else {
            format!("{} — {}", interface.name, interface.address)
        });
        addresses.push(Some(interface.address));
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
    (column, dropdown, addresses)
}

/// Asks where to expose the session, then starts the read-only relay.
fn start_sharing(
    window: &adw::ApplicationWindow,
    notebook: &SharedNotebook,
    session_id: Option<Uuid>,
) {
    let Some(session_id) = session_id.or_else(|| notebook.get_active_session_id()) else {
        show_toast_on_window(
            window,
            &i18n("Select a terminal before sharing a session"),
            ToastType::Warning,
        );
        return;
    };

    let (access_box, access, hosts) = access_widgets(&i18n(
        "Choose “This computer only” or the interface whose address the link should use.",
    ));

    let dialog = adw::AlertDialog::builder()
        .heading(i18n("Share this session"))
        .body(i18n(
            "The viewer gets the live terminal output read-only. A network link is not encrypted, so anyone who has it can watch this session.",
        ))
        .extra_child(&access_box)
        .default_response("cancel")
        .close_response("cancel")
        .build();
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("start", &i18n("Start sharing"));
    dialog.set_response_appearance("start", adw::ResponseAppearance::Suggested);

    let win = window.downgrade();
    let nb = notebook.clone();
    let access_row = access.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "start" {
            return;
        }
        let index = usize::try_from(access_row.selected()).unwrap_or(0);
        let host = hosts.get(index).copied().flatten();
        if let Some(win) = win.upgrade() {
            start_sharing_on(&win, &nb, Some(session_id), host);
        }
    });
    dialog.present(Some(window));
}

/// Binds the relay to loopback or to the chosen interface and shows its link.
fn start_sharing_on(
    window: &adw::ApplicationWindow,
    notebook: &SharedNotebook,
    session_id: Option<Uuid>,
    host: Option<IpAddr>,
) {
    let Some(session_id) = session_id.or_else(|| notebook.get_active_session_id()) else {
        return;
    };
    let network = host.is_some();
    let bind = match host {
        Some(ip) => SocketAddr::new(ip, 0),
        None => SocketAddr::from(([127, 0, 0, 1], 0)),
    };
    let host = host.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));

    let token = SecretString::from(uuid::Uuid::new_v4().to_string());
    let server = match with_runtime(|rt| rt.block_on(ShareServer::start(bind, token))) {
        Ok(Ok(server)) => server,
        Ok(Err(e)) => {
            show_error(
                window,
                &i18n("Could not start session sharing"),
                &e.to_string(),
            );
            return;
        }
        Err(e) => {
            show_error(window, &i18n("Could not start session sharing"), &e);
            return;
        }
    };

    // Raw PTY output goes into the relay's broadcast channel. Overflow drops
    // frames for slow viewers instead of blocking the terminal.
    let publisher = server.publisher();
    notebook.add_output_observer(session_id, move |bytes| {
        let _ = publisher.send(bytes.to_vec());
    });
    let (cols, rows) = notebook.terminal_grid(session_id).unwrap_or((80, 24));
    let url = format!("{}&cols={cols}&rows={rows}", server.local_url_for(host));
    ACTIVE_SHARE.with(|cell| {
        if let Some(old) = cell.borrow_mut().replace(server) {
            let _ = with_runtime(|rt| rt.block_on(old.stop()));
        }
    });

    let body = if network {
        i18n_f(
            "Anyone with this read-only link on the network can watch the session: {}",
            &[&url],
        )
    } else {
        i18n_f(
            "Anyone with this read-only link on this computer can watch the session: {}",
            &[&url],
        )
    };
    let dialog = adw::AlertDialog::builder()
        .heading(i18n("Session shared"))
        .body(&body)
        .default_response("close")
        .close_response("close")
        .build();
    dialog.add_response("copy", &i18n("Copy link"));
    dialog.add_response("stop", &i18n("Stop sharing"));
    dialog.add_response("close", &i18n("Close"));
    let win = window.downgrade();
    let url_for_copy = url.clone();
    dialog.connect_response(None, move |_, response| match response {
        "copy" => {
            if let Some(win) = win.upgrade() {
                let display = gtk4::prelude::WidgetExt::display(&win);
                display.clipboard().set_text(&url_for_copy);
                show_toast_on_window(&win, &i18n("Link copied to clipboard"), ToastType::Success);
            }
        }
        "stop" => {
            if let Some(server) = ACTIVE_SHARE.with(|cell| cell.borrow_mut().take()) {
                let _ = with_runtime(|rt| rt.block_on(server.stop()));
            }
        }
        _ => {}
    });
    dialog.present(Some(window));
}

/// Lets the user pick a directory, then the port and interface to expose.
fn choose_tftp_directory(window: &adw::ApplicationWindow) {
    let win = window.downgrade();
    gtk4::FileDialog::new().select_folder(Some(window), None::<&gio::Cancellable>, move |result| {
        let Ok(file) = result else { return };
        let Some(path) = file.path() else { return };
        let Some(win) = win.upgrade() else { return };
        choose_tftp_bind(&win, path);
    });
}

/// Asks for the port and interface, then starts the read-only server.
fn choose_tftp_bind(window: &adw::ApplicationWindow, root: PathBuf) {
    let ports = gtk4::StringList::new(&[
        "6969 — unprivileged",
        "69 — standard TFTP (needs privileges)",
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

    let (access_box, access, addresses) = access_widgets(&i18n(
        "Bind loopback or the interface a device connects to.",
    ));
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
        let bind = match addresses.get(index).copied().flatten() {
            Some(ip) => SocketAddr::new(ip, port),
            None => SocketAddr::from(([127, 0, 0, 1], port)),
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

/// Downloads a remote file, opens it in the system editor, then uploads on Save.
fn begin_remote_edit(
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
fn begin_remote_edit_connection(window: &adw::ApplicationWindow, connection: &Connection) {
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

fn show_error(window: &adw::ApplicationWindow, heading: &str, body: &str) {
    crate::alert::show_error(window, heading, body);
}
