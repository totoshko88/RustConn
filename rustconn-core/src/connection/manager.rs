//! Connection manager for CRUD operations
//!
//! This module provides the `ConnectionManager` which handles creating, reading,
//! updating, and deleting connections with persistence through `ConfigManager`.

use std::collections::HashMap;

use chrono::Utc;
use uuid::Uuid;

use crate::config::ConfigManager;
use crate::error::{ConfigError, ConfigResult};
use crate::models::{Connection, ConnectionGroup, ProtocolConfig};
use crate::performance::memory_optimizer;

/// Manager for connection CRUD operations
///
/// Provides in-memory storage with persistence through `ConfigManager`.
/// Supports hierarchical group organization and search/filtering.
#[derive(Debug)]
pub struct ConnectionManager {
    /// In-memory connection storage indexed by ID
    connections: HashMap<Uuid, Connection>,
    /// In-memory group storage indexed by ID
    groups: HashMap<Uuid, ConnectionGroup>,
    /// Configuration manager for persistence
    config_manager: ConfigManager,
}

impl ConnectionManager {
    /// Creates a new `ConnectionManager` with the given `ConfigManager`
    ///
    /// Loads existing connections and groups from storage.
    /// Interns frequently repeated strings (protocol names, hostnames, usernames)
    /// for memory efficiency.
    ///
    /// # Errors
    ///
    /// Returns an error if loading from storage fails.
    pub fn new(config_manager: ConfigManager) -> ConfigResult<Self> {
        let connections_vec = config_manager.load_connections()?;
        let groups_vec = config_manager.load_groups()?;

        // Intern strings for memory efficiency
        for conn in &connections_vec {
            Self::intern_connection_strings(conn);
        }

        let connections = connections_vec.into_iter().map(|c| (c.id, c)).collect();

        let groups = groups_vec.into_iter().map(|g| (g.id, g)).collect();

        Ok(Self {
            connections,
            groups,
            config_manager,
        })
    }

    /// Creates a new `ConnectionManager` with empty storage (for testing)
    #[cfg(test)]
    #[must_use]
    pub fn new_empty(config_manager: ConfigManager) -> Self {
        Self {
            connections: HashMap::new(),
            groups: HashMap::new(),
            config_manager,
        }
    }

    // ========== Connection CRUD Operations ==========

    /// Creates a new connection and persists it
    ///
    /// # Arguments
    ///
    /// * `name` - Human-readable name for the connection
    /// * `host` - Remote host address
    /// * `port` - Remote port number
    /// * `protocol_config` - Protocol-specific configuration
    ///
    /// # Returns
    ///
    /// The UUID of the newly created connection
    ///
    /// # Errors
    ///
    /// Returns an error if validation fails or persistence fails.
    pub fn create_connection(
        &mut self,
        name: String,
        host: String,
        port: u16,
        protocol_config: ProtocolConfig,
    ) -> ConfigResult<Uuid> {
        let connection = Connection::new(name, host, port, protocol_config);
        ConfigManager::validate_connection(&connection)?;

        // Intern strings for memory efficiency
        Self::intern_connection_strings(&connection);

        let id = connection.id;
        self.connections.insert(id, connection);
        self.persist_connections()?;

        Ok(id)
    }

    /// Creates a connection from an existing Connection object
    ///
    /// Useful for importing connections or restoring from backup.
    ///
    /// # Errors
    ///
    /// Returns an error if validation fails or persistence fails.
    pub fn create_connection_from(&mut self, connection: Connection) -> ConfigResult<Uuid> {
        ConfigManager::validate_connection(&connection)?;

        // Intern strings for memory efficiency
        Self::intern_connection_strings(&connection);

        let id = connection.id;
        self.connections.insert(id, connection);
        self.persist_connections()?;

        Ok(id)
    }

    /// Updates an existing connection
    ///
    /// Preserves the original ID and creation timestamp.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection doesn't exist, validation fails,
    /// or persistence fails.
    pub fn update_connection(&mut self, id: Uuid, mut updated: Connection) -> ConfigResult<()> {
        let existing = self
            .connections
            .get(&id)
            .ok_or_else(|| ConfigError::Validation {
                field: "id".to_string(),
                reason: format!("Connection with ID {id} not found"),
            })?;

        // Preserve original ID and creation timestamp
        updated.id = existing.id;
        updated.created_at = existing.created_at;
        updated.touch();

        ConfigManager::validate_connection(&updated)?;

        // Intern strings for memory efficiency
        Self::intern_connection_strings(&updated);

        self.connections.insert(id, updated);
        self.persist_connections()?;

        Ok(())
    }

    /// Deletes a connection by ID
    ///
    /// # Errors
    ///
    /// Returns an error if the connection doesn't exist or persistence fails.
    pub fn delete_connection(&mut self, id: Uuid) -> ConfigResult<()> {
        if self.connections.remove(&id).is_none() {
            return Err(ConfigError::Validation {
                field: "id".to_string(),
                reason: format!("Connection with ID {id} not found"),
            });
        }

        self.persist_connections()?;
        Ok(())
    }

    /// Gets a connection by ID
    #[must_use]
    pub fn get_connection(&self, id: Uuid) -> Option<&Connection> {
        self.connections.get(&id)
    }

    /// Gets a mutable reference to a connection by ID
    pub fn get_connection_mut(&mut self, id: Uuid) -> Option<&mut Connection> {
        self.connections.get_mut(&id)
    }

    /// Lists all connections
    #[must_use]
    pub fn list_connections(&self) -> Vec<&Connection> {
        self.connections.values().collect()
    }

    /// Gets all connections in a specific group
    #[must_use]
    pub fn get_by_group(&self, group_id: Uuid) -> Vec<&Connection> {
        self.connections
            .values()
            .filter(|c| c.group_id == Some(group_id))
            .collect()
    }

    /// Gets all connections without a group (root level)
    #[must_use]
    pub fn get_ungrouped(&self) -> Vec<&Connection> {
        self.connections
            .values()
            .filter(|c| c.group_id.is_none())
            .collect()
    }

    /// Returns the total number of connections
    #[must_use]
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    // ========== Group CRUD Operations ==========

