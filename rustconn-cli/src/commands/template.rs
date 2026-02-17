//! Template management commands.

use std::path::Path;

use rustconn_core::models::ConnectionTemplate;

use crate::cli::{OutputFormat, TemplateCommands};
use crate::error::CliError;
use crate::format::escape_csv_field;
use crate::util::create_config_manager;

/// Template command handler
pub fn cmd_template(config_path: Option<&Path>, subcmd: TemplateCommands) -> Result<(), CliError> {
    match subcmd {
        TemplateCommands::List { format, protocol } => {
            cmd_template_list(config_path, format, protocol.as_deref())
        }
        TemplateCommands::Show { name } => cmd_template_show(config_path, &name),
        TemplateCommands::Create {
            name,
            protocol,
            host,
            port,
            user,
            description,
        } => cmd_template_create(
            config_path,
            &name,
            &protocol,
            host.as_deref(),
            port,
            user.as_deref(),
            description.as_deref(),
        ),
        TemplateCommands::Delete { name } => cmd_template_delete(config_path, &name),
        TemplateCommands::Apply {
            template,
            name,
            host,
            port,
            user,
        } => cmd_template_apply(
            config_path,
            &template,
            name.as_deref(),
            host.as_deref(),
            port,
            user.as_deref(),
        ),
    }
}

fn cmd_template_list(
    config_path: Option<&Path>,
    format: OutputFormat,
    protocol: Option<&str>,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let templates = config_manager
        .load_templates()
        .map_err(|e| CliError::Template(format!("Failed to load templates: {e}")))?;

    let filtered: Vec<&ConnectionTemplate> = if let Some(proto) = protocol {
        let proto_lower = proto.to_lowercase();
        templates
            .iter()
            .filter(|t| t.protocol.as_str() == proto_lower)
            .collect()
    } else {
        templates.iter().collect()
    };

    match format {
        OutputFormat::Table => print_template_table(&filtered),
        OutputFormat::Json => print_template_json(&filtered)?,
        OutputFormat::Csv => print_template_csv(&filtered),
    }

    Ok(())
}

fn print_template_table(templates: &[&ConnectionTemplate]) {
    if templates.is_empty() {
        println!("No templates found.");
        return;
    }

    let name_width = templates
        .iter()
        .map(|t| t.name.len())
        .max()
        .unwrap_or(4)
        .max(4);

    println!("{:<name_width$}  PROTOCOL  PORT  HOST", "NAME");
    println!("{:-<name_width$}  {:-<8}  {:-<5}  {:-<20}", "", "", "", "");

    for template in templates {
        let host = if template.host.is_empty() {
            "-"
        } else {
            &template.host
        };
        let host_display = if host.len() > 20 {
            format!("{}...", &host[..17])
        } else {
            host.to_string()
        };
        println!(
            "{:<name_width$}  {:<8}  {:<5}  {host_display}",
            template.name, template.protocol, template.port
        );
    }
}

fn print_template_json(templates: &[&ConnectionTemplate]) -> Result<(), CliError> {
    let json = serde_json::to_string_pretty(templates)
        .map_err(|e| CliError::Template(format!("Failed to serialize: {e}")))?;
    println!("{json}");
    Ok(())
}

fn print_template_csv(templates: &[&ConnectionTemplate]) {
    println!("name,protocol,port,host,username");
    for template in templates {
        let name = escape_csv_field(&template.name);
        let host = &template.host;
        let user = template.username.as_deref().unwrap_or("");
        println!(
            "{name},{},{},{host},{user}",
            template.protocol, template.port
        );
    }
}

fn cmd_template_show(config_path: Option<&Path>, name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let templates = config_manager
        .load_templates()
        .map_err(|e| CliError::Template(format!("Failed to load templates: {e}")))?;

    let template = find_template(&templates, name)?;

    println!("Template Details:");
    println!("  ID:       {}", template.id);
    println!("  Name:     {}", template.name);
    println!("  Protocol: {}", template.protocol);
    println!("  Port:     {}", template.port);

    if !template.host.is_empty() {
        println!("  Host:     {}", template.host);
    }
    if let Some(ref user) = template.username {
        println!("  Username: {user}");
    }
    if let Some(ref desc) = template.description {
        println!("  Description: {desc}");
    }
    if !template.tags.is_empty() {
        println!("  Tags:     {}", template.tags.join(", "));
    }

    Ok(())
}

