//! Property-based tests for export functionality
//!
//! Tests correctness properties for exporting connections to various formats.

use proptest::prelude::*;
use rustconn_core::export::{AnsibleExporter, AsbruExporter, RemminaExporter, SshConfigExporter};
use rustconn_core::import::{
    AnsibleInventoryImporter, AsbruImporter, RemminaImporter, SshConfigImporter,
};
use rustconn_core::models::{Connection, ConnectionGroup, ProtocolConfig, ProtocolType};
use std::path::PathBuf;

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

/// Generates a valid username
fn arb_username() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-z_][a-z0-9_-]{0,30}")
        .unwrap()
        .prop_filter("username must not be empty", |s| !s.is_empty())
}

/// Generates a valid port number
fn arb_port() -> impl Strategy<Value = u16> {
    1u16..65535
}

/// Generates a valid SSH key path
fn arb_key_path() -> impl Strategy<Value = PathBuf> {
    prop::string::string_regex("[a-z0-9_]{1,20}")
        .unwrap()
        .prop_filter("name must not be empty", |s| !s.is_empty())
        .prop_map(|name| PathBuf::from(format!("/home/user/.ssh/id_{}", name)))
}

/// Generates a valid group name
fn arb_group_name() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-zA-Z][a-zA-Z0-9_-]{0,20}")
        .unwrap()
        .prop_filter("group name must not be empty", |s| !s.is_empty())
}

/// Represents a generated SSH connection for testing
#[derive(Debug, Clone)]
struct TestSshConnection {
    name: String,
    host: String,
    port: u16,
    username: Option<String>,
    key_path: Option<PathBuf>,
}

impl TestSshConnection {
    /// Converts to a Connection object
    fn to_connection(&self) -> Connection {
        let mut conn = Connection::new_ssh(self.name.clone(), self.host.clone(), self.port);
        conn.username.clone_from(&self.username);
        if let ProtocolConfig::Ssh(ref mut ssh_config) = conn.protocol_config {
            ssh_config.key_path.clone_from(&self.key_path);
        }
        conn
    }
}

/// Strategy for generating test SSH connections
fn arb_ssh_connection() -> impl Strategy<Value = TestSshConnection> {
    (
        arb_connection_name(),
        arb_hostname(),
        arb_port(),
        prop::option::of(arb_username()),
        prop::option::of(arb_key_path()),
    )
        .prop_map(|(name, host, port, username, key_path)| TestSshConnection {
            name,
            host,
            port,
            username,
            key_path,
        })
}

