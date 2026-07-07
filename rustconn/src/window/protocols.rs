//! Protocol-specific connection handlers for main window
//!
//! This module contains functions for starting connections for different protocols:
//! SSH, VNC, SPICE, Telnet, Serial, Kubernetes, and Zero Trust.

use super::MainWindow;
pub use super::protocols_ssh::{reconnect_ssh_in_place, start_ssh_connection};
use crate::i18n::i18n;
use crate::sidebar::ConnectionSidebar;
use crate::state::SharedAppState;
use crate::terminal::TerminalNotebook;
use crate::utils::spawn_blocking_with_callback;
use gtk4::glib;
use gtk4::prelude::*;
use rustconn_core::connection::automation_inheritance;
use rustconn_core::connection::check_port;
use rustconn_core::connection::ssh_inheritance;
use rustconn_core::models::AutomationConfig;
use rustconn_core::variables::{Variable, VariableManager, VariableScope};
use std::rc::Rc;
use uuid::Uuid;

/// Type alias for shared sidebar reference
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Type alias for shared notebook reference
pub type SharedNotebook = Rc<TerminalNotebook>;

/// Resolves the effective automation config for a connection, inheriting from
/// the group hierarchy if the connection has no own expect rules / post-login scripts.
pub(super) fn resolve_automation_for_connection(
    state: &SharedAppState,
    conn: &rustconn_core::Connection,
) -> AutomationConfig {
    state
        .try_borrow()
        .ok()
        .map(|s| {
            let groups: Vec<_> = s.list_groups().into_iter().cloned().collect();
            automation_inheritance::resolve_automation(conn, &groups)
        })
        .unwrap_or_else(|| conn.automation.clone())
}

/// Substitutes variables in a string using global variables from settings
///
/// Converts `${VAR_NAME}` references to their values from global variables.
/// If a variable is not found, the reference is left unchanged.
pub(super) fn substitute_variables(input: &str, global_variables: &[Variable]) -> String {
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
}

/// SSH failure patterns in terminal output.
///
/// When connecting through a jump host, the terminal cursor may advance
/// past the detection threshold due to jump host banners or SSH error
/// messages, even though the final destination is unreachable. This
/// function checks for known SSH error strings to avoid false positives.
const SSH_FAILURE_PATTERNS: &[&str] = &[
    "Connection timed out",
    "Connection refused",
    "No route to host",
    "Network is unreachable",
    "Host key verification failed",
    "Permission denied",
    "Too many authentication failures",
    "Connection closed by",
    "Connection reset by",
    "ssh: connect to host",
];

/// Returns `true` if the terminal text contains an SSH connection failure pattern
pub(super) fn contains_ssh_failure(text: &str) -> bool {
    let lower = text.to_lowercase();
    SSH_FAILURE_PATTERNS
        .iter()
        .any(|p| lower.contains(&p.to_lowercase()))
}

/// Delegates to [`rustconn_core::ssh_tunnel::append_proxy_command_destination`].
pub(super) fn append_proxy_command_destination(proxy_parts: &mut Vec<String>, jump_host: &str) {
    rustconn_core::ssh_tunnel::append_proxy_command_destination(proxy_parts, jump_host);
}

/// Parses a jump-host string (`[user@]host[:port]`) for tunnel ControlPath.
fn parse_jump_host_for_tunnel_control(jump_host: &str) -> (String, u16) {
    let host_port = jump_host
        .rfind('@')
        .map_or(jump_host, |at| &jump_host[at + 1..]);

    let (host, port_str) = if host_port.starts_with('[') {
        if let Some(end) = host_port.find(']') {
            let after = &host_port[end + 1..];
            if let Some(p) = after.strip_prefix(':') {
                (&host_port[1..end], Some(p))
            } else {
                (&host_port[1..end], None)
            }
        } else {
            (host_port, None)
        }
    } else if let Some(pos) = host_port.rfind(':') {
        let p = &host_port[pos + 1..];
        if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) {
            (&host_port[..pos], Some(p))
        } else {
            (host_port, None)
        }
    } else {
        (host_port, None)
    };

    let port = port_str.and_then(|p| p.parse().ok()).unwrap_or(22);
    (host.to_string(), port)
}

