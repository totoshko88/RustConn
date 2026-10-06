//! SSH Tunnel Manager dialog
//!
//! Provides a dialog for managing SSH port-forwarding tunnels
//! that run independently of terminal sessions. Each tunnel references
//! an existing SSH connection for host/key/password configuration.
//!
//! The add/edit functionality is delegated to `TunnelBuilderDialog`
//! (see `tunnel_builder` module).

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use rustconn_core::models::{Connection, StandaloneTunnel};
use rustconn_core::tunnel_manager::TunnelManagerError;
use uuid::Uuid;

use crate::alert;
use crate::dialogs::tunnel_builder::{
    NewConnectionOpener, TunnelBuilderContext, TunnelBuilderDialog,
};
use crate::dialogs::widgets::dialog_pack_action;
use crate::i18n::{i18n, i18n_f};
use crate::state::{SharedAppState, with_state, with_state_mut};
use crate::window::SharedTunnelManager;

// ---------------------------------------------------------------------------
// Tunnel Manager Dialog
// ---------------------------------------------------------------------------

/// Dialog for managing SSH tunnels (migrated from adw::Window to adw::Dialog
/// for consistent presentation across platforms — no native traffic lights on macOS)
pub struct TunnelManagerWindow {
    dialog: adw::Dialog,
    state: SharedAppState,
    tunnel_manager: SharedTunnelManager,
    content_stack: gtk4::Stack,
    active_group: Rc<RefCell<adw::PreferencesGroup>>,
    stopped_group: Rc<RefCell<adw::PreferencesGroup>>,
    prefs_page: adw::PreferencesPage,
    on_new_connection: NewConnectionOpener,
}

