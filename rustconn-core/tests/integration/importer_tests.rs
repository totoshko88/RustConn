//! Integration tests for import functionality
//!
//! These tests verify that importers can handle real-world data files
//! and edge cases correctly.

use rustconn_core::import::{
    AsbruImporter, ImportSource, RdmImporter, RemminaImporter, RoyalTsImporter, SshConfigImporter,
};
use rustconn_core::models::{ProtocolType, SshAuthMethod};
use std::path::PathBuf;
use tempfile::TempDir;

// ============================================================================
// RDM JSON Import Integration Tests
// ============================================================================

#[test]
fn test_rdm_import_real_world_structure() {
    let rdm_json = r#"{
        "Name": "My Connections",
        "Kind": "Group",
        "Connections": [
            {
                "Name": "Production Server",
                "Kind": "SSH",
                "Host": "prod.example.com",
                "Port": 22,
                "Username": "admin",
                "PrivateKeyFile": "/home/user/.ssh/id_rsa"
            },
            {
                "Name": "Development RDP",
                "Kind": "RDP",
                "Host": "dev.example.com",
                "Port": 3389,
                "Username": "developer",
                "Domain": "COMPANY"
            },
            {
                "Name": "VNC Desktop",
                "Kind": "VNC",
                "Host": "desktop.example.com",
                "Port": 5901
            }
        ],
        "Groups": [
            {
                "Name": "Database Servers",
                "Kind": "Group",
                "Connections": [
                    {
                        "Name": "MySQL Primary",
                        "Kind": "SSH",
                        "Host": "mysql-primary.example.com",
                        "Port": 22,
                        "Username": "dbadmin"
                    },
                    {
                        "Name": "PostgreSQL",
                        "Kind": "SSH", 
                        "Host": "postgres.example.com",
                        "Port": 22,
                        "Username": "postgres"
                    }
                ]
            }
        ]
    }"#;

    let importer = RdmImporter::new();
    let result = importer
        .import_from_content(rdm_json)
        .expect("Import should succeed");

    // Should import all connections
    assert_eq!(result.connections.len(), 5, "Should import 5 connections");

    // Should create groups
    assert_eq!(
        result.groups.len(),
        2,
        "Should create 2 groups (root + Database Servers)"
    );

    // Verify specific connections
    let prod_server = result
        .connections
        .iter()
        .find(|c| c.name == "Production Server")
        .expect("Production Server should be imported");

    assert_eq!(prod_server.host, "prod.example.com");
    assert_eq!(prod_server.port, 22);
    assert_eq!(prod_server.username, Some("admin".to_string()));
    assert_eq!(prod_server.protocol, ProtocolType::Ssh);

    let rdp_conn = result
        .connections
        .iter()
        .find(|c| c.name == "Development RDP")
        .expect("Development RDP should be imported");

    assert_eq!(rdp_conn.protocol, ProtocolType::Rdp);
    assert_eq!(rdp_conn.domain, Some("COMPANY".to_string()));

    // Verify nested group connections have correct group_id
    let mysql_conn = result
        .connections
        .iter()
        .find(|c| c.name == "MySQL Primary")
        .expect("MySQL Primary should be imported");

    let db_group = result
        .groups
        .iter()
        .find(|g| g.name == "Database Servers")
        .expect("Database Servers group should exist");

    assert_eq!(mysql_conn.group_id, Some(db_group.id));
}

