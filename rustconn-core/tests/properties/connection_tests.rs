//! Property-based tests for Connection CRUD operations
//!
//! **Feature: rustconn, Property 1: Connection CRUD Data Integrity**
//! **Validates: Requirements 1.1, 1.2, 1.3**

use chrono;
use proptest::prelude::*;
use rustconn_core::{
    ConfigManager, Connection, ConnectionManager, ProtocolConfig, RdpConfig, RdpGateway,
    Resolution, SshAuthMethod, SshConfig, SshKeySource, VncConfig,
};
use std::collections::HashMap;
use std::path::PathBuf;
use tempfile::TempDir;
use uuid::Uuid;

// ========== Generators ==========

// Strategy for generating valid connection names (non-empty)
fn arb_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_-]{0,31}".prop_map(|s| s)
}

// Strategy for generating valid hostnames (non-empty)
fn arb_host() -> impl Strategy<Value = String> {
    "[a-z0-9]([a-z0-9-]{0,15}[a-z0-9])?(\\.[a-z0-9]([a-z0-9-]{0,15}[a-z0-9])?)*".prop_map(|s| s)
}

// Strategy for generating valid ports (non-zero)
fn arb_port() -> impl Strategy<Value = u16> {
    1u16..=65535u16
}

// Strategy for generating optional usernames
fn arb_username() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), "[a-z][a-z0-9_]{0,15}".prop_map(Some),]
}

// Strategy for generating tags
fn arb_tags() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec("[a-z]{1,10}", 0..5)
}

// Strategy for SSH auth method
fn arb_ssh_auth_method() -> impl Strategy<Value = SshAuthMethod> {
    prop_oneof![
        Just(SshAuthMethod::Password),
        Just(SshAuthMethod::PublicKey),
        Just(SshAuthMethod::KeyboardInteractive),
        Just(SshAuthMethod::Agent),
    ]
}

// Strategy for optional PathBuf
fn arb_optional_path() -> impl Strategy<Value = Option<PathBuf>> {
    prop_oneof![
        Just(None),
        "[a-z]{1,10}(/[a-z]{1,10}){0,3}".prop_map(|s| Some(PathBuf::from(s))),
    ]
}

// Strategy for optional string
fn arb_optional_string() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), "[a-zA-Z0-9_-]{1,20}".prop_map(Some),]
}

// Strategy for custom SSH options
fn arb_custom_options() -> impl Strategy<Value = HashMap<String, String>> {
    prop::collection::hash_map("[A-Za-z]{1,20}", "[a-zA-Z0-9]{1,10}", 0..3)
}

// Strategy for SSH config
fn arb_ssh_config() -> impl Strategy<Value = SshConfig> {
    (
        arb_ssh_auth_method(),
        arb_optional_path(),
        arb_optional_string(),
        any::<bool>(),
        arb_custom_options(),
        arb_optional_string(),
    )
        .prop_map(
            |(
                auth_method,
                key_path,
                proxy_jump,
                use_control_master,
                custom_options,
                startup_command,
            )| {
                SshConfig {
                    auth_method,
                    key_path,
                    key_source: SshKeySource::Default,
                    agent_key_fingerprint: None,
                    identities_only: false,
                    proxy_jump,
                    use_control_master,
                    agent_forwarding: false,
                    custom_options,
                    startup_command,
                    jump_host_id: None,
                }
            },
        )
}

// Strategy for optional resolution
fn arb_optional_resolution() -> impl Strategy<Value = Option<Resolution>> {
    prop_oneof![
        Just(None),
        (640u32..4096u32, 480u32..2160u32).prop_map(|(w, h)| Some(Resolution::new(w, h))),
    ]
}

// Strategy for optional color depth
fn arb_optional_color_depth() -> impl Strategy<Value = Option<u8>> {
    prop_oneof![
        Just(None),
        prop_oneof![Just(8u8), Just(15u8), Just(16u8), Just(24u8), Just(32u8)].prop_map(Some),
    ]
}

// Strategy for optional RDP gateway
fn arb_optional_gateway() -> impl Strategy<Value = Option<RdpGateway>> {
    prop_oneof![
        Just(None),
        (arb_host(), 1u16..65535u16, arb_optional_string()).prop_map(
            |(hostname, port, username)| {
                Some(RdpGateway {
                    hostname,
                    port,
                    username,
                })
            }
        ),
    ]
}

// Strategy for custom args
fn arb_custom_args() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec("[a-zA-Z0-9_=-]{1,20}", 0..3)
}

// Strategy for RDP config
fn arb_rdp_config() -> impl Strategy<Value = RdpConfig> {
    (
        arb_optional_resolution(),
        arb_optional_color_depth(),
        any::<bool>(),
        arb_optional_gateway(),
        arb_custom_args(),
    )
        .prop_map(
            |(resolution, color_depth, audio_redirect, gateway, custom_args)| RdpConfig {
                resolution,
                color_depth,
                audio_redirect,
                gateway,
                shared_folders: Vec::new(),
                custom_args,
                client_mode: Default::default(),
                performance_mode: Default::default(),
            },
        )
}

// Strategy for optional encoding
fn arb_optional_encoding() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        Just(None),
        prop_oneof![
            Just("tight".to_string()),
            Just("zrle".to_string()),
            Just("hextile".to_string()),
        ]
        .prop_map(Some),
    ]
}

// Strategy for optional compression/quality (0-9)
fn arb_optional_level() -> impl Strategy<Value = Option<u8>> {
    prop_oneof![Just(None), (0u8..=9u8).prop_map(Some),]
}

// Strategy for VNC config
fn arb_vnc_config() -> impl Strategy<Value = VncConfig> {
    (
        arb_optional_encoding(),
        arb_optional_level(),
        arb_optional_level(),
        arb_custom_args(),
    )
        .prop_map(|(encoding, compression, quality, custom_args)| VncConfig {
            client_mode: Default::default(),
            performance_mode: Default::default(),
            encoding,
            compression,
            quality,
            view_only: false,
            scaling: true,
            clipboard_enabled: true,
            custom_args,
        })
}

// Strategy for protocol config
fn arb_protocol_config() -> impl Strategy<Value = ProtocolConfig> {
    prop_oneof![
        arb_ssh_config().prop_map(ProtocolConfig::Ssh),
        arb_rdp_config().prop_map(ProtocolConfig::Rdp),
        arb_vnc_config().prop_map(ProtocolConfig::Vnc),
    ]
}

// Strategy for generating a complete Connection
fn arb_connection() -> impl Strategy<Value = Connection> {
    (
        arb_name(),
        arb_host(),
        arb_port(),
        arb_protocol_config(),
        arb_username(),
        arb_tags(),
    )
        .prop_map(|(name, host, port, protocol_config, username, tags)| {
            let mut conn = Connection::new(name, host, port, protocol_config);
            if let Some(u) = username {
                conn = conn.with_username(u);
            }
            if !tags.is_empty() {
                conn = conn.with_tags(tags);
            }
            conn
        })
}

