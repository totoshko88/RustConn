//! RDP and VNC connection methods for main window
//!
//! This module contains functions for starting RDP and VNC connections
//! with password dialogs and credential handling.

use crate::dialogs::PasswordDialog;
use crate::embedded::{EmbeddedSessionTab, RdpLauncher};
use crate::sidebar::ConnectionSidebar;
use crate::split_view::SplitViewBridge;
use crate::state::SharedAppState;
use crate::terminal::TerminalNotebook;
use gtk4::prelude::*;
use rustconn_core::models::PasswordSource;
use secrecy::ExposeSecret;

use std::rc::Rc;
use uuid::Uuid;

/// Type alias for shared sidebar reference
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Type alias for shared notebook reference
pub type SharedNotebook = Rc<TerminalNotebook>;

/// Type alias for shared split view reference
pub type SharedSplitView = Rc<SplitViewBridge>;

/// Starts an RDP connection with password dialog
pub fn start_rdp_with_password_dialog(
    state: SharedAppState,
    notebook: SharedNotebook,
    split_view: SharedSplitView,
    sidebar: SharedSidebar,
    connection_id: Uuid,
    window: &gtk4::Window,
) {
    use rustconn_core::variables::{VariableManager, VariableScope};

    // Helper function to substitute variables
    let substitute_vars = |input: &str, global_variables: &[rustconn_core::Variable]| -> String {
        if !input.contains("${") {
            return input.to_string();
        }
        let mut manager = VariableManager::new();
        for var in global_variables {
            manager.set_global(var.clone());
        }
        manager
            .substitute_for_command(input, VariableScope::Global)
            .unwrap_or_else(|_| input.to_string())
    };

    // Check if we have cached credentials (fast, non-blocking)
    let cached = {
        let state_ref = state.borrow();
        state_ref.get_cached_credentials(connection_id).map(|c| {
            use secrecy::ExposeSecret;
            use zeroize::Zeroizing;
            (
                c.username.clone(),
                Zeroizing::new(c.password.expose_secret().to_string()),
                c.domain.clone(),
            )
        })
    };

    if let Some((username, password, domain)) = cached {
        // Use cached credentials directly
        start_rdp_session_with_credentials(
            &state,
            &notebook,
            &split_view,
            &sidebar,
            connection_id,
            &username,
            &password,
            &domain,
        );
        return;
    }

    // Get connection info for dialog with variable substitution
    let (conn_name, username, domain) = {
        let state_ref = state.borrow();
        if let Some(conn) = state_ref.get_connection(connection_id) {
            let global_variables = crate::state::resolve_global_variables(state_ref.settings());
            let raw_username = conn.username.clone().unwrap_or_default();
            let raw_domain = conn.domain.clone().unwrap_or_default();
            (
                conn.name.clone(),
                substitute_vars(&raw_username, &global_variables),
                substitute_vars(&raw_domain, &global_variables),
            )
        } else {
            return;
        }
    };

    // Create and show password dialog
    let dialog = PasswordDialog::new(Some(window));
    dialog.set_connection_name(&conn_name);
    dialog.set_username(&username);
    dialog.set_domain(&domain);

    let sidebar_clone = sidebar.clone();
    dialog.show(move |result| {
        let Some(creds) = result else {
            // User cancelled the password dialog — clear "connecting" status
            sidebar_clone.update_connection_status(&connection_id.to_string(), "");
            return;
        };
        {
            // Determine if we should save: explicit request OR password_source == Vault
            let should_save = creds.save_credentials || {
                let state_ref = state.borrow();
                state_ref
                    .get_connection(connection_id)
                    .map(|c| c.password_source == PasswordSource::Vault)
                    .unwrap_or(false)
            };

            if should_save {
                // Get connection details for vault save
                let conn_host = {
                    let state_ref = state.borrow();
                    state_ref
                        .get_connection(connection_id)
                        .map(|c| c.host.clone())
                        .unwrap_or_default()
                };

                if let Ok(state_ref) = state.try_borrow() {
                    let settings = state_ref.settings().clone();
                    let groups: Vec<_> = state_ref.list_groups().into_iter().cloned().collect();
                    let conn = state_ref.get_connection(connection_id);
                    let protocol = rustconn_core::models::ProtocolType::Rdp;

                    crate::state::save_password_to_vault(
                        &settings,
                        &groups,
                        conn,
                        &conn_name,
                        &conn_host,
                        protocol,
                        &creds.username,
                        &creds.password,
                        connection_id,
                    );
                }

                // Also cache for immediate use
                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    state_mut.cache_credentials(
                        connection_id,
                        &creds.username,
                        creds.password.expose_secret(),
                        &creds.domain,
                    );
                }
            }

            // Start RDP with credentials
            start_rdp_session_with_credentials(
                &state,
                &notebook,
                &split_view,
                &sidebar_clone,
                connection_id,
                &creds.username,
                creds.password.expose_secret(),
                &creds.domain,
            );
        }
    });
}

/// Starts RDP session with provided credentials
#[expect(
    clippy::too_many_arguments,
    reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
)]
pub fn start_rdp_session_with_credentials(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    split_view: &SharedSplitView,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    username: &str,
    password: &str,
    domain: &str,
) {
    // Port check is now done earlier in handle_rdp_credentials
    start_rdp_session_internal(
        state,
        notebook,
        split_view,
        sidebar,
        connection_id,
        username,
        password,
        domain,
    );
}

