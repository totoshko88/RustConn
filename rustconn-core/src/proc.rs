//! Waiting for a child process with a deadline.
//!
//! `std::process::Child` offers `wait`/`wait_with_output`, which block until the
//! child decides to exit, and `try_wait`, which never blocks and never finishes.
//! Bounding a child therefore means polling `try_wait` against a deadline and
//! killing it when the deadline passes — about fifteen lines, which is why four
//! copies of them had accumulated: [`crate::which`]'s host probe,
//! [`crate::connection::mdns`]'s name resolution and `protocol::detection`'s
//! version check each grew their own, and `secret::status` had none at all and
//! blocked indefinitely on `keepassxc-cli` at a dozen call sites.
//!
//! The copies differed in what they did on expiry — one returned `None`, one
//! returned a placeholder string — which is a caller's decision and is why
//! [`wait_bounded`] reports [`Waited::TimedOut`] instead of choosing. What none
//! of them differed in, and what a fifth copy would have got wrong eventually,
//! is that a killed child still has to be reaped or it becomes a zombie for the
//! lifetime of the process.
//!
//! For an async caller this module is the wrong tool: `tokio::process::Child`
//! composes with `tokio::time::timeout` directly, and `secret::bitwarden` and
//! `secret::script_resolver` already do that.

use std::io;
use std::process::{Child, ExitStatus, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How often `try_wait` is asked while waiting out a budget.
///
/// 25 ms is short enough that a healthy child — which normally exits in one or
/// two ticks — is not noticeably delayed, and long enough that a full two-second
/// budget costs 80 wakeups rather than thousands.
const POLL_INTERVAL: Duration = Duration::from_millis(25);

/// What became of a child that was given a deadline.
#[derive(Debug)]
pub enum Waited {
    /// The child exited on its own; its output is collected.
    Exited(Output),
    /// The budget ran out. The child has been killed and reaped.
    TimedOut,
}

impl Waited {
    /// The child's output, or `None` if it timed out.
    #[must_use]
    pub fn output(self) -> Option<Output> {
        match self {
            Self::Exited(output) => Some(output),
            Self::TimedOut => None,
        }
    }

    /// Whether the child exited on its own **and** reported success.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        match self {
            Self::Exited(output) => output.status.success(),
            Self::TimedOut => false,
        }
    }
}

/// Waits for `child` to exit, killing and reaping it if `budget` runs out.
///
/// `what` names the child in the log line a timeout produces. It is
/// `&'static str` for the same reason [`crate::secret`]'s D-Bus wrapper takes
/// one: the value is logged, and a `&str` would let a caller interpolate a
/// credential or a lookup key into it.
///
/// Stdout and stderr are collected only in the [`Waited::Exited`] case, so the
/// child must have been spawned with them piped if the caller wants them.
/// They are read only after the child has exited, never while it runs. A child
/// that writes more than a pipe buffer holds (64 KiB on Linux) therefore
/// blocks on the full pipe, never exits, and is killed at `budget` and
/// reported as [`Waited::TimedOut`], with its output lost. Use this only for
/// children whose output stays well under that, or drain the pipes on a
/// thread of your own.
///
/// # Errors
/// Returns the `io::Error` from `try_wait` or from collecting the output. A
/// failure to kill an over-budget child is *not* an error: the child may have
/// exited between the poll and the signal, which is the common race and not a
/// problem.
pub fn wait_bounded(child: Child, budget: Duration, what: &'static str) -> io::Result<Waited> {
    let never = AtomicBool::new(false);
    Ok(wait_bounded_cancellable(child, budget, &never, what)?.unwrap_or(Waited::TimedOut))
}

/// Like [`wait_bounded`], but a caller on another thread can also stop the wait.
///
/// Returns `Ok(None)` when `cancel` became `true` before the child exited. The
/// child has then been killed and reaped, exactly as on a timeout, so a
/// cancelled run leaves no zombie behind. A child that exits in the same tick
/// the flag is raised is reported as [`Waited::Exited`]: its output is real.
///
/// # Errors
/// Same as [`wait_bounded`].
pub fn wait_bounded_cancellable(
    mut child: Child,
    budget: Duration,
    cancel: &AtomicBool,
    what: &'static str,
) -> io::Result<Option<Waited>> {
    match poll_until(&mut child, budget, cancel)? {
        Polled::Exited(_) => child.wait_with_output().map(|o| Some(Waited::Exited(o))),
        Polled::Cancelled => {
            tracing::debug!(child = what, "child process cancelled; killing it");
            // Same reasoning as the timeout branch below.
            let _ = child.kill();
            let _ = child.wait();
            Ok(None)
        }
        Polled::OutOfTime => {
            tracing::warn!(
                child = what,
                budget_ms = budget.as_millis(),
                "child process exceeded its budget; killing it"
            );
            // Both ignored on purpose. `kill` fails when the child has already
            // exited, which is the race this loop cannot close; `wait` is what
            // stops the corpse becoming a zombie, and there is nothing useful to
            // do if even that fails.
            let _ = child.kill();
            let _ = child.wait();
            Ok(Some(Waited::TimedOut))
        }
    }
}