    /// Creates a new root-level group
    ///
    /// # Errors
    ///
    /// Returns an error if validation fails or persistence fails.
    pub fn create_group(&mut self, name: String) -> ConfigResult<Uuid> {
        let group = ConnectionGroup::new(name);
        ConfigManager::validate_group(&group)?;

        let id = group.id;
        self.groups.insert(id, group);
        self.persist_groups()?;

        Ok(id)
    }

    /// Creates a new group with a parent
    ///
    /// # Errors
    ///
    /// Returns an error if the parent doesn't exist, validation fails,
    /// or persistence fails.
    pub fn create_group_with_parent(
        &mut self,
        name: String,
        parent_id: Uuid,
    ) -> ConfigResult<Uuid> {
        // Verify parent exists
        if !self.groups.contains_key(&parent_id) {
            return Err(ConfigError::Validation {
                field: "parent_id".to_string(),
                reason: format!("Parent group with ID {parent_id} not found"),
            });
        }

        let group = ConnectionGroup::with_parent(name, parent_id);
        ConfigManager::validate_group(&group)?;

        let id = group.id;
        self.groups.insert(id, group);
        self.persist_groups()?;

        Ok(id)
    }

    /// Updates an existing group
    ///
    /// Preserves the original ID and creation timestamp.
    ///
    /// # Errors
    ///
    /// Returns an error if the group doesn't exist, validation fails,
    /// or persistence fails.
    pub fn update_group(&mut self, id: Uuid, mut updated: ConnectionGroup) -> ConfigResult<()> {
        let existing = self
            .groups
            .get(&id)
            .ok_or_else(|| ConfigError::Validation {
                field: "id".to_string(),
                reason: format!("Group with ID {id} not found"),
            })?;

        // Preserve original ID and creation timestamp
        updated.id = existing.id;
        updated.created_at = existing.created_at;

        ConfigManager::validate_group(&updated)?;

        self.groups.insert(id, updated);
        self.persist_groups()?;

        Ok(())
    }

    /// Deletes a group by ID
    ///
    /// Connections in the deleted group will have their `group_id` set to None.
    /// Child groups will be moved to the deleted group's parent.
    ///
    /// # Errors
    ///
    /// Returns an error if the group doesn't exist or persistence fails.
    pub fn delete_group(&mut self, id: Uuid) -> ConfigResult<()> {
        let group = self
            .groups
            .remove(&id)
            .ok_or_else(|| ConfigError::Validation {
                field: "id".to_string(),
                reason: format!("Group with ID {id} not found"),
            })?;

        let parent_id = group.parent_id;

        // Move connections in this group to ungrouped
        for conn in self.connections.values_mut() {
            if conn.group_id == Some(id) {
                conn.group_id = None;
                conn.touch();
            }
        }

        // Move child groups to the deleted group's parent
        for child_group in self.groups.values_mut() {
            if child_group.parent_id == Some(id) {
                child_group.parent_id = parent_id;
            }
        }

        self.persist_groups()?;
        self.persist_connections()?;

        Ok(())
    }

    /// Deletes a group and all connections within it (cascade delete)
    ///
    /// Unlike `delete_group`, this method removes all connections in the group
    /// rather than moving them to ungrouped. Child groups are also deleted
    /// along with their connections.
    ///
    /// # Errors
    ///
    /// Returns an error if the group doesn't exist or persistence fails.
    pub fn delete_group_cascade(&mut self, id: Uuid) -> ConfigResult<()> {
        // Verify group exists
        if !self.groups.contains_key(&id) {
            return Err(ConfigError::Validation {
                field: "id".to_string(),
                reason: format!("Group with ID {id} not found"),
            });
        }

        // Collect all groups to delete (this group and all descendants)
        let groups_to_delete = self.collect_descendant_groups(id);

        // Delete all connections in these groups
        let connections_to_delete: Vec<Uuid> = self
            .connections
            .iter()
            .filter(|(_, conn)| {
                conn.group_id
                    .is_some_and(|gid| groups_to_delete.contains(&gid))
            })
            .map(|(id, _)| *id)
            .collect();

        for conn_id in connections_to_delete {
            self.connections.remove(&conn_id);
        }

        // Delete all the groups
        for group_id in &groups_to_delete {
            self.groups.remove(group_id);
        }

        self.persist_groups()?;
        self.persist_connections()?;

        Ok(())
    }

    /// Collects a group and all its descendant groups
    fn collect_descendant_groups(&self, group_id: Uuid) -> Vec<Uuid> {
        let mut result = vec![group_id];
        let mut to_process = vec![group_id];

        while let Some(current_id) = to_process.pop() {
            for (id, group) in &self.groups {
                if group.parent_id == Some(current_id) && !result.contains(id) {
                    result.push(*id);
                    to_process.push(*id);
                }
            }
        }

        result
    }

    /// Counts connections in a group (including child groups)
    #[must_use]
    pub fn count_connections_in_group(&self, group_id: Uuid) -> usize {
        let groups = self.collect_descendant_groups(group_id);
        self.connections
            .values()
            .filter(|conn| conn.group_id.is_some_and(|gid| groups.contains(&gid)))
            .count()
    }

    /// Moves a group to a new parent
    ///
    /// # Arguments
    ///
    /// * `group_id` - The group to move
    /// * `new_parent_id` - The new parent (None for root level)
    ///
    /// # Errors
    ///
    /// Returns an error if the group doesn't exist, the new parent doesn't exist,
    /// or the move would create a cycle.
    pub fn move_group(&mut self, group_id: Uuid, new_parent_id: Option<Uuid>) -> ConfigResult<()> {
        // Verify group exists
        if !self.groups.contains_key(&group_id) {
            return Err(ConfigError::Validation {
                field: "group_id".to_string(),
                reason: format!("Group with ID {group_id} not found"),
            });
        }

        // Verify new parent exists (if specified)
        if let Some(parent_id) = new_parent_id {
            if !self.groups.contains_key(&parent_id) {
                return Err(ConfigError::Validation {
                    field: "new_parent_id".to_string(),
                    reason: format!("Parent group with ID {parent_id} not found"),
                });
            }

            // Check for cycles
            if self.would_create_cycle(group_id, parent_id) {
                return Err(ConfigError::Validation {
                    field: "new_parent_id".to_string(),
                    reason: "Moving group would create a cycle in the hierarchy".to_string(),
                });
            }
        }

        // Perform the move
        if let Some(group) = self.groups.get_mut(&group_id) {
            group.parent_id = new_parent_id;
        }

        self.persist_groups()?;
        Ok(())
    }

