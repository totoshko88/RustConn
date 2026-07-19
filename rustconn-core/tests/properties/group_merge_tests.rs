//! Property-based tests for `GroupMergeEngine::merge()`.
//!
//! Tests properties P1 and P3 from the Cloud Sync design:
//! - P1: Group Merge Completeness — every remote connection is either created,
//!   updates an existing local connection, or matches an unchanged local
//!   connection. No remote connection is silently dropped.
//! - P3: Group Merge Determinism — same inputs always produce the same
//!   `GroupMergeResult`.

use std::collections::HashSet;

use chrono::{Duration, Utc};
use proptest::prelude::*;
use rustconn_core::models::{
    AutomationConfig, Connection, ConnectionGroup, PasswordSource, ProtocolConfig, ProtocolType,
    SshConfig,
};
use rustconn_core::sync::group_export::{GroupSyncExport, SyncConnection, SyncGroup};
use rustconn_core::sync::group_merge::GroupMergeEngine;
use rustconn_core::sync::variable_template::VariableTemplate;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Creates a minimal `SyncConnection`.
fn make_sync_conn(
    name: &str,
    group_path: &str,
    updated_at: chrono::DateTime<chrono::Utc>,
) -> SyncConnection {
    SyncConnection {
        name: name.to_owned(),
        group_path: group_path.to_owned(),
        host: "10.0.0.1".to_owned(),
        port: 22,
        protocol: ProtocolType::Ssh,
        username: None,
        description: None,
        tags: Vec::new(),
        protocol_config: ProtocolConfig::Ssh(SshConfig::default()),
        password_source: PasswordSource::None,
        automation: AutomationConfig::default(),
        custom_properties: Vec::new(),
        pre_connect_task: None,
        post_disconnect_task: None,
        wol_config: None,
        icon: None,
        highlight_rules: Vec::new(),
        updated_at,
    }
}

/// Creates a minimal `SyncGroup`.
fn make_sync_group(name: &str, path: &str) -> SyncGroup {
    SyncGroup {
        name: name.to_owned(),
        path: path.to_owned(),
        description: None,
        icon: None,
        username: None,
        domain: None,
        ssh_auth_method: None,
        ssh_proxy_jump: None,
    }
}

/// Creates a minimal `GroupSyncExport`.
fn make_export(
    groups: Vec<SyncGroup>,
    connections: Vec<SyncConnection>,
    variable_templates: Vec<VariableTemplate>,
) -> GroupSyncExport {
    GroupSyncExport {
        sync_version: 1,
        sync_type: "group".to_owned(),
        exported_at: Utc::now(),
        app_version: "0.12.0".to_owned(),
        master_device_id: Uuid::from_u128(9999),
        master_device_name: "test-device".to_owned(),
        root_group: make_sync_group("Root", "Root"),
        groups,
        connections,
        variable_templates,
    }
}

/// Creates a local `ConnectionGroup` with a deterministic UUID.
fn make_local_group(name: &str, parent_id: Option<Uuid>, id: Uuid) -> ConnectionGroup {
    let mut group = if let Some(pid) = parent_id {
        ConnectionGroup::with_parent(name.to_owned(), pid)
    } else {
        ConnectionGroup::new(name.to_owned())
    };
    group.id = id;
    group
}

/// Creates a local `Connection` in a group with a deterministic UUID.
fn make_local_conn(
    name: &str,
    group_id: Uuid,
    conn_id: Uuid,
    updated_at: chrono::DateTime<chrono::Utc>,
) -> Connection {
    let mut c = Connection::new_ssh(name.to_owned(), "10.0.0.1".to_owned(), 22);
    c.group_id = Some(group_id);
    c.id = conn_id;
    c.updated_at = updated_at;
    c
}

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

/// Complete generated merge scenario.
#[derive(Debug, Clone)]
struct MergeScenario {
    local_groups: Vec<(String, Uuid, Option<Uuid>)>,
    local_connections: Vec<(String, Uuid, Uuid, String, chrono::DateTime<chrono::Utc>)>,
    remote_groups: Vec<SyncGroup>,
    remote_connections: Vec<(String, String, chrono::DateTime<chrono::Utc>)>,
    remote_variables: Vec<VariableTemplate>,
    local_variable_names: HashSet<String>,
}

