//! `KeePass` hierarchy management for connection password storage
//!
//! This module provides hierarchical storage of passwords in `KeePass` databases,
//! mirroring the connection group structure in `RustConn`.

use std::collections::HashSet;

use uuid::Uuid;

use crate::models::{Connection, ConnectionGroup};

/// Root group name for all `RustConn` entries in `KeePass`
pub const KEEPASS_ROOT_GROUP: &str = "RustConn";

/// Subfolder for group credentials in `KeePass`
pub const GROUPS_SUBFOLDER: &str = "Groups";

/// Separator used in `KeePass` entry paths
pub const PATH_SEPARATOR: char = '/';

/// Result of ensuring groups exist
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupCreationResult {
    /// Groups that already existed
    pub existing_groups: Vec<String>,
    /// Groups that were created
    pub created_groups: Vec<String>,
}

impl GroupCreationResult {
    /// Creates a new empty result
    #[must_use]
    pub const fn new() -> Self {
        Self {
            existing_groups: Vec::new(),
            created_groups: Vec::new(),
        }
    }

    /// Returns true if any groups were created
    #[must_use]
    pub fn any_created(&self) -> bool {
        !self.created_groups.is_empty()
    }

    /// Returns the total number of groups processed
    #[must_use]
    pub fn total_groups(&self) -> usize {
        self.existing_groups.len() + self.created_groups.len()
    }
}

impl Default for GroupCreationResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Manages hierarchical `KeePass` entry paths based on connection group structure
#[derive(Debug, Clone)]
pub struct KeePassHierarchy;

impl KeePassHierarchy {
    /// Builds the full `KeePass` entry path for a connection based on its group hierarchy.
    ///
    /// The path format is: `RustConn/GroupA/SubGroup/ConnectionName`
    ///
    /// # Arguments
    /// * `connection` - The connection to build a path for
    /// * `groups` - All available connection groups for hierarchy resolution
    ///
    /// # Returns
    /// A string representing the full hierarchical path for the `KeePass` entry
    ///
    /// # Example
    /// ```
    /// use rustconn_core::secret::hierarchy::KeePassHierarchy;
    /// use rustconn_core::models::{Connection, ConnectionGroup, ProtocolConfig, SshConfig};
    /// use uuid::Uuid;
    ///
    /// // Create a group hierarchy: Production -> Web Servers
    /// let production_group = ConnectionGroup::new("Production".to_string());
    /// let mut web_group = ConnectionGroup::with_parent("Web Servers".to_string(), production_group.id);
    ///
    /// let groups = vec![production_group.clone(), web_group.clone()];
    ///
    /// // Create a connection in the Web Servers group
    /// let mut connection = Connection::new(
    ///     "nginx-01".to_string(),
    ///     "192.168.1.10".to_string(),
    ///     22,
    ///     ProtocolConfig::Ssh(SshConfig::default()),
    /// );
    /// connection.group_id = Some(web_group.id);
    ///
    /// let path = KeePassHierarchy::build_entry_path(&connection, &groups);
    /// assert!(path.starts_with("RustConn/"));
    /// assert!(path.contains("Production"));
    /// assert!(path.contains("Web Servers"));
    /// assert!(path.ends_with("nginx-01"));
    /// ```
    #[must_use]
    pub fn build_entry_path(connection: &Connection, groups: &[ConnectionGroup]) -> String {
        let mut path_parts = vec![KEEPASS_ROOT_GROUP.to_string()];

        // Build path from connection's group hierarchy
        if let Some(group_id) = connection.group_id {
            let group_path = Self::resolve_group_path(group_id, groups);
            path_parts.extend(group_path);
        }

        // Use connection name, falling back to host if name is empty
        let connection_identifier = if connection.name.trim().is_empty() {
            &connection.host
        } else {
            &connection.name
        };
        path_parts.push(connection_identifier.clone());

        path_parts.join(&PATH_SEPARATOR.to_string())
    }

