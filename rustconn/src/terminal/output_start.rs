//! Where the current connection's own output starts in a session's buffer.
//!
//! Status detection ("has the session printed anything past RustConn's connect
//! banner?") compares the cursor row with a base row. A fresh session starts on
//! an empty buffer and needs no base. A reconnect does: the previous session's
//! scrollback may be kept (issue #253), and the reconnect feeds its own
//! separator and banner before the new process starts.
//!
//! VTE processes fed text asynchronously, in the same queue as the child's
//! output, so the row below the banner cannot be read when the banner is fed.
//! It used to be captured lazily on the first read, which went wrong whenever
//! VTE processed the banner and the child's first output in one pass — a hidden
//! tab, or a child that answers at once: that output became the base, was never
//! counted, and a reconnect that printed everything in one chunk stayed on
//! "connecting". The base is now located by content instead: the reconnect
//! records the banner's last line, and the first read finds that line in the
//! buffer, whatever else landed in the same pass.
//!
//! The search is kept free of GTK and VTE types so it can be unit-tested; the
//! notebook reads the row texts and stores the result.

/// How many rows above the cursor are searched for the reconnect banner.
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
    /// The absolute VTE row is known.
    Row(i64),
    /// RustConn fed a connect banner ending in `last_line`; the row below it
    /// is located on the next read.
    AfterBanner { last_line: String },
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
    fn a_short_line_of_output_is_not_taken_for_the_banner() {
        let last = banner_last_line(BANNER);
        assert_eq!(row_below_banner(&rows(&["⚡", "", ""]), 0, &last), None);
        assert_eq!(row_below_banner(&rows(&["x", "", ""]), 0, ""), None);
    }
}
