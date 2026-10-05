//! The concrete local changes of one Group Sync import.
//!
//! [`GroupMergeEngine`](super::group_merge::GroupMergeEngine) says *what*
//! differs, keyed by paths inside the synced group. [`GroupSyncPlan::build`]
//! decides *where* each change lands: it resolves every path to a local group
//! id, mints the new groups and connections, and records each one's Master id
//! in `sync_origin_id`. It does no I/O. The GUI applies the plan through the
//! `ConnectionManager`, the CLI through [`GroupSyncPlan::apply_to`] on the
//! loaded vectors, so the two build the same tree from the same file.

use std::collections::HashMap;

use uuid::Uuid;

use super::group_export::{apply_sync_connection_update, sync_connection_to_connection};
use super::group_merge::{GroupMergeResult, parent_relative_path, relative_path};
use crate::models::{Connection, ConnectionGroup};

/// The concrete changes one Group Sync import makes to the local tree.
///
/// Apply it in field order — groups created, groups updated, connections
/// created, connections updated, connections deleted, groups deleted. Every
/// connection the Master kept is moved to its new group *before* any group is
/// deleted, so deleting a group (which ungroups whatever is still in it) can
/// no longer strand a kept connection outside the Import tree, where the next
/// sync would recreate it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GroupSyncPlan {
    /// New groups, each parent before its children.
    pub groups_to_create: Vec<ConnectionGroup>,
    /// Existing groups in their new state: renamed, moved, or newly linked.
    pub groups_to_update: Vec<ConnectionGroup>,
    /// New connections, already filed in their group.
    pub connections_to_create: Vec<Connection>,
    /// Existing connections in their new state: updated, renamed, moved, or
    /// newly linked.
    pub connections_to_update: Vec<Connection>,
    /// Connections the Master no longer has.
    pub connections_to_delete: Vec<Uuid>,
    /// Groups the Master no longer has. Never the Import root.
    pub groups_to_delete: Vec<Uuid>,
}

