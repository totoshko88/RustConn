//! Group Sync merge engine for Import mode.
//!
//! [`GroupMergeEngine`] computes a diff between the local group tree and a
//! remote [`GroupSyncExport`], producing a [`GroupMergeResult`] that describes
//! which connections, groups, and variable templates need to be created,
//! updated, linked or deleted locally. [`GroupSyncPlan`] turns that result into
//! the concrete local entities, and both the GUI and the CLI apply that plan,
//! so the two build the same tree.
//!
//! Matching runs in two passes. The first is by **id**: since 0.22.13 every
//! exported group and connection carries its id on the Master, and since 0.23
//! every group and connection an import creates records that id in
//! `sync_origin_id`. A rename or move on the Master is therefore recognised as
//! the same entity and applied in place. The second pass takes whatever the
//! first left over — files written by 0.22.12 and earlier, and local entities
//! created before 0.23 — and matches by **name + group path** for connections
//! and **path** for groups. A pair the second pass finds is *linked*: its
//! `sync_origin_id` is recorded, so the next sync matches it by id.
//!
//! Every path is taken *inside the synced group*: the Master's root name is
//! removed from the front of each exported path and the Import root's own path
//! from each local one, so the root is `""` on both sides. Neither root's name,
//! nor where the Import root sits in the local tree, takes part in matching.
//!
//! For a connection's content, the newer `updated_at` wins. For names and
//! placement the Master is authoritative: a matched entity whose name or
//! parent differs is renamed or moved whatever its timestamp, because
//! `SyncGroup` has no timestamp at all and a placement that only one side
//! agrees with would be re-reported on every sync.
//!
//! [`GroupSyncPlan`]: super::group_apply::GroupSyncPlan

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use super::group_export::{GroupSyncExport, SyncConnection, SyncGroup, compute_group_path};
use super::variable_template::VariableTemplate;
use crate::models::{Connection, ConnectionGroup, collect_descendant_group_ids};

/// Id- and name-based merge engine for Group Sync Import mode.
///
/// Stateless — all inputs are passed to [`merge()`](Self::merge).
pub struct GroupMergeEngine;

/// Result of a group merge operation.
///
/// Each field describes a set of changes that the caller should apply to the
/// local tree — through [`GroupSyncPlan`](super::group_apply::GroupSyncPlan),
/// which resolves every path below to a local group id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroupMergeResult {
    /// Remote connections not present locally — should be created.
    pub connections_to_create: Vec<SyncConnection>,
    /// Local connections matched to a remote one that is newer, or whose name
    /// or group the Master changed — the tuple is
    /// `(local_connection_id, remote_data)`.
    pub connections_to_update: Vec<(Uuid, SyncConnection)>,
    /// Local connections not present in the remote export — should be deleted.
    pub connections_to_delete: Vec<Uuid>,
    /// Local connections matched by name and path that need nothing but their
    /// link to the Master's copy recorded — `(local_connection_id,
    /// master_id)`. Not a visible change, so a sync report does not count it.
    pub connections_to_link: Vec<(Uuid, Uuid)>,
    /// Remote groups (by path inside the synced group) not present locally —
    /// should be created. Each keeps the path the export wrote.
    pub groups_to_create: Vec<SyncGroup>,
    /// Local groups matched to a remote group whose name or parent differs —
    /// renamed or moved on the Master, or left under a parent that is going
    /// away. Applied in place, so the group keeps its id and its connections.
    /// The tuple is `(local_group_id, remote_data)`.
    pub groups_to_update: Vec<(Uuid, SyncGroup)>,
    /// Local groups (by path inside the synced group) not present in the
    /// remote export — should be deleted. Never the Import root itself.
    pub groups_to_delete: Vec<Uuid>,
    /// Local groups matched by path that need nothing but their link to the
    /// Master's group recorded — `(local_group_id, master_id)`.
    pub groups_to_link: Vec<(Uuid, Uuid)>,
    /// Every local subgroup that survives this sync, with the path inside the
    /// synced group it occupies afterwards, sorted by path.
    ///
    /// The apply side seeds its `path → local id` map with it, so a group or
    /// connection added under a subgroup that already exists lands there
    /// instead of creating a second copy of the subgroup.
    pub group_layout: Vec<(String, Uuid)>,
    /// Remote variable templates not present locally — should be created.
    pub variables_to_create: Vec<VariableTemplate>,
    /// The Master export's root group name, needed by the apply side to turn
    /// each created group/connection's full `path`/`group_path` into a path
    /// *relative to the synced root* and so rebuild the nested hierarchy under
    /// the Import root. Empty on a `Default` (no-op) result.
    pub remote_root: String,
}