    /// Builds the full `KeePass` entry path for a group's credentials.
    ///
    /// The path format is: `RustConn/Groups/GroupA/SubGroup`
    /// This stores group credentials separately from connection credentials.
    ///
    /// # Arguments
    /// * `group` - The group to build a path for
    /// * `groups` - All available connection groups for hierarchy resolution
    ///
    /// # Returns
    /// A string representing the full hierarchical path for the group's credentials
    ///
    /// # Example
    /// ```
    /// use rustconn_core::secret::hierarchy::KeePassHierarchy;
    /// use rustconn_core::models::ConnectionGroup;
    ///
    /// let production_group = ConnectionGroup::new("Production".to_string());
    /// let web_group = ConnectionGroup::with_parent("Web Servers".to_string(), production_group.id);
    ///
    /// let groups = vec![production_group.clone(), web_group.clone()];
    ///
    /// let path = KeePassHierarchy::build_group_entry_path(&web_group, &groups);
    /// assert!(path.starts_with("RustConn/Groups/"));
    /// assert!(path.contains("Production"));
    /// assert!(path.ends_with("Web Servers"));
    /// ```
    #[must_use]
    pub fn build_group_entry_path(group: &ConnectionGroup, groups: &[ConnectionGroup]) -> String {
        let mut path_parts = vec![KEEPASS_ROOT_GROUP.to_string(), GROUPS_SUBFOLDER.to_string()];

        // Build path from group's hierarchy
        let group_path = Self::resolve_group_path(group.id, groups);
        path_parts.extend(group_path);

        path_parts.join(&PATH_SEPARATOR.to_string())
    }

    /// Builds a simple lookup key for a group's credentials.
    ///
    /// This is used for backends that don't support hierarchical paths (like libsecret).
    /// Format: `group:{group_id}` or `group:{path}` for human-readable keys.
    ///
    /// # Arguments
    /// * `group` - The group to build a key for
    /// * `groups` - All available connection groups for hierarchy resolution
    /// * `use_path` - If true, use hierarchical path; if false, use UUID
    ///
    /// # Returns
    /// A string key for storing/retrieving the group's credentials
    #[must_use]
    pub fn build_group_lookup_key(
        group: &ConnectionGroup,
        groups: &[ConnectionGroup],
        use_path: bool,
    ) -> String {
        if use_path {
            let group_path = Self::resolve_group_path(group.id, groups);
            let path_str = group_path.join(&PATH_SEPARATOR.to_string());
            // Sanitize path for use as lookup key (replace / with -)
            let sanitized = path_str.replace(PATH_SEPARATOR, "-");
            format!("group:{sanitized}")
        } else {
            format!("group:{}", group.id)
        }
    }

    /// Resolves the full group path from a group ID by traversing parent groups.
    ///
    /// Returns the path from root to the specified group (not including the group itself
    /// in the returned path, but including all ancestors).
    ///
    /// # Arguments
    /// * `group_id` - The ID of the group to resolve
    /// * `groups` - All available connection groups
    ///
    /// # Returns
    /// A vector of group names from root to the specified group (inclusive)
    ///
    /// # Example
    /// ```
    /// use rustconn_core::secret::hierarchy::KeePassHierarchy;
    /// use rustconn_core::models::ConnectionGroup;
    ///
    /// let root = ConnectionGroup::new("Root".to_string());
    /// let child = ConnectionGroup::with_parent("Child".to_string(), root.id);
    /// let grandchild = ConnectionGroup::with_parent("Grandchild".to_string(), child.id);
    ///
    /// let groups = vec![root.clone(), child.clone(), grandchild.clone()];
    ///
    /// let path = KeePassHierarchy::resolve_group_path(grandchild.id, &groups);
    /// assert_eq!(path, vec!["Root", "Child", "Grandchild"]);
    /// ```
    #[must_use]
    pub fn resolve_group_path(group_id: Uuid, groups: &[ConnectionGroup]) -> Vec<String> {
        let mut path = Vec::new();
        let mut current_id = Some(group_id);

        // Traverse up the hierarchy, collecting group names
        while let Some(id) = current_id {
            if let Some(group) = groups.iter().find(|g| g.id == id) {
                path.insert(0, group.name.clone());
                current_id = group.parent_id;
            } else {
                // Group not found, stop traversal
                break;
            }
        }

        path
    }

