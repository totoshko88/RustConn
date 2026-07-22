//! CLI argument parsing types using `clap`.

use std::io::IsTerminal;
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
    #[arg(long, global = true)]
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
    #[cfg(feature = "client-launch")]
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

        /// Protocol type (ssh, rdp, vnc, spice, sftp, telnet, serial,
        /// mosh, kubernetes/k8s, zerotrust/zt)
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

        /// Custom icon (emoji/unicode or GTK icon name, e.g. "🏢",
        /// "starred-symbolic")
        #[arg(long)]
        icon: Option<String>,

        /// Custom SSH agent socket path (overrides global and auto-detected socket)
        #[arg(long, value_name = "PATH")]
        ssh_agent_socket: Option<String>,

        /// Zero Trust provider (for zerotrust/zt protocol)
        #[arg(
            long,
            value_name = "PROVIDER",
            value_parser = ["aws_ssm", "gcp_iap", "azure_bastion", "azure_ssh", "cloudflare_access", "teleport", "tailscale_ssh", "oci_bastion", "boundary", "hoop_dev", "generic"]
        )]
        provider: Option<String>,

        /// Hoop.dev connection name (required for --provider hoop_dev)
        #[arg(long, value_name = "NAME")]
        hoop_connection_name: Option<String>,

        /// Hoop.dev gateway URL (optional, for --provider hoop_dev)
        #[arg(long, value_name = "URL")]
        hoop_gateway_url: Option<String>,

        /// Hoop.dev gRPC URL (optional, for --provider hoop_dev)
        #[arg(long, value_name = "URL")]
        hoop_grpc_url: Option<String>,

        /// AWS SSM instance ID (for --provider aws_ssm, uses --host as target if not set)
        #[arg(long, value_name = "PROFILE")]
        aws_profile: Option<String>,

        /// AWS region (for --provider aws_ssm)
        #[arg(long, value_name = "REGION")]
        aws_region: Option<String>,

        /// GCP zone (for --provider gcp_iap)
        #[arg(long, value_name = "ZONE")]
        gcp_zone: Option<String>,

        /// GCP project (for --provider gcp_iap)
        #[arg(long, value_name = "PROJECT")]
        gcp_project: Option<String>,

        /// Azure resource group (for --provider azure_bastion or azure_ssh)
        #[arg(long, value_name = "GROUP")]
        resource_group: Option<String>,

        /// Azure Bastion host name (for --provider azure_bastion)
        #[arg(long, value_name = "NAME")]
        bastion_name: Option<String>,

        /// Azure VM name (for --provider azure_ssh)
        #[arg(long, value_name = "NAME")]
        vm_name: Option<String>,

        /// OCI Bastion OCID (for --provider oci_bastion)
        #[arg(long, value_name = "OCID")]
        bastion_id: Option<String>,

        /// OCI target resource OCID (for --provider oci_bastion)
        #[arg(long, value_name = "OCID")]
        target_resource_id: Option<String>,

        /// OCI target private IP (for --provider oci_bastion)
        #[arg(long, value_name = "IP")]
        target_private_ip: Option<String>,

        /// Teleport cluster (for --provider teleport)
        #[arg(long, value_name = "CLUSTER")]
        teleport_cluster: Option<String>,

        /// Boundary target (for --provider boundary)
        #[arg(long, value_name = "TARGET")]
        boundary_target: Option<String>,

        /// Boundary address (for --provider boundary)
        #[arg(long, value_name = "URL")]
        boundary_addr: Option<String>,

        /// Generic command template (for --provider generic)
        /// Placeholders: {host}, {user}, {port}
        #[arg(long, value_name = "COMMAND")]
        custom_command: Option<String>,

        /// Existing SSH connection name or UUID to use as a jump host
        /// (sets jump_host_id for SSH/RDP/VNC/SPICE/SFTP connections)
        #[arg(long, value_name = "NAME|UUID")]
        jump_host: Option<String>,

        /// SSH keep-alive interval in seconds (ServerAliveInterval)
        #[arg(long, value_name = "SECONDS")]
        keep_alive_interval: Option<u32>,

        /// SSH keep-alive count max (ServerAliveCountMax)
        #[arg(long, value_name = "COUNT")]
        keep_alive_count: Option<u32>,

        /// Enable SSH verbose/debug output (-v flag)
        #[arg(long)]
        ssh_verbose: bool,

        /// Enable Multipath TCP for SSH/RDP/VNC connections.
        /// SSH: passes -o TCPMultipath=yes (requires OpenSSH 9.9+).
        /// RDP/VNC embedded: uses MPTCP socket (requires Linux 5.6+ kernel).
        /// Falls back to regular TCP transparently when unavailable.
        #[arg(long)]
        mptcp: bool,

        /// Accept untrusted certificate for RDP or VNC (skip verification)
        #[arg(long)]
        ignore_certificate: bool,

        /// Comma-separated tags (e.g. "production,linux,critical")
        #[arg(long, value_name = "TAG[,TAG...]")]
        tags: Option<String>,

        /// Description text for the connection
        #[arg(long, value_name = "TEXT")]
        description: Option<String>,

        /// Group name to assign the connection to (creates the group if missing)
        #[arg(long, value_name = "NAME")]
        group: Option<String>,

        /// Windows domain for RDP/SPICE authentication
        #[arg(long, value_name = "DOMAIN")]
        domain: Option<String>,

        /// Window mode: embedded (default), external, or fullscreen.
        /// Currently honored only for RDP and VNC; ignored for other protocols.
        #[arg(
            long,
            value_name = "MODE",
            value_parser = ["embedded", "external", "fullscreen"]
        )]
        window_mode: Option<String>,

        /// Skip pre-connect TCP port check for this connection
        #[arg(long)]
        skip_port_check: bool,

        /// Enable X11 forwarding (-X flag) for SSH/SFTP connections
        #[arg(long)]
        x11_forwarding: bool,

        /// Enable SSH agent forwarding (-A flag) for SSH/SFTP connections
        #[arg(long)]
        agent_forwarding: bool,

        /// Enable compression (-C flag) for SSH/SFTP connections
        #[arg(long)]
        compression: bool,

        /// Command to execute on SSH connection startup
        #[arg(long, value_name = "TEXT")]
        startup_command: Option<String>,

        /// SSH ProxyCommand for connections that require a proxy
        /// (e.g., "ncat --proxy 127.0.0.1:9050 --proxy-type socks5 %h %p")
        #[arg(long, value_name = "TEXT")]
        proxy_command: Option<String>,

        /// Custom SSH option (repeatable, format: Key=Value)
        #[arg(long, value_name = "K=V", value_parser = parse_key_val)]
        ssh_option: Vec<(String, String)>,

        /// Local port forwarding (repeatable, format: LOCAL_PORT:REMOTE_HOST:REMOTE_PORT)
        #[arg(long, value_name = "L:H:P")]
        local_forward: Vec<String>,

        /// Remote port forwarding (repeatable, format: REMOTE_PORT:LOCAL_HOST:LOCAL_PORT)
        #[arg(long, value_name = "R:H:P")]
        remote_forward: Vec<String>,

        /// Dynamic (SOCKS) port forwarding (repeatable, format: PORT)
        #[arg(long, value_name = "PORT")]
        dynamic_forward: Vec<String>,

        /// RDP gateway hostname (enables gateway tunneling)
        #[arg(long, value_name = "HOST")]
        gateway: Option<String>,

        /// RDP gateway port (default: 443)
        #[arg(long, value_name = "PORT")]
        gateway_port: Option<u16>,

        /// RDP gateway username (if different from connection username)
        #[arg(long, value_name = "USER")]
        gateway_username: Option<String>,

        /// RemoteApp program path or alias (launches single app instead of full desktop)
        #[arg(long, value_name = "PATH")]
        remote_app_program: Option<String>,

        /// RemoteApp command-line arguments
        #[arg(long, value_name = "ARGS")]
        remote_app_args: Option<String>,

        /// RemoteApp display name (shown in taskbar/window title)
        #[arg(long, value_name = "NAME")]
        remote_app_name: Option<String>,

        /// RDP resolution (e.g. "1920x1080")
        #[arg(long, value_name = "WxH")]
        resolution: Option<String>,

        /// RDP color depth (8, 15, 16, 24, or 32)
        #[arg(long, value_name = "BITS")]
        color_depth: Option<u8>,

        /// Disable Network Level Authentication for RDP
        #[arg(long)]
        disable_nla: bool,

        /// RDP keyboard layout override (Windows KLID, e.g. 0x00000409 for US)
        #[arg(long, value_name = "KLID")]
        keyboard_layout: Option<u32>,

        /// Enable audio redirection for RDP
        #[arg(long)]
        audio_redirect: bool,

        /// Enable printer redirection for RDP (maps local CUPS printer)
        #[arg(long)]
        printer: bool,

        /// Shared folder for RDP drive redirection (repeatable, format: NAME:PATH)
        #[arg(long, value_name = "NAME:PATH")]
        shared_folder: Vec<String>,

        // --- VNC-specific flags ---
        /// VNC client mode: embedded (default) or external
        #[arg(long, value_name = "MODE", value_parser = ["embedded", "external"])]
        vnc_client_mode: Option<String>,

        /// VNC performance mode: quality, balanced (default), or speed
        #[arg(long, value_name = "MODE", value_parser = ["quality", "balanced", "speed"])]
        vnc_performance: Option<String>,

        /// VNC encoding: tight, zrle, or hextile
        #[arg(long, value_name = "ENC", value_parser = ["tight", "zrle", "hextile"])]
        vnc_encoding: Option<String>,

        /// VNC compression level (0-9)
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(0..=9))]
        vnc_compression: Option<u8>,

        /// VNC quality level (0-9)
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(0..=9))]
        vnc_quality: Option<u8>,

        /// VNC view-only mode (no keyboard/mouse input)
        #[arg(long)]
        vnc_view_only: bool,

        /// Disable VNC scaling (don't scale display to fit window)
        #[arg(long)]
        vnc_no_scaling: bool,

        /// Disable VNC clipboard sharing
        #[arg(long)]
        vnc_no_clipboard: bool,

        /// Custom VNC client argument (repeatable)
        #[arg(long, value_name = "ARG")]
        vnc_custom_arg: Vec<String>,

        // --- SPICE-specific flags ---
        /// Enable SPICE TLS encryption
        #[arg(long)]
        spice_tls: bool,

        /// SPICE CA certificate path for TLS verification
        #[arg(long, value_name = "PATH")]
        spice_ca_cert: Option<String>,

        /// Skip SPICE certificate verification (insecure)
        #[arg(long)]
        spice_skip_cert_verify: bool,

        /// Enable SPICE USB redirection
        #[arg(long)]
        spice_usb_redirection: bool,

        /// Disable SPICE clipboard sharing
        #[arg(long)]
        spice_no_clipboard: bool,

        /// SPICE image compression mode: auto, off, glz, lz, quic
        #[arg(long, value_name = "MODE", value_parser = ["auto", "off", "glz", "lz", "quic"])]
        spice_image_compression: Option<String>,

        /// SPICE proxy URL (e.g. http://proxy:3128)
        #[arg(long, value_name = "URL")]
        spice_proxy: Option<String>,

        /// SPICE shared folder (repeatable, format: NAME:PATH)
        #[arg(long, value_name = "NAME:PATH")]
        spice_shared_folder: Vec<String>,

        // --- MOSH-specific flags ---
        /// SSH port for MOSH initial handshake
        #[arg(long, value_name = "PORT")]
        mosh_ssh_port: Option<u16>,

        /// MOSH UDP port range (e.g. 60000:60010)
        #[arg(long, value_name = "RANGE")]
        mosh_port_range: Option<String>,

        /// Path to remote mosh-server binary
        #[arg(long, value_name = "PATH")]
        mosh_server_binary: Option<String>,

        /// MOSH prediction mode: adaptive (default), always, never
        #[arg(long, value_name = "MODE", value_parser = ["adaptive", "always", "never"])]
        mosh_predict: Option<String>,

        /// Custom MOSH client argument (repeatable)
        #[arg(long, value_name = "ARG")]
        mosh_custom_arg: Vec<String>,

        // --- Serial-specific wave-2 flags ---
        /// Serial data bits: 5, 6, 7, 8 (default)
        #[arg(long, value_name = "N", value_parser = ["5", "6", "7", "8"])]
        serial_data_bits: Option<String>,

        /// Serial stop bits: 1 (default), 2
        #[arg(long, value_name = "N", value_parser = ["1", "2"])]
        serial_stop_bits: Option<String>,

        /// Serial parity: none (default), odd, even
        #[arg(long, value_name = "MODE", value_parser = ["none", "odd", "even"])]
        serial_parity: Option<String>,

        /// Serial flow control: none (default), hardware, software
        #[arg(long, value_name = "MODE", value_parser = ["none", "hardware", "software"])]
        serial_flow_control: Option<String>,

        /// Custom serial client argument (repeatable)
        #[arg(long, value_name = "ARG")]
        serial_custom_arg: Vec<String>,

        // --- Web-specific flags ---
        /// Web browser mode: embedded (default on Linux), system, or custom
        #[arg(long, value_name = "MODE", value_parser = ["embedded", "system", "custom"])]
        browser_mode: Option<String>,

        /// Enable or disable JavaScript in the embedded WebView [default: true]
        #[arg(long, value_name = "BOOL", num_args = 0..=1, default_missing_value = "false")]
        javascript: Option<bool>,

        /// Custom user agent string for the embedded browser (max 512 chars)
        #[arg(long, value_name = "STRING")]
        user_agent: Option<String>,

        /// Accept invalid TLS certificates (self-signed, expired) in embedded browser [true/false]
        #[arg(long, value_name = "BOOL", num_args = 0..=1, default_missing_value = "true")]
        accept_invalid_certs: Option<bool>,

        /// Open in private/incognito mode (custom browser only)
        #[arg(long)]
        private_mode: bool,

        /// Zoom level for the embedded browser (0.3–3.0, default: 1.0)
        #[arg(long, value_name = "FLOAT")]
        zoom_level: Option<f64>,
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

        /// CSV delimiter (comma, semicolon, tab) — only for CSV format
        #[arg(long, value_parser = ["comma", "semicolon", "tab"])]
        csv_delimiter: Option<String>,

        /// CSV fields to include (comma-separated list of field names) — only for CSV format
        #[arg(long, value_name = "FIELDS")]
        csv_fields: Option<String>,
    },

    /// Import connections from external format
    #[command(about = "Import connections from various formats")]
    Import {
        /// Import format
        #[arg(short, long, value_enum)]
        format: ImportFormatArg,

        /// Input file path
        #[arg(conflicts_with = "auto")]
        file: Option<PathBuf>,

        /// Auto-detect available import sources (Asbru, Remmina, SSH config, etc.)
        #[arg(long, conflicts_with = "file")]
        auto: bool,

        /// Show what would be imported without saving
        #[arg(long)]
        dry_run: bool,
    },

    /// Test connection connectivity
    #[command(about = "Test connectivity to a connection")]
    Test {
        /// Connection name or ID (use "all" to test all connections)
        name: String,

        /// Connection timeout in seconds
        #[arg(short, long, default_value = "10")]
        timeout: u64,

        /// Output format (table, json, csv)
        #[arg(short, long, default_value = "table", value_enum)]
        format: OutputFormat,
    },

    /// Delete a connection
    #[command(about = "Delete a connection")]
    Delete {
        /// Connection name or UUID
        name: String,

        /// Skip confirmation prompt
        #[arg(short, long)]
        force: bool,
    },

    /// Show connection details
    #[command(about = "Show detailed information about a connection")]
    Show {
        /// Connection name or UUID
        name: String,

        /// Output format (table, json, csv)
        #[arg(short, long, default_value = "table", value_enum)]
        format: OutputFormat,
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

        /// Custom icon (emoji/unicode or GTK icon name, e.g. "🏢",
        /// "starred-symbolic")
        #[arg(long)]
        icon: Option<String>,

        /// Custom SSH agent socket path (overrides global and auto-detected socket)
        #[arg(long, value_name = "PATH")]
        ssh_agent_socket: Option<String>,

        /// Zero Trust provider (for zerotrust/zt protocol)
        #[arg(
            long,
            value_name = "PROVIDER",
            value_parser = ["aws_ssm", "gcp_iap", "azure_bastion", "azure_ssh", "cloudflare_access", "teleport", "tailscale_ssh", "oci_bastion", "boundary", "hoop_dev", "generic"]
        )]
        provider: Option<String>,

        /// Hoop.dev connection name (for --provider hoop_dev)
        #[arg(long, value_name = "NAME")]
        hoop_connection_name: Option<String>,

        /// Hoop.dev gateway URL (optional, for --provider hoop_dev)
        #[arg(long, value_name = "URL")]
        hoop_gateway_url: Option<String>,

        /// Hoop.dev gRPC URL (optional, for --provider hoop_dev)
        #[arg(long, value_name = "URL")]
        hoop_grpc_url: Option<String>,

        /// AWS profile (for --provider aws_ssm)
        #[arg(long, value_name = "PROFILE")]
        aws_profile: Option<String>,

        /// AWS region (for --provider aws_ssm)
        #[arg(long, value_name = "REGION")]
        aws_region: Option<String>,

        /// GCP zone (for --provider gcp_iap)
        #[arg(long, value_name = "ZONE")]
        gcp_zone: Option<String>,

        /// GCP project (for --provider gcp_iap)
        #[arg(long, value_name = "PROJECT")]
        gcp_project: Option<String>,

        /// Azure resource group (for --provider azure_bastion or azure_ssh)
        #[arg(long, value_name = "GROUP")]
        resource_group: Option<String>,

        /// Azure Bastion host name (for --provider azure_bastion)
        #[arg(long, value_name = "NAME")]
        bastion_name: Option<String>,

        /// Azure VM name (for --provider azure_ssh)
        #[arg(long, value_name = "NAME")]
        vm_name: Option<String>,

        /// OCI Bastion OCID (for --provider oci_bastion)
        #[arg(long, value_name = "OCID")]
        bastion_id: Option<String>,

        /// OCI target resource OCID (for --provider oci_bastion)
        #[arg(long, value_name = "OCID")]
        target_resource_id: Option<String>,

        /// OCI target private IP (for --provider oci_bastion)
        #[arg(long, value_name = "IP")]
        target_private_ip: Option<String>,

        /// Teleport cluster (for --provider teleport)
        #[arg(long, value_name = "CLUSTER")]
        teleport_cluster: Option<String>,

        /// Boundary target (for --provider boundary)
        #[arg(long, value_name = "TARGET")]
        boundary_target: Option<String>,

        /// Boundary address (for --provider boundary)
        #[arg(long, value_name = "URL")]
        boundary_addr: Option<String>,

        /// Generic command template (for --provider generic)
        #[arg(long, value_name = "COMMAND")]
        custom_command: Option<String>,

        /// Existing SSH connection name or UUID to use as a jump host
        /// (sets jump_host_id for SSH/RDP/VNC/SPICE/SFTP connections)
        #[arg(long, value_name = "NAME|UUID")]
        jump_host: Option<String>,

        /// SSH keep-alive interval in seconds (ServerAliveInterval)
        #[arg(long, value_name = "SECONDS")]
        keep_alive_interval: Option<u32>,

        /// SSH keep-alive count max (ServerAliveCountMax)
        #[arg(long, value_name = "COUNT")]
        keep_alive_count: Option<u32>,

        /// Enable SSH verbose/debug output (-v flag)
        #[arg(long)]
        ssh_verbose: bool,

        /// Enable or disable Multipath TCP for SSH/RDP/VNC connections.
        /// SSH: passes -o TCPMultipath=yes (requires OpenSSH 9.9+).
        /// RDP/VNC embedded: uses MPTCP socket (requires Linux 5.6+ kernel).
        /// Falls back to regular TCP transparently when unavailable.
        /// Bare --mptcp enables; --mptcp false disables.
        #[arg(long, value_name = "BOOL", num_args = 0..=1, default_missing_value = "true")]
        mptcp: Option<bool>,

        /// Accept untrusted certificate for RDP or VNC (skip verification)
        #[arg(long)]
        ignore_certificate: bool,

        /// Comma-separated tags (replaces existing tags; use --add-tag/--remove-tag for incremental edits)
        #[arg(long, value_name = "TAG[,TAG...]")]
        tags: Option<String>,

        /// Add a single tag (repeatable; preserves existing tags)
        #[arg(long, value_name = "TAG")]
        add_tag: Vec<String>,

        /// Remove a single tag (repeatable; no error if missing)
        #[arg(long, value_name = "TAG")]
        remove_tag: Vec<String>,

        /// New description text
        #[arg(long, value_name = "TEXT")]
        description: Option<String>,

        /// Move connection to a different group (creates the group if missing)
        #[arg(long, value_name = "NAME")]
        group: Option<String>,

        /// Windows domain for RDP/SPICE authentication
        #[arg(long, value_name = "DOMAIN")]
        domain: Option<String>,

        /// Window mode: embedded, external, or fullscreen.
        /// Currently honored only for RDP and VNC; ignored for other protocols.
        #[arg(
            long,
            value_name = "MODE",
            value_parser = ["embedded", "external", "fullscreen"]
        )]
        window_mode: Option<String>,

        /// Set skip-port-check flag (use --skip-port-check=false to clear)
        #[arg(long, num_args = 0..=1, default_missing_value = "true")]
        skip_port_check: Option<bool>,

        /// Enable X11 forwarding (-X flag) for SSH/SFTP connections
        #[arg(long)]
        x11_forwarding: bool,

        /// Enable SSH agent forwarding (-A flag) for SSH/SFTP connections
        #[arg(long)]
        agent_forwarding: bool,

        /// Enable compression (-C flag) for SSH/SFTP connections
        #[arg(long)]
        compression: bool,

        /// Command to execute on SSH connection startup
        #[arg(long, value_name = "TEXT")]
        startup_command: Option<String>,

        /// SSH ProxyCommand for connections that require a proxy
        /// (e.g., "ncat --proxy 127.0.0.1:9050 --proxy-type socks5 %h %p")
        #[arg(long, value_name = "TEXT")]
        proxy_command: Option<String>,

        /// Custom SSH option (repeatable, format: Key=Value)
        #[arg(long, value_name = "K=V", value_parser = parse_key_val)]
        ssh_option: Vec<(String, String)>,

        /// Local port forwarding (repeatable, format: LOCAL_PORT:REMOTE_HOST:REMOTE_PORT)
        #[arg(long, value_name = "L:H:P")]
        local_forward: Vec<String>,

        /// Remote port forwarding (repeatable, format: REMOTE_PORT:LOCAL_HOST:LOCAL_PORT)
        #[arg(long, value_name = "R:H:P")]
        remote_forward: Vec<String>,

        /// Dynamic (SOCKS) port forwarding (repeatable, format: PORT)
        #[arg(long, value_name = "PORT")]
        dynamic_forward: Vec<String>,

        /// RDP gateway hostname (enables gateway tunneling)
        #[arg(long, value_name = "HOST")]
        gateway: Option<String>,

        /// RDP gateway port (default: 443)
        #[arg(long, value_name = "PORT")]
        gateway_port: Option<u16>,

        /// RDP gateway username (if different from connection username)
        #[arg(long, value_name = "USER")]
        gateway_username: Option<String>,

        /// RemoteApp program path or alias (launches single app instead of full desktop)
        #[arg(long, value_name = "PATH")]
        remote_app_program: Option<String>,

        /// RemoteApp command-line arguments
        #[arg(long, value_name = "ARGS")]
        remote_app_args: Option<String>,

        /// RemoteApp display name (shown in taskbar/window title)
        #[arg(long, value_name = "NAME")]
        remote_app_name: Option<String>,

        /// RDP resolution (e.g. "1920x1080")
        #[arg(long, value_name = "WxH")]
        resolution: Option<String>,

        /// RDP color depth (8, 15, 16, 24, or 32)
        #[arg(long, value_name = "BITS")]
        color_depth: Option<u8>,

        /// Disable Network Level Authentication for RDP
        #[arg(long)]
        disable_nla: bool,

        /// RDP keyboard layout override (Windows KLID, e.g. 0x00000409 for US)
        #[arg(long, value_name = "KLID")]
        keyboard_layout: Option<u32>,

        /// Enable audio redirection for RDP
        #[arg(long)]
        audio_redirect: bool,

        /// Enable printer redirection for RDP (maps local CUPS printer)
        #[arg(long)]
        printer: bool,

        /// Shared folder for RDP drive redirection (repeatable, format: NAME:PATH)
        #[arg(long, value_name = "NAME:PATH")]
        shared_folder: Vec<String>,

        // --- VNC-specific flags ---
        /// VNC client mode: embedded (default) or external
        #[arg(long, value_name = "MODE", value_parser = ["embedded", "external"])]
        vnc_client_mode: Option<String>,

        /// VNC performance mode: quality, balanced (default), or speed
        #[arg(long, value_name = "MODE", value_parser = ["quality", "balanced", "speed"])]
        vnc_performance: Option<String>,

        /// VNC encoding: tight, zrle, or hextile
        #[arg(long, value_name = "ENC", value_parser = ["tight", "zrle", "hextile"])]
        vnc_encoding: Option<String>,

        /// VNC compression level (0-9)
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(0..=9))]
        vnc_compression: Option<u8>,

        /// VNC quality level (0-9)
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(0..=9))]
        vnc_quality: Option<u8>,

        /// VNC view-only mode (no keyboard/mouse input)
        #[arg(long)]
        vnc_view_only: bool,

        /// Disable VNC scaling (don't scale display to fit window)
        #[arg(long)]
        vnc_no_scaling: bool,

        /// Disable VNC clipboard sharing
        #[arg(long)]
        vnc_no_clipboard: bool,

        /// Custom VNC client argument (repeatable)
        #[arg(long, value_name = "ARG")]
        vnc_custom_arg: Vec<String>,

        // --- SPICE-specific flags ---
        /// Enable SPICE TLS encryption
        #[arg(long)]
        spice_tls: bool,

        /// SPICE CA certificate path for TLS verification
        #[arg(long, value_name = "PATH")]
        spice_ca_cert: Option<String>,

        /// Skip SPICE certificate verification (insecure)
        #[arg(long)]
        spice_skip_cert_verify: bool,

        /// Enable SPICE USB redirection
        #[arg(long)]
        spice_usb_redirection: bool,

        /// Disable SPICE clipboard sharing
        #[arg(long)]
        spice_no_clipboard: bool,

        /// SPICE image compression mode: auto, off, glz, lz, quic
        #[arg(long, value_name = "MODE", value_parser = ["auto", "off", "glz", "lz", "quic"])]
        spice_image_compression: Option<String>,

        /// SPICE proxy URL (e.g. http://proxy:3128)
        #[arg(long, value_name = "URL")]
        spice_proxy: Option<String>,

        /// SPICE shared folder (repeatable, format: NAME:PATH)
        #[arg(long, value_name = "NAME:PATH")]
        spice_shared_folder: Vec<String>,

        // --- MOSH-specific flags ---
        /// SSH port for MOSH initial handshake
        #[arg(long, value_name = "PORT")]
        mosh_ssh_port: Option<u16>,

        /// MOSH UDP port range (e.g. 60000:60010)
        #[arg(long, value_name = "RANGE")]
        mosh_port_range: Option<String>,

        /// Path to remote mosh-server binary
        #[arg(long, value_name = "PATH")]
        mosh_server_binary: Option<String>,

        /// MOSH prediction mode: adaptive (default), always, never
        #[arg(long, value_name = "MODE", value_parser = ["adaptive", "always", "never"])]
        mosh_predict: Option<String>,

        /// Custom MOSH client argument (repeatable)
        #[arg(long, value_name = "ARG")]
        mosh_custom_arg: Vec<String>,

        // --- Serial-specific wave-2 flags ---
        /// Serial data bits: 5, 6, 7, 8 (default)
        #[arg(long, value_name = "N", value_parser = ["5", "6", "7", "8"])]
        serial_data_bits: Option<String>,

        /// Serial stop bits: 1 (default), 2
        #[arg(long, value_name = "N", value_parser = ["1", "2"])]
        serial_stop_bits: Option<String>,

        /// Serial parity: none (default), odd, even
        #[arg(long, value_name = "MODE", value_parser = ["none", "odd", "even"])]
        serial_parity: Option<String>,

        /// Serial flow control: none (default), hardware, software
        #[arg(long, value_name = "MODE", value_parser = ["none", "hardware", "software"])]
        serial_flow_control: Option<String>,

        /// Custom serial client argument (repeatable)
        #[arg(long, value_name = "ARG")]
        serial_custom_arg: Vec<String>,

        // --- Web-specific flags ---
        /// Web browser mode: embedded (default on Linux), system, or custom
        #[arg(long, value_name = "MODE", value_parser = ["embedded", "system", "custom"])]
        browser_mode: Option<String>,

        /// Enable or disable JavaScript in the embedded WebView [default: true]
        #[arg(long, value_name = "BOOL", num_args = 0..=1, default_missing_value = "false")]
        javascript: Option<bool>,

        /// Custom user agent string for the embedded browser (max 512 chars)
        #[arg(long, value_name = "STRING")]
        user_agent: Option<String>,

        /// Accept invalid TLS certificates (self-signed, expired) in embedded browser [true/false]
        #[arg(long, value_name = "BOOL", num_args = 0..=1, default_missing_value = "true")]
        accept_invalid_certs: Option<bool>,

        /// Open in private/incognito mode (custom browser only)
        #[arg(long)]
        private_mode: bool,

        /// Zoom level for the embedded browser (0.3–3.0, default: 1.0)
        #[arg(long, value_name = "FLOAT")]
        zoom_level: Option<f64>,
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
    #[cfg(feature = "secret-management")]
    #[command(subcommand, about = "Manage secret backends and credentials")]
    Secret(SecretCommands),

    /// Manage smart folders
    #[command(
        subcommand,
        about = "Manage smart folders for dynamic connection grouping"
    )]
    SmartFolder(SmartFolderCommands),

    /// Manage dynamic folders
    #[command(
        subcommand,
        about = "Manage dynamic folders (script-generated connections)"
    )]
    DynamicFolder(DynamicFolderCommands),

    /// Manage session recordings
    #[command(subcommand, about = "Manage session recordings")]
    Recording(RecordingCommands),

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
    Stats {
        /// Output format (table, json, csv)
        #[arg(short, long, default_value = "table", value_enum)]
        format: OutputFormat,
    },

    /// Generate shell completions
    #[command(about = "Generate shell completion scripts")]
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Generate man page
    #[command(about = "Generate man page and write to stdout")]
    ManPage,

    /// Cloud Sync and inventory sync operations
    #[command(subcommand, about = "Cloud Sync operations and inventory sync")]
    Sync(SyncCommands),

    /// View and manage connection history
    #[command(subcommand, about = "View and manage connection history")]
    History(HistoryCommands),

    /// Pin a connection to favorites
    #[command(about = "Pin a connection to favorites")]
    Pin {
        /// Connection name or UUID
        name: String,
    },

    /// Unpin a connection from favorites
    #[command(about = "Unpin a connection from favorites")]
    Unpin {
        /// Connection name or UUID
        name: String,
    },

    /// Manage connection tags
    #[command(subcommand, about = "Manage connection tags")]
    Tag(TagCommands),

    /// Move a connection to a different group
    #[command(about = "Move a connection to a different group")]
    Move {
        /// Connection name or UUID
        name: String,

        /// Target group name (creates the group if missing)
        #[arg(short, long)]
        group: String,
    },

    /// Manage per-connection monitoring
    #[command(subcommand, about = "Manage per-connection monitoring")]
    Monitor(MonitorCommands),
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