#[test]
fn test_rdm_import_handles_missing_fields() {
    let rdm_json = r#"{
        "Name": "Minimal",
        "Kind": "Group",
        "Connections": [
            {
                "Name": "Minimal SSH",
                "Kind": "SSH",
                "Host": "minimal.example.com"
            },
            {
                "Name": "Port Only",
                "Kind": "RDP",
                "Host": "rdp.example.com",
                "Port": 3390
            }
        ]
    }"#;

    let importer = RdmImporter::new();
    let result = importer
        .import_from_content(rdm_json)
        .expect("Import should succeed");

    assert_eq!(result.connections.len(), 2);

    let ssh_conn = result
        .connections
        .iter()
        .find(|c| c.name == "Minimal SSH")
        .expect("Minimal SSH should be imported");

    // Should use default port for SSH
    assert_eq!(ssh_conn.port, 22);
    assert_eq!(ssh_conn.username, None);

    let rdp_conn = result
        .connections
        .iter()
        .find(|c| c.name == "Port Only")
        .expect("Port Only should be imported");

    assert_eq!(rdp_conn.port, 3390);
}

#[test]
fn test_rdm_import_skips_unsupported_protocols() {
    let rdm_json = r#"{
        "Name": "Mixed",
        "Kind": "Group", 
        "Connections": [
            {
                "Name": "Good SSH",
                "Kind": "SSH",
                "Host": "ssh.example.com"
            },
            {
                "Name": "Unsupported Telnet",
                "Kind": "Telnet",
                "Host": "telnet.example.com"
            },
            {
                "Name": "Good RDP",
                "Kind": "RDP",
                "Host": "rdp.example.com"
            }
        ]
    }"#;

    let importer = RdmImporter::new();
    let result = importer
        .import_from_content(rdm_json)
        .expect("Import should succeed");

    // Should import only supported protocols
    assert_eq!(result.connections.len(), 2);
    assert_eq!(result.skipped.len(), 1);

    let connection_names: Vec<&str> = result.connections.iter().map(|c| c.name.as_str()).collect();

    assert!(connection_names.contains(&"Good SSH"));
    assert!(connection_names.contains(&"Good RDP"));
    assert!(!connection_names.contains(&"Unsupported Telnet"));
}

// ============================================================================
// Royal TS Import Integration Tests
// ============================================================================

#[test]
fn test_royal_ts_import_real_world_rtsz() {
    // Create a temporary .rtsz file (ZIP with XML)
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let rtsz_path = temp_dir.path().join("test.rtsz");

    let royal_xml = r#"<?xml version="1.0" encoding="utf-8"?>
<RoyalDocument>
  <RoyalFolder Name="Root" ID="root-id">
    <RoyalFolder Name="Production" ID="prod-folder">
      <RoyalSSHConnection Name="Web Server" ID="web-ssh" 
                          URI="web.example.com" Port="22" 
                          UserName="admin" />
      <RoyalRDPConnection Name="Database Server" ID="db-rdp"
                          URI="db.example.com" Port="3389"
                          UserName="dbadmin" Domain="COMPANY" />
    </RoyalFolder>
    <RoyalFolder Name="Development" ID="dev-folder">
      <RoyalVNCConnection Name="Dev Desktop" ID="dev-vnc"
                          URI="dev-desktop.example.com" Port="5901" />
    </RoyalFolder>
  </RoyalFolder>
</RoyalDocument>"#;

    // Create ZIP file with XML content
    {
        use std::fs::File;
        use std::io::Write;
        use zip::write::FileOptions;
        use zip::ZipWriter;

        let file = File::create(&rtsz_path).expect("Failed to create ZIP file");
        let mut zip = ZipWriter::new(file);

        zip.start_file("document.xml", FileOptions::<()>::default())
            .expect("Failed to start ZIP entry");
        zip.write_all(royal_xml.as_bytes())
            .expect("Failed to write XML to ZIP");
        zip.finish().expect("Failed to finish ZIP");
    }

    let importer = RoyalTsImporter::new();
    let result = importer
        .import_from_path(&rtsz_path)
        .expect("Import should succeed");

    // Should import all connections
    assert_eq!(result.connections.len(), 3);

    // Should create folder structure
    assert_eq!(result.groups.len(), 2); // Production + Development

    // Verify SSH connection
    let ssh_conn = result
        .connections
        .iter()
        .find(|c| c.name == "Web Server")
        .expect("Web Server should be imported");

    assert_eq!(ssh_conn.protocol, ProtocolType::Ssh);
    assert_eq!(ssh_conn.host, "web.example.com");
    assert_eq!(ssh_conn.port, 22);
    assert_eq!(ssh_conn.username, Some("admin".to_string()));

    // Verify RDP connection
    let rdp_conn = result
        .connections
        .iter()
        .find(|c| c.name == "Database Server")
        .expect("Database Server should be imported");

    assert_eq!(rdp_conn.protocol, ProtocolType::Rdp);
    assert_eq!(rdp_conn.domain, Some("COMPANY".to_string()));

    // Verify VNC connection
    let vnc_conn = result
        .connections
        .iter()
        .find(|c| c.name == "Dev Desktop")
        .expect("Dev Desktop should be imported");

    assert_eq!(vnc_conn.protocol, ProtocolType::Vnc);
    assert_eq!(vnc_conn.port, 5901);
}

