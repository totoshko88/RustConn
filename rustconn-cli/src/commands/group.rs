//! Group management commands.

use std::path::Path;

use rustconn_core::models::ConnectionGroup;

use crate::cli::{GroupCommands, OutputFormat};
use crate::error::CliError;
use crate::format::escape_csv_field;
use crate::util::create_config_manager;

/// Group command handler
pub fn cmd_group(config_path: Option<&Path>, subcmd: GroupCommands) -> Result<(), CliError> {
    match subcmd {
        GroupCommands::List { format } => cmd_group_list(config_path, format.effective()),
        GroupCommands::Show { name } => cmd_group_show(config_path, &name),
        GroupCommands::Create {
            name,
            parent,
            description,
            icon,
        } => cmd_group_create(
            config_path,
            &name,
            parent.as_deref(),
            description.as_deref(),
            icon.as_deref(),
        ),
        GroupCommands::Delete { name } => cmd_group_delete(config_path, &name),
        GroupCommands::AddConnection { group, connection } => {
            cmd_group_add_connection(config_path, &group, &connection)
        }
        GroupCommands::RemoveConnection { group, connection } => {
            cmd_group_remove_connection(config_path, &group, &connection)
        }
    }
}

fn cmd_group_list(config_path: Option<&Path>, format: OutputFormat) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Group(format!("Failed to load groups: {e}")))?;

    match format {
        OutputFormat::Table => print_group_table(&groups),
        OutputFormat::Json => print_group_json(&groups)?,
        OutputFormat::Csv => print_group_csv(&groups),
    }

    Ok(())
}

fn print_group_table(groups: &[ConnectionGroup]) {
    if groups.is_empty() {
        println!("No groups found.");
        return;
    }

    let name_width = groups
        .iter()
        .map(|g| g.name.len())
        .max()
        .unwrap_or(4)
        .max(4);

    println!("{:<name_width$}  PARENT", "NAME");
    println!("{:-<name_width$}  {:-<20}", "", "");

    for group in groups {
        let parent = group.parent_id.map_or_else(
            || "-".to_string(),
            |id| {
                groups
                    .iter()
                    .find(|g| g.id == id)
                    .map_or_else(|| id.to_string(), |g| g.name.clone())
            },
        );
        let parent_display = if parent.len() > 20 {
            format!("{}...", &parent[..17])
        } else {
            parent
        };
        println!("{:<name_width$}  {parent_display}", group.name);
    }
}

fn print_group_json(groups: &[ConnectionGroup]) -> Result<(), CliError> {
    let json = serde_json::to_string_pretty(groups)
        .map_err(|e| CliError::Group(format!("Failed to serialize: {e}")))?;
    println!("{json}");
    Ok(())
}

fn print_group_csv(groups: &[ConnectionGroup]) {
    println!("name,parent_id");
    for group in groups {
        let name = escape_csv_field(&group.name);
        let parent = group.parent_id.map(|id| id.to_string()).unwrap_or_default();
        println!("{name},{parent}");
    }
}

fn cmd_group_show(config_path: Option<&Path>, name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Group(format!("Failed to load groups: {e}")))?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let group = find_group(&groups, name)?;

    println!("Group Details:");
    println!("  ID:   {}", group.id);
    println!("  Name: {}", group.name);

    if let Some(ref desc) = group.description {
        println!("  Description: {desc}");
    }
    if let Some(ref icon) = group.icon {
        println!("  Icon: {icon}");
    }

    if let Some(parent_id) = group.parent_id {
        let parent_name = groups
            .iter()
            .find(|g| g.id == parent_id)
            .map_or("(unknown)", |g| g.name.as_str());
        println!("  Parent: {parent_name} ({parent_id})");
    }

    let group_connections: Vec<_> = connections
        .iter()
        .filter(|c| c.group_id == Some(group.id))
        .collect();

    println!("\nConnections ({}):", group_connections.len());
    for conn in &group_connections {
        println!("  - {} ({})", conn.name, conn.host);
    }

    Ok(())
}

