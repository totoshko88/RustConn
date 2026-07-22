//! Add connection command.

use std::path::Path;

use rustconn_core::config::ConfigManager;
use rustconn_core::models::{
    AwsSsmConfig, AzureBastionConfig, AzureSshConfig, BoundaryConfig, CloudflareAccessConfig,
    Connection, GcpIapConfig, GenericZeroTrustConfig, HoopDevConfig, OciBastionConfig, PortForward,
    PortForwardDirection, ProtocolConfig, ProtocolType, RdpGateway, Resolution, SharedFolder,
    SshAuthMethod, TailscaleSshConfig, TeleportConfig, ZeroTrustConfig, ZeroTrustProvider,
    ZeroTrustProviderConfig,
};

use crate::error::CliError;
use crate::util::{
    create_config_manager, default_port_for_protocol, find_connection, parse_protocol_type,
};

/// Parameters for the `add` command
#[expect(
    clippy::struct_excessive_bools,
    reason = "AddParams/UpdateParams mirror Clap-derived flags 1:1; bundling related \
              booleans into enums would force callers to convert and obscure CLI mapping"
)]
pub(super) struct AddParams<'a> {
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
    pub mptcp: bool,
    pub ignore_certificate: bool,
    pub tags: Option<&'a str>,
    pub description: Option<&'a str>,
    pub group: Option<&'a str>,
    pub domain: Option<&'a str>,
    pub window_mode: Option<&'a str>,
    pub skip_port_check: bool,
    pub x11_forwarding: bool,
    pub agent_forwarding: bool,
    pub compression: bool,
    pub startup_command: Option<&'a str>,
    pub proxy_command: Option<&'a str>,
    pub ssh_option: &'a [(String, String)],
    pub local_forward: &'a [String],
    pub remote_forward: &'a [String],
    pub dynamic_forward: &'a [String],
    pub gateway: Option<&'a str>,
    pub gateway_port: Option<u16>,
    pub gateway_username: Option<&'a str>,
    pub remote_app_program: Option<&'a str>,
    pub remote_app_args: Option<&'a str>,
    pub remote_app_name: Option<&'a str>,
    pub resolution: Option<&'a str>,
    pub color_depth: Option<u8>,
    pub disable_nla: bool,
    pub keyboard_layout: Option<u32>,
    pub audio_redirect: bool,
    pub printer: bool,
    pub shared_folder: &'a [String],
    // VNC
    pub vnc_client_mode: Option<&'a str>,
    pub vnc_performance: Option<&'a str>,
    pub vnc_encoding: Option<&'a str>,
    pub vnc_compression: Option<u8>,
    pub vnc_quality: Option<u8>,
    pub vnc_view_only: bool,
    pub vnc_no_scaling: bool,
    pub vnc_no_clipboard: bool,
    pub vnc_custom_arg: &'a [String],
    // SPICE
    pub spice_tls: bool,
    pub spice_ca_cert: Option<&'a str>,
    pub spice_skip_cert_verify: bool,
    pub spice_usb_redirection: bool,
    pub spice_no_clipboard: bool,
    pub spice_image_compression: Option<&'a str>,
    pub spice_proxy: Option<&'a str>,
    pub spice_shared_folder: &'a [String],
    // MOSH
    pub mosh_ssh_port: Option<u16>,
    pub mosh_port_range: Option<&'a str>,
    pub mosh_server_binary: Option<&'a str>,
    pub mosh_predict: Option<&'a str>,
    pub mosh_custom_arg: &'a [String],
    // Serial wave-2
    pub serial_data_bits: Option<&'a str>,
    pub serial_stop_bits: Option<&'a str>,
    pub serial_parity: Option<&'a str>,
    pub serial_flow_control: Option<&'a str>,
    pub serial_custom_arg: &'a [String],
    // Web
    pub browser_mode: Option<&'a str>,
    pub javascript: Option<bool>,
    pub user_agent: Option<&'a str>,
    pub accept_invalid_certs: Option<bool>,
    pub private_mode: bool,
    pub zoom_level: Option<f64>,
}