impl TunnelManagerWindow {
    /// Creates a new tunnel manager dialog
    #[must_use]
    pub fn new(
        _parent: Option<&gtk4::Window>,
        state: SharedAppState,
        tunnel_manager: SharedTunnelManager,
        on_new_connection: NewConnectionOpener,
    ) -> Self {
        let dialog = adw::Dialog::builder()
            .title(i18n("SSH Tunnels"))
            .content_width(600)
            .content_height(700)
            .build();

        // Header bar with add button
        let header = adw::HeaderBar::new();

        let add_button = gtk4::Button::from_icon_name("list-add-symbolic");
        add_button.add_css_class("flat");
        add_button.set_tooltip_text(Some(&i18n("Add Tunnel")));
        add_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Add a new SSH tunnel",
        ))]);
        dialog_pack_action(&header, &add_button);

        // Content stack: empty state vs tunnel list
        let content_stack = gtk4::Stack::new();
        content_stack.set_transition_type(gtk4::StackTransitionType::Crossfade);

        // Empty state
        let empty_page = adw::StatusPage::builder()
            .icon_name("network-transmit-symbolic")
            .title(i18n("No Tunnels Configured"))
            .description(i18n(
                "SSH tunnels forward ports through encrypted connections",
            ))
            .build();

        let empty_add_button = gtk4::Button::builder()
            .label(i18n("Add Tunnel"))
            .halign(gtk4::Align::Center)
            .css_classes(["suggested-action", "pill"])
            .build();
        empty_page.set_child(Some(&empty_add_button));

        content_stack.add_named(&empty_page, Some("empty"));

        // Tunnel list
        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .tightening_threshold(400)
            .build();

        let scroll = gtk4::ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vexpand(true)
            .build();

        let prefs_page = adw::PreferencesPage::new();

        let active_group = Rc::new(RefCell::new(
            adw::PreferencesGroup::builder()
                .title(i18n("Active"))
                .build(),
        ));
        prefs_page.add(&*active_group.borrow());

        let stopped_group = Rc::new(RefCell::new(
            adw::PreferencesGroup::builder()
                .title(i18n("Stopped"))
                .build(),
        ));
        prefs_page.add(&*stopped_group.borrow());

        scroll.set_child(Some(&prefs_page));
        clamp.set_child(Some(&scroll));
        content_stack.add_named(&clamp, Some("list"));

        // Assemble toolbar view
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&content_stack));
        dialog.set_child(Some(&toolbar_view));

        let manager = Self {
            dialog,
            state,
            tunnel_manager,
            content_stack,
            active_group,
            stopped_group,
            prefs_page,
            on_new_connection,
        };

        // Wire up add buttons
        {
            let state_c = manager.state.clone();
            let dialog_c = manager.dialog.clone();
            let tm_c = manager.tunnel_manager.clone();
            let active_g = manager.active_group.clone();
            let stopped_g = manager.stopped_group.clone();
            let stack_c = manager.content_stack.clone();
            let page_c = manager.prefs_page.clone();
            let new_conn_c = manager.on_new_connection.clone();
            add_button.connect_clicked(move |_| {
                open_tunnel_builder(
                    &dialog_c,
                    &state_c,
                    None,
                    &tm_c,
                    &active_g,
                    &stopped_g,
                    &stack_c,
                    &page_c,
                    &new_conn_c,
                );
            });
        }
        {
            let state_c = manager.state.clone();
            let dialog_c = manager.dialog.clone();
            let tm_c = manager.tunnel_manager.clone();
            let active_g = manager.active_group.clone();
            let stopped_g = manager.stopped_group.clone();
            let stack_c = manager.content_stack.clone();
            let page_c = manager.prefs_page.clone();
            let new_conn_c = manager.on_new_connection.clone();
            empty_add_button.connect_clicked(move |_| {
                open_tunnel_builder(
                    &dialog_c,
                    &state_c,
                    None,
                    &tm_c,
                    &active_g,
                    &stopped_g,
                    &stack_c,
                    &page_c,
                    &new_conn_c,
                );
            });
        }

        manager.refresh_tunnel_list();
        manager
    }

    /// Refreshes the tunnel list from state
    pub fn refresh_tunnel_list(&self) {
        // Remove old groups and create fresh ones
        self.prefs_page.remove(&*self.active_group.borrow());
        self.prefs_page.remove(&*self.stopped_group.borrow());

        let new_active = adw::PreferencesGroup::builder()
            .title(i18n("Active"))
            .build();
        let new_stopped = adw::PreferencesGroup::builder()
            .title(i18n("Stopped"))
            .build();

        self.prefs_page.add(&new_active);
        self.prefs_page.add(&new_stopped);

        *self.active_group.borrow_mut() = new_active;
        *self.stopped_group.borrow_mut() = new_stopped;

        let tunnels = with_state(&self.state, |s| s.settings().standalone_tunnels.clone());
        let connections = with_state(&self.state, |s| {
            s.list_connections()
                .into_iter()
                .cloned()
                .collect::<Vec<_>>()
        });

        if tunnels.is_empty() {
            self.content_stack.set_visible_child_name("empty");
            return;
        }

        self.content_stack.set_visible_child_name("list");

        let ctx = Rc::new(TunnelRowContext {
            dialog: self.dialog.clone(),
            state: self.state.clone(),
            tunnel_manager: self.tunnel_manager.clone(),
            active_group: self.active_group.clone(),
            stopped_group: self.stopped_group.clone(),
            content_stack: self.content_stack.clone(),
            prefs_page: self.prefs_page.clone(),
            on_new_connection: self.on_new_connection.clone(),
        });

        let tm = self.tunnel_manager.borrow();
        for tunnel in &tunnels {
            let is_running = tm.is_running(tunnel.id);
            let row =
                build_tunnel_row(tunnel, &connections, is_running, tm.last_failure(tunnel.id));

            // Wire up edit/delete/start/stop buttons in the expanded content
            wire_tunnel_row_actions(&row, tunnel, &ctx);

            if is_running {
                self.active_group.borrow().add(&row);
            } else {
                self.stopped_group.borrow().add(&row);
            }
        }
    }

    /// Presents the tunnel manager dialog
    pub fn present(&self, parent: Option<&gtk4::Window>) {
        self.register_as_open();
        if let Some(p) = parent {
            self.dialog.present(Some(p));
        } else {
            self.dialog.present(gtk4::Widget::NONE);
        }
    }

    /// Publishes a refresh handle so the tunnel health check can redraw this
    /// dialog while it is open.
    ///
    /// The health check runs on a five-second timer in `window/mod.rs` and had no
    /// way to reach here, so a tunnel that died while the user was looking at this
    /// list kept its row in the Active group saying "Running". The warning icon,
    /// the accessible label and the Last Error row — the entire visible half of
    /// reporting a failed tunnel — only appeared after the dialog was closed and
    /// reopened, or after some other button happened to trigger a refresh.
    ///
    /// The struct itself is dropped immediately after `present`: the caller in
    /// `window/connection_actions.rs` builds it, presents it and lets it go, with
    /// GTK keeping the dialog alive. So this stores the pieces rather than a
    /// handle to `self`, and holds the dialog weakly — a strong reference here
    /// would keep a closed dialog alive for the life of the process.
    fn register_as_open(&self) {
        let ctx = TunnelRowContext {
            dialog: self.dialog.clone(),
            state: self.state.clone(),
            tunnel_manager: self.tunnel_manager.clone(),
            active_group: self.active_group.clone(),
            stopped_group: self.stopped_group.clone(),
            content_stack: self.content_stack.clone(),
            prefs_page: self.prefs_page.clone(),
            on_new_connection: self.on_new_connection.clone(),
        };

        let handle = OpenTunnelManager {
            dialog: self.dialog.downgrade(),
            refresh: Rc::new(move || refresh_from_context(&ctx)),
        };

        OPEN_TUNNEL_MANAGER.with(|slot| {
            *slot.borrow_mut() = Some(handle);
        });

        // Clearing on close is not strictly required — `refresh_open_manager`
        // drops a slot whose dialog is gone — but it releases the captured state
        // and connection list at the moment the user closes the dialog rather
        // than at the next health check.
        self.dialog.connect_closed(|_| {
            OPEN_TUNNEL_MANAGER.with(|slot| {
                slot.borrow_mut().take();
            });
        });
    }
}

