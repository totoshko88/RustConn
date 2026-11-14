//! Plans the copy of a whole group: its subgroups and its connections.
//!
//! The sidebar's **Duplicate** on a group builds a plan here and the GUI applies
//! it. The plan is plain data, so what is and is not carried over is settled
//! (and tested) once, away from any widget.

use std::collections::HashMap;

use chrono::Utc;
use uuid::Uuid;

use crate::models::{Connection, ConnectionGroup, PasswordSource};
use crate::sync::SyncMode;

/// A connection and the copy made of it.
///
/// The original is kept next to the copy because a vault credential is keyed by
/// both: copying the secret needs both sides of the pair.
#[derive(Debug, Clone)]
pub struct ConnectionCopy {
    /// The connection as it was.
    pub original: Connection,
    /// Its copy, in the copied group tree.
    pub copy: Connection,
}

/// Everything to create for one group copy.
#[derive(Debug, Clone)]
pub struct GroupCopyPlan {
    /// The copied groups, the copy of the chosen group first and every parent
    /// before its children, so they can be created in order.
    pub groups: Vec<ConnectionGroup>,
    /// The copied connections, each already placed in its copied group.
    pub connections: Vec<ConnectionCopy>,
}

/// Plans a copy of the group `root` with everything beneath it.
///
/// `new_root_name` names the copy of `root`. `copy_name` names the copy of each
/// connection; it sees the original, so a caller can keep names unique.
///
/// The copy is a plain folder tree, so what belongs to the original's
/// *relationship with the outside* is not carried over:
///
/// * Cloud Sync (`sync_mode`, `sync_file`, `last_synced_at`, `sync_origin_id`): a
///   second Master on the same file would fight the first. A Master copies as an
///   ordinary group.
/// * A saved group password (`PasswordSource::Vault`): the vault entry belongs to
///   the original group, and the copy would claim one it does not have. It falls
///   back to `PasswordSource::None`; the username and domain stay.
/// * Dynamic connections: a refresh regenerates them.
///
/// Returns `None` when `root` does not exist, is a Cloud Sync Import group (its
/// content mirrors the source) or is a dynamic folder (its content is generated).
#[must_use]
pub fn plan_group_copy(
    groups: &[ConnectionGroup],
    connections: &[Connection],
    root: Uuid,
    new_root_name: &str,
    mut copy_name: impl FnMut(&Connection) -> String,
) -> Option<GroupCopyPlan> {
    let root_group = groups.iter().find(|g| g.id == root)?;
    if root_group.sync_mode == SyncMode::Import || root_group.dynamic_folder.is_some() {
        return None;
    }

    let by_id: HashMap<Uuid, &ConnectionGroup> = groups.iter().map(|g| (g.id, g)).collect();
    let tree = crate::dynamic_folder::descendant_group_ids(root, groups);
    let new_ids: HashMap<Uuid, Uuid> = tree.iter().map(|id| (*id, Uuid::new_v4())).collect();
    let now = Utc::now();

    let copied_groups = tree
        .iter()
        .filter_map(|id| by_id.get(id).map(|g| (*id, *g)))
        .map(|(id, original)| {
            let is_root = id == root;
            let mut copy = original.clone();
            copy.id = new_ids[&id];
            copy.created_at = now;
            copy.updated_at = now;
            if is_root {
                // Beside the original: same parent, right after it.
                new_root_name.clone_into(&mut copy.name);
                copy.sort_order = original.sort_order.saturating_add(1);
            } else {
                copy.parent_id = original
                    .parent_id
                    .and_then(|parent| new_ids.get(&parent).copied());
            }
            copy.sync_mode = SyncMode::None;
            copy.sync_file = None;
            copy.last_synced_at = None;
            copy.sync_origin_id = None;
            if copy.password_source == Some(PasswordSource::Vault) {
                copy.password_source = Some(PasswordSource::None);
            }
            copy
        })
        .collect();

    let copied_connections = connections
        .iter()
        .filter(|c| !c.is_dynamic)
        .filter_map(|c| {
            let new_group = new_ids.get(&c.group_id?)?;
            let mut copy = c.duplicate_as(copy_name(c));
            copy.group_id = Some(*new_group);
            Some(ConnectionCopy {
                original: c.clone(),
                copy,
            })
        })
        .collect();

    Some(GroupCopyPlan {
        groups: copied_groups,
        connections: copied_connections,
    })
}

