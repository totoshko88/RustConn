//! `RustConn` CLI - Command-line interface for `RustConn` connection manager
//!
//! Provides commands for listing, adding, exporting, importing, and testing connections.

use std::fmt::Write as _;
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use rustconn_core::config::ConfigManager;
use rustconn_core::models::{Connection, ConnectionGroup, ProtocolType};

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

        /// Filter connections by protocol (ssh, rdp, vnc)
        #[arg(short, long)]
        protocol: Option<String>,
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
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::List { format, protocol } => cmd_list(format, protocol.as_deref()),
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
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(e.exit_code());
    }
}

/// List connections command handler
fn cmd_list(format: OutputFormat, protocol: Option<&str>) -> Result<(), CliError> {
    let config_manager = ConfigManager::new()
        .map_err(|e| CliError::Config(format!("Failed to initialize config: {e}")))?;

    let connections = config_manager
        .load_connections()
        .map_err(|e| CliError::Config(format!("Failed to load connections: {e}")))?;

    // Filter by protocol if specified
    let filtered: Vec<&Connection> = protocol.map_or_else(
        || connections.iter().collect(),
        |proto_filter| {
            let proto_lower = proto_filter.to_lowercase();
            connections
                .iter()
                .filter(|c| c.protocol.as_str() == proto_lower)
                .collect()
        },
    );

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
        SshConfigExporter,
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
        AnsibleInventoryImporter, AsbruImporter, ImportSource, RemminaImporter, SshConfigImporter,
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

    if connections.is_empty() {
        println!("No connections configured.");
        return Ok(());
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
            Self::Config(_) | Self::Export(_) | Self::Import(_) | Self::Io(_) => {
                exit_codes::GENERAL_ERROR
            }
        }
    }

    /// Returns true if this is a connection-related failure.
    #[must_use]
    pub const fn is_connection_failure(&self) -> bool {
        matches!(self, Self::TestFailed(_) | Self::ConnectionNotFound(_))
    }
}
