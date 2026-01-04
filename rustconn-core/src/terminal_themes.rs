//! Terminal color themes
//!
//! This module defines color themes for VTE terminals.

use serde::{Deserialize, Serialize};

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
}

impl TerminalTheme {
    /// Gets all available themes
    #[must_use]
    pub fn all_themes() -> Vec<Self> {
        vec![
            Self::dark_theme(),
            Self::light_theme(),
            Self::solarized_dark_theme(),
            Self::solarized_light_theme(),
            Self::monokai_theme(),
            Self::dracula_theme(),
        ]
    }

    /// Gets theme by name
    #[must_use]
    pub fn by_name(name: &str) -> Option<Self> {
        Self::all_themes().into_iter().find(|t| t.name == name)
    }

    /// Gets all theme names
    #[must_use]
    pub fn theme_names() -> Vec<String> {
        Self::all_themes().into_iter().map(|t| t.name).collect()
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
                Color::new(0.0, 0.0, 0.0), // Black
                Color::new(0.8, 0.0, 0.0), // Red
                Color::new(0.0, 0.8, 0.0), // Green
                Color::new(0.8, 0.8, 0.0), // Yellow
                Color::new(0.0, 0.0, 0.8), // Blue
                Color::new(0.8, 0.0, 0.8), // Magenta
                Color::new(0.0, 0.8, 0.8), // Cyan
                Color::new(0.8, 0.8, 0.8), // White
                Color::new(0.4, 0.4, 0.4), // Bright Black
                Color::new(1.0, 0.0, 0.0), // Bright Red
                Color::new(0.0, 1.0, 0.0), // Bright Green
                Color::new(1.0, 1.0, 0.0), // Bright Yellow
                Color::new(0.0, 0.0, 1.0), // Bright Blue
                Color::new(1.0, 0.0, 1.0), // Bright Magenta
                Color::new(0.0, 1.0, 1.0), // Bright Cyan
                Color::new(1.0, 1.0, 1.0), // Bright White
            ],
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
                Color::new(0.0, 0.0, 0.0), // Black
                Color::new(0.8, 0.0, 0.0), // Red
                Color::new(0.0, 0.6, 0.0), // Green
                Color::new(0.8, 0.6, 0.0), // Yellow
                Color::new(0.0, 0.0, 0.8), // Blue
                Color::new(0.8, 0.0, 0.8), // Magenta
                Color::new(0.0, 0.6, 0.6), // Cyan
                Color::new(0.6, 0.6, 0.6), // White
                Color::new(0.4, 0.4, 0.4), // Bright Black
                Color::new(1.0, 0.2, 0.2), // Bright Red
                Color::new(0.2, 0.8, 0.2), // Bright Green
                Color::new(1.0, 0.8, 0.2), // Bright Yellow
                Color::new(0.2, 0.2, 1.0), // Bright Blue
                Color::new(1.0, 0.2, 1.0), // Bright Magenta
                Color::new(0.2, 0.8, 0.8), // Bright Cyan
                Color::new(0.8, 0.8, 0.8), // Bright White
            ],
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
                Color::from_hex("#073642"), // Black
                Color::from_hex("#dc322f"), // Red
                Color::from_hex("#859900"), // Green
                Color::from_hex("#b58900"), // Yellow
                Color::from_hex("#268bd2"), // Blue
                Color::from_hex("#d33682"), // Magenta
                Color::from_hex("#2aa198"), // Cyan
                Color::from_hex("#eee8d5"), // White
                Color::from_hex("#002b36"), // Bright Black
                Color::from_hex("#cb4b16"), // Bright Red
                Color::from_hex("#586e75"), // Bright Green
                Color::from_hex("#657b83"), // Bright Yellow
                Color::from_hex("#839496"), // Bright Blue
                Color::from_hex("#6c71c4"), // Bright Magenta
                Color::from_hex("#93a1a1"), // Bright Cyan
                Color::from_hex("#fdf6e3"), // Bright White
            ],
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
                Color::from_hex("#073642"), // Black
                Color::from_hex("#dc322f"), // Red
                Color::from_hex("#859900"), // Green
                Color::from_hex("#b58900"), // Yellow
                Color::from_hex("#268bd2"), // Blue
                Color::from_hex("#d33682"), // Magenta
                Color::from_hex("#2aa198"), // Cyan
                Color::from_hex("#eee8d5"), // White
                Color::from_hex("#002b36"), // Bright Black
                Color::from_hex("#cb4b16"), // Bright Red
                Color::from_hex("#586e75"), // Bright Green
                Color::from_hex("#657b83"), // Bright Yellow
                Color::from_hex("#839496"), // Bright Blue
                Color::from_hex("#6c71c4"), // Bright Magenta
                Color::from_hex("#93a1a1"), // Bright Cyan
                Color::from_hex("#fdf6e3"), // Bright White
            ],
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
                Color::from_hex("#272822"), // Black
                Color::from_hex("#f92672"), // Red
                Color::from_hex("#a6e22e"), // Green
                Color::from_hex("#f4bf75"), // Yellow
                Color::from_hex("#66d9ef"), // Blue
                Color::from_hex("#ae81ff"), // Magenta
                Color::from_hex("#a1efe4"), // Cyan
                Color::from_hex("#f8f8f2"), // White
                Color::from_hex("#75715e"), // Bright Black
                Color::from_hex("#f92672"), // Bright Red
                Color::from_hex("#a6e22e"), // Bright Green
                Color::from_hex("#f4bf75"), // Bright Yellow
                Color::from_hex("#66d9ef"), // Bright Blue
                Color::from_hex("#ae81ff"), // Bright Magenta
                Color::from_hex("#a1efe4"), // Bright Cyan
                Color::from_hex("#f9f8f5"), // Bright White
            ],
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
                Color::from_hex("#000000"), // Black
                Color::from_hex("#ff5555"), // Red
                Color::from_hex("#50fa7b"), // Green
                Color::from_hex("#f1fa8c"), // Yellow
                Color::from_hex("#bd93f9"), // Blue
                Color::from_hex("#ff79c6"), // Magenta
                Color::from_hex("#8be9fd"), // Cyan
                Color::from_hex("#bfbfbf"), // White
                Color::from_hex("#4d4d4d"), // Bright Black
                Color::from_hex("#ff6e67"), // Bright Red
                Color::from_hex("#5af78e"), // Bright Green
                Color::from_hex("#f4f99d"), // Bright Yellow
                Color::from_hex("#caa9fa"), // Bright Blue
                Color::from_hex("#ff92d0"), // Bright Magenta
                Color::from_hex("#9aedfe"), // Bright Cyan
                Color::from_hex("#e6e6e6"), // Bright White
            ],
        }
    }
}