/// Internal function to start RDP session (after port check)
#[expect(
    clippy::too_many_arguments,
    reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
)]
fn start_rdp_session_internal(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    split_view: &SharedSplitView,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    username: &str,
    password: &str,
    domain: &str,
) {
    use rustconn_core::models::RdpClientMode;
    use rustconn_core::variables::{VariableManager, VariableScope};

    let state_ref = state.borrow();

    let Some(conn) = state_ref.get_connection(connection_id) else {
        return;
    };

    let conn_name = conn.name.clone();
    let port = conn.port;
    let window_mode = conn.window_mode;

    // Get global variables for substitution (secret values resolved from vault)
    let global_variables = crate::state::resolve_global_variables(state_ref.settings());

    // Helper function to substitute variables
    let substitute = |input: &str| -> String {
        if !input.contains("${") {
            return input.to_string();
        }
        let mut manager = VariableManager::new();
        for var in &global_variables {
            manager.set_global(var.clone());
        }
        manager
            .substitute_for_command(input, VariableScope::Global)
            .unwrap_or_else(|_| input.to_string())
    };

    // Apply variable substitution to host and username
    let host = substitute(&conn.host);
    let username = substitute(username);

    // Get RDP-specific options
    let rdp_config = if let rustconn_core::ProtocolConfig::Rdp(config) = &conn.protocol_config {
        config.clone()
    } else {
        rustconn_core::models::RdpConfig::default()
    };

    // --- SSH tunnel for jump host ---
    let (effective_host, effective_port, ssh_tunnel) = if let Some(jump_id) =
        rdp_config.jump_host_id
    {
        if let Some(jump_conn) = state_ref.get_connection(jump_id) {
            let mut jump_dest = jump_conn.host.clone();
            if let Some(user) = &jump_conn.username {
                jump_dest = format!("{user}@{}", jump_dest);
            }
            let jump_port = jump_conn.port;
            // Resolve key path via inheritance (connection → group → parent group → root)
            let groups: Vec<rustconn_core::models::ConnectionGroup> =
                state_ref.list_groups().into_iter().cloned().collect();
            let identity_file = rustconn_core::connection::ssh_inheritance::resolve_ssh_key_path(
                jump_conn, &groups,
            )
            .and_then(|p| rustconn_core::resolve_key_path(&p))
            .map(|p| p.to_string_lossy().to_string());

            // Resolve recursive jump host chain (e.g. jump_conn itself needs a jump host)
            let extra_args = super::protocols::resolve_jump_chain_for_tunnel(&state_ref, jump_conn);

            let params = rustconn_core::ssh_tunnel::SshTunnelParams {
                jump_host: jump_dest,
                jump_port,
                remote_host: host.clone(),
                remote_port: port,
                identity_file,
                password: state_ref
                    .get_cached_credentials(jump_id)
                    .filter(|c| {
                        use secrecy::ExposeSecret;
                        !c.password.expose_secret().is_empty()
                    })
                    .map(|c| c.password.clone()),
                extra_args,
            };

            // Clone connection for history before dropping state borrow
            let conn_for_history = conn.clone();
            drop(state_ref);

            match rustconn_core::ssh_tunnel::create_tunnel(&params) {
                Ok(mut tunnel) => {
                    let local_port = tunnel.local_port();
                    tracing::info!(
                        %connection_id,
                        local_port,
                        "SSH tunnel established for RDP connection"
                    );
                    // Wait for tunnel to accept connections
                    if let Err(e) = rustconn_core::ssh_tunnel::wait_for_tunnel_ready(
                        &mut tunnel,
                        40,
                        std::time::Duration::from_millis(250),
                    ) {
                        tracing::error!(%e, "SSH tunnel not ready for RDP");
                        sidebar.update_connection_status(&connection_id.to_string(), "failed");
                        return;
                    }

                    // Verify remote RDP port is reachable through the tunnel
                    if let Err(e) = rustconn_core::ssh_tunnel::probe_tunnel_remote(
                        &mut tunnel,
                        std::time::Duration::from_secs(5),
                    ) {
                        tracing::error!(%e, "Remote RDP port unreachable through SSH tunnel");
                        sidebar.update_connection_status(&connection_id.to_string(), "failed");
                        return;
                    }

                    // Record connection start in history
                    let history_entry_id = if let Ok(mut state_mut) = state.try_borrow_mut() {
                        Some(state_mut.record_connection_start(&conn_for_history, Some(&username)))
                    } else {
                        None
                    };

                    // Dispatch to embedded or external
                    if rdp_config.client_mode == RdpClientMode::Embedded {
                        start_embedded_rdp_session(
                            state,
                            notebook,
                            split_view,
                            sidebar,
                            connection_id,
                            &conn_name,
                            "127.0.0.1",
                            local_port,
                            &username,
                            password,
                            domain,
                            window_mode,
                            &rdp_config,
                            history_entry_id,
                            Some(tunnel),
                        );
                    } else {
                        start_external_rdp_session(
                            state,
                            notebook,
                            split_view,
                            sidebar,
                            connection_id,
                            &conn_name,
                            "127.0.0.1",
                            local_port,
                            &username,
                            password,
                            domain,
                            &rdp_config,
                            history_entry_id,
                            Some(tunnel),
                        );
                    }
                    return;
                }
                Err(e) => {
                    tracing::error!(%connection_id, %e, "Failed to create SSH tunnel for RDP");
                    crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n_f(
                        "SSH tunnel failed: {}",
                        &[&e.to_string()],
                    ));
                    return;
                }
            }
        }
        tracing::warn!(%connection_id, %jump_id, "Jump host connection not found");
        (host.clone(), port, None)
    } else {
        (host.clone(), port, None)
    };

    // Clone connection for history recording (no-tunnel path)
    let conn_for_history = conn.clone();

    drop(state_ref);

    // Record connection start in history
    let history_entry_id = if let Ok(mut state_mut) = state.try_borrow_mut() {
        Some(state_mut.record_connection_start(&conn_for_history, Some(&username)))
    } else {
        None
    };

    // Check client mode - if Embedded, use EmbeddedRdpWidget with fallback to external
    if rdp_config.client_mode == RdpClientMode::Embedded {
        start_embedded_rdp_session(
            state,
            notebook,
            split_view,
            sidebar,
            connection_id,
            &conn_name,
            &effective_host,
            effective_port,
            &username,
            password,
            domain,
            window_mode,
            &rdp_config,
            history_entry_id,
            ssh_tunnel,
        );
        return;
    }

    // External mode - use xfreerdp in external window
    start_external_rdp_session(
        state,
        notebook,
        split_view,
        sidebar,
        connection_id,
        &conn_name,
        &effective_host,
        effective_port,
        &username,
        password,
        domain,
        &rdp_config,
        history_entry_id,
        ssh_tunnel,
    );
}