/// Add connection command handler
///
/// # Errors
///
/// Returns:
/// - [`CliError::Config`] when connections cannot be loaded or saved, or when
///   the requested protocol / auth method / port combination is invalid
/// - [`CliError::Group`] when `--group` is set and the group cannot be created
#[expect(
    clippy::needless_pass_by_value,
    clippy::too_many_lines,
    reason = "AddParams is consumed by value to take ownership of borrowed flag values \
              from Clap; the long body builds every protocol's connection inline"
)]
pub(super) fn cmd_add(config_path: Option<&Path>, params: AddParams<'_>) -> Result<(), CliError> {
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

    // Apply RDP/VNC ignore-certificate setting
    if params.ignore_certificate {
        match connection.protocol_config {
            rustconn_core::models::ProtocolConfig::Rdp(ref mut cfg) => {
                cfg.ignore_certificate = true;
            }
            rustconn_core::models::ProtocolConfig::Vnc(ref mut cfg) => {
                cfg.accept_certificate = true;
            }
            _ => {
                tracing::warn!(
                    "--ignore-certificate is only applicable to RDP and VNC connections"
                );
            }
        }
    }

    // Apply MPTCP setting (SSH, RDP, VNC)
    if params.mptcp {
        match connection.protocol_config {
            rustconn_core::models::ProtocolConfig::Ssh(ref mut cfg) => {
                cfg.mptcp = true;
            }
            rustconn_core::models::ProtocolConfig::Sftp(ref mut cfg) => {
                cfg.mptcp = true;
            }
            rustconn_core::models::ProtocolConfig::Rdp(ref mut cfg) => {
                cfg.mptcp = true;
            }
            rustconn_core::models::ProtocolConfig::Vnc(ref mut cfg) => {
                cfg.mptcp = true;
            }
            _ => {
                tracing::warn!("--mptcp is only applicable to SSH, SFTP, RDP, and VNC connections");
            }
        }
    }

    // Apply SSH wave-2 fields: x11, agent forwarding, compression, startup/proxy command,
    // custom options, port forwards
    if params.x11_forwarding
        || params.agent_forwarding
        || params.compression
        || params.startup_command.is_some()
        || params.proxy_command.is_some()
        || !params.ssh_option.is_empty()
        || !params.local_forward.is_empty()
        || !params.remote_forward.is_empty()
        || !params.dynamic_forward.is_empty()
    {
        match connection.protocol_config {
            rustconn_core::models::ProtocolConfig::Ssh(ref mut cfg) => {
                apply_ssh_wave2_fields(
                    cfg,
                    params.x11_forwarding,
                    params.agent_forwarding,
                    params.compression,
                    params.startup_command,
                    params.proxy_command,
                    params.ssh_option,
                    params.local_forward,
                    params.remote_forward,
                    params.dynamic_forward,
                )?;
            }
            rustconn_core::models::ProtocolConfig::Sftp(ref mut cfg) => {
                apply_ssh_wave2_fields(
                    cfg,
                    params.x11_forwarding,
                    params.agent_forwarding,
                    params.compression,
                    params.startup_command,
                    params.proxy_command,
                    params.ssh_option,
                    params.local_forward,
                    params.remote_forward,
                    params.dynamic_forward,
                )?;
            }
            _ => {
                tracing::warn!(
                    "SSH-specific options (--x11-forwarding, --agent-forwarding, --compression, \
                     --startup-command, --proxy-command, --ssh-option, --local-forward, \
                     --remote-forward, --dynamic-forward) are only applicable to SSH/SFTP connections"
                );
            }
        }
    }

    // Apply common metadata: tags, description, domain, window_mode, skip_port_check
    if let Some(tags_str) = params.tags {
        connection.tags = tags_str
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
    }

    // Apply RDP-specific fields (gateway, RemoteApp, resolution, etc.)
    if params.gateway.is_some()
        || params.gateway_port.is_some()
        || params.gateway_username.is_some()
        || params.remote_app_program.is_some()
        || params.remote_app_args.is_some()
        || params.remote_app_name.is_some()
        || params.resolution.is_some()
        || params.color_depth.is_some()
        || params.disable_nla
        || params.keyboard_layout.is_some()
        || params.audio_redirect
        || params.printer
        || !params.shared_folder.is_empty()
    {
        if let rustconn_core::models::ProtocolConfig::Rdp(ref mut cfg) = connection.protocol_config
        {
            apply_rdp_fields(cfg, &params)?;
        } else {
            tracing::warn!(
                "RDP-specific options (--gateway, --remote-app-*, --resolution, --color-depth, \
                 --disable-nla, --keyboard-layout, --audio-redirect, --printer, --shared-folder) \
                 are only applicable to RDP connections"
            );
        }
    }

    // Apply VNC-specific fields
    if params.vnc_client_mode.is_some()
        || params.vnc_performance.is_some()
        || params.vnc_encoding.is_some()
        || params.vnc_compression.is_some()
        || params.vnc_quality.is_some()
        || params.vnc_view_only
        || params.vnc_no_scaling
        || params.vnc_no_clipboard
        || !params.vnc_custom_arg.is_empty()
    {
        if let rustconn_core::models::ProtocolConfig::Vnc(ref mut cfg) = connection.protocol_config
        {
            apply_vnc_fields(cfg, &params)?;
        } else {
            tracing::warn!("VNC-specific options (--vnc-*) are only applicable to VNC connections");
        }
    }

    // Apply SPICE-specific fields
    if params.spice_tls
        || params.spice_ca_cert.is_some()
        || params.spice_skip_cert_verify
        || params.spice_usb_redirection
        || params.spice_no_clipboard
        || params.spice_image_compression.is_some()
        || params.spice_proxy.is_some()
        || !params.spice_shared_folder.is_empty()
    {
        if let rustconn_core::models::ProtocolConfig::Spice(ref mut cfg) =
            connection.protocol_config
        {
            apply_spice_fields(cfg, &params)?;
        } else {
            tracing::warn!(
                "SPICE-specific options (--spice-*) are only applicable to SPICE connections"
            );
        }
    }

    // Apply MOSH-specific fields
    if params.mosh_ssh_port.is_some()
        || params.mosh_port_range.is_some()
        || params.mosh_server_binary.is_some()
        || params.mosh_predict.is_some()
        || !params.mosh_custom_arg.is_empty()
    {
        if let rustconn_core::models::ProtocolConfig::Mosh(ref mut cfg) = connection.protocol_config
        {
            apply_mosh_fields(cfg, &params)?;
        } else {
            tracing::warn!(
                "MOSH-specific options (--mosh-*) are only applicable to MOSH connections"
            );
        }
    }

    // Apply Serial wave-2 fields (data-bits, stop-bits, parity, flow-control, custom-arg)
    if params.serial_data_bits.is_some()
        || params.serial_stop_bits.is_some()
        || params.serial_parity.is_some()
        || params.serial_flow_control.is_some()
        || !params.serial_custom_arg.is_empty()
    {
        if let rustconn_core::models::ProtocolConfig::Serial(ref mut cfg) =
            connection.protocol_config
        {
            apply_serial_wave2_fields(cfg, &params)?;
        } else {
            tracing::warn!(
                "Serial-specific options (--serial-data-bits, --serial-stop-bits, \
                 --serial-parity, --serial-flow-control, --serial-custom-arg) \
                 are only applicable to Serial connections"
            );
        }
    }

    // Apply Web-specific settings
    if params.browser_mode.is_some()
        || params.javascript.is_some()
        || params.user_agent.is_some()
        || params.accept_invalid_certs.is_some()
        || params.private_mode
        || params.zoom_level.is_some()
    {
        if let rustconn_core::models::ProtocolConfig::Web(ref mut cfg) = connection.protocol_config
        {
            if let Some(mode) = params.browser_mode {
                cfg.browser_mode = match mode {
                    "system" => rustconn_core::models::WebBrowserMode::System,
                    "custom" => rustconn_core::models::WebBrowserMode::Custom,
                    // "embedded" or any other value → compile-time default
                    _ => rustconn_core::models::WebBrowserMode::default(),
                };
            }
            if let Some(js) = params.javascript {
                cfg.javascript_enabled = js;
            }
            if let Some(ua) = params.user_agent {
                if ua.chars().count() > 512 {
                    return Err(CliError::Config(
                        "--user-agent exceeds maximum allowed length of 512 characters".to_string(),
                    ));
                }
                cfg.user_agent = Some(ua.to_string());
            }
            if let Some(certs) = params.accept_invalid_certs {
                cfg.accept_invalid_certs = certs;
            }
            if params.private_mode {
                cfg.private_mode = true;
            }
            if let Some(zoom) = params.zoom_level {
                if !(0.3..=3.0).contains(&zoom) {
                    return Err(CliError::Config(
                        "--zoom-level must be between 0.3 and 3.0".to_string(),
                    ));
                }
                cfg.zoom_level = zoom;
            }
        } else {
            tracing::warn!(
                "Web-specific options (--browser-mode, --javascript, --user-agent, \
                 --accept-invalid-certs, --private-mode, --zoom-level) are only applicable to Web connections"
            );
        }
    }

    if let Some(desc) = params.description {
        connection.description = Some(desc.to_string());
    }

    if let Some(domain) = params.domain {
        connection.domain = Some(domain.to_string());
    }

    if let Some(mode_str) = params.window_mode {
        connection.window_mode = match mode_str {
            "external" => rustconn_core::models::WindowMode::External,
            "fullscreen" => rustconn_core::models::WindowMode::Fullscreen,
            _ => rustconn_core::models::WindowMode::Embedded,
        };
        if !connection.supports_window_mode() {
            tracing::warn!(
                "--window-mode is currently honoured only for RDP and VNC connections; \
                 ignored for {:?}",
                connection.protocol
            );
        }
    }

    if params.skip_port_check {
        connection.skip_port_check = true;
    }

    let config_manager = create_config_manager(config_path)?;

    // Resolve --group: find or create the group, then assign group_id
    if let Some(group_name) = params.group {
        let mut groups = config_manager
            .load_groups()
            .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;
        let groups_before = groups.len();
        let group_id = crate::util::find_or_create_group_id(&mut groups, group_name)?;
        if groups.len() > groups_before {
            config_manager
                .save_groups(&groups)
                .map_err(|e| CliError::Config(format!("Failed to save groups: {e}")))?;
            println!("Created group '{group_name}'");
        }
        connection.group_id = Some(group_id);
    }

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
pub(super) fn parse_auth_method(s: &str) -> Result<SshAuthMethod, CliError> {
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
pub(super) fn parse_protocol(protocol: &str) -> Result<(ProtocolType, u16), CliError> {
    let proto = parse_protocol_type(protocol)?;
    let port = default_port_for_protocol(proto);
    Ok((proto, port))
}

/// Create a connection with the specified parameters
#[expect(
    clippy::too_many_lines,
    reason = "create_connection branches over every protocol kind and applies wave-2 fields \
              inline; per-protocol helpers exist already and reduce this to a dispatcher"
)]
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
                port,
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
#[expect(
    clippy::too_many_lines,
    reason = "Zero Trust create dispatches across every supported provider; \
              per-provider helpers would duplicate the require/parse boilerplate"
)]
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
pub(super) fn apply_jump_host_id(
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

/// Apply SSH wave-2 fields (x11/agent forwarding, compression, startup/proxy command,
/// custom options, port forwards) to an `SshConfig`.
///
/// Used for both SSH and SFTP protocol configs (they share the same struct).
#[expect(
    clippy::too_many_arguments,
    reason = "wave-2 SSH/SFTP fields are flat in the Clap-derived AddParams; \
              regrouping into a struct would only restate the field list"
)]
pub(super) fn apply_ssh_wave2_fields(
    cfg: &mut rustconn_core::models::SshConfig,
    x11_forwarding: bool,
    agent_forwarding: bool,
    compression: bool,
    startup_command: Option<&str>,
    proxy_command: Option<&str>,
    ssh_option: &[(String, String)],
    local_forward: &[String],
    remote_forward: &[String],
    dynamic_forward: &[String],
) -> Result<(), CliError> {
    if x11_forwarding {
        cfg.x11_forwarding = true;
    }
    if agent_forwarding {
        cfg.agent_forwarding = true;
    }
    if compression {
        cfg.compression = true;
    }
    if let Some(cmd) = startup_command {
        cfg.startup_command = Some(cmd.to_string());
    }
    if let Some(proxy) = proxy_command {
        cfg.proxy_command = Some(proxy.to_string());
    }
    for (key, value) in ssh_option {
        cfg.custom_options.insert(key.clone(), value.clone());
    }
    for spec in local_forward {
        cfg.port_forwards.push(parse_port_forward(
            spec,
            PortForwardDirection::Local,
            "--local-forward",
        )?);
    }
    for spec in remote_forward {
        cfg.port_forwards.push(parse_port_forward(
            spec,
            PortForwardDirection::Remote,
            "--remote-forward",
        )?);
    }
    for spec in dynamic_forward {
        cfg.port_forwards.push(parse_dynamic_forward(spec)?);
    }
    Ok(())
}

