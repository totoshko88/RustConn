//! Connect command — initiate a connection to a remote server.

use std::path::Path;

use rustconn_core::models::{Connection, ProtocolType};

use crate::error::CliError;
use crate::util::{create_config_manager, find_connection};

/// Connect command handler
pub fn cmd_connect(config_path: Option<&Path>, name: &str, dry_run: bool) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    if connections.is_empty() {
        return Err(CliError::ConnectionNotFound(
            "No connections configured".to_string(),
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

/// Builds the command arguments for a connection based on its protocol
fn build_connection_command(connection: &Connection) -> ConnectionCommand {
    match connection.protocol {
        ProtocolType::Ssh => build_ssh_command(connection),
        ProtocolType::Rdp => build_rdp_command(connection),
        ProtocolType::Vnc => build_vnc_command(connection),
        ProtocolType::Spice => build_spice_command(connection),
        ProtocolType::ZeroTrust => build_zerotrust_command(connection),
        ProtocolType::Telnet => build_telnet_command(connection),
        ProtocolType::Serial => build_serial_command(connection),
        ProtocolType::Sftp => ConnectionCommand {
            program: "echo".to_string(),
            args: vec!["SFTP connections open a file manager. \
                 Use 'rustconn-cli sftp' instead."
                .to_string()],
        },
        ProtocolType::Kubernetes => build_kubernetes_command(connection),
    }
}

/// Builds SSH command arguments
fn build_ssh_command(connection: &Connection) -> ConnectionCommand {
    let mut args = Vec::new();

    if connection.port != 22 {
        args.push("-p".to_string());
        args.push(connection.port.to_string());
    }

    if let rustconn_core::models::ProtocolConfig::Ssh(ref ssh_config) = connection.protocol_config {
        if let Some(ref key_path) = ssh_config.key_path {
            args.push("-i".to_string());
            args.push(key_path.display().to_string());
        }
        if let Some(ref proxy_jump) = ssh_config.proxy_jump {
            args.push("-J".to_string());
            args.push(proxy_jump.clone());
        }
        if ssh_config.use_control_master {
            args.push("-o".to_string());
            args.push("ControlMaster=auto".to_string());
            args.push("-o".to_string());
            args.push("ControlPersist=10m".to_string());
        }
        if ssh_config.agent_forwarding {
            args.push("-A".to_string());
        }
        if ssh_config.x11_forwarding {
            args.push("-X".to_string());
        }
        if ssh_config.compression {
            args.push("-C".to_string());
        }
        for (key, value) in &ssh_config.custom_options {
            args.push("-o".to_string());
            args.push(format!("{key}={value}"));
        }
    }

    let destination = connection.username.as_ref().map_or_else(
        || connection.host.clone(),
        |u| format!("{u}@{}", connection.host),
    );
    args.push(destination);

    if let rustconn_core::models::ProtocolConfig::Ssh(ref ssh_config) = connection.protocol_config {
        if let Some(ref startup_cmd) = ssh_config.startup_command {
            args.push(startup_cmd.clone());
        }
    }

    ConnectionCommand {
        program: "ssh".to_string(),
        args,
    }
}

/// Builds RDP command arguments (using xfreerdp)
fn build_rdp_command(connection: &Connection) -> ConnectionCommand {
    let mut args = Vec::new();

    args.push(format!("/v:{}:{}", connection.host, connection.port));

    if let Some(ref username) = connection.username {
        args.push(format!("/u:{username}"));
    }
    if let Some(ref domain) = connection.domain {
        args.push(format!("/d:{domain}"));
    }

    if let rustconn_core::models::ProtocolConfig::Rdp(ref rdp_config) = connection.protocol_config {
        if let Some(ref resolution) = rdp_config.resolution {
            args.push(format!("/w:{}", resolution.width));
            args.push(format!("/h:{}", resolution.height));
        }
        if let Some(depth) = rdp_config.color_depth {
            args.push(format!("/bpp:{depth}"));
        }
        if rdp_config.audio_redirect {
            args.push("/sound".to_string());
        }
        if let Some(ref gateway) = rdp_config.gateway {
            args.push(format!("/g:{}:{}", gateway.hostname, gateway.port));
            if let Some(ref gw_user) = gateway.username {
                args.push(format!("/gu:{gw_user}"));
            }
        }
        for folder in &rdp_config.shared_folders {
            args.push(format!(
                "/drive:{},{}",
                folder.share_name,
                folder.local_path.display()
            ));
        }
        args.extend(rdp_config.custom_args.clone());
    }

    ConnectionCommand {
        program: "xfreerdp".to_string(),
        args,
    }
}

/// Builds VNC command arguments (using vncviewer)
fn build_vnc_command(connection: &Connection) -> ConnectionCommand {
    let mut args = Vec::new();

    if let rustconn_core::models::ProtocolConfig::Vnc(ref vnc_config) = connection.protocol_config {
        if let Some(ref encoding) = vnc_config.encoding {
            args.push("-encoding".to_string());
            args.push(encoding.clone());
        }
        if let Some(compression) = vnc_config.compression {
            args.push("-compresslevel".to_string());
            args.push(compression.to_string());
        }
        if let Some(quality) = vnc_config.quality {
            args.push("-quality".to_string());
            args.push(quality.to_string());
        }
        args.extend(vnc_config.custom_args.clone());
    }

    let display = if connection.port >= 5900 {
        connection.port - 5900
    } else {
        connection.port
    };
    args.push(format!("{}:{display}", connection.host));

    ConnectionCommand {
        program: "vncviewer".to_string(),
        args,
    }
}

/// Builds SPICE command arguments (using remote-viewer)
fn build_spice_command(connection: &Connection) -> ConnectionCommand {
    let mut args = Vec::new();

    let scheme = if let rustconn_core::models::ProtocolConfig::Spice(ref spice_config) =
        connection.protocol_config
    {
        if spice_config.tls_enabled {
            "spice+tls"
        } else {
            "spice"
        }
    } else {
        "spice"
    };

    let uri = format!("{scheme}://{}:{}", connection.host, connection.port);
    args.push(uri);

    if let rustconn_core::models::ProtocolConfig::Spice(ref spice_config) =
        connection.protocol_config
    {
        if let Some(ref ca_cert) = spice_config.ca_cert_path {
            args.push(format!("--spice-ca-file={}", ca_cert.display()));
        }
        if spice_config.usb_redirection {
            args.push("--spice-usbredir-redirect-on-connect=auto".to_string());
        }
        for folder in &spice_config.shared_folders {
            args.push(format!(
                "--spice-shared-dir={}",
                folder.local_path.display()
            ));
        }
    }

    ConnectionCommand {
        program: "remote-viewer".to_string(),
        args,
    }
}

/// Builds Zero Trust command arguments using cloud CLI tools
///
/// Zero Trust connections use cloud provider CLIs (aws, gcloud, az, oci,
/// etc.) to establish secure connections through identity-aware proxies.
fn build_zerotrust_command(connection: &Connection) -> ConnectionCommand {
    if let rustconn_core::models::ProtocolConfig::ZeroTrust(ref zt_config) =
        connection.protocol_config
    {
        let (program, mut args) = zt_config.build_command(connection.username.as_deref());
        args.extend(zt_config.custom_args.clone());
        ConnectionCommand { program, args }
    } else {
        eprintln!("Warning: ZeroTrust protocol type but no ZeroTrust config");
        ConnectionCommand {
            program: "echo".to_string(),
            args: vec!["Invalid Zero Trust configuration".to_string()],
        }
    }
}

/// Builds Telnet command arguments
fn build_telnet_command(connection: &Connection) -> ConnectionCommand {
    let mut args = Vec::new();

    if let rustconn_core::models::ProtocolConfig::Telnet(ref config) = connection.protocol_config {
        args.extend(config.custom_args.clone());
    }

    args.push(connection.host.clone());
    args.push(connection.port.to_string());

    ConnectionCommand {
        program: "telnet".to_string(),
        args,
    }
}

/// Builds Serial command arguments using `SerialProtocol::build_command()`
fn build_serial_command(connection: &Connection) -> ConnectionCommand {
    use rustconn_core::protocol::{Protocol, SerialProtocol};

    if let Some(cmd) = SerialProtocol::new().build_command(connection) {
        let (program, args) = cmd.split_first().map_or_else(
            || ("picocom".to_string(), Vec::new()),
            |(p, a)| (p.clone(), a.to_vec()),
        );
        ConnectionCommand { program, args }
    } else {
        ConnectionCommand {
            program: "echo".to_string(),
            args: vec!["Error: failed to build Serial command".to_string()],
        }
    }
}

/// Builds Kubernetes command arguments using
/// `KubernetesProtocol::build_command()`
fn build_kubernetes_command(connection: &Connection) -> ConnectionCommand {
    use rustconn_core::protocol::{KubernetesProtocol, Protocol};

    if let Some(cmd) = KubernetesProtocol::new().build_command(connection) {
        let (program, args) = cmd.split_first().map_or_else(
            || ("kubectl".to_string(), Vec::new()),
            |(p, a)| (p.clone(), a.to_vec()),
        );
        ConnectionCommand { program, args }
    } else {
        ConnectionCommand {
            program: "echo".to_string(),
            args: vec!["Error: not a Kubernetes connection".to_string()],
        }
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

        eprintln!("Executing: {}", format_command_for_log(command));

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

        eprintln!("Executing: {}", format_command_for_log(command));

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
