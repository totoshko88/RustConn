//! Shared utility functions used across command modules.

use std::path::Path;

use rustconn_core::config::ConfigManager;
use rustconn_core::models::Connection;

use crate::error::CliError;

/// Creates a `ConfigManager` using the optional custom config directory
/// from CLI args.
pub fn create_config_manager(config_path: Option<&Path>) -> Result<ConfigManager, CliError> {
    match config_path {
        Some(path) => Ok(ConfigManager::with_config_dir(path.to_path_buf())),
        None => ConfigManager::new()
            .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}"))),
    }
}

/// Parse a key=value pair for variable substitution
pub fn parse_key_val(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=value: no `=` found in `{s}`"))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

/// Find a connection by name or UUID
pub fn find_connection<'a>(
    connections: &'a [Connection],
    name_or_id: &str,
) -> Result<&'a Connection, CliError> {
    // First try to find by exact name match
    if let Some(conn) = connections.iter().find(|c| c.name == name_or_id) {
        return Ok(conn);
    }

    // Try to find by UUID
    if let Ok(uuid) = uuid::Uuid::parse_str(name_or_id) {
        if let Some(conn) = connections.iter().find(|c| c.id == uuid) {
            return Ok(conn);
        }
    }

    // Try case-insensitive name match
    if let Some(conn) = connections
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(name_or_id))
    {
        return Ok(conn);
    }

    // Try partial name match (prefix)
    let matches: Vec<_> = connections
        .iter()
        .filter(|c| {
            c.name
                .to_lowercase()
                .starts_with(&name_or_id.to_lowercase())
        })
        .collect();

    match matches.len() {
        0 => Err(CliError::ConnectionNotFound(name_or_id.to_string())),
        1 => Ok(matches[0]),
        _ => {
            let names: Vec<_> = matches.iter().map(|c| c.name.as_str()).collect();
            Err(CliError::Config(format!(
                "Ambiguous connection name '{}'. Matches: {}",
                name_or_id,
                names.join(", ")
            )))
        }
    }
}
