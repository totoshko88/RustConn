//! Property-based tests for SSH key inheritance resolution.
//!
//! Tests properties P5 and P6 from the Cloud Sync design:
//! - P5: `resolve_ssh_key_path()` terminates for any group hierarchy (including cycles)
//! - P6: `resolve_ssh_key_path()` returns the nearest ancestor's key or `None`

use std::path::PathBuf;

use proptest::prelude::*;
use rustconn_core::connection::ssh_inheritance::resolve_ssh_key_path;
use rustconn_core::models::{Connection, ConnectionGroup, ProtocolConfig, SshKeySource};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

/// Generates an arbitrary group hierarchy of 1–20 groups.
///
/// Each group gets a deterministic UUID. `parent_id` is randomly assigned:
/// some point to valid groups (forming trees or cycles), some are `None` (roots).
fn arb_group_hierarchy() -> impl Strategy<Value = Vec<ConnectionGroup>> {
    // Generate 1..=20 groups, each with an optional parent index
    (1usize..=20)
        .prop_flat_map(|n| {
            // For each group: (name_suffix, optional parent index, optional ssh_key_path)

            prop::collection::vec(
                (
                    "[a-z]{1,8}",                     // name suffix
                    prop::option::of(0usize..20),     // parent index (may be out of range)
                    prop::option::of("[/a-z]{1,20}"), // ssh_key_path
                ),
                n..=n,
            )
        })
        .prop_map(|specs| {
            // Create groups with deterministic UUIDs based on index
            let ids: Vec<Uuid> = (0..specs.len())
                .map(|i| Uuid::from_u128(i as u128 + 1000))
                .collect();

            specs
                .into_iter()
                .enumerate()
                .map(|(i, (name_suffix, parent_idx, key_path))| {
                    let mut group = ConnectionGroup::new(format!("group-{name_suffix}"));
                    group.id = ids[i];

                    // Assign parent_id: if index is valid and not self, use it
                    group.parent_id = parent_idx.and_then(|pi| {
                        if pi < ids.len() && pi != i {
                            Some(ids[pi])
                        } else if pi >= ids.len() {
                            // Out-of-range index: dangling reference (orphaned parent_id)
                            Some(Uuid::from_u128(pi as u128 + 1000))
                        } else {
                            None // self-reference → treat as root
                        }
                    });

                    group.ssh_key_path = key_path.map(PathBuf::from);
                    group
                })
                .collect()
        })
}

