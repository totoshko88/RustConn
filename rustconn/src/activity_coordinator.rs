//! Activity coordinator for terminal output monitoring.
//!
//! Manages per-session activity and silence detection following the
//! [`MonitoringCoordinator`](crate::monitoring::MonitoringCoordinator) pattern.
//! Each SSH session can independently track output events and notify
//! the user when activity resumes after a quiet period or when silence
//! exceeds a configurable timeout.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4::glib;
use rustconn_core::activity_monitor::MonitorMode;
use uuid::Uuid;

/// The type of notification fired by the activity coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationType {
    /// New output appeared after a quiet period (activity mode).
    Activity,
    /// No output occurred for the configured silence timeout (silence mode).
    Silence,
    /// The remote shell reported that a command finished (command mode).
    ///
    /// Carries the command's exit code, or `None` when the shell signalled the
    /// event without one. Unlike the other two this is not a timing heuristic over
    /// raw output — it comes from VTE's `vte.shell.postexec` termprop, so it fires
    /// exactly once per command and knows whether the command succeeded.
    CommandFinished {
        /// Exit code reported by the shell, if it sent one.
        exit_code: Option<u64>,
    },
}

/// Per-session state for activity monitoring.
struct SessionActivityState {
    /// Current monitoring mode for this session.
    mode: MonitorMode,
    /// Seconds of quiet before activity notification fires.
    quiet_period_secs: u32,
    /// Seconds of silence before silence notification fires.
    silence_timeout_secs: u32,
    /// Timestamp of the last terminal output event.
    last_output_time: Instant,
    /// Whether a notification is currently shown (cleared on tab switch).
    notification_active: bool,
    /// Handle to the pending silence timer, if any.
    silence_timer_id: Option<glib::SourceId>,
}

/// Shared inner state wrapped in `Rc<RefCell<>>` so that glib timer
/// closures can capture a clone without requiring `&self` to be `'static`.
struct CoordinatorInner {
    sessions: HashMap<Uuid, SessionActivityState>,
    silence_callback: Option<Box<dyn Fn(Uuid, NotificationType)>>,
}

/// Per-session activity and silence coordinator.
///
/// Follows the same session-keyed pattern as
/// [`MonitoringCoordinator`](crate::monitoring::MonitoringCoordinator).
/// Internal state is wrapped in `Rc<RefCell<>>` so that glib timer
/// callbacks can safely mutate session state.
pub struct ActivityCoordinator {
    inner: Rc<RefCell<CoordinatorInner>>,
}