/// A live tunnel manager dialog and the closure that redraws its list.
struct OpenTunnelManager {
    /// Weak on purpose: see [`TunnelManagerWindow::register_as_open`].
    dialog: glib::WeakRef<adw::Dialog>,
    refresh: Rc<dyn Fn()>,
}

thread_local! {
    /// The tunnel manager dialog currently on screen, if any.
    ///
    /// `thread_local` rather than a field on the main window because the dialog is
    /// opened from an action that does not keep it, and because everything here is
    /// GTK — main thread only by construction, so there is nothing to synchronise.
    /// Only one can be open at a time: it is modal to the window that presented it.
    static OPEN_TUNNEL_MANAGER: RefCell<Option<OpenTunnelManager>> = const { RefCell::new(None) };
}

/// Redraws the tunnel manager dialog if one is open.
///
/// Call after anything that changes a tunnel's state behind the user's back — the
/// periodic health check is the reason this exists. A no-op when no dialog is
/// open, which is the common case, so it is cheap to call on a timer.
pub fn refresh_open_manager() {
    OPEN_TUNNEL_MANAGER.with(|slot| {
        let refresh = {
            let mut guard = slot.borrow_mut();
            match guard.as_ref() {
                // The dialog is gone but `connect_closed` did not run, or ran
                // before the slot was replaced. Either way the closure would
                // redraw widgets nobody can see.
                Some(open) if open.dialog.upgrade().is_none() => {
                    guard.take();
                    None
                }
                Some(open) => Some(Rc::clone(&open.refresh)),
                None => None,
            }
        };
        // Called with the borrow released: the refresh rebuilds rows and wires
        // their handlers, and a handler that reopens the dialog would otherwise
        // find this still held.
        if let Some(refresh) = refresh {
            refresh();
        }
    });
}

// ---------------------------------------------------------------------------
// Helper: build a single tunnel ExpanderRow
// ---------------------------------------------------------------------------

/// Context for wiring tunnel row actions (avoids >6 params)
struct TunnelRowContext {
    dialog: adw::Dialog,
    state: SharedAppState,
    tunnel_manager: SharedTunnelManager,
    active_group: Rc<RefCell<adw::PreferencesGroup>>,
    stopped_group: Rc<RefCell<adw::PreferencesGroup>>,
    content_stack: gtk4::Stack,
    prefs_page: adw::PreferencesPage,
    on_new_connection: NewConnectionOpener,
}

