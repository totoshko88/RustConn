//! External viewer session registry (issue #209).
//!
//! Tracks VNC/RDP/SPICE sessions whose display is fully delegated to a separate
//! external viewer process (TigerVNC, xfreerdp, remote-viewer/virt-viewer). Such
//! sessions have no notebook tab; they are surfaced in the sidebar via a session
//! count and an external-viewer emblem, and their child processes are watched by
//! a single shared poll timer so the sidebar state clears when the viewer closes.
//!
//! This module is intentionally separate from [`crate::external_window`], which
//! reparents VTE terminals into RustConn-owned windows — a different concept.
//!
//! The registry keeps no direct sidebar/state coupling: the launch path injects
//! two callbacks ([`ExternalSessionCallbacks`]) that bridge into the sidebar
//! session count and the connection history.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::process::Child;
use std::rc::Rc;
use std::time::Duration;

use gtk4::glib;
use uuid::Uuid;

/// Poll interval for watching external viewer child processes.
///
/// 2000 ms mirrors the legacy per-tab RDP monitor (design R4.1): long enough to
/// stay cheap, short enough that the sidebar state clears within one cycle after
/// the viewer window is closed.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// A tracked external viewer session.
struct ExternalSession {
    /// The connection this session belongs to.
    connection_id: Uuid,
    /// `None` = a viewer RustConn does not own (a detaching viewer) — it cannot
    /// be killed and is never auto-closed by the poll timer (R5.7).
    child: Option<Child>,
    /// History entry recorded at launch, replayed to `record_connection_end`.
    history_entry_id: Option<Uuid>,
    /// Guards the exactly-once `on_ended` firing (defence against re-entrancy;
    /// the primary guard is removing the entry from the map).
    ended: bool,
}

/// Callbacks into sidebar/state, injected at construction so the registry stays
/// free of direct sidebar/state coupling (same pattern as other coordinators).
pub struct ExternalSessionCallbacks {
    /// Fired once when a session is registered: increment session count, show
    /// the external-viewer emblem (`record_connection_start` is done at the
    /// launch site and its entry id passed into `register`).
    pub on_registered: Box<dyn Fn(Uuid /* connection_id */)>,
    /// Fired exactly once when a session ends (viewer exit, Disconnect, or Stop
    /// tracking): decrement session count, clear the emblem, record history end.
    pub on_ended: Box<dyn Fn(Uuid /* connection_id */, Option<Uuid> /* history_entry_id */)>,
}

/// Lightweight registry of active external viewer sessions.
///
/// A single shared 2 s poll timer watches all owned child processes; it starts
/// on the first owned registration and stops when no owned children remain.
pub struct ExternalSessionRegistry {
    sessions: Rc<RefCell<HashMap<Uuid /* session_id */, ExternalSession>>>,
    callbacks: Rc<ExternalSessionCallbacks>,
    timer_running: Rc<Cell<bool>>,
}

impl ExternalSessionRegistry {
    /// Creates a new registry with the given sidebar/state callbacks.
    #[must_use]
    pub fn new(callbacks: ExternalSessionCallbacks) -> Rc<Self> {
        Rc::new(Self {
            sessions: Rc::new(RefCell::new(HashMap::new())),
            callbacks: Rc::new(callbacks),
            timer_running: Rc::new(Cell::new(false)),
        })
    }

    /// Registers a spawned viewer, fires `on_registered`, and ensures the poll
    /// timer runs for owned children.
    ///
    /// Pass `child: None` for a detaching viewer RustConn does not own; such a
    /// session is never killed and never auto-closed by the poll timer (R5.7).
    pub fn register(
        self: &Rc<Self>,
        session_id: Uuid,
        connection_id: Uuid,
        child: Option<Child>,
        history_entry_id: Option<Uuid>,
    ) {
        self.sessions.borrow_mut().insert(
            session_id,
            ExternalSession {
                connection_id,
                child,
                history_entry_id,
                ended: false,
            },
        );
        // Fire the callback after the borrow is released (avoids BorrowMutError
        // if a callback re-enters the registry).
        (self.callbacks.on_registered)(connection_id);
        self.ensure_timer();
    }