/// Resolves the recursive jump host chain for a given connection and returns
/// extra SSH args (`-J` or `-o ProxyCommand`) needed to reach it.
///
/// This is used by SSH tunnel creation (RDP, VNC, SPICE) where the jump host
/// itself may require another jump host to be reachable.
///
/// Returns a `Vec<String>` of extra args to pass to the SSH tunnel command.
pub fn resolve_jump_chain_for_tunnel(
    state_ref: &crate::state::AppState,
    jump_conn: &rustconn_core::Connection,
) -> Vec<String> {
    let groups: Vec<rustconn_core::ConnectionGroup> =
        state_ref.list_groups().into_iter().cloned().collect();

    // Check if the jump host itself has a jump host (recursive chain)
    let ssh_config = match &jump_conn.protocol_config {
        rustconn_core::ProtocolConfig::Ssh(cfg) => cfg,
        _ => return Vec::new(),
    };

    // Collect the chain of jump hosts above the immediate jump host
    let mut chain: Vec<String> = Vec::new();

    // First check for string-based proxy_jump on the jump host
    if let Some(proxy) =
        rustconn_core::connection::ssh_inheritance::resolve_ssh_proxy_jump(jump_conn, &groups)
    {
        chain.push(proxy);
    }

    // Then resolve reference-based jump hosts recursively
    // Also resolve the identity file for the first hop (needed for ProxyCommand)
    let mut first_hop_identity: Option<String> = None;
    // PKCS#11 provider of the first hop (not inherited via -J — inject into ProxyCommand)
    let mut first_hop_pkcs11: Option<String> = None;
    if let Some(parent_jump_id) = ssh_config.jump_host_id {
        let mut current_id = Some(parent_jump_id);
        let mut visited = std::collections::HashSet::new();
        visited.insert(jump_conn.id); // Avoid self-reference
        let mut is_first = true;

        for _ in 0..10 {
            if let Some(jid) = current_id {
                if visited.contains(&jid) {
                    break;
                }
                visited.insert(jid);

                if let Some(parent_conn) = state_ref.get_connection(jid) {
                    // Resolve identity file for the first hop
                    if is_first {
                        first_hop_identity =
                            rustconn_core::connection::ssh_inheritance::resolve_ssh_key_path(
                                parent_conn,
                                &groups,
                            )
                            .and_then(|p| rustconn_core::resolve_key_path(&p))
                            .map(|p| p.to_string_lossy().to_string());
                        if let rustconn_core::ProtocolConfig::Ssh(parent_cfg) =
                            &parent_conn.protocol_config
                        {
                            first_hop_pkcs11 = parent_cfg
                                .pkcs11_provider
                                .clone()
                                .filter(|p| !p.trim().is_empty());
                        }
                        is_first = false;
                    }

                    // Format: [user@]host[:port] for -J
                    let mut host_str = parent_conn.host.clone();
                    if let Some(user) = &parent_conn.username {
                        host_str = format!("{user}@{host_str}");
                    }
                    if parent_conn.port != 22 {
                        host_str = format!("{host_str}:{}", parent_conn.port);
                    }
                    chain.push(host_str);

                    // Continue up the chain
                    if let rustconn_core::ProtocolConfig::Ssh(parent_cfg) =
                        &parent_conn.protocol_config
                    {
                        if let Some(p) = &parent_cfg.proxy_jump {
                            chain.insert(chain.len() - 1, p.clone());
                        }
                        current_id = parent_cfg.jump_host_id;
                    } else {
                        current_id = None;
                    }
                } else {
                    current_id = None;
                }
            } else {
                break;
            }
        }
    }

    if chain.is_empty() {
        return Vec::new();
    }

    // In Flatpak, use ProxyCommand so the nested SSH inherits known_hosts
    let flatpak_kh = rustconn_core::get_flatpak_known_hosts_path();
    if flatpak_kh.is_some() || first_hop_pkcs11.is_some() {
        // Build ProxyCommand for the first hop in the chain. `-J` does not pass
        // -o/-i to the nested SSH, so Flatpak known_hosts and PKCS#11 tokens must
        // be injected explicitly here.
        let mut proxy_parts = vec!["ssh".to_string(), "-W".to_string(), "%h:%p".to_string()];
        if let Some(ref kh_path) = flatpak_kh {
            proxy_parts.push("-o".to_string());
            proxy_parts.push(format!("UserKnownHostsFile={}", kh_path.display()));
        }

        // Pass identity file for the first hop if available
        if let Some(ref key) = first_hop_identity {
            proxy_parts.push("-i".to_string());
            proxy_parts.push(key.clone());
            proxy_parts.push("-o".to_string());
            proxy_parts.push("IdentitiesOnly=yes".to_string());
        }

        // Pass PKCS#11 provider for the first hop (token also auths the bastion)
        if let Some(ref provider) = first_hop_pkcs11 {
            proxy_parts.push("-o".to_string());
            proxy_parts.push(format!("PKCS11Provider={}", provider.trim()));
        }

        // Reuse the jump host's ControlMaster socket so parallel tunnel
        // establishments share the already-authenticated link. Only set
        // ControlPath — if the master exists, SSH multiplexes; if not, it
        // connects standalone (no auth issues from missing identity).
        {
            let (jh_host, jh_port) = parse_jump_host_for_tunnel_control(&chain[0]);
            let jh_control = rustconn_core::ssh_control_path(&jh_host, jh_port);
            proxy_parts.push("-o".to_string());
            proxy_parts.push(format!("ControlPath={jh_control}"));
        }

        // ponytail: PKCS#11/identity reach only the first hop; deeper hops do
        // not get the bastion's own PKCS#11 token. Fine for the common
        // single-bastion case.
        //
        // Multi-hop: nest a ProxyCommand per remaining hop so EACH inherits the
        // identity file and Flatpak known_hosts. Plain `-J b,c` would drop them
        // and the deeper hops fail in Flatpak (issue #191 follow-up — double jump).
        if chain.len() > 1 {
            let inner_hops: Vec<&str> = chain[1..].iter().map(String::as_str).collect();
            let inner = rustconn_core::ssh_tunnel::build_nested_proxy_command(
                &inner_hops,
                first_hop_identity.as_deref(),
                flatpak_kh.as_deref(),
                false,
            );
            proxy_parts.push("-o".to_string());
            proxy_parts.push(format!(
                "ProxyCommand={}",
                rustconn_core::ssh_tunnel::shell_single_quote(&inner)
            ));
        }

        // Add the first hop destination with proper -p port parsing
        append_proxy_command_destination(&mut proxy_parts, &chain[0]);

        let proxy_cmd = proxy_parts.join(" ");
        tracing::debug!(
            proxy_command = %proxy_cmd,
            "Tunnel: using ProxyCommand for jump host chain (Flatpak known_hosts or PKCS#11)"
        );
        vec!["-o".to_string(), format!("ProxyCommand={proxy_cmd}")]
    } else {
        // Non-Flatpak: use standard -J. RustConn resolves chains target-first,
        // but OpenSSH `-J` visits hops client-first, so reverse them.
        let j_chain = rustconn_core::ssh_tunnel::proxy_jump_arg(&chain.join(","));
        tracing::debug!(
            jump_chain = %j_chain,
            "Tunnel: using -J for jump host chain"
        );
        vec!["-J".to_string(), j_chain]
    }
}

/// Starts an SSH connection
///
///
/// Creates a VNC session tab with native widget and initiates connection.
pub fn start_vnc_connection(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
) -> Option<Uuid> {
    // Check if port check is needed — skip when jump host is configured
    let settings = state.borrow().settings().clone();
    let should_check = conn.should_pre_connect_check(&settings.connection);

    if should_check {
        let host = conn.host.clone();
        let port = conn.port;
        let timeout = settings.connection.port_check_timeout_secs;
        let state_clone = state.clone();
        let notebook_clone = notebook.clone();
        let sidebar_clone = sidebar.clone();
        let conn_clone = conn.clone();

        // Run port check in background thread
        spawn_blocking_with_callback(
            move || check_port(&host, port, timeout),
            move |result| {
                match result {
                    Ok(_) => {
                        // Port is open, proceed with connection
                        start_vnc_connection_internal(
                            &state_clone,
                            &notebook_clone,
                            &sidebar_clone,
                            connection_id,
                            &conn_clone,
                        );
                    }
                    Err(e) => {
                        // Port check failed, show error with retry
                        tracing::warn!("Port check failed for VNC connection: {e}");
                        sidebar_clone
                            .update_connection_status(&connection_id.to_string(), "failed");
                        if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                            state_mut.record_connection_attempt_failed(
                                &conn_clone,
                                conn_clone.username.as_deref(),
                                &e.to_string(),
                            );
                        }
                        if let Some(root) = notebook_clone.widget().root()
                            && let Some(window) = root.downcast_ref::<gtk4::Window>()
                        {
                            crate::toast::show_retry_toast_on_window(
                                window,
                                &e.to_string(),
                                &connection_id.to_string(),
                            );
                        }
                    }
                }
            },
        );
        // Return None since the actual session will be created asynchronously
        None
    } else {
        // Port check disabled, proceed directly
        start_vnc_connection_internal(state, notebook, sidebar, connection_id, conn)
    }
}

/// Internal function to start VNC connection (after port check)
fn start_vnc_connection_internal(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
) -> Option<Uuid> {
    use rustconn_core::models::{VncClientMode, WindowMode};

    let conn_name = conn.name.clone();
    let port = conn.port;
    let window_mode = conn.window_mode;

    // Get global variables for substitution (secret values resolved from vault)
    let global_variables = state
        .try_borrow()
        .ok()
        .map(|s| crate::state::resolve_global_variables(s.settings()))
        .unwrap_or_default();

    // Apply variable substitution to host
    let host = substitute_variables(&conn.host, &global_variables);

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

    // Get password from cached credentials (set by credential resolution flow)
    let password: Option<zeroize::Zeroizing<String>> =
        state.try_borrow().ok().and_then(|state_ref| {
            state_ref.get_cached_credentials(connection_id).map(|c| {
                use secrecy::ExposeSecret;
                tracing::debug!("[VNC] Found cached credentials for connection");
                zeroize::Zeroizing::new(c.password.expose_secret().to_string())
            })
        });

    tracing::debug!(
        "[VNC] Password available: {}",
        if password.is_some() { "yes" } else { "no" }
    );

    // Create VNC session tab with native widget
    let session_id = notebook.create_vnc_session_tab(connection_id, &conn_name);

    // Record connection start in history
    let history_entry_id = if let Ok(mut state_mut) = state.try_borrow_mut() {
        Some(state_mut.record_connection_start(conn, conn.username.as_deref()))
    } else {
        None
    };

    // Store history entry ID in session for later use
    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Get the VNC widget and initiate connection with config
    if let Some(vnc_widget) = notebook.get_vnc_widget(session_id) {
        // Connect state change callback to mark tab as disconnected when session ends
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

        // Initiate connection with VNC config (respects client_mode setting)
        if let Err(e) = vnc_widget.connect_with_config(
            &host,
            port,
            password.as_ref().map(|p| p.as_str()),
            &vnc_config,
        ) {
            tracing::error!(%e, conn_name, "Failed to connect VNC session");
            sidebar.update_connection_status(&connection_id.to_string(), "failed");
        } else {
            sidebar.update_connection_status(&connection_id.to_string(), "connecting");
        }
    }

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

    Some(session_id)
}