/// Starts embedded RDP session
#[expect(
    clippy::too_many_arguments,
    reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
)]
fn start_embedded_rdp_session(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    split_view: &SharedSplitView,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn_name: &str,
    host: &str,
    port: u16,
    username: &str,
    password: &str,
    domain: &str,
    window_mode: rustconn_core::models::WindowMode,
    rdp_config: &rustconn_core::models::RdpConfig,
    history_entry_id: Option<Uuid>,
    ssh_tunnel: Option<rustconn_core::ssh_tunnel::SshTunnel>,
) {
    use crate::embedded_rdp::{EmbeddedRdpWidget, RdpConfig as EmbeddedRdpConfig};
    use gtk4::glib;

    // Create embedded RDP widget
    let embedded_widget = EmbeddedRdpWidget::new();

    // Populate scripts menu with user-defined Windows-compatible snippets
    {
        let state_ref = state.borrow();
        let user_snippets = state_ref.list_snippets();
        embedded_widget.update_scripts_menu(&user_snippets);
    }

    // We'll connect after the widget is realized to get actual size
    // For now, create config with placeholder resolution
    let mut embedded_config = EmbeddedRdpConfig::new(host)
        .with_port(port)
        .with_resolution(1280, 720) // Placeholder, will be updated
        .with_clipboard(true)
        .with_performance_mode(rdp_config.performance_mode);

    if !username.is_empty() {
        embedded_config = embedded_config.with_username(username);
    }
    if !password.is_empty() {
        embedded_config = embedded_config.with_password(password);
    }
    if !domain.is_empty() {
        embedded_config = embedded_config.with_domain(domain);
    }

    // Add extra args
    if !rdp_config.custom_args.is_empty() {
        embedded_config = embedded_config.with_extra_args(rdp_config.custom_args.clone());
    }

    // Add shared folders for drive redirection
    if !rdp_config.shared_folders.is_empty() {
        use crate::embedded_rdp::EmbeddedSharedFolder;
        let folders: Vec<EmbeddedSharedFolder> = rdp_config
            .shared_folders
            .iter()
            .map(|f| EmbeddedSharedFolder {
                local_path: f.local_path.clone(),
                share_name: f.share_name.clone(),
            })
            .collect();
        embedded_config = embedded_config.with_shared_folders(folders);
    }

    // Enable printer redirection (maps local CUPS printer into the session)
    embedded_config = embedded_config.with_printer(rdp_config.printer_enabled);

    // Pass keyboard layout override if configured
    embedded_config.keyboard_layout = rdp_config.keyboard_layout;

    // Pass scale override for HiDPI support
    embedded_config.scale_override = rdp_config.scale_override;

    // Pass local cursor visibility preference
    embedded_config.show_local_cursor = rdp_config.show_local_cursor;

    // Pass gateway configuration so IronRDP can detect it and fall back
    // to external xfreerdp (IronRDP 0.14 does not support RD Gateway)
    if let Some(ref gateway) = rdp_config.gateway {
        embedded_config.gateway_hostname = Some(gateway.hostname.clone());
        embedded_config.gateway_port = gateway.port;
        embedded_config.gateway_username = gateway.username.clone();
    }

    // Pass mouse jiggler settings
    embedded_config.jiggler_enabled = rdp_config.jiggler_enabled;
    embedded_config.jiggler_interval_secs = rdp_config.jiggler_interval_secs;

    // Pass autotype timing settings
    embedded_config.autotype_delay_ms = rdp_config.autotype_delay_ms;
    embedded_config.autotype_initial_delay_ms = rdp_config.autotype_initial_delay_ms;
    embedded_config.script_paste_via_clipboard = rdp_config.script_paste_via_clipboard;

    // Pass reconnect-on-resize preference (legacy server compatibility)
    embedded_config.reconnect_on_resize = rdp_config.reconnect_on_resize;

    // Pass RemoteApp configuration (forces FreeRDP fallback — RAIL not supported by IronRDP)
    embedded_config.remote_app_program = rdp_config.remote_app_program.clone();
    embedded_config.remote_app_args = rdp_config.remote_app_args.clone();
    embedded_config.remote_app_name = rdp_config.remote_app_name.clone();

    // Pass graphics pipeline mode (Issue #218 — user can disable GFX per-connection)
    embedded_config.graphics_mode = rdp_config.graphics_mode;

    // Pass certificate verification setting
    embedded_config.ignore_certificate = rdp_config.ignore_certificate;

    // Pass security layer and TLS level for automatic FreeRDP fallback
    embedded_config.security_layer = rdp_config.security_layer;
    embedded_config.tls_security_level = rdp_config.tls_security_level;

    // Add security layer and TLS level to extra_args for FreeRDP fallback
    if let Some(sec_arg) = rdp_config.security_layer.freerdp_arg() {
        embedded_config.extra_args.push(sec_arg.to_string());
    }
    if let Some(level) = rdp_config.tls_security_level {
        embedded_config
            .extra_args
            .push(format!("/tls-seclevel:{level}"));
    }
    if rdp_config.disable_nla {
        embedded_config.extra_args.push("/sec:nla:off".to_string());
    }

    // Wrap in Rc to keep widget alive in notebook
    let embedded_widget = Rc::new(embedded_widget);

    let session_id = Uuid::new_v4();

    // Connect state change callback
    let notebook_for_state = notebook.clone();
    let sidebar_for_state = sidebar.clone();
    let state_for_callback = state.clone();
    let was_ever_connected = Rc::new(std::cell::Cell::new(false));
    let was_connected_clone = was_ever_connected.clone();
    embedded_widget.connect_state_changed(move |rdp_state| match rdp_state {
        crate::embedded_rdp::RdpConnectionState::Disconnected => {
            notebook_for_state.stop_recording(session_id);
            if was_connected_clone.get() {
                // Was connected before — show disconnected tab for reconnect
                notebook_for_state.mark_tab_disconnected(session_id);
                sidebar_for_state.decrement_session_count(&connection_id.to_string(), false);
            }
            // Don't decrement/clear when never connected — the Error handler
            // already set "failed" status and closed the tab.
            // Record connection end in history
            if let Some(info) = notebook_for_state.get_session_info(session_id)
                && let Some(entry_id) = info.history_entry_id
                && let Ok(mut state_mut) = state_for_callback.try_borrow_mut()
            {
                state_mut.record_connection_end(entry_id);
            }
        }
        crate::embedded_rdp::RdpConnectionState::Connected => {
            was_connected_clone.set(true);
            notebook_for_state.mark_tab_connected(session_id);
            sidebar_for_state.increment_session_count(&connection_id.to_string());
        }
        crate::embedded_rdp::RdpConnectionState::Error => {
            // Record connection failure in history
            if let Some(info) = notebook_for_state.get_session_info(session_id)
                && let Some(entry_id) = info.history_entry_id
                && let Ok(mut state_mut) = state_for_callback.try_borrow_mut()
            {
                state_mut.record_connection_failed(entry_id, "RDP connection error");
            }
            // If never connected, close the tab — no point showing failed tab for initial failure
            if !was_connected_clone.get() {
                notebook_for_state.close_tab(session_id);
                // Note: specific error toast is shown by connect_error callback.
                // Only show generic fallback if on_error was not triggered.
            }
            sidebar_for_state.update_connection_status(&connection_id.to_string(), "failed");
        }
        crate::embedded_rdp::RdpConnectionState::Connecting => {}
    });

    // Connect error callback — shows specific error message as toast
    let notebook_for_error = notebook.clone();
    let state_for_error = state.clone();
    embedded_widget.connect_error(move |error_msg| {
        if let Some(window) = notebook_for_error
            .widget()
            .ancestor(gtk4::Window::static_type())
            .and_then(|w| w.downcast::<gtk4::Window>().ok())
        {
            crate::toast::show_toast_on_window(&window, error_msg, crate::toast::ToastType::Error);
        }

        // Persist the real error text in connection history. The Error state
        // handler records a generic "RDP connection error"; here we overwrite
        // it with the specific, user-friendly message so it can be inspected
        // later from the History dialog after the toast has disappeared.
        if let Some(info) = notebook_for_error.get_session_info(session_id)
            && let Some(entry_id) = info.history_entry_id
            && let Ok(mut state_mut) = state_for_error.try_borrow_mut()
        {
            state_mut.record_connection_failed(entry_id, error_msg);
        }
    });

    // Connect reconnect callback
    let widget_for_reconnect = embedded_widget.clone();
    embedded_widget.connect_reconnect(move || {
        if let Err(e) = widget_for_reconnect.reconnect() {
            tracing::error!(%e, "RDP reconnect failed");
        }
    });

    // Connect fallback callback — shows toast when IronRDP falls back to FreeRDP
    // (e.g. xrdp protocol incompatibility — IronRDP issue #139)
    let notebook_for_fallback = notebook.clone();
    embedded_widget.connect_fallback(move |reason| {
        tracing::warn!(protocol = "rdp", reason = %reason, "RDP fallback triggered");
        if let Some(window) = notebook_for_fallback
            .widget()
            .ancestor(gtk4::Window::static_type())
            .and_then(|w| w.downcast::<gtk4::Window>().ok())
        {
            crate::toast::show_toast_on_window(&window, reason, crate::toast::ToastType::Warning);
        }
    });

    // Add tab first, then connect after widget is realized
    notebook.add_embedded_rdp_tab(
        session_id,
        connection_id,
        conn_name,
        embedded_widget.clone(),
    );

    // Store SSH tunnel so it stays alive for the duration of the session
    if let Some(tunnel) = ssh_tunnel {
        notebook.store_ssh_tunnel(session_id, tunnel);
    }

    // Store history entry ID in session for later use
    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Show notebook for RDP session tab
    split_view.widget().set_visible(false);
    split_view.widget().set_vexpand(false);
    notebook.widget().set_vexpand(true);
    notebook.show_tab_view_content();

    // If Fullscreen mode, maximize the window
    if matches!(window_mode, rustconn_core::models::WindowMode::Fullscreen)
        && let Some(window) = notebook
            .widget()
            .ancestor(gtk4::ApplicationWindow::static_type())
        && let Some(app_window) = window.downcast_ref::<gtk4::ApplicationWindow>()
    {
        app_window.maximize();
    }

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }

    // Connect after a short delay to let GTK layout the widget
    // This ensures we get the actual widget size for RDP resolution
    let widget_for_connect = embedded_widget.clone();
    let sidebar_for_connect = sidebar.clone();
    let conn_name_owned = conn_name.to_string();

    // Show the toolbar BEFORE measuring — its height must be accounted for in
    // the initial resolution request. Otherwise the server allocates a desktop
    // that is ~46px taller than the drawing area, requiring an immediate snap
    // resize after connection (causing 3–4 redundant repaints and softened image).
    widget_for_connect.show_toolbar();

    glib::timeout_add_local_once(std::time::Duration::from_millis(100), move || {
        // Get actual widget size from drawing area
        let drawing_area = widget_for_connect.drawing_area();
        let raw_width = drawing_area.width().unsigned_abs();
        let raw_height = drawing_area.height().unsigned_abs();

        // Round down to multiple of 4 for RDP compatibility
        // Many RDP servers and codecs require dimensions divisible by 4
        let actual_width = ((raw_width / 4) * 4).max(640);
        let actual_height = ((raw_height / 4) * 4).max(480);

        tracing::info!(
            "[RDP Init] Actual widget size after layout: {}x{} (raw: {}x{})",
            actual_width,
            actual_height,
            raw_width,
            raw_height
        );

        // Update config with actual resolution
        let final_config = embedded_config.with_resolution(actual_width, actual_height);

        // Now connect with correct resolution
        if let Err(e) = widget_for_connect.connect(&final_config) {
            tracing::error!(%e, connection = %conn_name_owned, "RDP connection failed");
            sidebar_for_connect.update_connection_status(&connection_id.to_string(), "failed");
        } else {
            sidebar_for_connect.update_connection_status(&connection_id.to_string(), "connecting");
        }
    });
}

