//! Add connection command.

use std::path::Path;

use rustconn_core::config::ConfigManager;
use rustconn_core::models::{
    AwsSsmConfig, AzureBastionConfig, AzureSshConfig, BoundaryConfig, CloudflareAccessConfig,
    Connection, GcpIapConfig, GenericZeroTrustConfig, HoopDevConfig, OciBastionConfig,
    ProtocolConfig, ProtocolType, SshAuthMethod, TailscaleSshConfig, TeleportConfig,
    ZeroTrustConfig, ZeroTrustProvider, ZeroTrustProviderConfig,
};

use crate::error::CliError;
use crate::util::{
    create_config_manager, default_port_for_protocol, find_connection, parse_protocol_type,
};

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
    pub aws_profile: Option<&'a str>,
    pub aws_region: Option<&'a str>,
    pub gcp_zone: Option<&'a str>,
    pub gcp_project: Option<&'a str>,
    pub resource_group: Option<&'a str>,
    pub bastion_name: Option<&'a str>,
    pub vm_name: Option<&'a str>,
    pub bastion_id: Option<&'a str>,
    pub target_resource_id: Option<&'a str>,
    pub target_private_ip: Option<&'a str>,
    pub teleport_cluster: Option<&'a str>,
    pub boundary_target: Option<&'a str>,
    pub boundary_addr: Option<&'a str>,
    pub custom_command: Option<&'a str>,
    pub jump_host: Option<&'a str>,
    pub keep_alive_interval: Option<u32>,
    pub keep_alive_count: Option<u32>,
    pub ssh_verbose: bool,
    pub ignore_certificate: bool,
}

/// Add connection command handler
#[allow(clippy::needless_pass_by_value, clippy::too_many_lines)]
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

    // Apply SSH keep-alive and verbose settings
    if params.keep_alive_interval.is_some()
        || params.keep_alive_count.is_some()
        || params.ssh_verbose
    {
        match connection.protocol_config {
            rustconn_core::models::ProtocolConfig::Ssh(ref mut cfg) => {
                if let Some(interval) = params.keep_alive_interval {
                    cfg.keep_alive_interval = Some(interval);
                }
                if let Some(count) = params.keep_alive_count {
                    cfg.keep_alive_count_max = Some(count);
                }
                if params.ssh_verbose {
                    cfg.verbose = true;
                }
            }
            rustconn_core::models::ProtocolConfig::Sftp(ref mut cfg) => {
                if let Some(interval) = params.keep_alive_interval {
                    cfg.keep_alive_interval = Some(interval);
                }
                if let Some(count) = params.keep_alive_count {
                    cfg.keep_alive_count_max = Some(count);
                }
                if params.ssh_verbose {
                    cfg.verbose = true;
                }
            }
            _ => {
                if params.keep_alive_interval.is_some() || params.keep_alive_count.is_some() {
                    tracing::warn!(
                        "--keep-alive-interval/--keep-alive-count are only applicable to SSH/SFTP connections"
                    );
                }
                if params.ssh_verbose {
                    tracing::warn!("--ssh-verbose is only applicable to SSH/SFTP connections");
                }
            }
        }
    }

    // Apply RDP ignore-certificate setting
    if params.ignore_certificate {
        if let rustconn_core::models::ProtocolConfig::Rdp(ref mut cfg) = connection.protocol_config
        {
            cfg.ignore_certificate = true;
        } else {
            tracing::warn!("--ignore-certificate is only applicable to RDP connections");
        }
    }

    let config_manager = create_config_manager(config_path)?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    // Resolve --jump-host to a UUID by looking up existing connections
    if let Some(jump_host_ref) = params.jump_host {
        let jump_conn = find_connection(&connections, jump_host_ref)?;
        let jump_id = jump_conn.id;
        apply_jump_host_id(&mut connection, jump_id)?;
    }

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
        ProtocolType::Web => {
            if key.is_some() {
                tracing::warn!("--key option is ignored for Web connections");
            }
            if auth_method.is_some() {
                tracing::warn!("--auth-method is ignored for Web connections");
            }
            Connection::new(
                name.to_string(),
                host.to_string(),
                0,
                rustconn_core::models::ProtocolConfig::Web(
                    rustconn_core::models::WebConfig::default(),
                ),
            )
        }
    };
    Ok(conn)
}

