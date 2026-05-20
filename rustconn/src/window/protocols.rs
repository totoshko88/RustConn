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
use rustconn_core::connection::automation_inheritance;
use rustconn_core::connection::check_port;
use rustconn_core::connection::ssh_inheritance;
use rustconn_core::models::AutomationConfig;
use rustconn_core::variables::{Variable, VariableManager, VariableScope};
use secrecy::SecretString;
use std::rc::Rc;
use uuid::Uuid;

/// Type alias for shared sidebar reference
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Type alias for shared notebook reference
pub type SharedNotebook = Rc<TerminalNotebook>;

/// Resolves the effective automation config for a connection, inheriting from
/// the group hierarchy if the connection has no own expect rules / post-login scripts.
fn resolve_automation_for_connection(
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

/// Delegates to [`rustconn_core::ssh_tunnel::append_proxy_command_destination`].
fn append_proxy_command_destination(proxy_parts: &mut Vec<String>, jump_host: &str) {
    rustconn_core::ssh_tunnel::append_proxy_command_destination(proxy_parts, jump_host);
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
    if let Some(kh_path) = rustconn_core::get_flatpak_known_hosts_path() {
        // Build ProxyCommand for the first hop in the chain
        let mut proxy_parts = vec!["ssh".to_string(), "-W".to_string(), "%h:%p".to_string()];
        proxy_parts.push("-o".to_string());
        proxy_parts.push(format!("UserKnownHostsFile={}", kh_path.display()));

        // Pass identity file for the first hop if available
        if let Some(ref key) = first_hop_identity {
            proxy_parts.push("-i".to_string());
            proxy_parts.push(key.clone());
            proxy_parts.push("-o".to_string());
            proxy_parts.push("IdentitiesOnly=yes".to_string());
        }

        // If multiple hops, pass remaining via -J inside ProxyCommand
        if chain.len() > 1 {
            let inner_chain = chain[1..].join(",");
            proxy_parts.push("-J".to_string());
            proxy_parts.push(inner_chain);
        }

        // Add the first hop destination with proper -p port parsing
        append_proxy_command_destination(&mut proxy_parts, &chain[0]);

        let proxy_cmd = proxy_parts.join(" ");
        tracing::debug!(
            proxy_command = %proxy_cmd,
            "Tunnel: using ProxyCommand for jump host chain in Flatpak"
        );
        vec!["-o".to_string(), format!("ProxyCommand={proxy_cmd}")]
    } else {
        // Non-Flatpak: use standard -J
        let j_chain = chain.join(",");
        tracing::debug!(
            jump_chain = %j_chain,
            "Tunnel: using -J for jump host chain"
        );
        vec!["-J".to_string(), j_chain]
    }
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
    // Collect groups for SSH inheritance resolution (proxy_jump can be inherited from group)
    let groups: Vec<rustconn_core::ConnectionGroup> = state
        .try_borrow()
        .ok()
        .map(|s| s.list_groups().into_iter().cloned().collect())
        .unwrap_or_default();
    let has_inherited_proxy = ssh_inheritance::resolve_ssh_proxy_jump(conn, &groups).is_some();
    // Use centralized probe-bypass logic + inherited proxy jump from groups
    let should_check = conn.should_pre_connect_check(&settings.connection) && !has_inherited_proxy;

    if conn.bypasses_direct_probe() || has_inherited_proxy {
        tracing::debug!(
            protocol = "ssh",
            host = %conn.host,
            port = conn.port,
            "Skipping port check — connection bypasses direct probe"
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

    // Get global variables for substitution (secret values resolved from vault)
    let global_variables = state
        .try_borrow()
        .ok()
        .map(|s| crate::state::resolve_global_variables(s.settings()))
        .unwrap_or_default();

    // Resolve automation config with group inheritance
    let resolved_automation = resolve_automation_for_connection(state, conn);

    // Create terminal tab for SSH with user settings
    let session_id = notebook.create_terminal_tab_with_settings(
        connection_id,
        &conn.name,
        "ssh",
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

    // Store history entry ID in session for later use
    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Build and spawn SSH command
    let port = conn.port;

    // Collect groups for SSH inheritance resolution
    let groups: Vec<rustconn_core::ConnectionGroup> = state
        .try_borrow()
        .ok()
        .map(|s| s.list_groups().into_iter().cloned().collect())
        .unwrap_or_default();

    // Detect jump host / proxy for status detection and monitoring
    let has_jump_host = matches!(
        &conn.protocol_config,
        rustconn_core::ProtocolConfig::Ssh(ssh)
            if ssh.jump_host_id.is_some() || ssh.proxy_command.is_some()
    ) || ssh_inheritance::resolve_ssh_proxy_jump(conn, &groups).is_some();

    // Apply variable substitution to host and username (e.g., ${VAR_NAME} -> actual value)
    let host = substitute_variables(&conn.host, &global_variables);
    let username = conn
        .username
        .as_ref()
        .map(|u| substitute_variables(u, &global_variables));

    // Get SSH-specific options
    let (identity_file, extra_args, use_waypipe, jump_host_chain) =
        if let rustconn_core::ProtocolConfig::Ssh(ssh_config) = &conn.protocol_config {
            // Resolve key path via inheritance (connection → group → parent group → root)
            let key = ssh_inheritance::resolve_ssh_key_path(conn, &groups)
                .and_then(|p| {
                    // Resolve stale portal paths: if the stored path doesn't exist,
                    // check the Flatpak SSH dir for a file with the same name.
                    rustconn_core::resolve_key_path(&p)
                })
                .map(|p| p.to_string_lossy().to_string());

            // Use build_command_args() for all SSH-specific flags:
            // identity, IdentitiesOnly, proxy_jump, ControlMaster/Persist,
            // agent forwarding, X11, compression, custom options, port forwards
            let mut args = ssh_config.build_command_args();

            // Remove -i <path> from args because the identity file is already
            // resolved separately via resolve_ssh_key_path() and passed as
            // `identity_file` to spawn_ssh(). Keeping both causes the key to
            // appear twice in the final command line.
            if key.is_some()
                && let Some(pos) = args.iter().position(|a| a == "-i")
            {
                args.remove(pos); // remove "-i"
                if pos < args.len() {
                    args.remove(pos); // remove the path value
                }
            }

            // Resolve jump host chain from connection references (needs state access)
            let mut jump_hosts = Vec::new();

            // Handle string-based proxy jump (legacy/manual or inherited from group)
            if let Some(proxy) = ssh_inheritance::resolve_ssh_proxy_jump(conn, &groups) {
                jump_hosts.push(proxy);
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

                    // Add the first hop destination (parse user@host:port into -p port user@host)
                    append_proxy_command_destination(&mut proxy_parts, &jump_hosts[0]);

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
        let agent_socket = ssh_inheritance::resolve_ssh_agent_socket(conn, &groups);
        let startup_cmd = match &conn.protocol_config {
            rustconn_core::ProtocolConfig::Ssh(cfg) => cfg.startup_command.as_deref(),
            _ => None,
        };
        notebook.spawn_ssh(
            session_id,
            &host,
            port,
            username.as_deref(),
            identity_file.as_deref(),
            &extra_refs,
            use_waypipe,
            agent_socket.as_deref(),
            startup_cmd,
        );
    }

    // --- VTE password injection: detect "password:" prompt and feed cached password ---
    // This replaces the previous sshpass dependency. The terminal output is
    // monitored for SSH password prompts; when detected, the vault password
    // is sent via feed_child() exactly once.
    // NOTE: Passphrase prompts ("Enter passphrase for key") are explicitly
    // excluded to avoid sending the wrong secret when SSH auth is PublicKey.
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
            let lower = text.to_lowercase();

            // Reject passphrase prompts — these need key_passphrase, not password
            let last_line = lower.lines().last().unwrap_or("").trim();
            if last_line.contains("passphrase for key") || last_line.contains("passphrase for") {
                return;
            }

            // Check for SSH password prompts in multiple languages (case-insensitive)
            let has_prompt = lower.ends_with("password: ")
                || lower.ends_with("password:")
                || lower.contains("password: \n")
                || lower.lines().last().is_some_and(|line| {
                    let l = line.trim().to_lowercase();
                    l.ends_with("password:")
                        || l.ends_with("password: ")
                        || l.contains("'s password:")
                        // German
                        || l.ends_with("passwort:")
                        || l.ends_with("passwort: ")
                        || l.ends_with("kennwort:")
                        || l.ends_with("kennwort: ")
                        // French
                        || l.ends_with("mot de passe:")
                        || l.ends_with("mot de passe :")
                        || l.ends_with("mot de passe : ")
                        // Spanish
                        || l.ends_with("contraseña:")
                        || l.ends_with("contraseña: ")
                        // Portuguese
                        || l.ends_with("senha:")
                        || l.ends_with("senha: ")
                        // Italian
                        || l.ends_with("password:")
                        // Ukrainian / Belarusian
                        || l.ends_with("пароль:")
                        || l.ends_with("пароль: ")
                        // Polish
                        || l.ends_with("hasło:")
                        || l.ends_with("hasło: ")
                        // Czech/Slovak
                        || l.ends_with("heslo:")
                        || l.ends_with("heslo: ")
                        // Dutch
                        || l.ends_with("wachtwoord:")
                        || l.ends_with("wachtwoord: ")
                        // Swedish/Danish/Norwegian
                        || l.ends_with("lösenord:")
                        || l.ends_with("lösenord: ")
                        || l.ends_with("adgangskode:")
                        || l.ends_with("adgangskode: ")
                        // Chinese
                        || l.ends_with("密码:")
                        || l.ends_with("密码：")
                        || l.ends_with("密碼:")
                        || l.ends_with("密碼：")
                        // Japanese
                        || l.ends_with("パスワード:")
                        || l.ends_with("パスワード：")
                        // Korean
                        || l.ends_with("비밀번호:")
                        || l.ends_with("비밀번호：")
                        // Generic colon-terminated prompt (catch-all for PAM)
                        || l.ends_with("pass:")
                        || l.ends_with("pass: ")
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
            let identity_file_mon = ssh_inheritance::resolve_ssh_key_path(conn, &groups)
                .and_then(|p| rustconn_core::resolve_key_path(&p))
                .map(|p| p.to_string_lossy().to_string());
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
    let (effective_host, effective_port, ssh_tunnel) = if let Some(ref opts) = spice_opts
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
#[allow(clippy::too_many_arguments)]
pub fn reconnect_ssh_in_place(
    state: &SharedAppState,
    notebook: &SharedNotebook,
    sidebar: &SharedSidebar,
    monitoring: &super::types::SharedMonitoring,
    session_id: Uuid,
    connection_id: Uuid,
) -> bool {
    use rustconn_core::protocol::{format_command_message, format_connection_message};

    // Prepare the existing tab for reconnect
    if !notebook.prepare_for_reconnect(session_id) {
        tracing::warn!(%session_id, "Tab no longer exists, cannot reconnect in-place");
        return false;
    }

    // Show "connecting" status in sidebar immediately
    sidebar.update_connection_status(&connection_id.to_string(), "connecting");

    // Get connection data
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
    let history_entry_id = if let Ok(mut state_mut) = state.try_borrow_mut() {
        Some(state_mut.record_connection_start(&conn, conn.username.as_deref()))
    } else {
        None
    };
    if let Some(entry_id) = history_entry_id {
        notebook.set_history_entry_id(session_id, entry_id);
    }

    // Get global variables for substitution
    let global_variables = state
        .try_borrow()
        .ok()
        .map(|s| crate::state::resolve_global_variables(s.settings()))
        .unwrap_or_default();

    let host = substitute_variables(&conn.host, &global_variables);
    let username = conn
        .username
        .as_ref()
        .map(|u| substitute_variables(u, &global_variables));

    // Collect groups for SSH inheritance resolution
    let groups: Vec<rustconn_core::ConnectionGroup> = state
        .try_borrow()
        .ok()
        .map(|s| s.list_groups().into_iter().cloned().collect())
        .unwrap_or_default();

    let has_jump_host = matches!(
        &conn.protocol_config,
        rustconn_core::ProtocolConfig::Ssh(ssh)
            if ssh.jump_host_id.is_some() || ssh.proxy_command.is_some()
    ) || ssh_inheritance::resolve_ssh_proxy_jump(&conn, &groups).is_some();

    // Build SSH args (same logic as start_ssh_connection_internal)
    let (identity_file, extra_args, use_waypipe, jump_host_chain) =
        if let rustconn_core::ProtocolConfig::Ssh(ssh_config) = &conn.protocol_config {
            // Resolve key path via inheritance (connection → group → parent group → root)
            let key = ssh_inheritance::resolve_ssh_key_path(&conn, &groups)
                .and_then(|p| rustconn_core::resolve_key_path(&p))
                .map(|p| p.to_string_lossy().to_string());

            let mut args = ssh_config.build_command_args();

            // Remove -i <path> from args because the identity file is already
            // resolved separately via resolve_ssh_key_path() and passed as
            // `identity_file` to spawn_ssh(). Keeping both causes the key to
            // appear twice in the final command line.
            if key.is_some()
                && let Some(pos) = args.iter().position(|a| a == "-i")
            {
                args.remove(pos); // remove "-i"
                if pos < args.len() {
                    args.remove(pos); // remove the path value
                }
            }

            let mut jump_hosts = Vec::new();
            // Handle string-based proxy jump (legacy/manual or inherited from group)
            if let Some(proxy) = ssh_inheritance::resolve_ssh_proxy_jump(&conn, &groups) {
                jump_hosts.push(proxy);
            }
            if let Some(jump_id) = ssh_config.jump_host_id
                && let Ok(state_ref) = state.try_borrow()
            {
                let mut current_id = Some(jump_id);
                let mut visited = std::collections::HashSet::new();
                visited.insert(connection_id);
                for _ in 0..10 {
                    if let Some(jid) = current_id {
                        if visited.contains(&jid) {
                            break;
                        }
                        visited.insert(jid);
                        if let Some(jump_conn) = state_ref.get_connection(jid) {
                            let mut host_str = jump_conn.host.clone();
                            if let Some(user) = &jump_conn.username {
                                host_str = format!("{}@{}", user, host_str);
                            }
                            if jump_conn.port != 22 {
                                host_str = format!("{}:{}", host_str, jump_conn.port);
                            }
                            jump_hosts.push(host_str);
                            if let rustconn_core::ProtocolConfig::Ssh(jump_config) =
                                &jump_conn.protocol_config
                            {
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
                args.push("-o".to_string());
                args.push(format!("UserKnownHostsFile={}", kh_path.display()));
            }

            let jump_host_str = if jump_hosts.is_empty() {
                None
            } else {
                if ssh_config.proxy_jump.is_some()
                    && let Some(pos) = args.iter().position(|a| a == "-J")
                {
                    args.remove(pos);
                    if pos < args.len() {
                        args.remove(pos);
                    }
                }
                let chain = jump_hosts.join(",");
                if flatpak_known_hosts.is_some() {
                    let mut proxy_parts =
                        vec!["ssh".to_string(), "-W".to_string(), "%h:%p".to_string()];
                    if let Some(pos) = args.iter().position(|a| a == "-i")
                        && let Some(key_path) = args.get(pos + 1)
                    {
                        proxy_parts.push("-i".to_string());
                        proxy_parts.push(key_path.clone());
                        proxy_parts.push("-o".to_string());
                        proxy_parts.push("IdentitiesOnly=yes".to_string());
                    }
                    if let Some(ref kh_path) = flatpak_known_hosts {
                        proxy_parts.push("-o".to_string());
                        proxy_parts.push(format!("UserKnownHostsFile={}", kh_path.display()));
                    }
                    if jump_hosts.len() > 1 {
                        let inner_chain = jump_hosts[1..].join(",");
                        proxy_parts.push("-J".to_string());
                        proxy_parts.push(inner_chain);
                    }
                    append_proxy_command_destination(&mut proxy_parts, &jump_hosts[0]);
                    let proxy_cmd = proxy_parts.join(" ");
                    args.push("-o".to_string());
                    args.push(format!("ProxyCommand={proxy_cmd}"));
                } else {
                    args.push("-J".to_string());
                    args.push(chain.clone());
                }
                Some(chain)
            };

            let waypipe = ssh_config.waypipe && rustconn_core::protocol::detect_waypipe().installed;
            (key, args, waypipe, jump_host_str)
        } else {
            (None, Vec::new(), false, None)
        };

    // Re-wire child-exited handler for the new process
    MainWindow::setup_child_exited_handler(state, notebook, sidebar, session_id, connection_id);

    // Build SSH command string for display
    let port = conn.port;
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

    // Display CLI output feedback
    let conn_msg = format_connection_message("SSH", &host);
    let cmd_msg = format_command_message(&ssh_command);
    let feedback = format!("{conn_msg}\r\n{cmd_msg}\r\n\r\n");
    notebook.display_output(session_id, &feedback);

    // Retrieve cached credentials
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

    // Spawn SSH in the existing terminal
    {
        let extra_refs: Vec<&str> = extra_args.iter().map(std::string::String::as_str).collect();
        let agent_socket = ssh_inheritance::resolve_ssh_agent_socket(&conn, &groups);
        let startup_cmd = match &conn.protocol_config {
            rustconn_core::ProtocolConfig::Ssh(cfg) => cfg.startup_command.as_deref(),
            _ => None,
        };
        notebook.spawn_ssh(
            session_id,
            &host,
            port,
            username.as_deref(),
            identity_file.as_deref(),
            &extra_refs,
            use_waypipe,
            agent_socket.as_deref(),
            startup_cmd,
        );
    }

    // VTE password injection
    // NOTE: Passphrase prompts ("Enter passphrase for key") are explicitly
    // excluded to avoid sending the wrong secret when SSH auth is PublicKey.
    if let Some(vault_password) = cached_password {
        let notebook_clone = notebook.clone();
        let password_sent = std::rc::Rc::new(std::cell::Cell::new(false));
        let password_sent_clone = password_sent.clone();

        notebook.connect_contents_changed(session_id, move || {
            if password_sent_clone.get() {
                return;
            }
            let Some(text) = notebook_clone.get_terminal_text(session_id) else {
                return;
            };
            let lower = text.to_lowercase();

            // Reject passphrase prompts — these need key_passphrase, not password
            let last_line = lower.lines().last().unwrap_or("").trim();
            if last_line.contains("passphrase for key") || last_line.contains("passphrase for") {
                return;
            }

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
                let input = format!("{pw}\n");
                notebook_clone.send_text_to_session(session_id, &input);
                password_sent_clone.set(true);
            }
        });
    }

    // SSH status detection
    {
        let sidebar_clone = sidebar.clone();
        let notebook_clone = notebook.clone();
        let connection_id_str = connection_id.to_string();
        let session_connected = std::rc::Rc::new(std::cell::Cell::new(false));
        let session_connected_clone = session_connected.clone();
        let uses_jump_host = has_jump_host;

        notebook.connect_contents_changed(session_id, move || {
            if session_connected_clone.get() {
                return;
            }
            if let Some(row) = notebook_clone.get_terminal_cursor_row(session_id)
                && row > 2
            {
                if uses_jump_host
                    && let Some(text) = notebook_clone.get_terminal_text(session_id)
                    && contains_ssh_failure(&text)
                {
                    return;
                }
                sidebar_clone.increment_session_count(&connection_id_str);
                session_connected_clone.set(true);
            }
        });
    }

    // Deferred monitoring start
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
            let identity_file_mon = ssh_inheritance::resolve_ssh_key_path(&conn, &groups)
                .and_then(|p| rustconn_core::resolve_key_path(&p))
                .map(|p| p.to_string_lossy().to_string());
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
            let mon_jump_host = jump_host_chain;
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

    // Update last_connected timestamp
    if let Ok(mut state_mut) = state.try_borrow_mut()
        && let Err(e) = state_mut.update_last_connected(connection_id)
    {
        tracing::warn!(?e, "Failed to update last_connected");
    }

    true
}

/// Reconnects a generic VTE session (Telnet, Serial, Kubernetes, ZeroTrust, MOSH)
/// in-place, reusing the existing terminal tab.
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

            let spawn_cmd = command.join(" ");
            let wrapped = rustconn_core::flatpak::wrap_host_command(&spawn_cmd);
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
            notebook.spawn_command(session_id, &[&shell, "-c", &wrapped], None, None, None);
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
        let spawn_cmd = format!(
            "flatpak-spawn --host --env=TERM=xterm-256color -- script -qfc '{escaped}' /dev/null"
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
