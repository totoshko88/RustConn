//! Dynamic folder management commands.

use std::path::Path;

use rustconn_core::config::ConfigManager;
use rustconn_core::connection::ConnectionManager;
use rustconn_core::dynamic_folder;

use crate::cli::{DynamicFolderCommands, OutputFormat};
use crate::error::CliError;
use crate::util::create_config_manager;

/// Dynamic folder command handler
///
/// # Errors
///
/// Returns:
/// - [`CliError::Config`] when configuration cannot be read or written
/// - [`CliError::DynamicFolder`] when a dynamic folder operation fails
///   (duplicate name, missing folder, invalid filter expression)
pub(super) fn cmd_dynamic_folder(
    config_path: Option<&Path>,
    subcmd: DynamicFolderCommands,
) -> Result<(), CliError> {
    match subcmd {
        DynamicFolderCommands::List { format } => {
            cmd_dynamic_folder_list(config_path, format.effective())
        }
        DynamicFolderCommands::Show { name } => cmd_dynamic_folder_show(config_path, &name),
        DynamicFolderCommands::Refresh { name } => cmd_dynamic_folder_refresh(config_path, &name),
        DynamicFolderCommands::Set {
            name,
            script,
            workdir,
            timeout,
            refresh_interval,
        } => cmd_dynamic_folder_set(
            config_path,
            &name,
            &script,
            workdir.as_deref(),
            timeout,
            refresh_interval,
        ),
        DynamicFolderCommands::Remove { name } => cmd_dynamic_folder_remove(config_path, &name),
    }
}