/// Manually walks the group chain to find the expected nearest ancestor key.
///
/// This is the "oracle" implementation used to verify `resolve_ssh_key_path`.
fn expected_key_path(connection: &Connection, groups: &[ConnectionGroup]) -> Option<PathBuf> {
    // Check connection-level key first
    if let ProtocolConfig::Ssh(cfg) | ProtocolConfig::Sftp(cfg) = &connection.protocol_config {
        match &cfg.key_source {
            SshKeySource::File { path } if !path.as_os_str().is_empty() => {
                return Some(path.clone());
            }
            SshKeySource::Agent { .. } | SshKeySource::Default => return None,
            SshKeySource::Inherit | SshKeySource::File { .. } => {
                // Fall through to group walk
            }
        }
    }

    let mut visited = std::collections::HashSet::new();
    let mut current = connection.group_id;

    while let Some(gid) = current {
        if !visited.insert(gid) {
            return None; // cycle
        }
        let group = groups.iter().find(|g| g.id == gid)?;
        if let Some(ref path) = group.ssh_key_path {
            return Some(path.clone());
        }
        current = group.parent_id;
    }

    None
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// **Validates: Requirements 1.3**
    ///
    /// P5 — SSH Inheritance Termination: `resolve_ssh_key_path()` terminates
    /// for any group hierarchy, including cycles. If this test body executes
    /// to completion, the function terminated.
    #[test]
    fn p5_resolve_ssh_key_path_always_terminates(
        groups in arb_group_hierarchy(),
    ) {
        let n = groups.len();
        // Create a connection pointing to each group and call resolve
        for i in 0..n {
            let group_id = Uuid::from_u128(i as u128 + 1000);
            let mut conn = Connection::new_ssh("t".into(), "h".into(), 22);
            conn.group_id = Some(group_id);
            if let ProtocolConfig::Ssh(ref mut cfg) = conn.protocol_config {
                cfg.key_source = SshKeySource::Inherit;
            }
            // If this returns (doesn't hang), termination is proven
            let _result = resolve_ssh_key_path(&conn, &groups);
        }
    }

    /// **Validates: Requirements 1.2**
    ///
    /// P6 — SSH Inheritance Correctness: The result of `resolve_ssh_key_path()`
    /// matches a manual chain walk (the oracle). If the nearest ancestor has
    /// `ssh_key_path = Some(p)`, the function returns `Some(p)`. If none has
    /// it set, returns `None`.
    #[test]
    fn p6_resolve_ssh_key_path_matches_oracle(
        groups in arb_group_hierarchy(),
    ) {
        let n = groups.len();
        for i in 0..n {
            let group_id = Uuid::from_u128(i as u128 + 1000);
            let mut conn = Connection::new_ssh("t".into(), "h".into(), 22);
            conn.group_id = Some(group_id);
            if let ProtocolConfig::Ssh(ref mut cfg) = conn.protocol_config {
                cfg.key_source = SshKeySource::Inherit;
            }

            let actual = resolve_ssh_key_path(&conn, &groups);
            let expected = expected_key_path(&conn, &groups);

            prop_assert_eq!(
                actual, expected,
                "Mismatch for connection in group index {} (id={})",
                i, group_id
            );
        }
    }

    /// **Validates: Requirements 1.2**
    ///
    /// P6 supplement — Determinism: Same input always produces same output.
    #[test]
    fn p6_resolve_ssh_key_path_is_deterministic(
        groups in arb_group_hierarchy(),
        group_idx in 0usize..20,
    ) {
        let n = groups.len();
        let idx = group_idx % n;
        let group_id = Uuid::from_u128(idx as u128 + 1000);

        let mut conn = Connection::new_ssh("t".into(), "h".into(), 22);
        conn.group_id = Some(group_id);
        if let ProtocolConfig::Ssh(ref mut cfg) = conn.protocol_config {
            cfg.key_source = SshKeySource::Inherit;
        }

        let result1 = resolve_ssh_key_path(&conn, &groups);
        let result2 = resolve_ssh_key_path(&conn, &groups);

        prop_assert_eq!(result1, result2, "resolve_ssh_key_path is not deterministic");
    }

    /// **Validates: Requirements 1.3**
    ///
    /// P5 supplement — Cycles with keys: Even when a cycle exists, if a group
    /// in the cycle has a key set, the function returns it (before hitting
    /// the cycle again).
    #[test]
    fn p5_cycle_with_key_returns_key(
        key_path in "[/a-z]{1,20}",
        key_on_first in any::<bool>(),
    ) {
        let id_a = Uuid::from_u128(5000);
        let id_b = Uuid::from_u128(5001);

        let mut group_a = ConnectionGroup::new("A".into());
        group_a.id = id_a;
        group_a.parent_id = Some(id_b);

        let mut group_b = ConnectionGroup::new("B".into());
        group_b.id = id_b;
        group_b.parent_id = Some(id_a); // cycle: A → B → A

        if key_on_first {
            group_a.ssh_key_path = Some(PathBuf::from(&key_path));
        } else {
            group_b.ssh_key_path = Some(PathBuf::from(&key_path));
        }

        let mut conn = Connection::new_ssh("t".into(), "h".into(), 22);
        conn.group_id = Some(id_a);
        if let ProtocolConfig::Ssh(ref mut cfg) = conn.protocol_config {
            cfg.key_source = SshKeySource::Inherit;
        }

        let groups = vec![group_a, group_b];
        let result = resolve_ssh_key_path(&conn, &groups);

        // Group A is visited first. If key_on_first, A has the key → return it.
        // If !key_on_first, A has no key → visit B (which has key) → return it.
        prop_assert_eq!(result, Some(PathBuf::from(&key_path)));
    }
}