impl GroupMergeResult {
    /// Returns `true` when applying this result would change nothing locally.
    ///
    /// Links count as a change here — they are persisted — although a sync
    /// report does not show them.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.connections_to_create.is_empty()
            && self.connections_to_update.is_empty()
            && self.connections_to_delete.is_empty()
            && self.connections_to_link.is_empty()
            && self.groups_to_create.is_empty()
            && self.groups_to_update.is_empty()
            && self.groups_to_delete.is_empty()
            && self.groups_to_link.is_empty()
            && self.variables_to_create.is_empty()
    }
}

/// Composite key for connection lookup: `(name, group path inside the synced
/// group)`.
type ConnectionKey<'a> = (&'a str, &'a str);

/// Where a path inside the synced group points once this sync is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// An existing local group (the Import root included).
    Existing(Uuid),
    /// A group this sync creates.
    Created,
}

/// The layout of the Import tree after the sync: which local group owns each
/// path, and which paths are new. Built once from the group matches, then
/// consulted for every group's parent and every connection's group, with the
/// same rule [`GroupSyncPlan`](super::group_apply::GroupSyncPlan) resolves by.
struct FinalLayout<'a> {
    root_id: Uuid,
    owners: HashMap<&'a str, Uuid>,
    created: HashSet<&'a str>,
}

impl FinalLayout<'_> {
    /// Resolves `path`. A path nothing owns or creates — only an export that
    /// names a group it does not list produces one — falls back to the root,
    /// which is also where the apply side files it.
    fn target(&self, path: &str) -> Target {
        if path.is_empty() {
            return Target::Existing(self.root_id);
        }
        if let Some(id) = self.owners.get(path) {
            return Target::Existing(*id);
        }
        if self.created.contains(path) {
            return Target::Created;
        }
        Target::Existing(self.root_id)
    }
}

impl GroupMergeEngine {
    /// Computes the diff between the Import tree under `root_id` and a remote
    /// [`GroupSyncExport`].
    ///
    /// Keys are taken inside the synced group (see the module docs), so the
    /// Import root can carry any name — the Settings "Import" button names it
    /// after the file, not after the Master's group — and can sit anywhere in
    /// the local tree.
    ///
    /// # Algorithm
    ///
    /// 1. **Groups** — match by `sync_origin_id` against the exported id, then
    ///    the remainder by path. Unmatched remote → create; unmatched local →
    ///    delete; matched with a different name or parent → update; matched by
    ///    path only → link. The Import root stands for the export as a whole and
    ///    is never a candidate. The matches fix the final layout
    ///    ([`GroupMergeResult::group_layout`]).
    /// 2. **Connections** — match by `sync_origin_id`, then the remainder by
    ///    `(name, group path)`. Unmatched remote → create; unmatched local →
    ///    delete; matched and newer on the Master, or renamed, or filed in a
    ///    different group of the final layout → update; matched by name only
    ///    → link.
    /// 3. **Variable templates** — remote templates whose name is not among
    ///    `local_variable_names` → create.
    ///
    /// Groups and connections outside the root's subtree are ignored: they are
    /// not the Import group's to match, update or delete. A malformed export
    /// that lists one id twice has the second occurrence matched as if it had
    /// no id, with a warning.
    ///
    /// `local_groups` must include the Import root and its subtree; any others
    /// in the slice, such as the root's parent, are ignored.
    /// `local_connections` may hold connections outside the subtree too.
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
        let remote_groups = normalized_remote_groups(&remote.groups, remote_root);
        let remote_connections = normalized_remote_connections(&remote.connections);

