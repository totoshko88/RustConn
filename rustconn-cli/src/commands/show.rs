//! Show connection details command.

use std::path::Path;

use rustconn_core::models::{Connection, ProtocolConfig, SshAuthMethod};

use crate::cli::OutputFormat;
use crate::error::CliError;
use crate::util::{create_config_manager, find_connection};

/// Show connection details command handler
///
/// # Errors
///
/// Returns:
/// - [`CliError::Config`] when connections or groups cannot be loaded
/// - [`CliError::ConnectionNotFound`] when no connection matches `name`
pub(super) fn cmd_show(
    config_path: Option<&Path>,
    name: &str,
    format: OutputFormat,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;

    let connection = find_connection(&connections, name)?;

    match format {
        OutputFormat::Json => print_json(connection, &connections, &groups),
        OutputFormat::Csv => print_csv(connection),
        OutputFormat::Table => print_table(connection, &connections),
    }
}

/// Build a full JSON representation of a connection (without secrets).
#[expect(
    clippy::too_many_lines,
    reason = "JSON output documents every persisted Connection field; \
              splitting wouldn't reduce surface area"
)]
fn print_json(
    connection: &Connection,
    connections: &[Connection],
    groups: &[rustconn_core::models::ConnectionGroup],
) -> Result<(), CliError> {
    let group_name = connection
        .group_id
        .and_then(|gid| groups.iter().find(|g| g.id == gid).map(|g| g.name.clone()));

    let resolve_jump = |jump_id: uuid::Uuid| -> String {
        connections
            .iter()
            .find(|c| c.id == jump_id)
            .map_or_else(|| jump_id.to_string(), |c| c.name.clone())
    };

    let mut obj = serde_json::json!({
        "id": connection.id.to_string(),
        "name": connection.name,
        "host": connection.host,
        "port": connection.port,
        "protocol": connection.protocol.as_str(),
        "username": connection.username,
        "description": connection.description,
        "group_id": connection.group_id.map(|id| id.to_string()),
        "group_name": group_name,
        "tags": connection.tags,
        "icon": connection.icon,
        "is_pinned": connection.is_pinned,
        "created_at": connection.created_at.to_rfc3339(),
        "updated_at": connection.updated_at.to_rfc3339(),
        "last_connected": connection.last_connected.map(|t| t.to_rfc3339()),
        "password_source": format!("{:?}", connection.password_source),
        "domain": connection.domain,
        "window_mode": format!("{:?}", connection.window_mode),
        "skip_port_check": connection.skip_port_check,
        "session_recording_enabled": connection.session_recording_enabled,
        "is_dynamic": connection.is_dynamic,
    });

    // Add protocol-specific config
    let Some(map) = obj.as_object_mut() else {
        return Err(CliError::Config("Failed to build JSON object".to_string()));
    };

    match &connection.protocol_config {
        ProtocolConfig::Ssh(config) => {
            let method = match config.auth_method {
                SshAuthMethod::Password => "password",
                SshAuthMethod::PublicKey => "publickey",
                SshAuthMethod::KeyboardInteractive => "keyboard-interactive",
                SshAuthMethod::Agent => "agent",
                SshAuthMethod::SecurityKey => "security-key",
            };
            map.insert(
                "auth_method".to_string(),
                serde_json::Value::String(method.to_string()),
            );
            if let Some(ref key) = config.key_path {
                map.insert(
                    "key_path".to_string(),
                    serde_json::Value::String(key.display().to_string()),
                );
            }
            if let Some(ref jump) = config.proxy_jump {
                map.insert(
                    "proxy_jump".to_string(),
                    serde_json::Value::String(jump.clone()),
                );
            }
            if let Some(jump_id) = config.jump_host_id {
                map.insert(
                    "jump_host".to_string(),
                    serde_json::Value::String(resolve_jump(jump_id)),
                );
            }
            if let Some(ref socket) = config.ssh_agent_socket {
                map.insert(
                    "ssh_agent_socket".to_string(),
                    serde_json::Value::String(socket.clone()),
                );
            }
            if !config.port_forwards.is_empty() {
                let fwds: Vec<String> = config
                    .port_forwards
                    .iter()
                    .map(rustconn_core::PortForward::display_summary)
                    .collect();
                map.insert("port_forwards".to_string(), serde_json::json!(fwds));
            }
        }
        ProtocolConfig::Rdp(config) => {
            if let Some(ref res) = config.resolution {
                map.insert(
                    "resolution".to_string(),
                    serde_json::Value::String(format!("{}x{}", res.width, res.height)),
                );
            }
            map.insert(
                "clipboard_enabled".to_string(),
                serde_json::Value::Bool(config.clipboard_enabled),
            );
            if config.audio_redirect {
                map.insert("audio_redirect".to_string(), serde_json::Value::Bool(true));
            }
            if config.printer_enabled {
                map.insert("printer_enabled".to_string(), serde_json::Value::Bool(true));
            }
            if config.disable_nla {
                map.insert("nla_disabled".to_string(), serde_json::Value::Bool(true));
            }
            if let Some(jump_id) = config.jump_host_id {
                map.insert(
                    "jump_host".to_string(),
                    serde_json::Value::String(resolve_jump(jump_id)),
                );
            }
        }
        ProtocolConfig::Vnc(config) => {
            if let Some(jump_id) = config.jump_host_id {
                map.insert(
                    "jump_host".to_string(),
                    serde_json::Value::String(resolve_jump(jump_id)),
                );
            }
        }
        ProtocolConfig::Spice(config) => {
            if let Some(jump_id) = config.jump_host_id {
                map.insert(
                    "jump_host".to_string(),
                    serde_json::Value::String(resolve_jump(jump_id)),
                );
            }
        }
        ProtocolConfig::Serial(config) => {
            map.insert(
                "device".to_string(),
                serde_json::Value::String(config.device.clone()),
            );
            map.insert(
                "baud_rate".to_string(),
                serde_json::Value::String(config.baud_rate.display_name().to_string()),
            );
            map.insert(
                "data_bits".to_string(),
                serde_json::Value::String(config.data_bits.display_name().to_string()),
            );
            map.insert(
                "parity".to_string(),
                serde_json::Value::String(format!("{:?}", config.parity)),
            );
            map.insert(
                "stop_bits".to_string(),
                serde_json::Value::String(config.stop_bits.display_name().to_string()),
            );
            map.insert(
                "flow_control".to_string(),
                serde_json::Value::String(config.flow_control.display_name().to_string()),
            );
        }
        ProtocolConfig::ZeroTrust(zt_config) => {
            map.insert(
                "provider".to_string(),
                serde_json::Value::String(zt_config.provider.display_name().to_string()),
            );
            if !zt_config.custom_args.is_empty() {
                map.insert(
                    "custom_args".to_string(),
                    serde_json::json!(zt_config.custom_args),
                );
            }
        }
        ProtocolConfig::Sftp(config) => {
            if let Some(jump_id) = config.jump_host_id {
                map.insert(
                    "jump_host".to_string(),
                    serde_json::Value::String(resolve_jump(jump_id)),
                );
            }
        }
        ProtocolConfig::Web(config) => {
            map.insert(
                "browser_mode".to_string(),
                serde_json::Value::String(format!("{:?}", config.browser_mode)),
            );
            map.insert(
                "javascript_enabled".to_string(),
                serde_json::Value::Bool(config.javascript_enabled),
            );
            if let Some(ref browser) = config.browser {
                map.insert(
                    "browser_command".to_string(),
                    serde_json::Value::String(browser.clone()),
                );
            }
            if let Some(ref ua) = config.user_agent {
                map.insert(
                    "user_agent".to_string(),
                    serde_json::Value::String(ua.clone()),
                );
            }
            if config.accept_invalid_certs {
                map.insert(
                    "accept_invalid_certs".to_string(),
                    serde_json::Value::Bool(true),
                );
            }
            if config.private_mode {
                map.insert("private_mode".to_string(), serde_json::Value::Bool(true));
            }
            if (config.zoom_level - 1.0).abs() > f64::EPSILON {
                map.insert(
                    "zoom_level".to_string(),
                    serde_json::json!(config.zoom_level),
                );
            }
        }
        _ => {}
    }

    // Monitoring config
    if let Some(ref mon) = connection.monitoring_config {
        map.insert(
            "monitoring_enabled".to_string(),
            serde_json::json!(mon.enabled),
        );
        if let Some(interval) = mon.interval_secs {
            map.insert(
                "monitoring_interval_secs".to_string(),
                serde_json::json!(interval),
            );
        }
    }

    let json = serde_json::to_string_pretty(&obj)
        .map_err(|e| CliError::Config(format!("JSON serialization failed: {e}")))?;
    println!("{json}");
    Ok(())
}

