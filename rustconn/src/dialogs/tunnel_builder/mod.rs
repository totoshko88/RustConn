//! Visual Tunnel Builder — wizard dialog for creating/editing SSH tunnels
//!
//! Provides a 3-step wizard with a visual path diagram showing the
//! tunnel chain: localhost → bastion → target.

pub mod path_diagram;
pub mod step_connection;
pub mod step_forwards;
pub mod step_review;

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
pub use path_diagram::TunnelPathDiagram;
use rustconn_core::models::{Connection, PortForward, ProtocolConfig, StandaloneTunnel};
use rustconn_core::tunnel_preview::{TunnelPreviewParams, build_tunnel_preview_command};
pub use step_connection::StepConnectionPage;
pub use step_forwards::StepForwardsPage;
pub use step_review::StepReviewPage;
use uuid::Uuid;

use crate::i18n::i18n;
use crate::state::{SharedAppState, with_state, with_state_mut};
use crate::window::SharedTunnelManager;

// ---------------------------------------------------------------------------
// TunnelBuilderContext
// ---------------------------------------------------------------------------

/// Opens the "New SSH Connection" editor from the wizard.
///
/// The wizard passes in a refresh callback; the window layer (which owns the
/// connection dialog and sidebar reload) invokes it once a connection has been
/// created, so the wizard can reload its connection list. Injecting this as a
/// callback keeps the dialogs layer decoupled from the window layer's
/// connection-creation logic.
pub type NewConnectionOpener = Rc<dyn Fn(Rc<dyn Fn()>)>;

/// Context passed to `TunnelBuilderDialog` (avoids >6 params)
pub struct TunnelBuilderContext {
    pub state: SharedAppState,
    pub tunnel_manager: SharedTunnelManager,
    pub parent_window: adw::Dialog,
    /// Callback invoked after successful save (to refresh tunnel list)
    pub on_save: Rc<RefCell<Option<Box<dyn Fn()>>>>,
    /// Opens the "New SSH Connection" editor (see [`NewConnectionOpener`])
    pub on_new_connection: NewConnectionOpener,
}

// ---------------------------------------------------------------------------
// WizardState
// ---------------------------------------------------------------------------

/// Intermediate state held during wizard navigation
#[derive(Default)]
struct WizardState {
    /// Tunnel being edited (None = creating new)
    editing_id: Option<Uuid>,
    /// Selected SSH connection
    selected_connection: Option<Connection>,
    /// Resolved bastion host (from jump_host_id or manual selection)
    bastion_connection: Option<Connection>,
    /// Port forwarding rules
    forwards: Vec<PortForward>,
    /// Tunnel name
    name: String,
    /// Options
    auto_start: bool,
    auto_reconnect: bool,
}

// ---------------------------------------------------------------------------
// TunnelBuilderDialog
// ---------------------------------------------------------------------------

/// Visual tunnel builder wizard dialog
///
/// A 3-step wizard for creating or editing SSH tunnels:
/// 1. Connection & Name
/// 2. Port Forwards & Options
/// 3. Review & Confirm
pub struct TunnelBuilderDialog {
    dialog: adw::Dialog,
    nav_view: adw::NavigationView,
    step1: StepConnectionPage,
    step2: StepForwardsPage,
    step3: StepReviewPage,
    state: SharedAppState,
    tunnel_manager: SharedTunnelManager,
    wizard_state: Rc<RefCell<WizardState>>,
    on_save: Rc<RefCell<Option<Box<dyn Fn()>>>>,
    /// Opens the "New SSH Connection" editor (see [`NewConnectionOpener`])
    on_new_connection: NewConnectionOpener,
    /// Source ID for status polling (edit mode)
    poll_source_id: Rc<RefCell<Option<glib::SourceId>>>,
}

