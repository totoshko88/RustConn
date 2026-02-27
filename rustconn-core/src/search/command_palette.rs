//! Command Palette types for quick access to connections and actions.
//!
//! Provides the data model for a VS Code-style command palette:
//! - Empty query → recent connections
//! - `>` prefix → application commands
//! - `@` prefix → filter by tags
//! - `#` prefix → filter by groups
//! - Plain text → fuzzy search connections

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Action that can be executed from the Command Palette
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandPaletteAction {
    /// Connect to a saved connection
    Connect(Uuid),
    /// Open application settings
    OpenSettings,
    /// Create a new connection
    NewConnection,
    /// Create a new group
    NewGroup,
    /// Import connections from file
    Import,
    /// Export connections to file
    Export,
    /// Open a local shell tab
    LocalShell,
    /// Quick connect (ad-hoc host:port)
    QuickConnect,
    /// Execute a named GTK action (e.g. "win.toggle-fullscreen")
    GtkAction(String),
}

/// A single item displayed in the Command Palette results list
#[derive(Debug, Clone)]
pub struct PaletteItem {
    /// Display label
    pub label: String,
    /// Optional description / subtitle
    pub description: Option<String>,
    /// Icon name (GTK icon-name)
    pub icon: Option<String>,
    /// Action to execute when selected
    pub action: CommandPaletteAction,
    /// Sort priority (higher = closer to top)
    pub priority: i32,
}

impl PaletteItem {
    /// Creates a new palette item
    #[must_use]
    pub fn new(label: impl Into<String>, action: CommandPaletteAction) -> Self {
        Self {
            label: label.into(),
            description: None,
            icon: None,
            action,
            priority: 0,
        }
    }

    /// Sets the description
    #[must_use]
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Sets the icon name
    #[must_use]
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Sets the sort priority
    #[must_use]
    pub const fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }
}

/// Parsed prefix mode from the palette search entry
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteMode {
    /// Default: search connections
    Connections,
    /// `>` prefix: search commands
    Commands,
    /// `@` prefix: filter by tag
    Tags,
    /// `#` prefix: filter by group
    Groups,
}

/// Parses the raw input text into a mode and the remaining query
#[must_use]
pub fn parse_palette_input(input: &str) -> (PaletteMode, &str) {
    let trimmed = input.trim_start();
    if let Some(rest) = trimmed.strip_prefix('>') {
        (PaletteMode::Commands, rest.trim_start())
    } else if let Some(rest) = trimmed.strip_prefix('@') {
        (PaletteMode::Tags, rest.trim_start())
    } else if let Some(rest) = trimmed.strip_prefix('#') {
        (PaletteMode::Groups, rest.trim_start())
    } else {
        (PaletteMode::Connections, trimmed)
    }
}

/// Returns the built-in application commands for the `>` mode
#[must_use]
pub fn builtin_commands() -> Vec<PaletteItem> {
    vec![
        PaletteItem::new("New Connection", CommandPaletteAction::NewConnection)
            .with_icon("list-add-symbolic")
            .with_description("Ctrl+N")
            .with_priority(90),
        PaletteItem::new("New Group", CommandPaletteAction::NewGroup)
            .with_icon("folder-new-symbolic")
            .with_description("Ctrl+Shift+N")
            .with_priority(85),
        PaletteItem::new("Local Shell", CommandPaletteAction::LocalShell)
            .with_icon("utilities-terminal-symbolic")
            .with_description("Ctrl+Shift+T")
            .with_priority(80),
        PaletteItem::new("Quick Connect", CommandPaletteAction::QuickConnect)
            .with_icon("network-server-symbolic")
            .with_description("Ctrl+Shift+Q")
            .with_priority(75),
        PaletteItem::new("Import", CommandPaletteAction::Import)
            .with_icon("document-open-symbolic")
            .with_description("Ctrl+I")
            .with_priority(60),
        PaletteItem::new("Export", CommandPaletteAction::Export)
            .with_icon("document-save-symbolic")
            .with_description("Ctrl+Shift+E")
            .with_priority(55),
        PaletteItem::new("Settings", CommandPaletteAction::OpenSettings)
            .with_icon("preferences-system-symbolic")
            .with_description("Ctrl+,")
            .with_priority(50),
        PaletteItem::new(
            "Toggle Fullscreen",
            CommandPaletteAction::GtkAction("win.toggle-fullscreen".into()),
        )
        .with_icon("view-fullscreen-symbolic")
        .with_description("F11")
        .with_priority(40),
        PaletteItem::new(
            "Split Horizontal",
            CommandPaletteAction::GtkAction("win.split-horizontal".into()),
        )
        .with_icon("view-dual-symbolic")
        .with_description("Ctrl+Shift+H")
        .with_priority(35),
        PaletteItem::new(
            "Split Vertical",
            CommandPaletteAction::GtkAction("win.split-vertical".into()),
        )
        .with_icon("view-dual-symbolic")
        .with_description("Ctrl+Shift+S")
        .with_priority(30),
        PaletteItem::new(
            "Keyboard Shortcuts",
            CommandPaletteAction::GtkAction("app.shortcuts".into()),
        )
        .with_icon("preferences-desktop-keyboard-shortcuts-symbolic")
        .with_description("F1")
        .with_priority(20),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_palette_input_connections() {
        let (mode, query) = parse_palette_input("my server");
        assert_eq!(mode, PaletteMode::Connections);
        assert_eq!(query, "my server");
    }

    #[test]
    fn test_parse_palette_input_commands() {
        let (mode, query) = parse_palette_input("> settings");
        assert_eq!(mode, PaletteMode::Commands);
        assert_eq!(query, "settings");
    }

    #[test]
    fn test_parse_palette_input_tags() {
        let (mode, query) = parse_palette_input("@production");
        assert_eq!(mode, PaletteMode::Tags);
        assert_eq!(query, "production");
    }

    #[test]
    fn test_parse_palette_input_groups() {
        let (mode, query) = parse_palette_input("#servers");
        assert_eq!(mode, PaletteMode::Groups);
        assert_eq!(query, "servers");
    }

    #[test]
    fn test_parse_palette_input_empty() {
        let (mode, query) = parse_palette_input("");
        assert_eq!(mode, PaletteMode::Connections);
        assert_eq!(query, "");
    }

    #[test]
    fn test_builtin_commands_not_empty() {
        let cmds = builtin_commands();
        assert!(!cmds.is_empty());
        // All commands should have icons
        for cmd in &cmds {
            assert!(cmd.icon.is_some(), "Command '{}' missing icon", cmd.label);
        }
    }

    #[test]
    fn test_palette_item_builder() {
        let item = PaletteItem::new("Test", CommandPaletteAction::OpenSettings)
            .with_description("desc")
            .with_icon("icon")
            .with_priority(42);
        assert_eq!(item.label, "Test");
        assert_eq!(item.description.as_deref(), Some("desc"));
        assert_eq!(item.icon.as_deref(), Some("icon"));
        assert_eq!(item.priority, 42);
    }
}
