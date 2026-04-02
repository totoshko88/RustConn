//! Add connection command.

use std::path::Path;

use rustconn_core::config::ConfigManager;
use rustconn_core::models::{
    Connection, HoopDevConfig, ProtocolConfig, ProtocolType, SshAuthMethod, ZeroTrustConfig,
    ZeroTrustProvider, ZeroTrustProviderConfig,
};

use crate::error::CliError;
use crate::util::{create_config_manager, default_port_for_protocol, parse_protocol_type};

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
    pub icon: Option<&'a str>,
    pub ssh_agent_socket: Option<&'a str>,
    pub provider: Option<&'a str>,
    pub hoop_connection_name: Option<&'a str>,
    pub hoop_gateway_url: Option<&'a str>,
    pub hoop_grpc_url: Option<&'a str>,
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

    let mut connection = if protocol_type == ProtocolType::ZeroTrust {
        create_zerotrust_connection(params.name, &params)?
    } else {
        create_connection(
            params.name,
            effective_host,
            port,
            protocol_type,
            params.key,
            ssh_auth,
        )?
    };

    // Apply serial-specific settings
    if protocol_type == ProtocolType::Serial
        && let rustconn_core::models::ProtocolConfig::Serial(ref mut config) =
            connection.protocol_config
        && let Some(baud) = params.baud_rate
    {
        config.baud_rate = crate::util::parse_baud_rate(baud)?;
    }

    if let Some(username) = params.user {
        connection.username = Some(username.to_string());
    }

    if let Some(icon) = params.icon {
        connection.icon = Some(icon.to_string());
    }

    // Apply SSH agent socket for SSH/SFTP connections
    if let Some(socket) = params.ssh_agent_socket {
        match connection.protocol_config {
            rustconn_core::models::ProtocolConfig::Ssh(ref mut cfg) => {
                cfg.ssh_agent_socket = Some(socket.to_string());
            }
            rustconn_core::models::ProtocolConfig::Sftp(ref mut cfg) => {
                cfg.ssh_agent_socket = Some(socket.to_string());
            }
            _ => {
                tracing::warn!("--ssh-agent-socket is only applicable to SSH/SFTP connections");
            }
        }
    }

    let config_manager = create_config_manager(config_path)?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    ConfigManager::validate_connection(&connection)
        .map_err(|e| CliError::Config(format!("Invalid connection: {e}")))?;

    if connections.iter().any(|c| c.name == connection.name) {
        return Err(CliError::Config(format!(
            "Connection '{}' already exists. Use a different name or 'update' to modify it.",
            connection.name
        )));
    }

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
    let proto = parse_protocol_type(protocol)?;
    let port = default_port_for_protocol(proto);
    Ok((proto, port))
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
) -> Result<Connection, CliError> {
    let conn = match protocol_type {
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
                tracing::warn!("--key option is ignored for RDP connections");
            }
            if auth_method.is_some() {
                tracing::warn!("--auth-method is ignored for RDP connections");
            }
            Connection::new_rdp(name.to_string(), host.to_string(), port)
        }
        ProtocolType::Vnc => {
            if key.is_some() {
                tracing::warn!("--key option is ignored for VNC connections");
            }
            if auth_method.is_some() {
                tracing::warn!("--auth-method is ignored for VNC connections");
            }
            Connection::new_vnc(name.to_string(), host.to_string(), port)
        }
        ProtocolType::Spice => {
            if key.is_some() {
                tracing::warn!("--key option is ignored for SPICE connections");
            }
            if auth_method.is_some() {
                tracing::warn!("--auth-method is ignored for SPICE connections");
            }
            Connection::new_spice(name.to_string(), host.to_string(), port)
        }
        ProtocolType::ZeroTrust => {
            // ZeroTrust connections are handled by create_zerotrust_connection()
            // before this function is called
            return Err(CliError::Config(
                "Zero Trust connections require --provider. Use --protocol zt --provider hoop_dev"
                    .into(),
            ));
        }
        ProtocolType::Telnet => {
            if key.is_some() {
                tracing::warn!("--key option is ignored for Telnet connections");
            }
            if auth_method.is_some() {
                tracing::warn!("--auth-method is ignored for Telnet connections");
            }
            Connection::new_telnet(name.to_string(), host.to_string(), port)
        }
        ProtocolType::Serial => {
            if key.is_some() {
                tracing::warn!("--key option is ignored for Serial connections");
            }
            if auth_method.is_some() {
                tracing::warn!("--auth-method is ignored for Serial connections");
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
                tracing::warn!("--key option is ignored for Kubernetes connections");
            }
            if auth_method.is_some() {
                tracing::warn!("--auth-method is ignored for Kubernetes connections");
            }
            Connection::new_kubernetes(name.to_string())
        }
        ProtocolType::Mosh => {
            if key.is_some() {
                tracing::warn!("--key option is ignored for MOSH connections");
            }
            if auth_method.is_some() {
                tracing::warn!("--auth-method is ignored for MOSH connections");
            }
            Connection::new_mosh(name.to_string(), host.to_string(), port)
        }
    };
    Ok(conn)
}

/// Create a Zero Trust connection from CLI parameters
fn create_zerotrust_connection(name: &str, params: &AddParams<'_>) -> Result<Connection, CliError> {
    let provider_str = params.provider.ok_or_else(|| {
        CliError::Config(
            "Zero Trust connections require --provider. Use --provider hoop_dev".into(),
        )
    })?;

    match provider_str {
        "hoop_dev" => {
            let connection_name = params.hoop_connection_name.ok_or_else(|| {
                CliError::Config(
                    "--hoop-connection-name is required for --provider hoop_dev".into(),
                )
            })?;

            let zt_config = ZeroTrustConfig {
                provider: ZeroTrustProvider::HoopDev,
                provider_config: ZeroTrustProviderConfig::HoopDev(HoopDevConfig {
                    connection_name: connection_name.to_string(),
                    gateway_url: params.hoop_gateway_url.map(String::from),
                    grpc_url: params.hoop_grpc_url.map(String::from),
                }),
                custom_args: Vec::new(),
                detected_provider: None,
            };

            Ok(Connection::new(
                name.to_string(),
                params.host.to_string(),
                params.port.unwrap_or(22),
                ProtocolConfig::ZeroTrust(zt_config),
            ))
        }
        other => Err(CliError::Config(format!(
            "CLI creation for provider '{other}' is not yet supported. Use the GUI."
        ))),
    }
}
