//! `RustConn` CLI - Command-line interface for `RustConn` connection manager
//!
//! Provides commands for listing, adding, exporting, importing, testing connections,
//! managing snippets, groups, templates, clusters, variables, and Wake-on-LAN functionality.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use rustconn_core::cluster::Cluster;
use rustconn_core::config::ConfigManager;
use rustconn_core::models::{
    Connection, ConnectionGroup, ConnectionTemplate, ProtocolType, Snippet,
};
use rustconn_core::snippet::SnippetManager;
use rustconn_core::variables::Variable;
use rustconn_core::wol::{MacAddress, WolConfig};

/// `RustConn` command-line interface for managing remote connections
#[derive(Parser)]
#[command(name = "rustconn-cli")]
#[command(author, version, about = "RustConn command-line interface")]
#[command(propagate_version = true)]
pub struct Cli {
    /// Path to the configuration file
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

/// Available CLI commands
#[derive(Subcommand)]
pub enum Commands {
    /// List all connections
    #[command(about = "List all connections in the configuration")]
    List {
        /// Output format for the connection list
        #[arg(short, long, default_value = "table", value_enum)]
        format: OutputFormat,

        /// Filter connections by protocol (ssh, rdp, vnc, spice)
        #[arg(short, long)]
        protocol: Option<String>,

        /// Filter connections by group name
        #[arg(short, long)]
        group: Option<String>,

        /// Filter connections by tag
        #[arg(short, long)]
        tag: Option<String>,
    },

    /// Connect to a server by name or ID
    #[command(about = "Initiate a connection to a remote server")]
    Connect {
        /// Connection name or UUID
        name: String,
    },

    /// Add a new connection
    #[command(about = "Add a new connection to the configuration")]
    Add {
        /// Name for the new connection
        #[arg(short, long)]
        name: String,

        /// Host address (hostname or IP)
        #[arg(short = 'H', long)]
        host: String,

        /// Port number (defaults to protocol default: SSH=22, RDP=3389, VNC=5900)
        #[arg(short, long)]
        port: Option<u16>,

        /// Protocol type (ssh, rdp, vnc)
        #[arg(short = 'P', long, default_value = "ssh")]
        protocol: String,

        /// Username for authentication
        #[arg(short, long)]
        user: Option<String>,

        /// Path to SSH private key file
        #[arg(short, long)]
        key: Option<PathBuf>,
    },

    /// Export connections to external format
    #[command(about = "Export connections to various formats")]
    Export {
        /// Export format
        #[arg(short, long, value_enum)]
        format: ExportFormatArg,

        /// Output file or directory path
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Import connections from external format
    #[command(about = "Import connections from various formats")]
    Import {
        /// Import format
        #[arg(short, long, value_enum)]
        format: ImportFormatArg,

        /// Input file path
        file: PathBuf,
    },

    /// Test connection connectivity
    #[command(about = "Test connectivity to a connection")]
    Test {
        /// Connection name or ID (use "all" to test all connections)
        name: String,

        /// Connection timeout in seconds
        #[arg(short, long, default_value = "10")]
        timeout: u64,
    },

    /// Delete a connection
    #[command(about = "Delete a connection")]
    Delete {
        /// Connection name or UUID
        name: String,
    },

    /// Show connection details
    #[command(about = "Show connection details")]
    Show {
        /// Connection name or UUID
        name: String,
    },

    /// Update a connection
    #[command(about = "Update an existing connection")]
    Update {
        /// Connection name or UUID
        name: String,

        /// New name
        #[arg(short, long)]
        new_name: Option<String>,

        /// New host
        #[arg(short = 'H', long)]
        host: Option<String>,

        /// New port
        #[arg(short, long)]
        port: Option<u16>,

        /// New username
        #[arg(short, long)]
        user: Option<String>,
    },

    /// Send Wake-on-LAN magic packet
    #[command(about = "Wake a sleeping machine using Wake-on-LAN")]
    Wol {
        /// Connection name or MAC address (format: AA:BB:CC:DD:EE:FF or AA-BB-CC-DD-EE-FF)
        target: String,

        /// Broadcast address (default: 255.255.255.255)
        #[arg(short, long, default_value = "255.255.255.255")]
        broadcast: String,

        /// UDP port (default: 9)
        #[arg(short, long, default_value = "9")]
        port: u16,
    },

    /// Manage command snippets
    #[command(subcommand, about = "Manage command snippets")]
    Snippet(SnippetCommands),

    /// Manage connection groups
    #[command(subcommand, about = "Manage connection groups")]
    Group(GroupCommands),

    /// Manage connection templates
    #[command(subcommand, about = "Manage connection templates")]
    Template(TemplateCommands),

    /// Manage connection clusters
    #[command(subcommand, about = "Manage connection clusters")]
    Cluster(ClusterCommands),

    /// Manage global variables
    #[command(subcommand, about = "Manage global variables")]
    Var(VariableCommands),

    /// Duplicate a connection
    #[command(about = "Duplicate an existing connection")]
    Duplicate {
        /// Connection name or UUID to duplicate
        name: String,

        /// New name for the duplicated connection
        #[arg(short, long)]
        new_name: Option<String>,
    },

    /// Show connection statistics
    #[command(about = "Show connection statistics")]
    Stats,
}

/// Output format for the list command
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum OutputFormat {
    /// Display as formatted table
    Table,
    /// Output as JSON
    Json,
    /// Output as CSV
    Csv,
}

/// Export format options
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum ExportFormatArg {
    /// Ansible inventory format (INI or YAML)
    Ansible,
    /// OpenSSH config format
    SshConfig,
    /// Remmina connection files
    Remmina,
    /// Asbru-CM YAML format
    Asbru,
    /// Native `RustConn` format (.rcn)
    Native,
    /// Royal TS XML format (.rtsz)
    RoyalTs,
}

/// Import format options
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum ImportFormatArg {
    /// Ansible inventory format
    Ansible,
    /// OpenSSH config format
    SshConfig,
    /// Remmina connection files
    Remmina,
    /// Asbru-CM YAML format
    Asbru,
    /// Native `RustConn` format (.rcn)
    Native,
    /// Royal TS XML format (.rtsz)
    RoyalTs,
}

/// Snippet subcommands
#[derive(Subcommand)]
pub enum SnippetCommands {
    /// List all snippets
    #[command(about = "List all command snippets")]
    List {
        /// Output format
        #[arg(short, long, default_value = "table", value_enum)]
        format: OutputFormat,

        /// Filter by category
        #[arg(short, long)]
        category: Option<String>,

        /// Filter by tag
        #[arg(short, long)]
        tag: Option<String>,
    },

    /// Show snippet details
    #[command(about = "Show snippet details and variables")]
    Show {
        /// Snippet name or ID
        name: String,
    },

    /// Add a new snippet
    #[command(about = "Add a new command snippet")]
    Add {
        /// Snippet name
        #[arg(short, long)]
        name: String,

        /// Command template (use ${var} for variables)
        #[arg(short, long)]
        command: String,

        /// Description
        #[arg(short, long)]
        description: Option<String>,

        /// Category
        #[arg(long)]
        category: Option<String>,

        /// Tags (comma-separated)
        #[arg(short, long)]
        tags: Option<String>,
    },

    /// Delete a snippet
    #[command(about = "Delete a command snippet")]
    Delete {
        /// Snippet name or ID
        name: String,
    },

    /// Execute a snippet with variable substitution
    #[command(about = "Show snippet command with variable substitution")]
    Run {
        /// Snippet name or ID
        name: String,

        /// Variable values (format: var=value, can be repeated)
        #[arg(short, long, value_parser = parse_key_val)]
        var: Vec<(String, String)>,

        /// Actually execute the command (default: just print)
        #[arg(short, long)]
        execute: bool,
    },
}

/// Group subcommands
#[derive(Subcommand)]
pub enum GroupCommands {
    /// List all groups
    #[command(about = "List all connection groups")]
    List {
        /// Output format
        #[arg(short, long, default_value = "table", value_enum)]
        format: OutputFormat,
    },

    /// Show group details
    #[command(about = "Show group details and connections")]
    Show {
        /// Group name or ID
        name: String,
    },

