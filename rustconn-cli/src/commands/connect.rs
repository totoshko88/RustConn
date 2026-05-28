//! Connect command — initiate a connection to a remote server.

use std::path::Path;

use rustconn_core::models::{Connection, ProtocolType};
use rustconn_core::protocol::ProtocolRegistry;

use crate::error::CliError;
use crate::util::{create_config_manager, find_connection};

/// Connect command handler
///
/// # Errors
///
/// Returns:
/// - [`CliError::Config`] when the configuration cannot be read or no connections are configured
/// - [`CliError::ConnectionNotFound`] when no connection matches `name`
/// - [`CliError::Connection`] when the protocol-specific client (ssh, xfreerdp,
///   vncviewer, …) cannot be launched or exits with a non-zero status
pub fn cmd_connect(config_path: Option<&Path>, name: &str, dry_run: bool) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    if connections.is_empty() {
        return Err(CliError::Config(
            "No connections configured. Use 'rustconn-cli add' to create one.".to_string(),
        ));
    }

    let connection = find_connection(&connections, name)?;
    let command = build_connection_command(connection);

    if dry_run {
        println!("{} {}", command.program, command.args.join(" "));
        return Ok(());
    }

    println!(
        "Connecting to '{}' ({} {}:{})...",
        connection.name, connection.protocol, connection.host, connection.port
    );

    execute_connection_command(&command)
}

/// Command to execute for a connection
struct ConnectionCommand {
    /// The program to execute
    program: String,
    /// Command-line arguments
    args: Vec<String>,
}

/// Builds the command arguments for a connection based on its protocol.
///
/// Uses the core `ProtocolRegistry` to delegate command building to each
/// protocol handler's `build_command()` implementation. Falls back to
/// protocol-specific handling for `ZeroTrust` and `Sftp` which require
/// special treatment.
fn build_connection_command(connection: &Connection) -> ConnectionCommand {
    // Sftp opens a file manager, not a CLI command
    if connection.protocol == ProtocolType::Sftp {
        return ConnectionCommand {
            program: "echo".to_string(),
            args: vec![
                "SFTP connections open a file manager. \
                 Use 'rustconn-cli sftp' instead."
                    .to_string(),
            ],
        };
    }

    // Delegate to the core Protocol trait via the registry
    let registry = ProtocolRegistry::new();
    if let Some(handler) = registry.get_by_type(connection.protocol)
        && let Some(cmd_parts) = handler.build_command(connection)
        && let Some((program, args)) = cmd_parts.split_first()
    {
        return ConnectionCommand {
            program: program.clone(),
            args: args.to_vec(),
        };
    }

    // Fallback for protocols without build_command
    ConnectionCommand {
        program: "echo".to_string(),
        args: vec![format!("Unsupported protocol: {}", connection.protocol)],
    }
}

/// Executes the connection command
fn execute_connection_command(command: &ConnectionCommand) -> Result<(), CliError> {
    use std::process::Command;

    let program_check = Command::new("which")
        .arg(&command.program)
        .output()
        .map_err(|e| CliError::Config(format!("Failed to check for {}: {e}", command.program)))?;

    if !program_check.status.success() {
        return Err(CliError::Config(format!(
            "Required program '{}' not found. \
             Please install it to use this connection type.",
            command.program
        )));
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        let mut cmd = Command::new(&command.program);
        cmd.args(&command.args);

        tracing::info!("Executing: {}", format_command_for_log(command));

        let err = cmd.exec();
        Err(CliError::Config(format!(
            "Failed to execute {}: {err}",
            command.program
        )))
    }

    #[cfg(not(unix))]
    {
        let mut cmd = Command::new(&command.program);
        cmd.args(&command.args);

        tracing::info!("Executing: {}", format_command_for_log(command));

        let status = cmd
            .status()
            .map_err(|e| CliError::Config(format!("Failed to execute {}: {e}", command.program)))?;

        if status.success() {
            Ok(())
        } else {
            Err(CliError::Config(format!(
                "{} exited with status: {}",
                command.program,
                status.code().unwrap_or(-1)
            )))
        }
    }
}

/// Returns true if the argument contains a sensitive pattern that should
/// be masked in log output.
fn is_sensitive_arg(arg: &str) -> bool {
    let lower = arg.to_lowercase();
    lower.starts_with("/p:")
        || lower.starts_with("--password")
        || lower.starts_with("-p ")
        || lower.contains("password=")
        || lower.contains("passwd=")
        || lower.contains("secret=")
        || lower.contains("token=")
}

/// Masks the value portion of a sensitive argument, preserving the key
/// prefix for readability.
fn mask_arg(arg: &str) -> String {
    if arg.to_lowercase().starts_with("/p:") {
        return "/p:****".to_string();
    }

    // Handle `--key=value` and `--key value`-style flags.
    for sep in ['=', ' '] {
        if let Some(pos) = arg.find(sep) {
            let prefix = &arg[..=pos];
            return format!("{prefix}****");
        }
    }

    // Fallback: mask the entire argument.
    "****".to_string()
}

/// Formats a connection command for safe log output by masking sensitive
/// arguments such as passwords and tokens.
fn format_command_for_log(command: &ConnectionCommand) -> String {
    let masked_args: Vec<String> = command
        .args
        .iter()
        .map(|arg| {
            if is_sensitive_arg(arg) {
                mask_arg(arg)
            } else {
                arg.clone()
            }
        })
        .collect();

    format!("{} {}", command.program, masked_args.join(" "))
}
