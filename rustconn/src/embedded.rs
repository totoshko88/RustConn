//! Embedded session support for RDP/VNC connections
//!
//! This module provides support for embedding RDP and VNC sessions
//! within the main application window using native protocol clients.
//! On Wayland, sessions fall back to external windows.

use std::cell::RefCell;
use std::process::Child;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, DrawingArea, Label, Orientation, glib};
use libadwaita as adw;
use thiserror::Error;
use uuid::Uuid;

// Re-export DisplayServer from the unified display module for backward compatibility
pub use crate::display::DisplayServer;
use crate::i18n::{i18n, i18n_f};

/// Error type for embedding operations
#[derive(Debug, Clone, Error)]
pub enum EmbeddingError {
    /// Embedding not supported on Wayland
    #[error("Embedding not supported on Wayland for {protocol}")]
    WaylandNotSupported {
        /// The protocol that doesn't support embedding
        protocol: String,
    },
    /// Failed to get window ID for embedding
    #[error("Failed to get window ID for embedding")]
    WindowIdNotAvailable,
    /// Client process failed to start
    #[error("Failed to start client process: {0}")]
    ProcessStartFailed(String),
    /// No FreeRDP 3 client is installed. Carries the version of the FreeRDP
    /// that was refused — FreeRDP 2 — when one was found (issue #351).
    #[error("No supported FreeRDP client is installed (RustConn needs FreeRDP 3)")]
    NoSupportedFreeRdp(Option<rustconn_core::protocol::FreeRdpVersion>),
    /// Client exited unexpectedly
    #[error("Client exited with code {code}")]
    ClientExited {
        /// The exit code
        code: i32,
    },
}

/// Session controls for embedded sessions
#[derive(Clone)]
pub struct SessionControls {
    container: GtkBox,
    disconnect_button: Button,
    status_label: Label,
}

impl SessionControls {
    /// Creates new session controls
    #[must_use]
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Horizontal, 8);
        container.set_margin_start(12);
        container.set_margin_end(12);
        container.set_margin_top(6);
        container.set_margin_bottom(6);

        let status_label = Label::new(Some(&i18n("Connecting…")));
        status_label.set_hexpand(true);
        status_label.set_halign(gtk4::Align::Start);
        status_label.add_css_class("dim-label");
        container.append(&status_label);

        // No fullscreen button here. There was one, and pressing it flipped a
        // private `bool` and called no window API at all — a control that could
        // not do what its tooltip promised. It was never seen: the only
        // `EmbeddedSessionTab` built is the `force_external` one in
        // `window::rdp_vnc`, which exists to own the spawned viewer process and
        // is never added to the notebook. Fullscreen for a real session is
        // `win.toggle-fullscreen`.
        let disconnect_button = Button::from_icon_name("process-stop-symbolic");
        disconnect_button.set_tooltip_text(Some(&i18n("Disconnect")));
        disconnect_button.add_css_class("flat");
        disconnect_button.add_css_class("destructive-action");
        disconnect_button
            .update_property(&[gtk4::accessible::Property::Label(&i18n("Disconnect"))]);
        container.append(&disconnect_button);

        Self {
            container,
            disconnect_button,
            status_label,
        }
    }

    /// Returns the container widget
    #[must_use]
    pub const fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Sets the status text
    pub fn set_status(&self, status: &str) {
        self.status_label.set_text(status);
    }

    /// Connects a callback for the disconnect button
    pub fn connect_disconnect<F: Fn() + 'static>(&self, callback: F) {
        self.disconnect_button.connect_clicked(move |_| callback());
    }
}

impl Default for SessionControls {
    fn default() -> Self {
        Self::new()
    }
}

