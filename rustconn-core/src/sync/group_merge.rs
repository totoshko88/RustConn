//! Group Sync merge engine for Import mode.
//!
//! [`GroupMergeEngine`] computes a diff between the local group tree and a
//! remote [`GroupSyncExport`], producing a [`GroupMergeResult`] that describes
//! which connections, groups, and variable templates need to be created,
//! updated, or deleted locally.
//!
//! The merge algorithm uses **name + group path** as the primary key for
//! connections and **path** as the primary key for groups. Every path is taken
//! *inside the synced group*: the Master's root name is removed from the front
//! of each exported path and the Import root's own path from each local one, so
//! the root is `""` on both sides. Neither root's name, nor where the Import
//! root sits in the local tree, takes part in matching. Conflict resolution is
//! timestamp-based: if both sides have a connection under the same key, the
//! one with the newer `updated_at` wins.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use super::group_export::{GroupSyncExport, SyncConnection, SyncGroup, compute_group_path};
use super::variable_template::VariableTemplate;
use crate::models::{Connection, ConnectionGroup, collect_descendant_group_ids};

/// Name-based merge engine for Group Sync Import mode.
///
/// Stateless — all inputs are passed to [`merge()`](Self::merge).
pub struct GroupMergeEngine;

/// Result of a group merge operation.
///
/// Each field describes a set of changes that the caller (typically
/// [`SyncManager`](super::manager::SyncManager)) should apply to the local
/// [`ConnectionManager`](crate::connection_manager::ConnectionManager).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroupMergeResult {
    /// Remote connections not present locally — should be created.
    pub connections_to_create: Vec<SyncConnection>,
    /// Local connections that exist remotely with a newer `updated_at` —
    /// the tuple is `(local_connection_id, remote_data)`.
    pub connections_to_update: Vec<(Uuid, SyncConnection)>,
    /// Local connections not present in the remote export — should be deleted.
    pub connections_to_delete: Vec<Uuid>,
    /// Remote groups (by path inside the synced group) not present locally —
    /// should be created. Each keeps the path the export wrote.
    pub groups_to_create: Vec<SyncGroup>,
    /// Local groups matched to a remote group by **id** whose name or path
    /// changed on the Master — should be renamed/reparented in place rather
    /// than deleted and recreated. The tuple is `(local_group_id, remote_data)`.
    /// Empty until id-based group matching is wired (populated in a later step);
    /// name/path-only matching never produces entries here.
    pub groups_to_update: Vec<(Uuid, SyncGroup)>,
    /// Local groups (by path inside the synced group) not present in the
    /// remote export — should be deleted. Never the Import root itself.
    pub groups_to_delete: Vec<Uuid>,
    /// Remote variable templates not present locally — should be created.
    pub variables_to_create: Vec<VariableTemplate>,
}

/// Composite key for connection lookup: `(name, group path inside the synced
/// group)`.
type ConnectionKey<'a> = (&'a str, &'a str);

impl GroupMergeEngine {
    /// Computes the diff between the Import tree under `root_id` and a remote
    /// [`GroupSyncExport`].
    ///
    /// Keys are taken inside the synced group (see the module docs), so the
    /// Import root can carry any name — the Settings "Import" button names it
    /// after the file, not after the Master's group — and can sit anywhere in
    /// the local tree. When both roots have the same name the pairing is the
    /// one 0.22.12 and earlier made, minus one common prefix; when the names
    /// differ, those versions paired nothing and recreated everything.
    ///
    /// # Algorithm
    ///
    /// 1. **Phase 1 — Groups by path**: remote paths not in local → create;
    ///    local paths not in remote → delete. The Import root stands for the
    ///    export as a whole and is never a candidate.
    /// 2. **Phase 2 — Connections by (name, group path)**: remote not in local
    ///    → create; local not in remote → delete; both exist and
    ///    `remote.updated_at > local.updated_at` → update.
    /// 3. **Phase 3 — Variable templates**: remote templates whose name is not
    ///    found among `local_variable_names` → create.
    ///
    /// Groups and connections outside the root's subtree are ignored: they are
    /// not the Import group's to match, update or delete.
    ///
    /// # Arguments
    ///
    /// * `root_id` — the local Import root group.
    /// * `local_groups` — local groups including the Import root and its
    ///   subtree; any others in the slice, such as the root's parent, are
    ///   ignored.
    /// * `local_connections` — local connections belonging to those groups.
    /// * `remote` — the parsed remote export file.
    /// * `local_variable_names` — names of variables that already exist locally.
    #[must_use]
    pub fn merge(
        root_id: Uuid,
        local_groups: &[ConnectionGroup],
        local_connections: &[Connection],
        remote: &GroupSyncExport,
        local_variable_names: &HashSet<String>,
    ) -> GroupMergeResult {
        let mut result = GroupMergeResult::default();

        let local_paths = local_relative_paths(root_id, local_groups);
        let remote_root = remote.root_group.name.as_str();

        // --- Phase 1: Merge groups by path ---
        Self::merge_groups(root_id, &local_paths, remote, remote_root, &mut result);

        // --- Phase 2: Merge connections by (name, group path) ---
        Self::merge_connections(
            &local_paths,
            local_connections,
            remote,
            remote_root,
            &mut result,
        );

        // --- Phase 3: Variable templates ---
        Self::merge_variables(
            &remote.variable_templates,
            local_variable_names,
            &mut result,
        );

        result
    }

