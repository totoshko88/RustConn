//! Internationalization support via gettext
//!
//! This module initializes gettext for the RustConn GUI application
//! and provides convenience macros for translatable strings.
//!
//! # Usage
//!
//! ```ignore
//! use crate::i18n::i18n;
//!
//! let msg = i18n("Connection failed");
//! let msg = i18n_f("Deleted '{}'", &[&name]);
//! let msg = ni18n("1 connection", "{} connections", count);
//! ```

use gettextrs::{gettext, ngettext};

/// The gettext domain for RustConn
pub const GETTEXT_DOMAIN: &str = "rustconn";

/// Initializes gettext for the application.
///
/// Must be called once at startup before any translatable strings are used.
/// Sets up the locale, text domain, and locale directory.
pub fn init() {
    // Set locale from environment
    gettextrs::setlocale(gettextrs::LocaleCategory::LcAll, "");

    // Bind text domain to locale directory
    // In Flatpak: /app/share/locale
    // Native install: /usr/share/locale or ~/.local/share/locale
    let locale_dir = locale_dir();
    gettextrs::bindtextdomain(GETTEXT_DOMAIN, locale_dir).expect("bindtextdomain");
    gettextrs::bind_textdomain_codeset(GETTEXT_DOMAIN, "UTF-8").expect("bind_textdomain_codeset");
    gettextrs::textdomain(GETTEXT_DOMAIN).expect("textdomain");
}

/// Returns the locale directory path.
///
/// Checks `LOCALEDIR` env var first, then standard paths.
fn locale_dir() -> String {
    if let Ok(dir) = std::env::var("LOCALEDIR") {
        return dir;
    }

    // Flatpak
    if std::path::Path::new("/app/share/locale").exists() {
        return "/app/share/locale".to_string();
    }

    // Snap
    if let Ok(snap) = std::env::var("SNAP") {
        let snap_locale = format!("{snap}/share/locale");
        if std::path::Path::new(&snap_locale).exists() {
            return snap_locale;
        }
    }

    // System default
    "/usr/share/locale".to_string()
}

/// Translates a string using gettext.
#[inline]
pub fn i18n(msgid: &str) -> String {
    gettext(msgid)
}

/// Translates a string with format arguments.
///
/// Replaces `{}` placeholders left-to-right with the provided arguments.
///
/// # Example
///
/// ```ignore
/// let msg = i18n_f("Deleted '{}'", &[&connection_name]);
/// ```
pub fn i18n_f(msgid: &str, args: &[&str]) -> String {
    let mut result = gettext(msgid);
    for arg in args {
        if let Some(pos) = result.find("{}") {
            result.replace_range(pos..pos + 2, arg);
        }
    }
    result
}

/// Translates a string with singular/plural forms.
///
/// # Example
///
/// ```ignore
/// let msg = ni18n("{} connection", "{} connections", count);
/// ```
#[inline]
pub fn ni18n(singular: &str, plural: &str, n: u32) -> String {
    ngettext(singular, plural, n)
}

/// Translates a string with singular/plural forms and format arguments.
pub fn ni18n_f(singular: &str, plural: &str, n: u32, args: &[&str]) -> String {
    let mut result = ngettext(singular, plural, n);
    for arg in args {
        if let Some(pos) = result.find("{}") {
            result.replace_range(pos..pos + 2, arg);
        }
    }
    result
}

/// Available languages with their display names.
///
/// Returns a list of `(locale_code, display_name)` pairs.
/// The first entry is always `("system", "System")` for auto-detection.
#[must_use]
pub fn available_languages() -> Vec<(&'static str, &'static str)> {
    vec![
        ("system", "System"),
        ("be", "Беларуская"),
        ("cs", "Čeština"),
        ("da", "Dansk"),
        ("de", "Deutsch"),
        ("en", "English"),
        ("es", "Español"),
        ("fr", "Français"),
        ("it", "Italiano"),
        ("kk", "Қазақша"),
        ("nl", "Nederlands"),
        ("pl", "Polski"),
        ("pt", "Português"),
        ("sk", "Slovenčina"),
        ("sv", "Svenska"),
        ("uk", "Українська"),
    ]
}

/// Applies a language override by re-initializing gettext with the given locale.
///
/// Pass `"system"` to revert to system locale auto-detection.
/// This takes effect for all subsequent `i18n()` / `ni18n()` calls.
/// Note: already-rendered GTK labels are not updated — a restart is needed
/// for full UI translation.
pub fn apply_language(lang: &str) {
    if lang == "system" || lang.is_empty() {
        // Revert to system locale
        gettextrs::setlocale(gettextrs::LocaleCategory::LcMessages, "");
    } else {
        // Try lang.UTF-8 first, then plain lang
        let locale_utf8 = format!("{lang}.UTF-8");
        let result = gettextrs::setlocale(gettextrs::LocaleCategory::LcMessages, &*locale_utf8);
        if result.is_none() {
            gettextrs::setlocale(gettextrs::LocaleCategory::LcMessages, lang);
        }
    }

    // Re-bind domain so gettext picks up the new locale
    let locale_dir = locale_dir();
    let _ = gettextrs::bindtextdomain(GETTEXT_DOMAIN, locale_dir);
    let _ = gettextrs::bind_textdomain_codeset(GETTEXT_DOMAIN, "UTF-8");
    let _ = gettextrs::textdomain(GETTEXT_DOMAIN);
}