/// Parse a local or remote port forward spec: `LOCAL_PORT:HOST:REMOTE_PORT`.
pub(super) fn parse_port_forward(
    spec: &str,
    direction: PortForwardDirection,
    flag: &str,
) -> Result<PortForward, CliError> {
    let parts: Vec<&str> = spec.splitn(3, ':').collect();
    if parts.len() != 3 {
        return Err(CliError::Config(format!(
            "Invalid {flag} format '{spec}'. Expected: PORT:HOST:PORT (e.g. 8080:localhost:80)"
        )));
    }
    let local_port: u16 = parts[0].parse().map_err(|_| {
        CliError::Config(format!(
            "Invalid port number '{}' in {flag} '{spec}'",
            parts[0]
        ))
    })?;
    let remote_host = parts[1].to_string();
    let remote_port: u16 = parts[2].parse().map_err(|_| {
        CliError::Config(format!(
            "Invalid port number '{}' in {flag} '{spec}'",
            parts[2]
        ))
    })?;
    Ok(PortForward {
        direction,
        local_port,
        remote_host,
        remote_port,
    })
}

/// Parse a dynamic (SOCKS) port forward spec: just a port number.
pub(super) fn parse_dynamic_forward(spec: &str) -> Result<PortForward, CliError> {
    let port: u16 = spec.parse().map_err(|_| {
        CliError::Config(format!(
            "Invalid --dynamic-forward port '{spec}'. Expected a port number (e.g. 1080)"
        ))
    })?;
    Ok(PortForward {
        direction: PortForwardDirection::Dynamic,
        local_port: port,
        remote_host: String::new(),
        remote_port: 0,
    })
}

