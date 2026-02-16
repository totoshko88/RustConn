//! Keyboard layout detection for RDP sessions
//!
//! Detects the system keyboard layout and maps it to a Windows keyboard
//! layout identifier (KLID) for the RDP protocol. The server uses this
//! to interpret scancodes correctly.
//!
//! # Detection Strategy
//!
//! 1. Check `XKB_DEFAULT_LAYOUT` environment variable
//! 2. Parse `localectl status` output
//! 3. Fall back to US English (`0x0409`)

use std::process::Command;

/// US English keyboard layout (fallback default)
pub const LAYOUT_US_ENGLISH: u32 = 0x0409;

/// Detects the system keyboard layout and returns the Windows KLID.
///
/// Tries environment variables and `localectl` before falling back
/// to US English.
///
/// # Returns
///
/// Windows keyboard layout identifier (e.g. `0x0407` for German).
#[must_use]
pub fn detect_keyboard_layout() -> u32 {
    // 1. Check XKB_DEFAULT_LAYOUT (set by Wayland compositors)
    if let Ok(layout) = std::env::var("XKB_DEFAULT_LAYOUT") {
        let name = layout.split(',').next().unwrap_or(&layout).trim();
        if let Some(klid) = xkb_name_to_klid(name) {
            tracing::debug!(
                "Keyboard layout from XKB_DEFAULT_LAYOUT: {} -> 0x{:04X}",
                name,
                klid
            );
            return klid;
        }
    }

    // 2. Try localectl status
    if let Some(layout) = detect_from_localectl() {
        if let Some(klid) = xkb_name_to_klid(&layout) {
            tracing::debug!(
                "Keyboard layout from localectl: {} -> 0x{:04X}",
                layout,
                klid
            );
            return klid;
        }
    }

    tracing::debug!("Keyboard layout detection failed, using US English (0x0409)");
    LAYOUT_US_ENGLISH
}

/// Parses `localectl status` to extract the XKB layout name.
fn detect_from_localectl() -> Option<String> {
    let output = Command::new("localectl").arg("status").output().ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("X11 Layout:") || trimmed.starts_with("VC Keymap:") {
            let value = trimmed.split(':').nth(1)?.trim();
            // Take first layout if comma-separated
            let name = value.split(',').next().unwrap_or(value).trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Maps an XKB layout name to a Windows keyboard layout identifier (KLID).
///
/// Covers the most common layouts. Returns `None` for unknown layouts.
///
/// # Arguments
///
/// * `name` - XKB layout name (e.g. "de", "fr", "us")
#[must_use]
pub fn xkb_name_to_klid(name: &str) -> Option<u32> {
    // Map of XKB layout names to Windows KLIDs
    // Reference: https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/default-input-locales-for-windows-language-packs
    match name {
        "us" => Some(0x0409),
        "gb" | "uk" => Some(0x0809),
        "de" => Some(0x0407),
        "fr" => Some(0x040C),
        "es" => Some(0x040A),
        "it" => Some(0x0410),
        "pt" => Some(0x0816),
        "br" => Some(0x0416),
        "nl" => Some(0x0413),
        "be" => Some(0x080C), // Belgian French
        "ch" => Some(0x0807), // Swiss German
        "at" => Some(0x0C07), // Austrian German
        "se" => Some(0x041D),
        "no" => Some(0x0414),
        "dk" => Some(0x0406),
        "fi" => Some(0x040B),
        "pl" => Some(0x0415),
        "cz" => Some(0x0405),
        "sk" => Some(0x041B),
        "hu" => Some(0x040E),
        "ro" => Some(0x0418),
        "bg" => Some(0x0402),
        "hr" => Some(0x041A),
        "si" => Some(0x0424),
        "rs" | "sr" => Some(0x081A),
        "ru" => Some(0x0419),
        "ua" => Some(0x0422),
        "by" => Some(0x0423),
        "tr" => Some(0x041F),
        "gr" | "el" => Some(0x0408),
        "il" | "he" => Some(0x040D),
        "ar" => Some(0x0401),
        "jp" => Some(0x0411),
        "kr" | "ko" => Some(0x0412),
        "cn" | "zh" => Some(0x0804),
        "tw" => Some(0x0404),
        "th" => Some(0x041E),
        "in" => Some(0x0439), // Hindi
        "ie" => Some(0x1809), // Irish English
        "is" => Some(0x040F),
        "ee" => Some(0x0425),
        "lt" => Some(0x0427),
        "lv" => Some(0x0426),
        "latam" => Some(0x080A), // Latin American Spanish
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xkb_name_to_klid_common_layouts() {
        assert_eq!(xkb_name_to_klid("us"), Some(0x0409));
        assert_eq!(xkb_name_to_klid("de"), Some(0x0407));
        assert_eq!(xkb_name_to_klid("fr"), Some(0x040C));
        assert_eq!(xkb_name_to_klid("gb"), Some(0x0809));
        assert_eq!(xkb_name_to_klid("ru"), Some(0x0419));
        assert_eq!(xkb_name_to_klid("ua"), Some(0x0422));
        assert_eq!(xkb_name_to_klid("jp"), Some(0x0411));
    }

    #[test]
    fn test_xkb_name_to_klid_unknown() {
        assert_eq!(xkb_name_to_klid("unknown_layout"), None);
        assert_eq!(xkb_name_to_klid(""), None);
    }

    #[test]
    fn test_xkb_name_to_klid_aliases() {
        // UK alias
        assert_eq!(xkb_name_to_klid("uk"), Some(0x0809));
        // Serbian aliases
        assert_eq!(xkb_name_to_klid("rs"), Some(0x081A));
        assert_eq!(xkb_name_to_klid("sr"), Some(0x081A));
    }

    #[test]
    fn test_detect_keyboard_layout_returns_valid() {
        let klid = detect_keyboard_layout();
        // Should always return a valid KLID (at minimum the US fallback)
        assert!(klid > 0);
    }
}
