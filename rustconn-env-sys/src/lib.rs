//! Isolated FFI helper for the two environment variables RustConn sets at startup
//! (`GSK_RENDERER` and `LANGUAGE`).
//!
//! `std::env::set_var` is `unsafe` in the 2024 edition because it is a wrapper
//! around `setenv(3)`, which mutates the process-global environment block
//! without synchronisation. A concurrent `getenv` — from any thread, including
//! one inside a C library RustConn links — can read a half-updated block, or
//! keep using a string `setenv` has already freed. Every other crate in this
//! workspace denies `unsafe_code`, so the call lives here, behind a guard that
//! refuses it outside the window in which it is sound. `deny` rather than
//! `forbid` is what lets this crate re-open the lint for itself while still
//! inheriting the workspace's clippy set; see the `[lints]` note in `Cargo.toml`.
//!
//! It exposes one operation, [`set_startup_var`], and one way to close the
//! window in which that operation is allowed, [`seal_env`]. Reading the
//! environment is safe and is done directly from the other crates with
//! `std::env::var`; deliberately nothing of that is re-exported here, so this
//! crate stays a wrapper around the one call that needs a contract.
//!
//! # Why RustConn needs to write the environment at all
//!
//! Two startup settings are exposed by their host library as an environment
//! variable and nothing else — no API exists to pass either one:
//!
//! * **`GSK_RENDERER`**, GTK's only interface for choosing a GSK renderer, read
//!   while the first surface is realised and therefore needed before `gtk_init`.
//!   RustConn selects the Cairo renderer in the two situations where the GPU path
//!   is known to be worse than software rasterisation: X11 compositors that paint
//!   popovers blank until hovered
//!   ([#85](https://github.com/totoshko88/RustConn/issues/85)), and macOS guests
//!   on Apple Silicon, whose paravirtualised GPU offers Metal but no accelerated
//!   OpenGL, leaving GSK's GL renderer on a software fallback
//!   ([#274](https://github.com/totoshko88/RustConn/issues/274)).
//! * **`LANGUAGE`**, GNU gettext's primary catalogue-selection mechanism, needed
//!   before the first translatable string is evaluated. It matters beyond
//!   `setlocale` because gettext honours it even when the named locale is not
//!   installed, which is the normal case inside a Flatpak sandbox
//!   ([#158](https://github.com/totoshko88/RustConn/issues/158)).
//!
//! Both used to be applied by re-execing the process with the variable set in the
//! child, which sidesteps `set_var` entirely. That works on Linux but is
//! unavailable on macOS: replacing the process image destroys the LaunchServices
//! scene registration `NSStatusItem` needs, and the tray icon silently
//! disappears. Setting the variables in-process is what lets both platforms share
//! one code path, at the cost of the guarded `unsafe` below — and it removes two
//! process spawns from startup.
//!
//! # The contract
//!
//! [`set_startup_var`] may be called only:
//!
//! * from `main()`, before any thread is spawned — that means before a tokio
//!   runtime, before GTK/GIO, and before the tracing subscriber;
//! * from the same thread every time;
//! * before [`seal_env`], which the caller invokes once the startup environment
//!   is final.
//!
//! A violation of a *checked* clause panics rather than silently corrupting the
//! environment block. A panic here is a programming bug (M-PANIC-ON-BUG), not a
//! runtime condition: it means an environment write moved out of the startup
//! window, which no user input can cause.
//!
//! # What is checked, and what is not
//!
//! Enforced on every platform:
//!
//! * **Thread identity** — a call from a thread other than the first caller is
//!   refused. This is the clause that still catches a regression off Linux: any
//!   caller that appears after startup necessarily runs on a GTK, GIO or tokio
//!   thread.
//! * **Sealing** — a call after [`seal_env`] is refused.
//!
//! Enforced on Linux only:
//!
//! * **Thread count growth** — `/proc/self/task` is counted on the first
//!   admitted call to establish a *baseline*; a later call refuses if the count
//!   has grown past it, which means RustConn's own code spawned a thread in
//!   between. The baseline tolerates threads that already exist when `main()`
//!   starts, because shared-library constructors legitimately create them during
//!   ELF `.init_array` execution before `main()` is entered (observed on Fedora
//!   44 with glibc 2.43 / OpenSSL 3.5, issue #271). No other platform is asked,
//!   macOS included: obtaining the count there means
//!   `proc_pidinfo(PROC_PIDTASKINFO)` or `task_threads()`, which would add
//!   `libc` and a hand-transcribed C struct layout to the crate whose entire
//!   purpose is to keep the unsafe surface small. Since 0.21.0 the `macos-sys`
//!   CI job does compile and test this crate on macOS, so such a block would be
//!   checked; the reason to leave it out is the unsafe budget rather than the
//!   absence of a runner. The check is
//!   defence-in-depth; what makes the call sound is the call site, and the
//!   thread-identity clause above still guards a regression.
//!
//! This mirrors [`rustconn-locale-sys`], which guards `setlocale(3)` under the
//! same conditions. The guard is deliberately duplicated rather than shared: a
//! common crate would make each `-sys` crate depend on something other than
//! `std`, and the whole point of both is to be small enough to audit in one
//! sitting. Any change to the reasoning above belongs in both.
//!
//! [`rustconn-locale-sys`]: https://github.com/totoshko88/RustConn/tree/main/rustconn-locale-sys

