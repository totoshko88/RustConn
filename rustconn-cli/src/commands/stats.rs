//! Connection statistics command.

use std::collections::HashMap;
use std::path::Path;

use crate::error::CliError;
use crate::util::create_config_manager;

/// Show connection statistics
pub fn cmd_stats(config_path: Option<&Path>) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;

    let templates = config_manager
        .load_templates()
        .map_err(|e| CliError::Config(format!("Failed to load templates: {e}")))?;

    let clusters = config_manager
        .load_clusters()
        .map_err(|e| CliError::Config(format!("Failed to load clusters: {e}")))?;

    let snippets_count = config_manager.load_snippets().map(|s| s.len()).unwrap_or(0);

    let variables = config_manager
        .load_variables()
        .map_err(|e| CliError::Config(format!("Failed to load variables: {e}")))?;

    let mut by_protocol: HashMap<String, usize> = HashMap::new();
    for conn in &connections {
        *by_protocol
            .entry(conn.protocol.as_str().to_string())
            .or_insert(0) += 1;
    }

    let week_ago = chrono::Utc::now() - chrono::Duration::days(7);
    let recent_count = connections
        .iter()
        .filter(|c| c.last_connected.is_some_and(|t| t > week_ago))
        .count();

    let ever_used = connections
        .iter()
        .filter(|c| c.last_connected.is_some())
        .count();

    println!("RustConn Statistics");
    println!("===================\n");

    println!("Connections: {}", connections.len());
    for (proto, count) in &by_protocol {
        println!("  {proto}: {count}");
    }

    println!("\nGroups:     {}", groups.len());
    println!("Templates:  {}", templates.len());
    println!("Clusters:   {}", clusters.len());
    println!("Snippets:   {snippets_count}");
    println!("Variables:  {}", variables.len());

    println!("\nUsage:");
    println!("  Recently used (7 days): {recent_count}");
    println!("  Ever connected: {ever_used}");

    Ok(())
}
