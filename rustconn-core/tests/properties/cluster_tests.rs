//! Property-based tests for cluster management
//!
//! Cluster definitions: serialization, persistence and list consistency.

use proptest::prelude::*;
use rustconn_core::cluster::{Cluster, ClusterManager};
use uuid::Uuid;

// ============================================================================
// Strategies for generating test data
// ============================================================================

/// Strategy for generating valid cluster names
fn arb_cluster_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9 _-]{0,30}".prop_map(|s| s.trim().to_string())
}

/// Strategy for generating a list of connection IDs
fn arb_connection_ids(min: usize, max: usize) -> impl Strategy<Value = Vec<Uuid>> {
    prop::collection::vec(any::<u128>().prop_map(Uuid::from_u128), min..=max)
}

/// Strategy for generating a cluster with connections
fn arb_cluster_with_connections(
    min_conns: usize,
    max_conns: usize,
) -> impl Strategy<Value = Cluster> {
    (
        arb_cluster_name(),
        arb_connection_ids(min_conns, max_conns),
        any::<bool>(),
    )
        .prop_map(|(name, conn_ids, broadcast)| {
            let mut cluster = Cluster::new(name);
            for id in conn_ids {
                cluster.add_connection(id);
            }
            cluster.broadcast_enabled = broadcast;
            cluster
        })
}

// ============================================================================
// Cluster Serialization Tests
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-enhancements, Cluster Serialization**
    ///
    /// For any valid cluster, serializing to JSON and deserializing should
    /// produce an equivalent cluster.
    #[test]
    fn prop_cluster_json_round_trip(
        cluster in arb_cluster_with_connections(0, 10),
    ) {
        // Serialize to JSON
        let json = serde_json::to_string(&cluster)
            .expect("Cluster should serialize to JSON");

        // Deserialize back
        let deserialized: Cluster = serde_json::from_str(&json)
            .expect("JSON should deserialize to Cluster");

        // Verify all fields match
        prop_assert_eq!(cluster.id, deserialized.id);
        prop_assert_eq!(cluster.name, deserialized.name);
        prop_assert_eq!(cluster.connection_ids, deserialized.connection_ids);
        prop_assert_eq!(cluster.broadcast_enabled, deserialized.broadcast_enabled);
    }

    /// **Feature: rustconn-enhancements, Cluster Serialization**
    ///
    /// For any valid cluster, serializing to TOML and deserializing should
    /// produce an equivalent cluster.
    #[test]
    fn prop_cluster_toml_round_trip(
        cluster in arb_cluster_with_connections(0, 10),
    ) {
        // Serialize to TOML
        let toml_str = toml::to_string(&cluster)
            .expect("Cluster should serialize to TOML");

        // Deserialize back
        let deserialized: Cluster = toml::from_str(&toml_str)
            .expect("TOML should deserialize to Cluster");

        // Verify all fields match
        prop_assert_eq!(cluster.id, deserialized.id);
        prop_assert_eq!(cluster.name, deserialized.name);
        prop_assert_eq!(cluster.connection_ids, deserialized.connection_ids);
        prop_assert_eq!(cluster.broadcast_enabled, deserialized.broadcast_enabled);
    }
}

/// Test cluster serialization preserves all fields
#[test]
fn test_cluster_serialization_preserves_fields() {
    let mut cluster = Cluster::new("Test Cluster".to_string());
    let conn1 = Uuid::new_v4();
    let conn2 = Uuid::new_v4();
    cluster.add_connection(conn1);
    cluster.add_connection(conn2);
    cluster.broadcast_enabled = true;

    // JSON round-trip
    let json = serde_json::to_string(&cluster).unwrap();
    let from_json: Cluster = serde_json::from_str(&json).unwrap();

    assert_eq!(cluster.id, from_json.id);
    assert_eq!(cluster.name, from_json.name);
    assert_eq!(cluster.connection_ids, from_json.connection_ids);
    assert_eq!(cluster.broadcast_enabled, from_json.broadcast_enabled);

    // TOML round-trip
    let toml_str = toml::to_string(&cluster).unwrap();
    let from_toml: Cluster = toml::from_str(&toml_str).unwrap();

    assert_eq!(cluster.id, from_toml.id);
    assert_eq!(cluster.name, from_toml.name);
    assert_eq!(cluster.connection_ids, from_toml.connection_ids);
    assert_eq!(cluster.broadcast_enabled, from_toml.broadcast_enabled);
}

