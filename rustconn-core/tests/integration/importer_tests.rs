//! Integration tests for import functionality
//!
//! These tests verify that importers can handle real-world data files
//! and edge cases correctly.

use rustconn_core::import::{
    AsbruImporter, RdmImporter, RemminaImporter, RoyalTsImporter, SshConfigImporter,
};
use rustconn_core::models::ProtocolType;

// ============================================================================
// RDM JSON Import Integration Tests
// ============================================================================

#[test]
fn test_rdm_import_real_world_structure() {
    // RDM JSON format uses PascalCase with ID field required
    let rdm_json = r#"{
        "Connections": [
            {
                "ID": "conn-1",
                "Name": "Production Server",
                "ConnectionType": "SSH",
                "Host": "prod.example.com",
                "Port": 22,
                "Username": "admin",
                "PrivateKeyPath": "/home/user/.ssh/id_rsa"
            },
            {
                "ID": "conn-2",
                "Name": "Development RDP",
                "ConnectionType": "RDP",
                "Host": "dev.example.com",
                "Port": 3389,
                "Username": "developer",
                "Domain": "COMPANY"
            },
            {
                "ID": "conn-3",
                "Name": "VNC Desktop",
                "ConnectionType": "VNC",
                "Host": "desktop.example.com",
                "Port": 5901
            },
            {
                "ID": "conn-4",
                "Name": "MySQL Primary",
                "ConnectionType": "SSH",
                "Host": "mysql-primary.example.com",
                "Port": 22,
                "Username": "dbadmin",
                "ParentID": "folder-1"
            },
            {
                "ID": "conn-5",
                "Name": "PostgreSQL",
                "ConnectionType": "SSH",
                "Host": "postgres.example.com",
                "Port": 22,
                "Username": "postgres",
                "ParentID": "folder-1"
            }
        ],
        "Folders": [
            {
                "ID": "folder-1",
                "Name": "Database Servers"
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
    assert_eq!(result.groups.len(), 1, "Should create 1 group");

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
    // RDM JSON format - ID is required, other fields optional
    let rdm_json = r#"{
        "Connections": [
            {
                "ID": "conn-1",
                "Name": "Minimal SSH",
                "ConnectionType": "SSH",
                "Host": "minimal.example.com"
            },
            {
                "ID": "conn-2",
                "Name": "Port Only",
                "ConnectionType": "RDP",
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
    // RDM JSON format with unsupported protocol
    let rdm_json = r#"{
        "Connections": [
            {
                "ID": "conn-1",
                "Name": "Good SSH",
                "ConnectionType": "SSH",
                "Host": "ssh.example.com"
            },
            {
                "ID": "conn-2",
                "Name": "Unsupported FTP",
                "ConnectionType": "FTP",
                "Host": "ftp.example.com"
            },
            {
                "ID": "conn-3",
                "Name": "Good RDP",
                "ConnectionType": "RDP",
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
    assert!(!connection_names.contains(&"Unsupported FTP"));
}

// ============================================================================
// Royal TS Import Integration Tests
// ============================================================================

#[test]
fn test_royal_ts_import_real_world_rtsz() {
    // Royal TS importer uses XML parsing directly via parse_xml method
    let royal_xml = r#"<?xml version="1.0" encoding="utf-8"?>
<RoyalDocument>
  <RoyalFolder>
    <ID>prod-folder</ID>
    <Name>Production</Name>
  </RoyalFolder>
  <RoyalFolder>
    <ID>dev-folder</ID>
    <Name>Development</Name>
  </RoyalFolder>
  <RoyalSSHConnection>
    <ID>web-ssh</ID>
    <Name>Web Server</Name>
    <URI>web.example.com</URI>
    <Port>22</Port>
    <ParentID>prod-folder</ParentID>
  </RoyalSSHConnection>
  <RoyalRDPConnection>
    <ID>db-rdp</ID>
    <Name>Database Server</Name>
    <URI>db.example.com</URI>
    <Port>3389</Port>
    <ParentID>prod-folder</ParentID>
  </RoyalRDPConnection>
  <RoyalVNCConnection>
    <ID>dev-vnc</ID>
    <Name>Dev Desktop</Name>
    <URI>dev-desktop.example.com</URI>
    <Port>5901</Port>
    <ParentID>dev-folder</ParentID>
  </RoyalVNCConnection>
</RoyalDocument>"#;

    let importer = RoyalTsImporter::new();
    let result = importer.parse_xml(royal_xml, "test.rtsz");

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

    // Verify RDP connection
    let rdp_conn = result
        .connections
        .iter()
        .find(|c| c.name == "Database Server")
        .expect("Database Server should be imported");

    assert_eq!(rdp_conn.protocol, ProtocolType::Rdp);

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
    // Royal TS XML with credentials
    let royal_xml = r#"<?xml version="1.0" encoding="utf-8"?>
<RoyalDocument>
  <RoyalCredential>
    <ID>cred-1</ID>
    <Name>SSH Key User</Name>
    <UserName>keyuser</UserName>
  </RoyalCredential>
  <RoyalCredential>
    <ID>cred-2</ID>
    <Name>Domain User</Name>
    <UserName>domainuser</UserName>
    <Domain>WORKGROUP</Domain>
  </RoyalCredential>
  <RoyalSSHConnection>
    <ID>ssh-key</ID>
    <Name>SSH with Key</Name>
    <URI>ssh.example.com</URI>
    <Port>22</Port>
    <CredentialId>cred-1</CredentialId>
  </RoyalSSHConnection>
  <RoyalRDPConnection>
    <ID>rdp-domain</ID>
    <Name>RDP with Domain</Name>
    <URI>rdp.example.com</URI>
    <Port>3389</Port>
    <CredentialId>cred-2</CredentialId>
  </RoyalRDPConnection>
</RoyalDocument>"#;

    let importer = RoyalTsImporter::new();
    let result = importer.parse_xml(royal_xml, "creds.rtsz");

    assert_eq!(result.connections.len(), 2);

    // Verify SSH connection with credential
    let ssh_conn = result
        .connections
        .iter()
        .find(|c| c.name == "SSH with Key")
        .expect("SSH with Key should be imported");

    assert_eq!(ssh_conn.username, Some("keyuser".to_string()));

    // Verify RDP connection with domain credential
    let rdp_conn = result
        .connections
        .iter()
        .find(|c| c.name == "RDP with Domain")
        .expect("RDP with Domain should be imported");

    assert_eq!(rdp_conn.username, Some("domainuser".to_string()));
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
    // Asbru YAML format uses _is_group field
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
  port: 22
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
        )
        .expect("Failed to write to string");
    }

    let ssh_importer = SshConfigImporter::new();
    let start = std::time::Instant::now();
    let result = ssh_importer.parse_config(&large_ssh_config, "large_config");
    let duration = start.elapsed();

    assert_eq!(result.connections.len(), 1000);
    // Should complete within reasonable time (adjust threshold as needed)
    assert!(duration.as_secs() < 5, "Import took too long: {duration:?}");
}