fn cmd_group_create(
    config_path: Option<&Path>,
    name: &str,
    parent: Option<&str>,
    description: Option<&str>,
    icon: Option<&str>,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let mut groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Group(format!("Failed to load groups: {e}")))?;

    if groups.iter().any(|g| g.name.eq_ignore_ascii_case(name)) {
        return Err(CliError::Group(format!(
            "Group with name '{name}' already exists"
        )));
    }

    let mut group = if let Some(parent_name) = parent {
        let parent_group = find_group(&groups, parent_name)?;
        ConnectionGroup::with_parent(name.to_string(), parent_group.id)
    } else {
        ConnectionGroup::new(name.to_string())
    };

    if let Some(desc) = description {
        group.description = Some(desc.to_string());
    }
    if let Some(ic) = icon {
        group.icon = Some(ic.to_string());
    }

    let id = group.id;
    groups.push(group);

    config_manager
        .save_groups(&groups)
        .map_err(|e| CliError::Group(format!("Failed to save groups: {e}")))?;

    println!("Created group '{name}' with ID {id}");

    Ok(())
}

fn cmd_group_delete(config_path: Option<&Path>, name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let mut groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Group(format!("Failed to load groups: {e}")))?;

    let group = find_group(&groups, name)?;
    let id = group.id;
    let group_name = group.name.clone();

    groups.retain(|g| g.id != id);

    config_manager
        .save_groups(&groups)
        .map_err(|e| CliError::Group(format!("Failed to save groups: {e}")))?;

    println!("Deleted group '{group_name}' (ID: {id})");

    Ok(())
}

fn cmd_group_add_connection(
    config_path: Option<&Path>,
    group_name: &str,
    connection_name: &str,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Group(format!("Failed to load groups: {e}")))?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let group = find_group(&groups, group_name)?;
    let group_id = group.id;
    let grp_name = group.name.clone();

    let connection = connections
        .iter_mut()
        .find(|c| {
            c.name.eq_ignore_ascii_case(connection_name) || c.id.to_string() == connection_name
        })
        .ok_or_else(|| CliError::ConnectionNotFound(connection_name.to_string()))?;

    if connection.group_id == Some(group_id) {
        return Err(CliError::Group(format!(
            "Connection '{}' is already in group '{grp_name}'",
            connection.name
        )));
    }

    let conn_name = connection.name.clone();
    connection.group_id = Some(group_id);

    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    println!("Added connection '{conn_name}' to group '{grp_name}'");

    Ok(())
}

fn cmd_group_remove_connection(
    config_path: Option<&Path>,
    group_name: &str,
    connection_name: &str,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Group(format!("Failed to load groups: {e}")))?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let group = find_group(&groups, group_name)?;
    let group_id = group.id;
    let grp_name = group.name.clone();

    let connection = connections
        .iter_mut()
        .find(|c| {
            c.name.eq_ignore_ascii_case(connection_name) || c.id.to_string() == connection_name
        })
        .ok_or_else(|| CliError::ConnectionNotFound(connection_name.to_string()))?;

    if connection.group_id != Some(group_id) {
        return Err(CliError::Group(format!(
            "Connection '{}' is not in group '{grp_name}'",
            connection.name
        )));
    }

    let conn_name = connection.name.clone();
    connection.group_id = None;

    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    println!("Removed connection '{conn_name}' from group '{grp_name}'");

    Ok(())
}

/// Find a group by name or ID
fn find_group<'a>(
    groups: &'a [ConnectionGroup],
    name_or_id: &str,
) -> Result<&'a ConnectionGroup, CliError> {
    if let Ok(uuid) = uuid::Uuid::parse_str(name_or_id)
        && let Some(group) = groups.iter().find(|g| g.id == uuid)
    {
        return Ok(group);
    }

    let matches: Vec<_> = groups
        .iter()
        .filter(|g| g.name.eq_ignore_ascii_case(name_or_id))
        .collect();

    match matches.len() {
        0 => Err(CliError::Group(format!("Group not found: {name_or_id}"))),
        1 => Ok(matches[0]),
        _ => Err(CliError::Group(format!(
            "Ambiguous group name: {name_or_id}"
        ))),
    }
}
