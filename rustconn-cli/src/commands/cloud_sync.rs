//! Handlers for Cloud Sync CLI subcommands.
//!
//! Implements `sync status`, `sync list`, `sync export`, `sync import`,
//! `sync now`, and delegates `sync inventory` to the existing inventory
//! sync handler.

use std::collections::HashSet;
use std::path::Path;

use rustconn_core::sync::SyncManager;
use rustconn_core::sync::settings::SyncMode;

use super::sync::cmd_sync;
use crate::cli::{OutputFormat, SyncCommands};
use crate::color;
use crate::error::CliError;
use crate::format::escape_csv_field;
use crate::util::create_config_manager;

/// Dispatches a `sync` subcommand to the appropriate handler.
///
/// # Errors
///
/// Returns:
/// - [`CliError::Config`] when configuration or sync state cannot be read or written
/// - [`CliError::Import`] / [`CliError::Export`] when an inventory file is
///   missing, malformed, or cannot be written
pub(super) fn cmd_cloud_sync(
    config_path: Option<&Path>,
    subcmd: SyncCommands,
) -> Result<(), CliError> {
    match subcmd {
        SyncCommands::Status => cmd_sync_status(config_path),
        SyncCommands::List { format } => cmd_sync_list(config_path, format.effective()),
        SyncCommands::Export { group } => cmd_sync_export(config_path, &group),
        SyncCommands::Import { file } => cmd_sync_import(config_path, &file),
        SyncCommands::Now => cmd_sync_now(config_path),
        SyncCommands::Inventory {
            file,
            source,
            remove_stale,
            dry_run,
        } => cmd_sync(config_path, &file, &source, remove_stale, dry_run),
    }
}

/// `sync status` — displays sync directory, device name, and per-group sync status.
fn cmd_sync_status(config_path: Option<&Path>) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;
    let settings = config_manager
        .load_settings()
        .map_err(|e| CliError::Config(format!("Failed to load settings: {e}")))?;
    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;

    let sync = &settings.sync;

    // Sync directory
    let green = color::green();
    let yellow = color::yellow();
    let red = color::red();
    let reset = if color::enabled() { "\x1b[0m" } else { "" };

    println!("Cloud Sync Status");
    println!("─────────────────");

    match &sync.sync_dir {
        Some(dir) => {
            let status = if dir.is_dir() {
                format!("{green}configured{reset}")
            } else {
                format!("{red}not accessible{reset}")
            };
            println!("  Sync directory: {} ({})", dir.display(), status);
        }
        None => {
            println!("  Sync directory: {yellow}not configured{reset}");
        }
    }

    println!("  Device name:    {}", sync.device_name);
    println!("  Device ID:      {}", sync.device_id);
    println!("  Auto-import:    {}", sync.auto_import_on_start);
    println!("  Export debounce: {}s", sync.export_debounce_secs);

    // Per-group sync status
    let synced_groups: Vec<_> = groups
        .iter()
        .filter(|g| g.sync_mode != SyncMode::None)
        .collect();

    if synced_groups.is_empty() {
        println!("\n  No synced groups.");
    } else {
        println!("\n  Synced groups ({}):", synced_groups.len());
        for group in &synced_groups {
            let mode = match group.sync_mode {
                SyncMode::Master => format!("{green}Master{reset}"),
                SyncMode::Import => format!("{yellow}Import{reset}"),
                SyncMode::None => "None".to_owned(),
            };
            let file = group.sync_file.as_deref().unwrap_or("—");
            let last_sync = group
                .last_synced_at
                .map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "never".to_owned());
            println!(
                "    {} — {} | file: {} | last sync: {}",
                group.name, mode, file, last_sync
            );
        }
    }

    Ok(())
}

/// `sync list` — lists all synced groups with mode and last sync time.
fn cmd_sync_list(config_path: Option<&Path>, format: OutputFormat) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;
    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;

    let synced: Vec<_> = groups
        .iter()
        .filter(|g| g.sync_mode != SyncMode::None)
        .collect();

    match format {
        OutputFormat::Table => print_sync_list_table(&synced),
        OutputFormat::Json => print_sync_list_json(&synced)?,
        OutputFormat::Csv => print_sync_list_csv(&synced),
    }

    Ok(())
}

