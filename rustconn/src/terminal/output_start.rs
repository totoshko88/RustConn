//! Where the current connection's own output starts in a session's buffer.
//!
//! Status detection ("has the session printed anything past RustConn's connect
//! banner?") compares the cursor row with a base row: the row below the banner
//! RustConn prints before it starts the client. A fresh connect needs it so
//! that the banner itself is not taken for the client's output; a reconnect
//! also has the previous session's scrollback above it (issue #253) and a
//! separator.
//!
//! VTE processes fed text asynchronously, in the same queue as the child's
//! output, so the row below the banner cannot be read when the banner is fed.
//! It used to be captured lazily on the first read, which went wrong whenever
//! VTE processed the banner and the child's first output in one pass — a hidden
//! tab, or a child that answers at once: that output became the base, was never
//! counted, and a reconnect that printed everything in one chunk stayed on
//! "connecting". The base is now located by content instead: the connect
//! records the banner's last line, and the first read finds that line in the
//! buffer, whatever else landed in the same pass.
//!
//! The search is kept free of GTK and VTE types so it can be unit-tested; the
//! notebook reads the row texts and stores the result.

/// How many rows above the cursor are searched for the connect banner.
///
/// A reconnect's first processing pass normally holds the banner and at most a
/// few lines of output. If the banner is further up than this, the new process
/// has already printed a screenful, which is evidence enough of output.
pub(super) const BANNER_SEARCH_ROWS: i64 = 200;

/// Minimum length of a row that may be taken for the start of the banner line.
///
/// Every banner line starts with the command marker (`⚡ Executing: `, 13
/// characters), so a row that long which is also a prefix of the line is the
/// banner, not a coincidental short line of output.
const BANNER_MATCH_MIN_CHARS: usize = 12;

/// The recorded start of a session's current connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum OutputStart {
    /// The absolute VTE row is known, with no banner above it to look for.
    Row(i64),
    /// The absolute VTE row is known: the row below the connect banner whose
    /// last line is `last_line`.
    BelowBanner { row: i64, last_line: String },
    /// RustConn fed a connect banner ending in `last_line` while the cursor was
    /// on `fed_at_row`; the row below the banner is located on the next read.
    AfterBanner { last_line: String, fed_at_row: i64 },
}

/// What the session's current process has shown since its connect banner, as
/// the connection status detectors read it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConnectionOutput {
    /// How many rows below the first row of the process's output the cursor
    /// is; 0 while the process has printed no full line.
    pub row: i64,
    /// The text from the first row of the process's output to the cursor.
    pub text: String,
    /// The client has taken over the screen: the cursor is above the first
    /// row of its output and the connect banner is no longer shown above the
    /// cursor. A switch to the alternate screen (mosh-client, a full-screen
    /// program started on login) does this; rows of that screen cannot be
    /// compared with the base row, so `row` and `text` say nothing then.
    pub screen_replaced: bool,
}

/// The base to count from when the banner was not found above the cursor.
///
/// Only once the cursor is [`BANNER_SEARCH_ROWS`] below `fed_at_row`, the row
/// the banner was fed on, has the new process printed a screenful past the
/// search window; the window's top row is then below the banner. Before that,
/// the banner has simply not been processed yet, and taking the window top
/// would pin the base inside the kept scrollback, so this returns `None`.
#[must_use]
pub(super) fn base_without_banner(fed_at_row: i64, cursor_row: i64) -> Option<i64> {
    (cursor_row - fed_at_row >= BANNER_SEARCH_ROWS).then_some(cursor_row - BANNER_SEARCH_ROWS)
}

/// The first buffer row searched for the banner.
///
/// The search covers at most [`BANNER_SEARCH_ROWS`] rows above the cursor and
/// never starts above `fed_at_row`, the row the banner was fed on. Everything
/// above that row is the kept scrollback, so an identical banner left there by
/// an earlier reconnect cannot be matched before the new one is processed.
#[must_use]
pub(super) fn search_first_row(fed_at_row: i64, cursor_row: i64) -> i64 {
    (cursor_row - BANNER_SEARCH_ROWS).max(fed_at_row).max(0)
}