/// Apply RDP-specific fields (gateway, RemoteApp, resolution, etc.) to an `RdpConfig`.
pub(super) fn apply_rdp_fields(
    cfg: &mut rustconn_core::models::RdpConfig,
    params: &AddParams<'_>,
) -> Result<(), CliError> {
    // Gateway
    if let Some(gw_host) = params.gateway {
        let port = params.gateway_port.unwrap_or(443);
        cfg.gateway = Some(RdpGateway {
            hostname: gw_host.to_string(),
            port,
            username: params
                .gateway_username
                .map(std::string::ToString::to_string),
        });
    } else if params.gateway_port.is_some() || params.gateway_username.is_some() {
        tracing::warn!("--gateway-port/--gateway-username require --gateway to be set");
    }

    // RemoteApp
    if let Some(prog) = params.remote_app_program {
        cfg.remote_app_program = Some(prog.to_string());
    }
    if let Some(args) = params.remote_app_args {
        cfg.remote_app_args = Some(args.to_string());
    }
    if let Some(name) = params.remote_app_name {
        cfg.remote_app_name = Some(name.to_string());
    }

    // Resolution
    if let Some(res_str) = params.resolution {
        cfg.resolution = Some(parse_resolution(res_str)?);
    }

    // Color depth
    if let Some(depth) = params.color_depth {
        if !matches!(depth, 8 | 15 | 16 | 24 | 32) {
            return Err(CliError::Config(format!(
                "Invalid --color-depth '{depth}'. Valid: 8, 15, 16, 24, 32"
            )));
        }
        cfg.color_depth = Some(depth);
    }

    // NLA
    if params.disable_nla {
        cfg.disable_nla = true;
    }

    // Keyboard layout
    if let Some(klid) = params.keyboard_layout {
        cfg.keyboard_layout = Some(klid);
    }

    // Audio
    if params.audio_redirect {
        cfg.audio_redirect = true;
    }

    // Printer
    if params.printer {
        cfg.printer_enabled = true;
    }

    // Shared folders
    for spec in params.shared_folder {
        cfg.shared_folders.push(parse_shared_folder(spec)?);
    }

    Ok(())
}