impl OutputFormat {
    /// Returns the effective format, defaulting to JSON when stdout is not a terminal.
    ///
    /// Per clig.dev: "If stdin or stdout is not an interactive terminal,
    /// prefer structured output."
    #[must_use]
    pub(crate) fn effective(self) -> Self {
        if matches!(self, Self::Table) && !std::io::stdout().is_terminal() {
            Self::Json
        } else {
            self
        }
    }
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
    /// CSV format (.csv)
    Csv,
    /// SecureCRT session format (.ini directory)
    #[value(name = "secure-crt", alias = "securecrt")]
    SecureCrt,
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
    /// Microsoft RDP file (.rdp)
    #[value(name = "rdp", alias = "rdp-file")]
    Rdp,
    /// Remote Desktop Manager JSON export
    #[value(name = "rdm", alias = "remote-desktop-manager")]
    Rdm,
    /// Virt-Viewer connection file (.vv)
    #[value(name = "virt-viewer", alias = "vv")]
    VirtViewer,
    /// Libvirt domain XML / GNOME Boxes
    #[value(name = "libvirt", alias = "gnome-boxes")]
    Libvirt,
    /// CSV format (.csv)
    Csv,
    /// SecureCRT session format (.ini directory)
    #[value(name = "secure-crt", alias = "securecrt")]
    SecureCrt,
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

