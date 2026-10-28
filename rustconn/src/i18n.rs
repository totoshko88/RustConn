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
    // Set locale from environment.
    //
    // Routed through `rustconn-locale-sys` because `setlocale` is `unsafe` and
    // sound only while the process is single-threaded (RUSTSEC-2026-0244). That
    // crate checks the precondition and panics if it no longer holds, so the
    // requirement "call this first in `main()`" is enforced rather than assumed.
    //
    // The applied locale is dropped rather than reported, deliberately:
    //
    //  * there is nowhere to report it to. This runs as the first statement of
    //    `main()`, which is the whole point — the tracing subscriber is
    //    installed several statements later, so a `warn!` here is swallowed.
    //  * `None` is not an error state we can act on. It means the locale named
    //    by the environment is not installed, which is routine in a Flatpak
    //    sandbox (the runtime ships only the host language, issue #158). The
    //    previous locale stays in effect, gettext falls back to the untranslated
    //    strings, and the user sees English — visible without a log line.
    //  * where the outcome *is* actionable — a specific language configured by
    //    the user — it is checked: `apply_language_setlocale` below inspects the
    //    result and falls back to the system locale.
    let _startup_locale =
        rustconn_locale_sys::init_locale(rustconn_locale_sys::LocaleCategory::LcAll, "");

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
/// A non-system language is applied in two steps, both of which are needed: the
/// `LANGUAGE` environment variable is set (gettext's primary lookup mechanism,
/// written through [`rustconn_env_sys`] because `std::env::set_var` is `unsafe`
/// in edition 2024), and then the locale is applied with `setlocale`, which is
/// what makes gettext re-read the variable. See [`apply_configured_language`]
/// for why neither half suffices alone.
///
/// # Thread safety
///
/// This is the **only** place that applies a locale after [`init`], and it must
/// stay that way: `setlocale` mutates process-global locale state with no
/// synchronisation, so it is only sound while the process is still
/// single-threaded (RUSTSEC-2026-0244). The same is true of the environment
/// write. Both therefore happen here, called from `main()` before GTK, tokio or
/// the tracing subscriber exist. Applying either later — for example from the GTK
/// `activate` handler, where the GIO worker thread is already running — would
/// reintroduce the unsoundness.
///
/// On return the locale is sealed, so that "later" is a panic during development
/// rather than memory corruption in the field. The environment is sealed
/// separately, by [`crate::renderer::apply_renderer_preference`], which `main()`
/// calls after this — the renderer choice is the other startup variable and has
/// to be written before it can close the window for both.
///
/// # Panics
///
/// Panics if the environment write is refused, which means this ran outside the
/// startup window: after a thread was spawned, from a thread other than the one
/// `main()` runs on, or after the environment was sealed. All three are
/// programming errors in `main()`'s ordering, not runtime conditions.
pub fn apply_language_from_config() {
    apply_configured_language();

    // The startup locale is final. Nothing in the rest of the process lifetime
    // may change it — the Settings dialog only persists the choice for the next
    // start — so close the window `rustconn-locale-sys` guards.
    rustconn_locale_sys::seal_locale();
}

