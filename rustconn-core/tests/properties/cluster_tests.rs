//! Property-based tests for cluster management
//!
//! These tests validate the correctness properties for cluster session management
//! as defined in the design document for rustconn-enhancements.

use proptest::prelude::*;
use rustconn_core::cluster::{Cluster, ClusterManager, ClusterSession, ClusterSessionStatus};
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

/// Strategy for generating cluster session status
fn arb_session_status() -> impl Strategy<Value = ClusterSessionStatus> {
    prop_oneof![
        Just(ClusterSessionStatus::Pending),
        Just(ClusterSessionStatus::Connecting),
        Just(ClusterSessionStatus::Connected),
        Just(ClusterSessionStatus::Disconnected),
        Just(ClusterSessionStatus::Error),
    ]
}

// ============================================================================
// Property 13: Cluster Session Independence
// **Validates: Requirements 3.4**
//
// For any cluster with multiple connections, failure of one session should not
// affect other sessions.
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-enhancements, Property 13: Cluster Session Independence**
    /// **Validates: Requirements 3.4**
    ///
    /// For any cluster with multiple connections, when one session fails,
    /// the other sessions should maintain their state unchanged.
    #[test]
    fn prop_cluster_session_failure_does_not_affect_others(
        cluster in arb_cluster_with_connections(2, 10),
        fail_index in 0usize..10,
    ) {
        // Skip if cluster is empty (shouldn't happen with min_conns=2)
        prop_assume!(!cluster.is_empty());
        prop_assume!(cluster.connection_count() >= 2);

        let conn_ids: Vec<Uuid> = cluster.connection_ids.clone();
        let fail_index = fail_index % conn_ids.len();

        // Create a cluster session
        let mut session = ClusterSession::new(&cluster);

        // Set all sessions to Connected state
        for &conn_id in &conn_ids {
            session.update_session_status(conn_id, ClusterSessionStatus::Connected);
        }

        // Verify all are connected
        prop_assert_eq!(
            session.connected_count(),
            conn_ids.len(),
            "All sessions should be connected initially"
        );

        // Record the states of all sessions except the one we'll fail
        let states_before: Vec<(Uuid, ClusterSessionStatus)> = conn_ids
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != fail_index)
            .map(|(_, &id)| (id, session.get_session_state(id).unwrap().status))
            .collect();

        // Fail one session
        let failed_conn_id = conn_ids[fail_index];
        session.set_session_error(failed_conn_id, "Test failure".to_string());

        // Verify the failed session is in error state
        prop_assert_eq!(
            session.get_session_state(failed_conn_id).unwrap().status,
            ClusterSessionStatus::Error,
            "Failed session should be in Error state"
        );

        // Verify all other sessions are unchanged
        for (conn_id, expected_status) in states_before {
            let actual_status = session.get_session_state(conn_id).unwrap().status;
            prop_assert_eq!(
                actual_status,
                expected_status,
                "Session {} should be unchanged after another session failed",
                conn_id
            );
        }

        // Verify the cluster still has active sessions
        prop_assert!(
            session.any_connected(),
            "Cluster should still have connected sessions after one failure"
        );

        // Verify error count is exactly 1
        prop_assert_eq!(
            session.error_count(),
            1,
            "Only one session should be in error state"
        );
    }


    /// **Feature: rustconn-enhancements, Property 13: Cluster Session Independence**
    /// **Validates: Requirements 3.4**
    ///
    /// For any cluster, updating the status of one session should not affect
    /// the status of other sessions.
    #[test]
    fn prop_cluster_session_status_updates_are_independent(
        cluster in arb_cluster_with_connections(2, 10),
        statuses in prop::collection::vec(arb_session_status(), 2..=10),
    ) {
        prop_assume!(!cluster.is_empty());
        prop_assume!(cluster.connection_count() >= 2);

        let conn_ids: Vec<Uuid> = cluster.connection_ids.clone();
        let mut session = ClusterSession::new(&cluster);

        // Assign different statuses to each session
        let num_to_update = conn_ids.len().min(statuses.len());
        for i in 0..num_to_update {
            session.update_session_status(conn_ids[i], statuses[i]);
        }

        // Verify each session has its assigned status
        for i in 0..num_to_update {
            let actual_status = session.get_session_state(conn_ids[i]).unwrap().status;
            prop_assert_eq!(
                actual_status,
                statuses[i],
                "Session {} should have status {:?}",
                conn_ids[i],
                statuses[i]
            );
        }

        // Now update one session and verify others are unchanged
        if num_to_update >= 2 {
            let target_id = conn_ids[0];
            let new_status = ClusterSessionStatus::Disconnected;
            session.update_session_status(target_id, new_status);

            // Verify the target changed
            prop_assert_eq!(
                session.get_session_state(target_id).unwrap().status,
                new_status,
                "Target session should have new status"
            );

            // Verify others are unchanged
            for i in 1..num_to_update {
                let actual_status = session.get_session_state(conn_ids[i]).unwrap().status;
                prop_assert_eq!(
                    actual_status,
                    statuses[i],
                    "Session {} should be unchanged after updating session {}",
                    conn_ids[i],
                    target_id
                );
            }
        }
    }

    /// **Feature: rustconn-enhancements, Property 13: Cluster Session Independence**
    /// **Validates: Requirements 3.4**
    ///
    /// For any cluster managed by ClusterManager, failing a session in one cluster
    /// should not affect sessions in other clusters.
    #[test]
    fn prop_cluster_manager_session_isolation_across_clusters(
        cluster1 in arb_cluster_with_connections(1, 5),
        cluster2 in arb_cluster_with_connections(1, 5),
    ) {
        prop_assume!(!cluster1.is_empty());
        prop_assume!(!cluster2.is_empty());

        let cluster1_id = cluster1.id;
        let cluster2_id = cluster2.id;
        let cluster1_conns: Vec<Uuid> = cluster1.connection_ids.clone();
        let cluster2_conns: Vec<Uuid> = cluster2.connection_ids.clone();

        let mut manager = ClusterManager::new();
        manager.add_cluster(cluster1);
        manager.add_cluster(cluster2);

        // Start both sessions
        manager.start_session(cluster1_id).unwrap();
        manager.start_session(cluster2_id).unwrap();

        // Connect all sessions in both clusters
        for &conn_id in &cluster1_conns {
            manager.update_connection_status(cluster1_id, conn_id, ClusterSessionStatus::Connected);
        }
        for &conn_id in &cluster2_conns {
            manager.update_connection_status(cluster2_id, conn_id, ClusterSessionStatus::Connected);
        }

        // Fail a session in cluster1
        let failed_conn = cluster1_conns[0];
        manager.handle_session_failure(cluster1_id, failed_conn, "Test failure".to_string());

        // Verify cluster1 has the failure
        prop_assert!(
            manager.has_failures(cluster1_id),
            "Cluster 1 should have failures"
        );

        // Verify cluster2 is unaffected
        prop_assert!(
            !manager.has_failures(cluster2_id),
            "Cluster 2 should have no failures"
        );

        // Verify all cluster2 sessions are still connected
        let session2 = manager.get_session(cluster2_id).unwrap();
        prop_assert_eq!(
            session2.connected_count(),
            cluster2_conns.len(),
            "All cluster 2 sessions should still be connected"
        );
    }
}