#[test]
fn test_royal_ts_import_handles_credentials() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let rtsz_path = temp_dir.path().join("creds.rtsz");

    let royal_xml = r#"<?xml version="1.0" encoding="utf-8"?>
<RoyalDocument>
  <RoyalFolder Name="Root" ID="root-id">
    <RoyalSSHConnection Name="SSH with Key" ID="ssh-key"
                        URI="ssh.example.com" Port="22"
                        UserName="keyuser" 
                        PrivateKeyFile="/path/to/key.pem" />
    <RoyalRDPConnection Name="RDP with Domain" ID="rdp-domain"
                        URI="rdp.example.com" Port="3389"
                        UserName="domainuser" Domain="WORKGROUP" />
  </RoyalFolder>
</RoyalDocument>"#;

    // Create ZIP file
    {
        use std::fs::File;
        use std::io::Write;
        use zip::write::FileOptions;
        use zip::ZipWriter;

        let file = File::create(&rtsz_path).expect("Failed to create ZIP file");
        let mut zip = ZipWriter::new(file);

        zip.start_file("document.xml", FileOptions::<()>::default())
            .expect("Failed to start ZIP entry");
        zip.write_all(royal_xml.as_bytes())
            .expect("Failed to write XML to ZIP");
        zip.finish().expect("Failed to finish ZIP");
    }

    let importer = RoyalTsImporter::new();
    let result = importer
        .import_from_path(&rtsz_path)
        .expect("Import should succeed");

    // Verify SSH key authentication
    let ssh_conn = result
        .connections
        .iter()
        .find(|c| c.name == "SSH with Key")
        .expect("SSH with Key should be imported");

    if let rustconn_core::models::ProtocolConfig::Ssh(ssh_config) = &ssh_conn.protocol_config {
        assert_eq!(ssh_config.auth_method, SshAuthMethod::PublicKey);
        assert_eq!(ssh_config.key_path, Some(PathBuf::from("/path/to/key.pem")));
    } else {
        panic!("Expected SSH protocol config");
    }

    // Verify RDP domain
    let rdp_conn = result
        .connections
        .iter()
        .find(|c| c.name == "RDP with Domain")
        .expect("RDP with Domain should be imported");

    assert_eq!(rdp_conn.domain, Some("WORKGROUP".to_string()));
}

// ============================================================================
// SSH Config Import Edge Cases
// ============================================================================

