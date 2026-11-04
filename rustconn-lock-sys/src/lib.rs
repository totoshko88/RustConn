//! Isolated FFI helper for the macOS screen-lock and sleep notifications.
//!
//! On Linux RustConn learns that the screen locked or the system is about to
//! suspend over D-Bus (`rustconn_core::lock_events`). macOS has neither login1
//! nor a `ScreenSaver` service. It reports the same two moments as Cocoa
//! notifications, and observing them takes Objective-C calls that are `unsafe`
//! in objc2:
//!
//! - `NSWorkspaceWillSleepNotification` on `NSWorkspace.sharedWorkspace`'s own
//!   notification centre, posted before the system sleeps — [`LockSignal::WillSleep`];
//! - `com.apple.screenIsLocked` on `NSDistributedNotificationCenter`, posted by
//!   the login window when the screen locks — [`LockSignal::ScreenLocked`].
//!
//! This crate wraps those calls, so `rustconn`, which keeps a crate-local
//! `forbid(unsafe_code)`, can observe them without re-opening the lint. It
//! knows nothing about SSH keys or about `rustconn-core`: it reports a
//! [`LockSignal`] to a handler, and the GUI maps that onto `lock_events`.
//!
//! # Not observed, on purpose
//!
//! `NSWorkspaceScreensDidSleepNotification` (the displays turned off) is not a
//! lock: with "Require password after screen saver begins or display is turned
//! off" set, macOS locks the screen as well and posts `com.apple.screenIsLocked`,
//! and without it nothing is locked. Wake and unlock have no consumer today.
//!
//! # The contract
//!
//! [`observe`] must be called from the main thread, after GTK has been
//! initialised. The workspace centre posts on the main thread and the
//! distributed centre delivers through a run loop; registering on the main
//! thread puts both on the main run loop, which GDK's macOS backend drives. A
//! call from another thread is refused as [`ObserveOutcome::OffMainThread`]
//! (checked by `objc2::MainThreadMarker`, not assumed), not a panic: a missed
//! lock leaves the keys to their exit and 12-hour bounds, which does not justify
//! taking the process down.
//!
//! The handler is `Send + Sync`, so its soundness does not depend on which
//! thread a notification centre calls it from. It is called on that thread,
//! the main one in practice, and must return quickly; the GUI's handler only
//! queues the event for `lock_events`' own dispatch thread.
//!
//! The observers stay registered for as long as the returned [`LockObservers`]
//! lives, and are removed from both centres when it is dropped.
//!
//! # What CI actually checks
//!
//! Every job but `macos-sys` is Linux, where this crate contains no `unsafe`
//! and [`observe`] answers [`ObserveOutcome::Unsupported`]. The platform-free
//! part — the panic barrier every notification goes through, the notification
//! name, the outcomes — is tested everywhere; the AppKit calls are compiled and
//! clippy-checked only for an Apple target (`macos-sys`, or
//! `cargo clippy -p rustconn-lock-sys --target aarch64-apple-darwin` on Linux),
//! and only a Mac can show them firing.

// The one lint this crate re-opens out of the inherited workspace set, which is
// `deny` rather than `forbid` so that this line is possible. Wrapped in
// `cfg_attr` because off macOS there is no `unsafe` here at all, and a bare
// `expect` would then fire `unfulfilled_lint_expectations` on the Linux build.
#![cfg_attr(
    target_os = "macos",
    expect(
        unsafe_code,
        reason = "sanctioned FFI crate (M-UNSAFE); registering the NSWorkspace and distributed-notification observers is its entire purpose"
    )
)]

use std::sync::Arc;

// ============================================================================
// Public API
// ============================================================================

/// A moment at which state that must not outlive an unattended session should
/// be dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockSignal {
    /// The screen locked (`com.apple.screenIsLocked`).
    ScreenLocked,
    /// The system is about to sleep (`NSWorkspaceWillSleepNotification`).
    WillSleep,
}