// The one lint this crate re-opens out of the inherited workspace set, which is
// `deny` rather than `forbid` precisely so that this line is possible. `expect`
// rather than `allow`: if the `unsafe` call below ever goes away, the compiler
// says so instead of leaving a stale exemption behind — and a `-sys` crate with
// no `unsafe` left has no reason to exist and should be folded into its caller.
#![expect(
    unsafe_code,
    reason = "sanctioned FFI crate (M-UNSAFE); the guarded setenv call is its entire purpose"
)]

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::ThreadId;

// ============================================================================
// Startup guard
// ============================================================================

/// Why a [`set_startup_var`] call was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    /// The startup window was closed by [`seal_env`].
    Sealed,
    /// The thread count grew beyond the baseline, meaning RustConn spawned a
    /// thread between calls to [`set_startup_var`].
    ThreadCountGrew {
        /// Thread count observed on the first admitted call.
        baseline: usize,
        /// Thread count observed on this (refused) call.
        current: usize,
    },
    /// An earlier call came from a different thread.
    ForeignThread,
}

impl Refusal {
    /// Renders the refusal as a panic message that names the fix.
    fn describe(self) -> String {
        match self {
            Self::Sealed => "setenv after seal_env(): the startup environment is final. \
                 Writing it later is unsound — every thread in the process, including \
                 those inside GTK and OpenSSL, may be reading it. Persist the setting \
                 and let it take effect on the next start instead."
                .to_string(),
            Self::ThreadCountGrew { baseline, current } => format!(
                "setenv refused: thread count grew from {baseline} (at first call) to \
                 {current}. A thread was spawned between environment writes, so a \
                 concurrent getenv can observe a half-updated environment block. Move \
                 this call earlier in main(), before GTK, tokio and the tracing \
                 subscriber start."
            ),
            Self::ForeignThread => "setenv from a different thread than the earlier calls: \
                 the startup environment must be written entirely from the main thread."
                .to_string(),
        }
    }
}

/// Enforces the startup window in which `setenv` is sound.
///
/// Kept as a struct rather than three loose statics so the contract can be
/// exercised against a local instance in tests — calling the real
/// [`set_startup_var`] from a test is impossible by design, since the test
/// harness is itself multi-threaded.
struct StartupGuard {
    sealed: AtomicBool,
    first_caller: OnceLock<ThreadId>,
    /// Thread count observed on the first admitted call.
    ///
    /// A shared library's ELF constructor can spawn a thread before `main()` is
    /// entered, so "the process has exactly one thread" is not a state an
    /// application can arrange (issue #271). The guard records whatever it finds
    /// on the first call and refuses only when the count *grows* past it — that
    /// is the case the call site controls: a thread this program started between
    /// two environment writes.
    baseline_threads: OnceLock<usize>,
}

impl StartupGuard {
    const fn new() -> Self {
        Self {
            sealed: AtomicBool::new(false),
            first_caller: OnceLock::new(),
            baseline_threads: OnceLock::new(),
        }
    }

