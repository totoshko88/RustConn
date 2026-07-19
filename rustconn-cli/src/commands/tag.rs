//! Tag management commands.

use std::collections::BTreeMap;
use std::path::Path;

use crate::cli::{OutputFormat, TagCommands};
use crate::color;
use crate::error::CliError;
use crate::util::{create_config_manager, find_connection};

/// Tag command dispatcher
///
/// # Errors
///
/// Returns:
/// - [`CliError::Config`] when connections cannot be loaded or saved
/// - [`CliError::ConnectionNotFound`] when a referenced connection does not exist
pub(super) fn cmd_tag(config_path: Option<&Path>, subcmd: TagCommands) -> Result<(), CliError> {
    match subcmd {
        TagCommands::List { format } => cmd_tag_list(config_path, format.effective()),
        TagCommands::Add { connection, tag } => cmd_tag_add(config_path, &connection, &tag),
        TagCommands::Remove { connection, tag } => cmd_tag_remove(config_path, &connection, &tag),
    }
}

/// List all tags used across connections with usage counts
fn cmd_tag_list(config_path: Option<&Path>, format: OutputFormat) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let mut tag_counts: BTreeMap<String, usize> = BTreeMap::new();
    for conn in &connections {
        for tag in &conn.tags {
            *tag_counts.entry(tag.clone()).or_insert(0) += 1;
        }
    }

    if tag_counts.is_empty() {
        println!("No tags found.");
        return Ok(());
    }

    match format {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&tag_counts)
                .map_err(|e| CliError::Config(format!("JSON serialization failed: {e}")))?;
            println!("{json}");
        }
        OutputFormat::Csv => {
            println!("tag,count");
            for (tag, count) in &tag_counts {
                println!("{tag},{count}");
            }
        }
        OutputFormat::Table => {
            println!(
                "{}{:<30}  {:<6}{}",
                color::bold(),
                "Tag",
                "Count",
                color::reset(),
            );
            println!("{}", "-".repeat(38));
            for (tag, count) in &tag_counts {
                println!("{:<30}  {:<6}", tag, count);
            }
            println!("\nTotal: {} unique tags.", tag_counts.len());
        }
    }

    Ok(())
}

/// Add a tag to a connection
fn cmd_tag_add(
    config_path: Option<&Path>,
    connection_name: &str,
    tag: &str,
) -> Result<(), CliError> {
    let tag = tag.trim().to_string();
    if tag.is_empty() {
        return Err(CliError::Config("Tag cannot be empty.".to_string()));
    }

    let config_manager = create_config_manager(config_path)?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let conn = find_connection(&connections, connection_name)?;
    let conn_id = conn.id;
    let conn_name = conn.name.clone();

    if conn.tags.contains(&tag) {
        println!("Connection '{}' already has tag '{}'.", conn_name, tag);
        return Ok(());
    }

    let target = connections
        .iter_mut()
        .find(|c| c.id == conn_id)
        .ok_or_else(|| CliError::ConnectionNotFound(connection_name.to_string()))?;

    target.tags.push(tag.clone());
    target.touch();

    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    println!(
        "{}Added{} tag '{}' to connection '{}'.",
        color::green(),
        color::reset(),
        tag,
        conn_name
    );
    Ok(())
}

/// Remove a tag from a connection
fn cmd_tag_remove(
    config_path: Option<&Path>,
    connection_name: &str,
    tag: &str,
) -> Result<(), CliError> {
    let tag = tag.trim();
    if tag.is_empty() {
        return Err(CliError::Config("Tag cannot be empty.".to_string()));
    }

    let config_manager = create_config_manager(config_path)?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let conn = find_connection(&connections, connection_name)?;
    let conn_id = conn.id;
    let conn_name = conn.name.clone();

    if !conn.tags.iter().any(|t| t == tag) {
        println!("Connection '{}' does not have tag '{}'.", conn_name, tag);
        return Ok(());
    }

    let target = connections
        .iter_mut()
        .find(|c| c.id == conn_id)
        .ok_or_else(|| CliError::ConnectionNotFound(connection_name.to_string()))?;

    target.tags.retain(|t| t != tag);
    target.touch();

    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    println!(
        "{}Removed{} tag '{}' from connection '{}'.",
        color::yellow(),
        color::reset(),
        tag,
        conn_name
    );
    Ok(())
}
