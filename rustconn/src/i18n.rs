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
    // Development: OUT_DIR/locale (compiled by build.rs)
    let locale_dir = locale_dir();
    tracing::debug!(locale_dir, "gettext locale directory");
    gettextrs::bindtextdomain(GETTEXT_DOMAIN, locale_dir).expect("bindtextdomain");
    gettextrs::bind_textdomain_codeset(GETTEXT_DOMAIN, "UTF-8").expect("bind_textdomain_codeset");
    gettextrs::textdomain(GETTEXT_DOMAIN).expect("textdomain");
}

/// Reads the saved language from `config.toml` and applies it early.
///
/// This is called in `main()` right after `init()`, before GTK starts,
/// so that all `i18n()` calls during UI construction use the correct locale.
/// It reads the config file directly (without `ConfigManager`) to avoid
/// pulling in heavy dependencies at this early stage.
pub fn apply_language_from_config() {
    let lang = read_language_from_config().unwrap_or_default();
    if !lang.is_empty() && lang != "system" {
        apply_language(&lang);
    }
}

/// Reads just the `language` field from `~/.config/rustconn/config.toml`.
fn read_language_from_config() -> Option<String> {
    let config_dir = dirs::config_dir()?;
    let path = config_dir.join("rustconn").join("config.toml");
    let content = std::fs::read_to_string(path).ok()?;
    // Simple TOML parsing: find `language = "xx"` under [ui] section
    let mut in_ui_section = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_ui_section = trimmed == "[ui]";
            continue;
        }
        if in_ui_section {
            if let Some(rest) = trimmed.strip_prefix("language") {
                let rest = rest.trim_start();
                if let Some(rest) = rest.strip_prefix('=') {
                    let val = rest.trim().trim_matches('"');
                    if !val.is_empty() {
                        return Some(val.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Returns the locale directory path.
///
/// Resolution order:
/// 1. `LOCALEDIR` environment variable (explicit override)
/// 2. Build-time locale dir compiled by `build.rs` (`cargo run` development)
/// 3. Flatpak `/app/share/locale`
/// 4. Snap `$SNAP/share/locale`
/// 5. User-local `~/.local/share/locale` (install-desktop.sh)
/// 6. `XDG_DATA_HOME/locale`
/// 7. System `/usr/share/locale`
fn locale_dir() -> String {
    // 1. Explicit override
    if let Ok(dir) = std::env::var("LOCALEDIR") {
        return dir;
    }

    // 2. Build-time locale dir (set by build.rs via cargo:rustc-env)
    let build_locale = env!("RUSTCONN_LOCALE_DIR");
    if !build_locale.is_empty() && std::path::Path::new(build_locale).exists() {
        return build_locale.to_string();
    }

    // 3. Flatpak
    if std::path::Path::new("/app/share/locale").exists() {
        return "/app/share/locale".to_string();
    }

    // 4. Snap
    if let Ok(snap) = std::env::var("SNAP") {
        let snap_locale = format!("{snap}/share/locale");
        if std::path::Path::new(&snap_locale).exists() {
            return snap_locale;
        }
    }

    // 5. User-local install (install-desktop.sh)
    if let Ok(home) = std::env::var("HOME") {
        let local_locale = format!("{home}/.local/share/locale");
        if std::path::Path::new(&local_locale).exists() {
            return local_locale;
        }
    }

    // 6. XDG_DATA_HOME fallback
    if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
        let xdg_locale = format!("{xdg_data}/locale");
        if std::path::Path::new(&xdg_locale).exists() {
            return xdg_locale;
        }
    }

    // 7. System default
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

/// Maps a short language code to its full locale identifier.
///
/// Linux `setlocale` requires the full `ll_CC.UTF-8` form (e.g. `uk_UA.UTF-8`),
/// not just the language code (`uk`). This function provides the mapping.
fn lang_to_locale(lang: &str) -> String {
    let full = match lang {
        "be" => "be_BY",
        "cs" => "cs_CZ",
        "da" => "da_DK",
        "de" => "de_DE",
        "en" => "en_US",
        "es" => "es_ES",
        "fr" => "fr_FR",
        "it" => "it_IT",
        "kk" => "kk_KZ",
        "nl" => "nl_NL",
        "pl" => "pl_PL",
        "pt" => "pt_PT",
        "sk" => "sk_SK",
        "sv" => "sv_SE",
        "uk" => "uk_UA",
        other => other,
    };
    format!("{full}.UTF-8")
}

/// Applies a language override by re-initializing gettext with the given locale.
///
/// Pass `"system"` to revert to system locale auto-detection.
/// This takes effect for all subsequent `i18n()` / `ni18n()` calls.
/// Note: already-rendered GTK labels are not updated — a restart is needed
/// for full UI translation.
///
/// Uses the GNU gettext `LANGUAGE` environment variable as the primary mechanism,
/// which works even when the target locale is not installed on the system.
/// Falls back to `setlocale(LC_MESSAGES)` for completeness.
pub fn apply_language(lang: &str) {
    if lang == "system" || lang.is_empty() {
        // Revert to system locale — remove LANGUAGE override
        std::env::remove_var("LANGUAGE");
        std::env::remove_var("LC_MESSAGES");
        gettextrs::setlocale(gettextrs::LocaleCategory::LcMessages, "");
    } else {
        // Set LANGUAGE env var — this is the primary gettext lookup mechanism.
        // Unlike setlocale, it does NOT require the locale to be installed.
        // GNU gettext checks LANGUAGE first (before LC_MESSAGES) as long as
        // LC_MESSAGES is not "C" or "POSIX".
        std::env::set_var("LANGUAGE", lang);

        // Try to set the full locale for plural forms, collation, etc.
        let full_locale = lang_to_locale(lang);
        let result =
            gettextrs::setlocale(gettextrs::LocaleCategory::LcMessages, full_locale.as_str());
        if result.is_none() {
            // Locale not installed on the system. We need LC_MESSAGES to be
            // something other than "C"/"POSIX" so that GNU gettext honors
            // the LANGUAGE env var. Set LC_MESSAGES env var to the desired
            // locale and call setlocale("") to pick it up. If that also fails,
            // try en_US.UTF-8 as a safe non-C locale.
            tracing::info!(
                lang,
                "Locale {full_locale} not installed; trying fallback for gettext"
            );
            std::env::set_var("LC_MESSAGES", &full_locale);
            let retry = gettextrs::setlocale(gettextrs::LocaleCategory::LcMessages, "");
            if retry.is_none() {
                // Last resort: use en_US.UTF-8 which is almost always installed
                std::env::set_var("LC_MESSAGES", "en_US.UTF-8");
                gettextrs::setlocale(gettextrs::LocaleCategory::LcMessages, "en_US.UTF-8");
            }
        }
    }

    // Re-bind domain so gettext picks up the new locale
    let locale_dir = locale_dir();
    let _ = gettextrs::bindtextdomain(GETTEXT_DOMAIN, locale_dir);
    let _ = gettextrs::bind_textdomain_codeset(GETTEXT_DOMAIN, "UTF-8");
    let _ = gettextrs::textdomain(GETTEXT_DOMAIN);
}
