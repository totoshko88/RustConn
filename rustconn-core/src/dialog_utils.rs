//! Dialog utility functions for parsing and formatting connection data
//!
//! These functions handle the conversion between dialog field formats and
//! structured data types used in Connection configurations.

use std::collections::HashMap;

/// Parses a comma-separated list of key=value pairs into a `HashMap`.
///
/// Format: "Key1=Value1, Key2=Value2, ..."
///
/// # Examples
/// ```
/// use rustconn_core::dialog_utils::parse_custom_options;
///
/// let options = parse_custom_options("ForwardAgent=yes, StrictHostKeyChecking=no");
/// assert_eq!(options.get("ForwardAgent"), Some(&"yes".to_string()));
/// assert_eq!(options.get("StrictHostKeyChecking"), Some(&"no".to_string()));
/// ```
#[must_use]
pub fn parse_custom_options(text: &str) -> HashMap<String, String> {
    let mut options = HashMap::new();
    if text.trim().is_empty() {
        return options;
    }

    for part in text.split(',') {
        let part = part.trim();
        if let Some((key, value)) = part.split_once('=') {
            let key = key.trim().to_string();
            let value = value.trim().to_string();
            if !key.is_empty() {
                options.insert(key, value);
            }
        }
    }
    options
}

/// Formats a `HashMap` of options into a comma-separated key=value string.
///
/// This is the inverse of `parse_custom_options`.
///
/// # Examples
/// ```
/// use rustconn_core::dialog_utils::format_custom_options;
/// use std::collections::HashMap;
///
/// let mut options = HashMap::new();
/// options.insert("ForwardAgent".to_string(), "yes".to_string());
/// let formatted = format_custom_options(&options);
/// assert!(formatted.contains("ForwardAgent=yes"));
/// ```
#[must_use]
#[allow(clippy::implicit_hasher)]
pub fn format_custom_options(options: &HashMap<String, String>) -> String {
    let mut pairs: Vec<String> = options.iter().map(|(k, v)| format!("{k}={v}")).collect();
    pairs.sort(); // Sort for deterministic output
    pairs.join(", ")
}

/// Parses a space-separated string into a vector of arguments.
///
/// Note: This is a simple parser that doesn't handle quoted strings.
///
/// # Examples
/// ```
/// use rustconn_core::dialog_utils::parse_args;
///
/// let args = parse_args("/fullscreen /sound:sys:alsa");
/// assert_eq!(args, vec!["/fullscreen", "/sound:sys:alsa"]);
/// ```
#[must_use]
pub fn parse_args(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    text.split_whitespace()
        .map(std::string::ToString::to_string)
        .collect()
}

/// Formats a vector of arguments into a space-separated string.
///
/// This is the inverse of `parse_args`.
///
/// # Examples
/// ```
/// use rustconn_core::dialog_utils::format_args;
///
/// let args = vec!["/fullscreen".to_string(), "/sound:sys:alsa".to_string()];
/// let formatted = format_args(&args);
/// assert_eq!(formatted, "/fullscreen /sound:sys:alsa");
/// ```
#[must_use]
pub fn format_args(args: &[String]) -> String {
    args.join(" ")
}

/// Validates a connection name.
///
/// # Errors
///
/// Returns `Err` with a message if the name is empty or whitespace-only.
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Connection name is required".to_string());
    }
    Ok(())
}

/// Validates a host address.
///
/// # Errors
///
/// Returns `Err` with a message if the host is empty or contains spaces.
pub fn validate_host(host: &str) -> Result<(), String> {
    if host.trim().is_empty() {
        return Err("Host is required".to_string());
    }
    let host_str = host.trim();
    if host_str.contains(' ') {
        return Err("Host cannot contain spaces".to_string());
    }
    Ok(())
}

/// Validates a port number.
///
/// # Errors
///
/// Returns `Err` with a message if the port is zero.
pub fn validate_port(port: u16) -> Result<(), String> {
    if port == 0 {
        return Err("Port must be greater than 0".to_string());
    }
    Ok(())
}

/// Maximum length for a custom icon value.
const ICON_MAX_LEN: usize = 64;

/// Returns `true` if the character is likely an emoji or pictographic symbol.
///
/// Checks Unicode general categories commonly used by emoji: symbols (So),
/// modifier symbols, regional indicators, variation selectors, ZWJ, and
/// skin-tone modifiers.
fn is_emoji_char(c: char) -> bool {
    matches!(c,
        '\u{00A9}'                   // Copyright
        | '\u{00AE}'                 // Registered
        | '\u{200D}'                 // ZWJ
        | '\u{20E3}'                 // Combining enclosing keycap
        | '\u{2122}'                 // Trademark
        | '\u{2194}'..='\u{21AA}'    // Arrows used as emoji
        | '\u{2300}'..='\u{23FF}'    // Misc technical
        | '\u{25AA}'..='\u{25FE}'    // Geometric shapes
        | '\u{2600}'..='\u{27BF}'    // Misc symbols + dingbats
        | '\u{2934}'..='\u{2935}'    // Arrows
        | '\u{2B05}'..='\u{2B07}'    // Arrows
        | '\u{2B1B}'..='\u{2B1C}'    // Squares
        | '\u{2B50}'..='\u{2B55}'    // Stars, circles
        | '\u{3030}'                 // Wavy dash
        | '\u{303D}'                 // Part alternation mark
        | '\u{3297}'                 // Circled ideograph congratulation
        | '\u{3299}'                 // Circled ideograph secret
        | '\u{FE00}'..='\u{FE0F}'    // Variation selectors
        | '\u{E0020}'..='\u{E007F}'  // Tags (flag sequences)
        | '\u{1F000}'..='\u{1FAFF}'  // Main emoji blocks
    )
}

