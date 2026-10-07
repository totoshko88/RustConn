//! RDP and VNC connection methods for main window
//!
//! This module contains functions for starting RDP and VNC connections
//! with password dialogs and credential handling.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use rustconn_core::models::PasswordSource;
use secrecy::ExposeSecret;
use uuid::Uuid;

use crate::dialogs::PasswordDialog;
use crate::embedded::{EmbeddedSessionTab, RdpLauncher};
use crate::sidebar::ConnectionSidebar;
use crate::split_view::SplitViewBridge;
use crate::state::SharedAppState;
use crate::terminal::TerminalNotebook;

/// Type alias for shared sidebar reference
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Type alias for shared notebook reference
pub type SharedNotebook = Rc<TerminalNotebook>;

/// Type alias for shared split view reference
pub type SharedSplitView = Rc<SplitViewBridge>;

/// Starts an RDP password flow and observes an embedded session it creates.
pub fn start_rdp_with_password_dialog_observed(
    state: SharedAppState,
    notebook: SharedNotebook,
    split_view: SharedSplitView,
    sidebar: SharedSidebar,
    connection_id: Uuid,
    window: &gtk4::Window,
    observer: Option<super::types::SessionStartObserver>,
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
        start_rdp_session_with_credentials_observed(
            &state,
            &notebook,
            &split_view,
            &sidebar,
            connection_id,
            &username,
            &password,
            &domain,
            observer.clone(),
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
                    let groups: Vec<_> = state_ref.list_groups_owned();
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
            start_rdp_session_with_credentials_observed(
                &state,
                &notebook,
                &split_view,
                &sidebar_clone,
                connection_id,
                &creds.username,
                creds.password.expose_secret(),
                &creds.domain,
                observer.clone(),
            );
        }
    });
}