/// Applies the configured language: `LANGUAGE` first, then the locale.
///
/// Split from [`apply_language_from_config`] so that both of its early returns
/// are followed by the seal, without repeating the call.
fn apply_configured_language() {
    let lang = read_language_from_config().unwrap_or_default();
    if lang.is_empty() || lang == "system" {
        // `init()` already applied the system locale with
        // `setlocale(LC_ALL, "")` and bound the domain, and `LC_ALL` covers
        // `LC_MESSAGES`. Re-applying the system locale here would be a no-op.
        return;
    }

    // `LANGUAGE` is gettext's primary lookup mechanism and there is no API for
    // it — the variable is the interface, the same situation `GSK_RENDERER` is
    // in. So the write goes through `rustconn-env-sys`, which is the sanctioned
    // home for it: `std::env::set_var` is `unsafe` in edition 2024 and this
    // crate forbids `unsafe`. The guard there refuses the call once this program
    // has spawned a thread or the startup window has been sealed, which is
    // exactly the condition that makes it sound; `main()` calls us before GTK,
    // tokio and the tracing subscriber for that reason.
    //
    // Only `LANGUAGE` is set, never `LC_MESSAGES`/`LC_ALL`/`LANG`. Inside a
    // Flatpak sandbox the chosen locale (say `fr_FR.UTF-8`) is very often not
    // installed — the runtime ships only the host language via the
    // `org.gnome.Platform.Locale` extension — and pointing `LC_MESSAGES` at an
    // uninstalled locale makes glibc fall back to `C`, whereupon **gettext
    // ignores `LANGUAGE`** and nothing is translated at all (issue #158).
    // Leaving `LC_MESSAGES` inherited keeps it at a locale that does exist.
    //
    // Skipped when the variable already says what we want, which happens when
    // the desktop session set it: a write that changes nothing is still a write
    // to the process-global environment block, and this one has a guard to spend.
    if std::env::var("LANGUAGE").ok().as_deref() != Some(lang.as_str()) {
        rustconn_env_sys::set_startup_var("LANGUAGE", &lang);
    }

    // The variable alone is not enough, and this is the other half of #158:
    // gettext caches the catalogue it resolved and only re-reads `LANGUAGE` when
    // something bumps its internal counter, which `setlocale` does. This call is
    // that bump as much as it is the locale change — and it is made here, in
    // `main()`, rather than later from the GTK `activate` handler, because
    // `setlocale` is only sound while the process is single-threaded.
    //
    // This used to be reached by re-execing the process with `LANGUAGE` set in
    // the child, which sidesteps `set_var` but costs a process and is actively
    // harmful on macOS: replacing the process image destroys the LaunchServices
    // scene registration `NSStatusItem` needs, so anyone running a non-system
    // interface language silently lost the tray icon. That was the same defect
    // #274 fixed for the renderer; the sentinel variable the loop-guard needed
    // (`_RUSTCONN_LANG_SET`) is gone with it.
    apply_language_setlocale(&lang);
}

/// Reads just the `language` field from `~/.config/rustconn/config.toml`.
///
/// Shares the single-key scan with the renderer choice, which faces the same
/// constraint — see [`crate::startup_config`] for why neither can wait for the
/// application's own settings to load.
fn read_language_from_config() -> Option<String> {
    crate::startup_config::read_ui_string("language")
}

/// Checks whether a build-time locale directory actually contains at least
/// one compiled `.mo` file for the `rustconn` domain.
///
/// This prevents stale build-time paths (baked in via `cargo:rustc-env`) from
/// shadowing the real system locale directory in packaged installs.
fn build_locale_has_translations(dir: &str) -> bool {
    let path = std::path::Path::new(dir);
    if !path.is_dir() {
        return false;
    }
    // Expect structure: <dir>/<lang>/LC_MESSAGES/rustconn.mo
    let Ok(entries) = std::fs::read_dir(path) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .path()
            .join("LC_MESSAGES")
            .join("rustconn.mo")
            .is_file()
    })
}

