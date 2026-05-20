//! Per-connection monitoring commands.

use std::path::Path;

use rustconn_core::monitoring::MonitoringConfig;

use crate::cli::{MonitorCommands, OutputFormat};
use crate::color;
use crate::error::CliError;
use crate::util::{create_config_manager, find_connection};

/// Monitor command dispatcher
pub fn cmd_monitor(config_path: Option<&Path>, subcmd: MonitorCommands) -> Result<(), CliError> {
    match subcmd {
        MonitorCommands::Enable { name, interval } => {
            cmd_monitor_enable(config_path, &name, interval)
        }
        MonitorCommands::Disable { name } => cmd_monitor_disable(config_path, &name),
        MonitorCommands::Metrics { name, format } => {
            cmd_monitor_metrics(config_path, &name, format.effective())
        }
    }
}

/// Enable monitoring for a connection
fn cmd_monitor_enable(
    config_path: Option<&Path>,
    name: &str,
    interval: Option<u8>,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let conn = find_connection(&connections, name)?;
    let conn_id = conn.id;
    let conn_name = conn.name.clone();

    let target = connections
        .iter_mut()
        .find(|c| c.id == conn_id)
        .ok_or_else(|| CliError::ConnectionNotFound(name.to_string()))?;

    target.monitoring_config = Some(MonitoringConfig {
        enabled: Some(true),
        interval_secs: interval,
    });
    target.touch();

    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    let interval_msg = interval
        .map(|i| format!(" (interval: {i}s)"))
        .unwrap_or_default();

    println!(
        "{}Enabled{} monitoring for connection '{}'.{}",
        color::green(),
        color::reset(),
        conn_name,
        interval_msg
    );
    Ok(())
}

/// Disable monitoring for a connection
fn cmd_monitor_disable(config_path: Option<&Path>, name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let conn = find_connection(&connections, name)?;
    let conn_id = conn.id;
    let conn_name = conn.name.clone();

    let target = connections
        .iter_mut()
        .find(|c| c.id == conn_id)
        .ok_or_else(|| CliError::ConnectionNotFound(name.to_string()))?;

    target.monitoring_config = Some(MonitoringConfig {
        enabled: Some(false),
        interval_secs: None,
    });
    target.touch();

    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    println!(
        "{}Disabled{} monitoring for connection '{}'.",
        color::yellow(),
        color::reset(),
        conn_name
    );
    Ok(())
}

/// Show monitoring metrics/config for a connection
fn cmd_monitor_metrics(
    config_path: Option<&Path>,
    name: &str,
    format: OutputFormat,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let conn = find_connection(&connections, name)?;

    let monitoring = conn.monitoring_config.as_ref();
    let enabled = monitoring.and_then(|m| m.enabled).unwrap_or(false);
    let interval = monitoring.and_then(|m| m.interval_secs);

    match format {
        OutputFormat::Json => {
            let output = serde_json::json!({
                "connection": conn.name,
                "monitoring_enabled": enabled,
                "interval_secs": interval,
                "config_source": if monitoring.is_some() { "per-connection" } else { "global" },
            });
            let json = serde_json::to_string_pretty(&output)
                .map_err(|e| CliError::Config(format!("JSON serialization failed: {e}")))?;
            println!("{json}");
        }
        OutputFormat::Csv => {
            println!("connection,monitoring_enabled,interval_secs,config_source");
            println!(
                "{},{},{},{}",
                conn.name,
                enabled,
                interval.map(|i| i.to_string()).unwrap_or_default(),
                if monitoring.is_some() {
                    "per-connection"
                } else {
                    "global"
                },
            );
        }
        OutputFormat::Table => {
            println!(
                "{}Monitoring: {}{}",
                color::bold(),
                conn.name,
                color::reset()
            );
            println!("{}", "=".repeat(40));
            println!(
                "Status:          {}",
                if enabled {
                    format!("{}enabled{}", color::green(), color::reset())
                } else {
                    format!("{}disabled{}", color::yellow(), color::reset())
                }
            );
            println!(
                "Interval:        {}",
                interval
                    .map(|i| format!("{i}s"))
                    .unwrap_or_else(|| "global default".to_string())
            );
            println!(
                "Config source:   {}",
                if monitoring.is_some() {
                    "per-connection override"
                } else {
                    "global settings"
                }
            );
        }
    }

    Ok(())
}