#[test]
fn test_ssh_config_import_complex_real_world() {
    let ssh_config = r"
# Global settings
Host *
    ServerAliveInterval 60
    ServerAliveCountMax 3

# Production bastion
Host bastion
    HostName bastion.prod.example.com
    User admin
    Port 2222
    IdentityFile ~/.ssh/prod_key
    ForwardAgent yes

# Internal servers via bastion
Host prod-web
    HostName web.internal.example.com
    User webadmin
    ProxyJump bastion
    IdentityFile ~/.ssh/web_key

Host prod-db
    HostName db.internal.example.com
    User dbadmin
    ProxyJump admin@bastion.prod.example.com:2222

# Development servers
Host dev-*
    User developer
    IdentityFile ~/.ssh/dev_key

Host dev-web
    HostName dev-web.example.com
    Port 2223

# Skip wildcard patterns
Host *.local
    User localuser
";

    let importer = SshConfigImporter::new();
    let result = importer.parse_config(ssh_config, "complex_config");

    // Should import specific hosts, skip wildcards
    let imported_names: Vec<&str> = result.connections.iter().map(|c| c.name.as_str()).collect();

    assert!(imported_names.contains(&"bastion"));
    assert!(imported_names.contains(&"prod-web"));
    assert!(imported_names.contains(&"prod-db"));
    assert!(imported_names.contains(&"dev-web"));

    // Should skip wildcard patterns
    assert!(!imported_names.iter().any(|name| name.contains('*')));
    assert!(!imported_names.iter().any(|name| name.contains(".local")));

    // Verify bastion configuration
    let bastion = result
        .connections
        .iter()
        .find(|c| c.name == "bastion")
        .expect("Bastion should be imported");

    assert_eq!(bastion.host, "bastion.prod.example.com");
    assert_eq!(bastion.port, 2222);
    assert_eq!(bastion.username, Some("admin".to_string()));

    // Verify proxy jump configuration
    let prod_web = result
        .connections
        .iter()
        .find(|c| c.name == "prod-web")
        .expect("prod-web should be imported");

    if let rustconn_core::models::ProtocolConfig::Ssh(ssh_config) = &prod_web.protocol_config {
        assert_eq!(ssh_config.proxy_jump, Some("bastion".to_string()));
    } else {
        panic!("Expected SSH protocol config");
    }

    let prod_db = result
        .connections
        .iter()
        .find(|c| c.name == "prod-db")
        .expect("prod-db should be imported");

    if let rustconn_core::models::ProtocolConfig::Ssh(ssh_config) = &prod_db.protocol_config {
        assert_eq!(
            ssh_config.proxy_jump,
            Some("admin@bastion.prod.example.com:2222".to_string())
        );
    } else {
        panic!("Expected SSH protocol config");
    }
}

// ============================================================================
// Asbru Import Edge Cases
// ============================================================================

#[test]
fn test_asbru_import_handles_dynamic_variables() {
    let asbru_yaml = r#"
server1:
  _is_group: 0
  name: "Dynamic Server"
  ip: "${SERVER_IP}"
  method: "SSH"
  user: "${USERNAME}"
  port: 22

server2:
  _is_group: 0
  name: "Mixed Variables"
  ip: "static.example.com"
  method: "SSH"
  user: "${DEPLOY_USER}"
  port: "${SSH_PORT}"
"#;

    let importer = AsbruImporter::new();
    let result = importer.parse_config(asbru_yaml, "dynamic.yml");

    assert_eq!(result.connections.len(), 2);

    // Verify dynamic variables are preserved
    let dynamic_server = result
        .connections
        .iter()
        .find(|c| c.name == "Dynamic Server")
        .expect("Dynamic Server should be imported");

    assert_eq!(dynamic_server.host, "${SERVER_IP}");
    assert_eq!(dynamic_server.username, Some("${USERNAME}".to_string()));

    let mixed_server = result
        .connections
        .iter()
        .find(|c| c.name == "Mixed Variables")
        .expect("Mixed Variables should be imported");

    assert_eq!(mixed_server.host, "static.example.com");
    assert_eq!(mixed_server.username, Some("${DEPLOY_USER}".to_string()));
    // Port variables should be handled gracefully (default to protocol default)
    assert_eq!(mixed_server.port, 22); // Default SSH port when variable can't be parsed
}

