//! Test fixtures for import/export testing.
//!
//! This module provides sample connections and helper functions for testing
//! import/export functionality across different formats.

use std::path::PathBuf;

use rustconn_core::models::{
    Connection, ConnectionGroup, ProtocolConfig, RdpConfig, Resolution, SshAuthMethod, SshConfig,
    SshKeySource, VncConfig,
};

/// Creates a sample SSH connection with key file authentication.
///
/// This represents a typical production SSH server with:
/// - Public key authentication
/// - Custom key file path
/// - Standard SSH port
#[must_use]
pub fn sample_ssh_connection_with_key() -> Connection {
    let ssh_config = SshConfig {
        auth_method: SshAuthMethod::PublicKey,
        key_path: Some(PathBuf::from("~/.ssh/id_ed25519")),
        key_source: SshKeySource::Default,
        agent_key_fingerprint: None,
        identities_only: false,
        proxy_jump: None,
        use_control_master: false,
        agent_forwarding: false,
        custom_options: std::collections::HashMap::new(),
        startup_command: None,
        jump_host_id: None,
    };

    let mut conn = Connection::new(
        "web-server".to_string(),
        "192.168.1.10".to_string(),
        22,
        ProtocolConfig::Ssh(ssh_config),
    );
    conn.username = Some("deploy".to_string());
    conn
}

/// Creates a sample SSH connection with custom port.
///
/// This represents a database server with:
/// - Password authentication
/// - Non-standard SSH port (2222)
#[must_use]
pub fn sample_ssh_connection_custom_port() -> Connection {
    let ssh_config = SshConfig {
        auth_method: SshAuthMethod::Password,
        key_path: None,
        key_source: SshKeySource::Default,
        agent_key_fingerprint: None,
        identities_only: false,
        proxy_jump: None,
        use_control_master: false,
        agent_forwarding: false,
        custom_options: std::collections::HashMap::new(),
        startup_command: None,
        jump_host_id: None,
    };

    let mut conn = Connection::new(
        "db-server".to_string(),
        "10.0.0.50".to_string(),
        2222,
        ProtocolConfig::Ssh(ssh_config),
    );
    conn.username = Some("postgres".to_string());
    conn
}

/// Creates a sample SSH connection with proxy jump.
///
/// This represents an internal server accessible via bastion host.
#[must_use]
pub fn sample_ssh_connection_with_proxy() -> Connection {
    let ssh_config = SshConfig {
        auth_method: SshAuthMethod::PublicKey,
        key_path: Some(PathBuf::from("~/.ssh/internal_key")),
        key_source: SshKeySource::Default,
        agent_key_fingerprint: None,
        identities_only: false,
        proxy_jump: Some("bastion.example.com".to_string()),
        use_control_master: false,
        agent_forwarding: false,
        custom_options: std::collections::HashMap::new(),
        startup_command: None,
        jump_host_id: None,
    };

    let mut conn = Connection::new(
        "internal-server".to_string(),
        "10.10.0.5".to_string(),
        22,
        ProtocolConfig::Ssh(ssh_config),
    );
    conn.username = Some("admin".to_string());
    conn
}

/// Creates a sample RDP connection with domain.
///
/// This represents a Windows server with:
/// - Domain authentication
/// - Custom resolution
/// - Audio redirection enabled
#[must_use]
pub fn sample_rdp_connection_with_domain() -> Connection {
    let rdp_config = RdpConfig {
        resolution: Some(Resolution::new(1920, 1080)),
        color_depth: Some(32),
        audio_redirect: true,
        gateway: None,
        shared_folders: Vec::new(),
        custom_args: Vec::new(),
        client_mode: Default::default(),
        performance_mode: Default::default(),
    };

    let mut conn = Connection::new(
        "windows-server".to_string(),
        "192.168.1.100".to_string(),
        3389,
        ProtocolConfig::Rdp(rdp_config),
    );
    conn.username = Some("Administrator".to_string());
    conn.domain = Some("CORP".to_string());
    conn
}

/// Creates a sample VNC connection with custom port.
///
/// This represents a VNC desktop with:
/// - Non-standard VNC port (5901 = display :1)
#[must_use]
pub fn sample_vnc_connection_custom_port() -> Connection {
    let vnc_config = VncConfig {
        client_mode: Default::default(),
        performance_mode: Default::default(),
        encoding: Some("tight".to_string()),
        compression: Some(6),
        quality: Some(8),
        view_only: false,
        scaling: true,
        clipboard_enabled: true,
        custom_args: Vec::new(),
    };

    Connection::new(
        "vnc-desktop".to_string(),
        "192.168.1.75".to_string(),
        5901,
        ProtocolConfig::Vnc(vnc_config),
    )
}

/// Creates a sample connection group for organizing connections.
#[must_use]
pub fn sample_production_group() -> ConnectionGroup {
    ConnectionGroup::new("Production".to_string())
}