// ============================================================================
// Property 8: Cluster Serialization Round-Trip (Persistence)
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-bugfixes, Property 8: Cluster Serialization Round-Trip**
    ///
    /// For any valid Cluster, saving to ConfigManager and loading back should
    /// produce an equivalent cluster. This validates that clusters persist
    /// correctly across application restarts.
    #[test]
    fn prop_cluster_config_manager_round_trip(
        cluster in arb_cluster_with_connections(0, 10),
    ) {
        use rustconn_core::config::ConfigManager;
        use tempfile::TempDir;

        // Create a temporary directory for the config
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config_manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());

        // Save the cluster
        config_manager
            .save_clusters(std::slice::from_ref(&cluster))
            .expect("Failed to save cluster");

        // Load the cluster back
        let loaded_clusters = config_manager
            .load_clusters()
            .expect("Failed to load clusters");

        // Verify we got exactly one cluster back
        prop_assert_eq!(
            loaded_clusters.len(),
            1,
            "Should load exactly one cluster"
        );

        let loaded = &loaded_clusters[0];

        // Verify all fields match
        prop_assert_eq!(cluster.id, loaded.id, "Cluster ID should match");
        prop_assert_eq!(&cluster.name, &loaded.name, "Cluster name should match");
        prop_assert_eq!(
            &cluster.connection_ids,
            &loaded.connection_ids,
            "Connection IDs should match"
        );
        prop_assert_eq!(
            cluster.broadcast_enabled,
            loaded.broadcast_enabled,
            "Broadcast enabled should match"
        );
    }

    /// **Feature: rustconn-bugfixes, Property 8: Cluster Serialization Round-Trip**
    ///
    /// For any list of valid Clusters, saving to ConfigManager and loading back
    /// should produce equivalent clusters. This validates that multiple clusters
    /// persist correctly.
    #[test]
    fn prop_multiple_clusters_config_manager_round_trip(
        clusters in prop::collection::vec(arb_cluster_with_connections(0, 5), 0..10),
    ) {
        use rustconn_core::config::ConfigManager;
        use tempfile::TempDir;

        // Create a temporary directory for the config
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let config_manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());

        // Save all clusters
        config_manager
            .save_clusters(&clusters)
            .expect("Failed to save clusters");

        // Load the clusters back
        let loaded_clusters = config_manager
            .load_clusters()
            .expect("Failed to load clusters");

        // Verify we got the same number of clusters back
        prop_assert_eq!(
            loaded_clusters.len(),
            clusters.len(),
            "Should load the same number of clusters"
        );

        // Verify each cluster matches (order should be preserved)
        for (original, loaded) in clusters.iter().zip(loaded_clusters.iter()) {
            prop_assert_eq!(original.id, loaded.id, "Cluster ID should match");
            prop_assert_eq!(&original.name, &loaded.name, "Cluster name should match");
            prop_assert_eq!(
                &original.connection_ids,
                &loaded.connection_ids,
                "Connection IDs should match"
            );
            prop_assert_eq!(
                original.broadcast_enabled,
                loaded.broadcast_enabled,
                "Broadcast enabled should match"
            );
        }
    }
}

/// Test that ConfigManager properly persists clusters to disk
#[test]
fn test_cluster_persistence_through_config_manager() {
    use rustconn_core::config::ConfigManager;
    use tempfile::TempDir;

    let temp_dir = TempDir::new().unwrap();
    let config_manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());

    // Create a cluster with all fields populated
    let mut cluster = Cluster::new("Production Servers".to_string());
    let conn1 = Uuid::new_v4();
    let conn2 = Uuid::new_v4();
    let conn3 = Uuid::new_v4();
    cluster.add_connection(conn1);
    cluster.add_connection(conn2);
    cluster.add_connection(conn3);
    cluster.broadcast_enabled = true;

    // Save the cluster
    config_manager
        .save_clusters(std::slice::from_ref(&cluster))
        .expect("Failed to save cluster");

    // Verify the file was created
    let clusters_file = temp_dir.path().join("clusters.toml");
    assert!(clusters_file.exists(), "clusters.toml should be created");

    // Create a new ConfigManager instance (simulating app restart)
    let config_manager2 = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());

    // Load the cluster
    let loaded = config_manager2
        .load_clusters()
        .expect("Failed to load clusters");

    assert_eq!(loaded.len(), 1);
    let loaded_cluster = &loaded[0];

    assert_eq!(cluster.id, loaded_cluster.id);
    assert_eq!(cluster.name, loaded_cluster.name);
    assert_eq!(cluster.connection_ids, loaded_cluster.connection_ids);
    assert!(loaded_cluster.broadcast_enabled);
}

