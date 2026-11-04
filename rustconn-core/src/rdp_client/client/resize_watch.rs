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
//!
//! The session loop only stores the watch and calls in here:
//! [`ResizeWatch::after_send`] when a layout goes out,
//! [`ResizeWatch::take_applied`] at the top of every pass, and
//! [`ResizeWatch::expire`] when the deadline sleep wakes. The decisions live
//! here so they can be tested without a session.

use std::time::{Duration, Instant};

use super::super::RdpClientEvent;

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

    /// Takes the watch out of `watch` once the desktop size has changed, so a
    /// layout the server applied never reaches the deadline. Returns the
    /// applied watch; `watch` is left alone while the layout is pending.
    pub(super) fn take_applied(
        watch: &mut Option<Self>,
        current: (u16, u16),
        now: Instant,
    ) -> Option<Self> {
        watch.take_if(|watch| watch.verdict(current, now) == ResizeVerdict::Applied)
    }

    /// Decides the deadline branch: returns the event to send when the server
    /// ignored the layout, `DisplayControlUnavailable { layout_ignored: true }`
    /// for the requested size.
    ///
    /// The watch is cleared when it is decided either way, so the event goes
    /// out once per layout. A watch whose server still has time is kept.
    #[must_use]
    pub(super) fn expire(
        watch: &mut Option<Self>,
        current: (u16, u16),
        now: Instant,
    ) -> Option<RdpClientEvent> {
        let verdict = watch.as_ref()?.verdict(current, now);
        if verdict == ResizeVerdict::Waiting {
            return None;
        }
        let (width, height) = watch.take()?.requested();
        if verdict != ResizeVerdict::Ignored {
            return None;
        }
        tracing::info!(
            protocol = "rdp",
            width,
            height,
            "Server did not apply the monitor layout — falling back to reconnect"
        );
        Some(RdpClientEvent::DisplayControlUnavailable {
            width,
            height,
            layout_ignored: true,
        })
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

    /// Whether `event` is the fallback for a layout of `size` the server ignored.
    fn is_ignored_layout(event: Option<&RdpClientEvent>, size: (u16, u16)) -> bool {
        matches!(
            event,
            Some(RdpClientEvent::DisplayControlUnavailable {
                width,
                height,
                layout_ignored: true,
            }) if (*width, *height) == size
        )
    }

    #[test]
    fn the_deadline_reports_an_ignored_layout_once() {
        let now = Instant::now();
        let mut watch = ResizeWatch::after_send(true, TO, FROM, now);
        let deadline = now + LAYOUT_APPLY_TIMEOUT;

        let event = ResizeWatch::expire(&mut watch, FROM, deadline);
        assert!(is_ignored_layout(event.as_ref(), TO), "{event:?}");
        assert_eq!(watch, None, "the watch is spent");
        assert!(ResizeWatch::expire(&mut watch, FROM, deadline).is_none());
    }

    #[test]
    fn a_deadline_that_wakes_early_keeps_the_watch() {
        let now = Instant::now();
        let mut watch = ResizeWatch::arm(TO, FROM, now);
        let early = now + Duration::from_millis(2999);

        assert!(ResizeWatch::expire(&mut watch, FROM, early).is_none());
        assert_eq!(watch, ResizeWatch::arm(TO, FROM, now));
    }

    #[test]
    fn an_applied_layout_is_taken_before_its_deadline() {
        let now = Instant::now();
        let mut watch = ResizeWatch::arm(TO, FROM, now);
        let pass = now + Duration::from_millis(400);

        // No change yet: the loop pass leaves the watch armed.
        assert_eq!(ResizeWatch::take_applied(&mut watch, FROM, pass), None);
        assert!(watch.is_some());

        // A reactivation replaced the image: the next pass takes the watch, so
        // the deadline has nothing left to report.
        let applied = ResizeWatch::take_applied(&mut watch, TO, pass);
        assert_eq!(applied.map(|watch| watch.requested()), Some(TO));
        assert_eq!(watch, None);
        assert!(ResizeWatch::expire(&mut watch, TO, now + LAYOUT_APPLY_TIMEOUT).is_none());
    }

    #[test]
    fn a_layout_applied_in_the_pass_that_wakes_the_deadline_is_not_reported() {
        let now = Instant::now();
        let mut watch = ResizeWatch::arm(TO, FROM, now);
        assert!(ResizeWatch::expire(&mut watch, TO, now + LAYOUT_APPLY_TIMEOUT).is_none());
        assert_eq!(watch, None);
    }

    #[test]
    fn a_second_layout_rearms_the_watch_with_its_own_deadline() {
        let now = Instant::now();
        let second: (u16, u16) = (1920, 1080);
        let mut watch = ResizeWatch::after_send(true, TO, FROM, now);
        assert!(watch.is_some());
        let later = now + Duration::from_secs(2);
        watch = ResizeWatch::after_send(true, second, FROM, later);

        // The first layout's deadline passes without a report.
        assert!(ResizeWatch::expire(&mut watch, FROM, now + LAYOUT_APPLY_TIMEOUT).is_none());
        // The second one is reported at its own deadline, for its own size.
        let event = ResizeWatch::expire(&mut watch, FROM, later + LAYOUT_APPLY_TIMEOUT);
        assert!(is_ignored_layout(event.as_ref(), second), "{event:?}");
    }

    #[test]
    fn a_failed_send_leaves_no_deadline_to_report() {
        let now = Instant::now();
        let mut watch = ResizeWatch::after_send(true, TO, FROM, now);
        assert!(watch.is_some());
        watch = ResizeWatch::after_send(false, (1920, 1080), FROM, now + Duration::from_secs(1));
        assert!(ResizeWatch::expire(&mut watch, FROM, now + LAYOUT_APPLY_TIMEOUT * 2).is_none());
    }

    #[test]
    fn a_layout_back_to_the_current_size_cancels_the_watch() {
        let now = Instant::now();
        let mut watch = ResizeWatch::after_send(true, TO, FROM, now);
        assert!(watch.is_some());
        watch = ResizeWatch::after_send(true, FROM, FROM, now + Duration::from_secs(1));
        assert_eq!(watch, None);
    }
}
