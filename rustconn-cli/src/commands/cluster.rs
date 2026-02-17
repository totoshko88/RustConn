//! Cluster management commands.

use std::path::Path;

use rustconn_core::cluster::Cluster;

use crate::cli::{ClusterCommands, OutputFormat};
use crate::error::CliError;
use crate::format::escape_csv_field;
use crate::util::{create_config_manager, find_connection};

/// Cluster command handler
pub fn cmd_cluster(config_path: Option<&Path>, subcmd: ClusterCommands) -> Result<(), CliError> {
    match subcmd {
        ClusterCommands::List { format } => cmd_cluster_list(config_path, format),
        ClusterCommands::Show { name } => cmd_cluster_show(config_path, &name),
        ClusterCommands::Create {
            name,
            connections,
            broadcast,
        } => cmd_cluster_create(config_path, &name, connections.as_deref(), broadcast),
        ClusterCommands::Delete { name } => cmd_cluster_delete(config_path, &name),
        ClusterCommands::AddConnection {
            cluster,
            connection,
        } => cmd_cluster_add_connection(config_path, &cluster, &connection),
        ClusterCommands::RemoveConnection {
            cluster,
            connection,
        } => cmd_cluster_remove_connection(config_path, &cluster, &connection),
    }
}

fn cmd_cluster_list(config_path: Option<&Path>, format: OutputFormat) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let clusters = config_manager
        .load_clusters()
        .map_err(|e| CliError::Cluster(format!("Failed to load clusters: {e}")))?;

    match format {
        OutputFormat::Table => print_cluster_table(&clusters),
        OutputFormat::Json => print_cluster_json(&clusters)?,
        OutputFormat::Csv => print_cluster_csv(&clusters),
    }

    Ok(())
}

fn print_cluster_table(clusters: &[Cluster]) {
    if clusters.is_empty() {
        println!("No clusters found.");
        return;
    }

    let name_width = clusters
        .iter()
        .map(|c| c.name.len())
        .max()
        .unwrap_or(4)
        .max(4);

    println!("{:<name_width$}  CONNECTIONS  BROADCAST", "NAME");
    println!("{:-<name_width$}  {:-<11}  {:-<9}", "", "", "");

    for cluster in clusters {
        let broadcast = if cluster.broadcast_enabled {
            "Yes"
        } else {
            "No"
        };
        println!(
            "{:<name_width$}  {:<11}  {broadcast}",
            cluster.name,
            cluster.connection_count()
        );
    }
}

fn print_cluster_json(clusters: &[Cluster]) -> Result<(), CliError> {
    let json = serde_json::to_string_pretty(clusters)
        .map_err(|e| CliError::Cluster(format!("Failed to serialize: {e}")))?;
    println!("{json}");
    Ok(())
}

fn print_cluster_csv(clusters: &[Cluster]) {
    println!("name,connection_count,broadcast_enabled");
    for cluster in clusters {
        let name = escape_csv_field(&cluster.name);
        println!(
            "{name},{},{}",
            cluster.connection_count(),
            cluster.broadcast_enabled
        );
    }
}

fn cmd_cluster_show(config_path: Option<&Path>, name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let clusters = config_manager
        .load_clusters()
        .map_err(|e| CliError::Cluster(format!("Failed to load clusters: {e}")))?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let cluster = find_cluster(&clusters, name)?;

    println!("Cluster Details:");
    println!("  ID:        {}", cluster.id);
    println!("  Name:      {}", cluster.name);
    println!(
        "  Broadcast: {}",
        if cluster.broadcast_enabled {
            "Enabled"
        } else {
            "Disabled"
        }
    );

    println!("\nConnections ({}):", cluster.connection_count());
    for conn_id in &cluster.connection_ids {
        if let Some(conn) = connections.iter().find(|c| c.id == *conn_id) {
            println!(
                "  - {} ({} {}:{})",
                conn.name, conn.protocol, conn.host, conn.port
            );
        } else {
            println!("  - {conn_id} (not found)");
        }
    }

    Ok(())
}

