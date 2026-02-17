//! Add connection command.

use std::path::Path;

use rustconn_core::config::ConfigManager;
use rustconn_core::models::{Connection, ProtocolType, SshAuthMethod};

use crate::error::CliError;
use crate::util::create_config_manager;

/// Parameters for the `add` command
pub struct AddParams<'a> {
    pub name: &'a str,
    pub host: &'a str,
    pub port: Option<u16>,
    pub protocol: &'a str,
    pub user: Option<&'a str>,
    pub key: Option<&'a Path>,
    pub auth_method: Option<&'a str>,
    pub device: Option<&'a str>,
    pub baud_rate: Option<u32>,
}

/// Add connection command handler
pub fn cmd_add(config_path: Option<&Path>, params: AddParams<'_>) -> Result<(), CliError> {
    let (protocol_type, default_port) = parse_protocol(params.protocol)?;
    let port = params.port.unwrap_or(default_port);

    let ssh_auth = params.auth_method.map(parse_auth_method).transpose()?;

    // For serial, use --device if provided, otherwise use --host as device
    let effective_host = if protocol_type == ProtocolType::Serial {
        params.device.unwrap_or(params.host)
    } else {
        params.host
    };

    let mut connection = create_connection(
        params.name,
        effective_host,
        port,
        protocol_type,
        params.key,
        ssh_auth,
    );

    // Apply serial-specific settings
    if protocol_type == ProtocolType::Serial {
        if let rustconn_core::models::ProtocolConfig::Serial(ref mut config) =
            connection.protocol_config
        {
            if let Some(baud) = params.baud_rate {
                config.baud_rate = match baud {
                    9600 => rustconn_core::models::SerialBaudRate::B9600,
                    19_200 => rustconn_core::models::SerialBaudRate::B19200,
                    38_400 => rustconn_core::models::SerialBaudRate::B38400,
                    57_600 => rustconn_core::models::SerialBaudRate::B57600,
                    230_400 => rustconn_core::models::SerialBaudRate::B230400,
                    460_800 => rustconn_core::models::SerialBaudRate::B460800,
                    921_600 => rustconn_core::models::SerialBaudRate::B921600,
                    _ => rustconn_core::models::SerialBaudRate::B115200,
                };
            }
        }
    }

    if let Some(username) = params.user {
        connection.username = Some(username.to_string());
    }

    let config_manager = create_config_manager(config_path)?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    ConfigManager::validate_connection(&connection)
        .map_err(|e| CliError::Config(format!("Invalid connection: {e}")))?;

    connections.push(connection.clone());

    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    println!(
        "Added connection '{}' ({} {}:{}) with ID {}",
        connection.name, connection.protocol, connection.host, connection.port, connection.id
    );

    Ok(())
}

/// Parse auth method string into `SshAuthMethod`
pub fn parse_auth_method(s: &str) -> Result<SshAuthMethod, CliError> {
    match s.to_lowercase().as_str() {
        "password" => Ok(SshAuthMethod::Password),
        "publickey" | "public-key" => Ok(SshAuthMethod::PublicKey),
        "keyboard-interactive" | "keyboard_interactive" => Ok(SshAuthMethod::KeyboardInteractive),
        "agent" => Ok(SshAuthMethod::Agent),
        "security-key" | "security_key" | "securitykey" | "fido2" => Ok(SshAuthMethod::SecurityKey),
        _ => Err(CliError::Config(format!(
            "Unknown auth method '{s}'. Valid: password, publickey, \
             keyboard-interactive, agent, security-key"
        ))),
    }
}

/// Parse protocol string and return protocol type with default port
pub fn parse_protocol(protocol: &str) -> Result<(ProtocolType, u16), CliError> {
    match protocol.to_lowercase().as_str() {
        "ssh" => Ok((ProtocolType::Ssh, 22)),
        "rdp" => Ok((ProtocolType::Rdp, 3389)),
        "vnc" => Ok((ProtocolType::Vnc, 5900)),
        "spice" => Ok((ProtocolType::Spice, 5900)),
        "telnet" => Ok((ProtocolType::Telnet, 23)),
        "serial" => Ok((ProtocolType::Serial, 0)),
        "sftp" => Ok((ProtocolType::Sftp, 22)),
        "kubernetes" | "k8s" => Ok((ProtocolType::Kubernetes, 0)),
        _ => Err(CliError::Config(format!(
            "Unknown protocol '{protocol}'. \
             Supported protocols: ssh, rdp, vnc, spice, telnet, \
             serial, sftp, kubernetes"
        ))),
    }
}

