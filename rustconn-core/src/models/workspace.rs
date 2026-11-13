//! Workspace profile models
//!
//! A workspace profile is a named set of connections with their layout,
//! allowing users to save and restore entire working contexts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::session::SessionType;

/// An entry within a workspace profile — one connection to open
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    /// Connection ID to open
    pub connection_id: Uuid,
    /// Connection name at save time (for display when connection is deleted)
    pub connection_name: String,
    /// Protocol (ssh, rdp, vnc, etc.)
    pub protocol: String,
    /// Session type (embedded terminal or external window)
    pub session_type: SessionType,
    /// Tab index for ordering
    pub tab_index: usize,
    /// Panel ID for split view placement
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel_id: Option<String>,
    /// Named tab group this session belonged to (e.g. "Production"), if any
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tab_group: Option<String>,
}

impl WorkspaceEntry {
    /// Creates a new workspace entry
    #[must_use]
    pub fn new(
        connection_id: Uuid,
        connection_name: String,
        protocol: String,
        session_type: SessionType,
        tab_index: usize,
    ) -> Self {
        Self {
            connection_id,
            connection_name,
            protocol,
            session_type,
            tab_index,
            panel_id: None,
            tab_group: None,
        }
    }

    /// Sets the panel ID for split view placement
    #[must_use]
    pub fn with_panel_id(mut self, panel_id: impl Into<String>) -> Self {
        self.panel_id = Some(panel_id.into());
        self
    }

    /// Sets the named tab group this session belongs to.
    #[must_use]
    pub fn with_tab_group(mut self, group: impl Into<String>) -> Self {
        self.tab_group = Some(group.into());
        self
    }
}

/// Split layout saved within a workspace profile
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceSplitLayout {
    /// Whether the view is split
    pub is_split: bool,
    /// Split orientation (true = horizontal, false = vertical)
    pub horizontal: bool,
    /// Split ratio (0.0 to 1.0)
    pub split_ratio: f64,
    /// Index (into `WorkspaceProfile::entries`) of the session that occupies the
    /// second (guest) panel. Used for deferred split restore when the guest
    /// session connects asynchronously (e.g. RDP/VNC via vault credential
    /// resolution).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split_guest_entry_index: Option<usize>,
    /// Indices (into `WorkspaceProfile::entries`) of ALL guest sessions in the
    /// split, ordered by panel position. For a single split this has at most
    /// one element (same as `split_guest_entry_index`); for multi-panel splits
    /// (3+ panels) it lists every guest beyond the owner.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub split_guests: Vec<usize>,
    /// Number of additional splits beyond the first (0 = simple 2-panel split).
    /// Each additional split adds one empty panel to the bridge. Used to
    /// recreate the correct number of panels during restore.
    #[serde(default)]
    pub extra_splits: usize,
    /// Direction for each split in tree order (pre-order DFS). First element
    /// is the root split, subsequent are extra splits. `true` = horizontal,
    /// `false` = vertical. Kept in the file for compatibility, but NOT read on
    /// restore: the layout is rebuilt as a balanced grid
    /// (`split_view::run_balanced_splits`). Old profiles that carry it still load.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub split_directions: Vec<bool>,
    /// Index (into `WorkspaceProfile::entries`) of the session that OWNS the
    /// split — the tab on which `win.split-*` must fire during restore.
    /// Without this, `apply_layout` targets whatever tab happens to be active
    /// at idle time (usually the last-created tab), breaking multi-panel restore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split_owner_entry_index: Option<usize>,
}

impl Default for WorkspaceSplitLayout {
    fn default() -> Self {
        Self {
            is_split: false,
            horizontal: true,
            split_ratio: 0.5,
            split_guest_entry_index: None,
            split_guests: Vec::new(),
            extra_splits: 0,
            split_directions: Vec::new(),
            split_owner_entry_index: None,
        }
    }
}

// Manual Eq: f64 split_ratio uses finite values only in practice.
// ponytail: split_ratio is always 0.0..=1.0 from UI; if arbitrary precision needed, use ordered-float
impl Eq for WorkspaceSplitLayout {}

/// A named workspace profile
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceProfile {
    /// Unique identifier
    pub id: Uuid,
    /// Human-readable name (e.g. "Production", "Staging")
    pub name: String,
    /// Optional description
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Connections to open in this workspace
    pub entries: Vec<WorkspaceEntry>,
    /// Split view layout
    #[serde(default)]
    pub split_layout: WorkspaceSplitLayout,
    /// When this profile was created
    pub created_at: DateTime<Utc>,
    /// When this profile was last modified
    pub updated_at: DateTime<Utc>,
    /// Sort order among workspaces (lower = first)
    #[serde(default)]
    pub sort_order: i32,
}