impl GroupSyncPlan {
    /// Builds the plan for applying `result` to the Import tree under `root_id`.
    ///
    /// `local_groups` and `local_connections` are the local state the merge
    /// ran against; entities outside the Import tree are left alone. Paths are
    /// resolved through the layout the merge fixed
    /// ([`GroupMergeResult::group_layout`]) plus the groups created here, so a
    /// group or connection added under a subgroup that already exists lands in
    /// it. A path nothing resolves — an export naming a group it does not
    /// list — lands at the Import root, which is where the merge expects it.
    ///
    /// Local-only fields of an updated connection are kept. When the Master's
    /// copy is not newer, only its name and group are taken.
    #[must_use]
    pub fn build(
        root_id: Uuid,
        local_groups: &[ConnectionGroup],
        local_connections: &[Connection],
        result: &GroupMergeResult,
    ) -> Self {
        let remote_root = result.remote_root.as_str();
        let groups_by_id: HashMap<Uuid, &ConnectionGroup> =
            local_groups.iter().map(|g| (g.id, g)).collect();
        let connections_by_id: HashMap<Uuid, &Connection> =
            local_connections.iter().map(|c| (c.id, c)).collect();

        let mut paths = PathMap::new(root_id, &result.group_layout);
        let mut orders = SortOrders::new(local_groups, local_connections);
        let mut plan = Self::default();

        // --- Groups to create, parents first ---
        // A parent's path is a strict prefix of its child's, hence shorter.
        let mut creates: Vec<_> = result
            .groups_to_create
            .iter()
            .map(|g| (relative_path(&g.path, remote_root), g))
            .collect();
        creates.sort_by(|(a, _), (b, _)| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
        for (rel, remote) in creates {
            if rel.is_empty() || paths.contains(rel) {
                continue; // the root itself, or a path already filled
            }
            let parent_id = paths.resolve(parent_relative_path(rel, &remote.name));
            let mut group = ConnectionGroup::with_parent(remote.name.clone(), parent_id);
            group.description.clone_from(&remote.description);
            group.icon.clone_from(&remote.icon);
            group.username.clone_from(&remote.username);
            group.domain.clone_from(&remote.domain);
            group.ssh_auth_method.clone_from(&remote.ssh_auth_method);
            group.ssh_proxy_jump.clone_from(&remote.ssh_proxy_jump);
            group.sync_origin_id = remote.id;
            group.sort_order = orders.next_group(parent_id);
            paths.insert(rel, group.id);
            plan.groups_to_create.push(group);
        }

        // --- Groups to rename or move in place ---
        for (local_id, remote) in &result.groups_to_update {
            let Some(existing) = groups_by_id.get(local_id) else {
                continue;
            };
            let rel = relative_path(&remote.path, remote_root);
            let parent_id = paths.resolve(parent_relative_path(rel, &remote.name));
            let mut group = (*existing).clone();
            group.name.clone_from(&remote.name);
            if group.parent_id != Some(parent_id) {
                group.parent_id = Some(parent_id);
                group.sort_order = orders.next_group(parent_id);
            }
            if remote.id.is_some() {
                group.sync_origin_id = remote.id;
            }
            group.touch();
            plan.groups_to_update.push(group);
        }
        for (local_id, origin) in &result.groups_to_link {
            if let Some(existing) = groups_by_id.get(local_id) {
                let mut group = (*existing).clone();
                group.sync_origin_id = Some(*origin);
                plan.groups_to_update.push(group);
            }
        }

        // --- Connections ---
        for remote in &result.connections_to_create {
            let group_id = paths.resolve(relative_path(&remote.group_path, remote_root));
            let mut conn = sync_connection_to_connection(remote, group_id);
            conn.sync_origin_id = remote.id;
            conn.sort_order = orders.next_connection(group_id);
            plan.connections_to_create.push(conn);
        }
        for (local_id, remote) in &result.connections_to_update {
            let Some(existing) = connections_by_id.get(local_id) else {
                continue;
            };
            let mut conn = (*existing).clone();
            if remote.updated_at > existing.updated_at {
                apply_sync_connection_update(&mut conn, remote);
            } else {
                // Renamed or moved on the Master, but this copy is newer:
                // follow the Master's name and placement only.
                conn.name.clone_from(&remote.name);
            }
            let group_id = paths.resolve(relative_path(&remote.group_path, remote_root));
            if conn.group_id != Some(group_id) {
                conn.group_id = Some(group_id);
                conn.sort_order = orders.next_connection(group_id);
            }
            if remote.id.is_some() {
                conn.sync_origin_id = remote.id;
            }
            plan.connections_to_update.push(conn);
        }
        for (local_id, origin) in &result.connections_to_link {
            if let Some(existing) = connections_by_id.get(local_id) {
                let mut conn = (*existing).clone();
                conn.sync_origin_id = Some(*origin);
                plan.connections_to_update.push(conn);
            }
        }

        plan.connections_to_delete
            .clone_from(&result.connections_to_delete);
        plan.groups_to_delete.clone_from(&result.groups_to_delete);
        plan
    }

    /// Returns `true` when the plan changes nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.groups_to_create.is_empty()
            && self.groups_to_update.is_empty()
            && self.connections_to_create.is_empty()
            && self.connections_to_update.is_empty()
            && self.connections_to_delete.is_empty()
            && self.groups_to_delete.is_empty()
    }

    /// Applies the plan to in-memory `groups` and `connections`, in the order
    /// the type documents.
    ///
    /// Deleting a group behaves as `ConnectionManager::delete_group` does:
    /// its child groups move to its parent and its connections are ungrouped.
    /// A deleted connection is removed outright; there is no trash here.
    pub fn apply_to(&self, groups: &mut Vec<ConnectionGroup>, connections: &mut Vec<Connection>) {
        groups.extend(self.groups_to_create.iter().cloned());
        for updated in &self.groups_to_update {
            if let Some(slot) = groups.iter_mut().find(|g| g.id == updated.id) {
                slot.clone_from(updated);
            }
        }

        connections.extend(self.connections_to_create.iter().cloned());
        for updated in &self.connections_to_update {
            if let Some(slot) = connections.iter_mut().find(|c| c.id == updated.id) {
                slot.clone_from(updated);
            }
        }
        connections.retain(|c| !self.connections_to_delete.contains(&c.id));

        for id in &self.groups_to_delete {
            let Some(index) = groups.iter().position(|g| g.id == *id) else {
                continue;
            };
            let removed = groups.remove(index);
            for child in groups.iter_mut().filter(|g| g.parent_id == Some(*id)) {
                child.parent_id = removed.parent_id;
            }
            for conn in connections.iter_mut().filter(|c| c.group_id == Some(*id)) {
                conn.group_id = None;
            }
        }
    }
}

