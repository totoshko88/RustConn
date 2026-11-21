//! Session sharing: the read-only HTTP/SSE relay, the WebTransport (QUIC)
//! share and the viewer window that joins one.

use std::cell::{Cell, RefCell};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::rc::Rc;
use std::sync::OnceLock;

use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use rustconn_core::network_scan::local_ipv4_guess;
use rustconn_core::session::share::ShareServer;
use rustconn_core::session::share_quic::{
    QuicShareClient, QuicShareOptions, QuicShareServer, ShareLink, StunConsent,
};
use secrecy::SecretString;
use uuid::Uuid;
use vte4::prelude::TerminalExt;

use super::{Access, access_widgets, show_error};
use crate::i18n::{i18n, i18n_f};
use crate::toast::{ToastType, show_toast_on_window};
use crate::utils::spawn_blocking_with_callback;
use crate::window::types::SharedNotebook;

thread_local! {
    /// At most one relay and one TFTP server per process: both are local,
    /// read-only convenience services, not multi-tenant daemons.
    static ACTIVE_SHARE: RefCell<Option<ActiveShare>> = const { RefCell::new(None) };
    static SHARE_SERIAL: Cell<u64> = const { Cell::new(0) };
    static ACTIVE_QUIC: RefCell<Option<ActiveQuic>> = const { RefCell::new(None) };
}

/// The running relay and the tab it shares.
struct ActiveShare {
    server: ShareServer,
    session_id: Uuid,
    /// Tells the tab watcher whether it still belongs to this share.
    serial: u64,
    notebook: SharedNotebook,
}

/// The running WebTransport share and the tab it shares.
struct ActiveQuic {
    server: QuicShareServer,
    session_id: Uuid,
    /// Same counter as [`ActiveShare::serial`]; the tab watcher compares it.
    serial: u64,
    notebook: SharedNotebook,
}

/// A QUIC share outlives the worker thread that starts it, so it cannot use
/// the GUI thread's thread-local runtime; this one lives for the process.
static QUIC_RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();

fn quic_runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    QUIC_RUNTIME
        .get_or_init(|| tokio::runtime::Runtime::new().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(Clone::clone)
}

/// Asks where to expose the session, then starts the read-only relay.
pub(super) fn start_sharing(
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

    let (access_box, access, hosts) = access_widgets(
        &i18n(
            "Choose this computer only, the interface whose address the link should use, or the internet over WebTransport.",
        ),
        true,
    );

    let dialog = adw::AlertDialog::builder()
        .heading(i18n("Share this session"))
        .body(i18n(
            "The viewer gets the live terminal output read-only. Local and LAN links are unencrypted. The internet link opens in a browser over WebTransport (QUIC); RustConn asks before it contacts any server to find your public address. The tab shows ◉ and its menu has Stop Sharing.",
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
        let Some(win) = win.upgrade() else { return };
        let index = usize::try_from(access_row.selected()).unwrap_or(0);
        match hosts.get(index).copied().unwrap_or(Access::Loopback) {
            Access::Loopback => start_sharing_on(&win, &nb, Some(session_id), None),
            Access::Interface(ip) => start_sharing_on(&win, &nb, Some(session_id), Some(ip)),
            Access::Internet => ask_stun_consent(&win, &nb, session_id),
        }
    });
    dialog.present(Some(window));
}

/// Binds the relay to loopback or to the chosen interface and shows its link.
///
/// The bind runs off the main loop, so the window stays responsive.
fn start_sharing_on(
    window: &adw::ApplicationWindow,
    notebook: &SharedNotebook,
    session_id: Option<Uuid>,
    host: Option<IpAddr>,
) {
    let Some(session_id) = session_id.or_else(|| notebook.get_active_session_id()) else {
        return;
    };
    stop_quic_share();
    stop_http_share();
    let bind = match host {
        Some(ip) => SocketAddr::new(ip, 0),
        None => SocketAddr::from(([127, 0, 0, 1], 0)),
    };
    let token = SecretString::from(uuid::Uuid::new_v4().to_string());
    let win = window.downgrade();
    let nb = notebook.clone();
    spawn_blocking_with_callback(
        move || {
            // shortcut: reuses the process-wide runtime of the QUIC share, since
            // the accept loop must outlive this worker thread.
            quic_runtime().and_then(|runtime| {
                runtime
                    .block_on(ShareServer::start(bind, token))
                    .map_err(|error| error.to_string())
            })
        },
        move |result| {
            let Some(win) = win.upgrade() else { return };
            match result {
                Ok(server) => present_http_share(&win, &nb, session_id, server, host),
                Err(error) => show_error(&win, &i18n("Could not start session sharing"), &error),
            }
        },
    );
}