/// Starts external RDP session using xfreerdp
#[expect(
    clippy::too_many_arguments,
    reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
)]
fn start_external_rdp_session(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    _split_view: &SharedSplitView,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn_name: &str,
    host: &str,
    port: u16,
    username: &str,
    password: &str,
    domain: &str,
    rdp_config: &rustconn_core::models::RdpConfig,
    history_entry_id: Option<Uuid>,
    ssh_tunnel: Option<rustconn_core::ssh_tunnel::SshTunnel>,
) {
    // Issue #209: an external xfreerdp session gets no notebook tab. The tab is
    // still constructed because `RdpLauncher::start_with_geometry` builds its
    // args and spawns through it, but it is never added to the notebook; the
    // spawned child is handed to the shared registry instead of a per-tab timer.
    let (tab, _is_embedded) = EmbeddedSessionTab::new(connection_id, conn_name, "rdp", true);
    let session_id = tab.id();

    // Get resolution from RDP config
    let resolution = rdp_config.resolution.as_ref().map(|r| (r.width, r.height));

    // Get extra args from RDP config
    let mut extra_args = rdp_config.custom_args.clone();

    // Add security layer flag if not default (Negotiate)
    if let Some(sec_arg) = rdp_config.security_layer.freerdp_arg() {
        extra_args.push(sec_arg.to_string());
    }

    // Add TLS security level for legacy server compatibility
    if let Some(level) = rdp_config.tls_security_level {
        extra_args.push(format!("/tls-seclevel:{level}"));
    }

    // Add NLA disable flag if configured
    // FreeRDP 3.x syntax: /sec:nla:off (disables NLA while keeping other methods)
    if rdp_config.disable_nla {
        extra_args.push("/sec:nla:off".to_string());
    }

    // Add RemoteApp arguments for launching individual applications
    extra_args.extend(rdp_config.remote_app_freerdp_args());

    // Prepare domain (use None if empty)
    let domain_opt = if domain.is_empty() {
        None
    } else {
        Some(domain)
    };

    // Convert shared folders
    let shared_folders: Vec<(String, std::path::PathBuf)> = rdp_config
        .shared_folders
        .iter()
        .map(|f| (f.share_name.clone(), f.local_path.clone()))
        .collect();

    // Early-failure callback. With no tab, this rarely fires: the spawned child
    // is handed to the registry synchronously after `start_with_geometry`
    // returns, so the tab-based watcher usually finds an empty handle on its
    // first tick. Kept for the spawn/first-tick race — it reports the error and
    // records the failure (the registry has not been given the child yet).
    let on_early_failure = {
        let state = state.clone();
        let conn_name = conn_name.to_string();
        move |error: String| {
            tracing::error!(%error, connection = %conn_name, "RDP session failed shortly after start");
            crate::toast::show_error_toast_on_active_window(&error);
            if let Some(entry_id) = history_entry_id
                && let Ok(mut state_mut) = state.try_borrow_mut()
            {
                state_mut.record_connection_failed(entry_id, &error);
            }
        }
    };

    // Start RDP connection using xfreerdp. Spawn errors are returned
    // synchronously (R1.6: no tab + error toast); on success the spawned child
    // is moved into the shared registry.
    if let Err(e) = RdpLauncher::start_with_geometry(
        &tab,
        host,
        port,
        Some(username),
        Some(password),
        domain_opt,
        resolution,
        &extra_args,
        None,
        false,
        &shared_folders,
        rdp_config.printer_enabled,
        rdp_config.ignore_certificate,
        on_early_failure,
    ) {
        tracing::error!(%e, connection = %conn_name, "Failed to start RDP session");
        sidebar.update_connection_status(&connection_id.to_string(), "failed");
        crate::toast::show_error_toast_on_active_window(&e.to_string());
        if let Some(entry_id) = history_entry_id
            && let Ok(mut state_mut) = state.try_borrow_mut()
        {
            state_mut.record_connection_failed(entry_id, &e.to_string());
        }
        return;
    }

    // Take ownership of the spawned child from the tab handle and hand it
    // to the shared registry (issue #209): no notebook tab, the shared
    // poll timer reaps it and records the end when the window closes. The
    // registry's on_registered callback drives the sidebar session count
    // (R2.1); record_connection_start was already done by the caller, so
    // its entry id is passed straight through.
    let child = tab.process_handle().borrow_mut().take();
    match super::external_session_registry() {
        Some(registry) => {
            // ponytail: a tunnelled tabless RDP session keeps its
            // SshTunnel in the notebook map keyed by this session id; with
            // no tab-close event it is reclaimed at app exit. Move it into
            // the registry entry if this grows.
            if let Some(tunnel) = ssh_tunnel {
                notebook.store_ssh_tunnel(session_id, tunnel);
            }
            registry.register(session_id, connection_id, child, history_entry_id);
        }
        None => {
            tracing::error!(
                %connection_id,
                "External session registry unavailable; RDP viewer left untracked"
            );
        }
    }

    // Update last_connected
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }
}