/// Generates an arbitrary merge scenario with:
/// - 1–5 local subgroups under a root
/// - 0–10 local connections spread across those groups
/// - 0–5 remote subgroups (some overlapping, some new)
/// - 0–10 remote connections (some overlapping, some new)
/// - 0–3 variable templates
fn arb_merge_scenario() -> impl Strategy<Value = MergeScenario> {
    let root_id = Uuid::from_u128(1);

    (
        1usize..=5,  // num local subgroups
        0usize..=10, // num local connections
        0usize..=5,  // num remote subgroups
        0usize..=10, // num remote connections
        0usize..=3,  // num variable templates
        0usize..=3,  // num locally known variable names
    )
        .prop_flat_map(move |(n_lg, n_lc, n_rg, n_rc, n_vars, n_lv)| {
            // Group and connection names from shared pools for overlap
            let local_group_indices = prop::collection::vec(0usize..10, n_lg..=n_lg);
            let remote_group_indices = prop::collection::vec(0usize..10, n_rg..=n_rg);
            let local_conn_specs = prop::collection::vec((0usize..20, 0i64..100), n_lc..=n_lc);
            let remote_conn_specs = prop::collection::vec((0usize..20, 0i64..100), n_rc..=n_rc);
            let var_names = prop::collection::vec("[a-z]{3,8}", n_vars..=n_vars);
            let local_var_names = prop::collection::vec("[a-z]{3,8}", n_lv..=n_lv);

            (
                local_group_indices,
                remote_group_indices,
                local_conn_specs,
                remote_conn_specs,
                var_names,
                local_var_names,
            )
        })
        .prop_map(
            move |(
                local_group_idx,
                remote_group_idx,
                local_conn_specs,
                remote_conn_specs,
                var_names,
                local_var_names,
            )| {
                let group_pool: Vec<String> = (0..10).map(|i| format!("sub-{i}")).collect();
                let conn_pool: Vec<String> = (0..20).map(|i| format!("conn-{i}")).collect();

                let base_time = Utc::now() - Duration::hours(50);

                // Build local groups (deduplicated by name)
                let mut seen_names = HashSet::new();
                let mut local_groups = Vec::new();
                for &idx in &local_group_idx {
                    let name = &group_pool[idx];
                    if seen_names.insert(name.clone()) {
                        let gid = Uuid::from_u128(100 + idx as u128);
                        local_groups.push((name.clone(), gid, Some(root_id)));
                    }
                }

                // Build remote groups (deduplicated by path)
                let mut seen_paths = HashSet::new();
                let mut remote_groups = Vec::new();
                for &idx in &remote_group_idx {
                    let name = &group_pool[idx];
                    let path = format!("Root/{name}");
                    if seen_paths.insert(path.clone()) {
                        remote_groups.push(make_sync_group(name, &path));
                    }
                }

                // Build local connections (deduplicated by name+group_path)
                let mut seen_keys = HashSet::new();
                let mut local_connections = Vec::new();
                for (i, &(conn_idx, hours_offset)) in local_conn_specs.iter().enumerate() {
                    if local_groups.is_empty() {
                        continue;
                    }
                    let conn_name = &conn_pool[conn_idx];
                    let group = &local_groups[i % local_groups.len()];
                    let group_path = format!("Root/{}", group.0);
                    let key = (conn_name.clone(), group_path.clone());
                    if seen_keys.insert(key) {
                        let conn_id = Uuid::from_u128(1000 + i as u128);
                        let ts = base_time + Duration::hours(hours_offset);
                        local_connections.push((
                            conn_name.clone(),
                            conn_id,
                            group.1,
                            group_path,
                            ts,
                        ));
                    }
                }

                // Build remote connections (deduplicated by name+group_path)
                let mut seen_rkeys = HashSet::new();
                let mut remote_connections = Vec::new();
                let mut rpaths: Vec<String> =
                    remote_groups.iter().map(|g| g.path.clone()).collect();
                if rpaths.is_empty() {
                    rpaths.push("Root".to_owned());
                }
                for (i, &(conn_idx, hours_offset)) in remote_conn_specs.iter().enumerate() {
                    let conn_name = &conn_pool[conn_idx];
                    let group_path = &rpaths[i % rpaths.len()];
                    let key = (conn_name.clone(), group_path.clone());
                    if seen_rkeys.insert(key) {
                        let ts = base_time + Duration::hours(hours_offset);
                        remote_connections.push((conn_name.clone(), group_path.clone(), ts));
                    }
                }

                let local_var_set: HashSet<String> = local_var_names.into_iter().collect();
                let remote_vars: Vec<VariableTemplate> = var_names
                    .into_iter()
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .map(|name| VariableTemplate {
                        name,
                        description: None,
                        is_secret: true,
                        default_value: None,
                    })
                    .collect();

                MergeScenario {
                    local_groups,
                    local_connections,
                    remote_groups,
                    remote_connections,
                    remote_variables: remote_vars,
                    local_variable_names: local_var_set,
                }
            },
        )
}

