//! SSH Tunnel Manager window
//!
//! Provides a standalone window for managing SSH port-forwarding tunnels
//! that run independently of terminal sessions. Each tunnel references
//! an existing SSH connection for host/key/password configuration.
//!
//! The add/edit functionality is delegated to `TunnelBuilderDialog`
//! (see `tunnel_builder` module).

use crate::dialogs::tunnel_builder::{TunnelBuilderContext, TunnelBuilderDialog};
use crate::i18n::i18n;
use crate::state::{SharedAppState, with_state, with_state_mut};
use crate::window::SharedTunnelManager;
use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use rustconn_core::models::{Connection, StandaloneTunnel};
use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Tunnel Manager Window
// ---------------------------------------------------------------------------

/// Standalone window for managing SSH tunnels
pub struct TunnelManagerWindow {
    window: adw::Window,
    state: SharedAppState,
    tunnel_manager: SharedTunnelManager,
    content_stack: gtk4::Stack,
    active_group: Rc<RefCell<adw::PreferencesGroup>>,
    stopped_group: Rc<RefCell<adw::PreferencesGroup>>,
    prefs_page: adw::PreferencesPage,
}

impl TunnelManagerWindow {
    /// Creates a new tunnel manager window
    #[must_use]
    pub fn new(
        parent: Option<&gtk4::Window>,
        state: SharedAppState,
        tunnel_manager: SharedTunnelManager,
    ) -> Self {
        let window = adw::Window::builder()
            .title(i18n("SSH Tunnels"))
            .modal(true)
            .default_width(600)
            .default_height(700)
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p));
        }

        window.set_size_request(400, 350);

        // Header bar with add button
        let header = adw::HeaderBar::new();

        let add_button = gtk4::Button::from_icon_name("list-add-symbolic");
        add_button.add_css_class("flat");
        add_button.set_tooltip_text(Some(&i18n("Add Tunnel")));
        add_button.update_property(&[gtk4::accessible::Property::Label(&i18n(
            "Add a new SSH tunnel",
        ))]);
        header.pack_start(&add_button);

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
        window.set_content(Some(&toolbar_view));

        let manager = Self {
            window,
            state,
            tunnel_manager,
            content_stack,
            active_group,
            stopped_group,
            prefs_page,
        };

        // Wire up add buttons
        {
            let state_c = manager.state.clone();
            let window_c = manager.window.clone();
            let tm_c = manager.tunnel_manager.clone();
            let active_g = manager.active_group.clone();
            let stopped_g = manager.stopped_group.clone();
            let stack_c = manager.content_stack.clone();
            let page_c = manager.prefs_page.clone();
            add_button.connect_clicked(move |_| {
                open_tunnel_builder(
                    &window_c, &state_c, None, &tm_c, &active_g, &stopped_g, &stack_c, &page_c,
                );
            });
        }
        {
            let state_c = manager.state.clone();
            let window_c = manager.window.clone();
            let tm_c = manager.tunnel_manager.clone();
            let active_g = manager.active_group.clone();
            let stopped_g = manager.stopped_group.clone();
            let stack_c = manager.content_stack.clone();
            let page_c = manager.prefs_page.clone();
            empty_add_button.connect_clicked(move |_| {
                open_tunnel_builder(
                    &window_c, &state_c, None, &tm_c, &active_g, &stopped_g, &stack_c, &page_c,
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
            window: self.window.clone(),
            state: self.state.clone(),
            tunnel_manager: self.tunnel_manager.clone(),
            active_group: self.active_group.clone(),
            stopped_group: self.stopped_group.clone(),
            content_stack: self.content_stack.clone(),
            prefs_page: self.prefs_page.clone(),
        });

        let tm = self.tunnel_manager.borrow();
        for tunnel in &tunnels {
            let is_running = tm.is_running(tunnel.id);
            let row = build_tunnel_row(tunnel, &connections, is_running);

            // Wire up edit/delete/start/stop buttons in the expanded content
            wire_tunnel_row_actions(&row, tunnel, &ctx);

            if is_running {
                self.active_group.borrow().add(&row);
            } else {
                self.stopped_group.borrow().add(&row);
            }
        }
    }

    /// Presents the tunnel manager window
    pub fn present(&self) {
        self.window.present();
    }
}

// ---------------------------------------------------------------------------
// Helper: build a single tunnel ExpanderRow
// ---------------------------------------------------------------------------

/// Context for wiring tunnel row actions (avoids >6 params)
struct TunnelRowContext {
    window: adw::Window,
    state: SharedAppState,
    tunnel_manager: SharedTunnelManager,
    active_group: Rc<RefCell<adw::PreferencesGroup>>,
    stopped_group: Rc<RefCell<adw::PreferencesGroup>>,
    content_stack: gtk4::Stack,
    prefs_page: adw::PreferencesPage,
}

/// Builds an `adw::ExpanderRow` for a single tunnel definition
fn build_tunnel_row(
    tunnel: &StandaloneTunnel,
    connections: &[Connection],
    is_running: bool,
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

    // Status icon: green = running, gray = stopped
    let status_icon = gtk4::Image::from_icon_name("radio-symbolic");
    if is_running {
        status_icon.add_css_class("success");
    } else {
        status_icon.add_css_class("dim-label");
    }
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
                    if let Err(e) = ctx_c.tunnel_manager.borrow_mut().start(
                        &tunnel_c,
                        conn,
                        cached_pw.as_ref(),
                        &[],
                    ) {
                        tracing::warn!(tunnel = %tunnel_c.name, %e, "Failed to start tunnel");
                    }
                } else {
                    tracing::warn!(
                        tunnel = %tunnel_c.name,
                        connection_id = %tunnel_c.connection_id,
                        "SSH connection not found for tunnel"
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
                &ctx_c.window,
                &ctx_c.state,
                Some(&tunnel_c),
                &ctx_c.tunnel_manager,
                &ctx_c.active_group,
                &ctx_c.stopped_group,
                &ctx_c.content_stack,
                &ctx_c.prefs_page,
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

    confirm.present(Some(&ctx.window));
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
        window: ctx.window.clone(),
        state: ctx.state.clone(),
        tunnel_manager: ctx.tunnel_manager.clone(),
        active_group: ctx.active_group.clone(),
        stopped_group: ctx.stopped_group.clone(),
        content_stack: ctx.content_stack.clone(),
        prefs_page: ctx.prefs_page.clone(),
    });

    let tm = ctx.tunnel_manager.borrow();
    for tunnel in &tunnels {
        let is_running = tm.is_running(tunnel.id);
        let row = build_tunnel_row(tunnel, &connections, is_running);
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
    parent: &adw::Window,
    state: &SharedAppState,
    existing: Option<&StandaloneTunnel>,
    tunnel_manager: &SharedTunnelManager,
    active_group: &Rc<RefCell<adw::PreferencesGroup>>,
    stopped_group: &Rc<RefCell<adw::PreferencesGroup>>,
    content_stack: &gtk4::Stack,
    prefs_page: &adw::PreferencesPage,
) {
    // Build the on_save callback that refreshes the tunnel list
    let refresh_ctx = TunnelRowContext {
        window: parent.clone(),
        state: state.clone(),
        tunnel_manager: tunnel_manager.clone(),
        active_group: active_group.clone(),
        stopped_group: stopped_group.clone(),
        content_stack: content_stack.clone(),
        prefs_page: prefs_page.clone(),
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
    };

    let builder = TunnelBuilderDialog::new(ctx);

    if let Some(tunnel) = existing {
        builder.set_tunnel(tunnel);
    }

    builder.present(parent);
}