// ============================================================================
// Property 3: Cluster List Refresh After Modification
//
// For any cluster add/edit/delete operation, the cluster list should
// immediately reflect the change without requiring dialog close/reopen.
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-fixes-v2, Property 3: Cluster List Refresh After Modification**
    ///
    /// For any cluster creation operation, the cluster manager should immediately
    /// contain the new cluster and return it in get_all_clusters().
    #[test]
    fn prop_cluster_list_reflects_add_immediately(
        cluster in arb_cluster_with_connections(0, 10),
    ) {
        let mut manager = ClusterManager::new();
        let cluster_id = cluster.id;
        let cluster_name = cluster.name.clone();
        let initial_count = manager.cluster_count();

        // Add the cluster
        manager.add_cluster(cluster);

        // Verify the list immediately reflects the addition
        let all_clusters = manager.get_all_clusters();
        prop_assert_eq!(
            all_clusters.len(),
            initial_count + 1,
            "Cluster count should increase by 1 after add"
        );

        // Verify the cluster is retrievable by ID
        let retrieved = manager.get_cluster(cluster_id);
        prop_assert!(
            retrieved.is_some(),
            "Added cluster should be retrievable by ID"
        );
        prop_assert_eq!(
            &retrieved.unwrap().name,
            &cluster_name,
            "Retrieved cluster should have correct name"
        );

        // Verify the cluster appears in the list
        let found = all_clusters.iter().any(|c| c.id == cluster_id);
        prop_assert!(
            found,
            "Added cluster should appear in get_all_clusters()"
        );
    }

    /// **Feature: rustconn-fixes-v2, Property 3: Cluster List Refresh After Modification**
    ///
    /// For any cluster deletion operation, the cluster manager should immediately
    /// no longer contain the deleted cluster.
    #[test]
    fn prop_cluster_list_reflects_delete_immediately(
        cluster in arb_cluster_with_connections(0, 10),
    ) {
        let mut manager = ClusterManager::new();
        let cluster_id = cluster.id;

        // Add the cluster first
        manager.add_cluster(cluster);
        prop_assert_eq!(manager.cluster_count(), 1, "Should have 1 cluster after add");

        // Delete the cluster
        let removed = manager.remove_cluster(cluster_id);
        prop_assert!(removed.is_some(), "Remove should return the deleted cluster");

        // Verify the list immediately reflects the deletion
        let all_clusters = manager.get_all_clusters();
        prop_assert!(
            all_clusters.is_empty(),
            "Cluster list should be empty after delete"
        );

        // Verify the cluster is no longer retrievable by ID
        let retrieved = manager.get_cluster(cluster_id);
        prop_assert!(
            retrieved.is_none(),
            "Deleted cluster should not be retrievable by ID"
        );
    }

    /// **Feature: rustconn-fixes-v2, Property 3: Cluster List Refresh After Modification**
    ///
    /// For any cluster edit operation, the cluster manager should immediately
    /// reflect the updated cluster data.
    #[test]
    fn prop_cluster_list_reflects_edit_immediately(
        cluster in arb_cluster_with_connections(0, 10),
        new_name in arb_cluster_name(),
        new_broadcast in any::<bool>(),
    ) {
        let mut manager = ClusterManager::new();
        let cluster_id = cluster.id;

        // Add the cluster first
        manager.add_cluster(cluster);

        // Create an updated version
        let mut updated = manager.get_cluster(cluster_id).unwrap().clone();
        updated.name = new_name.clone();
        updated.broadcast_enabled = new_broadcast;

        // Update the cluster
        let result = manager.update_cluster(cluster_id, updated);
        prop_assert!(result.is_ok(), "Update should succeed");

        // Verify the list immediately reflects the edit
        let retrieved = manager.get_cluster(cluster_id);
        prop_assert!(retrieved.is_some(), "Updated cluster should be retrievable");

        let retrieved = retrieved.unwrap();
        prop_assert_eq!(
            &retrieved.name,
            &new_name,
            "Cluster name should be updated"
        );
        prop_assert_eq!(
            retrieved.broadcast_enabled,
            new_broadcast,
            "Cluster broadcast_enabled should be updated"
        );

        // Verify the cluster in get_all_clusters() also reflects the change
        let all_clusters = manager.get_all_clusters();
        let found = all_clusters.iter().find(|c| c.id == cluster_id);
        prop_assert!(found.is_some(), "Updated cluster should be in list");
        prop_assert_eq!(
            &found.unwrap().name,
            &new_name,
            "Cluster in list should have updated name"
        );
    }

    /// **Feature: rustconn-fixes-v2, Property 3: Cluster List Refresh After Modification**
    ///
    /// For any sequence of add/edit/delete operations, the cluster list should
    /// always accurately reflect the current state.
    #[test]
    fn prop_cluster_list_consistent_after_multiple_operations(
        clusters in prop::collection::vec(arb_cluster_with_connections(0, 5), 1..10),
        delete_indices in prop::collection::vec(any::<usize>(), 0..5),
    ) {
        let mut manager = ClusterManager::new();
        let mut expected_ids: Vec<Uuid> = Vec::new();

        // Add all clusters
        for cluster in &clusters {
            expected_ids.push(cluster.id);
            manager.add_cluster(cluster.clone());
        }

        // Verify all clusters are present
        prop_assert_eq!(
            manager.cluster_count(),
            clusters.len(),
            "All clusters should be added"
        );

        // Delete some clusters
        for &idx in &delete_indices {
            if !expected_ids.is_empty() {
                let idx = idx % expected_ids.len();
                let id_to_remove = expected_ids.remove(idx);
                manager.remove_cluster(id_to_remove);
            }
        }

        // Verify the list matches expected state
        let all_clusters = manager.get_all_clusters();
        prop_assert_eq!(
            all_clusters.len(),
            expected_ids.len(),
            "Cluster count should match expected after deletions"
        );

        // Verify each expected cluster is present
        for expected_id in &expected_ids {
            let found = all_clusters.iter().any(|c| c.id == *expected_id);
            prop_assert!(
                found,
                "Expected cluster {} should be in list",
                expected_id
            );
        }

        // Verify no unexpected clusters are present
        for cluster in all_clusters {
            prop_assert!(
                expected_ids.contains(&cluster.id),
                "Cluster {} should be in expected list",
                cluster.id
            );
        }
    }
}