        // --- Phase 1: groups, which also fixes the final layout ---
        let layout = Self::merge_groups(
            root_id,
            local_groups,
            &local_paths,
            &remote_groups,
            remote_root,
            &mut result,
        );

        // --- Phase 2: connections against that layout ---
        Self::merge_connections(
            &local_paths,
            local_connections,
            &remote_connections,
            remote_root,
            &layout,
            &mut result,
        );

        // --- Phase 3: Variable templates ---
        Self::merge_variables(
            &remote.variable_templates,
            local_variable_names,
            &mut result,
        );

        // Carry the Master root name so the apply side can rebuild nesting.
        result.remote_root = remote_root.to_owned();

        result
    }

    /// Phase 1: diff subgroups, matching by `sync_origin_id` first and by path
    /// for the remainder, and return the layout the matches produce.
    fn merge_groups<'a>(
        root_id: Uuid,
        local_groups: &[ConnectionGroup],
        local_paths: &HashMap<Uuid, String>,
        remote_groups: &'a [SyncGroup],
        remote_root: &str,
        result: &mut GroupMergeResult,
    ) -> FinalLayout<'a> {
        // In-scope local subgroups, sorted by id so that every "first wins"
        // below is deterministic. The root is left out by id, not by
        // `parent_id`: an Import root nested under another local group has a
        // parent, and until 0.22.13 that put it on the delete list of its own
        // sync.
        let mut in_scope: Vec<&ConnectionGroup> = local_groups
            .iter()
            .filter(|g| g.id != root_id && local_paths.contains_key(&g.id))
            .collect();
        in_scope.sort_by_key(|g| g.id);
        in_scope.dedup_by_key(|g| g.id);

        let mut consumed: HashSet<Uuid> = HashSet::new();
        // remote index → matched local group
        let mut matched: Vec<Option<&ConnectionGroup>> = vec![None; remote_groups.len()];

        // --- Pass A: the Master id this group was created from ---
        let mut by_origin: HashMap<Uuid, &ConnectionGroup> = HashMap::new();
        for group in in_scope.iter().copied() {
            if let Some(origin) = group.sync_origin_id {
                by_origin.entry(origin).or_insert(group);
            }
        }
        for (slot, remote_group) in matched.iter_mut().zip(remote_groups) {
            if let Some(local) = remote_group.id.and_then(|id| by_origin.get(&id).copied())
                && consumed.insert(local.id)
            {
                *slot = Some(local);
            }
        }

        // --- Pass B: the remainder by current path ---
        // Several locals can share a path only in a damaged tree; each remote
        // takes the one with the lowest id, the rest are deleted.
        let mut by_path: HashMap<&str, Vec<&ConnectionGroup>> = HashMap::new();
        for group in in_scope.iter().rev().copied() {
            if !consumed.contains(&group.id)
                && let Some(path) = local_paths.get(&group.id)
            {
                by_path.entry(path.as_str()).or_default().push(group);
            }
        }
        for (slot, remote_group) in matched.iter_mut().zip(remote_groups) {
            if slot.is_some() {
                continue;
            }
            let rel = relative_path(&remote_group.path, remote_root);
            if let Some(local) = by_path.get_mut(rel).and_then(Vec::pop) {
                consumed.insert(local.id);
                *slot = Some(local);
            }
        }

        // The layout: every matched local at its remote path, every unmatched
        // remote path as a creation. A path claimed twice — a malformed export
        // — goes to the first claimant; the others are neither kept there nor
        // created a second time.
        let mut layout = FinalLayout {
            root_id,
            owners: HashMap::new(),
            created: HashSet::new(),
        };
        for (slot, remote_group) in matched.iter().zip(remote_groups) {
            if let Some(local) = slot {
                let rel = relative_path(&remote_group.path, remote_root);
                layout.owners.entry(rel).or_insert(local.id);
            }
        }
        for (slot, remote_group) in matched.iter().zip(remote_groups) {
            if slot.is_some() {
                continue;
            }
            let rel = relative_path(&remote_group.path, remote_root);
            if layout.owners.contains_key(rel) || !layout.created.insert(rel) {
                tracing::warn!(
                    path = %remote_group.path,
                    "Group Sync: export lists one group path twice; keeping the first"
                );
                continue;
            }
            result.groups_to_create.push(remote_group.clone());
        }

        // Matched groups: rename/move in place, or just record the link.
        for (slot, remote_group) in matched.iter().zip(remote_groups) {
            let Some(local) = slot else { continue };
            let rel = relative_path(&remote_group.path, remote_root);
            let parent = layout.target(parent_relative_path(rel, &remote_group.name));
            let same_parent = parent == Target::Existing(local.parent_id.unwrap_or(root_id));
            if local.name != remote_group.name || !same_parent {
                result
                    .groups_to_update
                    .push((local.id, remote_group.clone()));
            } else if let Some(origin) = remote_group.id
                && local.sync_origin_id != Some(origin)
            {
                result.groups_to_link.push((local.id, origin));
            }
        }

        // Unmatched locals → delete.
        result.groups_to_delete.extend(
            in_scope
                .iter()
                .filter(|g| !consumed.contains(&g.id))
                .map(|g| g.id),
        );

        let mut kept: Vec<(String, Uuid)> = layout
            .owners
            .iter()
            .map(|(path, id)| ((*path).to_owned(), *id))
            .collect();
        kept.sort();
        result.group_layout = kept;

        layout
    }

    /// Phase 2: diff connections, matching by `sync_origin_id` first and by
    /// `(name, group path inside the synced group)` for the remainder.
    ///
    /// Matching by the id the Master gave the connection means a connection
    /// **renamed or moved** on the Master is updated in place — same local id,
    /// same vault entry — instead of being deleted and recreated, which
    /// churned the local id and broke the vault credential link (issue #263).
    fn merge_connections(
        local_paths: &HashMap<Uuid, String>,
        local_connections: &[Connection],
        remote_connections: &[SyncConnection],
        remote_root: &str,
        layout: &FinalLayout<'_>,
        result: &mut GroupMergeResult,
    ) {
        // Only connections inside the Import root's subtree are eligible: one
        // outside it gets no path here, so it is never matched, updated or
        // deleted by this sync. Sorted by id so every "first wins" is stable.
        let mut in_scope: Vec<&Connection> = local_connections
            .iter()
            .filter(|c| c.group_id.is_some_and(|g| local_paths.contains_key(&g)))
            .collect();
        in_scope.sort_by_key(|c| c.id);
        in_scope.dedup_by_key(|c| c.id);

        let mut consumed: HashSet<Uuid> = HashSet::new();
        let mut matched: Vec<Option<&Connection>> = vec![None; remote_connections.len()];

        // --- Pass A: the Master id this connection was created from ---
        let mut by_origin: HashMap<Uuid, &Connection> = HashMap::new();
        for conn in in_scope.iter().copied() {
            if let Some(origin) = conn.sync_origin_id {
                by_origin.entry(origin).or_insert(conn);
            }
        }
        for (slot, remote_conn) in matched.iter_mut().zip(remote_connections) {
            if let Some(local) = remote_conn.id.and_then(|id| by_origin.get(&id).copied())
                && consumed.insert(local.id)
            {
                *slot = Some(local);
            }
        }

        // --- Pass B: the remainder by (name, current group path) ---
        // A list per key, not a single entry: two connections may share a
        // name in one group, and a map keyed on the name alone kept only one
        // of them and recreated the other on every sync.
        let mut by_key: HashMap<ConnectionKey<'_>, Vec<&Connection>> = HashMap::new();
        for conn in in_scope.iter().rev().copied() {
            if consumed.contains(&conn.id) {
                continue;
            }
            if let Some(path) = conn.group_id.and_then(|g| local_paths.get(&g)) {
                by_key
                    .entry((conn.name.as_str(), path.as_str()))
                    .or_default()
                    .push(conn);
            }
        }
        for (slot, remote_conn) in matched.iter_mut().zip(remote_connections) {
            if slot.is_some() {
                continue;
            }
            let key = (
                remote_conn.name.as_str(),
                relative_path(&remote_conn.group_path, remote_root),
            );
            if let Some(local) = by_key.get_mut(&key).and_then(Vec::pop) {
                consumed.insert(local.id);
                *slot = Some(local);
            }
        }

        for (slot, remote_conn) in matched.iter().zip(remote_connections) {
            let Some(local) = slot else {
                result.connections_to_create.push(remote_conn.clone());
                continue;
            };
            let target = layout.target(relative_path(&remote_conn.group_path, remote_root));
            let same_group = local
                .group_id
                .is_some_and(|g| target == Target::Existing(g));
            if remote_conn.updated_at > local.updated_at
                || local.name != remote_conn.name
                || !same_group
            {
                result
                    .connections_to_update
                    .push((local.id, remote_conn.clone()));
            } else if let Some(origin) = remote_conn.id
                && local.sync_origin_id != Some(origin)
            {
                result.connections_to_link.push((local.id, origin));
            }
        }

        // Matched by neither pass → delete.
        result.connections_to_delete.extend(
            in_scope
                .iter()
                .filter(|c| !consumed.contains(&c.id))
                .map(|c| c.id),
        );
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

/// The export's subgroups, ready to match: an entry whose path is the root's
/// own (inside the synced group it would be `""`, which only the Import root
/// may occupy) is dropped, and the second occurrence of a repeated id loses
/// the id, so one Master id never matches two remote entries.
fn normalized_remote_groups(groups: &[SyncGroup], remote_root: &str) -> Vec<SyncGroup> {
    let mut seen: HashSet<Uuid> = HashSet::new();
    groups
        .iter()
        .filter(|g| {
            let is_root = relative_path(&g.path, remote_root).is_empty();
            if is_root {
                tracing::warn!(path = %g.path, "Group Sync: ignoring a subgroup with the root's path");
            }
            !is_root
        })
        .map(|g| {
            let mut g = g.clone();
            if let Some(id) = g.id
                && !seen.insert(id)
            {
                tracing::warn!(%id, path = %g.path, "Group Sync: export repeats a group id; matching the copy by path");
                g.id = None;
            }
            g
        })
        .collect()
}

/// The export's connections, ready to match: the second occurrence of a
/// repeated id loses the id (see [`normalized_remote_groups`]).
fn normalized_remote_connections(connections: &[SyncConnection]) -> Vec<SyncConnection> {
    let mut seen: HashSet<Uuid> = HashSet::new();
    connections
        .iter()
        .map(|c| {
            let mut c = c.clone();
            if let Some(id) = c.id
                && !seen.insert(id)
            {
                tracing::warn!(%id, name = %c.name, "Group Sync: export repeats a connection id; matching the copy by name");
                c.id = None;
            }
            c
        })
        .collect()
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
/// segment, so a `/` inside a group name cannot shift the split. Public so the
/// apply side maps a created group's or connection's full path onto the local
/// tree with exactly the semantics the merge used.
#[must_use]
pub fn relative_path<'a>(path: &'a str, root: &str) -> &'a str {
    if path == root {
        return "";
    }
    path.strip_prefix(root)
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(path)
}

