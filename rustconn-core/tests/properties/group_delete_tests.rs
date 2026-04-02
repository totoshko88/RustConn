//! Property-based tests for group deletion operations
//!
//! Tests cascade delete, reparenting delete, and leaf-group equivalence
//! to verify the orphaned-subgroups-on-delete bugfix.

use proptest::prelude::*;
use rustconn_core::{ConfigManager, Connection, ConnectionManager, ProtocolConfig, SshConfig};
use tempfile::TempDir;
use uuid::Uuid;

// ========== Test Helper ==========

/// Creates a fresh `ConnectionManager` backed by a temp directory.
/// Returns the manager, temp dir (must be kept alive), and Tokio runtime.
fn create_test_manager() -> (ConnectionManager, TempDir, tokio::runtime::Runtime) {
    let runtime =
        tokio::runtime::Runtime::new().expect("Tokio runtime creation should never fail in tests");
    let temp_dir = TempDir::new().expect("TempDir creation should never fail in tests");
    let config_manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());
    let manager = runtime.block_on(async {
        ConnectionManager::new(config_manager)
            .expect("ConnectionManager::new should never fail with a valid temp dir")
    });
    (manager, temp_dir, runtime)
}

/// Creates a simple SSH connection assigned to the given group.
fn create_grouped_connection(manager: &mut ConnectionManager, group_id: Uuid) -> Uuid {
    let conn = Connection::new(
        format!("conn-{}", Uuid::new_v4()),
        "localhost".to_string(),
        22,
        ProtocolConfig::Ssh(SshConfig::default()),
    )
    .with_group(group_id);
    manager
        .create_connection_from(conn)
        .expect("Creating a grouped connection should succeed")
}

// ========== Strategies ==========

/// Strategy for a valid group name (1-20 alphanumeric chars).
fn arb_group_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9]{0,19}"
}

/// Strategy that generates a tree depth (1-4 levels) and breadth (1-3 children per node).
/// Returns (depth, breadth) tuple used to build a group tree.
fn arb_tree_shape() -> impl Strategy<Value = (usize, usize)> {
    (1usize..=4, 1usize..=3)
}

/// Builds a group tree of the given depth and breadth under a root group.
/// Returns (root_id, all_group_ids_including_root).
fn build_group_tree(
    manager: &mut ConnectionManager,
    depth: usize,
    breadth: usize,
) -> (Uuid, Vec<Uuid>) {
    let root_id = manager
        .create_group("root".to_string())
        .expect("Root group creation should succeed");
    let mut all_ids = vec![root_id];
    let mut current_level = vec![root_id];

    for level in 1..depth {
        let mut next_level = Vec::new();
        for &parent_id in &current_level {
            for child_idx in 0..breadth {
                let name = format!("g-{level}-{child_idx}");
                let child_id = manager
                    .create_group_with_parent(name, parent_id)
                    .expect("Child group creation should succeed");
                all_ids.push(child_id);
                next_level.push(child_id);
            }
        }
        current_level = next_level;
    }

    (root_id, all_ids)
}

