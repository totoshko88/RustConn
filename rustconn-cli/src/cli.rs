//! CLI argument parsing types using `clap`.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

use crate::util::parse_key_val;

/// `RustConn` command-line interface for managing remote connections
#[derive(Parser)]
#[command(name = "rustconn-cli")]
#[command(author, version, about = "RustConn command-line interface")]
#[command(propagate_version = true)]
pub struct Cli {
    /// Path to the configuration directory
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,

    /// Increase output verbosity (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Suppress all output except errors
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Disable colored output
    #[arg(long, global = true, env = "NO_COLOR")]
    pub no_color: bool,

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

        /// Show the command that would be executed without running it
        #[arg(long)]
        dry_run: bool,
    },

    /// Add a new connection
    #[command(about = "Add a new connection to the configuration")]
    Add {
        /// Name for the new connection
        #[arg(short, long)]
        name: String,

        /// Host address (hostname or IP), or device path for serial
        #[arg(short = 'H', long)]
        host: String,

        /// Port number (defaults to protocol default: SSH=22, RDP=3389,
        /// VNC=5900)
        #[arg(short, long)]
        port: Option<u16>,

        /// Protocol type (ssh, rdp, vnc, spice, telnet, serial)
        #[arg(short = 'P', long, default_value = "ssh")]
        protocol: String,

        /// Username for authentication
        #[arg(short, long)]
        user: Option<String>,

        /// Path to SSH private key file
        #[arg(short, long)]
        key: Option<PathBuf>,

        /// SSH authentication method (password, publickey,
        /// keyboard-interactive, agent, security-key)
        #[arg(long, value_name = "METHOD")]
        auth_method: Option<String>,

        /// Serial device path (e.g., /dev/ttyUSB0). Alias for --host
        /// with serial protocol
        #[arg(long, value_name = "PATH")]
        device: Option<String>,

        /// Serial baud rate (default: 115200)
        #[arg(long, default_value = "115200")]
        baud_rate: Option<u32>,
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

        /// Path to SSH private key file
        #[arg(short, long)]
        key: Option<PathBuf>,

        /// SSH authentication method (password, publickey,
        /// keyboard-interactive, agent, security-key)
        #[arg(long, value_name = "METHOD")]
        auth_method: Option<String>,

        /// Serial device path
        #[arg(long, value_name = "PATH")]
        device: Option<String>,

        /// Serial baud rate
        #[arg(long)]
        baud_rate: Option<u32>,
    },

    /// Send Wake-on-LAN magic packet
    #[command(about = "Wake a sleeping machine using Wake-on-LAN")]
    Wol {
        /// Connection name or MAC address
        /// (format: AA:BB:CC:DD:EE:FF or AA-BB-CC-DD-EE-FF)
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

    /// Manage secret backends and credentials
    #[command(subcommand, about = "Manage secret backends and credentials")]
    Secret(SecretCommands),

    /// Duplicate a connection
    #[command(about = "Duplicate an existing connection")]
    Duplicate {
        /// Connection name or UUID to duplicate
        name: String,

        /// New name for the duplicated connection
        #[arg(short, long)]
        new_name: Option<String>,
    },

    /// Open SFTP session for an SSH connection
    #[command(about = "Open SFTP file browser or CLI session for an SSH connection")]
    Sftp {
        /// Connection name or UUID
        name: String,

        /// Use sftp CLI instead of file manager
        #[arg(long)]
        cli: bool,

        /// Open SFTP via Midnight Commander (mc) in terminal
        #[arg(long)]
        mc: bool,
    },

    /// Show connection statistics
    #[command(about = "Show connection statistics")]
    Stats,

    /// Generate shell completions
    #[command(about = "Generate shell completion scripts")]
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
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
    /// Native `RustConn` format (.rcn)
    Native,
    /// Royal TS XML format (.rtsz)
    RoyalTs,
    /// MobaXterm session format (.mxtsessions)
    MobaXterm,
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
    /// MobaXterm session format (.mxtsessions)
    MobaXterm,
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

/// Secret backend subcommands
#[derive(Subcommand)]
pub enum SecretCommands {
    /// Show available secret backends and their status
    #[command(about = "Show available secret backends and their status")]
    Status,

    /// Get password for a connection from secret backend
    #[command(about = "Get password for a connection from secret backend")]
    Get {
        /// Connection name or ID
        connection: String,

        /// Secret backend to use
        /// (keyring, keepass, bitwarden, 1password, passbolt)
        #[arg(short, long)]
        backend: Option<String>,
    },

    /// Store password for a connection in secret backend
    #[command(about = "Store password for a connection in secret backend")]
    Set {
        /// Connection name or ID
        connection: String,

        /// Username (optional, uses connection username if not specified)
        #[arg(short, long)]
        user: Option<String>,

        /// Password (if not provided, will prompt interactively)
        #[arg(short, long)]
        password: Option<String>,

        /// Secret backend to use
        /// (keyring, keepass, bitwarden, 1password, passbolt)
        #[arg(short, long)]
        backend: Option<String>,
    },

    /// Delete password for a connection from secret backend
    #[command(about = "Delete password for a connection from secret backend")]
    Delete {
        /// Connection name or ID
        connection: String,

        /// Secret backend to use
        /// (keyring, keepass, bitwarden, 1password, passbolt)
        #[arg(short, long)]
        backend: Option<String>,
    },

    /// Verify KeePass database credentials
    #[command(about = "Verify KeePass database credentials")]
    VerifyKeepass {
        /// Path to KDBX file
        #[arg(short, long)]
        database: PathBuf,

        /// Path to key file (optional)
        #[arg(short, long)]
        key_file: Option<PathBuf>,
    },
}