/// Starts a VNC connection with password dialog
pub fn start_vnc_with_password_dialog(
    state: SharedAppState,
    notebook: SharedNotebook,
    split_view: SharedSplitView,
    sidebar: SharedSidebar,
    connection_id: Uuid,
    window: &gtk4::Window,
) {
    // Check if we have cached credentials (fast, non-blocking)
    let cached_password = {
        let state_ref = state.borrow();
        state_ref.get_cached_credentials(connection_id).map(|c| {
            use secrecy::ExposeSecret;
            use zeroize::Zeroizing;
            Zeroizing::new(c.password.expose_secret().to_string())
        })
    };

    if let Some(password) = cached_password {
        // Use cached credentials directly
        start_vnc_session_with_password(
            &state,
            &notebook,
            &split_view,
            &sidebar,
            connection_id,
            &password,
        );
        return;
    }

    // Get connection info for dialog
    let (conn_name, lookup_key) = {
        let state_ref = state.borrow();
        if let Some(conn) = state_ref.get_connection(connection_id) {
            // Build hierarchical entry path using KeePassHierarchy
            let groups: Vec<rustconn_core::models::ConnectionGroup> =
                state_ref.list_groups().iter().cloned().cloned().collect();
            let entry_path =
                rustconn_core::secret::KeePassHierarchy::build_entry_path(conn, &groups);

            // Strip RustConn/ prefix since get_password_from_kdbx_with_key adds it back
            let entry_name = entry_path.strip_prefix("RustConn/").unwrap_or(&entry_path);
            let key = format!("{entry_name} (vnc)");

            (conn.name.clone(), key)
        } else {
            return;
        }
    };

    // Create and show password dialog
    let dialog = PasswordDialog::new(Some(window));
    dialog.set_connection_name(&conn_name);

    // Try to load password from KeePass asynchronously
    {
        use crate::utils::spawn_blocking_with_callback;
        let state_ref = state.borrow();
        let settings = state_ref.settings();

        if settings.secrets.kdbx_enabled
            && matches!(
                settings.secrets.preferred_backend,
                rustconn_core::config::SecretBackendType::KeePassXc
                    | rustconn_core::config::SecretBackendType::KdbxFile
            )
            && let Some(kdbx_path) = settings.secrets.kdbx_path.clone()
        {
            let db_password = settings.secrets.kdbx_password.clone();
            let key_file = settings.secrets.kdbx_key_file.clone();

            // Use pre-built lookup key with hierarchical path
            let lookup_key_clone = lookup_key.clone();

            // Get password entry for async callback
            let password_entry = dialog.password_entry().clone();

            // Drop state borrow before spawning
            drop(state_ref);

            // Run KeePass operation in background thread using utility function
            spawn_blocking_with_callback(
                move || {
                    rustconn_core::secret::KeePassStatus::get_password_from_kdbx_with_key(
                        &kdbx_path,
                        db_password.as_ref(),
                        key_file.as_deref(),
                        &lookup_key_clone,
                        None, // Protocol already included in lookup_key
                    )
                },
                move |result: rustconn_core::error::SecretResult<Option<secrecy::SecretString>>| {
                    if let Ok(Some(password)) = result {
                        use secrecy::ExposeSecret;
                        password_entry.set_text(password.expose_secret());
                    }
                    // Silently ignore errors - just continue without pre-fill
                },
            );
        }
    }

    let sidebar_clone = sidebar.clone();
    dialog.show(move |result| {
        let Some(creds) = result else {
            // User cancelled the password dialog — clear "connecting" status
            sidebar_clone.update_connection_status(&connection_id.to_string(), "");
            return;
        };
        {
            // Determine if we should save: explicit request OR password_source == Vault
            let should_save = creds.save_credentials || {
                let state_ref = state.borrow();
                state_ref
                    .get_connection(connection_id)
                    .map(|c| c.password_source == PasswordSource::Vault)
                    .unwrap_or(false)
            };

            if should_save {
                // Get connection details for vault save
                let conn_host = {
                    let state_ref = state.borrow();
                    state_ref
                        .get_connection(connection_id)
                        .map(|c| c.host.clone())
                        .unwrap_or_default()
                };

                if let Ok(state_ref) = state.try_borrow() {
                    let settings = state_ref.settings().clone();
                    let groups: Vec<_> = state_ref.list_groups().into_iter().cloned().collect();
                    let conn = state_ref.get_connection(connection_id);
                    let protocol = rustconn_core::models::ProtocolType::Vnc;

                    crate::state::save_password_to_vault(
                        &settings,
                        &groups,
                        conn,
                        &conn_name,
                        &conn_host,
                        protocol,
                        "", // VNC doesn't use username
                        &creds.password,
                        connection_id,
                    );
                }

                // Also cache for immediate use
                if let Ok(mut state_mut) = state.try_borrow_mut() {
                    state_mut.cache_credentials(
                        connection_id,
                        "",
                        creds.password.expose_secret(),
                        "",
                    );
                }
            }

            // Start VNC with password
            start_vnc_session_with_password(
                &state,
                &notebook,
                &split_view,
                &sidebar_clone,
                connection_id,
                creds.password.expose_secret(),
            );
        }
    });
}

