//! Opening URLs with Ctrl+click.
//!
//! VTE only *detects* links: `set_allow_hyperlink()` makes OSC 8 hyperlinks
//! hover-underline, and `match_add_regex()` does the same for plain-text URLs.
//! Opening a browser is the embedding application's job, and nothing outside
//! the Proton sign-in dialog did it — so a Ctrl+click on a printed URL did
//! nothing, where Konsole opens it. This module adds that click half to every
//! session terminal. It is platform-neutral GTK: `UriLauncher` hands the URI to
//! the system default browser on Linux and macOS alike; only the modifier
//! differs (see [`modifier_held`]).
//!
//! Links only, never filesystem paths: a path printed by an SSH, Telnet,
//! serial or Kubernetes session names a file on the *remote* machine, and
//! opening the same string locally would open the wrong file — one the remote
//! side gets to choose.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

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

/// Whether the event's modifier state opens a link: Ctrl on Linux, on macOS
/// either ⌘ or Ctrl — ⌘ is what Terminal.app and iTerm use, and GTK's macOS
/// backend may deliver a Ctrl+click as the emulated
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
/// *release* of the primary button, and only when press and release resolve
/// to the same link, so a Ctrl+drag that ends on a link does not open it.
pub(super) fn setup_link_opening(terminal: &Terminal) {
    for pattern in [URL_PATTERN] {
        match vte4::Regex::for_match(pattern, PCRE2_MULTILINE) {
            Ok(regex) => {
                let tag = terminal.match_add_regex(&regex, 0);
                terminal.match_set_cursor_name(tag, "pointer");
            }
            Err(error) => tracing::warn!(%error, "link regex rejected by VTE"),
        }
    }

    let term_pressed = terminal.clone();
    let term_released = terminal.clone();
    // The link the claimed press landed on; release opens it only if it
    // resolves to the same one.
    let pressed: Rc<RefCell<Option<String>>> = Rc::default();
    let pressed_on_release = Rc::clone(&pressed);
    let click = link_click_gesture();
    click.connect_pressed(move |gesture, _, x, y| {
        pressed.replace(None);
        let state = gesture.current_event_state();
        if !enabled() || !modifier_held(state) {
            return;
        }
        let Some(uri) = link_at(&term_pressed, x, y) else {
            return;
        };
        tracing::debug!(x, y, "link press claimed");
        gesture.set_state(gtk4::EventSequenceState::Claimed);
        pressed.replace(Some(uri));
    });
    click.connect_released(move |gesture, n_press, x, y| {
        let Some(pressed_uri) = pressed_on_release.take() else {
            return;
        };
        if n_press != 1 || !enabled() || !modifier_held(gesture.current_event_state()) {
            return;
        }
        if link_at(&term_released, x, y).as_deref() != Some(pressed_uri.as_str()) {
            return;
        }
        open_link(&term_released, &pressed_uri);
    });
    terminal.add_controller(click);
}

/// The click gesture for links: primary button only, so Ctrl+right-click
/// still reaches the context menu and Ctrl+middle-click still pastes.
///
/// Capture phase, and a claim on the press: VTE's own click controller
/// starts a selection on button 1 and would own the sequence, cancelling
/// this gesture before `released` — the plain-click case stays unclaimed,
/// so VTE keeps its selection handling (the split-view selection bug).
fn link_click_gesture() -> gtk4::GestureClick {
    let click = gtk4::GestureClick::new();
    click.set_button(gdk::BUTTON_PRIMARY);
    click.set_propagation_phase(gtk4::PropagationPhase::Capture);
    click
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
///   `file:` — stays closed.
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

/// Opens a web link in the browser, after the same checks every link gets;
/// anything else is refused.
fn open_link(terminal: &Terminal, text: &str) {
    if !is_openable_link(text) {
        tracing::warn!(%text, "refused to open a non-web link");
        return;
    }
    launch_uri(terminal, text);
}

/// Hands a validated URI to the system browser through GTK's URI launcher.
fn launch_uri(terminal: &Terminal, uri: &str) {
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
    use super::{
        PCRE2_MULTILINE, URL_PATTERN, is_openable_link, link_click_gesture, modifier_held,
        setup_link_opening,
    };
    use gtk4::gdk;
    use gtk4::prelude::*;
    use vte4::Terminal;
    use vte4::prelude::*;

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

    /// Ctrl opens (also on macOS, where ⌘ does — see `modifier_held`); nothing
    /// else does, on any platform this compiles for.
    #[test]
    fn the_open_modifier_is_ctrl_off_macos() {
        assert!(modifier_held(gdk::ModifierType::CONTROL_MASK));
        assert!(!modifier_held(gdk::ModifierType::empty()));
        assert!(!modifier_held(gdk::ModifierType::SHIFT_MASK));
    }

    /// Only the primary button opens links: a gesture on every button (GTK's
    /// default, button 0) claimed Ctrl+right-click and Ctrl+middle-click over
    /// a link, so the context menu never appeared and paste was lost.
    #[test]
    #[ignore = "initialises GTK: needs a display and its own process; run alone with `cargo test -p rustconn --bin rustconn -- --ignored --exact terminal::link_open::tests::the_link_gesture_listens_to_the_primary_button_only`"]
    fn the_link_gesture_listens_to_the_primary_button_only() {
        if gtk4::init().is_err() {
            return;
        }
        let click = link_click_gesture();
        assert_eq!(click.button(), gdk::BUTTON_PRIMARY);
        assert_eq!(click.propagation_phase(), gtk4::PropagationPhase::Capture);
    }

    /// VTE must accept the patterns — a rejected regex means no link ever
    /// underlines or opens, and nothing else in the module would notice.
    #[test]
    fn the_link_regexes_compile() {
        for pattern in [URL_PATTERN] {
            assert!(vte4::Regex::for_match(pattern, PCRE2_MULTILINE).is_ok());
        }
    }

    /// The registered regexes match at the pixels of what was fed: this is
    /// the check that catches a pattern VTE accepts but that matches
    /// nothing, which the compile check above cannot see. Cell size is not
    /// bound by `vte4` 0.10, so the sweep steps over the printed tokens' own
    /// pixels — hundreds of px wide at any font.
    #[test]
    #[ignore = "initialises GTK: needs a display and its own process; run alone with `cargo test -p rustconn --bin rustconn -- --ignored --exact terminal::link_open::tests::the_link_regexes_match_where_they_were_fed`"]
    fn the_link_regexes_match_where_they_were_fed() {
        if gtk4::init().is_err() {
            return;
        }
        let pump = |ms: u64| {
            let ctx = gtk4::glib::MainContext::default();
            let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
            while std::time::Instant::now() < end {
                ctx.iteration(false);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        let terminal = Terminal::new();
        setup_link_opening(&terminal);
        let host = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        host.append(&terminal);
        let window = gtk4::Window::builder()
            .default_width(700)
            .default_height(500)
            .child(&host)
            .build();
        window.present();
        terminal.feed(b"xx https://example.com/rustconn yy\r\n");
        // `feed()` queues the bytes; VTE parses them on a later main-loop pass.
        pump(300);

        let mut found = std::collections::HashSet::new();
        for x in (0..600).step_by(2) {
            for y in (0..60).step_by(2) {
                if let Some(uri) = terminal.check_match_at(f64::from(x), f64::from(y)).0 {
                    found.insert(uri.to_string());
                }
            }
        }
        assert!(
            found.contains("https://example.com/rustconn"),
            "the URL never resolved; found {found:?}"
        );
    }
}
