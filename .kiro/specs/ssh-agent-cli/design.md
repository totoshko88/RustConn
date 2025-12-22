# Design Document: SSH Agent Management, Export & CLI

## Overview

This design document describes the architecture for SSH Agent management, connection export functionality, and a Command Line Interface (CLI) for RustConn. These features enable better SSH key management, interoperability with other connection management tools, and automated testing/scripting capabilities.

## Architecture

The enhancements are organized into three main subsystems:

```
┌─────────────────────────────────────────────────────────────────┐
│                        rustconn (GUI)                           │
├─────────────────────────────────────────────────────────────────┤
│  ┌─────────────────┐  ┌─────────────────┐                      │
│  │ SSH Agent UI    │  │ Export Dialog   │                      │
│  │ (Settings)      │  │                 │                      │
│  └─────────────────┘  └─────────────────┘                      │
├─────────────────────────────────────────────────────────────────┤
│                      rustconn-cli (Binary)                      │
├─────────────────────────────────────────────────────────────────┤
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                    CLI Commands                          │   │
│  │  list | connect | add | export | import | test | help   │   │
│  └─────────────────────────────────────────────────────────┘   │
├─────────────────────────────────────────────────────────────────┤
│                      rustconn-core                              │
├─────────────────────────────────────────────────────────────────┤
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                    SSH Agent Module                      │   │
│  │  ┌───────────────┐ ┌───────────────┐ ┌───────────────┐ │   │
│  │  │ Agent Manager │ │ Key Parser    │ │ Agent Status  │ │   │
│  │  └───────────────┘ └───────────────┘ └───────────────┘ │   │
│  └─────────────────────────────────────────────────────────┘   │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                    Export Module                         │   │
│  │  ┌───────────────┐ ┌───────────────┐ ┌───────────────┐ │   │
│  │  │ Ansible Export│ │ SSH Config    │ │ Remmina Export│ │   │
│  │  └───────────────┘ └───────────────┘ └───────────────┘ │   │
│  │  ┌───────────────┐                                      │   │
│  │  │ Asbru Export  │                                      │   │
│  │  └───────────────┘                                      │   │
│  └─────────────────────────────────────────────────────────┘   │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                    Connection Testing                    │   │
│  │  ┌───────────────┐ ┌───────────────┐                    │   │
│  │  │ Port Checker  │ │ SSH Handshake │                    │   │
│  │  └───────────────┘ └───────────────┘                    │   │
│  └─────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘
```

## Components and Interfaces

### 1. SSH Agent Module (`rustconn-core/src/ssh_agent/`)

```rust
/// SSH Agent status and key information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStatus {
    pub running: bool,
    pub socket_path: Option<String>,
    pub keys: Vec<AgentKey>,
}

/// A key loaded in the SSH agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentKey {
    pub fingerprint: String,
    pub bits: u32,
    pub key_type: String,
    pub comment: String,
}

/// SSH Agent manager for interacting with ssh-agent
pub struct SshAgentManager {
    socket_path: Option<String>,
}

impl SshAgentManager {
    /// Start a new SSH agent and return the socket path
    pub fn start_agent() -> Result<String, AgentError>;
    
    /// Parse ssh-agent -s output to extract socket path
    pub fn parse_agent_output(output: &str) -> Result<String, AgentError>;
    
    /// Get current agent status
    pub fn get_status(&self) -> Result<AgentStatus, AgentError>;
    
    /// Parse ssh-add -l output to get loaded keys
    pub fn parse_key_list(output: &str) -> Result<Vec<AgentKey>, AgentError>;
    
    /// Add a key to the agent
    pub fn add_key(&self, key_path: &Path, passphrase: Option<&str>) -> Result<(), AgentError>;
    
    /// Remove a key from the agent
    pub fn remove_key(&self, key_path: &Path) -> Result<(), AgentError>;
    
    /// List available key files in ~/.ssh/
    pub fn list_key_files() -> Result<Vec<PathBuf>, AgentError>;
}

/// Key source for SSH connections
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SshKeySource {
    /// Key from file path
    File(PathBuf),
    /// Key from SSH agent (identified by fingerprint)
    Agent { fingerprint: String, comment: String },
    /// No specific key (use default)
    Default,
}
```

### 2. Export Module (`rustconn-core/src/export/`)

