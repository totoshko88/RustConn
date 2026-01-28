//! Import preview and merge strategies.
//!
//! Provides functionality for previewing imports before applying them
//! and handling duplicate connections during re-import.

use std::collections::HashMap;

use uuid::Uuid;

use crate::models::{Connection, ConnectionGroup};

use super::ImportResult;

/// Strategy for handling duplicate connections during import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MergeStrategy {
    /// Skip connections that already exist (match by host+port)
    #[default]
    SkipExisting,
    /// Update existing connections with imported data
    UpdateExisting,
    /// Create duplicates (import all, even if duplicates exist)
    CreateDuplicates,
    /// Ask user for each duplicate (requires UI callback)
    AskForEach,
}

impl MergeStrategy {
    /// Returns all available merge strategies
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::SkipExisting,
            Self::UpdateExisting,
            Self::CreateDuplicates,
            Self::AskForEach,
        ]
    }

    /// Returns the display name for this strategy
    #[must_use]
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::SkipExisting => "Skip existing",
            Self::UpdateExisting => "Update existing",
            Self::CreateDuplicates => "Create duplicates",
            Self::AskForEach => "Ask for each",
        }
    }

    /// Returns a description of this strategy
    #[must_use]
    pub const fn description(&self) -> &'static str {
        match self {
            Self::SkipExisting => "Skip connections that already exist (same host and port)",
            Self::UpdateExisting => "Update existing connections with imported data",
            Self::CreateDuplicates => "Import all connections, even if duplicates exist",
            Self::AskForEach => "Prompt for each duplicate connection",
        }
    }
}

/// Action to take for a specific duplicate connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplicateAction {
    /// Skip this connection
    Skip,
    /// Update the existing connection
    Update,
    /// Create a duplicate
    CreateDuplicate,
}

/// A preview entry for a connection to be imported.
#[derive(Debug, Clone)]
pub struct PreviewConnection {
    /// The connection to import
    pub connection: Connection,
    /// Whether this connection already exists
    pub is_duplicate: bool,
    /// ID of existing connection if duplicate
    pub existing_id: Option<Uuid>,
    /// Suggested action based on merge strategy
    pub suggested_action: DuplicateAction,
}

/// A preview entry for a group to be imported.
#[derive(Debug, Clone)]
pub struct PreviewGroup {
    /// The group to import
    pub group: ConnectionGroup,
    /// Whether this group already exists
    pub is_duplicate: bool,
    /// ID of existing group if duplicate
    pub existing_id: Option<Uuid>,
}

/// Preview of an import operation before it's applied.
#[derive(Debug, Clone, Default)]
pub struct ImportPreview {
    /// Connections to be imported with their status
    pub connections: Vec<PreviewConnection>,
    /// Groups to be imported with their status
    pub groups: Vec<PreviewGroup>,
    /// Source identifier (e.g., "ssh_config", "remmina")
    pub source_id: String,
    /// Source path or description
    pub source_path: String,
    /// Number of entries that will be skipped
    pub skip_count: usize,
    /// Number of entries that will be updated
    pub update_count: usize,
    /// Number of new entries that will be created
    pub new_count: usize,
    /// Number of duplicates that will be created
    pub duplicate_count: usize,
}

impl ImportPreview {
    /// Creates a new empty preview
    #[must_use]
    pub fn new(source_id: impl Into<String>, source_path: impl Into<String>) -> Self {
        Self {
            source_id: source_id.into(),
            source_path: source_path.into(),
            ..Default::default()
        }
    }

