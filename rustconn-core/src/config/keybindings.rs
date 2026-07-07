//! Custom keybinding configuration
//!
//! Provides [`KeybindingSettings`] for user-customizable keyboard shortcuts
//! and [`KeybindingDef`] for the default keybinding registry.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

/// Custom keybinding overrides stored in user settings.
///
/// Each entry maps a GTK action name (e.g. `"win.copy"`) to a GTK accelerator
/// string (e.g. `"<Control><Shift>c"`). Actions not present in `overrides`
/// use their built-in defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeybindingSettings {
    /// Action name → accelerator string mapping.
    ///
    /// Only overridden bindings are stored; defaults are implicit.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub overrides: HashMap<String, String>,

    /// Actions that remain active even in passthrough mode.
    ///
    /// When passthrough is enabled, all keybindings are disabled except those
    /// listed here. Defaults to the passthrough toggle itself, quit, and fullscreen.
    #[serde(default = "default_passthrough_exceptions")]
    #[serde(skip_serializing_if = "is_default_passthrough_exceptions")]
    pub passthrough_exceptions: Vec<String>,
}

impl Default for KeybindingSettings {
    fn default() -> Self {
        Self {
            overrides: HashMap::new(),
            passthrough_exceptions: default_passthrough_exceptions(),
        }
    }
}

/// Default actions that remain active in passthrough mode.
#[must_use]
pub fn default_passthrough_exceptions() -> Vec<String> {
    vec![
        "win.toggle-passthrough".into(),
        "app.quit".into(),
        "win.toggle-fullscreen".into(),
    ]
}

/// Returns `true` if the list matches the default passthrough exceptions (for serde skip).
fn is_default_passthrough_exceptions(exceptions: &[String]) -> bool {
    *exceptions == default_passthrough_exceptions()
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
#[expect(
    clippy::too_many_lines,
    reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
)]
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
            "win.new-connection-advanced",
            "<Control><Shift>n",
            "New Connection (Advanced)",
            Connections,
        ),
        KeybindingDef::new(
            "win.new-group",
            "<Control><Shift>g",
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
        KeybindingDef::new("win.search", "<Control>f", "Search", Navigation),
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
        KeybindingDef::new(
            "win.close-tab",
            "<Control>w|<Control><Shift>w",
            "Close Tab",
            Terminal,
        ),
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
        KeybindingDef::new(
            "win.tab-overview",
            "<Control><Shift>o",
            "Tab Overview",
            Terminal,
        ),
        KeybindingDef::new(
            "win.switch-tab-palette",
            "<Control>percent",
            "Switch Tab",
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
            "<Control><Shift>x",
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
        KeybindingDef::new("win.toggle-sidebar", "F9", "Toggle Sidebar", View),
        KeybindingDef::new(
            "win.toggle-compact",
            "<Control><Shift>d",
            "Toggle Compact Interface",
            View,
        ),
        KeybindingDef::new(
            "win.toggle-passthrough",
            "<Control><Shift>BackSpace",
            "Toggle Keyboard Passthrough",
            View,
        ),
        KeybindingDef::new(
            "win.toggle-broadcast",
            "<Control><Shift>b",
            "Toggle Split Broadcast",
            View,
        ),
        // Application (additional)
        KeybindingDef::new(
            "win.show-history",
            "<Control>h",
            "Connection History",
            Application,
        ),
        KeybindingDef::new(
            "win.show-statistics",
            "<Control><Shift>i",
            "Statistics",
            Application,
        ),
        KeybindingDef::new(
            "win.password-generator",
            "<Control>g",
            "Password Generator",
            Application,
        ),
        KeybindingDef::new(
            "win.wake-on-lan",
            "<Control><Shift>l",
            "Wake On LAN",
            Application,
        ),
        KeybindingDef::new(
            "win.ssh-tunnels",
            "<Control>t",
            "SSH Tunnel Manager",
            Application,
        ),
    ]
}

/// Returns `true` if two GTK accelerator strings denote the same shortcut.
///
/// Comparison ignores the order of modifier tokens, so `<Control><Shift>w`
/// and `<Shift><Control>w` are treated as equal. The `<Primary>` and `<Ctrl>`
/// aliases are normalised to `<Control>`. Key names are compared
/// case-insensitively.
///
/// This is a pure, GTK-independent equivalent of comparing the output of
/// `gtk4::accelerator_parse`, kept in `rustconn-core` so it can be unit-tested
/// without a display.
#[must_use]
pub fn accelerators_equivalent(a: &str, b: &str) -> bool {
    match (parse_accelerator(a), parse_accelerator(b)) {
        (Some(parsed_a), Some(parsed_b)) => parsed_a == parsed_b,
        // Fall back to exact comparison when either side cannot be parsed.
        _ => a == b,
    }
}

