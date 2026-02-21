//! SFTP session command.

use std::path::Path;

use rustconn_core::models::ProtocolType;

use crate::error::CliError;
use crate::util::{create_config_manager, find_connection};

/// Open SFTP session for an SSH connection
#[allow(clippy::too_many_lines)]
pub fn cmd_sftp(
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

    if connection.protocol != ProtocolType::Ssh {
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

    if !rustconn_core::sftp::ensure_key_in_agent(connection) {
        tracing::warn!("Could not add SSH key to agent. You may need to run ssh-add manually.");
    }

    if use_mc {
        let cmd = rustconn_core::sftp::build_mc_sftp_command(connection)
            .ok_or_else(|| CliError::Protocol("Failed to build mc command".to_string()))?;

        println!("Opening mc SFTP for '{}'...", connection.name);

        let mut proc = std::process::Command::new(&cmd[0]);
        proc.args(&cmd[1..]);
        rustconn_core::sftp::apply_agent_env(&mut proc);
        let status = proc.status().map_err(|e| {
            CliError::Connection(format!(
                "Failed to launch mc: {e}. \
                 Is midnight-commander installed?"
            ))
        })?;

        if !status.success() {
            return Err(CliError::Connection(
                "mc session ended with error".to_string(),
            ));
        }
    } else if use_cli {
        let cmd = rustconn_core::sftp::build_sftp_command(connection)
            .ok_or_else(|| CliError::Protocol("Failed to build SFTP command".to_string()))?;

        println!("Connecting via sftp CLI to '{}'...", connection.name);

        let mut proc = std::process::Command::new(&cmd[0]);
        proc.args(&cmd[1..]);
        rustconn_core::sftp::apply_agent_env(&mut proc);
        let status = proc
            .status()
            .map_err(|e| CliError::Connection(format!("Failed to launch sftp: {e}")))?;

        if !status.success() {
            return Err(CliError::Connection(
                "SFTP session ended with error".to_string(),
            ));
        }
    } else {
        let uri = rustconn_core::sftp::build_sftp_uri_from_connection(connection)
            .ok_or_else(|| CliError::Protocol("Failed to build SFTP URI".to_string()))?;

        println!("Opening SFTP file browser for '{}'...", connection.name);
        println!("URI: {uri}");

        // Launch file manager with agent env injected. On KDE,
        // xdg-open routes through D-Bus to an already-running
        // Dolphin that won't have our env.
        let mut proc = std::process::Command::new("dolphin");
        proc.args(["--new-window", &uri]);
        rustconn_core::sftp::apply_agent_env(&mut proc);
        if proc.spawn().is_ok() {
            return Ok(());
        }

        let mut proc = std::process::Command::new("nautilus");
        proc.args(["--new-window", &uri]);
        rustconn_core::sftp::apply_agent_env(&mut proc);
        if proc.spawn().is_ok() {
            return Ok(());
        }

        let mut proc = std::process::Command::new("xdg-open");
        proc.arg(&uri);
        rustconn_core::sftp::apply_agent_env(&mut proc);
        if proc.spawn().is_ok() {
            return Ok(());
        }

        return Err(CliError::Connection(
            "Failed to open file manager. Try --cli to use sftp \
             directly"
                .to_string(),
        ));
    }

    Ok(())
}
