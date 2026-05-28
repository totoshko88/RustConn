//! Terminal color themes
//!
//! This module defines color themes for VTE terminals.
//! Built-in themes are always available; user-created custom themes
//! are persisted to `~/.config/rustconn/custom_themes.json`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

/// RGB color representation
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Color {
    /// Red component (0.0-1.0)
    pub r: f32,
    /// Green component (0.0-1.0)
    pub g: f32,
    /// Blue component (0.0-1.0)
    pub b: f32,
}

impl Color {
    /// Creates a new color from RGB values (0.0-1.0)
    #[must_use]
    pub const fn new(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b }
    }

    /// Creates a color from hex string (e.g., "#FF0000")
    #[must_use]
    pub fn from_hex(hex: &str) -> Self {
        let hex = hex.trim_start_matches('#');
        if hex.len() != 6 {
            return Self::new(0.0, 0.0, 0.0);
        }

        let r = f32::from(u8::from_str_radix(&hex[0..2], 16).unwrap_or(0)) / 255.0;
        let g = f32::from(u8::from_str_radix(&hex[2..4], 16).unwrap_or(0)) / 255.0;
        let b = f32::from(u8::from_str_radix(&hex[4..6], 16).unwrap_or(0)) / 255.0;

        Self::new(r, g, b)
    }

    /// Converts this color to a `#RRGGBB` hex string.
    #[must_use]
    pub fn to_hex(&self) -> String {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        let r = (self.r * 255.0).round() as u8;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        let g = (self.g * 255.0).round() as u8;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        let b = (self.b * 255.0).round() as u8;
        format!("#{r:02X}{g:02X}{b:02X}")
    }
}

/// Terminal color theme
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminalTheme {
    /// Theme name
    pub name: String,
    /// Background color
    pub background: Color,
    /// Foreground (text) color
    pub foreground: Color,
    /// Cursor color
    pub cursor: Color,
    /// 16-color ANSI palette
    pub palette: [Color; 16],
    /// Whether this is a user-created custom theme (not built-in)
    #[serde(default)]
    pub is_custom: bool,
}

/// Global store for custom themes (loaded once, mutated via add/remove).
static CUSTOM_THEMES: Mutex<Option<Vec<TerminalTheme>>> = Mutex::new(None);

/// Returns the path to the custom themes JSON file.
fn custom_themes_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("rustconn").join("custom_themes.json"))
}

/// Loads custom themes from disk. Returns empty vec on any error.
fn load_custom_themes_from_disk() -> Vec<TerminalTheme> {
    let Some(path) = custom_themes_path() else {
        return Vec::new();
    };
    if !path.exists() {
        return Vec::new();
    }
    match std::fs::read_to_string(&path) {
        Ok(data) => serde_json::from_str::<Vec<TerminalTheme>>(&data).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Persists custom themes to disk using atomic write (temp file + rename).
fn save_custom_themes_to_disk(themes: &[TerminalTheme]) {
    let Some(path) = custom_themes_path() else {
        return;
    };
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::warn!(error = %e, "Failed to create custom themes directory");
        return;
    }
    let json = match serde_json::to_string_pretty(themes) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!(error = %e, "Failed to serialize custom themes");
            return;
        }
    };

    // Atomic write: temp file + rename (consistent with sync file writes)
    let temp_path = path.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&temp_path, &json) {
        tracing::warn!(error = %e, "Failed to write custom themes temp file");
        return;
    }
    if let Err(e) = std::fs::rename(&temp_path, &path) {
        tracing::warn!(error = %e, "Failed to rename custom themes temp file");
        let _ = std::fs::remove_file(&temp_path);
        return;
    }

    // Restrict file permissions to owner-only (0600) for consistency
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
}

/// Returns the cached custom themes, loading from disk on first access.
fn get_custom_themes() -> Vec<TerminalTheme> {
    let mut guard = CUSTOM_THEMES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if guard.is_none() {
        *guard = Some(load_custom_themes_from_disk());
    }
    guard.as_ref().cloned().unwrap_or_default()
}

impl TerminalTheme {
    /// Returns built-in themes only.
    #[must_use]
    pub fn builtin_themes() -> Vec<Self> {
        vec![
            Self::dark_theme(),
            Self::light_theme(),
            Self::solarized_dark_theme(),
            Self::solarized_light_theme(),
            Self::monokai_theme(),
            Self::dracula_theme(),
        ]
    }

    /// Gets all available themes (built-in + custom).
    #[must_use]
    pub fn all_themes() -> Vec<Self> {
        let mut themes = Self::builtin_themes();
        themes.extend(get_custom_themes());
        themes
    }

