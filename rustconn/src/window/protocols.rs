//! Protocol-specific connection handlers for main window
//!
//! This module contains functions for starting connections for different protocols:
//! SSH, VNC, SPICE, Telnet, Serial, Kubernetes, and Zero Trust.

use super::MainWindow;
use crate::i18n::i18n;
use crate::sidebar::ConnectionSidebar;
use crate::state::SharedAppState;
use crate::terminal::TerminalNotebook;
use crate::utils::spawn_blocking_with_callback;
use gtk4::glib;
use gtk4::prelude::*;
use rustconn_core::connection::check_port;
use rustconn_core::variables::{Variable, VariableManager, VariableScope};
use secrecy::SecretString;
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
fn contains_ssh_failure(text: &str) -> bool {
    let lower = text.to_lowercase();
    SSH_FAILURE_PATTERNS
        .iter()
        .any(|p| lower.contains(&p.to_lowercase()))
}

/// Starts an SSH connection
///
/// Creates a terminal tab and spawns the SSH process with the given configuration.
#[allow(clippy::too_many_arguments)]
pub fn start_ssh_connection(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    monitoring: &super::types::SharedMonitoring,
    connection_id: Uuid,
    conn: &rustconn_core::Connection,
    logging_enabled: bool,
) -> Option<Uuid> {
    // Check if port check is needed
    let settings = state.borrow().settings().clone();
    let has_jump_host = matches!(
        &conn.protocol_config,
        rustconn_core::ProtocolConfig::Ssh(ssh)
            if ssh.jump_host_id.is_some() || ssh.proxy_jump.is_some()
    );
    // Skip port check when a jump host is configured — the destination
    // is only reachable through the jump host, so a direct TCP probe
    // will always time out.
    let should_check =
        settings.connection.pre_connect_port_check && !conn.skip_port_check && !has_jump_host;

    if has_jump_host && settings.connection.pre_connect_port_check && !conn.skip_port_check {
        tracing::debug!(
            protocol = "ssh",
            host = %conn.host,
            port = conn.port,
            "Skipping port check — connection uses a jump host"
        );
    }

    if should_check {
        let host = conn.host.clone();
        let port = conn.port;
        let timeout = settings.connection.port_check_timeout_secs;
        let state_clone = state.clone();
        let notebook_clone = notebook.clone();
        let sidebar_clone = sidebar.clone();
        let monitoring_clone = Rc::clone(monitoring);
        let conn_clone = conn.clone();

        // Run port check in background thread
        spawn_blocking_with_callback(
            move || check_port(&host, port, timeout),
            move |result| {
                match result {
                    Ok(_) => {
                        // Port is open, proceed with connection
                        start_ssh_connection_internal(
                            &state_clone,
                            &notebook_clone,
                            &sidebar_clone,
                            &monitoring_clone,
                            connection_id,
                            &conn_clone,
                            logging_enabled,
                        );
                    }
                    Err(e) => {
                        // Port check failed, show error with retry
                        tracing::warn!(
                            protocol = "ssh",
                            host = %conn_clone.host,
                            port = conn_clone.port,
                            error = %e,
                            "Port check failed for SSH connection"
                        );
                        sidebar_clone
                            .update_connection_status(&connection_id.to_string(), "failed");
                        if let Some(root) = notebook_clone.widget().root()
                            && let Some(window) = root.downcast_ref::<gtk4::Window>()
                        {
                            crate::toast::show_retry_toast_on_window(
                                window,
                                &crate::i18n::i18n("Connection failed. Host unreachable."),
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
        start_ssh_connection_internal(
            state,
            notebook,
            sidebar,
            monitoring,
            connection_id,
            conn,
            logging_enabled,
        )
    }
}

/// Internal function to start SSH connection (after port check).
///
/// Creates a terminal tab and spawns the SSH process with the given configuration.
#[allow(clippy::too_many_arguments)]
fn start_ssh_connection_internal(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    monitoring: &super::types::SharedMonitoring,
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
        conn.theme_override.as_ref(),
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

    // Store history entry ID in session for later use
    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Build and spawn SSH command
    let port = conn.port;

    // Detect jump host for status detection and monitoring
    let has_jump_host = matches!(
        &conn.protocol_config,
        rustconn_core::ProtocolConfig::Ssh(ssh)
            if ssh.jump_host_id.is_some() || ssh.proxy_jump.is_some()
    );

    // Get global variables for substitution (secret values resolved from vault)
    let global_variables = state
        .try_borrow()
        .ok()
        .map(|s| crate::state::resolve_global_variables(s.settings()))
        .unwrap_or_default();

    // Apply variable substitution to host and username (e.g., ${VAR_NAME} -> actual value)
    let host = substitute_variables(&conn.host, &global_variables);
    let username = conn
        .username
        .as_ref()
        .map(|u| substitute_variables(u, &global_variables));

    // Get SSH-specific options
    let (identity_file, extra_args, use_waypipe, jump_host_chain) =
        if let rustconn_core::ProtocolConfig::Ssh(ssh_config) = &conn.protocol_config {
            let key = ssh_config
                .key_path
                .as_ref()
                .and_then(|p| {
                    // Resolve stale portal paths: if the stored path doesn't exist,
                    // check the Flatpak SSH dir for a file with the same name.
                    rustconn_core::resolve_key_path(p)
                })
                .map(|p| p.to_string_lossy().to_string());

            // Use build_command_args() for all SSH-specific flags:
            // identity, IdentitiesOnly, proxy_jump, ControlMaster/Persist,
            // agent forwarding, X11, compression, custom options, port forwards
            let mut args = ssh_config.build_command_args();

            // Resolve jump host chain from connection references (needs state access)
            let mut jump_hosts = Vec::new();

            // Handle string-based proxy jump (legacy/manual)
            if let Some(proxy) = &ssh_config.proxy_jump {
                jump_hosts.push(proxy.clone());
            }

            // Handle reference-based jump host (recursive resolution)
            if let Some(jump_id) = ssh_config.jump_host_id
                && let Ok(state_ref) = state.try_borrow()
            {
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

            // In Flatpak, ~/.ssh is read-only — point known_hosts to a writable path.
            // Must be set BEFORE jump host resolution because ProxyCommand needs it too.
            let flatpak_known_hosts = {
                let user_set = ssh_config
                    .custom_options
                    .keys()
                    .any(|k| k.eq_ignore_ascii_case("UserKnownHostsFile"));
                if user_set {
                    None
                } else {
                    rustconn_core::get_flatpak_known_hosts_path()
                }
            };
            if let Some(ref kh_path) = flatpak_known_hosts {
                tracing::debug!(
                    protocol = "ssh",
                    path = %kh_path.display(),
                    "Using Flatpak-writable known_hosts"
                );
                args.push("-o".to_string());
                args.push(format!("UserKnownHostsFile={}", kh_path.display()));
            }

            // Override proxy_jump with resolved jump host chain if we have
            // reference-based jump hosts (build_command_args already added -J
            // for the string-based proxy_jump, so only add if we have more)
            //
            // In Flatpak, -J (ProxyJump) spawns a nested SSH process that does NOT
            // inherit -o or -i flags from the outer command. This means the jump host
            // SSH tries to write to ~/.ssh/known_hosts (read-only) and cannot find
            // identity files. Fix: replace -J with -o ProxyCommand that passes
            // UserKnownHostsFile and identity to the jump host SSH process.
            let jump_host_str = if jump_hosts.is_empty() {
                None
            } else {
                // Remove the -J added by build_command_args (if proxy_jump was set)
                if ssh_config.proxy_jump.is_some()
                    && let Some(pos) = args.iter().position(|a| a == "-J")
                {
                    args.remove(pos); // remove "-J"
                    if pos < args.len() {
                        args.remove(pos); // remove the value
                    }
                }
                let chain = jump_hosts.join(",");

                if flatpak_known_hosts.is_some() {
                    // Flatpak: use ProxyCommand so jump host SSH inherits known_hosts
                    // and identity file. Build a ProxyCommand for the first hop;
                    // if there are multiple hops, nest them via -J within ProxyCommand.
                    let mut proxy_parts =
                        vec!["ssh".to_string(), "-W".to_string(), "%h:%p".to_string()];

                    // Pass identity file to jump host if we have one
                    if let Some(pos) = args.iter().position(|a| a == "-i")
                        && let Some(key_path) = args.get(pos + 1)
                    {
                        proxy_parts.push("-i".to_string());
                        proxy_parts.push(key_path.clone());
                        proxy_parts.push("-o".to_string());
                        proxy_parts.push("IdentitiesOnly=yes".to_string());
                    }

                    // Pass UserKnownHostsFile to jump host
                    if let Some(ref kh_path) = flatpak_known_hosts {
                        proxy_parts.push("-o".to_string());
                        proxy_parts.push(format!("UserKnownHostsFile={}", kh_path.display()));
                    }

                    // For multi-hop chains, pass remaining hops via -J inside ProxyCommand
                    if jump_hosts.len() > 1 {
                        let inner_chain = jump_hosts[1..].join(",");
                        proxy_parts.push("-J".to_string());
                        proxy_parts.push(inner_chain);
                    }

                    // Add the first hop destination
                    proxy_parts.push(jump_hosts[0].clone());

                    let proxy_cmd = proxy_parts.join(" ");
                    tracing::debug!(
                        protocol = "ssh",
                        proxy_command = %proxy_cmd,
                        "Using ProxyCommand instead of -J for Flatpak known_hosts compatibility"
                    );
                    args.push("-o".to_string());
                    args.push(format!("ProxyCommand={proxy_cmd}"));
                } else {
                    // Non-Flatpak: use standard -J
                    args.push("-J".to_string());
                    args.push(chain.clone());
                }

                Some(chain)
            };

            // Check waypipe: enabled in config + binary available on PATH
            let waypipe = ssh_config.waypipe && rustconn_core::protocol::detect_waypipe().installed;
            if ssh_config.waypipe && !waypipe {
                tracing::warn!(
                    protocol = "ssh",
                    host = %host,
                    "Waypipe enabled but not found on PATH, falling back to direct SSH"
                );
            }
            if waypipe {
                tracing::info!(
                    protocol = "ssh",
                    host = %host,
                    "Using waypipe for Wayland application forwarding"
                );
            }

            (key, args, waypipe, jump_host_str)
        } else {
            (None, Vec::new(), false, None)
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

    // Wire up child exited callback for session cleanup
    MainWindow::setup_child_exited_handler(state, notebook, sidebar, session_id, connection_id);

    // Build SSH command string for display
    let mut ssh_cmd_parts = if use_waypipe {
        vec!["waypipe".to_string(), "ssh".to_string()]
    } else {
        vec!["ssh".to_string()]
    };
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

    // Retrieve cached credentials (resolved from vault earlier)
    let cached_password: Option<SecretString> = state
        .try_borrow()
        .ok()
        .and_then(|s| s.get_cached_credentials(connection_id).cloned())
        .and_then(|c| {
            use secrecy::ExposeSecret;
            let pw = c.password.expose_secret();
            if pw.is_empty() {
                None
            } else {
                Some(c.password.clone())
            }
        });

    // Spawn SSH normally — password injection happens via VTE feed_child
    // when the terminal detects a password prompt (see below).
    {
        let extra_refs: Vec<&str> = extra_args.iter().map(std::string::String::as_str).collect();
        notebook.spawn_ssh(
            session_id,
            &host,
            port,
            username.as_deref(),
            identity_file.as_deref(),
            &extra_refs,
            use_waypipe,
            None,
        );
    }

    // --- VTE password injection: detect "password:" prompt and feed cached password ---
    // This replaces the previous sshpass dependency. The terminal output is
    // monitored for SSH password prompts; when detected, the vault password
    // is sent via feed_child() exactly once.
    if let Some(vault_password) = cached_password.clone() {
        let notebook_clone = notebook.clone();
        let password_sent = std::rc::Rc::new(std::cell::Cell::new(false));
        let password_sent_clone = password_sent.clone();

        tracing::info!(
            protocol = "ssh",
            host = %host,
            "Vault password available; will auto-fill on prompt"
        );

        notebook.connect_contents_changed(session_id, move || {
            if password_sent_clone.get() {
                return;
            }
            let Some(text) = notebook_clone.get_terminal_text(session_id) else {
                return;
            };
            // Check for common SSH password prompts (case-insensitive)
            let lower = text.to_lowercase();
            let has_prompt = lower.ends_with("password: ")
                || lower.ends_with("password:")
                || lower.contains("password: \n")
                || lower.lines().last().is_some_and(|line| {
                    let l = line.trim().to_lowercase();
                    l.ends_with("password:")
                        || l.ends_with("password: ")
                        || l.contains("'s password:")
                });

            if has_prompt {
                use secrecy::ExposeSecret;
                let pw = vault_password.expose_secret();
                // Send password + Enter
                let input = format!("{pw}\n");
                notebook_clone.send_text_to_session(session_id, &input);
                password_sent_clone.set(true);
                tracing::info!(
                    protocol = "ssh",
                    "Password prompt detected; credentials sent via VTE"
                );
            }
        });
    }

    // Wire up child exited callback for session cleanup (second call for terminal monitoring)
    MainWindow::setup_child_exited_handler(state, notebook, sidebar, session_id, connection_id);

    // --- SSH status detection: mark sidebar "connected" once terminal output appears ---
    // For jump host connections, also check terminal text for SSH failure patterns
    // to avoid false positives (jump host connects but destination times out).
    {
        let sidebar_clone = sidebar.clone();
        let notebook_clone = notebook.clone();
        let connection_id_str = connection_id.to_string();
        let session_connected = std::rc::Rc::new(std::cell::Cell::new(false));
        let session_connected_clone = session_connected.clone();
        let protocol_str = String::from("ssh");
        let uses_jump_host = has_jump_host;

        notebook.connect_contents_changed(session_id, move || {
            if session_connected_clone.get() {
                return;
            }
            if let Some(row) = notebook_clone.get_terminal_cursor_row(session_id) {
                tracing::debug!(
                    protocol = "ssh",
                    cursor_row = row,
                    threshold = 2,
                    "SSH status detection: checking cursor row"
                );
                if row > 2 {
                    // When using a jump host, the cursor may advance past row 2
                    // due to jump host banners or SSH error output even if the
                    // final destination is unreachable. Check terminal text for
                    // known SSH failure patterns before marking as connected.
                    if uses_jump_host
                        && let Some(text) = notebook_clone.get_terminal_text(session_id)
                        && contains_ssh_failure(&text)
                    {
                        tracing::debug!(
                            protocol = "ssh",
                            cursor_row = row,
                            "Jump host connection: SSH failure detected in terminal"
                        );
                        return;
                    }
                    sidebar_clone.increment_session_count(&connection_id_str);
                    session_connected_clone.set(true);
                    tracing::info!(
                        protocol = %protocol_str,
                        cursor_row = row,
                        "Terminal connection detected as established"
                    );
                }
            }
        });
    }

    // --- Auto-recording: start recording once SSH connection is established ---
    if conn.session_recording_enabled {
        let notebook_clone = notebook.clone();
        let recording_conn_name = conn_name.clone();
        let recording_started = std::rc::Rc::new(std::cell::Cell::new(false));
        let recording_started_clone = recording_started.clone();
        let recording_ssh_params = Some(crate::terminal::SshRecordingParams {
            host: host.clone(),
            port,
            username: username.clone(),
            identity_file: identity_file.clone(),
        });

        notebook.connect_contents_changed(session_id, move || {
            if recording_started_clone.get() {
                return;
            }
            // Wait for connection to be established (cursor row > 2)
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
                    "Auto-recording started after SSH connection established"
                );
            }
        });
    }

    // --- Deferred monitoring start: wait for SSH to connect before opening monitor ---
    if let Ok(state_ref) = state.try_borrow() {
        let settings = state_ref.settings().monitoring.clone();
        let mon_enabled = conn
            .monitoring_config
            .as_ref()
            .map_or(settings.enabled, |mc| mc.is_enabled(&settings));
        if mon_enabled {
            let effective = rustconn_core::MonitoringSettings {
                enabled: true,
                interval_secs: conn.monitoring_config.as_ref().map_or_else(
                    || settings.effective_interval_secs(),
                    |mc| mc.effective_interval(&settings),
                ),
                ..settings
            };
            let identity_file_mon =
                if let rustconn_core::ProtocolConfig::Ssh(ref ssh_cfg) = conn.protocol_config {
                    ssh_cfg
                        .key_path
                        .as_ref()
                        .and_then(|p| rustconn_core::resolve_key_path(p))
                        .map(|p| p.to_string_lossy().to_string())
                } else {
                    None
                };
            let cached_pw = state_ref
                .get_cached_credentials(connection_id)
                .and_then(|c| {
                    use secrecy::ExposeSecret;
                    let pw = c.password.expose_secret();
                    if pw.is_empty() {
                        None
                    } else {
                        Some(c.password.clone())
                    }
                });

            let monitoring_clone = Rc::clone(monitoring);
            let notebook_clone = notebook.clone();
            let mon_host = conn.host.clone();
            let mon_port = conn.port;
            let mon_username = conn.username.clone();
            let mon_jump_host = jump_host_chain.clone();
            let monitoring_started = std::rc::Rc::new(std::cell::Cell::new(false));
            let monitoring_started_clone = monitoring_started.clone();

            notebook.connect_contents_changed(session_id, move || {
                if monitoring_started_clone.get() {
                    return;
                }
                let Some(row) = notebook_clone.get_terminal_cursor_row(session_id) else {
                    return;
                };
                if row <= 2 {
                    return;
                }
                monitoring_started_clone.set(true);
                if let Some(container) = notebook_clone.get_session_container(session_id) {
                    monitoring_clone.start_monitoring(
                        session_id,
                        &container,
                        &effective,
                        &mon_host,
                        mon_port,
                        mon_username.as_deref(),
                        identity_file_mon.as_deref(),
                        cached_pw.clone(),
                        mon_jump_host.as_deref(),
                    );
                }
            });
        }
    }

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
                        // Port check failed, show error with retry
                        tracing::warn!("Port check failed for VNC connection: {e}");
                        sidebar_clone
                            .update_connection_status(&connection_id.to_string(), "failed");
                        if let Some(root) = notebook_clone.widget().root()
                            && let Some(window) = root.downcast_ref::<gtk4::Window>()
                        {
                            crate::toast::show_retry_toast_on_window(
                                window,
                                &crate::i18n::i18n("Connection failed. Host unreachable."),
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
        if let Err(e) =
            vnc_widget.connect_with_config(&host, port, password.as_deref(), &vnc_config)
        {
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
                        // Port check failed, show error with retry
                        tracing::warn!("Port check failed for SPICE connection: {e}");
                        sidebar_clone
                            .update_connection_status(&connection_id.to_string(), "failed");
                        if let Some(root) = notebook_clone.widget().root()
                            && let Some(window) = root.downcast_ref::<gtk4::Window>()
                        {
                            crate::toast::show_retry_toast_on_window(
                                window,
                                &crate::i18n::i18n("Connection failed. Host unreachable."),
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
        use rustconn_core::spice_client::SpiceClientConfig;
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

            // Configure local cursor visibility
            config.show_local_cursor = opts.show_local_cursor;
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

/// Starts a Telnet connection
///
/// Creates a terminal tab and spawns the telnet process.
#[allow(clippy::too_many_arguments)]
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
                    if let Some(root) = notebook_clone.widget().root()
                        && let Some(window) = root.downcast_ref::<gtk4::Window>()
                    {
                        crate::toast::show_retry_toast_on_window(
                            window,
                            &crate::i18n::i18n("Connection failed. Host unreachable."),
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
#[allow(clippy::too_many_arguments)]
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

    // Create terminal tab for Telnet
    let session_id = notebook.create_terminal_tab_with_settings(
        connection_id,
        &conn.name,
        "telnet",
        Some(&conn.automation),
        &terminal_settings,
        conn.theme_override.as_ref(),
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

    // Get global variables for substitution (secret values resolved from vault)
    let global_variables = state
        .try_borrow()
        .ok()
        .map(|s| crate::state::resolve_global_variables(s.settings()))
        .unwrap_or_default();

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

    // Wire up child exited callback (second call for terminal monitoring)
    MainWindow::setup_child_exited_handler(state, notebook, sidebar, session_id, connection_id);

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
            // Validate configuration before launch
            if let Err(e) = zt_config.validate() {
                tracing::error!(?e, "ZeroTrust config validation failed for {}", conn_name);
                if let Some(root) = notebook.widget().root()
                    && let Some(window) = root.downcast_ref::<gtk4::Window>()
                {
                    crate::toast::show_toast_on_window(
                        window,
                        &format!("Invalid config: {e}"),
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
        conn.theme_override.as_ref(),
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
    let spawn_command = rustconn_core::flatpak::wrap_host_command(&full_command);
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    notebook.spawn_command(
        session_id,
        &[&shell, "-c", &spawn_command],
        None,
        None,
        None,
    );

    Some(session_id)
}

/// Starts a Serial connection
///
/// Creates a terminal tab and spawns picocom with the serial configuration.
/// Shows user-friendly toasts when picocom is not found or device access fails.
#[allow(clippy::too_many_arguments)]
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

    // Create terminal tab for Serial
    let session_id = notebook.create_terminal_tab_with_settings(
        connection_id,
        &conn_name,
        "serial",
        Some(&conn.automation),
        &terminal_settings,
        conn.theme_override.as_ref(),
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
#[allow(clippy::too_many_arguments)]
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
    let kubectl_available = if rustconn_core::flatpak::is_flatpak() {
        rustconn_core::flatpak::is_host_command_available("kubectl")
    } else {
        let kubectl_info = detect_kubectl();
        kubectl_info.installed
    };
    if !kubectl_available {
        tracing::warn!(
            connection = %conn_name,
            flatpak = rustconn_core::flatpak::is_flatpak(),
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

    // Create terminal tab for Kubernetes
    let session_id = notebook.create_terminal_tab_with_settings(
        connection_id,
        &conn_name,
        "kubernetes",
        Some(&conn.automation),
        &terminal_settings,
        conn.theme_override.as_ref(),
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

    // Spawn kubectl via shell
    let spawn_command = rustconn_core::flatpak::wrap_host_command(&kubectl_command);
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
    notebook.spawn_command(
        session_id,
        &[&shell, "-c", &spawn_command],
        None,
        None,
        None,
    );

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
#[allow(clippy::too_many_arguments)]
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
    let should_check = settings.connection.pre_connect_port_check && !conn.skip_port_check;

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
                    if let Some(root) = notebook_clone.widget().root()
                        && let Some(window) = root.downcast_ref::<gtk4::Window>()
                    {
                        crate::toast::show_retry_toast_on_window(
                            window,
                            &crate::i18n::i18n("Connection failed. Host unreachable."),
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

    // Create terminal tab for MOSH
    let session_id = notebook.create_terminal_tab_with_settings(
        connection_id,
        &conn_name,
        "mosh",
        Some(&conn.automation),
        &terminal_settings,
        conn.theme_override.as_ref(),
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
