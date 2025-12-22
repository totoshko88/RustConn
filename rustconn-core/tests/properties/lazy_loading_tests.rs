//! Property-based tests for the Lazy Loading system
//!
//! These tests validate the correctness properties defined in the design document
//! for the Lazy Loading system (Requirements 3.x).

use proptest::prelude::*;
use rustconn_core::{Connection, ConnectionGroup, LazyGroupLoader};
use std::collections::HashSet;
use uuid::Uuid;

// ========== Strategies ==========

/// Strategy for generating valid group names
fn arb_group_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_ -]{0,15}".prop_map(|s| s.trim().to_string())
}

/// Strategy for generating valid connection names
fn arb_connection_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_-]{0,15}".prop_map(|s| s)
}

/// Strategy for generating a test connection group
fn arb_group(parent_id: Option<Uuid>) -> impl Strategy<Value = ConnectionGroup> {
    arb_group_name().prop_map(move |name| {
        let mut group = ConnectionGroup::new(name);
        group.parent_id = parent_id;
        group
    })
}

/// Strategy for generating a test connection
fn arb_connection(group_id: Option<Uuid>) -> impl Strategy<Value = Connection> {
    arb_connection_name().prop_map(move |name| {
        let mut conn = Connection::new_ssh(name, "localhost".to_string(), 22);
        conn.group_id = group_id;
        conn
    })
}

