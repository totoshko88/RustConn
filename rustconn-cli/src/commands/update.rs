//! Update connection command.

use std::path::Path;

use rustconn_core::config::ConfigManager;
use rustconn_core::models::RdpGateway;

use crate::commands::add::{
    apply_jump_host_id, apply_ssh_wave2_fields, parse_auth_method, parse_resolution,
    parse_shared_folder, parse_spice_image_compression,
};
use crate::error::CliError;
use crate::util::{create_config_manager, find_connection};

/// Parameters for the `update` command
#[allow(clippy::struct_excessive_bools)]
pub struct UpdateParams<'a> {
    pub name: &'a str,
    pub new_name: Option<&'a str>,
    pub host: Option<&'a str>,
    pub port: Option<u16>,
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
    pub tags: Option<&'a str>,
    pub add_tag: &'a [String],
    pub remove_tag: &'a [String],
    pub description: Option<&'a str>,
    pub group: Option<&'a str>,
    pub domain: Option<&'a str>,
    pub window_mode: Option<&'a str>,
    pub skip_port_check: Option<bool>,
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
}

/// Update connection command handler
#[allow(clippy::needless_pass_by_value, clippy::too_many_lines)]
pub fn cmd_update(config_path: Option<&Path>, params: UpdateParams<'_>) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let index = {
        let conn = find_connection(&connections, params.name)?;
        connections.iter().position(|c| c.id == conn.id).unwrap()
    };

    // Resolve --jump-host early (before mutable borrow of connection)
    let resolved_jump_id = if let Some(jump_host_ref) = params.jump_host {
        let jump_conn = find_connection(&connections, jump_host_ref)?;
        Some(jump_conn.id)
    } else {
        None
    };

    let connection = &mut connections[index];

    if let Some(new_name) = params.new_name {
        connection.name = new_name.to_string();
    }
    if let Some(host) = params.host {
        connection.host = host.to_string();
    }
    if let Some(port) = params.port {
        connection.port = port;
    }
    if let Some(user) = params.user {
        connection.username = Some(user.to_string());
    }

    // Update SSH-specific fields
    if params.key.is_some() || params.auth_method.is_some() {
        if let rustconn_core::models::ProtocolConfig::Ssh(ref mut cfg) = connection.protocol_config
        {
            if let Some(key_path) = params.key {
                cfg.key_path = Some(key_path.to_path_buf());
            }
            if let Some(method_str) = params.auth_method {
                cfg.auth_method = parse_auth_method(method_str)?;
            }
        } else {
            if params.key.is_some() {
                tracing::warn!("--key is only applicable to SSH connections");
            }
            if params.auth_method.is_some() {
                tracing::warn!("--auth-method is only applicable to SSH connections");
            }
        }
    }

    // Update Serial-specific fields
    if params.device.is_some() || params.baud_rate.is_some() {
        if let rustconn_core::models::ProtocolConfig::Serial(ref mut cfg) =
            connection.protocol_config
        {
            if let Some(dev) = params.device {
                cfg.device = dev.to_string();
            }
            if let Some(baud) = params.baud_rate {
                cfg.baud_rate = crate::util::parse_baud_rate(baud)?;
            }
        } else {
            if params.device.is_some() {
                tracing::warn!("--device is only applicable to Serial connections");
            }
            if params.baud_rate.is_some() {
                tracing::warn!("--baud-rate is only applicable to Serial connections");
            }
        }
    }

    // Update SSH agent socket for SSH/SFTP connections
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

    // Update ZeroTrust provider-specific fields
    if let rustconn_core::models::ProtocolConfig::ZeroTrust(ref mut zt_config) =
        connection.protocol_config
    {
        if let Some(provider) = params.provider {
            tracing::debug!("ZeroTrust provider hint: {provider}");
        }
        match zt_config.provider_config {
            rustconn_core::models::ZeroTrustProviderConfig::HoopDev(ref mut cfg) => {
                if let Some(conn_name) = params.hoop_connection_name {
                    cfg.connection_name = conn_name.to_string();
                }
                if let Some(url) = params.hoop_gateway_url {
                    cfg.gateway_url = Some(url.to_string());
                }
                if let Some(url) = params.hoop_grpc_url {
                    cfg.grpc_url = Some(url.to_string());
                }
            }
            rustconn_core::models::ZeroTrustProviderConfig::AwsSsm(ref mut cfg) => {
                if let Some(profile) = params.aws_profile {
                    cfg.profile = profile.to_string();
                }
                if let Some(region) = params.aws_region {
                    cfg.region = Some(region.to_string());
                }
                if let Some(host) = params.host {
                    cfg.target = host.to_string();
                }
            }
            rustconn_core::models::ZeroTrustProviderConfig::GcpIap(ref mut cfg) => {
                if let Some(host) = params.host {
                    cfg.instance = host.to_string();
                }
                if let Some(zone) = params.gcp_zone {
                    cfg.zone = zone.to_string();
                }
                if let Some(project) = params.gcp_project {
                    cfg.project = Some(project.to_string());
                }
            }
            rustconn_core::models::ZeroTrustProviderConfig::AzureBastion(ref mut cfg) => {
                if let Some(host) = params.host {
                    cfg.target_resource_id = host.to_string();
                }
                if let Some(rg) = params.resource_group {
                    cfg.resource_group = rg.to_string();
                }
                if let Some(bn) = params.bastion_name {
                    cfg.bastion_name = bn.to_string();
                }
            }
            rustconn_core::models::ZeroTrustProviderConfig::AzureSsh(ref mut cfg) => {
                if let Some(vm) = params.vm_name {
                    cfg.vm_name = vm.to_string();
                }
                if let Some(rg) = params.resource_group {
                    cfg.resource_group = rg.to_string();
                }
            }
            rustconn_core::models::ZeroTrustProviderConfig::OciBastion(ref mut cfg) => {
                if let Some(bid) = params.bastion_id {
                    cfg.bastion_id = bid.to_string();
                }
                if let Some(trid) = params.target_resource_id {
                    cfg.target_resource_id = trid.to_string();
                }
                if let Some(tip) = params.target_private_ip {
                    cfg.target_private_ip = tip.to_string();
                }
            }
            rustconn_core::models::ZeroTrustProviderConfig::CloudflareAccess(ref mut cfg) => {
                if let Some(host) = params.host {
                    cfg.hostname = host.to_string();
                }
                if let Some(user) = params.user {
                    cfg.username = Some(user.to_string());
                }
            }
            rustconn_core::models::ZeroTrustProviderConfig::Teleport(ref mut cfg) => {
                if let Some(host) = params.host {
                    cfg.host = host.to_string();
                }
                if let Some(user) = params.user {
                    cfg.username = Some(user.to_string());
                }
                if let Some(cluster) = params.teleport_cluster {
                    cfg.cluster = Some(cluster.to_string());
                }
            }
            rustconn_core::models::ZeroTrustProviderConfig::TailscaleSsh(ref mut cfg) => {
                if let Some(host) = params.host {
                    cfg.host = host.to_string();
                }
                if let Some(user) = params.user {
                    cfg.username = Some(user.to_string());
                }
            }
            rustconn_core::models::ZeroTrustProviderConfig::Boundary(ref mut cfg) => {
                if let Some(target) = params.boundary_target {
                    cfg.target = target.to_string();
                }
                if let Some(addr) = params.boundary_addr {
                    cfg.addr = Some(addr.to_string());
                }
            }
            rustconn_core::models::ZeroTrustProviderConfig::Generic(ref mut cfg) => {
                if let Some(cmd) = params.custom_command {
                    cfg.command_template = cmd.to_string();
                }
            }
        }
    }

    connection.updated_at = chrono::Utc::now();

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

    // Apply pre-resolved jump host ID
    if let Some(jump_id) = resolved_jump_id {
        if jump_id == connection.id {
            return Err(CliError::Config(
                "A connection cannot use itself as a jump host".into(),
            ));
        }
        apply_jump_host_id(connection, jump_id)?;
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
        || !params.shared_folder.is_empty()
    {
        if let rustconn_core::models::ProtocolConfig::Rdp(ref mut cfg) = connection.protocol_config
        {
            apply_rdp_fields_update(cfg, &params)?;
        } else {
            tracing::warn!(
                "RDP-specific options (--gateway, --remote-app-*, --resolution, --color-depth, \
                 --disable-nla, --keyboard-layout, --audio-redirect, --shared-folder) \
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
            apply_vnc_fields_update(cfg, &params)?;
        } else {
            tracing::warn!(
                "VNC-specific options (--vnc-*) are only applicable to VNC connections"
            );
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
            apply_spice_fields_update(cfg, &params)?;
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
        if let rustconn_core::models::ProtocolConfig::Mosh(ref mut cfg) =
            connection.protocol_config
        {
            apply_mosh_fields_update(cfg, &params)?;
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
            apply_serial_wave2_fields_update(cfg, &params)?;
        } else {
            tracing::warn!(
                "Serial-specific options (--serial-data-bits, --serial-stop-bits, \
                 --serial-parity, --serial-flow-control, --serial-custom-arg) \
                 are only applicable to Serial connections"
            );
        }
    }

    if let Some(icon) = params.icon {
        connection.icon = Some(icon.to_string());
    }

    // Apply common metadata: tags, description, domain, window_mode, skip_port_check
    if let Some(tags_str) = params.tags {
        connection.tags = tags_str
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
    }

    for tag in params.add_tag {
        let trimmed = tag.trim();
        if !trimmed.is_empty() && !connection.tags.iter().any(|t| t == trimmed) {
            connection.tags.push(trimmed.to_string());
        }
    }

    if !params.remove_tag.is_empty() {
        connection
            .tags
            .retain(|t| !params.remove_tag.iter().any(|r| r.trim() == t));
    }

    if let Some(desc) = params.description {
        connection.description = if desc.is_empty() {
            None
        } else {
            Some(desc.to_string())
        };
    }

    if let Some(domain) = params.domain {
        connection.domain = if domain.is_empty() {
            None
        } else {
            Some(domain.to_string())
        };
    }

    if let Some(mode_str) = params.window_mode {
        connection.window_mode = match mode_str {
            "external" => rustconn_core::models::WindowMode::External,
            "fullscreen" => rustconn_core::models::WindowMode::Fullscreen,
            _ => rustconn_core::models::WindowMode::Embedded,
        };
    }

    if let Some(flag) = params.skip_port_check {
        connection.skip_port_check = flag;
    }

    // Resolve --group: find or create the group, then assign group_id (defer save)
    let group_to_save = if let Some(group_name) = params.group {
        let mut groups = config_manager
            .load_groups()
            .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;
        let groups_before = groups.len();
        let group_id = crate::util::find_or_create_group_id(&mut groups, group_name)?;
        connection.group_id = Some(group_id);
        if groups.len() > groups_before {
            Some((groups, group_name.to_string()))
        } else {
            None
        }
    } else {
        None
    };

    ConfigManager::validate_connection(connection)
        .map_err(|e| CliError::Config(format!("Invalid connection: {e}")))?;

    let id = connection.id;
    let name = connection.name.clone();

    if let Some((groups, new_group_name)) = group_to_save {
        config_manager
            .save_groups(&groups)
            .map_err(|e| CliError::Config(format!("Failed to save groups: {e}")))?;
        println!("Created group '{new_group_name}'");
    }

    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    println!("Updated connection '{name}' (ID: {id})");

    Ok(())
}

/// Apply RDP-specific fields for the update command.
///
/// Same logic as `apply_rdp_fields` in add.rs but takes `UpdateParams`.
fn apply_rdp_fields_update(
    cfg: &mut rustconn_core::models::RdpConfig,
    params: &UpdateParams<'_>,
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
        // Update existing gateway fields if gateway already set
        if let Some(ref mut gw) = cfg.gateway {
            if let Some(port) = params.gateway_port {
                gw.port = port;
            }
            if let Some(user) = params.gateway_username {
                gw.username = Some(user.to_string());
            }
        } else {
            tracing::warn!(
                "--gateway-port/--gateway-username require --gateway to be set (or an existing gateway on the connection)"
            );
        }
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

    // Shared folders (appends to existing)
    for spec in params.shared_folder {
        cfg.shared_folders.push(parse_shared_folder(spec)?);
    }

    Ok(())
}

/// Apply VNC-specific fields for the update command.
fn apply_vnc_fields_update(
    cfg: &mut rustconn_core::models::VncConfig,
    params: &UpdateParams<'_>,
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

/// Apply SPICE-specific fields for the update command.
fn apply_spice_fields_update(
    cfg: &mut rustconn_core::models::SpiceConfig,
    params: &UpdateParams<'_>,
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

/// Apply MOSH-specific fields for the update command.
fn apply_mosh_fields_update(
    cfg: &mut rustconn_core::models::MoshConfig,
    params: &UpdateParams<'_>,
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

/// Apply Serial wave-2 fields for the update command.
fn apply_serial_wave2_fields_update(
    cfg: &mut rustconn_core::models::SerialConfig,
    params: &UpdateParams<'_>,
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