    /// Creates a preview from an import result and existing connections.
    ///
    /// # Arguments
    ///
    /// * `result` - The import result to preview
    /// * `existing_connections` - Existing connections to check for duplicates
    /// * `existing_groups` - Existing groups to check for duplicates
    /// * `strategy` - The merge strategy to use for determining actions
    ///
    /// # Performance Note
    ///
    /// This method clones connections for the preview. For very large imports (1000+),
    /// consider using indices or implementing a streaming preview approach.
    #[must_use]
    pub fn from_result(
        result: &ImportResult,
        existing_connections: &[Connection],
        existing_groups: &[ConnectionGroup],
        strategy: MergeStrategy,
        source_id: impl Into<String>,
        source_path: impl Into<String>,
    ) -> Self {
        let mut preview = Self::new(source_id, source_path);

        // Build lookup maps for existing items using references to avoid cloning
        let conn_lookup: HashMap<(&str, u16), &Connection> = existing_connections
            .iter()
            .map(|c| ((c.host.as_str(), c.port), c))
            .collect();

        let group_lookup: HashMap<(&str, Option<Uuid>), &ConnectionGroup> = existing_groups
            .iter()
            .map(|g| ((g.name.as_str(), g.parent_id), g))
            .collect();

        // Process connections
        for conn in &result.connections {
            let key = (conn.host.as_str(), conn.port);
            let existing = conn_lookup.get(&key);

            let (is_duplicate, existing_id, suggested_action) =
                if let Some(existing_conn) = existing {
                    let action = match strategy {
                        MergeStrategy::SkipExisting => DuplicateAction::Skip,
                        MergeStrategy::UpdateExisting => DuplicateAction::Update,
                        MergeStrategy::CreateDuplicates => DuplicateAction::CreateDuplicate,
                        MergeStrategy::AskForEach => DuplicateAction::Skip, // Default, UI will override
                    };
                    (true, Some(existing_conn.id), action)
                } else {
                    (false, None, DuplicateAction::CreateDuplicate)
                };

            // Update counts
            match (is_duplicate, suggested_action) {
                (true, DuplicateAction::Skip) => preview.skip_count += 1,
                (true, DuplicateAction::Update) => preview.update_count += 1,
                (true, DuplicateAction::CreateDuplicate) => preview.duplicate_count += 1,
                (false, _) => preview.new_count += 1,
            }

            preview.connections.push(PreviewConnection {
                connection: conn.clone(),
                is_duplicate,
                existing_id,
                suggested_action,
            });
        }

        // Process groups
        for group in &result.groups {
            let key = (group.name.as_str(), group.parent_id);
            let existing = group_lookup.get(&key);

            let (is_duplicate, existing_id) = if let Some(existing_group) = existing {
                (true, Some(existing_group.id))
            } else {
                (false, None)
            };

            preview.groups.push(PreviewGroup {
                group: group.clone(),
                is_duplicate,
                existing_id,
            });
        }

        preview
    }

    /// Returns the total number of connections in the preview
    #[must_use]
    pub fn total_connections(&self) -> usize {
        self.connections.len()
    }

    /// Returns the total number of groups in the preview
    #[must_use]
    pub fn total_groups(&self) -> usize {
        self.groups.len()
    }

    /// Returns the number of duplicate connections
    #[must_use]
    pub fn duplicate_connections(&self) -> usize {
        self.connections.iter().filter(|c| c.is_duplicate).count()
    }

    /// Returns the number of new connections (non-duplicates)
    #[must_use]
    pub fn new_connections(&self) -> usize {
        self.connections.iter().filter(|c| !c.is_duplicate).count()
    }