/// Parse a resolution string like "1920x1080" into a `Resolution`.
pub(super) fn parse_resolution(spec: &str) -> Result<Resolution, CliError> {
    let parts: Vec<&str> = spec.split('x').collect();
    if parts.len() != 2 {
        return Err(CliError::Config(format!(
            "Invalid --resolution format '{spec}'. Expected: WIDTHxHEIGHT (e.g. 1920x1080)"
        )));
    }
    let width: u32 = parts[0].parse().map_err(|_| {
        CliError::Config(format!(
            "Invalid width '{}' in --resolution '{spec}'",
            parts[0]
        ))
    })?;
    let height: u32 = parts[1].parse().map_err(|_| {
        CliError::Config(format!(
            "Invalid height '{}' in --resolution '{spec}'",
            parts[1]
        ))
    })?;
    Ok(Resolution { width, height })
}

/// Parse a shared folder spec: "NAME:PATH" (e.g. "docs:/home/user/Documents").
pub(super) fn parse_shared_folder(spec: &str) -> Result<SharedFolder, CliError> {
    let Some((name, path)) = spec.split_once(':') else {
        return Err(CliError::Config(format!(
            "Invalid --shared-folder format '{spec}'. Expected: NAME:PATH (e.g. docs:/home/user/Documents)"
        )));
    };
    if name.is_empty() || path.is_empty() {
        return Err(CliError::Config(format!(
            "Invalid --shared-folder format '{spec}'. Both NAME and PATH must be non-empty"
        )));
    }
    Ok(SharedFolder {
        share_name: name.to_string(),
        local_path: std::path::PathBuf::from(path),
    })
}