/// Test that cluster list is consistent after rapid add/delete cycles
#[test]
fn test_cluster_list_consistency_rapid_operations() {
    let mut manager = ClusterManager::new();

    // Rapid add/delete cycle
    for i in 0..100 {
        let cluster = Cluster::new(format!("Cluster {i}"));
        let id = cluster.id;
        manager.add_cluster(cluster);

        // Verify it's there
        assert!(manager.get_cluster(id).is_some());
        assert_eq!(manager.cluster_count(), 1);

        // Delete it
        manager.remove_cluster(id);

        // Verify it's gone
        assert!(manager.get_cluster(id).is_none());
        assert_eq!(manager.cluster_count(), 0);
    }
}

/// Test that editing a cluster preserves its ID and connections
#[test]
fn test_cluster_edit_preserves_identity() {
    let mut manager = ClusterManager::new();

    let mut cluster = Cluster::new("Original Name".to_string());
    let conn1 = Uuid::new_v4();
    let conn2 = Uuid::new_v4();
    cluster.add_connection(conn1);
    cluster.add_connection(conn2);
    let cluster_id = cluster.id;

    manager.add_cluster(cluster);

    // Edit the cluster
    let mut updated = manager.get_cluster(cluster_id).unwrap().clone();
    updated.name = "New Name".to_string();
    updated.broadcast_enabled = true;

    manager.update_cluster(cluster_id, updated).unwrap();

    // Verify the edit
    let retrieved = manager.get_cluster(cluster_id).unwrap();
    assert_eq!(retrieved.id, cluster_id, "ID should be preserved");
    assert_eq!(retrieved.name, "New Name");
    assert!(retrieved.broadcast_enabled);
    assert_eq!(retrieved.connection_ids.len(), 2);
    assert!(retrieved.contains_connection(conn1));
    assert!(retrieved.contains_connection(conn2));
}