/// The three mutually-exclusive outcomes of a tabless external RDP launch.
///
/// Grouped into one struct because they always travel together and a bare list
/// of three closures on [`RdpLauncher::start`] reads as noise at the call site.
/// Exactly one fires per launch: an early failure, a changed certificate the
/// user must decide on, or a session that survived the early window and is handed
/// to the shared registry.
pub struct RdpLaunchCallbacks {
    /// Fired with a user-facing message when the client fails shortly after launch.
    pub on_early_failure: Box<dyn FnOnce(String) + 'static>,
    /// Fired with `(host, port, message)` when FreeRDP reports a changed
    /// certificate; the caller shows a confirmation dialog. (#324)
    pub on_cert_changed: Box<dyn FnOnce(String, u16, String) + 'static>,
    /// Fired once the session survives the early window; the caller hands the
    /// spawned child to the external-session registry here.
    pub on_connected: Box<dyn FnOnce() + 'static>,
}

/// Names an embedded RDP or VNC viewer for screen readers.
///
/// A focusable drawing area has no text of its own, so without this Orca
/// announces a bare image and every remote desktop tab sounds the same. The
/// description says how to get the keyboard back, because once the viewer has
/// focus every key goes to the remote machine. Called when the tab is created
/// and again on each rename, so the name follows the tab title.
pub fn label_remote_view(view: &DrawingArea, connection_name: &str) {
    view.update_property(&[
        // Translators: screen-reader name of a remote desktop view; {} is the connection name.
        gtk4::accessible::Property::Label(&i18n_f("Remote desktop {}", &[connection_name])),
        gtk4::accessible::Property::Description(&remote_view_keyboard_hint()),
    ]);
}

/// How to take the keyboard back from an embedded viewer.
///
/// Names the current Toggle Keyboard Passthrough accelerator rather than its
/// default, so a remapped binding is read out. The live registration is safe
/// to read for this one action: it is a default passthrough exception and not
/// one of the single-Control chords suspended while a terminal has focus. With
/// no binding registered the sentence about it is left out.
/// Super+Escape is GNOME Shell's own key for restoring the desktop shortcuts
/// a viewer has inhibited.
fn remote_view_keyboard_hint() -> String {
    let passthrough_key = gtk4::gio::Application::default()
        .and_downcast::<gtk4::Application>()
        .and_then(|app| {
            app.accels_for_action("win.toggle-passthrough")
                .first()
                .cloned()
        })
        .and_then(|accel| gtk4::accelerator_parse(accel.as_str()))
        .map(|(key, mods)| gtk4::accelerator_get_label(key, mods));
    match passthrough_key {
        Some(key) => i18n_f(
            // Translators: read by a screen reader after the remote desktop's
            // name; {} is a key combination such as Shift+Ctrl+Backspace.
            "Keys typed here go to the remote desktop. {} turns keyboard passthrough on or off. In GNOME, Super+Escape gives the desktop shortcuts back.",
            &[key.as_str()],
        ),
        None => i18n(
            "Keys typed here go to the remote desktop. In GNOME, Super+Escape gives the desktop shortcuts back.",
        ),
    }
}

/// Embedded session tab for RDP/VNC connections
#[expect(dead_code, reason = "Fields kept for GTK widget lifecycle")]
pub struct EmbeddedSessionTab {
    id: Uuid,
    connection_id: Uuid,
    protocol: String,
    container: GtkBox,
    embed_area: DrawingArea,
    controls: SessionControls,
    process: Rc<RefCell<Option<Child>>>,
    is_embedded: bool,
}

/// Parameters for the early failure watcher
struct WatchParams {
    host: String,
    port: u16,
    ignore_certificate: bool,
}

