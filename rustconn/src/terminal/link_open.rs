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
use std::path::{Path, PathBuf};

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

/// Filesystem paths a click may open, next to the web URLs: tilde and
/// absolute POSIX paths, `file://` URIs, and Windows drive paths — the last
/// for the day RustConn runs on Windows, where GIO knows its own syntax. The
/// run stops at whitespace, quotes and shell separators, so `a && /bin/ls`
/// offers `/bin/ls` alone; relative `./…` is not matched at all — the
/// terminal's own working directory is not reliably known.
const POSIX_FILE_PATTERN: &str = r#"(?:~/|file://|/)[^\s"'&|;<>`$\\]+"#;

/// Windows drive paths. Parens are legal in Windows directory names, so only
/// the wildcard and environment characters stop the run.
const WINDOWS_FILE_PATTERN: &str = r#"[A-Za-z]:\\[^\s"'&|;<>`$*?]+"#;

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
    for pattern in [URL_PATTERN, POSIX_FILE_PATTERN, WINDOWS_FILE_PATTERN] {
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
    let click = gtk4::GestureClick::new();
    // Capture phase, and a claim on the press: VTE's own click controller
    // starts a selection on button 1 and would own the sequence, cancelling
    // this gesture before `released` — the plain-click case stays unclaimed,
    // so VTE keeps its selection handling (the split-view selection bug).
    click.set_propagation_phase(gtk4::PropagationPhase::Capture);
    click.connect_pressed(move |gesture, _, x, y| {
        let state = gesture.current_event_state();
        if !enabled() || !modifier_held(state) || link_at(&term_pressed, x, y).is_none() {
            return;
        }
        tracing::debug!(x, y, "link press claimed");
        gesture.set_state(gtk4::EventSequenceState::Claimed);
    });
    click.connect_released(move |gesture, n_press, x, y| {
        if n_press != 1 || !enabled() || !modifier_held(gesture.current_event_state()) {
            return;
        }
        let Some(uri) = link_at(&term_released, x, y) else {
            return;
        };
        open_link(&term_released, &uri);
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
///   `file:` — stays closed. `file:` links open through [`open_path`]'s checks
///   instead.
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

/// Hands whatever the click resolved to the right launcher: web schemes to
/// the browser, everything else — a path or a `file://` URI — to the file
/// checks.
fn open_link(terminal: &Terminal, text: &str) {
    if text.starts_with("http://") || text.starts_with("https://") || text.starts_with("ftp://") {
        if !is_openable_link(text) {
            tracing::warn!(%text, "refused to open a non-web link");
            return;
        }
        launch_uri(terminal, text);
    } else {
        open_path(terminal, text);
    }
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

/// Whether the printed token names a local path form at all: tilde, absolute
/// POSIX, `file://` or a Windows drive. Relative paths stay closed.
fn is_absolute_form(text: &str) -> bool {
    let bytes = text.as_bytes();
    text.starts_with('/')
        || text.starts_with('~')
        || text.starts_with("file://")
        || (bytes.first().is_some_and(u8::is_ascii_alphabetic)
            && bytes.get(1) == Some(&b':')
            && bytes.get(2) == Some(&b'\\'))
}

/// The local path a printed token names, canonicalised: tilde expanded, a
/// `file://` URI percent-decoded after its authority. Anything the filesystem
/// cannot name is `None`.
fn local_path(text: &str) -> Option<PathBuf> {
    if !is_absolute_form(text) {
        return None;
    }
    let printed = if let Some(rest) = text.strip_prefix("file://") {
        // Skip the authority (`file://localhost/…`), then decode the path.
        let encoded = &rest[rest.find('/')?..];
        glib::uri_unescape_segment(Some(encoded), None::<&str>, None::<&str>)?.to_string()
    } else {
        shellexpand::tilde(text).into_owned()
    };
    // Windows URIs keep a leading slash before the drive (`file:///C:/…`);
    // the filesystem API wants `C:/…`.
    let bytes = printed.as_bytes();
    let printed = if bytes.len() > 2
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b':'
    {
        printed[1..].to_string()
    } else {
        printed
    };
    std::fs::canonicalize(printed).ok()
}

/// Executable extensions — refused on every platform, since Windows has no
/// execute bit to judge by. The list is deny-by-name, not exhaustive; on unix
/// the execute bit catches everything else.
const EXECUTABLE_EXTENSIONS: &[&str] = &[
    "exe", "bat", "cmd", "com", "ps1", "msi", "scr", "vbs", "hta", "cpl", "jar", "lnk",
];

/// Whether opening `path` could run local code: an executable extension
/// anywhere, or the unix execute bit.
fn could_execute(path: &Path, meta: &std::fs::Metadata) -> bool {
    if path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|e| EXECUTABLE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
    {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 != 0 {
            return true;
        }
    }
    false
}

/// The paths a click may hand to the system: existing files and directories,
/// never anything that executes. The remote side controls what the terminal
/// prints, so a click must never run local code — a `run` on a printed
/// `/tmp/x.sh` would be a remote-to-local execution channel.
fn openable_path(path: &Path) -> Option<PathBuf> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.is_dir() {
        return Some(path.to_path_buf()); // a directory opens in the file manager
    }
    if !meta.is_file() || could_execute(path, &meta) {
        return None; // devices, fifos, sockets — a viewer hangs on /dev/random
    }
    Some(path.to_path_buf())
}

/// Hands a resolved, checked path to the system's default handler — the file
/// manager for a directory, the associated viewer for everything else.
fn open_path(terminal: &Terminal, text: &str) {
    let Some(path) = local_path(text).as_deref().and_then(openable_path) else {
        tracing::warn!(%text, "refused to open a filesystem path");
        return;
    };
    let window = terminal.root().and_downcast::<gtk4::Window>();
    gtk4::FileLauncher::new(Some(&gio::File::for_path(path))).launch(
        window.as_ref(),
        gio::Cancellable::NONE,
        |result| {
            if let Err(error) = result
                && !error.matches(gtk4::DialogError::Dismissed)
            {
                tracing::warn!(%error, "could not open the file");
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::{
        PCRE2_MULTILINE, POSIX_FILE_PATTERN, URL_PATTERN, WINDOWS_FILE_PATTERN, is_openable_link,
        local_path, modifier_held, openable_path, setup_link_opening,
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

    /// Only existing files and directories pass the checks; anything that
    /// could execute, anything the filesystem cannot name, is refused.
    #[test]
    fn only_existing_non_executable_paths_open() {
        let dir = tempfile::tempdir().expect("tempdir");
        let plain = dir.path().join("notes.txt");
        std::fs::write(&plain, b"hi").expect("write");
        assert!(openable_path(&plain).is_some());

        // The unix execute bit makes any name refused.
        let scripted = dir.path().join("run.sh");
        std::fs::write(&scripted, b"#!/bin/sh\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&scripted, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }
        assert!(openable_path(&scripted).is_none());

        // And an executable extension alone suffices, execute bit or not.
        let binned = dir.path().join("payload.exe");
        std::fs::write(&binned, b"hi").expect("write");
        assert!(openable_path(&binned).is_none());

        assert!(openable_path(dir.path()).is_some()); // a directory opens
        assert!(openable_path(&dir.path().join("missing.txt")).is_none());
    }

    /// Relative paths, schemes and non-path text never resolve; `file://`
    /// URIs decode and name the same file the plain path does.
    #[test]
    fn local_paths_resolve_tilde_file_uris_and_nothing_else() {
        assert!(local_path("./relative.txt").is_none());
        assert!(local_path("mailto:someone@example.com").is_none());

        let dir = tempfile::tempdir().expect("tempdir");
        let plain = dir.path().join("notes.txt");
        std::fs::write(&plain, b"hi").expect("write");
        let expected = std::fs::canonicalize(&plain).expect("canonicalize");

        assert_eq!(
            local_path(plain.to_str().expect("utf8")),
            Some(expected.clone())
        );
        let percent_encoded = dir.path().join("my file.txt");
        std::fs::write(&percent_encoded, b"hi").expect("write");
        let encoded = dir.path().to_str().expect("utf8").replace(' ', "%20");
        assert_eq!(
            local_path(&format!("file://{encoded}/my%20file.txt")),
            std::fs::canonicalize(&percent_encoded).ok()
        );
        assert_eq!(local_path("file:///nonexistent/zz"), None);
        assert!(local_path("~/").is_some());
        assert_eq!(local_path(plain.to_str().expect("utf8")), Some(expected));
    }

    /// Ctrl opens (also on macOS, where ⌘ does — see `modifier_held`); nothing
    /// else does, on any platform this compiles for.
    #[test]
    fn the_open_modifier_is_ctrl_off_macos() {
        assert!(modifier_held(gdk::ModifierType::CONTROL_MASK));
        assert!(!modifier_held(gdk::ModifierType::empty()));
        assert!(!modifier_held(gdk::ModifierType::SHIFT_MASK));
    }

    /// VTE must accept the patterns — a rejected regex means no link ever
    /// underlines or opens, and nothing else in the module would notice.
    #[test]
    fn the_link_regexes_compile() {
        for pattern in [URL_PATTERN, POSIX_FILE_PATTERN, WINDOWS_FILE_PATTERN] {
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
        terminal.feed(b"xx https://example.com/rustconn yy /etc/hostname\r\n");
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
        assert!(
            found.contains("/etc/hostname"),
            "the path never resolved; found {found:?}"
        );
    }
}
