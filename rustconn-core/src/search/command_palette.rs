//! Command Palette types for quick access to connections and actions.
//!
//! Provides the data model for a VS Code-style command palette:
//! - Empty query → recent connections
//! - `>` prefix → application commands
//! - `@` prefix → filter by tags
//! - `#` prefix → filter by groups
//! - `!` prefix → run a saved snippet
//! - Plain text → fuzzy search connections

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Action that can be executed from the Command Palette
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommandPaletteAction {
    /// Connect to a saved connection
    Connect(Uuid),
    /// Switch to an already-open tab by session ID
    SwitchTab(Uuid),
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
    /// Run a saved snippet in the active terminal, by snippet ID. The GUI
    /// dispatches it through the existing snippet execution path, so snippet
    /// variables and the per-snippet "confirm before run" still apply.
    RunSnippet(Uuid),
    /// Execute a named GTK action (e.g. "win.toggle-fullscreen")
    GtkAction(String),
}

impl CommandPaletteAction {
    /// The GTK action whose key binding the palette shows beside this command.
    ///
    /// The GUI looks the binding up in the user's keybinding settings, so a
    /// rebinding reaches the palette. `None` for actions that are not commands.
    #[must_use]
    pub fn keybinding_action(&self) -> Option<&str> {
        match self {
            Self::OpenSettings => Some("win.settings"),
            Self::NewConnection => Some("win.new-connection"),
            Self::NewGroup => Some("win.new-group"),
            Self::Import => Some("win.import"),
            Self::Export => Some("win.export"),
            Self::LocalShell => Some("win.local-shell"),
            Self::QuickConnect => Some("win.quick-connect"),
            Self::GtkAction(name) => Some(name),
            Self::Connect(_) | Self::SwitchTab(_) | Self::RunSnippet(_) => None,
        }
    }
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
    /// `%` prefix: switch to an open tab
    OpenTabs,
    /// `!` prefix: run a saved snippet
    Snippets,
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
    } else if let Some(rest) = trimmed.strip_prefix('%') {
        (PaletteMode::OpenTabs, rest.trim_start())
    } else if let Some(rest) = trimmed.strip_prefix('!') {
        (PaletteMode::Snippets, rest.trim_start())
    } else {
        (PaletteMode::Connections, trimmed)
    }
}

