//! Custom keybinding configuration
//!
//! Provides [`KeybindingSettings`] for user-customizable keyboard shortcuts
//! and [`KeybindingDef`] for the default keybinding registry.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Custom keybinding overrides stored in user settings.
///
/// Each entry maps a GTK action name (e.g. `"win.copy"`) to a GTK accelerator
/// string (e.g. `"<Control><Shift>c"`). Actions not present in `overrides`
/// use their built-in defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeybindingSettings {
    /// Action name → accelerator string mapping.
    ///
    /// Only overridden bindings are stored; defaults are implicit.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub overrides: HashMap<String, String>,
}

/// A single keybinding definition with its default accelerator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeybindingDef {
    /// GTK action name (e.g. `"win.new-connection"`)
    pub action: String,
    /// Default accelerator(s), pipe-separated for multiple (e.g. `"<Control>f|<Control>k"`)
    pub default_accels: String,
    /// Human-readable label for the settings UI
    pub label: String,
    /// Category for grouping in the settings UI
    pub category: KeybindingCategory,
}

/// Categories for organizing keybindings in the settings UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeybindingCategory {
    /// Application-level actions (quit, shortcuts help)
    Application,
    /// Connection management (new, import, export)
    Connections,
    /// Navigation (search, focus sidebar/terminal)
    Navigation,
    /// Terminal operations (copy, paste, close tab)
    Terminal,
    /// Split view operations
    SplitView,
    /// View controls (fullscreen)
    View,
}

impl KeybindingCategory {
    /// Returns the display label for this category.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Application => "Application",
            Self::Connections => "Connections",
            Self::Navigation => "Navigation",
            Self::Terminal => "Terminal",
            Self::SplitView => "Split View",
            Self::View => "View",
        }
    }

    /// Returns all categories in display order.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[
            Self::Application,
            Self::Connections,
            Self::Navigation,
            Self::Terminal,
            Self::SplitView,
            Self::View,
        ]
    }
}

impl KeybindingDef {
    /// Creates a new keybinding definition.
    #[must_use]
    pub fn new(
        action: impl Into<String>,
        default_accels: impl Into<String>,
        label: impl Into<String>,
        category: KeybindingCategory,
    ) -> Self {
        Self {
            action: action.into(),
            default_accels: default_accels.into(),
            label: label.into(),
            category,
        }
    }

    /// Splits the default accelerators into a list.
    #[must_use]
    pub fn default_accel_list(&self) -> Vec<&str> {
        self.default_accels.split('|').collect()
    }
}

impl KeybindingSettings {
    /// Returns the accelerator(s) for an action, falling back to the default.
    ///
    /// If the user has overridden the binding, returns the override.
    /// Otherwise returns the default from the provided definition.
    #[must_use]
    pub fn get_accel<'a>(&'a self, def: &'a KeybindingDef) -> &'a str {
        self.overrides
            .get(&def.action)
            .map(String::as_str)
            .unwrap_or(&def.default_accels)
    }

    /// Returns `true` if the user has overridden any keybindings.
    #[must_use]
    pub fn has_overrides(&self) -> bool {
        !self.overrides.is_empty()
    }

    /// Resets a single action to its default binding.
    pub fn reset(&mut self, action: &str) {
        self.overrides.remove(action);
    }

    /// Resets all overrides.
    pub fn reset_all(&mut self) {
        self.overrides.clear();
    }
}