    /// Create a new group
    #[command(about = "Create a new connection group")]
    Create {
        /// Group name
        #[arg(short, long)]
        name: String,

        /// Parent group name or ID
        #[arg(short, long)]
        parent: Option<String>,

        /// Description
        #[arg(short, long)]
        description: Option<String>,
    },

    /// Delete a group
    #[command(about = "Delete a connection group")]
    Delete {
        /// Group name or ID
        name: String,
    },

    /// Add a connection to a group
    #[command(about = "Add a connection to a group")]
    AddConnection {
        /// Group name or ID
        #[arg(short, long)]
        group: String,

        /// Connection name or ID
        #[arg(short, long)]
        connection: String,
    },

    /// Remove a connection from a group
    #[command(about = "Remove a connection from a group")]
    RemoveConnection {
        /// Group name or ID
        #[arg(short, long)]
        group: String,

        /// Connection name or ID
        #[arg(short, long)]
        connection: String,
    },
}

/// Template subcommands
#[derive(Subcommand)]
pub enum TemplateCommands {
    /// List all templates
    #[command(about = "List all connection templates")]
    List {
        /// Output format
        #[arg(short, long, default_value = "table", value_enum)]
        format: OutputFormat,

        /// Filter by protocol (ssh, rdp, vnc, spice)
        #[arg(short, long)]
        protocol: Option<String>,
    },

    /// Show template details
    #[command(about = "Show template details")]
    Show {
        /// Template name or ID
        name: String,
    },

    /// Create a new template
    #[command(about = "Create a new connection template")]
    Create {
        /// Template name
        #[arg(short, long)]
        name: String,

        /// Protocol type (ssh, rdp, vnc, spice)
        #[arg(short = 'P', long, default_value = "ssh")]
        protocol: String,

        /// Default host
        #[arg(short = 'H', long)]
        host: Option<String>,

        /// Default port
        #[arg(short, long)]
        port: Option<u16>,

        /// Default username
        #[arg(short, long)]
        user: Option<String>,

        /// Description
        #[arg(short, long)]
        description: Option<String>,
    },

    /// Delete a template
    #[command(about = "Delete a connection template")]
    Delete {
        /// Template name or ID
        name: String,
    },

    /// Create a connection from a template
    #[command(about = "Create a new connection from a template")]
    Apply {
        /// Template name or ID
        template: String,

        /// Name for the new connection
        #[arg(short, long)]
        name: Option<String>,

        /// Override host
        #[arg(short = 'H', long)]
        host: Option<String>,

        /// Override port
        #[arg(short, long)]
        port: Option<u16>,

        /// Override username
        #[arg(short, long)]
        user: Option<String>,
    },
}

/// Cluster subcommands
#[derive(Subcommand)]
pub enum ClusterCommands {
    /// List all clusters
    #[command(about = "List all connection clusters")]
    List {
        /// Output format
        #[arg(short, long, default_value = "table", value_enum)]
        format: OutputFormat,
    },

    /// Show cluster details
    #[command(about = "Show cluster details and connections")]
    Show {
        /// Cluster name or ID
        name: String,
    },

    /// Create a new cluster
    #[command(about = "Create a new connection cluster")]
    Create {
        /// Cluster name
        #[arg(short, long)]
        name: String,

        /// Connection names or IDs to include (comma-separated)
        #[arg(short, long)]
        connections: Option<String>,

        /// Enable broadcast mode by default
        #[arg(short, long)]
        broadcast: bool,
    },

    /// Delete a cluster
    #[command(about = "Delete a connection cluster")]
    Delete {
        /// Cluster name or ID
        name: String,
    },

    /// Add a connection to a cluster
    #[command(about = "Add a connection to a cluster")]
    AddConnection {
        /// Cluster name or ID
        #[arg(short = 'C', long)]
        cluster: String,

        /// Connection name or ID
        #[arg(short, long)]
        connection: String,
    },

    /// Remove a connection from a cluster
    #[command(about = "Remove a connection from a cluster")]
    RemoveConnection {
        /// Cluster name or ID
        #[arg(short = 'C', long)]
        cluster: String,

        /// Connection name or ID
        #[arg(short, long)]
        connection: String,
    },
}

/// Variable subcommands
#[derive(Subcommand)]
pub enum VariableCommands {
    /// List all global variables
    #[command(about = "List all global variables")]
    List {
        /// Output format
        #[arg(short, long, default_value = "table", value_enum)]
        format: OutputFormat,
    },

    /// Show variable details
    #[command(about = "Show variable value")]
    Show {
        /// Variable name
        name: String,
    },

    /// Set a global variable
    #[command(about = "Set a global variable value")]
    Set {
        /// Variable name
        name: String,

        /// Variable value
        value: String,

        /// Mark as secret (value will be masked in output)
        #[arg(short, long)]
        secret: bool,

        /// Description
        #[arg(short, long)]
        description: Option<String>,
    },

    /// Delete a global variable
    #[command(about = "Delete a global variable")]
    Delete {
        /// Variable name
        name: String,
    },
}

/// Parse a key=value pair for variable substitution
fn parse_key_val(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=value: no `=` found in `{s}`"))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::List {
            format,
            protocol,
            group,
            tag,
        } => cmd_list(
            format,
            protocol.as_deref(),
            group.as_deref(),
            tag.as_deref(),
        ),
        Commands::Connect { name } => cmd_connect(&name),
        Commands::Add {
            name,
            host,
            port,
            protocol,
            user,
            key,
        } => cmd_add(
            &name,
            &host,
            port,
            &protocol,
            user.as_deref(),
            key.as_deref(),
        ),
        Commands::Export { format, output } => cmd_export(format, &output),
        Commands::Import { format, file } => cmd_import(format, &file),
        Commands::Test { name, timeout } => cmd_test(&name, timeout),
        Commands::Delete { name } => cmd_delete(&name),
        Commands::Show { name } => cmd_show(&name),
        Commands::Update {
            name,
            new_name,
            host,
            port,
            user,
        } => cmd_update(
            &name,
            new_name.as_deref(),
            host.as_deref(),
            port,
            user.as_deref(),
        ),
        Commands::Wol {
            target,
            broadcast,
            port,
        } => cmd_wol(&target, &broadcast, port),
        Commands::Snippet(subcmd) => cmd_snippet(subcmd),
        Commands::Group(subcmd) => cmd_group(subcmd),
        Commands::Template(subcmd) => cmd_template(subcmd),
        Commands::Cluster(subcmd) => cmd_cluster(subcmd),
        Commands::Var(subcmd) => cmd_var(subcmd),
        Commands::Duplicate { name, new_name } => cmd_duplicate(&name, new_name.as_deref()),
        Commands::Stats => cmd_stats(),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(e.exit_code());
    }
}

