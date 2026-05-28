//! Snippet model for reusable command templates.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Target execution platform for a snippet.
///
/// Determines where and how the snippet command will be executed:
/// - `Terminal` — sent directly to a VTE terminal (SSH/local shell)
/// - `Windows` — executed on a remote Windows machine via RDP clipboard+paste
/// - `Any` — available in both contexts
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SnippetTarget {
    /// Execute in VTE terminal (Linux/SSH/local shell)
    #[default]
    Terminal,
    /// Execute via RDP clipboard+paste (Windows PowerShell)
    Windows,
    /// Execute in both contexts (universal commands)
    Any,
}

/// Delivery method for sending a snippet to the remote session.
///
/// Controls whether the snippet text is sent via clipboard paste (fast) or
/// character-by-character autotype (reliable for problematic environments).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScriptDelivery {
    /// Use the connection-level default (`script_paste_via_clipboard` setting)
    #[default]
    Auto,
    /// Always send via clipboard + Ctrl+V (instant, even for large scripts)
    Clipboard,
    /// Always send character-by-character (reliable for Citrix, old CMD, OTP fields)
    Autotype,
}

impl ScriptDelivery {
    /// All variants for UI dropdowns.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Auto, Self::Clipboard, Self::Autotype]
    }

    /// Display name for UI (English, wrapped in i18n at call site).
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Clipboard => "Clipboard (fast)",
            Self::Autotype => "Autotype (reliable)",
        }
    }
}

impl SnippetTarget {
    /// Returns `true` if this snippet is available for VTE terminal sessions.
    #[must_use]
    pub const fn is_terminal_compatible(self) -> bool {
        matches!(self, Self::Terminal | Self::Any)
    }

    /// Returns `true` if this snippet is available for RDP (Windows) sessions.
    #[must_use]
    pub const fn is_windows_compatible(self) -> bool {
        matches!(self, Self::Windows | Self::Any)
    }
}

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
    /// Target execution platform (terminal, windows, or any)
    #[serde(default)]
    pub target: SnippetTarget,
    /// Delivery method for RDP sessions (auto, clipboard, or autotype)
    #[serde(default, skip_serializing_if = "is_default_delivery")]
    pub delivery: ScriptDelivery,
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
            target: SnippetTarget::default(),
            delivery: ScriptDelivery::default(),
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

    /// Sets the target execution platform for this snippet
    #[must_use]
    pub const fn with_target(mut self, target: SnippetTarget) -> Self {
        self.target = target;
        self
    }

    /// Sets the delivery method for this snippet
    #[must_use]
    pub const fn with_delivery(mut self, delivery: ScriptDelivery) -> Self {
        self.delivery = delivery;
        self
    }
}

/// Helper for serde `skip_serializing_if` — skips when delivery is `Auto`.
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "&self chosen for API consistency with sibling impls that need the borrow"
)]
fn is_default_delivery(d: &ScriptDelivery) -> bool {
    matches!(d, ScriptDelivery::Auto)
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