/// Returns `true` if the string looks like an emoji sequence.
///
/// An emoji sequence is 1-10 codepoints where every character passes
/// [`is_emoji_char`].  The grapheme cluster count is at most 2 (to allow
/// flag sequences and family emoji that are a single visual glyph but
/// many codepoints).
fn is_emoji_sequence(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    if chars.is_empty() || chars.len() > 10 {
        return false;
    }
    chars.iter().all(|c| is_emoji_char(*c))
}

/// Returns `true` if the string is a valid GTK icon name.
///
/// Valid GTK icon names consist of ASCII lowercase letters, digits,
/// hyphens, and underscores.  They must not be empty and must not
/// exceed [`ICON_MAX_LEN`] characters.
fn is_valid_gtk_icon_name(s: &str) -> bool {
    if s.is_empty() || s.len() > ICON_MAX_LEN {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && !s.starts_with('-')
        && !s.starts_with('_')
}

/// Validates a custom icon value for a connection or group.
///
/// Accepted formats:
/// - Empty string (no icon)
/// - Emoji sequence (1-2 visible glyphs, e.g. "🇺🇦", "🖥️", "⚡")
/// - GTK icon name (lowercase ASCII + hyphens, e.g. "starred-symbolic")
///
/// # Errors
///
/// Returns `Err` with a human-readable message (English, suitable for
/// wrapping with `gettext` on the GUI side) when the value is invalid.
pub fn validate_icon(icon: &str) -> Result<(), String> {
    let icon = icon.trim();
    if icon.is_empty() {
        return Ok(());
    }

    // Try emoji first
    if is_emoji_sequence(icon) {
        return Ok(());
    }

    // Try GTK icon name
    if is_valid_gtk_icon_name(icon) {
        return Ok(());
    }

    // Neither — produce a helpful error
    if icon.len() > ICON_MAX_LEN {
        return Err("Icon value is too long (max 64 characters)".to_string());
    }

    Err(
        "Icon must be an emoji or a valid GTK icon name (lowercase letters, digits, hyphens)"
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_custom_options_empty() {
        let options = parse_custom_options("");
        assert!(options.is_empty());
    }

    #[test]
    fn test_parse_custom_options_single() {
        let options = parse_custom_options("Key=Value");
        assert_eq!(options.len(), 1);
        assert_eq!(options.get("Key"), Some(&"Value".to_string()));
    }

    #[test]
    fn test_parse_custom_options_multiple() {
        let options = parse_custom_options("Key1=Value1, Key2=Value2");
        assert_eq!(options.len(), 2);
        assert_eq!(options.get("Key1"), Some(&"Value1".to_string()));
        assert_eq!(options.get("Key2"), Some(&"Value2".to_string()));
    }

    #[test]
    fn test_parse_args_empty() {
        let args = parse_args("");
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_args_single() {
        let args = parse_args("/fullscreen");
        assert_eq!(args, vec!["/fullscreen"]);
    }

    #[test]
    fn test_parse_args_multiple() {
        let args = parse_args("/fullscreen /sound:sys:alsa");
        assert_eq!(args, vec!["/fullscreen", "/sound:sys:alsa"]);
    }

    #[test]
    fn test_validate_icon_empty() {
        assert!(validate_icon("").is_ok());
        assert!(validate_icon("  ").is_ok());
    }

    #[test]
    fn test_validate_icon_emoji() {
        assert!(validate_icon("⚡").is_ok());
        assert!(validate_icon("🖥️").is_ok());
        assert!(validate_icon("🇺🇦").is_ok());
        assert!(validate_icon("🔒").is_ok());
    }

    #[test]
    fn test_validate_icon_gtk_name() {
        assert!(validate_icon("starred-symbolic").is_ok());
        assert!(validate_icon("network-server-symbolic").is_ok());
        assert!(validate_icon("computer-symbolic").is_ok());
        assert!(validate_icon("folder-symbolic").is_ok());
    }

    #[test]
    fn test_validate_icon_rejects_arbitrary_text() {
        assert!(validate_icon("hello world").is_err());
        assert!(validate_icon("Ж").is_err());
        assert!(validate_icon("abc DEF").is_err());
        assert!(validate_icon("My Icon").is_err());
        assert!(validate_icon("<script>").is_err());
    }

    #[test]
    fn test_validate_icon_rejects_uppercase() {
        assert!(validate_icon("Starred-Symbolic").is_err());
        assert!(validate_icon("ICON").is_err());
    }

    #[test]
    fn test_validate_icon_rejects_too_long() {
        let long = "a".repeat(65);
        assert!(validate_icon(&long).is_err());
    }

    #[test]
    fn test_validate_icon_rejects_leading_hyphen() {
        assert!(validate_icon("-symbolic").is_err());
    }
}
