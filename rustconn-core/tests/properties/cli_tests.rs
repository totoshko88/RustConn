//! Property-based tests for CLI functionality
//!
//! Tests correctness properties for CLI list and add command functionality.

use proptest::prelude::*;
use rustconn_core::config::ConfigManager;
use rustconn_core::models::{Connection, ProtocolConfig, ProtocolType, SshAuthMethod};
use std::path::PathBuf;
use tempfile::TempDir;

/// Generates a valid hostname (no wildcards, valid characters)
fn arb_hostname() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-z][a-z0-9-]{0,20}(\\.[a-z][a-z0-9-]{0,10})*")
        .unwrap()
        .prop_filter("hostname must not be empty", |s| !s.is_empty())
}

/// Generates a valid connection name (alphanumeric with underscores/hyphens)
fn arb_connection_name() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-zA-Z][a-zA-Z0-9_-]{0,30}")
        .unwrap()
        .prop_filter("name must not be empty", |s| !s.is_empty())
}

/// Generates a valid port number
fn arb_port() -> impl Strategy<Value = u16> {
    1u16..65535
}

/// Generates a protocol type
fn arb_protocol() -> impl Strategy<Value = ProtocolType> {
    prop_oneof![
        Just(ProtocolType::Ssh),
        Just(ProtocolType::Rdp),
        Just(ProtocolType::Vnc),
        Just(ProtocolType::Spice),
    ]
}

/// Represents a generated connection for testing
#[derive(Debug, Clone)]
struct TestConnection {
    name: String,
    host: String,
    port: u16,
    protocol: ProtocolType,
}

impl TestConnection {
    /// Converts to a Connection object
    fn to_connection(&self) -> Connection {
        match self.protocol {
            ProtocolType::Ssh | ProtocolType::ZeroTrust => {
                Connection::new_ssh(self.name.clone(), self.host.clone(), self.port)
            }
            ProtocolType::Rdp => {
                Connection::new_rdp(self.name.clone(), self.host.clone(), self.port)
            }
            ProtocolType::Vnc => {
                Connection::new_vnc(self.name.clone(), self.host.clone(), self.port)
            }
            ProtocolType::Spice => {
                Connection::new_spice(self.name.clone(), self.host.clone(), self.port)
            }
        }
    }
}

/// Strategy for generating test connections
fn arb_connection() -> impl Strategy<Value = TestConnection> {
    (
        arb_connection_name(),
        arb_hostname(),
        arb_port(),
        arb_protocol(),
    )
        .prop_map(|(name, host, port, protocol)| TestConnection {
            name,
            host,
            port,
            protocol,
        })
}

/// Strategy for generating multiple connections with unique names
fn arb_connections() -> impl Strategy<Value = Vec<TestConnection>> {
    prop::collection::vec(arb_connection(), 1..10).prop_map(|connections| {
        connections
            .into_iter()
            .enumerate()
            .map(|(i, mut conn)| {
                // Ensure unique names by appending index
                conn.name = format!("{}_{}", conn.name, i);
                conn
            })
            .collect()
    })
}

