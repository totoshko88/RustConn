//! Terminal color themes
//!
//! This module defines color themes for VTE terminals.
//! Built-in themes are always available; user-created custom themes
//! are persisted to `~/.config/rustconn/custom_themes.json`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Parses the digits of a hex colour, `RRGGBB` or `RRGGBBAA` without the `#`,
/// into its red, green, blue and alpha bytes.
///
/// Alpha is `0xFF` when there are six digits. Every byte is checked to be an
/// ASCII hex digit before anything is sliced: `len()` counts bytes, so six bytes
/// can hold a multi-byte character — `0а0ff` with a Cyrillic `а` — and slicing
/// at byte 2 would split it and panic. That panic took the application down on
/// every terminal start in issue #343. Every colour parser goes through this one
/// so that no copy of the old code can keep the bug; the check also refuses the
/// leading `+` that `u8::from_str_radix` would accept.
#[must_use]
pub fn parse_hex_channels(digits: &str) -> Option<[u8; 4]> {
    if !matches!(digits.len(), 6 | 8) || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    let alpha = if digits.len() == 8 {
        channel(6)?
    } else {
        u8::MAX
    };
    Some([channel(0)?, channel(2)?, channel(4)?, alpha])
}

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
    ///
    /// Anything other than six hex digits, with or without the `#`, is black.
    #[must_use]
    pub fn from_hex(hex: &str) -> Self {
        let digits = hex.trim_start_matches('#');
        match parse_hex_channels(digits) {
            Some([r, g, b, _]) if digits.len() == 6 => Self::new(
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
            ),
            _ => Self::new(0.0, 0.0, 0.0),
        }
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

/// `color_theme` value meaning "match the desktop's light/dark preference".
///
/// Not a theme in its own right — it carries no colours and is never returned by
/// [`TerminalTheme::all_themes`] or [`TerminalTheme::by_name`]. It appears in
/// [`TerminalTheme::theme_names`] so the picker can offer it, and
/// [`TerminalTheme::resolve`] turns it into [`TerminalTheme::light_theme`] or
/// [`TerminalTheme::dark_theme`].
///
/// The string is stored verbatim in `settings.toml`, so it is part of the on-disk
/// format: changing it would silently reset every user who selected it.
pub const FOLLOW_SYSTEM_THEME: &str = "Follow System";

/// Global store for custom themes (loaded once, mutated via add/remove).
static CUSTOM_THEMES: Mutex<Option<Vec<TerminalTheme>>> = Mutex::new(None);

/// Where an unreadable `custom_themes.json` was copied aside, until it is reported.
///
/// Set by [`load_custom_themes_from_disk`], taken by
/// [`TerminalTheme::preload_custom_themes`].
static UNREADABLE_CUSTOM_THEMES: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Returns the path to the custom themes JSON file.
fn custom_themes_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("rustconn").join("custom_themes.json"))
}

/// Loads custom themes from disk. Returns empty vec on any error.
///
/// A file that is there but does not parse is copied aside first: the empty
/// list returned instead is what the next theme save writes over it.
fn load_custom_themes_from_disk() -> Vec<TerminalTheme> {
    let Some(path) = custom_themes_path() else {
        return Vec::new();
    };
    if !path.exists() {
        return Vec::new();
    }
    match std::fs::read_to_string(&path) {
        Ok(data) => match serde_json::from_str::<Vec<TerminalTheme>>(&data) {
            Ok(themes) => themes,
            Err(e) => {
                keep_unreadable_custom_themes(&path, &e);
                Vec::new()
            }
        },
        Err(_) => Vec::new(),
    }
}

