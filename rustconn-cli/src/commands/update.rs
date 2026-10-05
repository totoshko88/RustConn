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

/// Applies `--postpend-*` to the connection's output filter.
///
/// An empty `--postpend-command` removes the filter outright; a non-empty one
/// replaces the command and keeps the filter enabled unless
/// `--postpend-enabled false` says otherwise. `--postpend-enabled` on its own
/// toggles a filter that is already configured, which is the point of the
/// `enabled` flag existing separately from the command.
fn apply_output_filter(connection: &mut rustconn_core::models::Connection, params: &UpdateParams) {
    use rustconn_core::models::PostpendCommand;

    if let Some(command) = params.postpend_command {
        if command.trim().is_empty() {
            connection.postpend = None;
            println!("  Output filter: removed");
        } else {
            connection.postpend = Some(PostpendCommand {
                command: command.to_string(),
                args: params.postpend_arg.to_vec(),
                enabled: params.postpend_enabled.unwrap_or(true),
            });
            println!("  Output filter: {command}");
        }
        return;
    }

    let Some(filter) = connection.postpend.as_mut() else {
        if params.postpend_enabled.is_some() || !params.postpend_arg.is_empty() {
            tracing::warn!(
                "--postpend-enabled/--postpend-arg need a filter; set one with --postpend-command"
            );
        }
        return;
    };

    if !params.postpend_arg.is_empty() {
        filter.args = params.postpend_arg.to_vec();
    }
    if let Some(enabled) = params.postpend_enabled {
        filter.enabled = enabled;
        println!(
            "  Output filter: {}",
            if enabled { "enabled" } else { "disabled" }
        );
    }
}

/// Applies `--elevated-*` to an SSH connection's privilege-escalation settings.
///
/// `--elevated-enabled false` keeps the patterns and the delay so the feature can
/// be switched back on without retyping them; that is why the config is not
/// dropped here.
fn apply_elevated_credentials(
    connection: &mut rustconn_core::models::Connection,
    params: &UpdateParams,
) {
    use rustconn_core::models::ElevatedCredentials;

    if params.elevated_enabled.is_none()
        && params.elevated_prompt.is_empty()
        && params.elevated_delay.is_none()
    {
        return;
    }

    let rustconn_core::models::ProtocolConfig::Ssh(ref mut ssh) = connection.protocol_config else {
        tracing::warn!("--elevated-* options are only applicable to SSH connections");
        return;
    };

    let elevated = ssh
        .elevated
        .get_or_insert_with(ElevatedCredentials::default);
    if let Some(enabled) = params.elevated_enabled {
        elevated.enabled = enabled;
    }
    if !params.elevated_prompt.is_empty() {
        elevated.custom_prompts = params.elevated_prompt.to_vec();
    }
    if let Some(delay) = params.elevated_delay {
        elevated.delay_ms = delay;
    }

    println!(
        "  Elevated credentials: {}{}",
        if elevated.enabled {
            "enabled"
        } else {
            "disabled"
        },
        if elevated.custom_prompts.is_empty() {
            String::new()
        } else {
            format!(" ({} custom pattern(s))", elevated.custom_prompts.len())
        }
    );
}

/// Parameters for the `update` command
#[expect(
    clippy::struct_excessive_bools,
    reason = "AddParams/UpdateParams mirror Clap-derived flags 1:1; bundling related \
              booleans into enums would force callers to convert and obscure CLI mapping"
)]
pub(super) struct UpdateParams<'a> {
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
    pub mptcp: Option<bool>,
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
    pub network_mode: Option<&'a str>,
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
    pub kerberos: Option<bool>,
    pub kdc_address: Option<&'a str>,
    pub rdp_dynamic_resolution: Option<bool>,
    pub rdp_smart_sizing: Option<bool>,
    pub keyboard_layout: Option<u32>,
    pub audio_redirect: bool,
    pub audio_mode: Option<&'a str>,
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
    pub vnc_toolbar: Option<bool>,
    pub vnc_custom_arg: &'a [String],
    pub vnc_viewer: Option<&'a str>,
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
    // RDP
    pub rdp_display_mode: Option<&'a str>,
    pub rdp_resolution: Option<&'a str>,
    pub rdp_freerdp_client: Option<&'a str>,
    // Web
    pub browser_mode: Option<&'a str>,
    pub javascript: Option<bool>,
    pub user_agent: Option<&'a str>,
    pub accept_invalid_certs: Option<bool>,
    pub web_toolbar: Option<bool>,
    pub private_mode: bool,
    pub zoom_level: Option<f64>,
    // Output filter (postpend command) — terminal protocols
    pub postpend_command: Option<&'a str>,
    pub postpend_arg: &'a [String],
    pub postpend_enabled: Option<bool>,
    // Elevated credentials (sudo/su/doas injection) — SSH
    pub elevated_enabled: Option<bool>,
    pub elevated_prompt: &'a [String],
    pub elevated_delay: Option<u32>,
}

