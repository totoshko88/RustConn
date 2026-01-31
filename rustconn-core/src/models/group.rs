//! Connection group model for hierarchical organization.

use chrono::{DateTime, Utc};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::PasswordSource;

/// A hierarchical group for organizing connections
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionGroup {
    /// Unique identifier for the group
    pub id: Uuid,
    /// Human-readable name for the group
    pub name: String,
    /// Parent group ID (None for root-level groups)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<Uuid>,
    /// Whether the group is expanded in the UI
    #[serde(default)]
    pub expanded: bool,
    /// Timestamp when the group was created
    pub created_at: DateTime<Utc>,
    /// Sort order for manual ordering (lower values appear first)
    #[serde(default)]
    pub sort_order: i32,
    /// Username for inheritance
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Domain for inheritance
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Password source and config for inheritance
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_source: Option<PasswordSource>,
    /// Optional description/notes for the group
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl ConnectionGroup {
    /// Creates a new root-level group
    #[must_use]
    pub fn new(name: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            parent_id: None,
            expanded: true,
            created_at: Utc::now(),
            sort_order: 0,
            username: None,
            domain: None,
            password_source: None,
            description: None,
        }
    }

    /// Creates a new group with a parent
    #[must_use]
    pub fn with_parent(name: String, parent_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            parent_id: Some(parent_id),
            expanded: true,
            created_at: Utc::now(),
            sort_order: 0,
            username: None,
            domain: None,
            password_source: None,
            description: None,
        }
    }

    /// Returns true if this is a root-level group
    #[must_use]
    pub const fn is_root(&self) -> bool {
        self.parent_id.is_none()
    }
}