/// The handler every observed notification is reported to.
type Handler = Arc<dyn Fn(LockSignal) + Send + Sync>;

/// The name of the distributed notification the login window posts when the
/// screen locks. Not an SDK constant: Apple documents no symbol for it.
#[cfg_attr(
    not(target_os = "macos"),
    allow(
        dead_code,
        reason = "registered only on macOS; pinned by a test everywhere"
    )
)]
const SCREEN_LOCKED_NOTIFICATION: &str = "com.apple.screenIsLocked";

/// The registered observers. Dropping this removes them from both
/// notification centres, after which the handler is never called again.
#[must_use = "dropping the observers unregisters them at once"]
pub struct LockObservers {
    #[cfg(target_os = "macos")]
    _registration: macos::Registration,
    #[cfg(not(target_os = "macos"))]
    _private: (),
}

impl std::fmt::Debug for LockObservers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LockObservers").finish_non_exhaustive()
    }
}

/// What became of a request to observe screen lock and sleep.
#[derive(Debug)]
#[must_use = "dropping `Observing` unregisters the observers at once"]
pub enum ObserveOutcome {
    /// Both observers are registered until the [`LockObservers`] is dropped.
    Observing(LockObservers),
    /// The call came from a thread other than the main one; nothing was
    /// registered.
    OffMainThread,
    /// The platform has no such notifications. Every non-Apple target answers
    /// this; on Linux the D-Bus listener in `rustconn-core` is the source.
    Unsupported,
}

impl ObserveOutcome {
    /// Renders the outcome as a log-ready phrase naming the cause.
    #[must_use]
    pub const fn describe(&self) -> &'static str {
        match self {
            Self::Observing(_) => "observing macOS screen lock and system sleep",
            Self::OffMainThread => {
                "called off the main thread, whose run loop is the only one GTK drives; \
                 call this from the thread that runs the GTK main loop"
            }
            Self::Unsupported => {
                "this platform has no NSWorkspace or distributed notifications to observe"
            }
        }
    }
}

/// Starts reporting screen lock and system sleep to `handler`.
///
/// Returns [`ObserveOutcome::Observing`] with the guard that keeps the
/// observers registered; keep it for as long as the events should arrive. Never
/// panics, and is a no-op answering [`ObserveOutcome::Unsupported`] off Apple
/// platforms, so the caller does not need a `cfg` of its own.
///
/// `handler` is called on the thread the notification centre delivers on (the
/// main thread), so it must return quickly. A panic in it is stopped before it
/// can unwind into Foundation; the panic hook still reports it.
///
/// Must be called from the main thread, after GTK initialisation — the
/// contract described at the [crate] level.
pub fn observe(handler: impl Fn(LockSignal) + Send + Sync + 'static) -> ObserveOutcome {
    observe_platform(Arc::new(handler))
}

// ============================================================================
// Panic barrier
// ============================================================================

/// Calls `handler` with `signal`, stopping a panic at this frame.
///
/// The observer methods are entered from Foundation's notification posting,
/// and unwinding a Rust panic through those Objective-C frames is not
/// something they are written to survive. Returns whether the handler
/// returned normally.
#[cfg_attr(
    not(target_os = "macos"),
    allow(
        dead_code,
        reason = "called by the observer methods on macOS and by the tests"
    )
)]
fn deliver(handler: &(dyn Fn(LockSignal) + Send + Sync), signal: LockSignal) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(signal))).is_ok()
}

// ============================================================================
// Platform
// ============================================================================

#[cfg(not(target_os = "macos"))]
fn observe_platform(_handler: Handler) -> ObserveOutcome {
    ObserveOutcome::Unsupported
}