// ============================================================================
// Unit Tests for Cluster Session Independence
// ============================================================================

/// Test that multiple session failures are tracked independently
#[test]
fn test_multiple_session_failures_tracked_independently() {
    let mut cluster = Cluster::new("Test".to_string());
    let conn1 = Uuid::new_v4();
    let conn2 = Uuid::new_v4();
    let conn3 = Uuid::new_v4();
    cluster.add_connection(conn1);
    cluster.add_connection(conn2);
    cluster.add_connection(conn3);

    let mut session = ClusterSession::new(&cluster);

    // Connect all
    session.update_session_status(conn1, ClusterSessionStatus::Connected);
    session.update_session_status(conn2, ClusterSessionStatus::Connected);
    session.update_session_status(conn3, ClusterSessionStatus::Connected);

    // Fail conn1 with specific error
    session.set_session_error(conn1, "Error 1".to_string());

    // Fail conn2 with different error
    session.set_session_error(conn2, "Error 2".to_string());

    // Verify each has its own error message
    let state1 = session.get_session_state(conn1).unwrap();
    let state2 = session.get_session_state(conn2).unwrap();
    let state3 = session.get_session_state(conn3).unwrap();

    assert_eq!(state1.error_message, Some("Error 1".to_string()));
    assert_eq!(state2.error_message, Some("Error 2".to_string()));
    assert!(state3.error_message.is_none());
    assert_eq!(state3.status, ClusterSessionStatus::Connected);
}