    /// Phase 1: diff subgroups, matching by **id** first and falling back to
    /// path for groups without an id.
    ///
    /// Like connections, `SyncGroup` has carried a stable id since 0.22.13.
    /// Matching on it first means a subgroup **renamed or moved** on the Master
    /// is recognised as the same group and renamed/reparented in place
    /// (`groups_to_update`) instead of being deleted and recreated — which lost
    /// the group's id and every connection filed under it. `SyncGroup` carries
    /// no `updated_at`, so for a group the Master is authoritative: an id match
    /// whose path differs is always taken as a Master-side change. The path
    /// fallback preserves pre-0.22.13 (`id: None`) behaviour exactly.
    fn merge_groups(
        root_id: Uuid,
        local_paths: &HashMap<Uuid, String>,
        remote: &GroupSyncExport,
        remote_root: &str,
        result: &mut GroupMergeResult,
    ) {
        // Local subgroups keyed by path. The root is left out by id, not by
        // `parent_id`: an Import root nested under another local group has a
        // parent, and until 0.22.13 that put it on the delete list of its own
        // sync.
        let local_path_map: HashMap<&str, Uuid> = local_paths
            .iter()
            .filter(|&(id, _)| *id != root_id)
            .map(|(id, path)| (path.as_str(), *id))
            .collect();
        // The in-scope local group ids (same exclusion of the root), for id
        // matching.
        let local_ids: HashSet<Uuid> = local_paths
            .keys()
            .copied()
            .filter(|id| *id != root_id)
            .collect();

        let mut consumed_local: HashSet<Uuid> = HashSet::new();
        let mut consumed_remote_ids: HashSet<Uuid> = HashSet::new();

        // --- Pass A: match by id ---
        for remote_group in &remote.groups {
            let Some(remote_id) = remote_group.id else {
                continue; // legacy export without an id → handled in pass B
            };
            if local_ids.contains(&remote_id) {
                consumed_remote_ids.insert(remote_id);
                consumed_local.insert(remote_id);
                // Master authoritative: if the path changed (rename/move),
                // update in place rather than delete + recreate.
                let local_path = local_paths.get(&remote_id).map(String::as_str);
                if local_path != Some(relative_path(&remote_group.path, remote_root)) {
                    result
                        .groups_to_update
                        .push((remote_id, remote_group.clone()));
                }
            }
        }

        // --- Pass B: match the remainder by path ---
        let remote_paths: HashSet<&str> = remote
            .groups
            .iter()
            .filter(|g| g.id.is_none_or(|id| !consumed_remote_ids.contains(&id)))
            .map(|g| relative_path(&g.path, remote_root))
            .collect();

        // New remote paths (not id-consumed, not path-present locally) → create.
        for remote_group in &remote.groups {
            if remote_group
                .id
                .is_some_and(|id| consumed_remote_ids.contains(&id))
            {
                continue;
            }
            if !local_path_map.contains_key(relative_path(&remote_group.path, remote_root)) {
                result.groups_to_create.push(remote_group.clone());
            }
        }

        // Local paths absent from the remote → delete, unless the group was
        // already matched by id (a rename/move, handled as an update).
        for (path, group_id) in &local_path_map {
            if consumed_local.contains(group_id) {
                continue;
            }
            if !remote_paths.contains(path) {
                result.groups_to_delete.push(*group_id);
            }
        }
    }