    /// Admits one `setenv` call, or reports why it must not happen.
    ///
    /// `live_threads` is the caller's thread count if the platform can report
    /// one; `None` skips that check rather than assuming the process is
    /// single-threaded.
    fn admit(&self, current: ThreadId, live_threads: Option<usize>) -> Result<(), Refusal> {
        if self.sealed.load(Ordering::Acquire) {
            return Err(Refusal::Sealed);
        }

        // Thread-count check: refuse if the count grew beyond the baseline
        // established on the first call. This catches application-spawned
        // threads (tokio, GTK/GIO workers) while tolerating pre-existing
        // threads from shared-library constructors.
        if let Some(count) = live_threads {
            let baseline = *self.baseline_threads.get_or_init(|| count);
            if count > baseline {
                return Err(Refusal::ThreadCountGrew {
                    baseline,
                    current: count,
                });
            }
        }

        if *self.first_caller.get_or_init(|| current) != current {
            return Err(Refusal::ForeignThread);
        }

        Ok(())
    }

    fn seal(&self) {
        self.sealed.store(true, Ordering::Release);
    }

    /// Test-only: nothing in the crate's API needs to read the seal, and there
    /// is no public accessor for it either (see [`seal_env`]). The tests do, to
    /// check that `seal` and `admit` agree about the same bit.
    #[cfg(test)]
    fn is_sealed(&self) -> bool {
        self.sealed.load(Ordering::Acquire)
    }
}

static GUARD: StartupGuard = StartupGuard::new();

