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
//!
//! The press is claimed in the capture phase whenever Ctrl (⌘) is held over a
//! link, so in an app that reports the mouse — mc, vim with `mouse=a`, tmux
//! with `mouse on`, htop — a Ctrl+click on link text opens the link and is not
//! forwarded to the app. Clicks anywhere else still reach it, and Shift+drag
//! still selects text past the app's mouse reporting, as in any VTE terminal.
//! `vte4` has no getter for the mouse-tracking mode, so this cannot be skipped
//! only while an app is tracking.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{gdk, gio, glib};
use libadwaita as adw;
use vte4::Terminal;
use vte4::prelude::*;

use super::PCRE2_MULTILINE;
use crate::i18n::i18n;

thread_local! {
    /// Whether links may open. Mirrors `TerminalSettings::allow_hyperlinks` —
    /// the setting that already decides whether VTE treats links as links —
    /// recorded for the same reason as [`super::osc52`]: the click handler runs
    /// per event with no settings at hand, and GTK is single-threaded, so a
    /// thread-local is the whole-process value here, as in [`super::safe_paste`].
    static ENABLED: Cell<bool> = const { Cell::new(false) };
}

/// A terminal with link opening installed, and the tag of its link regex
/// while one is registered with VTE.
struct Registered {
    terminal: glib::WeakRef<Terminal>,
    tag: Rc<Cell<Option<i32>>>,
}

thread_local! {
    /// Every terminal [`setup_link_opening`] ran on, so a settings change can
    /// add or remove its link regex.
    static REGISTERED: RefCell<Vec<Registered>> = const { RefCell::new(Vec::new()) };
}

/// Records whether links may open, and adds or removes `terminal`'s link
/// regex to match: with **Allow hyperlinks** off, printed URLs neither
/// underline nor take the pointer cursor.
///
/// Called from `configure_terminal_with_settings`, which the settings dialog
/// re-runs on save for every open terminal, so already-open sessions follow
/// the change.
pub(super) fn set_enabled(terminal: &Terminal, enabled: bool) {
    ENABLED.with(|flag| flag.set(enabled));
    // ponytail: linear scan over the open terminals, a handful per window;
    // index by widget if sessions ever number in the hundreds.
    let tag = REGISTERED.with(|registered| {
        let mut registered = registered.borrow_mut();
        registered.retain(|entry| entry.terminal.upgrade().is_some());
        registered
            .iter()
            .find(|entry| entry.terminal.upgrade().as_ref() == Some(terminal))
            .map(|entry| Rc::clone(&entry.tag))
    });
    if let Some(tag) = tag {
        sync_link_match(terminal, &tag, enabled);
    }
}

/// Whether links may open.
fn enabled() -> bool {
    ENABLED.with(Cell::get)
}

/// Plain-text URLs: scheme (any case) plus non-space run. Trailing sentence
/// punctuation is trimmed after the match, in [`resolve_link`].
const URL_PATTERN: &str = r"(?i:https?)://[^\s]+";