/// Returns the locale directory path.
///
/// Resolution order:
/// 1. `LOCALEDIR` environment variable (explicit override).
///    Set in the Flatpak manifest to `/app/share/rustconn/locale` so our
///    translations bypass `flatpak-builder`'s automatic split of
///    `/app/share/locale/<lang>/` into per-language Locale extension
///    subsets (issue #158).
/// 2. Build-time locale dir compiled by `build.rs` (`cargo run` development)
/// 3. macOS .app bundle: `Contents/Resources/locale` relative to executable
/// 4. Flatpak `/app/share/locale` (legacy fallback for older builds)
/// 5. Snap `$SNAP/share/locale`
/// 6. AppImage `$APPDIR/usr/share/locale` (or `../share/locale` relative to
///    the executable's directory), guarded by catalogue presence (issue #400)
/// 7. User-local `~/.local/share/locale` (install-desktop.sh)
/// 8. `XDG_DATA_HOME/locale`
/// 9. System `/usr/share/locale`
fn locale_dir() -> String {
    // 1. Explicit override
    if let Ok(dir) = std::env::var("LOCALEDIR") {
        return dir;
    }

    // 2. Build-time locale dir (set by build.rs via cargo:rustc-env)
    //    Only use it if the directory actually contains .mo files —
    //    packaged installs (deb/rpm/flatpak) place translations in
    //    /usr/share/locale or /app/share/locale, so the stale build-time
    //    path must not shadow the real system locale directory.
    if let Some(build_locale) = option_env!("RUSTCONN_LOCALE_DIR")
        && !build_locale.is_empty()
        && build_locale_has_translations(build_locale)
    {
        return build_locale.to_string();
    }

    // 3. macOS .app bundle detection
    //    When launched via LaunchServices, LOCALEDIR is not set but the
    //    translations live at .app/Contents/Resources/locale/
    #[cfg(target_os = "macos")]
    if let Some(bundle_locale) = macos_bundle_locale_dir() {
        return bundle_locale;
    }

    // 4. Flatpak
    if std::path::Path::new("/app/share/locale").exists() {
        return "/app/share/locale".to_string();
    }

    // 5. Snap
    if let Ok(snap) = std::env::var("SNAP") {
        let snap_locale = format!("{snap}/share/locale");
        if std::path::Path::new(&snap_locale).exists() {
            return snap_locale;
        }
    }

    // 6. AppImage
    //    The bundle is FUSE-mounted at a random path exported as $APPDIR, and
    //    its catalogues live inside it, so no host path finds them (issue #400).
    #[cfg(target_os = "linux")]
    {
        let appdir = std::env::var("APPDIR").ok();
        let exe = std::env::current_exe().ok();
        let exe_dir = exe.as_deref().and_then(std::path::Path::parent);
        if let Some(appimage_locale) = appimage_locale_dir(appdir.as_deref(), exe_dir) {
            return appimage_locale;
        }
    }

    // 7. User-local install (install-desktop.sh)
    if let Ok(home) = std::env::var("HOME") {
        let local_locale = format!("{home}/.local/share/locale");
        if build_locale_has_translations(&local_locale) {
            return local_locale;
        }
    }

    // 8. XDG_DATA_HOME fallback
    if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
        let xdg_locale = format!("{xdg_data}/locale");
        if build_locale_has_translations(&xdg_locale) {
            return xdg_locale;
        }
    }

    // 9. System default
    "/usr/share/locale".to_string()
}

/// Detects the locale directory inside a macOS .app bundle.
///
/// When the executable is at `RustConn.app/Contents/MacOS/rustconn`,
/// translations are at `RustConn.app/Contents/Resources/locale/`.
#[cfg(target_os = "macos")]
fn macos_bundle_locale_dir() -> Option<String> {
    let exe_path = std::env::current_exe().ok()?;
    // exe is at .app/Contents/MacOS/rustconn
    let macos_dir = exe_path.parent()?;
    let contents_dir = macos_dir.parent()?;
    let bundle_dir = contents_dir.parent()?;

    // Verify this looks like a .app bundle
    let bundle_ext = bundle_dir.extension().and_then(|e| e.to_str())?;
    if !bundle_ext.eq_ignore_ascii_case("app") {
        return None;
    }

    let locale_dir = contents_dir.join("Resources").join("locale");
    if build_locale_has_translations(&locale_dir.to_string_lossy()) {
        Some(locale_dir.to_string_lossy().into_owned())
    } else {
        None
    }
}

/// Detects the locale directory inside a mounted AppImage.
///
/// `appdir` is the value of `$APPDIR` (the AppImage mount point) and `exe_dir`
/// the directory of the running executable; both are passed in rather than read
/// here so the lookup can be tested without touching the process environment.
/// Candidates, in order: `<appdir>/usr/share/locale`, then
/// `<exe_dir>/../share/locale` when `exe_dir` is a relocated `<root>/usr/bin`
/// other than the host's `/usr/bin` (the binary sits at `$APPDIR/usr/bin`). A
/// candidate is accepted only if it holds a `rustconn.mo`, so a normal `/usr`
/// install with a stray `$APPDIR` falls through to the system directory.
#[cfg(target_os = "linux")]
fn appimage_locale_dir(appdir: Option<&str>, exe_dir: Option<&std::path::Path>) -> Option<String> {
    let from_appdir = appdir.filter(|dir| !dir.is_empty()).map(|dir| {
        std::path::Path::new(dir)
            .join("usr")
            .join("share")
            .join("locale")
    });
    // Only a relocated `<root>/usr/bin` qualifies, never the host's own
    // `/usr/bin`: that would put `/usr/share/locale` ahead of the user-local
    // branch for a normal install.
    let from_exe = exe_dir
        .filter(|dir| {
            let usr = dir.parent();
            dir.file_name().is_some_and(|name| name == "bin")
                && usr
                    .and_then(std::path::Path::file_name)
                    .is_some_and(|name| name == "usr")
                && usr
                    .and_then(std::path::Path::parent)
                    .is_some_and(|root| root.parent().is_some())
        })
        .and_then(std::path::Path::parent)
        .map(|usr| usr.join("share").join("locale"));

    let found = [from_appdir, from_exe]
        .into_iter()
        .flatten()
        .map(|candidate| candidate.to_string_lossy().into_owned())
        .find(|candidate| build_locale_has_translations(candidate));
    if let Some(dir) = &found {
        tracing::debug!(locale_dir = %dir, "AppImage locale directory");
    }
    found
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
        ("zh_CN", "简体中文"),
    ]
}