/// Counts the threads in this process, where the OS makes that cheap to ask.
///
/// Linux exposes one directory entry per thread under `/proc/self/task`. On
/// macOS — and on any other platform, and in any sandbox without `/proc` — this
/// returns `None`, the thread-count check is skipped, and only the
/// thread-identity and sealing guards apply. See the crate-level "What is
/// checked, and what is not" for why macOS is not implemented.
fn live_thread_count() -> Option<usize> {
    #[cfg(target_os = "linux")]
    {
        // Counts every yielded entry, including `Err`s: a miscount must never
        // read as "fewer threads than there are".
        std::fs::read_dir("/proc/self/task")
            .ok()
            .map(Iterator::count)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

// ============================================================================
// Environment setup
// ============================================================================

/// Sets the environment variable `name` to `value` for the rest of the process.
///
/// Intended for variables that a C library reads later during its own
/// initialisation, where no API-level alternative exists — `GSK_RENDERER` and
/// `LANGUAGE` being the two cases this crate was written for.
///
/// # Panics
///
/// Panics if one of the *checked* clauses of the contract described at the
/// [crate] level is violated: the call comes from a different thread than
/// earlier calls, [`seal_env`] has already run, or — on Linux, where the count
/// is available — the thread count grew beyond the baseline established on the
/// first call (meaning application code spawned a thread).
///
/// Also panics if `name` is empty, contains `'='`, or either argument contains a
/// NUL byte, none of which can be passed to C. These mirror the panics
/// `std::env::set_var` performs itself; they are listed because a caller passing
/// a computed name should know they exist.
pub fn set_startup_var(name: &str, value: &str) {
    if let Err(refusal) = GUARD.admit(std::thread::current().id(), live_thread_count()) {
        panic!("rustconn-env-sys: {}", refusal.describe());
    }

    // SAFETY: `setenv` mutates the process-global environment block without
    // synchronisation, so it is sound only while nothing else can read that
    // block concurrently. What the guard immediately above has established,
    // exactly:
    //
    //  * the startup window is still open — `seal_env` has not run;
    //  * this is the thread that made every earlier call;
    //  * on Linux, and only there, `/proc/self/task` reported a thread count
    //    that has not grown beyond the baseline established on the first call.
    //
    //    Threads that already existed at that first call are tolerated rather
    //    than refused, and this is the one clause that is a judgement rather
    //    than a proof. A shared library's ELF constructor can spawn a thread
    //    before `main()` is entered — observed on Fedora 44 (issue #271) — and
    //    such a thread is not reachable from here to inspect: whether it reads
    //    the environment is not something this crate can establish. What it can
    //    establish is that nothing *this program* started is running, which is
    //    the case the call site controls and the case a regression would create.
    //
    //    Elsewhere — macOS — that count is unavailable and the absence of a new
    //    thread is *not* verified here; it follows from the call sites instead.
    //    There are two, `rustconn::i18n::apply_configured_language` (LANGUAGE)
    //    and `rustconn::renderer::apply_renderer_preference` (GSK_RENDERER), in
    //    that order. Both run from `main()` before GTK, GIO, tokio or the
    //    tracing subscriber start, and nothing between them spawns a thread; any
    //    call that appeared later would come from one of those threads and be
    //    refused by the thread-identity check above. The second of the two also
    //    calls `seal_env`, so a third caller added after startup is refused by
    //    the seal rather than relying on anyone noticing this comment.
    //
    // `name` and `value` are copied into NUL-terminated buffers before the C
    // call, so no borrow outlives it.
    unsafe { std::env::set_var(name, value) }
}

/// Closes the startup window: any later [`set_startup_var`] call panics.
///
/// Call this from `main()` once the startup environment is final and before
/// starting GTK, tokio or any other thread. It turns "someone added an
/// environment write to a running application" from latent memory unsoundness
/// into an immediate, obvious crash during development.
///
/// Idempotent, and safe to call from anywhere.
///
/// There is deliberately no public way to ask whether the seal is closed: the
/// guard reports a violation itself, with a message that names the fix, so no
/// caller needs to interrogate the state before calling.
pub fn seal_env() {
    GUARD.seal();
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// The main thread of a single-threaded process is admitted, repeatedly:
    /// startup may set more than one variable.
    #[test]
    fn admits_repeated_calls_from_the_startup_thread() {
        let guard = StartupGuard::new();
        let me = std::thread::current().id();
        assert_eq!(guard.admit(me, Some(1)), Ok(()));
        assert_eq!(guard.admit(me, Some(1)), Ok(()));
        assert_eq!(guard.admit(me, Some(1)), Ok(()));
    }

    /// Pre-existing threads from shared-library constructors are tolerated: the
    /// first call establishes a baseline, and the same count on subsequent calls
    /// is fine (issue #271 — Fedora 44).
    #[test]
    fn admits_when_library_threads_exist_at_baseline() {
        let guard = StartupGuard::new();
        let me = std::thread::current().id();
        assert_eq!(guard.admit(me, Some(2)), Ok(()));
        assert_eq!(guard.admit(me, Some(2)), Ok(()));
        // Even if the count dips (a library thread exited), that is fine too.
        assert_eq!(guard.admit(me, Some(1)), Ok(()));
    }

    /// Contract test (M-UNSAFE): the check that makes the `unsafe` call sound.
    /// A thread count that *grows* beyond the baseline is refused — that means
    /// application code spawned a thread between calls.
    #[test]
    fn refuses_when_thread_count_grows_beyond_baseline() {
        let guard = StartupGuard::new();
        let me = std::thread::current().id();
        assert_eq!(guard.admit(me, Some(1)), Ok(()));
        assert_eq!(
            guard.admit(me, Some(2)),
            Err(Refusal::ThreadCountGrew {
                baseline: 1,
                current: 2
            })
        );
    }

    /// Even with a higher baseline, growth is still detected.
    #[test]
    fn refuses_growth_from_elevated_baseline() {
        let guard = StartupGuard::new();
        let me = std::thread::current().id();
        assert_eq!(guard.admit(me, Some(3)), Ok(()));
        assert_eq!(guard.admit(me, Some(3)), Ok(()));
        assert_eq!(
            guard.admit(me, Some(4)),
            Err(Refusal::ThreadCountGrew {
                baseline: 3,
                current: 4
            })
        );
    }

    /// A call from a thread other than the first caller is refused even if the
    /// thread count is unavailable, so a platform without `/proc` is not left
    /// unguarded.
    #[test]
    fn refuses_a_second_thread_identity_without_a_thread_count() {
        let guard = StartupGuard::new();
        let me = std::thread::current().id();
        assert_eq!(guard.admit(me, None), Ok(()));

        let other = std::thread::spawn(|| std::thread::current().id())
            .join()
            .expect("the probe thread only reads its own id");
        assert_eq!(guard.admit(other, None), Err(Refusal::ForeignThread));

        // The rejected caller must not have taken ownership of the guard.
        assert_eq!(guard.admit(me, None), Ok(()));
    }

    /// Sealing closes the window permanently, including for the thread that
    /// legitimately used it.
    #[test]
    fn refuses_everything_after_sealing() {
        let guard = StartupGuard::new();
        let me = std::thread::current().id();
        assert_eq!(guard.admit(me, Some(1)), Ok(()));
        assert!(!guard.is_sealed());

        guard.seal();

        assert!(guard.is_sealed());
        assert_eq!(guard.admit(me, Some(1)), Err(Refusal::Sealed));
        // Idempotent.
        guard.seal();
        assert_eq!(guard.admit(me, Some(1)), Err(Refusal::Sealed));
    }

    /// How many threads the count test holds alive while it samples. Three
    /// rather than one so that an implementation answering some small constant
    /// is caught as well as one answering nothing.
    #[cfg(target_os = "linux")]
    const HELD_PROBE_THREADS: usize = 3;

    /// The smallest honest answer while those probes are parked: the probes,
    /// plus the thread sampling them.
    #[cfg(target_os = "linux")]
    const MINIMUM_LIVE_THREADS_WITH_PROBES: usize = HELD_PROBE_THREADS + 1;

    /// The thread count must reflect reality on Linux, otherwise the guard above
    /// is decorative.
    ///
    /// The count is measured against threads this test creates and holds alive,
    /// not against the harness's own: with `--test-threads=1` libtest may run
    /// tests inline on the main thread, and threads spawned by other tests are
    /// joined and reaped, so "the harness is multi-threaded" is not something a
    /// test may assert. This shape passes under any `--test-threads` value.
    #[cfg(target_os = "linux")]
    #[test]
    fn thread_count_reflects_threads_this_test_holds_alive() {
        use std::sync::mpsc;

        // True whatever the harness is doing: the thread asking is itself alive.
        let baseline = live_thread_count().expect("/proc/self/task is readable on Linux");
        assert!(
            baseline >= 1,
            "the calling thread must be counted, so the count cannot be {baseline}"
        );

        let mut probes = Vec::with_capacity(HELD_PROBE_THREADS);
        let mut releases = Vec::with_capacity(HELD_PROBE_THREADS);
        for _ in 0..HELD_PROBE_THREADS {
            let (started_tx, started_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel::<()>();
            probes.push(std::thread::spawn(move || {
                // Announce, then park until released, so the thread is
                // certainly present in /proc/self/task while we sample.
                started_tx
                    .send(())
                    .expect("the sampling thread is still waiting for this");
                let _ = release_rx.recv();
            }));
            started_rx
                .recv()
                .expect("each probe thread announces itself before parking");
            releases.push(release_tx);
        }

        let with_probes = live_thread_count().expect("/proc/self/task is readable on Linux");
        // Deliberately a floor, not `with_probes > baseline`: the other tests in
        // this binary run concurrently by default, and one of their threads
        // exiting between the two samples could defeat a strict rise — the same
        // dependence on the harness this test exists to remove. The floor holds
        // unconditionally, because these threads are alive right now.
        assert!(
            with_probes >= MINIMUM_LIVE_THREADS_WITH_PROBES,
            "{HELD_PROBE_THREADS} probe threads plus this one are alive, \
             so a working count cannot be {with_probes}"
        );

        // Dropping the senders wakes every probe out of `recv`.
        drop(releases);
        for probe in probes {
            probe.join().expect("a probe thread only parks and returns");
        }

        // Still answering once they are gone.
        let after = live_thread_count();
        assert!(
            after.is_some_and(|count| count >= 1),
            "the count must survive the probe threads exiting, got {after:?}"
        );
    }

    /// Off Linux the count is unavailable by design, not by accident: `admit`
    /// then falls back to the identity and sealing checks. Pinning it keeps the
    /// crate's "What is checked, and what is not" section honest.
    #[cfg(not(target_os = "linux"))]
    #[test]
    fn thread_count_is_unavailable_off_linux() {
        assert_eq!(
            live_thread_count(),
            None,
            "no non-Linux count is implemented; if one is added, update the crate docs \
             and the SAFETY comment in set_startup_var"
        );
    }

    /// Every refusal explains itself and names the call it is about, so a
    /// developer who trips the guard does not have to read this crate to
    /// understand why.
    #[test]
    fn refusals_explain_themselves() {
        for refusal in [
            Refusal::Sealed,
            Refusal::ThreadCountGrew {
                baseline: 1,
                current: 4,
            },
            Refusal::ForeignThread,
        ] {
            let text = refusal.describe();
            assert!(text.contains("setenv"), "{text}");
        }
    }
}