/// Whether the event's modifier state opens a link: Ctrl on Linux, on macOS
/// either ⌘ or Ctrl — ⌘ is what Terminal.app and iTerm use, and GTK's macOS
/// backend may deliver a Ctrl+click as the emulated right-click, which never
/// reaches the primary-button gesture at all.
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
    let tag: Rc<Cell<Option<i32>>> = Rc::default();
    sync_link_match(terminal, &tag, enabled());
    REGISTERED.with(|registered| {
        registered.borrow_mut().push(Registered {
            terminal: terminal.downgrade(),
            tag: Rc::clone(&tag),
        });
    });

    let tag_pressed = Rc::clone(&tag);
    let tag_released = tag;
    let term_pressed = terminal.clone();
    let term_released = terminal.clone();
    // The link the claimed press landed on; release opens it only if it
    // resolves to the same one.
    let pressed: Rc<RefCell<Option<Link>>> = Rc::default();
    let pressed_on_release = Rc::clone(&pressed);
    let click = link_click_gesture();
    click.connect_pressed(move |gesture, _, x, y| {
        pressed.replace(None);
        let state = gesture.current_event_state();
        if !enabled() || !modifier_held(state) {
            return;
        }
        let Some(link) = link_at(&term_pressed, tag_pressed.get(), x, y) else {
            return;
        };
        tracing::debug!(x, y, "link press claimed");
        gesture.set_state(gtk4::EventSequenceState::Claimed);
        pressed.replace(Some(link));
    });
    click.connect_released(move |gesture, n_press, x, y| {
        let Some(pressed_link) = pressed_on_release.take() else {
            return;
        };
        if n_press != 1 || !enabled() || !modifier_held(gesture.current_event_state()) {
            return;
        }
        if link_at(&term_released, tag_released.get(), x, y).as_ref() != Some(&pressed_link) {
            return;
        }
        open_link(&term_released, &pressed_link);
    });
    // An OSC 8 hyperlink shows a label and leads to a target the remote side
    // chose; show the target on hover so the user sees where a click goes.
    // VTE only reports a hover URI while hyperlinks are allowed.
    terminal.connect_hyperlink_hover_uri_notify(|terminal| {
        terminal.set_tooltip_text(terminal.hyperlink_hover_uri().as_deref());
    });
    terminal.add_controller(click);
}

/// Registers `terminal`'s link regex when enabled and none is registered,
/// removes it when disabled.
///
// ponytail: a regex re-added after a toggle sits behind the highlight-rule
// regexes, and VTE reports the first registered match at a cell, so a URL
// containing a highlighted word opens only off that word until the session is
// reopened; re-register the highlight regexes after this one if users hit it.
fn sync_link_match(terminal: &Terminal, tag: &Cell<Option<i32>>, enabled: bool) {
    match (enabled, tag.get()) {
        (true, None) => match vte4::Regex::for_match(URL_PATTERN, PCRE2_MULTILINE) {
            Ok(regex) => {
                let added = terminal.match_add_regex(&regex, 0);
                terminal.match_set_cursor_name(added, "pointer");
                tag.set(Some(added));
            }
            Err(error) => tracing::warn!(%error, "link regex rejected by VTE"),
        },
        (false, Some(registered)) => {
            terminal.match_remove(registered);
            tag.set(None);
        }
        _ => {}
    }
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

/// A link a click resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Link {
    /// The text under the pointer is where the link leads: a printed URL, or
    /// an OSC 8 hyperlink whose label is its own target. Opens directly.
    Shown(String),
    /// An OSC 8 hyperlink whose target is not the text under the pointer.
    /// The remote side picks both the label and the target, so the target is
    /// shown for confirmation before it opens.
    Hidden(String),
}

impl Link {
    /// The URI the link opens.
    fn uri(&self) -> &str {
        match self {
            Self::Shown(uri) | Self::Hidden(uri) => uri,
        }
    }
}

/// The link under the point. `link_tag` is the terminal's link regex tag: a
/// match from any other regex (a highlight rule's) is not a link.
fn link_at(terminal: &Terminal, link_tag: Option<i32>, x: f64, y: f64) -> Option<Link> {
    let (printed, hit) = terminal.check_match_at(x, y);
    let printed = printed.filter(|_| link_tag == Some(hit));
    let target = terminal.check_hyperlink_at(x, y);
    resolve_link(printed.as_deref(), target.as_deref())
}

/// Decides what a click opens from the printed URL under the pointer and the
/// OSC 8 target there. An OSC 8 target wins, since it is what the browser
/// would get; it opens directly only when the printed text under the pointer
/// is that same URL. VTE does not expose a hyperlink's label span, so that
/// printed match stands in for the label. A printed URL loses trailing
/// sentence punctuation (`https://example.com/page).` opens `…/page`).
fn resolve_link(printed: Option<&str>, target: Option<&str>) -> Option<Link> {
    let printed = printed.map(trim_trailing_punctuation);
    match (printed, target) {
        (Some(printed), Some(target)) if printed == target => Some(Link::Shown(target.to_owned())),
        (_, Some(target)) => Some(Link::Hidden(target.to_owned())),
        (Some(printed), None) => Some(Link::Shown(printed.to_owned())),
        (None, None) => None,
    }
}