/// Stops the relay without blocking the caller and clears the tab indicator.
///
/// Returns whether a relay was running.
pub(super) fn stop_http_share() -> bool {
    let Some(active) = ACTIVE_SHARE.with(|cell| cell.borrow_mut().take()) else {
        return false;
    };
    active.notebook.set_shared_marker(active.session_id, false);
    match quic_runtime() {
        Ok(runtime) => {
            runtime.spawn(active.server.stop());
        }
        Err(error) => tracing::warn!(%error, "share: no runtime to stop the relay"),
    }
    true
}

/// Stops the WebTransport share without blocking the caller and clears the
/// tab indicator.
///
/// Returns whether a share was running.
fn stop_quic_share() -> bool {
    let Some(active) = ACTIVE_QUIC.with(|cell| cell.borrow_mut().take()) else {
        return false;
    };
    active.notebook.set_shared_marker(active.session_id, false);
    match quic_runtime() {
        // The endpoint is closed from inside the runtime that owns it.
        Ok(runtime) => {
            runtime.spawn(async move { active.server.stop() });
        }
        Err(error) => tracing::warn!(%error, "share: no runtime to stop the share"),
    }
    true
}

/// Stops whatever this window shares (relay or WebTransport). For the
/// `win.stop-sharing` action in the tab menu.
pub(super) fn stop_all_sharing() -> bool {
    let quic = stop_quic_share();
    stop_http_share() || quic
}