/// Returns the complete list of default keybinding definitions.
///
/// This is the single source of truth for all keyboard shortcuts in the
/// application. The order matches the display order in the settings UI.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn default_keybindings() -> Vec<KeybindingDef> {
    use KeybindingCategory::{Application, Connections, Navigation, SplitView, Terminal, View};
    vec![
        // Application
        KeybindingDef::new("app.quit", "<Control>q", "Quit", Application),
        KeybindingDef::new(
            "app.shortcuts",
            "<Control>question|F1",
            "Keyboard Shortcuts",
            Application,
        ),
        // Connections
        KeybindingDef::new(
            "win.new-connection",
            "<Control>n",
            "New Connection",
            Connections,
        ),
        KeybindingDef::new(
            "win.new-group",
            "<Control><Shift>n",
            "New Group",
            Connections,
        ),
        KeybindingDef::new("win.import", "<Control>i", "Import", Connections),
        KeybindingDef::new("win.export", "<Control><Shift>e", "Export", Connections),
        KeybindingDef::new(
            "win.quick-connect",
            "<Control><Shift>q",
            "Quick Connect",
            Connections,
        ),
        KeybindingDef::new(
            "win.local-shell",
            "<Control><Shift>t",
            "Local Shell",
            Connections,
        ),
        KeybindingDef::new(
            "win.move-to-group",
            "<Control>m",
            "Move to Group",
            Connections,
        ),
        // Navigation
        KeybindingDef::new("win.search", "<Control>f|<Control>k", "Search", Navigation),
        KeybindingDef::new(
            "win.focus-sidebar",
            "<Control>1|<Alt>1",
            "Focus Sidebar",
            Navigation,
        ),
        KeybindingDef::new(
            "win.focus-terminal",
            "<Control>2|<Alt>2",
            "Focus Terminal",
            Navigation,
        ),
        KeybindingDef::new(
            "win.command-palette",
            "<Control>p",
            "Command Palette",
            Navigation,
        ),
        KeybindingDef::new(
            "win.command-palette-commands",
            "<Control><Shift>p",
            "Command Palette (Commands)",
            Navigation,
        ),
        KeybindingDef::new("win.settings", "<Control>comma", "Settings", Navigation),
        // Terminal
        KeybindingDef::new("win.copy", "<Control><Shift>c", "Copy", Terminal),
        KeybindingDef::new("win.paste", "<Control><Shift>v", "Paste", Terminal),
        KeybindingDef::new(
            "win.terminal-search",
            "<Control><Shift>f",
            "Find in Terminal",
            Terminal,
        ),
        KeybindingDef::new("win.close-tab", "<Control>w", "Close Tab", Terminal),
        KeybindingDef::new(
            "win.next-tab",
            "<Control>Tab|<Control>Page_Down",
            "Next Tab",
            Terminal,
        ),
        KeybindingDef::new(
            "win.prev-tab",
            "<Control><Shift>Tab|<Control>Page_Up",
            "Previous Tab",
            Terminal,
        ),
        // Split View
        KeybindingDef::new(
            "win.split-horizontal",
            "<Control><Shift>h",
            "Split Horizontal",
            SplitView,
        ),
        KeybindingDef::new(
            "win.split-vertical",
            "<Control><Shift>s",
            "Split Vertical",
            SplitView,
        ),
        KeybindingDef::new(
            "win.close-pane",
            "<Control><Shift>w",
            "Close Pane",
            SplitView,
        ),
        KeybindingDef::new(
            "win.focus-next-pane",
            "<Control>grave",
            "Focus Next Pane",
            SplitView,
        ),
        // View
        KeybindingDef::new("win.toggle-fullscreen", "F11", "Toggle Fullscreen", View),
    ]
}

/// Validates a GTK accelerator string.
///
/// Returns `true` if the string looks like a valid GTK accelerator
/// (contains at least one key name, optionally with modifiers).
#[must_use]
pub fn is_valid_accelerator(accel: &str) -> bool {
    if accel.is_empty() {
        return false;
    }
    // Basic validation: must end with a key name (not a modifier tag)
    let trimmed = accel.trim();
    !trimmed.is_empty() && !trimmed.ends_with('>')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_keybindings_are_non_empty() {
        let defs = default_keybindings();
        assert!(!defs.is_empty());
    }

    #[test]
    fn all_defaults_have_valid_accelerators() {
        for def in default_keybindings() {
            for accel in def.default_accel_list() {
                assert!(
                    is_valid_accelerator(accel),
                    "Invalid accelerator '{}' for action '{}'",
                    accel,
                    def.action
                );
            }
        }
    }

    #[test]
    fn all_actions_are_unique() {
        let defs = default_keybindings();
        let mut seen = std::collections::HashSet::new();
        for def in &defs {
            assert!(seen.insert(&def.action), "Duplicate action: {}", def.action);
        }
    }

    #[test]
    fn settings_get_accel_returns_override() {
        let mut settings = KeybindingSettings::default();
        let def = KeybindingDef::new(
            "win.copy",
            "<Control><Shift>c",
            "Copy",
            KeybindingCategory::Terminal,
        );
        assert_eq!(settings.get_accel(&def), "<Control><Shift>c");

        settings
            .overrides
            .insert("win.copy".into(), "<Control>c".into());
        assert_eq!(settings.get_accel(&def), "<Control>c");
    }

    #[test]
    fn settings_reset_removes_override() {
        let mut settings = KeybindingSettings::default();
        settings
            .overrides
            .insert("win.copy".into(), "<Control>c".into());
        assert!(settings.has_overrides());
        settings.reset("win.copy");
        assert!(!settings.has_overrides());
    }

    #[test]
    fn settings_reset_all_clears_everything() {
        let mut settings = KeybindingSettings::default();
        settings
            .overrides
            .insert("win.copy".into(), "<Control>c".into());
        settings
            .overrides
            .insert("win.paste".into(), "<Control>v".into());
        settings.reset_all();
        assert!(!settings.has_overrides());
    }

    #[test]
    fn all_categories_have_at_least_one_binding() {
        let defs = default_keybindings();
        for cat in KeybindingCategory::all() {
            assert!(
                defs.iter().any(|d| d.category == *cat),
                "Category {:?} has no bindings",
                cat
            );
        }
    }

    #[test]
    fn valid_accelerator_checks() {
        assert!(is_valid_accelerator("<Control>q"));
        assert!(is_valid_accelerator("F11"));
        assert!(is_valid_accelerator("<Control><Shift>c"));
        assert!(!is_valid_accelerator(""));
        assert!(!is_valid_accelerator("<Control>"));
    }
}