/// Update connection command handler
///
/// # Errors
///
/// Returns:
/// - [`CliError::Config`] when connections cannot be loaded or saved, or when
///   the requested protocol / auth method / port combination is invalid
/// - [`CliError::ConnectionNotFound`] when no connection matches `params.name`
///   or `--jump-host` references an unknown connection
/// - [`CliError::Group`] when `--group` is set and the group cannot be created
#[expect(
    clippy::needless_pass_by_value,
    clippy::too_many_lines,
    reason = "UpdateParams is consumed by value to take ownership of borrowed flag values \
              from Clap; the long body matches every editable field in turn"
)]
pub(super) fn cmd_update(
    config_path: Option<&Path>,
    params: UpdateParams<'_>,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let index = {
        let conn = find_connection(&connections, params.name)?;
        let conn_id = conn.id;
        connections
            .iter()
            .position(|c| c.id == conn_id)
            .ok_or_else(|| CliError::Config("Connection disappeared during lookup".to_string()))?
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

    // Update SSH-specific fields. Both SSH and SFTP carry an SSH config; any
    // other protocol has nowhere to put a key or auth method, so reject the
    // flag rather than dropping it silently (matches `add` behaviour).
    if params.key.is_some() || params.auth_method.is_some() {
        let ssh_cfg = match connection.protocol_config {
            rustconn_core::models::ProtocolConfig::Ssh(ref mut cfg)
            | rustconn_core::models::ProtocolConfig::Sftp(ref mut cfg) => cfg,
            _ => {
                let flag = if params.key.is_some() {
                    "--key"
                } else {
                    "--auth-method"
                };
                return Err(CliError::Config(format!(
                    "{flag} is only valid for SSH and SFTP connections, not {:?}",
                    connection.protocol
                )));
            }
        };
        if let Some(key_path) = params.key {
            ssh_cfg.key_path = Some(key_path.to_path_buf());
        }
        if let Some(method_str) = params.auth_method {
            ssh_cfg.auth_method = parse_auth_method(method_str)?;
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
    if let Some(mptcp_value) = params.mptcp {
        match connection.protocol_config {
            rustconn_core::models::ProtocolConfig::Ssh(ref mut cfg) => {
                cfg.mptcp = mptcp_value;
            }
            rustconn_core::models::ProtocolConfig::Sftp(ref mut cfg) => {
                cfg.mptcp = mptcp_value;
            }
            rustconn_core::models::ProtocolConfig::Rdp(ref mut cfg) => {
                cfg.mptcp = mptcp_value;
            }
            rustconn_core::models::ProtocolConfig::Vnc(ref mut cfg) => {
                cfg.mptcp = mptcp_value;
            }
            _ => {
                tracing::warn!("--mptcp is only applicable to SSH, SFTP, RDP, and VNC connections");
            }
        }
    }

    apply_output_filter(connection, &params);
    apply_elevated_credentials(connection, &params);

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
        || params.kerberos.is_some()
        || params.kdc_address.is_some()
        || params.rdp_dynamic_resolution.is_some()
        || params.rdp_smart_sizing.is_some()
        || params.rdp_freerdp_client.is_some()
        || params.keyboard_layout.is_some()
        || params.audio_redirect
        || params.audio_mode.is_some()
        || params.printer
        || !params.shared_folder.is_empty()
    {
        if let rustconn_core::models::ProtocolConfig::Rdp(ref mut cfg) = connection.protocol_config
        {
            apply_rdp_fields_update(cfg, &params)?;
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
        || params.vnc_toolbar.is_some()
        || !params.vnc_custom_arg.is_empty()
        || params.vnc_viewer.is_some()
    {
        if let rustconn_core::models::ProtocolConfig::Vnc(ref mut cfg) = connection.protocol_config
        {
            apply_vnc_fields_update(cfg, &params)?;
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
        if let rustconn_core::models::ProtocolConfig::Mosh(ref mut cfg) = connection.protocol_config
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

    // Apply RDP-specific settings
    crate::commands::add::apply_rdp_display_options(
        connection,
        params.rdp_display_mode,
        params.rdp_resolution,
    )?;

    // Apply Web-specific settings
    if params.browser_mode.is_some()
        || params.javascript.is_some()
        || params.user_agent.is_some()
        || params.accept_invalid_certs.is_some()
        || params.web_toolbar.is_some()
        || params.private_mode
        || params.zoom_level.is_some()
    {
        if let rustconn_core::models::ProtocolConfig::Web(ref mut cfg) = connection.protocol_config
        {
            if let Some(mode) = params.browser_mode {
                // Clap's `value_parser` already restricts this to the three
                // names, so the catch-all is unreachable in practice. "embedded"
                // is stored as asked even though the CLI never opens a WebView:
                // it used to land on the compile-time default instead, which on
                // this crate — `rustconn-core` with `default-features = false` —
                // is System, so `--browser-mode embedded` silently did the
                // opposite of what it said.
                cfg.browser_mode = match mode {
                    "embedded" => rustconn_core::models::WebBrowserMode::Embedded,
                    "custom" => rustconn_core::models::WebBrowserMode::Custom,
                    _ => rustconn_core::models::WebBrowserMode::System,
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
            // Inverted: the flag offers the toolbar, the field hides it.
            if let Some(toolbar) = params.web_toolbar {
                cfg.hide_floating_toolbar = !toolbar;
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

    if let Some(mode_str) = params.network_mode {
        // `value_parser` restricts this to the two known values, so the wildcard
        // is unreachable rather than a silent fallback.
        connection.network_mode = match mode_str {
            "direct" => rustconn_core::models::NetworkMode::Direct,
            _ => rustconn_core::models::NetworkMode::Inherit,
        };
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

    // Kerberos for NLA (issue #351). Tri-state: `--kerberos` enables,
    // `--kerberos false` disables. The KDC address uses the same
    // `normalize_kdc_url` validator as the GUI; an empty value clears it, a
    // malformed one is a hard error rather than a silently dropped setting.
    if let Some(value) = params.kerberos {
        cfg.kerberos_enabled = value;
    }
    if let Some(address) = params.kdc_address {
        cfg.kdc_proxy_url = if address.trim().is_empty() {
            None
        } else {
            rustconn_core::rdp_client::normalize_kdc_url(address)
                .map_err(|e| CliError::Config(format!("invalid KDC address: {e}")))?
        };
    }

    // Dynamic resolution / smart sizing (issue #341).
    if let Some(value) = params.rdp_dynamic_resolution {
        cfg.dynamic_resolution = value;
    }
    if let Some(value) = params.rdp_smart_sizing {
        cfg.smart_sizing = value;
    }
    if let Some(client) = params.rdp_freerdp_client {
        cfg.freerdp_client_override = if client.is_empty() {
            None
        } else {
            Some(client.to_string())
        };
    }

    // Keyboard layout
    if let Some(klid) = params.keyboard_layout {
        cfg.keyboard_layout = Some(klid);
    }

    // Audio. --audio-mode is the full three-state control; --audio-redirect is
    // kept as the shorthand for "local" and loses to an explicit mode.
    if params.audio_redirect {
        cfg.set_audio_mode(rustconn_core::models::RdpAudioMode::Local);
    }
    if let Some(mode) = params.audio_mode {
        // Already validated by clap's value_parser, so an unknown token here
        // would be a bug rather than bad input.
        if let Some(parsed) = rustconn_core::models::RdpAudioMode::from_cli_str(mode) {
            cfg.set_audio_mode(parsed);
        }
    }

    // Printer
    if params.printer {
        cfg.printer_enabled = true;
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
    // Inverted: the flag offers the toolbar, the field hides it.
    if let Some(toolbar) = params.vnc_toolbar {
        cfg.hide_floating_toolbar = !toolbar;
    }
    for arg in params.vnc_custom_arg {
        cfg.custom_args.push(arg.clone());
    }
    if let Some(viewer) = params.vnc_viewer {
        cfg.vnc_viewer_override = if viewer.is_empty() {
            None
        } else {
            Some(viewer.to_string())
        };
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
