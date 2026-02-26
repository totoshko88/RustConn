//! Snippet management commands.

use std::collections::HashMap;
use std::path::Path;

use rustconn_core::models::Snippet;
use rustconn_core::snippet::SnippetManager;

use crate::cli::{OutputFormat, SnippetCommands};
use crate::error::CliError;
use crate::format::escape_csv_field;
use crate::util::create_config_manager;

/// Snippet command handler
pub fn cmd_snippet(config_path: Option<&Path>, subcmd: SnippetCommands) -> Result<(), CliError> {
    match subcmd {
        SnippetCommands::List {
            format,
            category,
            tag,
        } => cmd_snippet_list(
            config_path,
            format.effective(),
            category.as_deref(),
            tag.as_deref(),
        ),
        SnippetCommands::Show { name } => cmd_snippet_show(config_path, &name),
        SnippetCommands::Add {
            name,
            command,
            description,
            category,
            tags,
        } => cmd_snippet_add(
            config_path,
            &name,
            &command,
            description.as_deref(),
            category,
            tags,
        ),
        SnippetCommands::Delete { name } => cmd_snippet_delete(config_path, &name),
        SnippetCommands::Run { name, var, execute } => {
            cmd_snippet_run(config_path, &name, &var, execute)
        }
    }
}

fn cmd_snippet_list(
    config_path: Option<&Path>,
    format: OutputFormat,
    category: Option<&str>,
    tag: Option<&str>,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let snippet_manager = SnippetManager::new(config_manager)
        .map_err(|e| CliError::Snippet(format!("Failed to load snippets: {e}")))?;

    let snippets: Vec<&Snippet> = match (category, tag) {
        (Some(cat), _) => snippet_manager.get_by_category(cat),
        (None, Some(t)) => snippet_manager.filter_by_tag(t),
        (None, None) => snippet_manager.list_snippets(),
    };

    match format {
        OutputFormat::Table => print_snippet_table(&snippets),
        OutputFormat::Json => print_snippet_json(&snippets)?,
        OutputFormat::Csv => print_snippet_csv(&snippets),
    }

    Ok(())
}