    /// Moves a connection to a different group
    ///
    /// # Arguments
    ///
    /// * `connection_id` - The connection to move
    /// * `group_id` - The target group (None for ungrouped)
    ///
    /// # Errors
    ///
    /// Returns an error if the connection or group doesn't exist.
    pub fn move_connection_to_group(
        &mut self,
        connection_id: Uuid,
        group_id: Option<Uuid>,
    ) -> ConfigResult<()> {
        // Verify group exists (if specified)
        if let Some(gid) = group_id {
            if !self.groups.contains_key(&gid) {
                return Err(ConfigError::Validation {
                    field: "group_id".to_string(),
                    reason: format!("Group with ID {gid} not found"),
                });
            }
        }

        // Calculate the new sort_order (append to end of target group)
        let new_sort_order = self
            .connections
            .values()
            .filter(|c| c.group_id == group_id && c.id != connection_id)
            .map(|c| c.sort_order)
            .max()
            .map_or(0, |max| max + 1);

        let conn =
            self.connections
                .get_mut(&connection_id)
                .ok_or_else(|| ConfigError::Validation {
                    field: "connection_id".to_string(),
                    reason: format!("Connection with ID {connection_id} not found"),
                })?;

        conn.group_id = group_id;
        conn.sort_order = new_sort_order;
        conn.touch();

        self.persist_connections()?;
        Ok(())
    }

    /// Gets a group by ID
    #[must_use]
    pub fn get_group(&self, id: Uuid) -> Option<&ConnectionGroup> {
        self.groups.get(&id)
    }

    /// Lists all groups
    #[must_use]
    pub fn list_groups(&self) -> Vec<&ConnectionGroup> {
        self.groups.values().collect()
    }

    /// Gets all root-level groups
    #[must_use]
    pub fn get_root_groups(&self) -> Vec<&ConnectionGroup> {
        self.groups
            .values()
            .filter(|g| g.parent_id.is_none())
            .collect()
    }

    /// Gets all child groups of a parent
    #[must_use]
    pub fn get_child_groups(&self, parent_id: Uuid) -> Vec<&ConnectionGroup> {
        self.groups
            .values()
            .filter(|g| g.parent_id == Some(parent_id))
            .collect()
    }

    /// Returns the total number of groups
    #[must_use]
    pub fn group_count(&self) -> usize {
        self.groups.len()
    }

    // ========== Search and Filtering ==========

    /// Searches connections by query string
    ///
    /// Matches against name, host, tags, and group path.
    /// Case-insensitive matching.
    ///
    /// # Arguments
    ///
    /// * `query` - The search query string
    ///
    /// # Returns
    ///
    /// A vector of references to matching connections
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<&Connection> {
        let query_lower = query.to_lowercase();

        self.connections
            .values()
            .filter(|conn| {
                // Match name
                if conn.name.to_lowercase().contains(&query_lower) {
                    return true;
                }

                // Match host
                if conn.host.to_lowercase().contains(&query_lower) {
                    return true;
                }

                // Match tags
                if conn
                    .tags
                    .iter()
                    .any(|tag| tag.to_lowercase().contains(&query_lower))
                {
                    return true;
                }

                // Match group path
                if let Some(group_id) = conn.group_id {
                    if let Some(path) = self.get_group_path(group_id) {
                        if path.to_lowercase().contains(&query_lower) {
                            return true;
                        }
                    }
                }

                false
            })
            .collect()
    }

    /// Filters connections by tag
    ///
    /// Returns all connections that have the specified tag.
    /// Case-insensitive matching.
    #[must_use]
    pub fn filter_by_tag(&self, tag: &str) -> Vec<&Connection> {
        let tag_lower = tag.to_lowercase();

        self.connections
            .values()
            .filter(|conn| conn.tags.iter().any(|t| t.to_lowercase() == tag_lower))
            .collect()
    }

    /// Filters connections by multiple tags (AND logic)
    ///
    /// Returns connections that have ALL specified tags.
    #[must_use]
    pub fn filter_by_tags(&self, tags: &[String]) -> Vec<&Connection> {
        let tags_lower: Vec<String> = tags.iter().map(|t| t.to_lowercase()).collect();

        self.connections
            .values()
            .filter(|conn| {
                let conn_tags_lower: Vec<String> =
                    conn.tags.iter().map(|t| t.to_lowercase()).collect();
                tags_lower.iter().all(|tag| conn_tags_lower.contains(tag))
            })
            .collect()
    }

    /// Gets all unique tags across all connections
    #[must_use]
    pub fn get_all_tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = self
            .connections
            .values()
            .flat_map(|c| c.tags.clone())
            .collect();

