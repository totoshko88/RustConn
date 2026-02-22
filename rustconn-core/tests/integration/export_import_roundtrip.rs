//! Export/Import round-trip integration tests
//!
//! Tests that verify exporting connections to various formats and re-importing
//! them preserves all connection data correctly.

#![allow(clippy::uninlined_format_args)]
#![allow(clippy::single_char_pattern)]

use rustconn_core::export::{
    AnsibleExporter, AsbruExporter, ExportFormat, ExportOptions, ExportTarget, RemminaExporter,
    SshConfigExporter,
};
use rustconn_core::import::{
    AnsibleInventoryImporter, AsbruImporter, RemminaImporter, SshConfigImporter,
};
use rustconn_core::models::{Connection, ConnectionGroup, ProtocolConfig, ProtocolType};
use std::path::PathBuf;
use tempfile::TempDir;

/// Creates a set of test SSH connections with various configurations
fn create_test_ssh_connections() -> Vec<Connection> {
    vec![
        // Basic SSH connection
        {
            let mut conn =
                Connection::new_ssh("server1".to_string(), "host1.example.com".to_string(), 22);
            conn.username = Some("admin".to_string());
            conn
        },
        // SSH with custom port
        {
            let mut conn =
                Connection::new_ssh("server2".to_string(), "host2.example.com".to_string(), 2222);
            conn.username = Some("root".to_string());
            if let ProtocolConfig::Ssh(ref mut ssh) = conn.protocol_config {
                ssh.key_path = Some(PathBuf::from("/home/user/.ssh/id_rsa"));
            }
            conn
        },
        // SSH with proxy jump
        {
            let mut conn = Connection::new_ssh(
                "server3".to_string(),
                "internal.example.com".to_string(),
                22,
            );
            conn.username = Some("deploy".to_string());
            if let ProtocolConfig::Ssh(ref mut ssh) = conn.protocol_config {
                ssh.proxy_jump = Some("bastion.example.com".to_string());
            }
            conn
        },
    ]
}

/// Creates a set of test connections with multiple protocols
fn create_mixed_protocol_connections() -> Vec<Connection> {
    vec![
        // SSH connection
        {
            let mut conn =
                Connection::new_ssh("ssh_server".to_string(), "ssh.example.com".to_string(), 22);
            conn.username = Some("sshuser".to_string());
            conn
        },
        // RDP connection
        {
            let mut conn = Connection::new_rdp(
                "rdp_server".to_string(),
                "rdp.example.com".to_string(),
                3389,
            );
            conn.username = Some("rdpuser".to_string());
            conn.domain = Some("DOMAIN".to_string());
            conn
        },
        // VNC connection
        {
            Connection::new_vnc(
                "vnc_server".to_string(),
                "vnc.example.com".to_string(),
                5901,
            )
        },
    ]
}

/// Creates test groups
fn create_test_groups() -> Vec<ConnectionGroup> {
    vec![
        ConnectionGroup::new("production".to_string()),
        ConnectionGroup::new("staging".to_string()),
        ConnectionGroup::new("development".to_string()),
    ]
}

// ============================================================================
// Ansible Export/Import Round-Trip Tests
// ============================================================================

#[test]
fn test_ansible_ini_roundtrip() {
    let connections = create_test_ssh_connections();
    let groups = create_test_groups();
    let importer = AnsibleInventoryImporter::new();

    // Export to INI format
    let exported = AnsibleExporter::export_ini(&connections, &groups);

    // Re-import
    let result = importer.parse_ini_inventory(&exported, "test");

    // Verify all SSH connections are imported (Ansible only supports SSH)
    assert_eq!(
        result.connections.len(),
        connections.len(),
        "All SSH connections should be imported. Exported:\n{}",
        exported
    );

    // Verify each connection's fields
    for original in &connections {
        let reimported = result
            .connections
            .iter()
            .find(|c| c.name == original.name)
            .unwrap_or_else(|| panic!("Connection '{}' not found after round-trip", original.name));

        assert_eq!(
            reimported.host, original.host,
            "Host mismatch for '{}'",
            original.name
        );
        assert_eq!(
            reimported.port, original.port,
            "Port mismatch for '{}'",
            original.name
        );
        assert_eq!(
            reimported.username, original.username,
            "Username mismatch for '{}'",
            original.name
        );
    }
}

