//! Screen-lock and suspend notifications, for anything that must drop state
//! when the user walks away.
//!
//! One listener, several consumers. A consumer calls [`subscribe`] once with a
//! handler; the listener calls every handler, in subscription order, for each
//! [`LockEvent`]. Today the SSH agent cleanup subscribes (it removes the keys
//! RustConn added, see [`crate::ssh_agent::evict_own_keys`]); other state that
//! should not outlive a locked screen can subscribe the same way.
//!
//! The subscriber list is plain Rust and always built. There are two sources of
//! events:
//!
//! - the D-Bus listener in `dbus`, started with `start()`. It exists only with
//!   the non-default `lock-events` feature and only off macOS: login1 and the
//!   `ScreenSaver` interface are Linux/BSD D-Bus services, and a headless build
//!   has no session to watch;
//! - [`notify`], for a source this crate does not own. The GUI feeds it the
//!   macOS screen-lock and sleep notifications from `rustconn-lock-sys`, so this
//!   crate stays free of AppKit and of `unsafe`.
//!
//! Without a source no event ever fires and subscribing is harmless.

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

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
    ///
    /// A handler that panics is logged and the next one still runs: one
    /// consumer's bug must not keep another from dropping its state.
    pub(crate) fn dispatch(&self, event: LockEvent) {
        let handlers = self
            .handlers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for (index, handler) in handlers.iter().enumerate() {
            // A handler shares nothing with the others through this loop, so
            // a panic cannot leave state here half-updated.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(event)));
            if outcome.is_err() {
                tracing::error!(?event, handler = index, "a lock-event handler panicked");
            }
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
/// Handlers run on the D-Bus listener's blocking pool or on the dispatch thread
/// [`notify`] starts, never the GTK main thread, so they must not touch GTK
/// objects. They run one after another: a slow handler delays the next one, and
/// on [`LockEvent::Suspending`] from D-Bus it delays the suspend up to logind's
/// `InhibitDelayMaxSec` (5 s by default).
pub fn subscribe(handler: impl Fn(LockEvent) + Send + Sync + 'static) {
    SUBSCRIBERS.add(Arc::new(handler));
}

/// Dispatches `event` to the process-wide subscribers.
#[cfg(all(feature = "lock-events", not(target_os = "macos")))]
fn dispatch(event: LockEvent) {
    SUBSCRIBERS.dispatch(event);
}

/// The queue into the dispatch thread behind [`notify`]; `None` when that
/// thread could not be spawned.
static NOTIFY_QUEUE: OnceLock<Option<Sender<LockEvent>>> = OnceLock::new();

/// Hands `event` from a source outside this crate to the subscribers, and
/// returns without waiting for them.
///
/// Meant for a caller on a thread that must not block: the GUI calls it from
/// the macOS main run loop, which is also the GTK main loop. The handlers run on
/// one dispatch thread, started on the first call, in the order the events
/// arrived, so two quick events (a lock straight before a sleep) never run the
/// same handler concurrently. Nothing waits for them, so unlike the D-Bus
/// listener's [`LockEvent::Suspending`] the system does not hold its sleep
/// for them.
///
/// If the dispatch thread cannot be spawned, the handlers run on the caller's
/// thread instead, with a warning: removing the keys late is better than not
/// removing them.
pub fn notify(event: LockEvent) {
    let queue = NOTIFY_QUEUE.get_or_init(|| spawn_dispatch_thread(&SUBSCRIBERS));
    if let Some(queue) = queue
        && queue.send(event).is_ok()
    {
        return;
    }
    tracing::warn!(
        ?event,
        "lock events: no dispatch thread; running the handlers on the caller's thread"
    );
    SUBSCRIBERS.dispatch(event);
}

/// Starts the thread that dispatches every event sent into the returned queue
/// to `subscribers`, one after another, for the life of the process.
fn spawn_dispatch_thread(subscribers: &'static Subscribers) -> Option<Sender<LockEvent>> {
    let (queue, events) = mpsc::channel::<LockEvent>();
    let spawned = std::thread::Builder::new()
        .name("rustconn-lock-dispatch".to_owned())
        .spawn(move || {
            for event in events {
                subscribers.dispatch(event);
            }
        });
    match spawned {
        Ok(_) => Some(queue),
        Err(e) => {
            tracing::warn!(error = %e, "lock events: could not spawn the dispatch thread");
            None
        }
    }
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
    fn a_panicking_handler_does_not_stop_a_later_one() {
        let subscribers = Subscribers::new();
        let seen: Arc<Mutex<Vec<LockEvent>>> = Arc::default();
        subscribers.add(Arc::new(|_| panic!("a consumer's bug")));
        let later = Arc::clone(&seen);
        subscribers.add(Arc::new(move |event| later.lock().unwrap().push(event)));

        subscribers.dispatch(LockEvent::Suspending);
        subscribers.dispatch(LockEvent::ScreenLocked);

        assert_eq!(
            *seen.lock().unwrap(),
            vec![LockEvent::Suspending, LockEvent::ScreenLocked]
        );
    }

    /// The dispatch thread behind `notify` runs the handlers off the caller's
    /// thread (the GTK main thread, in the GUI) and in the order the events
    /// were sent.
    #[test]
    fn the_dispatch_thread_runs_handlers_off_the_caller_in_order() {
        let subscribers: &'static Subscribers = Box::leak(Box::new(Subscribers::new()));
        let (seen, received) = mpsc::channel::<(Option<String>, LockEvent)>();
        let seen = Mutex::new(seen);
        subscribers.add(Arc::new(move |event| {
            let thread = std::thread::current().name().map(str::to_owned);
            seen.lock().unwrap().send((thread, event)).unwrap();
        }));

        let queue = spawn_dispatch_thread(subscribers).unwrap();
        queue.send(LockEvent::ScreenLocked).unwrap();
        queue.send(LockEvent::Suspending).unwrap();

        let timeout = std::time::Duration::from_secs(10);
        let first = received.recv_timeout(timeout).unwrap();
        let second = received.recv_timeout(timeout).unwrap();
        let dispatch_thread = Some("rustconn-lock-dispatch".to_owned());
        assert_eq!(first, (dispatch_thread.clone(), LockEvent::ScreenLocked));
        assert_eq!(second, (dispatch_thread, LockEvent::Suspending));
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
