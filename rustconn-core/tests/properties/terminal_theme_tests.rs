//! Property tests for terminal themes

use proptest::prelude::*;
use rustconn_core::terminal_themes::{Color, TerminalTheme};

// ============================================================================
// Color Tests
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn color_new_preserves_values(r in 0.0f32..=1.0, g in 0.0f32..=1.0, b in 0.0f32..=1.0) {
        let color = Color::new(r, g, b);
        prop_assert!((color.r - r).abs() < f32::EPSILON);
        prop_assert!((color.g - g).abs() < f32::EPSILON);
        prop_assert!((color.b - b).abs() < f32::EPSILON);
    }

    #[test]
    fn color_from_hex_valid_produces_valid_range(
        r in 0u8..=255,
        g in 0u8..=255,
        b in 0u8..=255
    ) {
        let hex = format!("#{r:02X}{g:02X}{b:02X}");
        let color = Color::from_hex(&hex);

        prop_assert!(color.r >= 0.0 && color.r <= 1.0);
        prop_assert!(color.g >= 0.0 && color.g <= 1.0);
        prop_assert!(color.b >= 0.0 && color.b <= 1.0);
    }

    #[test]
    fn color_from_hex_roundtrip(r in 0u8..=255, g in 0u8..=255, b in 0u8..=255) {
        let hex = format!("#{r:02X}{g:02X}{b:02X}");
        let color = Color::from_hex(&hex);

        // Convert back to 0-255 range
        let r_back = (color.r * 255.0).round() as u8;
        let g_back = (color.g * 255.0).round() as u8;
        let b_back = (color.b * 255.0).round() as u8;

        prop_assert_eq!(r, r_back);
        prop_assert_eq!(g, g_back);
        prop_assert_eq!(b, b_back);
    }
}

// ============================================================================
// Color Edge Case Tests
// ============================================================================

#[test]
fn color_from_hex_without_hash() {
    let color = Color::from_hex("FF0000");
    assert!((color.r - 1.0).abs() < 0.01);
    assert!(color.g.abs() < 0.01);
    assert!(color.b.abs() < 0.01);
}

#[test]
fn color_from_hex_with_hash() {
    let color = Color::from_hex("#00FF00");
    assert!(color.r.abs() < 0.01);
    assert!((color.g - 1.0).abs() < 0.01);
    assert!(color.b.abs() < 0.01);
}

#[test]
fn color_from_hex_invalid_length_returns_black() {
    let color = Color::from_hex("#FFF");
    assert!(color.r.abs() < f32::EPSILON);
    assert!(color.g.abs() < f32::EPSILON);
    assert!(color.b.abs() < f32::EPSILON);
}

#[test]
fn color_from_hex_invalid_chars_returns_zero() {
    let color = Color::from_hex("#GGGGGG");
    assert!(color.r.abs() < f32::EPSILON);
    assert!(color.g.abs() < f32::EPSILON);
    assert!(color.b.abs() < f32::EPSILON);
}

#[test]
fn color_from_hex_empty_returns_black() {
    let color = Color::from_hex("");
    assert!(color.r.abs() < f32::EPSILON);
    assert!(color.g.abs() < f32::EPSILON);
    assert!(color.b.abs() < f32::EPSILON);
}

#[test]
fn color_equality() {
    let c1 = Color::new(0.5, 0.5, 0.5);
    let c2 = Color::new(0.5, 0.5, 0.5);
    assert_eq!(c1, c2);
}

#[test]
fn color_clone() {
    let c1 = Color::new(0.3, 0.6, 0.9);
    let c2 = c1.clone();
    assert_eq!(c1, c2);
}

// ============================================================================
// TerminalTheme Tests
// ============================================================================

#[test]
fn all_themes_returns_non_empty() {
    let themes = TerminalTheme::all_themes();
    assert!(!themes.is_empty());
}

#[test]
fn all_themes_have_unique_names() {
    let themes = TerminalTheme::all_themes();
    let names: Vec<_> = themes.iter().map(|t| &t.name).collect();
    let unique_names: std::collections::HashSet<_> = names.iter().collect();
    assert_eq!(names.len(), unique_names.len());
}

#[test]
fn all_themes_have_16_color_palette() {
    for theme in TerminalTheme::all_themes() {
        assert_eq!(
            theme.palette.len(),
            16,
            "Theme {} should have 16 colors",
            theme.name
        );
    }
}

