//! Property-based tests for `GroupMergeEngine::merge()`.
//!
//! Tests properties P1 and P3 from the Cloud Sync design:
//! - P1: Group Merge Completeness — every remote connection is either created,
//!   updates an existing local connection, or matches an unchanged local
//!   connection. No remote connection is silently dropped.
//! - P3: Group Merge Determinism — same inputs always produce the same
//!   `GroupMergeResult`.
//!
//! And, since 0.22.13, that matching happens inside the synced group: an
//! export merged into a local tree that already mirrors it changes nothing,
//! whatever either root is called and wherever the Import root sits.

use std::collections::HashSet;

use chrono::{Duration, Utc};
use proptest::prelude::*;
use rustconn_core::models::{
    AutomationConfig, Connection, ConnectionGroup, PasswordSource, ProtocolConfig, ProtocolType,
    SshConfig,
};
use rustconn_core::sync::group_export::{GroupSyncExport, SyncConnection, SyncGroup};
use rustconn_core::sync::group_merge::GroupMergeEngine;
use rustconn_core::sync::manager::SyncManager;
use rustconn_core::sync::settings::{SyncMode, SyncSettings};
use rustconn_core::sync::variable_template::VariableTemplate;
use tempfile::TempDir;
use uuid::Uuid;

/// The local Import root of every generated P1/P3 scenario.
const ROOT_ID: Uuid = Uuid::from_u128(1);

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
        id: None,
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
        monitoring_config: None,
        updated_at,
    }
}