    /// Phase 2: diff connections, matching by **id** first and falling back to
    /// `(name, group path inside the synced group)` for entries without an id.
    ///
    /// Exports written since 0.22.13 carry `SyncConnection::id`. Matching on it
    /// first means a connection **renamed or moved** on the Master is recognised
    /// as the same entity and emitted as an *update* (carrying the new name and
    /// group path) instead of a delete + create — which churned the local id and
    /// broke the vault credential link (issue #263). The name/path fallback keeps
    /// pre-0.22.13 exports (`id: None`) and genuinely new connections behaving
    /// exactly as before.
    fn merge_connections(
        local_paths: &HashMap<Uuid, String>,
        local_connections: &[Connection],
        remote: &GroupSyncExport,
        remote_root: &str,
        result: &mut GroupMergeResult,
    ) {
        // Only connections inside the Import root's subtree are eligible: one
        // outside it gets no path here, so it is never matched, updated or
        // deleted by this sync (preserved from the name-only implementation).
        let local_in_scope: Vec<&Connection> = local_connections
            .iter()
            .filter(|c| c.group_id.is_some_and(|g| local_paths.contains_key(&g)))
            .collect();

        // Consumed sets so neither pass double-counts an entity matched by the
        // other: a remote id-matched here must not also be created, and a local
        // id-matched here must not also be deleted.
        let mut consumed_local: HashSet<Uuid> = HashSet::new();
        let mut consumed_remote: HashSet<Uuid> = HashSet::new();

        // --- Pass A: match by id ---
        let local_by_id: HashMap<Uuid, &Connection> =
            local_in_scope.iter().map(|c| (c.id, *c)).collect();
        for remote_conn in &remote.connections {
            let Some(remote_id) = remote_conn.id else {
                continue; // legacy export without an id → handled in pass B
            };
            if let Some(local_conn) = local_by_id.get(&remote_id) {
                consumed_remote.insert(remote_id);
                consumed_local.insert(local_conn.id);
                // Update on a newer remote. A rename/move keeps the same id but
                // a different name/group_path, so it lands here (not create).
                if remote_conn.updated_at > local_conn.updated_at {
                    result
                        .connections_to_update
                        .push((local_conn.id, remote_conn.clone()));
                }
            }
        }

        // --- Pass B: match the remainder by (name, group path) ---
        let remote_by_key: HashMap<ConnectionKey<'_>, &SyncConnection> = remote
            .connections
            .iter()
            .filter(|c| c.id.is_none_or(|id| !consumed_remote.contains(&id)))
            .map(|c| {
                let key = (c.name.as_str(), relative_path(&c.group_path, remote_root));
                (key, c)
            })
            .collect();

        let local_by_key: HashMap<ConnectionKey<'_>, &Connection> = local_in_scope
            .iter()
            .filter(|c| !consumed_local.contains(&c.id))
            .filter_map(|c| {
                let path = local_paths.get(&c.group_id?)?;
                Some(((c.name.as_str(), path.as_str()), *c))
            })
            .collect();

        // Remote not matched locally → create; matched with newer remote → update.
        for (key, remote_conn) in &remote_by_key {
            if let Some(local_conn) = local_by_key.get(key) {
                if remote_conn.updated_at > local_conn.updated_at {
                    result
                        .connections_to_update
                        .push((local_conn.id, (*remote_conn).clone()));
                }
            } else {
                result.connections_to_create.push((*remote_conn).clone());
            }
        }

        // Local not matched by id (pass A) and not matched by name (pass B)
        // → delete. An id-consumed local is skipped by the pass-B filter above,
        // so it can never be deleted here.
        for (key, local_conn) in &local_by_key {
            if !remote_by_key.contains_key(key) {
                result.connections_to_delete.push(local_conn.id);
            }
        }
    }

    /// Phase 3: collect variable templates not present locally.
    fn merge_variables(
        remote_templates: &[VariableTemplate],
        local_variable_names: &HashSet<String>,
        result: &mut GroupMergeResult,
    ) {
        for template in remote_templates {
            if !local_variable_names.contains(&template.name) {
                result.variables_to_create.push(template.clone());
            }
        }
    }
}

/// Paths of the Import root and its subtree as seen from inside the root,
/// keyed by group id; the root itself maps to `""`.
///
/// Only the root and its descendants are listed. A group above or beside the
/// root in `local_groups` is not part of the Import tree, so it must never be
/// matched — and above all never deleted.
fn local_relative_paths(root_id: Uuid, local_groups: &[ConnectionGroup]) -> HashMap<Uuid, String> {
    // The root's own path includes whatever ancestors `local_groups` holds,
    // e.g. "Work/production-servers"; `compute_group_path` puts the same
    // prefix in front of every descendant, so removing it leaves the part
    // inside the root.
    let root_path = compute_group_path(root_id, local_groups);
    collect_descendant_group_ids(root_id, local_groups)
        .into_iter()
        .map(|id| {
            let path = compute_group_path(id, local_groups);
            let relative = relative_path(&path, &root_path).to_owned();
            (id, relative)
        })
        .collect()
}