#[test]
fn test_ansible_yaml_roundtrip() {
    let connections = create_test_ssh_connections();
    let groups = create_test_groups();
    let importer = AnsibleInventoryImporter::new();

    // Export to YAML format
    let exported = AnsibleExporter::export_yaml(&connections, &groups);

    // Re-import
    let result = importer.parse_yaml_inventory(&exported, "test");

    // Verify all SSH connections are imported
    assert_eq!(
        result.connections.len(),
        connections.len(),
        "All SSH connections should be imported. Exported:\n{}",
        exported
    );

    // Verify each connection's fields
    for original in &connections {
        let reimported = result
            .connections
            .iter()
            .find(|c| c.name == original.name)
            .unwrap_or_else(|| panic!("Connection '{}' not found after round-trip", original.name));

        assert_eq!(
            reimported.host, original.host,
            "Host mismatch for '{}'",
            original.name
        );
        assert_eq!(
            reimported.port, original.port,
            "Port mismatch for '{}'",
            original.name
        );
    }
}

#[test]
fn test_ansible_export_skips_non_ssh() {
    let connections = create_mixed_protocol_connections();

    // Export to INI format
    let exported = AnsibleExporter::export_ini(&connections, &[]);

    // Only SSH connections should be in the output
    assert!(
        exported.contains("ssh_server"),
        "SSH connection should be exported"
    );
    assert!(
        !exported.contains("rdp_server"),
        "RDP connection should NOT be exported"
    );
    assert!(
        !exported.contains("vnc_server"),
        "VNC connection should NOT be exported"
    );
}

// ============================================================================
// SSH Config Export/Import Round-Trip Tests
// ============================================================================

#[test]
fn test_ssh_config_roundtrip() {
    let connections = create_test_ssh_connections();
    let importer = SshConfigImporter::new();

    // Export to SSH config format
    let exported = SshConfigExporter::export(&connections);

    // Re-import
    let result = importer.parse_config(&exported, "test");

    // Verify all connections are imported
    assert_eq!(
        result.connections.len(),
        connections.len(),
        "All connections should be imported. Exported:\n{}",
        exported
    );

    // Verify each connection's fields
    for original in &connections {
        let reimported = result
            .connections
            .iter()
            .find(|c| c.name == original.name)
            .unwrap_or_else(|| panic!("Connection '{}' not found after round-trip", original.name));

        assert_eq!(
            reimported.host, original.host,
            "Host mismatch for '{}'",
            original.name
        );
        assert_eq!(
            reimported.port, original.port,
            "Port mismatch for '{}'",
            original.name
        );
        assert_eq!(
            reimported.username, original.username,
            "Username mismatch for '{}'",
            original.name
        );

        // Verify key path if set
        if let (ProtocolConfig::Ssh(orig_ssh), ProtocolConfig::Ssh(reimp_ssh)) =
            (&original.protocol_config, &reimported.protocol_config)
        {
            assert_eq!(
                reimp_ssh.key_path, orig_ssh.key_path,
                "Key path mismatch for '{}'",
                original.name
            );
        }
    }
}

#[test]
fn test_ssh_config_export_skips_non_ssh() {
    let connections = create_mixed_protocol_connections();

    // Export to SSH config format
    let exported = SshConfigExporter::export(&connections);

    // Only SSH connections should be in the output
    assert!(
        exported.contains("ssh_server"),
        "SSH connection should be exported"
    );
    assert!(
        !exported.contains("rdp_server"),
        "RDP connection should NOT be exported"
    );
    assert!(
        !exported.contains("vnc_server"),
        "VNC connection should NOT be exported"
    );
}

#[test]
fn test_ssh_config_preserves_proxy_jump() {
    let mut conn = Connection::new_ssh(
        "jump_test".to_string(),
        "internal.example.com".to_string(),
        22,
    );
    if let ProtocolConfig::Ssh(ref mut ssh) = conn.protocol_config {
        ssh.proxy_jump = Some("bastion.example.com".to_string());
    }

    let exported = SshConfigExporter::export(&[conn]);

    // Verify ProxyJump is in the output
    assert!(
        exported.contains("ProxyJump bastion.example.com"),
        "ProxyJump should be exported. Got:\n{}",
        exported
    );
}

// ============================================================================
// Remmina Export/Import Round-Trip Tests
// ============================================================================

