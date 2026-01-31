//! Protocol-specific connection handlers for main window
//!
//! This module contains functions for starting connections for different protocols:
//! SSH, VNC, SPICE, and Zero Trust.

use crate::sidebar::ConnectionSidebar;
use crate::state::SharedAppState;
use crate::terminal::TerminalNotebook;
use crate::utils::spawn_blocking_with_callback;
use crate::window::MainWindow;
use gtk4::prelude::*;
use rustconn_core::check_port;
use rustconn_core::variables::{Variable, VariableManager, VariableScope};
use std::rc::Rc;
use uuid::Uuid;

/// Type alias for shared sidebar reference
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Type alias for shared notebook reference
pub type SharedNotebook = Rc<TerminalNotebook>;

/// Substitutes variables in a string using global variables from settings
///
/// Converts `${VAR_NAME}` references to their values from global variables.
/// If a variable is not found, the reference is left unchanged.
fn substitute_variables(input: &str, global_variables: &[Variable]) -> String {
    if !input.contains("${") {
        return input.to_string();
    }

    let mut manager = VariableManager::new();
    for var in global_variables {
        manager.set_global(var.clone());
    }

    manager
        .substitute(input, VariableScope::Global)
        .unwrap_or_else(|_| input.to_string())
}
/// Starts an SSH connection
///
/// Creates a terminal tab and spawns the SSH process with the given configuration.
#[allow(clippy::too_many_arguments)]
pub fn start_ssh_connection(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    logging_enabled: bool,
) -> Option<Uuid> {
    use rustconn_core::protocol::{format_command_message, format_connection_message};

    let conn_name = conn.name.clone();

    // Get terminal settings from state
    let terminal_settings = state
        .try_borrow()
        .ok()
        .map(|s| s.settings().terminal.clone())
        .unwrap_or_default();

    // Create terminal tab for SSH with user settings
    let session_id = notebook.create_terminal_tab_with_settings(
        connection_id,
        &conn.name,
        "ssh",
        Some(&conn.automation),
        &terminal_settings,
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

    // Build and spawn SSH command
    let port = conn.port;

    // Get global variables for substitution
    let global_variables = state
        .try_borrow()
        .ok()
        .map(|s| s.settings().global_variables.clone())
        .unwrap_or_default();

    // Apply variable substitution to host and username (e.g., ${VAR_NAME} -> actual value)
    let host = substitute_variables(&conn.host, &global_variables);
    let username = conn
        .username
        .as_ref()
        .map(|u| substitute_variables(u, &global_variables));

    // Get SSH-specific options
    let (identity_file, extra_args) =
        if let rustconn_core::ProtocolConfig::Ssh(ssh_config) = &conn.protocol_config {
            let key = ssh_config
                .key_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string());
            let mut args = Vec::new();

            let mut jump_hosts = Vec::new();

            // Handle string-based proxy jump (legacy/manual)
            if let Some(proxy) = &ssh_config.proxy_jump {
                jump_hosts.push(proxy.clone());
            }

            // Handle reference-based jump host (recursive resolution)
            if let Some(jump_id) = ssh_config.jump_host_id {
                if let Ok(state_ref) = state.try_borrow() {
                    let mut current_id = Some(jump_id);
                    let mut visited = std::collections::HashSet::new();
                    visited.insert(connection_id); // Avoid self-reference loop

                    // Limit recursion depth to avoid infinite loops
                    for _ in 0..10 {
                        if let Some(jid) = current_id {
                            if visited.contains(&jid) {
                                break;
                            }
                            visited.insert(jid);

                            if let Some(jump_conn) = state_ref.get_connection(jid) {
                                // Format: [user@]host[:port]
                                let mut host_str = jump_conn.host.clone();
                                if let Some(user) = &jump_conn.username {
                                    host_str = format!("{}@{}", user, host_str);
                                }
                                if jump_conn.port != 22 {
                                    host_str = format!("{}:{}", host_str, jump_conn.port);
                                }
                                jump_hosts.push(host_str);

                                // Check if this jump host has its own jumper
                                if let rustconn_core::ProtocolConfig::Ssh(jump_config) =
                                    &jump_conn.protocol_config
                                {
                                    // Prepend manual proxy if exists on jump host (unlikely but possible)
                                    if let Some(p) = &jump_config.proxy_jump {
                                        jump_hosts.insert(jump_hosts.len() - 1, p.clone());
                                    }
                                    current_id = jump_config.jump_host_id;
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
            }

            if !jump_hosts.is_empty() {
                args.push("-J".to_string());
                // SSH expects comma-separated list: jump1,jump2,dest
                // But -J option takes jumps to reach dest.
                // If we chain: A -> B -> C -> Dest.
                // We need `ssh -J B,C Dest` (from A).
                // My recursion pushed B then C. So `jump_hosts` is [B, C].
                // Join with comma.
                args.push(jump_hosts.join(","));
            }

            if ssh_config.use_control_master {
                args.push("-o".to_string());
                args.push("ControlMaster=auto".to_string());
            }

            if ssh_config.agent_forwarding {
                args.push("-A".to_string());
            }

            for (k, v) in &ssh_config.custom_options {
                args.push("-o".to_string());
                args.push(format!("{k}={v}"));
            }

            (key, args)
        } else {
            (None, Vec::new())
        };

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut() {
        let _ = state_mut.update_last_connected(connection_id);
    }

    // Set up session logging if enabled
    if logging_enabled {
        MainWindow::setup_session_logging(state, notebook, session_id, connection_id, &conn_name);
    }

    // Wire up child exited callback for session cleanup
    MainWindow::setup_child_exited_handler(state, notebook, sidebar, session_id, connection_id);

    // Build SSH command string for display
    let mut ssh_cmd_parts = vec!["ssh".to_string()];
    if port != 22 {
        ssh_cmd_parts.push("-p".to_string());
        ssh_cmd_parts.push(port.to_string());
    }
    if let Some(ref key) = identity_file {
        ssh_cmd_parts.push("-i".to_string());
        ssh_cmd_parts.push(key.clone());
    }
    ssh_cmd_parts.extend(extra_args.clone());
    let destination = if let Some(ref user) = username {
        format!("{user}@{host}")
    } else {
        host.clone()
    };
    ssh_cmd_parts.push(destination);
    let ssh_command = ssh_cmd_parts.join(" ");

    // Display CLI output feedback before executing command
    let conn_msg = format_connection_message("SSH", &host);
    let cmd_msg = format_command_message(&ssh_command);
    let feedback = format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n");
    notebook.display_output(session_id, &feedback);

    // Spawn SSH
    let extra_refs: Vec<&str> = extra_args.iter().map(std::string::String::as_str).collect();
    notebook.spawn_ssh(
        session_id,
        &host,
        port,
        username.as_deref(),
        identity_file.as_deref(),
        &extra_refs,
    );

    // Wire up child exited callback for session cleanup (second call for terminal monitoring)
    MainWindow::setup_child_exited_handler(state, notebook, sidebar, session_id, connection_id);

    Some(session_id)
}

/// Starts a VNC connection
///
/// Creates a VNC session tab with native widget and initiates connection.
pub fn start_vnc_connection(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
) -> Option<Uuid> {
    // Check if port check is needed
    let settings = state.borrow().settings().clone();
    let should_check = settings.connection.pre_connect_port_check && !conn.skip_port_check;

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
                        // Port check failed, show error
                        tracing::warn!("Port check failed for VNC connection: {e}");
                        sidebar_clone
                            .update_connection_status(&connection_id.to_string(), "failed");
                        if let Some(root) = notebook_clone.widget().root() {
                            if let Some(window) = root.downcast_ref::<gtk4::Window>() {
                                crate::alert::show_error(
                                    window,
                                    "Connection Failed",
                                    &format!(
                                        "{e}\n\nThe host may be offline or the port may be blocked."
                                    ),
                                );
                            }
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
    let conn_name = conn.name.clone();
    let port = conn.port;

    // Get global variables for substitution
    let global_variables = state
        .try_borrow()
        .ok()
        .map(|s| s.settings().global_variables.clone())
        .unwrap_or_default();

    // Apply variable substitution to host
    let host = substitute_variables(&conn.host, &global_variables);

    // Get VNC-specific configuration
    let vnc_config = if let rustconn_core::ProtocolConfig::Vnc(config) = &conn.protocol_config {
        config.clone()
    } else {
        rustconn_core::models::VncConfig::default()
    };

    // Get password from cached credentials (set by credential resolution flow)
    let password: Option<String> = state.try_borrow().ok().and_then(|state_ref| {
        state_ref.get_cached_credentials(connection_id).map(|c| {
            use secrecy::ExposeSecret;
            tracing::debug!("[VNC] Found cached credentials for connection");
            c.password.expose_secret().to_string()
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
                notebook_for_state.mark_tab_disconnected(session_id);
                sidebar_for_state.decrement_session_count(&connection_id.to_string(), false);
                // Record connection end in history
                if let Some(info) = notebook_for_state.get_session_info(session_id) {
                    if let Some(entry_id) = info.history_entry_id {
                        if let Ok(mut state_mut) = state_for_callback.try_borrow_mut() {
                            state_mut.record_connection_end(entry_id);
                        }
                    }
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
                eprintln!("VNC reconnect failed: {}", e);
            }
        });

        // Initiate connection with VNC config (respects client_mode setting)
        if let Err(e) =
            vnc_widget.connect_with_config(&host, port, password.as_deref(), &vnc_config)
        {
            eprintln!("Failed to connect VNC session '{}': {}", conn_name, e);
            sidebar.update_connection_status(&connection_id.to_string(), "failed");
        } else {
            sidebar.update_connection_status(&connection_id.to_string(), "connecting");
        }
    }

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut() {
        let _ = state_mut.update_last_connected(connection_id);
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
    // Check if port check is needed
    let settings = state.borrow().settings().clone();
    let should_check = settings.connection.pre_connect_port_check && !conn.skip_port_check;

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
                        // Port check failed, show error
                        tracing::warn!("Port check failed for SPICE connection: {e}");
                        sidebar_clone
                            .update_connection_status(&connection_id.to_string(), "failed");
                        if let Some(root) = notebook_clone.widget().root() {
                            if let Some(window) = root.downcast_ref::<gtk4::Window>() {
                                crate::alert::show_error(
                                    window,
                                    "Connection Failed",
                                    &format!(
                                        "{e}\n\nThe host may be offline or the port may be blocked."
                                    ),
                                );
                            }
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

    // Get global variables for substitution
    let global_variables = state
        .try_borrow()
        .ok()
        .map(|s| s.settings().global_variables.clone())
        .unwrap_or_default();

    // Apply variable substitution to host
    let host = substitute_variables(&conn.host, &global_variables);

    // Get SPICE-specific options from connection config
    let spice_opts = if let rustconn_core::ProtocolConfig::Spice(config) = &conn.protocol_config {
        Some(config.clone())
    } else {
        None
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

    // Get the SPICE widget and initiate connection
    if let Some(spice_widget) = notebook.get_spice_widget(session_id) {
        // Build connection config using SpiceClientConfig from spice_client module
        use rustconn_core::SpiceClientConfig;
        let mut config = SpiceClientConfig::new(&host).with_port(port);

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
                notebook_for_state.mark_tab_disconnected(session_id);
                sidebar_for_state.decrement_session_count(
                    &connection_id.to_string(),
                    spice_state == SpiceConnectionState::Error,
                );
                // Record connection end/failure in history
                if let Some(info) = notebook_for_state.get_session_info(session_id) {
                    if let Some(entry_id) = info.history_entry_id {
                        if let Ok(mut state_mut) = state_for_callback.try_borrow_mut() {
                            if spice_state == SpiceConnectionState::Error {
                                state_mut
                                    .record_connection_failed(entry_id, "SPICE connection error");
                            } else {
                                state_mut.record_connection_end(entry_id);
                            }
                        }
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
                eprintln!("SPICE reconnect failed: {}", e);
            }
        });

        // Initiate connection
        if let Err(e) = spice_widget.connect(&config) {
            eprintln!("Failed to connect SPICE session '{conn_name}': {e}");
        }
    }

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut() {
        let _ = state_mut.update_last_connected(connection_id);
    }

    Some(session_id)
}

/// Starts a Zero Trust connection
///
/// Creates a terminal tab and spawns the Zero Trust provider command.
#[allow(clippy::too_many_arguments)]
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
            let (prog, args) = zt_config.build_command(username.as_deref());
            let provider = zt_config.provider.display_name();
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
                rustconn_core::models::ZeroTrustProvider::Generic => "generic",
            };
            (prog, args, provider, key)
        } else {
            return None;
        };

    let automation_config = conn.automation.clone();

    // Get terminal settings from state
    let terminal_settings = state
        .try_borrow()
        .ok()
        .map(|s| s.settings().terminal.clone())
        .unwrap_or_default();

    // Create terminal tab for Zero Trust with provider-specific protocol
    let tab_protocol = format!("zerotrust:{provider_key}");
    let session_id = notebook.create_terminal_tab_with_settings(
        connection_id,
        &conn_name,
        &tab_protocol,
        Some(&automation_config),
        &terminal_settings,
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
    if let Ok(mut state_mut) = state.try_borrow_mut() {
        let _ = state_mut.update_last_connected(connection_id);
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

    // Spawn the Zero Trust command through shell to use full PATH
    // This is needed because VTE doesn't see snap/flatpak paths
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    notebook.spawn_command(session_id, &[&shell, "-c", &full_command], None, None);

    Some(session_id)
}