fn cmd_dynamic_folder_list(
    config_path: Option<&Path>,
    format: OutputFormat,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;
    let conn_manager = load_connection_manager(&config_manager)?;

    let groups: Vec<_> = conn_manager
        .list_groups()
        .into_iter()
        .filter(|g| g.dynamic_folder.is_some())
        .collect();

    if groups.is_empty() {
        println!("No dynamic folders configured.");
        return Ok(());
    }

    match format {
        OutputFormat::Table => {
            let name_width = groups
                .iter()
                .map(|g| g.name.len())
                .max()
                .unwrap_or(4)
                .max(4);
            println!(
                "{:<name_width$}  {:>10}  {:>7}  LAST REFRESHED",
                "GROUP", "TIMEOUT", "REFRESH"
            );
            println!("{:-<name_width$}  {:-<10}  {:-<7}  {:-<20}", "", "", "", "");

            for group in &groups {
                // `groups` was filtered on `dynamic_folder.is_some()` above.
                let df = group
                    .dynamic_folder
                    .as_ref()
                    .unwrap_or_else(|| unreachable!("groups pre-filtered to dynamic folders"));
                let refresh = df
                    .refresh_interval_secs
                    .map_or_else(|| "manual".to_string(), |s| format!("{s}s"));
                let last = df.last_refreshed_at.map_or_else(
                    || "never".to_string(),
                    |t| t.format("%Y-%m-%d %H:%M").to_string(),
                );
                println!(
                    "{:<name_width$}  {:>8}s  {:>7}  {last}",
                    group.name, df.timeout_secs, refresh
                );
            }
        }
        OutputFormat::Json => {
            let json_groups: Vec<_> = groups
                .iter()
                .map(|g| {
                    // `groups` was filtered on `dynamic_folder.is_some()` above.
                    let df = g
                        .dynamic_folder
                        .as_ref()
                        .unwrap_or_else(|| unreachable!("groups pre-filtered to dynamic folders"));
                    serde_json::json!({
                        "id": g.id.to_string(),
                        "name": g.name,
                        "script": df.script,
                        "working_directory": df.working_directory,
                        "timeout_secs": df.timeout_secs,
                        "refresh_interval_secs": df.refresh_interval_secs,
                        "last_refreshed_at": df.last_refreshed_at,
                        "last_error": df.last_error,
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json_groups)
                    .map_err(|e| CliError::DynamicFolder(e.to_string()))?
            );
        }
        OutputFormat::Csv => {
            println!("group,script,timeout_secs,refresh_interval_secs,last_refreshed_at");
            for group in &groups {
                // `groups` was filtered on `dynamic_folder.is_some()` above.
                let df = group
                    .dynamic_folder
                    .as_ref()
                    .unwrap_or_else(|| unreachable!("groups pre-filtered to dynamic folders"));
                let refresh = df
                    .refresh_interval_secs
                    .map_or_else(String::new, |s| s.to_string());
                let last = df
                    .last_refreshed_at
                    .map_or_else(String::new, |t| t.to_rfc3339());
                println!(
                    "{},{},{},{},{}",
                    crate::format::escape_csv_field(&group.name),
                    crate::format::escape_csv_field(&df.script),
                    df.timeout_secs,
                    refresh,
                    last
                );
            }
        }
    }

    Ok(())
}

fn cmd_dynamic_folder_show(config_path: Option<&Path>, name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;
    let conn_manager = load_connection_manager(&config_manager)?;

    let group = find_group_with_dynamic_folder(&conn_manager, name)?;
    let df = group
        .dynamic_folder
        .as_ref()
        .ok_or_else(|| CliError::DynamicFolder("Group has no dynamic folder".to_string()))?;

    println!("Group:             {}", group.name);
    println!("ID:                {}", group.id);
    println!("Script:            {}", df.script);
    if let Some(ref dir) = df.working_directory {
        println!("Working Directory: {}", dir.display());
    }
    println!("Timeout:           {}s", df.timeout_secs);
    println!(
        "Refresh Interval:  {}",
        df.refresh_interval_secs
            .map_or_else(|| "manual".to_string(), |s| format!("{s}s"))
    );
    println!(
        "Last Refreshed:    {}",
        df.last_refreshed_at.map_or_else(
            || "never".to_string(),
            |t| t.format("%Y-%m-%d %H:%M:%S").to_string()
        )
    );
    if let Some(ref err) = df.last_error {
        println!("Last Error:        {err}");
    }

    // Show generated connections
    let connections: Vec<_> = conn_manager
        .list_connections()
        .into_iter()
        .filter(|c| c.group_id == Some(group.id) && c.is_dynamic)
        .collect();

    if connections.is_empty() {
        println!("\nNo generated connections (run `dynamic-folder refresh` first).");
    } else {
        println!("\nGenerated Connections ({}):", connections.len());
        let name_w = connections
            .iter()
            .map(|c| c.name.len())
            .max()
            .unwrap_or(4)
            .max(4);
        println!("  {:<name_w$}  HOST", "NAME");
        for conn in &connections {
            println!("  {:<name_w$}  {}", conn.name, conn.host);
        }
    }

    Ok(())
}

fn cmd_dynamic_folder_refresh(config_path: Option<&Path>, name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;
    let mut conn_manager = load_connection_manager(&config_manager)?;

    let group = find_group_with_dynamic_folder(&conn_manager, name)?;
    let group_id = group.id;
    let group_name = group.name.clone();
    let config = group
        .dynamic_folder
        .ok_or_else(|| CliError::DynamicFolder("Group has no dynamic folder".to_string()))?;

    println!("Refreshing dynamic folder '{group_name}'...");

    // Execute the script
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| CliError::DynamicFolder(format!("Failed to create runtime: {e}")))?;

    let result = rt
        .block_on(dynamic_folder::execute_script(&config))
        .map_err(|e| CliError::DynamicFolder(e.to_string()))?;

    // Print warnings
    for warning in &result.warnings {
        eprintln!("  Warning: {warning}");
    }

    // Remove old dynamic connections across the folder's whole subtree —
    // entries may have been placed in sub-groups by a previous refresh, not
    // only directly under the base group.
    let subtree = dynamic_folder::descendant_group_ids(group_id, &conn_manager.list_groups_owned());
    let old_dynamic: Vec<uuid::Uuid> = conn_manager
        .list_connections()
        .into_iter()
        .filter(|c| c.is_dynamic && c.group_id.is_some_and(|g| subtree.contains(&g)))
        .map(|c| c.id)
        .collect();

    for conn_id in old_dynamic {
        let _ = conn_manager.delete_connection(conn_id);
    }

    // Materialise the refresh: create any sub-groups the entries' `group` paths
    // ask for (idempotent — stable ids, skip existing), then the connections.
    let plan = dynamic_folder::plan_dynamic_refresh(group_id, &result.entries);
    let existing_group_ids: std::collections::HashSet<uuid::Uuid> =
        conn_manager.list_groups().iter().map(|g| g.id).collect();
    for subgroup in plan.subgroups {
        if !existing_group_ids.contains(&subgroup.id) {
            let _ = conn_manager.create_group_from(subgroup);
        }
    }
    for conn in plan.connections {
        let _ = conn_manager.create_connection_from(conn);
    }

    // Update group's last_refreshed_at
    if let Some(mut group) = conn_manager.get_group(group_id).cloned() {
        if let Some(ref mut df) = group.dynamic_folder {
            df.last_refreshed_at = Some(chrono::Utc::now());
            df.last_error = None;
        }
        let _ = conn_manager.update_group(group_id, group);
    }

    println!(
        "Done: {} connections generated in {:.1}s",
        result.entries.len(),
        result.duration.as_secs_f64()
    );

    Ok(())
}

/// Finds a group by name or UUID that has a dynamic folder configured.
fn find_group_with_dynamic_folder(
    conn_manager: &ConnectionManager,
    name: &str,
) -> Result<rustconn_core::ConnectionGroup, CliError> {
    // Try UUID first
    if let Ok(id) = uuid::Uuid::parse_str(name)
        && let Some(group) = conn_manager.get_group(id)
    {
        if group.dynamic_folder.is_some() {
            return Ok(group.clone());
        }
        return Err(CliError::DynamicFolder(format!(
            "Group '{}' has no dynamic folder configured",
            group.name
        )));
    }

    // Search by name (case-insensitive)
    let name_lower = name.to_lowercase();
    let matches: Vec<_> = conn_manager
        .list_groups()
        .into_iter()
        .filter(|g| g.name.to_lowercase() == name_lower && g.dynamic_folder.is_some())
        .collect();

    match matches.len() {
        0 => Err(CliError::DynamicFolder(format!(
            "No dynamic folder group found matching '{name}'"
        ))),
        1 => Ok(matches[0].clone()),
        _ => Err(CliError::DynamicFolder(format!(
            "Multiple groups match '{name}'. Use the group UUID instead."
        ))),
    }
}

/// Loads a ConnectionManager from the config.
fn load_connection_manager(config_manager: &ConfigManager) -> Result<ConnectionManager, CliError> {
    ConnectionManager::new(config_manager.clone())
        .map_err(|e| CliError::DynamicFolder(format!("Failed to load connections: {e}")))
}

fn cmd_dynamic_folder_set(
    config_path: Option<&Path>,
    name: &str,
    script: &str,
    workdir: Option<&str>,
    timeout: u64,
    refresh_interval: u64,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;
    let mut conn_manager = load_connection_manager(&config_manager)?;

    let group = find_group_by_name(&conn_manager, name)?;
    let group_id = group.id;
    let group_name = group.name.clone();

    let mut config = rustconn_core::DynamicFolderConfig::new(script.to_string());
    if let Some(dir) = workdir {
        config.working_directory = Some(std::path::PathBuf::from(dir));
    }
    config.timeout_secs = timeout;
    config.refresh_interval_secs = if refresh_interval > 0 {
        Some(refresh_interval)
    } else {
        None
    };

    // Preserve existing last_refreshed_at if updating
    if let Some(ref existing) = group.dynamic_folder {
        config.last_refreshed_at = existing.last_refreshed_at;
    }

    let mut updated = group.clone();
    updated.dynamic_folder = Some(config);
    conn_manager
        .update_group(group_id, updated)
        .map_err(|e| CliError::DynamicFolder(format!("Failed to update group: {e}")))?;

    let action = if group.dynamic_folder.is_some() {
        "Updated"
    } else {
        "Created"
    };
    println!("{action} dynamic folder on group '{group_name}'.");
    Ok(())
}

fn cmd_dynamic_folder_remove(config_path: Option<&Path>, name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;
    let mut conn_manager = load_connection_manager(&config_manager)?;

    let group = find_group_with_dynamic_folder(&conn_manager, name)?;
    let group_id = group.id;
    let group_name = group.name.clone();

    // Remove dynamic connections
    let dynamic_conns: Vec<uuid::Uuid> = conn_manager
        .list_connections()
        .into_iter()
        .filter(|c| c.group_id == Some(group_id) && c.is_dynamic)
        .map(|c| c.id)
        .collect();

    for conn_id in dynamic_conns {
        let _ = conn_manager.delete_connection(conn_id);
    }

    // Clear dynamic folder config
    let mut updated = group;
    updated.dynamic_folder = None;
    conn_manager
        .update_group(group_id, updated)
        .map_err(|e| CliError::DynamicFolder(format!("Failed to update group: {e}")))?;

    println!("Removed dynamic folder from group '{group_name}'.");
    Ok(())
}

/// Finds a group by name or UUID (does not require dynamic folder).
fn find_group_by_name(
    conn_manager: &ConnectionManager,
    name: &str,
) -> Result<rustconn_core::ConnectionGroup, CliError> {
    // Try UUID first
    if let Ok(id) = uuid::Uuid::parse_str(name)
        && let Some(group) = conn_manager.get_group(id)
    {
        return Ok(group.clone());
    }

    // Search by name (case-insensitive)
    let name_lower = name.to_lowercase();
    let matches: Vec<_> = conn_manager
        .list_groups()
        .into_iter()
        .filter(|g| g.name.to_lowercase() == name_lower)
        .collect();

    match matches.len() {
        0 => Err(CliError::DynamicFolder(format!(
            "No group found matching '{name}'"
        ))),
        1 => Ok(matches[0].clone()),
        _ => Err(CliError::DynamicFolder(format!(
            "Multiple groups match '{name}'. Use the group UUID instead."
        ))),
    }
}