/// Test that session recovery doesn't affect other sessions
#[test]
fn test_session_recovery_independence() {
    let mut cluster = Cluster::new("Test".to_string());
    let conn1 = Uuid::new_v4();
    let conn2 = Uuid::new_v4();
    cluster.add_connection(conn1);
    cluster.add_connection(conn2);

    let mut session = ClusterSession::new(&cluster);

    // Connect both
    session.update_session_status(conn1, ClusterSessionStatus::Connected);
    session.update_session_status(conn2, ClusterSessionStatus::Connected);

    // Fail conn1
    session.set_session_error(conn1, "Connection lost".to_string());
    assert_eq!(session.error_count(), 1);

    // "Recover" conn1 by setting it back to Connected
    session.update_session_status(conn1, ClusterSessionStatus::Connected);

    // Verify conn1 is recovered
    let state1 = session.get_session_state(conn1).unwrap();
    assert_eq!(state1.status, ClusterSessionStatus::Connected);
    assert!(state1.error_message.is_none());

    // Verify conn2 is unchanged
    let state2 = session.get_session_state(conn2).unwrap();
    assert_eq!(state2.status, ClusterSessionStatus::Connected);

    // Verify no errors remain
    assert_eq!(session.error_count(), 0);
}

// ============================================================================
// Broadcast Mode Tests
// **Validates: Requirements 3.3**
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-enhancements, Broadcast Mode**
    /// **Validates: Requirements 3.3**
    ///
    /// When broadcast mode is enabled, input should be distributed to all
    /// connected sessions.
    #[test]
    fn prop_broadcast_mode_distributes_to_all_connected(
        cluster in arb_cluster_with_connections(2, 10),
        connect_flags in prop::collection::vec(any::<bool>(), 2..=10),
    ) {
        prop_assume!(!cluster.is_empty());
        prop_assume!(cluster.connection_count() >= 2);

        let conn_ids: Vec<Uuid> = cluster.connection_ids.clone();
        let mut session = ClusterSession::new(&cluster);

        // Enable broadcast mode
        session.set_broadcast_mode(true);

        // Connect some sessions based on flags
        let num_to_process = conn_ids.len().min(connect_flags.len());
        let mut expected_connected = 0;
        for i in 0..num_to_process {
            if connect_flags[i] {
                session.update_session_status(conn_ids[i], ClusterSessionStatus::Connected);
                expected_connected += 1;
            }
        }

        // Get broadcast targets
        let targets = session.broadcast_input("test input");

        // Should have exactly the number of connected sessions
        prop_assert_eq!(
            targets.len(),
            expected_connected,
            "Broadcast should target all {} connected sessions",
            expected_connected
        );

        // All targets should be connected
        for target in &targets {
            let state = session.get_session_state(*target).unwrap();
            prop_assert_eq!(
                state.status,
                ClusterSessionStatus::Connected,
                "Broadcast target {} should be connected",
                target
            );
        }
    }

    /// **Feature: rustconn-enhancements, Broadcast Mode**
    /// **Validates: Requirements 3.3**
    ///
    /// When broadcast mode is disabled, no targets should be returned.
    #[test]
    fn prop_broadcast_mode_disabled_returns_no_targets(
        cluster in arb_cluster_with_connections(1, 10),
    ) {
        prop_assume!(!cluster.is_empty());

        let conn_ids: Vec<Uuid> = cluster.connection_ids.clone();
        let mut session = ClusterSession::new(&cluster);

        // Ensure broadcast mode is disabled
        session.set_broadcast_mode(false);

        // Connect all sessions
        for &conn_id in &conn_ids {
            session.update_session_status(conn_id, ClusterSessionStatus::Connected);
        }

        // Get broadcast targets - should be empty
        let targets = session.broadcast_input("test input");
        prop_assert!(
            targets.is_empty(),
            "Broadcast should return no targets when disabled"
        );

        // get_input_targets should also return empty
        let input_targets = session.get_input_targets();
        prop_assert!(
            input_targets.is_empty(),
            "get_input_targets should return empty when broadcast disabled"
        );
    }

    /// **Feature: rustconn-enhancements, Broadcast Mode**
    /// **Validates: Requirements 3.6**
    ///
    /// Toggling broadcast mode should correctly switch between enabled and disabled.
    #[test]
    fn prop_broadcast_mode_toggle(
        cluster in arb_cluster_with_connections(1, 5),
        initial_state in any::<bool>(),
        num_toggles in 1usize..10,
    ) {
        prop_assume!(!cluster.is_empty());

        let mut session = ClusterSession::new(&cluster);
        session.set_broadcast_mode(initial_state);

        let mut expected_state = initial_state;
        for _ in 0..num_toggles {
            let new_state = session.toggle_broadcast_mode();
            expected_state = !expected_state;
            prop_assert_eq!(
                new_state,
                expected_state,
                "Toggle should return the new state"
            );
            prop_assert_eq!(
                session.is_broadcast_mode(),
                expected_state,
                "is_broadcast_mode should match the toggled state"
            );
        }
    }
}