/// `path inside the synced group → local group id` for the tree after the
/// sync: the root at `""`, every surviving subgroup at its final path, and
/// each created group as soon as it is minted.
struct PathMap {
    root_id: Uuid,
    ids: HashMap<String, Uuid>,
}

impl PathMap {
    fn new(root_id: Uuid, layout: &[(String, Uuid)]) -> Self {
        let mut ids = HashMap::with_capacity(layout.len() + 1);
        ids.insert(String::new(), root_id);
        for (path, id) in layout {
            ids.entry(path.clone()).or_insert(*id);
        }
        Self { root_id, ids }
    }

    fn contains(&self, path: &str) -> bool {
        self.ids.contains_key(path)
    }

    fn insert(&mut self, path: &str, id: Uuid) {
        self.ids.insert(path.to_owned(), id);
    }

    /// The group at `path`, or the Import root when nothing is there.
    fn resolve(&self, path: &str) -> Uuid {
        self.ids.get(path).copied().unwrap_or(self.root_id)
    }
}

/// Next free `sort_order` per parent group, so new and moved entities append
/// after their siblings instead of colliding at 0.
struct SortOrders {
    groups: HashMap<Uuid, i32>,
    connections: HashMap<Uuid, i32>,
}

impl SortOrders {
    fn new(groups: &[ConnectionGroup], connections: &[Connection]) -> Self {
        let mut next_groups: HashMap<Uuid, i32> = HashMap::new();
        for group in groups {
            if let Some(parent) = group.parent_id {
                let next = next_groups.entry(parent).or_insert(0);
                *next = (*next).max(group.sort_order.saturating_add(1));
            }
        }
        let mut next_connections: HashMap<Uuid, i32> = HashMap::new();
        for conn in connections {
            if let Some(group) = conn.group_id {
                let next = next_connections.entry(group).or_insert(0);
                *next = (*next).max(conn.sort_order.saturating_add(1));
            }
        }
        Self {
            groups: next_groups,
            connections: next_connections,
        }
    }

    fn next_group(&mut self, parent: Uuid) -> i32 {
        Self::take(&mut self.groups, parent)
    }

    fn next_connection(&mut self, group: Uuid) -> i32 {
        Self::take(&mut self.connections, group)
    }