/// Builds an `adw::ExpanderRow` for a single tunnel definition.
///
/// `failure` is why the tunnel last exited on its own, when it did. A tunnel
/// that crashed and one the user stopped both sit in the Stopped group, so
/// without it the two are indistinguishable — which is what made a dying tunnel
/// invisible.
fn build_tunnel_row(
    tunnel: &StandaloneTunnel,
    connections: &[Connection],
    is_running: bool,
    failure: Option<&str>,
) -> adw::ExpanderRow {
    let summary = if tunnel.forwards.is_empty() {
        i18n("No port forwards configured")
    } else {
        tunnel.forwards_summary()
    };

    let row = adw::ExpanderRow::builder()
        .title(&tunnel.name)
        .subtitle(&summary)
        .build();

    // A tunnel that exited on its own only counts as reported if it looks
    // different from one the user stopped, since both sit in the Stopped group.
    let crashed = failure.filter(|_| !is_running);

    // Status icon: green = running, red warning = exited on its own, gray =
    // stopped. The icon changes along with the colour, because colour alone is
    // not a signal (gnome-hig.md), and the state is also in the accessible
    // label rather than only in the tooltip.
    let (icon, css_class, state_text) = match crashed {
        Some(_) => (
            "dialog-warning-symbolic",
            "error",
            i18n("Stopped unexpectedly"),
        ),
        None if is_running => ("radio-symbolic", "success", i18n("Running")),
        None => ("radio-symbolic", "dim-label", i18n("Stopped")),
    };
    let status_icon = gtk4::Image::from_icon_name(icon);
    status_icon.add_css_class(css_class);
    status_icon.set_tooltip_text(Some(&state_text));
    status_icon.update_property(&[gtk4::accessible::Property::Label(&state_text)]);
    row.add_prefix(&status_icon);

    // Start/Stop toggle button (suffix)
    let (icon_name, tooltip, a11y_label) = if is_running {
        (
            "media-playback-stop-symbolic",
            i18n("Stop Tunnel"),
            i18n("Stop tunnel"),
        )
    } else {
        (
            "media-playback-start-symbolic",
            i18n("Start Tunnel"),
            i18n("Start tunnel"),
        )
    };

    let toggle_btn = gtk4::Button::from_icon_name(icon_name);
    toggle_btn.add_css_class("flat");
    toggle_btn.set_valign(gtk4::Align::Center);
    toggle_btn.set_tooltip_text(Some(&tooltip));
    toggle_btn.update_property(&[gtk4::accessible::Property::Label(&a11y_label)]);
    row.add_suffix(&toggle_btn);

    // Expanded content: connection name row
    let conn_name = connections
        .iter()
        .find(|c| c.id == tunnel.connection_id)
        .map(|c| {
            let user = c.username.as_deref().unwrap_or("?");
            // Escape markup-sensitive characters in connection details
            let escaped_name = glib::markup_escape_text(&c.name);
            let escaped_user = glib::markup_escape_text(user);
            let escaped_host = glib::markup_escape_text(&c.host);
            format!("{escaped_name} ({escaped_user}@{escaped_host})")
        })
        .unwrap_or_else(|| i18n("Unknown connection"));

    let conn_row = adw::ActionRow::builder()
        .title(i18n("SSH Connection"))
        .subtitle(&conn_name)
        .build();
    row.add_row(&conn_row);

    // Why it died, in full. This lives in the expanded body rather than the
    // subtitle because it carries the process's own stderr, which is arbitrarily
    // long and would wreck the collapsed row. `ssh` writes the useful part here
    // — "Permission denied", "Address already in use" — and until now it went
    // only to the log.
    if let Some(reason) = crashed {
        let error_row = adw::ActionRow::builder()
            .title(i18n("Last Error"))
            .subtitle(glib::markup_escape_text(reason.trim()).as_str())
            .subtitle_selectable(true)
            .build();
        error_row.add_css_class("error");
        row.add_row(&error_row);
    }

    // Action buttons row
    let actions_row = adw::ActionRow::builder().title(i18n("Actions")).build();

    let edit_btn = gtk4::Button::from_icon_name("document-edit-symbolic");
    edit_btn.add_css_class("flat");
    edit_btn.set_valign(gtk4::Align::Center);
    edit_btn.set_tooltip_text(Some(&i18n("Edit Tunnel")));
    edit_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Edit this tunnel"))]);

    let delete_btn = gtk4::Button::from_icon_name("user-trash-symbolic");
    delete_btn.add_css_class("flat");
    delete_btn.add_css_class("destructive-action");
    delete_btn.set_valign(gtk4::Align::Center);
    delete_btn.set_tooltip_text(Some(&i18n("Delete Tunnel")));
    delete_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Delete this tunnel",
    ))]);

    actions_row.add_suffix(&edit_btn);
    actions_row.add_suffix(&delete_btn);
    row.add_row(&actions_row);

    row
}