/// Returns the built-in application commands for the `>` mode
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "one flat list of palette entries; splitting it only relocates the ordering"
)]
pub fn builtin_commands() -> Vec<PaletteItem> {
    vec![
        PaletteItem::new("New Connection", CommandPaletteAction::NewConnection)
            .with_icon("list-add-symbolic")
            .with_priority(90),
        PaletteItem::new("New Group", CommandPaletteAction::NewGroup)
            .with_icon("folder-new-symbolic")
            .with_priority(85),
        PaletteItem::new("Local Shell", CommandPaletteAction::LocalShell)
            .with_icon("utilities-terminal-symbolic")
            .with_priority(80),
        PaletteItem::new("Quick Connect", CommandPaletteAction::QuickConnect)
            .with_icon("network-server-symbolic")
            .with_priority(75),
        PaletteItem::new("Import", CommandPaletteAction::Import)
            .with_icon("document-open-symbolic")
            .with_priority(60),
        PaletteItem::new("Export", CommandPaletteAction::Export)
            .with_icon("media-floppy-symbolic")
            .with_priority(55),
        PaletteItem::new("Settings", CommandPaletteAction::OpenSettings)
            .with_icon("preferences-system-symbolic")
            .with_priority(50),
        PaletteItem::new(
            "Toggle Fullscreen",
            CommandPaletteAction::GtkAction("win.toggle-fullscreen".into()),
        )
        .with_icon("view-fullscreen-symbolic")
        .with_priority(40),
        PaletteItem::new(
            "Toggle Keyboard Passthrough",
            CommandPaletteAction::GtkAction("win.toggle-passthrough".into()),
        )
        .with_icon("input-keyboard-symbolic")
        .with_priority(38),
        PaletteItem::new(
            "Split Down",
            CommandPaletteAction::GtkAction("win.split-horizontal".into()),
        )
        .with_icon("view-dual-symbolic")
        .with_priority(35),
        PaletteItem::new(
            "Split Right",
            CommandPaletteAction::GtkAction("win.split-vertical".into()),
        )
        .with_icon("view-dual-symbolic")
        .with_priority(30),
        PaletteItem::new(
            "Remove Split",
            CommandPaletteAction::GtkAction("win.unsplit".into()),
        )
        .with_icon("view-restore-symbolic")
        .with_priority(28),
        PaletteItem::new(
            "Network Scan",
            CommandPaletteAction::GtkAction("win.network-scan".into()),
        )
        .with_icon("network-workgroup-symbolic")
        .with_priority(20),
        PaletteItem::new(
            "Join Shared Session…",
            CommandPaletteAction::GtkAction("win.join-shared-session".into()),
        )
        .with_icon("network-transmit-receive-symbolic")
        .with_priority(18),
        PaletteItem::new(
            "Keyboard Shortcuts",
            CommandPaletteAction::GtkAction("app.shortcuts".into()),
        )
        .with_icon("preferences-desktop-keyboard-shortcuts-symbolic")
        .with_priority(20),
        PaletteItem::new(
            "Tab Overview",
            CommandPaletteAction::GtkAction("win.tab-overview".into()),
        )
        .with_icon("view-grid-symbolic")
        .with_priority(25),
        PaletteItem::new(
            "Switch Tab",
            CommandPaletteAction::GtkAction("win.switch-tab-palette".into()),
        )
        .with_icon("tab-new-symbolic")
        .with_priority(22),
        PaletteItem::new(
            "Share Session (read-only)",
            CommandPaletteAction::GtkAction("win.share-session".into()),
        )
        .with_icon("emblem-shared-symbolic")
        .with_priority(18),
        PaletteItem::new(
            "Stop Sharing",
            CommandPaletteAction::GtkAction("win.stop-sharing".into()),
        )
        .with_icon("process-stop-symbolic")
        .with_priority(15),
        PaletteItem::new(
            "Stop Script Recording",
            CommandPaletteAction::GtkAction("win.stop-record-script".into()),
        )
        .with_icon("media-playback-stop-symbolic")
        .with_priority(14),
        PaletteItem::new(
            "Start TFTP Server",
            CommandPaletteAction::GtkAction("win.start-tftp".into()),
        )
        .with_icon("network-workgroup-symbolic")
        .with_priority(17),
        PaletteItem::new(
            "Edit Remote File",
            CommandPaletteAction::GtkAction("win.edit-remote-file".into()),
        )
        .with_icon("document-edit-symbolic")
        .with_priority(16),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Palette actions with no entry in the keybinding registry (no default key).
    const UNBOUND_BY_DEFAULT: &[&str] = &[
        "win.network-scan",
        "win.join-shared-session",
        "win.share-session",
        "win.start-tftp",
        "win.edit-remote-file",
        "win.stop-sharing",
        "win.stop-record-script",
    ];

    #[test]
    fn builtin_commands_hold_no_typed_shortcut_text() {
        // The key shown beside a command comes from the user's keybindings at
        // display time; a string typed here would show the default after a
        // rebinding, and F1 where Ctrl+? is bound.
        for item in builtin_commands() {
            assert!(
                item.description.is_none(),
                "{} carries a typed description",
                item.label
            );
        }
    }

    #[test]
    fn keybinding_actions_exist_in_the_registry() {
        let registry = crate::config::keybindings::default_keybindings();
        let mut bound = 0;
        for item in builtin_commands() {
            if let Some(action) = item.action.keybinding_action() {
                bound += 1;
                // `win.network-scan` and the like have no default key; the ones
                // listed here must still be real actions of the window.
                let known = registry.iter().any(|d| d.action == action)
                    || UNBOUND_BY_DEFAULT.contains(&action);
                assert!(known, "{} maps to unknown action {action}", item.label);
            }
        }
        assert!(bound >= 10, "only {bound} commands map to an action");
    }

    #[test]
    fn shortcuts_command_maps_to_the_shortcuts_action() {
        let item = builtin_commands()
            .into_iter()
            .find(
                |i| matches!(&i.action, CommandPaletteAction::GtkAction(a) if a == "app.shortcuts"),
            )
            .expect("shortcuts command");
        assert_eq!(item.action.keybinding_action(), Some("app.shortcuts"));
    }

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
    fn test_parse_palette_input_open_tabs() {
        let (mode, query) = parse_palette_input("%prod");
        assert_eq!(mode, PaletteMode::OpenTabs);
        assert_eq!(query, "prod");
    }

    #[test]
    fn test_parse_palette_input_snippets() {
        let (mode, query) = parse_palette_input("!deploy");
        assert_eq!(mode, PaletteMode::Snippets);
        assert_eq!(query, "deploy");
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