    fn take(map: &mut HashMap<Uuid, i32>, key: Uuid) -> i32 {
        let next = map.entry(key).or_insert(0);
        let value = *next;
        *next = next.saturating_add(1);
        value
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use chrono::{Duration, Utc};

    use super::*;
    use crate::sync::group_export::{GroupSyncExport, SyncConnection, SyncGroup};
    use crate::sync::group_merge::GroupMergeEngine;
    use crate::sync::{collect_variable_templates, compute_group_path};

    /// The Master side of a test: a root group and its tree.
    struct Master {
        groups: Vec<ConnectionGroup>,
        connections: Vec<Connection>,
        root_id: Uuid,
    }

    impl Master {
        fn new(root_name: &str) -> Self {
            let root = ConnectionGroup::new(root_name.to_owned());
            Self {
                root_id: root.id,
                groups: vec![root],
                connections: Vec::new(),
            }
        }

        fn group(&mut self, name: &str, parent: Uuid) -> Uuid {
            let group = ConnectionGroup::with_parent(name.to_owned(), parent);
            let id = group.id;
            self.groups.push(group);
            id
        }

        fn connection(&mut self, name: &str, group: Uuid) -> Uuid {
            let mut conn = Connection::new_ssh(name.to_owned(), "10.0.0.1".to_owned(), 22);
            conn.group_id = Some(group);
            conn.updated_at = Utc::now() - Duration::hours(10);
            let id = conn.id;
            self.connections.push(conn);
            id
        }

        /// What `SyncManager::export_group` writes, without the file.
        fn export(&self) -> GroupSyncExport {
            let root = self
                .groups
                .iter()
                .find(|g| g.id == self.root_id)
                .map(|g| SyncGroup::from_group(g, &g.name))
                .unwrap();
            let groups = self
                .groups
                .iter()
                .filter(|g| g.id != self.root_id)
                .map(|g| SyncGroup::from_group(g, &compute_group_path(g.id, &self.groups)))
                .collect();
            let connections = self
                .connections
                .iter()
                .map(|c| {
                    let path = compute_group_path(c.group_id.unwrap(), &self.groups);
                    SyncConnection::from_connection(c, &path)
                })
                .collect();
            GroupSyncExport::from_group_tree(
                "0.23.0".to_owned(),
                Uuid::new_v4(),
                "master".to_owned(),
                root,
                groups,
                connections,
                collect_variable_templates(&self.connections, &[]),
            )
        }
    }

    /// The Import side: a root (named after the file) and whatever syncs
    /// have built under it.
    struct Import {
        groups: Vec<ConnectionGroup>,
        connections: Vec<Connection>,
        root_id: Uuid,
    }

    impl Import {
        fn new() -> Self {
            let root = ConnectionGroup::new("master-file".to_owned());
            Self {
                root_id: root.id,
                groups: vec![root],
                connections: Vec::new(),
            }
        }

        fn merge(&self, export: &GroupSyncExport) -> GroupMergeResult {
            GroupMergeEngine::merge(
                self.root_id,
                &self.groups,
                &self.connections,
                export,
                &HashSet::new(),
            )
        }

        /// One sync: merge, plan, apply. Returns the merge result.
        fn sync(&mut self, export: &GroupSyncExport) -> GroupMergeResult {
            let result = self.merge(export);
            let plan = GroupSyncPlan::build(self.root_id, &self.groups, &self.connections, &result);
            plan.apply_to(&mut self.groups, &mut self.connections);
            result
        }

        fn path_of(&self, conn_name: &str) -> String {
            let conn = self
                .connections
                .iter()
                .find(|c| c.name == conn_name)
                .unwrap();
            compute_group_path(conn.group_id.unwrap(), &self.groups)
        }

        fn group_count(&self, name: &str) -> usize {
            self.groups.iter().filter(|g| g.name == name).count()
        }
    }

    #[test]
    fn nested_tree_is_rebuilt_and_a_resync_changes_nothing() {
        let mut master = Master::new("Production");
        let web = master.group("Web", master.root_id);
        let prod = master.group("Prod", web);
        master.connection("nginx-1", prod);
        master.connection("bastion", master.root_id);

        let mut import = Import::new();
        import.sync(&master.export());

        assert_eq!(import.path_of("nginx-1"), "master-file/Web/Prod");
        assert_eq!(import.path_of("bastion"), "master-file");
        let again = import.merge(&master.export());
        assert!(again.is_empty(), "second sync must be a no-op: {again:?}");
    }

    /// Finding: adding `Web/Prod` when `Web` already existed created a second
    /// `Web`, and a connection added to an existing subgroup landed at the
    /// Import root and was then moved back and forth on every sync.
    #[test]
    fn additions_under_an_existing_subgroup_land_in_it() {
        let mut master = Master::new("Production");
        let web = master.group("Web", master.root_id);
        master.connection("nginx-1", web);
        let mut import = Import::new();
        import.sync(&master.export());

        let prod = master.group("Prod", web);
        master.connection("nginx-2", web);
        master.connection("api-1", prod);
        import.sync(&master.export());

        assert_eq!(import.group_count("Web"), 1, "Web must not be duplicated");
        assert_eq!(import.path_of("nginx-2"), "master-file/Web");
        assert_eq!(import.path_of("api-1"), "master-file/Web/Prod");
        assert!(import.merge(&master.export()).is_empty());
    }

    /// Issue #263 end to end: export → import (create) → rename and move on
    /// the Master → re-export → merge is an update of the same local
    /// connection, not a delete + create, and the group keeps its id too.
    #[test]
    fn master_rename_and_move_update_the_imported_copy_in_place() {
        let mut master = Master::new("Production");
        let web = master.group("Web", master.root_id);
        let db = master.group("DB", master.root_id);
        let conn = master.connection("db-prod", web);

        let mut import = Import::new();
        import.sync(&master.export());
        let local_conn_id = import.connections[0].id;
        assert_ne!(local_conn_id, conn, "Import never adopts the Master's id");
        assert_eq!(import.connections[0].sync_origin_id, Some(conn));
        let local_web_id = import.groups.iter().find(|g| g.name == "Web").unwrap().id;

        // Master: rename the connection and move it to DB; rename Web and
        // move it under DB.
        for c in &mut master.connections {
            c.name = "db-production".to_owned();
            c.group_id = Some(db);
            c.updated_at = Utc::now();
        }
        for g in &mut master.groups {
            if g.id == web {
                g.name = "Frontend".to_owned();
                g.parent_id = Some(db);
            }
        }

        let result = import.merge(&master.export());
        assert_eq!(result.connections_to_update.len(), 1);
        assert_eq!(result.connections_to_update[0].0, local_conn_id);
        assert!(result.connections_to_create.is_empty());
        assert!(result.connections_to_delete.is_empty());
        assert_eq!(result.groups_to_update.len(), 1);
        assert_eq!(result.groups_to_update[0].0, local_web_id);
        assert!(result.groups_to_create.is_empty());
        assert!(result.groups_to_delete.is_empty());

        import.sync(&master.export());
        assert_eq!(import.connections.len(), 1);
        assert_eq!(import.connections[0].id, local_conn_id);
        assert_eq!(import.path_of("db-production"), "master-file/DB");
        let frontend = import.groups.iter().find(|g| g.name == "Frontend").unwrap();
        assert_eq!(frontend.id, local_web_id);
        assert_eq!(
            compute_group_path(frontend.id, &import.groups),
            "master-file/DB/Frontend"
        );
        assert!(import.merge(&master.export()).is_empty());
    }

    /// A kept connection whose old group goes away must be moved before the
    /// group is deleted — deleting first ungrouped it, put it outside the
    /// Import tree, and the next sync created it a second time.
    #[test]
    fn a_connection_moved_out_of_a_deleted_group_survives() {
        let mut master = Master::new("Production");
        let old = master.group("Old", master.root_id);
        let new = master.group("New", master.root_id);
        master.connection("keep", old);
        let mut import = Import::new();
        import.sync(&master.export());
        let kept_id = import.connections[0].id;

        master.groups.retain(|g| g.id != old);
        for c in &mut master.connections {
            c.group_id = Some(new);
        }
        import.sync(&master.export());

        assert_eq!(import.connections.len(), 1);
        assert_eq!(import.connections[0].id, kept_id);
        assert_eq!(import.path_of("keep"), "master-file/New");
        assert_eq!(import.group_count("Old"), 0);
        assert!(import.merge(&master.export()).is_empty());
    }

    /// A tree imported before 0.23 has no `sync_origin_id`. The first sync
    /// matches it by name and path and links it; nothing is recreated, and
    /// the sync after that matches by id.
    #[test]
    fn a_tree_imported_before_0_23_is_linked_not_recreated() {
        let mut master = Master::new("Production");
        let web = master.group("Web", master.root_id);
        master.connection("nginx-1", web);
        let mut import = Import::new();
        import.sync(&master.export());
        for c in &mut import.connections {
            c.sync_origin_id = None;
        }
        for g in &mut import.groups {
            g.sync_origin_id = None;
        }
        let conn_id = import.connections[0].id;

        let result = import.sync(&master.export());
        assert_eq!(result.connections_to_link.len(), 1);
        assert_eq!(result.groups_to_link.len(), 1);
        assert!(result.connections_to_create.is_empty());
        assert!(result.connections_to_update.is_empty());
        assert!(result.groups_to_update.is_empty());
        assert_eq!(import.connections[0].id, conn_id);
        assert!(import.connections[0].sync_origin_id.is_some());
        assert!(import.merge(&master.export()).is_empty());
    }

    /// A created group keeps the Master's description, icon and inherited
    /// settings instead of arriving empty.
    #[test]
    fn a_created_group_carries_its_synced_fields() {
        let mut master = Master::new("Production");
        let web = master.group("Web", master.root_id);
        for g in &mut master.groups {
            if g.id == web {
                g.description = Some("front".to_owned());
                g.icon = Some("🌐".to_owned());
                g.username = Some("deploy".to_owned());
                g.ssh_proxy_jump = Some("bastion".to_owned());
            }
        }
        let mut import = Import::new();
        import.sync(&master.export());
        let created = import.groups.iter().find(|g| g.name == "Web").unwrap();
        assert_eq!(created.description.as_deref(), Some("front"));
        assert_eq!(created.icon.as_deref(), Some("🌐"));
        assert_eq!(created.username.as_deref(), Some("deploy"));
        assert_eq!(created.ssh_proxy_jump.as_deref(), Some("bastion"));
        assert_eq!(created.sync_origin_id, Some(web));
    }

    /// A group whose name contains `/` stays one group: its parent is found
    /// by removing the name from the end of the path, not by splitting at
    /// the last `/`.
    #[test]
    fn a_slash_in_a_group_name_does_not_split_it() {
        let mut master = Master::new("Production");
        let odd = master.group("eu/west", master.root_id);
        master.connection("lb", odd);
        let mut import = Import::new();
        import.sync(&master.export());

        assert_eq!(import.group_count("eu/west"), 1);
        assert_eq!(import.group_count("eu"), 0);
        assert_eq!(import.group_count("west"), 0);
        assert_eq!(import.path_of("lb"), "master-file/eu/west");
        assert!(import.merge(&master.export()).is_empty());
    }

    /// `sync_origin_id` is optional on disk: configs written before 0.23 load
    /// with `None`, an unlinked entity does not write the key, and a linked
    /// one round-trips.
    #[test]
    fn sync_origin_id_is_optional_in_saved_configs() {
        let mut conn = Connection::new_ssh("a".to_owned(), "h".to_owned(), 22);
        let mut group = ConnectionGroup::new("g".to_owned());
        let conn_toml = toml::to_string(&conn).unwrap();
        let group_toml = toml::to_string(&group).unwrap();
        assert!(!conn_toml.contains("sync_origin_id"));
        assert!(!group_toml.contains("sync_origin_id"));
        let old_conn: Connection = toml::from_str(&conn_toml).unwrap();
        let old_group: ConnectionGroup = toml::from_str(&group_toml).unwrap();
        assert_eq!(old_conn.sync_origin_id, None);
        assert_eq!(old_group.sync_origin_id, None);

        let origin = Uuid::new_v4();
        conn.sync_origin_id = Some(origin);
        group.sync_origin_id = Some(origin);
        let conn_back: Connection = toml::from_str(&toml::to_string(&conn).unwrap()).unwrap();
        let group_back: ConnectionGroup =
            toml::from_str(&toml::to_string(&group).unwrap()).unwrap();
        assert_eq!(conn_back.sync_origin_id, Some(origin));
        assert_eq!(group_back.sync_origin_id, Some(origin));
    }

    /// Two remote entries with the same id (a damaged file): the first is
    /// matched by id, the second by name, and neither is dropped.
    #[test]
    fn a_repeated_remote_id_matches_one_local_only() {
        let mut master = Master::new("Production");
        master.connection("a", master.root_id);
        let mut export = master.export();
        let mut twin = export.connections[0].clone();
        twin.name = "b".to_owned();
        export.connections.push(twin);

        let mut import = Import::new();
        import.sync(&export);
        assert_eq!(import.connections.len(), 2);
        let linked: Vec<_> = import
            .connections
            .iter()
            .filter(|c| c.sync_origin_id.is_some())
            .collect();
        assert_eq!(linked.len(), 1, "one Master id links one connection");
        assert_eq!(linked[0].name, "a");
        assert!(import.merge(&export).is_empty());
    }
}