/// Starts a SPICE connection
///
/// Creates a SPICE session tab with native widget and initiates connection.
pub fn start_spice_connection(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
) -> Option<Uuid> {
    // Check if port check is needed — centralized probe-bypass logic
    let settings = state.borrow().settings().clone();
    let should_check = conn.should_pre_connect_check(&settings.connection);

    if should_check {
        let host = conn.host.clone();
        let port = conn.port;
        let timeout = settings.connection.port_check_timeout_secs;
        let state_clone = state.clone();
        let notebook_clone = notebook.clone();
        let sidebar_clone = sidebar.clone();
        let conn_clone = conn.clone();

        // Run port check in background thread
        spawn_blocking_with_callback(
            move || check_port(&host, port, timeout),
            move |result| {
                match result {
                    Ok(_) => {
                        // Port is open, proceed with connection
                        start_spice_connection_internal(
                            &state_clone,
                            &notebook_clone,
                            &sidebar_clone,
                            connection_id,
                            &conn_clone,
                        );
                    }
                    Err(e) => {
                        // Port check failed, show error with retry
                        tracing::warn!("Port check failed for SPICE connection: {e}");
                        sidebar_clone
                            .update_connection_status(&connection_id.to_string(), "failed");
                        if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                            state_mut.record_connection_attempt_failed(
                                &conn_clone,
                                conn_clone.username.as_deref(),
                                &e.to_string(),
                            );
                        }
                        if let Some(root) = notebook_clone.widget().root()
                            && let Some(window) = root.downcast_ref::<gtk4::Window>()
                        {
                            crate::toast::show_retry_toast_on_window(
                                window,
                                &e.to_string(),
                                &connection_id.to_string(),
                            );
                        }
                    }
                }
            },
        );
        // Return None since the actual session will be created asynchronously
        None
    } else {
        // Port check disabled, proceed directly
        start_spice_connection_internal(state, notebook, sidebar, connection_id, conn)
    }
}

/// Internal function to start SPICE connection (after port check)
fn start_spice_connection_internal(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
) -> Option<Uuid> {
    let conn_name = conn.name.clone();
    let port = conn.port;

    // Get global variables for substitution (secret values resolved from vault)
    let global_variables = state
        .try_borrow()
        .ok()
        .map(|s| crate::state::resolve_global_variables(s.settings()))
        .unwrap_or_default();

    // Apply variable substitution to host
    let host = substitute_variables(&conn.host, &global_variables);

    // Get SPICE-specific options from connection config
    let spice_opts = if let rustconn_core::ProtocolConfig::Spice(config) = &conn.protocol_config {
        Some(config.clone())
    } else {
        None
    };

    // --- SSH tunnel for jump host ---
    // Unix-socket mode connects locally, so the jump host is ignored (no tunnel).
    let (effective_host, effective_port, ssh_tunnel) = if let Some(ref opts) = spice_opts
        && opts.unix_socket_path.is_none()
        && let Some(jump_id) = opts.jump_host_id
    {
        if let Ok(state_ref) = state.try_borrow()
            && let Some(jump_conn) = state_ref.get_connection(jump_id)
        {
            let mut jump_dest = jump_conn.host.clone();
            if let Some(user) = &jump_conn.username {
                jump_dest = format!("{user}@{}", jump_dest);
            }
            let jump_port = jump_conn.port;
            // Resolve key path via inheritance (connection → group → parent group → root)
            let groups: Vec<rustconn_core::models::ConnectionGroup> =
                state_ref.list_groups().into_iter().cloned().collect();
            let identity_file = ssh_inheritance::resolve_ssh_key_path(jump_conn, &groups)
                .and_then(|p| rustconn_core::resolve_key_path(&p))
                .map(|p| p.to_string_lossy().to_string());

            // Resolve recursive jump host chain (e.g. jump_conn itself needs a jump host)
            let extra_args = resolve_jump_chain_for_tunnel(&state_ref, jump_conn);

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
                        "SSH tunnel established for SPICE connection"
                    );
                    // Wait for tunnel to accept connections
                    if let Err(e) = rustconn_core::ssh_tunnel::wait_for_tunnel_ready(
                        &mut tunnel,
                        40,
                        std::time::Duration::from_millis(250),
                    ) {
                        tracing::error!(%e, "SSH tunnel not ready for SPICE");
                        sidebar.update_connection_status(&connection_id.to_string(), "failed");
                        return None;
                    }

                    // Verify remote SPICE port is reachable through the tunnel
                    if let Err(e) = rustconn_core::ssh_tunnel::probe_tunnel_remote(
                        &mut tunnel,
                        std::time::Duration::from_secs(5),
                    ) {
                        tracing::error!(%e, "Remote SPICE port unreachable through SSH tunnel");
                        sidebar.update_connection_status(&connection_id.to_string(), "failed");
                        return None;
                    }

                    ("127.0.0.1".to_string(), local_port, Some(tunnel))
                }
                Err(e) => {
                    tracing::error!(%e, "Failed to create SSH tunnel for SPICE");
                    sidebar.update_connection_status(&connection_id.to_string(), "failed");
                    return None;
                }
            }
        } else {
            tracing::warn!(%jump_id, "Jump host connection not found for SPICE");
            (host, port, None)
        }
    } else {
        (host, port, None)
    };

    // Create SPICE session tab with native widget
    let session_id = notebook.create_spice_session_tab(connection_id, &conn_name);

    // Record connection start in history
    let history_entry_id = if let Ok(mut state_mut) = state.try_borrow_mut() {
        Some(state_mut.record_connection_start(conn, conn.username.as_deref()))
    } else {
        None
    };

    // Store history entry ID in session for later use
    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Store SSH tunnel so it stays alive for the duration of the session
    if let Some(tunnel) = ssh_tunnel {
        notebook.store_ssh_tunnel(session_id, tunnel);
    }

    // Get the SPICE widget and initiate connection
    if let Some(spice_widget) = notebook.get_spice_widget(session_id) {
        // Build connection config using SpiceClientConfig from spice_client module
        use rustconn_core::spice_client::SpiceClientConfig;
        let mut config = SpiceClientConfig::new(&effective_host).with_port(effective_port);

        // Apply SPICE-specific settings if available
        if let Some(opts) = spice_opts {
            // Configure TLS
            config = config.with_tls(opts.tls_enabled);
            if let Some(ca_path) = &opts.ca_cert_path {
                config = config.with_ca_cert(ca_path);
            }
            config = config.with_skip_cert_verify(opts.skip_cert_verify);

            // Configure USB redirection
            config = config.with_usb_redirection(opts.usb_redirection);

            // Configure clipboard
            config = config.with_clipboard(opts.clipboard_enabled);

            // Configure local cursor visibility
            config.show_local_cursor = opts.show_local_cursor;

            // Configure unix socket if set
            if let Some(ref socket_path) = opts.unix_socket_path {
                config = config.with_unix_socket(socket_path);
            }
        }

        // Connect state change callback to mark tab as disconnected
        let notebook_for_state = notebook.clone();
        let sidebar_for_state = sidebar.clone();
        let state_for_callback = state.clone();
        spice_widget.connect_state_changed(move |spice_state| {
            use crate::embedded_spice::SpiceConnectionState;
            if spice_state == SpiceConnectionState::Disconnected
                || spice_state == SpiceConnectionState::Error
            {
                notebook_for_state.stop_recording(session_id);
                notebook_for_state.mark_tab_disconnected(session_id);
                sidebar_for_state.decrement_session_count(
                    &connection_id.to_string(),
                    spice_state == SpiceConnectionState::Error,
                );
                // Record connection end/failure in history
                if let Some(info) = notebook_for_state.get_session_info(session_id)
                    && let Some(entry_id) = info.history_entry_id
                    && let Ok(mut state_mut) = state_for_callback.try_borrow_mut()
                {
                    if spice_state == SpiceConnectionState::Error {
                        state_mut.record_connection_failed(entry_id, "SPICE connection error");
                    } else {
                        state_mut.record_connection_end(entry_id);
                    }
                }
            } else if spice_state == SpiceConnectionState::Connected {
                notebook_for_state.mark_tab_connected(session_id);
                sidebar_for_state.increment_session_count(&connection_id.to_string());
            }
        });

        // Connect reconnect callback
        let widget_for_reconnect = spice_widget.clone();
        spice_widget.connect_reconnect(move || {
            if let Err(e) = widget_for_reconnect.reconnect() {
                tracing::error!(%e, "SPICE reconnect failed");
            }
        });

        // Initiate connection
        if let Err(e) = spice_widget.connect(&config) {
            tracing::error!(%e, conn_name, "Failed to connect SPICE session");
        }
    }

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }

    Some(session_id)
}