/// Converts a `MergeScenario` into the concrete types needed by
/// `GroupMergeEngine::merge()`.
fn build_merge_inputs(
    scenario: &MergeScenario,
) -> (
    Vec<ConnectionGroup>,
    Vec<Connection>,
    GroupSyncExport,
    HashSet<String>,
) {
    let root_id = Uuid::from_u128(1);

    // Build local groups
    let mut local_groups = vec![make_local_group("Root", None, root_id)];
    for (name, id, parent_id) in &scenario.local_groups {
        local_groups.push(make_local_group(name, *parent_id, *id));
    }

    // Build local connections
    let local_connections: Vec<Connection> = scenario
        .local_connections
        .iter()
        .map(|(name, conn_id, group_id, _, updated_at)| {
            make_local_conn(name, *group_id, *conn_id, *updated_at)
        })
        .collect();

    // Build remote connections
    let remote_connections: Vec<SyncConnection> = scenario
        .remote_connections
        .iter()
        .map(|(name, group_path, updated_at)| make_sync_conn(name, group_path, *updated_at))
        .collect();

    let export = make_export(
        scenario.remote_groups.clone(),
        remote_connections,
        scenario.remote_variables.clone(),
    );

    (
        local_groups,
        local_connections,
        export,
        scenario.local_variable_names.clone(),
    )
}