/// Test that broadcast mode indicator is correctly reflected in summary
#[test]
fn test_broadcast_mode_in_summary() {
    let mut manager = ClusterManager::new();
    let mut cluster = Cluster::new("Test".to_string());
    cluster.add_connection(Uuid::new_v4());
    cluster.broadcast_enabled = false;
    let cluster_id = cluster.id;

    manager.add_cluster(cluster);
    manager.start_session(cluster_id).unwrap();

    // Initially broadcast should be disabled
    let summary = manager.get_session_summary(cluster_id).unwrap();
    assert!(!summary.broadcast_mode);

    // Enable broadcast mode
    manager
        .get_session_mut(cluster_id)
        .unwrap()
        .set_broadcast_mode(true);

    // Summary should reflect the change
    let summary = manager.get_session_summary(cluster_id).unwrap();
    assert!(summary.broadcast_mode);
}

/// Test that broadcast targets exclude disconnected and error sessions
#[test]
fn test_broadcast_excludes_inactive_sessions() {
    let mut cluster = Cluster::new("Test".to_string());
    let conn1 = Uuid::new_v4();
    let conn2 = Uuid::new_v4();
    let conn3 = Uuid::new_v4();
    let conn4 = Uuid::new_v4();
    cluster.add_connection(conn1);
    cluster.add_connection(conn2);
    cluster.add_connection(conn3);
    cluster.add_connection(conn4);

    let mut session = ClusterSession::new(&cluster);
    session.set_broadcast_mode(true);

    // Set different statuses
    session.update_session_status(conn1, ClusterSessionStatus::Connected);
    session.update_session_status(conn2, ClusterSessionStatus::Disconnected);
    session.update_session_status(conn3, ClusterSessionStatus::Connected);
    session.set_session_error(conn4, "Error".to_string());

    let targets = session.broadcast_input("test");

    // Should only include connected sessions
    assert_eq!(targets.len(), 2);
    assert!(targets.contains(&conn1));
    assert!(targets.contains(&conn3));
    assert!(!targets.contains(&conn2));
    assert!(!targets.contains(&conn4));
}

// ============================================================================
// Cluster Serialization Tests
// **Validates: Requirements 3.1**
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-enhancements, Cluster Serialization**
    /// **Validates: Requirements 3.1**
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
    /// **Validates: Requirements 3.1**
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
// **Validates: Requirements 10.1, 10.2**
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-bugfixes, Property 8: Cluster Serialization Round-Trip**
    /// **Validates: Requirements 10.1, 10.2**
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
    /// **Validates: Requirements 10.1, 10.2**
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
// **Validates: Requirements 4.1, 4.2, 4.3**
//
// For any cluster add/edit/delete operation, the cluster list should
// immediately reflect the change without requiring dialog close/reopen.
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-fixes-v2, Property 3: Cluster List Refresh After Modification**
    /// **Validates: Requirements 4.1, 4.2, 4.3**
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
    /// **Validates: Requirements 4.2**
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
    /// **Validates: Requirements 4.3**
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
    /// **Validates: Requirements 4.1, 4.2, 4.3**
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