/// Apply VNC-specific fields to a `VncConfig`.
pub(super) fn apply_vnc_fields(
    cfg: &mut rustconn_core::models::VncConfig,
    params: &AddParams<'_>,
) -> Result<(), CliError> {
    if let Some(mode) = params.vnc_client_mode {
        cfg.client_mode = match mode {
            "external" => rustconn_core::models::VncClientMode::External,
            _ => rustconn_core::models::VncClientMode::Embedded,
        };
    }
    if let Some(perf) = params.vnc_performance {
        cfg.performance_mode = match perf {
            "quality" => rustconn_core::models::VncPerformanceMode::Quality,
            "speed" => rustconn_core::models::VncPerformanceMode::Speed,
            _ => rustconn_core::models::VncPerformanceMode::Balanced,
        };
    }
    if let Some(enc) = params.vnc_encoding {
        cfg.encoding = Some(enc.to_string());
    }
    if let Some(comp) = params.vnc_compression {
        cfg.compression = Some(comp);
    }
    if let Some(qual) = params.vnc_quality {
        cfg.quality = Some(qual);
    }
    if params.vnc_view_only {
        cfg.view_only = true;
    }
    if params.vnc_no_scaling {
        cfg.scaling = false;
    }
    if params.vnc_no_clipboard {
        cfg.clipboard_enabled = false;
    }
    for arg in params.vnc_custom_arg {
        cfg.custom_args.push(arg.clone());
    }
    Ok(())
}

/// Apply SPICE-specific fields to a `SpiceConfig`.
pub(super) fn apply_spice_fields(
    cfg: &mut rustconn_core::models::SpiceConfig,
    params: &AddParams<'_>,
) -> Result<(), CliError> {
    if params.spice_tls {
        cfg.tls_enabled = true;
    }
    if let Some(ca) = params.spice_ca_cert {
        cfg.ca_cert_path = Some(std::path::PathBuf::from(ca));
    }
    if params.spice_skip_cert_verify {
        cfg.skip_cert_verify = true;
    }
    if params.spice_usb_redirection {
        cfg.usb_redirection = true;
    }
    if params.spice_no_clipboard {
        cfg.clipboard_enabled = false;
    }
    if let Some(mode) = params.spice_image_compression {
        cfg.image_compression = Some(parse_spice_image_compression(mode)?);
    }
    if let Some(proxy) = params.spice_proxy {
        cfg.proxy = Some(proxy.to_string());
    }
    for spec in params.spice_shared_folder {
        cfg.shared_folders.push(parse_shared_folder(spec)?);
    }
    Ok(())
}

