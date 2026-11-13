//! Isolated FFI helpers for RustConn's PTY layer.
//!
//! This crate is one of the workspace's five sanctioned locations for `unsafe`
//! code (per the M-UNSAFE guideline), alongside `rustconn-locale-sys` for the
//! startup `setlocale` call, `rustconn-env-sys` for the startup environment
//! writes, `rustconn-dock-sys` for the macOS Dock tile image, and
//! `rustconn-lock-sys` for the macOS screen-lock and sleep notifications. It
//! was the first, hence the wording it used to carry. It provides:
//!
//! - [`set_controlling_terminal`] — `pre_exec` hook for `setsid` + `TIOCSCTTY`
//! - [`open_pty_pair`] — creates a PTY master/slave pair via `openpty(2)`
//! - [`pty_set_winsize`] — sends `TIOCSWINSZ` to size the terminal
//! - [`pty_wait_readable`] — `poll(2)` with a timeout, so a reader thread can stop
//! - [`dup_fd`] — close-on-exec duplicate of a descriptor
//!
//! Reading and writing the descriptor itself is deliberately absent: the caller
//! turns the master into a [`std::fs::File`], so no session data passes through
//! any `unsafe` code.

// The one lint this crate re-opens out of the inherited workspace set, which is
// `deny` rather than `forbid` precisely so that this line is possible. `expect`
// rather than `allow`: if the `unsafe` blocks below ever go away, the compiler
// says so instead of leaving a stale exemption behind — and a `-sys` crate with
// no `unsafe` left has no reason to exist and should be folded into its caller.
#![expect(
    unsafe_code,
    reason = "sanctioned FFI crate (M-UNSAFE); the libc calls below are its entire purpose"
)]

use std::io;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};

// ============================================================================
// Controlling terminal (macOS PTY fix, #175)
// ============================================================================

#[cfg(unix)]
mod controlling_terminal {
    use std::io;
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    /// `TIOCSCTTY`, typed as `ioctl`'s request parameter.
    ///
    /// Split by platform because the two need different expressions, not merely
    /// different spellings of one. macOS declares the constant as `c_uint` and
    /// glibc as `c_ulong`, while `ioctl` takes `c_ulong` on both — so macOS needs
    /// a widening conversion and glibc needs none, and a conversion that is a
    /// no-op is itself a lint.
    ///
    /// Verified on `aarch64-apple-darwin`: at the call site the widening form
    /// tripped `clippy::cast_lossless` whichever way the target type was written,
    /// which is what ruled out collapsing the two arms. The glibc arm is the
    /// one every Linux CI job builds; it is the expression that needs no
    /// conversion at all.
    ///
    /// Not `unsafe`, and not a precondition guard: it only names the integer
    /// handed to `ioctl`. A type mismatch is a compile error, so there is nothing
    /// for a test to assert.
    ///
    /// `musl` types the constant, and `ioctl`'s parameter, as `c_int`, so neither
    /// arm would compile there. No target of this project is musl, and the
    /// `TIOCSWINSZ` call has always had the same property.
    #[cfg(target_os = "macos")]
    const TIOCSCTTY_REQUEST: libc::c_ulong = libc::TIOCSCTTY as libc::c_ulong;

    /// `TIOCSCTTY`, typed as `ioctl`'s request parameter — already `c_ulong` with
    /// glibc, so no conversion. See the macOS arm above for why this is split.
    #[cfg(not(target_os = "macos"))]
    const TIOCSCTTY_REQUEST: libc::c_ulong = libc::TIOCSCTTY;

