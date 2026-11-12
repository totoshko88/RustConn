//! SFTP session command.

use std::path::Path;

use rustconn_core::models::ProtocolType;

use crate::error::CliError;
use crate::util::{create_config_manager, find_connection};

/// Runs Midnight Commander against the connection's remote host.
///
/// mc's `sh://` VFS shells out to bare `ssh` with a fixed argument list, so the
/// connection's jump host, identity file and custom options can only reach it
/// through a generated `ssh_config` behind a `PATH` wrapper (issue #255).
///
/// # Errors
///
/// Returns [`CliError::Protocol`] if the connection is not SSH-family and
/// [`CliError::Connection`] if mc cannot be launched or exits non-zero.
fn run_mc(
    connection: &rustconn_core::Connection,
    connections: &[rustconn_core::Connection],
    groups: &[rustconn_core::ConnectionGroup],
    network: &rustconn_core::NetworkSettings,
) -> Result<(), CliError> {
    let cmd = rustconn_core::sftp::build_mc_sftp_command(connection)
        .ok_or_else(|| CliError::Protocol("Failed to build mc command".to_string()))?;

    println!("Opening mc SFTP for '{}'...", connection.name);

    let mut proc = std::process::Command::new(&cmd[0]);
    proc.args(&cmd[1..]);
    rustconn_core::sftp::apply_agent_env(&mut proc);

    // A fresh id per invocation keeps two concurrent runs against the same
    // connection from tearing down each other's wrapper directory.
    let wrapper_id = uuid::Uuid::new_v4();
    // No password: the CLI does not read the secret backend for mc, so a
    // password-authenticated connection is typed in mc's terminal.
    let mc_ssh = rustconn_core::prepare_mc_ssh_env(
        wrapper_id,
        connection,
        connections,
        groups,
        network,
        None,
    );
    if let Some(ref env) = mc_ssh {
        proc.env("PATH", env.path_env().trim_start_matches("PATH="));
    }

    let status = proc.status().map_err(|e| {
        CliError::Connection(format!(
            "Failed to launch mc: {e}. \
             Is midnight-commander installed?"
        ))
    });
    // Runs whether mc started or not, so a failed spawn leaves nothing behind.
    if mc_ssh.is_some() {
        rustconn_core::cleanup_mc_ssh_env(wrapper_id);
    }

    if status?.success() {
        Ok(())
    } else {
        Err(CliError::Connection(
            "mc session ended with error".to_string(),
        ))
    }
}

/// Opens the connection's remote filesystem in a desktop file manager.
///
/// Tries Dolphin, then Nautilus, then the platform URL opener, injecting the
/// agent environment into each — on KDE, `xdg-open` routes through D-Bus to an
/// already-running Dolphin that would not otherwise see it.
///
/// # Errors
///
/// Returns [`CliError::Protocol`] if the browser URI cannot be built and
/// [`CliError::Connection`] if no file manager could be launched.
#[cfg(feature = "client-launch")]
fn open_in_file_manager(
    connection: &rustconn_core::Connection,
    connections: &[rustconn_core::Connection],
    groups: &[rustconn_core::ConnectionGroup],
    network: &rustconn_core::NetworkSettings,
) -> Result<(), CliError> {
    // Resolve the login home directory so the file manager opens where the
    // user has access instead of the server root (issue #212).
    let uri = rustconn_core::sftp::build_sftp_browser_uri(connection, connections, groups, network)
        .ok_or_else(|| CliError::Protocol("Failed to build SFTP URI".to_string()))?;

    // GVFS spawns its own `ssh` with a hardcoded argument list and reads no
    // configuration we can point it at, so a bastion cannot be applied on this
    // path at all.
    if !rustconn_core::connection::resolve_jump_chain(connection, connections, groups, network)
        .is_empty()
    {
        tracing::warn!(
            name = %connection.name,
            "This connection uses a jump host, which the file-manager \
             path cannot route through. Use --mc or --cli instead."
        );
    }

    tracing::info!(name = %connection.name, %uri, "Opening SFTP file browser");

    for launcher in ["dolphin", "nautilus"] {
        let mut proc = std::process::Command::new(launcher);
        proc.args(["--new-window", &uri]);
        rustconn_core::sftp::apply_agent_env(&mut proc);
        if proc.spawn().is_ok() {
            return Ok(());
        }
    }

    let mut proc = std::process::Command::new(rustconn_core::secret::url_open_command());
    proc.arg(&uri);
    rustconn_core::sftp::apply_agent_env(&mut proc);
    if proc.spawn().is_ok() {
        return Ok(());
    }

    Err(CliError::Connection(
        "Failed to open file manager. Try --cli to use sftp directly".to_string(),
    ))
}