    /// Gets theme by name (searches built-in first, then custom).
    #[must_use]
    pub fn by_name(name: &str) -> Option<Self> {
        Self::all_themes().into_iter().find(|t| t.name == name)
    }

    /// Gets all theme names (built-in + custom).
    #[must_use]
    pub fn theme_names() -> Vec<String> {
        Self::all_themes().into_iter().map(|t| t.name).collect()
    }

    /// Returns only custom theme names.
    #[must_use]
    pub fn custom_theme_names() -> Vec<String> {
        get_custom_themes().into_iter().map(|t| t.name).collect()
    }

    /// Checks whether a theme name belongs to a built-in theme.
    #[must_use]
    pub fn is_builtin(name: &str) -> bool {
        Self::builtin_themes().iter().any(|t| t.name == name)
    }

    /// Adds or updates a custom theme and persists to disk.
    #[expect(
        clippy::missing_panics_doc,
        clippy::significant_drop_tightening,
        reason = "Mutex guard is intentionally held across the operation; panic only on poisoned lock"
    )]
    pub fn save_custom_theme(theme: Self) {
        let mut guard = CUSTOM_THEMES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.is_none() {
            *guard = Some(load_custom_themes_from_disk());
        }
        let themes = guard.as_mut().expect("just initialized");
        if let Some(existing) = themes.iter_mut().find(|t| t.name == theme.name) {
            *existing = theme;
        } else {
            themes.push(theme);
        }
        save_custom_themes_to_disk(themes);
    }

    /// Removes a custom theme by name and persists to disk.
    ///
    /// Returns `true` if the theme was found and removed.
    #[expect(
        clippy::missing_panics_doc,
        clippy::significant_drop_tightening,
        reason = "Mutex guard is intentionally held across the operation; panic only on poisoned lock"
    )]
    pub fn remove_custom_theme(name: &str) -> bool {
        let mut guard = CUSTOM_THEMES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.is_none() {
            *guard = Some(load_custom_themes_from_disk());
        }
        let themes = guard.as_mut().expect("just initialized");
        let before = themes.len();
        themes.retain(|t| t.name != name);
        let removed = themes.len() < before;
        if removed {
            save_custom_themes_to_disk(themes);
        }
        removed
    }

    /// Creates a new custom theme with default dark colors and the given name.
    #[must_use]
    pub fn new_custom(name: &str) -> Self {
        let mut theme = Self::dark_theme();
        theme.name = name.to_string();
        theme.is_custom = true;
        theme
    }

    /// Dark theme (default)
    #[must_use]
    pub fn dark_theme() -> Self {
        Self {
            name: "Dark".to_string(),
            background: Color::new(0.1, 0.1, 0.1),
            foreground: Color::new(0.9, 0.9, 0.9),
            cursor: Color::new(0.9, 0.9, 0.9),
            palette: [
                Color::new(0.0, 0.0, 0.0),
                Color::new(0.8, 0.0, 0.0),
                Color::new(0.0, 0.8, 0.0),
                Color::new(0.8, 0.8, 0.0),
                Color::new(0.0, 0.0, 0.8),
                Color::new(0.8, 0.0, 0.8),
                Color::new(0.0, 0.8, 0.8),
                Color::new(0.8, 0.8, 0.8),
                Color::new(0.4, 0.4, 0.4),
                Color::new(1.0, 0.0, 0.0),
                Color::new(0.0, 1.0, 0.0),
                Color::new(1.0, 1.0, 0.0),
                Color::new(0.0, 0.0, 1.0),
                Color::new(1.0, 0.0, 1.0),
                Color::new(0.0, 1.0, 1.0),
                Color::new(1.0, 1.0, 1.0),
            ],
            is_custom: false,
        }
    }

    /// Light theme
    #[must_use]
    pub fn light_theme() -> Self {
        Self {
            name: "Light".to_string(),
            background: Color::new(0.98, 0.98, 0.98),
            foreground: Color::new(0.2, 0.2, 0.2),
            cursor: Color::new(0.2, 0.2, 0.2),
            palette: [
                Color::new(0.0, 0.0, 0.0),
                Color::new(0.8, 0.0, 0.0),
                Color::new(0.0, 0.6, 0.0),
                Color::new(0.8, 0.6, 0.0),
                Color::new(0.0, 0.0, 0.8),
                Color::new(0.8, 0.0, 0.8),
                Color::new(0.0, 0.6, 0.6),
                Color::new(0.6, 0.6, 0.6),
                Color::new(0.4, 0.4, 0.4),
                Color::new(1.0, 0.2, 0.2),
                Color::new(0.2, 0.8, 0.2),
                Color::new(1.0, 0.8, 0.2),
                Color::new(0.2, 0.2, 1.0),
                Color::new(1.0, 0.2, 1.0),
                Color::new(0.2, 0.8, 0.8),
                Color::new(0.8, 0.8, 0.8),
            ],
            is_custom: false,
        }
    }

    /// Solarized Dark theme
    #[must_use]
    pub fn solarized_dark_theme() -> Self {
        Self {
            name: "Solarized Dark".to_string(),
            background: Color::from_hex("#002b36"),
            foreground: Color::from_hex("#839496"),
            cursor: Color::from_hex("#839496"),
            palette: [
                Color::from_hex("#073642"),
                Color::from_hex("#dc322f"),
                Color::from_hex("#859900"),
                Color::from_hex("#b58900"),
                Color::from_hex("#268bd2"),
                Color::from_hex("#d33682"),
                Color::from_hex("#2aa198"),
                Color::from_hex("#eee8d5"),
                Color::from_hex("#002b36"),
                Color::from_hex("#cb4b16"),
                Color::from_hex("#586e75"),
                Color::from_hex("#657b83"),
                Color::from_hex("#839496"),
                Color::from_hex("#6c71c4"),
                Color::from_hex("#93a1a1"),
                Color::from_hex("#fdf6e3"),
            ],
            is_custom: false,
        }
    }

    /// Solarized Light theme
    #[must_use]
    pub fn solarized_light_theme() -> Self {
        Self {
            name: "Solarized Light".to_string(),
            background: Color::from_hex("#fdf6e3"),
            foreground: Color::from_hex("#657b83"),
            cursor: Color::from_hex("#657b83"),
            palette: [
                Color::from_hex("#073642"),
                Color::from_hex("#dc322f"),
                Color::from_hex("#859900"),
                Color::from_hex("#b58900"),
                Color::from_hex("#268bd2"),
                Color::from_hex("#d33682"),
                Color::from_hex("#2aa198"),
                Color::from_hex("#eee8d5"),
                Color::from_hex("#002b36"),
                Color::from_hex("#cb4b16"),
                Color::from_hex("#586e75"),
                Color::from_hex("#657b83"),
                Color::from_hex("#839496"),
                Color::from_hex("#6c71c4"),
                Color::from_hex("#93a1a1"),
                Color::from_hex("#fdf6e3"),
            ],
            is_custom: false,
        }
    }

    /// Monokai theme
    #[must_use]
    pub fn monokai_theme() -> Self {
        Self {
            name: "Monokai".to_string(),
            background: Color::from_hex("#272822"),
            foreground: Color::from_hex("#f8f8f2"),
            cursor: Color::from_hex("#f8f8f2"),
            palette: [
                Color::from_hex("#272822"),
                Color::from_hex("#f92672"),
                Color::from_hex("#a6e22e"),
                Color::from_hex("#f4bf75"),
                Color::from_hex("#66d9ef"),
                Color::from_hex("#ae81ff"),
                Color::from_hex("#a1efe4"),
                Color::from_hex("#f8f8f2"),
                Color::from_hex("#75715e"),
                Color::from_hex("#f92672"),
                Color::from_hex("#a6e22e"),
                Color::from_hex("#f4bf75"),
                Color::from_hex("#66d9ef"),
                Color::from_hex("#ae81ff"),
                Color::from_hex("#a1efe4"),
                Color::from_hex("#f9f8f5"),
            ],
            is_custom: false,
        }
    }

    /// Dracula theme
    #[must_use]
    pub fn dracula_theme() -> Self {
        Self {
            name: "Dracula".to_string(),
            background: Color::from_hex("#282a36"),
            foreground: Color::from_hex("#f8f8f2"),
            cursor: Color::from_hex("#f8f8f2"),
            palette: [
                Color::from_hex("#000000"),
                Color::from_hex("#ff5555"),
                Color::from_hex("#50fa7b"),
                Color::from_hex("#f1fa8c"),
                Color::from_hex("#bd93f9"),
                Color::from_hex("#ff79c6"),
                Color::from_hex("#8be9fd"),
                Color::from_hex("#bfbfbf"),
                Color::from_hex("#4d4d4d"),
                Color::from_hex("#ff6e67"),
                Color::from_hex("#5af78e"),
                Color::from_hex("#f4f99d"),
                Color::from_hex("#caa9fa"),
                Color::from_hex("#ff92d0"),
                Color::from_hex("#9aedfe"),
                Color::from_hex("#e6e6e6"),
            ],
            is_custom: false,
        }
    }
}
