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

    /// Called when the user switches to this session's tab.
    /// Clears the `notification_active` flag so the indicator can be removed.
    pub fn on_tab_switched(&self, session_id: Uuid) {
        if let Some(state) = self.inner.borrow_mut().sessions.get_mut(&session_id) {
            state.notification_active = false;
        }
    }

    /// Cycles the monitoring mode for a session: Off -> Activity -> Silence -> Off.
    ///
    /// Returns the new mode. If the session is not tracked, returns `Off`.
    pub fn cycle_mode(&self, session_id: Uuid) -> MonitorMode {
        let new_mode = {
            let inner = self.inner.borrow();
            match inner.sessions.get(&session_id) {
                Some(state) => state.mode.next(),
                None => return MonitorMode::Off,
            }
        };
        self.set_mode(session_id, new_mode);
        new_mode
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