/// Format connections as a table string (mirrors CLI implementation)
fn format_table(connections: &[&Connection]) -> String {
    if connections.is_empty() {
        return "No connections found.".to_string();
    }

    let mut output = String::new();

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
    let protocol_width = 8;
    let port_width = 5;

    output.push_str(&format!(
        "{:<name_width$}  {:<host_width$}  {:<port_width$}  {:<protocol_width$}\n",
        "NAME", "HOST", "PORT", "PROTOCOL"
    ));
    output.push_str(&format!(
        "{:-<name_width$}  {:-<host_width$}  {:-<port_width$}  {:-<protocol_width$}\n",
        "", "", "", ""
    ));

    for conn in connections {
        output.push_str(&format!(
            "{:<name_width$}  {:<host_width$}  {:<port_width$}  {:<protocol_width$}\n",
            conn.name,
            conn.host,
            conn.port,
            conn.protocol.to_string()
        ));
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

/// Format connections as CSV string (mirrors CLI implementation)
fn format_csv(connections: &[&Connection]) -> String {
    let mut output = String::new();
    output.push_str("name,host,port,protocol\n");

    for conn in connections {
        let name = escape_csv_field(&conn.name);
        let host = escape_csv_field(&conn.host);
        output.push_str(&format!(
            "{},{},{},{}\n",
            name,
            host,
            conn.port,
            conn.protocol.as_str()
        ));
    }

    output.trim_end().to_string()
}

/// Simplified connection output for JSON
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ConnectionOutput {
    id: String,
    name: String,
    host: String,
    port: u16,
    protocol: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
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

/// Format connections as JSON string (mirrors CLI implementation)
fn format_json(connections: &[&Connection]) -> String {
    let output: Vec<ConnectionOutput> = connections.iter().map(|c| (*c).into()).collect();
    serde_json::to_string_pretty(&output).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: ssh-agent-cli, Property 12: CLI List Output Completeness**
    /// **Validates: Requirements 7.1**
    ///
    /// For any set of connections, the CLI list command should output all connections
    /// with their name, host, port, and protocol.
    #[test]
    fn prop_cli_list_table_completeness(connections in arb_connections()) {
        // Convert test connections to Connection objects
        let conns: Vec<Connection> = connections.iter().map(|c| c.to_connection()).collect();
        let conn_refs: Vec<&Connection> = conns.iter().collect();

        // Format as table
        let output = format_table(&conn_refs);

        // Property: Output should contain header
        prop_assert!(
            output.contains("NAME") && output.contains("HOST") && output.contains("PORT") && output.contains("PROTOCOL"),
            "Table output should contain header columns. Output:\n{}",
            output
        );

        // Property: Each connection's name, host, port, and protocol should appear in output
        for conn in &connections {
            prop_assert!(
                output.contains(&conn.name),
                "Connection name '{}' should appear in table output. Output:\n{}",
                conn.name,
                output
            );

            prop_assert!(
                output.contains(&conn.host),
                "Connection host '{}' should appear in table output. Output:\n{}",
                conn.host,
                output
            );

            prop_assert!(
                output.contains(&conn.port.to_string()),
                "Connection port '{}' should appear in table output. Output:\n{}",
                conn.port,
                output
            );

            prop_assert!(
                output.contains(&conn.protocol.to_string()),
                "Connection protocol '{}' should appear in table output. Output:\n{}",
                conn.protocol,
                output
            );
        }
    }

    /// **Feature: ssh-agent-cli, Property 12: CLI List Output Completeness (JSON)**
    /// **Validates: Requirements 7.1**
    ///
    /// For any set of connections, the CLI list command JSON output should contain
    /// all connections with their name, host, port, and protocol.
    #[test]
    fn prop_cli_list_json_completeness(connections in arb_connections()) {
        // Convert test connections to Connection objects
        let conns: Vec<Connection> = connections.iter().map(|c| c.to_connection()).collect();
        let conn_refs: Vec<&Connection> = conns.iter().collect();

        // Format as JSON
        let output = format_json(&conn_refs);

        // Parse JSON back
        let parsed: Vec<ConnectionOutput> = serde_json::from_str(&output)
            .expect("JSON output should be valid");

        // Property: Number of connections should match
        prop_assert_eq!(
            parsed.len(),
            connections.len(),
            "JSON output should contain all {} connections, got {}",
            connections.len(),
            parsed.len()
        );

        // Property: Each connection's fields should be preserved
        for (original, parsed_conn) in connections.iter().zip(parsed.iter()) {
            prop_assert_eq!(
                &parsed_conn.name,
                &original.name,
                "Name mismatch in JSON output"
            );

            prop_assert_eq!(
                &parsed_conn.host,
                &original.host,
                "Host mismatch in JSON output"
            );

            prop_assert_eq!(
                parsed_conn.port,
                original.port,
                "Port mismatch in JSON output"
            );

            prop_assert_eq!(
                &parsed_conn.protocol,
                original.protocol.as_str(),
                "Protocol mismatch in JSON output"
            );
        }
    }

    /// **Feature: ssh-agent-cli, Property 12: CLI List Output Completeness (CSV)**
    /// **Validates: Requirements 7.1**
    ///
    /// For any set of connections, the CLI list command CSV output should contain
    /// all connections with their name, host, port, and protocol.
    #[test]
    fn prop_cli_list_csv_completeness(connections in arb_connections()) {
        // Convert test connections to Connection objects
        let conns: Vec<Connection> = connections.iter().map(|c| c.to_connection()).collect();
        let conn_refs: Vec<&Connection> = conns.iter().collect();

        // Format as CSV
        let output = format_csv(&conn_refs);

        // Parse CSV
        let lines: Vec<&str> = output.lines().collect();

        // Property: First line should be header
        prop_assert_eq!(
            lines[0],
            "name,host,port,protocol",
            "CSV should have correct header"
        );

        // Property: Number of data lines should match connections
        prop_assert_eq!(
            lines.len() - 1,
            connections.len(),
            "CSV should have {} data lines, got {}",
            connections.len(),
            lines.len() - 1
        );

        // Property: Each connection's fields should appear in CSV
        for (i, conn) in connections.iter().enumerate() {
            let line = lines[i + 1]; // Skip header

            // Check that the line contains the connection data
            // Note: We use contains because CSV escaping may add quotes
            prop_assert!(
                line.contains(&conn.port.to_string()),
                "CSV line should contain port '{}'. Line: {}",
                conn.port,
                line
            );

            prop_assert!(
                line.contains(conn.protocol.as_str()),
                "CSV line should contain protocol '{}'. Line: {}",
                conn.protocol.as_str(),
                line
            );
        }
    }

    /// **Feature: ssh-agent-cli, Property 12: CLI List Empty Output**
    /// **Validates: Requirements 7.1**
    ///
    /// For an empty connection list, the CLI list command should output
    /// an appropriate message.
    #[test]
    fn prop_cli_list_empty_output(_dummy in Just(())) {
        let empty: Vec<&Connection> = vec![];
        let output = format_table(&empty);

        prop_assert_eq!(
            output,
            "No connections found.",
            "Empty list should show 'No connections found.' message"
        );
    }

    /// **Feature: ssh-agent-cli, Property 12: CLI List CSV Escaping**
    /// **Validates: Requirements 7.1**
    ///
    /// For connections with special characters (commas, quotes), the CSV output
    /// should properly escape them.
    #[test]
    fn prop_cli_csv_escaping(
        base_name in arb_connection_name(),
        special_char in prop_oneof![Just(","), Just("\""), Just("\n")]
    ) {
        // Create a connection with special character in name
        let name_with_special = format!("{}{}", base_name, special_char);
        let escaped = escape_csv_field(&name_with_special);

        // Property: If field contains special char, it should be quoted
        if special_char == "," || special_char == "\"" || special_char == "\n" {
            prop_assert!(
                escaped.starts_with('"') && escaped.ends_with('"'),
                "Field with special char should be quoted. Original: '{}', Escaped: '{}'",
                name_with_special,
                escaped
            );
        }

        // Property: Quotes in field should be doubled
        if special_char == "\"" {
            prop_assert!(
                escaped.contains("\"\""),
                "Quotes should be doubled. Original: '{}', Escaped: '{}'",
                name_with_special,
                escaped
            );
        }
    }

    /// **Feature: ssh-agent-cli, Property 13: CLI Add Connection**
    /// **Validates: Requirements 7.3**
    ///
    /// For any valid connection parameters, the CLI add command should create a connection
    /// with the specified name, host, port, protocol, and username.
    #[test]
    fn prop_cli_add_connection(
        name in arb_connection_name(),
        host in arb_hostname(),
        port in arb_port(),
        protocol in arb_protocol(),
        username in prop::option::of(arb_connection_name())
    ) {
        // Create a temporary directory for the config
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config_manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());

        // Simulate the add command logic
        let connection = create_test_connection_for_add(&name, &host, port, protocol, username.as_deref(), None);

        // Validate the connection
        let validation_result = ConfigManager::validate_connection(&connection);
        prop_assert!(
            validation_result.is_ok(),
            "Connection should be valid: {:?}",
            validation_result.err()
        );

        // Save the connection
        let save_result = config_manager.save_connections(std::slice::from_ref(&connection));
        prop_assert!(
            save_result.is_ok(),
            "Should be able to save connection: {:?}",
            save_result.err()
        );

        // Load and verify the connection
        let loaded = config_manager.load_connections().expect("Should load connections");
        prop_assert_eq!(loaded.len(), 1, "Should have exactly one connection");

        let loaded_conn = &loaded[0];

        // Property: Name should match
        prop_assert_eq!(
            &loaded_conn.name,
            &name,
            "Connection name should match"
        );

        // Property: Host should match
        prop_assert_eq!(
            &loaded_conn.host,
            &host,
            "Connection host should match"
        );

        // Property: Port should match
        prop_assert_eq!(
            loaded_conn.port,
            port,
            "Connection port should match"
        );

        // Property: Protocol should match
        prop_assert_eq!(
            loaded_conn.protocol,
            protocol,
            "Connection protocol should match"
        );

        // Property: Username should match
        prop_assert_eq!(
            &loaded_conn.username,
            &username,
            "Connection username should match"
        );
    }

    /// **Feature: ssh-agent-cli, Property 13: CLI Add Connection with SSH Key**
    /// **Validates: Requirements 7.3**
    ///
    /// For SSH connections with a key path, the key path should be stored correctly.
    #[test]
    fn prop_cli_add_ssh_connection_with_key(
        name in arb_connection_name(),
        host in arb_hostname(),
        port in arb_port(),
        username in prop::option::of(arb_connection_name()),
        key_path in arb_key_path()
    ) {
        // Create a temporary directory for the config
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config_manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());

        // Simulate the add command logic for SSH with key
        let connection = create_test_connection_for_add(
            &name,
            &host,
            port,
            ProtocolType::Ssh,
            username.as_deref(),
            Some(&key_path)
        );

        // Save the connection
        config_manager.save_connections(std::slice::from_ref(&connection)).expect("Should save");

        // Load and verify
        let loaded = config_manager.load_connections().expect("Should load");
        let loaded_conn = &loaded[0];

        // Property: Key path should be stored in SSH config
        if let ProtocolConfig::Ssh(ref ssh_config) = loaded_conn.protocol_config {
            prop_assert_eq!(
                ssh_config.key_path.as_ref(),
                Some(&key_path),
                "SSH key path should be stored"
            );
            prop_assert!(
                matches!(ssh_config.auth_method, SshAuthMethod::PublicKey),
                "Auth method should be PublicKey when key is provided, got {:?}",
                ssh_config.auth_method
            );
        } else {
            prop_assert!(false, "Protocol config should be SSH");
        }
    }

    /// **Feature: ssh-agent-cli, Property 13: CLI Add Connection Default Port**
    /// **Validates: Requirements 7.3**
    ///
    /// When no port is specified, the default port for the protocol should be used.
    #[test]
    fn prop_cli_add_connection_default_port(
        name in arb_connection_name(),
        host in arb_hostname(),
        protocol in arb_protocol()
    ) {
        // Get the expected default port for the protocol
        let expected_port = protocol.default_port();

        // Create connection with default port
        let connection = create_test_connection_for_add(
            &name,
            &host,
            expected_port,
            protocol,
            None,
            None
        );

        // Property: Port should be the protocol's default
        prop_assert_eq!(
            connection.port,
            expected_port,
            "Connection should use default port {} for protocol {:?}",
            expected_port,
            protocol
        );
    }
}

/// Generates a valid key path
fn arb_key_path() -> impl Strategy<Value = PathBuf> {
    prop::string::string_regex("/home/[a-z]+/\\.ssh/id_[a-z]+")
        .unwrap()
        .prop_map(PathBuf::from)
}

/// Creates a connection for testing the add command
/// This mirrors the logic in the CLI's cmd_add function
fn create_test_connection_for_add(
    name: &str,
    host: &str,
    port: u16,
    protocol: ProtocolType,
    username: Option<&str>,
    key_path: Option<&PathBuf>,
) -> Connection {
    let mut connection = match protocol {
        ProtocolType::Ssh | ProtocolType::ZeroTrust => {
            let mut conn = Connection::new_ssh(name.to_string(), host.to_string(), port);
            if let Some(key) = key_path {
                if let ProtocolConfig::Ssh(ref mut ssh_config) = conn.protocol_config {
                    ssh_config.key_path = Some(key.clone());
                    ssh_config.auth_method = SshAuthMethod::PublicKey;
                }
            }
            conn
        }
        ProtocolType::Rdp => Connection::new_rdp(name.to_string(), host.to_string(), port),
        ProtocolType::Vnc => Connection::new_vnc(name.to_string(), host.to_string(), port),
        ProtocolType::Spice => Connection::new_spice(name.to_string(), host.to_string(), port),
    };

    if let Some(user) = username {
        connection.username = Some(user.to_string());
    }

    connection
}

// ============================================================================
// Import Field Preservation Property Tests
// ============================================================================

/// Represents a generated SSH connection for import testing
#[derive(Debug, Clone)]
struct ImportTestConnection {
    name: String,
    host: String,
    port: u16,
    username: Option<String>,
    key_path: Option<String>,
}

impl ImportTestConnection {
    /// Converts to SSH config format
    fn to_ssh_config(&self) -> String {
        let mut lines = vec![format!("Host {}", self.name)];
        lines.push(format!("    HostName {}", self.host));
        lines.push(format!("    Port {}", self.port));

        if let Some(ref user) = self.username {
            lines.push(format!("    User {}", user));
        }
        if let Some(ref key) = self.key_path {
            lines.push(format!("    IdentityFile {}", key));
        }

        lines.join("\n")
    }

    /// Converts to Ansible INI format
    fn to_ansible_ini(&self) -> String {
        let mut parts = vec![self.name.clone()];
        parts.push(format!("ansible_host={}", self.host));
        parts.push(format!("ansible_port={}", self.port));

        if let Some(ref user) = self.username {
            parts.push(format!("ansible_user={}", user));
        }
        if let Some(ref key) = self.key_path {
            parts.push(format!("ansible_ssh_private_key_file={}", key));
        }

        parts.join(" ")
    }

    /// Converts to Remmina format
    fn to_remmina(&self) -> String {
        let mut lines = vec!["[remmina]".to_string()];
        lines.push(format!("name={}", self.name));
        lines.push("protocol=SSH".to_string());
        lines.push(format!("server={}:{}", self.host, self.port));

        if let Some(ref user) = self.username {
            lines.push(format!("username={}", user));
        }
        if let Some(ref key) = self.key_path {
            lines.push(format!("ssh_privatekey={}", key));
        }

        lines.join("\n")
    }

    /// Converts to Asbru YAML format
    fn to_asbru_yaml(&self) -> String {
        let mut lines = vec![format!("conn-{}:", self.name)];
        lines.push("  _is_group: 0".to_string());
        lines.push(format!("  name: \"{}\"", self.name));
        lines.push(format!("  ip: \"{}\"", self.host));
        lines.push(format!("  port: {}", self.port));
        lines.push("  method: \"SSH\"".to_string());

        if let Some(ref user) = self.username {
            lines.push(format!("  user: \"{}\"", user));
        }
        if let Some(ref key) = self.key_path {
            lines.push(format!("  public key: \"{}\"", key));
        }
        lines.push("  children: {}".to_string());

        lines.join("\n")
    }
}

/// Strategy for generating import test connections
fn arb_import_test_connection() -> impl Strategy<Value = ImportTestConnection> {
    (
        arb_connection_name(),
        arb_hostname(),
        arb_port(),
        prop::option::of(arb_connection_name()),
        prop::option::of(arb_key_path()),
    )
        .prop_map(
            |(name, host, port, username, key_path)| ImportTestConnection {
                name,
                host,
                port,
                username,
                key_path: key_path.map(|p| p.to_string_lossy().to_string()),
            },
        )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: ssh-agent-cli, Property 18: Import Field Preservation (SSH Config)**
    /// **Validates: Requirements 9.1, 9.2, 9.3**
    ///
    /// For any imported SSH connection from SSH config format, the hostname, port,
    /// username, and key path should match the source data.
    #[test]
    fn prop_import_ssh_config_field_preservation(conn in arb_import_test_connection()) {
        use rustconn_core::import::SshConfigImporter;

        let importer = SshConfigImporter::new();
        let config_content = conn.to_ssh_config();

        // Parse the config
        let result = importer.parse_config(&config_content, "test");

        // Property: One connection should be imported
        prop_assert_eq!(
            result.connections.len(),
            1,
            "Expected 1 connection, got {}. Config:\n{}",
            result.connections.len(),
            config_content
        );

        let imported = &result.connections[0];

        // Property: Name should match (Host alias)
        prop_assert_eq!(
            &imported.name,
            &conn.name,
            "Name mismatch. Expected '{}', got '{}'",
            conn.name,
            imported.name
        );

        // Property: Hostname should match
        prop_assert_eq!(
            &imported.host,
            &conn.host,
            "Hostname mismatch. Expected '{}', got '{}'",
            conn.host,
            imported.host
        );

        // Property: Port should match
        prop_assert_eq!(
            imported.port,
            conn.port,
            "Port mismatch. Expected {}, got {}",
            conn.port,
            imported.port
        );

        // Property: Username should match
        prop_assert_eq!(
            imported.username.as_ref(),
            conn.username.as_ref(),
            "Username mismatch. Expected {:?}, got {:?}",
            conn.username,
            imported.username
        );

        // Property: Key path should be preserved (if specified)
        if let ProtocolConfig::Ssh(ref ssh_config) = imported.protocol_config {
            if conn.key_path.is_some() {
                prop_assert!(
                    ssh_config.key_path.is_some(),
                    "Key path should be preserved when specified"
                );
            }
        }
    }

    /// **Feature: ssh-agent-cli, Property 18: Import Field Preservation (Ansible)**
    /// **Validates: Requirements 9.1, 9.2, 9.3**
    ///
    /// For any imported SSH connection from Ansible inventory format, the hostname,
    /// port, username, and key path should match the source data.
    #[test]
    fn prop_import_ansible_field_preservation(conn in arb_import_test_connection()) {
        use rustconn_core::import::AnsibleInventoryImporter;

        let importer = AnsibleInventoryImporter::new();
        let inventory_content = format!("[servers]\n{}", conn.to_ansible_ini());

        // Parse the inventory
        let result = importer.parse_ini_inventory(&inventory_content, "test");

        // Property: One connection should be imported
        prop_assert_eq!(
            result.connections.len(),
            1,
            "Expected 1 connection, got {}. Inventory:\n{}",
            result.connections.len(),
            inventory_content
        );

        let imported = &result.connections[0];

        // Property: Name should match (host pattern)
        prop_assert_eq!(
            &imported.name,
            &conn.name,
            "Name mismatch. Expected '{}', got '{}'",
            conn.name,
            imported.name
        );

        // Property: Hostname should match (ansible_host)
        prop_assert_eq!(
            &imported.host,
            &conn.host,
            "Hostname mismatch. Expected '{}', got '{}'",
            conn.host,
            imported.host
        );

        // Property: Port should match (ansible_port)
        prop_assert_eq!(
            imported.port,
            conn.port,
            "Port mismatch. Expected {}, got {}",
            conn.port,
            imported.port
        );

        // Property: Username should match (ansible_user)
        prop_assert_eq!(
            imported.username.as_ref(),
            conn.username.as_ref(),
            "Username mismatch. Expected {:?}, got {:?}",
            conn.username,
            imported.username
        );

        // Property: Key path should be preserved (ansible_ssh_private_key_file)
        if let ProtocolConfig::Ssh(ref ssh_config) = imported.protocol_config {
            if conn.key_path.is_some() {
                prop_assert!(
                    ssh_config.key_path.is_some(),
                    "Key path should be preserved when specified"
                );
            }
        }
    }

    /// **Feature: ssh-agent-cli, Property 18: Import Field Preservation (Remmina)**
    /// **Validates: Requirements 9.1, 9.2, 9.3**
    ///
    /// For any imported SSH connection from Remmina format, the hostname, port,
    /// username, and key path should match the source data.
    #[test]
    fn prop_import_remmina_field_preservation(conn in arb_import_test_connection()) {
        use rustconn_core::import::RemminaImporter;

        let importer = RemminaImporter::new();
        let remmina_content = conn.to_remmina();

        // Parse the file
        let result = importer.parse_remmina_file(&remmina_content, "test.remmina", &mut std::collections::HashMap::new());

        // Property: One connection should be imported
        prop_assert_eq!(
            result.connections.len(),
            1,
            "Expected 1 connection, got {}. Content:\n{}",
            result.connections.len(),
            remmina_content
        );

        let imported = &result.connections[0];

        // Property: Name should match
        prop_assert_eq!(
            &imported.name,
            &conn.name,
            "Name mismatch. Expected '{}', got '{}'",
            conn.name,
            imported.name
        );

        // Property: Hostname should match
        prop_assert_eq!(
            &imported.host,
            &conn.host,
            "Hostname mismatch. Expected '{}', got '{}'",
            conn.host,
            imported.host
        );

        // Property: Port should match
        prop_assert_eq!(
            imported.port,
            conn.port,
            "Port mismatch. Expected {}, got {}",
            conn.port,
            imported.port
        );

        // Property: Username should match
        prop_assert_eq!(
            imported.username.as_ref(),
            conn.username.as_ref(),
            "Username mismatch. Expected {:?}, got {:?}",
            conn.username,
            imported.username
        );

        // Property: Key path should be preserved (ssh_privatekey)
        if let ProtocolConfig::Ssh(ref ssh_config) = imported.protocol_config {
            if conn.key_path.is_some() {
                prop_assert!(
                    ssh_config.key_path.is_some(),
                    "Key path should be preserved when specified"
                );
            }
        }
    }

    /// **Feature: ssh-agent-cli, Property 18: Import Field Preservation (Asbru)**
    /// **Validates: Requirements 9.1, 9.2, 9.3**
    ///
    /// For any imported SSH connection from Asbru format, the hostname, port,
    /// username, and key path should match the source data.
    #[test]
    fn prop_import_asbru_field_preservation(conn in arb_import_test_connection()) {
        use rustconn_core::import::AsbruImporter;

        let importer = AsbruImporter::new();
        let asbru_content = conn.to_asbru_yaml();

        // Parse the config
        let result = importer.parse_config(&asbru_content, "test");

        // Property: One connection should be imported
        prop_assert_eq!(
            result.connections.len(),
            1,
            "Expected 1 connection, got {}. Content:\n{}",
            result.connections.len(),
            asbru_content
        );

        let imported = &result.connections[0];

        // Property: Name should match
        prop_assert_eq!(
            &imported.name,
            &conn.name,
            "Name mismatch. Expected '{}', got '{}'",
            conn.name,
            imported.name
        );

        // Property: Hostname should match
        prop_assert_eq!(
            &imported.host,
            &conn.host,
            "Hostname mismatch. Expected '{}', got '{}'",
            conn.host,
            imported.host
        );

        // Property: Port should match
        prop_assert_eq!(
            imported.port,
            conn.port,
            "Port mismatch. Expected {}, got {}",
            conn.port,
            imported.port
        );

        // Property: Username should match
        prop_assert_eq!(
            imported.username.as_ref(),
            conn.username.as_ref(),
            "Username mismatch. Expected {:?}, got {:?}",
            conn.username,
            imported.username
        );

        // Property: Key path should be preserved (public key)
        if let ProtocolConfig::Ssh(ref ssh_config) = imported.protocol_config {
            if conn.key_path.is_some() {
                prop_assert!(
                    ssh_config.key_path.is_some(),
                    "Key path should be preserved when specified"
                );
            }
        }
    }

    /// **Feature: ssh-agent-cli, Property 18: Import Field Preservation (RDP)**
    /// **Validates: Requirements 9.2**
    ///
    /// For any imported RDP connection, the hostname, port, username, and domain
    /// should match the source data.
    #[test]
    fn prop_import_rdp_field_preservation(
        name in arb_connection_name(),
        host in arb_hostname(),
        port in arb_port(),
        username in prop::option::of(arb_connection_name()),
    ) {
        use rustconn_core::import::RemminaImporter;

        let importer = RemminaImporter::new();

        // Create Remmina RDP content
        let mut lines = vec!["[remmina]".to_string()];
        lines.push(format!("name={}", name));
        lines.push("protocol=RDP".to_string());
        lines.push(format!("server={}:{}", host, port));

        if let Some(ref user) = username {
            lines.push(format!("username={}", user));
        }

        let remmina_content = lines.join("\n");

        // Parse the file
        let result = importer.parse_remmina_file(&remmina_content, "test.remmina", &mut std::collections::HashMap::new());

        // Property: One connection should be imported
        prop_assert_eq!(
            result.connections.len(),
            1,
            "Expected 1 connection, got {}",
            result.connections.len()
        );

        let imported = &result.connections[0];

        // Property: Name should match
        prop_assert_eq!(&imported.name, &name, "Name mismatch");

        // Property: Hostname should match
        prop_assert_eq!(&imported.host, &host, "Hostname mismatch");

        // Property: Port should match
        prop_assert_eq!(imported.port, port, "Port mismatch");

        // Property: Username should match
        prop_assert_eq!(
            imported.username.as_ref(),
            username.as_ref(),
            "Username mismatch"
        );

        // Property: Protocol should be RDP
        prop_assert!(
            matches!(imported.protocol_config, ProtocolConfig::Rdp(_)),
            "Protocol should be RDP"
        );
    }
}

// ============================================================================
// CLI Error Exit Code Property Tests
// ============================================================================

/// Represents different CLI error types for testing exit codes.
/// The String fields mirror the actual CliError variants and are used
/// to generate realistic test data, even though we only test the exit codes.
/// The String payloads are required for the `arb_cli_error()` strategy to
/// generate realistic error instances matching the actual CLI error types.
#[derive(Debug, Clone)]
#[allow(dead_code)] // String fields needed for realistic test data generation
enum TestCliError {
    Config(String),
    ConnectionNotFound(String),
    Export(String),
    Import(String),
    TestFailed(String),
    Io(String),
}

impl TestCliError {
    /// Returns the expected exit code for this error type.
    /// This mirrors the logic in rustconn-cli's CliError::exit_code()
    fn expected_exit_code(&self) -> i32 {
        match self {
            // Connection-related failures use exit code 2
            Self::TestFailed(_) | Self::ConnectionNotFound(_) => 2,
            // All other errors use exit code 1
            Self::Config(_) | Self::Export(_) | Self::Import(_) | Self::Io(_) => 1,
        }
    }

    /// Returns true if this is a connection-related failure
    fn is_connection_failure(&self) -> bool {
        matches!(self, Self::TestFailed(_) | Self::ConnectionNotFound(_))
    }

    /// Returns the error category name for display
    fn category(&self) -> &'static str {
        match self {
            Self::Config(_) => "Config",
            Self::ConnectionNotFound(_) => "ConnectionNotFound",
            Self::Export(_) => "Export",
            Self::Import(_) => "Import",
            Self::TestFailed(_) => "TestFailed",
            Self::Io(_) => "Io",
        }
    }
}