impl WorkspaceProfile {
    /// Creates a new empty workspace profile with the given name
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: None,
            entries: Vec::new(),
            split_layout: WorkspaceSplitLayout::default(),
            created_at: now,
            updated_at: now,
            sort_order: 0,
        }
    }

    /// Sets the description
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Adds an entry to the workspace
    pub fn add_entry(&mut self, entry: WorkspaceEntry) {
        self.entries.push(entry);
        self.touch();
    }

    /// Sets the split layout
    pub fn set_split_layout(&mut self, layout: WorkspaceSplitLayout) {
        self.split_layout = layout;
        self.touch();
    }

    /// Updates the `updated_at` timestamp
    pub fn touch(&mut self) {
        self.updated_at = Utc::now();
    }

    /// Returns the number of connections in this workspace
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Checks if the workspace is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Checks if a specific connection is part of this workspace
    #[must_use]
    pub fn contains_connection(&self, connection_id: Uuid) -> bool {
        self.entries
            .iter()
            .any(|e| e.connection_id == connection_id)
    }

    /// Removes entries referencing a deleted connection
    ///
    /// Returns the number of entries removed.
    pub fn remove_connection(&mut self, connection_id: Uuid) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| e.connection_id != connection_id);
        let removed = before - self.entries.len();
        if removed > 0 {
            // Re-index tab positions
            for (i, entry) in self.entries.iter_mut().enumerate() {
                entry.tab_index = i;
            }
            self.touch();
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_profile_new() {
        let ws = WorkspaceProfile::new("Production");
        assert_eq!(ws.name, "Production");
        assert!(ws.is_empty());
        assert_eq!(ws.entry_count(), 0);
        assert!(ws.description.is_none());
    }

    #[test]
    fn test_workspace_profile_with_description() {
        let ws = WorkspaceProfile::new("Staging").with_description("Staging environment servers");
        assert_eq!(
            ws.description.as_deref(),
            Some("Staging environment servers")
        );
    }

    #[test]
    fn test_workspace_add_entry() {
        let mut ws = WorkspaceProfile::new("Test");
        let conn_id = Uuid::new_v4();
        ws.add_entry(WorkspaceEntry::new(
            conn_id,
            "web01".to_string(),
            "ssh".to_string(),
            SessionType::Embedded,
            0,
        ));
        assert_eq!(ws.entry_count(), 1);
        assert!(ws.contains_connection(conn_id));
    }

    #[test]
    fn test_workspace_remove_connection() {
        let mut ws = WorkspaceProfile::new("Test");
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        ws.add_entry(WorkspaceEntry::new(
            id1,
            "web01".to_string(),
            "ssh".to_string(),
            SessionType::Embedded,
            0,
        ));
        ws.add_entry(WorkspaceEntry::new(
            id2,
            "db01".to_string(),
            "ssh".to_string(),
            SessionType::Embedded,
            1,
        ));

        let removed = ws.remove_connection(id1);
        assert_eq!(removed, 1);
        assert_eq!(ws.entry_count(), 1);
        assert!(!ws.contains_connection(id1));
        assert!(ws.contains_connection(id2));
        // Tab index re-indexed
        assert_eq!(ws.entries[0].tab_index, 0);
    }

    #[test]
    fn test_workspace_entry_with_panel() {
        let entry = WorkspaceEntry::new(
            Uuid::new_v4(),
            "server".to_string(),
            "ssh".to_string(),
            SessionType::Embedded,
            0,
        )
        .with_panel_id("panel-left");
        assert_eq!(entry.panel_id.as_deref(), Some("panel-left"));
    }

    #[test]
    fn test_workspace_serialization_roundtrip() {
        let mut ws = WorkspaceProfile::new("Roundtrip Test");
        ws.add_entry(WorkspaceEntry::new(
            Uuid::new_v4(),
            "host".to_string(),
            "rdp".to_string(),
            SessionType::External,
            0,
        ));
        ws.set_split_layout(WorkspaceSplitLayout {
            is_split: true,
            horizontal: false,
            split_ratio: 0.6,
            split_guest_entry_index: None,
            split_guests: Vec::new(),
            extra_splits: 0,
            split_directions: Vec::new(),
            split_owner_entry_index: None,
        });

        let toml_str = toml::to_string(&ws).expect("serialize");
        let restored: WorkspaceProfile = toml::from_str(&toml_str).expect("deserialize");
        assert_eq!(restored.name, "Roundtrip Test");
        assert_eq!(restored.entry_count(), 1);
        assert!(restored.split_layout.is_split);
    }

    #[test]
    fn split_layout_loads_with_and_without_split_directions() {
        let base = "is_split = true\nhorizontal = true\nsplit_ratio = 0.5\n";
        let old: WorkspaceSplitLayout = toml::from_str(base).expect("old profile");
        assert!(old.split_directions.is_empty());
        let new: WorkspaceSplitLayout =
            toml::from_str(&format!("{base}split_directions = [true, false]\n")).expect("new");
        assert_eq!(new.split_directions, vec![true, false]);
    }
}
