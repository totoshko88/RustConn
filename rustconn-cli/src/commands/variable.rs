//! Variable management commands.

use std::path::Path;

use rustconn_core::variables::Variable;

use crate::cli::{OutputFormat, VariableCommands};
use crate::error::CliError;
use crate::format::escape_csv_field;
use crate::util::create_config_manager;

/// Variable command handler
pub fn cmd_var(config_path: Option<&Path>, subcmd: VariableCommands) -> Result<(), CliError> {
    match subcmd {
        VariableCommands::List { format } => cmd_var_list(config_path, format.effective()),
        VariableCommands::Show { name } => cmd_var_show(config_path, &name),
        VariableCommands::Set {
            name,
            value,
            secret,
            description,
        } => cmd_var_set(config_path, &name, &value, secret, description.as_deref()),
        VariableCommands::Delete { name } => cmd_var_delete(config_path, &name),
    }
}

fn cmd_var_list(config_path: Option<&Path>, format: OutputFormat) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let variables = config_manager
        .load_variables()
        .map_err(|e| CliError::Variable(format!("Failed to load variables: {e}")))?;

    match format {
        OutputFormat::Table => print_var_table(&variables),
        OutputFormat::Json => print_var_json(&variables)?,
        OutputFormat::Csv => print_var_csv(&variables),
    }

    Ok(())
}

fn print_var_table(variables: &[Variable]) {
    if variables.is_empty() {
        println!("No variables found.");
        return;
    }

    let name_width = variables
        .iter()
        .map(|v| v.name.len())
        .max()
        .unwrap_or(4)
        .max(4);

    println!("{:<name_width$}  SECRET  VALUE", "NAME");
    println!("{:-<name_width$}  {:-<6}  {:-<30}", "", "", "");

    for var in variables {
        let secret = if var.is_secret { "Yes" } else { "No" };
        let value = var.display_value();
        let value_display = if value.len() > 30 {
            format!("{}...", &value[..27])
        } else {
            value.to_string()
        };
        println!("{:<name_width$}  {:<6}  {value_display}", var.name, secret);
    }
}

fn print_var_json(variables: &[Variable]) -> Result<(), CliError> {
    let safe_output: Vec<_> = variables
        .iter()
        .map(|v| {
            serde_json::json!({
                "name": v.name,
                "value": v.display_value(),
                "is_secret": v.is_secret,
                "description": v.description
            })
        })
        .collect();

    let json = serde_json::to_string_pretty(&safe_output)
        .map_err(|e| CliError::Variable(format!("Failed to serialize: {e}")))?;
    println!("{json}");
    Ok(())
}

fn print_var_csv(variables: &[Variable]) {
    println!("name,value,is_secret,description");
    for var in variables {
        let name = escape_csv_field(&var.name);
        let value = escape_csv_field(var.display_value());
        let desc = var.description.as_deref().unwrap_or("");
        println!(
            "{name},{value},{},{}",
            var.is_secret,
            escape_csv_field(desc)
        );
    }
}

fn cmd_var_show(config_path: Option<&Path>, name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let variables = config_manager
        .load_variables()
        .map_err(|e| CliError::Variable(format!("Failed to load variables: {e}")))?;

    let var = variables
        .iter()
        .find(|v| v.name == name)
        .ok_or_else(|| CliError::Variable(format!("Variable not found: {name}")))?;

    println!("Variable Details:");
    println!("  Name:   {}", var.name);
    println!("  Value:  {}", var.display_value());
    println!("  Secret: {}", if var.is_secret { "Yes" } else { "No" });

    if let Some(ref desc) = var.description {
        println!("  Description: {desc}");
    }

    Ok(())
}

fn cmd_var_set(
    config_path: Option<&Path>,
    name: &str,
    value: &str,
    secret: bool,
    description: Option<&str>,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let mut variables = config_manager
        .load_variables()
        .map_err(|e| CliError::Variable(format!("Failed to load variables: {e}")))?;

    let existing_idx = variables.iter().position(|v| v.name == name);

    let var = if secret {
        Variable::new_secret(name, value)
    } else {
        Variable::new(name, value)
    };

    let var = if let Some(desc) = description {
        var.with_description(desc)
    } else {
        var
    };

    let action = if let Some(idx) = existing_idx {
        variables[idx] = var;
        "Updated"
    } else {
        variables.push(var);
        "Created"
    };

    config_manager
        .save_variables(&variables)
        .map_err(|e| CliError::Variable(format!("Failed to save variables: {e}")))?;

    println!("{action} variable '{name}'");

    Ok(())
}

fn cmd_var_delete(config_path: Option<&Path>, name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let mut variables = config_manager
        .load_variables()
        .map_err(|e| CliError::Variable(format!("Failed to load variables: {e}")))?;

    let initial_len = variables.len();
    variables.retain(|v| v.name != name);

    if variables.len() == initial_len {
        return Err(CliError::Variable(format!("Variable not found: {name}")));
    }

    config_manager
        .save_variables(&variables)
        .map_err(|e| CliError::Variable(format!("Failed to save variables: {e}")))?;

    println!("Deleted variable '{name}'");

    Ok(())
}