// ---------------------------------------------------------------------------
// Helper: wire edit/delete actions on a tunnel row
// ---------------------------------------------------------------------------

/// Wires edit and delete button actions for a tunnel expander row
fn wire_tunnel_row_actions(
    row: &adw::ExpanderRow,
    tunnel: &StandaloneTunnel,
    ctx: &Rc<TunnelRowContext>,
) {
    // Find the edit and delete buttons inside the actions row.
    // We walk the widget tree to find buttons by icon name.
    let tunnel_id = tunnel.id;
    let tunnel_clone = tunnel.clone();

    // Wire start/stop toggle button
    let is_running = ctx.tunnel_manager.borrow().is_running(tunnel_id);
    let toggle_icon = if is_running {
        "media-playback-stop-symbolic"
    } else {
        "media-playback-start-symbolic"
    };
    if let Some(toggle_btn) = find_button_in_expander(row, toggle_icon) {
        let ctx_c = ctx.clone();
        let tunnel_c = tunnel_clone.clone();
        toggle_btn.connect_clicked(move |_| {
            let running = ctx_c.tunnel_manager.borrow().is_running(tunnel_c.id);
            if running {
                // Stop the tunnel
                if let Err(e) = ctx_c.tunnel_manager.borrow_mut().stop(tunnel_c.id) {
                    tracing::warn!(tunnel = %tunnel_c.name, %e, "Failed to stop tunnel");
                }
            } else {
                // Start the tunnel — find the connection from state
                let connections = with_state(&ctx_c.state, |s| {
                    s.list_connections()
                        .into_iter()
                        .cloned()
                        .collect::<Vec<_>>()
                });
                if let Some(conn) = connections.iter().find(|c| c.id == tunnel_c.connection_id) {
                    // Resolve cached password for the connection
                    let cached_pw: Option<secrecy::SecretString> = with_state(&ctx_c.state, |s| {
                        s.get_cached_credentials(tunnel_c.connection_id)
                            .and_then(|c| {
                                use secrecy::ExposeSecret;
                                let pw = c.password.expose_secret();
                                if pw.is_empty() {
                                    None
                                } else {
                                    Some(c.password.clone())
                                }
                            })
                    });
                    let start_result = ctx_c.tunnel_manager.borrow_mut().start(
                        &tunnel_c,
                        conn,
                        cached_pw.as_ref(),
                        &[],
                    );
                    if let Err(e) = start_result {
                        tracing::warn!(tunnel = %tunnel_c.name, %e, "Failed to start tunnel");
                        // The row is about to be redrawn as "Stopped", which on
                        // its own is indistinguishable from the button not having
                        // been pressed. A start the user asked for and did not get
                        // is a half-finished action, so it gets a dialog rather
                        // than a toast (gnome-hig.md).
                        alert::show_error(
                            &ctx_c.dialog,
                            &i18n("Tunnel Did Not Start"),
                            &tunnel_start_error_body(&tunnel_c.name, &e),
                        );
                    }
                } else {
                    tracing::warn!(
                        tunnel = %tunnel_c.name,
                        connection_id = %tunnel_c.connection_id,
                        "SSH connection not found for tunnel"
                    );
                    alert::show_error(
                        &ctx_c.dialog,
                        &i18n("Tunnel Did Not Start"),
                        &i18n_f(
                            "“{}” refers to an SSH connection that no longer exists. Edit the tunnel and pick a connection.",
                            &[&tunnel_c.name],
                        ),
                    );
                }
            }
            refresh_from_context(&ctx_c);
        });
    }

    if let Some(edit_btn) = find_button_in_expander(row, "document-edit-symbolic") {
        let ctx_c = ctx.clone();
        let tunnel_c = tunnel_clone.clone();
        edit_btn.connect_clicked(move |_| {
            open_tunnel_builder(
                &ctx_c.dialog,
                &ctx_c.state,
                Some(&tunnel_c),
                &ctx_c.tunnel_manager,
                &ctx_c.active_group,
                &ctx_c.stopped_group,
                &ctx_c.content_stack,
                &ctx_c.prefs_page,
                &ctx_c.on_new_connection,
            );
        });
    }

    if let Some(delete_btn) = find_button_in_expander(row, "user-trash-symbolic") {
        let ctx_c = ctx.clone();
        delete_btn.connect_clicked(move |_| {
            delete_tunnel(tunnel_id, &ctx_c);
        });
    }
}