/// Helper to build a `Connection` from a `ZeroTrustConfig`.
fn zt_connection(name: &str, params: &AddParams<'_>, zt_config: ZeroTrustConfig) -> Connection {
    Connection::new(
        name.to_string(),
        params.host.to_string(),
        params.port.unwrap_or(22),
        ProtocolConfig::ZeroTrust(zt_config),
    )
}

/// Shorthand for building a [`ZeroTrustConfig`].
fn zt_cfg(
    provider: ZeroTrustProvider,
    provider_config: ZeroTrustProviderConfig,
) -> ZeroTrustConfig {
    ZeroTrustConfig {
        provider,
        provider_config,
        custom_args: Vec::new(),
        detected_provider: None,
    }
}

/// Require a CLI option or return a descriptive error.
fn require<'a>(value: Option<&'a str>, flag: &str, provider: &str) -> Result<&'a str, CliError> {
    value.ok_or_else(|| CliError::Config(format!("{flag} is required for --provider {provider}")))
}

/// Create a Zero Trust connection from CLI parameters
#[allow(clippy::too_many_lines)]
fn create_zerotrust_connection(name: &str, params: &AddParams<'_>) -> Result<Connection, CliError> {
    let provider_str = params.provider.ok_or_else(|| {
        CliError::Config(
            "Zero Trust connections require --provider. Use --provider hoop_dev".into(),
        )
    })?;

    match provider_str {
        "hoop_dev" => {
            let connection_name = require(
                params.hoop_connection_name,
                "--hoop-connection-name",
                "hoop_dev",
            )?;
            let cfg = zt_cfg(
                ZeroTrustProvider::HoopDev,
                ZeroTrustProviderConfig::HoopDev(HoopDevConfig {
                    connection_name: connection_name.to_string(),
                    gateway_url: params.hoop_gateway_url.map(String::from),
                    grpc_url: params.hoop_grpc_url.map(String::from),
                }),
            );
            Ok(zt_connection(name, params, cfg))
        }
        "aws_ssm" => {
            let cfg = zt_cfg(
                ZeroTrustProvider::AwsSsm,
                ZeroTrustProviderConfig::AwsSsm(AwsSsmConfig {
                    target: params.host.to_string(),
                    profile: params.aws_profile.unwrap_or("default").to_string(),
                    region: params.aws_region.map(String::from),
                }),
            );
            Ok(zt_connection(name, params, cfg))
        }
        "gcp_iap" => {
            let zone = require(params.gcp_zone, "--gcp-zone", "gcp_iap")?;
            let cfg = zt_cfg(
                ZeroTrustProvider::GcpIap,
                ZeroTrustProviderConfig::GcpIap(GcpIapConfig {
                    instance: params.host.to_string(),
                    zone: zone.to_string(),
                    project: params.gcp_project.map(String::from),
                }),
            );
            Ok(zt_connection(name, params, cfg))
        }
        "azure_bastion" => {
            let rg = require(params.resource_group, "--resource-group", "azure_bastion")?;
            let bn = require(params.bastion_name, "--bastion-name", "azure_bastion")?;
            let cfg = zt_cfg(
                ZeroTrustProvider::AzureBastion,
                ZeroTrustProviderConfig::AzureBastion(AzureBastionConfig {
                    target_resource_id: params.host.to_string(),
                    resource_group: rg.to_string(),
                    bastion_name: bn.to_string(),
                }),
            );
            Ok(zt_connection(name, params, cfg))
        }
        "azure_ssh" => {
            let vm = require(params.vm_name, "--vm-name", "azure_ssh")?;
            let rg = require(params.resource_group, "--resource-group", "azure_ssh")?;
            let cfg = zt_cfg(
                ZeroTrustProvider::AzureSsh,
                ZeroTrustProviderConfig::AzureSsh(AzureSshConfig {
                    vm_name: vm.to_string(),
                    resource_group: rg.to_string(),
                }),
            );
            Ok(zt_connection(name, params, cfg))
        }
        "oci_bastion" => {
            let bid = require(params.bastion_id, "--bastion-id", "oci_bastion")?;
            let trid = require(
                params.target_resource_id,
                "--target-resource-id",
                "oci_bastion",
            )?;
            let tip = require(
                params.target_private_ip,
                "--target-private-ip",
                "oci_bastion",
            )?;
            let cfg = zt_cfg(
                ZeroTrustProvider::OciBastion,
                ZeroTrustProviderConfig::OciBastion(OciBastionConfig {
                    bastion_id: bid.to_string(),
                    target_resource_id: trid.to_string(),
                    target_private_ip: tip.to_string(),
                    ..OciBastionConfig::default()
                }),
            );
            Ok(zt_connection(name, params, cfg))
        }
        "cloudflare_access" => {
            let cfg = zt_cfg(
                ZeroTrustProvider::CloudflareAccess,
                ZeroTrustProviderConfig::CloudflareAccess(CloudflareAccessConfig {
                    hostname: params.host.to_string(),
                    username: params.user.map(String::from),
                }),
            );
            Ok(zt_connection(name, params, cfg))
        }
        "teleport" => {
            let cfg = zt_cfg(
                ZeroTrustProvider::Teleport,
                ZeroTrustProviderConfig::Teleport(TeleportConfig {
                    host: params.host.to_string(),
                    username: params.user.map(String::from),
                    cluster: params.teleport_cluster.map(String::from),
                }),
            );
            Ok(zt_connection(name, params, cfg))
        }
        "tailscale_ssh" => {
            let cfg = zt_cfg(
                ZeroTrustProvider::TailscaleSsh,
                ZeroTrustProviderConfig::TailscaleSsh(TailscaleSshConfig {
                    host: params.host.to_string(),
                    username: params.user.map(String::from),
                }),
            );
            Ok(zt_connection(name, params, cfg))
        }
        "boundary" => {
            let target = require(params.boundary_target, "--boundary-target", "boundary")?;
            let cfg = zt_cfg(
                ZeroTrustProvider::Boundary,
                ZeroTrustProviderConfig::Boundary(BoundaryConfig {
                    target: target.to_string(),
                    addr: params.boundary_addr.map(String::from),
                }),
            );
            Ok(zt_connection(name, params, cfg))
        }
        "generic" => {
            let cmd = require(params.custom_command, "--custom-command", "generic")?;
            let cfg = zt_cfg(
                ZeroTrustProvider::Generic,
                ZeroTrustProviderConfig::Generic(GenericZeroTrustConfig {
                    command_template: cmd.to_string(),
                }),
            );
            Ok(zt_connection(name, params, cfg))
        }
        other => Err(CliError::Config(format!(
            "Unknown provider '{other}'. Valid: aws_ssm, gcp_iap, azure_bastion, azure_ssh, \
             oci_bastion, cloudflare_access, teleport, tailscale_ssh, boundary, hoop_dev, generic"
        ))),
    }
}

/// Set `jump_host_id` on a connection's protocol config.
///
/// Supported protocols: SSH, SFTP, RDP, VNC, SPICE.
/// Returns an error for protocols that don't support jump hosts.
pub fn apply_jump_host_id(
    connection: &mut Connection,
    jump_id: uuid::Uuid,
) -> Result<(), CliError> {
    match connection.protocol_config {
        ProtocolConfig::Ssh(ref mut cfg) => {
            cfg.jump_host_id = Some(jump_id);
        }
        ProtocolConfig::Sftp(ref mut cfg) => {
            cfg.jump_host_id = Some(jump_id);
        }
        ProtocolConfig::Rdp(ref mut cfg) => {
            cfg.jump_host_id = Some(jump_id);
        }
        rustconn_core::models::ProtocolConfig::Vnc(ref mut cfg) => {
            cfg.jump_host_id = Some(jump_id);
        }
        rustconn_core::models::ProtocolConfig::Spice(ref mut cfg) => {
            cfg.jump_host_id = Some(jump_id);
        }
        _ => {
            return Err(CliError::Config(format!(
                "--jump-host is not supported for {} connections",
                connection.protocol
            )));
        }
    }
    Ok(())
}