#[cfg(target_os = "macos")]
fn observe_platform(handler: Handler) -> ObserveOutcome {
    macos::observe(handler)
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::rc::Retained;
    use objc2::runtime::NSObject;
    use objc2::{AnyThread, DefinedClass, MainThreadMarker, define_class, msg_send, sel};
    use objc2_app_kit::{NSWorkspace, NSWorkspaceWillSleepNotification};
    use objc2_foundation::{
        NSDistributedNotificationCenter, NSNotification, NSNotificationCenter,
        NSNotificationSuspensionBehavior, NSString,
    };

    use super::{
        Handler, LockObservers, LockSignal, ObserveOutcome, SCREEN_LOCKED_NOTIFICATION, deliver,
    };

    /// What the observer object carries: the handler to report to.
    pub(super) struct Ivars {
        handler: Handler,
    }

    define_class!(
        // SAFETY: `NSObject` has no subclassing requirements, and `Observer`
        // implements no `Drop`, so none of the rules for a `Drop` on a defined
        // class can be broken.
        #[unsafe(super(NSObject))]
        #[ivars = Ivars]
        pub(super) struct Observer;

        impl Observer {
            // SAFETY: a selector registered with `addObserver:selector:…` is
            // sent as `- (void)name:(NSNotification *)notification`, which is
            // exactly this signature: one object argument, read here as an
            // optional `NSNotification` (it is not used), and no return value.
            #[unsafe(method(screenLocked:))]
            fn screen_locked(&self, _notification: Option<&NSNotification>) {
                deliver(&*self.ivars().handler, LockSignal::ScreenLocked);
            }

            // SAFETY: as for `screenLocked:` above, the signature the
            // notification centre sends a registered selector.
            #[unsafe(method(willSleep:))]
            fn will_sleep(&self, _notification: Option<&NSNotification>) {
                deliver(&*self.ivars().handler, LockSignal::WillSleep);
            }
        }
    );

    impl Observer {
        fn new(handler: Handler) -> Retained<Self> {
            let this = Self::alloc().set_ivars(Ivars { handler });
            // SAFETY: `init` is `NSObject`'s designated initialiser and takes
            // no arguments; it is sent once, through `super`, to the instance
            // `alloc` just returned with its ivars already set — the sequence
            // `define_class!` requires for a class with ivars.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// Both registrations, removed together on drop.
    pub(super) struct Registration {
        observer: Retained<Observer>,
        workspace: Retained<NSNotificationCenter>,
        distributed: Retained<NSDistributedNotificationCenter>,
    }

    impl Drop for Registration {
        fn drop(&mut self) {
            // A notification centre does not retain its observers (since macOS
            // 10.11 it holds them weakly), so without this the registrations
            // would still go away with `observer`. Removing them explicitly
            // makes the end of the guard the end of the events, whatever else
            // may still hold the observer.
            //
            // SAFETY: `removeObserver:` accepts any object and removes every
            // registration it has for it; `observer` is alive, since this
            // struct owns a strong reference to it.
            unsafe { self.workspace.removeObserver(&self.observer) };
            // SAFETY: as above, for the distributed centre.
            unsafe { self.distributed.removeObserver(&self.observer) };
        }
    }

    pub(super) fn observe(handler: Handler) -> ObserveOutcome {
        // Distributed notifications are delivered through the run loop of the
        // thread that registered, and only the main run loop is driven by GDK.
        // `MainThreadMarker::new` asks the runtime instead of trusting the
        // caller.
        if MainThreadMarker::new().is_none() {
            return ObserveOutcome::OffMainThread;
        }

        let observer = Observer::new(handler);

        // NSWorkspace notifications are posted only to the workspace's own
        // centre, never to `NSNotificationCenter.defaultCenter`.
        let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
        // SAFETY: reading an `extern` static: AppKit exports this name as an
        // immutable `NSString *` constant, bound by dyld when AppKit is loaded,
        // which happens before `main` runs since this binary links AppKit.
        let will_sleep = unsafe { NSWorkspaceWillSleepNotification };
        // SAFETY: `observer` is a live `Observer`, whose class implements
        // `willSleep:` with the signature a notification centre sends (see
        // `define_class!` above); `sel!` yields a valid selector; the name is
        // AppKit's constant and the sender filter is `nil` (any sender).
        unsafe {
            workspace.addObserver_selector_name_object(
                &observer,
                sel!(willSleep:),
                Some(will_sleep),
                None,
            );
        }

        // `DeliverImmediately`: AppKit suspends distributed notifications while
        // the application is inactive, and with the default behaviour a lock
        // that happens while RustConn is in the background would be held back
        // until the user returns to it — after unlocking.
        let distributed = NSDistributedNotificationCenter::defaultCenter();
        let screen_locked = NSString::from_str(SCREEN_LOCKED_NOTIFICATION);
        // SAFETY: `observer` is a live `Observer`, whose class implements
        // `screenLocked:` with the signature a notification centre sends;
        // `sel!` yields a valid selector; the name is an `NSString` and the
        // sender filter is `nil` (any sender).
        unsafe {
            distributed.addObserver_selector_name_object_suspensionBehavior(
                &observer,
                sel!(screenLocked:),
                Some(&screen_locked),
                None,
                NSNotificationSuspensionBehavior::DeliverImmediately,
            );
        }

        ObserveOutcome::Observing(LockObservers {
            _registration: Registration {
                observer,
                workspace,
                distributed,
            },
        })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// The name is the one the login window posts, byte for byte. A typo here
    /// would register an observer that never fires and fail nowhere else.
    #[test]
    fn screen_locked_notification_is_the_login_windows() {
        assert_eq!(SCREEN_LOCKED_NOTIFICATION, "com.apple.screenIsLocked");
    }

    /// The barrier hands the handler the signal it was given.
    #[test]
    fn deliver_passes_the_signal_through() {
        let seen = Mutex::new(Vec::new());
        let handler = |signal| seen.lock().unwrap().push(signal);
        assert!(deliver(&handler, LockSignal::ScreenLocked));
        assert!(deliver(&handler, LockSignal::WillSleep));
        assert_eq!(
            *seen.lock().unwrap(),
            vec![LockSignal::ScreenLocked, LockSignal::WillSleep]
        );
    }

    /// A panicking handler is stopped at the barrier instead of unwinding into
    /// the caller, which on macOS is Foundation's notification posting.
    #[test]
    fn deliver_stops_a_panicking_handler() {
        let handler = |_| panic!("a handler's bug");
        assert!(!deliver(&handler, LockSignal::WillSleep));
    }

    /// Off Apple platforms `observe` registers nothing and keeps nothing: the
    /// handler is dropped before it returns, so a Linux build holds no stray
    /// reference to whatever the handler captured.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn observe_is_a_no_op_elsewhere() {
        let captured = Arc::new(());
        let held = Arc::clone(&captured);
        let outcome = observe(move |_| {
            let _ = &held;
        });
        assert!(matches!(outcome, ObserveOutcome::Unsupported));
        assert_eq!(Arc::strong_count(&captured), 1);
    }

    /// The main-thread guard refuses rather than registering on a thread whose
    /// run loop nothing drives. Asked from a spawned thread on purpose: libtest
    /// with `--test-threads=1` and nextest run a test body on the main thread,
    /// where the guard would pass.
    #[cfg(target_os = "macos")]
    #[test]
    fn observe_refuses_a_call_from_another_thread() {
        let refused =
            std::thread::spawn(|| matches!(observe(|_| {}), ObserveOutcome::OffMainThread))
                .join()
                .expect("the probe thread must not panic: the guard returns an outcome");
        assert!(
            refused,
            "a non-main thread must be refused before AppKit is reached"
        );
    }

    /// Every refusal explains itself in a log line. (`Observing` can only be
    /// built by a real registration on macOS, so it is left out.)
    #[test]
    fn refusals_explain_themselves() {
        for outcome in [ObserveOutcome::OffMainThread, ObserveOutcome::Unsupported] {
            let text = outcome.describe();
            assert!(text.len() > 20, "{outcome:?} says only {text:?}");
        }
    }
}