    /// Deregisters a session and fires `on_ended`, dropping the child handle
    /// WITHOUT killing it (R5.4). Dropping [`std::process::Child`] does not kill
    /// the process, so the viewer keeps running.
    pub fn stop_tracking(&self, session_id: Uuid) {
        self.finish(session_id);
    }

    /// Terminates an owned viewer child and deregisters it (R5.2/5.3).
    ///
    /// Returns `false` without doing anything for a session RustConn does not own
    /// (`child: None`, a detaching viewer, R5.5); the session is left unchanged.
    ///
    /// This never blocks the GTK main thread: it sends the kill signal and leaves
    /// the reap to the shared poll timer. In the common case the child has already
    /// exited by the immediate [`Self::try_reap`] below, so the sidebar clears at
    /// once; otherwise the emblem clears within one poll cycle (≤2 s, R4.3).
    #[must_use]
    pub fn disconnect(self: &Rc<Self>, session_id: Uuid) -> bool {
        {
            let mut map = self.sessions.borrow_mut();
            let Some(session) = map.get_mut(&session_id) else {
                return false;
            };
            // Not owned (detaching viewer) — cannot terminate (R5.5). Leave it.
            let Some(child) = session.child.as_mut() else {
                return false;
            };
            // ponytail: std::process::Child only offers kill() (SIGKILL); there is no
            // graceful SIGTERM step. Fine for closing a viewer window; upgrade to
            // nix::sys::signal if a SIGTERM→SIGKILL escalation is ever needed.
            if let Err(e) = child.kill() {
                tracing::warn!(
                    %session_id,
                    error = %e,
                    "Failed to signal external viewer; reaping anyway"
                );
            }
        }

        // SIGKILL is usually reaped within a millisecond: try once (non-blocking)
        // so the sidebar clears immediately. If the child has not exited yet, the
        // shared poll timer reaps it and fires `on_ended` within one cycle.
        if self.try_reap(session_id) {
            self.finish(session_id);
        } else {
            self.ensure_timer();
        }
        true
    }

    /// Non-blocking reap of a session's owned child.
    ///
    /// Returns `true` only if the child had already exited (and was reaped by the
    /// `try_wait` here). Never blocks and never removes the session entry.
    fn try_reap(&self, session_id: Uuid) -> bool {
        let mut map = self.sessions.borrow_mut();
        match map.get_mut(&session_id).and_then(|s| s.child.as_mut()) {
            Some(child) => matches!(child.try_wait(), Ok(Some(_))),
            None => false,
        }
    }

