//! List connections command.

use std::fmt::Write as _;
use std::path::Path;

use rustconn_core::models::{Connection, SshAuthMethod};

use crate::cli::OutputFormat;
use crate::error::CliError;
use crate::format::escape_csv_field;
use crate::util::create_config_manager;

/// List connections command handler
pub fn cmd_list(
    config_path: Option<&Path>,
    format: OutputFormat,
    protocol: Option<&str>,
    group: Option<&str>,
    tag: Option<&str>,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;

    // Find group ID if group filter is specified
    let group_id: Option<uuid::Uuid> = group
        .map(|group_filter| {
            let group_lower = group_filter.to_lowercase();
            groups
                .iter()
                .find(|g| g.name.to_lowercase() == group_lower)
                .map(|g| g.id)
                .ok_or_else(|| CliError::Group(format!("Group not found: {group_filter}")))
        })
        .transpose()?;

    // Filter connections
    let filtered: Vec<&Connection> = connections
        .iter()
        .filter(|c| {
            // Filter by protocol
            if let Some(proto_filter) = protocol {
                if c.protocol.as_str() != proto_filter.to_lowercase() {
                    return false;
                }
            }

            // Filter by group
            if let Some(gid) = group_id {
                if c.group_id != Some(gid) {
                    return false;
                }
            }

            // Filter by tag
            if let Some(tag_filter) = tag {
                let tag_lower = tag_filter.to_lowercase();
                if !c.tags.iter().any(|t| t.to_lowercase() == tag_lower) {
                    return false;
                }
            }

            true
        })
        .collect();

    match format {
        OutputFormat::Table => print_table(&filtered),
        OutputFormat::Json => print_json(&filtered)?,
        OutputFormat::Csv => print_csv(&filtered),
    }

    Ok(())
}

/// Print connections as a formatted table
fn print_table(connections: &[&Connection]) {
    println!("{}", format_table(connections));
}

/// Format connections as a table string
#[must_use]
pub fn format_table(connections: &[&Connection]) -> String {
    if connections.is_empty() {
        return "No connections found.".to_string();
    }

    let mut output = String::new();

    // Calculate column widths
    let name_width = connections
        .iter()
        .map(|c| c.name.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let host_width = connections
        .iter()
        .map(|c| c.host.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let protocol_width = 8;
    let port_width = 5;

    // Print header
    let _ = writeln!(
        output,
        "{:<name_width$}  {:<host_width$}  \
         {:<port_width$}  {:<protocol_width$}",
        "NAME", "HOST", "PORT", "PROTOCOL"
    );
    let _ = writeln!(
        output,
        "{:-<name_width$}  {:-<host_width$}  \
         {:-<port_width$}  {:-<protocol_width$}",
        "", "", "", ""
    );

    // Print rows
    for conn in connections {
        let _ = writeln!(
            output,
            "{:<name_width$}  {:<host_width$}  \
             {:<port_width$}  {:<protocol_width$}",
            conn.name, conn.host, conn.port, conn.protocol
        );
    }

    output.trim_end().to_string()
}

/// Print connections as JSON
fn print_json(connections: &[&Connection]) -> Result<(), CliError> {
    let json = format_json(connections)?;
    println!("{json}");
    Ok(())
}

/// Format connections as JSON string
///
/// # Errors
///
/// Returns `CliError::Config` if JSON serialization fails.
pub fn format_json(connections: &[&Connection]) -> Result<String, CliError> {
    let output: Vec<ConnectionOutput> = connections.iter().map(|c| (*c).into()).collect();
    serde_json::to_string_pretty(&output)
        .map_err(|e| CliError::Config(format!("Failed to serialize to JSON: {e}")))
}

/// Print connections as CSV
fn print_csv(connections: &[&Connection]) {
    println!("{}", format_csv(connections));
}

/// Format connections as CSV string
#[must_use]
pub fn format_csv(connections: &[&Connection]) -> String {
    let mut output = String::new();

    // Print header
    output.push_str("name,host,port,protocol\n");

    // Print rows
    for conn in connections {
        let name = escape_csv_field(&conn.name);
        let host = escape_csv_field(&conn.host);
        let _ = writeln!(
            output,
            "{},{},{},{}",
            name,
            host,
            conn.port,
            conn.protocol.as_str()
        );
    }

    output.trim_end().to_string()
}

/// Simplified connection output for CLI
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConnectionOutput {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub protocol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baud_rate: Option<String>,
}

impl From<&Connection> for ConnectionOutput {
    fn from(conn: &Connection) -> Self {
        let auth_method =
            if let rustconn_core::models::ProtocolConfig::Ssh(ref cfg) = conn.protocol_config {
                let s = match cfg.auth_method {
                    SshAuthMethod::Password => "password",
                    SshAuthMethod::PublicKey => "publickey",
                    SshAuthMethod::KeyboardInteractive => "keyboard-interactive",
                    SshAuthMethod::Agent => "agent",
                    SshAuthMethod::SecurityKey => "security-key",
                };
                Some(s.to_string())
            } else {
                None
            };

        let (device, baud_rate) =
            if let rustconn_core::models::ProtocolConfig::Serial(ref cfg) = conn.protocol_config {
                (
                    Some(cfg.device.clone()),
                    Some(cfg.baud_rate.display_name().to_string()),
                )
            } else {
                (None, None)
            };

        Self {
            id: conn.id.to_string(),
            name: conn.name.clone(),
            host: conn.host.clone(),
            port: conn.port,
            protocol: conn.protocol.as_str().to_string(),
            username: conn.username.clone(),
            auth_method,
            device,
            baud_rate,
        }
    }
}
