//! Property tests for Simple Sync bidirectional merge engine.
//!
//! **Validates: Requirements 9.2, 9.3, 9.4, 9.6**
//!
//! Tests correctness properties P7 (convergence) and P8 (tombstone consistency)
//! from the design document.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use proptest::prelude::*;
use rustconn_core::models::Connection;
use rustconn_core::sync::full_export::FullSyncExport;
use rustconn_core::sync::full_merge::{FullMergeEngine, LocalState};
use rustconn_core::sync::tombstone::{SyncEntityType, Tombstone};
use uuid::Uuid;

/// Strategy for generating a fixed timestamp within a reasonable range.
fn arb_timestamp() -> impl Strategy<Value = DateTime<Utc>> {
    (0i64..365 * 24 * 3600).prop_map(|secs| Utc::now() - Duration::seconds(secs))
}

/// Strategy for generating a set of entity IDs with timestamps.
fn arb_entity_map(max_size: usize) -> impl Strategy<Value = HashMap<Uuid, DateTime<Utc>>> {
    proptest::collection::hash_map(
        proptest::arbitrary::any::<[u8; 16]>().prop_map(|b| Uuid::from_bytes(b)),
        arb_timestamp(),
        0..max_size,
    )
}

/// Build a FullSyncExport from a set of connection IDs + timestamps.
fn build_remote(
    device_id: Uuid,
    connections: &HashMap<Uuid, DateTime<Utc>>,
    tombstones: Vec<Tombstone>,
) -> FullSyncExport {
    let conns: Vec<Connection> = connections
        .iter()
        .map(|(&id, &updated_at)| {
            let mut c = Connection::new_ssh(
                format!("conn-{}", &id.to_string()[..8]),
                "10.0.0.1".to_owned(),
                22,
            );
            c.id = id;
            c.updated_at = updated_at;
            c
        })
        .collect();

    FullSyncExport {
        sync_version: 1,
        sync_type: "full".to_owned(),
        exported_at: Utc::now(),
        app_version: "0.12.0".to_owned(),
        device_id,
        device_name: "remote".to_owned(),
        connections: conns,
        groups: Vec::new(),
        templates: Vec::new(),
        snippets: Vec::new(),
        clusters: Vec::new(),
        variables: Vec::new(),
        tombstones,
    }
}