/// Open SFTP session for an SSH connection
///
/// # Errors
///
/// Returns:
/// - [`CliError::Config`] when connections cannot be loaded
/// - [`CliError::ConnectionNotFound`] when no connection matches `name`
/// - [`CliError::Protocol`] when the connection is not an SSH connection
/// - [`CliError::Connection`] when the SFTP client (sftp / Midnight Commander /
///   GIO file manager) cannot be launched
pub(super) fn cmd_sftp(
    config_path: Option<&Path>,
    name: &str,
    use_cli: bool,
    use_mc: bool,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let connection = find_connection(&connections, name)?;

    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;

    // The outermost bastion tier: a global ProxyJump or jump host applies to
    // every connection that inherits, including this one.
    let network = config_manager
        .load_settings()
        .map(|settings| settings.network)
        .unwrap_or_default();

    // A connection whose protocol *is* SFTP carries the same `SshConfig` and is
    // exactly what this command is for; only rejecting non-SSH-family protocols
    // is correct here.
    if !matches!(connection.protocol, ProtocolType::Ssh | ProtocolType::Sftp) {
        return Err(CliError::Protocol(format!(
            "SFTP is only available for SSH connections, '{}' uses {}",
            connection.name,
            connection.protocol.as_str()
        )));
    }

    if let Some(info) = rustconn_core::sftp::ensure_ssh_agent() {
        rustconn_core::sftp::set_agent_info(info);
    } else {
        tracing::warn!("ssh-agent is not running. SFTP may require manual setup.");
    }

    if !rustconn_core::sftp::ensure_key_in_agent(connection, &groups) {
        tracing::warn!("Could not add SSH key to agent. You may need to run ssh-add manually.");
    }

    if use_mc || use_cli {
        // Fallback jump hosts: the file-manager path above cannot use a bastion.
        use rustconn_core::connection::gateway_failover::{
            GatewayOutcome, gateway_probes_by, probe_gateways, use_gateway,
        };
        let probes = gateway_probes_by(
            connection,
            |id| connections.iter().find(|c| c.id == id),
            &groups,
            &network,
        );
        let timeout = config_manager
            .load_settings()
            .unwrap_or_default()
            .connection
            .port_check_timeout_secs;
        let mut routed = connection.clone();
        if let Some(selection) = probe_gateways(&probes, timeout) {
            use_gateway(&mut routed, selection.gateway_id);
            match selection.outcome {
                GatewayOutcome::Primary => {}
                GatewayOutcome::FailedOver => {
                    eprintln!(
                        "Jump host unreachable, connecting through '{}'.",
                        selection.gateway_name
                    );
                }
                GatewayOutcome::NoneReachable => {
                    eprintln!(
                        "No jump host responded, trying '{}'.",
                        selection.gateway_name
                    );
                }
            }
        }
        let connection = &routed;

        let result = if use_mc {
            run_mc(connection, &connections, &groups, &network)
        } else {
            run_sftp_cli(connection, &connections, &groups, &network)
        };
        // sftp and mc have exited, so the key added above for them leaves the
        // agent now instead of staying for its whole lifetime. Only a key this
        // command added is removed; one the agent already held is left alone.
        //
        // That before/after check is what keeps the GUI's keys safe: a key the
        // GUI (or the user) loaded before this command ran is in the agent's
        // "before" answer, so it is neither added nor recorded here. The other
        // order is a known limit: each process keeps its own record and they
        // do not talk, so when the GUI needs a key while this command is the
        // one that added it, the GUI finds it in the agent and leaves it
        // alone, and this eviction then takes it from the GUI's sessions too.
        // The GUI's next connect or launch adds it again. Not worth IPC.
        let removed = rustconn_core::ssh_agent::evict_own_keys();
        tracing::debug!(
            removed,
            "removed the SSH keys this command added to the agent"
        );
        return result;
    }

    // The file manager is detached and outlives this command, so the key stays
    // in the agent for it, bounded by its lifetime.
    #[cfg(feature = "client-launch")]
    {
        open_in_file_manager(connection, &connections, &groups, &network)
    }

    #[cfg(not(feature = "client-launch"))]
    {
        Err(CliError::Connection(
            "SFTP file-browser launch is not compiled in this minimal CLI. \
             Use --cli or --mc, or build with the client-launch feature."
                .to_string(),
        ))
    }
}

/// Runs the `sftp` client against the connection and waits for it to exit.
///
/// # Errors
///
/// Returns [`CliError::Protocol`] if the command cannot be built and
/// [`CliError::Connection`] if sftp cannot be launched or exits non-zero.
fn run_sftp_cli(
    connection: &rustconn_core::Connection,
    connections: &[rustconn_core::Connection],
    groups: &[rustconn_core::ConnectionGroup],
    network: &rustconn_core::NetworkSettings,
) -> Result<(), CliError> {
    let cmd = rustconn_core::sftp::build_sftp_command(connection, connections, groups, network)
        .ok_or_else(|| CliError::Protocol("Failed to build SFTP command".to_string()))?;

    println!("Connecting via sftp CLI to '{}'...", connection.name);

    let mut proc = std::process::Command::new(&cmd[0]);
    proc.args(&cmd[1..]);
    rustconn_core::sftp::apply_agent_env(&mut proc);
    let status = proc
        .status()
        .map_err(|e| CliError::Connection(format!("Failed to launch sftp: {e}")))?;

    if status.success() {
        Ok(())
    } else {
        Err(CliError::Connection(
            "SFTP session ended with error".to_string(),
        ))
    }
}