/// Reconnects an SSH session in-place, reusing the existing terminal tab.
///
/// Instead of closing the old tab and creating a new one (which disrupts
/// tab ordering when managing 10+ sessions), this function:
/// 1. Prepares the existing tab (removes banner, resets VTE)
/// 2. Re-applies highlight rules and automation
/// 3. Re-spawns the SSH process in the same terminal
/// 4. Re-wires password injection, status detection, and monitoring
///
/// Returns `true` if reconnect was initiated, `false` if the tab no longer exists.
pub fn reconnect_generic_vte_in_place(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    session_id: Uuid,
    connection_id: Uuid,
) -> bool {
    use rustconn_core::protocol::{
        KubernetesProtocol, MoshProtocol, Protocol, SerialProtocol, format_command_message,
        format_connection_message,
    };

    if !notebook.prepare_for_reconnect(session_id) {
        tracing::warn!(%session_id, "Tab no longer exists, cannot reconnect in-place");
        return false;
    }

    // Show "connecting" status in sidebar immediately
    sidebar.update_connection_status(&connection_id.to_string(), "connecting");

    let conn = {
        let Ok(state_ref) = state.try_borrow() else {
            return false;
        };
        match state_ref.get_connection(connection_id) {
            Some(c) => c.clone(),
            None => return false,
        }
    };

    // Re-apply highlight rules
    {
        let global_rules = state
            .try_borrow()
            .ok()
            .map(|s| s.settings().highlight_rules.clone())
            .unwrap_or_default();
        notebook.set_highlight_rules(session_id, &global_rules, &conn.highlight_rules);
    }

    // Record connection start in history
    if let Ok(mut state_mut) = state.try_borrow_mut() {
        let entry_id = state_mut.record_connection_start(&conn, conn.username.as_deref());
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Re-wire child-exited handler
    MainWindow::setup_child_exited_handler(state, notebook, sidebar, session_id, connection_id);

    // Build and spawn command based on protocol
    match &conn.protocol_config {
        rustconn_core::ProtocolConfig::ZeroTrust(zt_config) => {
            let (program, args) = zt_config.build_command(conn.username.as_deref());
            let provider_name = zt_config.provider.display_name();
            let full_command = std::iter::once(program.as_str())
                .chain(args.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(" ");

            let conn_msg = format_connection_message(provider_name, &conn.name);
            let cmd_msg = format_command_message(&full_command);
            notebook.display_output(session_id, &format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n"));

            let spawn_command = rustconn_core::flatpak::wrap_host_command(&full_command);
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
            notebook.spawn_command(
                session_id,
                &[&shell, "-c", &spawn_command],
                None,
                None,
                None,
            );
        }
        rustconn_core::ProtocolConfig::Telnet(telnet_config) => {
            let conn_msg = format_connection_message("Telnet", &conn.host);
            let cmd_msg = format_command_message(&format!("telnet {} {}", conn.host, conn.port));
            notebook.display_output(session_id, &format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n"));

            notebook.spawn_telnet(
                session_id,
                &conn.host,
                conn.port,
                &[],
                telnet_config.backspace_sends,
                telnet_config.delete_sends,
            );
        }
        rustconn_core::ProtocolConfig::Serial(_) => {
            let serial = SerialProtocol::new();
            let Some(command) = serial.build_command(&conn) else {
                tracing::warn!(%session_id, "Failed to build Serial command for reconnect");
                return false;
            };
            let serial_config =
                if let rustconn_core::ProtocolConfig::Serial(ref cfg) = conn.protocol_config {
                    cfg
                } else {
                    return false;
                };

            let conn_msg = format_connection_message("Serial", &serial_config.device);
            let cmd_msg = format_command_message(&command.join(" "));
            notebook.display_output(session_id, &format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n"));

            notebook.spawn_serial(session_id, &command);
        }
        rustconn_core::ProtocolConfig::Kubernetes(_) => {
            let k8s = KubernetesProtocol::new();
            let Some(command) = k8s.build_command(&conn) else {
                tracing::warn!(%session_id, "Failed to build Kubernetes command for reconnect");
                return false;
            };

            let conn_msg = format_connection_message("Kubernetes", &conn.name);
            let cmd_msg = format_command_message(&command.join(" "));
            notebook.display_output(session_id, &format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n"));

            // Spawn argv directly (no `sh -c`) so namespace/pod/container fields
            // are never shell-interpreted. This prevents command injection from
            // shell metachars in (possibly imported, untrusted) kubectl configs.
            // kubectl runs in-sandbox under Flatpak (Flatpak Components), so the
            // old flatpak-spawn wrapper is no longer needed — same as Mosh below.
            let argv: Vec<&str> = command.iter().map(String::as_str).collect();
            notebook.spawn_command(session_id, &argv, None, None, None);
        }
        rustconn_core::ProtocolConfig::Mosh(_) => {
            let mosh = MoshProtocol::new();
            let Some(command) = mosh.build_command(&conn) else {
                tracing::warn!(%session_id, "Failed to build MOSH command for reconnect");
                return false;
            };

            let conn_msg = format_connection_message("MOSH", &conn.host);
            let cmd_msg = format_command_message(&command.join(" "));
            notebook.display_output(session_id, &format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n"));

            // Mosh uses direct exec (no shell wrapper needed)
            let argv: Vec<&str> = command.iter().map(String::as_str).collect();
            notebook.spawn_command(session_id, &argv, None, None, None);
        }
        _ => {
            tracing::warn!("Unsupported protocol for generic VTE reconnect");
            return false;
        }
    }

    // Update last_connected
    if let Ok(mut state_mut) = state.try_borrow_mut() {
        let _ = state_mut.update_last_connected(connection_id);
    }

    // Status detection: mark connected when cursor advances past initial output
    {
        let sidebar_clone = sidebar.clone();
        let notebook_clone = notebook.clone();
        let connection_id_str = connection_id.to_string();
        let session_connected = std::rc::Rc::new(std::cell::Cell::new(false));
        let session_connected_clone = session_connected.clone();

        notebook.connect_contents_changed(session_id, move || {
            if session_connected_clone.get() {
                return;
            }
            if let Some(row) = notebook_clone.get_terminal_cursor_row(session_id)
                && row > 2
            {
                sidebar_clone.increment_session_count(&connection_id_str);
                session_connected_clone.set(true);
            }
        });
    }

    true
}

/// Starts a Telnet connection
///
/// Creates a terminal tab and spawns the telnet process.
pub fn start_telnet_connection(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    logging_enabled: bool,
) -> Option<Uuid> {
    // Check if port check is needed
    let settings = state.borrow().settings().clone();
    let should_check = conn.should_pre_connect_check(&settings.connection);

    if should_check {
        let host = conn.host.clone();
        let port = conn.port;
        let timeout = settings.connection.port_check_timeout_secs;
        let state_clone = state.clone();
        let notebook_clone = notebook.clone();
        let sidebar_clone = sidebar.clone();
        let conn_clone = conn.clone();

        // Run port check in background thread
        spawn_blocking_with_callback(
            move || check_port(&host, port, timeout),
            move |result| match result {
                Ok(_) => {
                    start_telnet_connection_internal(
                        &state_clone,
                        &notebook_clone,
                        &sidebar_clone,
                        connection_id,
                        &conn_clone,
                        logging_enabled,
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        protocol = "telnet",
                        host = %conn_clone.host,
                        port = conn_clone.port,
                        error = %e,
                        "Port check failed for Telnet connection"
                    );
                    sidebar_clone.update_connection_status(&connection_id.to_string(), "failed");
                    if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                        state_mut.record_connection_attempt_failed(
                            &conn_clone,
                            conn_clone.username.as_deref(),
                            &e.to_string(),
                        );
                    }
                    if let Some(root) = notebook_clone.widget().root()
                        && let Some(window) = root.downcast_ref::<gtk4::Window>()
                    {
                        crate::toast::show_retry_toast_on_window(
                            window,
                            &e.to_string(),
                            &connection_id.to_string(),
                        );
                    }
                }
            },
        );
        None
    } else {
        start_telnet_connection_internal(
            state,
            notebook,
            sidebar,
            connection_id,
            conn,
            logging_enabled,
        )
    }
}

/// Internal function to start Telnet connection (after port check).
///
/// Creates a terminal tab and spawns the telnet process.
fn start_telnet_connection_internal(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    logging_enabled: bool,
) -> Option<Uuid> {
    use rustconn_core::protocol::{format_command_message, format_connection_message};

    let conn_name = conn.name.clone();
    let port = conn.port;

    // Get terminal settings from state
    let terminal_settings = state
        .try_borrow()
        .ok()
        .map(|s| s.settings().terminal.clone())
        .unwrap_or_default();

    // Get global variables for substitution (secret values resolved from vault)
    let global_variables = state
        .try_borrow()
        .ok()
        .map(|s| crate::state::resolve_global_variables(s.settings()))
        .unwrap_or_default();

    // Resolve automation config with group inheritance
    let resolved_automation = resolve_automation_for_connection(state, conn);

    // Create terminal tab for Telnet
    let session_id = notebook.create_terminal_tab_with_settings(
        connection_id,
        &conn.name,
        "telnet",
        Some(&resolved_automation),
        &terminal_settings,
        conn.theme_override.as_ref(),
        &global_variables,
    );

    // Apply highlight rules (built-in defaults + global + per-connection)
    {
        let global_rules = state
            .try_borrow()
            .ok()
            .map(|s| s.settings().highlight_rules.clone())
            .unwrap_or_default();
        notebook.set_highlight_rules(session_id, &global_rules, &conn.highlight_rules);
    }

    // Record connection start in history
    let history_entry_id = if let Ok(mut state_mut) = state.try_borrow_mut() {
        Some(state_mut.record_connection_start(conn, conn.username.as_deref()))
    } else {
        None
    };

    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    let host = substitute_variables(&conn.host, &global_variables);

    // Get custom args and keyboard settings from TelnetConfig
    let (extra_args, backspace_sends, delete_sends) =
        if let rustconn_core::ProtocolConfig::Telnet(ref config) = conn.protocol_config {
            (
                config.custom_args.clone(),
                config.backspace_sends,
                config.delete_sends,
            )
        } else {
            (
                Vec::new(),
                rustconn_core::models::TelnetBackspaceSends::Automatic,
                rustconn_core::models::TelnetDeleteSends::Automatic,
            )
        };

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }

    // Set up session logging if enabled
    if logging_enabled {
        MainWindow::setup_session_logging(state, notebook, session_id, connection_id, &conn_name);
    }

    // Wire up child exited callback
    MainWindow::setup_child_exited_handler(state, notebook, sidebar, session_id, connection_id);

    // Build telnet command string for display
    let mut cmd_parts = vec!["telnet".to_string()];
    cmd_parts.extend(extra_args.clone());
    cmd_parts.push(host.clone());
    cmd_parts.push(port.to_string());
    let telnet_command = cmd_parts.join(" ");

    // Display CLI output feedback
    let conn_msg = format_connection_message("Telnet", &host);
    let cmd_msg = format_command_message(&telnet_command);
    let feedback = format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n");
    notebook.display_output(session_id, &feedback);

    // Spawn telnet
    let extra_refs: Vec<&str> = extra_args.iter().map(String::as_str).collect();
    notebook.spawn_telnet(
        session_id,
        &host,
        port,
        &extra_refs,
        backspace_sends,
        delete_sends,
    );

    // --- Auto-recording for Telnet ---
    if conn.session_recording_enabled {
        let notebook_clone = notebook.clone();
        let recording_conn_name = conn_name.clone();
        let recording_started = std::rc::Rc::new(std::cell::Cell::new(false));
        let recording_started_clone = recording_started.clone();
        let recording_ssh_params = Some(crate::terminal::SshRecordingParams {
            host: host.clone(),
            port,
            username: conn.username.clone(),
            identity_file: None,
        });

        notebook.connect_contents_changed(session_id, move || {
            if recording_started_clone.get() {
                return;
            }
            if let Some(row) = notebook_clone.get_terminal_cursor_row(session_id)
                && row > 0
            {
                recording_started_clone.set(true);
                notebook_clone.start_recording(
                    session_id,
                    &recording_conn_name,
                    rustconn_core::session::SanitizeConfig::default(),
                    recording_ssh_params.clone(),
                );
                tracing::info!(
                    %session_id,
                    "Auto-recording started after Telnet connection"
                );
            }
        });
    }

    Some(session_id)
}

/// Starts a Zero Trust connection
///
/// Creates a terminal tab and spawns the Zero Trust provider command.
pub fn start_zerotrust_connection(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    logging_enabled: bool,
) -> Option<Uuid> {
    use rustconn_core::protocol::{format_command_message, format_connection_message};

    let conn_name = conn.name.clone();
    let username = conn.username.clone();

    // Get Zero Trust config and build command
    let (program, args, provider_name, provider_key) =
        if let rustconn_core::ProtocolConfig::ZeroTrust(zt_config) = &conn.protocol_config {
            // Validate configuration before launch
            if let Err(e) = zt_config.validate() {
                tracing::error!(?e, "ZeroTrust config validation failed for {}", conn_name);
                if let Some(root) = notebook.widget().root()
                    && let Some(window) = root.downcast_ref::<gtk4::Window>()
                {
                    crate::toast::show_toast_on_window(
                        window,
                        &crate::i18n::i18n_f("Invalid config: {}", &[&e.to_string()]),
                        crate::toast::ToastType::Error,
                    );
                }
                return None;
            }

            let (prog, args) = zt_config.build_command(username.as_deref());
            let provider = zt_config.provider.display_name();

            // Check CLI tool availability before launch
            let cli = zt_config.provider.cli_command();
            if !cli.is_empty() && !rustconn_core::flatpak::is_host_command_available(cli) {
                tracing::warn!(
                    provider = %provider,
                    cli,
                    flatpak = rustconn_core::flatpak::is_flatpak(),
                    "ZeroTrust CLI tool not found"
                );
                if let Some(root) = notebook.widget().root()
                    && let Some(window) = root.downcast_ref::<gtk4::Window>()
                {
                    crate::toast::show_missing_cli_toast(
                        window,
                        &format!("{provider} requires '{cli}' CLI tool"),
                    );
                }
                return None;
            }

            tracing::info!(
                provider = %provider,
                cli = %prog,
                connection = %conn_name,
                "Launching ZeroTrust connection"
            );

            // Get provider key for icon matching
            let key = match zt_config.provider {
                rustconn_core::models::ZeroTrustProvider::AwsSsm => "aws",
                rustconn_core::models::ZeroTrustProvider::GcpIap => "gcloud",
                rustconn_core::models::ZeroTrustProvider::AzureBastion => "azure",
                rustconn_core::models::ZeroTrustProvider::AzureSsh => "azure_ssh",
                rustconn_core::models::ZeroTrustProvider::OciBastion => "oci",
                rustconn_core::models::ZeroTrustProvider::CloudflareAccess => "cloudflare",
                rustconn_core::models::ZeroTrustProvider::Teleport => "teleport",
                rustconn_core::models::ZeroTrustProvider::TailscaleSsh => "tailscale",
                rustconn_core::models::ZeroTrustProvider::Boundary => "boundary",
                rustconn_core::models::ZeroTrustProvider::HoopDev => "hoop",
                rustconn_core::models::ZeroTrustProvider::Generic => "generic",
            };
            (prog, args, provider, key)
        } else {
            return None;
        };

    let automation_config = resolve_automation_for_connection(state, conn);

    // Get terminal settings from state
    let terminal_settings = state
        .try_borrow()
        .ok()
        .map(|s| s.settings().terminal.clone())
        .unwrap_or_default();

    // Get global variables for substitution in Expect responses
    let global_variables = state
        .try_borrow()
        .ok()
        .map(|s| crate::state::resolve_global_variables(s.settings()))
        .unwrap_or_default();

    // Create terminal tab for Zero Trust with provider-specific protocol
    let tab_protocol = format!("zerotrust:{provider_key}");
    let session_id = notebook.create_terminal_tab_with_settings(
        connection_id,
        &conn_name,
        &tab_protocol,
        Some(&automation_config),
        &terminal_settings,
        conn.theme_override.as_ref(),
        &global_variables,
    );

    // Record connection start in history
    let history_entry_id = if let Ok(mut state_mut) = state.try_borrow_mut() {
        Some(state_mut.record_connection_start(conn, conn.username.as_deref()))
    } else {
        None
    };

    // Store history entry ID in session for later use
    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }

    // Set up session logging if enabled
    if logging_enabled {
        MainWindow::setup_session_logging(state, notebook, session_id, connection_id, &conn_name);
    }

    // Wire up child exited callback for session cleanup
    MainWindow::setup_child_exited_handler(state, notebook, sidebar, session_id, connection_id);

    // Build the full command string for display
    let full_command = std::iter::once(program.as_str())
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ");

    // Display CLI output feedback before executing command
    let conn_msg = format_connection_message(provider_name, &conn_name);
    let cmd_msg = format_command_message(&full_command);
    let feedback = format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n");
    notebook.display_output(session_id, &feedback);

    // Spawn the Zero Trust command through shell
    //
    // For Generic provider in Flatpak: the user's custom command likely refers
    // to host-side binaries (not installed in sandbox via Flatpak Components).
    // Wrap with flatpak-spawn --host + script (for PTY) — same approach as
    // Local Shell (#122). Other providers have their CLIs installed in-sandbox.
    let is_generic = matches!(
        &conn.protocol_config,
        rustconn_core::ProtocolConfig::ZeroTrust(zt)
            if matches!(zt.provider, rustconn_core::models::ZeroTrustProvider::Generic)
    );

    if is_generic && rustconn_core::flatpak::is_flatpak() {
        // Generic command_template is already a shell command string.
        // Extract it from the full_command which is "sh -c <template>".
        // We need just the template part for flatpak-spawn.
        let template = full_command.strip_prefix("sh -c ").unwrap_or(&full_command);
        // Escape single quotes for safe embedding in '...' shell string:
        // replace ' with '\'' (end quote, escaped quote, start quote)
        let escaped = template.replace('\'', "'\\''");
        // Run on the host through a *login* shell (`sh -lc`) so the host PATH is
        // used to resolve binaries. `script` (util-linux) allocates a host PTY
        // for interactive TUI tools, but it is not present on every host
        // (atomic distros, or `script` outside the sandbox PATH) and GUI tools
        // (e.g. WinBox, #190) do not need a PTY at all. So we probe the host for
        // `script` and fall back to a plain `sh -c` when it is missing — this
        // fixes both the "Failed to start command: script" portal error and
        // launching GUI programs from a Generic command.
        // ponytail: single login-shell probe per launch; fine for one-shot
        // command spawn (not in a hot path).
        let host_runner = "if command -v script >/dev/null 2>&1; then exec script -qfc \"$1\" /dev/null; else exec sh -c \"$1\"; fi";
        let spawn_cmd = format!(
            "flatpak-spawn --host --env=TERM=xterm-256color -- sh -lc '{host_runner}' rustconn '{escaped}'"
        );
        notebook.spawn_command(session_id, &["/bin/sh", "-c", &spawn_cmd], None, None, None);
    } else {
        let spawn_command = rustconn_core::flatpak::wrap_host_command(&full_command);
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        notebook.spawn_command(
            session_id,
            &[&shell, "-c", &spawn_command],
            None,
            None,
            None,
        );
    }

    Some(session_id)
}

/// Starts a Serial connection
///
/// Creates a terminal tab and spawns picocom with the serial configuration.
/// Shows user-friendly toasts when picocom is not found or device access fails.
pub fn start_serial_connection(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    logging_enabled: bool,
) -> Option<Uuid> {
    use rustconn_core::protocol::{
        Protocol, SerialProtocol, detect_picocom, format_command_message, format_connection_message,
    };

    let conn_name = conn.name.clone();

    // Check picocom availability before attempting to launch
    let picocom_info = detect_picocom();
    if !picocom_info.installed {
        tracing::warn!(
            connection = %conn_name,
            "picocom not found for Serial connection"
        );
        if let Some(root) = notebook.widget().root()
            && let Some(window) = root.downcast_ref::<gtk4::Window>()
        {
            crate::toast::show_missing_cli_toast(
                window,
                &i18n("Install picocom for Serial connections"),
            );
        }
        return None;
    }

    // Build picocom command via SerialProtocol
    let serial = SerialProtocol::new();
    let Some(command) = serial.build_command(conn) else {
        tracing::error!(
            connection = %conn_name,
            "Failed to build picocom command for Serial connection"
        );
        return None;
    };

    tracing::info!(
        connection = %conn_name,
        connection_id = %connection_id,
        "Starting Serial connection"
    );

    // Get terminal settings from state
    let terminal_settings = state
        .try_borrow()
        .ok()
        .map(|s| s.settings().terminal.clone())
        .unwrap_or_default();

    // Get global variables for substitution in Expect responses
    let global_variables = state
        .try_borrow()
        .ok()
        .map(|s| crate::state::resolve_global_variables(s.settings()))
        .unwrap_or_default();

    // Resolve automation config with group inheritance
    let resolved_automation = resolve_automation_for_connection(state, conn);

    // Create terminal tab for Serial
    let session_id = notebook.create_terminal_tab_with_settings(
        connection_id,
        &conn_name,
        "serial",
        Some(&resolved_automation),
        &terminal_settings,
        conn.theme_override.as_ref(),
        &global_variables,
    );

    // Apply highlight rules (built-in defaults + global + per-connection)
    {
        let global_rules = state
            .try_borrow()
            .ok()
            .map(|s| s.settings().highlight_rules.clone())
            .unwrap_or_default();
        notebook.set_highlight_rules(session_id, &global_rules, &conn.highlight_rules);
    }

    // Record connection start in history
    let history_entry_id = if let Ok(mut state_mut) = state.try_borrow_mut() {
        Some(state_mut.record_connection_start(conn, conn.username.as_deref()))
    } else {
        None
    };

    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }

    // Set up session logging if enabled
    if logging_enabled {
        MainWindow::setup_session_logging(state, notebook, session_id, connection_id, &conn_name);
    }

    // Wire up child exited callback
    MainWindow::setup_child_exited_handler(state, notebook, sidebar, session_id, connection_id);

    // Get device name for display
    let device = if let rustconn_core::ProtocolConfig::Serial(ref cfg) = conn.protocol_config {
        cfg.device.clone()
    } else {
        String::new()
    };

    // Build command string for display
    let serial_command = command.join(" ");
    let conn_msg = format_connection_message("Serial", &device);
    let cmd_msg = format_command_message(&serial_command);
    let feedback = format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n");
    notebook.display_output(session_id, &feedback);

    // Spawn picocom
    notebook.spawn_serial(session_id, &command);

    // --- Auto-recording for Serial ---
    if conn.session_recording_enabled {
        let notebook_clone = notebook.clone();
        let recording_conn_name = conn_name;
        // Serial is local — no SSH params needed
        glib::timeout_add_local_once(std::time::Duration::from_secs(1), move || {
            notebook_clone.start_recording(
                session_id,
                &recording_conn_name,
                rustconn_core::session::SanitizeConfig::default(),
                None,
            );
            tracing::info!(
                %session_id,
                "Auto-recording started for Serial connection"
            );
        });
    }

    Some(session_id)
}

/// Starts a Kubernetes connection
///
/// Creates a terminal tab and spawns `kubectl exec` or `kubectl run`
/// with the Kubernetes configuration. Uses `Protocol::build_command()`
/// from `KubernetesProtocol` to generate the command.
pub fn start_kubernetes_connection(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    logging_enabled: bool,
) -> Option<Uuid> {
    use rustconn_core::protocol::{
        KubernetesProtocol, Protocol, detect_kubectl, format_command_message,
        format_connection_message,
    };

    let conn_name = conn.name.clone();

    // Check kubectl availability before attempting to launch
    let kubectl_available = if rustconn_core::is_sandboxed() {
        rustconn_core::flatpak::is_host_command_available("kubectl")
    } else {
        let kubectl_info = detect_kubectl();
        kubectl_info.installed
    };
    if !kubectl_available {
        tracing::warn!(
            connection = %conn_name,
            sandboxed = rustconn_core::is_sandboxed(),
            "kubectl not found for Kubernetes connection"
        );
        if let Some(root) = notebook.widget().root()
            && let Some(window) = root.downcast_ref::<gtk4::Window>()
        {
            crate::toast::show_missing_cli_toast(
                window,
                &i18n("Install kubectl for Kubernetes connections"),
            );
        }
        return None;
    }

    // Build kubectl command via KubernetesProtocol
    let k8s = KubernetesProtocol::new();
    let Some(command) = k8s.build_command(conn) else {
        tracing::error!(
            connection = %conn_name,
            "Failed to build kubectl command for Kubernetes connection"
        );
        if let Some(root) = notebook.widget().root()
            && let Some(window) = root.downcast_ref::<gtk4::Window>()
        {
            crate::toast::show_toast_on_window(
                window,
                &i18n("Configure pod and container for Kubernetes"),
                crate::toast::ToastType::Error,
            );
        }
        return None;
    };

    tracing::info!(
        connection = %conn_name,
        connection_id = %connection_id,
        "Starting Kubernetes connection"
    );

    // Get terminal settings from state
    let terminal_settings = state
        .try_borrow()
        .ok()
        .map(|s| s.settings().terminal.clone())
        .unwrap_or_default();

    // Get global variables for substitution in Expect responses
    let global_variables = state
        .try_borrow()
        .ok()
        .map(|s| crate::state::resolve_global_variables(s.settings()))
        .unwrap_or_default();

    // Resolve automation config with group inheritance
    let resolved_automation = resolve_automation_for_connection(state, conn);

    // Create terminal tab for Kubernetes
    let session_id = notebook.create_terminal_tab_with_settings(
        connection_id,
        &conn_name,
        "kubernetes",
        Some(&resolved_automation),
        &terminal_settings,
        conn.theme_override.as_ref(),
        &global_variables,
    );

    // Apply highlight rules (built-in defaults + global + per-connection)
    {
        let global_rules = state
            .try_borrow()
            .ok()
            .map(|s| s.settings().highlight_rules.clone())
            .unwrap_or_default();
        notebook.set_highlight_rules(session_id, &global_rules, &conn.highlight_rules);
    }

    // Record connection start in history
    let history_entry_id = if let Ok(mut state_mut) = state.try_borrow_mut() {
        Some(state_mut.record_connection_start(conn, conn.username.as_deref()))
    } else {
        None
    };

    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }

    // Set up session logging if enabled
    if logging_enabled {
        MainWindow::setup_session_logging(state, notebook, session_id, connection_id, &conn_name);
    }

    // Wire up child exited callback
    MainWindow::setup_child_exited_handler(state, notebook, sidebar, session_id, connection_id);

    // Get pod/busybox info for display
    let target = if let rustconn_core::ProtocolConfig::Kubernetes(ref cfg) = conn.protocol_config {
        if cfg.use_busybox {
            format!("busybox ({})", cfg.busybox_image)
        } else {
            cfg.pod.clone().unwrap_or_default()
        }
    } else {
        String::new()
    };

    // Build command string for display
    let kubectl_command = command.join(" ");
    let conn_msg = format_connection_message("Kubernetes", &target);
    let cmd_msg = format_command_message(&kubectl_command);
    let feedback = format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n");
    notebook.display_output(session_id, &feedback);

    // Spawn argv directly (no `sh -c`) so namespace/pod/container fields are
    // never shell-interpreted — prevents command injection from shell metachars
    // in (possibly imported, untrusted) kubectl configs. kubectl runs in-sandbox
    // under Flatpak (Flatpak Components), so no host wrapper is needed.
    let argv: Vec<&str> = command.iter().map(String::as_str).collect();
    notebook.spawn_command(session_id, &argv, None, None, None);

    // --- Auto-recording for Kubernetes ---
    if conn.session_recording_enabled {
        let notebook_clone = notebook.clone();
        let recording_conn_name = conn_name;
        let recording_started = std::rc::Rc::new(std::cell::Cell::new(false));
        let recording_started_clone = recording_started.clone();

        notebook.connect_contents_changed(session_id, move || {
            if recording_started_clone.get() {
                return;
            }
            if let Some(row) = notebook_clone.get_terminal_cursor_row(session_id)
                && row > 0
            {
                recording_started_clone.set(true);
                notebook_clone.start_recording(
                    session_id,
                    &recording_conn_name,
                    rustconn_core::session::SanitizeConfig::default(),
                    None,
                );
                tracing::info!(
                    %session_id,
                    "Auto-recording started after Kubernetes connection"
                );
            }
        });
    }

    Some(session_id)
}

/// Starts a MOSH connection
///
/// Creates a terminal tab and spawns the `mosh` process with SSH port,
/// predict mode, server binary, and port range from `MoshConfig`.
/// Uses `MoshProtocol::build_command()` to generate the argv.
pub fn start_mosh_connection(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    logging_enabled: bool,
) -> Option<Uuid> {
    // Port check uses the SSH port (mosh handshake goes over SSH)
    let settings = state.borrow().settings().clone();
    let should_check = conn.should_pre_connect_check(&settings.connection);

    if should_check {
        let ssh_port = if let rustconn_core::ProtocolConfig::Mosh(ref cfg) = conn.protocol_config {
            cfg.ssh_port.unwrap_or(22)
        } else {
            22
        };
        let host = conn.host.clone();
        let timeout = settings.connection.port_check_timeout_secs;
        let state_clone = state.clone();
        let notebook_clone = notebook.clone();
        let sidebar_clone = sidebar.clone();
        let conn_clone = conn.clone();

        spawn_blocking_with_callback(
            move || check_port(&host, ssh_port, timeout),
            move |result| match result {
                Ok(_) => {
                    start_mosh_connection_internal(
                        &state_clone,
                        &notebook_clone,
                        &sidebar_clone,
                        connection_id,
                        &conn_clone,
                        logging_enabled,
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        protocol = "mosh",
                        host = %conn_clone.host,
                        error = %e,
                        "Port check failed for MOSH connection"
                    );
                    sidebar_clone.update_connection_status(&connection_id.to_string(), "failed");
                    if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                        state_mut.record_connection_attempt_failed(
                            &conn_clone,
                            conn_clone.username.as_deref(),
                            &e.to_string(),
                        );
                    }
                    if let Some(root) = notebook_clone.widget().root()
                        && let Some(window) = root.downcast_ref::<gtk4::Window>()
                    {
                        crate::toast::show_retry_toast_on_window(
                            window,
                            &e.to_string(),
                            &connection_id.to_string(),
                        );
                    }
                }
            },
        );
        None
    } else {
        start_mosh_connection_internal(
            state,
            notebook,
            sidebar,
            connection_id,
            conn,
            logging_enabled,
        )
    }
}

/// Internal function to start MOSH connection (after port check).
fn start_mosh_connection_internal(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    logging_enabled: bool,
) -> Option<Uuid> {
    use rustconn_core::protocol::{
        MoshProtocol, Protocol, detect_mosh, format_command_message, format_connection_message,
    };

    let conn_name = conn.name.clone();

    // Check mosh availability
    let mosh_info = detect_mosh();
    if !mosh_info.installed {
        tracing::warn!(
            connection = %conn_name,
            "mosh not found for MOSH connection"
        );
        if let Some(root) = notebook.widget().root()
            && let Some(window) = root.downcast_ref::<gtk4::Window>()
        {
            crate::toast::show_missing_cli_toast(
                window,
                &i18n("Install mosh for MOSH connections"),
            );
        }
        return None;
    }

    // Build mosh command via MoshProtocol
    let mosh = MoshProtocol::new();
    let Some(command) = mosh.build_command(conn) else {
        tracing::error!(
            connection = %conn_name,
            "Failed to build mosh command"
        );
        return None;
    };

    tracing::info!(
        connection = %conn_name,
        connection_id = %connection_id,
        "Starting MOSH connection"
    );

    // Get terminal settings from state
    let terminal_settings = state
        .try_borrow()
        .ok()
        .map(|s| s.settings().terminal.clone())
        .unwrap_or_default();

    // Get global variables for substitution in Expect responses
    let global_variables = state
        .try_borrow()
        .ok()
        .map(|s| crate::state::resolve_global_variables(s.settings()))
        .unwrap_or_default();

    // Resolve automation config with group inheritance
    let resolved_automation = resolve_automation_for_connection(state, conn);

    // Create terminal tab for MOSH
    let session_id = notebook.create_terminal_tab_with_settings(
        connection_id,
        &conn_name,
        "mosh",
        Some(&resolved_automation),
        &terminal_settings,
        conn.theme_override.as_ref(),
        &global_variables,
    );

    // Apply highlight rules (built-in defaults + global + per-connection)
    {
        let global_rules = state
            .try_borrow()
            .ok()
            .map(|s| s.settings().highlight_rules.clone())
            .unwrap_or_default();
        notebook.set_highlight_rules(session_id, &global_rules, &conn.highlight_rules);
    }

    // Record connection start in history
    let history_entry_id = if let Ok(mut state_mut) = state.try_borrow_mut() {
        Some(state_mut.record_connection_start(conn, conn.username.as_deref()))
    } else {
        None
    };

    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }

    // Set up session logging if enabled
    if logging_enabled {
        MainWindow::setup_session_logging(state, notebook, session_id, connection_id, &conn_name);
    }

    // Wire up child exited callback
    MainWindow::setup_child_exited_handler(state, notebook, sidebar, session_id, connection_id);

    // Build command string for display
    let mosh_command = command.join(" ");
    let conn_msg = format_connection_message("MOSH", &conn.host);
    let cmd_msg = format_command_message(&mosh_command);
    let feedback = format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n");
    notebook.display_output(session_id, &feedback);

    // Spawn mosh — uses exec (no shell wrapper needed)
    let argv: Vec<&str> = command.iter().map(String::as_str).collect();
    notebook.spawn_command(session_id, &argv, None, None, None);

    // --- Auto-recording for MOSH ---
    if conn.session_recording_enabled {
        let notebook_clone = notebook.clone();
        let recording_conn_name = conn_name;
        let recording_started = std::rc::Rc::new(std::cell::Cell::new(false));
        let recording_started_clone = recording_started.clone();
        let ssh_port = if let rustconn_core::ProtocolConfig::Mosh(ref cfg) = conn.protocol_config {
            cfg.ssh_port.unwrap_or(22)
        } else {
            22
        };
        let recording_ssh_params = Some(crate::terminal::SshRecordingParams {
            host: conn.host.clone(),
            port: ssh_port,
            username: conn.username.clone(),
            identity_file: None,
        });

        notebook.connect_contents_changed(session_id, move || {
            if recording_started_clone.get() {
                return;
            }
            if let Some(row) = notebook_clone.get_terminal_cursor_row(session_id)
                && row > 2
            {
                recording_started_clone.set(true);
                notebook_clone.start_recording(
                    session_id,
                    &recording_conn_name,
                    rustconn_core::session::SanitizeConfig::default(),
                    recording_ssh_params.clone(),
                );
                tracing::info!(
                    %session_id,
                    "Auto-recording started after MOSH connection"
                );
            }
        });
    }

    Some(session_id)
}
