//! Opening URLs with Ctrl+click.
//!
//! VTE only *detects* links: `set_allow_hyperlink()` makes OSC 8 hyperlinks
//! hover-underline, and `match_add_regex()` does the same for plain-text URLs.
//! Opening a browser is the embedding application's job, and nothing outside
//! the Proton sign-in dialog did it — so a Ctrl+click on a printed URL did
//! nothing, where Konsole opens it. This module adds that click half to every
//! session terminal. It is platform-neutral GTK: `UriLauncher` hands the URI to
//! the system default browser on Linux, macOS and Windows alike; only the
//! modifier differs (see [`modifier_held`]).

use std::cell::Cell;

use gtk4::prelude::*;
use gtk4::{gdk, gio, glib};
use vte4::Terminal;
use vte4::prelude::*;

thread_local! {
    /// Whether links may open. Mirrors `TerminalSettings::allow_hyperlinks` —
    /// the setting that already decides whether VTE treats links as links —
    /// recorded for the same reason as [`super::osc52`]: the click handler runs
    /// per event with no settings at hand, and GTK is single-threaded, so a
    /// thread-local is the whole-process value here, as in [`super::safe_paste`].
    static ENABLED: Cell<bool> = const { Cell::new(false) };
}

/// Records whether links may open.
///
/// Called from `configure_terminal_with_settings`, which the settings dialog
/// re-runs on save, so already-open sessions follow the change.
pub(super) fn set_enabled(enabled: bool) {
    ENABLED.with(|flag| flag.set(enabled));
}

/// Whether links may open.
fn enabled() -> bool {
    ENABLED.with(Cell::get)
}

/// PCRE2 multiline compile flag — required by VTE's `match_add_regex()`.
const PCRE2_MULTILINE: u32 = 0x0000_0400;

/// Plain-text URLs: scheme plus non-space run.
///
// ponytail: trailing sentence punctuation (`. , > )`) stays in the match and
// reaches the browser; trim end characters in [`link_at`] if users hit it.
const URL_PATTERN: &str = r"(?:https?|ftp)://[^\s]+";

/// Whether the event's modifier state opens a link: Ctrl on Linux and
/// Windows, on macOS either ⌘ or Ctrl — ⌘ is what Terminal.app and iTerm use,
/// and GTK's macOS backend may deliver a Ctrl+click as the emulated
/// right-click, which never reaches the primary-button gesture at all.
fn modifier_held(state: gdk::ModifierType) -> bool {
    if cfg!(target_os = "macos") {
        state.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SUPER_MASK)
    } else {
        state.contains(gdk::ModifierType::CONTROL_MASK)
    }
}

/// Underlines URLs under the pointer and opens one on Ctrl+click.
///
/// Installed once per terminal at creation, next to
/// [`super::config::setup_context_menu`]; moving the widget between a tab, a
/// split pane and a detached window carries it along. Opening happens on
/// *release* of the primary button, and the gesture never claims the sequence —
/// VTE keeps its own mouse handling, which a claiming handler broke before
/// (split-view selection, #84's Copy/Paste).
pub(super) fn setup_link_opening(terminal: &Terminal) {
    match vte4::Regex::for_match(URL_PATTERN, PCRE2_MULTILINE) {
        Ok(regex) => {
            let tag = terminal.match_add_regex(&regex, 0);
            terminal.match_set_cursor_name(tag, "pointer");
        }
        Err(error) => tracing::warn!(%error, "URL regex rejected by VTE"),
    }

    let term = terminal.clone();
    let click = gtk4::GestureClick::new();
    click.connect_released(move |gesture, n_press, x, y| {
        // A double Ctrl+click is two releases — open once.
        if n_press != 1 || !enabled() || !modifier_held(gesture.current_event_state()) {
            return;
        }
        let Some(uri) = link_at(&term, x, y) else {
            return;
        };
        open_link(&term, &uri);
    });
    terminal.add_controller(click);
}

/// The link under the point: a regex URL match, else an OSC 8 hyperlink.
fn link_at(terminal: &Terminal, x: f64, y: f64) -> Option<String> {
    terminal
        .check_match_at(x, y)
        .0
        .or_else(|| terminal.check_hyperlink_at(x, y))
        .map(String::from)
}

/// True for a web link GLib and the browser agree on.
///
/// A terminal prints anything, and the browser is handed the raw string, so
/// refuse everything but the plain web schemes with a host:
///
/// - a backslash makes browsers read another authority than GLib does
///   (`https://evil.com\@proton.me/`);
/// - userinfo (`https://proton.me@evil.com/`) makes the *human* read another
///   host than the browser opens — the same trick, seen rather than parsed;
/// - control characters (ESC sequences, CR/LF) have no place in a browser
///   argument;
/// - everything off the web schemes — `ssh:`, `javascript:`, `data:`,
///   `file:` — stays closed. Widen [`is_openable_link`] and `URL_PATTERN`
///   together if one is asked for.
fn is_openable_link(uri: &str) -> bool {
    if uri.contains('\\') || uri.chars().any(char::is_control) {
        return false;
    }
    glib::Uri::parse(uri, glib::UriFlags::NONE).is_ok_and(|u| {
        u.userinfo().is_none()
            && matches!(u.scheme().as_str(), "http" | "https" | "ftp")
            && u.host().is_some_and(|h| !h.is_empty())
    })
}

/// Hands a validated URI to the system browser through GTK's URI launcher.
fn open_link(terminal: &Terminal, uri: &str) {
    if !is_openable_link(uri) {
        tracing::warn!(%uri, "refused to open a non-web link");
        return;
    }
    let window = terminal.root().and_downcast::<gtk4::Window>();
    gtk4::UriLauncher::new(uri).launch(window.as_ref(), gio::Cancellable::NONE, |result| {
        if let Err(error) = result
            && !error.matches(gtk4::DialogError::Dismissed)
        {
            tracing::warn!(%error, "could not open the link");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::is_openable_link;

    #[test]
    fn only_web_links_open() {
        assert!(is_openable_link("https://example.com/a?b=1"));
        assert!(is_openable_link("http://example.com/"));
        assert!(is_openable_link("ftp://example.com/pub/"));
        assert!(!is_openable_link("javascript:alert(1)"));
        assert!(!is_openable_link("data:text/html;base64,AAAA"));
        assert!(!is_openable_link("ssh://git@host/repo"));
        assert!(!is_openable_link("file:///etc/passwd"));
        assert!(!is_openable_link("not a uri"));
        assert!(!is_openable_link("https://"));
        // The backslash trick — GLib reads host example.com, a browser does not.
        assert!(!is_openable_link("https://evil.com\\@example.com/"));
        // Userinfo — the human reads proton.me, the browser opens evil.com.
        assert!(!is_openable_link("https://proton.me@evil.com/"));
        assert!(!is_openable_link("https://user:pw@example.com/"));
        // Control characters in the payload (terminal output can embed ESC).
        let esc = char::from_u32(0x1b).unwrap();
        assert!(!is_openable_link(&format!("https://example.com/{esc}x")));
    }
}