    /// Arranges for `cmd`'s child to acquire its standard input terminal as a
    /// controlling terminal.
    ///
    /// The child is placed in a new session via `setsid(2)` and then claims
    /// the terminal on file descriptor 0 with the `TIOCSCTTY` ioctl. This lets
    /// interactive programs (notably `ssh`) open `/dev/tty` to prompt for a
    /// password. Without it, a child of a GUI process that has no controlling
    /// terminal cannot read the password and authentication fails instantly.
    ///
    /// # Preconditions
    ///
    /// * The caller MUST connect a PTY slave to the child's stdin (fd 0)
    ///   before spawning (e.g. via [`Command::stdin`]).
    /// * The caller MUST NOT also set [`CommandExt::process_group`]: `setsid(2)`
    ///   fails with `EPERM` when the calling process is already a process-group
    ///   leader. `setsid(2)` already makes the child a session and
    ///   process-group leader, which is sufficient for job control
    ///   (`Ctrl-C` → `SIGINT` to the foreground group).
    pub fn set_controlling_terminal(cmd: &mut Command) {
        // The hook is defined outside the `unsafe` block below on purpose. An
        // `unsafe` block extends lexically into a closure body, so writing the
        // closure inline would put these two calls inside the block that
        // registers it — one block covering three distinct contracts, which is
        // what `clippy::multiple_unsafe_ops_per_block` objects to. Defining it
        // here lets each call carry the SAFETY comment that actually applies to
        // it.
        let hook = || -> io::Result<()> {
            // New session: detach from any inherited controlling terminal
            // and become a session + process-group leader.
            //
            // SAFETY: `setsid` takes no arguments and mutates only the calling
            // process's own session and process-group membership. It is
            // async-signal-safe, which is the requirement that matters here
            // because this runs in the forked child.
            if unsafe { libc::setsid() } == -1 {
                return Err(io::Error::last_os_error());
            }
            // Claim fd 0 (the PTY slave) as the controlling terminal.
            //
            // SAFETY: fd 0 is the PTY slave the caller is required to have
            // wired up before spawning (see this function's docs). `TIOCSCTTY`
            // takes no pointer argument, so the third parameter is an ignored
            // integer and there is no buffer for the kernel to write through.
            // Note that `ioctl` is *not* on POSIX's async-signal-safe list; what
            // holds is the weaker property `pre_exec` actually needs — both
            // glibc and Apple's libc implement it as a thin syscall wrapper that
            // neither allocates nor takes a lock.
            if unsafe { libc::ioctl(0, TIOCSCTTY_REQUEST, 0) } == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        };

        // SAFETY: the registered hook runs in the forked child, after `std` has
        // wired up the stdio descriptors and before `execvp`. Nothing it does
        // allocates, takes a lock or touches shared state, which is the contract
        // of `CommandExt::pre_exec`. Of the three things it calls, `setsid` is
        // async-signal-safe outright; `ioctl` is not on POSIX's AS-safe list but
        // holds the weaker property that matters here, argued at its own call
        // site above; and `io::Error::last_os_error()` only reads `errno` and
        // wraps the integer, with no allocation and no lock on any supported
        // platform. Nothing on the path can panic, so nothing can unwind into C.
        unsafe {
            cmd.pre_exec(hook);
        }
    }
}

#[cfg(unix)]
pub use controlling_terminal::set_controlling_terminal;

// ============================================================================
// PTY pair creation
// ============================================================================

/// A PTY master/slave pair.
///
/// Both file descriptors are owned; the caller must arrange for the slave to
/// be passed to the child process (via `Command::stdin`/`stdout`/`stderr`)
/// and then dropped in the parent.
pub struct PtyPair {
    /// Master side — the parent reads output and writes input here.
    pub master: OwnedFd,
    /// Slave side — connected to the child's stdio.
    pub slave: OwnedFd,
}

/// Creates a new PTY master/slave pair via `openpty(2)`.
///
/// # Errors
///
/// Returns `io::Error` if `openpty` fails (e.g. out of PTY devices). The error
/// keeps the OS `errno`, so `kind()` and `raw_os_error()` stay meaningful.
///
/// `openpty` does not set `FD_CLOEXEC`, so both descriptors are marked
/// close-on-exec here; otherwise any child spawned meanwhile (from any session)
/// would inherit the master and the reader would never see a hangup.
///
/// The flag is set after `openpty` returns, so a child forked by another thread
/// in that instant can still inherit the pair. This narrows the leak to that
/// window; it does not remove it.
// ponytail: two-call window between openpty and fcntl; fine while spawns are
// rare. Upgrade: posix_openpt(O_CLOEXEC) + grantpt/unlockpt, no new unsafe.
#[cfg(unix)]
pub fn open_pty_pair() -> io::Result<PtyPair> {
    let result =
        nix::pty::openpty(None, None).map_err(|e| io::Error::from_raw_os_error(e as i32))?;
    let pair = PtyPair {
        master: result.master,
        slave: result.slave,
    };
    set_cloexec(&pair.master)?;
    set_cloexec(&pair.slave)?;
    Ok(pair)
}