fn print_sync_list_table(groups: &[&rustconn_core::models::ConnectionGroup]) {
    if groups.is_empty() {
        println!("No synced groups.");
        return;
    }

    // Column widths
    let name_w = groups
        .iter()
        .map(|g| g.name.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let mode_w = 6; // "Master" is longest
    let file_w = groups
        .iter()
        .map(|g| g.sync_file.as_deref().unwrap_or("—").len())
        .max()
        .unwrap_or(4)
        .max(4);

    let green = color::green();
    let yellow = color::yellow();
    let reset = if color::enabled() { "\x1b[0m" } else { "" };

    println!(
        "{:<name_w$}  {:<mode_w$}  {:<file_w$}  LAST SYNC",
        "NAME", "MODE", "FILE"
    );
    println!(
        "{:─<name_w$}  {:─<mode_w$}  {:─<file_w$}  {:─<19}",
        "", "", "", ""
    );

    for group in groups {
        let mode_str = match group.sync_mode {
            SyncMode::Master => format!("{green}Master{reset}"),
            SyncMode::Import => format!("{yellow}Import{reset}"),
            SyncMode::None => "None".to_owned(),
        };
        // Plain mode string for width calculation
        let mode_plain = match group.sync_mode {
            SyncMode::Master => "Master",
            SyncMode::Import => "Import",
            SyncMode::None => "None",
        };
        let file = group.sync_file.as_deref().unwrap_or("—");
        let last_sync = group
            .last_synced_at
            .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "never".to_owned());

        // Print with padding adjusted for ANSI escape codes
        let mode_padding = mode_w.saturating_sub(mode_plain.len());
        println!(
            "{:<name_w$}  {}{:padding$}  {:<file_w$}  {}",
            group.name,
            mode_str,
            "",
            file,
            last_sync,
            padding = mode_padding,
        );
    }
}

