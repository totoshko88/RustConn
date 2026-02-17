//! Wake-on-LAN command.

use std::path::Path;

use rustconn_core::wol::{MacAddress, WolConfig};

use crate::error::CliError;
use crate::util::{create_config_manager, find_connection};

/// Wake-on-LAN command handler
pub fn cmd_wol(
    config_path: Option<&Path>,
    target: &str,
    broadcast: &str,
    port: u16,
) -> Result<(), CliError> {
    let mac = if let Ok(mac) = target.parse::<MacAddress>() {
        mac
    } else {
        let config_manager = create_config_manager(config_path)?;

        let connections = config_manager
            .load_connections()
            .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

        let connection = find_connection(&connections, target)?;

        connection
            .wol_config
            .as_ref()
            .map(|wol| wol.mac_address)
            .ok_or_else(|| {
                CliError::Wol(format!(
                    "Connection '{}' does not have Wake-on-LAN \
                     configured",
                    connection.name
                ))
            })?
    };

    let config = WolConfig::new(mac)
        .with_broadcast_address(broadcast)
        .with_port(port);

    println!("Sending Wake-on-LAN magic packet...");
    println!("  MAC Address: {mac}");
    println!("  Broadcast:   {broadcast}:{port}");

    rustconn_core::wol::send_wol_with_retry(&config, 3, 500)
        .map_err(|e| CliError::Wol(e.to_string()))?;

    println!("Magic packet sent successfully (3 packets)!");
    println!(
        "Note: The target machine may take up to {} seconds to \
         wake up.",
        config.wait_seconds
    );

    Ok(())
}