/// Parse a SPICE image compression mode string.
pub(super) fn parse_spice_image_compression(
    mode: &str,
) -> Result<rustconn_core::models::SpiceImageCompression, CliError> {
    use rustconn_core::models::SpiceImageCompression;
    match mode {
        "auto" => Ok(SpiceImageCompression::Auto),
        "off" => Ok(SpiceImageCompression::Off),
        "glz" => Ok(SpiceImageCompression::Glz),
        "lz" => Ok(SpiceImageCompression::Lz),
        "quic" => Ok(SpiceImageCompression::Quic),
        _ => Err(CliError::Config(format!(
            "Invalid --spice-image-compression '{mode}'. Valid: auto, off, glz, lz, quic"
        ))),
    }
}

/// Apply MOSH-specific fields to a `MoshConfig`.
pub(super) fn apply_mosh_fields(
    cfg: &mut rustconn_core::models::MoshConfig,
    params: &AddParams<'_>,
) -> Result<(), CliError> {
    if let Some(port) = params.mosh_ssh_port {
        cfg.ssh_port = Some(port);
    }
    if let Some(range) = params.mosh_port_range {
        // Validate format: START:END
        let parts: Vec<&str> = range.split(':').collect();
        if parts.len() != 2 {
            return Err(CliError::Config(format!(
                "Invalid --mosh-port-range '{range}'. Expected: START:END (e.g. 60000:60010)"
            )));
        }
        let _start: u16 = parts[0].parse().map_err(|_| {
            CliError::Config(format!(
                "Invalid start port '{}' in --mosh-port-range '{range}'",
                parts[0]
            ))
        })?;
        let _end: u16 = parts[1].parse().map_err(|_| {
            CliError::Config(format!(
                "Invalid end port '{}' in --mosh-port-range '{range}'",
                parts[1]
            ))
        })?;
        cfg.port_range = Some(range.to_string());
    }
    if let Some(bin) = params.mosh_server_binary {
        cfg.server_binary = Some(bin.to_string());
    }
    if let Some(mode) = params.mosh_predict {
        cfg.predict_mode = match mode {
            "always" => rustconn_core::models::MoshPredictMode::Always,
            "never" => rustconn_core::models::MoshPredictMode::Never,
            _ => rustconn_core::models::MoshPredictMode::Adaptive,
        };
    }
    for arg in params.mosh_custom_arg {
        cfg.custom_args.push(arg.clone());
    }
    Ok(())
}

/// Apply Serial wave-2 fields (data-bits, stop-bits, parity, flow-control, custom-arg).
pub(super) fn apply_serial_wave2_fields(
    cfg: &mut rustconn_core::models::SerialConfig,
    params: &AddParams<'_>,
) -> Result<(), CliError> {
    if let Some(bits) = params.serial_data_bits {
        cfg.data_bits = match bits {
            "5" => rustconn_core::models::SerialDataBits::Five,
            "6" => rustconn_core::models::SerialDataBits::Six,
            "7" => rustconn_core::models::SerialDataBits::Seven,
            _ => rustconn_core::models::SerialDataBits::Eight,
        };
    }
    if let Some(bits) = params.serial_stop_bits {
        cfg.stop_bits = match bits {
            "2" => rustconn_core::models::SerialStopBits::Two,
            _ => rustconn_core::models::SerialStopBits::One,
        };
    }
    if let Some(parity) = params.serial_parity {
        cfg.parity = match parity {
            "odd" => rustconn_core::models::SerialParity::Odd,
            "even" => rustconn_core::models::SerialParity::Even,
            _ => rustconn_core::models::SerialParity::None,
        };
    }
    if let Some(fc) = params.serial_flow_control {
        cfg.flow_control = match fc {
            "hardware" => rustconn_core::models::SerialFlowControl::Hardware,
            "software" => rustconn_core::models::SerialFlowControl::Software,
            _ => rustconn_core::models::SerialFlowControl::None,
        };
    }
    for arg in params.serial_custom_arg {
        cfg.custom_args.push(arg.clone());
    }
    Ok(())
}