/// List connections command handler
fn cmd_list(
    format: OutputFormat,
    protocol: Option<&str>,
    group: Option<&str>,
    tag: Option<&str>,
) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

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
    let protocol_width = 8; // "PROTOCOL" or "SSH/RDP/VNC/SPICE"
    let port_width = 5; // "PORT" or max 5 digits

    // Print header
    let _ = writeln!(
        output,
        "{:<name_width$}  {:<host_width$}  {:<port_width$}  {:<protocol_width$}",
        "NAME", "HOST", "PORT", "PROTOCOL"
    );
    let _ = writeln!(
        output,
        "{:-<name_width$}  {:-<host_width$}  {:-<port_width$}  {:-<protocol_width$}",
        "", "", "", ""
    );

    // Print rows
    for conn in connections {
        let _ = writeln!(
            output,
            "{:<name_width$}  {:<host_width$}  {:<port_width$}  {:<protocol_width$}",
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
        // Escape fields that might contain commas or quotes
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

/// Escape a CSV field if it contains special characters
fn escape_csv_field(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
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
}

impl From<&Connection> for ConnectionOutput {
    fn from(conn: &Connection) -> Self {
        Self {
            id: conn.id.to_string(),
            name: conn.name.clone(),
            host: conn.host.clone(),
            port: conn.port,
            protocol: conn.protocol.as_str().to_string(),
            username: conn.username.clone(),
        }
    }
}

/// Connect command handler
fn cmd_connect(name: &str) -> Result<(), CliError> {
    // Load connections
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    if connections.is_empty() {
        return Err(CliError::ConnectionNotFound(
            "No connections configured".to_string(),
        ));
    }

    // Find the connection
    let connection = find_connection(&connections, name)?;

    println!(
        "Connecting to '{}' ({} {}:{})...",
        connection.name, connection.protocol, connection.host, connection.port
    );

    // Build and execute the connection command
    let command = build_connection_command(connection);
    execute_connection_command(&command)
}

/// Builds the command arguments for a connection based on its protocol
fn build_connection_command(connection: &Connection) -> ConnectionCommand {
    match connection.protocol {
        ProtocolType::Ssh => build_ssh_command(connection),
        ProtocolType::Rdp => build_rdp_command(connection),
        ProtocolType::Vnc => build_vnc_command(connection),
        ProtocolType::Spice => build_spice_command(connection),
        ProtocolType::ZeroTrust => build_zerotrust_command(connection),
    }
}

/// Command to execute for a connection
struct ConnectionCommand {
    /// The program to execute
    program: String,
    /// Command-line arguments
    args: Vec<String>,
}

/// Builds SSH command arguments
fn build_ssh_command(connection: &Connection) -> ConnectionCommand {
    let mut args = Vec::new();

    // Add port if not default
    if connection.port != 22 {
        args.push("-p".to_string());
        args.push(connection.port.to_string());
    }

    // Get SSH-specific config
    if let rustconn_core::models::ProtocolConfig::Ssh(ref ssh_config) = connection.protocol_config {
        // Add identity file if specified
        if let Some(ref key_path) = ssh_config.key_path {
            args.push("-i".to_string());
            args.push(key_path.display().to_string());
        }

        // Add proxy jump if specified
        if let Some(ref proxy_jump) = ssh_config.proxy_jump {
            args.push("-J".to_string());
            args.push(proxy_jump.clone());
        }

        // Add control master options if enabled
        if ssh_config.use_control_master {
            args.push("-o".to_string());
            args.push("ControlMaster=auto".to_string());
            args.push("-o".to_string());
            args.push("ControlPersist=10m".to_string());
        }

        // Add agent forwarding if enabled
        if ssh_config.agent_forwarding {
            args.push("-A".to_string());
        }

        // Add custom options
        for (key, value) in &ssh_config.custom_options {
            args.push("-o".to_string());
            args.push(format!("{key}={value}"));
        }
    }

    // Build the destination (user@host or just host)
    let destination = connection.username.as_ref().map_or_else(
        || connection.host.clone(),
        |u| format!("{u}@{}", connection.host),
    );
    args.push(destination);

    // Add startup command if specified
    if let rustconn_core::models::ProtocolConfig::Ssh(ref ssh_config) = connection.protocol_config {
        if let Some(ref startup_cmd) = ssh_config.startup_command {
            args.push(startup_cmd.clone());
        }
    }

    ConnectionCommand {
        program: "ssh".to_string(),
        args,
    }
}

/// Builds RDP command arguments (using xfreerdp)
fn build_rdp_command(connection: &Connection) -> ConnectionCommand {
    let mut args = Vec::new();

    // Server address with port
    args.push(format!("/v:{}:{}", connection.host, connection.port));

    // Username
    if let Some(ref username) = connection.username {
        args.push(format!("/u:{username}"));
    }

    // Domain
    if let Some(ref domain) = connection.domain {
        args.push(format!("/d:{domain}"));
    }

    // Get RDP-specific config
    if let rustconn_core::models::ProtocolConfig::Rdp(ref rdp_config) = connection.protocol_config {
        // Resolution
        if let Some(ref resolution) = rdp_config.resolution {
            args.push(format!("/w:{}", resolution.width));
            args.push(format!("/h:{}", resolution.height));
        }

        // Color depth
        if let Some(depth) = rdp_config.color_depth {
            args.push(format!("/bpp:{depth}"));
        }

        // Audio redirection
        if rdp_config.audio_redirect {
            args.push("/sound".to_string());
        }

        // Gateway
        if let Some(ref gateway) = rdp_config.gateway {
            args.push(format!("/g:{}:{}", gateway.hostname, gateway.port));
            if let Some(ref gw_user) = gateway.username {
                args.push(format!("/gu:{gw_user}"));
            }
        }

        // Shared folders
        for folder in &rdp_config.shared_folders {
            args.push(format!(
                "/drive:{},{}",
                folder.share_name,
                folder.local_path.display()
            ));
        }

        // Custom arguments
        args.extend(rdp_config.custom_args.clone());
    }

    ConnectionCommand {
        program: "xfreerdp".to_string(),
        args,
    }
}

/// Builds VNC command arguments (using vncviewer)
fn build_vnc_command(connection: &Connection) -> ConnectionCommand {
    let mut args = Vec::new();

    // Get VNC-specific config
    if let rustconn_core::models::ProtocolConfig::Vnc(ref vnc_config) = connection.protocol_config {
        // Encoding
        if let Some(ref encoding) = vnc_config.encoding {
            args.push("-encoding".to_string());
            args.push(encoding.clone());
        }

        // Compression level
        if let Some(compression) = vnc_config.compression {
            args.push("-compresslevel".to_string());
            args.push(compression.to_string());
        }

        // Quality level
        if let Some(quality) = vnc_config.quality {
            args.push("-quality".to_string());
            args.push(quality.to_string());
        }

        // Custom arguments
        args.extend(vnc_config.custom_args.clone());
    }

    // Server address with port (VNC uses display number format)
    // Port 5900 = display :0, 5901 = display :1, etc.
    let display = if connection.port >= 5900 {
        connection.port - 5900
    } else {
        connection.port
    };
    args.push(format!("{}:{display}", connection.host));

    ConnectionCommand {
        program: "vncviewer".to_string(),
        args,
    }
}

/// Builds SPICE command arguments (using remote-viewer)
fn build_spice_command(connection: &Connection) -> ConnectionCommand {
    let mut args = Vec::new();

    // Build SPICE URI
    let scheme = if let rustconn_core::models::ProtocolConfig::Spice(ref spice_config) =
        connection.protocol_config
    {
        if spice_config.tls_enabled {
            "spice+tls"
        } else {
            "spice"
        }
    } else {
        "spice"
    };

    let uri = format!("{scheme}://{}:{}", connection.host, connection.port);
    args.push(uri);

    // Get SPICE-specific config
    if let rustconn_core::models::ProtocolConfig::Spice(ref spice_config) =
        connection.protocol_config
    {
        // CA certificate
        if let Some(ref ca_cert) = spice_config.ca_cert_path {
            args.push(format!("--spice-ca-file={}", ca_cert.display()));
        }

        // USB redirection
        if spice_config.usb_redirection {
            args.push("--spice-usbredir-redirect-on-connect=auto".to_string());
        }

        // Shared folders
        for folder in &spice_config.shared_folders {
            args.push(format!(
                "--spice-shared-dir={}",
                folder.local_path.display()
            ));
        }
    }

    ConnectionCommand {
        program: "remote-viewer".to_string(),
        args,
    }
}

/// Builds Zero Trust command arguments using cloud CLI tools
///
/// Zero Trust connections use cloud provider CLIs (aws, gcloud, az, oci, etc.)
/// to establish secure connections through identity-aware proxies.
fn build_zerotrust_command(connection: &Connection) -> ConnectionCommand {
    if let rustconn_core::models::ProtocolConfig::ZeroTrust(ref zt_config) =
        connection.protocol_config
    {
        // Use the build_command method from ZeroTrustConfig
        let (program, mut args) = zt_config.build_command(connection.username.as_deref());

        // Add any custom arguments
        args.extend(zt_config.custom_args.clone());

        ConnectionCommand { program, args }
    } else {
        // Fallback - should not happen if protocol type matches
        eprintln!("Warning: ZeroTrust protocol type but no ZeroTrust config");
        ConnectionCommand {
            program: "echo".to_string(),
            args: vec!["Invalid Zero Trust configuration".to_string()],
        }
    }
}

/// Executes the connection command
fn execute_connection_command(command: &ConnectionCommand) -> Result<(), CliError> {
    use std::process::Command;

    // Check if the program exists
    let program_check = Command::new("which")
        .arg(&command.program)
        .output()
        .map_err(|e| CliError::Config(format!("Failed to check for {}: {e}", command.program)))?;

    if !program_check.status.success() {
        return Err(CliError::Config(format!(
            "Required program '{}' not found. Please install it to use this connection type.",
            command.program
        )));
    }

    // Execute the connection command
    // We use exec on Unix to replace the current process with the connection
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        let mut cmd = Command::new(&command.program);
        cmd.args(&command.args);

        // Print the command being executed
        eprintln!("Executing: {} {}", command.program, command.args.join(" "));

        // exec() replaces the current process - this never returns on success
        let err = cmd.exec();
        Err(CliError::Config(format!(
            "Failed to execute {}: {err}",
            command.program
        )))
    }

    #[cfg(not(unix))]
    {
        // On non-Unix systems, spawn the process and wait
        let mut cmd = Command::new(&command.program);
        cmd.args(&command.args);

        // Print the command being executed
        eprintln!("Executing: {} {}", command.program, command.args.join(" "));

        let status = cmd
            .status()
            .map_err(|e| CliError::Config(format!("Failed to execute {}: {e}", command.program)))?;

        if status.success() {
            Ok(())
        } else {
            Err(CliError::Config(format!(
                "{} exited with status: {}",
                command.program,
                status.code().unwrap_or(-1)
            )))
        }
    }
}

/// Add connection command handler
fn cmd_add(
    name: &str,
    host: &str,
    port: Option<u16>,
    protocol: &str,
    user: Option<&str>,
    key: Option<&std::path::Path>,
) -> Result<(), CliError> {
    // Parse protocol and determine default port
    let (protocol_type, default_port) = parse_protocol(protocol)?;
    let port = port.unwrap_or(default_port);

    // Create the connection based on protocol
    let mut connection = create_connection(name, host, port, protocol_type, key);

    // Set username if provided
    if let Some(username) = user {
        connection.username = Some(username.to_string());
    }

    // Load existing connections
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    // Validate the new connection
    ConfigManager::validate_connection(&connection)
        .map_err(|e| CliError::Config(format!("Invalid connection: {e}")))?;

    // Add the new connection
    connections.push(connection.clone());

    // Save connections
    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    println!(
        "Added connection '{}' ({} {}:{}) with ID {}",
        connection.name, connection.protocol, connection.host, connection.port, connection.id
    );

    Ok(())
}

/// Parse protocol string and return protocol type with default port
fn parse_protocol(protocol: &str) -> Result<(ProtocolType, u16), CliError> {
    match protocol.to_lowercase().as_str() {
        "ssh" => Ok((ProtocolType::Ssh, 22)),
        "rdp" => Ok((ProtocolType::Rdp, 3389)),
        "vnc" => Ok((ProtocolType::Vnc, 5900)),
        "spice" => Ok((ProtocolType::Spice, 5900)),
        _ => Err(CliError::Config(format!(
            "Unknown protocol '{protocol}'. Supported protocols: ssh, rdp, vnc, spice"
        ))),
    }
}

/// Create a connection with the specified parameters
fn create_connection(
    name: &str,
    host: &str,
    port: u16,
    protocol_type: ProtocolType,
    key: Option<&std::path::Path>,
) -> Connection {
    match protocol_type {
        ProtocolType::Ssh => {
            let mut conn = Connection::new_ssh(name.to_string(), host.to_string(), port);
            // Set key path if provided
            if let Some(key_path) = key {
                if let rustconn_core::models::ProtocolConfig::Ssh(ref mut ssh_config) =
                    conn.protocol_config
                {
                    ssh_config.key_path = Some(key_path.to_path_buf());
                    ssh_config.auth_method = rustconn_core::models::SshAuthMethod::PublicKey;
                }
            }
            conn
        }
        ProtocolType::Rdp => {
            if key.is_some() {
                eprintln!("Warning: --key option is ignored for RDP connections");
            }
            Connection::new_rdp(name.to_string(), host.to_string(), port)
        }
        ProtocolType::Vnc => {
            if key.is_some() {
                eprintln!("Warning: --key option is ignored for VNC connections");
            }
            Connection::new_vnc(name.to_string(), host.to_string(), port)
        }
        ProtocolType::Spice => {
            if key.is_some() {
                eprintln!("Warning: --key option is ignored for SPICE connections");
            }
            Connection::new_spice(name.to_string(), host.to_string(), port)
        }
        ProtocolType::ZeroTrust => {
            eprintln!("Error: Zero Trust connections cannot be created via CLI quick-connect");
            eprintln!("Use the GUI to configure Zero Trust connections");
            // Return SSH as fallback
            Connection::new_ssh(name.to_string(), host.to_string(), port)
        }
    }
}

/// Export connections command handler
fn cmd_export(format: ExportFormatArg, output: &std::path::Path) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load groups: {e}")))?;

    // Convert CLI format to export format
    let export_format = match format {
        ExportFormatArg::Ansible => rustconn_core::export::ExportFormat::Ansible,
        ExportFormatArg::SshConfig => rustconn_core::export::ExportFormat::SshConfig,
        ExportFormatArg::Remmina => rustconn_core::export::ExportFormat::Remmina,
        ExportFormatArg::Asbru => rustconn_core::export::ExportFormat::Asbru,
        ExportFormatArg::Native => rustconn_core::export::ExportFormat::Native,
        ExportFormatArg::RoyalTs => rustconn_core::export::ExportFormat::RoyalTs,
    };

    // Create export options
    let options = rustconn_core::export::ExportOptions::new(export_format, output.to_path_buf());

    // Call the appropriate exporter
    let result = export_connections(&connections, &groups, &options)?;

    // Display results
    println!(
        "Export complete: {} connections exported, {} skipped",
        result.exported_count, result.skipped_count
    );

    if !result.warnings.is_empty() {
        eprintln!("\nWarnings:");
        for warning in &result.warnings {
            eprintln!("  - {warning}");
        }
    }

    if !result.output_files.is_empty() {
        println!("\nOutput files:");
        for file in &result.output_files {
            println!("  - {}", file.display());
        }
    }

    Ok(())
}