/// Maps a short language code to its full locale identifier.
///
/// Linux `setlocale` requires the full `ll_CC.UTF-8` form (e.g. `uk_UA.UTF-8`),
/// not just the language code (`uk`). This function provides the mapping.
///
/// `zh-cn` is accepted alongside `zh_CN` because that is what the catalogue —
/// and therefore this list, and therefore a stored `ui.language` — used to be
/// called. The hyphenated form was never a valid locale directory name, so
/// gettext looked for `zh_CN/LC_MESSAGES/rustconn.mo` while every packaging
/// script installed `zh-cn/LC_MESSAGES/rustconn.mo` and the Chinese translation
/// never loaded anywhere. The file is now `po/zh_CN.po`; the alias only keeps an
/// existing configuration working.
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
        "zh-cn" | "zh_CN" => "zh_CN",
        other => other,
    };
    format!("{full}.UTF-8")
}

/// Applies a language override using `setlocale` only (best effort).
///
/// It works when the target locale is installed on the system. For full gettext
/// support (including uninstalled locales) the `LANGUAGE` variable does the
/// work — see [`apply_configured_language`], which sets it just before calling
/// this. This function is still needed alongside `LANGUAGE` twice over: gettext
/// ignores `LANGUAGE` when `LC_MESSAGES` resolves to `"C"` (issue #158), and
/// `setlocale` is what invalidates the catalogue gettext has already resolved,
/// so without it a variable set in-process would not be noticed.
///
/// # Safety-adjacent invariant
///
/// Callable only from [`apply_configured_language`], which runs in `main()`
/// before any thread is spawned. `setlocale` writes process-global locale state
/// without synchronisation, so calling this once other threads exist is unsound
/// (RUSTSEC-2026-0244). [`rustconn_locale_sys::init_locale`] checks that and
/// panics rather than proceeding, but keep it private and keep the single call
/// site anyway: a panic at startup is still a broken build.
fn apply_language_setlocale(lang: &str) {
    use rustconn_locale_sys::{LocaleCategory, init_locale};

    /// Applies the environment's `LC_MESSAGES`, reporting a failure.
    ///
    /// This is the last resort on every path through
    /// [`apply_language_setlocale`], so unlike the startup call in [`init`] the
    /// outcome is worth a line: if even the inherited locale cannot be applied,
    /// `LC_MESSAGES` keeps whatever it had and gettext may serve untranslated
    /// strings (issue #158).
    ///
    /// Best effort, like the other `tracing` calls in this module: `main()`
    /// installs the subscriber after `apply_language_from_config()`, so the
    /// record is only seen if that order ever changes. Locale setup has to run
    /// first — it is single-threaded-only — so the alternative is not reporting
    /// at all.
    fn apply_system_messages_locale() {
        if init_locale(LocaleCategory::LcMessages, "").is_none() {
            tracing::warn!(
                "setlocale(LC_MESSAGES, \"\") failed: the locale named by the environment \
                 is not installed. Keeping the previous LC_MESSAGES; menus and messages \
                 may stay untranslated."
            );
        }
    }

    if lang == "system" || lang.is_empty() {
        apply_system_messages_locale();
    } else {
        let full_locale = lang_to_locale(lang);
        let result = init_locale(LocaleCategory::LcMessages, full_locale.as_str());
        if result.is_none() {
            tracing::info!(
                lang,
                "Locale {full_locale} not installed; \
                 falling back to system locale (LANGUAGE env var still applies)"
            );
            // Fall back to the system locale ("") rather than a hardcoded
            // en_US.UTF-8: in Flatpak sandboxes en_US.UTF-8 is itself often
            // not installed, which would leave LC_MESSAGES=C and disable
            // gettext's LANGUAGE lookup entirely (issue #158). The system
            // locale inherited from the host is guaranteed to exist.
            apply_system_messages_locale();
        }
    }

    // Re-bind domain so gettext picks up the new locale
    let locale_dir = locale_dir();
    // best-effort: bindtextdomain returns the bound directory or NULL on
    // OOM. There is nothing the app can do here, and falling back to the
    // previous binding is acceptable.
    let _ = gettextrs::bindtextdomain(GETTEXT_DOMAIN, locale_dir);
    let _ = gettextrs::bind_textdomain_codeset(GETTEXT_DOMAIN, "UTF-8");
    let _ = gettextrs::textdomain(GETTEXT_DOMAIN);
}

