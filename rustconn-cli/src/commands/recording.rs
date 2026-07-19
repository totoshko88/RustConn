//! Session recording management commands.

use std::io::{self, BufRead, Write};
use std::path::Path;

use rustconn_core::session::recording::{RecordingEntry, RecordingManager, default_recordings_dir};

use crate::cli::{OutputFormat, RecordingCommands};
use crate::error::CliError;
use crate::format::escape_csv_field;

/// Recording command handler.
///
/// # Errors
///
/// Returns:
/// - [`CliError::Recording`] when listing, deleting, or importing recordings fails
/// - [`CliError::Io`] when the recordings directory cannot be created or read
pub(super) fn cmd_recording(subcmd: RecordingCommands) -> Result<(), CliError> {
    match subcmd {
        RecordingCommands::List { format } => cmd_recording_list(format.effective()),
        RecordingCommands::Delete { name, force } => cmd_recording_delete(&name, force),
        RecordingCommands::Import {
            data_file,
            timing_file,
        } => cmd_recording_import(&data_file, &timing_file),
    }
}

/// Returns a `RecordingManager` for the default recordings directory.
fn recordings_manager() -> Result<RecordingManager, CliError> {
    let dir = default_recordings_dir()
        .ok_or_else(|| CliError::Recording("Cannot determine recordings directory".into()))?;
    Ok(RecordingManager::new(dir))
}

// ── List ──────────────────────────────────────────────────────────────

fn cmd_recording_list(format: OutputFormat) -> Result<(), CliError> {
    let manager = recordings_manager()?;
    let entries = manager
        .list()
        .map_err(|e| CliError::Recording(format!("Failed to list recordings: {e}")))?;

    match format {
        OutputFormat::Table => print_table(&entries),
        OutputFormat::Json => print_json(&entries)?,
        OutputFormat::Csv => print_csv(&entries),
    }

    Ok(())
}

fn display_name(entry: &RecordingEntry) -> &str {
    entry
        .metadata
        .display_name
        .as_deref()
        .unwrap_or(&entry.metadata.connection_name)
}

fn format_duration(secs: f64) -> String {
    let total = secs as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}h {m:02}m {s:02}s")
    } else {
        format!("{m}m {s:02}s")
    }
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn print_table(entries: &[RecordingEntry]) {
    if entries.is_empty() {
        println!("No recordings found.");
        return;
    }

    let name_width = entries
        .iter()
        .map(|e| display_name(e).len())
        .max()
        .unwrap_or(4)
        .max(4);

    println!(
        "{:<name_width$}  {:<19}  {:>12}  {:>8}",
        "NAME", "DATE", "DURATION", "SIZE"
    );
    println!("{:-<name_width$}  {:-<19}  {:->12}  {:->8}", "", "", "", "");

    for entry in entries {
        let name = display_name(entry);
        let date = entry.metadata.created_at.format("%Y-%m-%d %H:%M:%S");
        let dur = format_duration(entry.metadata.duration_secs);
        let size = format_size(entry.metadata.total_size_bytes);
        println!("{name:<name_width$}  {date}  {dur:>12}  {size:>8}");
    }
}

fn print_json(entries: &[RecordingEntry]) -> Result<(), CliError> {
    let items: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "name": display_name(e),
                "connection_name": e.metadata.connection_name,
                "display_name": e.metadata.display_name,
                "created_at": e.metadata.created_at.to_rfc3339(),
                "duration_secs": e.metadata.duration_secs,
                "total_size_bytes": e.metadata.total_size_bytes,
                "data_path": e.data_path.display().to_string(),
            })
        })
        .collect();

    let json = serde_json::to_string_pretty(&items)
        .map_err(|e| CliError::Recording(format!("Failed to serialize: {e}")))?;
    println!("{json}");
    Ok(())
}

fn print_csv(entries: &[RecordingEntry]) {
    println!("name,connection_name,date,duration_secs,size_bytes");
    for entry in entries {
        let name = escape_csv_field(display_name(entry));
        let conn = escape_csv_field(&entry.metadata.connection_name);
        let date = entry.metadata.created_at.format("%Y-%m-%d %H:%M:%S");
        println!(
            "{name},{conn},{date},{:.1},{}",
            entry.metadata.duration_secs, entry.metadata.total_size_bytes
        );
    }
}

// ── Delete ────────────────────────────────────────────────────────────

fn cmd_recording_delete(name: &str, force: bool) -> Result<(), CliError> {
    let manager = recordings_manager()?;
    let entries = manager
        .list()
        .map_err(|e| CliError::Recording(format!("Failed to list recordings: {e}")))?;

    let entry = find_recording(&entries, name)?;

    if !force {
        eprint!("Delete recording '{}'? [y/N] ", display_name(entry));
        io::stderr().flush().ok();

        let mut answer = String::new();
        io::stdin()
            .lock()
            .read_line(&mut answer)
            .map_err(|e| CliError::Recording(format!("Failed to read input: {e}")))?;

        if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes") {
            println!("Aborted.");
            return Ok(());
        }
    }

    manager
        .delete(&entry.data_path)
        .map_err(|e| CliError::Recording(format!("Failed to delete recording: {e}")))?;

    println!("Deleted recording '{}'", display_name(entry));
    Ok(())
}

/// Find a recording by display_name first, then connection_name.
fn find_recording<'a>(
    entries: &'a [RecordingEntry],
    name: &str,
) -> Result<&'a RecordingEntry, CliError> {
    // Search by display_name first.
    if let Some(entry) = entries.iter().find(|e| {
        e.metadata
            .display_name
            .as_deref()
            .is_some_and(|dn| dn.eq_ignore_ascii_case(name))
    }) {
        return Ok(entry);
    }

    // Then by connection_name.
    if let Some(entry) = entries
        .iter()
        .find(|e| e.metadata.connection_name.eq_ignore_ascii_case(name))
    {
        return Ok(entry);
    }

    Err(CliError::Recording(format!("Recording not found: {name}")))
}

// ── Import ────────────────────────────────────────────────────────────

fn cmd_recording_import(data_file: &Path, timing_file: &Path) -> Result<(), CliError> {
    let manager = recordings_manager()?;

    let entry = manager
        .import(data_file, timing_file)
        .map_err(|e| CliError::Recording(format!("Failed to import recording: {e}")))?;

    println!(
        "Imported recording '{}' ({}).",
        display_name(&entry),
        format_size(entry.metadata.total_size_bytes)
    );
    Ok(())
}