    /// Terminates every owned viewer child and finalizes all tracked sessions.
    ///
    /// Called on application shutdown (window close / `app.quit`): owned children
    /// are SIGKILLed so they do not outlive RustConn as orphans, and `on_ended`
    /// fires exactly once per session so each open history entry is closed.
    /// Detaching viewers (`child: None`) keep running — RustConn cannot terminate
    /// them (R5.7) — but their tracking and history entry are still closed.
    ///
    /// Idempotent: a second call finds an empty registry and does nothing, so it
    /// is safe if both the `close_request` and `app.quit` paths run.
    pub fn shutdown(&self) {
        let ids: Vec<Uuid> = self.sessions.borrow().keys().copied().collect();
        for id in ids {
            // Best-effort kill of an owned child, scoped so the borrow is
            // released before `finish` (which borrows the map again). Errors are
            // ignored: the child may have already exited. We then reap it so it
            // does not linger as a zombie while the app finishes quitting — and
            // so an idempotent second shutdown (or a `close_request` path that
            // does not actually exit) stays clean. `wait` returns almost
            // immediately because SIGKILL is delivered promptly.
            {
                let mut map = self.sessions.borrow_mut();
                if let Some(child) = map.get_mut(&id).and_then(|s| s.child.as_mut()) {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
            self.finish(id);
        }
    }

    /// Returns the number of active tracked sessions (owned and detaching).
    ///
    /// Used by the close-confirmation dialog so a window holding only tabless
    /// external-viewer sessions still warns before quitting (issue #209).
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.sessions.borrow().values().filter(|s| !s.ended).count()
    }

    /// Returns whether the connection has at least one active external session.
    #[must_use]
    pub fn has_active_session(&self, connection_id: Uuid) -> bool {
        self.sessions
            .borrow()
            .values()
            .any(|s| s.connection_id == connection_id && !s.ended)
    }

    /// Returns the session ids of the connection's active external sessions.
    #[must_use]
    pub fn active_session_ids(&self, connection_id: Uuid) -> Vec<Uuid> {
        self.sessions
            .borrow()
            .iter()
            .filter(|(_, s)| s.connection_id == connection_id && !s.ended)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Removes a session and fires `on_ended` exactly once.
    ///
    /// Returns whether the callback fired (i.e. the session existed and had not
    /// already ended). Removal is the primary exactly-once guard; the `ended`
    /// flag defends against re-entrant firing.
    fn finish(&self, session_id: Uuid) -> bool {
        let removed = self.sessions.borrow_mut().remove(&session_id);
        match removed {
            Some(session) if !session.ended => {
                (self.callbacks.on_ended)(session.connection_id, session.history_entry_id);
                true
            }
            _ => false,
        }
    }

    /// Returns whether any tracked session still owns a live child process.
    fn has_owned_child(&self) -> bool {
        self.sessions
            .borrow()
            .values()
            .any(|s| s.child.is_some() && !s.ended)
    }

    /// Starts the shared poll timer if it is not already running and there is at
    /// least one owned child to watch (R4.6).
    fn ensure_timer(self: &Rc<Self>) {
        if self.timer_running.get() || !self.has_owned_child() {
            return;
        }
        self.timer_running.set(true);
        let this = Rc::clone(self);
        glib::timeout_add_local(POLL_INTERVAL, move || this.poll_once());
    }

    /// One poll cycle: reap exited owned children, fire `on_ended` for each, and
    /// stop the timer when no owned children remain (R4.2/4.3/4.7).
    fn poll_once(&self) -> glib::ControlFlow {
        // Phase 1: collect exited session ids under a short borrow. try_wait needs
        // &mut Child, so borrow_mut here — but never hold the borrow across the
        // on_ended callbacks fired in phase 2.
        let exited: Vec<Uuid> = {
            let mut map = self.sessions.borrow_mut();
            map.iter_mut()
                .filter_map(|(id, session)| {
                    let child = session.child.as_mut()?;
                    match child.try_wait() {
                        Ok(Some(_status)) => Some(*id),
                        Ok(None) => None,
                        Err(e) => {
                            tracing::warn!(
                                session_id = %id,
                                error = %e,
                                "try_wait failed for external viewer; dropping tracking"
                            );
                            Some(*id)
                        }
                    }
                })
                .collect()
        };
        for id in exited {
            self.finish(id);
        }

        // Phase 2: stop once no owned children remain (detaching child: None
        // sessions do not keep the timer alive, R5.7).
        if self.has_owned_child() {
            glib::ControlFlow::Continue
        } else {
            self.timer_running.set(false);
            glib::ControlFlow::Break
        }
    }
}

#[cfg(test)]
mod tests {
    //! State-machine tests for [`ExternalSessionRegistry`] (design Properties 6–8).
    //!
    //! No GTK widgets are involved. `register` internally calls
    //! `glib::timeout_add_local`, which attaches to the thread-default main
    //! context, so every test runs inside a fresh [`glib::MainContext`] pushed as
    //! the thread default via [`run_in_ctx`]. The real timer never fires (no main
    //! loop runs); the poll logic is exercised by calling `poll_once` directly.
    //! Owned-child cases spawn short-lived real processes (`sleep`, `true`) and
    //! clean them up.

    use std::cell::{Cell, RefCell};
    use std::process::{Child, Command};
    use std::rc::Rc;

    use gtk4::glib;
    use uuid::Uuid;

    use super::{ExternalSessionCallbacks, ExternalSessionRegistry};

    /// Serializes every glib-context-touching test body.
    ///
    /// `register` calls `glib::timeout_add_local`, which acquires the thread-default
    /// main context. Under the default multi-threaded test harness two tests can race
    /// on context acquisition, tripping glib's "default main context already acquired
    /// by another thread" abort. Holding this lock for the whole test body makes the
    /// fresh-context tests mutually exclusive, so the suite is green without
    /// `--test-threads=1`.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Runs `f` inside a fresh thread-default `MainContext`, serialized against
    /// other context-touching tests via [`TEST_LOCK`].
    ///
    /// This guarantees `glib::timeout_add_local` (called from `register`) has a
    /// context to attach to, and that the created timer source is destroyed when
    /// the context is dropped at the end of the test — no leaked real timers.
    fn run_in_ctx<F: FnOnce()>(f: F) {
        // Poison-tolerant: an earlier panicking test must not wedge the rest.
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let ctx = glib::MainContext::new();
        ctx.with_thread_default(f)
            .expect("acquire fresh thread-default MainContext");
    }

    /// Observed callback firings, so tests can assert exactly-once semantics.
    struct Counters {
        registered: Rc<Cell<u32>>,
        ended: Rc<Cell<u32>>,
        ended_connections: Rc<RefCell<Vec<Uuid>>>,
    }

    /// Builds a registry whose callbacks bump the returned [`Counters`].
    fn make_registry() -> (Rc<ExternalSessionRegistry>, Counters) {
        let registered = Rc::new(Cell::new(0u32));
        let ended = Rc::new(Cell::new(0u32));
        let ended_connections = Rc::new(RefCell::new(Vec::new()));

        let on_registered = {
            let registered = Rc::clone(&registered);
            Box::new(move |_conn: Uuid| registered.set(registered.get() + 1))
        };
        let on_ended = {
            let ended = Rc::clone(&ended);
            let ended_connections = Rc::clone(&ended_connections);
            Box::new(move |conn: Uuid, _entry: Option<Uuid>| {
                ended.set(ended.get() + 1);
                ended_connections.borrow_mut().push(conn);
            })
        };

        let registry = ExternalSessionRegistry::new(ExternalSessionCallbacks {
            on_registered,
            on_ended,
        });
        (
            registry,
            Counters {
                registered,
                ended,
                ended_connections,
            },
        )
    }

    /// Spawns a long-lived child (`sleep 30`) that stays alive until killed.
    fn spawn_sleep() -> Child {
        Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn `sleep 30` for test")
    }

    /// Spawns a child that exits immediately (`true`).
    fn spawn_true() -> Child {
        Command::new("true").spawn().expect("spawn `true` for test")
    }

    /// Whether the given pid still refers to a live (non-reaped) process.
    ///
    /// Uses `kill -0` (portable Linux/macOS): success means the process exists.
    fn pid_alive(pid: u32) -> bool {
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Best-effort cleanup for a still-running test child.
    fn kill_pid(pid: u32) {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }

    /// Drives the poll loop until `pid` is reaped.
    ///
    /// `disconnect` is non-blocking (it kills, then leaves the reap to the shared
    /// timer). No real main loop runs in tests, so this pumps `poll_once` — the
    /// same work the timer would do — until the SIGKILLed child is reaped.
    fn drive_until_reaped(registry: &Rc<ExternalSessionRegistry>, pid: u32) {
        for _ in 0..40 {
            if !pid_alive(pid) {
                return;
            }
            let _ = registry.poll_once();
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    // Property 7 (no auto-kill on Stop tracking) + at-most-once on_ended.
    #[test]
    fn stop_tracking_ends_without_killing_owned_child() {
        run_in_ctx(|| {
            let (registry, counters) = make_registry();
            let child = spawn_sleep();
            let pid = child.id();
            let session_id = Uuid::new_v4();
            let connection_id = Uuid::new_v4();

            registry.register(session_id, connection_id, Some(child), None);
            assert_eq!(counters.registered.get(), 1);
            assert!(pid_alive(pid), "child should be running after register");

            registry.stop_tracking(session_id);

            assert_eq!(counters.ended.get(), 1, "on_ended fires exactly once");
            assert!(!registry.has_active_session(connection_id));
            assert!(
                pid_alive(pid),
                "stop_tracking must NOT kill the owned child (R5.4)"
            );

            kill_pid(pid);
        });
    }

    // Property 7 (ownership gate on kill) — owned child is terminated.
    #[test]
    fn disconnect_terminates_owned_child_and_returns_true() {
        run_in_ctx(|| {
            let (registry, counters) = make_registry();
            let child = spawn_sleep();
            let pid = child.id();
            let session_id = Uuid::new_v4();
            let connection_id = Uuid::new_v4();

            registry.register(session_id, connection_id, Some(child), None);
            assert!(pid_alive(pid));

            let terminated = registry.disconnect(session_id);
            assert!(terminated, "disconnect returns true for an owned child");

            // disconnect is non-blocking: it SIGKILLs and hands the reap to the
            // shared poll timer, pumped manually here (no main loop in tests).
            drive_until_reaped(&registry, pid);

            assert_eq!(counters.ended.get(), 1, "on_ended fires exactly once");
            assert!(!registry.has_active_session(connection_id));
            assert!(
                !pid_alive(pid),
                "disconnect must terminate and reap the owned child (R5.2/5.3)"
            );
        });
    }

    // Property 7 (ownership gate) — child: None is never killed / left unchanged.
    #[test]
    fn disconnect_no_ops_for_not_owned_session() {
        run_in_ctx(|| {
            let (registry, counters) = make_registry();
            let session_id = Uuid::new_v4();
            let connection_id = Uuid::new_v4();

            registry.register(session_id, connection_id, None, None);
            assert_eq!(counters.registered.get(), 1);

            let terminated = registry.disconnect(session_id);

            assert!(
                !terminated,
                "disconnect returns false for a not-owned session (R5.5)"
            );
            assert_eq!(counters.ended.get(), 0, "on_ended must not fire");
            assert!(
                registry.has_active_session(connection_id),
                "the detaching session stays active until Stop tracking (R5.7)"
            );
        });
    }

    // Property 6 — on_ended fires at most once per process across repeated calls.
    #[test]
    fn on_ended_fires_at_most_once_per_session() {
        run_in_ctx(|| {
            let (registry, counters) = make_registry();
            let session_id = Uuid::new_v4();
            let connection_id = Uuid::new_v4();

            registry.register(session_id, connection_id, None, None);
            registry.stop_tracking(session_id);
            assert_eq!(counters.ended.get(), 1);

            // Every subsequent end attempt on the same id is a no-op.
            registry.stop_tracking(session_id);
            assert!(!registry.disconnect(session_id));
            assert_eq!(
                counters.ended.get(),
                1,
                "on_ended never fires a second time"
            );
        });
    }

    // shutdown() kills owned children, finalizes every session exactly once,
    // and leaves the registry empty (app-quit cleanup, issue #209).
    #[test]
    fn shutdown_kills_owned_and_finalizes_all() {
        run_in_ctx(|| {
            let (registry, counters) = make_registry();
            let owned = spawn_sleep();
            let owned_pid = owned.id();
            let owned_session = Uuid::new_v4();
            let detaching_session = Uuid::new_v4();
            let connection_id = Uuid::new_v4();

            registry.register(owned_session, connection_id, Some(owned), None);
            registry.register(detaching_session, connection_id, None, None);
            assert_eq!(registry.active_count(), 2);

            registry.shutdown();

            // Both sessions finalized exactly once, registry drained.
            assert_eq!(counters.ended.get(), 2, "on_ended fires once per session");
            assert_eq!(registry.active_count(), 0);
            assert!(!registry.has_active_session(connection_id));

            // The owned child was signalled; wait briefly for the kernel to
            // deliver SIGKILL (it may still be a zombie, but no longer live).
            for _ in 0..40 {
                if !pid_alive(owned_pid) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            assert!(!pid_alive(owned_pid), "shutdown must kill the owned child");

            // A second shutdown is a no-op (idempotent).
            registry.shutdown();
            assert_eq!(counters.ended.get(), 2);
        });
    }

    #[test]
    fn active_session_tracking_follows_lifecycle() {
        run_in_ctx(|| {
            let (registry, _counters) = make_registry();
            let connection_id = Uuid::new_v4();
            let session_a = Uuid::new_v4();
            let session_b = Uuid::new_v4();

            assert!(!registry.has_active_session(connection_id));
            assert!(registry.active_session_ids(connection_id).is_empty());

            registry.register(session_a, connection_id, None, None);
            registry.register(session_b, connection_id, None, None);

            let mut ids = registry.active_session_ids(connection_id);
            ids.sort();
            let mut expected = vec![session_a, session_b];
            expected.sort();
            assert_eq!(ids, expected);
            assert!(registry.has_active_session(connection_id));

            registry.stop_tracking(session_a);
            assert_eq!(registry.active_session_ids(connection_id), vec![session_b]);
            assert!(registry.has_active_session(connection_id));

            registry.stop_tracking(session_b);
            assert!(!registry.has_active_session(connection_id));
            assert!(registry.active_session_ids(connection_id).is_empty());
        });
    }

    // Property 8 — the timer only runs for owned children.
    #[test]
    fn timer_stays_stopped_for_detaching_child_none() {
        run_in_ctx(|| {
            let (registry, _counters) = make_registry();
            registry.register(Uuid::new_v4(), Uuid::new_v4(), None, None);
            assert!(
                !registry.timer_running.get(),
                "a child: None registration must not start the poll timer"
            );
        });
    }

    #[test]
    fn timer_starts_when_owned_child_registered() {
        run_in_ctx(|| {
            let (registry, _counters) = make_registry();
            let child = spawn_sleep();
            let pid = child.id();
            let session_id = Uuid::new_v4();

            registry.register(session_id, Uuid::new_v4(), Some(child), None);
            assert!(
                registry.timer_running.get(),
                "registering an owned child starts the poll timer (R4.6)"
            );

            assert!(registry.disconnect(session_id));
            drive_until_reaped(&registry, pid);
            assert!(!pid_alive(pid));
        });
    }

    // Property 6 (exactly-once via poll) + Property 8 (timer stops empty / restarts).
    #[test]
    fn poll_reaps_exited_child_once_and_manages_timer() {
        run_in_ctx(|| {
            let (registry, counters) = make_registry();
            let session_id = Uuid::new_v4();
            let connection_id = Uuid::new_v4();

            registry.register(session_id, connection_id, Some(spawn_true()), None);
            assert!(registry.timer_running.get());

            // Drive the poll manually until the exited child is reaped. Looping
            // handles the small spawn/exit race without relying on the real timer.
            let mut broke = false;
            for _ in 0..40 {
                if matches!(registry.poll_once(), glib::ControlFlow::Break) {
                    broke = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }

            assert!(
                broke,
                "poll_once should Break once no owned children remain"
            );
            assert_eq!(
                counters.ended.get(),
                1,
                "on_ended fires exactly once for the exited process (R4.3)"
            );
            assert_eq!(
                counters.ended_connections.borrow().as_slice(),
                &[connection_id]
            );
            assert!(
                !registry.timer_running.get(),
                "timer stops when the registry has no owned children (R4.7)"
            );
            assert!(!registry.has_active_session(connection_id));

            // A further poll fires nothing more and stays stopped.
            assert!(matches!(registry.poll_once(), glib::ControlFlow::Break));
            assert_eq!(counters.ended.get(), 1);

            // Property 8 — registering a new owned child restarts the timer.
            let child = spawn_sleep();
            let restart_child_pid = child.id();
            let restart_session = Uuid::new_v4();
            registry.register(restart_session, connection_id, Some(child), None);
            assert!(
                registry.timer_running.get(),
                "register restarts the poll timer after it stopped (R4.6)"
            );
            assert!(registry.disconnect(restart_session));
            drive_until_reaped(&registry, restart_child_pid);
        });
    }
}