/// Marks the tab, starts the output feed and shows the link.
fn present_http_share(
    window: &adw::ApplicationWindow,
    notebook: &SharedNotebook,
    session_id: Uuid,
    server: ShareServer,
    host: Option<IpAddr>,
) {
    let network = host.is_some();
    let host = host.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    // Raw PTY output goes into the relay's broadcast channel. Overflow makes a
    // slow viewer resync instead of blocking the terminal. With no viewer there
    // is nobody to copy the bytes for.
    let publisher = server.publisher();
    let alt = server.alt_screen();
    notebook.add_output_observer(session_id, move |bytes| {
        alt.observe(bytes);
        if publisher.receiver_count() > 0 {
            let _ = publisher.send(bytes.to_vec());
        }
    });
    let (cols, rows) = notebook.terminal_grid(session_id).unwrap_or((80, 24));
    let url = format!("{}&cols={cols}&rows={rows}", server.local_url_for(host));
    stop_http_share();
    notebook.set_shared_marker(session_id, true);
    let serial = next_serial();
    ACTIVE_SHARE.with(|cell| {
        *cell.borrow_mut() = Some(ActiveShare {
            server,
            session_id,
            serial,
            notebook: notebook.clone(),
        });
    });
    watch_shared_tab(serial, session_id, notebook.clone());

    let body = if network {
        i18n_f(
            "Anyone who has this link can watch the session read-only, any number of times, until you stop sharing. The link uses plain HTTP, so other computers on your network can read what is shown. The tab is marked ◉ while it is shared. {}",
            &[&url],
        )
    } else {
        i18n_f(
            "Anyone on this computer who has this link can watch the session read-only, any number of times, until you stop sharing. The tab is marked ◉ while it is shared. {}",
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
    dialog.set_response_appearance("stop", adw::ResponseAppearance::Destructive);
    dialog.add_response("close", &i18n("Close"));
    let win = window.downgrade();
    dialog.connect_response(None, move |_, response| match response {
        "copy" => {
            if let Some(win) = win.upgrade() {
                let display = gtk4::prelude::WidgetExt::display(&win);
                display.clipboard().set_text(&url);
                show_toast_on_window(&win, &i18n("Link copied to clipboard"), ToastType::Success);
            }
        }
        "stop" => {
            stop_http_share();
        }
        _ => {}
    });
    dialog.present(Some(window));
}

/// A share that nobody stopped ends by itself after this long: the link has no
/// expiry of its own, so a forgotten share would otherwise stay open.
const SHARE_MAX_AGE: std::time::Duration = std::time::Duration::from_hours(1);
/// How long before [`SHARE_MAX_AGE`] the user is told the share is ending.
const SHARE_WARN_BEFORE: std::time::Duration = std::time::Duration::from_mins(5);

/// Live viewers of the share that is running now, relay or WebTransport.
fn viewer_count() -> usize {
    ACTIVE_SHARE
        .with(|cell| {
            cell.borrow()
                .as_ref()
                .map(|a| a.server.publisher().receiver_count())
        })
        .or_else(|| {
            ACTIVE_QUIC.with(|cell| {
                cell.borrow()
                    .as_ref()
                    .map(|a| a.server.publisher().receiver_count())
            })
        })
        .unwrap_or(0)
}

/// Watches a share. Stops it when the shared tab goes away (a share with no
/// terminal behind it would stay open and look active) or when it has run for
/// [`SHARE_MAX_AGE`], and tells the user when the number of viewers changes.
fn watch_shared_tab(serial: u64, session_id: Uuid, notebook: SharedNotebook) {
    let started = std::time::Instant::now();
    let mut viewers = 0;
    let mut warned = false;
    glib::timeout_add_seconds_local(2, move || {
        if current_serial() != Some(serial) {
            return glib::ControlFlow::Break;
        }
        if notebook.terminal_grid(session_id).is_none() {
            stop_all_sharing();
            return glib::ControlFlow::Break;
        }
        if started.elapsed() >= SHARE_MAX_AGE {
            stop_all_sharing();
            crate::toast::show_warning_toast_on_active_window(&i18n(
                "Session sharing stopped after one hour. Share again to continue",
            ));
            return glib::ControlFlow::Break;
        }
        if !warned && started.elapsed() + SHARE_WARN_BEFORE >= SHARE_MAX_AGE {
            warned = true;
            crate::toast::show_warning_toast_on_active_window(&i18n(
                "Session sharing stops in 5 minutes. Share again afterwards to continue",
            ));
        }
        let now = viewer_count();
        if now != viewers {
            viewers = now;
            crate::toast::show_info_toast_on_active_window(&i18n_f(
                "Viewers watching the shared session: {}",
                &[&now.to_string()],
            ));
        }
        glib::ControlFlow::Continue
    });
}

/// Serial of the share that is running now, relay or WebTransport.
fn current_serial() -> Option<u64> {
    ACTIVE_SHARE
        .with(|cell| cell.borrow().as_ref().map(|a| a.serial))
        .or_else(|| ACTIVE_QUIC.with(|cell| cell.borrow().as_ref().map(|a| a.serial)))
}

fn next_serial() -> u64 {
    SHARE_SERIAL.with(|n| {
        n.set(n.get() + 1);
        n.get()
    })
}

/// What a button of the STUN consent dialog means. `None` is a cancel.
fn stun_consent_for(response: &str) -> Option<StunConsent> {
    match response {
        "lookup" => Some(StunConsent::Granted),
        "local" => Some(StunConsent::Declined),
        _ => None,
    }
}

/// Asks before any third-party server is contacted. Nothing leaves this
/// computer until a button is pressed; Cancel and Escape start nothing.
fn ask_stun_consent(window: &adw::ApplicationWindow, notebook: &SharedNotebook, session_id: Uuid) {
    let dialog = adw::AlertDialog::builder()
        .heading(i18n("Look Up Your Public Address?"))
        .body(i18n(
            "To build a link that works over the internet, RustConn can ask public STUN servers run by Google (stun.l.google.com) and Cloudflare (stun.cloudflare.com) for this computer's public address and port. Those companies see this computer's IP address. No terminal data is sent to them. Without the lookup, the link works only on your local network.",
        ))
        .default_response("local")
        .close_response("cancel")
        .build();
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("local", &i18n("Local Network Only"));
    dialog.add_response("lookup", &i18n("Look Up Public Address"));
    dialog.set_response_appearance("lookup", adw::ResponseAppearance::Suggested);
    let win = window.downgrade();
    let nb = notebook.clone();
    dialog.connect_response(None, move |_, response| {
        let Some(consent) = stun_consent_for(response) else {
            return;
        };
        if let Some(win) = win.upgrade() {
            start_quic_sharing(&win, &nb, session_id, consent);
        }
    });
    dialog.present(Some(window));
}

/// Starts the WebTransport share and shows the link a viewer opens in a
/// browser. With `stun` granted it first asks the STUN servers for the public
/// UDP endpoint.
///
/// STUN needs a few seconds on a hostile network, so it runs off the main loop.
fn start_quic_sharing(
    window: &adw::ApplicationWindow,
    notebook: &SharedNotebook,
    session_id: Uuid,
    stun: StunConsent,
) {
    let token = SecretString::from(Uuid::new_v4().to_string());
    // A public endpoint through NAT needs the wildcard address. Without STUN
    // the link only reaches the LAN, so listen on the LAN address alone (or on
    // loopback when there is none) rather than on every interface.
    let bind_ip = match stun {
        StunConsent::Granted => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        StunConsent::Declined => {
            local_ipv4_guess().map_or(IpAddr::V4(Ipv4Addr::LOCALHOST), IpAddr::V4)
        }
    };
    let win = window.downgrade();
    let nb = notebook.clone();
    spawn_blocking_with_callback(
        move || {
            quic_runtime().and_then(|runtime| {
                runtime
                    .block_on(QuicShareServer::start(
                        SocketAddr::new(bind_ip, 0),
                        token,
                        QuicShareOptions {
                            stun,
                            ..QuicShareOptions::default()
                        },
                    ))
                    .map_err(|error| error.to_string())
            })
        },
        move |result| {
            let Some(win) = win.upgrade() else { return };
            match result {
                Ok(server) => present_quic_share(&win, &nb, session_id, server),
                Err(error) => show_error(&win, &i18n("Could not start session sharing"), &error),
            }
        },
    );
}

/// Shows the public address, the terminal-sized link and the stop action.
fn present_quic_share(
    window: &adw::ApplicationWindow,
    notebook: &SharedNotebook,
    session_id: Uuid,
    server: QuicShareServer,
) {
    // With no viewer there is nobody to copy the bytes for.
    let publisher = server.publisher();
    let alt = server.alt_screen();
    notebook.add_output_observer(session_id, move |bytes| {
        alt.observe(bytes);
        if publisher.receiver_count() > 0 {
            let _ = publisher.send(bytes.to_vec());
        }
    });

    let (cols, rows) = notebook.terminal_grid(session_id).unwrap_or((80, 24));
    let stun_available = !server.public_addrs().is_empty();
    let mut targets = server.public_addrs().to_vec();
    if targets.is_empty() {
        // Without STUN the viewer must be on this LAN (or this machine).
        let port = server.udp_addr().port();
        let ip = local_ipv4_guess().map_or(IpAddr::V4(Ipv4Addr::LOCALHOST), IpAddr::V4);
        targets.push(SocketAddr::new(ip, port));
    }
    let page_port = server.page_addr().port();
    let links: Vec<String> = targets
        .iter()
        .map(|target| server.viewer_link_for(*target, cols, rows))
        .collect();
    let labels: Vec<String> = targets
        .iter()
        .map(|target| format!("{}:{page_port}", url_host(target.ip())))
        .collect();

    let address_label = if stun_available {
        i18n("Public endpoint (STUN)")
    } else {
        i18n("Endpoint (local network)")
    };
    let tcp_hint = server.page_addr().port().to_string();
    let udp_hint = server.udp_addr().port().to_string();
    let hint = if tcp_hint == udp_hint {
        i18n_f(
            "If the viewer times out, forward TCP and UDP port {} to this computer on the router. Another RustConn needs only UDP.",
            &[&tcp_hint],
        )
    } else {
        i18n_f(
            "If the viewer times out, forward TCP port {} and UDP port {} to this computer on the router. Another RustConn needs only UDP.",
            &[&tcp_hint, &udp_hint],
        )
    };
    let body = if stun_available {
        i18n(
            "Anyone who has this link can watch read-only until you stop sharing. Send it only over a channel you trust. Another RustConn checks the certificate by itself. A browser shows a certificate warning: before you continue, compare the certificate's SHA-256 fingerprint with the fp= part of the link. The tab is marked ◉ while it is shared.",
        )
    } else {
        i18n(
            "This link only reaches viewers on the local network. Anyone who has it can watch read-only until you stop sharing. Another RustConn checks the certificate by itself. A browser shows a certificate warning: before you continue, compare the certificate's SHA-256 fingerprint with the fp= part of the link. The tab is marked ◉ while it is shared.",
        )
    };
    let built = build_quic_share_dialog(&body, &address_label, &links, &labels, &hint);

    stop_http_share();
    stop_quic_share();
    notebook.set_shared_marker(session_id, true);
    let serial = next_serial();
    ACTIVE_QUIC.with(|cell| {
        *cell.borrow_mut() = Some(ActiveQuic {
            server,
            session_id,
            serial,
            notebook: notebook.clone(),
        });
    });
    watch_shared_tab(serial, session_id, notebook.clone());

    let win = window.downgrade();
    let link_for_copy = built.link.clone();
    built
        .dialog
        .connect_response(None, move |_, response| match response {
            "copy" => {
                if let Some(win) = win.upgrade() {
                    let display = gtk4::prelude::WidgetExt::display(&win);
                    display.clipboard().set_text(&link_for_copy.text());
                    show_toast_on_window(
                        &win,
                        &i18n("Link copied to clipboard"),
                        ToastType::Success,
                    );
                }
            }
            "stop" => {
                stop_quic_share();
            }
            _ => {}
        });
    built.dialog.present(Some(window));
}

/// The parts of the share dialog a test can inspect without presenting it.
struct QuicShareDialog {
    dialog: adw::AlertDialog,
    link: gtk4::Entry,
    /// Read by the GTK test; the dialog shows it without production reading it.
    #[allow(dead_code, reason = "asserted by the ignored GTK test")]
    address: gtk4::DropDown,
    /// Read by the GTK test; the dialog shows it without production reading it.
    #[allow(dead_code, reason = "asserted by the ignored GTK test")]
    hint: gtk4::Label,
}

/// Builds the share dialog around a fixed set of endpoints and links.
fn build_quic_share_dialog(
    body: &str,
    address_label: &str,
    links: &[String],
    labels: &[String],
    hint: &str,
) -> QuicShareDialog {
    let address_title = gtk4::Label::builder()
        .label(address_label)
        .xalign(0.0)
        .build();
    address_title.add_css_class("heading");
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let address = gtk4::DropDown::new(
        Some(gtk4::StringList::new(&label_refs)),
        gtk4::Expression::NONE,
    );
    address.set_selected(0);
    address.set_hexpand(true);
    address.update_property(&[gtk4::accessible::Property::Label(address_label)]);

    let link_label = i18n("Read-only link");
    let link_title = gtk4::Label::builder()
        .label(&link_label)
        .xalign(0.0)
        .build();
    link_title.add_css_class("heading");
    let link = gtk4::Entry::new();
    link.set_text(links.first().map_or("", String::as_str));
    link.set_editable(false);
    link.set_hexpand(true);
    link.update_property(&[gtk4::accessible::Property::Label(&link_label)]);

    let port_hint = gtk4::Label::builder()
        .label(hint)
        .xalign(0.0)
        .wrap(true)
        .build();
    port_hint.add_css_class("dim-label");

    let rows_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    rows_box.append(&address_title);
    rows_box.append(&address);
    rows_box.append(&link_title);
    rows_box.append(&link);
    rows_box.append(&port_hint);

    let link_row = link.clone();
    let links_for_select = links.to_vec();
    address.connect_selected_notify(move |row| {
        let index = usize::try_from(row.selected()).unwrap_or(0);
        if let Some(url) = links_for_select.get(index) {
            link_row.set_text(url);
        }
    });

    let dialog = adw::AlertDialog::builder()
        .heading(i18n("Session shared"))
        .body(body)
        .extra_child(&rows_box)
        .default_response("close")
        .close_response("close")
        .build();
    dialog.add_response("copy", &i18n("Copy link"));
    dialog.add_response("stop", &i18n("Stop sharing"));
    dialog.add_response("close", &i18n("Close"));
    dialog.set_response_appearance("copy", adw::ResponseAppearance::Suggested);
    dialog.set_response_appearance("stop", adw::ResponseAppearance::Destructive);
    QuicShareDialog {
        dialog,
        link,
        address,
        hint: port_hint,
    }
}

/// Asks for a share link, then connects to it over WebTransport.
pub(super) fn join_shared_session(window: &adw::ApplicationWindow) {
    let entry = gtk4::Entry::new();
    entry.set_placeholder_text(Some(&i18n("Paste an https:// share link")));
    entry.set_hexpand(true);
    entry.update_property(&[gtk4::accessible::Property::Label(&i18n("Share link"))]);

    let dialog = adw::AlertDialog::builder()
        .heading(i18n("Join a shared session"))
        .body(i18n(
            "Paste the link the host sent you. RustConn connects directly over QUIC (UDP); the host's UDP port must be reachable.",
        ))
        .extra_child(&entry)
        .default_response("cancel")
        .close_response("cancel")
        .build();
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("connect", &i18n("Connect"));
    dialog.set_response_appearance("connect", adw::ResponseAppearance::Suggested);

    let win = window.downgrade();
    dialog.connect_response(None, move |_, response| {
        if response != "connect" {
            return;
        }
        let Some(win) = win.upgrade() else { return };
        match ShareLink::parse(&entry.text()) {
            Ok(link) => connect_shared_session(&win, link),
            Err(error) => show_error(
                &win,
                &i18n("Could not join the shared session"),
                &error.to_string(),
            ),
        }
    });
    dialog.present(Some(window));
}

/// Connects off the main loop, then opens the read-only viewer window.
fn connect_shared_session(window: &adw::ApplicationWindow, link: ShareLink) {
    let host = link.host.to_string();
    let (cols, rows) = (link.cols, link.rows);
    let (tx, rx) = async_channel::bounded::<Vec<u8>>(64);
    let win = window.downgrade();
    spawn_blocking_with_callback(
        move || {
            quic_runtime().and_then(|runtime| {
                runtime
                    .block_on(QuicShareClient::connect(&link, move |bytes| {
                        let _ = tx.try_send(bytes);
                    }))
                    .map_err(|error| error.to_string())
            })
        },
        move |result| {
            let Some(win) = win.upgrade() else { return };
            match result {
                Ok(client) => present_shared_session(&win, &host, cols, rows, client, rx),
                Err(error) => {
                    show_error(&win, &i18n("Could not join the shared session"), &error);
                }
            }
        },
    );
}

/// Opens a read-only terminal fed by the WebTransport stream.
fn present_shared_session(
    parent: &adw::ApplicationWindow,
    host: &str,
    cols: u16,
    rows: u16,
    client: QuicShareClient,
    bytes: async_channel::Receiver<Vec<u8>>,
) {
    let (window, _terminal) = build_shared_session_window(Some(parent), host, cols, rows, bytes);
    let client = Rc::new(RefCell::new(Some(client)));
    let client_for_close = Rc::clone(&client);
    window.connect_close_request(move |_| {
        if let Some(client) = client_for_close.borrow_mut().take() {
            client.stop();
        }
        glib::Propagation::Proceed
    });
    window.present();
}

/// Builds the read-only viewer window; returning the terminal keeps it testable.
fn build_shared_session_window(
    parent: Option<&adw::ApplicationWindow>,
    host: &str,
    cols: u16,
    rows: u16,
    bytes: async_channel::Receiver<Vec<u8>>,
) -> (adw::Window, vte4::Terminal) {
    let terminal = crate::terminal::read_only_terminal();
    terminal.set_hexpand(true);
    terminal.set_vexpand(true);
    terminal.update_property(&[gtk4::accessible::Property::Label(&i18n_f(
        "Read-only shared session from {}",
        &[host],
    ))]);

    let header = adw::HeaderBar::new();
    let title = adw::WindowTitle::new(
        &i18n("Shared session"),
        &i18n_f("Read-only from {}", &[host]),
    );
    header.set_title_widget(Some(&title));

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&terminal));
    let window = adw::Window::builder()
        .title(i18n("Shared session"))
        .default_width((i32::from(cols) * 9 + 24).clamp(520, 1600))
        .default_height((i32::from(rows) * 18 + 80).clamp(320, 1000))
        .build();
    if let Some(parent) = parent {
        window.set_transient_for(Some(parent));
    }
    // A separate top-level does not inherit the main window's CSS classes.
    #[cfg(target_os = "macos")]
    window.add_css_class("macos");
    window.set_content(Some(&toolbar));

    let term = terminal.clone();
    glib::spawn_future_local(async move {
        while let Ok(chunk) = bytes.recv().await {
            term.feed(&chunk);
        }
    });
    (window, terminal)
}