/// Creates a minimal `SyncGroup`.
fn make_sync_group(name: &str, path: &str) -> SyncGroup {
    SyncGroup {
        id: None,
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
    let root_id = ROOT_ID;

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
    let root_id = ROOT_ID;

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
            ROOT_ID,
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
            ROOT_ID,
            &local_groups,
            &local_connections,
            &export,
            &local_vars,
        );

        let result2 = GroupMergeEngine::merge(
            ROOT_ID,
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
            ROOT_ID,
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

// ---------------------------------------------------------------------------
// Matching inside the synced group (0.22.13)
// ---------------------------------------------------------------------------

/// File name both sides use for the export in the root-name property.
const SYNC_FILE: &str = "master.rcn";

/// Root group names, drawn independently for the Master and the Import side.
/// A `/` is allowed on purpose: a root's path is removed as a whole string, so
/// a slash inside a root's name must not shift the split.
const ROOT_NAME: &str = "[A-Za-z0-9][A-Za-z0-9 ._/-]{0,15}";

/// The shape of one generated tree, built once per side so the two sides are
/// equivalent. `subgroups[i]` is `(parent, name)`, where `parent` is `None`
/// for the root or the index of an earlier subgroup; each connection is
/// `(group, name, age_hours)`, where `group` is `None` for the root.
#[derive(Debug, Clone)]
struct TreeShape {
    subgroups: Vec<(Option<usize>, String)>,
    connections: Vec<(Option<usize>, String, i64)>,
}

/// Generates up to six subgroups nested to any depth, and up to ten
/// connections over the root and those subgroups, unique by name per group.
fn arb_tree_shape() -> impl Strategy<Value = TreeShape> {
    (
        prop::collection::vec((any::<bool>(), 0usize..64), 0..=6),
        prop::collection::vec((0usize..64, 0usize..8, 0i64..100), 0..=10),
    )
        .prop_map(|(group_specs, conn_specs)| {
            let subgroups: Vec<(Option<usize>, String)> = group_specs
                .iter()
                .enumerate()
                .map(|(i, &(under_root, pick))| {
                    let parent = if under_root || i == 0 {
                        None
                    } else {
                        Some(pick % i)
                    };
                    (parent, format!("sub-{i}"))
                })
                .collect();

            // Slot 0 is the root, slot `k + 1` is subgroup `k`.
            let mut seen = HashSet::new();
            let connections: Vec<(Option<usize>, String, i64)> = conn_specs
                .iter()
                .filter_map(|&(slot, name_index, age)| {
                    let group = (slot % (subgroups.len() + 1)).checked_sub(1);
                    let name = format!("conn-{name_index}");
                    let fresh = seen.insert((group, name.clone()));
                    fresh.then_some((group, name, age))
                })
                .collect();

            TreeShape {
                subgroups,
                connections,
            }
        })
}

/// Builds `shape` under `root` with fresh ids, returning the groups (root
/// first) and the connections. Both sides are built from the same `base`, so
/// they carry the same timestamps and no update is due.
fn build_tree(
    root: ConnectionGroup,
    shape: &TreeShape,
    base: chrono::DateTime<chrono::Utc>,
) -> (Vec<ConnectionGroup>, Vec<Connection>) {
    let root_id = root.id;
    let mut groups = vec![root];
    let mut subgroup_ids: Vec<Uuid> = Vec::with_capacity(shape.subgroups.len());
    for (parent, name) in &shape.subgroups {
        let parent_id = parent.map_or(root_id, |p| subgroup_ids[p]);
        let group = ConnectionGroup::with_parent(name.clone(), parent_id);
        subgroup_ids.push(group.id);
        groups.push(group);
    }

    let connections: Vec<Connection> = shape
        .connections
        .iter()
        .map(|(group, name, age)| {
            let group_id = group.map_or(root_id, |g| subgroup_ids[g]);
            let mut conn = Connection::new_ssh(name.clone(), "10.0.0.1".to_owned(), 22);
            conn.group_id = Some(group_id);
            conn.updated_at = base + Duration::hours(*age);
            conn
        })
        .collect();

    (groups, connections)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Re-importing what is already there changes nothing, whatever either
    /// root is called and wherever the Import root sits.
    ///
    /// Paths are compared inside the synced group, so neither the Master's
    /// root name nor the Import root's own path takes part. Up to 0.22.12 the
    /// keys carried both, and any difference between them — the Settings
    /// "Import" button names the Import group after the file — turned every
    /// sync into deleting and recreating the whole tree. The export goes
    /// through the real writer and reader.
    #[test]
    fn equivalent_tree_merges_to_nothing_for_any_root_names(
        shape in arb_tree_shape(),
        master_name in ROOT_NAME,
        import_name in ROOT_NAME,
        nested in any::<bool>(),
    ) {
        let dir = TempDir::new().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let settings = SyncSettings {
            sync_dir: Some(dir.path().to_owned()),
            ..SyncSettings::default()
        };
        let base = Utc::now() - Duration::hours(200);

        // Master: the tree under `master_name`, written by the real exporter.
        let mut master_root = ConnectionGroup::new(master_name);
        master_root.sync_mode = SyncMode::Master;
        master_root.sync_file = Some(SYNC_FILE.to_owned());
        let master_root_id = master_root.id;
        let (master_groups, master_connections) = build_tree(master_root, &shape, base);
        let mut master = SyncManager::new(settings.clone());
        master
            .export_group(master_root_id, &master_groups, &master_connections, &[], "0.22.13")
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        let export = GroupSyncExport::from_file(&dir.path().join(SYNC_FILE))
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(export.sync_version, 1);
        prop_assert!(export.connections.iter().all(|c| c.id.is_some()));

        // Import: the same tree under `import_name`, optionally inside "Work".
        let work = ConnectionGroup::new("Work".to_owned());
        let mut import_root = if nested {
            ConnectionGroup::with_parent(import_name, work.id)
        } else {
            ConnectionGroup::new(import_name)
        };
        import_root.sync_mode = SyncMode::Import;
        import_root.sync_file = Some(SYNC_FILE.to_owned());
        let import_root_id = import_root.id;
        let (mut local_groups, local_connections) = build_tree(import_root, &shape, base);
        if nested {
            local_groups.push(work);
        }
        let no_variables = HashSet::new();

        // With every local group in the slice, the root's parent included...
        let direct = GroupMergeEngine::merge(
            import_root_id,
            &local_groups,
            &local_connections,
            &export,
            &no_variables,
        );
        prop_assert!(direct.connections_to_create.is_empty());
        prop_assert!(direct.connections_to_update.is_empty());
        prop_assert!(direct.connections_to_delete.is_empty());
        prop_assert!(direct.groups_to_create.is_empty());
        prop_assert!(direct.groups_to_update.is_empty());
        prop_assert!(direct.groups_to_delete.is_empty());
        prop_assert!(direct.variables_to_create.is_empty());

        // ...and through `SyncManager`, which hands the engine the subtree only.
        let mut importer = SyncManager::new(settings);
        let (via_manager, report) = importer
            .import_group(import_root_id, &local_groups, &local_connections, &no_variables)
            .map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert!(via_manager.connections_to_create.is_empty());
        prop_assert!(via_manager.connections_to_update.is_empty());
        prop_assert!(via_manager.connections_to_delete.is_empty());
        prop_assert!(via_manager.groups_to_create.is_empty());
        prop_assert!(via_manager.groups_to_update.is_empty());
        prop_assert!(via_manager.groups_to_delete.is_empty());
        prop_assert!(via_manager.variables_to_create.is_empty());
        prop_assert_eq!(report.group_id, import_root_id);
    }
}
