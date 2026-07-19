//! Group Sync merge engine for Import mode.
//!
//! [`GroupMergeEngine`] computes a diff between the local group tree and a
//! remote [`GroupSyncExport`], producing a [`GroupMergeResult`] that describes
//! which connections, groups, and variable templates need to be created,
//! updated, or deleted locally.
//!
//! The merge algorithm uses **name + group_path** as the primary key for
//! connections and **path** as the primary key for groups. Conflict resolution
//! is timestamp-based: if both sides have a connection with the same name, the
//! one with the newer `updated_at` wins.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use super::group_export::{GroupSyncExport, SyncConnection, SyncGroup, compute_group_path};
use super::variable_template::VariableTemplate;
use crate::models::{Connection, ConnectionGroup};

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
    /// Remote groups (by path) not present locally — should be created.
    pub groups_to_create: Vec<SyncGroup>,
    /// Local groups (by path) not present in the remote export — should be deleted.
    pub groups_to_delete: Vec<Uuid>,
    /// Remote variable templates not present locally — should be created.
    pub variables_to_create: Vec<VariableTemplate>,
}

/// Composite key for connection lookup: `(name, group_path)`.
type ConnectionKey = (String, String);

impl GroupMergeEngine {
    /// Computes the diff between local state and a remote [`GroupSyncExport`].
    ///
    /// # Algorithm
    ///
    /// 1. **Phase 1 — Groups by path**: remote paths not in local → create;
    ///    local paths not in remote → delete.
    /// 2. **Phase 2 — Connections by (name, group_path)**: remote not in local
    ///    → create; local not in remote → delete; both exist and
    ///    `remote.updated_at > local.updated_at` → update.
    /// 3. **Phase 3 — Variable templates**: remote templates whose name is not
    ///    found among `local_variable_names` → create.
    ///
    /// # Arguments
    ///
    /// * `local_groups` — all local groups belonging to the Import root group
    ///   (including the root itself).
    /// * `local_connections` — all local connections belonging to those groups.
    /// * `remote` — the parsed remote export file.
    /// * `local_variable_names` — names of variables that already exist locally.
    #[must_use]
    pub fn merge(
        local_groups: &[ConnectionGroup],
        local_connections: &[Connection],
        remote: &GroupSyncExport,
        local_variable_names: &HashSet<String>,
    ) -> GroupMergeResult {
        let mut result = GroupMergeResult::default();

        // --- Phase 1: Merge groups by path ---
        Self::merge_groups(local_groups, remote, &mut result);

        // --- Phase 2: Merge connections by (name, group_path) ---
        Self::merge_connections(local_groups, local_connections, remote, &mut result);

        // --- Phase 3: Variable templates ---
        Self::merge_variables(
            &remote.variable_templates,
            local_variable_names,
            &mut result,
        );

        result
    }

    /// Phase 1: diff groups by hierarchical path.
    fn merge_groups(
        local_groups: &[ConnectionGroup],
        remote: &GroupSyncExport,
        result: &mut GroupMergeResult,
    ) {
        // Build set of remote paths (subgroups only, not root).
        let remote_paths: HashSet<&str> = remote.groups.iter().map(|g| g.path.as_str()).collect();

        // Build map of local paths → group id (subgroups only).
        let local_path_map: HashMap<String, Uuid> = local_groups
            .iter()
            .filter(|g| g.parent_id.is_some())
            .map(|g| (compute_group_path(g.id, local_groups), g.id))
            .collect();

        let local_paths: HashSet<&str> = local_path_map.keys().map(String::as_str).collect();

        // New remote paths → groups_to_create
        for remote_group in &remote.groups {
            if !local_paths.contains(remote_group.path.as_str()) {
                result.groups_to_create.push(remote_group.clone());
            }
        }

        // Missing remote paths → groups_to_delete
        for (path, group_id) in &local_path_map {
            if !remote_paths.contains(path.as_str()) {
                result.groups_to_delete.push(*group_id);
            }
        }
    }

    /// Phase 2: diff connections by `(name, group_path)`.
    fn merge_connections(
        local_groups: &[ConnectionGroup],
        local_connections: &[Connection],
        remote: &GroupSyncExport,
        result: &mut GroupMergeResult,
    ) {
        // Index remote connections by (name, group_path).
        let remote_by_key: HashMap<ConnectionKey, &SyncConnection> = remote
            .connections
            .iter()
            .map(|c| ((c.name.clone(), c.group_path.clone()), c))
            .collect();

        // Build a group_id → path lookup for local connections.
        let group_path_lookup: HashMap<Uuid, String> = local_groups
            .iter()
            .map(|g| (g.id, compute_group_path(g.id, local_groups)))
            .collect();

        // Index local connections by (name, group_path).
        let local_by_key: HashMap<ConnectionKey, &Connection> = local_connections
            .iter()
            .map(|c| {
                let path = c
                    .group_id
                    .and_then(|gid| group_path_lookup.get(&gid))
                    .cloned()
                    .unwrap_or_default();
                ((c.name.clone(), path), c)
            })
            .collect();

        // Remote connections not in local → create.
        // Remote connections in local with newer updated_at → update.
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

        // Local connections not in remote → delete.
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

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};

    use super::*;
    use crate::models::{
        AutomationConfig, PasswordSource, ProtocolConfig, ProtocolType, SshConfig,
    };

    /// Helper: create a minimal `SyncConnection`.
    fn make_sync_conn(name: &str, group_path: &str) -> SyncConnection {
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
            updated_at: Utc::now(),
        }
    }

    /// Helper: create a minimal `SyncGroup`.
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
        let result = GroupMergeEngine::merge(&[root], &[], &export, &HashSet::new());

        assert_eq!(result.groups_to_create.len(), 1);
        assert_eq!(result.groups_to_create[0].path, "Root/Web");
        assert!(result.groups_to_delete.is_empty());
    }

    #[test]
    fn missing_remote_group_is_deleted() {
        let root = make_local_group("Root", None);
        let child = make_local_group("OldGroup", Some(root.id));
        let export = make_export(vec![], vec![], vec![]);

        let result = GroupMergeEngine::merge(&[root, child.clone()], &[], &export, &HashSet::new());

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

        let result = GroupMergeEngine::merge(&[root, child], &[], &export, &HashSet::new());

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

        let result = GroupMergeEngine::merge(&[root], &[], &export, &HashSet::new());

        assert_eq!(result.connections_to_create.len(), 1);
        assert_eq!(result.connections_to_create[0].name, "nginx-1");
    }

    #[test]
    fn missing_remote_connection_is_deleted() {
        let root = make_local_group("Root", None);
        let local_conn = make_local_conn("old-server", root.id);
        let export = make_export(vec![], vec![], vec![]);

        let result = GroupMergeEngine::merge(
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
        let result = GroupMergeEngine::merge(&[root], &[local_conn], &export, &HashSet::new());

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
        let result = GroupMergeEngine::merge(&[root], &[local_conn], &export, &HashSet::new());

        assert!(result.connections_to_update.is_empty());
        assert!(result.connections_to_create.is_empty());
        assert!(result.connections_to_delete.is_empty());
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

        let result = GroupMergeEngine::merge(&[], &[], &export, &HashSet::new());

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
        let result = GroupMergeEngine::merge(&[], &[], &export, &local_vars);

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
}