/// Builds the body text for a failed tunnel start.
///
/// GNOME HIG asks an error to say what happened *and* what to do, so the
/// variants that have a remedy get one. `ProgramNotFound` carries the program
/// name from core on purpose: an MPTCP-enabled connection runs `mptcpize` rather
/// than `ssh`, and naming the wrong one sends the user to install something they
/// already have.
///
/// Which is what the remedy sentence itself used to do. It said "Install the
/// OpenSSH client" whatever `program` held, so an MPTCP connection missing
/// `mptcpize` produced "needs mptcpize … install the OpenSSH client" — the exact
/// misdirection the variant exists to avoid, reintroduced one line below the
/// comment explaining it. The advice now follows the program that is actually
/// missing, and [`missing_program_remedy`] is where a third carrier would be
/// added.
fn tunnel_start_error_body(name: &str, error: &TunnelManagerError) -> String {
    match error {
        TunnelManagerError::ProgramNotFound { program } => i18n_f(
            "“{}” needs {}, which is not installed or not on PATH. {}",
            &[name, program, &missing_program_remedy(program)],
        ),
        TunnelManagerError::NotSshConnection(_) => i18n_f(
            "“{}” is attached to a connection that is not SSH. Port forwarding needs an SSH connection.",
            &[name],
        ),
        TunnelManagerError::AlreadyRunning(_) => i18n_f("“{}” is already running.", &[name]),
        // SpawnFailed, ConnectionNotFound and TunnelNotFound have no single
        // remedy, so show what the system reported rather than inventing advice.
        _ => i18n_f("“{}” could not be started: {}", &[name, &error.to_string()]),
    }
}

/// What to do about the program that carries the tunnel being absent.
///
/// Two programs can reach `ProgramNotFound`, and they ship in different packages:
/// `ssh` comes with the OpenSSH client, `mptcpize` with the Multipath TCP tools
/// (`mptcpd` on most distributions). Naming the wrong one is worse than naming
/// none, because the user installs something they already have and the tunnel
/// still does not start.
///
/// An unrecognised name gets generic advice rather than a guess: the string comes
/// from `rustconn-core`, so a future third carrier reaches here before it reaches
/// this `match`, and a wrong package name would be silent.
fn missing_program_remedy(program: &str) -> String {
    match program {
        "ssh" => i18n("Install the OpenSSH client and try again."),
        "mptcpize" => i18n(
            "It ships with the Multipath TCP tools (package “mptcpd” on most distributions). Install them, or turn off Multipath TCP for this connection.",
        ),
        _ => i18n("Install it, or make sure it can be found on PATH."),
    }
}

/// Searches for a button with a specific icon name inside an expander row's widget tree
fn find_button_in_expander(row: &adw::ExpanderRow, icon_name: &str) -> Option<gtk4::Button> {
    find_button_recursive(&row.clone().upcast::<gtk4::Widget>(), icon_name)
}