// A public `apply_language()` used to exist here so the GTK `activate` handler
// could re-apply the saved language after the app state was built. It was
// removed in 0.19.19: by that point the GIO worker thread is running, and
// `setlocale` is unsound once the process is multi-threaded
// (RUSTSEC-2026-0244). It was also redundant — `apply_language_from_config()`
// now applies the locale on every path from `main()`, before any thread starts,
// including the case that call was there to rescue (LANGUAGE set but
// LC_MESSAGES stuck at "C" inside a Flatpak sandbox, issue #158).
//
// Re-adding it would now panic on the first run instead of quietly corrupting
// locale state: `setlocale` lives behind `rustconn_locale_sys::init_locale`, and
// the locale is sealed at the end of `apply_language_from_config()`.
//
// Changing the language from the Settings dialog still only persists the value
// to `config.toml`; it takes effect on the next start. That was already true —
// the old runtime call could not retranslate already-rendered GTK labels.

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::appimage_locale_dir;

    /// Creates `<root>/usr/share/locale/uk/LC_MESSAGES/rustconn.mo`.
    fn fake_catalogue(root: &std::path::Path) {
        let messages = root.join("usr/share/locale/uk/LC_MESSAGES");
        std::fs::create_dir_all(&messages).unwrap();
        std::fs::write(messages.join("rustconn.mo"), b"").unwrap();
    }

    #[test]
    fn appimage_locale_dir_resolves_appdir_catalogue() {
        let tmp = tempfile::TempDir::new().unwrap();
        fake_catalogue(tmp.path());
        let appdir = tmp.path().to_str().unwrap();

        let expected = tmp.path().join("usr").join("share").join("locale");
        assert_eq!(
            appimage_locale_dir(Some(appdir), None),
            Some(expected.to_string_lossy().into_owned())
        );
    }

    #[test]
    fn appimage_locale_dir_falls_through_without_catalogue() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("usr/share/locale/uk/LC_MESSAGES")).unwrap();
        std::fs::create_dir_all(tmp.path().join("usr/bin")).unwrap();
        let appdir = tmp.path().to_str().unwrap();

        assert_eq!(
            appimage_locale_dir(Some(appdir), Some(&tmp.path().join("usr/bin"))),
            None
        );
        assert_eq!(appimage_locale_dir(None, None), None);
        // A normal install's `/usr/bin` is never treated as a bundle, whatever
        // `/usr/share/locale` holds on this host.
        assert_eq!(
            appimage_locale_dir(None, Some(std::path::Path::new("/usr/bin"))),
            None
        );
    }

    #[test]
    fn appimage_locale_dir_falls_back_to_executable_relative() {
        let tmp = tempfile::TempDir::new().unwrap();
        fake_catalogue(tmp.path());
        let exe_dir = tmp.path().join("usr").join("bin");
        std::fs::create_dir_all(&exe_dir).unwrap();

        let found = appimage_locale_dir(None, Some(&exe_dir)).unwrap();
        assert_eq!(
            std::fs::canonicalize(found).unwrap(),
            std::fs::canonicalize(tmp.path().join("usr/share/locale")).unwrap()
        );
    }
}