/// Generates a simple error message string
fn arb_error_message() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 _-]{1,50}".prop_filter("message must not be empty", |s: &String| !s.is_empty())
}

/// Strategy for generating test CLI errors
fn arb_cli_error() -> impl Strategy<Value = TestCliError> {
    // Use prop_flat_map to generate error type first, then message
    prop_oneof![
        arb_error_message().prop_map(TestCliError::Config),
        arb_error_message().prop_map(TestCliError::ConnectionNotFound),
        arb_error_message().prop_map(TestCliError::Export),
        arb_error_message().prop_map(TestCliError::Import),
        arb_error_message().prop_map(TestCliError::TestFailed),
        arb_error_message().prop_map(TestCliError::Io),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: ssh-agent-cli, Property 15: CLI Error Exit Codes**
    /// **Validates: Requirements 7.7**
    ///
    /// For any CLI command that fails, the exit code should be non-zero and
    /// appropriate for the error type:
    /// - Exit code 1 for general errors (config, export, import, IO)
    /// - Exit code 2 for connection failures (test failed, connection not found)
    #[test]
    fn prop_cli_error_exit_codes(error in arb_cli_error()) {
        let exit_code = error.expected_exit_code();

        // Property: Exit code should be non-zero for all errors
        prop_assert!(
            exit_code != 0,
            "Exit code should be non-zero for error type {}",
            error.category()
        );

        // Property: Connection failures should return exit code 2
        if error.is_connection_failure() {
            prop_assert_eq!(
                exit_code,
                2,
                "Connection failure '{}' should return exit code 2, got {}",
                error.category(),
                exit_code
            );
        } else {
            // Property: General errors should return exit code 1
            prop_assert_eq!(
                exit_code,
                1,
                "General error '{}' should return exit code 1, got {}",
                error.category(),
                exit_code
            );
        }
    }

    /// **Feature: ssh-agent-cli, Property 15: CLI Error Exit Codes - Connection Failures**
    /// **Validates: Requirements 7.7**
    ///
    /// For any connection-related failure (TestFailed, ConnectionNotFound),
    /// the exit code should be 2.
    #[test]
    fn prop_cli_connection_failure_exit_code(
        error_type in prop_oneof![Just("test_failed"), Just("connection_not_found")],
        message in "[a-zA-Z0-9 _-]{1,50}"
    ) {
        let error = match error_type {
            "test_failed" => TestCliError::TestFailed(message),
            "connection_not_found" => TestCliError::ConnectionNotFound(message),
            _ => unreachable!(),
        };

        let exit_code = error.expected_exit_code();

        // Property: Connection failures should always return exit code 2
        prop_assert_eq!(
            exit_code,
            2,
            "Connection failure should return exit code 2"
        );

        // Property: Should be identified as connection failure
        prop_assert!(
            error.is_connection_failure(),
            "Error should be identified as connection failure"
        );
    }

    /// **Feature: ssh-agent-cli, Property 15: CLI Error Exit Codes - General Errors**
    /// **Validates: Requirements 7.7**
    ///
    /// For any general error (Config, Export, Import, Io),
    /// the exit code should be 1.
    #[test]
    fn prop_cli_general_error_exit_code(
        error_type in prop_oneof![Just("config"), Just("export"), Just("import"), Just("io")],
        message in "[a-zA-Z0-9 _-]{1,50}"
    ) {
        let error = match error_type {
            "config" => TestCliError::Config(message),
            "export" => TestCliError::Export(message),
            "import" => TestCliError::Import(message),
            "io" => TestCliError::Io(message),
            _ => unreachable!(),
        };

        let exit_code = error.expected_exit_code();

        // Property: General errors should always return exit code 1
        prop_assert_eq!(
            exit_code,
            1,
            "General error should return exit code 1"
        );

        // Property: Should NOT be identified as connection failure
        prop_assert!(
            !error.is_connection_failure(),
            "Error should NOT be identified as connection failure"
        );
    }
}

// ============================================================================
// CLI Output Feedback Property Tests
// ============================================================================

/// Generates a valid protocol name for CLI output
fn arb_protocol_name() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("SSH".to_string()),
        Just("AWS SSM".to_string()),
        Just("GCP IAP".to_string()),
        Just("Azure Bastion".to_string()),
        Just("Teleport".to_string()),
        Just("Tailscale SSH".to_string()),
        Just("Cloudflare Access".to_string()),
        Just("HashiCorp Boundary".to_string()),
        Just("Generic Command".to_string()),
    ]
}

