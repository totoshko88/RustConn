//! Snippet model for reusable command templates.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A reusable command template
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snippet {
    /// Unique identifier for the snippet
    pub id: Uuid,
    /// Human-readable name for the snippet
    pub name: String,
    /// Optional description of what the snippet does
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Command template (may contain ${variable} placeholders)
    pub command: String,
    /// Variables that can be substituted in the command
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<SnippetVariable>,
    /// Category for organization
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Tags for filtering
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// When the snippet was created
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
    /// When the snippet was last modified
    #[serde(default = "Utc::now")]
    pub updated_at: DateTime<Utc>,
}

impl Snippet {
    /// Creates a new snippet with the given name and command
    #[must_use]
    pub fn new(name: String, command: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            description: None,
            command,
            variables: Vec::new(),
            category: None,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Sets the description for this snippet
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Sets the category for this snippet
    #[must_use]
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// Adds variables to this snippet
    #[must_use]
    pub fn with_variables(mut self, variables: Vec<SnippetVariable>) -> Self {
        self.variables = variables;
        self
    }

    /// Adds tags to this snippet
    #[must_use]
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }
}

/// A variable placeholder in a snippet command
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnippetVariable {
    /// Variable name (used in ${name} placeholders)
    pub name: String,
    /// Optional description of the variable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Default value for the variable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_value: Option<String>,
}

impl SnippetVariable {
    /// Creates a new variable with the given name
    #[must_use]
    pub const fn new(name: String) -> Self {
        Self {
            name,
            description: None,
            default_value: None,
        }
    }

    /// Sets the description for this variable
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Sets the default value for this variable
    #[must_use]
    pub fn with_default(mut self, default_value: impl Into<String>) -> Self {
        self.default_value = Some(default_value.into());
        self
    }
}