        tags.sort();
        tags.dedup();
        tags
    }

    // ========== Group Path Utilities ==========

    /// Gets the full path of a group (e.g., "Production/Web Servers")
    #[must_use]
    pub fn get_group_path(&self, group_id: Uuid) -> Option<String> {
        let mut path_parts = Vec::new();
        let mut current_id = Some(group_id);

        // Walk up the hierarchy
        while let Some(id) = current_id {
            if let Some(group) = self.groups.get(&id) {
                path_parts.push(group.name.clone());
                current_id = group.parent_id;
            } else {
                break;
            }
        }

        if path_parts.is_empty() {
            None
        } else {
            path_parts.reverse();
            Some(path_parts.join("/"))
        }
    }

    /// Checks if moving a group would create a cycle
    fn would_create_cycle(&self, group_id: Uuid, new_parent_id: Uuid) -> bool {
        // A cycle would be created if new_parent_id is a descendant of group_id
        // (or is group_id itself)
        if group_id == new_parent_id {
            return true;
        }

        let mut current_id = Some(new_parent_id);
        while let Some(id) = current_id {
            if id == group_id {
                return true;
            }
            current_id = self.groups.get(&id).and_then(|g| g.parent_id);
        }

        false
    }

    /// Validates that the group hierarchy is acyclic
    ///
    /// Returns true if the hierarchy is valid (no cycles).
    #[must_use]
    pub fn validate_hierarchy(&self) -> bool {
        for group in self.groups.values() {
            let mut visited = std::collections::HashSet::new();
            let mut current_id = Some(group.id);

            while let Some(id) = current_id {
                if !visited.insert(id) {
                    // We've seen this ID before - cycle detected
                    return false;
                }
                current_id = self.groups.get(&id).and_then(|g| g.parent_id);
            }
        }

        true
    }

    // ========== Persistence ==========

    /// Persists all connections to storage
    fn persist_connections(&self) -> ConfigResult<()> {
        let connections: Vec<Connection> = self.connections.values().cloned().collect();
        self.config_manager.save_connections(&connections)
    }

    /// Persists all groups to storage
    fn persist_groups(&self) -> ConfigResult<()> {
        let groups: Vec<ConnectionGroup> = self.groups.values().cloned().collect();
        self.config_manager.save_groups(&groups)
    }

    /// Reloads connections and groups from storage
    ///
    /// # Errors
    ///
    /// Returns an error if loading fails.
    pub fn reload(&mut self) -> ConfigResult<()> {
        let connections_vec = self.config_manager.load_connections()?;
        let groups_vec = self.config_manager.load_groups()?;

        // Intern strings for memory efficiency
        for conn in &connections_vec {
            Self::intern_connection_strings(conn);
        }

        self.connections = connections_vec.into_iter().map(|c| (c.id, c)).collect();

        self.groups = groups_vec.into_iter().map(|g| (g.id, g)).collect();

        Ok(())
    }

    /// Interns frequently repeated strings from a connection for memory efficiency
    ///
    /// This method interns protocol names, hostnames, and usernames which are
    /// often repeated across many connections.
    fn intern_connection_strings(conn: &Connection) {
        let interner = memory_optimizer().interner();

        // Intern protocol names (frequently repeated)
        let _ = interner.intern(conn.protocol.as_str());

        // Intern hostnames (often repeated across connections)
        let _ = interner.intern(&conn.host);

        // Intern usernames if present
        if let Some(ref username) = conn.username {
            let _ = interner.intern(username);
        }
    }

    // ========== Sorting Operations ==========

    /// Sorts connections within a specific group alphabetically by name
    ///
    /// Updates the `sort_order` field for each connection in the group.
    /// Connections in other groups are not affected.
    ///
    /// # Arguments
    ///
    /// * `group_id` - The ID of the group to sort
    ///
    /// # Errors
    ///
    /// Returns an error if the group doesn't exist or persistence fails.
    pub fn sort_group(&mut self, group_id: Uuid) -> ConfigResult<()> {
        // Verify group exists
        if !self.groups.contains_key(&group_id) {
            return Err(ConfigError::Validation {
                field: "group_id".to_string(),
                reason: format!("Group with ID {group_id} not found"),
            });
        }

        // Get connections in this group and sort them
        let mut group_connections: Vec<_> = self
            .connections
            .values()
            .filter(|c| c.group_id == Some(group_id))
            .map(|c| c.id)
            .collect();

        // Sort by name (case-insensitive)
        group_connections.sort_by(|a, b| {
            let name_a = self
                .connections
                .get(a)
                .map(|c| c.name.to_lowercase())
                .unwrap_or_default();
            let name_b = self
                .connections
                .get(b)
                .map(|c| c.name.to_lowercase())
                .unwrap_or_default();
            name_a.cmp(&name_b)
        });

        // Update sort_order for each connection
        for (idx, conn_id) in group_connections.iter().enumerate() {
            if let Some(conn) = self.connections.get_mut(conn_id) {
                #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                {
                    conn.sort_order = idx as i32;
                }
                conn.touch();
            }
        }

        self.persist_connections()?;
        Ok(())
    }

    /// Sorts all groups and their connections alphabetically
    ///
    /// This method:
    /// 1. Sorts root-level groups alphabetically
    /// 2. Sorts connections within each group alphabetically
    /// 3. Sorts ungrouped connections alphabetically
    ///
    /// # Errors
    ///
    /// Returns an error if persistence fails.
    #[allow(clippy::too_many_lines)]
    pub fn sort_all(&mut self) -> ConfigResult<()> {
        // Sort root groups
        let mut root_groups: Vec<_> = self
            .groups
            .values()
            .filter(|g| g.parent_id.is_none())
            .map(|g| g.id)
            .collect();

        root_groups.sort_by(|a, b| {
            let name_a = self
                .groups
                .get(a)
                .map(|g| g.name.to_lowercase())
                .unwrap_or_default();
            let name_b = self
                .groups
                .get(b)
                .map(|g| g.name.to_lowercase())
                .unwrap_or_default();
            name_a.cmp(&name_b)
        });

        // Update sort_order for root groups
        for (idx, group_id) in root_groups.iter().enumerate() {
            if let Some(group) = self.groups.get_mut(group_id) {
                #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                {
                    group.sort_order = idx as i32;
                }
            }
        }

        // Sort connections within each group (including nested groups)
        let all_group_ids: Vec<Uuid> = self.groups.keys().copied().collect();
        for group_id in all_group_ids {
            // Sort child groups within this group
            let mut child_groups: Vec<_> = self
                .groups
                .values()
                .filter(|g| g.parent_id == Some(group_id))
                .map(|g| g.id)
                .collect();

            child_groups.sort_by(|a, b| {
                let name_a = self
                    .groups
                    .get(a)
                    .map(|g| g.name.to_lowercase())
                    .unwrap_or_default();
                let name_b = self
                    .groups
                    .get(b)
                    .map(|g| g.name.to_lowercase())
                    .unwrap_or_default();
                name_a.cmp(&name_b)
            });

            for (idx, child_id) in child_groups.iter().enumerate() {
                if let Some(group) = self.groups.get_mut(child_id) {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                    {
                        group.sort_order = idx as i32;
                    }
                }
            }

            // Sort connections in this group
            let mut group_connections: Vec<_> = self
                .connections
                .values()
                .filter(|c| c.group_id == Some(group_id))
                .map(|c| c.id)
                .collect();

            group_connections.sort_by(|a, b| {
                let name_a = self
                    .connections
                    .get(a)
                    .map(|c| c.name.to_lowercase())
                    .unwrap_or_default();
                let name_b = self
                    .connections
                    .get(b)
                    .map(|c| c.name.to_lowercase())
                    .unwrap_or_default();
                name_a.cmp(&name_b)
            });

            for (idx, conn_id) in group_connections.iter().enumerate() {
                if let Some(conn) = self.connections.get_mut(conn_id) {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                    {
                        conn.sort_order = idx as i32;
                    }
                    conn.touch();
                }
            }
        }

        // Sort ungrouped connections
        let mut ungrouped: Vec<_> = self
            .connections
            .values()
            .filter(|c| c.group_id.is_none())
            .map(|c| c.id)
            .collect();

        ungrouped.sort_by(|a, b| {
            let name_a = self
                .connections
                .get(a)
                .map(|c| c.name.to_lowercase())
                .unwrap_or_default();
            let name_b = self
                .connections
                .get(b)
                .map(|c| c.name.to_lowercase())
                .unwrap_or_default();
            name_a.cmp(&name_b)
        });

        for (idx, conn_id) in ungrouped.iter().enumerate() {
            if let Some(conn) = self.connections.get_mut(conn_id) {
                #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                {
                    conn.sort_order = idx as i32;
                }
                conn.touch();
            }
        }

        self.persist_groups()?;
        self.persist_connections()?;
        Ok(())
    }

    /// Updates the `last_connected` timestamp for a connection
    ///
    /// Sets the `last_connected` field to the current time. This should be called
    /// when a connection is initiated.
    ///
    /// # Arguments
    ///
    /// * `connection_id` - The ID of the connection to update
    ///
    /// # Errors
    ///
    /// Returns an error if the connection doesn't exist or persistence fails.
    pub fn update_last_connected(&mut self, connection_id: Uuid) -> ConfigResult<()> {
        let conn =
            self.connections
                .get_mut(&connection_id)
                .ok_or_else(|| ConfigError::Validation {
                    field: "connection_id".to_string(),
                    reason: format!("Connection with ID {connection_id} not found"),
                })?;

        conn.last_connected = Some(Utc::now());
        conn.touch();

        self.persist_connections()?;
        Ok(())
    }

    /// Sorts all connections by `last_connected` timestamp (most recent first)
    ///
    /// Connections with a `last_connected` timestamp are sorted in descending order
    /// (most recently used first). Connections without a timestamp are placed at
    /// the end, sorted alphabetically by name.
    ///
    /// # Errors
    ///
    /// Returns an error if persistence fails.
    pub fn sort_by_recent(&mut self) -> ConfigResult<()> {
        // Get all connection IDs
        let mut conn_ids: Vec<Uuid> = self.connections.keys().copied().collect();

        // Sort by last_connected descending, then by name for those without timestamp
        conn_ids.sort_by(|a, b| {
            let conn_a = self.connections.get(a);
            let conn_b = self.connections.get(b);

            match (
                conn_a.and_then(|c| c.last_connected),
                conn_b.and_then(|c| c.last_connected),
            ) {
                // Both have timestamps - sort descending (most recent first)
                (Some(time_a), Some(time_b)) => time_b.cmp(&time_a),
                // Only a has timestamp - a comes first
                (Some(_), None) => std::cmp::Ordering::Less,
                // Only b has timestamp - b comes first
                (None, Some(_)) => std::cmp::Ordering::Greater,
                // Neither has timestamp - sort by name
                (None, None) => {
                    let name_a = conn_a.map(|c| c.name.to_lowercase()).unwrap_or_default();
                    let name_b = conn_b.map(|c| c.name.to_lowercase()).unwrap_or_default();
                    name_a.cmp(&name_b)
                }
            }
        });

        // Update sort_order for all connections
        for (idx, conn_id) in conn_ids.iter().enumerate() {
            if let Some(conn) = self.connections.get_mut(conn_id) {
                #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                {
                    conn.sort_order = idx as i32;
                }
                conn.touch();
            }
        }

        self.persist_connections()?;
        Ok(())
    }

    // ========== Drag-Drop Reordering Operations ==========

    /// Reorders a connection to be positioned after another connection
    ///
    /// Both connections must be in the same group. The source connection
    /// will be placed immediately after the target connection in sort order.
    ///
    /// # Arguments
    ///
    /// * `connection_id` - The connection to move
    /// * `target_id` - The connection to position after
    ///
    /// # Errors
    ///
    /// Returns an error if either connection doesn't exist, they're in different
    /// groups, or persistence fails.
    pub fn reorder_connection(&mut self, connection_id: Uuid, target_id: Uuid) -> ConfigResult<()> {
        // Verify both connections exist
        let source_group = self
            .connections
            .get(&connection_id)
            .ok_or_else(|| ConfigError::Validation {
                field: "connection_id".to_string(),
                reason: format!("Connection with ID {connection_id} not found"),
            })?
            .group_id;

        let target_group = self
            .connections
            .get(&target_id)
            .ok_or_else(|| ConfigError::Validation {
                field: "target_id".to_string(),
                reason: format!("Connection with ID {target_id} not found"),
            })?
            .group_id;

        // Verify they're in the same group
        if source_group != target_group {
            return Err(ConfigError::Validation {
                field: "group_id".to_string(),
                reason: "Connections must be in the same group for reordering".to_string(),
            });
        }

        // Get target's sort_order
        let target_sort_order = self.connections.get(&target_id).map_or(0, |c| c.sort_order);

        // Get all connections in the same group, sorted by sort_order
        let mut group_connections: Vec<_> = self
            .connections
            .values()
            .filter(|c| c.group_id == source_group && c.id != connection_id)
            .map(|c| (c.id, c.sort_order))
            .collect();

        group_connections.sort_by_key(|(_, order)| *order);

        // Find the position to insert (after target)
        let insert_pos = group_connections
            .iter()
            .position(|(id, _)| *id == target_id)
            .map_or(group_connections.len(), |p| p + 1);

        // Insert the source connection at the new position
        group_connections.insert(insert_pos, (connection_id, target_sort_order));

        // Update sort_order for all connections in the group
        for (idx, (conn_id, _)) in group_connections.iter().enumerate() {
            if let Some(conn) = self.connections.get_mut(conn_id) {
                #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                {
                    conn.sort_order = idx as i32;
                }
                conn.touch();
            }
        }

        self.persist_connections()?;
        Ok(())
    }

    /// Reorders a group to be positioned after another group
    ///
    /// Both groups must be at the same level (same parent). The source group
    /// will be placed immediately after the target group in sort order.
    ///
    /// # Arguments
    ///
    /// * `group_id` - The group to move
    /// * `target_id` - The group to position after
    ///
    /// # Errors
    ///
    /// Returns an error if either group doesn't exist, they have different
    /// parents, or persistence fails.
    pub fn reorder_group(&mut self, group_id: Uuid, target_id: Uuid) -> ConfigResult<()> {
        // Verify both groups exist
        let source_parent = self
            .groups
            .get(&group_id)
            .ok_or_else(|| ConfigError::Validation {
                field: "group_id".to_string(),
                reason: format!("Group with ID {group_id} not found"),
            })?
            .parent_id;

        let target_parent = self
            .groups
            .get(&target_id)
            .ok_or_else(|| ConfigError::Validation {
                field: "target_id".to_string(),
                reason: format!("Group with ID {target_id} not found"),
            })?
            .parent_id;

        // Verify they have the same parent
        if source_parent != target_parent {
            return Err(ConfigError::Validation {
                field: "parent_id".to_string(),
                reason: "Groups must have the same parent for reordering".to_string(),
            });
        }

        // Get target's sort_order
        let target_sort_order = self.groups.get(&target_id).map_or(0, |g| g.sort_order);

        // Get all sibling groups, sorted by sort_order
        let mut sibling_groups: Vec<_> = self
            .groups
            .values()
            .filter(|g| g.parent_id == source_parent && g.id != group_id)
            .map(|g| (g.id, g.sort_order))
            .collect();

        sibling_groups.sort_by_key(|(_, order)| *order);

        // Find the position to insert (after target)
        let insert_pos = sibling_groups
            .iter()
            .position(|(id, _)| *id == target_id)
            .map_or(sibling_groups.len(), |p| p + 1);

        // Insert the source group at the new position
        sibling_groups.insert(insert_pos, (group_id, target_sort_order));

        // Update sort_order for all sibling groups
        for (idx, (gid, _)) in sibling_groups.iter().enumerate() {
            if let Some(group) = self.groups.get_mut(gid) {
                #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                {
                    group.sort_order = idx as i32;
                }
            }
        }

        self.persist_groups()?;
        Ok(())
    }

    // ========== Connection Naming Operations ==========

    /// Generates a unique name for a connection
    ///
    /// If the base name already exists, appends a protocol suffix (e.g., "server (RDP)").
    /// If that still exists, appends a numeric suffix (e.g., "server (RDP) 2").
    ///
    /// # Arguments
    ///
    /// * `base_name` - The desired name for the connection
    /// * `protocol` - The protocol type for the connection
    ///
    /// # Returns
    ///
    /// A unique name that doesn't conflict with existing connections
    #[must_use]
    pub fn generate_unique_name(
        &self,
        base_name: &str,
        protocol: crate::models::ProtocolType,
    ) -> String {
        // Check if base name is unique
        if !self.name_exists(base_name, None) {
            return base_name.to_string();
        }

        // Try with protocol suffix
        let name_with_protocol = format!("{base_name} ({protocol})");
        if !self.name_exists(&name_with_protocol, None) {
            return name_with_protocol;
        }

        // Try with numeric suffix
        let mut counter = 2u32;
        loop {
            let name_with_number = format!("{name_with_protocol} {counter}");
            if !self.name_exists(&name_with_number, None) {
                return name_with_number;
            }
            counter += 1;
            // Safety limit to prevent infinite loop
            if counter > 1000 {
                // Fall back to UUID suffix
                return format!("{} ({})", base_name, uuid::Uuid::new_v4());
            }
        }
    }

    /// Checks if a connection name already exists
    ///
    /// # Arguments
    ///
    /// * `name` - The name to check
    /// * `exclude_id` - Optional ID to exclude from the check (for updates)
    ///
    /// # Returns
    ///
    /// `true` if a connection with this name exists (excluding the specified ID)
    #[must_use]
    pub fn name_exists(&self, name: &str, exclude_id: Option<Uuid>) -> bool {
        self.connections
            .values()
            .any(|c| c.name.eq_ignore_ascii_case(name) && (exclude_id != Some(c.id)))
    }

    /// Checks if a name needs deduplication
    ///
    /// # Arguments
    ///
    /// * `name` - The name to check
    /// * `exclude_id` - Optional ID to exclude from the check
    ///
    /// # Returns
    ///
    /// `true` if the name conflicts with an existing connection
    #[must_use]
    pub fn needs_deduplication(&self, name: &str, exclude_id: Option<Uuid>) -> bool {
        self.name_exists(name, exclude_id)
    }

    /// Normalizes a connection name by removing auto-generated suffixes if the base name is now unique
    ///
    /// Auto-generated suffixes follow the pattern:
    /// - " (PROTOCOL)" - e.g., " (SSH)", " (RDP)", " (VNC)"
    /// - " (PROTOCOL) N" - e.g., " (SSH) 2", " (RDP) 3"
    ///
    /// # Arguments
    ///
    /// * `name` - The current name of the connection
    /// * `connection_id` - The ID of the connection being renamed (to exclude from uniqueness check)
    ///
    /// # Returns
    ///
    /// The normalized name with suffix removed if the base name is now unique,
    /// otherwise returns the original name
    #[must_use]
    pub fn normalize_name(&self, name: &str, connection_id: Uuid) -> String {
        // Try to extract base name by removing auto-generated suffixes
        if let Some(base_name) = Self::extract_base_name(name) {
            // Check if the base name is now unique (excluding this connection)
            if !self.name_exists(&base_name, Some(connection_id)) {
                return base_name;
            }
        }

        // Return original name if no normalization possible
        name.to_string()
    }

    /// Extracts the base name from a potentially suffixed name
    ///
    /// Handles patterns like:
    /// - "server (SSH)" -> "server"
    /// - "server (RDP) 2" -> "server"
    /// - "server" -> None (no suffix to remove)
    fn extract_base_name(name: &str) -> Option<String> {
        // Pattern: "base (PROTOCOL) N" or "base (PROTOCOL)"
        let protocols = ["SSH", "RDP", "VNC", "SPICE", "Zero Trust"];

        for protocol in protocols {
            // Check for " (PROTOCOL) N" pattern
            let suffix_with_number = format!(" ({protocol}) ");
            if let Some(pos) = name.find(&suffix_with_number) {
                // Verify the rest is a number
                let after_suffix = &name[pos + suffix_with_number.len()..];
                if after_suffix.chars().all(|c| c.is_ascii_digit()) && !after_suffix.is_empty() {
                    return Some(name[..pos].to_string());
                }
            }

            // Check for " (PROTOCOL)" pattern at the end
            let suffix = format!(" ({protocol})");
            if name.ends_with(&suffix) {
                return Some(name[..name.len() - suffix.len()].to_string());
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProtocolConfig, SshConfig};
    use tempfile::TempDir;

    fn create_test_manager() -> (ConnectionManager, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let config_manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());
        let manager = ConnectionManager::new_empty(config_manager);
        (manager, temp_dir)
    }

    #[test]
    fn test_create_connection() {
        let (mut manager, _temp) = create_test_manager();

        let id = manager
            .create_connection(
                "Test Server".to_string(),
                "example.com".to_string(),
                22,
                ProtocolConfig::Ssh(SshConfig::default()),
            )
            .unwrap();

        assert_eq!(manager.connection_count(), 1);
        let conn = manager.get_connection(id).unwrap();
        assert_eq!(conn.name, "Test Server");
        assert_eq!(conn.host, "example.com");
        assert_eq!(conn.port, 22);
    }

    #[test]
    fn test_update_connection() {
        let (mut manager, _temp) = create_test_manager();

        let id = manager
            .create_connection(
                "Test Server".to_string(),
                "example.com".to_string(),
                22,
                ProtocolConfig::Ssh(SshConfig::default()),
            )
            .unwrap();

        let mut updated = manager.get_connection(id).unwrap().clone();
        updated.name = "Updated Server".to_string();
        updated.host = "new.example.com".to_string();

        manager.update_connection(id, updated).unwrap();

        let conn = manager.get_connection(id).unwrap();
        assert_eq!(conn.name, "Updated Server");
        assert_eq!(conn.host, "new.example.com");
    }

    #[test]
    fn test_delete_connection() {
        let (mut manager, _temp) = create_test_manager();

        let id = manager
            .create_connection(
                "Test Server".to_string(),
                "example.com".to_string(),
                22,
                ProtocolConfig::Ssh(SshConfig::default()),
            )
            .unwrap();

        assert_eq!(manager.connection_count(), 1);
        manager.delete_connection(id).unwrap();
        assert_eq!(manager.connection_count(), 0);
        assert!(manager.get_connection(id).is_none());
    }

    #[test]
    fn test_create_group() {
        let (mut manager, _temp) = create_test_manager();

        let id = manager.create_group("Production".to_string()).unwrap();

        assert_eq!(manager.group_count(), 1);
        let group = manager.get_group(id).unwrap();
        assert_eq!(group.name, "Production");
        assert!(group.parent_id.is_none());
    }

    #[test]
    fn test_create_group_with_parent() {
        let (mut manager, _temp) = create_test_manager();

        let parent_id = manager.create_group("Production".to_string()).unwrap();
        let child_id = manager
            .create_group_with_parent("Web Servers".to_string(), parent_id)
            .unwrap();

        let child = manager.get_group(child_id).unwrap();
        assert_eq!(child.parent_id, Some(parent_id));
    }

    #[test]
    fn test_delete_group_moves_connections() {
        let (mut manager, _temp) = create_test_manager();

        let group_id = manager.create_group("Production".to_string()).unwrap();
        let conn_id = manager
            .create_connection(
                "Test Server".to_string(),
                "example.com".to_string(),
                22,
                ProtocolConfig::Ssh(SshConfig::default()),
            )
            .unwrap();

        manager
            .move_connection_to_group(conn_id, Some(group_id))
            .unwrap();
        assert_eq!(
            manager.get_connection(conn_id).unwrap().group_id,
            Some(group_id)
        );

        manager.delete_group(group_id).unwrap();
        assert!(manager.get_connection(conn_id).unwrap().group_id.is_none());
    }

    #[test]
    fn test_move_group_prevents_cycle() {
        let (mut manager, _temp) = create_test_manager();

        let parent_id = manager.create_group("Parent".to_string()).unwrap();
        let child_id = manager
            .create_group_with_parent("Child".to_string(), parent_id)
            .unwrap();

        // Try to make parent a child of child (would create cycle)
        let result = manager.move_group(parent_id, Some(child_id));
        assert!(result.is_err());
    }

    #[test]
    fn test_search_by_name() {
        let (mut manager, _temp) = create_test_manager();

        manager
            .create_connection(
                "Production Server".to_string(),
                "prod.example.com".to_string(),
                22,
                ProtocolConfig::Ssh(SshConfig::default()),
            )
            .unwrap();

        manager
            .create_connection(
                "Development Server".to_string(),
                "dev.example.com".to_string(),
                22,
                ProtocolConfig::Ssh(SshConfig::default()),
            )
            .unwrap();

        let results = manager.search("production");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Production Server");
    }

    #[test]
    fn test_search_by_host() {
        let (mut manager, _temp) = create_test_manager();

        manager
            .create_connection(
                "Server 1".to_string(),
                "prod.example.com".to_string(),
                22,
                ProtocolConfig::Ssh(SshConfig::default()),
            )
            .unwrap();

        let results = manager.search("prod.example");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_filter_by_tag() {
        let (mut manager, _temp) = create_test_manager();

        let id = manager
            .create_connection(
                "Server".to_string(),
                "example.com".to_string(),
                22,
                ProtocolConfig::Ssh(SshConfig::default()),
            )
            .unwrap();

        // Add tags
        if let Some(conn) = manager.get_connection_mut(id) {
            conn.tags = vec!["production".to_string(), "web".to_string()];
        }

        let results = manager.filter_by_tag("production");
        assert_eq!(results.len(), 1);

        let results = manager.filter_by_tag("staging");
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_get_group_path() {
        let (mut manager, _temp) = create_test_manager();

        let root_id = manager.create_group("Production".to_string()).unwrap();
        let child_id = manager
            .create_group_with_parent("Web Servers".to_string(), root_id)
            .unwrap();
        let grandchild_id = manager
            .create_group_with_parent("Frontend".to_string(), child_id)
            .unwrap();

        let path = manager.get_group_path(grandchild_id).unwrap();
        assert_eq!(path, "Production/Web Servers/Frontend");
    }

    #[test]
    fn test_validate_hierarchy() {
        let (mut manager, _temp) = create_test_manager();

        manager.create_group("Root".to_string()).unwrap();
        assert!(manager.validate_hierarchy());
    }

    // ========== Connection Naming Tests ==========

    #[test]
    fn test_generate_unique_name_no_conflict() {
        let (manager, _temp) = create_test_manager();

        let name = manager.generate_unique_name("server", crate::models::ProtocolType::Ssh);
        assert_eq!(name, "server");
    }

    #[test]
    fn test_generate_unique_name_with_protocol_suffix() {
        let (mut manager, _temp) = create_test_manager();

        // Create a connection with the base name
        manager
            .create_connection(
                "server".to_string(),
                "example.com".to_string(),
                22,
                ProtocolConfig::Ssh(SshConfig::default()),
            )
            .unwrap();

        // Generate unique name for RDP connection with same base name
        let name = manager.generate_unique_name("server", crate::models::ProtocolType::Rdp);
        assert_eq!(name, "server (RDP)");
    }

    #[test]
    fn test_generate_unique_name_with_numeric_suffix() {
        let (mut manager, _temp) = create_test_manager();

        // Create connections with base name and protocol suffix
        manager
            .create_connection(
                "server".to_string(),
                "example.com".to_string(),
                22,
                ProtocolConfig::Ssh(SshConfig::default()),
            )
            .unwrap();

        manager
            .create_connection(
                "server (SSH)".to_string(),
                "example2.com".to_string(),
                22,
                ProtocolConfig::Ssh(SshConfig::default()),
            )
            .unwrap();

        // Generate unique name - should get numeric suffix
        let name = manager.generate_unique_name("server", crate::models::ProtocolType::Ssh);
        assert_eq!(name, "server (SSH) 2");
    }

    #[test]
    fn test_name_exists() {
        let (mut manager, _temp) = create_test_manager();

        let id = manager
            .create_connection(
                "Test Server".to_string(),
                "example.com".to_string(),
                22,
                ProtocolConfig::Ssh(SshConfig::default()),
            )
            .unwrap();

        // Name should exist
        assert!(manager.name_exists("Test Server", None));
        // Case insensitive
        assert!(manager.name_exists("test server", None));
        // Different name should not exist
        assert!(!manager.name_exists("Other Server", None));
        // Should not exist when excluding the connection
        assert!(!manager.name_exists("Test Server", Some(id)));
    }

    #[test]
    fn test_normalize_name_removes_protocol_suffix() {
        let (mut manager, _temp) = create_test_manager();

        // Create a connection with protocol suffix
        let id = manager
            .create_connection(
                "server (SSH)".to_string(),
                "example.com".to_string(),
                22,
                ProtocolConfig::Ssh(SshConfig::default()),
            )
            .unwrap();

        // Base name "server" is now unique, so suffix should be removed
        let normalized = manager.normalize_name("server (SSH)", id);
        assert_eq!(normalized, "server");
    }

    #[test]
    fn test_normalize_name_removes_numeric_suffix() {
        let (mut manager, _temp) = create_test_manager();

        // Create a connection with numeric suffix
        let id = manager
            .create_connection(
                "server (SSH) 2".to_string(),
                "example.com".to_string(),
                22,
                ProtocolConfig::Ssh(SshConfig::default()),
            )
            .unwrap();

        // Base name "server" is now unique, so suffix should be removed
        let normalized = manager.normalize_name("server (SSH) 2", id);
        assert_eq!(normalized, "server");
    }

    #[test]
    fn test_normalize_name_keeps_suffix_when_conflict() {
        let (mut manager, _temp) = create_test_manager();

        // Create two connections
        manager
            .create_connection(
                "server".to_string(),
                "example.com".to_string(),
                22,
                ProtocolConfig::Ssh(SshConfig::default()),
            )
            .unwrap();

        let id2 = manager
            .create_connection(
                "server (SSH)".to_string(),
                "example2.com".to_string(),
                22,
                ProtocolConfig::Ssh(SshConfig::default()),
            )
            .unwrap();

        // Base name "server" is taken, so suffix should be kept
        let normalized = manager.normalize_name("server (SSH)", id2);
        assert_eq!(normalized, "server (SSH)");
    }

    #[test]
    fn test_extract_base_name() {
        // Test protocol suffix extraction
        assert_eq!(
            ConnectionManager::extract_base_name("server (SSH)"),
            Some("server".to_string())
        );
        assert_eq!(
            ConnectionManager::extract_base_name("server (RDP)"),
            Some("server".to_string())
        );
        assert_eq!(
            ConnectionManager::extract_base_name("server (VNC)"),
            Some("server".to_string())
        );
        assert_eq!(
            ConnectionManager::extract_base_name("server (SPICE)"),
            Some("server".to_string())
        );

        // Test numeric suffix extraction
        assert_eq!(
            ConnectionManager::extract_base_name("server (SSH) 2"),
            Some("server".to_string())
        );
        assert_eq!(
            ConnectionManager::extract_base_name("server (RDP) 10"),
            Some("server".to_string())
        );

        // Test no suffix
        assert_eq!(ConnectionManager::extract_base_name("server"), None);
        assert_eq!(ConnectionManager::extract_base_name("my server name"), None);

        // Test partial matches that shouldn't extract
        assert_eq!(
            ConnectionManager::extract_base_name("server (custom)"),
            None
        );
    }
}