fn cmd_template_create(
    config_path: Option<&Path>,
    name: &str,
    protocol: &str,
    host: Option<&str>,
    port: Option<u16>,
    user: Option<&str>,
    description: Option<&str>,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let mut templates = config_manager
        .load_templates()
        .map_err(|e| CliError::Template(format!("Failed to load templates: {e}")))?;

    let mut template = match protocol.to_lowercase().as_str() {
        "ssh" => ConnectionTemplate::new_ssh(name.to_string()),
        "rdp" => ConnectionTemplate::new_rdp(name.to_string()),
        "vnc" => ConnectionTemplate::new_vnc(name.to_string()),
        "spice" => ConnectionTemplate::new_spice(name.to_string()),
        _ => {
            return Err(CliError::Template(format!(
                "Unknown protocol '{protocol}'. \
                 Supported: ssh, rdp, vnc, spice"
            )))
        }
    };

    if let Some(h) = host {
        template = template.with_host(h);
    }
    if let Some(p) = port {
        template = template.with_port(p);
    }
    if let Some(u) = user {
        template = template.with_username(u);
    }
    if let Some(d) = description {
        template = template.with_description(d);
    }

    let id = template.id;
    templates.push(template);

    config_manager
        .save_templates(&templates)
        .map_err(|e| CliError::Template(format!("Failed to save templates: {e}")))?;

    println!("Created template '{name}' with ID {id}");

    Ok(())
}

fn cmd_template_delete(config_path: Option<&Path>, name: &str) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let mut templates = config_manager
        .load_templates()
        .map_err(|e| CliError::Template(format!("Failed to load templates: {e}")))?;

    let template = find_template(&templates, name)?;
    let id = template.id;
    let template_name = template.name.clone();

    templates.retain(|t| t.id != id);

    config_manager
        .save_templates(&templates)
        .map_err(|e| CliError::Template(format!("Failed to save templates: {e}")))?;

    println!("Deleted template '{template_name}' (ID: {id})");

    Ok(())
}

fn cmd_template_apply(
    config_path: Option<&Path>,
    template_name: &str,
    conn_name: Option<&str>,
    host: Option<&str>,
    port: Option<u16>,
    user: Option<&str>,
) -> Result<(), CliError> {
    let config_manager = create_config_manager(config_path)?;

    let templates = config_manager
        .load_templates()
        .map_err(|e| CliError::Template(format!("Failed to load templates: {e}")))?;

    let template = find_template(&templates, template_name)?;

    let mut connection = template.apply(conn_name.map(String::from));

    if let Some(h) = host {
        connection.host = h.to_string();
    }
    if let Some(p) = port {
        connection.port = p;
    }
    if let Some(u) = user {
        connection.username = Some(u.to_string());
    }

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let id = connection.id;
    let name = connection.name.clone();
    connections.push(connection);

    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    println!(
        "Created connection '{name}' from template \
         '{template_name}' (ID: {id})"
    );

    Ok(())
}

/// Find a template by name or ID
fn find_template<'a>(
    templates: &'a [ConnectionTemplate],
    name_or_id: &str,
) -> Result<&'a ConnectionTemplate, CliError> {
    if let Ok(uuid) = uuid::Uuid::parse_str(name_or_id) {
        if let Some(template) = templates.iter().find(|t| t.id == uuid) {
            return Ok(template);
        }
    }

    let matches: Vec<_> = templates
        .iter()
        .filter(|t| t.name.eq_ignore_ascii_case(name_or_id))
        .collect();

    match matches.len() {
        0 => Err(CliError::Template(format!(
            "Template not found: {name_or_id}"
        ))),
        1 => Ok(matches[0]),
        _ => Err(CliError::Template(format!(
            "Ambiguous template name: {name_or_id}"
        ))),
    }
}