/// Returns `path` as seen from inside `root`: `""` for the root itself, the
/// part after `root/` for anything below it, and `path` unchanged otherwise.
///
/// The root's path is removed as a whole string rather than segment by
/// segment, so a `/` inside a group name cannot shift the split.
fn relative_path<'a>(path: &'a str, root: &str) -> &'a str {
    if path == root {
        return "";
    }
    path.strip_prefix(root)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration, Utc};

    use super::*;
    use crate::models::{
        AutomationConfig, PasswordSource, ProtocolConfig, ProtocolType, SshConfig,
    };

    /// Helper: create a minimal `SyncConnection`.
    fn make_sync_conn(name: &str, group_path: &str) -> SyncConnection {
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
            updated_at: Utc::now(),
        }
    }

    /// Helper: create a minimal `SyncGroup`.
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

    /// Helper: create a minimal `GroupSyncExport`.
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
            master_device_id: uuid::Uuid::new_v4(),
            master_device_name: "test-device".to_owned(),
            root_group: make_sync_group("Root", "Root"),
            groups,
            connections,
            variable_templates,
        }
    }

    /// Helper: create a local `ConnectionGroup`.
    fn make_local_group(name: &str, parent_id: Option<Uuid>) -> ConnectionGroup {
        if let Some(pid) = parent_id {
            ConnectionGroup::with_parent(name.to_owned(), pid)
        } else {
            ConnectionGroup::new(name.to_owned())
        }
    }

    /// Helper: create a local `Connection` in a group.
    fn make_local_conn(name: &str, group_id: Uuid) -> Connection {
        let mut c = Connection::new_ssh(name.to_owned(), "10.0.0.1".to_owned(), 22);
        c.group_id = Some(group_id);
        c
    }

    // ---------------------------------------------------------------
    // Phase 1: Group merge tests
    // ---------------------------------------------------------------

    #[test]
    fn empty_inputs_produce_empty_result() {
        let result = GroupMergeEngine::merge(
            Uuid::new_v4(),
            &[],
            &[],
            &make_export(vec![], vec![], vec![]),
            &HashSet::new(),
        );
        assert_eq!(result, GroupMergeResult::default());
    }

    #[test]
    fn new_remote_group_is_created() {
        let remote_group = make_sync_group("Web", "Root/Web");
        let export = make_export(vec![remote_group], vec![], vec![]);

        let root = make_local_group("Root", None);
        let result = GroupMergeEngine::merge(root.id, &[root], &[], &export, &HashSet::new());

        assert_eq!(result.groups_to_create.len(), 1);
        assert_eq!(result.groups_to_create[0].path, "Root/Web");
        assert!(result.groups_to_delete.is_empty());
    }

    #[test]
    fn missing_remote_group_is_deleted() {
        let root = make_local_group("Root", None);
        let child = make_local_group("OldGroup", Some(root.id));
        let export = make_export(vec![], vec![], vec![]);

        let result = GroupMergeEngine::merge(
            root.id,
            &[root, child.clone()],
            &[],
            &export,
            &HashSet::new(),
        );

        assert!(result.groups_to_create.is_empty());
        assert_eq!(result.groups_to_delete.len(), 1);
        assert_eq!(result.groups_to_delete[0], child.id);
    }

    #[test]
    fn matching_group_paths_are_unchanged() {
        let root = make_local_group("Root", None);
        let child = make_local_group("Web", Some(root.id));
        let remote_group = make_sync_group("Web", "Root/Web");
        let export = make_export(vec![remote_group], vec![], vec![]);

        let result =
            GroupMergeEngine::merge(root.id, &[root, child], &[], &export, &HashSet::new());

        assert!(result.groups_to_create.is_empty());
        assert!(result.groups_to_delete.is_empty());
    }

    // ---------------------------------------------------------------
    // Phase 2: Connection merge tests
    // ---------------------------------------------------------------

    #[test]
    fn new_remote_connection_is_created() {
        let root = make_local_group("Root", None);
        let remote_conn = make_sync_conn("nginx-1", "Root");
        let export = make_export(vec![], vec![remote_conn], vec![]);

        let result = GroupMergeEngine::merge(root.id, &[root], &[], &export, &HashSet::new());

        assert_eq!(result.connections_to_create.len(), 1);
        assert_eq!(result.connections_to_create[0].name, "nginx-1");
    }

    #[test]
    fn missing_remote_connection_is_deleted() {
        let root = make_local_group("Root", None);
        let local_conn = make_local_conn("old-server", root.id);
        let export = make_export(vec![], vec![], vec![]);

        let result = GroupMergeEngine::merge(
            root.id,
            &[root],
            std::slice::from_ref(&local_conn),
            &export,
            &HashSet::new(),
        );

        assert_eq!(result.connections_to_delete.len(), 1);
        assert_eq!(result.connections_to_delete[0], local_conn.id);
    }

    #[test]
    fn newer_remote_connection_triggers_update() {
        let root = make_local_group("Root", None);
        let mut local_conn = make_local_conn("nginx-1", root.id);
        local_conn.updated_at = Utc::now() - Duration::hours(1);

        let mut remote_conn = make_sync_conn("nginx-1", "Root");
        remote_conn.updated_at = Utc::now();

        let export = make_export(vec![], vec![remote_conn], vec![]);
        let result = GroupMergeEngine::merge(
            root.id,
            &[root],
            std::slice::from_ref(&local_conn),
            &export,
            &HashSet::new(),
        );

        assert_eq!(result.connections_to_update.len(), 1);
        assert_eq!(result.connections_to_update[0].0, local_conn.id);
    }

    #[test]
    fn older_remote_connection_is_unchanged() {
        let root = make_local_group("Root", None);
        let mut local_conn = make_local_conn("nginx-1", root.id);
        local_conn.updated_at = Utc::now();

        let mut remote_conn = make_sync_conn("nginx-1", "Root");
        remote_conn.updated_at = Utc::now() - Duration::hours(1);

        let export = make_export(vec![], vec![remote_conn], vec![]);
        let result =
            GroupMergeEngine::merge(root.id, &[root], &[local_conn], &export, &HashSet::new());

        assert!(result.connections_to_update.is_empty());
        assert!(result.connections_to_create.is_empty());
        assert!(result.connections_to_delete.is_empty());
    }

    #[test]
    fn same_timestamp_connection_is_unchanged() {
        let root = make_local_group("Root", None);
        let ts = Utc::now();
        let mut local_conn = make_local_conn("nginx-1", root.id);
        local_conn.updated_at = ts;

        let mut remote_conn = make_sync_conn("nginx-1", "Root");
        remote_conn.updated_at = ts;

        let export = make_export(vec![], vec![remote_conn], vec![]);
        let result =
            GroupMergeEngine::merge(root.id, &[root], &[local_conn], &export, &HashSet::new());

        assert!(result.connections_to_update.is_empty());
        assert!(result.connections_to_create.is_empty());
        assert!(result.connections_to_delete.is_empty());
    }

    // ---------------------------------------------------------------
    // Phase 2: id-based matching (SYNC-1)
    // ---------------------------------------------------------------

    #[test]
    fn renamed_connection_matched_by_id_is_updated_not_recreated() {
        // Same id, different name (renamed on the Master). Must be an update
        // carrying the new name — NOT delete(old) + create(new), which would
        // churn the local id and break the vault link (issue #263).
        let root = make_local_group("Root", None);
        let mut local_conn = make_local_conn("old-name", root.id);
        local_conn.updated_at = Utc::now() - Duration::hours(1);

        let mut remote_conn = make_sync_conn("new-name", "Root");
        remote_conn.id = Some(local_conn.id);
        remote_conn.updated_at = Utc::now();

        let export = make_export(vec![], vec![remote_conn], vec![]);
        let result = GroupMergeEngine::merge(
            root.id,
            &[root],
            std::slice::from_ref(&local_conn),
            &export,
            &HashSet::new(),
        );

        assert_eq!(result.connections_to_update.len(), 1, "expected one update");
        assert_eq!(result.connections_to_update[0].0, local_conn.id);
        assert_eq!(result.connections_to_update[0].1.name, "new-name");
        assert!(
            result.connections_to_create.is_empty(),
            "a rename must not create"
        );
        assert!(
            result.connections_to_delete.is_empty(),
            "a rename must not delete"
        );
    }

    #[test]
    fn moved_connection_matched_by_id_is_updated_not_recreated() {
        // Same id, same name, different group path (moved to another subgroup
        // on the Master). The id match keeps it a single update.
        let root = make_local_group("Root", None);
        let web = make_local_group("Web", Some(root.id));
        let db = make_local_group("DB", Some(root.id));
        let mut local_conn = make_local_conn("server-1", web.id);
        local_conn.updated_at = Utc::now() - Duration::hours(1);

        let mut remote_conn = make_sync_conn("server-1", "Root/DB");
        remote_conn.id = Some(local_conn.id);
        remote_conn.updated_at = Utc::now();

        let export = make_export(
            vec![
                make_sync_group("Web", "Root/Web"),
                make_sync_group("DB", "Root/DB"),
            ],
            vec![remote_conn],
            vec![],
        );
        let result = GroupMergeEngine::merge(
            root.id,
            &[root, web, db],
            std::slice::from_ref(&local_conn),
            &export,
            &HashSet::new(),
        );

        assert_eq!(result.connections_to_update.len(), 1);
        assert_eq!(result.connections_to_update[0].0, local_conn.id);
        assert_eq!(result.connections_to_update[0].1.group_path, "Root/DB");
        assert!(result.connections_to_create.is_empty());
        assert!(result.connections_to_delete.is_empty());
    }

    #[test]
    fn two_connections_same_name_different_ids_stay_distinct() {
        // Legal now that id is identity: two connections with the same name in
        // the same path but different ids. Each matches its own id; the old
        // name-keyed map would have collapsed them into one.
        let root = make_local_group("Root", None);
        let mut a = make_local_conn("dup", root.id);
        let mut b = make_local_conn("dup", root.id);
        let past = Utc::now() - Duration::hours(1);
        a.updated_at = past;
        b.updated_at = past;

        let now = Utc::now();
        let mut remote_a = make_sync_conn("dup", "Root");
        remote_a.id = Some(a.id);
        remote_a.updated_at = now;
        let mut remote_b = make_sync_conn("dup", "Root");
        remote_b.id = Some(b.id);
        remote_b.updated_at = now;

        let export = make_export(vec![], vec![remote_a, remote_b], vec![]);
        let result = GroupMergeEngine::merge(
            root.id,
            &[root],
            &[a.clone(), b.clone()],
            &export,
            &HashSet::new(),
        );

        // Both updated, by their own id; nothing created or deleted.
        assert_eq!(result.connections_to_update.len(), 2);
        let updated_ids: HashSet<Uuid> = result
            .connections_to_update
            .iter()
            .map(|(id, _)| *id)
            .collect();
        assert!(updated_ids.contains(&a.id));
        assert!(updated_ids.contains(&b.id));
        assert!(result.connections_to_create.is_empty());
        assert!(result.connections_to_delete.is_empty());
    }

    #[test]
    fn remote_id_matching_a_connection_outside_the_subtree_is_ignored() {
        // A local connection outside the Import root's subtree must never be
        // pulled in, even when a remote entry carries its id.
        let root = make_local_group("Root", None);
        let outside_group = make_local_group("Elsewhere", None);
        let mut outside_conn = make_local_conn("secret", outside_group.id);
        outside_conn.updated_at = Utc::now() - Duration::hours(1);

        let mut remote_conn = make_sync_conn("secret", "Root");
        remote_conn.id = Some(outside_conn.id);
        remote_conn.updated_at = Utc::now();

        let export = make_export(vec![], vec![remote_conn], vec![]);
        // Only `root` is in the synced subtree; `outside_group` is not passed as
        // part of the Import root's groups.
        let result = GroupMergeEngine::merge(
            root.id,
            &[root],
            std::slice::from_ref(&outside_conn),
            &export,
            &HashSet::new(),
        );

        // The out-of-scope local is untouched; the remote is treated as new.
        assert!(
            result.connections_to_update.is_empty(),
            "out-of-subtree local must not be updated"
        );
        assert!(
            result.connections_to_delete.is_empty(),
            "out-of-subtree local must not be deleted"
        );
        assert_eq!(
            result.connections_to_create.len(),
            1,
            "remote with no in-scope match is created"
        );
    }

    // ---------------------------------------------------------------
    // Phase 1: id-based group matching (SYNC-1)
    // ---------------------------------------------------------------

    #[test]
    fn renamed_group_matched_by_id_is_updated_not_recreated() {
        // Local "Web" subgroup; the Master renamed it to "Prod" (same id). With
        // id matching this is a single update, not delete("Web") + create("Prod")
        // which would have orphaned every connection filed under it.
        let root = make_local_group("Root", None);
        let web = make_local_group("Web", Some(root.id));

        let mut remote_group = make_sync_group("Prod", "Root/Prod");
        remote_group.id = Some(web.id);

        let export = make_export(vec![remote_group], vec![], vec![]);
        let result =
            GroupMergeEngine::merge(root.id, &[root, web.clone()], &[], &export, &HashSet::new());

        assert_eq!(
            result.groups_to_update.len(),
            1,
            "expected one group update"
        );
        assert_eq!(result.groups_to_update[0].0, web.id);
        assert_eq!(result.groups_to_update[0].1.name, "Prod");
        assert!(
            result.groups_to_create.is_empty(),
            "a group rename must not create"
        );
        assert!(
            result.groups_to_delete.is_empty(),
            "a group rename must not delete"
        );
    }

    #[test]
    fn renamed_group_without_id_falls_back_to_path_create_delete() {
        // Pre-0.22.13 export (id: None): no id to match on, so the renamed
        // subgroup still looks like a delete + create by path. This preserves
        // legacy behaviour exactly.
        let root = make_local_group("Root", None);
        let web = make_local_group("Web", Some(root.id));

        let remote_group = make_sync_group("Prod", "Root/Prod"); // id = None
        let export = make_export(vec![remote_group], vec![], vec![]);
        let result =
            GroupMergeEngine::merge(root.id, &[root, web.clone()], &[], &export, &HashSet::new());

        assert!(
            result.groups_to_update.is_empty(),
            "no id → no in-place update"
        );
        assert_eq!(result.groups_to_create.len(), 1);
        assert_eq!(result.groups_to_create[0].path, "Root/Prod");
        assert_eq!(result.groups_to_delete, vec![web.id]);
    }

    // ---------------------------------------------------------------
    // Phase 3: Variable template tests
    // ---------------------------------------------------------------

    #[test]
    fn new_variable_template_is_created() {
        let template = VariableTemplate {
            name: "web_key".to_owned(),
            description: Some("SSH key".to_owned()),
            is_secret: true,
            default_value: None,
        };
        let export = make_export(vec![], vec![], vec![template]);

        let result = GroupMergeEngine::merge(Uuid::new_v4(), &[], &[], &export, &HashSet::new());

        assert_eq!(result.variables_to_create.len(), 1);
        assert_eq!(result.variables_to_create[0].name, "web_key");
    }

    #[test]
    fn existing_variable_template_is_skipped() {
        let template = VariableTemplate {
            name: "web_key".to_owned(),
            description: None,
            is_secret: true,
            default_value: None,
        };
        let export = make_export(vec![], vec![], vec![template]);

        let local_vars: HashSet<String> = std::iter::once("web_key".to_owned()).collect();
        let result = GroupMergeEngine::merge(Uuid::new_v4(), &[], &[], &export, &local_vars);

        assert!(result.variables_to_create.is_empty());
    }

    // ---------------------------------------------------------------
    // Combined scenario
    // ---------------------------------------------------------------

    #[test]
    fn full_merge_scenario() {
        // Local: Root group with "Web" subgroup, connections "nginx-1" and "old-server"
        let root = make_local_group("Root", None);
        let web = make_local_group("Web", Some(root.id));

        let mut nginx = make_local_conn("nginx-1", web.id);
        nginx.updated_at = Utc::now() - Duration::hours(2);

        let old_server = make_local_conn("old-server", web.id);

        // Remote: Root with "Web" and new "DB" subgroup,
        // connections "nginx-1" (updated) and "new-server"
        let remote_web = make_sync_group("Web", "Root/Web");
        let remote_db = make_sync_group("DB", "Root/DB");

        let mut remote_nginx = make_sync_conn("nginx-1", "Root/Web");
        remote_nginx.updated_at = Utc::now();
        remote_nginx.host = "10.0.0.99".to_owned();

        let remote_new = make_sync_conn("new-server", "Root/Web");

        let template = VariableTemplate {
            name: "db_pass".to_owned(),
            description: None,
            is_secret: true,
            default_value: None,
        };

        let export = make_export(
            vec![remote_web, remote_db],
            vec![remote_nginx, remote_new],
            vec![template],
        );

        let result = GroupMergeEngine::merge(
            root.id,
            &[root, web],
            &[nginx.clone(), old_server.clone()],
            &export,
            &HashSet::new(),
        );

        // DB group is new
        assert_eq!(result.groups_to_create.len(), 1);
        assert_eq!(result.groups_to_create[0].path, "Root/DB");

        // No groups deleted (Web still exists remotely)
        assert!(result.groups_to_delete.is_empty());

        // "new-server" is created
        assert_eq!(result.connections_to_create.len(), 1);
        assert_eq!(result.connections_to_create[0].name, "new-server");

        // "nginx-1" is updated (remote is newer)
        assert_eq!(result.connections_to_update.len(), 1);
        assert_eq!(result.connections_to_update[0].0, nginx.id);

        // "old-server" is deleted (not in remote)
        assert_eq!(result.connections_to_delete.len(), 1);
        assert_eq!(result.connections_to_delete[0], old_server.id);

        // Variable template created
        assert_eq!(result.variables_to_create.len(), 1);
        assert_eq!(result.variables_to_create[0].name, "db_pass");
    }

    // ---------------------------------------------------------------
    // Matching inside the synced group (0.22.13)
    //
    // Up to 0.22.12 every key carried its own root's name, so these
    // scenarios reported churn on every sync: with the Import root named
    // "production-servers" against the Master's "Production Servers",
    // `connections_to_create` held "bastion" and "nginx-1",
    // `connections_to_delete` both local copies, and "Web" was in both
    // `groups_to_create` and `groups_to_delete`. A nested Import root was
    // also in its own `groups_to_delete`, whatever its name.
    // ---------------------------------------------------------------

    /// The Master's tree as the exporter writes it: every path starts with
    /// the Master root's name, "Production Servers".
    fn production_servers_export(ts: DateTime<Utc>) -> GroupSyncExport {
        let mut bastion = make_sync_conn("bastion", "Production Servers");
        bastion.updated_at = ts;
        let mut nginx = make_sync_conn("nginx-1", "Production Servers/Web");
        nginx.updated_at = ts;

        let mut export = make_export(
            vec![make_sync_group("Web", "Production Servers/Web")],
            vec![bastion, nginx],
            vec![],
        );
        export.root_group = make_sync_group("Production Servers", "Production Servers");
        export
    }

    /// The local copy of [`production_servers_export`] under `root`: "bastion"
    /// in the root, "nginx-1" in a "Web" subgroup. Returns the subgroup and
    /// the connections.
    fn mirror_under(
        root: &ConnectionGroup,
        ts: DateTime<Utc>,
    ) -> (ConnectionGroup, Vec<Connection>) {
        let web = make_local_group("Web", Some(root.id));
        let mut bastion = make_local_conn("bastion", root.id);
        bastion.updated_at = ts;
        let mut nginx = make_local_conn("nginx-1", web.id);
        nginx.updated_at = ts;
        (web, vec![bastion, nginx])
    }

    /// The Settings "Import" button names the Import group after the file
    /// slug, so the Import root is "production-servers" while the Master's
    /// is "Production Servers".
    #[test]
    fn import_root_named_after_the_file_matches_the_master_tree() {
        let ts = Utc::now();
        let export = production_servers_export(ts);
        let root = make_local_group("production-servers", None);
        let (web, connections) = mirror_under(&root, ts);

        let result = GroupMergeEngine::merge(
            root.id,
            &[root, web],
            &connections,
            &export,
            &HashSet::new(),
        );

        assert_eq!(result, GroupMergeResult::default());
    }

    /// Renaming the local Import group is a local choice; it must not
    /// recreate anything.
    #[test]
    fn renamed_import_root_still_matches() {
        let ts = Utc::now();
        let export = production_servers_export(ts);
        let mut root = make_local_group("Production Servers", None);
        let (web, connections) = mirror_under(&root, ts);

        let before = GroupMergeEngine::merge(
            root.id,
            &[root.clone(), web.clone()],
            &connections,
            &export,
            &HashSet::new(),
        );
        assert_eq!(before, GroupMergeResult::default());

        root.name = "Prod (team copy)".to_owned();
        let after = GroupMergeEngine::merge(
            root.id,
            &[root, web],
            &connections,
            &export,
            &HashSet::new(),
        );
        assert_eq!(after, GroupMergeResult::default());
    }

    /// An Import root inside another local group. With the parent in the
    /// slice its own path is "Work/production-servers"; without it — what
    /// `SyncManager` passes — the root still has a `parent_id`. Neither the
    /// root, nor the parent, nor a connection in the parent may be touched.
    #[test]
    fn nested_import_root_matches_and_is_never_deleted() {
        let ts = Utc::now();
        let export = production_servers_export(ts);
        let work = make_local_group("Work", None);
        let root = make_local_group("production-servers", Some(work.id));
        let (web, connections) = mirror_under(&root, ts);

        let mut with_outsider = connections.clone();
        with_outsider.push(make_local_conn("laptop", work.id));
        let with_parent = GroupMergeEngine::merge(
            root.id,
            &[work, root.clone(), web.clone()],
            &with_outsider,
            &export,
            &HashSet::new(),
        );
        assert_eq!(with_parent, GroupMergeResult::default());

        let subtree_only = GroupMergeEngine::merge(
            root.id,
            &[root, web],
            &connections,
            &export,
            &HashSet::new(),
        );
        assert_eq!(subtree_only, GroupMergeResult::default());
    }

    /// Matching inside the synced group must not weaken deletion: a
    /// connection the Master no longer has still goes.
    #[test]
    fn connection_missing_from_the_export_is_still_deleted_under_a_renamed_root() {
        let ts = Utc::now();
        let export = production_servers_export(ts);
        let root = make_local_group("production-servers", None);
        let (web, mut connections) = mirror_under(&root, ts);
        let stale = make_local_conn("decommissioned", web.id);
        let stale_id = stale.id;
        connections.push(stale);

        let result = GroupMergeEngine::merge(
            root.id,
            &[root, web],
            &connections,
            &export,
            &HashSet::new(),
        );

        assert_eq!(result.connections_to_delete, vec![stale_id]);
        assert!(result.connections_to_create.is_empty());
        assert!(result.connections_to_update.is_empty());
        assert!(result.groups_to_create.is_empty());
        assert!(result.groups_to_delete.is_empty());
    }

    /// A newer copy on the Master is an update of the matched local
    /// connection, not a delete-and-recreate.
    #[test]
    fn newer_remote_connection_updates_under_a_renamed_root() {
        let ts = Utc::now() - Duration::hours(1);
        let mut export = production_servers_export(ts);
        let root = make_local_group("production-servers", None);
        let (web, connections) = mirror_under(&root, ts);
        let nginx_id = connections[1].id;
        for conn in &mut export.connections {
            if conn.name == "nginx-1" {
                conn.updated_at = Utc::now();
            }
        }

        let result = GroupMergeEngine::merge(
            root.id,
            &[root, web],
            &connections,
            &export,
            &HashSet::new(),
        );

        assert_eq!(result.connections_to_update.len(), 1);
        assert_eq!(result.connections_to_update[0].0, nginx_id);
        assert!(result.connections_to_create.is_empty());
        assert!(result.connections_to_delete.is_empty());
    }

    #[test]
    fn relative_path_strips_the_root_as_a_whole() {
        assert_eq!(relative_path("Root", "Root"), "");
        assert_eq!(relative_path("Root/Web", "Root"), "Web");
        // A `/` inside the root's name does not shift the split.
        assert_eq!(relative_path("A/B/Web", "A/B"), "Web");
        // A sibling whose name merely starts with the root's is not inside it.
        assert_eq!(relative_path("Production/Web", "Prod"), "Production/Web");
        // A path from outside the root is left alone.
        assert_eq!(relative_path("Other/Web", "Root"), "Other/Web");
    }
}