/// `url` without the punctuation a sentence puts after it. A closing
/// bracket stays when it closes one opened inside the URL
/// (`https://en.wikipedia.org/wiki/Rust_(programming_language)`).
fn trim_trailing_punctuation(url: &str) -> &str {
    let mut url = url;
    while let Some(last) = url.chars().last() {
        let unbalanced = |open: char| url.matches(open).count() < url.matches(last).count();
        let trailing = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' | '>' => true,
            ')' => unbalanced('('),
            ']' => unbalanced('['),
            '}' => unbalanced('{'),
            _ => false,
        };
        if !trailing {
            break;
        }
        url = &url[..url.len() - last.len_utf8()];
    }
    url
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
/// - everything off the web schemes — `ftp:`, `ssh:`, `javascript:`,
///   `data:`, `file:` — stays closed. The scheme compares case-blind, so
///   `HTTPS://` opens like `https://`.
fn is_openable_link(uri: &str) -> bool {
    if uri.contains('\\') || uri.chars().any(char::is_control) {
        return false;
    }
    glib::Uri::parse(uri, glib::UriFlags::NONE).is_ok_and(|u| {
        u.userinfo().is_none()
            && matches!(u.scheme().to_ascii_lowercase().as_str(), "http" | "https")
            && u.host().is_some_and(|h| !h.is_empty())
    })
}

/// Opens a web link in the browser, after the same checks every link gets;
/// anything else is refused. A hyperlink whose target is not what the
/// terminal shows asks first.
fn open_link(terminal: &Terminal, link: &Link) {
    let uri = link.uri();
    if !is_openable_link(uri) {
        // The URI stays out of the log: a refused link may carry userinfo,
        // a password included.
        tracing::warn!("refused to open a link that is not a plain http(s) URL");
        if let Some(window) = terminal.root().and_downcast::<gtk4::Window>() {
            crate::toast::show_toast_on_window(
                &window,
                &i18n("Link not opened: only plain http and https links can be opened"),
                crate::toast::ToastType::Warning,
            );
        }
        return;
    }
    match link {
        Link::Shown(uri) => launch_uri(terminal, uri),
        Link::Hidden(uri) => confirm_and_launch(terminal, uri),
    }
}