```rust
/// Export format types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Ansible,
    SshConfig,
    Remmina,
    Asbru,
}

/// Export options
#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub format: ExportFormat,
    pub include_passwords: bool,
    pub include_groups: bool,
    pub output_path: PathBuf,
}

/// Export result
#[derive(Debug)]
pub struct ExportResult {
    pub exported_count: usize,
    pub skipped_count: usize,
    pub warnings: Vec<String>,
    pub output_files: Vec<PathBuf>,
}

/// Trait for export implementations
pub trait ExportTarget: Send + Sync {
    /// Export format identifier
    fn format_id(&self) -> ExportFormat;
    
    /// Export connections to the target format
    fn export(
        &self,
        connections: &[Connection],
        groups: &[ConnectionGroup],
        options: &ExportOptions,
    ) -> Result<ExportResult, ExportError>;
    
    /// Export a single connection to string
    fn export_connection(&self, connection: &Connection) -> Result<String, ExportError>;
}
```

### 3. Ansible Exporter (`rustconn-core/src/export/ansible.rs`)

```rust
/// Ansible inventory exporter
pub struct AnsibleExporter;

impl AnsibleExporter {
    /// Export connections to INI format inventory
    pub fn export_ini(
        connections: &[Connection],
        groups: &[ConnectionGroup],
    ) -> Result<String, ExportError>;
    
    /// Export connections to YAML format inventory
    pub fn export_yaml(
        connections: &[Connection],
        groups: &[ConnectionGroup],
    ) -> Result<String, ExportError>;
    
    /// Format a single host entry
    fn format_host_entry(connection: &Connection) -> String;
}
```

### 4. SSH Config Exporter (`rustconn-core/src/export/ssh_config.rs`)

```rust
/// SSH config file exporter
pub struct SshConfigExporter;

impl SshConfigExporter {
    /// Export connections to SSH config format
    pub fn export(connections: &[Connection]) -> Result<String, ExportError>;
    
    /// Format a single Host entry
    fn format_host_entry(connection: &Connection) -> String;
    
    /// Escape special characters in values
    fn escape_value(value: &str) -> String;
}
```

### 5. Remmina Exporter (`rustconn-core/src/export/remmina.rs`)

```rust
/// Remmina connection file exporter
pub struct RemminaExporter;

impl RemminaExporter {
    /// Export a connection to .remmina file content
    pub fn export_connection(connection: &Connection) -> Result<String, ExportError>;
    
    /// Generate filename for a connection
    pub fn generate_filename(connection: &Connection) -> String;
    
    /// Export all connections to a directory
    pub fn export_to_directory(
        connections: &[Connection],
        output_dir: &Path,
    ) -> Result<ExportResult, ExportError>;
}
```

### 6. Asbru Exporter (`rustconn-core/src/export/asbru.rs`)

```rust
/// Asbru-CM YAML exporter
pub struct AsbruExporter;

impl AsbruExporter {
    /// Export connections to Asbru YAML format
    pub fn export(
        connections: &[Connection],
        groups: &[ConnectionGroup],
    ) -> Result<String, ExportError>;
    
    /// Convert connection to Asbru entry
    fn connection_to_entry(connection: &Connection) -> serde_yaml::Value;
    
    /// Convert group to Asbru entry
    fn group_to_entry(group: &ConnectionGroup) -> serde_yaml::Value;
}
```

### 7. CLI Module (`rustconn-cli/src/`)

```rust
/// CLI application structure
#[derive(Parser)]
#[command(name = "rustconn-cli")]
#[command(about = "RustConn command-line interface")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
    
    /// Config file path
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// List all connections
    List {
        /// Output format (table, json, csv)
        #[arg(short, long, default_value = "table")]
        format: OutputFormat,
        
        /// Filter by protocol
        #[arg(short, long)]
        protocol: Option<String>,
    },
    
    /// Connect to a server
    Connect {
        /// Connection name or ID
        name: String,
    },
    
    /// Add a new connection
    Add {
        /// Connection name
        #[arg(short, long)]
        name: String,
        
        /// Host address
        #[arg(short = 'H', long)]
        host: String,
        
        /// Port number
        #[arg(short, long)]
        port: Option<u16>,
        
        /// Protocol (ssh, rdp, vnc)
        #[arg(short = 'P', long, default_value = "ssh")]
        protocol: String,
        
        /// Username
        #[arg(short, long)]
        user: Option<String>,
        
        /// SSH key path
        #[arg(short, long)]
        key: Option<PathBuf>,
    },
    
    /// Export connections
    Export {
        /// Export format (ansible, ssh-config, remmina, asbru)
        #[arg(short, long)]
        format: ExportFormat,
        
        /// Output file or directory
        #[arg(short, long)]
        output: PathBuf,
    },
    
    /// Import connections
    Import {
        /// Import format (ansible, ssh-config, remmina, asbru)
        #[arg(short, long)]
        format: ImportFormat,
        
        /// Input file
        file: PathBuf,
    },
    
    /// Test connection
    Test {
        /// Connection name or ID (or "all" for batch test)
        name: String,
        
        /// Timeout in seconds
        #[arg(short, long, default_value = "10")]
        timeout: u64,
    },
}

/// Output format for list command
#[derive(Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
    Csv,
}
```

