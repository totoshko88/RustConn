//! Lazy loading for connection groups
//!
//! This module provides the `LazyGroupLoader` which tracks which groups have been
//! loaded and provides methods to load children on demand. This improves startup
//! performance for large connection databases by only loading root-level items
//! initially and loading children when groups are expanded.

use std::collections::HashSet;
use uuid::Uuid;

use crate::models::{Connection, ConnectionGroup};

/// Tracks which groups have been loaded for lazy loading
///
/// The `LazyGroupLoader` maintains state about which groups have had their
/// children loaded. This allows the sidebar to load only root-level items
/// initially and load children on demand when groups are expanded.
#[derive(Debug, Clone, Default)]
pub struct LazyGroupLoader {
    /// Set of group IDs that have been loaded
    loaded_groups: HashSet<Uuid>,
    /// Whether root-level items have been loaded
    root_loaded: bool,
}

impl LazyGroupLoader {
    /// Creates a new `LazyGroupLoader` with no groups loaded
    #[must_use]
    pub fn new() -> Self {
        Self {
            loaded_groups: HashSet::new(),
            root_loaded: false,
        }
    }

    /// Returns whether root-level items have been loaded
    #[must_use]
    pub const fn is_root_loaded(&self) -> bool {
        self.root_loaded
    }

    /// Marks root-level items as loaded
    pub const fn mark_root_loaded(&mut self) {
        self.root_loaded = true;
    }

    /// Returns whether a specific group's children have been loaded
    ///
    /// # Arguments
    ///
    /// * `group_id` - The UUID of the group to check
    #[must_use]
    pub fn is_group_loaded(&self, group_id: Uuid) -> bool {
        self.loaded_groups.contains(&group_id)
    }

    /// Marks a group's children as loaded
    ///
    /// # Arguments
    ///
    /// * `group_id` - The UUID of the group to mark as loaded
    pub fn mark_group_loaded(&mut self, group_id: Uuid) {
        self.loaded_groups.insert(group_id);
    }

    /// Marks a group as unloaded (for re-loading)
    ///
    /// # Arguments
    ///
    /// * `group_id` - The UUID of the group to mark as unloaded
    pub fn mark_group_unloaded(&mut self, group_id: Uuid) {
        self.loaded_groups.remove(&group_id);
    }

    /// Resets all loading state
    ///
    /// This clears all loaded groups and resets `root_loaded` to false.
    /// Useful when the connection database is reloaded.
    pub fn reset(&mut self) {
        self.loaded_groups.clear();
        self.root_loaded = false;
    }

    /// Returns the number of groups that have been loaded
    #[must_use]
    pub fn loaded_count(&self) -> usize {
        self.loaded_groups.len()
    }

    /// Gets the children to load for a specific group
    ///
    /// Returns the child groups and connections that belong to the specified
    /// parent group. This filters from the full set of groups and connections.
    ///
    /// # Arguments
    ///
    /// * `group_id` - The UUID of the parent group
    /// * `all_groups` - All groups in the database
    /// * `all_connections` - All connections in the database
    ///
    /// # Returns
    ///
    /// A tuple of (`child_groups`, `child_connections`) for the specified parent
    #[must_use]
    pub fn get_children_to_load<'a>(
        &self,
        group_id: Uuid,
        all_groups: &'a [ConnectionGroup],
        all_connections: &'a [Connection],
    ) -> (Vec<&'a ConnectionGroup>, Vec<&'a Connection>) {
        let child_groups: Vec<&'a ConnectionGroup> = all_groups
            .iter()
            .filter(|g| g.parent_id == Some(group_id))
            .collect();

        let child_connections: Vec<&'a Connection> = all_connections
            .iter()
            .filter(|c| c.group_id == Some(group_id))
            .collect();

        (child_groups, child_connections)
    }

    /// Gets root-level items to load
    ///
    /// Returns root-level groups (those with no parent) and ungrouped connections.
    ///
    /// # Arguments
    ///
    /// * `all_groups` - All groups in the database
    /// * `all_connections` - All connections in the database
    ///
    /// # Returns
    ///
    /// A tuple of (`root_groups`, `ungrouped_connections`)
    #[must_use]
    pub fn get_root_items_to_load<'a>(
        &self,
        all_groups: &'a [ConnectionGroup],
        all_connections: &'a [Connection],
    ) -> (Vec<&'a ConnectionGroup>, Vec<&'a Connection>) {
        let root_groups: Vec<&'a ConnectionGroup> = all_groups
            .iter()
            .filter(|g| g.parent_id.is_none())
            .collect();

        let ungrouped_connections: Vec<&'a Connection> = all_connections
            .iter()
            .filter(|c| c.group_id.is_none())
            .collect();

        (root_groups, ungrouped_connections)
    }

    /// Checks if a group needs to be loaded
    ///
    /// Returns true if the group has not been loaded yet.
    ///
    /// # Arguments
    ///
    /// * `group_id` - The UUID of the group to check
    #[must_use]
    pub fn needs_loading(&self, group_id: Uuid) -> bool {
        !self.loaded_groups.contains(&group_id)
    }
}