/// Copies an unreadable `custom_themes.json` aside and records where, for startup to report.
fn keep_unreadable_custom_themes(path: &Path, error: &serde_json::Error) {
    // serde_json can quote the offending value, so the message stays at debug.
    tracing::debug!(error = %error, "custom_themes.json parse error");
    match crate::config::quarantine_file(path) {
        Ok(kept) => {
            tracing::warn!(
                file = %path.display(),
                kept_as = %kept.display(),
                "Custom themes could not be read; kept a copy and using none"
            );
            let mut slot = UNREADABLE_CUSTOM_THEMES
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *slot = Some(kept);
        }
        Err(e) => {
            tracing::warn!(
                file = %path.display(),
                error = %e,
                "Custom themes could not be read or kept aside; using none"
            );
        }
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

    if let Err(e) =
        crate::fs_atomic::atomic_write(&path, json.as_bytes(), crate::fs_atomic::MODE_PRIVATE)
    {
        tracing::warn!(error = %e, "Failed to write custom themes");
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

    /// Resolves a stored `color_theme` value to concrete colours.
    ///
    /// `system_dark` is the desktop's resolved dark preference. This crate is
    /// headless, so the caller supplies it as a plain bool — in the GUI that is
    /// `AdwStyleManager::is_dark()`. It is only consulted for
    /// [`FOLLOW_SYSTEM_THEME`]; a named theme resolves the same way regardless.
    ///
    /// An unknown name falls back to [`Self::dark_theme`], matching what every
    /// call site did before this function existed. Deliberately *not* changed to
    /// follow the system too: that would silently repaint terminals belonging to a
    /// custom theme the user deleted, which is a separate decision from this one.
    #[must_use]
    pub fn resolve(name: &str, system_dark: bool) -> Self {
        if name == FOLLOW_SYSTEM_THEME {
            return if system_dark {
                Self::dark_theme()
            } else {
                Self::light_theme()
            };
        }
        Self::by_name(name).unwrap_or_else(Self::dark_theme)
    }

    /// Gets all selectable theme names, follow-system first, then built-in and custom.
    ///
    /// [`FOLLOW_SYSTEM_THEME`] leads the list because it is the recommended
    /// default; every caller that maps a picker index back to a name goes through
    /// this same function, so the extra entry shifts nothing.
    #[must_use]
    pub fn theme_names() -> Vec<String> {
        let mut names = vec![FOLLOW_SYSTEM_THEME.to_string()];
        names.extend(Self::all_themes().into_iter().map(|t| t.name));
        names
    }

    /// Returns only custom theme names.
    #[must_use]
    pub fn custom_theme_names() -> Vec<String> {
        get_custom_themes().into_iter().map(|t| t.name).collect()
    }

    /// Loads the custom themes now rather than on first use, and reports an unreadable file.
    ///
    /// Returns where an unreadable `custom_themes.json` was copied aside, once:
    /// `None` if the file was readable or missing, if keeping the copy failed
    /// (that is logged), or if it was already reported. Startup calls this so
    /// the file is reported with the config files rather than whenever a
    /// terminal first needs a theme.
    #[must_use]
    pub fn preload_custom_themes() -> Option<PathBuf> {
        {
            let mut themes = CUSTOM_THEMES
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if themes.is_none() {
                *themes = Some(load_custom_themes_from_disk());
            }
        }
        UNREADABLE_CUSTOM_THEMES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    /// Checks whether a theme name is built-in, and so not editable or removable.
    ///
    /// [`FOLLOW_SYSTEM_THEME`] counts as built-in. It is not in
    /// [`Self::builtin_themes`] because it has no colours, but callers use this
    /// predicate to decide whether Edit and Delete apply — and they must not.
    #[must_use]
    pub fn is_builtin(name: &str) -> bool {
        name == FOLLOW_SYSTEM_THEME || Self::builtin_themes().iter().any(|t| t.name == name)
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

#[cfg(test)]
mod tests {
    use super::{Color, FOLLOW_SYSTEM_THEME, TerminalTheme, parse_hex_channels};

    #[test]
    fn hex_channels_parse_six_and_eight_digits() {
        assert_eq!(parse_hex_channels("FF8000"), Some([0xFF, 0x80, 0x00, 0xFF]));
        assert_eq!(parse_hex_channels("aaBBcc"), Some([0xAA, 0xBB, 0xCC, 0xFF]));
        assert_eq!(
            parse_hex_channels("10203040"),
            Some([0x10, 0x20, 0x30, 0x40])
        );
    }

    #[test]
    fn hex_channels_refuse_wrong_lengths_and_non_hex() {
        for digits in [
            "",
            "FFF",
            "FFFFF",
            "FFFFFFF",
            "FFFFFFFFF",
            "GGGGGG",
            "#FFFFFF",
        ] {
            assert_eq!(parse_hex_channels(digits), None, "{digits:?}");
        }
    }

    /// Six bytes are not six hex digits: a Cyrillic `а` is two bytes, so a
    /// length check alone let `0а0ff` through to a slice that split the
    /// character and panicked (issue #343). `from_str_radix` also takes a `+`.
    #[test]
    fn hex_channels_refuse_multibyte_characters_and_signs_without_panicking() {
        for digits in ["0а0ff", "ффф", "€000", "0а0ff0а0", "+f+f+f", "+f+f+f+f"] {
            assert_eq!(parse_hex_channels(digits), None, "{digits:?}");
        }
    }

    /// The theme constructor shares the parser, so a stored or imported value
    /// holding a multi-byte character is black instead of a crash.
    #[test]
    fn color_from_hex_refuses_multibyte_input_without_panicking() {
        assert_eq!(Color::from_hex("#0а0ff"), Color::new(0.0, 0.0, 0.0));
        assert_eq!(Color::from_hex("#10203040"), Color::new(0.0, 0.0, 0.0));
        assert_eq!(Color::from_hex("#FF0000"), Color::new(1.0, 0.0, 0.0));
    }

    #[test]
    fn follow_system_resolves_to_dark_when_the_desktop_is_dark() {
        let resolved = TerminalTheme::resolve(FOLLOW_SYSTEM_THEME, true);
        assert_eq!(resolved.name, TerminalTheme::dark_theme().name);
        assert_eq!(resolved.background, TerminalTheme::dark_theme().background);
    }

    #[test]
    fn follow_system_resolves_to_light_when_the_desktop_is_light() {
        let resolved = TerminalTheme::resolve(FOLLOW_SYSTEM_THEME, false);
        assert_eq!(resolved.name, TerminalTheme::light_theme().name);
        assert_eq!(resolved.background, TerminalTheme::light_theme().background);
    }

    #[test]
    fn a_named_theme_ignores_the_desktop_preference() {
        // The whole point of picking a theme by name is that it stays put.
        let dark_desktop = TerminalTheme::resolve("Monokai", true);
        let light_desktop = TerminalTheme::resolve("Monokai", false);
        assert_eq!(dark_desktop.name, "Monokai");
        assert_eq!(dark_desktop, light_desktop);
    }

    #[test]
    fn an_unknown_name_falls_back_to_dark_regardless_of_the_desktop() {
        // Pinned deliberately: a deleted custom theme must not start tracking the
        // system, because that is a different feature from asking for it.
        for system_dark in [true, false] {
            let resolved = TerminalTheme::resolve("no such theme", system_dark);
            assert_eq!(resolved.name, TerminalTheme::dark_theme().name);
        }
    }

    #[test]
    fn theme_names_offers_follow_system_first() {
        let names = TerminalTheme::theme_names();
        assert_eq!(names.first().map(String::as_str), Some(FOLLOW_SYSTEM_THEME));
        assert!(names.iter().any(|n| n == "Dark"));
        assert!(names.iter().any(|n| n == "Light"));
    }

    #[test]
    fn follow_system_is_not_editable_or_removable() {
        // The picker gates Edit/Delete on `is_builtin`, and the sentinel has no
        // colours to edit and no file to remove.
        assert!(TerminalTheme::is_builtin(FOLLOW_SYSTEM_THEME));
    }

    #[test]
    fn follow_system_is_absent_from_the_real_theme_lists() {
        // It is a marker, not a theme: anything iterating actual colours — the
        // per-connection override picker, the custom-theme editor — must not see it.
        assert!(
            !TerminalTheme::all_themes()
                .iter()
                .any(|t| t.name == FOLLOW_SYSTEM_THEME)
        );
        assert!(
            !TerminalTheme::builtin_themes()
                .iter()
                .any(|t| t.name == FOLLOW_SYSTEM_THEME)
        );
    }
}
