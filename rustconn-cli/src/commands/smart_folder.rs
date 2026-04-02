//! Smart folder management commands.

use std::path::Path;

use rustconn_core::models::SmartFolder;
use rustconn_core::smart_folder::SmartFolderManager;

use crate::cli::{OutputFormat, SmartFolderCommands};
use crate::error::CliError;
use crate::format::escape_csv_field;
use crate::util::{create_config_manager, parse_protocol_type};

/// Smart folder command handler
pub fn cmd_smart_folder(
    config_path: Option<&Path>,
    subcmd: SmartFolderCommands,
) -> Result<(), CliError> {
    match subcmd {
        SmartFolderCommands::List { format } => {
            cmd_smart_folder_list(config_path, format.effective())
        }
        SmartFolderCommands::Show { name } => cmd_smart_folder_show(config_path, &name),
        SmartFolderCommands::Create {
            name,
            protocol,
            host_pattern,
            tags,
        } => cmd_smart_folder_create(
            config_path,
            &name,
            protocol.as_deref(),
            host_pattern.as_deref(),
            tags.as_deref(),
        ),
        SmartFolderCommands::Delete { name } => cmd_smart_folder_delete(config_path, &name),
    }
}

fn cmd_smart_folder_list(config_path: Option<&Path>, format: OutputFormat) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;
    let settings = config_manager
        .load_settings()
        .map_err(|e| CliError::SmartFolder(format!("Failed to load settings: {e}")))?;

    let folders = &settings.smart_folders;

    match format {
        OutputFormat::Table => print_table(folders),
        OutputFormat::Json => print_json(folders)?,
        OutputFormat::Csv => print_csv(folders),
    }

    Ok(())
}

fn print_table(folders: &[SmartFolder]) {
    if folders.is_empty() {
        println!("No smart folders found.");
        return;
    }

    let name_width = folders
        .iter()
        .map(|f| f.name.len())
        .max()
        .unwrap_or(4)
        .max(4);

    println!("{:<name_width$}  PROTOCOL    HOST PATTERN", "NAME");
    println!("{:-<name_width$}  {:-<10}  {:-<20}", "", "", "");

    for folder in folders {
        let proto = folder
            .filter_protocol
            .as_ref()
            .map_or_else(|| "-".to_string(), |p| format!("{p:?}").to_lowercase());
        let host = folder.filter_host_pattern.as_deref().unwrap_or("-");
        println!("{:<name_width$}  {proto:<10}  {host}", folder.name);
    }
}

fn print_json(folders: &[SmartFolder]) -> Result<(), CliError> {
    let json = serde_json::to_string_pretty(folders)
        .map_err(|e| CliError::SmartFolder(format!("Failed to serialize: {e}")))?;
    println!("{json}");
    Ok(())
}

fn print_csv(folders: &[SmartFolder]) {
    println!("name,protocol,host_pattern,tags");
    for folder in folders {
        let name = escape_csv_field(&folder.name);
        let proto = folder
            .filter_protocol
            .as_ref()
            .map_or_else(String::new, |p| format!("{p:?}").to_lowercase());
        let host = folder
            .filter_host_pattern
            .as_deref()
            .unwrap_or_default()
            .to_string();
        let tags = folder.filter_tags.join(";");
        println!(
            "{name},{proto},{},{}",
            escape_csv_field(&host),
            escape_csv_field(&tags)
        );
    }
}

fn cmd_smart_folder_show(config_path: Option<&Path>, name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;
    let settings = config_manager
        .load_settings()
        .map_err(|e| CliError::SmartFolder(format!("Failed to load settings: {e}")))?;
    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let folder = find_smart_folder(&settings.smart_folders, name)?;

    println!("Smart Folder: {}", folder.name);
    println!("  ID: {}", folder.id);
    if let Some(ref proto) = folder.filter_protocol {
        println!("  Protocol: {proto:?}");
    }
    if let Some(ref pattern) = folder.filter_host_pattern {
        println!("  Host Pattern: {pattern}");
    }
    if !folder.filter_tags.is_empty() {
        println!("  Tags: {}", folder.filter_tags.join(", "));
    }

    let manager = SmartFolderManager::new();
    let matched = manager.evaluate(folder, &connections);

    println!("\nMatching connections ({}):", matched.len());
    for conn in &matched {
        println!("  - {} ({})", conn.name, conn.host);
    }

    Ok(())
}

fn cmd_smart_folder_create(
    config_path: Option<&Path>,
    name: &str,
    protocol: Option<&str>,
    host_pattern: Option<&str>,
    tags: Option<&str>,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;
    let mut settings = config_manager
        .load_settings()
        .map_err(|e| CliError::SmartFolder(format!("Failed to load settings: {e}")))?;

    if settings
        .smart_folders
        .iter()
        .any(|f| f.name.eq_ignore_ascii_case(name))
    {
        return Err(CliError::SmartFolder(format!(
            "Smart folder with name '{name}' already exists"
        )));
    }

    let filter_protocol = protocol.map(parse_protocol_type).transpose()?;

    let filter_tags = tags
        .map(|t| t.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    let folder = SmartFolder {
        id: uuid::Uuid::new_v4(),
        name: name.to_string(),
        filter_protocol,
        filter_tags,
        filter_host_pattern: host_pattern.map(String::from),
        filter_group_id: None,
        sort_order: settings.smart_folders.len() as i32,
    };

    let id = folder.id;
    settings.smart_folders.push(folder);

    config_manager
        .save_settings(&settings)
        .map_err(|e| CliError::SmartFolder(format!("Failed to save settings: {e}")))?;

    println!("Created smart folder '{name}' with ID {id}");

    Ok(())
}

fn cmd_smart_folder_delete(config_path: Option<&Path>, name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;
    let mut settings = config_manager
        .load_settings()
        .map_err(|e| CliError::SmartFolder(format!("Failed to load settings: {e}")))?;

    let folder = find_smart_folder(&settings.smart_folders, name)?;
    let id = folder.id;
    let folder_name = folder.name.clone();

    settings.smart_folders.retain(|f| f.id != id);

    config_manager
        .save_settings(&settings)
        .map_err(|e| CliError::SmartFolder(format!("Failed to save settings: {e}")))?;

    println!("Deleted smart folder '{folder_name}' (ID: {id})");

    Ok(())
}

/// Find a smart folder by name or ID
fn find_smart_folder<'a>(
    folders: &'a [SmartFolder],
    name_or_id: &str,
) -> Result<&'a SmartFolder, CliError> {
    if let Ok(uuid) = uuid::Uuid::parse_str(name_or_id)
        && let Some(folder) = folders.iter().find(|f| f.id == uuid)
    {
        return Ok(folder);
    }

    let matches: Vec<_> = folders
        .iter()
        .filter(|f| f.name.eq_ignore_ascii_case(name_or_id))
        .collect();

    match matches.len() {
        0 => Err(CliError::SmartFolder(format!(
            "Smart folder not found: {name_or_id}"
        ))),
        1 => Ok(matches[0]),
        _ => Err(CliError::SmartFolder(format!(
            "Ambiguous smart folder name: {name_or_id}"
        ))),
    }
}