    /// Extracts all group paths that need to exist for a given entry path.
    ///
    /// For path "RustConn/A/B/C/entry", returns:
    /// - "`RustConn`"
    /// - "RustConn/A"
    /// - "RustConn/A/B"
    /// - "RustConn/A/B/C"
    ///
    /// # Arguments
    /// * `entry_path` - The full entry path
    ///
    /// # Returns
    /// A vector of group paths that must exist (excluding the entry itself)
    #[must_use]
    pub fn extract_group_paths(entry_path: &str) -> Vec<String> {
        let parts: Vec<&str> = entry_path.split(PATH_SEPARATOR).collect();
        let mut group_paths = Vec::new();
        let mut current_path = String::new();

        // Skip the last part (entry name) and build cumulative paths
        for part in &parts[..parts.len().saturating_sub(1)] {
            if !current_path.is_empty() {
                current_path.push(PATH_SEPARATOR);
            }
            current_path.push_str(part);
            group_paths.push(current_path.clone());
        }

        group_paths
    }

    /// Computes the new entry path when a connection's group changes.
    ///
    /// # Arguments
    /// * `connection` - The connection with updated `group_id`
    /// * `groups` - All available connection groups
    ///
    /// # Returns
    /// The new hierarchical path for the connection
    #[must_use]
    pub fn compute_new_path(connection: &Connection, groups: &[ConnectionGroup]) -> String {
        Self::build_entry_path(connection, groups)
    }

    /// Extracts the entry name (last component) from a full path.
    ///
    /// # Arguments
    /// * `path` - The full entry path
    ///
    /// # Returns
    /// The entry name (last path component)
    #[must_use]
    pub fn extract_entry_name(path: &str) -> &str {
        path.rsplit(PATH_SEPARATOR).next().unwrap_or(path)
    }

    /// Extracts the parent group path from a full entry path.
    ///
    /// # Arguments
    /// * `path` - The full entry path
    ///
    /// # Returns
    /// The parent group path, or None if the path has no parent
    #[must_use]
    pub fn extract_parent_path(path: &str) -> Option<String> {
        let last_sep = path.rfind(PATH_SEPARATOR)?;
        if last_sep == 0 {
            return None;
        }
        Some(path[..last_sep].to_string())
    }

    /// Determines which groups need to be created for a given entry path.
    ///
    /// This method compares the required group paths against a set of existing
    /// groups and returns only those that need to be created.
    ///
    /// # Arguments
    /// * `entry_path` - The full entry path
    /// * `existing_groups` - Set of group paths that already exist
    ///
    /// # Returns
    /// A `GroupCreationResult` containing existing and to-be-created groups
    ///
    /// # Example
    /// ```
    /// use rustconn_core::secret::hierarchy::KeePassHierarchy;
    /// use std::collections::HashSet;
    ///
    /// let mut existing = HashSet::new();
    /// existing.insert("RustConn".to_string());
    ///
    /// let result = KeePassHierarchy::ensure_groups_exist(
    ///     "RustConn/Production/Web/nginx-01",
    ///     &existing
    /// );
    ///
    /// assert_eq!(result.existing_groups, vec!["RustConn"]);
    /// assert_eq!(result.created_groups, vec!["RustConn/Production", "RustConn/Production/Web"]);
    /// ```
    #[must_use]
    pub fn ensure_groups_exist(
        entry_path: &str,
        existing_groups: &HashSet<String>,
    ) -> GroupCreationResult {
        let required_paths = Self::extract_group_paths(entry_path);
        let mut result = GroupCreationResult::new();

        for path in required_paths {
            if existing_groups.contains(&path) {
                result.existing_groups.push(path);
            } else {
                result.created_groups.push(path);
            }
        }

        result
    }