/// Sets `FD_CLOEXEC` on an open descriptor.
#[cfg(unix)]
fn set_cloexec(fd: &OwnedFd) -> io::Result<()> {
    // SAFETY: `fd` is a valid open descriptor borrowed for the call; F_GETFD
    // only reads its descriptor flags.
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: as above; F_SETFD only writes the descriptor flags.
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, flags | libc::FD_CLOEXEC) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

// ============================================================================
// Window size (TIOCSWINSZ)
// ============================================================================

/// Sets the terminal window size on the given PTY master fd.
///
/// This triggers `SIGWINCH` in the child's process group so programs like
/// `vim`, `less`, and shells redraw at the new size.
///
/// # Errors
///
/// Returns `io::Error` if the ioctl fails.
#[cfg(unix)]
pub fn pty_set_winsize(master_fd: impl AsFd, rows: u16, cols: u16) -> io::Result<()> {
    let ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCSWINSZ is a well-defined ioctl; the descriptor is valid for
    // the duration of the call because `AsFd` borrows it; `ws` is a stack-local
    // fully initialised struct that the ioctl only reads.
    //
    // `&raw const` rather than `&ws as *const _`: the cast form creates a real
    // reference first and then discards it, which asserts to the compiler an
    // aliasing guarantee that C code on the other side of the call has never
    // agreed to. The raw-borrow operator produces the pointer without ever
    // materialising that reference.
    let ret = unsafe {
        libc::ioctl(
            master_fd.as_fd().as_raw_fd(),
            libc::TIOCSWINSZ as libc::c_ulong,
            &raw const ws,
        )
    };
    if ret == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

// ============================================================================
// Readable wait
// ============================================================================

/// Waits until the descriptor has data to read, or the timeout expires.
///
/// Returns `true` when a following read will not block. This exists so a
/// blocking reader thread can notice that it has been asked to stop: without a
/// timeout it would sit in `read` until the child exits and the PTY reports
/// `EIO`, which is too late for a session the user just closed. `POLLHUP`
/// counts as readable, because the read that follows is what turns a hangup
/// into the end of the stream.
///
/// `EINTR` is reported as "not ready" rather than as an error, since the caller
/// simply loops.
///
/// # Errors
///
/// Returns `io::Error` if `poll(2)` fails for any reason other than `EINTR`, or
/// `EBADF` if the descriptor is not open (`POLLNVAL`) — reporting that as "not
/// ready" would make the caller's loop spin without blocking.
#[cfg(unix)]
pub fn pty_wait_readable(fd: impl AsFd, timeout: std::time::Duration) -> io::Result<bool> {
    let mut pollfd = libc::pollfd {
        fd: fd.as_fd().as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // `poll` takes whole milliseconds; a sub-millisecond timeout would busy-loop,
    // so it is rounded up to one.
    let timeout_ms = i32::try_from(timeout.as_millis().max(1)).unwrap_or(i32::MAX);

    // SAFETY: `pollfd` is a single fully initialised struct and the length
    // passed matches; the descriptor is borrowed for the duration of the call.
    // `&raw mut` rather than an implicit `&mut` coercion, for the reason given at
    // the `TIOCSWINSZ` call above: no `&mut` is created, so no uniqueness claim
    // is made about memory `poll(2)` is about to write through a raw pointer.
    let ret = unsafe { libc::poll(&raw mut pollfd, 1, timeout_ms) };
    match ret {
        -1 => {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                Ok(false)
            } else {
                Err(err)
            }
        }
        0 => Ok(false),
        _ => classify_revents(pollfd.revents),
    }
}

/// Maps the `revents` of a ready `poll(2)` result to "readable or hung up".
///
/// `POLLNVAL` means the descriptor is not open: an error (`EBADF`), never "not
/// ready", which would make a reader loop spin.
fn classify_revents(revents: libc::c_short) -> io::Result<bool> {
    if revents & libc::POLLNVAL != 0 {
        return Err(io::Error::from_raw_os_error(libc::EBADF));
    }
    Ok(revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0)
}

// ============================================================================
// Descriptor duplication
// ============================================================================

/// Duplicates an `OwnedFd` as a close-on-exec descriptor.
///
/// Both descriptors refer to the same open file description, so either can be
/// used to talk to the same PTY. `FD_CLOEXEC` is set on the duplicate because
/// the callers hand these to `Command::stdin`/`stdout`/`stderr`: the standard
/// library `dup2`s them onto 0/1/2 in the child and leaves the originals open,
/// which would give every session's child three stray descriptors pointing at
/// its own terminal. `dup2` clears the flag on its target, so 0/1/2 survive
/// `exec` as intended while the spares do not.
///
/// # Errors
///
/// Returns `io::Error` if `fcntl(F_DUPFD_CLOEXEC)` fails.
#[cfg(unix)]
pub fn dup_fd(fd: &OwnedFd) -> io::Result<OwnedFd> {
    // SAFETY: fd is a valid open fd (owned). F_DUPFD_CLOEXEC returns the
    // lowest-numbered free descriptor above the third argument, which we now
    // own exclusively.
    let new_raw = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
    if new_raw == -1 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: new_raw is a freshly duplicated fd that we now own.
        Ok(unsafe { OwnedFd::from_raw_fd(new_raw) })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(all(unix, test))]
mod tests {
    // `AsRawFd` is not re-imported: `use super::*` already brings it in from the
    // module's own imports, and `redundant_imports` flags the duplicate.
    use std::process::{Command, Stdio};

    use super::*;

    /// Contract test (M-UNSAFE): proves the `pre_exec` hook is actually
    /// registered and runs in the forked child.
    ///
    /// With stdin redirected to `/dev/null` (never a terminal), the hook's
    /// `setsid()` succeeds but `ioctl(0, TIOCSCTTY)` fails — `ENOTTY` on Linux,
    /// `ENODEV` on macOS — returning `Err` from the closure, which makes
    /// `spawn()` fail. If the hook were not wired, `true` would spawn fine.
    #[test]
    fn pre_exec_hook_runs_and_fails_without_a_tty() {
        let mut cmd = Command::new("true");
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        set_controlling_terminal(&mut cmd);

        let err = cmd
            .spawn()
            .expect_err("spawn must fail: TIOCSCTTY on a /dev/null stdin errors, proving the pre_exec hook ran in the child");

        // Pin the errno to one the hook itself can produce. `is_err()` alone was
        // satisfied by *any* spawn failure — including `ENOENT` if `true` were
        // missing from the image — so the test could pass without the hook ever
        // running. Three values are accepted rather than one because which call
        // fails first, and with what, depends on the platform and the harness:
        //
        // * normally `setsid` succeeds and `TIOCSCTTY` rejects the non-terminal
        //   stdin — as `ENOTTY` on Linux, and as `ENODEV` on macOS, whose
        //   `/dev/null` reports "operation not supported by device" instead;
        // * if the test process is already a process-group leader, `setsid`
        //   fails first with `EPERM`.
        //
        // The `ENODEV` arm was missing, which made this the one test in the
        // workspace that failed on macOS — invisibly, because no CI job built
        // macOS then (the `macos-sys` job does since 0.21.0). Accepting the three keeps the assertion narrow enough to still rule
        // out an unrelated spawn failure.
        let errno = err.raw_os_error();
        assert!(
            errno == Some(libc::ENOTTY)
                || errno == Some(libc::ENODEV)
                || errno == Some(libc::EPERM),
            "expected ENOTTY/ENODEV (TIOCSCTTY path) or EPERM (setsid path), got {errno:?}: {err}",
        );
    }

    /// Verifies that `open_pty_pair` returns valid fds.
    #[test]
    fn open_pty_pair_returns_valid_fds() {
        let pair = open_pty_pair().expect("openpty should succeed");
        assert!(pair.master.as_raw_fd() >= 0);
        assert!(pair.slave.as_raw_fd() >= 0);
        assert_ne!(pair.master.as_raw_fd(), pair.slave.as_raw_fd());
    }

    /// Verifies that `pty_set_winsize` succeeds on a valid PTY master.
    #[test]
    fn set_winsize_succeeds() {
        let pair = open_pty_pair().expect("openpty");
        pty_set_winsize(&pair.master, 24, 80).expect("TIOCSWINSZ should succeed");
    }

    /// An idle PTY reports "not readable" and returns after the timeout.
    #[test]
    fn wait_readable_times_out_on_a_quiet_pty() {
        let pair = open_pty_pair().expect("openpty");
        let started = std::time::Instant::now();
        let ready = pty_wait_readable(&pair.master, std::time::Duration::from_millis(50))
            .expect("poll should succeed");
        assert!(!ready, "nothing was written, so nothing is readable");
        assert!(
            started.elapsed() >= std::time::Duration::from_millis(40),
            "the call must actually wait, otherwise a reader thread busy-loops"
        );
    }

    /// Data written to the slave makes the master readable before the timeout.
    #[test]
    fn wait_readable_reports_pending_output() {
        use std::io::Write;

        let pair = open_pty_pair().expect("openpty");
        let mut slave = std::fs::File::from(pair.slave);
        slave.write_all(b"output\n").expect("write to slave");
        slave.flush().expect("flush");

        assert!(
            pty_wait_readable(&pair.master, std::time::Duration::from_secs(2))
                .expect("poll should succeed"),
            "the master must become readable once the child writes"
        );
    }

    /// Verifies that `dup_fd` returns a distinct fd and marks it close-on-exec.
    ///
    /// The flag is the point of the function: without it every session's child
    /// would inherit three spare descriptors pointing at its own PTY.
    #[test]
    fn dup_fd_returns_distinct_cloexec_fd() {
        let pair = open_pty_pair().expect("openpty");
        let duped = dup_fd(&pair.master).expect("dup");
        assert_ne!(pair.master.as_raw_fd(), duped.as_raw_fd());

        // SAFETY: duped is a valid open fd; F_GETFD only reads flags.
        let flags = unsafe { libc::fcntl(duped.as_raw_fd(), libc::F_GETFD) };
        assert!(flags >= 0, "F_GETFD should succeed");
        assert_ne!(
            flags & libc::FD_CLOEXEC,
            0,
            "the duplicate must be close-on-exec"
        );
    }

    /// Both ends of a fresh pair must be close-on-exec (`openpty` does not set it).
    #[test]
    fn open_pty_pair_sets_cloexec_on_both_ends() {
        let pair = open_pty_pair().expect("openpty");
        for fd in [&pair.master, &pair.slave] {
            // SAFETY: fd is a valid open fd; F_GETFD only reads flags.
            let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) };
            assert_ne!(flags & libc::FD_CLOEXEC, 0, "FD_CLOEXEC must be set");
        }
    }

    /// A closed descriptor is an error, not "not ready" (which would busy-loop).
    ///
    /// Tested on the pure mapping: polling a descriptor number that was just
    /// closed races with parallel tests, which may be handed the same number.
    #[test]
    fn closed_fd_revents_is_ebadf() {
        let err = classify_revents(libc::POLLNVAL).expect_err("closed fd must be an error");
        assert_eq!(err.raw_os_error(), Some(libc::EBADF));
        assert!(classify_revents(libc::POLLIN).expect("readable"));
        assert!(classify_revents(libc::POLLHUP).expect("hung up"));
        assert!(!classify_revents(0).expect("not ready"));
    }
}