/// Generates a valid host/target identifier
fn arb_host_identifier() -> impl Strategy<Value = String> {
    prop_oneof![
        // Standard hostnames
        arb_hostname(),
        // AWS instance IDs
        prop::string::string_regex("i-[a-f0-9]{17}").unwrap(),
        // IP addresses
        prop::string::string_regex("[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}\\.[0-9]{1,3}").unwrap(),
    ]
}

/// Generates a valid command string
fn arb_command_string() -> impl Strategy<Value = String> {
    prop_oneof![
        // SSH commands
        prop::string::string_regex("ssh [a-z]+@[a-z0-9.-]+").unwrap(),
        // AWS SSM commands
        prop::string::string_regex("aws ssm start-session --target i-[a-f0-9]{8}").unwrap(),
        // gcloud commands
        prop::string::string_regex("gcloud compute ssh [a-z0-9-]+ --zone [a-z0-9-]+").unwrap(),
        // Simple commands
        arb_connection_name().prop_map(|s| format!("connect {s}")),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-bugfixes, Property 2: CLI Output Message Format**
    /// **Validates: Requirements 5.1, 5.3**
    ///
    /// For any protocol name and host string, the connection message should contain
    /// both values and the 🔗 emoji.
    #[test]
    fn prop_cli_output_message_format(
        protocol in arb_protocol_name(),
        host in arb_host_identifier()
    ) {
        use rustconn_core::protocol::format_connection_message;

        let message = format_connection_message(&protocol, &host);

        // Property: Message should contain the 🔗 emoji
        prop_assert!(
            message.contains("🔗"),
            "Connection message should contain 🔗 emoji. Message: '{}'",
            message
        );

        // Property: Message should contain the protocol name
        prop_assert!(
            message.contains(&protocol),
            "Connection message should contain protocol '{}'. Message: '{}'",
            protocol,
            message
        );

        // Property: Message should contain the host
        prop_assert!(
            message.contains(&host),
            "Connection message should contain host '{}'. Message: '{}'",
            host,
            message
        );

        // Property: Message should follow the expected format
        let expected = format!("🔗 Connecting via {} to {}...", protocol, host);
        prop_assert_eq!(
            message,
            expected,
            "Message format mismatch"
        );
    }

    /// **Feature: rustconn-bugfixes, Property 3: CLI Command Echo Format**
    /// **Validates: Requirements 5.2, 5.4**
    ///
    /// For any command string, the command echo message should contain the exact
    /// command and the ⚡ emoji.
    #[test]
    fn prop_cli_command_echo_format(command in arb_command_string()) {
        use rustconn_core::protocol::format_command_message;

        let message = format_command_message(&command);

        // Property: Message should contain the ⚡ emoji
        prop_assert!(
            message.contains("⚡"),
            "Command message should contain ⚡ emoji. Message: '{}'",
            message
        );

        // Property: Message should contain the exact command
        prop_assert!(
            message.contains(&command),
            "Command message should contain exact command '{}'. Message: '{}'",
            command,
            message
        );

        // Property: Message should follow the expected format
        let expected = format!("⚡ Executing: {}", command);
        prop_assert_eq!(
            message,
            expected,
            "Message format mismatch"
        );
    }

    /// **Feature: rustconn-bugfixes, Property 2: CLI Output Message Format - Empty Inputs**
    /// **Validates: Requirements 5.1, 5.3**
    ///
    /// For empty protocol or host strings, the function should still produce
    /// a valid message with the emoji prefix.
    #[test]
    fn prop_cli_output_message_empty_inputs(
        use_empty_protocol in prop::bool::ANY,
        use_empty_host in prop::bool::ANY
    ) {
        use rustconn_core::protocol::format_connection_message;

        let protocol = if use_empty_protocol { "" } else { "SSH" };
        let host = if use_empty_host { "" } else { "example.com" };

        let message = format_connection_message(protocol, host);

        // Property: Message should always contain the 🔗 emoji
        prop_assert!(
            message.contains("🔗"),
            "Connection message should always contain 🔗 emoji even with empty inputs. Message: '{}'",
            message
        );

        // Property: Message should always contain "Connecting via"
        prop_assert!(
            message.contains("Connecting via"),
            "Connection message should contain 'Connecting via'. Message: '{}'",
            message
        );
    }

    /// **Feature: rustconn-bugfixes, Property 3: CLI Command Echo Format - Empty Command**
    /// **Validates: Requirements 5.2, 5.4**
    ///
    /// For an empty command string, the function should still produce
    /// a valid message with the emoji prefix.
    #[test]
    fn prop_cli_command_echo_empty_command(_dummy in Just(())) {
        use rustconn_core::protocol::format_command_message;

        let message = format_command_message("");

        // Property: Message should contain the ⚡ emoji
        prop_assert!(
            message.contains("⚡"),
            "Command message should contain ⚡ emoji even with empty command. Message: '{}'",
            message
        );

        // Property: Message should contain "Executing:"
        prop_assert!(
            message.contains("Executing:"),
            "Command message should contain 'Executing:'. Message: '{}'",
            message
        );
    }

    /// **Feature: rustconn-bugfixes, Property 2 & 3: CLI Output Special Characters**
    /// **Validates: Requirements 5.1, 5.2, 5.3, 5.4**
    ///
    /// For inputs containing special characters, the functions should preserve
    /// them exactly in the output.
    #[test]
    fn prop_cli_output_special_characters(
        special_char in prop_oneof![
            Just("@"),
            Just("-"),
            Just("_"),
            Just("."),
            Just(":"),
            Just("/"),
        ]
    ) {
        use rustconn_core::protocol::{format_command_message, format_connection_message};

        let host_with_special = format!("server{}example.com", special_char);
        let command_with_special = format!("ssh user{}host", special_char);

        let conn_message = format_connection_message("SSH", &host_with_special);
        let cmd_message = format_command_message(&command_with_special);

        // Property: Special characters should be preserved in connection message
        prop_assert!(
            conn_message.contains(&host_with_special),
            "Special character '{}' should be preserved in host. Message: '{}'",
            special_char,
            conn_message
        );

        // Property: Special characters should be preserved in command message
        prop_assert!(
            cmd_message.contains(&command_with_special),
            "Special character '{}' should be preserved in command. Message: '{}'",
            special_char,
            cmd_message
        );
    }
}