#[test]
fn test_asbru_import_nested_groups() {
    let asbru_yaml = r#"
root-group:
  _is_group: 1
  name: "Root Group"
  children: {}

parent-group:
  _is_group: 1
  name: "Parent Group"
  parent: "root-group"
  children: {}

child-group:
  _is_group: 1
  name: "Child Group"
  parent: "parent-group"
  children: {}

server1:
  _is_group: 0
  name: "Nested Server"
  ip: "nested.example.com"
  method: "SSH"
  parent: "child-group"
"#;

    let importer = AsbruImporter::new();
    let result = importer.parse_config(asbru_yaml, "nested.yml");

    // Should create all groups
    assert_eq!(result.groups.len(), 3);

    // Should import connection with correct group assignment
    assert_eq!(result.connections.len(), 1);

    let nested_server = &result.connections[0];
    assert_eq!(nested_server.name, "Nested Server");

    // Should be assigned to the child group
    let child_group = result
        .groups
        .iter()
        .find(|g| g.name == "Child Group")
        .expect("Child Group should exist");

    assert_eq!(nested_server.group_id, Some(child_group.id));
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[test]
fn test_importers_handle_malformed_input() {
    // Test each importer with malformed input

    // RDM with invalid JSON
    let rdm_importer = RdmImporter::new();
    let rdm_result = rdm_importer.import_from_content("{ invalid json");
    assert!(rdm_result.is_err());

    // SSH config with malformed entries
    let ssh_importer = SshConfigImporter::new();
    let ssh_result = ssh_importer.parse_config("Host\n  InvalidLine", "bad_config");
    // Should not panic, may skip malformed entries
    assert!(ssh_result.connections.is_empty() || !ssh_result.skipped.is_empty());

    // Asbru with invalid YAML
    let asbru_importer = AsbruImporter::new();
    let asbru_result = asbru_importer.parse_config("invalid: yaml: structure:", "bad.yml");
    assert!(asbru_result.connections.is_empty());

    // Remmina with missing required fields
    let remmina_importer = RemminaImporter::new();
    let remmina_result =
        remmina_importer.parse_remmina_file("[remmina]\nprotocol=SSH", "bad.remmina");
    // Should handle missing server field gracefully
    assert!(remmina_result.connections.is_empty() || !remmina_result.skipped.is_empty());
}

#[test]
fn test_importers_handle_empty_input() {
    // All importers should handle empty input gracefully

    let rdm_importer = RdmImporter::new();
    let rdm_result = rdm_importer
        .import_from_content("{}")
        .expect("Should handle empty JSON");
    assert!(rdm_result.connections.is_empty());
    assert!(rdm_result.groups.is_empty());

    let ssh_importer = SshConfigImporter::new();
    let ssh_result = ssh_importer.parse_config("", "empty_config");
    assert!(ssh_result.connections.is_empty());

    let asbru_importer = AsbruImporter::new();
    let asbru_result = asbru_importer.parse_config("", "empty.yml");
    assert!(asbru_result.connections.is_empty());

    let remmina_importer = RemminaImporter::new();
    let remmina_result = remmina_importer.parse_remmina_file("", "empty.remmina");
    assert!(remmina_result.connections.is_empty());
}

// ============================================================================
// Performance Tests
// ============================================================================

#[test]
fn test_large_import_performance() {
    // Test with a reasonably large dataset to ensure importers scale
    let mut large_ssh_config = String::new();

    for i in 0..1000 {
        use std::fmt::Write;
        write!(
            large_ssh_config,
            "Host server{i}\n    HostName server{i}.example.com\n    Port 22\n    User admin\n\n"
        ).expect("Failed to write to string");
    }

    let ssh_importer = SshConfigImporter::new();
    let start = std::time::Instant::now();
    let result = ssh_importer.parse_config(&large_ssh_config, "large_config");
    let duration = start.elapsed();

    assert_eq!(result.connections.len(), 1000);
    // Should complete within reasonable time (adjust threshold as needed)
    assert!(
        duration.as_secs() < 5,
        "Import took too long: {duration:?}"
    );
}