/// Parses an accelerator into its set of modifiers and its key name.
///
/// Returns `None` when the string is empty, has an unterminated `<...>` token,
/// or has no trailing key after the modifiers.
fn parse_accelerator(accel: &str) -> Option<(BTreeSet<String>, String)> {
    let trimmed = accel.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut modifiers = BTreeSet::new();
    let mut rest = trimmed;
    while let Some(after_open) = rest.strip_prefix('<') {
        let close = after_open.find('>')?;
        let token = after_open[..close].trim().to_ascii_lowercase();
        if token.is_empty() {
            return None;
        }
        // Normalise the only unambiguous GTK aliases; leave others as-is so we
        // never merge genuinely distinct modifiers (e.g. Super vs Meta).
        let canonical = match token.as_str() {
            "primary" | "ctrl" => "control",
            other => other,
        };
        modifiers.insert(canonical.to_owned());
        rest = &after_open[close + 1..];
    }

    let key = rest.trim();
    if key.is_empty() {
        return None;
    }
    Some((modifiers, key.to_ascii_lowercase()))
}

/// Validates a GTK accelerator string.
///
/// Returns `true` if the string looks like a valid GTK accelerator
/// (contains at least one key name with at least one modifier, or is
/// a function/special key). Bare character keys without modifiers are
/// rejected to avoid conflicts with text input.
#[must_use]
pub fn is_valid_accelerator(accel: &str) -> bool {
    if accel.is_empty() {
        return false;
    }
    let trimmed = accel.trim();
    if trimmed.is_empty() || trimmed.ends_with('>') {
        return false;
    }
    // Allow function keys (F1–F12), Escape, Delete, etc. without modifiers
    let has_modifier = trimmed.contains('<');
    if has_modifier {
        return true;
    }
    // Without modifiers, only allow special keys (multi-char names)
    // Single characters like "a", "x" conflict with text input
    let key_name = trimmed;
    key_name.len() > 1
        && key_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
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
        // With modifiers — always valid
        assert!(is_valid_accelerator("<Control>q"));
        assert!(is_valid_accelerator("<Control><Shift>c"));
        assert!(is_valid_accelerator("<Alt>F4"));
        assert!(is_valid_accelerator("<Super>e"));

        // Special/function keys without modifiers — valid (multi-char)
        assert!(is_valid_accelerator("F11"));
        assert!(is_valid_accelerator("F1"));
        assert!(is_valid_accelerator("Escape"));
        assert!(is_valid_accelerator("Delete"));
        assert!(is_valid_accelerator("Home"));
        assert!(is_valid_accelerator("Return"));

        // Invalid: empty or incomplete
        assert!(!is_valid_accelerator(""));
        assert!(!is_valid_accelerator("<Control>"));

        // Invalid: bare single characters conflict with text input
        assert!(!is_valid_accelerator("a"));
        assert!(!is_valid_accelerator("x"));
        assert!(!is_valid_accelerator("1"));
    }

    #[test]
    fn accels_equivalent_ignores_modifier_order() {
        assert!(accelerators_equivalent(
            "<Control><Shift>w",
            "<Shift><Control>w"
        ));
        assert!(accelerators_equivalent(
            "<Control><Shift><Alt>p",
            "<Alt><Shift><Control>p"
        ));
    }

    #[test]
    fn accels_equivalent_identical_strings() {
        assert!(accelerators_equivalent("<Control>q", "<Control>q"));
        assert!(accelerators_equivalent("F11", "F11"));
    }

    #[test]
    fn accels_equivalent_normalises_primary_and_ctrl_aliases() {
        assert!(accelerators_equivalent("<Primary>c", "<Control>c"));
        assert!(accelerators_equivalent("<Ctrl>c", "<Control>c"));
        assert!(accelerators_equivalent(
            "<Primary><Shift>c",
            "<Shift><Control>c"
        ));
    }

    #[test]
    fn accels_equivalent_key_case_insensitive() {
        assert!(accelerators_equivalent("<Control>W", "<Control>w"));
    }

    #[test]
    fn accels_not_equivalent_different_key() {
        assert!(!accelerators_equivalent("<Control>w", "<Control>q"));
    }

    #[test]
    fn accels_not_equivalent_different_modifiers() {
        assert!(!accelerators_equivalent("<Control>w", "<Control><Shift>w"));
        assert!(!accelerators_equivalent("<Control>w", "<Alt>w"));
        // Super and Meta must not be merged.
        assert!(!accelerators_equivalent("<Super>e", "<Meta>e"));
    }

    #[test]
    fn accels_equivalent_unparseable_falls_back_to_exact() {
        // Unterminated token cannot be parsed; equal strings still match.
        assert!(accelerators_equivalent("<Control", "<Control"));
        assert!(!accelerators_equivalent("<Control", "<Shift"));
    }

    #[test]
    fn all_default_accels_are_self_equivalent() {
        // Every default accelerator must compare equal to itself under the
        // order-insensitive comparison (guards the conflict-detection path).
        for def in default_keybindings() {
            for accel in def.default_accel_list() {
                assert!(
                    accelerators_equivalent(accel, accel),
                    "accel '{}' not self-equivalent",
                    accel
                );
            }
        }
    }
}
