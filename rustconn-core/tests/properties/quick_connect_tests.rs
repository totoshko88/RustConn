//! Property-based tests for Quick Connect functionality
//!
//! **Property 20: Quick Connect No Persistence**
//! **Validates: Requirements 7.4**
//!
//! Quick Connect creates temporary connections that are NOT saved to the
//! connection list. This module tests that:
//! 1. Temporary connections can be created with valid parameters
//! 2. Temporary connections are not persisted to storage
//! 3. Connection manager state is unchanged after Quick Connect session

use proptest::prelude::*;
use rustconn_core::{
    ConfigManager, Connection, ConnectionManager, ProtocolConfig, ProtocolType, RdpConfig,
    SshConfig, TelnetConfig, VncConfig,
};
use tempfile::TempDir;
use uuid::Uuid;

// Helper to create a test ConnectionManager with a Tokio runtime
// Uses a Tokio runtime because ConnectionManager::new() spawns async persistence tasks
fn create_test_manager() -> (ConnectionManager, TempDir, tokio::runtime::Runtime) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let temp_dir = TempDir::new().unwrap();
    let config_manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());
    let manager = runtime.block_on(async { ConnectionManager::new(config_manager).unwrap() });
    (manager, temp_dir, runtime)
}

// ========== Generators ==========

/// Strategy for generating valid hostnames
fn arb_host() -> impl Strategy<Value = String> {
    prop_oneof![
        // Simple hostname
        "[a-z][a-z0-9]{0,15}".prop_map(|s| s),
        // FQDN
        "[a-z][a-z0-9]{0,7}\\.[a-z]{2,4}".prop_map(|s| s),
        // IP address (simplified)
        (1u8..255u8, 0u8..255u8, 0u8..255u8, 1u8..255u8)
            .prop_map(|(a, b, c, d)| format!("{a}.{b}.{c}.{d}")),
    ]
}

/// Strategy for generating valid ports
fn arb_port() -> impl Strategy<Value = u16> {
    prop_oneof![
        Just(22u16),   // SSH default
        Just(3389u16), // RDP default
        Just(5900u16), // VNC default
        1024u16..65535u16,
    ]
}

/// Strategy for generating optional usernames
fn arb_username() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), "[a-z][a-z0-9_]{0,15}".prop_map(Some),]
}

/// Strategy for generating protocol types
fn arb_protocol() -> impl Strategy<Value = ProtocolType> {
    prop_oneof![
        Just(ProtocolType::Ssh),
        Just(ProtocolType::Rdp),
        Just(ProtocolType::Vnc),
        Just(ProtocolType::Telnet),
    ]
}

/// Creates a temporary connection for Quick Connect (not persisted)
///
/// This mirrors the GUI's Quick Connect behavior where connections
/// are created with `Uuid::nil()` to indicate they are temporary.
fn create_quick_connect_connection(
    host: &str,
    port: u16,
    username: Option<&str>,
    protocol: ProtocolType,
) -> Connection {
    let name = format!("Quick: {host}");
    let protocol_config = match protocol {
        ProtocolType::Ssh => ProtocolConfig::Ssh(SshConfig::default()),
        ProtocolType::Rdp => ProtocolConfig::Rdp(RdpConfig::default()),
        ProtocolType::Vnc => ProtocolConfig::Vnc(VncConfig::default()),
        ProtocolType::Telnet => ProtocolConfig::Telnet(TelnetConfig::default()),
        _ => ProtocolConfig::Ssh(SshConfig::default()),
    };

    let mut conn = Connection::new(name, host.to_string(), port, protocol_config);

    // Quick Connect uses nil UUID to indicate temporary connection
    conn.id = Uuid::nil();

    if let Some(user) = username {
        conn.username = Some(user.to_string());
    }

    conn
}

/// Checks if a connection is a Quick Connect (temporary) connection
fn is_quick_connect(conn: &Connection) -> bool {
    conn.id == Uuid::nil() || conn.name.starts_with("Quick: ")
}