/// Exports connections using the appropriate exporter based on format
fn export_connections(
    connections: &[Connection],
    groups: &[ConnectionGroup],
    options: &rustconn_core::export::ExportOptions,
) -> Result<rustconn_core::export::ExportResult, CliError> {
    use rustconn_core::export::{
        AnsibleExporter, AsbruExporter, ExportFormat, ExportTarget, NativeExport, RemminaExporter,
        RoyalTsExporter, SshConfigExporter,
    };

    let result = match options.format {
        ExportFormat::Ansible => {
            let exporter = AnsibleExporter::new();
            exporter
                .export(connections, groups, options)
                .map_err(|e| CliError::Export(e.to_string()))?
        }
        ExportFormat::SshConfig => {
            let exporter = SshConfigExporter::new();
            exporter
                .export(connections, groups, options)
                .map_err(|e| CliError::Export(e.to_string()))?
        }
        ExportFormat::Remmina => {
            let exporter = RemminaExporter::new();
            exporter
                .export(connections, groups, options)
                .map_err(|e| CliError::Export(e.to_string()))?
        }
        ExportFormat::Asbru => {
            let exporter = AsbruExporter::new();
            exporter
                .export(connections, groups, options)
                .map_err(|e| CliError::Export(e.to_string()))?
        }
        ExportFormat::Native => {
            // Native format exports all data including templates, clusters, and variables
            let native_export = NativeExport::with_data(
                connections.to_vec(),
                groups.to_vec(),
                Vec::new(), // Templates not available in this context
                Vec::new(), // Clusters not available in this context
                Vec::new(), // Variables not available in this context
            );

            // Write to output path
            native_export
                .to_file(&options.output_path)
                .map_err(|e| CliError::Export(e.to_string()))?;
            rustconn_core::export::ExportResult {
                exported_count: connections.len(),
                skipped_count: 0,
                warnings: Vec::new(),
                output_files: vec![options.output_path.clone()],
            }
        }
        ExportFormat::RoyalTs => {
            let exporter = RoyalTsExporter::new();
            exporter
                .export(connections, groups, options)
                .map_err(|e| CliError::Export(e.to_string()))?
        }
    };

    Ok(result)
}

