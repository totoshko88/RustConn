//! Delete connection command.

use std::path::Path;

use crate::error::CliError;
use crate::util::{create_config_manager, find_connection};

/// Delete connection command handler
pub fn cmd_delete(config_path: Option<&Path>, name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let connection = find_connection(&connections, name)?;
    let id = connection.id;
    let conn_name = connection.name.clone();

    let mut connections = connections;
    connections.retain(|c| c.id != id);

    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    println!("Deleted connection '{conn_name}' (ID: {id})");

    Ok(())
}