/// Starts VNC session with provided password
pub fn start_vnc_session_with_password(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    split_view: &SharedSplitView,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    password: &str,
) {
    // Port check is now done earlier in handle_vnc_credentials
    start_vnc_session_internal(
        state,
        notebook,
        split_view,
        sidebar,
        connection_id,
        password,
    );
}

/// Internal function to start VNC session (after port check)
fn start_vnc_session_internal(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    split_view: &SharedSplitView,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    password: &str,
) {
    use rustconn_core::models::{VncClientMode, WindowMode};
    use rustconn_core::variables::{VariableManager, VariableScope};

    let state_ref = state.borrow();

    let Some(conn) = state_ref.get_connection(connection_id) else {
        return;
    };

    let conn_name = conn.name.clone();
    let port = conn.port;
    let window_mode = conn.window_mode;

    // Get global variables for substitution (secret values resolved from vault)
    let global_variables = crate::state::resolve_global_variables(state_ref.settings());

    // Apply variable substitution to host
    let host = if conn.host.contains("${") {
        let mut manager = VariableManager::new();
        for var in &global_variables {
            manager.set_global(var.clone());
        }
        manager
            .substitute_for_command(&conn.host, VariableScope::Global)
            .unwrap_or_else(|_| conn.host.clone())
    } else {
        conn.host.clone()
    };

    // Get VNC-specific configuration
    let mut vnc_config = if let rustconn_core::ProtocolConfig::Vnc(config) = &conn.protocol_config {
        config.clone()
    } else {
        rustconn_core::models::VncConfig::default()
    };

    // Apply window_mode: External forces external viewer
    if window_mode == WindowMode::External {
        vnc_config.client_mode = VncClientMode::External;
        tracing::info!(
            protocol = "vnc",
            host = %host,
            "Window mode is External, using external VNC viewer"
        );
    }

    // --- SSH tunnel for jump host ---
    let (effective_host, effective_port, ssh_tunnel) = if let Some(jump_id) =
        vnc_config.jump_host_id
    {
        if let Some(jump_conn) = state_ref.get_connection(jump_id) {
            let mut jump_dest = jump_conn.host.clone();
            if let Some(user) = &jump_conn.username {
                jump_dest = format!("{user}@{}", jump_dest);
            }
            let jump_port = jump_conn.port;
            // Resolve key path via inheritance (connection → group → parent group → root)
            let groups: Vec<rustconn_core::models::ConnectionGroup> =
                state_ref.list_groups().into_iter().cloned().collect();
            let identity_file = rustconn_core::connection::ssh_inheritance::resolve_ssh_key_path(
                jump_conn, &groups,
            )
            .and_then(|p| rustconn_core::resolve_key_path(&p))
            .map(|p| p.to_string_lossy().to_string());

            // Resolve recursive jump host chain (e.g. jump_conn itself needs a jump host)
            let extra_args = super::protocols::resolve_jump_chain_for_tunnel(&state_ref, jump_conn);

            let params = rustconn_core::ssh_tunnel::SshTunnelParams {
                jump_host: jump_dest,
                jump_port,
                remote_host: host.clone(),
                remote_port: port,
                identity_file,
                password: state_ref
                    .get_cached_credentials(jump_id)
                    .filter(|c| {
                        use secrecy::ExposeSecret;
                        !c.password.expose_secret().is_empty()
                    })
                    .map(|c| c.password.clone()),
                extra_args,
            };

            drop(state_ref);

            match rustconn_core::ssh_tunnel::create_tunnel(&params) {
                Ok(mut tunnel) => {
                    let local_port = tunnel.local_port();
                    tracing::info!(
                        %connection_id,
                        local_port,
                        "SSH tunnel established for VNC connection"
                    );
                    // Wait for tunnel to accept connections
                    if let Err(e) = rustconn_core::ssh_tunnel::wait_for_tunnel_ready(
                        &mut tunnel,
                        40,
                        std::time::Duration::from_millis(250),
                    ) {
                        tracing::error!(%e, "SSH tunnel not ready for VNC");
                        sidebar.update_connection_status(&connection_id.to_string(), "failed");
                        return;
                    }

                    // Verify remote VNC port is reachable through the tunnel
                    if let Err(e) = rustconn_core::ssh_tunnel::probe_tunnel_remote(
                        &mut tunnel,
                        std::time::Duration::from_secs(5),
                    ) {
                        tracing::error!(%e, "Remote VNC port unreachable through SSH tunnel");
                        sidebar.update_connection_status(&connection_id.to_string(), "failed");
                        return;
                    }

                    ("127.0.0.1".to_string(), local_port, Some(tunnel))
                }
                Err(e) => {
                    tracing::error!(%e, "Failed to create SSH tunnel for VNC");
                    sidebar.update_connection_status(&connection_id.to_string(), "failed");
                    return;
                }
            }
        } else {
            tracing::warn!(%jump_id, "Jump host connection not found for VNC");
            drop(state_ref);
            (host, port, None)
        }
    } else {
        drop(state_ref);
        (host, port, None)
    };

    // Clone connection for history recording
    let conn_for_history = if let Ok(s) = state.try_borrow() {
        s.get_connection(connection_id).cloned()
    } else {
        None
    };

    // Issue #209: an external-viewer VNC session gets no notebook tab. Spawn the
    // viewer and register it (through the same shared predicate + core builder as
    // the protocols.rs path) so the sidebar surfaces it without a dead tab
    // (R1.1, R1.3). The password is handled by the viewer, never on the argv.
    if let Some(ref conn_hist) = conn_for_history
        && conn_hist.uses_external_viewer()
    {
        let Some(viewer) = crate::session::VncSessionWidget::detect_vnc_viewer() else {
            tracing::error!(connection = %conn_name, "No external VNC viewer installed");
            crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n(
                "No VNC viewer found. Install TigerVNC or Remmina.",
            ));
            sidebar.update_connection_status(&connection_id.to_string(), "failed");
            return;
        };
        let (program, args) = rustconn_core::protocol::VncProtocol::build_external_viewer_command(
            &viewer,
            &effective_host,
            effective_port,
            &vnc_config,
        );
        super::protocols::spawn_and_register_external_viewer(
            state,
            notebook,
            sidebar,
            connection_id,
            conn_hist,
            &program,
            &args,
            ssh_tunnel,
        );
        return;
    }

    // Record connection start in history
    let history_entry_id = if let Some(ref conn_hist) = conn_for_history
        && let Ok(mut state_mut) = state.try_borrow_mut()
    {
        Some(state_mut.record_connection_start(conn_hist, conn_hist.username.as_deref()))
    } else {
        None
    };

    // Create VNC session tab with native widget
    let session_id = notebook.create_vnc_session_tab(connection_id, &conn_name);

    // Store history entry ID in session for later use
    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Store SSH tunnel so it stays alive for the duration of the session
    if let Some(tunnel) = ssh_tunnel {
        notebook.store_ssh_tunnel(session_id, tunnel);
    }

    // Get the VNC widget and initiate connection with config
    if let Some(vnc_widget) = notebook.get_vnc_widget(session_id) {
        // Connect state change callback
        let notebook_for_state = notebook.clone();
        let sidebar_for_state = sidebar.clone();
        let state_for_callback = state.clone();
        vnc_widget.connect_state_changed(move |vnc_state| {
            if vnc_state == crate::session::SessionState::Disconnected {
                notebook_for_state.stop_recording(session_id);
                notebook_for_state.mark_tab_disconnected(session_id);
                sidebar_for_state.decrement_session_count(&connection_id.to_string(), false);
                // Record connection end in history
                if let Some(info) = notebook_for_state.get_session_info(session_id)
                    && let Some(entry_id) = info.history_entry_id
                    && let Ok(mut state_mut) = state_for_callback.try_borrow_mut()
                {
                    state_mut.record_connection_end(entry_id);
                }
            } else if vnc_state == crate::session::SessionState::Connected {
                notebook_for_state.mark_tab_connected(session_id);
                sidebar_for_state.increment_session_count(&connection_id.to_string());
            }
        });

        // Connect reconnect callback
        let widget_for_reconnect = vnc_widget.clone();
        vnc_widget.connect_reconnect(move || {
            if let Err(e) = widget_for_reconnect.reconnect() {
                tracing::error!(%e, "VNC reconnect failed");
            }
        });

        // Initiate connection with VNC config
        if let Err(e) = vnc_widget.connect_with_config(
            &effective_host,
            effective_port,
            Some(password),
            &vnc_config,
        ) {
            tracing::error!(%e, connection = %conn_name, "Failed to connect VNC session");
            sidebar.update_connection_status(&connection_id.to_string(), "failed");
        } else {
            sidebar.update_connection_status(&connection_id.to_string(), "connecting");
        }
    }

    // VNC displays in notebook tab - hide split view and expand notebook
    split_view.widget().set_visible(false);
    split_view.widget().set_vexpand(false);
    notebook.widget().set_vexpand(true);
    notebook.show_tab_view_content();

    // If Fullscreen mode, maximize the window (same pattern as RDP)
    if matches!(window_mode, WindowMode::Fullscreen)
        && let Some(window) = notebook
            .widget()
            .ancestor(gtk4::ApplicationWindow::static_type())
        && let Some(app_window) = window.downcast_ref::<gtk4::ApplicationWindow>()
    {
        app_window.maximize();
    }

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }
}