/// Returns the path of a group's parent, given the group's own path `rel`
/// inside the synced group and its `name`.
///
/// The exporter builds a path as the parent's path, `/`, then the name
/// ([`compute_group_path`]) without escaping, so the parent is found by
/// removing the name from the end — not by splitting at the last `/`, which
/// filed a group named `"a/b"` as a group `"b"` inside a new group `"a"`. A
/// path that does not end in the name (a hand-edited file) falls back to the
/// last `/`.
#[must_use]
pub fn parent_relative_path<'a>(rel: &'a str, name: &str) -> &'a str {
    if rel == name {
        return "";
    }
    if let Some(parent) = rel.strip_suffix(name).and_then(|p| p.strip_suffix('/')) {
        return parent;
    }
    rel.rsplit_once('/').map_or("", |(parent, _)| parent)
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration, Utc};

    use super::*;
    use crate::models::{
        AutomationConfig, PasswordSource, ProtocolConfig, ProtocolType, SshConfig,
    };

    /// Asserts the merge produced no local changes. Checks the action buckets
    /// rather than `== GroupMergeResult::default()`, because the result also
    /// carries metadata (`remote_root`) that is legitimately populated even on
    /// a no-op merge.
    fn assert_no_changes(result: &GroupMergeResult) {
        assert!(
            result.connections_to_create.is_empty(),
            "unexpected creates"
        );
        assert!(
            result.connections_to_update.is_empty(),
            "unexpected updates"
        );
        assert!(
            result.connections_to_delete.is_empty(),
            "unexpected deletes"
        );
        assert!(
            result.groups_to_create.is_empty(),
            "unexpected group creates"
        );
        assert!(
            result.groups_to_update.is_empty(),
            "unexpected group updates"
        );
        assert!(
            result.groups_to_delete.is_empty(),
            "unexpected group deletes"
        );
        assert!(
            result.variables_to_create.is_empty(),
            "unexpected variables"
        );
    }

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
        assert_no_changes(&result);
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
        // The Import copy has an id of its own and records the Master's in
        // `sync_origin_id`, as an import creates it.
        let master_id = Uuid::new_v4();
        let root = make_local_group("Root", None);
        let mut local_conn = make_local_conn("old-name", root.id);
        local_conn.sync_origin_id = Some(master_id);
        local_conn.updated_at = Utc::now() - Duration::hours(1);

        let mut remote_conn = make_sync_conn("new-name", "Root");
        remote_conn.id = Some(master_id);
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
        let master_id = Uuid::new_v4();
        let mut local_conn = make_local_conn("server-1", web.id);
        local_conn.sync_origin_id = Some(master_id);
        local_conn.updated_at = Utc::now() - Duration::hours(1);

        let mut remote_conn = make_sync_conn("server-1", "Root/DB");
        remote_conn.id = Some(master_id);
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
        let (master_a, master_b) = (Uuid::new_v4(), Uuid::new_v4());
        let mut a = make_local_conn("dup", root.id);
        let mut b = make_local_conn("dup", root.id);
        a.sync_origin_id = Some(master_a);
        b.sync_origin_id = Some(master_b);
        let past = Utc::now() - Duration::hours(1);
        a.updated_at = past;
        b.updated_at = past;

        let now = Utc::now();
        let mut remote_a = make_sync_conn("dup", "Root");
        remote_a.id = Some(master_a);
        remote_a.updated_at = now;
        let mut remote_b = make_sync_conn("dup", "Root");
        remote_b.id = Some(master_b);
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
        let master_id = Uuid::new_v4();
        let mut outside_conn = make_local_conn("secret", outside_group.id);
        outside_conn.sync_origin_id = Some(master_id);
        outside_conn.updated_at = Utc::now() - Duration::hours(1);

        let mut remote_conn = make_sync_conn("secret", "Root");
        remote_conn.id = Some(master_id);
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

    #[test]
    fn merge_result_carries_remote_root_for_nested_rebuild() {
        // The apply side rebuilds nesting by stripping the Master root from each
        // created group's full path. Verify the result exposes that root and
        // that a nested group arrives with a full path that relative_path turns
        // into the correct synced-root-relative path.
        let root = make_local_group("Root", None);
        // Remote has a nested subgroup Web/Prod that is absent locally.
        let remote_web = make_sync_group("Web", "Root/Web");
        let remote_prod = make_sync_group("Prod", "Root/Web/Prod");
        let export = make_export(vec![remote_web, remote_prod], vec![], vec![]);

        let result = GroupMergeEngine::merge(root.id, &[root], &[], &export, &HashSet::new());

        assert_eq!(result.remote_root, "Root", "root name must be carried");
        assert_eq!(result.groups_to_create.len(), 2);

        // The deeper group's path, made relative to the carried root, is the
        // nested path the apply side files it under.
        let prod = result
            .groups_to_create
            .iter()
            .find(|g| g.name == "Prod")
            .expect("Prod group created");
        assert_eq!(relative_path(&prod.path, &result.remote_root), "Web/Prod");
        let web = result
            .groups_to_create
            .iter()
            .find(|g| g.name == "Web")
            .expect("Web group created");
        assert_eq!(relative_path(&web.path, &result.remote_root), "Web");
    }

    // ---------------------------------------------------------------
    // Phase 1: id-based group matching (SYNC-1)
    // ---------------------------------------------------------------

    #[test]
    fn export_rename_remerge_round_trip_is_an_update_not_a_duplicate() {
        // End-to-end for SYNC-1: a connection is exported on the "Master" via
        // the real `SyncConnection::from_connection` (which stamps the id),
        // renamed on the Master, then re-merged on the "Import" side against the
        // original local connection. With id matching this is a single update
        // carrying the new name — not delete(old) + create(new).
        use super::super::group_export::SyncConnection;

        // Two separate entities, as on two devices: the Master's connection and
        // the Import copy an earlier sync created from it, which has an id of
        // its own and the Master's id in `sync_origin_id`. Until 0.23 this test
        // used one connection for both sides, so it matched on an id the Import
        // copy never actually had.
        let master_root = make_local_group("Root", None);
        let mut master_conn = make_local_conn("db-prod", master_root.id);
        master_conn.updated_at = Utc::now() - Duration::hours(2);

        let root = make_local_group("Root", None);
        let mut local_conn = super::super::group_export::sync_connection_to_connection(
            &SyncConnection::from_connection(&master_conn, "Root"),
            root.id,
        );
        local_conn.sync_origin_id = Some(master_conn.id);
        assert_ne!(local_conn.id, master_conn.id);

        // Master renames it and exports again (the id is carried).
        master_conn.name = "db-production".to_owned();
        master_conn.updated_at = Utc::now();
        let exported = SyncConnection::from_connection(&master_conn, "Root");
        assert_eq!(
            exported.id,
            Some(master_conn.id),
            "export must carry the id"
        );

        let export = make_export(vec![], vec![exported], vec![]);
        let result = GroupMergeEngine::merge(
            root.id,
            &[root],
            std::slice::from_ref(&local_conn),
            &export,
            &HashSet::new(),
        );

        assert_eq!(
            result.connections_to_update.len(),
            1,
            "a Master rename must re-merge as exactly one update"
        );
        assert_eq!(result.connections_to_update[0].0, local_conn.id);
        assert_eq!(result.connections_to_update[0].1.name, "db-production");
        assert!(
            result.connections_to_create.is_empty(),
            "rename round-trip must not create a duplicate"
        );
        assert!(
            result.connections_to_delete.is_empty(),
            "rename round-trip must not delete the original"
        );
    }

    #[test]
    fn renamed_group_matched_by_id_is_updated_not_recreated() {
        // Local "Web" subgroup; the Master renamed it to "Prod" (same id). With
        // id matching this is a single update, not delete("Web") + create("Prod")
        // which would have orphaned every connection filed under it.
        let master_id = Uuid::new_v4();
        let root = make_local_group("Root", None);
        let mut web = make_local_group("Web", Some(root.id));
        web.sync_origin_id = Some(master_id);

        let mut remote_group = make_sync_group("Prod", "Root/Prod");
        remote_group.id = Some(master_id);

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

    /// Matching is on `sync_origin_id`, never on a local entity's own `id`. A
    /// local whose own id happens to equal an exported one (the Master's own
    /// copy on the same device would be exactly that) is not taken for the
    /// Import copy.
    #[test]
    fn a_local_id_equal_to_the_remote_id_is_not_an_id_match() {
        let root = make_local_group("Root", None);
        let mut local_conn = make_local_conn("old-name", root.id);
        local_conn.updated_at = Utc::now() - Duration::hours(1);
        let mut remote_conn = make_sync_conn("new-name", "Root");
        remote_conn.id = Some(local_conn.id);

        let export = make_export(vec![], vec![remote_conn], vec![]);
        let result = GroupMergeEngine::merge(
            root.id,
            &[root],
            std::slice::from_ref(&local_conn),
            &export,
            &HashSet::new(),
        );

        assert!(result.connections_to_update.is_empty());
        assert_eq!(result.connections_to_create.len(), 1);
        assert_eq!(result.connections_to_delete, vec![local_conn.id]);
    }

    /// Renaming "Web" to "Prod" and creating a new "Web" on the Master: the
    /// old group is renamed, the new one created, nothing deleted. Path
    /// matching used to see the new "Web" as the existing local one.
    #[test]
    fn a_renamed_group_frees_its_old_path_for_a_new_group() {
        let master_web = Uuid::new_v4();
        let root = make_local_group("Root", None);
        let mut web = make_local_group("Web", Some(root.id));
        web.sync_origin_id = Some(master_web);

        let mut renamed = make_sync_group("Prod", "Root/Prod");
        renamed.id = Some(master_web);
        let mut fresh = make_sync_group("Web", "Root/Web");
        fresh.id = Some(Uuid::new_v4());
        let export = make_export(vec![renamed, fresh], vec![], vec![]);

        let result =
            GroupMergeEngine::merge(root.id, &[root, web.clone()], &[], &export, &HashSet::new());

        assert_eq!(result.groups_to_update.len(), 1);
        assert_eq!(result.groups_to_update[0].0, web.id);
        assert_eq!(result.groups_to_create.len(), 1);
        assert_eq!(result.groups_to_create[0].path, "Root/Web");
        assert!(result.groups_to_delete.is_empty());
        assert_eq!(result.group_layout, vec![("Prod".to_owned(), web.id)]);
    }

    #[test]
    fn parent_relative_path_removes_the_name_not_the_last_segment() {
        assert_eq!(parent_relative_path("Web", "Web"), "");
        assert_eq!(parent_relative_path("Web/Prod", "Prod"), "Web");
        assert_eq!(parent_relative_path("Web/eu/west", "eu/west"), "Web");
        assert_eq!(parent_relative_path("eu/west", "eu/west"), "");
        // A path that does not end in the name falls back to the last `/`.
        assert_eq!(parent_relative_path("Web/Other", "Prod"), "Web");
        assert_eq!(parent_relative_path("Other", "Prod"), "");
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

        assert_no_changes(&result);
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
        assert_no_changes(&before);

        root.name = "Prod (team copy)".to_owned();
        let after = GroupMergeEngine::merge(
            root.id,
            &[root, web],
            &connections,
            &export,
            &HashSet::new(),
        );
        assert_no_changes(&after);
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
        assert_no_changes(&with_parent);

        let subtree_only = GroupMergeEngine::merge(
            root.id,
            &[root, web],
            &connections,
            &export,
            &HashSet::new(),
        );
        assert_no_changes(&subtree_only);
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