    /// Returns a summary string of the preview
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "New: {}, Update: {}, Skip: {}, Duplicates: {}, Groups: {}",
            self.new_count,
            self.update_count,
            self.skip_count,
            self.duplicate_count,
            self.groups.len()
        )
    }

    /// Updates the action for a specific connection by index.
    ///
    /// Returns `true` if the action was updated, `false` if index is out of bounds.
    pub fn set_connection_action(&mut self, index: usize, action: DuplicateAction) -> bool {
        if let Some(preview_conn) = self.connections.get_mut(index) {
            let old_action = preview_conn.suggested_action;
            preview_conn.suggested_action = action;

            // Update counts
            if preview_conn.is_duplicate {
                match old_action {
                    DuplicateAction::Skip => self.skip_count = self.skip_count.saturating_sub(1),
                    DuplicateAction::Update => {
                        self.update_count = self.update_count.saturating_sub(1);
                    }
                    DuplicateAction::CreateDuplicate => {
                        self.duplicate_count = self.duplicate_count.saturating_sub(1);
                    }
                }
                match action {
                    DuplicateAction::Skip => self.skip_count += 1,
                    DuplicateAction::Update => self.update_count += 1,
                    DuplicateAction::CreateDuplicate => self.duplicate_count += 1,
                }
            }

            true
        } else {
            false
        }
    }

    /// Applies the preview to generate the final import result.
    ///
    /// Returns connections to create, connections to update (with their existing IDs),
    /// and groups to create.
    #[must_use]
    pub fn apply(
        &self,
    ) -> (
        Vec<Connection>,
        Vec<(Uuid, Connection)>,
        Vec<ConnectionGroup>,
    ) {
        let mut to_create = Vec::new();
        let mut to_update = Vec::new();

        for preview_conn in &self.connections {
            match preview_conn.suggested_action {
                DuplicateAction::Skip => {
                    // Skip this connection
                }
                DuplicateAction::Update => {
                    if let Some(existing_id) = preview_conn.existing_id {
                        to_update.push((existing_id, preview_conn.connection.clone()));
                    }
                }
                DuplicateAction::CreateDuplicate => {
                    to_create.push(preview_conn.connection.clone());
                }
            }
        }

        // For groups, only create non-duplicates
        let groups_to_create: Vec<ConnectionGroup> = self
            .groups
            .iter()
            .filter(|g| !g.is_duplicate)
            .map(|g| g.group.clone())
            .collect();

        (to_create, to_update, groups_to_create)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ProtocolConfig;

    fn create_test_connection(name: &str, host: &str, port: u16) -> Connection {
        Connection::new(
            name.to_string(),
            host.to_string(),
            port,
            ProtocolConfig::Ssh(crate::models::SshConfig::default()),
        )
    }

    #[test]
    fn test_merge_strategy_display() {
        assert_eq!(MergeStrategy::SkipExisting.display_name(), "Skip existing");
        assert_eq!(
            MergeStrategy::UpdateExisting.display_name(),
            "Update existing"
        );
        assert_eq!(
            MergeStrategy::CreateDuplicates.display_name(),
            "Create duplicates"
        );
    }

    #[test]
    fn test_preview_no_duplicates() {
        let mut result = ImportResult::new();
        result.add_connection(create_test_connection("Server 1", "host1.example.com", 22));
        result.add_connection(create_test_connection("Server 2", "host2.example.com", 22));

        let existing: Vec<Connection> = vec![];
        let existing_groups: Vec<ConnectionGroup> = vec![];

        let preview = ImportPreview::from_result(
            &result,
            &existing,
            &existing_groups,
            MergeStrategy::SkipExisting,
            "test",
            "test.yml",
        );

        assert_eq!(preview.total_connections(), 2);
        assert_eq!(preview.new_connections(), 2);
        assert_eq!(preview.duplicate_connections(), 0);
        assert_eq!(preview.new_count, 2);
        assert_eq!(preview.skip_count, 0);
    }

    #[test]
    fn test_preview_with_duplicates_skip() {
        let mut result = ImportResult::new();
        result.add_connection(create_test_connection("Server 1", "host1.example.com", 22));
        result.add_connection(create_test_connection("Server 2", "host2.example.com", 22));

        let existing = vec![create_test_connection(
            "Existing Server",
            "host1.example.com",
            22,
        )];
        let existing_groups: Vec<ConnectionGroup> = vec![];

        let preview = ImportPreview::from_result(
            &result,
            &existing,
            &existing_groups,
            MergeStrategy::SkipExisting,
            "test",
            "test.yml",
        );

        assert_eq!(preview.total_connections(), 2);
        assert_eq!(preview.new_connections(), 1);
        assert_eq!(preview.duplicate_connections(), 1);
        assert_eq!(preview.new_count, 1);
        assert_eq!(preview.skip_count, 1);
    }

    #[test]
    fn test_preview_with_duplicates_update() {
        let mut result = ImportResult::new();
        result.add_connection(create_test_connection("Server 1", "host1.example.com", 22));

        let existing = vec![create_test_connection(
            "Existing Server",
            "host1.example.com",
            22,
        )];
        let existing_groups: Vec<ConnectionGroup> = vec![];

        let preview = ImportPreview::from_result(
            &result,
            &existing,
            &existing_groups,
            MergeStrategy::UpdateExisting,
            "test",
            "test.yml",
        );

        assert_eq!(preview.update_count, 1);
        assert_eq!(preview.skip_count, 0);
        assert!(preview.connections[0].is_duplicate);
        assert_eq!(
            preview.connections[0].suggested_action,
            DuplicateAction::Update
        );
    }

    #[test]
    fn test_preview_with_duplicates_create() {
        let mut result = ImportResult::new();
        result.add_connection(create_test_connection("Server 1", "host1.example.com", 22));

        let existing = vec![create_test_connection(
            "Existing Server",
            "host1.example.com",
            22,
        )];
        let existing_groups: Vec<ConnectionGroup> = vec![];

        let preview = ImportPreview::from_result(
            &result,
            &existing,
            &existing_groups,
            MergeStrategy::CreateDuplicates,
            "test",
            "test.yml",
        );

        assert_eq!(preview.duplicate_count, 1);
        assert_eq!(preview.skip_count, 0);
        assert!(preview.connections[0].is_duplicate);
        assert_eq!(
            preview.connections[0].suggested_action,
            DuplicateAction::CreateDuplicate
        );
    }

    #[test]
    fn test_set_connection_action() {
        let mut result = ImportResult::new();
        result.add_connection(create_test_connection("Server 1", "host1.example.com", 22));

        let existing = vec![create_test_connection(
            "Existing Server",
            "host1.example.com",
            22,
        )];
        let existing_groups: Vec<ConnectionGroup> = vec![];

        let mut preview = ImportPreview::from_result(
            &result,
            &existing,
            &existing_groups,
            MergeStrategy::SkipExisting,
            "test",
            "test.yml",
        );

        assert_eq!(preview.skip_count, 1);
        assert_eq!(preview.update_count, 0);

        // Change action from Skip to Update
        assert!(preview.set_connection_action(0, DuplicateAction::Update));

        assert_eq!(preview.skip_count, 0);
        assert_eq!(preview.update_count, 1);
    }

    #[test]
    fn test_apply_preview() {
        let mut result = ImportResult::new();
        result.add_connection(create_test_connection("New Server", "new.example.com", 22));
        result.add_connection(create_test_connection(
            "Duplicate Server",
            "existing.example.com",
            22,
        ));

        let existing = vec![create_test_connection(
            "Existing Server",
            "existing.example.com",
            22,
        )];
        let existing_groups: Vec<ConnectionGroup> = vec![];

        let mut preview = ImportPreview::from_result(
            &result,
            &existing,
            &existing_groups,
            MergeStrategy::SkipExisting,
            "test",
            "test.yml",
        );

        // Change duplicate to update
        preview.set_connection_action(1, DuplicateAction::Update);

        let (to_create, to_update, _groups) = preview.apply();

        assert_eq!(to_create.len(), 1);
        assert_eq!(to_create[0].host, "new.example.com");

        assert_eq!(to_update.len(), 1);
        assert_eq!(to_update[0].1.host, "existing.example.com");
    }

    #[test]
    fn test_group_deduplication() {
        let mut result = ImportResult::new();
        result.add_group(ConnectionGroup::new("Production".to_string()));
        result.add_group(ConnectionGroup::new("Development".to_string()));

        let existing_groups = vec![ConnectionGroup::new("Production".to_string())];
        let existing_connections: Vec<Connection> = vec![];

        let preview = ImportPreview::from_result(
            &result,
            &existing_connections,
            &existing_groups,
            MergeStrategy::SkipExisting,
            "test",
            "test.yml",
        );

        assert_eq!(preview.total_groups(), 2);

        let duplicate_groups = preview.groups.iter().filter(|g| g.is_duplicate).count();
        let new_groups = preview.groups.iter().filter(|g| !g.is_duplicate).count();

        assert_eq!(duplicate_groups, 1);
        assert_eq!(new_groups, 1);
    }

    #[test]
    fn test_summary() {
        let mut result = ImportResult::new();
        result.add_connection(create_test_connection("Server 1", "host1.example.com", 22));
        result.add_connection(create_test_connection("Server 2", "host2.example.com", 22));
        result.add_group(ConnectionGroup::new("Group".to_string()));

        let existing = vec![create_test_connection("Existing", "host1.example.com", 22)];
        let existing_groups: Vec<ConnectionGroup> = vec![];

        let preview = ImportPreview::from_result(
            &result,
            &existing,
            &existing_groups,
            MergeStrategy::SkipExisting,
            "test",
            "test.yml",
        );

        let summary = preview.summary();
        assert!(summary.contains("New: 1"));
        assert!(summary.contains("Skip: 1"));
        assert!(summary.contains("Groups: 1"));
    }
}