/// Import connections command handler
fn cmd_import(format: ImportFormatArg, file: &std::path::Path) -> Result<(), CliError> {
    // Check if file exists
    if !file.exists() {
        return Err(CliError::Import(format!(
            "File not found: {}",
            file.display()
        )));
    }

    // Load existing connections
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

    let mut existing_connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load existing connections: {e}")))?;

    let mut existing_groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Config(format!("Failed to load existing groups: {e}")))?;

    // Import connections using the appropriate importer
    let import_result = import_connections(format, file)?;

    // Display import summary
    println!("Import Summary:");
    println!(
        "  Connections imported: {}",
        import_result.connections.len()
    );
    println!("  Groups imported: {}", import_result.groups.len());
    println!("  Entries skipped: {}", import_result.skipped.len());
    println!("  Errors: {}", import_result.errors.len());

    // Display skipped entries if any
    if !import_result.skipped.is_empty() {
        eprintln!("\nSkipped entries:");
        for skipped in &import_result.skipped {
            if let Some(ref location) = skipped.location {
                eprintln!(
                    "  - {} ({}): {}",
                    skipped.identifier, location, skipped.reason
                );
            } else {
                eprintln!("  - {}: {}", skipped.identifier, skipped.reason);
            }
        }
    }

    // Display errors if any
    if !import_result.errors.is_empty() {
        eprintln!("\nErrors:");
        for error in &import_result.errors {
            eprintln!("  - {error}");
        }
    }

    // Merge imported connections with existing
    let initial_count = existing_connections.len();
    let initial_group_count = existing_groups.len();

    // Add imported groups (avoiding duplicates by name)
    for group in import_result.groups {
        if !existing_groups.iter().any(|g| g.name == group.name) {
            existing_groups.push(group);
        }
    }

    // Add imported connections (avoiding duplicates by name and host)
    for conn in import_result.connections {
        let is_duplicate = existing_connections
            .iter()
            .any(|c| c.name == conn.name && c.host == conn.host);

        if !is_duplicate {
            existing_connections.push(conn);
        }
    }

    let new_connections = existing_connections.len() - initial_count;
    let new_groups = existing_groups.len() - initial_group_count;

    // Save merged connections
    config_manager
        .save_connections(&existing_connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    config_manager
        .save_groups(&existing_groups)
        .map_err(|e| CliError::Config(format!("Failed to save groups: {e}")))?;

    println!("\nMerge results:");
    println!("  New connections added: {new_connections}");
    println!("  New groups added: {new_groups}");
    println!("  Total connections: {}", existing_connections.len());
    println!("  Total groups: {}", existing_groups.len());

    Ok(())
}

/// Imports connections using the appropriate importer based on format
fn import_connections(
    format: ImportFormatArg,
    file: &std::path::Path,
) -> Result<rustconn_core::import::ImportResult, CliError> {
    use rustconn_core::import::{
        AnsibleInventoryImporter, AsbruImporter, ImportResult, ImportSource, RemminaImporter,
        RoyalTsImporter, SshConfigImporter,
    };

    let result = match format {
        ImportFormatArg::Ansible => {
            let importer = AnsibleInventoryImporter::new();
            importer
                .import_from_path(file)
                .map_err(|e| CliError::Import(e.to_string()))?
        }
        ImportFormatArg::SshConfig => {
            let importer = SshConfigImporter::new();
            importer
                .import_from_path(file)
                .map_err(|e| CliError::Import(e.to_string()))?
        }
        ImportFormatArg::Remmina => {
            let importer = RemminaImporter::new();
            importer
                .import_from_path(file)
                .map_err(|e| CliError::Import(e.to_string()))?
        }
        ImportFormatArg::Asbru => {
            let importer = AsbruImporter::new();
            importer
                .import_from_path(file)
                .map_err(|e| CliError::Import(e.to_string()))?
        }
        ImportFormatArg::Native => {
            // Native format uses NativeExport::from_file
            let native = rustconn_core::export::NativeExport::from_file(file)
                .map_err(|e| CliError::Import(e.to_string()))?;

            ImportResult {
                connections: native.connections,
                groups: native.groups,
                skipped: Vec::new(),
                errors: Vec::new(),
            }
        }
        ImportFormatArg::RoyalTs => {
            let importer = RoyalTsImporter::new();
            importer
                .import_from_path(file)
                .map_err(|e| CliError::Import(e.to_string()))?
        }
    };

    Ok(result)
}

/// Test connection command handler
fn cmd_test(name: &str, timeout: u64) -> Result<(), CliError> {
    // Load connections
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    // Handle empty connections case
    if connections.is_empty() {
        if name.eq_ignore_ascii_case("all") {
            println!("No connections configured.");
            return Ok(());
        }
        return Err(CliError::ConnectionNotFound(name.to_string()));
    }

    // Create the connection tester with the specified timeout
    let tester = rustconn_core::testing::ConnectionTester::with_timeout(
        std::time::Duration::from_secs(timeout),
    );

    // Create a tokio runtime for async operations
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| CliError::TestFailed(format!("Failed to create async runtime: {e}")))?;

    // Determine which connections to test
    if name.eq_ignore_ascii_case("all") {
        // Test all connections
        println!("Testing {} connections...\n", connections.len());

        let summary = runtime.block_on(tester.test_batch(&connections));

        // Display individual results
        for result in &summary.results {
            print_test_result(result);
        }

        // Display summary
        println!();
        print_test_summary(&summary);

        // Return error exit code if any tests failed
        if summary.has_failures() {
            return Err(CliError::TestFailed(format!(
                "{} of {} tests failed",
                summary.failed, summary.total
            )));
        }
    } else {
        // Find connection by name or ID
        let connection = find_connection(&connections, name)?;

        println!("Testing connection '{}'...\n", connection.name);

        let result = runtime.block_on(tester.test_connection(connection));
        print_test_result(&result);

        if result.is_failure() {
            return Err(CliError::TestFailed(
                result.error.unwrap_or_else(|| "Unknown error".to_string()),
            ));
        }
    }

    Ok(())
}

/// Find a connection by name or UUID
fn find_connection<'a>(
    connections: &'a [Connection],
    name_or_id: &str,
) -> Result<&'a Connection, CliError> {
    // First try to find by exact name match
    if let Some(conn) = connections.iter().find(|c| c.name == name_or_id) {
        return Ok(conn);
    }

    // Try to find by UUID
    if let Ok(uuid) = uuid::Uuid::parse_str(name_or_id) {
        if let Some(conn) = connections.iter().find(|c| c.id == uuid) {
            return Ok(conn);
        }
    }

    // Try case-insensitive name match
    if let Some(conn) = connections
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(name_or_id))
    {
        return Ok(conn);
    }

    // Try partial name match (prefix)
    let matches: Vec<_> = connections
        .iter()
        .filter(|c| {
            c.name
                .to_lowercase()
                .starts_with(&name_or_id.to_lowercase())
        })
        .collect();

    match matches.len() {
        0 => Err(CliError::ConnectionNotFound(name_or_id.to_string())),
        1 => Ok(matches[0]),
        _ => {
            let names: Vec<_> = matches.iter().map(|c| c.name.as_str()).collect();
            Err(CliError::Config(format!(
                "Ambiguous connection name '{}'. Matches: {}",
                name_or_id,
                names.join(", ")
            )))
        }
    }
}

/// Print a single test result with colors
fn print_test_result(result: &rustconn_core::testing::TestResult) {
    // ANSI color codes
    const GREEN: &str = "\x1b[32m";
    const RED: &str = "\x1b[31m";
    const YELLOW: &str = "\x1b[33m";
    const CYAN: &str = "\x1b[36m";
    const RESET: &str = "\x1b[0m";
    const BOLD: &str = "\x1b[1m";

    if result.success {
        // Success: green checkmark
        print!("{GREEN}{BOLD}✓{RESET} ");
        print!("{}", result.connection_name);

        if let Some(latency) = result.latency_ms {
            print!(" {CYAN}({latency}ms){RESET}");
        }

        // Print protocol detail if available
        if let Some(protocol) = result.details.get("protocol") {
            print!(" [{protocol}]");
        }

        println!();
    } else {
        // Failure: red X
        print!("{RED}{BOLD}✗{RESET} ");
        print!("{}", result.connection_name);

        if let Some(ref error) = result.error {
            print!(" {YELLOW}- {error}{RESET}");
        }

        println!();

        // Print additional details for failures
        if !result.details.is_empty() {
            for (key, value) in &result.details {
                println!("    {key}: {value}");
            }
        }
    }
}