### 8. Connection Tester (`rustconn-core/src/testing/`)

```rust
/// Connection test result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub connection_id: Uuid,
    pub connection_name: String,
    pub success: bool,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
    pub details: HashMap<String, String>,
}

/// Batch test summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub results: Vec<TestResult>,
}

/// Connection tester
pub struct ConnectionTester {
    timeout: Duration,
}

impl ConnectionTester {
    /// Test a single connection
    pub async fn test_connection(&self, connection: &Connection) -> TestResult;
    
    /// Test multiple connections
    pub async fn test_batch(&self, connections: &[Connection]) -> TestSummary;
    
    /// Test TCP port connectivity
    async fn test_port(&self, host: &str, port: u16) -> Result<Duration, TestError>;
    
    /// Test SSH handshake
    async fn test_ssh(&self, connection: &Connection) -> Result<(), TestError>;
}
```

## Data Models

### Extended SSH Config

```rust
/// Extended SSH configuration with agent key support
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshConfig {
    pub auth_method: SshAuthMethod,
    pub key_path: Option<PathBuf>,
    /// Key source (file, agent, or default)
    pub key_source: SshKeySource,
    /// Agent key fingerprint (when using agent)
    pub agent_key_fingerprint: Option<String>,
    pub proxy_jump: Option<String>,
    pub use_control_master: bool,
    pub custom_options: HashMap<String, String>,
    pub startup_command: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SshAuthMethod {
    Password,
    PublicKey,
    Agent,
    KeyboardInteractive,
}
```

## Correctness Properties

*A property is a characteristic or behavior that should hold true across all valid executions of a system-essentially, a formal statement about what the system should do. Properties serve as the bridge between human-readable specifications and machine-verifiable correctness guarantees.*

### Property 1: SSH Agent Output Parsing
*For any* valid ssh-agent -s output string, parsing should extract the SSH_AUTH_SOCK path correctly.
**Validates: Requirements 1.2**

### Property 2: SSH Key List Parsing
*For any* valid ssh-add -l output string, parsing should extract all key fingerprints, types, and comments.
**Validates: Requirements 1.6**

### Property 3: Agent Key Fingerprint Storage
*For any* SSH connection configured with an agent key, the fingerprint should be stored and retrievable for identification.
**Validates: Requirements 2.3**

### Property 4: Ansible Export Round-Trip
*For any* set of SSH connections, exporting to Ansible inventory format and re-importing should preserve hostname, port, username, and key path.
**Validates: Requirements 3.6, 9.5**

### Property 5: Ansible Export Completeness
*For any* SSH connection with non-default port, the exported Ansible inventory should include ansible_host, ansible_user, ansible_port, and ansible_ssh_private_key_file when applicable.
**Validates: Requirements 3.2, 3.3, 3.4**

### Property 6: SSH Config Export Round-Trip
*For any* set of SSH connections, exporting to SSH config format and re-importing should preserve hostname, port, username, and identity file.
**Validates: Requirements 4.6, 9.5**

### Property 7: SSH Config Export Completeness
*For any* SSH connection, the exported SSH config should include Host (from name), HostName, User, Port, and IdentityFile directives when applicable.
**Validates: Requirements 4.2, 4.3, 4.4**

### Property 8: Remmina Export Round-Trip
*For any* connection (SSH, RDP, or VNC), exporting to Remmina format and re-importing should preserve protocol type, hostname, port, and username.
**Validates: Requirements 5.6, 9.5**