fn find_button_recursive(widget: &gtk4::Widget, icon_name: &str) -> Option<gtk4::Button> {
    if let Some(btn) = widget.downcast_ref::<gtk4::Button>()
        && btn.icon_name().as_deref() == Some(icon_name)
    {
        return Some(btn.clone());
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(found) = find_button_recursive(&c, icon_name) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

// ---------------------------------------------------------------------------
// Delete tunnel
// ---------------------------------------------------------------------------

/// Shows a confirmation dialog before deleting a tunnel (GNOME HIG: destructive actions)
fn delete_tunnel(tunnel_id: Uuid, ctx: &Rc<TunnelRowContext>) {
    // Look up the tunnel name for the confirmation message
    let tunnel_name = with_state(&ctx.state, |s| {
        s.settings()
            .standalone_tunnels
            .iter()
            .find(|t| t.id == tunnel_id)
            .map(|t| t.name.clone())
            .unwrap_or_default()
    });

    let confirm = adw::AlertDialog::builder()
        .heading(i18n("Delete Tunnel?"))
        .body(crate::i18n::i18n_f(
            "Tunnel \"{}\" will be permanently removed.",
            &[&tunnel_name],
        ))
        .build();

    confirm.add_response("cancel", &i18n("Cancel"));
    confirm.add_response("delete", &i18n("Delete"));
    confirm.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    confirm.set_default_response(Some("cancel"));
    confirm.set_close_response("cancel");

    let ctx_c = ctx.clone();
    confirm.connect_response(None, move |_, response| {
        if response != "delete" {
            return;
        }

        // Stop the tunnel if it's running
        if ctx_c.tunnel_manager.borrow().is_running(tunnel_id) {
            let _ = ctx_c.tunnel_manager.borrow_mut().stop(tunnel_id);
        }

        with_state_mut(&ctx_c.state, |s| {
            s.settings_mut()
                .standalone_tunnels
                .retain(|t| t.id != tunnel_id);
            if let Err(e) = s.save_settings() {
                tracing::error!(%e, "Failed to save settings after tunnel delete");
            }
        });
        refresh_from_context(&ctx_c);
    });

    confirm.present(Some(&ctx.dialog));
}

/// Refreshes the tunnel list using a `TunnelRowContext`
fn refresh_from_context(ctx: &TunnelRowContext) {
    // Remove old groups and create fresh ones
    ctx.prefs_page.remove(&*ctx.active_group.borrow());
    ctx.prefs_page.remove(&*ctx.stopped_group.borrow());

    let new_active = adw::PreferencesGroup::builder()
        .title(i18n("Active"))
        .build();
    let new_stopped = adw::PreferencesGroup::builder()
        .title(i18n("Stopped"))
        .build();

    ctx.prefs_page.add(&new_active);
    ctx.prefs_page.add(&new_stopped);

    *ctx.active_group.borrow_mut() = new_active;
    *ctx.stopped_group.borrow_mut() = new_stopped;

    let tunnels = with_state(&ctx.state, |s| s.settings().standalone_tunnels.clone());
    let connections = with_state(&ctx.state, |s| {
        s.list_connections()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>()
    });

    if tunnels.is_empty() {
        ctx.content_stack.set_visible_child_name("empty");
        return;
    }

    ctx.content_stack.set_visible_child_name("list");

    let rc_ctx = Rc::new(TunnelRowContext {
        dialog: ctx.dialog.clone(),
        state: ctx.state.clone(),
        tunnel_manager: ctx.tunnel_manager.clone(),
        active_group: ctx.active_group.clone(),
        stopped_group: ctx.stopped_group.clone(),
        content_stack: ctx.content_stack.clone(),
        prefs_page: ctx.prefs_page.clone(),
        on_new_connection: ctx.on_new_connection.clone(),
    });

    let tm = ctx.tunnel_manager.borrow();
    for tunnel in &tunnels {
        let is_running = tm.is_running(tunnel.id);
        let row = build_tunnel_row(tunnel, &connections, is_running, tm.last_failure(tunnel.id));
        wire_tunnel_row_actions(&row, tunnel, &rc_ctx);
        if is_running {
            ctx.active_group.borrow().add(&row);
        } else {
            ctx.stopped_group.borrow().add(&row);
        }
    }
}

// ---------------------------------------------------------------------------
// Open Tunnel Builder Dialog
// ---------------------------------------------------------------------------

/// Opens the `TunnelBuilderDialog` wizard for creating or editing a tunnel.
///
/// When `existing` is `Some`, the wizard is pre-populated for editing (preserves UUID).
/// When `None`, a blank wizard is shown for creating a new tunnel.
#[expect(
    clippy::too_many_arguments,
    reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
)]
fn open_tunnel_builder(
    parent: &adw::Dialog,
    state: &SharedAppState,
    existing: Option<&StandaloneTunnel>,
    tunnel_manager: &SharedTunnelManager,
    active_group: &Rc<RefCell<adw::PreferencesGroup>>,
    stopped_group: &Rc<RefCell<adw::PreferencesGroup>>,
    content_stack: &gtk4::Stack,
    prefs_page: &adw::PreferencesPage,
    on_new_connection: &NewConnectionOpener,
) {
    // Build the on_save callback that refreshes the tunnel list
    let refresh_ctx = TunnelRowContext {
        dialog: parent.clone(),
        state: state.clone(),
        tunnel_manager: tunnel_manager.clone(),
        active_group: active_group.clone(),
        stopped_group: stopped_group.clone(),
        content_stack: content_stack.clone(),
        prefs_page: prefs_page.clone(),
        on_new_connection: on_new_connection.clone(),
    };

    let on_save: Rc<RefCell<Option<Box<dyn Fn()>>>> =
        Rc::new(RefCell::new(Some(Box::new(move || {
            refresh_from_context(&refresh_ctx);
        }))));

    let ctx = TunnelBuilderContext {
        state: state.clone(),
        tunnel_manager: tunnel_manager.clone(),
        parent_window: parent.clone(),
        on_save,
        on_new_connection: on_new_connection.clone(),
    };

    let builder = TunnelBuilderDialog::new(ctx);

    if let Some(tunnel) = existing {
        builder.set_tunnel(tunnel);
    }

    builder.present(parent);
}