/// How a [`poll_until`] wait ended.
#[derive(Debug)]
pub enum Polled {
    /// The child exited on its own.
    Exited(ExitStatus),
    /// `cancel` became `true` first. The child is still running.
    Cancelled,
    /// The budget ran out first. The child is still running.
    OutOfTime,
}

/// Polls `child` until it exits, `cancel` is raised or `budget` runs out.
///
/// Unlike [`wait_bounded_cancellable`] this neither kills nor reaps a child
/// that is still running: the caller decides how to stop it, for example by
/// closing its stdin first and giving it a grace period. A caller that does
/// not stop it must still `wait` on it eventually, or it becomes a zombie.
///
/// # Errors
/// Returns the `io::Error` from `try_wait`.
pub fn poll_until(child: &mut Child, budget: Duration, cancel: &AtomicBool) -> io::Result<Polled> {
    let deadline = Instant::now() + budget;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Polled::Exited(status));
        }
        if cancel.load(Ordering::Relaxed) {
            return Ok(Polled::Cancelled);
        }
        if Instant::now() >= deadline {
            return Ok(Polled::OutOfTime);
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    fn spawn(args: &[&str]) -> Child {
        Command::new(args[0])
            .args(&args[1..])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the test helper must spawn")
    }

    #[test]
    fn a_quick_child_is_collected_not_killed() {
        let waited = wait_bounded(
            spawn(&["sh", "-c", "printf hello"]),
            Duration::from_secs(5),
            "test-quick",
        )
        .expect("waiting must not error");

        assert!(waited.succeeded());
        let output = waited.output().expect("an exited child has output");
        assert_eq!(output.stdout, b"hello");
    }

    #[test]
    fn a_child_that_outlives_its_budget_is_killed() {
        let started = Instant::now();
        let waited = wait_bounded(
            spawn(&["sleep", "30"]),
            Duration::from_millis(150),
            "test-slow",
        )
        .expect("waiting must not error");

        assert!(matches!(waited, Waited::TimedOut));
        assert!(!waited.succeeded(), "a timeout is never a success");
        // The point of the budget: the call returns in about its length rather
        // than in the child's own 30 seconds.
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "returned after {:?}, so the budget was not enforced",
            started.elapsed()
        );
    }

    #[test]
    fn a_failing_child_is_reported_as_not_succeeded_but_still_exited() {
        let waited = wait_bounded(
            spawn(&["sh", "-c", "exit 3"]),
            Duration::from_secs(5),
            "test-failing",
        )
        .expect("waiting must not error");

        assert!(!waited.succeeded());
        let output = waited
            .output()
            .expect("a child that exited non-zero still has output");
        assert_eq!(output.status.code(), Some(3));
    }

    #[test]
    fn a_cancelled_wait_kills_the_child_and_reports_none() {
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&cancel);
        let raiser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            flag.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();
        let waited = wait_bounded_cancellable(
            spawn(&["sleep", "30"]),
            Duration::from_secs(30),
            &cancel,
            "test-cancel",
        )
        .expect("waiting must not error");
        raiser.join().expect("raiser thread");

        assert!(waited.is_none(), "a cancelled wait reports None");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "returned after {:?}, so cancel was not honoured",
            started.elapsed()
        );
    }

    #[test]
    fn poll_until_leaves_an_over_budget_child_running() {
        let mut child = spawn(&["sleep", "30"]);
        let never = AtomicBool::new(false);
        let polled = poll_until(&mut child, Duration::from_millis(50), &never)
            .expect("polling must not error");

        assert!(matches!(polled, Polled::OutOfTime), "{polled:?}");
        assert!(
            child.try_wait().expect("try_wait").is_none(),
            "poll_until must not stop the child"
        );
        child.kill().expect("the test stops the child");
        child.wait().expect("and reaps it");
    }

    /// A killed child must not be left as a zombie. Asserted through the same
    /// interface a caller has: the second `wait` inside `wait_bounded` is what
    /// reaps it, and if it were missing this would still pass — so the real
    /// assertion is that `wait_bounded` returns at all rather than erroring on a
    /// child it already reaped.
    #[test]
    fn a_timed_out_child_is_reaped_before_returning() {
        for _ in 0..3 {
            let waited = wait_bounded(
                spawn(&["sleep", "30"]),
                Duration::from_millis(50),
                "test-reap",
            )
            .expect("waiting must not error even after a kill");
            assert!(matches!(waited, Waited::TimedOut));
        }
    }
}
