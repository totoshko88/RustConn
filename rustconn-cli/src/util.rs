//! Shared utility functions used across command modules.

use std::path::Path;

use rustconn_core::config::ConfigManager;
use rustconn_core::models::Connection;

use crate::error::CliError;

/// Creates a `ConfigManager` using the optional custom config directory
/// from CLI args, falling back to the `RUSTCONN_CONFIG_DIR` environment
/// variable when no explicit path is provided.
pub fn create_config_manager(config_path: Option<&Path>) -> Result<ConfigManager, CliError> {
    if let Some(path) = config_path {
        return Ok(ConfigManager::with_config_dir(path.to_path_buf()));
    }

    if let Ok(env_dir) = std::env::var("RUSTCONN_CONFIG_DIR") {
        if !env_dir.is_empty() {
            return Ok(ConfigManager::with_config_dir(std::path::PathBuf::from(
                env_dir,
            )));
        }
    }

    ConfigManager::new().map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))
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
        0 => {
            // Fuzzy substring suggestions (CLI-08)
            let needle = name_or_id.to_lowercase();
            let suggestions: Vec<&str> = connections
                .iter()
                .filter(|c| c.name.to_lowercase().contains(&needle))
                .take(5)
                .map(|c| c.name.as_str())
                .collect();

            if suggestions.is_empty() {
                Err(CliError::ConnectionNotFound(name_or_id.to_string()))
            } else {
                Err(CliError::ConnectionNotFound(format!(
                    "'{}'. Did you mean: {}?",
                    name_or_id,
                    suggestions.join(", ")
                )))
            }
        }
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

/// Outputs content through a pager (`less`) if stdout is a terminal and
/// the content exceeds 40 lines. Falls back to direct printing otherwise.
///
/// # Errors
///
/// Returns `CliError` if writing to the pager fails.
pub fn output_with_pager(content: &str) -> Result<(), CliError> {
    use std::io::{IsTerminal, Write};

    if !std::io::stdout().is_terminal() || content.lines().count() < 40 {
        print!("{content}");
        return Ok(());
    }

    let pager = std::process::Command::new("less")
        .args(["-FIRX"])
        .stdin(std::process::Stdio::piped())
        .spawn();

    if let Ok(mut child) = pager {
        if let Some(ref mut stdin) = child.stdin {
            let _ = stdin.write_all(content.as_bytes());
        }
        let _ = child.wait();
        Ok(())
    } else {
        // Fallback: print directly if less is not available
        print!("{content}");
        Ok(())
    }
}
