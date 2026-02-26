//! Show connection details command.

use std::path::Path;

use rustconn_core::models::SshAuthMethod;

use crate::error::CliError;
use crate::util::{create_config_manager, find_connection};

/// Show connection details command handler
pub fn cmd_show(config_path: Option<&Path>, name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let connection = find_connection(&connections, name)?;

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

    match connection.protocol_config {
        rustconn_core::models::ProtocolConfig::Ssh(ref config) => {
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
        }
        rustconn_core::models::ProtocolConfig::Rdp(ref config) => {
            if let Some(ref domain) = connection.domain {
                println!("  Domain:   {domain}");
            }
            if let Some(ref res) = config.resolution {
                println!("  Resolution: {}x{}", res.width, res.height);
            }
        }
        rustconn_core::models::ProtocolConfig::Serial(ref config) => {
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