proptest! {
    /// **Validates: Requirements 9.2** — UUID-based merge
    ///
    /// P7 partial: Every remote connection is either created, updated, or
    /// skipped (local is same/newer). No remote connection is silently dropped.
    #[test]
    fn merge_classifies_every_remote_connection(
        local_conns in arb_entity_map(10),
        remote_conns in arb_entity_map(10),
    ) {
        let local_device = Uuid::new_v4();
        let remote_device = Uuid::new_v4();

        let local = LocalState {
            device_id: local_device,
            connections: local_conns.clone(),
            groups: HashMap::new(),
            templates: HashMap::new(),
            snippets: HashMap::new(),
            clusters: HashMap::new(),
            tombstones: Vec::new(),
            retention_days: 30,
        };

        let remote = build_remote(remote_device, &remote_conns, Vec::new());
        let result = FullMergeEngine::merge(&local, &remote);

        // Every remote connection must be accounted for
        for &remote_id in remote_conns.keys() {
            let created = result.to_create.iter().any(|a| a.id == remote_id);
            let updated = result.to_update.iter().any(|a| a.id == remote_id);
            let skipped = local_conns.get(&remote_id).is_some_and(|&local_ts| {
                local_ts >= remote_conns[&remote_id]
            });

            prop_assert!(
                created || updated || skipped,
                "Remote connection {} was not classified", remote_id
            );
        }

        // No duplicates in create + update
        let mut seen = std::collections::HashSet::new();
        for action in result.to_create.iter().chain(result.to_update.iter()) {
            if action.entity_type == SyncEntityType::Connection {
                prop_assert!(seen.insert(action.id), "Duplicate action for {}", action.id);
            }
        }
    }

    /// **Validates: Requirements 9.3** — Tombstone consistency (P8)
    ///
    /// If a tombstone has `deleted_at > entity.updated_at`, the entity must
    /// be in `to_delete` (for remote tombstones applied to local entities).
    #[test]
    fn tombstone_deletes_older_entities(
        local_conns in arb_entity_map(8),
    ) {
        let local_device = Uuid::new_v4();
        let remote_device = Uuid::new_v4();

        // Create tombstones for all local connections with deleted_at = now
        // (which is guaranteed to be >= any local updated_at from arb_timestamp)
        let now = Utc::now();
        let tombstones: Vec<Tombstone> = local_conns.keys().map(|&id| {
            Tombstone::with_deleted_at(SyncEntityType::Connection, id, now)
        }).collect();

        let local = LocalState {
            device_id: local_device,
            connections: local_conns.clone(),
            groups: HashMap::new(),
            templates: HashMap::new(),
            snippets: HashMap::new(),
            clusters: HashMap::new(),
            tombstones: Vec::new(),
            retention_days: 30,
        };

        let remote = build_remote(remote_device, &HashMap::new(), tombstones);
        let result = FullMergeEngine::merge(&local, &remote);

        // Every local connection should be deleted (tombstone.deleted_at = now >= all updated_at)
        for &conn_id in local_conns.keys() {
            let deleted = result.to_delete.iter().any(|a| a.id == conn_id);
            prop_assert!(deleted, "Connection {} should be deleted by tombstone", conn_id);
        }
    }

    /// **Validates: Requirements 9.4** — Tombstone cleanup
    ///
    /// Tombstones older than retention_days appear in tombstones_to_remove.
    #[test]
    fn expired_tombstones_cleaned_up(
        retention_days in 1u32..365,
        expired_count in 0usize..5,
        fresh_count in 0usize..5,
    ) {
        let now = Utc::now();
        let mut tombstones = Vec::new();

        // Expired tombstones
        for _ in 0..expired_count {
            tombstones.push(Tombstone::with_deleted_at(
                SyncEntityType::Connection,
                Uuid::new_v4(),
                now - Duration::days(i64::from(retention_days) + 1),
            ));
        }

        // Fresh tombstones (well within retention period)
        for _ in 0..fresh_count {
            tombstones.push(Tombstone::with_deleted_at(
                SyncEntityType::Connection,
                Uuid::new_v4(),
                now,
            ));
        }

        let local = LocalState {
            device_id: Uuid::new_v4(),
            connections: HashMap::new(),
            groups: HashMap::new(),
            templates: HashMap::new(),
            snippets: HashMap::new(),
            clusters: HashMap::new(),
            tombstones,
            retention_days,
        };

        let remote = build_remote(Uuid::new_v4(), &HashMap::new(), Vec::new());
        let result = FullMergeEngine::merge(&local, &remote);

        prop_assert_eq!(result.tombstones_to_remove.len(), expired_count);
    }

    /// **Validates: Requirements 9.6** — Convergence (P7)
    ///
    /// After A exports → B imports, both devices agree on which connections
    /// exist (no concurrent edits scenario).
    #[test]
    fn convergence_after_one_sync_cycle(
        shared_conns in arb_entity_map(5),
        a_only_conns in arb_entity_map(3),
        b_only_conns in arb_entity_map(3),
    ) {
        let device_a = Uuid::new_v4();
        let device_b = Uuid::new_v4();

        // Device A has shared + a_only
        let mut a_conns = shared_conns.clone();
        a_conns.extend(&a_only_conns);

        // Device B has shared + b_only
        let mut b_conns = shared_conns.clone();
        b_conns.extend(&b_only_conns);

        // A exports, B imports
        let a_export = build_remote(device_a, &a_conns, Vec::new());
        let b_local = LocalState {
            device_id: device_b,
            connections: b_conns.clone(),
            groups: HashMap::new(),
            templates: HashMap::new(),
            snippets: HashMap::new(),
            clusters: HashMap::new(),
            tombstones: Vec::new(),
            retention_days: 30,
        };

        let result = FullMergeEngine::merge(&b_local, &a_export);

        // After merge, B should have all of A's connections either as
        // create (new) or update (newer) or already present (same/older)
        for &a_id in a_conns.keys() {
            let in_b = b_conns.contains_key(&a_id);
            let created = result.to_create.iter().any(|a| a.id == a_id);
            let updated = result.to_update.iter().any(|a| a.id == a_id);

            prop_assert!(
                in_b || created || updated,
                "A's connection {} not accounted for in B's merge result", a_id
            );
        }
    }
}
