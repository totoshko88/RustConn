//! Handler for the `sync` CLI command.

use std::path::Path;

use rustconn_core::sync::{load_inventory, sync_inventory};

use crate::error::CliError;
use crate::util::create_config_manager;

/// Executes the `sync` command.
///
/// Loads an inventory file (JSON/YAML), synchronizes it against the
/// existing connections, and persists the result.
///
/// # Errors
///
/// Returns:
/// - [`CliError::Import`] when the inventory file does not exist or cannot be parsed
/// - [`CliError::Config`] when connections cannot be loaded or saved
pub(super) fn cmd_sync(
    config_path: Option<&Path>,
    file: &Path,
    source: &str,
    remove_stale: bool,
    dry_run: bool,
) -> Result<(), CliError> {
    if !file.exists() {
        return Err(CliError::Import(format!(
            "Inventory file not found: {}",
            file.display()
        )));
    }

    if source.trim().is_empty() {
        return Err(CliError::Config(
            "Source identifier cannot be empty".to_string(),
        ));
    }

    let inventory = load_inventory(file).map_err(|e| CliError::Import(e.to_string()))?;

    println!(
        "Inventory loaded: {} connections from '{}'",
        inventory.connections.len(),
        file.display()
    );

    let config_manager = create_config_manager(config_path)?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let mut groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;

    let initial_conns = connections.len();
    let initial_groups = groups.len();

    let result = sync_inventory(
        &inventory,
        source,
        &mut connections,
        &mut groups,
        remove_stale,
    );

    println!("\nSync results (source: {source}):");
    println!("  Added:   {}", result.added);
    println!("  Updated: {}", result.updated);
    println!("  Removed: {}", result.removed);
    println!("  Skipped: {}", result.skipped);

    for reason in &result.skip_reasons {
        tracing::warn!("{reason}");
    }

    if dry_run {
        println!("\n(dry run — no changes saved)");
        return Ok(());
    }

    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    config_manager
        .save_groups(&groups)
        .map_err(|e| CliError::Config(format!("Failed to save groups: {e}")))?;

    println!(
        "\nTotal connections: {} (was {})",
        connections.len(),
        initial_conns
    );
    println!("Total groups: {} (was {})", groups.len(), initial_groups);

    Ok(())
}