fn print_snippet_table(snippets: &[&Snippet]) {
    if snippets.is_empty() {
        println!("No snippets found.");
        return;
    }

    let name_width = snippets
        .iter()
        .map(|s| s.name.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let cat_width = snippets
        .iter()
        .filter_map(|s| s.category.as_ref())
        .map(String::len)
        .max()
        .unwrap_or(8)
        .max(8);

    println!(
        "{:<name_width$}  {:<cat_width$}  COMMAND",
        "NAME", "CATEGORY"
    );
    println!("{:-<name_width$}  {:-<cat_width$}  {:-<40}", "", "", "");

    for snippet in snippets {
        let category = snippet.category.as_deref().unwrap_or("-");
        let command = if snippet.command.len() > 40 {
            format!("{}...", &snippet.command[..37])
        } else {
            snippet.command.clone()
        };
        println!(
            "{:<name_width$}  {:<cat_width$}  {command}",
            snippet.name, category
        );
    }
}

fn print_snippet_json(snippets: &[&Snippet]) -> Result<(), CliError> {
    let json = serde_json::to_string_pretty(snippets)
        .map_err(|e| CliError::Snippet(format!("Failed to serialize: {e}")))?;
    println!("{json}");
    Ok(())
}

fn print_snippet_csv(snippets: &[&Snippet]) {
    println!("name,category,command,tags");
    for snippet in snippets {
        let name = escape_csv_field(&snippet.name);
        let category = snippet.category.as_deref().unwrap_or("");
        let command = escape_csv_field(&snippet.command);
        let tags = snippet.tags.join(";");
        println!("{name},{category},{command},{tags}");
    }
}

fn cmd_snippet_show(config_path: Option<&Path>, name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let snippet_manager = SnippetManager::new(config_manager)
        .map_err(|e| CliError::Snippet(format!("Failed to load snippets: {e}")))?;

    let snippet = find_snippet(&snippet_manager, name)?;

    println!("Snippet Details:");
    println!("  ID:       {}", snippet.id);
    println!("  Name:     {}", snippet.name);
    println!("  Command:  {}", snippet.command);

    if let Some(ref desc) = snippet.description {
        println!("  Description: {desc}");
    }
    if let Some(ref cat) = snippet.category {
        println!("  Category: {cat}");
    }
    if !snippet.tags.is_empty() {
        println!("  Tags:     {}", snippet.tags.join(", "));
    }

    println!(
        "  Created:  {}",
        snippet.created_at.format("%Y-%m-%d %H:%M")
    );
    println!(
        "  Updated:  {}",
        snippet.updated_at.format("%Y-%m-%d %H:%M")
    );

    let variables = SnippetManager::extract_variables(&snippet.command);
    if !variables.is_empty() {
        println!("\nVariables:");
        for var in &variables {
            let default = snippet
                .variables
                .iter()
                .find(|v| &v.name == var)
                .and_then(|v| v.default_value.as_ref());
            if let Some(def) = default {
                println!("  ${{{var}}} (default: {def})");
            } else {
                println!("  ${{{var}}}");
            }
        }
    }

    Ok(())
}

fn cmd_snippet_add(
    config_path: Option<&Path>,
    name: &str,
    command: &str,
    description: Option<&str>,
    category: Option<String>,
    tags: Option<String>,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let mut snippet_manager = SnippetManager::new(config_manager)
        .map_err(|e| CliError::Snippet(format!("Failed to load snippets: {e}")))?;

    let mut snippet = Snippet::new(name.to_string(), command.to_string());

    if let Some(desc) = description {
        snippet.description = Some(desc.to_string());
    }
    if let Some(cat) = category {
        snippet = snippet.with_category(&cat);
    }
    if let Some(tags_str) = tags {
        let tag_vec: Vec<String> = tags_str.split(',').map(|s| s.trim().to_string()).collect();
        snippet = snippet.with_tags(tag_vec);
    }

    let variables = SnippetManager::extract_variable_objects(command);
    snippet = snippet.with_variables(variables);

    let id = snippet_manager
        .create_snippet_from(snippet)
        .map_err(|e| CliError::Snippet(format!("Failed to create snippet: {e}")))?;

    println!("Created snippet '{name}' with ID {id}");

    let vars = SnippetManager::extract_variables(command);
    if !vars.is_empty() {
        println!("Variables: {}", vars.join(", "));
    }

    Ok(())
}

fn cmd_snippet_delete(config_path: Option<&Path>, name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let mut snippet_manager = SnippetManager::new(config_manager)
        .map_err(|e| CliError::Snippet(format!("Failed to load snippets: {e}")))?;

    let snippet = find_snippet(&snippet_manager, name)?;
    let id = snippet.id;
    let snippet_name = snippet.name.clone();

    snippet_manager
        .delete_snippet(id)
        .map_err(|e| CliError::Snippet(format!("Failed to delete snippet: {e}")))?;

    println!("Deleted snippet '{snippet_name}' (ID: {id})");

    Ok(())
}

fn cmd_snippet_run(
    config_path: Option<&Path>,
    name: &str,
    vars: &[(String, String)],
    execute: bool,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let snippet_manager = SnippetManager::new(config_manager)
        .map_err(|e| CliError::Snippet(format!("Failed to load snippets: {e}")))?;

    let snippet = find_snippet(&snippet_manager, name)?;

    let values: HashMap<String, String> = vars.iter().cloned().collect();

    let missing = SnippetManager::get_missing_variables(snippet, &values);
    if !missing.is_empty() {
        return Err(CliError::Snippet(format!(
            "Missing required variables: {}. \
             Use --var name=value to provide them.",
            missing.join(", ")
        )));
    }

    let command = SnippetManager::substitute_with_defaults(snippet, &values);

    if execute {
        println!("Executing: {command}");
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(&command)
            .status()
            .map_err(|e| CliError::Snippet(format!("Failed to execute command: {e}")))?;

        if !status.success() {
            return Err(CliError::Snippet(format!(
                "Command exited with status: {}",
                status.code().unwrap_or(-1)
            )));
        }
    } else {
        println!("{command}");
    }

    Ok(())
}

/// Find a snippet by name or ID
fn find_snippet<'a>(
    manager: &'a SnippetManager,
    name_or_id: &str,
) -> Result<&'a Snippet, CliError> {
    if let Ok(uuid) = uuid::Uuid::parse_str(name_or_id)
        && let Some(snippet) = manager.get_snippet(uuid)
    {
        return Ok(snippet);
    }

    let snippets = manager.list_snippets();
    let matches: Vec<_> = snippets
        .iter()
        .filter(|s| s.name.eq_ignore_ascii_case(name_or_id))
        .collect();

    match matches.len() {
        0 => Err(CliError::Snippet(format!(
            "Snippet not found: {name_or_id}"
        ))),
        1 => Ok(matches[0]),
        _ => Err(CliError::Snippet(format!(
            "Ambiguous snippet name: {name_or_id}"
        ))),
    }
}