/// Starts RDP and observes the embedded session UUID it creates.
#[expect(
    clippy::too_many_arguments,
    reason = "RDP startup requires four UI owners, connection identity, three credential fields, and observer state"
)]
pub fn start_rdp_session_with_credentials_observed(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    split_view: &SharedSplitView,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    username: &str,
    password: &str,
    domain: &str,
    observer: Option<super::types::SessionStartObserver>,
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
        observer,
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
    observer: Option<super::types::SessionStartObserver>,
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
    // Issue #241: an mDNS `.local` name the sandbox cannot resolve is replaced by
    // the address the Flatpak host resolves it to. No-op outside Flatpak.
    let host = rustconn_core::connection::resolve_sandboxed_hostname(&host)
        .map_or(host, |ip| ip.to_string());
    let username = substitute(username);

    // Get RDP-specific options
    let rdp_config = if let rustconn_core::ProtocolConfig::Rdp(config) = &conn.protocol_config {
        config.clone()
    } else {
        rustconn_core::models::RdpConfig::default()
    };

    // --- SSH tunnel for jump host ---
    //
    // Resolved through the three tiers rather than read off `rdp_config`: a Jump
    // Host set on a group or in Preferences → Network applies to RDP exactly as
    // it does to SSH, which is what 0.20.9 said it had done and had not (#301).
    let (effective_host, effective_port, ssh_tunnel) = if let Some(jump_id) =
        super::protocols::resolve_first_hop_id(&state_ref, conn)
    {
        if let Some(jump_conn) = state_ref.get_connection(jump_id) {
            let mut jump_dest = jump_conn.host.clone();
            if let Some(user) = &jump_conn.username {
                jump_dest = format!("{user}@{}", jump_dest);
            }
            let jump_port = jump_conn.port;
            // Resolve key path via inheritance (connection → group → parent group → root)
            let groups: Vec<rustconn_core::models::ConnectionGroup> = state_ref.list_groups_owned();
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
                            observer.clone(),
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
            observer,
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
    observer: Option<super::types::SessionStartObserver>,
) {
    use gtk4::glib;

    use crate::embedded_rdp::{EmbeddedRdpWidget, RdpConfig as EmbeddedRdpConfig};

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
        .with_clipboard(rdp_config.clipboard_enabled)
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

    // Where the session audio plays. Before 0.19.9 this was collected in the
    // dialog and then never read at connect time, so every session silently
    // ran with audio disabled (issue #245).
    embedded_config = embedded_config.with_audio_mode(rdp_config.effective_audio_mode());

    // Pass keyboard layout override if configured
    embedded_config.keyboard_layout = rdp_config.keyboard_layout;

    // Pass scale override for HiDPI support
    embedded_config.scale_override = rdp_config.scale_override;

    // How an external FreeRDP window is sized if this session ends up handed
    // over — when IronRDP cannot serve the server, or when the connection needs
    // a capability only FreeRDP has. Without these the fallback sized its
    // standalone window from `width`/`height`, which by then hold the embedded
    // viewer's own logical `DrawingArea` geometry: on a 4K display at 200% that
    // is under 1700x1000, so the client opened at roughly a quarter of the
    // screen. The scale percentage is read from the application window rather
    // than the viewer widget because both sit on the same monitor and the
    // external client is placed there too.
    embedded_config.external_display_mode = rdp_config.external_display_mode;
    embedded_config.external_resolution = rdp_config.resolution.clone();
    embedded_config.color_depth = rdp_config.color_depth;
    embedded_config.system_scale_percent = crate::utils::active_display_scale_percent();

    // Pass local cursor visibility preference
    embedded_config.show_local_cursor = rdp_config.show_local_cursor;

    // Pass gateway configuration. The embedded client tunnels through it with
    // MS-TSGU (`rd-gateway` feature); without the feature, or for a target port
    // other than 3389, the connect path hands the session to external FreeRDP.
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

    // Pass external-window sizing preferences (issue #341). Only the external
    // FreeRDP client reads these; smart-sizing wins over dynamic-resolution.
    embedded_config.dynamic_resolution = rdp_config.dynamic_resolution;
    embedded_config.smart_sizing = rdp_config.smart_sizing;

    // Pass the explicit FreeRDP client choice (issue #340). Only the external
    // launch path reads it; auto-detection covers None.
    embedded_config.freerdp_client_override = rdp_config.freerdp_client_override.clone();

    // Pass RemoteApp configuration (forces FreeRDP fallback — RAIL not supported by IronRDP)
    embedded_config.remote_app_program = rdp_config.remote_app_program.clone();
    embedded_config.remote_app_args = rdp_config.remote_app_args.clone();
    embedded_config.remote_app_name = rdp_config.remote_app_name.clone();

    // Pass graphics pipeline mode (Issue #218 — user can disable GFX per-connection)
    embedded_config.graphics_mode = rdp_config.graphics_mode;

    // Pass certificate verification setting
    embedded_config.ignore_certificate = rdp_config.ignore_certificate;

    // Security settings for an automatic FreeRDP fallback. These are fields, not
    // entries in `extra_args`: the shared argument builder turns them into
    // `/sec:`, `/tls-seclevel:` and `/sec:nla:off` itself, so pushing them here
    // as well would send each of them twice. It also stops `extra_args` from
    // looking non-empty for a connection whose only unusual setting is a
    // security layer — which made the embedded viewer warn that the user's
    // custom FreeRDP arguments were being ignored when they had set none.
    embedded_config.security_layer = rdp_config.security_layer;
    embedded_config.tls_security_level = rdp_config.tls_security_level;
    embedded_config.disable_nla = rdp_config.disable_nla;

    // Console/admin session (issue #366). External FreeRDP only — the embedded
    // IronRDP client has no admin flag yet (IronRDP #1629) — so this only takes
    // effect on an external launch or the automatic FreeRDP fallback.
    embedded_config.admin_session = rdp_config.admin_session;

    // FIDO2/WebAuthn device redirection (FreeRDP 3.x only, external mode)
    embedded_config.fido2_enabled = rdp_config.fido2_enabled;

    // Kerberos NLA opt-in (issue #351). Embedded IronRDP path only — what an
    // account in AD "Protected Users" needs, since NTLM is refused for it. There
    // is no NTLM fallback once Kerberos is on. The KDC Address (stored as
    // `kdc_proxy_url`) names a domain controller or an MS-KKDCP proxy; when it
    // is empty the KDC is looked up instead.
    embedded_config.kerberos_enabled = rdp_config.kerberos_enabled;
    embedded_config.kdc_proxy_url = rdp_config.kdc_proxy_url.clone();

    // Carry the per-connection MPTCP opt-in to the embedded client (was dropped
    // here, so the RDP Multipath TCP toggle was a silent no-op).
    embedded_config.mptcp = rdp_config.mptcp;

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
                // The attempt never reached a session, so the credentials it
                // used may be what failed. Drop the cached copy so the next
                // attempt reads the vault again (issue #351). A session that
                // connected and dropped later proved its credentials and keeps
                // them.
                crate::state::forget_cached_credentials(&state_for_callback, connection_id);
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

    // Standard RDP Security is weaker than TLS/NLA. Never retry with it until
    // the user explicitly approves this connection attempt.
    let notebook_for_legacy_security = notebook.clone();
    embedded_widget.connect_legacy_security_required(move |decision| {
        let Some(window) = notebook_for_legacy_security
            .widget()
            .ancestor(gtk4::Window::static_type())
            .and_then(|widget| widget.downcast::<gtk4::Window>().ok())
        else {
            decision(false);
            return;
        };

        let decision = Rc::new(RefCell::new(Some(decision)));
        crate::alert::show_confirm(
            &window,
            &crate::i18n::i18n("Use Legacy RDP Security?"),
            &crate::i18n::i18n(
                "This server only supports Standard RDP Security, which is weaker than TLS or Network Level Authentication. Continue with the external RDP client?",
            ),
            &crate::i18n::i18n("Connect Anyway"),
            false,
            move |accepted| {
                if let Some(decision) = decision.borrow_mut().take() {
                    decision(accepted);
                }
            },
        );
    });

    // Connect certificate-changed callback — shows a confirmation dialog when
    // FreeRDP detects the server certificate has changed since the last connection.
    // On acceptance, removes the old certificate from FreeRDP's TOFU store and
    // reconnects so the new certificate is trusted automatically.
    let notebook_for_cert = notebook.clone();
    let widget_for_cert = embedded_widget.clone();
    embedded_widget.connect_cert_changed(move |host, port, message| {
        let Some(window) = notebook_for_cert
            .widget()
            .ancestor(gtk4::Window::static_type())
            .and_then(|w| w.downcast::<gtk4::Window>().ok())
        else {
            return;
        };

        let host = host.to_owned();
        let widget = widget_for_cert.clone();
        crate::alert::show_confirm(
            &window,
            &crate::i18n::i18n("Certificate changed"),
            message,
            &crate::i18n::i18n("Accept new certificate"),
            false,
            move |accepted| {
                if accepted {
                    // The same dialog serves both the external FreeRDP client and
                    // the embedded IronRDP client, which keep separate TOFU
                    // stores. Forget the host in both so whichever path the
                    // reconnect takes accepts and re-records the new certificate.
                    // Both calls are idempotent no-ops when the host is absent.
                    crate::embedded_rdp::cert::remove_known_certificate(&host, port);
                    #[cfg(feature = "rdp-embedded")]
                    match rustconn_core::rdp_client::tofu::forget(&host, port) {
                        Ok(_) => {}
                        Err(e) => tracing::warn!(
                            %host,
                            port,
                            %e,
                            "Could not forget the stored IronRDP certificate"
                        ),
                    }
                    if let Err(e) = widget.reconnect() {
                        tracing::error!(%e, "RDP reconnect after cert accept failed");
                    }
                } else {
                    // Declining leaves the certificate untrusted. The embedded
                    // IronRDP path keeps its widget in Connecting while the dialog
                    // is open (it must, so an accepted reconnect can reuse the
                    // tab), so an explicit disconnect is needed here or the tab
                    // would sit in Connecting forever. This is a no-op for a
                    // session that already ended, e.g. the FreeRDP path.
                    widget.disconnect();
                }
            },
        );
    });

    // Add tab first, then connect after widget is realized
    notebook.add_embedded_rdp_tab(
        session_id,
        connection_id,
        conn_name,
        embedded_widget.clone(),
    );
    if let Some(observer) = observer {
        observer.complete(session_id);
    }

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

    // A connection can opt out of the floating toolbar entirely (issue #260).
    // Applied before anything reveals it, and before the split view can wrap
    // this session in its own corner buttons.
    widget_for_connect.set_toolbar_enabled(!rdp_config.hide_floating_toolbar);

    // Reveal the toolbar briefly so the user sees the actions that are
    // available while the connection is being established; the auto-hide timer
    // takes over once connected, and this is a no-op when the connection asked
    // for no toolbar.
    //
    // This used to be justified by the measurement below — "its height must be
    // accounted for in the initial resolution request, otherwise the server
    // allocates a desktop ~46px taller than the drawing area". That stopped
    // being true when the toolbar became an overlay child of the viewer
    // (issue #259): it floats over the `DrawingArea` and consumes no vertical
    // space, so revealing it cannot change the size measured 100 ms from now.
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
    // Everything the external client is told now travels as a field of the
    // shared config. The security layer, TLS level and RemoteApp arguments used
    // to be pushed into `extra_args` here; the shared builder emits them from
    // these fields, so doing both would send each of them twice.
    let launch_config = rustconn_core::protocol::FreeRdpConfig {
        host: host.to_string(),
        port,
        username: (!username.is_empty()).then(|| username.to_string()),
        password: (!password.is_empty()).then(|| secrecy::SecretString::from(password.to_string())),
        domain: (!domain.is_empty()).then(|| domain.to_string()),
        display_mode: rdp_config.external_display_mode,
        resolution: rdp_config.resolution.clone(),
        scale_override: rdp_config.scale_override,
        system_scale_percent: crate::utils::active_display_scale_percent(),
        color_depth: rdp_config.color_depth,
        clipboard_enabled: rdp_config.clipboard_enabled,
        shared_folders: rdp_config
            .shared_folders
            .iter()
            .map(|folder| rustconn_core::protocol::freerdp::SharedFolder {
                local_path: folder.local_path.clone(),
                share_name: folder.share_name.clone(),
            })
            .collect(),
        printer_enabled: rdp_config.printer_enabled,
        audio_mode: rdp_config.effective_audio_mode(),
        gateway: rdp_config.gateway.clone(),
        remote_app_program: rdp_config.remote_app_program.clone(),
        remote_app_args: rdp_config.remote_app_args.clone(),
        remote_app_name: rdp_config.remote_app_name.clone(),
        security_layer: rdp_config.security_layer,
        tls_security_level: rdp_config.tls_security_level,
        disable_nla: rdp_config.disable_nla,
        admin_session: rdp_config.admin_session,
        dynamic_resolution: rdp_config.dynamic_resolution,
        smart_sizing: rdp_config.smart_sizing,
        extra_args: rdp_config.custom_args.clone(),
        // Issue #209: a tabless external session has no stored geometry to
        // restore, so the client places its own window.
        window_geometry: None,
        remember_window_position: false,
        ignore_certificate: rdp_config.ignore_certificate,
        fido2_enabled: rdp_config.fido2_enabled,
        client_override: rdp_config.freerdp_client_override.clone(),
        // Lets a RemoteApp session negotiate Kerberos instead of NTLM, which an
        // AD "Protected Users" account cannot use (issue #351), and gates the
        // KDC Address below.
        kerberos_enabled: rdp_config.kerberos_enabled,
        // Reaches FreeRDP as `/kerberos:kdc-url:` when it is an MS-KKDCP proxy;
        // it used to reach only the embedded client.
        kdc_proxy_url: rdp_config.kdc_proxy_url.clone(),
    };

    // A tunnelled session's SshTunnel must outlive every launch attempt: a
    // changed-certificate retry (#324) creates a fresh session and reuses the
    // same local endpoint, so the tunnel is shared across attempts rather than
    // consumed by the first one.
    let shared_tunnel = Rc::new(RefCell::new(ssh_tunnel));

    spawn_external_rdp_attempt(
        state.clone(),
        notebook.clone(),
        sidebar.clone(),
        connection_id,
        conn_name.to_string(),
        launch_config,
        history_entry_id,
        shared_tunnel,
    );
}

/// Spawns one external FreeRDP attempt for a tabless session and wires its three
/// outcomes.
///
/// Issue #209: the tab is constructed only so `RdpLauncher::start` has a process
/// handle to spawn through; it is never added to the notebook. On a surviving
/// connection the child moves to the shared registry, whose poll timer reaps it
/// and records the end when the viewer window closes.
///
/// Issue #324: FreeRDP prints its changed-certificate banner to stdout and then
/// waits on stdin, so it never surfaces as an exit. The launcher's watcher spots
/// the banner and calls back here; this shows a confirmation dialog, and on
/// acceptance forgets the stored certificate and retries — the same trust flow
/// the embedded widget already offers, now available to the external client.
#[expect(
    clippy::too_many_arguments,
    reason = "parameters mirror the launch config and shared state threaded through the certificate-retry loop"
)]
fn spawn_external_rdp_attempt(
    state: SharedAppState,
    notebook: SharedNotebook,
    sidebar: SharedSidebar,
    connection_id: Uuid,
    conn_name: String,
    launch_config: rustconn_core::protocol::FreeRdpConfig,
    history_entry_id: Option<Uuid>,
    shared_tunnel: Rc<RefCell<Option<rustconn_core::ssh_tunnel::SshTunnel>>>,
) {
    let (tab, _is_embedded) = EmbeddedSessionTab::new(connection_id, &conn_name, "rdp", true);
    let session_id = tab.id();

    // Early-failure callback: report the error and record the failed attempt.
    let on_early_failure = {
        let state = state.clone();
        let conn_name = conn_name.clone();
        move |error: String| {
            tracing::error!(%error, connection = %conn_name, "RDP session failed shortly after start");
            // As for the embedded widget: a client that exits right after
            // launch may have been refused, so the retry reads the vault again
            // (issue #351).
            crate::state::forget_cached_credentials(&state, connection_id);
            crate::toast::show_error_toast_on_active_window(&error);
            if let Some(entry_id) = history_entry_id
                && let Ok(mut state_mut) = state.try_borrow_mut()
            {
                state_mut.record_connection_failed(entry_id, &error);
            }
        }
    };

    // Changed-certificate callback: confirm with the user, then forget the old
    // certificate and retry so FreeRDP's TOFU store accepts the new one. (#324)
    let on_cert_changed = {
        let state = state.clone();
        let notebook = notebook.clone();
        let sidebar = sidebar.clone();
        let conn_name = conn_name.clone();
        let launch_config = launch_config.clone();
        let shared_tunnel = shared_tunnel.clone();
        move |host: String, port: u16, message: String| {
            let Some(window) = notebook
                .widget()
                .ancestor(gtk4::Window::static_type())
                .and_then(|w| w.downcast::<gtk4::Window>().ok())
            else {
                tracing::warn!(%host, port, "No window to host the certificate dialog");
                return;
            };
            crate::alert::show_confirm(
                &window,
                &crate::i18n::i18n("Certificate changed"),
                &message,
                &crate::i18n::i18n("Accept new certificate"),
                false,
                move |accepted| {
                    if !accepted {
                        return;
                    }
                    crate::embedded_rdp::cert::remove_known_certificate(&host, port);
                    spawn_external_rdp_attempt(
                        state.clone(),
                        notebook.clone(),
                        sidebar.clone(),
                        connection_id,
                        conn_name.clone(),
                        launch_config.clone(),
                        history_entry_id,
                        shared_tunnel.clone(),
                    );
                },
            );
        }
    };

    // Connected callback: hand the surviving child to the shared registry. The
    // handoff is deferred to this point (issue #209 + #324) so the launcher's
    // watcher owns the process while the certificate decision is still open;
    // the registry's exit-only poll takes over liveness from here.
    let on_connected = {
        let notebook = notebook.clone();
        let process = tab.process_handle();
        let shared_tunnel = shared_tunnel.clone();
        move || {
            let child = process.borrow_mut().take();
            if let Some(registry) = super::external_session_registry() {
                // A tunnelled tabless RDP session keeps its SshTunnel in the
                // notebook map keyed by this session id; with no tab-close event
                // it is reclaimed at app exit.
                if let Some(tunnel) = shared_tunnel.borrow_mut().take() {
                    notebook.store_ssh_tunnel(session_id, tunnel);
                }
                registry.register(session_id, connection_id, child, history_entry_id);
            } else {
                tracing::error!(
                    %connection_id,
                    "External session registry unavailable; terminating untracked RDP viewer"
                );
                if let Some(child) = child {
                    crate::embedded_rdp::launcher::cleanup_child_without_blocking(child);
                }
            }
        }
    };

    // Start RDP connection using xfreerdp. Spawn errors are returned
    // synchronously (R1.6: no tab + error toast).
    let callbacks = crate::embedded::RdpLaunchCallbacks {
        on_early_failure: Box::new(on_early_failure),
        on_cert_changed: Box::new(on_cert_changed),
        on_connected: Box::new(on_connected),
    };
    if let Err(e) = RdpLauncher::start(&tab, &launch_config, callbacks) {
        tracing::error!(%e, connection = %conn_name, "Failed to start RDP session");
        sidebar.update_connection_status(&connection_id.to_string(), "failed");
        // No FreeRDP 3 client: name the FreeRDP 2 that was refused, once, then
        // say what to install (issue #351).
        let message = match &e {
            crate::embedded::EmbeddingError::NoSupportedFreeRdp(unsupported) => {
                crate::embedded_rdp::connection::warn_unsupported_freerdp(*unsupported);
                crate::embedded_rdp::connection::no_supported_freerdp_message()
            }
            other => other.to_string(),
        };
        crate::toast::show_error_toast_on_active_window(&message);
        if let Some(entry_id) = history_entry_id
            && let Ok(mut state_mut) = state.try_borrow_mut()
        {
            state_mut.record_connection_failed(entry_id, &message);
        }
        return;
    }

    // Update last_connected
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }
}

