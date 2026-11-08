//! Performance timing for vault and SSH operations (`rustconn::perf`).
//!
//! The 0.24 release is a performance release, and its targets are measured, not
//! felt: every vault operation and every SSH-connect phase is timed by a
//! [`PerfSpan`], which on drop emits one structured line to the [`PERF_TARGET`]
//! tracing target. The line carries fields, never a formatted string
//! (M-LOG-STRUCTURED), so a baseline table and a post-change table can be
//! compared field by field rather than parsed out of prose.
//!
//! The target is off by default: the application initialises its subscriber from
//! `RUST_LOG` (`tracing_subscriber::EnvFilter::from_default_env`), so these
//! lines appear only under `RUST_LOG=rustconn::perf=info`. That is deliberate —
//! the perf stream is a measurement instrument switched on for a benchmarking
//! run, not noise added to every session's log.
//!
//! # What a line never carries
//!
//! A `PerfSpan` field is a timing fact: an operation name, a transport name, an
//! elapsed millisecond count, a call count. It is never a secret, a credential, a
//! vault item name, a hostname, or a username. The timing of an operation is not
//! sensitive; what it operated on is. Keep it that way: pass only the fixed
//! `&'static str` labels the constructors take, never user data.

use std::time::Instant;

/// The `tracing` target every performance line is emitted under.
///
/// A single constant so the target cannot drift between call sites and so a
/// subscriber directive (`RUST_LOG=rustconn::perf=info`) has one spelling to
/// match. The `::` form groups it under the crate's logical namespace without
/// being a real Rust module path.
pub const PERF_TARGET: &str = "rustconn::perf";

/// A timed scope that logs its duration to [`PERF_TARGET`] when dropped.
///
/// Construct one at the start of an operation and hold it for the operation's
/// lifetime; its [`Drop`] records `op`, `transport`, `elapsed_ms` and `calls`.
/// Dropping is the only thing that emits — there is no explicit finish call to
/// forget and no second line if the scope ends early on a `?`.
///
/// `op` and `transport` are fixed labels (see the module note on what a line must
/// never carry). `calls` defaults to 1 — one logical operation — and is raised
/// with [`PerfSpan::add_calls`] when the scope fans out into several underlying
/// calls (several `bw` spawns behind one store, several agent round trips behind
/// one key lookup), which is exactly the count the release is driving down.
///
/// # Example
///
/// ```
/// use rustconn_core::tracing::perf::PerfSpan;
///
/// let span = PerfSpan::new("vault.retrieve", "bitwarden");
/// // ... do the work ...
/// drop(span); // emits: op="vault.retrieve" transport="bitwarden" elapsed_ms=.. calls=1
/// ```
#[derive(Debug)]
pub struct PerfSpan {
    op: &'static str,
    transport: &'static str,
    calls: u32,
    started: Instant,
}

impl PerfSpan {
    /// Starts a timed scope for `op` over `transport`.
    ///
    /// Both labels are fixed strings chosen at the call site — `"vault.store"`,
    /// `"connect.resolve"`, `"bitwarden"`, `"ssh-agent"`. The clock starts
    /// now; the line is emitted on drop.
    #[must_use]
    pub fn new(op: &'static str, transport: &'static str) -> Self {
        Self {
            op,
            transport,
            calls: 1,
            started: Instant::now(),
        }
    }

    /// Adds to the underlying-call count recorded on drop.
    ///
    /// Use it when one logical operation performed several concrete calls whose
    /// number is the thing being measured: `span.add_calls(2)` after a store that
    /// spawned `bw` twice. Saturating, because a count can only ever grow and an
    /// overflow here must not panic in the middle of a hot path (M-PANIC-ON-BUG).
    pub fn add_calls(&mut self, extra: u32) {
        self.calls = self.calls.saturating_add(extra);
    }

    /// The elapsed time so far, without ending the scope.
    ///
    /// For a caller that needs the number for its own logic (a threshold check)
    /// as well as the logged line. Reading it does not stop the clock.
    #[must_use]
    pub fn elapsed_ms(&self) -> u128 {
        self.started.elapsed().as_millis()
    }
}

impl Drop for PerfSpan {
    fn drop(&mut self) {
        tracing::info!(
            target: PERF_TARGET,
            op = self.op,
            transport = self.transport,
            elapsed_ms = self.started.elapsed().as_millis(),
            calls = self.calls,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_is_the_documented_namespace() {
        assert_eq!(PERF_TARGET, "rustconn::perf");
    }

    #[test]
    fn new_span_starts_at_one_call() {
        let span = PerfSpan::new("vault.retrieve", "bitwarden");
        assert_eq!(span.calls, 1);
    }

    #[test]
    fn add_calls_accumulates() {
        let mut span = PerfSpan::new("vault.store", "bitwarden");
        span.add_calls(2);
        span.add_calls(1);
        assert_eq!(span.calls, 4);
    }

    #[test]
    fn add_calls_saturates_instead_of_panicking() {
        let mut span = PerfSpan::new("vault.store", "bitwarden");
        span.add_calls(u32::MAX);
        span.add_calls(5);
        assert_eq!(span.calls, u32::MAX);
    }

    #[test]
    fn elapsed_is_readable_without_dropping() {
        let span = PerfSpan::new("connect.resolve", "credentials");
        // Does not panic and does not end the scope; the exact value is a clock
        // reading, so only its existence is asserted.
        let _ = span.elapsed_ms();
        let _ = span.elapsed_ms();
    }
}