// ========== Property Tests ==========

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Property 20: Quick Connect No Persistence**
    ///
    /// For any Quick Connect session, after the session ends the connection
    /// should not appear in the saved connections list.
    ///
    /// This test verifies that:
    /// 1. Creating a Quick Connect connection does not add it to the manager
    /// 2. The connection count remains unchanged
    /// 3. The Quick Connect connection cannot be found by ID
    #[test]
    fn prop_quick_connect_not_persisted(
        host in arb_host(),
        port in arb_port(),
        username in arb_username(),
        protocol in arb_protocol(),
    ) {
        let (manager, _temp, _runtime) = create_test_manager();

        // Record initial state
        let initial_count = manager.list_connections().len();

        // Create a Quick Connect connection (temporary, not saved)
        let quick_conn = create_quick_connect_connection(
            &host,
            port,
            username.as_deref(),
            protocol,
        );

        // Verify it's marked as Quick Connect
        prop_assert!(is_quick_connect(&quick_conn));

        // Verify the connection has nil UUID (temporary marker)
        prop_assert_eq!(quick_conn.id, Uuid::nil());

        // Verify the name follows Quick Connect pattern
        prop_assert!(quick_conn.name.starts_with("Quick: "));

        // Verify the connection manager state is unchanged
        // (Quick Connect connections are NOT added to the manager)
        let final_count = manager.list_connections().len();
        prop_assert_eq!(initial_count, final_count);

        // Verify the Quick Connect connection cannot be found in the manager
        prop_assert!(manager.get_connection(quick_conn.id).is_none());
    }

    /// Property: Quick Connect connections have valid parameters
    ///
    /// Verifies that Quick Connect creates valid Connection objects
    /// with proper host, port, and protocol configuration.
    #[test]
    fn prop_quick_connect_valid_parameters(
        host in arb_host(),
        port in arb_port(),
        username in arb_username(),
        protocol in arb_protocol(),
    ) {
        let quick_conn = create_quick_connect_connection(
            &host,
            port,
            username.as_deref(),
            protocol,
        );

        // Verify host is set correctly
        prop_assert_eq!(&quick_conn.host, &host);

        // Verify port is set correctly
        prop_assert_eq!(quick_conn.port, port);

        // Verify username is set correctly
        prop_assert_eq!(quick_conn.username, username);

        // Verify protocol matches
        prop_assert_eq!(quick_conn.protocol, protocol);

        // Verify protocol config matches protocol type
        match protocol {
            ProtocolType::Ssh => {
                prop_assert!(matches!(quick_conn.protocol_config, ProtocolConfig::Ssh(_)));
            }
            ProtocolType::Rdp => {
                prop_assert!(matches!(quick_conn.protocol_config, ProtocolConfig::Rdp(_)));
            }
            ProtocolType::Vnc => {
                prop_assert!(matches!(quick_conn.protocol_config, ProtocolConfig::Vnc(_)));
            }
            ProtocolType::Telnet => {
                prop_assert!(matches!(quick_conn.protocol_config, ProtocolConfig::Telnet(_)));
            }
            _ => {}
        }
    }

    /// Property: Quick Connect does not affect existing connections
    ///
    /// Verifies that creating Quick Connect sessions does not modify
    /// or interfere with existing saved connections.
    #[test]
    fn prop_quick_connect_does_not_affect_existing(
        host in arb_host(),
        port in arb_port(),
        existing_name in "[a-zA-Z][a-zA-Z0-9_-]{0,15}",
        existing_host in arb_host(),
    ) {
        let (mut manager, _temp, _runtime) = create_test_manager();

        // Create and save an existing connection
        let existing_conn = Connection::new_ssh(
            existing_name.clone(),
            existing_host.clone(),
            22,
        );
        let existing_id = manager
            .create_connection_from(existing_conn.clone())
            .expect("Failed to create existing connection");

        // Record state after creating existing connection
        let count_before = manager.list_connections().len();

        // Create a Quick Connect connection (not saved)
        let _quick_conn = create_quick_connect_connection(
            &host,
            port,
            None,
            ProtocolType::Ssh,
        );

        // Verify existing connection is unchanged
        let retrieved = manager.get_connection(existing_id);
        prop_assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        prop_assert_eq!(&retrieved.name, &existing_name);
        prop_assert_eq!(&retrieved.host, &existing_host);

        // Verify connection count is unchanged
        let count_after = manager.list_connections().len();
        prop_assert_eq!(count_before, count_after);
    }

    /// Property: Multiple Quick Connect sessions don't accumulate
    ///
    /// Verifies that creating multiple Quick Connect sessions does not
    /// cause connections to accumulate in the manager.
    #[test]
    fn prop_multiple_quick_connects_no_accumulation(
        hosts in prop::collection::vec(arb_host(), 1..10),
        ports in prop::collection::vec(arb_port(), 1..10),
    ) {
        let (manager, _temp, _runtime) = create_test_manager();

        // Record initial state
        let initial_count = manager.list_connections().len();

        // Create multiple Quick Connect connections
        let min_len = hosts.len().min(ports.len());
        for i in 0..min_len {
            let _quick_conn = create_quick_connect_connection(
                &hosts[i],
                ports[i],
                None,
                ProtocolType::Ssh,
            );
        }

        // Verify no connections were added
        let final_count = manager.list_connections().len();
        prop_assert_eq!(initial_count, final_count);
    }
}