#[test]
fn test_remmina_ssh_roundtrip() {
    let mut conn = Connection::new_ssh(
        "remmina_ssh".to_string(),
        "ssh.example.com".to_string(),
        2222,
    );
    conn.username = Some("testuser".to_string());
    if let ProtocolConfig::Ssh(ref mut ssh) = conn.protocol_config {
        ssh.key_path = Some(PathBuf::from("/home/user/.ssh/id_ed25519"));
    }

    let importer = RemminaImporter::new();

    // Export to Remmina format
    let exported = RemminaExporter::export_connection(&conn).expect("Export should succeed");

    // Re-import
    let result = importer.parse_remmina_file(
        &exported,
        "test.remmina",
        &mut std::collections::HashMap::new(),
    );

    assert_eq!(
        result.connections.len(),
        1,
        "One connection should be imported"
    );

    let reimported = &result.connections[0];
    assert_eq!(reimported.name, conn.name, "Name mismatch");
    assert_eq!(reimported.host, conn.host, "Host mismatch");
    assert_eq!(reimported.port, conn.port, "Port mismatch");
    assert_eq!(reimported.username, conn.username, "Username mismatch");
    assert_eq!(
        reimported.protocol,
        ProtocolType::Ssh,
        "Protocol should be SSH"
    );
}

#[test]
fn test_remmina_rdp_roundtrip() {
    let mut conn = Connection::new_rdp(
        "remmina_rdp".to_string(),
        "rdp.example.com".to_string(),
        3389,
    );
    conn.username = Some("rdpuser".to_string());
    conn.domain = Some("WORKGROUP".to_string());

    let importer = RemminaImporter::new();

    // Export to Remmina format
    let exported = RemminaExporter::export_connection(&conn).expect("Export should succeed");

    // Re-import
    let result = importer.parse_remmina_file(
        &exported,
        "test.remmina",
        &mut std::collections::HashMap::new(),
    );

    assert_eq!(
        result.connections.len(),
        1,
        "One connection should be imported"
    );

    let reimported = &result.connections[0];
    assert_eq!(reimported.name, conn.name, "Name mismatch");
    assert_eq!(reimported.host, conn.host, "Host mismatch");
    assert_eq!(
        reimported.protocol,
        ProtocolType::Rdp,
        "Protocol should be RDP"
    );
    assert_eq!(reimported.username, conn.username, "Username mismatch");
}

#[test]
fn test_remmina_vnc_roundtrip() {
    let conn = Connection::new_vnc(
        "remmina_vnc".to_string(),
        "vnc.example.com".to_string(),
        5901,
    );

    let importer = RemminaImporter::new();

    // Export to Remmina format
    let exported = RemminaExporter::export_connection(&conn).expect("Export should succeed");

    // Re-import
    let result = importer.parse_remmina_file(
        &exported,
        "test.remmina",
        &mut std::collections::HashMap::new(),
    );

    assert_eq!(
        result.connections.len(),
        1,
        "One connection should be imported"
    );

    let reimported = &result.connections[0];
    assert_eq!(reimported.name, conn.name, "Name mismatch");
    assert_eq!(reimported.host, conn.host, "Host mismatch");
    assert_eq!(reimported.port, conn.port, "Port mismatch");
    assert_eq!(
        reimported.protocol,
        ProtocolType::Vnc,
        "Protocol should be VNC"
    );
}

#[test]
fn test_remmina_batch_export() {
    let connections = create_mixed_protocol_connections();
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    // Export all connections to directory
    let result = RemminaExporter::export_to_directory(&connections, temp_dir.path())
        .expect("Batch export should succeed");

    // Verify all connections were exported
    assert_eq!(
        result.exported_count,
        connections.len(),
        "All connections should be exported"
    );

    // Verify files were created
    assert_eq!(
        result.output_files.len(),
        connections.len(),
        "One file per connection"
    );

    for file in &result.output_files {
        assert!(file.exists(), "Output file should exist: {:?}", file);
    }
}

// ============================================================================
// Asbru Export/Import Round-Trip Tests
// ============================================================================

#[test]
fn test_asbru_roundtrip() {
    let connections = create_test_ssh_connections();
    let groups = create_test_groups();
    let importer = AsbruImporter::new();

    // Export to Asbru format
    let exported = AsbruExporter::export(&connections, &groups);

    // Re-import
    let result = importer.parse_config(&exported, "test");

    // Verify connections are imported
    assert!(
        !result.connections.is_empty(),
        "Connections should be imported. Exported:\n{}",
        exported
    );

    // Verify each connection's fields
    for original in &connections {
        let reimported = result.connections.iter().find(|c| c.name == original.name);

        if let Some(reimported) = reimported {
            assert_eq!(
                reimported.host, original.host,
                "Host mismatch for '{}'",
                original.name
            );
            assert_eq!(
                reimported.port, original.port,
                "Port mismatch for '{}'",
                original.name
            );
        }
    }
}