    /// Edit an existing snippet
    #[command(about = "Edit an existing command snippet")]
    Edit {
        /// Snippet name or ID
        name: String,

        /// New name
        #[arg(long)]
        new_name: Option<String>,

        /// New command template (use ${var} for variables)
        #[arg(short, long)]
        command: Option<String>,

        /// New description
        #[arg(short, long)]
        description: Option<String>,

        /// New category
        #[arg(long)]
        category: Option<String>,

        /// New tags (comma-separated, replaces existing)
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

        /// Output format (table, json, csv)
        #[arg(short, long, default_value = "table", value_enum)]
        format: OutputFormat,
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

        /// Custom icon (emoji/unicode or GTK icon name, e.g. "🏢",
        /// "starred-symbolic")
        #[arg(long)]
        icon: Option<String>,
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

    /// Edit group SSH inheritance settings
    #[command(about = "Edit group properties (name, icon, parent, SSH inheritance)")]
    Edit {
        /// Group name or ID
        name: String,

        /// New name for the group
        #[arg(long)]
        new_name: Option<String>,

        /// New parent group name or ID (use "none" to move to root)
        #[arg(long)]
        parent: Option<String>,

        /// Description
        #[arg(long)]
        description: Option<String>,

        /// Custom icon (emoji or GTK icon name)
        #[arg(long)]
        icon: Option<String>,

        /// SSH key path for inheritance by child connections (local-only)
        #[arg(long)]
        ssh_key_path: Option<String>,

        /// SSH authentication method (password, publickey, agent,
        /// keyboard-interactive, security-key)
        #[arg(long)]
        ssh_auth_method: Option<String>,

        /// SSH ProxyJump host for inheritance
        #[arg(long)]
        ssh_proxy_jump: Option<String>,

        /// SSH agent socket override for inheritance (local-only)
        #[arg(long)]
        ssh_agent_socket: Option<String>,

        /// Add an expect rule (JSON: {"pattern":"...","response":"...","priority":0,"timeout_ms":0,"one_shot":true})
        /// Can be specified multiple times
        #[arg(long, value_name = "JSON")]
        add_expect_rule: Vec<String>,

        /// Remove all existing expect rules before adding new ones
        #[arg(long)]
        clear_expect_rules: bool,

        /// Add a post-login script command. Can be specified multiple times
        #[arg(long, value_name = "COMMAND")]
        add_post_login_script: Vec<String>,

        /// Remove all existing post-login scripts before adding new ones
        #[arg(long)]
        clear_post_login_scripts: bool,
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

    /// Edit an existing template
    #[command(about = "Edit an existing connection template")]
    Edit {
        /// Template name or ID
        name: String,

        /// New name
        #[arg(long)]
        new_name: Option<String>,

        /// New default host
        #[arg(short = 'H', long)]
        host: Option<String>,

        /// New default port
        #[arg(short, long)]
        port: Option<u16>,

        /// New default username
        #[arg(short, long)]
        user: Option<String>,

        /// New description
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

    /// Edit a cluster (rename or toggle broadcast)
    #[command(about = "Edit a cluster's name or broadcast setting")]
    Edit {
        /// Cluster name or ID
        name: String,

        /// New name
        #[arg(long)]
        new_name: Option<String>,

        /// Enable or disable broadcast mode (true/false)
        #[arg(short, long)]
        broadcast: Option<bool>,
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
#[cfg(feature = "secret-management")]
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

        /// Password via argument (DEPRECATED: visible in /proc/cmdline;
        /// prefer --password-stdin or interactive prompt)
        #[arg(short, long, conflicts_with = "password_stdin")]
        password: Option<String>,

        /// Read password from stdin (one line, no trailing newline required).
        /// Safer than --password because the value never appears in process
        /// listings. Example: echo "s3cret" | rustconn-cli secret set myhost --password-stdin
        #[arg(long, conflicts_with = "password")]
        password_stdin: bool,

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

/// Smart folder subcommands
#[derive(Subcommand)]
pub enum SmartFolderCommands {
    /// List all smart folders
    #[command(about = "List all smart folders")]
    List {
        /// Output format
        #[arg(short, long, default_value = "table", value_enum)]
        format: OutputFormat,
    },

    /// Show connections matching a smart folder
    #[command(about = "Show connections matching a smart folder's filters")]
    Show {
        /// Smart folder name or ID
        name: String,
    },

    /// Create a new smart folder
    #[command(about = "Create a new smart folder with filter criteria")]
    Create {
        /// Smart folder name
        #[arg(short, long)]
        name: String,

        /// Custom icon (emoji, e.g. "🚀")
        #[arg(short, long)]
        icon: Option<String>,

        /// Filter by protocol (ssh, rdp, vnc, etc.)
        #[arg(short, long)]
        protocol: Option<String>,

        /// Filter by host glob pattern (e.g. "*.prod.*")
        #[arg(short = 'H', long)]
        host_pattern: Option<String>,

        /// Filter by tags (comma-separated)
        #[arg(short, long)]
        tags: Option<String>,
    },

    /// Edit an existing smart folder
    #[command(about = "Edit a smart folder's filters")]
    Edit {
        /// Smart folder name or ID
        name: String,

        /// New name
        #[arg(long)]
        new_name: Option<String>,

        /// New icon (emoji; use "none" to clear)
        #[arg(short, long)]
        icon: Option<String>,

        /// New protocol filter (ssh, rdp, vnc, etc.; use "none" to clear)
        #[arg(short, long)]
        protocol: Option<String>,

        /// New host glob pattern (use "none" to clear)
        #[arg(short = 'H', long)]
        host_pattern: Option<String>,

        /// New tags (comma-separated, replaces existing; use "none" to clear)
        #[arg(short, long)]
        tags: Option<String>,
    },

    /// Delete a smart folder
    #[command(about = "Delete a smart folder")]
    Delete {
        /// Smart folder name or ID
        name: String,
    },
}

/// Dynamic folder subcommands
#[derive(Subcommand)]
pub enum DynamicFolderCommands {
    /// List groups with dynamic folder configuration
    #[command(about = "List groups with dynamic folder configuration")]
    List {
        /// Output format
        #[arg(short, long, default_value = "table", value_enum)]
        format: OutputFormat,
    },

    /// Refresh a dynamic folder (execute its script)
    #[command(about = "Execute the dynamic folder script and update connections")]
    Refresh {
        /// Group name or ID containing the dynamic folder
        name: String,
    },

    /// Show dynamic folder configuration for a group
    #[command(about = "Show dynamic folder configuration and generated connections")]
    Show {
        /// Group name or ID
        name: String,
    },

    /// Set (create or update) a dynamic folder on a group
    #[command(about = "Configure a dynamic folder script on a group")]
    Set {
        /// Group name or ID
        name: String,

        /// Shell script to execute (run via sh -c)
        #[arg(short, long)]
        script: String,

        /// Working directory for the script
        #[arg(short, long)]
        workdir: Option<String>,

        /// Script timeout in seconds (default: 30)
        #[arg(short, long, default_value = "30")]
        timeout: u64,

        /// Auto-refresh interval in seconds (0 = manual only)
        #[arg(short, long, default_value = "0")]
        refresh_interval: u64,
    },

    /// Remove dynamic folder configuration from a group
    #[command(about = "Remove dynamic folder configuration from a group")]
    Remove {
        /// Group name or ID
        name: String,
    },
}

/// Recording subcommands
#[derive(Subcommand)]
pub enum RecordingCommands {
    /// List all recordings with metadata
    #[command(about = "List all recordings with metadata")]
    List {
        /// Output format
        #[arg(long, default_value = "table", value_enum)]
        format: OutputFormat,
    },

    /// Delete a recording by name
    #[command(about = "Delete a recording by name")]
    Delete {
        /// Recording display name or connection name
        name: String,

        /// Skip confirmation prompt
        #[arg(long)]
        force: bool,
    },

    /// Import external scriptreplay files
    #[command(about = "Import external scriptreplay files")]
    Import {
        /// Path to the data file
        data_file: PathBuf,

        /// Path to the timing file
        timing_file: PathBuf,
    },
}

/// Sync subcommands (Cloud Sync + inventory sync)
#[derive(Subcommand)]
pub enum SyncCommands {
    /// Show Cloud Sync status (sync directory, device name, per-group status)
    #[command(about = "Show Cloud Sync status")]
    Status,

    /// List all synced groups with mode and last sync time
    #[command(about = "List all synced groups")]
    List {
        /// Output format
        #[arg(short, long, default_value = "table", value_enum)]
        format: OutputFormat,
    },

    /// Export a Master group to its sync file
    #[command(about = "Export a Master group to its cloud sync file")]
    Export {
        /// Group name or ID to export
        group: String,
    },

    /// Import a .rcn sync file
    #[command(about = "Import a .rcn cloud sync file")]
    Import {
        /// Path to the .rcn file
        file: String,
    },

    /// Export all Master groups and import all Import groups
    #[command(about = "Sync now: export all Master groups, import all Import groups")]
    Now,

    /// Sync connections from a dynamic inventory source (JSON/YAML)
    #[command(about = "Sync connections from a dynamic inventory source (JSON/YAML)")]
    Inventory {
        /// Path to inventory file (JSON or YAML)
        file: PathBuf,

        /// Source identifier for tagging (e.g. "netbox", "ansible")
        #[arg(short, long)]
        source: String,

        /// Remove connections from this source that are no longer in the inventory
        #[arg(long)]
        remove_stale: bool,

        /// Dry run — show what would change without modifying anything
        #[arg(long)]
        dry_run: bool,
    },
}

/// History subcommands
#[derive(Subcommand)]
pub enum HistoryCommands {
    /// List recent connection history
    #[command(about = "List recent connection history entries")]
    List {
        /// Output format
        #[arg(short, long, default_value = "table", value_enum)]
        format: OutputFormat,

        /// Maximum number of entries to show
        #[arg(short, long, default_value = "20")]
        limit: usize,

        /// Filter by connection name
        #[arg(short, long)]
        connection: Option<String>,
    },

    /// Show details of a specific history entry
    #[command(about = "Show details of a specific history entry")]
    Show {
        /// History entry ID (UUID)
        id: String,
    },

    /// Clear all connection history
    #[command(about = "Clear all connection history")]
    Clear {
        /// Skip confirmation prompt
        #[arg(short, long)]
        force: bool,
    },
}

/// Tag subcommands
#[derive(Subcommand)]
pub enum TagCommands {
    /// List all tags used across connections
    #[command(about = "List all tags used across connections")]
    List {
        /// Output format
        #[arg(short, long, default_value = "table", value_enum)]
        format: OutputFormat,
    },

    /// Add a tag to a connection
    #[command(about = "Add a tag to a connection")]
    Add {
        /// Connection name or UUID
        #[arg(short, long)]
        connection: String,

        /// Tag to add
        #[arg(short, long)]
        tag: String,
    },

    /// Remove a tag from a connection
    #[command(about = "Remove a tag from a connection")]
    Remove {
        /// Connection name or UUID
        #[arg(short, long)]
        connection: String,

        /// Tag to remove
        #[arg(short, long)]
        tag: String,
    },
}

/// Monitor subcommands
#[derive(Subcommand)]
pub enum MonitorCommands {
    /// Enable monitoring for a connection
    #[command(about = "Enable per-connection monitoring")]
    Enable {
        /// Connection name or UUID
        name: String,

        /// Polling interval in seconds (overrides global setting)
        #[arg(short, long)]
        interval: Option<u8>,
    },

    /// Disable monitoring for a connection
    #[command(about = "Disable per-connection monitoring")]
    Disable {
        /// Connection name or UUID
        name: String,
    },

    /// Show monitoring metrics for a connection
    #[command(about = "Show monitoring metrics for a connection")]
    Metrics {
        /// Connection name or UUID
        name: String,

        /// Output format
        #[arg(short, long, default_value = "table", value_enum)]
        format: OutputFormat,
    },
}