fn cmd_cluster_create(
    config_path: Option<&Path>,
    name: &str,
    connections: Option<&str>,
    broadcast: bool,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let mut clusters = config_manager
        .load_clusters()
        .map_err(|e| CliError::Cluster(format!("Failed to load clusters: {e}")))?;

    let all_connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let mut cluster = Cluster::new(name.to_string());
    cluster.broadcast_enabled = broadcast;

    if let Some(conn_list) = connections {
        for conn_name in conn_list.split(',').map(str::trim) {
            let conn = find_connection(&all_connections, conn_name)?;
            cluster.add_connection(conn.id);
        }
    }

    let id = cluster.id;
    clusters.push(cluster);

    config_manager
        .save_clusters(&clusters)
        .map_err(|e| CliError::Cluster(format!("Failed to save clusters: {e}")))?;

    println!("Created cluster '{name}' with ID {id}");

    Ok(())
}

fn cmd_cluster_delete(config_path: Option<&Path>, name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let mut clusters = config_manager
        .load_clusters()
        .map_err(|e| CliError::Cluster(format!("Failed to load clusters: {e}")))?;

    let cluster = find_cluster(&clusters, name)?;
    let id = cluster.id;
    let cluster_name = cluster.name.clone();

    clusters.retain(|c| c.id != id);

    config_manager
        .save_clusters(&clusters)
        .map_err(|e| CliError::Cluster(format!("Failed to save clusters: {e}")))?;

    println!("Deleted cluster '{cluster_name}' (ID: {id})");

    Ok(())
}

fn cmd_cluster_add_connection(
    config_path: Option<&Path>,
    cluster_name: &str,
    connection_name: &str,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let mut clusters = config_manager
        .load_clusters()
        .map_err(|e| CliError::Cluster(format!("Failed to load clusters: {e}")))?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let connection = find_connection(&connections, connection_name)?;
    let conn_id = connection.id;
    let conn_name = connection.name.clone();

    let cluster = clusters
        .iter_mut()
        .find(|c| c.name.eq_ignore_ascii_case(cluster_name) || c.id.to_string() == cluster_name)
        .ok_or_else(|| CliError::Cluster(format!("Cluster not found: {cluster_name}")))?;

    if cluster.contains_connection(conn_id) {
        return Err(CliError::Cluster(format!(
            "Connection '{conn_name}' is already in cluster '{}'",
            cluster.name
        )));
    }

    let clust_name = cluster.name.clone();
    cluster.add_connection(conn_id);

    config_manager
        .save_clusters(&clusters)
        .map_err(|e| CliError::Cluster(format!("Failed to save clusters: {e}")))?;

    println!("Added connection '{conn_name}' to cluster '{clust_name}'");

    Ok(())
}

fn cmd_cluster_remove_connection(
    config_path: Option<&Path>,
    cluster_name: &str,
    connection_name: &str,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let mut clusters = config_manager
        .load_clusters()
        .map_err(|e| CliError::Cluster(format!("Failed to load clusters: {e}")))?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let connection = find_connection(&connections, connection_name)?;
    let conn_id = connection.id;
    let conn_name = connection.name.clone();

    let cluster = clusters
        .iter_mut()
        .find(|c| c.name.eq_ignore_ascii_case(cluster_name) || c.id.to_string() == cluster_name)
        .ok_or_else(|| CliError::Cluster(format!("Cluster not found: {cluster_name}")))?;

    if !cluster.contains_connection(conn_id) {
        return Err(CliError::Cluster(format!(
            "Connection '{conn_name}' is not in cluster '{}'",
            cluster.name
        )));
    }

    let clust_name = cluster.name.clone();
    cluster.remove_connection(conn_id);

    config_manager
        .save_clusters(&clusters)
        .map_err(|e| CliError::Cluster(format!("Failed to save clusters: {e}")))?;

    println!(
        "Removed connection '{conn_name}' from cluster \
         '{clust_name}'"
    );

    Ok(())
}

/// Find a cluster by name or ID
fn find_cluster<'a>(clusters: &'a [Cluster], name_or_id: &str) -> Result<&'a Cluster, CliError> {
    if let Ok(uuid) = uuid::Uuid::parse_str(name_or_id) {
        if let Some(cluster) = clusters.iter().find(|c| c.id == uuid) {
            return Ok(cluster);
        }
    }

    let matches: Vec<_> = clusters
        .iter()
        .filter(|c| c.name.eq_ignore_ascii_case(name_or_id))
        .collect();

    match matches.len() {
        0 => Err(CliError::Cluster(format!(
            "Cluster not found: {name_or_id}"
        ))),
        1 => Ok(matches[0]),
        _ => Err(CliError::Cluster(format!(
            "Ambiguous cluster name: {name_or_id}"
        ))),
    }
}
