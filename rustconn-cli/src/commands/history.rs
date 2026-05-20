//! Connection history commands.

use std::io::{self, IsTerminal, Write};
use std::path::Path;

use crate::cli::{HistoryCommands, OutputFormat};
use crate::color;
use crate::error::CliError;
use crate::format::escape_csv_field;
use crate::util::create_config_manager;

/// History command dispatcher
pub fn cmd_history(config_path: Option<&Path>, subcmd: HistoryCommands) -> Result<(), CliError> {
    match subcmd {
        HistoryCommands::List {
            format,
            limit,
            connection,
        } => cmd_history_list(
            config_path,
            format.effective(),
            limit,
            connection.as_deref(),
        ),
        HistoryCommands::Show { id } => cmd_history_show(config_path, &id),
        HistoryCommands::Clear { force } => cmd_history_clear(config_path, force),
    }
}

/// List recent connection history entries
fn cmd_history_list(
    config_path: Option<&Path>,
    format: OutputFormat,
    limit: usize,
    connection_filter: Option<&str>,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let entries = config_manager
        .load_history()
        .map_err(|e| CliError::Config(format!("Failed to load history: {e}")))?;

    let mut filtered: Vec<_> = if let Some(filter) = connection_filter {
        let lower = filter.to_lowercase();
        entries
            .into_iter()
            .filter(|e| e.connection_name.to_lowercase().contains(&lower))
            .collect()
    } else {
        entries
    };

    // Sort by most recent first
    filtered.sort_by_key(|e| std::cmp::Reverse(e.started_at));
    filtered.truncate(limit);

    if filtered.is_empty() {
        println!("No history entries found.");
        return Ok(());
    }

    match format {
        OutputFormat::Json => {
            let json = serde_json::to_string_pretty(&filtered)
                .map_err(|e| CliError::Config(format!("JSON serialization failed: {e}")))?;
            println!("{json}");
        }
        OutputFormat::Csv => {
            println!(
                "id,connection_name,host,port,protocol,username,started_at,successful,duration_seconds"
            );
            for entry in &filtered {
                println!(
                    "{},{},{},{},{},{},{},{},{}",
                    entry.id,
                    escape_csv_field(&entry.connection_name),
                    escape_csv_field(&entry.host),
                    entry.port,
                    escape_csv_field(&entry.protocol),
                    entry.username.as_deref().unwrap_or(""),
                    entry.started_at.format("%Y-%m-%dT%H:%M:%SZ"),
                    entry.successful,
                    entry.duration_seconds.unwrap_or(0),
                );
            }
        }
        OutputFormat::Table => {
            println!(
                "{}{:<36}  {:<20}  {:<20}  {:<6}  {:<7}  {:<20}{}",
                color::bold(),
                "ID",
                "Connection",
                "Host",
                "Proto",
                "Status",
                "Started",
                color::reset(),
            );
            println!("{}", "-".repeat(115));

            for entry in &filtered {
                let status = if entry.successful {
                    format!("{}OK{}", color::green(), color::reset())
                } else {
                    format!("{}FAIL{}", color::red(), color::reset())
                };
                let name_display = if entry.connection_name.len() > 18 {
                    format!("{}…", &entry.connection_name[..17])
                } else {
                    entry.connection_name.clone()
                };
                let host_display = if entry.host.len() > 18 {
                    format!("{}…", &entry.host[..17])
                } else {
                    entry.host.clone()
                };
                println!(
                    "{:<36}  {:<20}  {:<20}  {:<6}  {:<7}  {}",
                    entry.id,
                    name_display,
                    host_display,
                    entry.protocol,
                    status,
                    entry.started_at.format("%Y-%m-%d %H:%M"),
                );
            }

            println!(
                "\nShowing {} of {} entries.",
                filtered.len(),
                filtered.len()
            );
        }
    }

    Ok(())
}

/// Show details of a specific history entry
fn cmd_history_show(config_path: Option<&Path>, id: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let entries = config_manager
        .load_history()
        .map_err(|e| CliError::Config(format!("Failed to load history: {e}")))?;

    let uuid =
        uuid::Uuid::parse_str(id).map_err(|_| CliError::Config(format!("Invalid UUID: {id}")))?;

    let entry = entries
        .iter()
        .find(|e| e.id == uuid)
        .ok_or_else(|| CliError::Config(format!("History entry not found: {id}")))?;

    println!("{}History Entry{}", color::bold(), color::reset());
    println!("{}", "=".repeat(40));
    println!("ID:              {}", entry.id);
    println!("Connection:      {}", entry.connection_name);
    println!("Connection ID:   {}", entry.connection_id);
    println!("Host:            {}", entry.host);
    println!("Port:            {}", entry.port);
    println!("Protocol:        {}", entry.protocol);
    println!(
        "Username:        {}",
        entry.username.as_deref().unwrap_or("-")
    );
    println!(
        "Started:         {}",
        entry.started_at.format("%Y-%m-%d %H:%M:%S UTC")
    );
    if let Some(ended) = entry.ended_at {
        println!("Ended:           {}", ended.format("%Y-%m-%d %H:%M:%S UTC"));
    }
    println!(
        "Successful:      {}",
        if entry.successful { "Yes" } else { "No" }
    );
    if let Some(duration) = entry.duration_seconds {
        let hours = duration / 3600;
        let minutes = (duration % 3600) / 60;
        let seconds = duration % 60;
        if hours > 0 {
            println!("Duration:        {hours}h {minutes}m {seconds}s");
        } else if minutes > 0 {
            println!("Duration:        {minutes}m {seconds}s");
        } else {
            println!("Duration:        {seconds}s");
        }
    }
    if let Some(ref err) = entry.error_message {
        println!("Error:           {}{}{}", color::red(), err, color::reset());
    }

    Ok(())
}

/// Clear all connection history
fn cmd_history_clear(config_path: Option<&Path>, force: bool) -> Result<(), CliError> {
    if !force {
        if !io::stdin().is_terminal() {
            return Err(CliError::Config(
                "Cannot prompt for confirmation in non-interactive mode. Use --force.".to_string(),
            ));
        }
        print!("Clear all connection history? [y/N] ");
        io::stdout()
            .flush()
            .map_err(|e| CliError::Config(format!("IO error: {e}")))?;

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .map_err(|e| CliError::Config(format!("IO error: {e}")))?;

        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Aborted.");
            return Ok(());
        }
    }

    let config_manager = create_config_manager(config_path)?;
    config_manager
        .save_history(&[])
        .map_err(|e| CliError::Config(format!("Failed to clear history: {e}")))?;

    println!("History cleared.");
    Ok(())
}