impl EmbeddedSessionTab {
    /// Creates a new embedded session tab
    ///
    /// If `force_external` is `true`, the tab always shows the external-window
    /// StatusPage regardless of display server capabilities.
    #[must_use]
    pub fn new(
        connection_id: Uuid,
        connection_name: &str,
        protocol: &str,
        force_external: bool,
    ) -> (Self, bool) {
        let id = Uuid::new_v4();
        let display_server = DisplayServer::detect();
        let is_embedded = !force_external && display_server.supports_embedding();

        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        let controls = SessionControls::new();
        container.append(controls.widget());

        let embed_area = DrawingArea::new();
        embed_area.set_hexpand(true);
        embed_area.set_vexpand(true);
        // The client draws its own window in here; the grab key is the client's
        // (FreeRDP's Right Ctrl), the same one the external-window page lists.
        embed_area.update_property(&[
            gtk4::accessible::Property::Label(&i18n_f("Remote desktop {}", &[connection_name])),
            gtk4::accessible::Property::Description(&i18n(
                "Right Ctrl releases the keyboard and mouse grab",
            )),
        ]);

        let protocol_name = protocol.to_uppercase();
        if is_embedded {
            embed_area.set_content_width(800);
            embed_area.set_content_height(600);
            controls.set_status(&i18n_f(
                // Translators: the first {} is the protocol (RDP, VNC), the second the connection name.
                "{} session - {} (embedded)",
                &[&protocol_name, connection_name],
            ));
        } else {
            controls.set_status(&i18n_f(
                // Translators: the first {} is the protocol (RDP, VNC), the second the connection name.
                "{} session - {} (external window)",
                &[&protocol_name, connection_name],
            ));

            // StatusPage for external sessions — shows hotkeys and connection info.
            // adw::StatusPage description already supports Pango markup natively.
            let description = format!(
                "{}\n\n<b>Ctrl+Alt+Enter</b>  —  {}\n<b>Right Ctrl</b>  —  {}\n<b>Ctrl+Alt+C</b>  —  {}\n\n<small>{}</small>",
                glib::markup_escape_text(connection_name),
                i18n("Toggle fullscreen"),
                i18n("Release keyboard/mouse grab"),
                i18n("Toggle remote control (assistance)"),
                i18n("This tab will close automatically when the session ends"),
            );
            let status_page = adw::StatusPage::builder()
                .icon_name("preferences-desktop-remote-desktop-symbolic")
                .title(i18n("Session running in separate window"))
                .description(description)
                .hexpand(true)
                .vexpand(true)
                .build();
            container.append(&status_page);

            tracing::debug!(
                connection = %connection_name,
                "External RDP tab: StatusPage appended to container (children: controls + status_page)"
            );
        }

        if is_embedded {
            container.append(&embed_area);
        }

        let tab = Self {
            id,
            connection_id,
            protocol: protocol.to_string(),
            container,
            embed_area,
            controls,
            process: Rc::new(RefCell::new(None)),
            is_embedded,
        };

        tab.setup_controls();

        (tab, is_embedded)
    }

    fn setup_controls(&self) {
        let process = self.process.clone();
        self.controls.connect_disconnect(move || {
            if let Some(mut child) = process.borrow_mut().take() {
                let _ = child.kill();
            }
        });
    }

    /// Returns the session UUID
    #[must_use]
    pub const fn id(&self) -> Uuid {
        self.id
    }

    /// Returns the connection UUID
    #[must_use]
    pub const fn connection_id(&self) -> Uuid {
        self.connection_id
    }

    /// Returns the protocol type
    #[must_use]
    pub fn protocol(&self) -> &str {
        &self.protocol
    }

    /// Returns the main container widget
    #[must_use]
    pub const fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Returns whether the session is embedded
    #[must_use]
    pub const fn is_embedded(&self) -> bool {
        self.is_embedded
    }

    /// Sets the status text
    pub fn set_status(&self, status: &str) {
        self.controls.set_status(status);
    }

    /// Sets the child process
    pub fn set_process(&self, child: Child) {
        *self.process.borrow_mut() = Some(child);
    }

    /// Returns a clone of the process handle for external cleanup
    #[must_use]
    pub fn process_handle(&self) -> Rc<RefCell<Option<Child>>> {
        self.process.clone()
    }
}

/// RDP session launcher for embedded and external sessions
pub struct RdpLauncher;