    /// Validates that all parent groups exist for a given entry path.
    ///
    /// # Arguments
    /// * `entry_path` - The full entry path to validate
    /// * `existing_groups` - Set of group paths that exist
    ///
    /// # Returns
    /// `true` if all required parent groups exist, `false` otherwise
    #[must_use]
    pub fn all_groups_exist(entry_path: &str, existing_groups: &HashSet<String>) -> bool {
        let required_paths = Self::extract_group_paths(entry_path);
        required_paths
            .iter()
            .all(|path| existing_groups.contains(path))
    }

    /// Computes the groups that need to be created in order (from root to leaf).
    ///
    /// This ensures groups are created in the correct order - parent before child.
    ///
    /// # Arguments
    /// * `entry_path` - The full entry path
    /// * `existing_groups` - Set of group paths that already exist
    ///
    /// # Returns
    /// A vector of group paths to create, ordered from root to leaf
    #[must_use]
    pub fn groups_to_create_ordered(
        entry_path: &str,
        existing_groups: &HashSet<String>,
    ) -> Vec<String> {
        let result = Self::ensure_groups_exist(entry_path, existing_groups);
        // created_groups is already in order from root to leaf due to how extract_group_paths works
        result.created_groups
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProtocolConfig, SshConfig};

    fn create_test_connection(name: &str, group_id: Option<Uuid>) -> Connection {
        let mut conn = Connection::new(
            name.to_string(),
            "192.168.1.1".to_string(),
            22,
            ProtocolConfig::Ssh(SshConfig::default()),
        );
        conn.group_id = group_id;
        conn
    }

    #[test]
    fn test_build_entry_path_no_group() {
        let conn = create_test_connection("my-server", None);
        let path = KeePassHierarchy::build_entry_path(&conn, &[]);
        assert_eq!(path, "RustConn/my-server");
    }

    #[test]
    fn test_build_entry_path_with_single_group() {
        let group = ConnectionGroup::new("Production".to_string());
        let conn = create_test_connection("my-server", Some(group.id));
        let path = KeePassHierarchy::build_entry_path(&conn, &[group]);
        assert_eq!(path, "RustConn/Production/my-server");
    }

    #[test]
    fn test_build_entry_path_with_nested_groups() {
        let root = ConnectionGroup::new("Production".to_string());
        let child = ConnectionGroup::with_parent("Web".to_string(), root.id);
        let grandchild = ConnectionGroup::with_parent("Frontend".to_string(), child.id);

        let groups = vec![root, child, grandchild.clone()];
        let conn = create_test_connection("nginx-01", Some(grandchild.id));

        let path = KeePassHierarchy::build_entry_path(&conn, &groups);
        assert_eq!(path, "RustConn/Production/Web/Frontend/nginx-01");
    }

    #[test]
    fn test_build_entry_path_empty_name_uses_host() {
        let conn = create_test_connection("", None);
        let path = KeePassHierarchy::build_entry_path(&conn, &[]);
        assert_eq!(path, "RustConn/192.168.1.1");
    }

    #[test]
    fn test_resolve_group_path_single_group() {
        let group = ConnectionGroup::new("Servers".to_string());
        let path = KeePassHierarchy::resolve_group_path(group.id, std::slice::from_ref(&group));
        assert_eq!(path, vec!["Servers"]);
    }

    #[test]
    fn test_resolve_group_path_nested() {
        let root = ConnectionGroup::new("Root".to_string());
        let child = ConnectionGroup::with_parent("Child".to_string(), root.id);
        let grandchild = ConnectionGroup::with_parent("Grandchild".to_string(), child.id);

        let groups = vec![root, child, grandchild.clone()];
        let path = KeePassHierarchy::resolve_group_path(grandchild.id, &groups);
        assert_eq!(path, vec!["Root", "Child", "Grandchild"]);
    }

    #[test]
    fn test_resolve_group_path_missing_group() {
        let path = KeePassHierarchy::resolve_group_path(Uuid::new_v4(), &[]);
        assert!(path.is_empty());
    }

