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

/// Reads the saved language from `config.toml` and applies it at startup.
///
/// If a non-system language is configured and the `LANGUAGE` env var is
/// not already set to it, this function re-executes the current process
/// with `LANGUAGE` set. This is the only reliable way to make GNU gettext
/// use a specific language without calling `std::env::set_var` (which is
/// `unsafe` in Rust 2024 edition).
///
/// The re-exec happens before GTK or tokio start, so it is safe.
/// A sentinel env var (`_RUSTCONN_LANG_SET`) prevents infinite re-exec loops.
pub fn apply_language_from_config() {
    use std::os::unix::process::CommandExt;

    let lang = read_language_from_config().unwrap_or_default();
    if lang.is_empty() || lang == "system" {
        return;
    }

    // Check if LANGUAGE is already set correctly (e.g. after re-exec
    // or if the user/desktop set it). If so, nothing to do.
    if std::env::var("LANGUAGE").ok().as_deref() == Some(lang.as_str()) {
        return;
    }

    // Check sentinel to avoid infinite re-exec loop
    if std::env::var("_RUSTCONN_LANG_SET").ok().as_deref() == Some("1") {
        // We already re-execed once — don't loop. Fall through to
        // best-effort setlocale below.
        apply_language_setlocale(&lang);
        return;
    }

    // Re-exec ourselves with LANGUAGE set. This replaces the current
    // process image, so nothing after this line executes on success.
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(?e, "Cannot determine current exe for language re-exec");
            apply_language_setlocale(&lang);
            return;
        }
    };

    let args: Vec<String> = std::env::args().collect();
    let full_locale = lang_to_locale(&lang);

    let err = std::process::Command::new(exe)
        .args(&args[1..])
        .env("LANGUAGE", &lang)
        .env("LC_MESSAGES", &full_locale)
        .env("_RUSTCONN_LANG_SET", "1")
        .exec();

    // exec() only returns on error
    tracing::warn!(?err, "Language re-exec failed; using setlocale fallback");
    apply_language_setlocale(&lang);
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
        if in_ui_section && let Some(rest) = trimmed.strip_prefix("language") {
            let rest = rest.trim_start();
            if let Some(rest) = rest.strip_prefix('=') {
                let val = rest.trim().trim_matches('"');
                if !val.is_empty() {
                    return Some(val.to_string());
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
    if let Some(build_locale) = option_env!("RUSTCONN_LOCALE_DIR")
        && !build_locale.is_empty()
        && std::path::Path::new(build_locale).exists()
    {
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

/// Applies a language override using `setlocale` only (best effort).
///
/// This is the runtime fallback used when `set_var` is unavailable.
/// It works when the target locale is installed on the system.
/// For full gettext support (including uninstalled locales), the
/// `LANGUAGE` env var must be set before process start — see
/// [`apply_language_from_config`] which handles this via re-exec.
fn apply_language_setlocale(lang: &str) {
    if lang == "system" || lang.is_empty() {
        gettextrs::setlocale(gettextrs::LocaleCategory::LcMessages, "");
    } else {
        let full_locale = lang_to_locale(lang);
        let result =
            gettextrs::setlocale(gettextrs::LocaleCategory::LcMessages, full_locale.as_str());
        if result.is_none() {
            tracing::info!(
                lang,
                "Locale {full_locale} not installed; \
                 translations may not take effect until restart"
            );
            // Try en_US.UTF-8 as a non-C locale so gettext doesn't
            // fall back to msgids. The LANGUAGE env var (set at startup
            // via re-exec) is the primary lookup mechanism.
            gettextrs::setlocale(gettextrs::LocaleCategory::LcMessages, "en_US.UTF-8");
        }
    }

    // Re-bind domain so gettext picks up the new locale
    let locale_dir = locale_dir();
    let _ = gettextrs::bindtextdomain(GETTEXT_DOMAIN, locale_dir);
    let _ = gettextrs::bind_textdomain_codeset(GETTEXT_DOMAIN, "UTF-8");
    let _ = gettextrs::textdomain(GETTEXT_DOMAIN);
}

/// Applies a language override by re-initializing gettext with the given locale.
///
/// Pass `"system"` to revert to system locale auto-detection.
///
/// At runtime (e.g. from the Settings dialog), this uses `setlocale` only.
/// The `LANGUAGE` env var cannot be changed without `unsafe` in Rust 2024,
/// so full locale switching (especially for locales not installed on the
/// system) requires an application restart. The setting is persisted to
/// `config.toml` and applied at next startup via [`apply_language_from_config`].
///
/// Note: already-rendered GTK labels are not updated — a restart is always
/// needed for full UI translation.
pub fn apply_language(lang: &str) {
    apply_language_setlocale(lang);
}
