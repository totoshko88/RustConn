//! Tracks where shell prompts are in a session's scrollback.
//!
//! The remote shell, when it has OSC 133 shell integration sourced (`vte.sh` or
//! equivalent), marks each prompt and the end of each command it runs. VTE ≥ 0.78
//! surfaces the command-end exit code as the `vte.shell.postexec` termprop (see
//! [`super::termprops`]); the prompt's screen position is captured by the GTK
//! wiring at the moment the boundary fires and recorded here.
//!
//! This registry is the navigable index built from those events: "jump to the
//! previous/next prompt" and "mark the command that failed" both read it. It is
//! deliberately free of GTK and VTE types so it can be unit-tested without a
//! display — the wiring in [`super::termprops`] converts VTE events into the
//! plain `i64` buffer rows this module stores, and the window actions turn a
//! looked-up row back into a scroll.
//!
//! # Coordinate space
//!
//! A `row` is a row in VTE's buffer coordinate space, the same space
//! [`super::highlight_overlay`] documents: row 0 is the oldest line still in the
//! buffer and the viewport covers `[vadjustment.value() .. + row_count)`. VTE
//! trims the oldest rows once scrollback fills, which renumbers nothing but drops
//! the low end — so a recorded mark can fall off the bottom of the buffer, and
//! [`MarkRegistry::trim_below`] is how the caller discards those before a stored
//! row can point past the live buffer.

// On a build below the `vte-0-78` feature the shell-integration termprop hook is
// a no-op, so `record_prompt`/`set_last_exit` have no caller there (the prompt
// data only ever arrives through that hook). `PromptMark::row` is likewise read
// only by code paths that may be compiled out. A module-level `allow` keeps the
// registry a single coherent unit across both feature builds rather than
// scattering `cfg`-conditional `expect`s over individual members.
#![allow(
    dead_code,
    reason = "record_prompt/set_last_exit have no caller on a build below vte-0-78 (no-op hook)"
)]

/// One recorded shell prompt and the result of the command run at it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PromptMark {
    /// Row in VTE's buffer coordinate space where the prompt began.
    row: i64,
    /// Exit code of the command run at this prompt, once the shell reports it.
    /// `None` until a command-end event arrives, or when the shell signalled the
    /// end without a code.
    exit_code: Option<u64>,
}

impl PromptMark {
    pub(crate) const fn row(self) -> i64 {
        self.row
    }

    /// Whether the command run at this prompt ended with a non-zero exit code.
    pub(crate) const fn failed(self) -> bool {
        matches!(self.exit_code, Some(code) if code != 0)
    }
}

/// The ordered set of prompt marks for one terminal session.
///
/// Marks are kept sorted by row, which they naturally are: a prompt is always
/// recorded at or below the previous one, and `trim_below` only drops a prefix.
/// That ordering is what lets navigation use a binary search and failure marking
/// iterate in screen order.
#[derive(Debug, Default)]
pub(crate) struct MarkRegistry {
    marks: Vec<PromptMark>,
}