/// Strategy for generating multiple SSH connections with unique names
fn arb_ssh_connections() -> impl Strategy<Value = Vec<TestSshConnection>> {
    prop::collection::vec(arb_ssh_connection(), 1..10).prop_map(|connections| {
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

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: ssh-agent-cli, Property 4: Ansible Export Round-Trip**
    /// **Validates: Requirements 3.6, 9.5**
    ///
    /// For any set of SSH connections, exporting to Ansible inventory format (INI)
    /// and re-importing should preserve hostname, port, username, and key path.
    #[test]
    fn prop_ansible_export_roundtrip_ini(connections in arb_ssh_connections()) {
        let importer = AnsibleInventoryImporter::new();

        // Convert test connections to Connection objects
        let conns: Vec<Connection> = connections.iter().map(|c| c.to_connection()).collect();

        // Export to INI format
        let exported = AnsibleExporter::export_ini(&conns, &[]);

        // Re-import the exported content
        let result = importer.parse_ini_inventory(&exported, "test");

        // Property: All connections should be re-imported
        prop_assert_eq!(
            result.connections.len(),
            connections.len(),
            "Expected {} connections, got {}. Exported:\n{}",
            connections.len(),
            result.connections.len(),
            exported
        );

        // Property: Each connection should preserve key fields
        for original in &connections {
            let reimported = result
                .connections
                .iter()
                .find(|c| c.name == original.name)
                .expect(&format!("Connection '{}' not found after round-trip", original.name));

            // Host should match
            prop_assert_eq!(
                &reimported.host,
                &original.host,
                "Host mismatch for '{}'. Expected '{}', got '{}'",
                original.name,
                original.host,
                reimported.host
            );

            // Port should match
            prop_assert_eq!(
                reimported.port,
                original.port,
                "Port mismatch for '{}'. Expected {}, got {}",
                original.name,
                original.port,
                reimported.port
            );

            // Username should match
            prop_assert_eq!(
                reimported.username.as_ref(),
                original.username.as_ref(),
                "Username mismatch for '{}'",
                original.name
            );

            // Key path should match (if set)
            if let (ProtocolConfig::Ssh(reimported_ssh), Some(ref original_key)) =
                (&reimported.protocol_config, &original.key_path)
            {
                prop_assert_eq!(
                    reimported_ssh.key_path.as_ref(),
                    Some(original_key),
                    "Key path mismatch for '{}'",
                    original.name
                );
            }
        }
    }

    /// **Feature: ssh-agent-cli, Property 4: Ansible Export Round-Trip (YAML)**
    /// **Validates: Requirements 3.6, 9.5**
    ///
    /// For any set of SSH connections, exporting to Ansible inventory format (YAML)
    /// and re-importing should preserve hostname, port, username, and key path.
    #[test]
    fn prop_ansible_export_roundtrip_yaml(connections in arb_ssh_connections()) {
        let importer = AnsibleInventoryImporter::new();

        // Convert test connections to Connection objects
        let conns: Vec<Connection> = connections.iter().map(|c| c.to_connection()).collect();

        // Export to YAML format
        let exported = AnsibleExporter::export_yaml(&conns, &[]);

        // Re-import the exported content
        let result = importer.parse_yaml_inventory(&exported, "test");

        // Property: All connections should be re-imported
        prop_assert_eq!(
            result.connections.len(),
            connections.len(),
            "Expected {} connections, got {}. Exported:\n{}",
            connections.len(),
            result.connections.len(),
            exported
        );

        // Property: Each connection should preserve key fields
        for original in &connections {
            let reimported = result
                .connections
                .iter()
                .find(|c| c.name == original.name)
                .expect(&format!("Connection '{}' not found after round-trip", original.name));

            // Host should match
            prop_assert_eq!(
                &reimported.host,
                &original.host,
                "Host mismatch for '{}'. Expected '{}', got '{}'",
                original.name,
                original.host,
                reimported.host
            );

            // Port should match
            prop_assert_eq!(
                reimported.port,
                original.port,
                "Port mismatch for '{}'. Expected {}, got {}",
                original.name,
                original.port,
                reimported.port
            );

            // Username should match
            prop_assert_eq!(
                reimported.username.as_ref(),
                original.username.as_ref(),
                "Username mismatch for '{}'",
                original.name
            );
        }
    }

    /// **Feature: ssh-agent-cli, Property 5: Ansible Export Completeness**
    /// **Validates: Requirements 3.2, 3.3, 3.4**
    ///
    /// For any SSH connection with non-default port, the exported Ansible inventory
    /// should include ansible_host, ansible_user, ansible_port, and
    /// ansible_ssh_private_key_file when applicable.
    #[test]
    fn prop_ansible_export_completeness(connection in arb_ssh_connection()) {
        // Convert to Connection object
        let conn = connection.to_connection();

        // Export single connection
        let entry = AnsibleExporter::format_host_entry(&conn);

        // Property: ansible_host should be present if host differs from name
        if conn.host != conn.name {
            prop_assert!(
                entry.contains(&format!("ansible_host={}", conn.host)),
                "ansible_host should be present when host differs from name. Entry: {}",
                entry
            );
        }

        // Property: ansible_user should be present if username is set
        if let Some(ref user) = conn.username {
            prop_assert!(
                entry.contains(&format!("ansible_user={}", user)),
                "ansible_user should be present when username is set. Entry: {}",
                entry
            );
        }

        // Property: ansible_port should be present only if not default (22)
        if conn.port == 22 {
            prop_assert!(
                !entry.contains("ansible_port="),
                "ansible_port should NOT be present for default port 22. Entry: {}",
                entry
            );
        } else {
            prop_assert!(
                entry.contains(&format!("ansible_port={}", conn.port)),
                "ansible_port should be present for non-default port. Entry: {}",
                entry
            );
        }

        // Property: ansible_ssh_private_key_file should be present if key_path is set
        if let ProtocolConfig::Ssh(ref ssh_config) = conn.protocol_config {
            if let Some(ref key_path) = ssh_config.key_path {
                prop_assert!(
                    entry.contains(&format!("ansible_ssh_private_key_file={}", key_path.display())),
                    "ansible_ssh_private_key_file should be present when key_path is set. Entry: {}",
                    entry
                );
            }
        }
    }

    /// **Feature: ssh-agent-cli, Property 5: Ansible Export Completeness (Groups)**
    /// **Validates: Requirements 3.3**
    ///
    /// For any set of connections organized in groups, the exported Ansible inventory
    /// should organize hosts under their respective group sections.
    #[test]
    fn prop_ansible_export_groups(
        group_name in arb_group_name(),
        connections in arb_ssh_connections()
    ) {
        // Create a group
        let group = ConnectionGroup::new(group_name.clone());
        let group_id = group.id;

        // Assign all connections to the group
        let conns: Vec<Connection> = connections
            .iter()
            .map(|c| c.to_connection().with_group(group_id))
            .collect();

        // Export to INI format
        let exported = AnsibleExporter::export_ini(&conns, std::slice::from_ref(&group));

        // Property: Group section should be present
        // Group names are sanitized (spaces become underscores)
        let sanitized_group_name: String = group_name
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
            .collect();

        prop_assert!(
            exported.contains(&format!("[{}]", sanitized_group_name)),
            "Group section [{}] should be present in exported INI. Exported:\n{}",
            sanitized_group_name,
            exported
        );

        // Property: All connections should be listed under the group
        for conn in &connections {
            prop_assert!(
                exported.contains(&conn.name),
                "Connection '{}' should be present in exported INI. Exported:\n{}",
                conn.name,
                exported
            );
        }
    }

    /// **Feature: ssh-agent-cli, Property 5: Ansible Export Completeness (YAML Groups)**
    /// **Validates: Requirements 3.3**
    ///
    /// For any set of connections organized in groups, the exported YAML inventory
    /// should have a nested structure with children groups.
    #[test]
    fn prop_ansible_export_yaml_groups(
        group_name in arb_group_name(),
        connections in arb_ssh_connections()
    ) {
        // Create a group
        let group = ConnectionGroup::new(group_name.clone());
        let group_id = group.id;

        // Assign all connections to the group
        let conns: Vec<Connection> = connections
            .iter()
            .map(|c| c.to_connection().with_group(group_id))
            .collect();

        // Export to YAML format
        let exported = AnsibleExporter::export_yaml(&conns, std::slice::from_ref(&group));

        // Property: YAML should have 'children' section
        prop_assert!(
            exported.contains("children:"),
            "YAML should have 'children' section. Exported:\n{}",
            exported
        );

        // Property: Group name should be present under children
        let sanitized_group_name: String = group_name
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
            .collect();

        prop_assert!(
            exported.contains(&format!("{}:", sanitized_group_name)),
            "Group '{}' should be present in YAML. Exported:\n{}",
            sanitized_group_name,
            exported
        );
    }

    /// **Feature: ssh-agent-cli, Property: Non-SSH connections are skipped**
    /// **Validates: Requirements 3.1**
    ///
    /// For any non-SSH connection mixed with SSH connections, the Ansible exporter
    /// should only export the SSH connections (Ansible inventory only supports SSH).
    #[test]
    fn prop_ansible_export_skips_non_ssh(
        ssh_port in arb_port(),
        non_ssh_port in arb_port(),
        protocol_type in prop_oneof![Just(ProtocolType::Rdp), Just(ProtocolType::Vnc)]
    ) {
        // Use fixed, distinct names to avoid substring matching issues
        let ssh_name = "ssh_server_test";
        let ssh_host = "ssh.example.com";
        let non_ssh_name = "rdp_vnc_server_test";
        let non_ssh_host = "rdp.example.com";

        // Create an SSH connection
        let ssh_conn = Connection::new_ssh(ssh_name.to_string(), ssh_host.to_string(), ssh_port);

        // Create a non-SSH connection
        let non_ssh_conn = match protocol_type {
            ProtocolType::Rdp => Connection::new_rdp(non_ssh_name.to_string(), non_ssh_host.to_string(), non_ssh_port),
            ProtocolType::Vnc => Connection::new_vnc(non_ssh_name.to_string(), non_ssh_host.to_string(), non_ssh_port),
            _ => unreachable!(),
        };

        // Export both connections to INI format
        let exported = AnsibleExporter::export_ini(&[ssh_conn, non_ssh_conn], &[]);

        // Property: SSH connection should appear in output (check for the host entry line)
        prop_assert!(
            exported.contains(ssh_name),
            "SSH connection '{}' should appear in Ansible export. Exported:\n{}",
            ssh_name,
            exported
        );

        // Property: Non-SSH connection should NOT appear in output
        prop_assert!(
            !exported.contains(non_ssh_name),
            "Non-SSH connection '{}' should not appear in Ansible export. Exported:\n{}",
            non_ssh_name,
            exported
        );
    }

    /// **Feature: ssh-agent-cli, Property 6: SSH Config Export Round-Trip**
    /// **Validates: Requirements 4.6, 9.5**
    ///
    /// For any set of SSH connections, exporting to SSH config format
    /// and re-importing should preserve hostname, port, username, and identity file.
    #[test]
    fn prop_ssh_config_export_roundtrip(connections in arb_ssh_connections()) {
        let importer = SshConfigImporter::new();

        // Convert test connections to Connection objects
        let conns: Vec<Connection> = connections.iter().map(|c| c.to_connection()).collect();

        // Export to SSH config format
        let exported = SshConfigExporter::export(&conns);

        // Re-import the exported content
        let result = importer.parse_config(&exported, "test");

        // Property: All connections should be re-imported
        prop_assert_eq!(
            result.connections.len(),
            connections.len(),
            "Expected {} connections, got {}. Exported:\n{}",
            connections.len(),
            result.connections.len(),
            exported
        );

        // Property: Each connection should preserve key fields
        for original in &connections {
            let reimported = result
                .connections
                .iter()
                .find(|c| c.name == original.name)
                .expect(&format!("Connection '{}' not found after round-trip", original.name));

            // Host should match
            prop_assert_eq!(
                &reimported.host,
                &original.host,
                "Host mismatch for '{}'. Expected '{}', got '{}'",
                original.name,
                original.host,
                reimported.host
            );

            // Port should match
            prop_assert_eq!(
                reimported.port,
                original.port,
                "Port mismatch for '{}'. Expected {}, got {}",
                original.name,
                original.port,
                reimported.port
            );

            // Username should match
            prop_assert_eq!(
                reimported.username.as_ref(),
                original.username.as_ref(),
                "Username mismatch for '{}'",
                original.name
            );

            // Key path should match (if set)
            if let (ProtocolConfig::Ssh(reimported_ssh), Some(ref original_key)) =
                (&reimported.protocol_config, &original.key_path)
            {
                prop_assert_eq!(
                    reimported_ssh.key_path.as_ref(),
                    Some(original_key),
                    "Key path mismatch for '{}'",
                    original.name
                );
            }
        }
    }

    /// **Feature: ssh-agent-cli, Property 7: SSH Config Export Completeness**
    /// **Validates: Requirements 4.2, 4.3, 4.4**
    ///
    /// For any SSH connection, the exported SSH config should include Host (from name),
    /// HostName, User, Port, and IdentityFile directives when applicable.
    #[test]
    fn prop_ssh_config_export_completeness(connection in arb_ssh_connection()) {
        // Convert to Connection object
        let conn = connection.to_connection();

        // Export single connection
        let entry = SshConfigExporter::format_host_entry(&conn);

        // Property: Host alias should be present (connection name)
        prop_assert!(
            entry.contains(&format!("Host {}", conn.name)),
            "Host alias should be present. Entry:\n{}",
            entry
        );

        // Property: HostName should always be present
        prop_assert!(
            entry.contains(&format!("HostName {}", conn.host)),
            "HostName should be present. Entry:\n{}",
            entry
        );

        // Property: User should be present if username is set
        if let Some(ref user) = conn.username {
            prop_assert!(
                entry.contains(&format!("User {}", user)),
                "User should be present when username is set. Entry:\n{}",
                entry
            );
        }

        // Property: Port should be present only if not default (22)
        if conn.port == 22 {
            prop_assert!(
                !entry.contains("Port "),
                "Port should NOT be present for default port 22. Entry:\n{}",
                entry
            );
        } else {
            prop_assert!(
                entry.contains(&format!("Port {}", conn.port)),
                "Port should be present for non-default port. Entry:\n{}",
                entry
            );
        }

        // Property: IdentityFile should be present if key_path is set
        if let ProtocolConfig::Ssh(ref ssh_config) = conn.protocol_config {
            if let Some(ref key_path) = ssh_config.key_path {
                prop_assert!(
                    entry.contains(&format!("IdentityFile {}", key_path.display())),
                    "IdentityFile should be present when key_path is set. Entry:\n{}",
                    entry
                );
            }
        }
    }

    /// **Feature: ssh-agent-cli, Property 7: SSH Config Export with ProxyJump**
    /// **Validates: Requirements 4.2, 4.3**
    ///
    /// For any SSH connection with ProxyJump configured, the exported SSH config
    /// should include the ProxyJump directive.
    #[test]
    fn prop_ssh_config_export_proxy_jump(
        connection in arb_ssh_connection(),
        proxy_host in arb_hostname()
    ) {
        // Convert to Connection object and add proxy jump
        let mut conn = connection.to_connection();
        if let ProtocolConfig::Ssh(ref mut ssh_config) = conn.protocol_config {
            ssh_config.proxy_jump = Some(proxy_host.clone());
        }

        // Export single connection
        let entry = SshConfigExporter::format_host_entry(&conn);

        // Property: ProxyJump should be present
        prop_assert!(
            entry.contains(&format!("ProxyJump {}", proxy_host)),
            "ProxyJump should be present when configured. Entry:\n{}",
            entry
        );
    }

    /// **Feature: ssh-agent-cli, Property: SSH Config Export skips non-SSH**
    /// **Validates: Requirements 4.1**
    ///
    /// For any non-SSH connection mixed with SSH connections, the SSH config exporter
    /// should only export the SSH connections.
    #[test]
    fn prop_ssh_config_export_skips_non_ssh(
        ssh_port in arb_port(),
        non_ssh_port in arb_port(),
        protocol_type in prop_oneof![Just(ProtocolType::Rdp), Just(ProtocolType::Vnc)]
    ) {
        // Use fixed, distinct names to avoid substring matching issues
        let ssh_name = "ssh_server_test";
        let ssh_host = "ssh.example.com";
        let non_ssh_name = "rdp_vnc_server_test";
        let non_ssh_host = "rdp.example.com";

        // Create an SSH connection
        let ssh_conn = Connection::new_ssh(ssh_name.to_string(), ssh_host.to_string(), ssh_port);

        // Create a non-SSH connection
        let non_ssh_conn = match protocol_type {
            ProtocolType::Rdp => Connection::new_rdp(non_ssh_name.to_string(), non_ssh_host.to_string(), non_ssh_port),
            ProtocolType::Vnc => Connection::new_vnc(non_ssh_name.to_string(), non_ssh_host.to_string(), non_ssh_port),
            _ => unreachable!(),
        };

        // Export both connections to SSH config format
        let exported = SshConfigExporter::export(&[ssh_conn, non_ssh_conn]);

        // Property: SSH connection should appear in output
        prop_assert!(
            exported.contains(ssh_name),
            "SSH connection '{}' should appear in SSH config export. Exported:\n{}",
            ssh_name,
            exported
        );

        // Property: Non-SSH connection should NOT appear in output
        prop_assert!(
            !exported.contains(non_ssh_name),
            "Non-SSH connection '{}' should not appear in SSH config export. Exported:\n{}",
            non_ssh_name,
            exported
        );
    }

    /// **Feature: ssh-agent-cli, Property 8: Remmina Export Round-Trip**
    /// **Validates: Requirements 5.6, 9.5**
    ///
    /// For any connection (SSH, RDP, or VNC), exporting to Remmina format
    /// and re-importing should preserve protocol type, hostname, port, and username.
    #[test]
    fn prop_remmina_export_roundtrip_ssh(connection in arb_ssh_connection()) {
        let importer = RemminaImporter::new();

        // Convert to Connection object
        let conn = connection.to_connection();

        // Export to Remmina format
        let exported = RemminaExporter::export_connection(&conn).unwrap();

        // Re-import the exported content
        let result = importer.parse_remmina_file(&exported, "test.remmina");

        // Property: Connection should be re-imported
        prop_assert_eq!(
            result.connections.len(),
            1,
            "Expected 1 connection, got {}. Exported:\n{}",
            result.connections.len(),
            exported
        );

        let reimported = &result.connections[0];

        // Property: Name should match
        prop_assert_eq!(
            &reimported.name,
            &conn.name,
            "Name mismatch. Expected '{}', got '{}'. Exported:\n{}",
            conn.name,
            reimported.name,
            exported
        );

        // Property: Host should match
        prop_assert_eq!(
            &reimported.host,
            &conn.host,
            "Host mismatch. Expected '{}', got '{}'. Exported:\n{}",
            conn.host,
            reimported.host,
            exported
        );

        // Property: Port should match
        prop_assert_eq!(
            reimported.port,
            conn.port,
            "Port mismatch. Expected {}, got {}. Exported:\n{}",
            conn.port,
            reimported.port,
            exported
        );

        // Property: Protocol should match
        prop_assert_eq!(
            reimported.protocol,
            ProtocolType::Ssh,
            "Protocol should be SSH. Exported:\n{}",
            exported
        );

        // Property: Username should match
        prop_assert_eq!(
            reimported.username.as_ref(),
            conn.username.as_ref(),
            "Username mismatch. Exported:\n{}",
            exported
        );
    }

    /// **Feature: ssh-agent-cli, Property 8: Remmina Export Round-Trip (RDP)**
    /// **Validates: Requirements 5.6, 9.5**
    ///
    /// For any RDP connection, exporting to Remmina format and re-importing
    /// should preserve protocol type, hostname, port, and username.
    #[test]
    fn prop_remmina_export_roundtrip_rdp(
        name in arb_connection_name(),
        host in arb_hostname(),
        port in arb_port(),
        username in prop::option::of(arb_username()),
        domain in prop::option::of(arb_username())
    ) {
        let importer = RemminaImporter::new();

        // Create RDP connection
        let mut conn = Connection::new_rdp(name.clone(), host.clone(), port);
        conn.username = username.clone();
        conn.domain = domain.clone();

        // Export to Remmina format
        let exported = RemminaExporter::export_connection(&conn).unwrap();

        // Re-import the exported content
        let result = importer.parse_remmina_file(&exported, "test.remmina");

        // Property: Connection should be re-imported
        prop_assert_eq!(
            result.connections.len(),
            1,
            "Expected 1 connection, got {}. Exported:\n{}",
            result.connections.len(),
            exported
        );

        let reimported = &result.connections[0];

        // Property: Name should match
        prop_assert_eq!(
            &reimported.name,
            &name,
            "Name mismatch. Expected '{}', got '{}'. Exported:\n{}",
            name,
            reimported.name,
            exported
        );

        // Property: Host should match
        prop_assert_eq!(
            &reimported.host,
            &host,
            "Host mismatch. Expected '{}', got '{}'. Exported:\n{}",
            host,
            reimported.host,
            exported
        );

        // Property: Protocol should be RDP
        prop_assert_eq!(
            reimported.protocol,
            ProtocolType::Rdp,
            "Protocol should be RDP. Exported:\n{}",
            exported
        );

        // Property: Username should match
        prop_assert_eq!(
            reimported.username.as_ref(),
            username.as_ref(),
            "Username mismatch. Exported:\n{}",
            exported
        );
    }

    /// **Feature: ssh-agent-cli, Property 8: Remmina Export Round-Trip (VNC)**
    /// **Validates: Requirements 5.6, 9.5**
    ///
    /// For any VNC connection, exporting to Remmina format and re-importing
    /// should preserve protocol type, hostname, and port.
    #[test]
    fn prop_remmina_export_roundtrip_vnc(
        name in arb_connection_name(),
        host in arb_hostname(),
        port in arb_port()
    ) {
        let importer = RemminaImporter::new();

        // Create VNC connection
        let conn = Connection::new_vnc(name.clone(), host.clone(), port);

        // Export to Remmina format
        let exported = RemminaExporter::export_connection(&conn).unwrap();

        // Re-import the exported content
        let result = importer.parse_remmina_file(&exported, "test.remmina");

        // Property: Connection should be re-imported
        prop_assert_eq!(
            result.connections.len(),
            1,
            "Expected 1 connection, got {}. Exported:\n{}",
            result.connections.len(),
            exported
        );

        let reimported = &result.connections[0];

        // Property: Name should match
        prop_assert_eq!(
            &reimported.name,
            &name,
            "Name mismatch. Expected '{}', got '{}'. Exported:\n{}",
            name,
            reimported.name,
            exported
        );

        // Property: Host should match
        prop_assert_eq!(
            &reimported.host,
            &host,
            "Host mismatch. Expected '{}', got '{}'. Exported:\n{}",
            host,
            reimported.host,
            exported
        );

        // Property: Port should match
        prop_assert_eq!(
            reimported.port,
            port,
            "Port mismatch. Expected {}, got {}. Exported:\n{}",
            port,
            reimported.port,
            exported
        );

        // Property: Protocol should be VNC
        prop_assert_eq!(
            reimported.protocol,
            ProtocolType::Vnc,
            "Protocol should be VNC. Exported:\n{}",
            exported
        );
    }

    /// **Feature: ssh-agent-cli, Property 9: Remmina Export Protocol Handling (SSH)**
    /// **Validates: Requirements 5.2, 5.3, 5.4**
    ///
    /// For any SSH connection, the exported Remmina file should contain
    /// the correct protocol-specific fields (protocol=SSH, ssh_privatekey).
    #[test]
    fn prop_remmina_export_protocol_ssh(connection in arb_ssh_connection()) {
        // Convert to Connection object
        let conn = connection.to_connection();

        // Export to Remmina format
        let exported = RemminaExporter::export_connection(&conn).unwrap();

        // Property: Should contain protocol=SSH
        prop_assert!(
            exported.contains("protocol=SSH"),
            "SSH connection should have protocol=SSH. Exported:\n{}",
            exported
        );

        // Property: Should contain server field
        prop_assert!(
            exported.contains("server="),
            "SSH connection should have server field. Exported:\n{}",
            exported
        );

        // Property: If key_path is set, should contain ssh_privatekey
        if let ProtocolConfig::Ssh(ref ssh_config) = conn.protocol_config {
            if let Some(ref key_path) = ssh_config.key_path {
                prop_assert!(
                    exported.contains(&format!("ssh_privatekey={}", key_path.display())),
                    "SSH connection with key should have ssh_privatekey. Exported:\n{}",
                    exported
                );
            }
        }

        // Property: If username is set, should contain username field
        if let Some(ref username) = conn.username {
            prop_assert!(
                exported.contains(&format!("username={}", username)),
                "SSH connection with username should have username field. Exported:\n{}",
                exported
            );
        }
    }

    /// **Feature: ssh-agent-cli, Property 9: Remmina Export Protocol Handling (RDP)**
    /// **Validates: Requirements 5.2, 5.3, 5.4**
    ///
    /// For any RDP connection, the exported Remmina file should contain
    /// the correct protocol-specific fields (protocol=RDP, domain).
    #[test]
    fn prop_remmina_export_protocol_rdp(
        name in arb_connection_name(),
        host in arb_hostname(),
        port in arb_port(),
        username in prop::option::of(arb_username()),
        domain in prop::option::of(arb_username())
    ) {
        // Create RDP connection
        let mut conn = Connection::new_rdp(name, host, port);
        conn.username = username.clone();
        conn.domain = domain.clone();

        // Export to Remmina format
        let exported = RemminaExporter::export_connection(&conn).unwrap();

        // Property: Should contain protocol=RDP
        prop_assert!(
            exported.contains("protocol=RDP"),
            "RDP connection should have protocol=RDP. Exported:\n{}",
            exported
        );

        // Property: Should contain server field
        prop_assert!(
            exported.contains("server="),
            "RDP connection should have server field. Exported:\n{}",
            exported
        );

        // Property: If domain is set, should contain domain field
        if let Some(ref dom) = domain {
            prop_assert!(
                exported.contains(&format!("domain={}", dom)),
                "RDP connection with domain should have domain field. Exported:\n{}",
                exported
            );
        }

        // Property: If username is set, should contain username field
        if let Some(ref user) = username {
            prop_assert!(
                exported.contains(&format!("username={}", user)),
                "RDP connection with username should have username field. Exported:\n{}",
                exported
            );
        }
    }

    /// **Feature: ssh-agent-cli, Property 9: Remmina Export Protocol Handling (VNC)**
    /// **Validates: Requirements 5.2, 5.3, 5.4**
    ///
    /// For any VNC connection, the exported Remmina file should contain
    /// the correct protocol-specific fields (protocol=VNC, server with port).
    #[test]
    fn prop_remmina_export_protocol_vnc(
        name in arb_connection_name(),
        host in arb_hostname(),
        port in arb_port()
    ) {
        // Create VNC connection
        let conn = Connection::new_vnc(name, host.clone(), port);

        // Export to Remmina format
        let exported = RemminaExporter::export_connection(&conn).unwrap();

        // Property: Should contain protocol=VNC
        prop_assert!(
            exported.contains("protocol=VNC"),
            "VNC connection should have protocol=VNC. Exported:\n{}",
            exported
        );

        // Property: Should contain server field with port (VNC always includes port)
        prop_assert!(
            exported.contains(&format!("server={}:{}", host, port)),
            "VNC connection should have server with port. Exported:\n{}",
            exported
        );
    }

    /// **Feature: ssh-agent-cli, Property 10: Asbru Export Round-Trip**
    /// **Validates: Requirements 6.5, 9.5**
    ///
    /// For any set of connections with groups, exporting to Asbru format
    /// and re-importing should preserve connection properties and group hierarchy.
    #[test]
    fn prop_asbru_export_roundtrip(connections in arb_ssh_connections()) {
        let importer = AsbruImporter::new();

        // Convert test connections to Connection objects
        let conns: Vec<Connection> = connections.iter().map(|c| c.to_connection()).collect();

        // Export to Asbru YAML format
        let exported = AsbruExporter::export(&conns, &[]);

        // Re-import the exported content
        let result = importer.parse_config(&exported, "test");

        // Property: All connections should be re-imported
        prop_assert_eq!(
            result.connections.len(),
            connections.len(),
            "Expected {} connections, got {}. Exported:\n{}",
            connections.len(),
            result.connections.len(),
            exported
        );

        // Property: Each connection should preserve key fields
        for original in &connections {
            let reimported = result
                .connections
                .iter()
                .find(|c| c.name == original.name)
                .expect(&format!("Connection '{}' not found after round-trip", original.name));

            // Host should match
            prop_assert_eq!(
                &reimported.host,
                &original.host,
                "Host mismatch for '{}'. Expected '{}', got '{}'",
                original.name,
                original.host,
                reimported.host
            );

            // Port should match
            prop_assert_eq!(
                reimported.port,
                original.port,
                "Port mismatch for '{}'. Expected {}, got {}",
                original.name,
                original.port,
                reimported.port
            );

            // Username should match
            prop_assert_eq!(
                reimported.username.as_ref(),
                original.username.as_ref(),
                "Username mismatch for '{}'",
                original.name
            );

            // Protocol should be SSH
            prop_assert_eq!(
                reimported.protocol,
                ProtocolType::Ssh,
                "Protocol should be SSH for '{}'",
                original.name
            );
        }
    }

    /// **Feature: ssh-agent-cli, Property 10: Asbru Export Round-Trip (RDP)**
    /// **Validates: Requirements 6.5, 9.5**
    ///
    /// For any RDP connection, exporting to Asbru format and re-importing
    /// should preserve protocol type, hostname, port, and username.
    #[test]
    fn prop_asbru_export_roundtrip_rdp(
        name in arb_connection_name(),
        host in arb_hostname(),
        port in arb_port(),
        username in prop::option::of(arb_username())
    ) {
        let importer = AsbruImporter::new();

        // Create RDP connection
        let mut conn = Connection::new_rdp(name.clone(), host.clone(), port);
        conn.username = username.clone();

        // Export to Asbru YAML format
        let exported = AsbruExporter::export(&[conn], &[]);

        // Re-import the exported content
        let result = importer.parse_config(&exported, "test");

        // Property: Connection should be re-imported
        prop_assert_eq!(
            result.connections.len(),
            1,
            "Expected 1 connection, got {}. Exported:\n{}",
            result.connections.len(),
            exported
        );

        let reimported = &result.connections[0];

        // Property: Name should match
        prop_assert_eq!(
            &reimported.name,
            &name,
            "Name mismatch. Expected '{}', got '{}'. Exported:\n{}",
            name,
            reimported.name,
            exported
        );

        // Property: Host should match
        prop_assert_eq!(
            &reimported.host,
            &host,
            "Host mismatch. Expected '{}', got '{}'. Exported:\n{}",
            host,
            reimported.host,
            exported
        );

        // Property: Port should match
        prop_assert_eq!(
            reimported.port,
            port,
            "Port mismatch. Expected {}, got {}. Exported:\n{}",
            port,
            reimported.port,
            exported
        );

        // Property: Protocol should be RDP
        prop_assert_eq!(
            reimported.protocol,
            ProtocolType::Rdp,
            "Protocol should be RDP. Exported:\n{}",
            exported
        );
    }

    /// **Feature: ssh-agent-cli, Property 10: Asbru Export Round-Trip (VNC)**
    /// **Validates: Requirements 6.5, 9.5**
    ///
    /// For any VNC connection, exporting to Asbru format and re-importing
    /// should preserve protocol type, hostname, and port.
    #[test]
    fn prop_asbru_export_roundtrip_vnc(
        name in arb_connection_name(),
        host in arb_hostname(),
        port in arb_port()
    ) {
        let importer = AsbruImporter::new();

        // Create VNC connection
        let conn = Connection::new_vnc(name.clone(), host.clone(), port);

        // Export to Asbru YAML format
        let exported = AsbruExporter::export(&[conn], &[]);

        // Re-import the exported content
        let result = importer.parse_config(&exported, "test");

        // Property: Connection should be re-imported
        prop_assert_eq!(
            result.connections.len(),
            1,
            "Expected 1 connection, got {}. Exported:\n{}",
            result.connections.len(),
            exported
        );

        let reimported = &result.connections[0];

        // Property: Name should match
        prop_assert_eq!(
            &reimported.name,
            &name,
            "Name mismatch. Expected '{}', got '{}'. Exported:\n{}",
            name,
            reimported.name,
            exported
        );

        // Property: Host should match
        prop_assert_eq!(
            &reimported.host,
            &host,
            "Host mismatch. Expected '{}', got '{}'. Exported:\n{}",
            host,
            reimported.host,
            exported
        );

        // Property: Port should match
        prop_assert_eq!(
            reimported.port,
            port,
            "Port mismatch. Expected {}, got {}. Exported:\n{}",
            port,
            reimported.port,
            exported
        );

        // Property: Protocol should be VNC
        prop_assert_eq!(
            reimported.protocol,
            ProtocolType::Vnc,
            "Protocol should be VNC. Exported:\n{}",
            exported
        );
    }

    /// **Feature: ssh-agent-cli, Property 11: Asbru Export Group Hierarchy**
    /// **Validates: Requirements 6.2, 6.3**
    ///
    /// For any set of connections organized in groups, the exported Asbru YAML
    /// should preserve the parent-child relationships.
    #[test]
    fn prop_asbru_export_group_hierarchy(
        group_name in arb_group_name(),
        connections in arb_ssh_connections()
    ) {
        let importer = AsbruImporter::new();

        // Create a group
        let group = ConnectionGroup::new(group_name.clone());
        let group_id = group.id;

        // Assign all connections to the group
        let conns: Vec<Connection> = connections
            .iter()
            .map(|c| c.to_connection().with_group(group_id))
            .collect();

        // Export to Asbru YAML format
        let exported = AsbruExporter::export(&conns, std::slice::from_ref(&group));

        // Property: Group should be present in output with _is_group: 1
        prop_assert!(
            exported.contains("_is_group: 1"),
            "Group should have _is_group: 1. Exported:\n{}",
            exported
        );

        // Property: Group name should be present
        prop_assert!(
            exported.contains(&format!("name: \"{}\"", group_name)),
            "Group name '{}' should be present. Exported:\n{}",
            group_name,
            exported
        );

        // Property: Connections should have _is_group: 0
        // Count occurrences of _is_group: 0 (should equal number of connections)
        let conn_count = exported.matches("_is_group: 0").count();
        prop_assert_eq!(
            conn_count,
            connections.len(),
            "Expected {} connections with _is_group: 0, found {}. Exported:\n{}",
            connections.len(),
            conn_count,
            exported
        );

        // Property: Connections should have parent field referencing the group
        // Count occurrences of parent: (should equal number of connections)
        let parent_count = exported.matches("parent:").count();
        prop_assert_eq!(
            parent_count,
            connections.len(),
            "Expected {} connections with parent field, found {}. Exported:\n{}",
            connections.len(),
            parent_count,
            exported
        );

        // Re-import and verify group membership is preserved
        let result = importer.parse_config(&exported, "test");

        // Property: Group should be re-imported
        prop_assert_eq!(
            result.groups.len(),
            1,
            "Expected 1 group, got {}. Exported:\n{}",
            result.groups.len(),
            exported
        );

        // Property: All connections should have group_id set
        for reimported in &result.connections {
            prop_assert!(
                reimported.group_id.is_some(),
                "Connection '{}' should have group_id set after round-trip. Exported:\n{}",
                reimported.name,
                exported
            );
        }
    }

    /// **Feature: ssh-agent-cli, Property 11: Asbru Export Nested Group Hierarchy**
    /// **Validates: Requirements 6.2, 6.3**
    ///
    /// For nested groups (parent-child groups), the exported Asbru YAML
    /// should preserve the nested hierarchy.
    #[test]
    fn prop_asbru_export_nested_groups(
        parent_name in arb_group_name(),
        child_name in arb_group_name()
    ) {
        // Create parent and child groups
        let parent_group = ConnectionGroup::new(parent_name.clone());
        let child_group = ConnectionGroup::with_parent(child_name.clone(), parent_group.id);

        // Create a connection in the child group
        let conn = Connection::new_ssh("test_server".to_string(), "192.168.1.1".to_string(), 22)
            .with_group(child_group.id);

        // Export to Asbru YAML format
        let exported = AsbruExporter::export(&[conn], &[parent_group.clone(), child_group.clone()]);

        // Property: Both groups should be present with _is_group: 1
        let group_count = exported.matches("_is_group: 1").count();
        prop_assert_eq!(
            group_count,
            2,
            "Expected 2 groups with _is_group: 1, found {}. Exported:\n{}",
            group_count,
            exported
        );

        // Property: Parent group name should be present
        prop_assert!(
            exported.contains(&format!("name: \"{}\"", parent_name)),
            "Parent group name '{}' should be present. Exported:\n{}",
            parent_name,
            exported
        );

        // Property: Child group name should be present
        prop_assert!(
            exported.contains(&format!("name: \"{}\"", child_name)),
            "Child group name '{}' should be present. Exported:\n{}",
            child_name,
            exported
        );

        // Property: Child group should have parent field
        // (at least 2 parent fields: one for child group, one for connection)
        let parent_count = exported.matches("parent:").count();
        prop_assert!(
            parent_count >= 2,
            "Expected at least 2 parent fields (child group + connection), found {}. Exported:\n{}",
            parent_count,
            exported
        );
    }
}