fn print_sync_list_json(
    groups: &[&rustconn_core::models::ConnectionGroup],
) -> Result<(), CliError> {
    let items: Vec<serde_json::Value> = groups
        .iter()
        .map(|g| {
            serde_json::json!({
                "name": g.name,
                "mode": format!("{:?}", g.sync_mode),
                "sync_file": g.sync_file,
                "last_synced_at": g.last_synced_at.map(|t| t.to_rfc3339()),
            })
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(&items)
            .map_err(|e| CliError::Export(format!("JSON serialization failed: {e}")))?
    );
    Ok(())
}

fn print_sync_list_csv(groups: &[&rustconn_core::models::ConnectionGroup]) {
    println!("name,mode,sync_file,last_synced_at");
    for g in groups {
        let mode = format!("{:?}", g.sync_mode);
        let file = g.sync_file.as_deref().unwrap_or("");
        let last = g.last_synced_at.map(|t| t.to_rfc3339()).unwrap_or_default();
        println!(
            "{},{},{},{}",
            escape_csv_field(&g.name),
            escape_csv_field(&mode),
            escape_csv_field(file),
            escape_csv_field(&last),
        );
    }
}

/// `sync export <group>` — exports the specified Master group to its sync file.
fn cmd_sync_export(config_path: Option<&Path>, group_name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;
    let settings = config_manager
        .load_settings()
        .map_err(|e| CliError::Config(format!("Failed to load settings: {e}")))?;
    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;
    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;
    let variables = config_manager
        .load_variables()
        .map_err(|e| CliError::Config(format!("Failed to load variables: {e}")))?;

    // Find the group by name (case-insensitive) or UUID
    let group = find_group(&groups, group_name)?;

    if group.sync_mode != SyncMode::Master {
        return Err(CliError::Config(format!(
            "Group '{}' is not in Master mode (current mode: {:?})",
            group.name, group.sync_mode
        )));
    }

    let mut sync_manager = SyncManager::new(settings.sync);

    let app_version = env!("CARGO_PKG_VERSION");
    let report = sync_manager
        .export_group(group.id, &groups, &connections, &variables, app_version)
        .map_err(|e| CliError::Config(format!("Export failed: {e}")))?;

    let green = color::green();
    let reset = if color::enabled() { "\x1b[0m" } else { "" };

    println!(
        "{green}Exported{reset} group '{}': {} connections, {} groups, {} variable templates",
        report.group_name, report.connections_added, report.groups_added, report.variables_created,
    );

    Ok(())
}

/// `sync import <file>` — imports the specified .rcn file.
fn cmd_sync_import(config_path: Option<&Path>, file_path: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;
    let settings = config_manager
        .load_settings()
        .map_err(|e| CliError::Config(format!("Failed to load settings: {e}")))?;
    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;
    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;
    let variables = config_manager
        .load_variables()
        .map_err(|e| CliError::Config(format!("Failed to load variables: {e}")))?;

    // Find Import group that references this file
    let file_name = std::path::Path::new(file_path)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or(file_path);

    let import_group = groups
        .iter()
        .find(|g| g.sync_mode == SyncMode::Import && g.sync_file.as_deref() == Some(file_name))
        .ok_or_else(|| {
            CliError::Config(format!(
                "No Import group found for file '{}'. \
                 Configure an Import group in the GUI first, or use the full file path.",
                file_name
            ))
        })?;

    let import_group_id = import_group.id;
    let local_variable_names: HashSet<String> = variables.iter().map(|v| v.name.clone()).collect();

    let mut sync_manager = SyncManager::new(settings.sync);

    let (merge_result, report) = sync_manager
        .import_group(
            import_group_id,
            &groups,
            &connections,
            &local_variable_names,
        )
        .map_err(|e| CliError::Config(format!("Import failed: {e}")))?;

    let green = color::green();
    let reset = if color::enabled() { "\x1b[0m" } else { "" };

    println!(
        "{green}Imported{reset} '{}': +{} connections, ~{} updated, -{} removed, {} variables",
        report.group_name,
        report.connections_added,
        report.connections_updated,
        report.connections_removed,
        report.variables_created,
    );

    // Apply merge result to local data store
    let mut connections = connections;
    let mut groups = groups;

    // Create new connections from remote
    for sync_conn in &merge_result.connections_to_create {
        let new_conn = rustconn_core::sync::group_export::sync_connection_to_connection(
            sync_conn,
            import_group_id,
        );
        connections.push(new_conn);
    }

    // Update existing connections
    for (local_id, sync_conn) in &merge_result.connections_to_update {
        if let Some(conn) = connections.iter_mut().find(|c| c.id == *local_id) {
            rustconn_core::sync::group_export::apply_sync_connection_update(conn, sync_conn);
        }
    }

    // Delete connections removed from remote
    let delete_ids: std::collections::HashSet<_> =
        merge_result.connections_to_delete.iter().collect();
    connections.retain(|c| !delete_ids.contains(&c.id));

    // Delete groups removed from remote
    let delete_group_ids: std::collections::HashSet<_> =
        merge_result.groups_to_delete.iter().collect();
    groups.retain(|g| !delete_group_ids.contains(&g.id));

    // Save updated data
    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    config_manager
        .save_groups(&groups)
        .map_err(|e| CliError::Config(format!("Failed to save groups: {e}")))?;

    if !merge_result.connections_to_create.is_empty()
        || !merge_result.connections_to_update.is_empty()
        || !merge_result.connections_to_delete.is_empty()
        || !merge_result.groups_to_create.is_empty()
        || !merge_result.groups_to_delete.is_empty()
    {
        println!("\nChanges applied to local data store.");
    } else {
        println!("\nAlready up to date.");
    }

    Ok(())
}

/// `sync now` — exports all Master groups and imports all Import groups.
fn cmd_sync_now(config_path: Option<&Path>) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;
    let settings = config_manager
        .load_settings()
        .map_err(|e| CliError::Config(format!("Failed to load settings: {e}")))?;
    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;
    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;
    let variables = config_manager
        .load_variables()
        .map_err(|e| CliError::Config(format!("Failed to load variables: {e}")))?;

    let mut sync_manager = SyncManager::new(settings.sync);
    let app_version = env!("CARGO_PKG_VERSION");

    let green = color::green();
    let yellow = color::yellow();
    let red = color::red();
    let reset = if color::enabled() { "\x1b[0m" } else { "" };

    // Phase 1: Export all Master groups
    let master_groups: Vec<_> = groups
        .iter()
        .filter(|g| g.sync_mode == SyncMode::Master && g.is_root())
        .collect();

    if master_groups.is_empty() {
        println!("No Master groups to export.");
    } else {
        println!("Exporting {} Master group(s)...", master_groups.len());
        for group in &master_groups {
            match sync_manager.export_group(
                group.id,
                &groups,
                &connections,
                &variables,
                app_version,
            ) {
                Ok(report) => {
                    println!(
                        "  {green}✓{reset} {} — {} connections exported",
                        report.group_name, report.connections_added,
                    );
                }
                Err(e) => {
                    println!("  {red}✗{reset} {} — export failed: {e}", group.name);
                }
            }
        }
    }

    // Phase 2: Import all Import groups
    let import_groups: Vec<_> = groups
        .iter()
        .filter(|g| g.sync_mode == SyncMode::Import && g.sync_file.is_some())
        .collect();

    let local_variable_names: HashSet<String> = variables.iter().map(|v| v.name.clone()).collect();

    if import_groups.is_empty() {
        println!("No Import groups to sync.");
    } else {
        println!("\nImporting {} Import group(s)...", import_groups.len());
        for group in &import_groups {
            match sync_manager.import_group(group.id, &groups, &connections, &local_variable_names)
            {
                Ok((_merge_result, report)) => {
                    println!(
                        "  {green}✓{reset} {} — +{} ~{} -{}",
                        report.group_name,
                        report.connections_added,
                        report.connections_updated,
                        report.connections_removed,
                    );
                }
                Err(e) => {
                    println!("  {yellow}⚠{reset} {} — import failed: {e}", group.name);
                }
            }
        }
    }

    println!("\n{green}Sync complete.{reset}");

    Ok(())
}

/// Finds a group by name (case-insensitive) or UUID.
fn find_group<'a>(
    groups: &'a [rustconn_core::models::ConnectionGroup],
    name_or_id: &str,
) -> Result<&'a rustconn_core::models::ConnectionGroup, CliError> {
    // Exact name match
    if let Some(g) = groups.iter().find(|g| g.name == name_or_id) {
        return Ok(g);
    }

    // UUID match
    if let Ok(uuid) = uuid::Uuid::parse_str(name_or_id)
        && let Some(g) = groups.iter().find(|g| g.id == uuid)
    {
        return Ok(g);
    }

    // Case-insensitive match
    if let Some(g) = groups
        .iter()
        .find(|g| g.name.eq_ignore_ascii_case(name_or_id))
    {
        return Ok(g);
    }

    Err(CliError::Config(format!("Group not found: '{name_or_id}'")))
}