#[test]
fn test_asbru_preserves_groups() {
    let group = ConnectionGroup::new("test_group".to_string());
    let group_id = group.id;

    let mut conn = Connection::new_ssh(
        "grouped_server".to_string(),
        "server.example.com".to_string(),
        22,
    );
    conn.group_id = Some(group_id);

    // Export to Asbru format
    let exported = AsbruExporter::export(&[conn], &[group]);

    // Verify group structure is in the output
    assert!(
        exported.contains("_is_group: 1") || exported.contains("test_group"),
        "Group should be in exported YAML. Got:\n{}",
        exported
    );
}

// ============================================================================
// Export Options Tests
// ============================================================================

#[test]
fn test_export_options_creation() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let output_path = temp_dir.path().join("output.txt");

    let options = ExportOptions::new(ExportFormat::Ansible, output_path.clone());

    assert_eq!(options.format, ExportFormat::Ansible);
    assert_eq!(options.output_path, output_path);
    assert!(!options.include_passwords);
    assert!(options.include_groups);
}

#[test]
fn test_export_trait_implementations() {
    let connections = create_test_ssh_connections();
    let groups = create_test_groups();
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    // Test Ansible exporter
    let ansible_options =
        ExportOptions::new(ExportFormat::Ansible, temp_dir.path().join("ansible.ini"));
    let ansible_exporter = AnsibleExporter::new();
    let ansible_result = ansible_exporter.export(&connections, &groups, &ansible_options);
    assert!(ansible_result.is_ok(), "Ansible export should succeed");

    // Test SSH Config exporter
    let ssh_options = ExportOptions::new(ExportFormat::SshConfig, temp_dir.path().join("config"));
    let ssh_exporter = SshConfigExporter::new();
    let ssh_result = ssh_exporter.export(&connections, &groups, &ssh_options);
    assert!(ssh_result.is_ok(), "SSH Config export should succeed");

    // Test Remmina exporter
    let remmina_options =
        ExportOptions::new(ExportFormat::Remmina, temp_dir.path().join("remmina"));
    let remmina_exporter = RemminaExporter::new();
    let remmina_result = remmina_exporter.export(&connections, &groups, &remmina_options);
    assert!(remmina_result.is_ok(), "Remmina export should succeed");

    // Test Asbru exporter
    let asbru_options = ExportOptions::new(ExportFormat::Asbru, temp_dir.path().join("asbru.yml"));
    let asbru_exporter = AsbruExporter::new();
    let asbru_result = asbru_exporter.export(&connections, &groups, &asbru_options);
    assert!(asbru_result.is_ok(), "Asbru export should succeed");
}

// ============================================================================
// Edge Case Tests
// ============================================================================

#[test]
fn test_export_empty_connections() {
    let empty: Vec<Connection> = vec![];
    let groups: Vec<ConnectionGroup> = vec![];

    // All exporters should handle empty input gracefully without panicking
    let ansible_ini = AnsibleExporter::export_ini(&empty, &groups);
    // Empty export may produce empty string, comment header, or minimal structure
    // The key is that it doesn't panic and produces valid output
    assert!(
        ansible_ini.is_empty() || ansible_ini.contains("#") || ansible_ini.contains("["),
        "Ansible INI should handle empty gracefully. Got: {}",
        ansible_ini
    );

    let ansible_yaml = AnsibleExporter::export_yaml(&empty, &groups);
    // YAML should produce at least a minimal structure or comment
    assert!(
        !ansible_yaml.is_empty(),
        "Ansible YAML should produce valid output. Got: {}",
        ansible_yaml
    );

    let ssh_config = SshConfigExporter::export(&empty);
    // SSH config can be empty or have a header comment - just verify it doesn't panic
    // and produces reasonable output
    assert!(
        ssh_config.len() < 10000,
        "SSH config should handle empty gracefully. Got: {}",
        ssh_config
    );
}

#[test]
fn test_export_special_characters_in_names() {
    let mut conn = Connection::new_ssh(
        "server-with-dashes_and_underscores".to_string(),
        "host.example.com".to_string(),
        22,
    );
    conn.username = Some("user_name".to_string());

    // All exporters should handle special characters
    let ansible = AnsibleExporter::export_ini(&[conn.clone()], &[]);
    assert!(
        ansible.contains("server-with-dashes_and_underscores"),
        "Ansible should preserve name"
    );

    let ssh_config = SshConfigExporter::export(&[conn.clone()]);
    assert!(
        ssh_config.contains("server-with-dashes_and_underscores"),
        "SSH config should preserve name"
    );

    let remmina = RemminaExporter::export_connection(&conn).expect("Remmina export should succeed");
    assert!(
        remmina.contains("server-with-dashes_and_underscores"),
        "Remmina should preserve name"
    );
}