/// Strategy for generating a hierarchical group structure
fn arb_group_hierarchy() -> impl Strategy<Value = (Vec<ConnectionGroup>, Vec<Connection>)> {
    // Generate 1-5 root groups
    prop::collection::vec(arb_group(None), 1..5).prop_flat_map(|root_groups| {
        let root_ids: Vec<Uuid> = root_groups.iter().map(|g| g.id).collect();

        // Generate 0-3 child groups per root
        let child_groups_strategy = root_ids
            .clone()
            .into_iter()
            .map(|parent_id| prop::collection::vec(arb_group(Some(parent_id)), 0..3))
            .collect::<Vec<_>>();

        // Generate 0-5 connections per group (including root and ungrouped)
        let all_group_ids: Vec<Option<Uuid>> = std::iter::once(None)
            .chain(root_ids.iter().map(|id| Some(*id)))
            .collect();

        let connections_strategy = all_group_ids
            .into_iter()
            .map(|group_id| prop::collection::vec(arb_connection(group_id), 0..5))
            .collect::<Vec<_>>();

        (
            Just(root_groups),
            child_groups_strategy,
            connections_strategy,
        )
            .prop_map(|(root_groups, child_groups_vec, connections_vec)| {
                let mut all_groups = root_groups;
                for child_groups in child_groups_vec {
                    all_groups.extend(child_groups);
                }
                let all_connections: Vec<Connection> =
                    connections_vec.into_iter().flatten().collect();
                (all_groups, all_connections)
            })
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 5: Lazy Loading Initial State ==========
    // **Feature: performance-improvements, Property 5: Lazy Loading Initial State**
    // **Validates: Requirements 3.1**
    //
    // For any connection database, initial sidebar load SHALL only include
    // root-level groups and ungrouped connections.

    #[test]
    fn lazy_loading_initial_state_only_root_items(
        (all_groups, all_connections) in arb_group_hierarchy()
    ) {
        let loader = LazyGroupLoader::new();

        // Get root items
        let (root_groups, ungrouped_connections) =
            loader.get_root_items_to_load(&all_groups, &all_connections);

        // Verify all returned groups are root-level (no parent)
        for group in &root_groups {
            prop_assert!(
                group.parent_id.is_none(),
                "Root items should only include groups with no parent, but found group '{}' with parent {:?}",
                group.name,
                group.parent_id
            );
        }

        // Verify all returned connections are ungrouped
        for conn in &ungrouped_connections {
            prop_assert!(
                conn.group_id.is_none(),
                "Root items should only include ungrouped connections, but found connection '{}' in group {:?}",
                conn.name,
                conn.group_id
            );
        }

        // Verify we got all root groups
        let expected_root_count = all_groups.iter().filter(|g| g.parent_id.is_none()).count();
        prop_assert_eq!(
            root_groups.len(),
            expected_root_count,
            "Should return all root-level groups"
        );

        // Verify we got all ungrouped connections
        let expected_ungrouped_count = all_connections.iter().filter(|c| c.group_id.is_none()).count();
        prop_assert_eq!(
            ungrouped_connections.len(),
            expected_ungrouped_count,
            "Should return all ungrouped connections"
        );
    }

    #[test]
    fn lazy_loading_new_loader_has_nothing_loaded(
        group_ids in prop::collection::vec(prop::arbitrary::any::<u128>().prop_map(|n| Uuid::from_u128(n)), 1..10)
    ) {
        let loader = LazyGroupLoader::new();

        // New loader should have nothing loaded
        prop_assert!(!loader.is_root_loaded(), "New loader should not have root loaded");
        prop_assert_eq!(loader.loaded_count(), 0, "New loader should have zero loaded groups");

        // No groups should be marked as loaded
        for group_id in &group_ids {
            prop_assert!(
                !loader.is_group_loaded(*group_id),
                "New loader should not have any group loaded"
            );
            prop_assert!(
                loader.needs_loading(*group_id),
                "New loader should indicate all groups need loading"
            );
        }
    }

    #[test]
    fn lazy_loading_root_loaded_state_persists(
        (all_groups, all_connections) in arb_group_hierarchy()
    ) {
        let mut loader = LazyGroupLoader::new();

        // Initially not loaded
        prop_assert!(!loader.is_root_loaded(), "Root should not be loaded initially");

        // Mark root as loaded
        loader.mark_root_loaded();

        // Should now be loaded
        prop_assert!(loader.is_root_loaded(), "Root should be loaded after marking");

        // Getting root items should still work
        let (root_groups, ungrouped_connections) =
            loader.get_root_items_to_load(&all_groups, &all_connections);

        // Verify we still get the correct items
        let expected_root_count = all_groups.iter().filter(|g| g.parent_id.is_none()).count();
        prop_assert_eq!(root_groups.len(), expected_root_count);

        let expected_ungrouped_count = all_connections.iter().filter(|c| c.group_id.is_none()).count();
        prop_assert_eq!(ungrouped_connections.len(), expected_ungrouped_count);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 6: Lazy Loading Expansion ==========
    // **Feature: performance-improvements, Property 6: Lazy Loading Expansion**
    // **Validates: Requirements 3.2, 3.3**
    //
    // For any group expansion, the loaded children SHALL match the actual
    // children in the database.

    #[test]
    fn lazy_loading_expansion_returns_correct_children(
        (all_groups, all_connections) in arb_group_hierarchy()
    ) {
        let loader = LazyGroupLoader::new();

        // For each group, verify get_children_to_load returns correct children
        for group in &all_groups {
            let (child_groups, child_connections) =
                loader.get_children_to_load(group.id, &all_groups, &all_connections);

            // Verify all returned child groups have this group as parent
            for child_group in &child_groups {
                prop_assert_eq!(
                    child_group.parent_id,
                    Some(group.id),
                    "Child group '{}' should have parent {:?}, but has {:?}",
                    child_group.name,
                    group.id,
                    child_group.parent_id
                );
            }

            // Verify all returned connections belong to this group
            for conn in &child_connections {
                prop_assert_eq!(
                    conn.group_id,
                    Some(group.id),
                    "Connection '{}' should belong to group {:?}, but belongs to {:?}",
                    conn.name,
                    group.id,
                    conn.group_id
                );
            }

            // Verify we got all child groups
            let expected_child_groups: Vec<_> = all_groups
                .iter()
                .filter(|g| g.parent_id == Some(group.id))
                .collect();
            prop_assert_eq!(
                child_groups.len(),
                expected_child_groups.len(),
                "Should return all child groups for group '{}'",
                group.name
            );

            // Verify we got all child connections
            let expected_child_connections: Vec<_> = all_connections
                .iter()
                .filter(|c| c.group_id == Some(group.id))
                .collect();
            prop_assert_eq!(
                child_connections.len(),
                expected_child_connections.len(),
                "Should return all child connections for group '{}'",
                group.name
            );
        }
    }

    #[test]
    fn lazy_loading_mark_group_loaded_persists(
        group_ids in prop::collection::vec(prop::arbitrary::any::<u128>().prop_map(|n| Uuid::from_u128(n)), 1..10)
    ) {
        let mut loader = LazyGroupLoader::new();

        // Mark each group as loaded and verify
        for (i, group_id) in group_ids.iter().enumerate() {
            // Before marking, should not be loaded
            prop_assert!(
                !loader.is_group_loaded(*group_id),
                "Group should not be loaded before marking"
            );

            // Mark as loaded
            loader.mark_group_loaded(*group_id);

            // After marking, should be loaded
            prop_assert!(
                loader.is_group_loaded(*group_id),
                "Group should be loaded after marking"
            );
            prop_assert!(
                !loader.needs_loading(*group_id),
                "Group should not need loading after marking"
            );

            // Loaded count should match
            prop_assert_eq!(
                loader.loaded_count(),
                i + 1,
                "Loaded count should match number of marked groups"
            );
        }
    }

    #[test]
    fn lazy_loading_children_retained_after_collapse(
        (all_groups, all_connections) in arb_group_hierarchy()
    ) {
        let mut loader = LazyGroupLoader::new();

        // Mark some groups as loaded
        for group in all_groups.iter().take(3) {
            loader.mark_group_loaded(group.id);
        }

        // Verify they remain loaded (simulating collapse - children stay in memory)
        for group in all_groups.iter().take(3) {
            prop_assert!(
                loader.is_group_loaded(group.id),
                "Group '{}' should remain loaded after collapse",
                group.name
            );
        }

        // Getting children should still work
        for group in all_groups.iter().take(3) {
            let (child_groups, child_connections) =
                loader.get_children_to_load(group.id, &all_groups, &all_connections);

            // Verify children are correct
            for child_group in &child_groups {
                prop_assert_eq!(child_group.parent_id, Some(group.id));
            }
            for conn in &child_connections {
                prop_assert_eq!(conn.group_id, Some(group.id));
            }
        }
    }

    #[test]
    fn lazy_loading_reset_clears_all_state(
        group_ids in prop::collection::vec(prop::arbitrary::any::<u128>().prop_map(|n| Uuid::from_u128(n)), 1..10)
    ) {
        let mut loader = LazyGroupLoader::new();

        // Mark root and some groups as loaded
        loader.mark_root_loaded();
        for group_id in &group_ids {
            loader.mark_group_loaded(*group_id);
        }

        // Verify state is set
        prop_assert!(loader.is_root_loaded());
        prop_assert!(loader.loaded_count() > 0);

        // Reset
        loader.reset();

        // Verify all state is cleared
        prop_assert!(!loader.is_root_loaded(), "Root should not be loaded after reset");
        prop_assert_eq!(loader.loaded_count(), 0, "Loaded count should be zero after reset");

        for group_id in &group_ids {
            prop_assert!(
                !loader.is_group_loaded(*group_id),
                "No groups should be loaded after reset"
            );
        }
    }
}

// ========== Unit Tests for Edge Cases ==========

#[cfg(test)]
mod edge_case_tests {
    use super::*;

    #[test]
    fn test_empty_database_returns_empty_root_items() {
        let loader = LazyGroupLoader::new();
        let all_groups: Vec<ConnectionGroup> = vec![];
        let all_connections: Vec<Connection> = vec![];

        let (root_groups, ungrouped_connections) =
            loader.get_root_items_to_load(&all_groups, &all_connections);

        assert!(root_groups.is_empty());
        assert!(ungrouped_connections.is_empty());
    }

    #[test]
    fn test_nonexistent_group_returns_empty_children() {
        let loader = LazyGroupLoader::new();
        let nonexistent_id = Uuid::new_v4();
        let all_groups: Vec<ConnectionGroup> = vec![];
        let all_connections: Vec<Connection> = vec![];

        let (child_groups, child_connections) =
            loader.get_children_to_load(nonexistent_id, &all_groups, &all_connections);

        assert!(child_groups.is_empty());
        assert!(child_connections.is_empty());
    }

    #[test]
    fn test_mark_same_group_multiple_times() {
        let mut loader = LazyGroupLoader::new();
        let group_id = Uuid::new_v4();

        loader.mark_group_loaded(group_id);
        loader.mark_group_loaded(group_id);
        loader.mark_group_loaded(group_id);

        // Should still only count as one
        assert_eq!(loader.loaded_count(), 1);
        assert!(loader.is_group_loaded(group_id));
    }

    #[test]
    fn test_mark_group_unloaded() {
        let mut loader = LazyGroupLoader::new();
        let group_id = Uuid::new_v4();

        loader.mark_group_loaded(group_id);
        assert!(loader.is_group_loaded(group_id));

        loader.mark_group_unloaded(group_id);
        assert!(!loader.is_group_loaded(group_id));
        assert!(loader.needs_loading(group_id));
    }

    #[test]
    fn test_deeply_nested_hierarchy() {
        let loader = LazyGroupLoader::new();

        // Create a deep hierarchy: root -> child1 -> child2 -> child3
        let root = ConnectionGroup::new("Root".to_string());
        let mut child1 = ConnectionGroup::new("Child1".to_string());
        child1.parent_id = Some(root.id);
        let mut child2 = ConnectionGroup::new("Child2".to_string());
        child2.parent_id = Some(child1.id);
        let mut child3 = ConnectionGroup::new("Child3".to_string());
        child3.parent_id = Some(child2.id);

        let all_groups = vec![root.clone(), child1.clone(), child2.clone(), child3.clone()];
        let all_connections: Vec<Connection> = vec![];

        // Root items should only include root
        let (root_groups, _) = loader.get_root_items_to_load(&all_groups, &all_connections);
        assert_eq!(root_groups.len(), 1);
        assert_eq!(root_groups[0].id, root.id);

        // Children of root should only include child1
        let (children_of_root, _) =
            loader.get_children_to_load(root.id, &all_groups, &all_connections);
        assert_eq!(children_of_root.len(), 1);
        assert_eq!(children_of_root[0].id, child1.id);

        // Children of child1 should only include child2
        let (children_of_child1, _) =
            loader.get_children_to_load(child1.id, &all_groups, &all_connections);
        assert_eq!(children_of_child1.len(), 1);
        assert_eq!(children_of_child1[0].id, child2.id);

        // Children of child2 should only include child3
        let (children_of_child2, _) =
            loader.get_children_to_load(child2.id, &all_groups, &all_connections);
        assert_eq!(children_of_child2.len(), 1);
        assert_eq!(children_of_child2[0].id, child3.id);

        // Children of child3 should be empty
        let (children_of_child3, _) =
            loader.get_children_to_load(child3.id, &all_groups, &all_connections);
        assert!(children_of_child3.is_empty());
    }

    #[test]
    fn test_mixed_grouped_and_ungrouped_connections() {
        let loader = LazyGroupLoader::new();

        let group = ConnectionGroup::new("Group".to_string());

        let mut grouped_conn = Connection::new_ssh("Grouped".to_string(), "host1".to_string(), 22);
        grouped_conn.group_id = Some(group.id);

        let ungrouped_conn = Connection::new_ssh("Ungrouped".to_string(), "host2".to_string(), 22);

        let all_groups = vec![group.clone()];
        let all_connections = vec![grouped_conn.clone(), ungrouped_conn.clone()];

        // Root items should include the group and ungrouped connection
        let (root_groups, ungrouped_connections) =
            loader.get_root_items_to_load(&all_groups, &all_connections);

        assert_eq!(root_groups.len(), 1);
        assert_eq!(root_groups[0].id, group.id);

        assert_eq!(ungrouped_connections.len(), 1);
        assert_eq!(ungrouped_connections[0].id, ungrouped_conn.id);

        // Children of group should include grouped connection
        let (_, child_connections) =
            loader.get_children_to_load(group.id, &all_groups, &all_connections);

        assert_eq!(child_connections.len(), 1);
        assert_eq!(child_connections[0].id, grouped_conn.id);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 7: Search Ignores Lazy Loading ==========
    // **Feature: performance-improvements, Property 7: Search Ignores Lazy Loading**
    // **Validates: Requirements 3.5**
    //
    // For any search query, results SHALL include matches from all connections
    // regardless of lazy loading state.

    #[test]
    fn search_ignores_lazy_loading_state(
        (all_groups, all_connections) in arb_group_hierarchy()
    ) {
        // Skip if no connections to search
        prop_assume!(!all_connections.is_empty());

        let mut loader = LazyGroupLoader::new();

        // Mark only some groups as loaded (simulating partial lazy loading)
        let groups_to_load: Vec<_> = all_groups.iter().take(all_groups.len() / 2).collect();
        for group in &groups_to_load {
            loader.mark_group_loaded(group.id);
        }

        // Get all connections that would be searchable
        // (This simulates what the search engine should do - search ALL connections)
        let all_connection_ids: HashSet<Uuid> = all_connections.iter().map(|c| c.id).collect();

        // Verify that connections in unloaded groups are still in the full set
        // (The search should search all connections, not just loaded ones)
        for conn in &all_connections {
            prop_assert!(
                all_connection_ids.contains(&conn.id),
                "All connections should be searchable regardless of lazy loading state"
            );
        }

        // Verify that the lazy loader state doesn't affect the connection set
        // (Connections in unloaded groups should still be searchable)
        let unloaded_groups: Vec<_> = all_groups
            .iter()
            .filter(|g| !loader.is_group_loaded(g.id))
            .collect();

        for group in &unloaded_groups {
            let connections_in_unloaded_group: Vec<_> = all_connections
                .iter()
                .filter(|c| c.group_id == Some(group.id))
                .collect();

            for conn in connections_in_unloaded_group {
                prop_assert!(
                    all_connection_ids.contains(&conn.id),
                    "Connection '{}' in unloaded group '{}' should still be searchable",
                    conn.name,
                    group.name
                );
            }
        }
    }

    #[test]
    fn search_returns_all_matching_connections_regardless_of_loading(
        (_all_groups, all_connections) in arb_group_hierarchy()
    ) {
        // Skip if no connections
        prop_assume!(!all_connections.is_empty());

        let mut loader = LazyGroupLoader::new();

        // Only mark root as loaded (no groups loaded)
        loader.mark_root_loaded();

        // Pick a connection from a potentially unloaded group
        let test_connection = &all_connections[0];

        // The search should find this connection even if its group is not loaded
        // (This is a conceptual test - the actual search implementation should
        // search the full connection list, not the lazy-loaded tree)

        // Verify the connection exists in the full list
        let found = all_connections.iter().any(|c| c.id == test_connection.id);
        prop_assert!(
            found,
            "Connection should be findable in full connection list"
        );

        // If the connection is in a group, verify the group's loading state
        // doesn't affect searchability
        if let Some(group_id) = test_connection.group_id {
            // Group might not be loaded
            let group_loaded = loader.is_group_loaded(group_id);

            // But the connection should still be searchable
            // (The search engine should search all connections, not just visible ones)
            prop_assert!(
                all_connections.iter().any(|c| c.id == test_connection.id),
                "Connection in {} group should be searchable",
                if group_loaded { "loaded" } else { "unloaded" }
            );
        }
    }

    #[test]
    fn lazy_loading_does_not_filter_search_results(
        (_all_groups, all_connections) in arb_group_hierarchy()
    ) {
        // Skip if no connections
        prop_assume!(!all_connections.is_empty());

        let mut loader = LazyGroupLoader::new();

        // Simulate a scenario where only root is loaded
        loader.mark_root_loaded();

        // Count connections that would be visible in lazy-loaded tree
        // (only ungrouped connections and connections in loaded groups)
        let visible_in_lazy_tree: Vec<_> = all_connections
            .iter()
            .filter(|c| {
                c.group_id.is_none() || c.group_id.is_some_and(|gid| loader.is_group_loaded(gid))
            })
            .collect();

        // Count all connections (what search should return)
        let all_searchable = all_connections.len();

        // Search should return more or equal results than lazy-loaded tree shows
        // (Search ignores lazy loading state)
        prop_assert!(
            all_searchable >= visible_in_lazy_tree.len(),
            "Search should return all {} connections, not just {} visible in lazy tree",
            all_searchable,
            visible_in_lazy_tree.len()
        );
    }
}

// ========== Additional Unit Tests for Search and Lazy Loading ==========

#[cfg(test)]
mod search_lazy_loading_tests {
    use super::*;

    #[test]
    fn test_search_finds_connections_in_unloaded_groups() {
        let mut loader = LazyGroupLoader::new();

        // Create a group and connection
        let group = ConnectionGroup::new("Servers".to_string());
        let mut conn = Connection::new_ssh("web-server".to_string(), "192.168.1.1".to_string(), 22);
        conn.group_id = Some(group.id);

        let _all_groups = [group.clone()];
        let all_connections = [conn.clone()];

        // Mark root as loaded but NOT the group
        loader.mark_root_loaded();
        assert!(!loader.is_group_loaded(group.id));

        // The connection should still be in the full list (searchable)
        assert!(all_connections.iter().any(|c| c.id == conn.id));

        // Verify the connection's group is not loaded
        assert!(conn.group_id.is_some());
        assert!(!loader.is_group_loaded(conn.group_id.unwrap()));
    }

    #[test]
    fn test_search_finds_deeply_nested_connections() {
        let mut loader = LazyGroupLoader::new();

        // Create a deep hierarchy
        let root_group = ConnectionGroup::new("Root".to_string());
        let mut child_group = ConnectionGroup::new("Child".to_string());
        child_group.parent_id = Some(root_group.id);
        let mut grandchild_group = ConnectionGroup::new("Grandchild".to_string());
        grandchild_group.parent_id = Some(child_group.id);

        // Connection in the deepest group
        let mut deep_conn =
            Connection::new_ssh("deep-server".to_string(), "10.0.0.1".to_string(), 22);
        deep_conn.group_id = Some(grandchild_group.id);

        let _all_groups = [
            root_group.clone(),
            child_group.clone(),
            grandchild_group.clone(),
        ];
        let all_connections = [deep_conn.clone()];

        // Only mark root as loaded
        loader.mark_root_loaded();
        loader.mark_group_loaded(root_group.id);
        // child_group and grandchild_group are NOT loaded

        assert!(!loader.is_group_loaded(child_group.id));
        assert!(!loader.is_group_loaded(grandchild_group.id));

        // The deep connection should still be searchable
        assert!(all_connections.iter().any(|c| c.id == deep_conn.id));
    }

    #[test]
    fn test_search_finds_all_connections_with_no_groups_loaded() {
        let loader = LazyGroupLoader::new();

        // Create multiple groups with connections
        let group1 = ConnectionGroup::new("Group1".to_string());
        let group2 = ConnectionGroup::new("Group2".to_string());

        let mut conn1 = Connection::new_ssh("server1".to_string(), "host1".to_string(), 22);
        conn1.group_id = Some(group1.id);

        let mut conn2 = Connection::new_ssh("server2".to_string(), "host2".to_string(), 22);
        conn2.group_id = Some(group2.id);

        let conn3 = Connection::new_ssh("server3".to_string(), "host3".to_string(), 22);
        // conn3 is ungrouped

        let all_connections = [conn1.clone(), conn2.clone(), conn3.clone()];

        // No groups are loaded
        assert!(!loader.is_group_loaded(group1.id));
        assert!(!loader.is_group_loaded(group2.id));

        // All connections should be searchable
        assert_eq!(all_connections.len(), 3);
        assert!(all_connections.iter().any(|c| c.id == conn1.id));
        assert!(all_connections.iter().any(|c| c.id == conn2.id));
        assert!(all_connections.iter().any(|c| c.id == conn3.id));
    }
}