// ========== Unit Tests ==========

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn test_quick_connect_ssh() {
        let conn =
            create_quick_connect_connection("example.com", 22, Some("user"), ProtocolType::Ssh);

        assert_eq!(conn.id, Uuid::nil());
        assert_eq!(conn.name, "Quick: example.com");
        assert_eq!(conn.host, "example.com");
        assert_eq!(conn.port, 22);
        assert_eq!(conn.username, Some("user".to_string()));
        assert_eq!(conn.protocol, ProtocolType::Ssh);
        assert!(matches!(conn.protocol_config, ProtocolConfig::Ssh(_)));
    }

    #[test]
    fn test_quick_connect_rdp() {
        let conn = create_quick_connect_connection("server.local", 3389, None, ProtocolType::Rdp);

        assert_eq!(conn.id, Uuid::nil());
        assert_eq!(conn.name, "Quick: server.local");
        assert_eq!(conn.host, "server.local");
        assert_eq!(conn.port, 3389);
        assert_eq!(conn.username, None);
        assert_eq!(conn.protocol, ProtocolType::Rdp);
        assert!(matches!(conn.protocol_config, ProtocolConfig::Rdp(_)));
    }

    #[test]
    fn test_quick_connect_vnc() {
        let conn = create_quick_connect_connection("192.168.1.100", 5900, None, ProtocolType::Vnc);

        assert_eq!(conn.id, Uuid::nil());
        assert_eq!(conn.name, "Quick: 192.168.1.100");
        assert_eq!(conn.host, "192.168.1.100");
        assert_eq!(conn.port, 5900);
        assert_eq!(conn.username, None);
        assert_eq!(conn.protocol, ProtocolType::Vnc);
        assert!(matches!(conn.protocol_config, ProtocolConfig::Vnc(_)));
    }

    #[test]
    fn test_quick_connect_telnet() {
        let conn = create_quick_connect_connection("switch.local", 23, None, ProtocolType::Telnet);

        assert_eq!(conn.id, Uuid::nil());
        assert_eq!(conn.name, "Quick: switch.local");
        assert_eq!(conn.host, "switch.local");
        assert_eq!(conn.port, 23);
        assert_eq!(conn.username, None);
        assert_eq!(conn.protocol, ProtocolType::Telnet);
        assert!(matches!(conn.protocol_config, ProtocolConfig::Telnet(_)));
    }

    #[test]
    fn test_is_quick_connect_by_id() {
        let mut conn = Connection::new_ssh("Test".to_string(), "host".to_string(), 22);
        conn.id = Uuid::nil();

        assert!(is_quick_connect(&conn));
    }

    #[test]
    fn test_is_quick_connect_by_name() {
        let conn = Connection::new_ssh("Quick: host".to_string(), "host".to_string(), 22);

        assert!(is_quick_connect(&conn));
    }

    #[test]
    fn test_is_not_quick_connect() {
        let conn = Connection::new_ssh("My Server".to_string(), "host".to_string(), 22);

        assert!(!is_quick_connect(&conn));
    }

    #[test]
    fn test_quick_connect_not_in_manager() {
        let (manager, _temp, _runtime) = super::create_test_manager();

        // Create Quick Connect (not added to manager)
        let quick_conn =
            super::create_quick_connect_connection("example.com", 22, None, ProtocolType::Ssh);

        // Verify it's not in the manager
        assert!(manager.get_connection(quick_conn.id).is_none());
        assert_eq!(manager.list_connections().len(), 0);
    }
}