/// The last non-blank line of a banner as fed to the terminal.
///
/// The banner is `"{connecting}\r\n{command}\r\n\r\n"`, so this is the command
/// line, without the carriage returns VTE never shows.
#[must_use]
pub(super) fn banner_last_line(banner: &str) -> String {
    banner
        .lines()
        .map(str::trim_end)
        .rfind(|line| !line.is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// Whether a row's text is where `last_line` starts.
///
/// A banner line longer than the terminal is wide wraps; its first row is then a
/// prefix of the line rather than the whole of it.
fn is_banner_start(row_text: &str, last_line: &str) -> bool {
    let text = row_text.trim_end();
    !text.is_empty()
        && last_line.starts_with(text)
        && text.chars().count() >= last_line.chars().count().min(BANNER_MATCH_MIN_CHARS)
}

/// Whether `text`, the rows above the cursor as one string (a wrapped line
/// joined into one), still holds the banner line `last_line`.
///
/// VTE has no API that says which screen is active (`vte4` binds none up to
/// its `v0_84` tier). The alternate screen has a buffer and row numbering of
/// its own (`Terminal::switch_screen` in VTE's `vteseq.cc`), and
/// `cursor_position()` and `text_range_format()` read the active screen, so a
/// banner that vanished from above the cursor is the evidence that the client
/// switched to it. On the normal screen the banner stays in the buffer, even
/// after a resize rewrapped it into other rows.
#[must_use]
pub(super) fn shows_banner(text: &str, last_line: &str) -> bool {
    text.lines().any(|line| is_banner_start(line, last_line))
}

/// Finds the row just below the most recent banner.
///
/// `rows[i]` is the text of buffer row `first_row + i`, and the last entry is
/// the cursor row. The banner's last line is followed by one blank row (the
/// banner ends in `\r\n\r\n`), and the new process's output starts below that.
/// The banner nearest the cursor wins, so a previous session's banner kept in
/// the scrollback is never matched.
///
/// Returns `None` while the banner has not fully landed.
#[must_use]
pub(super) fn row_below_banner(rows: &[String], first_row: i64, last_line: &str) -> Option<i64> {
    if last_line.is_empty() {
        return None;
    }
    let start = rows
        .iter()
        .rposition(|row| is_banner_start(row, last_line))?;
    // The blank row closing the banner. It cannot be the cursor row: the
    // banner's final line break puts the cursor on the row below it.
    let blank = rows
        .iter()
        .enumerate()
        .skip(start + 1)
        .find(|(_, row)| row.trim().is_empty())
        .map(|(index, _)| index)?;
    if blank + 1 >= rows.len() {
        return None;
    }
    let offset = i64::try_from(blank + 1).ok()?;
    Some(first_row + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BANNER: &str = "🔗 Connecting via AWS SSM to web...\r\n⚡ Executing: aws ssm start-session --target i-1\r\n\r\n";

    fn rows(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|line| (*line).to_owned()).collect()
    }

    #[test]
    fn last_line_is_the_command_line() {
        assert_eq!(
            banner_last_line(BANNER),
            "⚡ Executing: aws ssm start-session --target i-1"
        );
        assert_eq!(banner_last_line("\r\n\r\n"), "");
    }

    #[test]
    fn search_never_starts_in_the_kept_scrollback() {
        // The new banner was fed on row 500 but not processed yet; an older,
        // identical banner sits on row 450. The search must start at 500.
        assert_eq!(search_first_row(500, 500), 500);
        assert_eq!(search_first_row(500, 520), 500);
        // Far below the banner the window bound takes over again.
        assert_eq!(search_first_row(500, 900), 700);
        // A fresh buffer never goes negative.
        assert_eq!(search_first_row(0, 3), 0);
    }

    #[test]
    fn banner_alone_puts_the_base_on_the_cursor_row() {
        let last = banner_last_line(BANNER);
        let buffer = rows(&[
            "🔗 Connecting via AWS SSM to web...",
            "⚡ Executing: aws ssm start-session --target i-1",
            "",
            "",
        ]);
        assert_eq!(row_below_banner(&buffer, 10, &last), Some(13));
    }

    #[test]
    fn output_processed_with_the_banner_is_below_the_base() {
        // The case the lazy capture lost: banner and the whole reply in one pass.
        let last = banner_last_line(BANNER);
        let buffer = rows(&[
            "⚡ Executing: aws ssm start-session --target i-1",
            "",
            "",
            "Starting session with SessionId: me-0123",
            "sh-5.2$ ",
        ]);
        assert_eq!(row_below_banner(&buffer, 0, &last), Some(2));
    }

    #[test]
    fn a_wrapped_banner_line_is_found_by_its_first_row() {
        let last = banner_last_line(BANNER);
        let buffer = rows(&[
            "⚡ Executing: aws ssm start-se",
            "ssion --target i-1",
            "",
            "",
        ]);
        assert_eq!(row_below_banner(&buffer, 5, &last), Some(8));
    }

    #[test]
    fn the_newest_banner_wins_over_one_kept_in_the_scrollback() {
        let last = banner_last_line(BANNER);
        let buffer = rows(&[
            "⚡ Executing: aws ssm start-session --target i-1",
            "",
            "old session output",
            "── Reconnected at 2026-01-01 00:00:00 ──",
            "⚡ Executing: aws ssm start-session --target i-1",
            "",
            "",
        ]);
        assert_eq!(row_below_banner(&buffer, 0, &last), Some(6));
    }

    #[test]
    fn a_banner_that_has_not_fully_landed_is_not_resolved() {
        let last = banner_last_line(BANNER);
        assert_eq!(row_below_banner(&rows(&["old output", ""]), 0, &last), None);
        assert_eq!(
            row_below_banner(
                &rows(&["⚡ Executing: aws ssm start-session --target i-1", ""]),
                0,
                &last
            ),
            None
        );
    }

    #[test]
    fn a_banner_not_processed_yet_never_falls_back_into_the_scrollback() {
        // 500 rows of kept history; the banner was fed with the cursor on row
        // 500 and VTE has not processed it yet.
        assert_eq!(base_without_banner(500, 500), None);
        assert_eq!(base_without_banner(500, 699), None);
        // A screenful printed since the banner: the window top is below it.
        assert_eq!(base_without_banner(500, 700), Some(500));
        assert_eq!(base_without_banner(500, 900), Some(700));
    }

    #[test]
    fn the_normal_screen_still_shows_the_banner() {
        let last = banner_last_line(BANNER);
        // A resize may rewrap the rows, and VTE joins a wrapped line in one.
        let normal = "old output\n⚡ Executing: aws ssm start-session --target i-1\n\nuser@web:~$ ";
        assert!(shows_banner(normal, &last));
    }

    #[test]
    fn the_alternate_screen_does_not_show_the_banner() {
        let last = banner_last_line(BANNER);
        // What mosh-client draws: the remote screen, and nothing of RustConn's.
        let alternate = "Last login: Mon Nov  3 10:00:00 2026\nuser@web:~ ❯ ";
        assert!(!shows_banner(alternate, &last));
        assert!(!shows_banner("", &last));
    }

    #[test]
    fn a_short_line_of_output_is_not_taken_for_the_banner() {
        let last = banner_last_line(BANNER);
        assert_eq!(row_below_banner(&rows(&["⚡", "", ""]), 0, &last), None);
        assert_eq!(row_below_banner(&rows(&["x", "", ""]), 0, ""), None);
    }
}
