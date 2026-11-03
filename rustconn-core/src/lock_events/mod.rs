//! Screen-lock and suspend notifications, for anything that must drop state
//! when the user walks away.
//!
//! One listener, several consumers. A consumer calls [`subscribe`] once with a
//! handler; the listener calls every handler, in subscription order, for each
//! [`LockEvent`]. Today the SSH agent cleanup subscribes (it removes the keys
//! RustConn added, see [`crate::ssh_agent::evict_own_keys`]); other state that
//! should not outlive a locked screen can subscribe the same way.
//!
//! The subscriber list is plain Rust and always built. The source of the events
//! is the D-Bus listener in `dbus`, started with `start()`. It exists only with
//! the non-default `lock-events` feature and only off macOS: login1 and the
//! `ScreenSaver` interface are Linux/BSD D-Bus services, and a headless build
//! has no session to watch. Without it no event ever fires and subscribing is
//! harmless.

use std::sync::{Arc, Mutex, PoisonError};

#[cfg(all(feature = "lock-events", not(target_os = "macos")))]
mod dbus;

#[cfg(all(feature = "lock-events", not(target_os = "macos")))]
pub use dbus::start;

/// Something the user's session just did that state should react to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockEvent {
    /// The screen locked (the screensaver became active).
    ScreenLocked,
    /// The system is about to suspend or hibernate. The listener holds a login1
    /// delay lock until every handler has returned, so a handler can still
    /// reach a local socket before the machine sleeps.
    Suspending,
}

/// A registered handler. `Arc` so [`Subscribers::dispatch`] can call the
/// handlers without holding the lock (a handler may then subscribe another).
type Handler = Arc<dyn Fn(LockEvent) + Send + Sync>;

/// The handlers to call for each event.
///
/// The process has one, behind [`subscribe`]; tests build their own so they
/// never share state with each other.
pub(crate) struct Subscribers {
    handlers: Mutex<Vec<Handler>>,
}

impl Subscribers {
    pub(crate) const fn new() -> Self {
        Self {
            handlers: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn add(&self, handler: Handler) {
        self.handlers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(handler);
    }

    /// Calls every handler with `event`, in subscription order, and returns
    /// once all of them have.
    #[cfg_attr(
        not(all(feature = "lock-events", not(target_os = "macos"))),
        allow(
            dead_code,
            reason = "only the D-Bus listener and the tests dispatch; neither is built here"
        )
    )]
    pub(crate) fn dispatch(&self, event: LockEvent) {
        let handlers = self
            .handlers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for handler in &handlers {
            handler(event);
        }
    }
}

impl std::fmt::Debug for Subscribers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self
            .handlers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        f.debug_struct("Subscribers")
            .field("handlers", &count)
            .finish()
    }
}

/// The process-wide subscriber list the listener dispatches to.
static SUBSCRIBERS: Subscribers = Subscribers::new();

/// Registers `handler` to be called on every [`LockEvent`].
///
/// Handlers run on the listener's own thread, never the GTK main thread, so
/// they must not touch GTK objects. They run one after another: a slow handler
/// delays the next one, and on [`LockEvent::Suspending`] it delays the suspend
/// up to logind's `InhibitDelayMaxSec` (5 s by default).
pub fn subscribe(handler: impl Fn(LockEvent) + Send + Sync + 'static) {
    SUBSCRIBERS.add(Arc::new(handler));
}

/// Dispatches `event` to the process-wide subscribers.
#[cfg(all(feature = "lock-events", not(target_os = "macos")))]
fn dispatch(event: LockEvent) {
    SUBSCRIBERS.dispatch(event);
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn every_consumer_sees_every_event_in_order() {
        let subscribers = Subscribers::new();
        let seen: Arc<Mutex<Vec<(u8, LockEvent)>>> = Arc::default();
        for id in 0..2u8 {
            let seen = Arc::clone(&seen);
            subscribers.add(Arc::new(move |event| {
                seen.lock().unwrap().push((id, event));
            }));
        }

        subscribers.dispatch(LockEvent::ScreenLocked);
        subscribers.dispatch(LockEvent::Suspending);

        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                (0, LockEvent::ScreenLocked),
                (1, LockEvent::ScreenLocked),
                (0, LockEvent::Suspending),
                (1, LockEvent::Suspending),
            ]
        );
    }

    #[test]
    fn a_handler_may_subscribe_another_without_deadlocking() {
        let subscribers = Arc::new(Subscribers::new());
        let inner = Arc::clone(&subscribers);
        subscribers.add(Arc::new(move |_| inner.add(Arc::new(|_| {}))));
        subscribers.dispatch(LockEvent::ScreenLocked);
        assert_eq!(format!("{subscribers:?}"), "Subscribers { handlers: 2 }");
    }
}