/// A name for a copy: `"{name} (copy)"`, or `"{name} (copy 2)"`, `"… 3"` and so
/// on until `is_taken` no longer claims it.
#[must_use]
pub fn unique_copy_name(name: &str, is_taken: impl Fn(&str) -> bool) -> String {
    let first = format!("{name} (copy)");
    if !is_taken(&first) {
        return first;
    }
    // Bounded so a predicate that claims every name cannot hang the caller;
    // past that the plain "(copy)" form is returned and left to validation.
    (2_u32..=u32::from(u16::MAX))
        .map(|n| format!("{name} (copy {n})"))
        .find(|candidate| !is_taken(candidate))
        .unwrap_or(first)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ConnectionGroup, PasswordSource};

    fn group(name: &str, parent: Option<Uuid>) -> ConnectionGroup {
        let mut g = ConnectionGroup::new(name.to_owned());
        g.parent_id = parent;
        g
    }

    fn ssh_in(name: &str, group_id: Option<Uuid>) -> Connection {
        let mut c = Connection::new_ssh(name.to_owned(), "h".to_owned(), 22);
        c.group_id = group_id;
        c
    }

    /// root ─ child ─ grandchild, with one connection in each, plus an outsider.
    fn tree() -> (Vec<ConnectionGroup>, Vec<Connection>, Uuid) {
        let root = group("root", None);
        let child = group("child", Some(root.id));
        let grandchild = group("grandchild", Some(child.id));
        let other = group("other", None);
        let connections = vec![
            ssh_in("in-root", Some(root.id)),
            ssh_in("in-child", Some(child.id)),
            ssh_in("in-grandchild", Some(grandchild.id)),
            ssh_in("elsewhere", Some(other.id)),
            ssh_in("ungrouped", None),
        ];
        let root_id = root.id;
        (vec![root, child, grandchild, other], connections, root_id)
    }

    fn plan(
        groups: &[ConnectionGroup],
        connections: &[Connection],
        root: Uuid,
    ) -> Option<GroupCopyPlan> {
        plan_group_copy(groups, connections, root, "root (copy)", |c| {
            format!("{} (copy)", c.name)
        })
    }

    #[test]
    fn copies_the_whole_subtree_and_nothing_else() {
        let (groups, connections, root) = tree();
        let plan = plan(&groups, &connections, root).unwrap();

        let names: Vec<_> = plan.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["root (copy)", "child", "grandchild"]);
        let mut copied: Vec<_> = plan
            .connections
            .iter()
            .map(|c| c.copy.name.as_str())
            .collect();
        copied.sort_unstable();
        assert_eq!(
            copied,
            ["in-child (copy)", "in-grandchild (copy)", "in-root (copy)"]
        );
    }

    #[test]
    fn the_copy_hangs_together_under_new_ids() {
        let (groups, connections, root) = tree();
        let plan = plan(&groups, &connections, root).unwrap();

        // The copy of the root sits beside the original; the rest nest under copies.
        assert_eq!(plan.groups[0].parent_id, None);
        assert_eq!(plan.groups[1].parent_id, Some(plan.groups[0].id));
        assert_eq!(plan.groups[2].parent_id, Some(plan.groups[1].id));

        // Every id is new, so nothing collides with the original tree.
        let old_ids: Vec<_> = groups.iter().map(|g| g.id).collect();
        assert!(plan.groups.iter().all(|g| !old_ids.contains(&g.id)));

        // Each connection lands in the copy of its own group.
        for pair in &plan.connections {
            let original_group = groups
                .iter()
                .position(|g| Some(g.id) == pair.original.group_id)
                .unwrap();
            assert_eq!(
                pair.copy.group_id,
                Some(plan.groups[original_group].id),
                "{} is in the wrong copy",
                pair.copy.name
            );
            assert_ne!(pair.copy.id, pair.original.id);
        }
    }

    #[test]
    fn a_copy_of_a_nested_group_keeps_its_parent() {
        let (groups, connections, _) = tree();
        let child = groups.iter().find(|g| g.name == "child").unwrap().id;
        let plan = plan(&groups, &connections, child).unwrap();

        // The root of this copy stays inside `root`, beside the original `child`.
        assert_eq!(plan.groups[0].parent_id, groups[1].parent_id);
        assert_eq!(plan.groups.len(), 2);
    }

    #[test]
    fn cloud_sync_links_are_not_copied() {
        let (mut groups, connections, root) = tree();
        groups[0].sync_mode = SyncMode::Master;
        groups[0].sync_file = Some("root.rcn".to_owned());
        groups[0].last_synced_at = Some(Utc::now());
        groups[0].sync_origin_id = Some(Uuid::new_v4());

        let copy = &plan(&groups, &connections, root).unwrap().groups[0];

        assert_eq!(copy.sync_mode, SyncMode::None);
        assert_eq!(copy.sync_file, None);
        assert_eq!(copy.last_synced_at, None);
        assert_eq!(copy.sync_origin_id, None);
    }

    #[test]
    fn a_vault_password_is_not_claimed_by_the_copy() {
        let (mut groups, connections, root) = tree();
        groups[0].username = Some("admin".to_owned());
        groups[0].password_source = Some(PasswordSource::Vault);

        let copy = &plan(&groups, &connections, root).unwrap().groups[0];

        assert_eq!(copy.password_source, Some(PasswordSource::None));
        assert_eq!(copy.username.as_deref(), Some("admin"));
    }

    #[test]
    fn dynamic_connections_are_left_to_the_refresh() {
        let (groups, mut connections, root) = tree();
        connections[0].is_dynamic = true;

        let plan = plan(&groups, &connections, root).unwrap();

        assert!(
            plan.connections
                .iter()
                .all(|c| c.original.name != "in-root")
        );
    }

    #[test]
    fn import_groups_and_unknown_ids_are_refused() {
        let (mut groups, connections, root) = tree();
        assert!(plan(&groups, &connections, Uuid::new_v4()).is_none());

        groups[0].sync_mode = SyncMode::Import;
        assert!(plan(&groups, &connections, root).is_none());
    }

    #[test]
    fn copy_names_count_up_until_one_is_free() {
        let taken = ["a (copy)", "a (copy 2)"];
        assert_eq!(unique_copy_name("b", |n| taken.contains(&n)), "b (copy)");
        assert_eq!(unique_copy_name("a", |n| taken.contains(&n)), "a (copy 3)");
        assert_eq!(unique_copy_name("a", |_| false), "a (copy)");
    }
}