/// Print the test summary with colors
fn print_test_summary(summary: &rustconn_core::testing::TestSummary) {
    // ANSI color codes
    const GREEN: &str = "\x1b[32m";
    const RED: &str = "\x1b[31m";
    const RESET: &str = "\x1b[0m";
    const BOLD: &str = "\x1b[1m";

    println!("{BOLD}Test Summary:{RESET}");
    println!("  Total:  {}", summary.total);

    if summary.passed > 0 {
        println!("  {GREEN}Passed: {}{RESET}", summary.passed);
    } else {
        println!("  Passed: {}", summary.passed);
    }

    if summary.failed > 0 {
        println!("  {RED}Failed: {}{RESET}", summary.failed);
    } else {
        println!("  Failed: {}", summary.failed);
    }

    // Pass rate
    let pass_rate = summary.pass_rate();
    if pass_rate >= 100.0 {
        println!("  {GREEN}Pass rate: {pass_rate:.1}%{RESET}");
    } else if pass_rate >= 50.0 {
        println!("  Pass rate: {pass_rate:.1}%");
    } else {
        println!("  {RED}Pass rate: {pass_rate:.1}%{RESET}");
    }
}

/// Delete connection command handler
fn cmd_delete(name: &str) -> Result<(), CliError> {
    // Load connections
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    // Find the connection to get its ID
    let connection = find_connection(&connections, name)?;
    let id = connection.id;
    let conn_name = connection.name.clone();

    // Remove from list
    let mut connections = connections;
    connections.retain(|c| c.id != id);

    // Save connections
    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    println!("Deleted connection '{conn_name}' (ID: {id})");

    Ok(())
}

/// Show connection details command handler
fn cmd_show(name: &str) -> Result<(), CliError> {
    // Load connections
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    // Find the connection
    let connection = find_connection(&connections, name)?;

    println!("Connection Details:");
    println!("  ID:       {}", connection.id);
    println!("  Name:     {}", connection.name);
    println!("  Host:     {}", connection.host);
    println!("  Port:     {}", connection.port);
    println!("  Protocol: {}", connection.protocol);

    if let Some(ref user) = connection.username {
        println!("  Username: {user}");
    }

    // Protocol specific details
    match connection.protocol_config {
        rustconn_core::models::ProtocolConfig::Ssh(ref config) => {
            if let Some(ref key) = config.key_path {
                println!("  Key Path: {}", key.display());
            }
            if let Some(ref jump) = config.proxy_jump {
                println!("  Proxy Jump: {jump}");
            }
        }
        rustconn_core::models::ProtocolConfig::Rdp(ref config) => {
            if let Some(ref domain) = connection.domain {
                println!("  Domain:   {domain}");
            }
            if let Some(ref res) = config.resolution {
                println!("  Resolution: {}x{}", res.width, res.height);
            }
        }
        _ => {}
    }

    Ok(())
}

/// Update connection command handler
fn cmd_update(
    name: &str,
    new_name: Option<&str>,
    host: Option<&str>,
    port: Option<u16>,
    user: Option<&str>,
) -> Result<(), CliError> {
    // Load connections
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    // Find the connection index
    let index = connections
        .iter()
        .position(|c| c.name == name || c.id.to_string() == name)
        .ok_or_else(|| CliError::ConnectionNotFound(name.to_string()))?;

    let connection = &mut connections[index];

    // Update fields
    if let Some(new_name) = new_name {
        connection.name = new_name.to_string();
    }
    if let Some(host) = host {
        connection.host = host.to_string();
    }
    if let Some(port) = port {
        connection.port = port;
    }
    if let Some(user) = user {
        connection.username = Some(user.to_string());
    }

    connection.updated_at = chrono::Utc::now();

    // Validate
    ConfigManager::validate_connection(connection)
        .map_err(|e| CliError::Config(format!("Invalid connection: {e}")))?;

    let id = connection.id;
    let name = connection.name.clone();

    // Save connections
    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    println!("Updated connection '{name}' (ID: {id})");

    Ok(())
}

/// Exit codes for CLI operations
pub mod exit_codes {
    /// Success - operation completed successfully
    pub const SUCCESS: i32 = 0;
    /// General error - configuration, validation, or other non-connection errors
    pub const GENERAL_ERROR: i32 = 1;
    /// Connection failure - connection test failed or connection could not be established
    pub const CONNECTION_FAILURE: i32 = 2;
}

/// CLI error type
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// Configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Connection not found
    #[error("Connection not found: {0}")]
    ConnectionNotFound(String),

    /// Export error
    #[error("Export error: {0}")]
    Export(String),

    /// Import error
    #[error("Import error: {0}")]
    Import(String),

    /// Connection test failed
    #[error("Connection test failed: {0}")]
    TestFailed(String),

    /// Wake-on-LAN error
    #[error("Wake-on-LAN error: {0}")]
    Wol(String),

    /// Snippet error
    #[error("Snippet error: {0}")]
    Snippet(String),

    /// Group error
    #[error("Group error: {0}")]
    Group(String),

    /// Template error
    #[error("Template error: {0}")]
    Template(String),

    /// Cluster error
    #[error("Cluster error: {0}")]
    Cluster(String),

    /// Variable error
    #[error("Variable error: {0}")]
    Variable(String),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl CliError {
    /// Returns the appropriate exit code for this error type.
    ///
    /// Exit codes:
    /// - 0: Success (not an error)
    /// - 1: General error (configuration, validation, export, import, IO)
    /// - 2: Connection failure (test failed, connection not found)
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        match self {
            // Connection-related failures use exit code 2
            Self::TestFailed(_) | Self::ConnectionNotFound(_) => exit_codes::CONNECTION_FAILURE,
            // All other errors use exit code 1
            Self::Config(_)
            | Self::Export(_)
            | Self::Import(_)
            | Self::Io(_)
            | Self::Wol(_)
            | Self::Snippet(_)
            | Self::Group(_)
            | Self::Template(_)
            | Self::Cluster(_)
            | Self::Variable(_) => exit_codes::GENERAL_ERROR,
        }
    }

    /// Returns true if this is a connection-related failure.
    #[must_use]
    pub const fn is_connection_failure(&self) -> bool {
        matches!(self, Self::TestFailed(_) | Self::ConnectionNotFound(_))
    }
}

// ============================================================================
// Wake-on-LAN command
// ============================================================================

/// Wake-on-LAN command handler
fn cmd_wol(target: &str, broadcast: &str, port: u16) -> Result<(), CliError> {
    // Try to parse target as MAC address first
    let mac = if let Ok(mac) = target.parse::<MacAddress>() {
        mac
    } else {
        // Try to find connection by name and get its WOL config
        let config_manager = ConfigManager::new()
            .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

        let connections = config_manager
            .load_connections()
            .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

        let connection = find_connection(&connections, target)?;

        connection
            .wol_config
            .as_ref()
            .map(|wol| wol.mac_address)
            .ok_or_else(|| {
                CliError::Wol(format!(
                    "Connection '{}' does not have Wake-on-LAN configured",
                    connection.name
                ))
            })?
    };

    let config = WolConfig::new(mac)
        .with_broadcast_address(broadcast)
        .with_port(port);

    println!("Sending Wake-on-LAN magic packet...");
    println!("  MAC Address: {mac}");
    println!("  Broadcast:   {broadcast}:{port}");

    rustconn_core::wol::send_wol(&config).map_err(|e| CliError::Wol(e.to_string()))?;

    println!("Magic packet sent successfully!");
    println!(
        "Note: The target machine may take up to {} seconds to wake up.",
        config.wait_seconds
    );

    Ok(())
}

// ============================================================================
// Snippet commands
// ============================================================================

/// Snippet command handler
fn cmd_snippet(subcmd: SnippetCommands) -> Result<(), CliError> {
    match subcmd {
        SnippetCommands::List {
            format,
            category,
            tag,
        } => cmd_snippet_list(format, category.as_deref(), tag.as_deref()),
        SnippetCommands::Show { name } => cmd_snippet_show(&name),
        SnippetCommands::Add {
            name,
            command,
            description,
            category,
            tags,
        } => cmd_snippet_add(&name, &command, description.as_deref(), category, tags),
        SnippetCommands::Delete { name } => cmd_snippet_delete(&name),
        SnippetCommands::Run { name, var, execute } => cmd_snippet_run(&name, &var, execute),
    }
}