// ---------------------------------------------------------------------------
// Property tests
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// **Validates: Requirements 4.7**
    ///
    /// P1 — Group Merge Completeness: Every connection in the remote export
    /// is either created locally, updates an existing local connection, or
    /// matches an unchanged local connection. No remote connection is silently
    /// dropped.
    #[test]
    fn p1_every_remote_connection_is_accounted_for(
        scenario in arb_merge_scenario(),
    ) {
        let (local_groups, local_connections, export, local_vars) =
            build_merge_inputs(&scenario);

        let result = GroupMergeEngine::merge(
            &local_groups,
            &local_connections,
            &export,
            &local_vars,
        );

        // Build the set of remote (name, group_path) keys
        let remote_keys: HashSet<(String, String)> = export
            .connections
            .iter()
            .map(|c| (c.name.clone(), c.group_path.clone()))
            .collect();

        // Build the set of local (name, group_path) keys
        let local_keys: HashSet<(String, String)> = scenario
            .local_connections
            .iter()
            .map(|(name, _, _, group_path, _)| (name.clone(), group_path.clone()))
            .collect();

        // Collect keys from create and update results
        let created_keys: HashSet<(String, String)> = result
            .connections_to_create
            .iter()
            .map(|c| (c.name.clone(), c.group_path.clone()))
            .collect();

        let updated_keys: HashSet<(String, String)> = result
            .connections_to_update
            .iter()
            .map(|(_, c)| (c.name.clone(), c.group_path.clone()))
            .collect();

        for key in &remote_keys {
            let in_create = created_keys.contains(key);
            let in_update = updated_keys.contains(key);
            let in_local = local_keys.contains(key);

            // If not created and not updated, it must exist locally
            // (unchanged because local is same or newer)
            if !in_create && !in_update {
                prop_assert!(
                    in_local,
                    "Remote connection {:?} was silently dropped — \
                     not in create, update, or local",
                    key
                );
            }

            // Must not appear in both create and update
            prop_assert!(
                !(in_create && in_update),
                "Remote connection {:?} appears in both create and update",
                key
            );
        }
    }

    /// **Validates: Requirements 4.6**
    ///
    /// P3 — Group Merge Determinism: Given the same local state and remote
    /// export, `GroupMergeEngine::merge()` always produces the same
    /// `GroupMergeResult` (compared as sets, since Vec ordering may vary
    /// due to HashMap iteration order).
    #[test]
    fn p3_merge_is_deterministic(
        scenario in arb_merge_scenario(),
    ) {
        let (local_groups, local_connections, export, local_vars) =
            build_merge_inputs(&scenario);

        let result1 = GroupMergeEngine::merge(
            &local_groups,
            &local_connections,
            &export,
            &local_vars,
        );

        let result2 = GroupMergeEngine::merge(
            &local_groups,
            &local_connections,
            &export,
            &local_vars,
        );

        // Compare as sets — the merge is deterministic in content but
        // HashMap iteration order may vary between calls.
        let create_set1: HashSet<String> = result1
            .connections_to_create
            .iter()
            .map(|c| format!("{}:{}", c.name, c.group_path))
            .collect();
        let create_set2: HashSet<String> = result2
            .connections_to_create
            .iter()
            .map(|c| format!("{}:{}", c.name, c.group_path))
            .collect();
        prop_assert_eq!(&create_set1, &create_set2, "connections_to_create differs");

        let update_set1: HashSet<String> = result1
            .connections_to_update
            .iter()
            .map(|(id, c)| format!("{id}:{}:{}", c.name, c.group_path))
            .collect();
        let update_set2: HashSet<String> = result2
            .connections_to_update
            .iter()
            .map(|(id, c)| format!("{id}:{}:{}", c.name, c.group_path))
            .collect();
        prop_assert_eq!(&update_set1, &update_set2, "connections_to_update differs");

        let delete_set1: HashSet<Uuid> =
            result1.connections_to_delete.iter().copied().collect();
        let delete_set2: HashSet<Uuid> =
            result2.connections_to_delete.iter().copied().collect();
        prop_assert_eq!(&delete_set1, &delete_set2, "connections_to_delete differs");

        let group_create_set1: HashSet<String> = result1
            .groups_to_create
            .iter()
            .map(|g| g.path.clone())
            .collect();
        let group_create_set2: HashSet<String> = result2
            .groups_to_create
            .iter()
            .map(|g| g.path.clone())
            .collect();
        prop_assert_eq!(&group_create_set1, &group_create_set2, "groups_to_create differs");

        let group_delete_set1: HashSet<Uuid> =
            result1.groups_to_delete.iter().copied().collect();
        let group_delete_set2: HashSet<Uuid> =
            result2.groups_to_delete.iter().copied().collect();
        prop_assert_eq!(&group_delete_set1, &group_delete_set2, "groups_to_delete differs");

        let var_set1: HashSet<String> = result1
            .variables_to_create
            .iter()
            .map(|v| v.name.clone())
            .collect();
        let var_set2: HashSet<String> = result2
            .variables_to_create
            .iter()
            .map(|v| v.name.clone())
            .collect();
        prop_assert_eq!(&var_set1, &var_set2, "variables_to_create differs");
    }

    /// **Validates: Requirements 4.7**
    ///
    /// P1 supplement — Local-only connections deleted: connections that exist
    /// only locally (not in remote) must appear in `connections_to_delete`.
    #[test]
    fn p1_local_only_connections_are_deleted(
        scenario in arb_merge_scenario(),
    ) {
        let (local_groups, local_connections, export, local_vars) =
            build_merge_inputs(&scenario);

        let result = GroupMergeEngine::merge(
            &local_groups,
            &local_connections,
            &export,
            &local_vars,
        );

        // Build remote keys
        let remote_keys: HashSet<(String, String)> = export
            .connections
            .iter()
            .map(|c| (c.name.clone(), c.group_path.clone()))
            .collect();

        // Every local connection NOT in remote must be in connections_to_delete
        let deleted_ids: HashSet<Uuid> =
            result.connections_to_delete.iter().copied().collect();

        for (name, conn_id, _, group_path, _) in &scenario.local_connections {
            let key = (name.clone(), group_path.clone());
            if !remote_keys.contains(&key) {
                prop_assert!(
                    deleted_ids.contains(conn_id),
                    "Local-only connection {:?} (id={}) was not marked for deletion",
                    key,
                    conn_id
                );
            }
        }
    }
}
