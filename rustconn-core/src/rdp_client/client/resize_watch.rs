//! Watches whether the server applied a monitor layout it was sent.
//!
//! A resize over the Display Control channel (MS-RDPEDISP) only sends the new
//! layout; the server applies it by changing the desktop size the client
//! renders into. Windows does that with a Deactivation-Reactivation Sequence
//! (`ActiveStageOutput::DeactivateAll`) or, on the graphics pipeline, with a
//! `ResetGraphics` PDU. Both replace the session image with one of the new
//! size, so the image size is the one signal that covers both paths.
//!
//! Some servers accept the layout and never follow it: xrdp resizes its own
//! screen but sends neither, and the client keeps drawing the old size with
//! black bars. Nothing on the wire says so, so the session loop gives the server
//! [`LAYOUT_APPLY_TIMEOUT`] and then reports the layout as ignored; the GUI
//! falls back to a reconnect at the new size.

use std::time::{Duration, Instant};

/// How long the server gets to apply a monitor layout before it counts as
/// ignored. Windows answers within about a second; three leaves room for a
/// slow link without leaving black bars on screen for long.
pub(super) const LAYOUT_APPLY_TIMEOUT: Duration = Duration::from_secs(3);

/// A monitor layout sent to the server and not yet applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ResizeWatch {
    requested: (u16, u16),
    sent_from: (u16, u16),
    deadline: Instant,
}

/// What became of a watched layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResizeVerdict {
    /// The desktop size changed: the server applied a layout.
    Applied,
    /// No change yet, and the server still has time.
    Waiting,
    /// No change by the deadline: the server ignored the layout.
    Ignored,
}

impl ResizeWatch {
    /// Starts watching a layout of `requested` size sent while the desktop was
    /// `current`. `None` when the two are equal: there is nothing to apply.
    #[must_use]
    pub(super) fn arm(requested: (u16, u16), current: (u16, u16), now: Instant) -> Option<Self> {
        (requested != current).then_some(Self {
            requested,
            sent_from: current,
            deadline: now + LAYOUT_APPLY_TIMEOUT,
        })
    }

    /// The watch to keep after a layout of `requested` size was offered to the
    /// server while the desktop was `current`.
    ///
    /// It replaces any earlier watch. A layout that did not go out (`sent` is
    /// `false`) leaves nothing to watch. Keeping the earlier watch armed would
    /// let it time out after the failed send, report a layout the user no
    /// longer wants as ignored, and switch the session to reconnect-on-resize.
    #[must_use]
    pub(super) fn after_send(
        sent: bool,
        requested: (u16, u16),
        current: (u16, u16),
        now: Instant,
    ) -> Option<Self> {
        if sent {
            Self::arm(requested, current, now)
        } else {
            None
        }
    }

    /// The size the layout asked for.
    #[must_use]
    pub(super) const fn requested(&self) -> (u16, u16) {
        self.requested
    }

    /// When the layout counts as ignored if the desktop has not changed.
    #[must_use]
    pub(super) const fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Decides the layout's fate from the desktop size now.
    ///
    /// Any change from the size the layout was sent at counts as applied, not
    /// only the exact request: a server may round the size, and a later
    /// request replaces this watch anyway.
    #[must_use]
    pub(super) fn verdict(&self, current: (u16, u16), now: Instant) -> ResizeVerdict {
        if current != self.sent_from {
            ResizeVerdict::Applied
        } else if now >= self.deadline {
            ResizeVerdict::Ignored
        } else {
            ResizeVerdict::Waiting
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FROM: (u16, u16) = (1280, 800);
    const TO: (u16, u16) = (1600, 900);

    #[test]
    fn a_layout_for_the_current_size_is_not_watched() {
        assert_eq!(ResizeWatch::arm(FROM, FROM, Instant::now()), None);
    }

    #[test]
    fn a_reactivation_or_graphics_reset_within_the_timeout_is_applied() {
        let now = Instant::now();
        let watch = ResizeWatch::arm(TO, FROM, now).expect("sizes differ");
        assert_eq!(watch.requested(), TO);
        assert_eq!(
            watch.verdict(TO, now + Duration::from_millis(400)),
            ResizeVerdict::Applied
        );
        // A server that rounds the size still applied the layout.
        assert_eq!(
            watch.verdict((1600, 896), now + Duration::from_millis(400)),
            ResizeVerdict::Applied
        );
    }

    #[test]
    fn an_unchanged_desktop_waits_until_the_deadline() {
        let now = Instant::now();
        let watch = ResizeWatch::arm(TO, FROM, now).expect("sizes differ");
        assert_eq!(
            watch.verdict(FROM, now + Duration::from_millis(2999)),
            ResizeVerdict::Waiting
        );
    }

    #[test]
    fn an_unchanged_desktop_at_the_deadline_is_ignored() {
        let now = Instant::now();
        let watch = ResizeWatch::arm(TO, FROM, now).expect("sizes differ");
        assert_eq!(watch.deadline(), now + LAYOUT_APPLY_TIMEOUT);
        assert_eq!(
            watch.verdict(FROM, now + LAYOUT_APPLY_TIMEOUT),
            ResizeVerdict::Ignored
        );
    }

    #[test]
    fn a_failed_send_clears_the_watch_and_a_sent_one_replaces_it() {
        let now = Instant::now();
        assert_eq!(ResizeWatch::after_send(false, TO, FROM, now), None);
        assert_eq!(
            ResizeWatch::after_send(true, TO, FROM, now),
            ResizeWatch::arm(TO, FROM, now)
        );
    }

    #[test]
    fn a_late_change_still_counts_as_applied() {
        // The size changed before the timer branch ran: never a fallback.
        let now = Instant::now();
        let watch = ResizeWatch::arm(TO, FROM, now).expect("sizes differ");
        assert_eq!(
            watch.verdict(TO, now + LAYOUT_APPLY_TIMEOUT * 2),
            ResizeVerdict::Applied
        );
    }
}