impl RdpLauncher {
    /// Starts an RDP session in an external FreeRDP window.
    ///
    /// Every connection parameter comes from `config`, and the argument list is
    /// built by [`rustconn_core::protocol::build_freerdp_args`] — the same
    /// builder the embedded client's FreeRDP fallback uses. This function used
    /// to assemble its own list from fifteen loose parameters, and being the
    /// older of the two it had drifted: it emitted no `/gateway:` at all, so a
    /// connection behind an RD Gateway dialled the target host directly, and it
    /// passed the user's custom arguments through unfiltered, so a stray `/p:`
    /// aborted the launch instead of being dropped.
    ///
    /// One of [`RdpLaunchCallbacks`] fires on the main loop per launch: an early
    /// failure with a user-facing message, a changed certificate the user must
    /// accept or reject, or a surviving session handed to the registry. The
    /// spawned child stays in `tab`'s handle until one of the first two resolves
    /// or `on_connected` fires — so the changed-certificate decision (#324) is
    /// made while this watcher, not the registry's exit-only poll, owns it.
    ///
    /// # Errors
    /// Returns [`EmbeddingError::NoSupportedFreeRdp`] if no FreeRDP 3 client is
    /// installed, or an error if the process fails to spawn. Early post-spawn
    /// failures are reported asynchronously through `callbacks` instead, so the
    /// GTK main loop is never blocked.
    pub fn start(
        tab: &EmbeddedSessionTab,
        config: &rustconn_core::protocol::FreeRdpConfig,
        callbacks: RdpLaunchCallbacks,
    ) -> Result<(), EmbeddingError> {
        use secrecy::ExposeSecret;
        use std::process::{Command, Stdio};

        let RdpLaunchCallbacks {
            on_early_failure,
            on_cert_changed,
            on_connected,
        } = callbacks;

        // Honour the connection's explicit FreeRDP client choice, falling back
        // to auto-detection when it is unavailable (issue #340). Shares the one
        // resolver with the embedded-widget launcher rather than keeping this
        // path's own candidate list, and with it the FreeRDP 3 requirement:
        // FreeRDP 2 rejects the `/args-from:` command line below (issue #351).
        // Runs on the main thread; each version probe is bounded and cached.
        let selection = crate::embedded_rdp::detect::resolve_freerdp_binary(
            config.client_override.as_deref(),
            config.is_remote_app(),
            None,
        );
        let binary = match selection {
            rustconn_core::protocol::FreeRdpSelection::Supported(binary) => binary,
            refused => {
                return Err(EmbeddingError::NoSupportedFreeRdp(
                    refused.unsupported_version(),
                ));
            }
        };

        // A Flatpak host client comes back as `host:<name>`; keep the marker
        // for logging but spawn through `flatpak-spawn --host` with the bare
        // name (mirrors the embedded-widget launcher).
        let (actual_binary, via_host) = binary
            .strip_prefix("host:")
            .map_or_else(|| (binary.clone(), false), |bare| (bare.to_string(), true));

        let host = config.host.as_str();

        // Forgetting the stored certificate is a local side effect rather than
        // an argument, so it stays here instead of in the shared builder. It goes
        // through the shared helper: this used to be a second copy of the same
        // logic, and it kept the substring match (`line.contains("host port")`)
        // that the helper was written to replace, so `db.example.com` also
        // dropped `my-db.example.com` — and it never looked in `freerdp3/`.
        if config.ignore_certificate {
            crate::embedded_rdp::cert::remove_known_certificate(host, config.port);
        }

        // Connection arguments are written to a guarded file so credentials
        // never appear in the FreeRDP process argument vector.
        let mut plain_args = rustconn_core::protocol::build_freerdp_args(config);

        // SDL-FreeRDP draws its certificate prompt in its own SDL window and
        // never prints the "Certificate … has changed!!!" banner to stdout, so
        // the watcher below would have nothing to read. Force the console
        // callback so it behaves like xfreerdp3 (prints the report to stdout,
        // reads the answer from stdin). Shares the detection helper with the
        // embedded-widget launcher rather than repeating it. (#324)
        if crate::embedded_rdp::launcher::is_sdl_freerdp_binary(&actual_binary) {
            plain_args.push("+force-console-callbacks".to_string());
        }

        let password = config
            .password
            .as_ref()
            .filter(|value| !value.expose_secret().is_empty());
        let mut secret_args = Vec::new();
        if let Some(password) = password {
            secret_args.push(("p", password));
        }
        // Log the binary and the full plain argument vector. The password is
        // written to the args file, never into `plain_args`, so this is safe to
        // log — and it is the only record of exactly which options FreeRDP was
        // asked to parse. Without it, a client that rejects one option leaves
        // nothing but the opaque "Unexpected keyword" in the log, and no way to
        // tell which option (issue #339). The embedded-widget launcher already
        // logs its argv; this tabless path did not.
        tracing::debug!(
            protocol = "rdp",
            binary = %binary,
            host = %host,
            port = config.port,
            args = ?plain_args,
            "[FreeRDP] Launching external client (tabless)"
        );

        let prepared_args = crate::embedded_rdp::SafeFreeRdpLauncher::prepare_args_file(
            &actual_binary,
            &plain_args,
            &secret_args,
        )
        .map_err(|error| EmbeddingError::ProcessStartFailed(error.to_string()))?;

        // Spawn directly, or via `flatpak-spawn --host` when the resolved client
        // lives on the Flatpak host (issue #340).
        let mut cmd = if via_host {
            let mut c = Command::new("flatpak-spawn");
            c.args(["--host", "--watch-bus", &actual_binary]);
            c
        } else {
            Command::new(&actual_binary)
        };
        cmd.arg(prepared_args.argument());

        // Never block on a prompt nobody can answer. On a changed certificate
        // FreeRDP asks "Do you trust the above certificate? (Y/T/N)" and reads
        // stdin; inherited from a terminal it blocks forever, which is exactly
        // how #324 presented ("no errors, no warnings, no connection"). With
        // stdin at `/dev/null` it reads EOF, declines, and exits with a
        // certificate error the watcher can classify — after the banner has
        // been captured from stdout below and turned into a GUI dialog. (#324)
        cmd.stdin(Stdio::null());

        // Capture stderr for error detection.
        cmd.stderr(Stdio::piped());

        // Capture stdout too: FreeRDP prints the whole certificate report — the
        // changed-certificate banner and both thumbprints — to stdout with plain
        // `printf`, while only the `ERRCONNECT_*` codes go to stderr. Reading
        // stderr alone is why this path never noticed a changed certificate. (#324)
        cmd.stdout(Stdio::piped());

        match cmd.spawn() {
            Ok(mut child) => {
                prepared_args.retain_for_post_spawn_parse();

                // Drain stdout on a background thread into a shared buffer so the
                // watcher can spot the changed-certificate banner while the client
                // is still running (it prints the banner, then waits on stdin).
                let stdout_lines: crate::embedded_rdp::StdoutLines =
                    std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
                if let Some(stdout) = child.stdout.take() {
                    let lines = std::sync::Arc::clone(&stdout_lines);
                    std::thread::spawn(move || {
                        use std::io::{BufRead, BufReader};
                        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                            let trimmed = line.trim();
                            if !trimmed.is_empty()
                                && let Ok(mut buf) = lines.lock()
                            {
                                buf.push(trimmed.to_owned());
                            }
                        }
                    });
                }

                tab.set_process(child);
                tab.set_status(&i18n_f("Connecting to {}…", &[host]));
                Self::watch_early_failure(
                    tab,
                    WatchParams {
                        host: host.to_string(),
                        port: config.port,
                        ignore_certificate: config.ignore_certificate,
                    },
                    stdout_lines,
                    on_early_failure,
                    on_cert_changed,
                    on_connected,
                );
                Ok(())
            }
            Err(e) => Err(EmbeddingError::ProcessStartFailed(e.to_string())),
        }
    }

    /// Watches a freshly spawned FreeRDP process for immediate failures
    /// (certificate errors, auth failures) without blocking the GTK main loop.
    ///
    /// FreeRDP exits within ~1s on such errors; the 1500ms window (6 ticks ×
    /// 250ms) matches the blocking detection delay this replaces. It is shorter
    /// than the 2s session monitor in `rdp_vnc.rs`, so an early failure is
    /// always reported here first: the child is taken out of the shared handle,
    /// which makes the session monitor stop without double-closing the tab.
    ///
    /// Also watches `stdout_lines` for the changed-certificate banner. FreeRDP
    /// prints that banner and then waits on stdin, so it never surfaces as an
    /// exit; the watcher must notice the banner, stop the client, and hand the
    /// trust decision to a GUI dialog through `on_cert_changed`. This is the
    /// tabless-path counterpart of the embedded widget's watchdog. (#324)
    fn watch_early_failure(
        tab: &EmbeddedSessionTab,
        params: WatchParams,
        stdout_lines: crate::embedded_rdp::StdoutLines,
        on_early_failure: Box<dyn FnOnce(String) + 'static>,
        on_cert_changed: Box<dyn FnOnce(String, u16, String) + 'static>,
        on_connected: Box<dyn FnOnce() + 'static>,
    ) {
        // A changed certificate makes FreeRDP refuse the TLS handshake and print
        // its banner well inside this window, then wait on stdin; the stdout
        // check below catches it before the process is handed to the registry.
        const EARLY_FAILURE_TICKS: u32 = 6;

        let process = tab.process_handle();
        let controls = tab.controls.clone();
        let host = params.host;
        let mut on_failure = Some(on_early_failure);
        let mut on_cert = Some(on_cert_changed);
        let ignore_certificate = params.ignore_certificate;
        let mut on_connected = Some(on_connected);
        let mut ticks = 0u32;

        glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
            ticks += 1;
            let mut guard = process.borrow_mut();
            let Some(child) = guard.as_mut() else {
                // Process was taken (user disconnected) — nothing to watch.
                return glib::ControlFlow::Break;
            };

            // A changed certificate stalls the client on a stdin prompt rather
            // than exiting, so check the captured stdout before try_wait.
            let certificate_changed = {
                let lines = stdout_lines.lock().unwrap_or_else(|e| e.into_inner());
                crate::embedded_rdp::connection::reports_changed_certificate(&lines.join(" "))
            };
            if certificate_changed {
                tracing::info!(
                    protocol = "rdp",
                    %host,
                    port = params.port,
                    "[FreeRDP] Server certificate changed — stopping the client and asking the user"
                );
                let message =
                    crate::embedded_rdp::connection::certificate_changed_message(&stdout_lines);
                // Take the child so the session monitor sees an empty handle and
                // stops without reporting an error over the dialog.
                if let Some(mut child) = guard.take() {
                    let _ = child.kill();
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                }
                drop(guard);
                if let Some(callback) = on_cert.take() {
                    callback(host.clone(), params.port, message);
                }
                return glib::ControlFlow::Break;
            }

            match child.try_wait() {
                Ok(Some(status)) if !status.success() => {
                    // Early exit with error — take the child so the session
                    // monitor sees an empty handle and stops silently.
                    let error_msg = guard
                        .take()
                        .and_then(|mut child| child.stderr.take())
                        .and_then(|stderr| {
                            use std::io::Read;
                            let mut buf = String::new();
                            let mut reader = std::io::BufReader::new(stderr);
                            reader.read_to_string(&mut buf).ok()?;
                            Some(buf)
                        })
                        .unwrap_or_default();
                    drop(guard);

                    let user_error = Self::parse_freerdp_error(&error_msg, ignore_certificate);
                    if let Some(callback) = on_failure.take() {
                        callback(user_error);
                    }
                    glib::ControlFlow::Break
                }
                Ok(Some(_)) => {
                    // Exited cleanly right away — the session monitor closes the tab.
                    glib::ControlFlow::Break
                }
                Ok(None) if ticks >= EARLY_FAILURE_TICKS => {
                    // Survived the early window with no changed-certificate
                    // banner. A changed certificate makes FreeRDP refuse the TLS
                    // handshake within this window (it prints the banner and
                    // waits, which the check above catches), so a live process
                    // here is a real session. Hand ownership to the shared
                    // registry now — deferred until this point precisely so the
                    // watcher, not the registry, owns the child while the
                    // certificate decision is still open.
                    drop(guard);
                    controls.set_status(&i18n_f("Connected to {}", &[&host]));
                    if let Some(callback) = on_connected.take() {
                        callback();
                    }
                    glib::ControlFlow::Break
                }
                Ok(None) => glib::ControlFlow::Continue,
                Err(_) => glib::ControlFlow::Break,
            }
        });
    }

    /// Parses FreeRDP stderr output to extract a user-friendly error message.
    ///
    /// `ignore_certificate` is the connection's current setting: when it is
    /// already on, a TLS failure is not about an untrusted certificate, so the
    /// message must not tell the user to enable a toggle that is already set
    /// (issue #339). A legacy server (e.g. Windows 2008 R2) that only speaks the
    /// old RDP security layer surfaces the same TLS error, and switching the
    /// security layer to RDP is the real fix there.
    fn parse_freerdp_error(stderr: &str, ignore_certificate: bool) -> String {
        // FreeRDP's command-line parser (winpr) rejects an option it does not
        // recognise with "Unexpected keyword", printed before it ever reaches
        // the server. The bare string is meaningless to a user, and it names a
        // client/argument mismatch rather than anything about the connection —
        // so it gets its own, actionable message that points at the installed
        // FreeRDP version and names the rejected option when winpr reported it
        // (issue #339). The debug log in `start` carries the full argument list.
        if stderr.contains("Unexpected keyword") {
            return match Self::rejected_freerdp_option(stderr) {
                Some(option) => i18n_f(
                    "The installed FreeRDP client rejected the option '{}'. Your FreeRDP version may be too old for it; update FreeRDP or report this at the RustConn issue tracker.",
                    &[&option],
                ),
                None => i18n(
                    "The installed FreeRDP client rejected one of the connection options. Your FreeRDP version may be too old; update FreeRDP or report this at the RustConn issue tracker.",
                ),
            };
        }
        if stderr.contains("certificate not trusted")
            || stderr.contains("ERRCONNECT_TLS_CONNECT_FAILED")
        {
            if stderr.contains("NEW HOST IDENTIFICATION") || stderr.contains("has changed") {
                return i18n(
                    "RDP certificate has changed. Enable 'Accept Certificate' or accept the new certificate.",
                );
            }
            // With certificate checking already bypassed, a TLS failure is not
            // about trust. A legacy server (e.g. Windows 2008 R2) that only
            // offers the old RDP security layer fails the TLS handshake the same
            // way, and setting the security layer to RDP is what fixes it. Do
            // not tell the user to enable a setting that is already on (#339).
            if ignore_certificate {
                return i18n(
                    "TLS handshake failed. For a legacy server (e.g. Windows 2008 R2), set the security layer to RDP in connection settings.",
                );
            }
            return i18n(
                "TLS certificate verification failed. Enable 'Accept Certificate' in connection settings, or set the security layer to RDP for a legacy server.",
            );
        }
        if stderr.contains("ERRCONNECT_CONNECT_CANCELLED")
            || stderr.contains("nla_client_setup_identity")
        {
            return "NLA authentication failed. Check username/password or disable NLA."
                .to_string();
        }
        if stderr.contains("ERRCONNECT_CONNECT_TRANSPORT_FAILED") {
            return "Connection refused. Check host and port.".to_string();
        }
        if stderr.contains("ERRCONNECT_DNS_NAME_NOT_FOUND") {
            return "Host not found. Check the hostname.".to_string();
        }
        // Fallback: return last ERROR line or generic message
        stderr
            .lines()
            .rev()
            .find(|line| line.contains("[ERROR]"))
            .map(|line| {
                // Extract the message part after the last ]:
                line.rsplit("]: ").next().unwrap_or(line).trim().to_string()
            })
            .unwrap_or_else(|| "FreeRDP exited with error (exit code non-zero)".to_string())
    }

    /// Extracts the option winpr rejected from an "Unexpected keyword" line.
    ///
    /// FreeRDP prints `Failed at index N [-<option>]: Unexpected keyword`. The
    /// offending option is in the *last* `[...]` on the line — the wLog prefix
    /// contributes earlier brackets — carried with a leading `-`, `+`, or `/`.
    /// Returns the option name without that sigil, or `None` when the line does
    /// not carry the bracketed form (older wLog builds omit it).
    fn rejected_freerdp_option(stderr: &str) -> Option<String> {
        let line = stderr
            .lines()
            .find(|line| line.contains("Unexpected keyword"))?;
        // Take the content of the last `[...]` pair: split on the final `[`,
        // then keep everything before the closing `]`.
        let after_last_open = line.rsplit_once('[')?.1;
        let inner = after_last_open.split(']').next()?;
        // Treat it as an option only when it carries an option sigil; otherwise
        // the last bracket is the wLog component tag (e.g. `com.winpr.commandline`).
        if !inner.starts_with(['-', '+', '/']) {
            return None;
        }
        let name = inner.trim_start_matches(['-', '+', '/']).trim();
        (!name.is_empty()).then(|| name.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::RdpLauncher;

    /// The winpr command-line parser prints this shape when it rejects an
    /// option token; the offending option is in the last `[...]` pair, after a
    /// wLog prefix that carries its own brackets (issue #339).
    const WINPR_UNEXPECTED_KEYWORD: &str = "[13:12:10] [1234:5678] [ERROR][com.winpr.commandline] - [log_error]: Failed at index 6 [-glyph-cache]: Unexpected keyword";

    #[test]
    fn unexpected_keyword_names_the_rejected_option() {
        let message = RdpLauncher::parse_freerdp_error(WINPR_UNEXPECTED_KEYWORD, false);
        // The bare winpr string never reaches the user: the message explains it
        // is a client/argument mismatch and names the option winpr flagged.
        assert!(message.contains("glyph-cache"), "{message}");
        assert!(message.contains("FreeRDP"), "{message}");
        assert!(
            !message.trim().eq_ignore_ascii_case("Unexpected keyword"),
            "the opaque winpr string must be replaced: {message}"
        );
    }

    #[test]
    fn extracts_option_from_the_last_bracket_not_the_log_prefix() {
        assert_eq!(
            RdpLauncher::rejected_freerdp_option(WINPR_UNEXPECTED_KEYWORD).as_deref(),
            Some("glyph-cache"),
        );
    }

    #[test]
    fn unexpected_keyword_without_bracketed_option_still_explains_it() {
        // Older wLog builds omit the `[-option]` token; the message must still
        // steer the user rather than fall through to the raw string.
        let stderr = "[ERROR][com.winpr.commandline]: Unexpected keyword";
        let message = RdpLauncher::parse_freerdp_error(stderr, false);
        assert!(message.contains("FreeRDP"), "{message}");
        assert!(
            RdpLauncher::rejected_freerdp_option(stderr).is_none(),
            "no bracketed option means no name to quote"
        );
    }

    /// A genuine connection failure must not be mistaken for an argument
    /// mismatch — the "Unexpected keyword" branch is checked first, so this
    /// guards that ordering does not swallow the real classifiers.
    #[test]
    fn connection_errors_are_unaffected_by_the_new_branch() {
        let dns = "[ERROR][com.freerdp.core] ERRCONNECT_DNS_NAME_NOT_FOUND [0x0002000C]";
        assert!(RdpLauncher::parse_freerdp_error(dns, false).contains("Host not found"));
    }

    /// Issue #339 follow-up: when the certificate is already ignored, a TLS
    /// failure must not tell the user to turn on a setting that is already on;
    /// it points at the legacy RDP security layer instead.
    #[test]
    fn tls_failure_with_ignored_cert_suggests_legacy_security_layer() {
        let tls = "[ERROR][com.freerdp.core] ERRCONNECT_TLS_CONNECT_FAILED";
        let message = RdpLauncher::parse_freerdp_error(tls, true);
        assert!(message.contains("RDP"), "{message}");
        assert!(
            !message.contains("Accept Certificate"),
            "must not suggest a toggle that is already on: {message}"
        );
    }

    #[test]
    fn tls_failure_without_ignored_cert_still_mentions_accept_certificate() {
        let tls = "[ERROR][com.freerdp.core] ERRCONNECT_TLS_CONNECT_FAILED";
        let message = RdpLauncher::parse_freerdp_error(tls, false);
        assert!(message.contains("Accept Certificate"), "{message}");
    }
}