    #[test]
    fn test_extract_group_paths() {
        let paths = KeePassHierarchy::extract_group_paths("RustConn/A/B/C/entry");
        assert_eq!(
            paths,
            vec!["RustConn", "RustConn/A", "RustConn/A/B", "RustConn/A/B/C"]
        );
    }

    #[test]
    fn test_extract_group_paths_single_level() {
        let paths = KeePassHierarchy::extract_group_paths("RustConn/entry");
        assert_eq!(paths, vec!["RustConn"]);
    }

    #[test]
    fn test_extract_entry_name() {
        assert_eq!(
            KeePassHierarchy::extract_entry_name("RustConn/A/B/entry"),
            "entry"
        );
        assert_eq!(KeePassHierarchy::extract_entry_name("entry"), "entry");
    }

    #[test]
    fn test_extract_parent_path() {
        assert_eq!(
            KeePassHierarchy::extract_parent_path("RustConn/A/B/entry"),
            Some("RustConn/A/B".to_string())
        );
        assert_eq!(
            KeePassHierarchy::extract_parent_path("RustConn/entry"),
            Some("RustConn".to_string())
        );
        assert_eq!(KeePassHierarchy::extract_parent_path("entry"), None);
    }

    #[test]
    fn test_ensure_groups_exist_all_new() {
        let existing = HashSet::new();
        let result =
            KeePassHierarchy::ensure_groups_exist("RustConn/Production/Web/nginx-01", &existing);

        assert!(result.existing_groups.is_empty());
        assert_eq!(
            result.created_groups,
            vec!["RustConn", "RustConn/Production", "RustConn/Production/Web"]
        );
    }

    #[test]
    fn test_ensure_groups_exist_some_existing() {
        let mut existing = HashSet::new();
        existing.insert("RustConn".to_string());
        existing.insert("RustConn/Production".to_string());

        let result =
            KeePassHierarchy::ensure_groups_exist("RustConn/Production/Web/nginx-01", &existing);

        assert_eq!(
            result.existing_groups,
            vec!["RustConn", "RustConn/Production"]
        );
        assert_eq!(result.created_groups, vec!["RustConn/Production/Web"]);
    }

    #[test]
    fn test_ensure_groups_exist_all_existing() {
        let mut existing = HashSet::new();
        existing.insert("RustConn".to_string());
        existing.insert("RustConn/Production".to_string());
        existing.insert("RustConn/Production/Web".to_string());

        let result =
            KeePassHierarchy::ensure_groups_exist("RustConn/Production/Web/nginx-01", &existing);

        assert_eq!(
            result.existing_groups,
            vec!["RustConn", "RustConn/Production", "RustConn/Production/Web"]
        );
        assert!(result.created_groups.is_empty());
    }

    #[test]
    fn test_all_groups_exist() {
        let mut existing = HashSet::new();
        existing.insert("RustConn".to_string());

        assert!(!KeePassHierarchy::all_groups_exist(
            "RustConn/Production/entry",
            &existing
        ));

        existing.insert("RustConn/Production".to_string());
        assert!(KeePassHierarchy::all_groups_exist(
            "RustConn/Production/entry",
            &existing
        ));
    }

    #[test]
    fn test_groups_to_create_ordered() {
        let mut existing = HashSet::new();
        existing.insert("RustConn".to_string());

        let to_create =
            KeePassHierarchy::groups_to_create_ordered("RustConn/A/B/C/entry", &existing);

        // Should be in order from root to leaf
        assert_eq!(
            to_create,
            vec!["RustConn/A", "RustConn/A/B", "RustConn/A/B/C"]
        );
    }

    #[test]
    fn test_group_creation_result() {
        let mut result = GroupCreationResult::new();
        assert!(!result.any_created());
        assert_eq!(result.total_groups(), 0);

        result.existing_groups.push("RustConn".to_string());
        assert!(!result.any_created());
        assert_eq!(result.total_groups(), 1);

        result.created_groups.push("RustConn/New".to_string());
        assert!(result.any_created());
        assert_eq!(result.total_groups(), 2);
    }
}