/// Creates a sample connection group for development.
#[must_use]
pub fn sample_development_group() -> ConnectionGroup {
    ConnectionGroup::new("Development".to_string())
}

/// Returns all sample connections as a vector.
///
/// Useful for batch testing export functionality.
#[must_use]
pub fn all_sample_connections() -> Vec<Connection> {
    vec![
        sample_ssh_connection_with_key(),
        sample_ssh_connection_custom_port(),
        sample_ssh_connection_with_proxy(),
        sample_rdp_connection_with_domain(),
        sample_vnc_connection_custom_port(),
    ]
}

/// Returns all sample groups as a vector.
///
/// Reserved for future tests that need to verify group-related functionality
/// without the associated connections.
#[must_use]
#[allow(dead_code)]
pub fn all_sample_groups() -> Vec<ConnectionGroup> {
    vec![sample_production_group(), sample_development_group()]
}

/// Returns sample connections organized by groups.
///
/// Returns (connections, groups) where connections have their group_id set.
#[must_use]
pub fn sample_connections_with_groups() -> (Vec<Connection>, Vec<ConnectionGroup>) {
    let prod_group = sample_production_group();
    let dev_group = sample_development_group();

    let mut ssh_key = sample_ssh_connection_with_key();
    ssh_key.group_id = Some(prod_group.id);

    let mut ssh_port = sample_ssh_connection_custom_port();
    ssh_port.group_id = Some(prod_group.id);

    let mut rdp = sample_rdp_connection_with_domain();
    rdp.group_id = Some(dev_group.id);

    let mut vnc = sample_vnc_connection_custom_port();
    vnc.group_id = Some(dev_group.id);

    let connections = vec![ssh_key, ssh_port, rdp, vnc];
    let groups = vec![prod_group, dev_group];

    (connections, groups)
}

/// Path to the Ansible inventory test fixture.
#[must_use]
pub fn ansible_inventory_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("ansible_inventory.ini")
}

/// Path to the SSH config test fixture.
#[must_use]
pub fn ssh_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("ssh_config")
}

/// Path to the Remmina test fixtures directory.
#[must_use]
pub fn remmina_fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("remmina")
}

/// Path to the Asbru test fixture.
#[must_use]
pub fn asbru_config_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("asbru.yml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sample_ssh_connection_with_key() {
        let conn = sample_ssh_connection_with_key();
        assert_eq!(conn.name, "web-server");
        assert_eq!(conn.host, "192.168.1.10");
        assert_eq!(conn.port, 22);
        assert_eq!(conn.username, Some("deploy".to_string()));

        if let ProtocolConfig::Ssh(ssh) = &conn.protocol_config {
            assert_eq!(ssh.auth_method, SshAuthMethod::PublicKey);
            assert!(ssh.key_path.is_some());
        } else {
            panic!("Expected SSH protocol config");
        }
    }

    #[test]
    fn test_sample_rdp_connection_with_domain() {
        let conn = sample_rdp_connection_with_domain();
        assert_eq!(conn.name, "windows-server");
        assert_eq!(conn.host, "192.168.1.100");
        assert_eq!(conn.port, 3389);
        assert_eq!(conn.domain, Some("CORP".to_string()));

        if let ProtocolConfig::Rdp(rdp) = &conn.protocol_config {
            assert!(rdp.resolution.is_some());
            assert_eq!(rdp.color_depth, Some(32));
        } else {
            panic!("Expected RDP protocol config");
        }
    }

    #[test]
    fn test_sample_vnc_connection_custom_port() {
        let conn = sample_vnc_connection_custom_port();
        assert_eq!(conn.name, "vnc-desktop");
        assert_eq!(conn.host, "192.168.1.75");
        assert_eq!(conn.port, 5901);

        assert!(matches!(conn.protocol_config, ProtocolConfig::Vnc(_)));
    }

    #[test]
    fn test_all_sample_connections() {
        let connections = all_sample_connections();
        assert_eq!(connections.len(), 5);
    }

    #[test]
    fn test_sample_connections_with_groups() {
        let (connections, groups) = sample_connections_with_groups();
        assert_eq!(connections.len(), 4);
        assert_eq!(groups.len(), 2);

        // Verify all connections have group_id set
        for conn in &connections {
            assert!(conn.group_id.is_some());
        }
    }

    #[test]
    fn test_fixture_paths_exist() {
        // These tests verify the fixture files were created correctly
        assert!(
            ansible_inventory_path().exists(),
            "Ansible inventory fixture should exist"
        );
        assert!(
            ssh_config_path().exists(),
            "SSH config fixture should exist"
        );
        assert!(
            remmina_fixtures_dir().exists(),
            "Remmina fixtures directory should exist"
        );
        assert!(
            asbru_config_path().exists(),
            "Asbru config fixture should exist"
        );
    }
}