/// List snippets command
fn cmd_snippet_list(
    format: OutputFormat,
    category: Option<&str>,
    tag: Option<&str>,
) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

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

/// Print snippets as table
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

/// Print snippets as JSON
fn print_snippet_json(snippets: &[&Snippet]) -> Result<(), CliError> {
    let json = serde_json::to_string_pretty(snippets)
        .map_err(|e| CliError::Snippet(format!("Failed to serialize: {e}")))?;
    println!("{json}");
    Ok(())
}

/// Print snippets as CSV
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

/// Show snippet details
fn cmd_snippet_show(name: &str) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

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

    // Show variables
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

/// Add a new snippet
fn cmd_snippet_add(
    name: &str,
    command: &str,
    description: Option<&str>,
    category: Option<String>,
    tags: Option<String>,
) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

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

    // Extract and add variables
    let variables = SnippetManager::extract_variable_objects(command);
    snippet = snippet.with_variables(variables);

    let id = snippet_manager
        .create_snippet_from(snippet)
        .map_err(|e| CliError::Snippet(format!("Failed to create snippet: {e}")))?;

    println!("Created snippet '{name}' with ID {id}");

    // Show extracted variables
    let vars = SnippetManager::extract_variables(command);
    if !vars.is_empty() {
        println!("Variables: {}", vars.join(", "));
    }

    Ok(())
}

/// Delete a snippet
fn cmd_snippet_delete(name: &str) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

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

/// Run a snippet with variable substitution
fn cmd_snippet_run(name: &str, vars: &[(String, String)], execute: bool) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

    let snippet_manager = SnippetManager::new(config_manager)
        .map_err(|e| CliError::Snippet(format!("Failed to load snippets: {e}")))?;

    let snippet = find_snippet(&snippet_manager, name)?;

    // Build values map
    let values: HashMap<String, String> = vars.iter().cloned().collect();

    // Check for missing variables
    let missing = SnippetManager::get_missing_variables(snippet, &values);
    if !missing.is_empty() {
        return Err(CliError::Snippet(format!(
            "Missing required variables: {}. Use --var name=value to provide them.",
            missing.join(", ")
        )));
    }

    // Substitute variables
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
    // Try UUID first
    if let Ok(uuid) = uuid::Uuid::parse_str(name_or_id) {
        if let Some(snippet) = manager.get_snippet(uuid) {
            return Ok(snippet);
        }
    }

    // Search by name
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

// ============================================================================
// Group commands
// ============================================================================

/// Group command handler
fn cmd_group(subcmd: GroupCommands) -> Result<(), CliError> {
    match subcmd {
        GroupCommands::List { format } => cmd_group_list(format),
        GroupCommands::Show { name } => cmd_group_show(&name),
        GroupCommands::Create {
            name,
            parent,
            description,
        } => cmd_group_create(&name, parent.as_deref(), description.as_deref()),
        GroupCommands::Delete { name } => cmd_group_delete(&name),
        GroupCommands::AddConnection { group, connection } => {
            cmd_group_add_connection(&group, &connection)
        }
        GroupCommands::RemoveConnection { group, connection } => {
            cmd_group_remove_connection(&group, &connection)
        }
    }
}

/// List groups command
fn cmd_group_list(format: OutputFormat) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

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

/// Print groups as table
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

/// Print groups as JSON
fn print_group_json(groups: &[ConnectionGroup]) -> Result<(), CliError> {
    let json = serde_json::to_string_pretty(groups)
        .map_err(|e| CliError::Group(format!("Failed to serialize: {e}")))?;
    println!("{json}");
    Ok(())
}

/// Print groups as CSV
fn print_group_csv(groups: &[ConnectionGroup]) {
    println!("name,parent_id");
    for group in groups {
        let name = escape_csv_field(&group.name);
        let parent = group.parent_id.map(|id| id.to_string()).unwrap_or_default();
        println!("{name},{parent}");
    }
}

/// Show group details
fn cmd_group_show(name: &str) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

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

    if let Some(parent_id) = group.parent_id {
        let parent_name = groups
            .iter()
            .find(|g| g.id == parent_id)
            .map_or("(unknown)", |g| g.name.as_str());
        println!("  Parent: {parent_name} ({parent_id})");
    }

    // Find connections in this group
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

/// Create a new group
fn cmd_group_create(
    name: &str,
    parent: Option<&str>,
    _description: Option<&str>,
) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

    let mut groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Group(format!("Failed to load groups: {e}")))?;

    // Check for duplicate name
    if groups.iter().any(|g| g.name.eq_ignore_ascii_case(name)) {
        return Err(CliError::Group(format!(
            "Group with name '{name}' already exists"
        )));
    }

    let group = if let Some(parent_name) = parent {
        let parent_group = find_group(&groups, parent_name)?;
        ConnectionGroup::with_parent(name.to_string(), parent_group.id)
    } else {
        ConnectionGroup::new(name.to_string())
    };

    let id = group.id;
    groups.push(group);

    config_manager
        .save_groups(&groups)
        .map_err(|e| CliError::Group(format!("Failed to save groups: {e}")))?;

    println!("Created group '{name}' with ID {id}");

    Ok(())
}

/// Delete a group
fn cmd_group_delete(name: &str) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

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

/// Add a connection to a group
fn cmd_group_add_connection(group_name: &str, connection_name: &str) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Group(format!("Failed to load groups: {e}")))?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let group = find_group(&groups, group_name)?;
    let group_id = group.id;
    let grp_name = group.name.clone();

    // Find and update the connection
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

/// Remove a connection from a group
fn cmd_group_remove_connection(group_name: &str, connection_name: &str) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

    let groups = config_manager
        .load_groups()
        .map_err(|e| CliError::Group(format!("Failed to load groups: {e}")))?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let group = find_group(&groups, group_name)?;
    let group_id = group.id;
    let grp_name = group.name.clone();

    // Find and update the connection
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
    // Try UUID first
    if let Ok(uuid) = uuid::Uuid::parse_str(name_or_id) {
        if let Some(group) = groups.iter().find(|g| g.id == uuid) {
            return Ok(group);
        }
    }

    // Search by name (case-insensitive)
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

// ============================================================================
// Template commands
// ============================================================================

/// Template command handler
fn cmd_template(subcmd: TemplateCommands) -> Result<(), CliError> {
    match subcmd {
        TemplateCommands::List { format, protocol } => {
            cmd_template_list(format, protocol.as_deref())
        }
        TemplateCommands::Show { name } => cmd_template_show(&name),
        TemplateCommands::Create {
            name,
            protocol,
            host,
            port,
            user,
            description,
        } => cmd_template_create(
            &name,
            &protocol,
            host.as_deref(),
            port,
            user.as_deref(),
            description.as_deref(),
        ),
        TemplateCommands::Delete { name } => cmd_template_delete(&name),
        TemplateCommands::Apply {
            template,
            name,
            host,
            port,
            user,
        } => cmd_template_apply(
            &template,
            name.as_deref(),
            host.as_deref(),
            port,
            user.as_deref(),
        ),
    }
}

/// List templates command
fn cmd_template_list(format: OutputFormat, protocol: Option<&str>) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

    let templates = config_manager
        .load_templates()
        .map_err(|e| CliError::Template(format!("Failed to load templates: {e}")))?;

    // Filter by protocol if specified
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

/// Print templates as table
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

/// Print templates as JSON
fn print_template_json(templates: &[&ConnectionTemplate]) -> Result<(), CliError> {
    let json = serde_json::to_string_pretty(templates)
        .map_err(|e| CliError::Template(format!("Failed to serialize: {e}")))?;
    println!("{json}");
    Ok(())
}

/// Print templates as CSV
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

/// Show template details
fn cmd_template_show(name: &str) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

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