### Property 9: Remmina Export Protocol Handling
*For any* connection, the exported Remmina file should contain the correct protocol-specific fields (SSH: ssh_privatekey; RDP: domain; VNC: port).
**Validates: Requirements 5.2, 5.3, 5.4**

### Property 10: Asbru Export Round-Trip
*For any* set of connections with groups, exporting to Asbru format and re-importing should preserve connection properties and group hierarchy.
**Validates: Requirements 6.5, 9.5**

### Property 11: Asbru Export Group Hierarchy
*For any* set of connections organized in groups, the exported Asbru YAML should preserve the parent-child relationships.
**Validates: Requirements 6.2, 6.3**

### Property 12: CLI List Output Completeness
*For any* set of connections, the CLI list command should output all connections with their name, host, port, and protocol.
**Validates: Requirements 7.1**

### Property 13: CLI Add Connection
*For any* valid connection parameters, the CLI add command should create a connection with the specified name, host, port, protocol, and username.
**Validates: Requirements 7.3**

### Property 14: CLI Export Format Selection
*For any* export format (ansible, ssh-config, remmina, asbru), the CLI export command should produce output in the correct format.
**Validates: Requirements 7.4**

### Property 15: CLI Error Exit Codes
*For any* CLI command that fails, the exit code should be non-zero and an error message should be displayed.
**Validates: Requirements 7.7**

### Property 16: Test Result Error Details
*For any* failed connection test, the result should include a descriptive error message explaining the failure.
**Validates: Requirements 8.5**

### Property 17: Batch Test Summary Accuracy
*For any* batch test of N connections, the summary should report exactly N total tests with passed + failed = N.
**Validates: Requirements 8.6**

### Property 18: Import Field Preservation
*For any* imported connection, the hostname, port, username, and authentication details should match the source data.
**Validates: Requirements 9.1, 9.2, 9.3**

## Error Handling

```rust
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("SSH agent not running")]
    NotRunning,
    #[error("Failed to start agent: {0}")]
    StartFailed(String),
    #[error("Failed to parse agent output: {0}")]
    ParseError(String),
    #[error("Key not found: {0}")]
    KeyNotFound(String),
    #[error("Failed to add key: {0}")]
    AddKeyFailed(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("Unsupported protocol for export: {0}")]
    UnsupportedProtocol(String),
    #[error("Failed to write output: {0}")]
    WriteError(String),
    #[error("Invalid connection data: {0}")]
    InvalidData(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum TestError {
    #[error("Connection timeout")]
    Timeout,
    #[error("Connection refused")]
    ConnectionRefused,
    #[error("Host unreachable: {0}")]
    HostUnreachable(String),
    #[error("Authentication failed")]
    AuthFailed,
    #[error("Protocol error: {0}")]
    ProtocolError(String),
}
```

## Testing Strategy

### Dual Testing Approach

1. **Unit Tests**: Verify specific examples and edge cases
2. **Property-Based Tests**: Verify universal properties using `proptest`

### Property-Based Testing Framework

Using `proptest` crate:

```rust
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]
    
    /// **Feature: ssh-agent-cli, Property 4: Ansible Export Round-Trip**
    /// **Validates: Requirements 3.6, 9.5**
    #[test]
    fn test_ansible_export_roundtrip(
        name in "[a-z][a-z0-9_-]{0,20}",
        host in "[a-z0-9.-]{1,50}",
        port in 1u16..65535u16,
        user in prop::option::of("[a-z][a-z0-9_]{0,15}"),
    ) {
        let connection = Connection::new_ssh(name.clone(), host.clone(), port)
            .with_username(user.clone().unwrap_or_default());
        
        let exported = AnsibleExporter::export_ini(&[connection.clone()], &[]).unwrap();
        let imported = AnsibleInventoryImporter::new()
            .parse_ini_inventory(&exported, "test");
        
        prop_assert_eq!(imported.connections.len(), 1);
        let reimported = &imported.connections[0];
        prop_assert_eq!(reimported.host, host);
        prop_assert_eq!(reimported.port, port);
    }
}
```

### Test Organization

- Property tests in `rustconn-core/tests/properties/`
- Unit tests co-located with source files
- CLI integration tests in `rustconn-cli/tests/`

### Test Data Files

Create test data files for import/export testing:
- `tests/fixtures/ansible_inventory.ini`
- `tests/fixtures/ssh_config`
- `tests/fixtures/remmina/test.remmina`
- `tests/fixtures/asbru.yml`