impl ActivityCoordinator {
    /// Creates a new coordinator with no active sessions.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(CoordinatorInner {
                sessions: HashMap::new(),
                silence_callback: None,
            })),
        }
    }

    /// Registers a silence-timer callback.
    ///
    /// When a silence timer expires the coordinator invokes this closure
    /// with the session ID and [`NotificationType::Silence`].
    pub fn set_silence_callback<F: Fn(Uuid, NotificationType) + 'static>(&self, cb: F) {
        self.inner.borrow_mut().silence_callback = Some(Box::new(cb));
    }

    /// Starts monitoring for a session.
    ///
    /// If the session is already tracked it is replaced (the old silence
    /// timer, if any, is cancelled first).
    pub fn start(&self, session_id: Uuid, mode: MonitorMode, quiet: u32, silence: u32) {
        // Cancel any existing timer for this session
        Self::cancel_silence_timer_inner(&self.inner, session_id);

        let mut state = SessionActivityState {
            mode,
            quiet_period_secs: quiet,
            silence_timeout_secs: silence,
            last_output_time: Instant::now(),
            notification_active: false,
            silence_timer_id: None,
        };

        // If starting in silence mode, arm the timer immediately
        if mode == MonitorMode::Silence {
            state.silence_timer_id = Self::arm_silence_timer(&self.inner, session_id, silence);
        }

        self.inner.borrow_mut().sessions.insert(session_id, state);
    }

    /// Stops monitoring for a session and cancels any pending silence timer.
    pub fn stop(&self, session_id: Uuid) {
        Self::cancel_silence_timer_inner(&self.inner, session_id);
        self.inner.borrow_mut().sessions.remove(&session_id);
    }

    /// Stops all active monitoring sessions (e.g. on app shutdown).
    pub fn stop_all(&self) {
        let ids: Vec<Uuid> = self.inner.borrow().sessions.keys().copied().collect();
        for id in ids {
            self.stop(id);
        }
    }

    /// Called on VTE `contents_changed`. Returns `Some(notification_type)`
    /// when a notification should be fired.
    ///
    /// - **Activity mode**: fires if elapsed since last output >= quiet period.
    /// - **Silence mode**: resets the silence timer on every output event.
    /// - **Off mode**: no-op, returns `None`.
    pub fn on_output(&self, session_id: Uuid) -> Option<NotificationType> {
        let mode;
        let quiet_period_secs;
        let silence_timeout_secs;

        {
            let mut inner = self.inner.borrow_mut();
            let state = inner.sessions.get_mut(&session_id)?;

            mode = state.mode;
            quiet_period_secs = state.quiet_period_secs;
            silence_timeout_secs = state.silence_timeout_secs;

            match state.mode {
                MonitorMode::Off => return None,
                MonitorMode::Activity => {
                    let now = Instant::now();
                    let elapsed = now.duration_since(state.last_output_time);
                    let quiet = Duration::from_secs(u64::from(state.quiet_period_secs));
                    state.last_output_time = now;

                    if elapsed >= quiet && !state.notification_active {
                        state.notification_active = true;
                        return Some(NotificationType::Activity);
                    }
                    return None;
                }
                MonitorMode::Silence => {
                    state.last_output_time = Instant::now();
                    // Cancel the existing silence timer
                    if let Some(source_id) = state.silence_timer_id.take() {
                        source_id.remove();
                    }
                    // Drop the borrow before arming a new timer
                }
                MonitorMode::Command => {
                    // Event-driven: the notification comes from
                    // `on_command_finished`, not from bytes arriving. Tracking the
                    // output time anyway keeps the state honest if the user cycles
                    // into a timing mode on a live session.
                    state.last_output_time = Instant::now();
                    return None;
                }
            }
        }

        // Arm a new silence timer (only reached for Silence mode)
        if mode == MonitorMode::Silence {
            let new_id = Self::arm_silence_timer(&self.inner, session_id, silence_timeout_secs);
            if let Some(state) = self.inner.borrow_mut().sessions.get_mut(&session_id) {
                state.silence_timer_id = new_id;
            }
        }
        // Silence mode output resets the timer but never fires a notification directly
        let _ = quiet_period_secs; // used only in Activity branch above
        None
    }

    /// Called when the remote shell reports that a command has returned.
    ///
    /// Returns `Some` only in [`MonitorMode::Command`] for a tracked session, so the
    /// caller can wire the termprop signal once per terminal and let the mode decide
    /// whether anything is delivered — the same shape as [`Self::on_output`].
    ///
    /// Unlike the timing modes this does not consult `notification_active`: every
    /// finished command is a distinct event, and suppressing the second one because
    /// the first was never acknowledged would hide exactly the case the mode is for
    /// (several commands completing while the user is on another tab).
    pub fn on_command_finished(
        &self,
        session_id: Uuid,
        exit_code: Option<u64>,
    ) -> Option<NotificationType> {
        let mut inner = self.inner.borrow_mut();
        let state = inner.sessions.get_mut(&session_id)?;
        if state.mode != MonitorMode::Command {
            return None;
        }
        state.last_output_time = Instant::now();
        state.notification_active = true;
        Some(NotificationType::CommandFinished { exit_code })
    }

    /// Called when the user switches to this session's tab.
    /// Clears the `notification_active` flag so the indicator can be removed.
    pub fn on_tab_switched(&self, session_id: Uuid) {
        if let Some(state) = self.inner.borrow_mut().sessions.get_mut(&session_id) {
            state.notification_active = false;
        }
    }

    /// Sets the monitoring mode for a session.
    ///
    /// Handles timer lifecycle: cancels silence timers when leaving silence
    /// mode, arms them when entering silence mode.
    pub fn set_mode(&self, session_id: Uuid, mode: MonitorMode) {
        // Cancel any existing silence timer first
        Self::cancel_silence_timer_inner(&self.inner, session_id);

        let silence_timeout = {
            let mut inner = self.inner.borrow_mut();
            let Some(state) = inner.sessions.get_mut(&session_id) else {
                return;
            };
            state.mode = mode;
            state.notification_active = false;
            state.last_output_time = Instant::now();
            state.silence_timeout_secs
        };

        // Arm silence timer if entering silence mode
        if mode == MonitorMode::Silence {
            let new_id = Self::arm_silence_timer(&self.inner, session_id, silence_timeout);
            if let Some(state) = self.inner.borrow_mut().sessions.get_mut(&session_id) {
                state.silence_timer_id = new_id;
            }
        }
    }

    /// Returns the current monitoring mode for a session, if tracked.
    #[must_use]
    pub fn get_mode(&self, session_id: Uuid) -> Option<MonitorMode> {
        self.inner
            .borrow()
            .sessions
            .get(&session_id)
            .map(|s| s.mode)
    }

    /// Arms a one-shot silence timer that fires after `timeout_secs`.
    ///
    /// When the timer expires it sets `notification_active = true` on the
    /// session and invokes the registered silence callback.
    fn arm_silence_timer(
        inner: &Rc<RefCell<CoordinatorInner>>,
        session_id: Uuid,
        timeout_secs: u32,
    ) -> Option<glib::SourceId> {
        let inner_clone = Rc::clone(inner);

        let source_id =
            glib::timeout_add_local_once(Duration::from_secs(u64::from(timeout_secs)), move || {
                let should_notify = {
                    if let Ok(mut guard) = inner_clone.try_borrow_mut() {
                        if let Some(state) = guard.sessions.get_mut(&session_id) {
                            state.notification_active = true;
                            state.silence_timer_id = None;
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };

                if should_notify
                    && let Ok(guard) = inner_clone.try_borrow()
                    && let Some(cb) = guard.silence_callback.as_ref()
                {
                    cb(session_id, NotificationType::Silence);
                }
            });

        Some(source_id)
    }

    /// Cancels the silence timer for a session, if one is armed.
    fn cancel_silence_timer_inner(inner: &Rc<RefCell<CoordinatorInner>>, session_id: Uuid) {
        if let Ok(mut guard) = inner.try_borrow_mut()
            && let Some(state) = guard.sessions.get_mut(&session_id)
            && let Some(source_id) = state.silence_timer_id.take()
        {
            source_id.remove();
        }
    }
}

impl Default for ActivityCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod command_mode_tests {
    use rustconn_core::activity_monitor::MonitorMode;
    use uuid::Uuid;

    use super::{ActivityCoordinator, NotificationType};

    /// Quiet period and silence timeout that Command mode must ignore.
    const QUIET: u32 = 10;
    const SILENCE: u32 = 30;

    fn started(mode: MonitorMode) -> (ActivityCoordinator, Uuid) {
        let coordinator = ActivityCoordinator::new();
        let session_id = Uuid::new_v4();
        coordinator.start(session_id, mode, QUIET, SILENCE);
        (coordinator, session_id)
    }

    #[test]
    fn a_finished_command_notifies_in_command_mode() {
        let (coordinator, session_id) = started(MonitorMode::Command);
        assert_eq!(
            coordinator.on_command_finished(session_id, Some(0)),
            Some(NotificationType::CommandFinished { exit_code: Some(0) })
        );
    }

    #[test]
    fn the_exit_code_reaches_the_notification_unchanged() {
        // The whole reason this mode exists rather than reusing Activity: the caller
        // needs to tell success from failure, so the code cannot be flattened.
        let (coordinator, session_id) = started(MonitorMode::Command);
        for code in [None, Some(0), Some(1), Some(127)] {
            assert_eq!(
                coordinator.on_command_finished(session_id, code),
                Some(NotificationType::CommandFinished { exit_code: code })
            );
        }
    }

    #[test]
    fn the_other_modes_ignore_a_finished_command() {
        // The termprop handler is wired for every session regardless of mode, so the
        // mode check has to happen here or Off would start notifying.
        for mode in [
            MonitorMode::Off,
            MonitorMode::Activity,
            MonitorMode::Silence,
        ] {
            let (coordinator, session_id) = started(mode);
            assert_eq!(
                coordinator.on_command_finished(session_id, Some(0)),
                None,
                "{mode:?} must not notify on a finished command"
            );
        }
    }

    #[test]
    fn an_untracked_session_ignores_a_finished_command() {
        // A terminal can outlive its coordinator entry: `stop` runs on child exit,
        // and a late signal must not resurrect it.
        let coordinator = ActivityCoordinator::new();
        assert_eq!(
            coordinator.on_command_finished(Uuid::new_v4(), Some(0)),
            None
        );
    }

    #[test]
    fn command_mode_ignores_terminal_output() {
        // Output is not the event. If this returned Some, every byte from a remote
        // command would notify, which is what the timing modes are for.
        let (coordinator, session_id) = started(MonitorMode::Command);
        assert_eq!(coordinator.on_output(session_id), None);
        assert_eq!(coordinator.on_output(session_id), None);
    }

    #[test]
    fn consecutive_commands_each_notify() {
        // Deliberately unlike the timing modes, which suppress a repeat until the
        // user visits the tab: several commands finishing while the user is away is
        // exactly the case this mode is for, so each one is news.
        let (coordinator, session_id) = started(MonitorMode::Command);
        assert!(
            coordinator
                .on_command_finished(session_id, Some(0))
                .is_some()
        );
        assert!(
            coordinator
                .on_command_finished(session_id, Some(1))
                .is_some()
        );
        assert!(
            coordinator
                .on_command_finished(session_id, Some(0))
                .is_some()
        );
    }

    #[test]
    fn cycling_a_live_session_into_command_mode_starts_notifying() {
        // The per-tab Monitor menu cycles on a running session (issue #180), and the
        // termprop handler is already wired, so the mode change alone must be enough.
        let (coordinator, session_id) = started(MonitorMode::Off);
        assert_eq!(coordinator.on_command_finished(session_id, Some(0)), None);

        coordinator.set_mode(session_id, MonitorMode::Command);
        assert_eq!(coordinator.get_mode(session_id), Some(MonitorMode::Command));
        assert!(
            coordinator
                .on_command_finished(session_id, Some(0))
                .is_some()
        );
    }

    #[test]
    fn cycling_out_of_command_mode_stops_notifying() {
        let (coordinator, session_id) = started(MonitorMode::Command);
        assert!(
            coordinator
                .on_command_finished(session_id, Some(0))
                .is_some()
        );

        coordinator.set_mode(session_id, MonitorMode::Off);
        assert_eq!(coordinator.on_command_finished(session_id, Some(0)), None);
    }
}