/// Create a new template
fn cmd_template_create(
    name: &str,
    protocol: &str,
    host: Option<&str>,
    port: Option<u16>,
    user: Option<&str>,
    description: Option<&str>,
) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

    let mut templates = config_manager
        .load_templates()
        .map_err(|e| CliError::Template(format!("Failed to load templates: {e}")))?;

    // Create template based on protocol
    let mut template = match protocol.to_lowercase().as_str() {
        "ssh" => ConnectionTemplate::new_ssh(name.to_string()),
        "rdp" => ConnectionTemplate::new_rdp(name.to_string()),
        "vnc" => ConnectionTemplate::new_vnc(name.to_string()),
        "spice" => ConnectionTemplate::new_spice(name.to_string()),
        _ => {
            return Err(CliError::Template(format!(
                "Unknown protocol '{protocol}'. Supported: ssh, rdp, vnc, spice"
            )))
        }
    };

    // Apply optional fields
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

/// Delete a template
fn cmd_template_delete(name: &str) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

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

/// Apply a template to create a new connection
fn cmd_template_apply(
    template_name: &str,
    conn_name: Option<&str>,
    host: Option<&str>,
    port: Option<u16>,
    user: Option<&str>,
) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

    let templates = config_manager
        .load_templates()
        .map_err(|e| CliError::Template(format!("Failed to load templates: {e}")))?;

    let template = find_template(&templates, template_name)?;

    // Apply template to create connection
    let mut connection = template.apply(conn_name.map(String::from));

    // Override with provided values
    if let Some(h) = host {
        connection.host = h.to_string();
    }
    if let Some(p) = port {
        connection.port = p;
    }
    if let Some(u) = user {
        connection.username = Some(u.to_string());
    }

    // Load and save connections
    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let id = connection.id;
    let name = connection.name.clone();
    connections.push(connection);

    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    println!("Created connection '{name}' from template '{template_name}' (ID: {id})");

    Ok(())
}

/// Find a template by name or ID
fn find_template<'a>(
    templates: &'a [ConnectionTemplate],
    name_or_id: &str,
) -> Result<&'a ConnectionTemplate, CliError> {
    // Try UUID first
    if let Ok(uuid) = uuid::Uuid::parse_str(name_or_id) {
        if let Some(template) = templates.iter().find(|t| t.id == uuid) {
            return Ok(template);
        }
    }

    // Search by name (case-insensitive)
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

// ============================================================================
// Cluster commands
// ============================================================================

/// Cluster command handler
fn cmd_cluster(subcmd: ClusterCommands) -> Result<(), CliError> {
    match subcmd {
        ClusterCommands::List { format } => cmd_cluster_list(format),
        ClusterCommands::Show { name } => cmd_cluster_show(&name),
        ClusterCommands::Create {
            name,
            connections,
            broadcast,
        } => cmd_cluster_create(&name, connections.as_deref(), broadcast),
        ClusterCommands::Delete { name } => cmd_cluster_delete(&name),
        ClusterCommands::AddConnection {
            cluster,
            connection,
        } => cmd_cluster_add_connection(&cluster, &connection),
        ClusterCommands::RemoveConnection {
            cluster,
            connection,
        } => cmd_cluster_remove_connection(&cluster, &connection),
    }
}

/// List clusters command
fn cmd_cluster_list(format: OutputFormat) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

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

/// Print clusters as table
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

/// Print clusters as JSON
fn print_cluster_json(clusters: &[Cluster]) -> Result<(), CliError> {
    let json = serde_json::to_string_pretty(clusters)
        .map_err(|e| CliError::Cluster(format!("Failed to serialize: {e}")))?;
    println!("{json}");
    Ok(())
}

/// Print clusters as CSV
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

/// Show cluster details
fn cmd_cluster_show(name: &str) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

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

/// Create a new cluster
fn cmd_cluster_create(
    name: &str,
    connections: Option<&str>,
    broadcast: bool,
) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

    let mut clusters = config_manager
        .load_clusters()
        .map_err(|e| CliError::Cluster(format!("Failed to load clusters: {e}")))?;

    let all_connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let mut cluster = Cluster::new(name.to_string());
    cluster.broadcast_enabled = broadcast;

    // Add connections if specified
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

/// Delete a cluster
fn cmd_cluster_delete(name: &str) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

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

/// Add a connection to a cluster
fn cmd_cluster_add_connection(cluster_name: &str, connection_name: &str) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

    let mut clusters = config_manager
        .load_clusters()
        .map_err(|e| CliError::Cluster(format!("Failed to load clusters: {e}")))?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let connection = find_connection(&connections, connection_name)?;
    let conn_id = connection.id;
    let conn_name = connection.name.clone();

    // Find and update cluster
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

/// Remove a connection from a cluster
fn cmd_cluster_remove_connection(
    cluster_name: &str,
    connection_name: &str,
) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

    let mut clusters = config_manager
        .load_clusters()
        .map_err(|e| CliError::Cluster(format!("Failed to load clusters: {e}")))?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let connection = find_connection(&connections, connection_name)?;
    let conn_id = connection.id;
    let conn_name = connection.name.clone();

    // Find and update cluster
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

    println!("Removed connection '{conn_name}' from cluster '{clust_name}'");

    Ok(())
}

/// Find a cluster by name or ID
fn find_cluster<'a>(clusters: &'a [Cluster], name_or_id: &str) -> Result<&'a Cluster, CliError> {
    // Try UUID first
    if let Ok(uuid) = uuid::Uuid::parse_str(name_or_id) {
        if let Some(cluster) = clusters.iter().find(|c| c.id == uuid) {
            return Ok(cluster);
        }
    }

    // Search by name (case-insensitive)
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

// ============================================================================
// Variable commands
// ============================================================================

/// Variable command handler
fn cmd_var(subcmd: VariableCommands) -> Result<(), CliError> {
    match subcmd {
        VariableCommands::List { format } => cmd_var_list(format),
        VariableCommands::Show { name } => cmd_var_show(&name),
        VariableCommands::Set {
            name,
            value,
            secret,
            description,
        } => cmd_var_set(&name, &value, secret, description.as_deref()),
        VariableCommands::Delete { name } => cmd_var_delete(&name),
    }
}

/// List variables command
fn cmd_var_list(format: OutputFormat) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

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

/// Print variables as table
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

/// Print variables as JSON
fn print_var_json(variables: &[Variable]) -> Result<(), CliError> {
    // Create a safe output that masks secret values
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

/// Print variables as CSV
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

/// Show variable details
fn cmd_var_show(name: &str) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

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

/// Set a variable
fn cmd_var_set(
    name: &str,
    value: &str,
    secret: bool,
    description: Option<&str>,
) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

    let mut variables = config_manager
        .load_variables()
        .map_err(|e| CliError::Variable(format!("Failed to load variables: {e}")))?;

    // Check if variable exists
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

/// Delete a variable
fn cmd_var_delete(name: &str) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

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

// ============================================================================
// Duplicate and Stats commands
// ============================================================================

/// Duplicate a connection
fn cmd_duplicate(name: &str, new_name: Option<&str>) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

    let mut connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    let source = find_connection(&connections, name)?;

    // Create a duplicate with new ID
    let mut duplicate = source.clone();
    duplicate.id = uuid::Uuid::new_v4();
    duplicate.name = new_name
        .map(String::from)
        .unwrap_or_else(|| format!("{} (copy)", source.name));
    duplicate.created_at = chrono::Utc::now();
    duplicate.updated_at = chrono::Utc::now();
    duplicate.last_connected = None;

    let id = duplicate.id;
    let dup_name = duplicate.name.clone();
    connections.push(duplicate);

    config_manager
        .save_connections(&connections)
        .map_err(|e| CliError::Config(format!("Failed to save connections: {e}")))?;

    println!("Created duplicate connection '{dup_name}' (ID: {id})");

    Ok(())
}

/// Show connection statistics
fn cmd_stats() -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

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

    // Count by protocol
    let mut by_protocol: HashMap<String, usize> = HashMap::new();
    for conn in &connections {
        *by_protocol
            .entry(conn.protocol.as_str().to_string())
            .or_insert(0) += 1;
    }

    // Count recently used (last 7 days)
    let week_ago = chrono::Utc::now() - chrono::Duration::days(7);
    let recent_count = connections
        .iter()
        .filter(|c| c.last_connected.is_some_and(|t| t > week_ago))
        .count();

    // Count connections with last_connected set (ever used)
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