/// Print connection details as CSV (key,value pairs).
fn print_csv(connection: &Connection) -> Result<(), CliError> {
    println!("field,value");
    println!("id,{}", connection.id);
    println!("name,{}", crate::format::escape_csv_field(&connection.name));
    println!("host,{}", crate::format::escape_csv_field(&connection.host));
    println!("port,{}", connection.port);
    println!("protocol,{}", connection.protocol.as_str());
    if let Some(ref user) = connection.username {
        println!("username,{}", crate::format::escape_csv_field(user));
    }
    if !connection.tags.is_empty() {
        println!(
            "tags,{}",
            crate::format::escape_csv_field(&connection.tags.join(";"))
        );
    }
    if let Some(ref desc) = connection.description {
        println!("description,{}", crate::format::escape_csv_field(desc));
    }
    println!("created_at,{}", connection.created_at.to_rfc3339());
    println!("updated_at,{}", connection.updated_at.to_rfc3339());
    if let Some(last) = connection.last_connected {
        println!("last_connected,{}", last.to_rfc3339());
    }
    Ok(())
}

/// Print connection details as human-readable table (original behavior).
#[expect(
    clippy::too_many_lines,
    reason = "table output enumerates every persisted Connection field with a label; \
              splitting per protocol only fragments the linear render"
)]
fn print_table(connection: &Connection, connections: &[Connection]) -> Result<(), CliError> {
    let resolve_jump = |jump_id: uuid::Uuid| -> String {
        connections
            .iter()
            .find(|c| c.id == jump_id)
            .map_or_else(|| jump_id.to_string(), |c| c.name.clone())
    };

    println!("Connection Details:");
    println!("  ID:       {}", connection.id);
    println!("  Name:     {}", connection.name);
    println!("  Host:     {}", connection.host);
    println!("  Port:     {}", connection.port);
    println!("  Protocol: {}", connection.protocol);

    if let Some(ref desc) = connection.description {
        println!("  Description: {desc}");
    }
    if let Some(ref icon) = connection.icon {
        println!("  Icon:     {icon}");
    }
    if connection.is_pinned {
        println!("  Pinned:   yes");
    }

    if let Some(ref user) = connection.username {
        println!("  Username: {user}");
    }

    if !connection.tags.is_empty() {
        println!("  Tags:     {}", connection.tags.join(", "));
    }

    if let Some(last) = connection.last_connected {
        println!("  Last used: {}", last.format("%Y-%m-%d %H:%M:%S"));
    }

    match connection.protocol_config {
        ProtocolConfig::Ssh(ref config) => {
            let method = match config.auth_method {
                SshAuthMethod::Password => "password",
                SshAuthMethod::PublicKey => "publickey",
                SshAuthMethod::KeyboardInteractive => "keyboard-interactive",
                SshAuthMethod::Agent => "agent",
                SshAuthMethod::SecurityKey => "security-key",
            };
            println!("  Auth:     {method}");
            if let Some(ref key) = config.key_path {
                println!("  Key Path: {}", key.display());
            }
            if let Some(ref jump) = config.proxy_jump {
                println!("  Proxy Jump: {jump}");
            }
            if let Some(jump_id) = config.jump_host_id {
                println!("  Jump Host: {}", resolve_jump(jump_id));
            }
            if let Some(ref socket) = config.ssh_agent_socket {
                println!("  SSH Agent Socket: {socket}");
            }
        }
        ProtocolConfig::Rdp(ref config) => {
            if let Some(ref domain) = connection.domain {
                println!("  Domain:   {domain}");
            }
            if let Some(ref res) = config.resolution {
                println!("  Resolution: {}x{}", res.width, res.height);
            }
            if config.disable_nla {
                println!("  NLA:      disabled");
            }
            if !matches!(
                config.security_layer,
                rustconn_core::models::RdpSecurityLayer::Negotiate
            ) {
                println!("  Security: {}", config.security_layer.display_name());
            }
            if let Some(level) = config.tls_security_level {
                println!("  TLS Level: {level}");
            }
            if !config.clipboard_enabled {
                println!("  Clipboard: disabled");
            }
            if config.audio_redirect {
                println!("  Audio:    enabled");
            }
            if config.printer_enabled {
                println!("  Printer:  enabled");
            }
            if let Some(jump_id) = config.jump_host_id {
                println!("  Jump Host: {}", resolve_jump(jump_id));
            }
            if config.autotype_delay_ms != 20 {
                println!("  Autotype Delay: {}ms", config.autotype_delay_ms);
            }
            if config.autotype_initial_delay_ms != 0 {
                println!(
                    "  Autotype Initial Delay: {}ms",
                    config.autotype_initial_delay_ms
                );
            }
        }
        ProtocolConfig::Serial(ref config) => {
            println!("  Device:   {}", config.device);
            println!("  Baud:     {}", config.baud_rate.display_name());
            println!(
                "  Config:   {}{}{} flow={}",
                config.data_bits.display_name(),
                match config.parity {
                    rustconn_core::models::SerialParity::None => "N",
                    rustconn_core::models::SerialParity::Odd => "O",
                    rustconn_core::models::SerialParity::Even => "E",
                },
                match config.stop_bits {
                    rustconn_core::models::SerialStopBits::One => "1",
                    rustconn_core::models::SerialStopBits::Two => "2",
                },
                config.flow_control.display_name(),
            );
        }
        ProtocolConfig::Sftp(ref config) => {
            if let Some(ref socket) = config.ssh_agent_socket {
                println!("  SSH Agent Socket: {socket}");
            }
            if let Some(jump_id) = config.jump_host_id {
                println!("  Jump Host: {}", resolve_jump(jump_id));
            }
        }
        ProtocolConfig::ZeroTrust(ref zt_config) => {
            println!("  Provider: {}", zt_config.provider);
            if let rustconn_core::models::ZeroTrustProviderConfig::HoopDev(ref cfg) =
                zt_config.provider_config
            {
                println!("  Connection Name: {}", cfg.connection_name);
                if let Some(ref url) = cfg.gateway_url {
                    println!("  Gateway URL: {url}");
                }
                if let Some(ref url) = cfg.grpc_url {
                    println!("  gRPC URL: {url}");
                }
            }
            if !zt_config.custom_args.is_empty() {
                println!("  Custom Args: {}", zt_config.custom_args.join(" "));
            }
        }
        ProtocolConfig::Vnc(ref config) => {
            if let Some(jump_id) = config.jump_host_id {
                println!("  Jump Host: {}", resolve_jump(jump_id));
            }
        }
        ProtocolConfig::Spice(ref config) => {
            if let Some(jump_id) = config.jump_host_id {
                println!("  Jump Host: {}", resolve_jump(jump_id));
            }
        }
        ProtocolConfig::Web(ref config) => {
            println!("  Browser Mode: {:?}", config.browser_mode);
            println!(
                "  JavaScript:   {}",
                if config.javascript_enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            );
            if let Some(ref browser) = config.browser {
                println!("  Browser Cmd:  {browser}");
            }
            if let Some(ref ua) = config.user_agent {
                println!("  User Agent:   {ua}");
            }
            if config.accept_invalid_certs {
                println!("  Accept Invalid TLS: yes");
            }
            if config.private_mode {
                println!("  Private Mode: yes");
            }
            if (config.zoom_level - 1.0).abs() > f64::EPSILON {
                println!("  Zoom Level:   {:.0}%", config.zoom_level * 100.0);
            }
        }
        _ => {}
    }

    if let Some(ref mon) = connection.monitoring_config {
        let enabled = mon
            .enabled
            .map_or("global", |e| if e { "yes" } else { "no" });
        println!("  Monitoring: {enabled}");
        if let Some(interval) = mon.interval_secs {
            println!("  Mon. interval: {interval}s");
        }
    }

    Ok(())
}