impl TunnelBuilderDialog {
    /// Creates a new tunnel builder wizard
    #[must_use]
    pub fn new(ctx: TunnelBuilderContext) -> Rc<Self> {
        let wizard_state = Rc::new(RefCell::new(WizardState::default()));
        let poll_source_id: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));

        // Create step pages
        let step1 = StepConnectionPage::new(ctx.state.clone());
        let step2 = StepForwardsPage::new();
        let step3 = StepReviewPage::new();

        // Navigation view
        let nav_view = adw::NavigationView::new();
        nav_view.add(&step1.page);

        // Dialog
        let dialog = adw::Dialog::builder()
            .title(i18n("New Tunnel"))
            .content_width(600)
            .content_height(780)
            .child(&nav_view)
            .build();

        let builder = Rc::new(Self {
            dialog,
            nav_view,
            step1,
            step2,
            step3,
            state: ctx.state,
            tunnel_manager: ctx.tunnel_manager,
            wizard_state,
            on_save: ctx.on_save,
            on_new_connection: ctx.on_new_connection,
            poll_source_id,
        });

        // Wire step navigation
        Self::wire_step1_next(&builder);
        Self::wire_step2_next(&builder);
        Self::wire_step3_save(&builder);
        Self::wire_new_connection(&builder);

        // Stop polling when dialog closes
        let poll_id_c = builder.poll_source_id.clone();
        builder.dialog.connect_closed(move |_| {
            if let Some(id) = poll_id_c.borrow_mut().take() {
                id.remove();
            }
        });

        builder
    }

    /// Pre-populates the wizard with an existing tunnel (edit mode)
    pub fn set_tunnel(&self, tunnel: &StandaloneTunnel) {
        // Store editing ID
        self.wizard_state.borrow_mut().editing_id = Some(tunnel.id);

        // Update dialog title
        self.dialog.set_title(&i18n("Edit Tunnel"));
        self.step1.set_title(&i18n("Edit Tunnel"));

        // Set tunnel name
        self.step1.set_tunnel_name(&tunnel.name);

        // Find and set the connection
        let connection = with_state(&self.state, |s| {
            s.get_connection(tunnel.connection_id).cloned()
        });

        if let Some(ref conn) = connection {
            self.step1.set_connection(conn);

            // Set jump host override if the tunnel's connection has one
            if let ProtocolConfig::Ssh(ref ssh_cfg) = conn.protocol_config {
                self.step1.set_jump_host(ssh_cfg.jump_host_id);
            }
        } else {
            // Connection no longer exists — show error
            self.show_missing_connection_error();
        }

        // Set forwards
        self.step2.set_forwards(&tunnel.forwards);

        // Set options
        self.step2.set_auto_start(tunnel.auto_start);
        self.step2.set_auto_reconnect(tunnel.auto_reconnect);

        // Update save button label
        self.step3.set_save_button_label(&i18n("Save"));

        // Start status polling for edit mode
        self.start_status_polling(tunnel.id);
    }

    /// Registers a callback invoked after successful save
    pub fn connect_save<F: Fn() + 'static>(&self, f: F) {
        *self.on_save.borrow_mut() = Some(Box::new(f));
    }

    /// Presents the dialog as a child of the given widget
    pub fn present(&self, parent: &impl IsA<gtk4::Widget>) {
        self.dialog.present(Some(parent));
    }

    // -----------------------------------------------------------------------
    // Step navigation wiring
    // -----------------------------------------------------------------------

    /// Wires Step 1 "Next" → push Step 2
    fn wire_step1_next(builder: &Rc<Self>) {
        let b = builder.clone();
        builder.step1.connect_next(move || {
            // Collect data from step 1
            let name = b.step1.tunnel_name();
            let conn_id = b.step1.selected_connection_id();
            let bastion = b.step1.bastion_connection();

            let connection =
                conn_id.and_then(|id| with_state(&b.state, |s| s.get_connection(id).cloned()));

            // Update wizard state
            {
                let mut ws = b.wizard_state.borrow_mut();
                ws.name = name;
                ws.selected_connection = connection;
                ws.bastion_connection = bastion;
            }

            // Update step 2 diagram with bastion info from step 1
            let bastion_host = b
                .wizard_state
                .borrow()
                .bastion_connection
                .as_ref()
                .map(|c| c.host.clone());
            b.step2.set_bastion(bastion_host.as_deref());

            // Update step 2 diagram with target host from step 1
            let conn_host = b
                .wizard_state
                .borrow()
                .selected_connection
                .as_ref()
                .map(|c| c.host.clone());
            b.step2.set_target_host(conn_host.as_deref());

            // Push step 2
            b.nav_view.push(&b.step2.page);

            // Move focus to the first interactive element on step 2
            b.step2.grab_initial_focus();
        });
    }

    /// Wires Step 2 "Next" → push Step 3, update summary and SSH preview
    fn wire_step2_next(builder: &Rc<Self>) {
        let b = builder.clone();
        builder.step2.connect_next(move || {
            // Validate forwards before proceeding
            if !b.step2.is_valid() {
                return;
            }

            // Collect data from step 2
            let forwards = b.step2.forwards();
            let auto_start = b.step2.auto_start();
            let auto_reconnect = b.step2.auto_reconnect();

            // Update wizard state
            {
                let mut ws = b.wizard_state.borrow_mut();
                ws.forwards = forwards;
                ws.auto_start = auto_start;
                ws.auto_reconnect = auto_reconnect;
            }

            // Update step 3 summary and preview
            b.update_review_page();

            // Push step 3
            b.nav_view.push(&b.step3.page);

            // Move focus to the save button on step 3
            b.step3.grab_initial_focus();
        });
    }

    /// Wires Step 3 "Save" → save tunnel, call on_save, close dialog
    fn wire_step3_save(builder: &Rc<Self>) {
        let b = builder.clone();
        builder.step3.connect_save(move || {
            let ws = b.wizard_state.borrow();

            // Check if tunnel is running in edit mode — show warning
            if let Some(tunnel_id) = ws.editing_id {
                let status = b.tunnel_manager.borrow().status(tunnel_id);
                if status.is_running() {
                    drop(ws);
                    b.show_running_tunnel_warning();
                    return;
                }
            }

            drop(ws);
            b.perform_save();
        });
    }

    /// Wires both "New SSH Connection" buttons (inline + empty state) on Step 1
    /// to open the connection editor and refresh the connection list on success.
    fn wire_new_connection(builder: &Rc<Self>) {
        let opener = builder.on_new_connection.clone();
        // Weak back-reference avoids a reference cycle (the button closure would
        // otherwise keep the dialog alive forever through `step1`).
        let weak = Rc::downgrade(builder);
        builder.step1.connect_new_connection(move || {
            let Some(b) = weak.upgrade() else {
                return;
            };
            let b_refresh = b.clone();
            let refresh: Rc<dyn Fn()> = Rc::new(move || b_refresh.step1.refresh_connections());
            opener(refresh);
        });
    }

    // -----------------------------------------------------------------------
    // Save logic
    // -----------------------------------------------------------------------

    /// Performs the actual save operation
    fn perform_save(&self) {
        let ws = self.wizard_state.borrow();

        let Some(ref conn) = ws.selected_connection else {
            tracing::warn!("Cannot save tunnel: no connection selected");
            return;
        };

        let connection_id = conn.id;
        let name = ws.name.clone();
        let forwards = ws.forwards.clone();
        let auto_start = ws.auto_start;
        let auto_reconnect = ws.auto_reconnect;
        let editing_id = ws.editing_id;
        drop(ws);

        Self::save_tunnel_to_state(
            &self.state,
            editing_id,
            &name,
            connection_id,
            &forwards,
            auto_start,
            auto_reconnect,
        );

        // Call on_save callback
        if let Some(ref cb) = *self.on_save.borrow() {
            cb();
        }

        // Close dialog
        self.dialog.close();
    }

    /// Writes tunnel data to state (shared between normal save and running-tunnel save)
    fn save_tunnel_to_state(
        state: &SharedAppState,
        editing_id: Option<Uuid>,
        name: &str,
        connection_id: Uuid,
        forwards: &[PortForward],
        auto_start: bool,
        auto_reconnect: bool,
    ) {
        with_state_mut(state, |s| {
            if let Some(id) = editing_id {
                // Update existing tunnel (preserve UUID)
                if let Some(tunnel) = s
                    .settings_mut()
                    .standalone_tunnels
                    .iter_mut()
                    .find(|t| t.id == id)
                {
                    tunnel.name = name.to_string();
                    tunnel.connection_id = connection_id;
                    tunnel.forwards = forwards.to_vec();
                    tunnel.auto_start = auto_start;
                    tunnel.auto_reconnect = auto_reconnect;
                }
            } else {
                // Create new tunnel
                let mut tunnel = StandaloneTunnel::new(name.to_string(), connection_id);
                tunnel.forwards = forwards.to_vec();
                tunnel.auto_start = auto_start;
                tunnel.auto_reconnect = auto_reconnect;
                s.settings_mut().standalone_tunnels.push(tunnel);
            }

            if let Err(e) = s.save_settings() {
                tracing::error!(%e, "Failed to save settings after tunnel save");
            }
        });
    }

    // -----------------------------------------------------------------------
    // Review page update
    // -----------------------------------------------------------------------

    /// Updates the review page with current wizard state
    fn update_review_page(&self) {
        let ws = self.wizard_state.borrow();

        // Connection label
        let connection_label = ws
            .selected_connection
            .as_ref()
            .map(|c| {
                let user = c.username.as_deref().unwrap_or("?");
                format!("{} ({}@{})", c.name, user, c.host)
            })
            .unwrap_or_else(|| i18n("None"));

        // Bastion label
        let bastion_label = ws
            .bastion_connection
            .as_ref()
            .map(|c| format!("{} ({})", c.name, c.host));

        // Update summary
        self.step3.update_summary(
            &ws.name,
            &connection_label,
            bastion_label.as_deref(),
            &ws.forwards,
            ws.auto_start,
            ws.auto_reconnect,
        );

        // Build SSH command preview
        if let Some(ref conn) = ws.selected_connection {
            let proxy_jump = self.resolve_proxy_jump(conn, ws.bastion_connection.as_ref());
            let identity_file = self.resolve_identity_file(conn);

            let params = TunnelPreviewParams {
                host: &conn.host,
                port: conn.port,
                username: conn.username.as_deref(),
                forwards: &ws.forwards,
                proxy_jump: proxy_jump.as_deref(),
                identity_file: identity_file.as_deref(),
            };

            let command = build_tunnel_preview_command(&params);
            self.step3.update_preview_command(&command);
        }

        // Update step 3 diagram
        if let Some(first_fwd) = ws.forwards.first() {
            let bastion_str = ws.bastion_connection.as_ref().map(|c| c.host.clone());
            let target_host = ws.selected_connection.as_ref().map(|c| c.host.clone());

            self.step3.diagram().update(
                Some(first_fwd.local_port),
                bastion_str.as_deref(),
                target_host.as_deref(),
                Some(first_fwd.remote_port),
                Some(first_fwd.direction.clone()),
            );
        } else {
            let bastion_str = ws.bastion_connection.as_ref().map(|c| c.host.clone());
            let target_host = ws.selected_connection.as_ref().map(|c| c.host.clone());

            self.step3.diagram().update(
                None,
                bastion_str.as_deref(),
                target_host.as_deref(),
                None,
                None,
            );
        }
    }

    /// Resolves the proxy jump string for SSH command preview
    fn resolve_proxy_jump(
        &self,
        conn: &Connection,
        bastion: Option<&Connection>,
    ) -> Option<String> {
        // If there's an explicit bastion connection, use it
        if let Some(b) = bastion {
            let user = b.username.as_deref().unwrap_or("root");
            return if b.port == 22 {
                Some(format!("{user}@{}", b.host))
            } else {
                Some(format!("{user}@{}:{}", b.host, b.port))
            };
        }

        // Fall back to connection's proxy_jump string
        if let ProtocolConfig::Ssh(ref ssh_cfg) = conn.protocol_config {
            return ssh_cfg.proxy_jump.clone();
        }

        None
    }

    /// Resolves the identity file path from the connection's SSH config
    fn resolve_identity_file(&self, conn: &Connection) -> Option<String> {
        if let ProtocolConfig::Ssh(ref ssh_cfg) = conn.protocol_config {
            return ssh_cfg.key_path.as_ref().map(|p| p.display().to_string());
        }
        None
    }

    // -----------------------------------------------------------------------
    // Status polling (edit mode)
    // -----------------------------------------------------------------------

    /// Starts polling tunnel status every 2 seconds
    fn start_status_polling(&self, tunnel_id: Uuid) {
        let tm = self.tunnel_manager.clone();
        let step3_diagram = self.step3.diagram().clone();
        let poll_id = self.poll_source_id.clone();

        let source_id = glib::timeout_add_seconds_local(2, move || {
            // Stop if polling was cancelled
            if poll_id.borrow().is_none() {
                return glib::ControlFlow::Break;
            }

            let status = tm.borrow().status(tunnel_id);
            step3_diagram.set_status(&status);

            glib::ControlFlow::Continue
        });

        *self.poll_source_id.borrow_mut() = Some(source_id);
    }

    // -----------------------------------------------------------------------
    // Warning dialogs
    // -----------------------------------------------------------------------

    /// Shows a warning when saving a running tunnel
    fn show_running_tunnel_warning(&self) {
        let warning = adw::AlertDialog::builder()
            .heading(i18n("Tunnel is Running"))
            .body(i18n(
                "This tunnel is currently running. Saving changes will not restart it automatically. You may need to stop and restart the tunnel for changes to take effect.",
            ))
            .build();

        warning.add_response("cancel", &i18n("Cancel"));
        warning.add_response("save", &i18n("Save Anyway"));
        warning.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        warning.set_default_response(Some("cancel"));
        warning.set_close_response("cancel");

        let self_dialog = self.dialog.clone();
        let state = self.state.clone();
        let wizard_state = self.wizard_state.clone();
        let on_save = self.on_save.clone();

        warning.connect_response(None, move |_, response| {
            if response != "save" {
                return;
            }

            // Perform save using shared helper
            let ws = wizard_state.borrow();
            let Some(ref conn) = ws.selected_connection else {
                return;
            };

            let connection_id = conn.id;
            let name = ws.name.clone();
            let forwards = ws.forwards.clone();
            let auto_start = ws.auto_start;
            let auto_reconnect = ws.auto_reconnect;
            let editing_id = ws.editing_id;
            drop(ws);

            Self::save_tunnel_to_state(
                &state,
                editing_id,
                &name,
                connection_id,
                &forwards,
                auto_start,
                auto_reconnect,
            );

            if let Some(ref cb) = *on_save.borrow() {
                cb();
            }

            self_dialog.close();
        });

        warning.present(Some(&self.dialog));
    }

    /// Shows an error when the referenced connection no longer exists
    fn show_missing_connection_error(&self) {
        let error = adw::AlertDialog::builder()
            .heading(i18n("Connection Not Found"))
            .body(i18n(
                "The SSH connection referenced by this tunnel no longer exists. Select a different connection.",
            ))
            .build();

        error.add_response("ok", &i18n("Close"));
        error.set_default_response(Some("ok"));
        error.set_close_response("ok");

        error.present(Some(&self.dialog));
    }
}
