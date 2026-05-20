//! Move connection to a different group command.

use std::path::Path;

use crate::color;
use crate::error::CliError;
use crate::util::{create_config_manager, find_connection, find_or_create_group_id};

/// Move a connection to a different group
pub fn cmd_move(config_path: Option<&Path>, name: &str, group_name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let mut groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;

    let conn = find_connection(&connections, name)?;
    let conn_id = conn.id;
    let conn_name = conn.name.clone();

    let group_id = find_or_create_group_id(&mut groups, group_name)?;

    // Find the target group name (may differ in case from input)
    let resolved_group_name = groups
        .iter()
        .find(|g| g.id == group_id)
        .map(|g| g.name.clone())
        .unwrap_or_else(|| group_name.to_string());

    let target = connections
        .iter_mut()
        .find(|c| c.id == conn_id)
        .ok_or_else(|| CliError::ConnectionNotFound(name.to_string()))?;

    target.group_id = Some(group_id);
    target.touch();

    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    config_manager
        .save_groups(&groups)
        .map_err(|e| CliError::Config(format!("Failed to save groups: {e}")))?;

    println!(
        "{}Moved{} connection '{}' to group '{}'.",
        color::green(),
        color::reset(),
        conn_name,
        resolved_group_name
    );
    Ok(())
}