#[test]
fn theme_names_matches_all_themes() {
    let themes = TerminalTheme::all_themes();
    let names = TerminalTheme::theme_names();
    assert_eq!(themes.len(), names.len());

    for theme in &themes {
        assert!(names.contains(&theme.name));
    }
}

#[test]
fn by_name_finds_existing_themes() {
    for name in TerminalTheme::theme_names() {
        let theme = TerminalTheme::by_name(&name);
        assert!(theme.is_some(), "Theme '{}' should be found", name);
        assert_eq!(theme.unwrap().name, name);
    }
}

#[test]
fn by_name_returns_none_for_unknown() {
    assert!(TerminalTheme::by_name("NonExistentTheme").is_none());
    assert!(TerminalTheme::by_name("").is_none());
}

#[test]
fn dark_theme_has_dark_background() {
    let theme = TerminalTheme::dark_theme();
    // Dark theme should have low luminance background
    let luminance =
        0.299 * theme.background.r + 0.587 * theme.background.g + 0.114 * theme.background.b;
    assert!(luminance < 0.5, "Dark theme background should be dark");
}

#[test]
fn light_theme_has_light_background() {
    let theme = TerminalTheme::light_theme();
    // Light theme should have high luminance background
    let luminance =
        0.299 * theme.background.r + 0.587 * theme.background.g + 0.114 * theme.background.b;
    assert!(luminance > 0.5, "Light theme background should be light");
}

#[test]
fn solarized_dark_theme_exists() {
    let theme = TerminalTheme::solarized_dark_theme();
    assert_eq!(theme.name, "Solarized Dark");
}

#[test]
fn solarized_light_theme_exists() {
    let theme = TerminalTheme::solarized_light_theme();
    assert_eq!(theme.name, "Solarized Light");
}

#[test]
fn monokai_theme_exists() {
    let theme = TerminalTheme::monokai_theme();
    assert_eq!(theme.name, "Monokai");
}

#[test]
fn dracula_theme_exists() {
    let theme = TerminalTheme::dracula_theme();
    assert_eq!(theme.name, "Dracula");
}

#[test]
fn all_theme_colors_in_valid_range() {
    for theme in TerminalTheme::all_themes() {
        // Check background
        assert!(theme.background.r >= 0.0 && theme.background.r <= 1.0);
        assert!(theme.background.g >= 0.0 && theme.background.g <= 1.0);
        assert!(theme.background.b >= 0.0 && theme.background.b <= 1.0);

        // Check foreground
        assert!(theme.foreground.r >= 0.0 && theme.foreground.r <= 1.0);
        assert!(theme.foreground.g >= 0.0 && theme.foreground.g <= 1.0);
        assert!(theme.foreground.b >= 0.0 && theme.foreground.b <= 1.0);

        // Check cursor
        assert!(theme.cursor.r >= 0.0 && theme.cursor.r <= 1.0);
        assert!(theme.cursor.g >= 0.0 && theme.cursor.g <= 1.0);
        assert!(theme.cursor.b >= 0.0 && theme.cursor.b <= 1.0);

        // Check palette
        for (i, color) in theme.palette.iter().enumerate() {
            assert!(
                color.r >= 0.0 && color.r <= 1.0,
                "Theme {} palette[{}] red out of range",
                theme.name,
                i
            );
            assert!(
                color.g >= 0.0 && color.g <= 1.0,
                "Theme {} palette[{}] green out of range",
                theme.name,
                i
            );
            assert!(
                color.b >= 0.0 && color.b <= 1.0,
                "Theme {} palette[{}] blue out of range",
                theme.name,
                i
            );
        }
    }
}

#[test]
fn theme_serialization_roundtrip() {
    for theme in TerminalTheme::all_themes() {
        let json = serde_json::to_string(&theme).expect("serialize");
        let restored: TerminalTheme = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(theme.name, restored.name);
        assert_eq!(theme.palette.len(), restored.palette.len());
    }
}

#[test]
fn color_serialization_roundtrip() {
    let color = Color::new(0.25, 0.5, 0.75);
    let json = serde_json::to_string(&color).expect("serialize");
    let restored: Color = serde_json::from_str(&json).expect("deserialize");
    assert!((color.r - restored.r).abs() < f32::EPSILON);
    assert!((color.g - restored.g).abs() < f32::EPSILON);
    assert!((color.b - restored.b).abs() < f32::EPSILON);
}

#[test]
fn theme_clone_is_independent() {
    let theme1 = TerminalTheme::dark_theme();
    let theme2 = theme1.clone();
    assert_eq!(theme1.name, theme2.name);
    assert_eq!(theme1.palette.len(), theme2.palette.len());
}