/// Formats an IP for a URL authority.
fn url_host(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(_) => ip.to_string(),
        IpAddr::V6(ip) => format!("[{ip}]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vte4::prelude::TerminalExt;

    #[test]
    fn only_the_lookup_button_grants_stun_consent() {
        assert_eq!(stun_consent_for("lookup"), Some(StunConsent::Granted));
        assert_eq!(stun_consent_for("local"), Some(StunConsent::Declined));
        // Cancel, Escape and anything unknown start nothing.
        assert_eq!(stun_consent_for("cancel"), None);
        assert_eq!(stun_consent_for(""), None);
        assert_eq!(stun_consent_for("close"), None);
    }

    #[test]
    #[ignore = "initialises GTK: needs a display and its own process; run with xvfb-run -a scripts/run-ignored-gui-tests.sh"]
    fn share_dialog_switches_the_link_with_the_endpoint() {
        if gtk4::init().is_err() {
            return;
        }

        let links = vec![
            "https://203.0.113.9:51988/#token=a".to_owned(),
            "https://203.0.113.10:51988/#token=a".to_owned(),
        ];
        let labels = vec![
            "203.0.113.9:51988".to_owned(),
            "203.0.113.10:51988".to_owned(),
        ];
        let built = build_quic_share_dialog(
            "body text",
            "Public endpoint (STUN)",
            &links,
            &labels,
            "hint text",
        );

        let heading = i18n("Session shared");
        assert_eq!(built.dialog.heading().as_deref(), Some(heading.as_str()));
        assert_eq!(built.dialog.body().as_str(), "body text");
        assert_eq!(built.link.text(), links[0]);
        assert!(!built.link.is_editable());
        assert_eq!(built.hint.text(), "hint text");
        assert_eq!(built.address.selected(), 0);

        built.address.set_selected(1);
        for _ in 0..20 {
            glib::MainContext::default().iteration(false);
        }
        assert_eq!(built.link.text(), links[1]);
    }

    #[test]
    #[ignore = "initialises GTK: needs a display and its own process; run with xvfb-run -a scripts/run-ignored-gui-tests.sh"]
    fn viewer_window_is_read_only_and_sized_for_the_link() {
        if gtk4::init().is_err() {
            return;
        }

        let (_tx, rx) = async_channel::bounded::<Vec<u8>>(4);
        let (window, terminal) = build_shared_session_window(None, "203.0.113.9", 131, 42, rx);

        let title = i18n("Shared session");
        assert_eq!(window.title().as_deref(), Some(title.as_str()));
        assert!(!terminal.is_input_enabled());
        assert_eq!(window.default_width(), 1203);
        assert_eq!(window.default_height(), 836);
    }
}
