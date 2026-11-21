//! Contract tests for the VTE behaviours the PTY relay is built on (#247).
//!
//! The transcript has been rewritten three times because each attempt reasoned
//! about how VTE addresses rows and delivers input instead of checking. These
//! tests check. They need a display and initialise GTK, which can only happen
//! once per process, so they are opt-in:
//!
//! ```text
//! cargo test -p rustconn --bin rustconn -- --ignored --exact \
//!     terminal::vte_contract_tests::<name>
//! ```
//!
//! Moved out of `terminal/mod.rs`; behaviour is unchanged.

use vte4::TerminalExt;

const OPT_IN: &str = "initialises GTK: needs a display and its own process";

/// Pumps the GLib main context until `ready` holds, or the deadline passes.
///
/// VTE parses fed bytes from a queued source rather than inside `feed`, so
/// nothing is observable until the loop runs.
fn pump_until(ready: impl Fn() -> bool) -> bool {
    let ctx = gtk4::glib::MainContext::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !ready() && std::time::Instant::now() < deadline {
        ctx.iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    ready()
}

/// `commit` carries keyboard and pasted input even with no PTY attached.
///
/// This is what lets RustConn own the master file descriptor: VTE renders
/// and handles keys, and every byte it would have written to a PTY of its
/// own arrives here instead. VTE guarantees it explicitly — `send_child`
/// emits `commit` before checking for a PTY, for
/// [vte#222](https://gitlab.gnome.org/GNOME/vte/-/issues/222) — and the
/// whole relay collapses without it, so it is pinned here.
#[test]
#[ignore = "initialises GTK: needs a display and its own process"]
fn commit_fires_without_a_pty() {
    if gtk4::init().is_err() {
        return;
    }
    let terminal = super::Terminal::new();
    assert!(terminal.pty().is_none(), "no PTY is the point of this test");

    let seen = std::rc::Rc::new(std::cell::RefCell::new(String::new()));
    let seen_in_handler = seen.clone();
    terminal.connect_commit(move |_t, text, _size| {
        seen_in_handler.borrow_mut().push_str(text);
    });

    terminal.feed_child(b"whoami\r");
    assert!(
        pump_until(|| !seen.borrow().is_empty()),
        "commit must fire without a PTY, or the relay cannot receive input ({OPT_IN})"
    );
    assert_eq!(
        *seen.borrow(),
        "whoami\r",
        "commit must carry the bytes verbatim"
    );
}

/// Neither erase binding may be left for VTE to resolve on its own.
///
/// Owning the PTY means VTE has no termios to read `VERASE` from, and
/// vte 0.84 does not treat that as a reason to stop: `map_erase_binding()`
/// reaches `assert(auto_mode != eTTY)` and aborts the process, so one
/// Backspace press was enough to take the window down. A configured
/// terminal must therefore name both bindings — see
/// [`super::config::apply_erase_bindings`].
#[test]
#[ignore = "initialises GTK: needs a display and its own process"]
fn erase_bindings_are_never_left_to_vte() {
    if gtk4::init().is_err() {
        return;
    }
    let terminal = super::Terminal::new();
    assert!(terminal.pty().is_none(), "no PTY is the point of this test");

    super::config::configure_terminal_with_settings(
        &terminal,
        &rustconn_core::config::TerminalSettings::default(),
    );

    for (key, binding) in [
        ("Backspace", terminal.backspace_binding()),
        ("Delete", terminal.delete_binding()),
    ] {
        assert!(
            !matches!(binding, vte4::EraseBinding::Auto | vte4::EraseBinding::Tty),
            "{key} is left as {binding:?}: with no PTY, vte 0.84 aborts \
             while resolving that ({OPT_IN})"
        );
    }
}

/// A per-connection erase mode may not reintroduce the abort either.
///
/// `Automatic` is the tempting place to hand the decision back to VTE, and
/// the option exists for hosts whose users press Backspace constantly, so
/// every combination offered by the connection editor (issue
/// [#271](https://github.com/totoshko88/RustConn/issues/271)) is checked
/// against the same rule as the defaults above.
#[test]
#[ignore = "initialises GTK: needs a display and its own process"]
fn configured_erase_modes_are_never_left_to_vte() {
    use rustconn_core::models::{BackspaceSends, DeleteSends};

    if gtk4::init().is_err() {
        return;
    }
    let terminal = super::Terminal::new();
    assert!(terminal.pty().is_none(), "no PTY is the point of this test");

    for backspace in BackspaceSends::all() {
        for delete in DeleteSends::all() {
            super::config::apply_erase_mode(&terminal, *backspace, *delete);

            for (key, binding) in [
                ("Backspace", terminal.backspace_binding()),
                ("Delete", terminal.delete_binding()),
            ] {
                assert!(
                    !matches!(binding, vte4::EraseBinding::Auto | vte4::EraseBinding::Tty),
                    "{backspace:?}/{delete:?} leaves {key} as {binding:?}: \
                     with no PTY, vte 0.84 aborts while resolving that ({OPT_IN})"
                );
            }
        }
    }
}

/// No VTE signal announces a geometry change, which is why the relay polls.
///
/// Owning the PTY means RustConn has to tell the child when the window
/// changes size, and there is no event to hang that on: VTE exposes no
/// row-count or column-count signal, `char-size-changed` covers font
/// metrics only, GTK4 removed `GtkWidget::size-allocate`, and
/// `contents-changed` — the obvious candidate, already used elsewhere in
/// this file — does not fire for a resize, as asserted below. So
/// [`super::pty_relay::PtyRelay`] compares the grid size on a timer
/// instead. Should a future VTE grow a suitable signal, this test starts
/// failing and the poll can go.
#[test]
#[ignore = "initialises GTK: needs a display and its own process"]
fn geometry_change_raises_no_contents_changed() {
    if gtk4::init().is_err() {
        return;
    }
    let terminal = super::Terminal::new();
    terminal.set_size(80, 24);
    terminal.feed(b"anchor\r\n");
    assert!(pump_until(|| terminal.cursor_position().1 > 0), "{OPT_IN}");

    let fired = std::rc::Rc::new(std::cell::Cell::new(false));
    let fired_in_handler = fired.clone();
    terminal.connect_contents_changed(move |_t| fired_in_handler.set(true));

    terminal.set_size(100, 30);
    // Give it every chance to arrive before concluding that it does not.
    pump_until(|| fired.get());
    assert!(
        !fired.get(),
        "contents-changed now fires on a resize: the relay's winsize poll \
         can be replaced by this signal"
    );
    assert_eq!(
        (terminal.column_count(), terminal.row_count()),
        (100, 30),
        "the grid itself did change, so polling it detects the resize"
    );
}

/// A scroll position between two rows stays where it was put.
///
/// Touchpad scrolling and a drag on the scrollbar leave the view part-way
/// through a row, and the highlight overlay draws its rows at the same
/// sub-row offset VTE does (issue #343). That offset exists only because
/// VTE keeps the fraction instead of snapping the adjustment to a whole row,
/// so it is pinned here: should a future VTE start rounding, this fails and
/// the overlay's offset is always zero.
#[test]
#[ignore = "initialises GTK: needs a display and its own process"]
fn fractional_scroll_position_is_kept() {
    use gtk4::prelude::{AdjustmentExt, GtkWindowExt, ScrollableExt};

    if gtk4::init().is_err() {
        return;
    }
    let terminal = super::Terminal::new();
    // VTE sizes its vertical adjustment (upper, page_size) in size_allocate,
    // so a terminal never placed in a mapped window reports 0 for
    // both (measured on VTE 0.84.1). Give it a real allocation first.
    let window = gtk4::Window::new();
    window.set_child(Some(&terminal));
    window.present();
    terminal.set_size(80, 10);
    for line in 0..40 {
        terminal.feed(format!("line {line}\r\n").as_bytes());
    }
    let adjustment = terminal
        .vadjustment()
        .expect("VTE creates its own vertical adjustment");
    assert!(
        pump_until(|| adjustment.upper() - adjustment.page_size() >= 20.0),
        "forty lines in ten rows leave scrollback to scroll through ({OPT_IN})"
    );

    adjustment.set_value(12.5);
    // Give VTE every chance to snap it before concluding that it does not.
    pump_until(|| (adjustment.value() - 12.5).abs() > f64::EPSILON);
    assert!(
        (adjustment.value() - 12.5).abs() <= f64::EPSILON,
        "VTE moved the scroll position from 12.5 to {}",
        adjustment.value()
    );
}

/// `cursor_position` and `text_range_format` address the same rows.
///
/// Kept from the row-anchored transcript that the relay replaced: prompt
/// detection (`cursor_line_text`) still mixes the two, and issue
/// [#253](https://github.com/totoshko88/RustConn/issues/253) came from
/// getting this wrong.
#[test]
#[ignore = "initialises GTK: needs a display and its own process"]
fn cursor_row_and_text_range_share_coordinates() {
    if gtk4::init().is_err() {
        return;
    }
    let terminal = super::Terminal::new();
    terminal.set_size(80, 24);
    terminal.feed(b"alpha\r\nbravo\r\ncharlie\r\ndelta\r\n");
    assert!(pump_until(|| terminal.cursor_position().1 > 0), "{OPT_IN}");

    assert_eq!(
        terminal.cursor_position().1,
        4,
        "four newline-terminated lines leave the cursor on row 4"
    );

    let (first, _) = terminal.text_range_format(vte4::Format::Text, 0, 0, 0, 80);
    assert_eq!(
        first.map(|g| g.to_string().trim_end().to_owned()),
        Some("alpha".to_owned()),
        "row 0 is the first line fed, so both APIs count rows the same way"
    );

    let (bounded, _) = terminal.text_range_format(vte4::Format::Text, 1, 0, 2, 80);
    let bounded = bounded.map(|g| g.to_string()).unwrap_or_default();
    assert!(
        bounded.contains("bravo") && bounded.contains("charlie"),
        "requested rows must be present: {bounded:?}"
    );
    assert!(
        !bounded.contains("delta"),
        "the end row must bound the range: {bounded:?}"
    );
}

/// Widening the terminal renumbers every row below a wrapped line.
///
/// This is why the transcript is no longer scraped from the widget at all.
/// A session's terminal is resized when the widget receives its real
/// allocation — around the first capture of a session — and again whenever
/// the user resizes the window. VTE rewraps, a wrapped logical line stops
/// occupying two rows, and everything below it moves: an absolute row
/// recorded before the change points at different text afterwards, which
/// duplicated some lines and skipped others (issue #247).
#[test]
#[ignore = "initialises GTK: needs a display and its own process"]
fn widening_renumbers_rows_below_a_wrapped_line() {
    if gtk4::init().is_err() {
        return;
    }
    let terminal = super::Terminal::new();
    terminal.set_size(40, 10);
    terminal.feed(b"short\r\n");
    terminal.feed("wrapped-".repeat(9).as_bytes()); // 72 chars: two rows at 40
    terminal.feed(b"\r\n");
    assert!(pump_until(|| terminal.cursor_position().1 >= 3), "{OPT_IN}");

    let before_cursor = terminal.cursor_position().1;
    let (before_row0, _) = terminal.text_range_format(vte4::Format::Text, 0, 0, 0, 40);

    terminal.set_size(100, 10);
    assert!(
        pump_until(|| terminal.cursor_position().1 < before_cursor),
        "{OPT_IN}"
    );

    let (after_row0, _) = terminal.text_range_format(vte4::Format::Text, 0, 0, 0, 100);
    assert_eq!(
        before_row0.map(|g| g.to_string().trim_end().to_owned()),
        after_row0.map(|g| g.to_string().trim_end().to_owned()),
        "a row above the wrap point keeps its content"
    );
    assert_eq!(
        (before_cursor, terminal.cursor_position().1),
        (3, 2),
        "the wrapped line collapses to one row and pulls the rest upwards"
    );
}

const MOSH_BANNER: &str = "🔗 Connecting via MOSH to web...\r\n⚡ Executing: mosh user@web\r\n\r\n";

/// What mosh-client does when it starts: switch to the alternate screen,
/// clear it, home the cursor and draw the remote screen.
const MOSH_DRAW: &[u8] = "\x1b[?1049h\x1b[2J\x1b[HLast login: today\r\nuser@web:~ ❯ ".as_bytes();

/// A terminal with 40 rows of kept history in 24 visible rows, as after a
/// reconnect with "keep history" on, and the start a connect banner
/// records before VTE has processed it.
fn terminal_with_history() -> (super::Terminal, Option<super::output_start::OutputStart>) {
    let terminal = super::Terminal::new();
    terminal.set_size(80, 24);
    for line in 0..40 {
        terminal.feed(format!("old session output {line}\r\n").as_bytes());
    }
    assert!(
        pump_until(|| terminal.cursor_position().1 >= 40),
        "{OPT_IN}"
    );
    let start = Some(super::output_start::OutputStart::AfterBanner {
        last_line: super::output_start::banner_last_line(MOSH_BANNER),
        fed_at_row: terminal.cursor_position().1,
    });
    (terminal, start)
}

/// The alternate-screen route of the status verdict, against a real VTE.
///
/// Once the banner has been processed in a pass of its own, a client's
/// switch to the alternate screen keeps the buffer's row numbering: VTE
/// (0.8x, measured with 40 rows of kept history) puts the alternate
/// screen's rows below the start row, so the rows from the start to the
/// cursor hold what the client drew and the row-based verdict reads it.
/// `ConnectionOutput::screen_replaced`, for a cursor above the start row,
/// does not arise on this route.
#[test]
#[ignore = "initialises GTK: needs a display and its own process"]
fn a_client_on_the_alternate_screen_is_read_below_the_start_row() {
    if gtk4::init().is_err() {
        return;
    }
    let (terminal, mut start) = terminal_with_history();
    let base = terminal.cursor_position().1 + 3;
    terminal.feed(MOSH_BANNER.as_bytes());
    assert!(
        pump_until(|| terminal.cursor_position().1 >= base),
        "{OPT_IN}"
    );

    let (output, resolved) = super::read_connection_output(&terminal, start.as_ref());
    let output = output.expect("the banner alone resolves the start");
    assert_eq!((output.row, output.screen_replaced), (0, false));
    assert!(resolved.is_some(), "the start is resolved once");
    start = resolved;

    terminal.feed(MOSH_DRAW);
    assert!(pump_until(|| terminal.cursor_position().0 > 0), "{OPT_IN}");
    let (output, _) = super::read_connection_output(&terminal, start.as_ref());
    assert!(
        output.as_ref().is_some_and(|o| !o.screen_replaced
            && o.row > 0
            && o.text.contains("user@web:~ ❯")
            && !o.text.contains("old session output")),
        "cursor {:?}, start row {base}, output {output:?}",
        terminal.cursor_position(),
    );
}

/// The known limit of that route: when VTE processes the banner and the
/// switch to the alternate screen in one pass, the banner is never seen
/// on the normal screen, the start is never resolved, and the detectors
/// stay undecided (the session stays on "connecting"). Documented in the
/// CHANGELOG; this pins it so a change in VTE or in the search shows up.
#[test]
#[ignore = "initialises GTK: needs a display and its own process"]
fn a_banner_and_the_alternate_screen_in_one_pass_stay_undecided() {
    if gtk4::init().is_err() {
        return;
    }
    let (terminal, start) = terminal_with_history();
    let fed_at_row = terminal.cursor_position().1;
    terminal.feed(MOSH_BANNER.as_bytes());
    terminal.feed(MOSH_DRAW);
    assert!(pump_until(|| terminal.cursor_position().0 > 0), "{OPT_IN}");
    let (output, resolved) = super::read_connection_output(&terminal, start.as_ref());
    assert_eq!(
        (&output, &resolved),
        (&None, &None),
        "cursor {:?}, fed at {fed_at_row}",
        terminal.cursor_position(),
    );
}