/// Shows the full target of a hyperlink and opens it on approval.
fn confirm_and_launch(terminal: &Terminal, uri: &str) {
    // The body is the URI itself, as plain text (markup stays off), so the
    // user reads exactly what the browser will get.
    let dialog = adw::AlertDialog::new(Some(&i18n("Open link?")), Some(uri));
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("open", &i18n("Open"));
    dialog.set_response_appearance("open", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let terminal_for_response = terminal.clone();
    let uri = uri.to_owned();
    dialog.connect_response(None, move |_, response| {
        if response == "open" {
            launch_uri(&terminal_for_response, &uri);
        }
    });
    // Modal to the terminal's own window, detached sessions included.
    dialog.present(Some(terminal));
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
        Link, PCRE2_MULTILINE, REGISTERED, URL_PATTERN, is_openable_link, link_at,
        link_click_gesture, modifier_held, resolve_link, set_enabled, setup_link_opening,
        trim_trailing_punctuation,
    };
    use gtk4::gdk;
    use gtk4::prelude::*;
    use vte4::Terminal;
    use vte4::prelude::*;

    #[test]
    fn only_web_links_open() {
        assert!(is_openable_link("https://example.com/a?b=1"));
        assert!(is_openable_link("http://example.com/"));
        assert!(is_openable_link("HTTPS://Example.com/"));
        assert!(!is_openable_link("ftp://example.com/pub/"));
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

    /// A printed URL and an OSC 8 hyperlink labelled with its own target open
    /// directly; an OSC 8 target that differs from the text under the pointer
    /// (`Docs` leading to another host) is shown for confirmation first.
    #[test]
    fn hidden_hyperlink_targets_ask_first() {
        let url = "https://example.com/a";
        assert_eq!(resolve_link(None, None), None);
        assert_eq!(
            resolve_link(Some(url), None),
            Some(Link::Shown(url.to_owned()))
        );
        assert_eq!(
            resolve_link(Some(url), Some(url)),
            Some(Link::Shown(url.to_owned()))
        );
        assert_eq!(
            resolve_link(None, Some("https://evil.example/")),
            Some(Link::Hidden("https://evil.example/".to_owned()))
        );
        // A label that looks like a URL but leads elsewhere: the target wins.
        assert_eq!(
            resolve_link(Some(url), Some("https://evil.example/")),
            Some(Link::Hidden("https://evil.example/".to_owned()))
        );
    }

    /// Sentence punctuation after a printed URL is not part of it; brackets
    /// the URL opened itself are.
    #[test]
    fn trailing_punctuation_is_trimmed() {
        assert_eq!(
            trim_trailing_punctuation("https://example.com/page)."),
            "https://example.com/page"
        );
        assert_eq!(
            trim_trailing_punctuation("https://example.com/a,"),
            "https://example.com/a"
        );
        assert_eq!(
            trim_trailing_punctuation("https://en.wikipedia.org/wiki/Rust_(language)"),
            "https://en.wikipedia.org/wiki/Rust_(language)"
        );
        assert_eq!(
            trim_trailing_punctuation("https://example.com/"),
            "https://example.com/"
        );
        assert_eq!(
            resolve_link(Some("https://example.com/page)."), None),
            Some(Link::Shown("https://example.com/page".to_owned()))
        );
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
        assert!(vte4::Regex::for_match(URL_PATTERN, PCRE2_MULTILINE).is_ok());
    }

    /// The link regex matches at the pixels of what was fed, a click inside
    /// the path part of a URL resolves to the whole URL, and turning
    /// **Allow hyperlinks** off and on removes and re-adds the regex. This is
    /// the check that catches a pattern VTE accepts but that matches nothing,
    /// which the compile check above cannot see. Cell size is not bound by
    /// `vte4` 0.10, so the sweep steps over the printed tokens' own pixels —
    /// hundreds of px wide at any font.
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
        let url = "https://example.com/rustconn";
        let terminal = Terminal::new();
        setup_link_opening(&terminal);
        set_enabled(&terminal, true);
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

        // The rightmost pixel that matches sits on the URL's last character,
        // inside its path part.
        let mut rightmost = None;
        for x in (0..600).step_by(2) {
            for y in (0..60).step_by(2) {
                let (x, y) = (f64::from(x), f64::from(y));
                if terminal.check_match_at(x, y).0.as_deref() == Some(url) {
                    rightmost = Some((x, y));
                }
            }
        }
        let Some((x, y)) = rightmost else {
            panic!("the URL never resolved");
        };
        let tag = REGISTERED.with(|registered| {
            registered
                .borrow()
                .iter()
                .find(|entry| entry.terminal.upgrade().as_ref() == Some(&terminal))
                .and_then(|entry| entry.tag.get())
        });
        assert!(tag.is_some(), "no link regex registered while enabled");
        assert_eq!(
            link_at(&terminal, tag, x, y),
            Some(Link::Shown(url.to_owned()))
        );

        set_enabled(&terminal, false);
        assert_eq!(
            terminal.check_match_at(x, y).0,
            None,
            "regex kept while off"
        );
        set_enabled(&terminal, true);
        assert_eq!(terminal.check_match_at(x, y).0.as_deref(), Some(url));
    }
}