/// Starts a VNC password flow and observes an embedded session it creates.
pub fn start_vnc_with_password_dialog_observed(
    state: SharedAppState,
    notebook: SharedNotebook,
    split_view: SharedSplitView,
    sidebar: SharedSidebar,
    connection_id: Uuid,
    window: &gtk4::Window,
    observer: Option<super::types::SessionStartObserver>,
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
        start_vnc_session_with_password_observed(
            &state,
            &notebook,
            &split_view,
            &sidebar,
            connection_id,
            &password,
            observer.clone(),
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
            let yubikey_slot = settings.secrets.kdbx_yubikey_slot.clone();

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
                        yubikey_slot.as_deref(),
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
                    let groups: Vec<_> = state_ref.list_groups_owned();
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
            start_vnc_session_with_password_observed(
                &state,
                &notebook,
                &split_view,
                &sidebar_clone,
                connection_id,
                creds.password.expose_secret(),
                observer.clone(),
            );
        }
    });
}

/// Starts VNC and observes the embedded session UUID it creates.
pub fn start_vnc_session_with_password_observed(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    split_view: &SharedSplitView,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    password: &str,
    observer: Option<super::types::SessionStartObserver>,
) {
    // Port check is now done earlier in handle_vnc_credentials
    start_vnc_session_internal(
        state,
        notebook,
        split_view,
        sidebar,
        connection_id,
        password,
        observer,
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
    observer: Option<super::types::SessionStartObserver>,
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
    // Issue #241: an mDNS `.local` name the sandbox cannot resolve is replaced by
    // the address the Flatpak host resolves it to. No-op outside Flatpak.
    let host = rustconn_core::connection::resolve_sandboxed_hostname(&host)
        .map_or(host, |ip| ip.to_string());

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
    //
    // Three-tier resolution, as for RDP above and for the same reason (#301).
    let (effective_host, effective_port, ssh_tunnel) = if let Some(jump_id) =
        super::protocols::resolve_first_hop_id(&state_ref, conn)
    {
        if let Some(jump_conn) = state_ref.get_connection(jump_id) {
            let mut jump_dest = jump_conn.host.clone();
            if let Some(user) = &jump_conn.username {
                jump_dest = format!("{user}@{}", jump_dest);
            }
            let jump_port = jump_conn.port;
            // Resolve key path via inheritance (connection → group → parent group → root)
            let groups: Vec<rustconn_core::models::ConnectionGroup> = state_ref.list_groups_owned();
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
        let Some(viewer) =
            rustconn_core::protocol::resolve_vnc_viewer(vnc_config.vnc_viewer_override.as_deref())
        else {
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
    if let Some(observer) = observer {
        observer.complete(session_id);
    }

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
        let was_ever_connected = std::cell::Cell::new(false);
        vnc_widget.connect_state_changed(move |vnc_state| {
            if matches!(vnc_state, crate::session::SessionState::Error(_))
                && !was_ever_connected.get()
            {
                // Same rule as RDP: an attempt that never reached a session may
                // have been refused, so the retry reads the vault again
                // instead of the cached password (issue #351).
                crate::state::forget_cached_credentials(&state_for_callback, connection_id);
            }
            if vnc_state == crate::session::SessionState::Connected {
                was_ever_connected.set(true);
            }
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
