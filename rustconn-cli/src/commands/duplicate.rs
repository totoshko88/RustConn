//! Duplicate connection command.

use std::path::Path;

use crate::error::CliError;
use crate::util::{create_config_manager, find_connection};

/// Duplicate a connection
pub fn cmd_duplicate(
    config_path: Option<&Path>,
    name: &str,
    new_name: Option<&str>,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let source = find_connection(&connections, name)?;

    let mut duplicate = source.clone();
    duplicate.id = uuid::Uuid::new_v4();
    duplicate.name = new_name
        .map(String::from)
        .unwrap_or_else(|| format!("{} (copy)", source.name));
    duplicate.created_at = chrono::Utc::now();
    duplicate.updated_at = chrono::Utc::now();
    duplicate.last_connected = None;

    let id = duplicate.id;
    let dup_name = duplicate.name.clone();
    connections.push(duplicate);

    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    println!("Created duplicate connection '{dup_name}' (ID: {id})");

    Ok(())
}