// ========== Property Tests ==========

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    /// **Property 1: Cascade delete removes all empty descendants**
    /// **Validates: Requirements 2.1, 2.2**
    ///
    /// For any group tree where all subgroups are empty (zero connections),
    /// calling `delete_group_cascade()` on the root removes every descendant
    /// group and leaves no dangling `parent_id` references.
    #[test]
    fn cascade_delete_removes_all_descendants_no_dangling_refs(
        (depth, breadth) in arb_tree_shape(),
    ) {
        let (mut manager, _temp, _runtime) = create_test_manager();
        let (root_id, all_ids) = build_group_tree(&mut manager, depth, breadth);

        // Precondition: all groups exist
        for &gid in &all_ids {
            prop_assert!(manager.get_group(gid).is_some(), "Group {gid} should exist before delete");
        }

        // Act: cascade delete the root
        manager
            .delete_group_cascade(root_id)
            .expect("delete_group_cascade should succeed");

        // Assert: every group in the tree is gone
        for &gid in &all_ids {
            prop_assert!(
                manager.get_group(gid).is_none(),
                "Group {gid} should be removed after cascade delete"
            );
        }

        // Assert: no remaining group references any deleted ID as parent_id
        let deleted_set: std::collections::HashSet<Uuid> = all_ids.iter().copied().collect();
        for group in manager.list_groups() {
            if let Some(pid) = group.parent_id {
                prop_assert!(
                    !deleted_set.contains(&pid),
                    "Group {} has dangling parent_id {pid} referencing a deleted group",
                    group.id
                );
            }
        }
    }

    /// **Property 2: delete_group reparents children and ungroups connections**
    /// **Validates: Requirements 2.3, 2.4**
    ///
    /// For any group with child subgroups, calling `delete_group()` reparents
    /// all direct children to the deleted group's parent and ungroups all
    /// direct connections. No dangling `parent_id` references remain.
    #[test]
    fn delete_group_reparents_children_and_ungroups_connections(
        num_children in 1usize..=5,
        num_connections in 0usize..=4,
        child_name_seed in arb_group_name(),
    ) {
        let (mut manager, _temp, _runtime) = create_test_manager();

        // Create a root group and a target group under it
        let root_id = manager
            .create_group("root".to_string())
            .expect("Root group creation should succeed");
        let target_id = manager
            .create_group_with_parent("target".to_string(), root_id)
            .expect("Target group creation should succeed");

        // The target's parent is root_id
        let target_parent = manager.get_group(target_id).and_then(|g| g.parent_id);
        prop_assert_eq!(target_parent, Some(root_id));

        // Create child groups under the target
        let mut child_ids = Vec::new();
        for i in 0..num_children {
            let name = format!("{child_name_seed}-{i}");
            let cid = manager
                .create_group_with_parent(name, target_id)
                .expect("Child group creation should succeed");
            child_ids.push(cid);
        }

        // Create connections directly in the target group
        let mut conn_ids = Vec::new();
        for _ in 0..num_connections {
            let cid = create_grouped_connection(&mut manager, target_id);
            conn_ids.push(cid);
        }

        // Act: delete the target group (non-cascade)
        manager
            .delete_group(target_id)
            .expect("delete_group should succeed");

        // Assert: target is gone
        prop_assert!(manager.get_group(target_id).is_none(), "Target group should be removed");

        // Assert: all children reparented to root_id (the target's former parent)
        for &cid in &child_ids {
            let child = manager.get_group(cid);
            prop_assert!(child.is_some(), "Child group {cid} should still exist");
            let child = child.expect("already checked");
            prop_assert_eq!(
                child.parent_id,
                Some(root_id),
                "Child should be reparented to root_id"
            );
        }

        // Assert: all direct connections are ungrouped
        for &conn_id in &conn_ids {
            let conn = manager.get_connection(conn_id);
            prop_assert!(conn.is_some(), "Connection {conn_id} should still exist");
            let conn = conn.expect("already checked");
            prop_assert_eq!(
                conn.group_id, None,
                "Connection should be ungrouped after delete_group"
            );
        }

        // Assert: no dangling parent_id references to the deleted target
        for group in manager.list_groups() {
            if let Some(pid) = group.parent_id {
                prop_assert!(
                    pid != target_id,
                    "Group {} has dangling parent_id referencing deleted target {target_id}",
                    group.id
                );
            }
        }
    }

    /// **Property 3: Leaf group deletion is identical for delete_group and delete_group_cascade**
    /// **Validates: Requirements 3.4, 3.5, 3.6**
    ///
    /// For any leaf group (no subgroups), deleting it with `delete_group()` or
    /// `delete_group_cascade()` produces the same result: the group is removed,
    /// its direct connections are ungrouped, and no other groups are affected.
    #[test]
    fn leaf_delete_equivalent_for_both_methods(
        num_connections in 0usize..=4,
        use_cascade in proptest::bool::ANY,
    ) {
        let (mut manager, _temp, _runtime) = create_test_manager();

        // Create a root group and a leaf group under it
        let root_id = manager
            .create_group("root".to_string())
            .expect("Root group creation should succeed");
        let leaf_id = manager
            .create_group_with_parent("leaf".to_string(), root_id)
            .expect("Leaf group creation should succeed");

        // Create connections in the leaf group
        let mut conn_ids = Vec::new();
        for _ in 0..num_connections {
            let cid = create_grouped_connection(&mut manager, leaf_id);
            conn_ids.push(cid);
        }

        // Snapshot: count groups before delete (root + leaf = 2)
        let groups_before = manager.group_count();

        // Act: delete the leaf using whichever method the strategy chose
        if use_cascade {
            manager
                .delete_group_cascade(leaf_id)
                .expect("delete_group_cascade on leaf should succeed");
        } else {
            manager
                .delete_group(leaf_id)
                .expect("delete_group on leaf should succeed");
        }

        // Assert: leaf is gone
        prop_assert!(manager.get_group(leaf_id).is_none(), "Leaf group should be removed");

        // Assert: exactly one group was removed
        prop_assert_eq!(
            manager.group_count(),
            groups_before - 1,
            "Exactly one group should be removed"
        );

        // Assert: root group is unaffected
        prop_assert!(manager.get_group(root_id).is_some(), "Root group should still exist");

        // Assert: connections handled appropriately
        // - delete_group: connections ungrouped (still active with group_id = None)
        // - delete_group_cascade: connections moved to trash
        if use_cascade {
            for &conn_id in &conn_ids {
                if let Some(conn) = manager.get_connection(conn_id) {
                    prop_assert_eq!(conn.group_id, None);
                }
                // Otherwise connection is in trash — correct cascade behavior
            }
        } else {
            for &conn_id in &conn_ids {
                let conn = manager.get_connection(conn_id);
                prop_assert!(conn.is_some(), "Connection {conn_id} should still exist");
                prop_assert_eq!(
                    conn.expect("already checked").group_id,
                    None,
                    "Connection should be ungrouped"
                );
            }
        }

        // Assert: no dangling parent_id references
        for group in manager.list_groups() {
            if let Some(pid) = group.parent_id {
                prop_assert!(
                    pid != leaf_id,
                    "Group {} has dangling parent_id referencing deleted leaf {leaf_id}",
                    group.id
                );
            }
        }
    }
}