impl MarkRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Records a prompt that began at `row`.
    ///
    /// A repeat of the current last row (the shell re-emitting a prompt mark
    /// without any command in between, e.g. an empty Enter) updates in place
    /// rather than stacking duplicates, so navigation steps once per visible
    /// prompt. A row before the last recorded one is out of order and ignored —
    /// prompts only ever move down the buffer — which keeps the `marks` sort
    /// invariant without a re-sort.
    pub(crate) fn record_prompt(&mut self, row: i64) {
        match self.marks.last() {
            Some(last) if row == last.row => { /* same prompt line, keep it */ }
            Some(last) if row < last.row => { /* out of order, ignore */ }
            _ => self.marks.push(PromptMark {
                row,
                exit_code: None,
            }),
        }
    }

    /// Associates a command-end exit code with the most recent prompt.
    ///
    /// A no-op when no prompt has been recorded yet: a command-end with no prompt
    /// before it has nowhere to attach and is dropped.
    pub(crate) fn set_last_exit(&mut self, code: Option<u64>) {
        if let Some(last) = self.marks.last_mut() {
            last.exit_code = code;
        }
    }

    /// Drops every mark whose row is below `oldest_row`.
    ///
    /// Call this when VTE trims scrollback, with the row number of the oldest
    /// line still in the buffer, so no stored row points past the live buffer.
    /// Marks are sorted, so this is a prefix drop.
    pub(crate) fn trim_below(&mut self, oldest_row: i64) {
        let keep_from = self.marks.partition_point(|mark| mark.row < oldest_row);
        if keep_from > 0 {
            self.marks.drain(..keep_from);
        }
    }

    /// The nearest prompt row strictly above `row`, if any.
    pub(crate) fn prev_before(&self, row: i64) -> Option<i64> {
        let idx = self.marks.partition_point(|mark| mark.row < row);
        idx.checked_sub(1).map(|i| self.marks[i].row)
    }

    /// The nearest prompt row strictly below `row`, if any.
    pub(crate) fn next_after(&self, row: i64) -> Option<i64> {
        let idx = self.marks.partition_point(|mark| mark.row <= row);
        self.marks.get(idx).map(|mark| mark.row)
    }

    /// Rows of prompts whose command failed (non-zero exit code), in screen
    /// order, for the failed-command marker.
    pub(crate) fn failed_rows(&self) -> impl Iterator<Item = i64> + '_ {
        self.marks
            .iter()
            .filter(|mark| mark.failed())
            .map(|mark| mark.row)
    }

    /// Whether any prompt has been recorded. Used to keep navigation a silent
    /// no-op on a session with no shell integration.
    pub(crate) fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_in_order_and_assigns_exit_to_latest() {
        let mut reg = MarkRegistry::new();
        reg.record_prompt(10);
        reg.set_last_exit(Some(0));
        reg.record_prompt(20);
        reg.set_last_exit(Some(1));

        assert_eq!(reg.failed_rows().collect::<Vec<_>>(), vec![20]);
    }

    #[test]
    fn repeat_of_last_row_does_not_duplicate() {
        let mut reg = MarkRegistry::new();
        reg.record_prompt(10);
        reg.record_prompt(10);
        // Only one mark, so stepping down from above it lands once.
        assert_eq!(reg.next_after(5), Some(10));
        assert_eq!(reg.next_after(10), None);
    }

    #[test]
    fn out_of_order_row_is_ignored() {
        let mut reg = MarkRegistry::new();
        reg.record_prompt(20);
        reg.record_prompt(10); // earlier than the last — ignored
        // The registry still only knows about row 20.
        assert_eq!(reg.prev_before(100), Some(20));
        assert_eq!(reg.prev_before(15), None);
    }

    #[test]
    fn set_last_exit_without_a_prompt_is_a_noop() {
        let mut reg = MarkRegistry::new();
        reg.set_last_exit(Some(1));
        assert!(reg.failed_rows().next().is_none());
    }

    #[test]
    fn exit_code_zero_and_none_are_not_failures() {
        let mut reg = MarkRegistry::new();
        reg.record_prompt(10);
        reg.set_last_exit(Some(0));
        reg.record_prompt(20);
        reg.set_last_exit(None);
        assert!(reg.failed_rows().next().is_none());
    }

    #[test]
    fn prev_before_is_strictly_above() {
        let mut reg = MarkRegistry::new();
        for row in [10, 20, 30] {
            reg.record_prompt(row);
        }
        assert_eq!(reg.prev_before(25), Some(20));
        assert_eq!(reg.prev_before(20), Some(10)); // strict: not 20 itself
        assert_eq!(reg.prev_before(10), None);
        assert_eq!(reg.prev_before(5), None);
        assert_eq!(reg.prev_before(1000), Some(30));
    }

    #[test]
    fn next_after_is_strictly_below() {
        let mut reg = MarkRegistry::new();
        for row in [10, 20, 30] {
            reg.record_prompt(row);
        }
        assert_eq!(reg.next_after(15), Some(20));
        assert_eq!(reg.next_after(20), Some(30)); // strict: not 20 itself
        assert_eq!(reg.next_after(30), None);
        assert_eq!(reg.next_after(1000), None);
        assert_eq!(reg.next_after(0), Some(10));
    }

    #[test]
    fn trim_below_drops_only_the_prefix() {
        let mut reg = MarkRegistry::new();
        for row in [10, 20, 30, 40] {
            reg.record_prompt(row);
        }
        reg.trim_below(25);
        assert_eq!(reg.prev_before(1000), Some(40));
        assert_eq!(reg.next_after(0), Some(30)); // 10 and 20 gone
        assert_eq!(reg.prev_before(30), None); // nothing below 30 anymore
    }

    #[test]
    fn trim_below_floor_keeps_boundary_row() {
        let mut reg = MarkRegistry::new();
        for row in [10, 20, 30] {
            reg.record_prompt(row);
        }
        reg.trim_below(20); // keep rows >= 20
        assert_eq!(reg.next_after(0), Some(20));
    }

    #[test]
    fn trim_everything_leaves_empty() {
        let mut reg = MarkRegistry::new();
        reg.record_prompt(10);
        reg.record_prompt(20);
        reg.trim_below(100);
        assert!(reg.is_empty());
        assert_eq!(reg.prev_before(50), None);
        assert_eq!(reg.next_after(5), None);
    }

    #[test]
    fn empty_registry_navigates_to_nothing() {
        let reg = MarkRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.prev_before(10), None);
        assert_eq!(reg.next_after(10), None);
        assert!(reg.failed_rows().next().is_none());
    }

    #[test]
    fn failed_rows_are_in_screen_order() {
        let mut reg = MarkRegistry::new();
        reg.record_prompt(10);
        reg.set_last_exit(Some(2));
        reg.record_prompt(20);
        reg.set_last_exit(Some(0));
        reg.record_prompt(30);
        reg.set_last_exit(Some(127));
        assert_eq!(reg.failed_rows().collect::<Vec<_>>(), vec![10, 30]);
    }
}