// Helper to create a test ConnectionManager
fn create_test_manager() -> (ConnectionManager, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let config_manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());
    let manager = ConnectionManager::new(config_manager).unwrap();
    (manager, temp_dir)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn, Property 1: Connection CRUD Data Integrity**
    /// **Validates: Requirements 1.1, 1.2, 1.3**
    ///
    /// For any valid connection configuration, creating a connection and then
    /// retrieving it by ID should return a connection with identical name, host,
    /// port, protocol type, and all other configuration fields.
    #[test]
    fn create_then_retrieve_preserves_data(
        name in arb_name(),
        host in arb_host(),
        port in arb_port(),
        protocol_config in arb_protocol_config(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create connection
        let id = manager
            .create_connection(name.clone(), host.clone(), port, protocol_config.clone())
            .expect("Should create connection");

        // Retrieve connection
        let retrieved = manager
            .get_connection(id)
            .expect("Should retrieve connection");

        // Verify all fields are preserved
        prop_assert_eq!(retrieved.id, id, "ID should match");
        prop_assert_eq!(&retrieved.name, &name, "Name should be preserved");
        prop_assert_eq!(&retrieved.host, &host, "Host should be preserved");
        prop_assert_eq!(retrieved.port, port, "Port should be preserved");
        prop_assert_eq!(&retrieved.protocol_config, &protocol_config, "Protocol config should be preserved");
        prop_assert_eq!(retrieved.protocol, protocol_config.protocol_type(), "Protocol type should match config");
    }

    /// **Feature: rustconn, Property 1: Connection CRUD Data Integrity**
    /// **Validates: Requirements 1.1, 1.2**
    ///
    /// For any existing connection and valid update, updating the connection
    /// should preserve the original ID while changing only the specified fields.
    #[test]
    fn update_preserves_id_and_changes_fields(
        original_name in arb_name(),
        original_host in arb_host(),
        original_port in arb_port(),
        original_config in arb_protocol_config(),
        new_name in arb_name(),
        new_host in arb_host(),
        new_port in arb_port(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create original connection
        let id = manager
            .create_connection(original_name, original_host, original_port, original_config.clone())
            .expect("Should create connection");

        let original_created_at = manager.get_connection(id).unwrap().created_at;

        // Create updated connection with new values
        let mut updated = manager.get_connection(id).unwrap().clone();
        updated.name = new_name.clone();
        updated.host = new_host.clone();
        updated.port = new_port;

        // Update connection
        manager
            .update_connection(id, updated)
            .expect("Should update connection");

        // Retrieve updated connection
        let retrieved = manager
            .get_connection(id)
            .expect("Should retrieve updated connection");

        // Verify ID is preserved
        prop_assert_eq!(retrieved.id, id, "ID should be preserved after update");

        // Verify created_at is preserved
        prop_assert_eq!(
            retrieved.created_at.timestamp(),
            original_created_at.timestamp(),
            "Created timestamp should be preserved"
        );

        // Verify fields are updated
        prop_assert_eq!(&retrieved.name, &new_name, "Name should be updated");
        prop_assert_eq!(&retrieved.host, &new_host, "Host should be updated");
        prop_assert_eq!(retrieved.port, new_port, "Port should be updated");

        // Verify updated_at changed (should be >= created_at)
        prop_assert!(
            retrieved.updated_at >= retrieved.created_at,
            "Updated timestamp should be >= created timestamp"
        );
    }

    /// **Feature: rustconn, Property 1: Connection CRUD Data Integrity**
    /// **Validates: Requirements 1.3**
    ///
    /// For any existing connection, deleting it should result in the connection
    /// being absent from all queries.
    #[test]
    fn delete_removes_connection(
        name in arb_name(),
        host in arb_host(),
        port in arb_port(),
        protocol_config in arb_protocol_config(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create connection
        let id = manager
            .create_connection(name, host, port, protocol_config)
            .expect("Should create connection");

        // Verify it exists
        prop_assert!(manager.get_connection(id).is_some(), "Connection should exist before delete");
        prop_assert_eq!(manager.connection_count(), 1, "Should have 1 connection");

        // Delete connection
        manager
            .delete_connection(id)
            .expect("Should delete connection");

        // Verify it's gone
        prop_assert!(manager.get_connection(id).is_none(), "Connection should not exist after delete");
        prop_assert_eq!(manager.connection_count(), 0, "Should have 0 connections");

        // Verify it's not in list
        let all_connections = manager.list_connections();
        prop_assert!(
            !all_connections.iter().any(|c| c.id == id),
            "Deleted connection should not appear in list"
        );
    }

    /// **Feature: rustconn, Property 1: Connection CRUD Data Integrity**
    /// **Validates: Requirements 1.1, 1.3**
    ///
    /// Creating multiple connections and deleting one should only remove that
    /// specific connection, leaving others intact.
    #[test]
    fn delete_only_affects_target_connection(
        conn1 in arb_connection(),
        conn2 in arb_connection(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create two connections
        let id1 = manager
            .create_connection_from(conn1.clone())
            .expect("Should create first connection");

        let id2 = manager
            .create_connection_from(conn2.clone())
            .expect("Should create second connection");

        prop_assert_eq!(manager.connection_count(), 2, "Should have 2 connections");

        // Delete first connection
        manager
            .delete_connection(id1)
            .expect("Should delete first connection");

        // Verify first is gone
        prop_assert!(manager.get_connection(id1).is_none(), "First connection should be deleted");

        // Verify second still exists with correct data
        let remaining = manager
            .get_connection(id2)
            .expect("Second connection should still exist");

        prop_assert_eq!(&remaining.name, &conn2.name, "Second connection name should be preserved");
        prop_assert_eq!(&remaining.host, &conn2.host, "Second connection host should be preserved");
        prop_assert_eq!(remaining.port, conn2.port, "Second connection port should be preserved");
    }

    /// **Feature: rustconn, Property 1: Connection CRUD Data Integrity**
    /// **Validates: Requirements 1.1**
    ///
    /// Creating a connection from an existing Connection object should preserve
    /// all fields including the original ID.
    #[test]
    fn create_from_preserves_all_fields(conn in arb_connection()) {
        let (mut manager, _temp) = create_test_manager();

        let original_id = conn.id;

        // Create from existing connection
        let id = manager
            .create_connection_from(conn.clone())
            .expect("Should create connection from existing");

        // ID should be the same as the original
        prop_assert_eq!(id, original_id, "Should preserve original ID");

        // Retrieve and verify all fields
        let retrieved = manager
            .get_connection(id)
            .expect("Should retrieve connection");

        prop_assert_eq!(retrieved.id, conn.id, "ID should be preserved");
        prop_assert_eq!(&retrieved.name, &conn.name, "Name should be preserved");
        prop_assert_eq!(&retrieved.host, &conn.host, "Host should be preserved");
        prop_assert_eq!(retrieved.port, conn.port, "Port should be preserved");
        prop_assert_eq!(&retrieved.username, &conn.username, "Username should be preserved");
        prop_assert_eq!(&retrieved.tags, &conn.tags, "Tags should be preserved");
        prop_assert_eq!(&retrieved.protocol_config, &conn.protocol_config, "Protocol config should be preserved");
    }
}

// ========== Group Hierarchy Property Tests ==========

// Strategy for generating valid group names
fn arb_group_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_ -]{0,31}".prop_map(|s| s)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn, Property 14: Group Hierarchy Integrity**
    /// **Validates: Requirements 1.4**
    ///
    /// For any sequence of group creation and nesting operations, the resulting
    /// hierarchy should be acyclic (no group is its own ancestor).
    #[test]
    fn group_hierarchy_is_acyclic_after_creation(
        names in prop::collection::vec(arb_group_name(), 1..10),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create groups with random parent relationships
        let mut group_ids = Vec::new();

        for (i, name) in names.iter().enumerate() {
            let id = if i == 0 || group_ids.is_empty() {
                // First group is always root
                manager.create_group(name.clone()).expect("Should create root group")
            } else {
                // Randomly choose to be root or have a parent
                if i % 2 == 0 {
                    manager.create_group(name.clone()).expect("Should create root group")
                } else {
                    // Pick a random existing group as parent
                    let parent_idx = i % group_ids.len();
                    let parent_id = group_ids[parent_idx];
                    manager
                        .create_group_with_parent(name.clone(), parent_id)
                        .expect("Should create child group")
                }
            };
            group_ids.push(id);
        }

        // Verify hierarchy is acyclic
        prop_assert!(
            manager.validate_hierarchy(),
            "Hierarchy should be acyclic after group creation"
        );
    }

    /// **Feature: rustconn, Property 14: Group Hierarchy Integrity**
    /// **Validates: Requirements 1.4**
    ///
    /// Moving a group should never create a cycle in the hierarchy.
    #[test]
    fn move_group_prevents_cycles(
        names in prop::collection::vec(arb_group_name(), 3..6),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create a chain of groups: A -> B -> C -> ...
        let mut group_ids = Vec::new();
        for (i, name) in names.iter().enumerate() {
            let id = if i == 0 {
                manager.create_group(name.clone()).expect("Should create root group")
            } else {
                let parent_id = group_ids[i - 1];
                manager
                    .create_group_with_parent(name.clone(), parent_id)
                    .expect("Should create child group")
            };
            group_ids.push(id);
        }

        // Try to move the root to be a child of the last group (would create cycle)
        if group_ids.len() >= 2 {
            let root_id = group_ids[0];
            let last_id = group_ids[group_ids.len() - 1];

            let result = manager.move_group(root_id, Some(last_id));
            prop_assert!(
                result.is_err(),
                "Moving root to be child of descendant should fail"
            );

            // Hierarchy should still be valid
            prop_assert!(
                manager.validate_hierarchy(),
                "Hierarchy should remain acyclic after failed move"
            );
        }
    }

    /// **Feature: rustconn, Property 14: Group Hierarchy Integrity**
    /// **Validates: Requirements 1.4**
    ///
    /// All parent references should point to existing groups.
    #[test]
    fn all_parent_references_are_valid(
        names in prop::collection::vec(arb_group_name(), 1..8),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create groups with various parent relationships
        let mut group_ids = Vec::new();
        for (i, name) in names.iter().enumerate() {
            let id = if i == 0 {
                manager.create_group(name.clone()).expect("Should create root group")
            } else if i % 3 == 0 {
                // Create as root
                manager.create_group(name.clone()).expect("Should create root group")
            } else {
                // Create with parent
                let parent_idx = (i - 1) % group_ids.len();
                let parent_id = group_ids[parent_idx];
                manager
                    .create_group_with_parent(name.clone(), parent_id)
                    .expect("Should create child group")
            };
            group_ids.push(id);
        }

        // Verify all parent references point to existing groups
        for group in manager.list_groups() {
            if let Some(parent_id) = group.parent_id {
                prop_assert!(
                    manager.get_group(parent_id).is_some(),
                    "Parent reference should point to existing group"
                );
            }
        }
    }

    /// **Feature: rustconn, Property 14: Group Hierarchy Integrity**
    /// **Validates: Requirements 1.4**
    ///
    /// Deleting a group should maintain valid parent references for child groups.
    #[test]
    fn delete_group_maintains_valid_references(
        parent_name in arb_group_name(),
        child_name in arb_group_name(),
        grandchild_name in arb_group_name(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create three-level hierarchy
        let parent_id = manager.create_group(parent_name).expect("Should create parent");
        let child_id = manager
            .create_group_with_parent(child_name, parent_id)
            .expect("Should create child");
        let grandchild_id = manager
            .create_group_with_parent(grandchild_name, child_id)
            .expect("Should create grandchild");

        // Delete the middle group
        manager.delete_group(child_id).expect("Should delete child group");

        // Grandchild should now point to parent (the deleted group's parent)
        let grandchild = manager.get_group(grandchild_id).expect("Grandchild should exist");
        prop_assert_eq!(
            grandchild.parent_id,
            Some(parent_id),
            "Grandchild should be moved to deleted group's parent"
        );

        // Hierarchy should still be valid
        prop_assert!(
            manager.validate_hierarchy(),
            "Hierarchy should be valid after group deletion"
        );

        // All parent references should be valid
        for group in manager.list_groups() {
            if let Some(pid) = group.parent_id {
                prop_assert!(
                    manager.get_group(pid).is_some(),
                    "All parent references should be valid after deletion"
                );
            }
        }
    }

    /// **Feature: rustconn, Property 14: Group Hierarchy Integrity**
    /// **Validates: Requirements 1.4**
    ///
    /// Moving a group to root (None parent) should always succeed and maintain hierarchy.
    #[test]
    fn move_to_root_always_succeeds(
        parent_name in arb_group_name(),
        child_name in arb_group_name(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create parent-child relationship
        let parent_id = manager.create_group(parent_name).expect("Should create parent");
        let child_id = manager
            .create_group_with_parent(child_name, parent_id)
            .expect("Should create child");

        // Move child to root
        manager
            .move_group(child_id, None)
            .expect("Moving to root should succeed");

        // Verify child is now root
        let child = manager.get_group(child_id).expect("Child should exist");
        prop_assert!(child.parent_id.is_none(), "Child should be root after move");

        // Hierarchy should be valid
        prop_assert!(
            manager.validate_hierarchy(),
            "Hierarchy should be valid after move to root"
        );
    }

    /// **Feature: rustconn-enhancements, Property 6: Group Hierarchy Acyclicity**
    /// **Validates: Requirements 9.1, 9.2**
    ///
    /// For any sequence of group creation and move operations, the resulting
    /// group hierarchy must remain acyclic - no group can be its own ancestor.
    #[test]
    fn group_hierarchy_acyclicity_property(
        group_names in prop::collection::vec(arb_group_name(), 2..8),
        move_attempts in prop::collection::vec((0usize..8usize, 0usize..8usize), 1..5),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create groups with various parent relationships
        let mut group_ids = Vec::new();
        for (i, name) in group_names.iter().enumerate() {
            let id = if i == 0 {
                // First group is always root
                manager.create_group(name.clone()).expect("Should create root group")
            } else {
                // Alternate between root and child groups
                if i % 2 == 0 {
                    manager.create_group(name.clone()).expect("Should create root group")
                } else {
                    // Create as child of a previous group
                    let parent_idx = (i - 1) % group_ids.len();
                    let parent_id = group_ids[parent_idx];
                    manager
                        .create_group_with_parent(name.clone(), parent_id)
                        .expect("Should create child group")
                }
            };
            group_ids.push(id);
        }

        // Verify initial hierarchy is acyclic
        prop_assert!(
            manager.validate_hierarchy(),
            "Initial hierarchy should be acyclic"
        );

        // Attempt various move operations
        for (from_idx, to_idx) in move_attempts {
            if from_idx < group_ids.len() && to_idx < group_ids.len() {
                let from_id = group_ids[from_idx];
                let to_id = if from_idx == to_idx {
                    None // Move to root
                } else {
                    Some(group_ids[to_idx])
                };

                // Attempt the move - it may succeed or fail depending on cycle detection
                let _ = manager.move_group(from_id, to_id);

                // After any move attempt (success or failure), hierarchy must remain acyclic
                prop_assert!(
                    manager.validate_hierarchy(),
                    "Hierarchy must remain acyclic after move attempt from {} to {:?}",
                    from_idx, to_idx
                );
            }
        }

        // Final verification: all parent references must be valid
        for group in manager.list_groups() {
            if let Some(parent_id) = group.parent_id {
                prop_assert!(
                    manager.get_group(parent_id).is_some(),
                    "All parent references must point to existing groups"
                );
            }
        }
    }

    /// **Feature: rustconn-enhancements, Property 6: Group Hierarchy Acyclicity**
    /// **Validates: Requirements 9.1, 9.2**
    ///
    /// Creating a group with a parent should correctly establish the parent-child
    /// relationship and the group path should reflect the hierarchy.
    #[test]
    fn create_group_with_parent_establishes_hierarchy(
        root_name in arb_group_name(),
        child_name in arb_group_name(),
        grandchild_name in arb_group_name(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create three-level hierarchy
        let root_id = manager.create_group(root_name.clone()).expect("Should create root");
        let child_id = manager
            .create_group_with_parent(child_name.clone(), root_id)
            .expect("Should create child");
        let grandchild_id = manager
            .create_group_with_parent(grandchild_name.clone(), child_id)
            .expect("Should create grandchild");

        // Verify parent relationships
        let root = manager.get_group(root_id).expect("Root should exist");
        let child = manager.get_group(child_id).expect("Child should exist");
        let grandchild = manager.get_group(grandchild_id).expect("Grandchild should exist");

        prop_assert!(root.parent_id.is_none(), "Root should have no parent");
        prop_assert_eq!(child.parent_id, Some(root_id), "Child should have root as parent");
        prop_assert_eq!(grandchild.parent_id, Some(child_id), "Grandchild should have child as parent");

        // Verify group paths
        let root_path = manager.get_group_path(root_id).expect("Root path should exist");
        let child_path = manager.get_group_path(child_id).expect("Child path should exist");
        let grandchild_path = manager.get_group_path(grandchild_id).expect("Grandchild path should exist");

        prop_assert_eq!(&root_path, &root_name, "Root path should be just the root name");
        prop_assert!(
            child_path.contains(&root_name) && child_path.contains(&child_name),
            "Child path should contain both root and child names"
        );
        prop_assert!(
            grandchild_path.contains(&root_name) && grandchild_path.contains(&child_name) && grandchild_path.contains(&grandchild_name),
            "Grandchild path should contain all three names"
        );

        // Hierarchy should be acyclic
        prop_assert!(
            manager.validate_hierarchy(),
            "Hierarchy should be acyclic"
        );
    }

    /// **Feature: rustconn-enhancements, Property 6: Group Hierarchy Acyclicity**
    /// **Validates: Requirements 9.1, 9.2**
    ///
    /// Moving a connection to a group should update the connection's group_id correctly.
    #[test]
    fn move_connection_to_group_updates_group_id(
        conn in arb_connection(),
        group_name in arb_group_name(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create connection and group
        let conn_id = manager.create_connection_from(conn).expect("Should create connection");
        let group_id = manager.create_group(group_name).expect("Should create group");

        // Initially connection should be ungrouped
        let connection = manager.get_connection(conn_id).expect("Connection should exist");
        prop_assert!(connection.group_id.is_none(), "Connection should initially be ungrouped");

        // Move connection to group
        manager
            .move_connection_to_group(conn_id, Some(group_id))
            .expect("Should move connection to group");

        // Verify connection is now in the group
        let connection = manager.get_connection(conn_id).expect("Connection should exist");
        prop_assert_eq!(
            connection.group_id,
            Some(group_id),
            "Connection should be in the group after move"
        );

        // Move connection back to ungrouped
        manager
            .move_connection_to_group(conn_id, None)
            .expect("Should move connection to ungrouped");

        // Verify connection is ungrouped again
        let connection = manager.get_connection(conn_id).expect("Connection should exist");
        prop_assert!(
            connection.group_id.is_none(),
            "Connection should be ungrouped after move to None"
        );
    }
}

// ========== Search Property Tests ==========

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn, Property 2: Connection Search Correctness**
    /// **Validates: Requirements 1.5, 1.6**
    ///
    /// For any set of connections and search query, all returned results must
    /// match the query against at least one of: name, host, tags, or group path.
    #[test]
    fn search_results_match_query(
        connections in prop::collection::vec(arb_connection(), 1..20),
        query in "[a-z]{1,5}",
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Add all connections
        for conn in &connections {
            let _ = manager.create_connection_from(conn.clone());
        }

        // Perform search
        let results = manager.search(&query);
        let query_lower = query.to_lowercase();

        // Verify all results match the query
        for result in results {
            let name_matches = result.name.to_lowercase().contains(&query_lower);
            let host_matches = result.host.to_lowercase().contains(&query_lower);
            let tags_match = result.tags.iter().any(|t| t.to_lowercase().contains(&query_lower));
            let group_matches = result.group_id
                .and_then(|gid| manager.get_group_path(gid))
                .map(|path| path.to_lowercase().contains(&query_lower))
                .unwrap_or(false);

            prop_assert!(
                name_matches || host_matches || tags_match || group_matches,
                "Search result should match query in name, host, tags, or group path. \
                 Query: '{}', Name: '{}', Host: '{}', Tags: {:?}",
                query, result.name, result.host, result.tags
            );
        }
    }

    /// **Feature: rustconn, Property 2: Connection Search Correctness**
    /// **Validates: Requirements 1.5, 1.6**
    ///
    /// No connection matching the query should be excluded from results.
    #[test]
    fn search_includes_all_matches(
        name in arb_name(),
        host in arb_host(),
        port in arb_port(),
        protocol_config in arb_protocol_config(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create a connection
        let id = manager
            .create_connection(name.clone(), host.clone(), port, protocol_config)
            .expect("Should create connection");

        // Search by exact name (should find it)
        let results = manager.search(&name);
        prop_assert!(
            results.iter().any(|c| c.id == id),
            "Search by exact name should find the connection"
        );

        // Search by partial name (first 3 chars if long enough)
        if name.len() >= 3 {
            let partial = &name[0..3];
            let results = manager.search(partial);
            prop_assert!(
                results.iter().any(|c| c.id == id),
                "Search by partial name should find the connection"
            );
        }

        // Search by host
        let results = manager.search(&host);
        prop_assert!(
            results.iter().any(|c| c.id == id),
            "Search by host should find the connection"
        );
    }

    /// **Feature: rustconn, Property 2: Connection Search Correctness**
    /// **Validates: Requirements 1.6**
    ///
    /// Tag-based filtering should return only connections with the specified tag.
    #[test]
    fn filter_by_tag_returns_only_tagged_connections(
        conn1 in arb_connection(),
        conn2 in arb_connection(),
        tag in "[a-z]{3,10}",
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create first connection with the tag
        let mut conn1_with_tag = conn1.clone();
        conn1_with_tag.tags = vec![tag.clone()];
        let id1 = manager
            .create_connection_from(conn1_with_tag)
            .expect("Should create first connection");

        // Create second connection without the tag
        let mut conn2_without_tag = conn2.clone();
        conn2_without_tag.tags = vec!["other_tag".to_string()];
        let id2 = manager
            .create_connection_from(conn2_without_tag)
            .expect("Should create second connection");

        // Filter by tag
        let results = manager.filter_by_tag(&tag);

        // Should include first connection
        prop_assert!(
            results.iter().any(|c| c.id == id1),
            "Filter should include connection with the tag"
        );

        // Should not include second connection
        prop_assert!(
            !results.iter().any(|c| c.id == id2),
            "Filter should not include connection without the tag"
        );
    }

    /// **Feature: rustconn, Property 2: Connection Search Correctness**
    /// **Validates: Requirements 1.6**
    ///
    /// Filtering by multiple tags should return only connections with ALL tags.
    #[test]
    fn filter_by_multiple_tags_uses_and_logic(
        conn in arb_connection(),
        tag1 in "[a-z]{3,8}",
        tag2 in "[a-z]{3,8}",
    ) {
        // Skip if tags are the same
        prop_assume!(tag1 != tag2);

        let (mut manager, _temp) = create_test_manager();

        // Create connection with both tags
        let mut conn_both = conn.clone();
        conn_both.tags = vec![tag1.clone(), tag2.clone()];
        let id_both = manager
            .create_connection_from(conn_both)
            .expect("Should create connection with both tags");

        // Create connection with only first tag
        let mut conn_one = Connection::new(
            "Single Tag".to_string(),
            "single.example.com".to_string(),
            22,
            ProtocolConfig::Ssh(SshConfig::default()),
        );
        conn_one.tags = vec![tag1.clone()];
        let id_one = manager
            .create_connection_from(conn_one)
            .expect("Should create connection with one tag");

        // Filter by both tags
        let results = manager.filter_by_tags(&[tag1.clone(), tag2.clone()]);

        // Should include connection with both tags
        prop_assert!(
            results.iter().any(|c| c.id == id_both),
            "Filter should include connection with both tags"
        );

        // Should not include connection with only one tag
        prop_assert!(
            !results.iter().any(|c| c.id == id_one),
            "Filter should not include connection with only one tag"
        );
    }

    /// **Feature: rustconn, Property 2: Connection Search Correctness**
    /// **Validates: Requirements 1.5**
    ///
    /// Search should be case-insensitive.
    #[test]
    fn search_is_case_insensitive(
        name in "[A-Z][a-z]{2,10}",
        host in arb_host(),
        port in arb_port(),
        protocol_config in arb_protocol_config(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create connection with mixed case name
        let id = manager
            .create_connection(name.clone(), host, port, protocol_config)
            .expect("Should create connection");

        // Search with lowercase
        let results_lower = manager.search(&name.to_lowercase());
        prop_assert!(
            results_lower.iter().any(|c| c.id == id),
            "Lowercase search should find mixed case name"
        );

        // Search with uppercase
        let results_upper = manager.search(&name.to_uppercase());
        prop_assert!(
            results_upper.iter().any(|c| c.id == id),
            "Uppercase search should find mixed case name"
        );
    }

    /// **Feature: rustconn, Property 2: Connection Search Correctness**
    /// **Validates: Requirements 1.5**
    ///
    /// Search by group path should find connections in that group.
    #[test]
    fn search_by_group_path_finds_connections(
        group_name in arb_group_name(),
        conn_name in arb_name(),
        host in arb_host(),
        port in arb_port(),
        protocol_config in arb_protocol_config(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create group
        let group_id = manager.create_group(group_name.clone()).expect("Should create group");

        // Create connection in group
        let conn_id = manager
            .create_connection(conn_name, host, port, protocol_config)
            .expect("Should create connection");

        manager
            .move_connection_to_group(conn_id, Some(group_id))
            .expect("Should move connection to group");

        // Search by group name
        let results = manager.search(&group_name);

        prop_assert!(
            results.iter().any(|c| c.id == conn_id),
            "Search by group name should find connection in that group"
        );
    }

    /// **Feature: rustconn-enhancements, Property 2: Bulk Delete Completeness**
    /// **Validates: Requirements 3.2, 3.3, 3.4**
    ///
    /// For any set of selected connections, after bulk delete completes successfully,
    /// none of the deleted connection IDs should exist in the connection manager,
    /// and the count of deleted items should equal the original selection count minus any failures.
    #[test]
    fn bulk_delete_removes_all_selected_connections(
        connections in prop::collection::vec(arb_connection(), 2..10),
        delete_indices in prop::collection::hash_set(0usize..10usize, 1..5),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create all connections
        let mut created_ids = Vec::new();
        for conn in &connections {
            let id = manager
                .create_connection_from(conn.clone())
                .expect("Should create connection");
            created_ids.push(id);
        }

        let initial_count = manager.connection_count();
        prop_assert_eq!(initial_count, connections.len(), "Should have all connections created");

        // Select connections to delete (filter to valid indices)
        let ids_to_delete: Vec<Uuid> = delete_indices
            .iter()
            .filter(|&&idx| idx < created_ids.len())
            .map(|&idx| created_ids[idx])
            .collect();

        let delete_count = ids_to_delete.len();

        // Perform bulk delete
        let mut success_count = 0;
        let mut failures: Vec<(Uuid, String)> = Vec::new();

        for id in &ids_to_delete {
            match manager.delete_connection(*id) {
                Ok(()) => success_count += 1,
                Err(e) => failures.push((*id, e.to_string())),
            }
        }

        // Property 1: Success count + failure count should equal total delete attempts
        prop_assert_eq!(
            success_count + failures.len(),
            delete_count,
            "Success + failures should equal total delete attempts"
        );

        // Property 2: None of the successfully deleted IDs should exist
        for id in &ids_to_delete {
            if !failures.iter().any(|(fid, _)| fid == id) {
                prop_assert!(
                    manager.get_connection(*id).is_none(),
                    "Deleted connection {:?} should not exist in manager",
                    id
                );
            }
        }

        // Property 3: Connection count should be reduced by success_count
        let final_count = manager.connection_count();
        prop_assert_eq!(
            final_count,
            initial_count - success_count,
            "Connection count should be reduced by number of successful deletions"
        );

        // Property 4: Non-deleted connections should still exist
        for (idx, id) in created_ids.iter().enumerate() {
            if !ids_to_delete.contains(id) {
                prop_assert!(
                    manager.get_connection(*id).is_some(),
                    "Non-deleted connection at index {} should still exist",
                    idx
                );
            }
        }
    }

    /// **Feature: rustconn-enhancements, Property 2: Bulk Delete Completeness**
    /// **Validates: Requirements 3.2, 3.3, 3.4**
    ///
    /// Bulk delete should continue processing remaining items even if some deletions fail.
    /// This tests the "continue on failure" behavior.
    #[test]
    fn bulk_delete_continues_on_failure(
        connections in prop::collection::vec(arb_connection(), 3..8),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create connections
        let mut created_ids = Vec::new();
        for conn in &connections {
            let id = manager
                .create_connection_from(conn.clone())
                .expect("Should create connection");
            created_ids.push(id);
        }

        // Create a list with some valid IDs and some invalid (non-existent) IDs
        let mut ids_to_delete = Vec::new();

        // Add first valid ID
        if !created_ids.is_empty() {
            ids_to_delete.push(created_ids[0]);
        }

        // Add a non-existent ID (will fail)
        ids_to_delete.push(Uuid::new_v4());

        // Add second valid ID if available
        if created_ids.len() > 1 {
            ids_to_delete.push(created_ids[1]);
        }

        let initial_count = manager.connection_count();

        // Perform bulk delete
        let mut success_count = 0;
        let mut failure_count = 0;

        for id in &ids_to_delete {
            match manager.delete_connection(*id) {
                Ok(()) => success_count += 1,
                Err(_) => failure_count += 1,
            }
        }

        // Should have at least one failure (the non-existent ID)
        prop_assert!(
            failure_count >= 1,
            "Should have at least one failure for non-existent ID"
        );

        // Should have successfully deleted the valid IDs
        let expected_successes = ids_to_delete
            .iter()
            .filter(|id| created_ids.contains(id))
            .count();

        prop_assert_eq!(
            success_count,
            expected_successes,
            "Should successfully delete all valid IDs despite failures"
        );

        // Verify the valid IDs were actually deleted
        for id in &ids_to_delete {
            if created_ids.contains(id) {
                prop_assert!(
                    manager.get_connection(*id).is_none(),
                    "Valid ID {:?} should be deleted",
                    id
                );
            }
        }

        // Verify remaining connections are intact
        let final_count = manager.connection_count();
        prop_assert_eq!(
            final_count,
            initial_count - success_count,
            "Remaining connection count should be correct"
        );
    }

    /// **Feature: rustconn-enhancements, Property 2: Bulk Delete Completeness**
    /// **Validates: Requirements 3.2, 3.3, 3.4**
    ///
    /// Bulk delete of groups should also work correctly, moving connections to ungrouped.
    #[test]
    fn bulk_delete_groups_moves_connections_to_ungrouped(
        group_names in prop::collection::vec(arb_group_name(), 2..5),
        conn in arb_connection(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create groups
        let mut group_ids = Vec::new();
        for name in &group_names {
            let id = manager.create_group(name.clone()).expect("Should create group");
            group_ids.push(id);
        }

        // Create a connection in the first group
        let conn_id = manager
            .create_connection_from(conn)
            .expect("Should create connection");

        manager
            .move_connection_to_group(conn_id, Some(group_ids[0]))
            .expect("Should move connection to group");

        // Verify connection is in group
        let conn_before = manager.get_connection(conn_id).expect("Connection should exist");
        prop_assert_eq!(
            conn_before.group_id,
            Some(group_ids[0]),
            "Connection should be in first group"
        );

        // Delete the first group
        manager
            .delete_group(group_ids[0])
            .expect("Should delete group");

        // Verify connection is now ungrouped
        let conn_after = manager.get_connection(conn_id).expect("Connection should still exist");
        prop_assert!(
            conn_after.group_id.is_none(),
            "Connection should be ungrouped after group deletion"
        );

        // Verify the group is gone
        prop_assert!(
            manager.get_group(group_ids[0]).is_none(),
            "Deleted group should not exist"
        );

        // Verify other groups still exist
        for &gid in group_ids.iter().skip(1) {
            prop_assert!(
                manager.get_group(gid).is_some(),
                "Other groups should still exist"
            );
        }
    }

    /// **Feature: rustconn-bugfixes, Property 3: Connection Duplication**
    /// **Validates: Requirements 3.3**
    ///
    /// For any connection, duplicating it SHALL create a new connection with "(copy)"
    /// suffix and different UUID while preserving all other fields.
    #[test]
    fn connection_duplication_preserves_fields_with_new_id(
        conn in arb_connection(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create original connection
        let original_id = manager
            .create_connection_from(conn.clone())
            .expect("Should create original connection");

        // Get the original connection
        let original = manager
            .get_connection(original_id)
            .expect("Original should exist")
            .clone();

        // Simulate duplication: clone, change name with "(copy)" suffix, generate new UUID
        let mut duplicate = original.clone();
        duplicate.id = Uuid::new_v4();
        duplicate.name = format!("{} (copy)", original.name);

        // Create the duplicate
        let duplicate_id = manager
            .create_connection_from(duplicate.clone())
            .expect("Should create duplicate connection");

        // Verify duplicate has different UUID
        prop_assert_ne!(
            duplicate_id, original_id,
            "Duplicate should have different UUID"
        );

        // Retrieve both connections
        let retrieved_original = manager
            .get_connection(original_id)
            .expect("Original should still exist");
        let retrieved_duplicate = manager
            .get_connection(duplicate_id)
            .expect("Duplicate should exist");

        // Verify name has "(copy)" suffix
        prop_assert!(
            retrieved_duplicate.name.contains("(copy)"),
            "Duplicate name should contain '(copy)' suffix"
        );

        // Verify all other fields are preserved
        prop_assert_eq!(
            &retrieved_duplicate.host,
            &retrieved_original.host,
            "Host should be preserved"
        );
        prop_assert_eq!(
            retrieved_duplicate.port,
            retrieved_original.port,
            "Port should be preserved"
        );
        prop_assert_eq!(
            &retrieved_duplicate.username,
            &retrieved_original.username,
            "Username should be preserved"
        );
        prop_assert_eq!(
            &retrieved_duplicate.protocol_config,
            &retrieved_original.protocol_config,
            "Protocol config should be preserved"
        );
        prop_assert_eq!(
            &retrieved_duplicate.tags,
            &retrieved_original.tags,
            "Tags should be preserved"
        );
        prop_assert_eq!(
            retrieved_duplicate.group_id,
            retrieved_original.group_id,
            "Group ID should be preserved"
        );

        // Verify both connections exist independently
        prop_assert_eq!(
            manager.connection_count(),
            2,
            "Should have 2 connections after duplication"
        );
    }

    /// **Feature: rustconn-bugfixes, Property 4: Group Deletion Cascade**
    /// **Validates: Requirements 3.7**
    ///
    /// For any group with connections, deleting the group with cascade SHALL remove
    /// all connections within that group.
    #[test]
    fn group_deletion_cascade_removes_all_connections(
        group_name in arb_group_name(),
        connections in prop::collection::vec(arb_connection(), 1..5),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create a group
        let group_id = manager
            .create_group(group_name)
            .expect("Should create group");

        // Create connections and move them to the group
        let mut conn_ids = Vec::new();
        for conn in connections {
            let conn_id = manager
                .create_connection_from(conn)
                .expect("Should create connection");
            manager
                .move_connection_to_group(conn_id, Some(group_id))
                .expect("Should move connection to group");
            conn_ids.push(conn_id);
        }

        // Verify connections are in the group
        let count_before = manager.count_connections_in_group(group_id);
        prop_assert_eq!(
            count_before,
            conn_ids.len(),
            "All connections should be in the group"
        );

        // Delete the group with cascade
        manager
            .delete_group_cascade(group_id)
            .expect("Should delete group with cascade");

        // Verify group is deleted
        prop_assert!(
            manager.get_group(group_id).is_none(),
            "Group should be deleted"
        );

        // Verify all connections in the group are deleted
        for conn_id in &conn_ids {
            prop_assert!(
                manager.get_connection(*conn_id).is_none(),
                "Connection should be deleted with group"
            );
        }

        // Verify connection count is 0
        prop_assert_eq!(
            manager.connection_count(),
            0,
            "All connections should be deleted"
        );
    }

    /// **Feature: rustconn-bugfixes, Property 4: Group Deletion Cascade**
    /// **Validates: Requirements 3.7**
    ///
    /// Cascade delete should also delete connections in nested child groups.
    #[test]
    fn group_deletion_cascade_includes_nested_groups(
        parent_name in arb_group_name(),
        child_name in arb_group_name(),
        parent_conn in arb_connection(),
        child_conn in arb_connection(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create parent group
        let parent_id = manager
            .create_group(parent_name)
            .expect("Should create parent group");

        // Create child group under parent
        let child_id = manager
            .create_group_with_parent(child_name, parent_id)
            .expect("Should create child group");

        // Create connection in parent group
        let parent_conn_id = manager
            .create_connection_from(parent_conn)
            .expect("Should create parent connection");
        manager
            .move_connection_to_group(parent_conn_id, Some(parent_id))
            .expect("Should move connection to parent group");

        // Create connection in child group
        let child_conn_id = manager
            .create_connection_from(child_conn)
            .expect("Should create child connection");
        manager
            .move_connection_to_group(child_conn_id, Some(child_id))
            .expect("Should move connection to child group");

        // Verify both connections are counted
        let count = manager.count_connections_in_group(parent_id);
        prop_assert_eq!(count, 2, "Should count connections in parent and child groups");

        // Delete parent group with cascade
        manager
            .delete_group_cascade(parent_id)
            .expect("Should delete parent group with cascade");

        // Verify both groups are deleted
        prop_assert!(
            manager.get_group(parent_id).is_none(),
            "Parent group should be deleted"
        );
        prop_assert!(
            manager.get_group(child_id).is_none(),
            "Child group should be deleted"
        );

        // Verify both connections are deleted
        prop_assert!(
            manager.get_connection(parent_conn_id).is_none(),
            "Parent connection should be deleted"
        );
        prop_assert!(
            manager.get_connection(child_conn_id).is_none(),
            "Child connection should be deleted"
        );
    }
}

// ========== Group-Scoped Sorting Property Tests ==========

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-bugfixes, Property 5: Group-Scoped Sorting**
    /// **Validates: Requirements 4.1, 4.2, 4.3, 4.4**
    ///
    /// For any set of connections, sorting within a group SHALL only reorder
    /// connections in that group, leaving other groups unchanged.
    #[test]
    fn sort_group_only_affects_target_group(
        group1_name in arb_group_name(),
        group2_name in arb_group_name(),
        conn_names_g1 in prop::collection::vec(arb_name(), 2..5),
        conn_names_g2 in prop::collection::vec(arb_name(), 2..5),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create two groups
        let group1_id = manager.create_group(group1_name).expect("Should create group 1");
        let group2_id = manager.create_group(group2_name).expect("Should create group 2");

        // Create connections in group 1
        let mut group1_conn_ids = Vec::new();
        for name in &conn_names_g1 {
            let conn = Connection::new_ssh(name.clone(), "host1.example.com".to_string(), 22)
                .with_group(group1_id);
            let id = manager.create_connection_from(conn).expect("Should create connection");
            group1_conn_ids.push(id);
        }

        // Create connections in group 2
        let mut group2_conn_ids = Vec::new();
        for name in &conn_names_g2 {
            let conn = Connection::new_ssh(name.clone(), "host2.example.com".to_string(), 22)
                .with_group(group2_id);
            let id = manager.create_connection_from(conn).expect("Should create connection");
            group2_conn_ids.push(id);
        }

        // Record original sort_order values for group 2 connections
        let original_group2_orders: Vec<(Uuid, i32)> = group2_conn_ids
            .iter()
            .map(|&id| {
                let conn = manager.get_connection(id).unwrap();
                (id, conn.sort_order)
            })
            .collect();

        // Sort only group 1
        manager.sort_group(group1_id).expect("Should sort group 1");

        // Verify group 1 connections are sorted alphabetically
        let group1_conns: Vec<_> = manager.get_by_group(group1_id);
        let mut sorted_names: Vec<_> = group1_conns.iter().map(|c| c.name.to_lowercase()).collect();
        sorted_names.sort();

        let actual_names: Vec<_> = {
            let mut conns: Vec<_> = group1_conns.iter().collect();
            conns.sort_by_key(|c| c.sort_order);
            conns.iter().map(|c| c.name.to_lowercase()).collect()
        };

        prop_assert_eq!(
            actual_names, sorted_names,
            "Group 1 connections should be sorted alphabetically by sort_order"
        );

        // Verify group 2 connections are unchanged (sort_order preserved)
        for (id, original_order) in &original_group2_orders {
            let conn = manager.get_connection(*id).expect("Connection should exist");
            prop_assert_eq!(
                conn.sort_order, *original_order,
                "Group 2 connection sort_order should be unchanged after sorting group 1"
            );
        }
    }

    /// **Feature: rustconn-bugfixes, Property 5: Group-Scoped Sorting**
    /// **Validates: Requirements 4.2, 4.3**
    ///
    /// For any set of connections, sort_all SHALL sort all groups and their
    /// connections alphabetically, including ungrouped connections.
    #[test]
    fn sort_all_sorts_everything_alphabetically(
        group_names in prop::collection::vec(arb_group_name(), 1..4),
        conn_names in prop::collection::vec(arb_name(), 3..8),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create groups
        let mut group_ids = Vec::new();
        for name in &group_names {
            let id = manager.create_group(name.clone()).expect("Should create group");
            group_ids.push(id);
        }

        // Create connections - some in groups, some ungrouped
        for (i, name) in conn_names.iter().enumerate() {
            let conn = if i < group_ids.len() {
                // Put in a group
                Connection::new_ssh(name.clone(), "host.example.com".to_string(), 22)
                    .with_group(group_ids[i % group_ids.len()])
            } else {
                // Ungrouped
                Connection::new_ssh(name.clone(), "host.example.com".to_string(), 22)
            };
            manager.create_connection_from(conn).expect("Should create connection");
        }

        // Sort all
        manager.sort_all().expect("Should sort all");

        // Verify root groups are sorted alphabetically by sort_order
        let root_groups = manager.get_root_groups();
        let mut sorted_group_names: Vec<_> = root_groups.iter().map(|g| g.name.to_lowercase()).collect();
        sorted_group_names.sort();

        let actual_group_names: Vec<_> = {
            let mut groups: Vec<_> = root_groups.iter().collect();
            groups.sort_by_key(|g| g.sort_order);
            groups.iter().map(|g| g.name.to_lowercase()).collect()
        };

        prop_assert_eq!(
            actual_group_names, sorted_group_names,
            "Root groups should be sorted alphabetically by sort_order"
        );

        // Verify ungrouped connections are sorted alphabetically by sort_order
        let ungrouped = manager.get_ungrouped();
        if !ungrouped.is_empty() {
            let mut sorted_conn_names: Vec<_> = ungrouped.iter().map(|c| c.name.to_lowercase()).collect();
            sorted_conn_names.sort();

            let actual_conn_names: Vec<_> = {
                let mut conns: Vec<_> = ungrouped.iter().collect();
                conns.sort_by_key(|c| c.sort_order);
                conns.iter().map(|c| c.name.to_lowercase()).collect()
            };

            prop_assert_eq!(
                actual_conn_names, sorted_conn_names,
                "Ungrouped connections should be sorted alphabetically by sort_order"
            );
        }

        // Verify connections within each group are sorted alphabetically
        for group_id in &group_ids {
            let group_conns = manager.get_by_group(*group_id);
            if !group_conns.is_empty() {
                let mut sorted_names: Vec<_> = group_conns.iter().map(|c| c.name.to_lowercase()).collect();
                sorted_names.sort();

                let actual_names: Vec<_> = {
                    let mut conns: Vec<_> = group_conns.iter().collect();
                    conns.sort_by_key(|c| c.sort_order);
                    conns.iter().map(|c| c.name.to_lowercase()).collect()
                };

                prop_assert_eq!(
                    actual_names, sorted_names,
                    "Connections in group should be sorted alphabetically by sort_order"
                );
            }
        }
    }

    /// **Feature: rustconn-bugfixes, Property 5: Group-Scoped Sorting**
    /// **Validates: Requirements 4.4**
    ///
    /// After sorting, the sort order SHALL be persisted to configuration.
    #[test]
    fn sort_persists_to_configuration(
        group_name in arb_group_name(),
        conn_names in prop::collection::vec(arb_name(), 2..5),
    ) {
        let temp_dir = TempDir::new().unwrap();
        let config_manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());

        // Create manager and add data
        let group_id;
        let conn_ids: Vec<Uuid>;
        {
            let mut manager = ConnectionManager::new(config_manager.clone()).unwrap();

            group_id = manager.create_group(group_name).expect("Should create group");

            conn_ids = conn_names
                .iter()
                .map(|name| {
                    let conn = Connection::new_ssh(name.clone(), "host.example.com".to_string(), 22)
                        .with_group(group_id);
                    manager.create_connection_from(conn).expect("Should create connection")
                })
                .collect();

            // Sort the group
            manager.sort_group(group_id).expect("Should sort group");
        }

        // Create a new manager to reload from disk
        let manager2 = ConnectionManager::new(config_manager).unwrap();

        // Verify sort_order values were persisted
        for conn_id in &conn_ids {
            let conn = manager2.get_connection(*conn_id).expect("Connection should exist after reload");
            // Just verify the connection exists and has a valid sort_order
            prop_assert!(
                conn.sort_order >= 0,
                "Sort order should be non-negative after reload"
            );
        }

        // Verify the connections are still sorted alphabetically by sort_order
        let group_conns = manager2.get_by_group(group_id);
        let mut sorted_names: Vec<_> = group_conns.iter().map(|c| c.name.to_lowercase()).collect();
        sorted_names.sort();

        let actual_names: Vec<_> = {
            let mut conns: Vec<_> = group_conns.iter().collect();
            conns.sort_by_key(|c| c.sort_order);
            conns.iter().map(|c| c.name.to_lowercase()).collect()
        };

        prop_assert_eq!(
            actual_names, sorted_names,
            "Sort order should be persisted and connections should remain sorted after reload"
        );
    }

    /// **Feature: rustconn-bugfixes, Property 6: Drag-Drop Reordering**
    /// **Validates: Requirements 5.1, 5.2, 5.3, 5.5**
    ///
    /// For any connection moved via drag-drop, the connection's group_id and sort_order
    /// SHALL be updated correctly.
    #[test]
    fn drag_drop_reorder_updates_sort_order(
        group_name in arb_group_name(),
        conn_names in prop::collection::vec(arb_name(), 3..6),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create a group
        let group_id = manager
            .create_group(group_name)
            .expect("Should create group");

        // Create connections in the group
        let mut conn_ids = Vec::new();
        for name in &conn_names {
            let conn = Connection::new_ssh(name.clone(), "host.example.com".to_string(), 22)
                .with_group(group_id);
            let conn_id = manager
                .create_connection_from(conn)
                .expect("Should create connection");
            conn_ids.push(conn_id);
        }

        // Verify we have at least 3 connections
        prop_assert!(conn_ids.len() >= 3, "Need at least 3 connections for reorder test");

        // Get initial sort orders (for verification that they change)
        let _initial_orders: Vec<i32> = conn_ids
            .iter()
            .map(|id| manager.get_connection(*id).unwrap().sort_order)
            .collect();

        // Reorder: move first connection after the last one
        let source_id = conn_ids[0];
        let target_id = conn_ids[conn_ids.len() - 1];

        manager
            .reorder_connection(source_id, target_id)
            .expect("Should reorder connection");

        // Verify the source connection is now after the target
        let source_order = manager.get_connection(source_id).unwrap().sort_order;
        let target_order = manager.get_connection(target_id).unwrap().sort_order;

        prop_assert!(
            source_order > target_order || source_order == target_order + 1,
            "Source should be positioned after target: source_order={}, target_order={}",
            source_order, target_order
        );

        // Verify all connections still have unique sort orders
        let mut orders: Vec<i32> = conn_ids
            .iter()
            .map(|id| manager.get_connection(*id).unwrap().sort_order)
            .collect();
        orders.sort();
        orders.dedup();
        prop_assert_eq!(
            orders.len(),
            conn_ids.len(),
            "All connections should have unique sort orders"
        );

        // Verify sort orders are sequential (0, 1, 2, ...)
        for (idx, order) in orders.iter().enumerate() {
            prop_assert_eq!(
                *order,
                i32::try_from(idx).unwrap(),
                "Sort orders should be sequential"
            );
        }
    }

    /// **Feature: rustconn-bugfixes, Property 6: Drag-Drop Reordering**
    /// **Validates: Requirements 5.2, 5.3**
    ///
    /// Moving a connection to a different group via drag-drop SHALL update
    /// the group_id and assign a valid sort_order in the new group.
    #[test]
    fn drag_drop_move_to_group_updates_group_id(
        group1_name in arb_group_name(),
        group2_name in arb_group_name(),
        conn in arb_connection(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create two groups
        let group1_id = manager
            .create_group(group1_name)
            .expect("Should create group 1");
        let group2_id = manager
            .create_group(group2_name)
            .expect("Should create group 2");

        // Create connection in group 1
        let conn_id = manager
            .create_connection_from(conn)
            .expect("Should create connection");
        manager
            .move_connection_to_group(conn_id, Some(group1_id))
            .expect("Should move to group 1");

        // Verify connection is in group 1
        let conn_before = manager.get_connection(conn_id).unwrap();
        prop_assert_eq!(
            conn_before.group_id,
            Some(group1_id),
            "Connection should be in group 1"
        );

        // Move connection to group 2
        manager
            .move_connection_to_group(conn_id, Some(group2_id))
            .expect("Should move to group 2");

        // Verify connection is now in group 2
        let conn_after = manager.get_connection(conn_id).unwrap();
        prop_assert_eq!(
            conn_after.group_id,
            Some(group2_id),
            "Connection should be in group 2"
        );

        // Verify sort_order is valid (non-negative)
        prop_assert!(
            conn_after.sort_order >= 0,
            "Sort order should be non-negative"
        );
    }

    /// **Feature: rustconn-bugfixes, Property 6: Drag-Drop Reordering**
    /// **Validates: Requirements 5.3**
    ///
    /// Moving a connection to root (no group) SHALL set group_id to None
    /// and assign a valid sort_order.
    #[test]
    fn drag_drop_move_to_root_removes_group(
        group_name in arb_group_name(),
        conn in arb_connection(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create a group
        let group_id = manager
            .create_group(group_name)
            .expect("Should create group");

        // Create connection in the group
        let conn_id = manager
            .create_connection_from(conn)
            .expect("Should create connection");
        manager
            .move_connection_to_group(conn_id, Some(group_id))
            .expect("Should move to group");

        // Verify connection is in the group
        let conn_before = manager.get_connection(conn_id).unwrap();
        prop_assert_eq!(
            conn_before.group_id,
            Some(group_id),
            "Connection should be in group"
        );

        // Move connection to root (None)
        manager
            .move_connection_to_group(conn_id, None)
            .expect("Should move to root");

        // Verify connection is now ungrouped
        let conn_after = manager.get_connection(conn_id).unwrap();
        prop_assert!(
            conn_after.group_id.is_none(),
            "Connection should be ungrouped"
        );

        // Verify sort_order is valid (non-negative)
        prop_assert!(
            conn_after.sort_order >= 0,
            "Sort order should be non-negative"
        );
    }

    /// **Feature: rustconn-bugfixes, Property 6: Drag-Drop Reordering**
    /// **Validates: Requirements 5.5**
    ///
    /// After drag-drop reordering, the new order SHALL be persisted to configuration.
    #[test]
    fn drag_drop_reorder_persists_to_config(
        group_name in arb_group_name(),
        conn_names in prop::collection::vec(arb_name(), 3..5),
    ) {
        let temp_dir = TempDir::new().unwrap();
        let config_manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());

        let group_id;
        let conn_ids: Vec<Uuid>;
        let source_id;
        let target_id;

        // Create and reorder in first manager instance
        {
            let mut manager = ConnectionManager::new(config_manager.clone()).unwrap();

            group_id = manager.create_group(group_name).expect("Should create group");

            conn_ids = conn_names
                .iter()
                .map(|name| {
                    let conn = Connection::new_ssh(name.clone(), "host.example.com".to_string(), 22)
                        .with_group(group_id);
                    manager.create_connection_from(conn).expect("Should create connection")
                })
                .collect();

            source_id = conn_ids[0];
            target_id = conn_ids[conn_ids.len() - 1];

            // Reorder: move first connection after the last one
            manager
                .reorder_connection(source_id, target_id)
                .expect("Should reorder connection");
        }

        // Create a new manager to reload from disk
        let manager2 = ConnectionManager::new(config_manager).unwrap();

        // Verify the reordering was persisted
        let source_order = manager2.get_connection(source_id).unwrap().sort_order;
        let target_order = manager2.get_connection(target_id).unwrap().sort_order;

        prop_assert!(
            source_order > target_order || source_order == target_order + 1,
            "Reordering should be persisted: source_order={}, target_order={}",
            source_order, target_order
        );
    }
}

// ========== Sort Recent Property Tests ==========

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-bugfixes, Property 7: Recent Sort Ordering**
    /// **Validates: Requirements 6.1, 6.2**
    ///
    /// For any set of connections with timestamps, sorting by recent SHALL place
    /// connections with more recent timestamps first, and connections without
    /// timestamps last.
    #[test]
    fn sort_by_recent_orders_by_timestamp(
        conn_names in prop::collection::vec(arb_name(), 3..8),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create connections
        let mut conn_ids = Vec::new();
        for name in &conn_names {
            let id = manager
                .create_connection(
                    name.clone(),
                    "host.example.com".to_string(),
                    22,
                    ProtocolConfig::Ssh(SshConfig::default()),
                )
                .expect("Should create connection");
            conn_ids.push(id);
        }

        // Set last_connected timestamps for some connections (not all)
        // Use different timestamps to ensure ordering
        let now = chrono::Utc::now();
        for (i, &conn_id) in conn_ids.iter().enumerate() {
            if i % 2 == 0 {
                // Set timestamp for even-indexed connections
                // Earlier connections get older timestamps
                if let Some(conn) = manager.get_connection_mut(conn_id) {
                    conn.last_connected = Some(now - chrono::Duration::hours(i as i64));
                }
            }
            // Odd-indexed connections have no timestamp (None)
        }

        // Sort by recent
        manager.sort_by_recent().expect("Should sort by recent");

        // Verify ordering
        let connections: Vec<_> = manager.list_connections();
        let mut sorted_conns: Vec<_> = connections.iter().collect();
        sorted_conns.sort_by_key(|c| c.sort_order);

        // Check that connections with timestamps come before those without
        let mut seen_none = false;
        let mut prev_timestamp: Option<chrono::DateTime<chrono::Utc>> = None;

        for conn in &sorted_conns {
            match conn.last_connected {
                Some(ts) => {
                    // Should not see a timestamp after we've seen None
                    prop_assert!(
                        !seen_none,
                        "Connections with timestamps should come before those without"
                    );
                    // Timestamps should be in descending order (most recent first)
                    if let Some(prev) = prev_timestamp {
                        prop_assert!(
                            ts <= prev,
                            "Timestamps should be in descending order: {} should be <= {}",
                            ts, prev
                        );
                    }
                    prev_timestamp = Some(ts);
                }
                None => {
                    seen_none = true;
                }
            }
        }
    }

    /// **Feature: rustconn-bugfixes, Property 7: Recent Sort Ordering**
    /// **Validates: Requirements 6.1, 6.2**
    ///
    /// Connections without last_connected timestamp SHALL be placed at the end
    /// of the list when sorting by recent.
    #[test]
    fn sort_by_recent_places_none_timestamps_last(
        conn_names in prop::collection::vec(arb_name(), 2..6),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create connections - first half with timestamps, second half without
        let mut conn_ids = Vec::new();
        for name in &conn_names {
            let id = manager
                .create_connection(
                    name.clone(),
                    "host.example.com".to_string(),
                    22,
                    ProtocolConfig::Ssh(SshConfig::default()),
                )
                .expect("Should create connection");
            conn_ids.push(id);
        }

        // Set timestamps for first half only
        let now = chrono::Utc::now();
        let half = conn_ids.len() / 2;
        for (i, &conn_id) in conn_ids.iter().take(half.max(1)).enumerate() {
            if let Some(conn) = manager.get_connection_mut(conn_id) {
                conn.last_connected = Some(now - chrono::Duration::minutes(i as i64));
            }
        }

        // Sort by recent
        manager.sort_by_recent().expect("Should sort by recent");

        // Get sorted connections
        let connections: Vec<_> = manager.list_connections();
        let mut sorted_conns: Vec<_> = connections.iter().collect();
        sorted_conns.sort_by_key(|c| c.sort_order);

        // Count connections with and without timestamps
        let with_timestamp: Vec<_> = sorted_conns.iter().filter(|c| c.last_connected.is_some()).collect();
        let without_timestamp: Vec<_> = sorted_conns.iter().filter(|c| c.last_connected.is_none()).collect();

        // Verify all connections with timestamps have lower sort_order than those without
        if !with_timestamp.is_empty() && !without_timestamp.is_empty() {
            let max_with_ts = with_timestamp.iter().map(|c| c.sort_order).max().unwrap();
            let min_without_ts = without_timestamp.iter().map(|c| c.sort_order).min().unwrap();

            prop_assert!(
                max_with_ts < min_without_ts,
                "All connections with timestamps should have lower sort_order than those without: max_with={}, min_without={}",
                max_with_ts, min_without_ts
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-bugfixes, Property 8: Last Connected Update**
    /// **Validates: Requirements 6.4**
    ///
    /// For any connection, after calling update_last_connected, the last_connected
    /// timestamp SHALL be updated to approximately the current time.
    #[test]
    fn update_last_connected_sets_timestamp(
        name in arb_name(),
        host in arb_host(),
        port in arb_port(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create a connection
        let conn_id = manager
            .create_connection(
                name,
                host,
                port,
                ProtocolConfig::Ssh(SshConfig::default()),
            )
            .expect("Should create connection");

        // Verify initial last_connected is None
        let conn_before = manager.get_connection(conn_id).expect("Should get connection");
        prop_assert!(
            conn_before.last_connected.is_none(),
            "Initial last_connected should be None"
        );

        // Record time before update
        let before_update = chrono::Utc::now();

        // Update last_connected
        manager
            .update_last_connected(conn_id)
            .expect("Should update last_connected");

        // Record time after update
        let after_update = chrono::Utc::now();

        // Verify last_connected is now set
        let conn_after = manager.get_connection(conn_id).expect("Should get connection");
        prop_assert!(
            conn_after.last_connected.is_some(),
            "last_connected should be set after update"
        );

        let timestamp = conn_after.last_connected.unwrap();

        // Verify timestamp is within the expected range
        prop_assert!(
            timestamp >= before_update && timestamp <= after_update,
            "Timestamp {} should be between {} and {}",
            timestamp, before_update, after_update
        );
    }

    /// **Feature: rustconn-bugfixes, Property 8: Last Connected Update**
    /// **Validates: Requirements 6.4**
    ///
    /// Calling update_last_connected multiple times SHALL update the timestamp
    /// to a more recent value each time.
    #[test]
    fn update_last_connected_updates_to_newer_timestamp(
        name in arb_name(),
        host in arb_host(),
        port in arb_port(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create a connection
        let conn_id = manager
            .create_connection(
                name,
                host,
                port,
                ProtocolConfig::Ssh(SshConfig::default()),
            )
            .expect("Should create connection");

        // First update
        manager
            .update_last_connected(conn_id)
            .expect("Should update last_connected");

        let first_timestamp = manager
            .get_connection(conn_id)
            .expect("Should get connection")
            .last_connected
            .expect("Should have timestamp");

        // Small delay to ensure different timestamp
        std::thread::sleep(std::time::Duration::from_millis(10));

        // Second update
        manager
            .update_last_connected(conn_id)
            .expect("Should update last_connected again");

        let second_timestamp = manager
            .get_connection(conn_id)
            .expect("Should get connection")
            .last_connected
            .expect("Should have timestamp");

        // Second timestamp should be >= first (could be equal if very fast)
        prop_assert!(
            second_timestamp >= first_timestamp,
            "Second timestamp {} should be >= first timestamp {}",
            second_timestamp, first_timestamp
        );
    }
}

// ========== Connection Naming Property Tests ==========

use rustconn_core::ProtocolType;

// Strategy for generating protocol types
fn arb_protocol_type() -> impl Strategy<Value = ProtocolType> {
    prop_oneof![
        Just(ProtocolType::Ssh),
        Just(ProtocolType::Rdp),
        Just(ProtocolType::Vnc),
        Just(ProtocolType::Spice),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: native-protocol-embedding, Property 13: Connection Name Deduplication**
    /// **Validates: Requirements 4.1, 4.2**
    ///
    /// For any new connection with a name that already exists, the generated name
    /// should include a protocol suffix and be unique among all connections.
    #[test]
    fn connection_name_deduplication_generates_unique_names(
        base_name in arb_name(),
        protocols in prop::collection::vec(arb_protocol_type(), 1..5),
        hosts in prop::collection::vec(arb_host(), 1..5),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create multiple connections with the same base name
        let mut created_names = Vec::new();

        for (i, protocol) in protocols.iter().enumerate() {
            let host = hosts.get(i % hosts.len()).cloned().unwrap_or_else(|| "example.com".to_string());

            // Generate unique name
            let unique_name = manager.generate_unique_name(&base_name, *protocol);

            // Verify the generated name is unique among all existing connections
            prop_assert!(
                !manager.name_exists(&unique_name, None),
                "Generated name '{}' should be unique before creation",
                unique_name
            );

            // Create the connection with the generated name
            let protocol_config = match protocol {
                ProtocolType::Ssh => ProtocolConfig::Ssh(SshConfig::default()),
                ProtocolType::Rdp => ProtocolConfig::Rdp(RdpConfig::default()),
                ProtocolType::Vnc => ProtocolConfig::Vnc(VncConfig::default()),
                ProtocolType::Spice => ProtocolConfig::Spice(rustconn_core::SpiceConfig::default()),
                ProtocolType::ZeroTrust => ProtocolConfig::Ssh(SshConfig::default()), // Fallback
            };

            manager
                .create_connection(unique_name.clone(), host, 22, protocol_config)
                .expect("Should create connection");

            // Verify the name is now taken
            prop_assert!(
                manager.name_exists(&unique_name, None),
                "Name '{}' should exist after creation",
                unique_name
            );

            // Verify all created names are unique
            prop_assert!(
                !created_names.contains(&unique_name),
                "Generated name '{}' should not duplicate previous names",
                unique_name
            );

            created_names.push(unique_name);
        }

        // Verify all connections have unique names
        let all_names: Vec<String> = manager.list_connections().iter().map(|c| c.name.clone()).collect();
        let unique_count = {
            let mut sorted = all_names.clone();
            sorted.sort();
            sorted.dedup();
            sorted.len()
        };
        prop_assert_eq!(
            all_names.len(),
            unique_count,
            "All connection names should be unique"
        );
    }

    /// **Feature: native-protocol-embedding, Property 13: Connection Name Deduplication**
    /// **Validates: Requirements 4.1, 4.2**
    ///
    /// When a name already exists with a different protocol, the generated name
    /// should include the protocol suffix.
    #[test]
    fn name_deduplication_adds_protocol_suffix_for_different_protocol(
        base_name in arb_name(),
        first_protocol in arb_protocol_type(),
        second_protocol in arb_protocol_type(),
        host in arb_host(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create first connection with base name
        let first_config = match first_protocol {
            ProtocolType::Ssh => ProtocolConfig::Ssh(SshConfig::default()),
            ProtocolType::Rdp => ProtocolConfig::Rdp(RdpConfig::default()),
            ProtocolType::Vnc => ProtocolConfig::Vnc(VncConfig::default()),
            ProtocolType::Spice => ProtocolConfig::Spice(rustconn_core::SpiceConfig::default()),
            ProtocolType::ZeroTrust => ProtocolConfig::Ssh(SshConfig::default()),
        };

        manager
            .create_connection(base_name.clone(), host.clone(), 22, first_config)
            .expect("Should create first connection");

        // Generate name for second connection
        let second_name = manager.generate_unique_name(&base_name, second_protocol);

        // If protocols are different, name should have protocol suffix
        // If protocols are same, name should have protocol suffix (since base is taken)
        prop_assert!(
            second_name != base_name,
            "Second name '{}' should differ from base name '{}' since base is taken",
            second_name, base_name
        );

        // Name should contain protocol suffix
        let expected_suffix = format!("({})", second_protocol);
        prop_assert!(
            second_name.contains(&expected_suffix),
            "Second name '{}' should contain protocol suffix '{}'",
            second_name, expected_suffix
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: native-protocol-embedding, Property 14: Name Suffix Removal**
    /// **Validates: Requirements 4.3**
    ///
    /// For any connection rename to a unique name, any auto-generated suffix
    /// should be removed from the final name.
    #[test]
    fn name_suffix_removal_when_base_becomes_unique(
        base_name in arb_name(),
        protocol in arb_protocol_type(),
        host in arb_host(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create a connection with a suffixed name (simulating previous deduplication)
        let suffixed_name = format!("{} ({})", base_name, protocol);

        let protocol_config = match protocol {
            ProtocolType::Ssh => ProtocolConfig::Ssh(SshConfig::default()),
            ProtocolType::Rdp => ProtocolConfig::Rdp(RdpConfig::default()),
            ProtocolType::Vnc => ProtocolConfig::Vnc(VncConfig::default()),
            ProtocolType::Spice => ProtocolConfig::Spice(rustconn_core::SpiceConfig::default()),
            ProtocolType::ZeroTrust => ProtocolConfig::Ssh(SshConfig::default()),
        };

        let conn_id = manager
            .create_connection(suffixed_name.clone(), host, 22, protocol_config)
            .expect("Should create connection");

        // Since base_name is unique (no other connection has it), normalize should remove suffix
        let normalized = manager.normalize_name(&suffixed_name, conn_id);

        // The normalized name should be the base name (suffix removed)
        prop_assert_eq!(
            &normalized, &base_name,
            "Normalized name '{}' should equal base name '{}' when base is unique",
            normalized, base_name
        );
    }

    /// **Feature: native-protocol-embedding, Property 14: Name Suffix Removal**
    /// **Validates: Requirements 4.3**
    ///
    /// When the base name is still taken by another connection, the suffix
    /// should be preserved.
    #[test]
    fn name_suffix_preserved_when_base_is_taken(
        base_name in arb_name(),
        protocol in arb_protocol_type(),
        host1 in arb_host(),
        host2 in arb_host(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create first connection with base name
        let first_config = match protocol {
            ProtocolType::Ssh => ProtocolConfig::Ssh(SshConfig::default()),
            ProtocolType::Rdp => ProtocolConfig::Rdp(RdpConfig::default()),
            ProtocolType::Vnc => ProtocolConfig::Vnc(VncConfig::default()),
            ProtocolType::Spice => ProtocolConfig::Spice(rustconn_core::SpiceConfig::default()),
            ProtocolType::ZeroTrust => ProtocolConfig::Ssh(SshConfig::default()),
        };

        manager
            .create_connection(base_name.clone(), host1, 22, first_config.clone())
            .expect("Should create first connection");

        // Create second connection with suffixed name
        let suffixed_name = format!("{} ({})", base_name, protocol);
        let second_id = manager
            .create_connection(suffixed_name.clone(), host2, 22, first_config)
            .expect("Should create second connection");

        // Since base_name is taken, normalize should preserve the suffix
        let normalized = manager.normalize_name(&suffixed_name, second_id);

        // The normalized name should still have the suffix
        prop_assert_eq!(
            &normalized, &suffixed_name,
            "Normalized name '{}' should preserve suffix when base '{}' is taken",
            normalized, base_name
        );
    }

    /// **Feature: native-protocol-embedding, Property 14: Name Suffix Removal**
    /// **Validates: Requirements 4.3**
    ///
    /// Numeric suffixes should also be removed when the base name becomes unique.
    #[test]
    fn numeric_suffix_removal_when_base_becomes_unique(
        base_name in arb_name(),
        protocol in arb_protocol_type(),
        suffix_number in 2u32..100u32,
        host in arb_host(),
    ) {
        let (mut manager, _temp) = create_test_manager();

        // Create a connection with a numeric suffixed name
        let suffixed_name = format!("{} ({}) {}", base_name, protocol, suffix_number);

        let protocol_config = match protocol {
            ProtocolType::Ssh => ProtocolConfig::Ssh(SshConfig::default()),
            ProtocolType::Rdp => ProtocolConfig::Rdp(RdpConfig::default()),
            ProtocolType::Vnc => ProtocolConfig::Vnc(VncConfig::default()),
            ProtocolType::Spice => ProtocolConfig::Spice(rustconn_core::SpiceConfig::default()),
            ProtocolType::ZeroTrust => ProtocolConfig::Ssh(SshConfig::default()),
        };

        let conn_id = manager
            .create_connection(suffixed_name.clone(), host, 22, protocol_config)
            .expect("Should create connection");

        // Since base_name is unique, normalize should remove the entire suffix
        let normalized = manager.normalize_name(&suffixed_name, conn_id);

        // The normalized name should be the base name
        prop_assert_eq!(
            &normalized, &base_name,
            "Normalized name '{}' should equal base name '{}' when base is unique",
            normalized, base_name
        );
    }
}