/// Create a connection with the specified parameters
#[allow(clippy::too_many_lines)]
fn create_connection(
    name: &str,
    host: &str,
    port: u16,
    protocol_type: ProtocolType,
    key: Option<&Path>,
    auth_method: Option<SshAuthMethod>,
) -> Connection {
    match protocol_type {
        ProtocolType::Ssh => {
            let mut conn = Connection::new_ssh(name.to_string(), host.to_string(), port);
            if let rustconn_core::models::ProtocolConfig::Ssh(ref mut ssh_config) =
                conn.protocol_config
            {
                if let Some(key_path) = key {
                    ssh_config.key_path = Some(key_path.to_path_buf());
                }
                if let Some(method) = auth_method {
                    ssh_config.auth_method = method;
                } else if key.is_some() {
                    ssh_config.auth_method = SshAuthMethod::PublicKey;
                }
            }
            conn
        }
        ProtocolType::Rdp => {
            if key.is_some() {
                eprintln!("Warning: --key option is ignored for RDP connections");
            }
            if auth_method.is_some() {
                eprintln!("Warning: --auth-method is ignored for RDP connections");
            }
            Connection::new_rdp(name.to_string(), host.to_string(), port)
        }
        ProtocolType::Vnc => {
            if key.is_some() {
                eprintln!("Warning: --key option is ignored for VNC connections");
            }
            if auth_method.is_some() {
                eprintln!("Warning: --auth-method is ignored for VNC connections");
            }
            Connection::new_vnc(name.to_string(), host.to_string(), port)
        }
        ProtocolType::Spice => {
            if key.is_some() {
                eprintln!("Warning: --key option is ignored for SPICE connections");
            }
            if auth_method.is_some() {
                eprintln!(
                    "Warning: --auth-method is ignored for \
                     SPICE connections"
                );
            }
            Connection::new_spice(name.to_string(), host.to_string(), port)
        }
        ProtocolType::ZeroTrust => {
            eprintln!(
                "Error: Zero Trust connections cannot be created via CLI \
                 quick-connect"
            );
            eprintln!("Use the GUI to configure Zero Trust connections");
            Connection::new_ssh(name.to_string(), host.to_string(), port)
        }
        ProtocolType::Telnet => {
            if key.is_some() {
                eprintln!(
                    "Warning: --key option is ignored for \
                     Telnet connections"
                );
            }
            if auth_method.is_some() {
                eprintln!(
                    "Warning: --auth-method is ignored for \
                     Telnet connections"
                );
            }
            Connection::new_telnet(name.to_string(), host.to_string(), port)
        }
        ProtocolType::Serial => {
            if key.is_some() {
                eprintln!(
                    "Warning: --key option is ignored for \
                     Serial connections"
                );
            }
            if auth_method.is_some() {
                eprintln!(
                    "Warning: --auth-method is ignored for \
                     Serial connections"
                );
            }
            Connection::new_serial(name.to_string(), host.to_string())
        }
        ProtocolType::Sftp => {
            let mut conn = Connection::new_sftp(name.to_string(), host.to_string(), port);
            if let rustconn_core::models::ProtocolConfig::Sftp(ref mut ssh_config) =
                conn.protocol_config
            {
                if let Some(key_path) = key {
                    ssh_config.key_path = Some(key_path.to_path_buf());
                }
                if let Some(method) = auth_method {
                    ssh_config.auth_method = method;
                } else if key.is_some() {
                    ssh_config.auth_method = SshAuthMethod::PublicKey;
                }
            }
            conn
        }
        ProtocolType::Kubernetes => {
            if key.is_some() {
                eprintln!(
                    "Warning: --key option is ignored for \
                     Kubernetes connections"
                );
            }
            if auth_method.is_some() {
                eprintln!(
                    "Warning: --auth-method is ignored for \
                     Kubernetes connections"
                );
            }
            Connection::new_kubernetes(name.to_string())
        }
    }
}