#[cfg(test)]
#[allow(clippy::similar_names)]
mod tests {
    use super::*;
    use crate::models::ProtocolConfig;

    fn create_test_group(name: &str, parent_id: Option<Uuid>) -> ConnectionGroup {
        let mut group = ConnectionGroup::new(name.to_string());
        group.parent_id = parent_id;
        group
    }

    fn create_test_connection(name: &str, group_id: Option<Uuid>) -> Connection {
        let mut conn = Connection::new(
            name.to_string(),
            "localhost".to_string(),
            22,
            ProtocolConfig::Ssh(crate::models::SshConfig::default()),
        );
        conn.group_id = group_id;
        conn
    }

    #[test]
    fn test_new_loader_has_nothing_loaded() {
        let loader = LazyGroupLoader::new();
        assert!(!loader.is_root_loaded());
        assert_eq!(loader.loaded_count(), 0);
    }

    #[test]
    fn test_mark_root_loaded() {
        let mut loader = LazyGroupLoader::new();
        assert!(!loader.is_root_loaded());

        loader.mark_root_loaded();
        assert!(loader.is_root_loaded());
    }

    #[test]
    fn test_mark_group_loaded() {
        let mut loader = LazyGroupLoader::new();
        let group_id = Uuid::new_v4();

        assert!(!loader.is_group_loaded(group_id));
        assert!(loader.needs_loading(group_id));

        loader.mark_group_loaded(group_id);

        assert!(loader.is_group_loaded(group_id));
        assert!(!loader.needs_loading(group_id));
        assert_eq!(loader.loaded_count(), 1);
    }

    #[test]
    fn test_mark_group_unloaded() {
        let mut loader = LazyGroupLoader::new();
        let group_id = Uuid::new_v4();

        loader.mark_group_loaded(group_id);
        assert!(loader.is_group_loaded(group_id));

        loader.mark_group_unloaded(group_id);
        assert!(!loader.is_group_loaded(group_id));
    }

    #[test]
    fn test_reset() {
        let mut loader = LazyGroupLoader::new();
        let group_id = Uuid::new_v4();

        loader.mark_root_loaded();
        loader.mark_group_loaded(group_id);

        assert!(loader.is_root_loaded());
        assert!(loader.is_group_loaded(group_id));

        loader.reset();

        assert!(!loader.is_root_loaded());
        assert!(!loader.is_group_loaded(group_id));
        assert_eq!(loader.loaded_count(), 0);
    }

    #[test]
    fn test_get_root_items_to_load() {
        let loader = LazyGroupLoader::new();

        let root_group = create_test_group("Root", None);
        let child_group = create_test_group("Child", Some(root_group.id));
        let ungrouped_conn = create_test_connection("Ungrouped", None);
        let grouped_conn = create_test_connection("Grouped", Some(root_group.id));

        let all_groups = vec![root_group.clone(), child_group];
        let all_connections = vec![ungrouped_conn.clone(), grouped_conn];

        let (root_groups, ungrouped_connections) =
            loader.get_root_items_to_load(&all_groups, &all_connections);

        assert_eq!(root_groups.len(), 1);
        assert_eq!(root_groups[0].id, root_group.id);

        assert_eq!(ungrouped_connections.len(), 1);
        assert_eq!(ungrouped_connections[0].id, ungrouped_conn.id);
    }

    #[test]
    fn test_get_children_to_load() {
        let loader = LazyGroupLoader::new();

        let root_group = create_test_group("Root", None);
        let child_group1 = create_test_group("Child1", Some(root_group.id));
        let child_group2 = create_test_group("Child2", Some(root_group.id));
        let grandchild_group = create_test_group("Grandchild", Some(child_group1.id));

        let conn1 = create_test_connection("Conn1", Some(root_group.id));
        let conn2 = create_test_connection("Conn2", Some(root_group.id));
        let conn3 = create_test_connection("Conn3", Some(child_group1.id));

        let all_groups = vec![
            root_group.clone(),
            child_group1.clone(),
            child_group2.clone(),
            grandchild_group,
        ];
        let all_connections = vec![conn1.clone(), conn2.clone(), conn3];

        let (child_groups, child_connections) =
            loader.get_children_to_load(root_group.id, &all_groups, &all_connections);

        assert_eq!(child_groups.len(), 2);
        assert!(child_groups.iter().any(|g| g.id == child_group1.id));
        assert!(child_groups.iter().any(|g| g.id == child_group2.id));

        assert_eq!(child_connections.len(), 2);
        assert!(child_connections.iter().any(|c| c.id == conn1.id));
        assert!(child_connections.iter().any(|c| c.id == conn2.id));
    }

    #[test]
    fn test_multiple_groups_loaded() {
        let mut loader = LazyGroupLoader::new();
        let group1 = Uuid::new_v4();
        let group2 = Uuid::new_v4();
        let group3 = Uuid::new_v4();

        loader.mark_group_loaded(group1);
        loader.mark_group_loaded(group2);

        assert!(loader.is_group_loaded(group1));
        assert!(loader.is_group_loaded(group2));
        assert!(!loader.is_group_loaded(group3));
        assert_eq!(loader.loaded_count(), 2);
    }
}