#[cfg(test)]
mod start_error_tests {
    use rustconn_core::tunnel_manager::TunnelManagerError;
    use uuid::Uuid;

    use super::{missing_program_remedy, tunnel_start_error_body};

    /// These are pure string builders — no widget is constructed — so they run
    /// without a display. `i18n` is `gettext`, which returns the msgid unchanged
    /// with no catalogue loaded, so the assertions below read the English source
    /// strings.
    fn program_not_found(program: &str) -> TunnelManagerError {
        TunnelManagerError::ProgramNotFound {
            program: program.to_string(),
        }
    }

    /// The regression this file's own comment describes and then contradicted:
    /// `ProgramNotFound` carries the program name precisely so the user is not
    /// sent to install something they already have, and the remedy sentence said
    /// "Install the OpenSSH client" regardless.
    #[test]
    fn a_missing_mptcpize_does_not_send_the_user_to_install_openssh() {
        let body = tunnel_start_error_body("mysql prod", &program_not_found("mptcpize"));

        assert!(body.contains("mptcpize"), "program name missing: {body}");
        assert!(
            !body.contains("OpenSSH"),
            "still naming the wrong package: {body}"
        );
        // And it has to say what to do instead, not merely omit the wrong advice.
        assert!(body.contains("mptcpd"), "no remedy offered: {body}");
    }

    #[test]
    fn a_missing_ssh_still_names_the_openssh_client() {
        let body = tunnel_start_error_body("mysql prod", &program_not_found("ssh"));

        assert!(body.contains("OpenSSH"), "remedy missing: {body}");
        assert!(!body.contains("mptcpd"), "wrong remedy: {body}");
    }

    /// The name comes from `rustconn-core`, so a third carrier added there reaches
    /// this function before it reaches the `match`. It must fall back to generic
    /// advice rather than guessing a package.
    #[test]
    fn an_unknown_program_gets_generic_advice_and_never_a_guessed_package() {
        let remedy = missing_program_remedy("some-future-wrapper");

        assert!(!remedy.is_empty());
        assert!(!remedy.contains("OpenSSH"));
        assert!(!remedy.contains("mptcpd"));
        assert!(remedy.contains("PATH"), "no actionable advice: {remedy}");
    }

    /// Every variant has to name the tunnel, because the dialog is not otherwise
    /// attached to a row and several tunnels can be listed at once.
    #[test]
    fn every_variant_names_the_tunnel_it_is_about() {
        let id = Uuid::new_v4();
        let errors = [
            program_not_found("ssh"),
            TunnelManagerError::NotSshConnection(id),
            TunnelManagerError::AlreadyRunning(id),
            TunnelManagerError::TunnelNotFound(id),
            TunnelManagerError::ConnectionNotFound(id),
        ];

        for error in errors {
            let body = tunnel_start_error_body("mysql prod", &error);
            assert!(
                body.contains("mysql prod"),
                "{error:?} produced a body with no tunnel name: {body}"
            );
        }
    }

    /// The fallback arm exists to show what the system reported rather than
    /// inventing advice, so the underlying error text has to survive into it.
    #[test]
    fn the_fallback_arm_passes_the_systems_own_words_through() {
        let error = TunnelManagerError::SpawnFailed(std::io::Error::other("no pty available"));
        let body = tunnel_start_error_body("mysql prod", &error);

        assert!(
            body.contains("no pty available"),
            "system message was dropped: {body}"
        );
    }
}
