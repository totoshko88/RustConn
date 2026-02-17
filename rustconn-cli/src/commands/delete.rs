//! Delete connection command.

use std::io::IsTerminal;
use std::path::Path;

use crate::error::CliError;
use crate::util::{create_config_manager, find_connection};

/// Prompts the user for confirmation on an interactive terminal.
///
/// Returns `true` if the user confirms (types "y" or "Y"), or if stdin
/// is not a terminal (non-interactive mode assumes yes).
fn confirm(message: &str) -> bool {
    if !std::io::stdin().is_terminal() {
        return true;
    }
    eprint!("{message} [y/N] ");
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .is_ok_and(|_| input.trim().eq_ignore_ascii_case("y"))
}

/// Delete connection command handler
pub fn cmd_delete(config_path: Option<&Path>, name: &str, force: bool) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let connection = find_connection(&connections, name)?;
    let id = connection.id;
    let conn_name = connection.name.clone();
    let protocol = format!("{:?}", connection.protocol);

    if !force && !confirm(&format!("Delete connection '{conn_name}' ({protocol})?")) {
        tracing::info!("Delete aborted by user for '{conn_name}'");
        return Ok(());
    }

    let mut connections = connections;
    connections.retain(|c| c.id != id);

    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    println!("Deleted connection '{conn_name}' (ID: {id})");

    Ok(())
}
