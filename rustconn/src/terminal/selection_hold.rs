//! Holds PTY output back while the user drags out a mouse selection.
//!
//! VTE drops a selection whenever new output arrives before the drag ends. In
//! `process_incoming` it compares the selected text with the PRIMARY copy it
//! takes on button release, and mid-drag there is no such copy yet, so any
//! output at all deselects. For its own PTY, VTE avoids this by not reading
//! while a selection is in progress (`disconnect_pty_read` in
//! `start_selection`). RustConn owns the PTY and calls `feed`, which bypasses
//! that pause. A remote that redraws on a timer, such as byobu's status clock,
//! would cancel every drag that takes longer than a second.
//!
//! [`SelectionHold`] restores the pause on the relay side. The output loop
//! waits while the primary button is down and VTE has a selection, and resumes
//! on release. A click that VTE reports to a mouse-tracking app (tmux with mouse
//! on, vim) creates no selection, so it never pauses output. Nothing is
//! buffered here: while the loop waits, the relay's bounded queue fills and the
//! reader thread blocks, the same backpressure VTE gets.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{EventControllerFocus, EventControllerLegacy, PropagationPhase, gdk, glib};
use vte4::Terminal;

/// Tracks whether a selection drag is in progress on one terminal.
pub(super) struct SelectionHold {
    held: Rc<Cell<bool>>,
    wake: async_channel::Receiver<()>,
    controllers: Option<(EventControllerLegacy, EventControllerFocus)>,
}

impl SelectionHold {
    /// Starts watching the primary button on `terminal`.
    ///
    /// The controllers only observe: every event is passed on to VTE.
    pub(super) fn attach(terminal: &Terminal) -> Self {
        let (held, wake_tx, wake) = Self::state();

        let buttons = EventControllerLegacy::new();
        buttons.set_propagation_phase(PropagationPhase::Capture);
        let held_on_button = Rc::clone(&held);
        let wake_on_button = wake_tx.clone();
        buttons.connect_event(move |_, event| {
            let primary = event
                .downcast_ref::<gdk::ButtonEvent>()
                .is_some_and(|button| button.button() == gdk::BUTTON_PRIMARY);
            if primary {
                match event.event_type() {
                    gdk::EventType::ButtonPress => held_on_button.set(true),
                    gdk::EventType::ButtonRelease => {
                        release(&held_on_button, &wake_on_button);
                    }
                    _ => {}
                }
            }
            glib::Propagation::Proceed
        });

        // A release that never reaches the terminal (the window lost focus mid
        // drag) would otherwise stall the session's output until the next click.
        let focus = EventControllerFocus::new();
        let held_on_leave = Rc::clone(&held);
        focus.connect_leave(move |_| release(&held_on_leave, &wake_tx));

        terminal.add_controller(buttons.clone());
        terminal.add_controller(focus.clone());

        Self {
            held,
            wake,
            controllers: Some((buttons, focus)),
        }
    }

    fn state() -> (
        Rc<Cell<bool>>,
        async_channel::Sender<()>,
        async_channel::Receiver<()>,
    ) {
        // One slot is enough: the waiter re-checks the state after every wake.
        let (wake_tx, wake) = async_channel::bounded(1);
        (Rc::new(Cell::new(false)), wake_tx, wake)
    }

    /// Returns once the primary button is up or `has_selection` turns false.
    ///
    /// `has_selection` is re-checked after every wake. The caller passes VTE's
    /// own `has_selection` through a weak reference, so a terminal closed mid
    /// drag is not kept alive by its output loop.
    #[expect(
        clippy::future_not_send,
        reason = "GTK state; awaited only from glib::spawn_future_local on the main thread"
    )]
    pub(super) async fn wait_while_selecting(&self, has_selection: impl Fn() -> bool) {
        while self.wake.try_recv().is_ok() {}
        while self.held.get() && has_selection() {
            if self.wake.recv().await.is_err() {
                return;
            }
        }
    }

    /// Removes the controllers again, so a reconnect does not stack them.
    pub(super) fn detach(mut self, terminal: &Terminal) {
        if let Some((buttons, focus)) = self.controllers.take() {
            terminal.remove_controller(&buttons);
            terminal.remove_controller(&focus);
        }
    }
}

fn release(held: &Cell<bool>, wake: &async_channel::Sender<()>) {
    held.set(false);
    // Full means a wake is already pending, which is all the waiter needs.
    let _ = wake.try_send(());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hold whose button state the test drives, since GDK events cannot be
    /// synthesised without a display.
    fn manual_hold() -> (SelectionHold, async_channel::Sender<()>) {
        let (held, wake_tx, wake) = SelectionHold::state();
        let manual = SelectionHold {
            held,
            wake,
            controllers: None,
        };
        (manual, wake_tx)
    }

    fn pump(ctx: &glib::MainContext) {
        while ctx.iteration(false) {}
    }

    #[test]
    fn a_click_without_a_selection_does_not_wait() {
        let (hold, _wake_tx) = manual_hold();
        hold.held.set(true);
        glib::MainContext::new().block_on(hold.wait_while_selecting(|| false));
    }

    #[test]
    fn a_finished_selection_does_not_wait() {
        let (hold, _wake_tx) = manual_hold();
        glib::MainContext::new().block_on(hold.wait_while_selecting(|| true));
    }

    #[test]
    fn a_drag_waits_until_the_release() {
        let ctx = glib::MainContext::new();
        let (hold, wake_tx) = manual_hold();
        hold.held.set(true);
        // A wake left over from an earlier drag must not end this one.
        assert!(wake_tx.try_send(()).is_ok());

        let hold = Rc::new(hold);
        let done = Rc::new(Cell::new(false));
        let (hold_task, done_task) = (Rc::clone(&hold), Rc::clone(&done));
        drop(ctx.spawn_local(async move {
            hold_task.wait_while_selecting(|| true).await;
            done_task.set(true);
        }));

        pump(&ctx);
        assert!(!done.get(), "output resumed mid drag");
        release(&hold.held, &wake_tx);
        pump(&ctx);
        assert!(done.get(), "output did not resume on release");
    }
}
