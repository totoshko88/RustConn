//! Connect command — initiate a connection to a remote server.

use std::path::Path;

use rustconn_core::models::{Connection, ProtocolType};
use rustconn_core::protocol::{
    ProtocolRegistry, contains_freerdp_secret_field, freerdp_secret_field_takes_following_value,
};

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

    // Dangling-bastion warning (#345): a `jump_host_id` — the connection's own,
    // or one inherited from a group or the global network settings — can point
    // at a connection that has since been deleted. The launch path silently
    // drops such a hop and connects direct, the one outcome a bastion exists to
    // prevent. Warn (to stderr, so stdout and --dry-run output stay clean) and
    // then proceed: warn-and-direct, matching the GUI. The group/network loads
    // are best-effort — this is advisory and must never block a connection.
    let groups = config_manager.load_groups().unwrap_or_default();
    let network = config_manager
        .load_settings()
        .map(|settings| settings.network)
        .unwrap_or_default();
    for dangling in rustconn_core::connection::jump_chain::find_dangling_bastions(
        connection,
        &connections,
        &groups,
        &network,
    ) {
        let where_set = match dangling.origin {
            rustconn_core::connection::jump_chain::BastionRefOrigin::Connection => {
                "its own Jump Host setting".to_string()
            }
            rustconn_core::connection::jump_chain::BastionRefOrigin::Group(_) => {
                "an inherited group Jump Host setting".to_string()
            }
            rustconn_core::connection::jump_chain::BastionRefOrigin::Network => {
                "the global Network Jump Host setting".to_string()
            }
        };
        eprintln!(
            "Warning: jump host {} is missing (referenced by {where_set}); \
             connecting directly.",
            dangling.missing_id
        );
    }

    // Kerberos preflight (#351): when a connection connects RDP NLA with
    // Kerberos, a few settings make the sign-in fail with errors that do not
    // point back at them — an IP address instead of a DNS name, a NetBIOS realm
    // instead of the DNS domain, or no domain at all. The GUI names these up
    // front; the CLI prints the same hints to stderr and connects anyway
    // (warn-and-connect), so stdout and --dry-run output stay clean.
    if let rustconn_core::models::ProtocolConfig::Rdp(rdp) = &connection.protocol_config
        && rdp.kerberos_enabled
    {
        for hint in rustconn_core::rdp_client::kerberos_preflight(
            &connection.host,
            connection.username.as_deref(),
            connection.domain.as_deref(),
        ) {
            let detail = match hint {
                rustconn_core::rdp_client::KerberosHint::HostNotDnsName => {
                    "Kerberos needs the server's DNS name; an IP address or an SSH tunnel fails"
                }
                rustconn_core::rdp_client::KerberosHint::ShortDomainName => {
                    "Kerberos needs the DNS domain (e.g. EXAMPLE.COM), not the short domain name"
                }
                rustconn_core::rdp_client::KerberosHint::MissingDomain => {
                    "Kerberos needs the DNS domain; set it in the connection's domain field"
                }
            };
            eprintln!("Warning: {detail}.");
        }
    }

    let command = build_connection_command(connection);

    if dry_run {
        let formatted = format_command_for_log(&command);
        println!("{formatted}");
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
/// protocol handler's `build_command()` implementation. `Sftp` is handled
/// specially because it opens a file manager rather than a CLI command.
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

    if !rustconn_core::which::is_available(&command.program) {
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
    contains_freerdp_secret_field(arg)
        || lower.starts_with("--password")
        || lower == "--passwd"
        || lower == "-p"
        || lower.starts_with("-p ")
        || lower.starts_with("--token")
        || lower.starts_with("--secret")
        || lower.contains("password=")
        || lower.contains("passwd=")
        || lower.contains("secret=")
        || lower.contains("token=")
}

fn sensitive_arg_takes_following_value(arg: &str) -> bool {
    let lower = arg.trim().to_ascii_lowercase();
    freerdp_secret_field_takes_following_value(arg)
        || matches!(
            lower.as_str(),
            "-p" | "--password" | "--passwd" | "--secret" | "--token"
        )
}

/// Masks the value portion of a sensitive argument, preserving the key
/// prefix for readability.
fn mask_arg(arg: &str) -> String {
    if contains_freerdp_secret_field(arg) && !arg.trim_start().starts_with('-') {
        return String::from("****");
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
    let mut mask_following_value = false;
    let masked_args: Vec<String> = command
        .args
        .iter()
        .map(|arg| {
            if mask_following_value {
                mask_following_value = false;
                return String::from("****");
            }
            if is_sensitive_arg(arg) {
                mask_following_value = sensitive_arg_takes_following_value(arg);
                mask_arg(arg)
            } else {
                arg.clone()
            }
        })
        .collect();

    format!("{} {}", command.program, masked_args.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freerdp_secret_aliases_are_fully_masked() {
        for argument in [
            " /P:session-secret",
            "//PASSWORD:password-secret",
            "/gateway:g:host, /Gp:gateway-secret",
            "/gateway:g:host,//GATEWAY-PASSWORD:alias-secret",
            "/gateway:g:host,PTH:hash-secret",
            "/p whitespace-secret",
            "/gateway:g:host,gp whitespace-gateway-secret",
        ] {
            let command = ConnectionCommand {
                program: "xfreerdp".to_string(),
                args: vec![argument.to_string()],
            };

            assert!(is_sensitive_arg(argument));
            assert_eq!(format_command_for_log(&command), "xfreerdp ****");
        }
    }

    #[test]
    fn formatted_output_never_contains_freerdp_secret_values() {
        let command = ConnectionCommand {
            program: "xfreerdp".to_string(),
            args: vec![
                "/gateway:g:host,p:composite-password".to_string(),
                "/PASSWORD:top-level-password".to_string(),
                "/u:alice".to_string(),
            ],
        };

        let formatted = format_command_for_log(&command);

        assert_eq!(formatted, "xfreerdp **** **** /u:alice");
        assert!(!formatted.contains("composite-password"));
        assert!(!formatted.contains("top-level-password"));
    }

    #[test]
    fn split_sensitive_values_are_masked() {
        let command = ConnectionCommand {
            program: "client".to_string(),
            args: vec![
                "--password".to_string(),
                "split-password".to_string(),
                "--token".to_string(),
                "split-token".to_string(),
                "/gp".to_string(),
                "gateway-password".to_string(),
                "--user=alice".to_string(),
            ],
        };

        let formatted = format_command_for_log(&command);
        assert_eq!(
            formatted,
            "client **** **** **** **** **** **** --user=alice"
        );
        assert!(!formatted.contains("split-password"));
        assert!(!formatted.contains("split-token"));
        assert!(!formatted.contains("gateway-password"));
    }

    #[test]
    fn generic_password_and_token_arguments_remain_masked() {
        let command = ConnectionCommand {
            program: "ssh-client".to_string(),
            args: vec![
                "--password=ssh-secret".to_string(),
                "token=api-secret".to_string(),
                "--user=alice".to_string(),
            ],
        };

        assert_eq!(
            format_command_for_log(&command),
            "ssh-client --password=**** token=**** --user=alice"
        );
    }
}
