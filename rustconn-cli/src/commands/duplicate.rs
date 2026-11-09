//! Duplicate connection command.

use std::path::Path;

use crate::error::CliError;
use crate::util::{create_config_manager, find_connection};

/// Duplicate a connection
///
/// # Errors
///
/// Returns:
/// - [`CliError::Config`] when connections cannot be loaded or saved
/// - [`CliError::ConnectionNotFound`] when no connection matches `name`
pub(super) fn cmd_duplicate(
    config_path: Option<&Path>,
    name: &str,
    new_name: Option<&str>,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let source = find_connection(&connections, name)?;

    let duplicate = source.duplicate_as(
        new_name
            .map(String::from)
            .unwrap_or_else(|| format!("{} (copy)", source.name)),
    );

    let id = duplicate.id;
    let dup_name = duplicate.name.clone();
    connections.push(duplicate);

    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    println!("Created duplicate connection '{dup_name}' (ID: {id})");

    Ok(())
}

/// Duplicate a connection into one copy per number of a range
///
/// # Errors
///
/// Returns [`CliError::Config`] when the configuration cannot be read or saved,
/// a pattern is unusable, or the host pattern cannot be derived;
/// [`CliError::ConnectionNotFound`] when no connection matches `name`.
pub(super) fn cmd_duplicate_range(
    config_path: Option<&Path>,
    name: &str,
    name_pattern: &str,
    host_pattern: Option<&str>,
) -> Result<(), CliError> {
    use rustconn_core::connection::host_range::{derive_host_pattern, plan_range_duplicates};

    let config_manager = create_config_manager(config_path)?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let source = find_connection(&connections, name)?;
    let host_pattern = host_pattern
        .map(String::from)
        .or_else(|| derive_host_pattern(source, name_pattern))
        .ok_or_else(|| {
            CliError::Config(format!(
                "'{}' has host '{}', which does not contain its name; pass --host-range",
                source.name, source.host
            ))
        })?;

    let plan = plan_range_duplicates(source, name_pattern, &host_pattern, |candidate| {
        connections
            .iter()
            .any(|c| c.name.eq_ignore_ascii_case(candidate))
    })
    .map_err(|e| CliError::Config(e.to_string()))?;

    for copy in &plan.created {
        println!("Created '{}' ({})", copy.name, copy.host);
    }
    for skipped in &plan.skipped {
        println!("Skipped '{skipped}': name already exists");
    }

    if !plan.created.is_empty() {
        connections.extend(plan.created);
        config_manager
            .save_connections(&connections)
            .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;
    }

    Ok(())
}
