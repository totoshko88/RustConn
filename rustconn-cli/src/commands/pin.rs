//! Pin/Unpin connection commands.

use std::path::Path;

use crate::color;
use crate::error::CliError;
use crate::util::{create_config_manager, find_connection};

/// Pin a connection to favorites
pub fn cmd_pin(config_path: Option<&Path>, name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let conn = find_connection(&connections, name)?;
    let conn_id = conn.id;
    let conn_name = conn.name.clone();

    if conn.is_pinned {
        println!("Connection '{}' is already pinned.", conn_name);
        return Ok(());
    }

    // Find the highest pin_order to place this one after
    let max_order = connections
        .iter()
        .filter(|c| c.is_pinned)
        .map(|c| c.pin_order)
        .max()
        .unwrap_or(0);

    let target = connections
        .iter_mut()
        .find(|c| c.id == conn_id)
        .ok_or_else(|| CliError::ConnectionNotFound(name.to_string()))?;

    target.set_pinned(true, max_order + 1);

    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    println!(
        "{}Pinned{} connection '{}'.",
        color::green(),
        color::reset(),
        conn_name
    );
    Ok(())
}

/// Unpin a connection from favorites
pub fn cmd_unpin(config_path: Option<&Path>, name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let conn = find_connection(&connections, name)?;
    let conn_id = conn.id;
    let conn_name = conn.name.clone();

    if !conn.is_pinned {
        println!("Connection '{}' is not pinned.", conn_name);
        return Ok(());
    }

    let target = connections
        .iter_mut()
        .find(|c| c.id == conn_id)
        .ok_or_else(|| CliError::ConnectionNotFound(name.to_string()))?;

    target.set_pinned(false, 0);

    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    println!(
        "{}Unpinned{} connection '{}'.",
        color::yellow(),
        color::reset(),
        conn_name
    );
    Ok(())
}
