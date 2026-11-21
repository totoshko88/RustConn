//! Moves bytes between a session's PTY and the rest of the application.
//!
//! The relay owns the master descriptor produced by [`super::pty_spawn`] and
//! runs two threads around it:
//!
//! - a **reader** that blocks on the descriptor and publishes every chunk the
//!   child wrote on a channel, and
//! - a **writer** that drains a queue of input onto the descriptor.
//!
//! Both exist to keep the GTK main thread out of blocking I/O. The reader must
//! block, because output has to reach the terminal the moment it is produced;
//! the writer must block, because a PTY write stalls when the child is not
//! reading (a paste into a stopped process is enough) and doing that on the
//! main thread would freeze the window.
//!
//! Output is delivered as [`Zeroizing`] chunks, and input is queued the same
//! way: everything a session types passes through here, including the password
//! an expect rule answers a prompt with.
//!
//! Nothing in this module touches GTK, so it is tested against a real PTY pair
//! without a display. The GTK side of the wiring — feeding the terminal,
//! forwarding `commit`, and the window-size poll — lives in
//! [`super::TerminalNotebook`].

use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use zeroize::Zeroizing;

/// Read buffer size, matching VTE's own chunk size.
///
/// Large enough that a burst of output costs few syscalls, small enough that the
/// first chunk of a slow line still arrives promptly.
const READ_CHUNK: usize = 8192;

/// How long the reader blocks before checking whether it should stop.
///
/// Output latency is unaffected — `poll` returns as soon as a byte arrives — so
/// this only bounds how long a closing session waits for its thread to finish.
const STOP_CHECK_INTERVAL: Duration = Duration::from_millis(200);

/// How long a session close waits for the reader thread before giving up on it.
///
/// The thread is detached rather than blocked on: a descriptor lingering for a
/// moment is better than a window that stops repainting.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

/// Maximum output chunks buffered between the reader and the main thread.
///
/// The bound is the backpressure: once the main thread falls behind, the reader
/// blocks, the PTY buffer fills and the kernel stops the child — which is what
/// keeps `yes` from growing the queue until the process is killed. At
/// [`READ_CHUNK`] per slot this caps the queue at half a megabyte.
const OUTPUT_QUEUE_CHUNKS: usize = 64;

/// A chunk of bytes the child wrote.
pub type OutputChunk = Zeroizing<Vec<u8>>;

/// Recovers the bytes behind a `commit` signal.
///
/// VTE emits the signal as a NUL-terminated C string plus the true byte count,
/// and the binding turns the pointer into a `&str` that stops at the first NUL.
/// A key that sends NUL — `Ctrl+Space`, which Emacs and readline use to set the
/// mark — therefore arrives as an empty string with a `size` of 1, and
/// forwarding the string alone would drop the keystroke.
///
/// Bytes missing from the string are restored as NUL. That is exact for
/// keyboard input, where VTE emits one `commit` per key press, and approximate
/// only for pasted text containing embedded NULs, where the position of the
/// NULs within the chunk is unrecoverable — and where the bytes were never
/// going to survive a `char*` signal in the first place.
#[must_use]
pub fn commit_bytes(text: &str, size: u32) -> Zeroizing<Vec<u8>> {
    let declared = size as usize;
    let mut bytes = Zeroizing::new(text.as_bytes().to_vec());
    if declared > bytes.len() {
        bytes.resize(declared, 0);
    }
    bytes
}

/// Receiving end of a session's output stream.
pub type OutputStream = async_channel::Receiver<OutputChunk>;

/// Owns a session's PTY master descriptor and the threads around it.
///
/// Dropping the relay stops both threads: the writer ends when its queue is
/// closed, and the reader ends at the next [`STOP_CHECK_INTERVAL`] boundary, or
/// at once if it is blocked handing a chunk to a consumer that stopped draining
/// (the output queue is closed on drop).
pub struct PtyRelay {
    /// Queue of pending input; closing it stops the writer thread.
    input: Option<async_channel::Sender<Zeroizing<Vec<u8>>>>,
    /// Descriptor kept for `TIOCSWINSZ`, which needs no thread.
    winsize_fd: OwnedFd,
    /// Set to ask the reader thread to stop.
    stop: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<()>>,
    /// A second handle on the output queue, used only to close it on drop.
    ///
    /// The consumer may be parked (a selection drag holds output back) with the
    /// queue full, which leaves the reader blocked in `send_blocking` where it
    /// never sees `stop`. Closing the queue fails that send, so the reader ends
    /// immediately instead of costing the main thread [`SHUTDOWN_GRACE`].
    output_close: OutputStream,
    /// Last size pushed to the child, so an unchanged poll costs nothing.
    last_size: std::cell::Cell<(u16, u16)>,
}

impl PtyRelay {
    /// Starts relaying on `master`, returning the relay and its output stream.
    ///
    /// The caller drives the stream on the main thread; when the relay is
    /// dropped the stream ends, which is how the consumer learns to stop.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if the descriptor cannot be duplicated for the
    /// writer thread, or if a thread cannot be started — both of which mean the
    /// process is out of descriptors or out of threads, so the session simply
    /// cannot run.
    pub fn start(
        master: OwnedFd,
        initial_size: (u16, u16),
    ) -> std::io::Result<(Self, OutputStream)> {
        // Three views of the same PTY: one per thread, plus one for the ioctl.
        // They are separate descriptors so that closing one thread's copy
        // cannot pull the descriptor out from under another.
        let write_fd = rustconn_pty_sys::dup_fd(&master)?;
        let winsize_fd = rustconn_pty_sys::dup_fd(&master)?;

        let stop = Arc::new(AtomicBool::new(false));
        let (output_tx, output_rx) = async_channel::bounded(OUTPUT_QUEUE_CHUNKS);
        let output_close = output_rx.clone();
        let (input_tx, input_rx) = async_channel::unbounded::<Zeroizing<Vec<u8>>>();

        let reader_stop = Arc::clone(&stop);
        let reader = std::thread::Builder::new()
            .name("pty-read".to_owned())
            .spawn(move || read_loop(master, &reader_stop, &output_tx))?;

        let writer = std::thread::Builder::new()
            .name("pty-write".to_owned())
            .spawn(move || write_loop(write_fd, &input_rx))?;

        Ok((
            Self {
                input: Some(input_tx),
                winsize_fd,
                stop,
                reader: Some(reader),
                writer: Some(writer),
                output_close,
                last_size: std::cell::Cell::new(initial_size),
            },
            output_rx,
        ))
    }

    /// Queues input for the child.
    ///
    /// Returns `false` once the writer thread is gone, which happens when the
    /// child has exited; the caller treats that as "this session is over"
    /// rather than as an error worth showing.
    pub fn write_input(&self, data: &[u8]) -> bool {
        let Some(ref input) = self.input else {
            return false;
        };
        input.send_blocking(Zeroizing::new(data.to_vec())).is_ok()
    }

    /// Whether the child's terminal echoes input.
    ///
    /// `false` while a program reads a password. An error reading the state
    /// counts as echoing, so an unreadable terminal is recorded as typed
    /// rather than hidden.
    pub fn echo_enabled(&self) -> bool {
        rustconn_pty_sys::pty_echo_enabled(&self.winsize_fd).unwrap_or(true)
    }

    /// Tells the child the window size, if it differs from the last one sent.
    ///
    /// Returns `true` when a change was pushed. Sending an unchanged size would
    /// deliver a pointless `SIGWINCH`, which full-screen programs answer with a
    /// full redraw.
    pub fn sync_size(&self, rows: u16, cols: u16) -> bool {
        if rows == 0 || cols == 0 || self.last_size.get() == (rows, cols) {
            return false;
        }
        self.last_size.set((rows, cols));
        if let Err(e) = rustconn_pty_sys::pty_set_winsize(&self.winsize_fd, rows, cols) {
            tracing::warn!(%e, rows, cols, "TIOCSWINSZ failed");
        }
        true
    }
}

impl Drop for PtyRelay {
    fn drop(&mut self) {
        // Closing the input queue ends the writer thread; the flag ends the
        // reader at its next poll timeout.
        self.input = None;
        self.stop.store(true, Ordering::Relaxed);
        // Chunks already queued stay readable; only the blocked sender is freed.
        self.output_close.close();

        for handle in [self.writer.take(), self.reader.take()]
            .into_iter()
            .flatten()
        {
            // Threads that are already finished join immediately; one still
            // inside a poll is left to notice the flag on its own rather than
            // blocking the main loop for it.
            let deadline = std::time::Instant::now() + SHUTDOWN_GRACE;
            while !handle.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            if handle.is_finished() {
                let _ = handle.join();
            } else {
                tracing::debug!("PTY thread still running at shutdown; detaching");
            }
        }
    }
}

/// Publishes everything the child writes until it exits or the relay stops.
fn read_loop(master: OwnedFd, stop: &AtomicBool, output: &async_channel::Sender<OutputChunk>) {
    let mut file = std::fs::File::from(master);
    let mut buf = Zeroizing::new(vec![0_u8; READ_CHUNK]);

    while !stop.load(Ordering::Relaxed) {
        match rustconn_pty_sys::pty_wait_readable(&file, STOP_CHECK_INTERVAL) {
            Ok(false) => continue,
            Ok(true) => {}
            Err(e) => {
                tracing::warn!(%e, "PTY poll failed; ending session output");
                break;
            }
        }

        match file.read(&mut buf) {
            // End of stream: the child closed the slave.
            Ok(0) => break,
            Ok(n) => {
                if output
                    .send_blocking(Zeroizing::new(buf[..n].to_vec()))
                    .is_err()
                {
                    // The consumer is gone, so the session is being torn down.
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => {
                // A PTY master reports EIO rather than end-of-file once the
                // last descriptor on the slave side is closed, which is the
                // ordinary way a session ends.
                if e.raw_os_error() != Some(nix::errno::Errno::EIO as i32) {
                    tracing::warn!(%e, "PTY read failed; ending session output");
                }
                break;
            }
        }
    }
}

/// Writes queued input to the child in order, until the queue is closed.
fn write_loop(write_fd: OwnedFd, input: &async_channel::Receiver<Zeroizing<Vec<u8>>>) {
    let mut file = std::fs::File::from(write_fd);
    while let Ok(chunk) = input.recv_blocking() {
        if let Err(e) = file.write_all(&chunk) {
            // A closed PTY means the child is gone; anything else is worth a
            // line in the log, but either way there is nowhere left to write.
            if e.kind() != std::io::ErrorKind::BrokenPipe {
                tracing::warn!(%e, "PTY write failed; dropping session input");
            }
            break;
        }
        if let Err(e) = file.flush() {
            tracing::warn!(%e, "PTY flush failed");
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use super::*;

    /// Collects output until `wanted` shows up, or the deadline passes.
    fn collect_until(stream: &OutputStream, wanted: &str) -> String {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut seen = String::new();
        while std::time::Instant::now() < deadline {
            match stream.recv_blocking() {
                Ok(chunk) => {
                    seen.push_str(&String::from_utf8_lossy(&chunk));
                    if seen.contains(wanted) {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        seen
    }

    fn reap(pid: u32) {
        let pid = nix::unistd::Pid::from_raw(pid as i32);
        let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
        let _ = nix::sys::wait::waitpid(pid, None);
    }

    #[test]
    fn commit_bytes_passes_ordinary_input_through() {
        assert_eq!(commit_bytes("ls -la\r", 7).as_slice(), b"ls -la\r");
    }

    #[test]
    fn commit_bytes_restores_a_lone_nul() {
        // Ctrl+Space: the C string is empty, the declared size is 1.
        assert_eq!(commit_bytes("", 1).as_slice(), b"\0");
    }

    #[test]
    fn commit_bytes_ignores_a_short_declared_size() {
        // A size smaller than the string would mean VTE contradicted itself;
        // truncating the user's keystrokes on that basis would be worse than
        // sending one byte too many.
        assert_eq!(commit_bytes("abc", 1).as_slice(), b"abc");
    }

    #[test]
    fn output_written_by_the_child_reaches_the_stream() {
        let child =
            super::super::pty_spawn::spawn_on_pty(&["echo", "relayed-output"], &[], None, (24, 80))
                .expect("echo should spawn");
        let pid = child.pid;
        let (relay, stream) = PtyRelay::start(child.master, (24, 80)).expect("relay starts");

        let seen = collect_until(&stream, "relayed-output");
        drop(relay);
        reap(pid);
        assert!(seen.contains("relayed-output"), "got {seen:?}");
    }

    #[test]
    fn input_queued_on_the_relay_reaches_the_child() {
        // `cat` echoes whatever it is given, so a round trip proves both
        // directions at once.
        let child = super::super::pty_spawn::spawn_on_pty(&["cat"], &[], None, (24, 80))
            .expect("cat should spawn");
        let pid = child.pid;
        let (relay, stream) = PtyRelay::start(child.master, (24, 80)).expect("relay starts");

        assert!(relay.write_input(b"round-trip\n"), "input must be accepted");
        let seen = collect_until(&stream, "round-trip");

        drop(relay);
        reap(pid);
        assert!(seen.contains("round-trip"), "got {seen:?}");
    }

    #[test]
    fn the_stream_ends_when_the_child_exits() {
        let child = super::super::pty_spawn::spawn_on_pty(&["true"], &[], None, (24, 80))
            .expect("true should spawn");
        let pid = child.pid;
        let (relay, stream) = PtyRelay::start(child.master, (24, 80)).expect("relay starts");

        // Draining to the end proves the reader recognises the PTY's EIO as the
        // end of the session rather than logging it as a failure forever.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if stream.recv_blocking().is_err() {
                break;
            }
        }
        assert!(
            stream.recv_blocking().is_err(),
            "the stream must close once the child is gone"
        );
        drop(relay);
        reap(pid);
    }

    #[test]
    fn a_size_change_is_pushed_once_and_seen_by_the_child() {
        // `sh` reporting its own geometry after a SIGWINCH proves the ioctl
        // reached the child, not merely that the call succeeded.
        let child = super::super::pty_spawn::spawn_on_pty(
            &[
                "sh",
                "-c",
                "trap 'stty size' WINCH; sleep 5 & wait; stty size",
            ],
            &[],
            None,
            (24, 80),
        )
        .expect("sh should spawn");
        let pid = child.pid;
        let (relay, stream) = PtyRelay::start(child.master, (24, 80)).expect("relay starts");

        // Give the shell time to install its trap before resizing.
        std::thread::sleep(Duration::from_millis(300));
        assert!(relay.sync_size(44, 133), "a new size must be pushed");
        assert!(
            !relay.sync_size(44, 133),
            "the same size must not be pushed twice: a needless SIGWINCH makes \
             full-screen programs redraw"
        );

        let seen = collect_until(&stream, "44 133");
        drop(relay);
        reap(pid);
        assert!(seen.contains("44 133"), "got {seen:?}");
    }

    #[test]
    fn a_zero_size_is_ignored_and_does_not_disturb_the_stored_one() {
        // A widget that has not been allocated yet reports zero rows; pushing
        // that would tell the child its terminal has no size at all.
        let pair = rustconn_pty_sys::open_pty_pair().expect("openpty");
        let (relay, _stream) = PtyRelay::start(pair.master, (24, 80)).expect("relay starts");
        assert!(!relay.sync_size(0, 80), "zero rows must be refused");
        assert!(!relay.sync_size(24, 0), "zero columns must be refused");
        assert!(
            !relay.sync_size(24, 80),
            "the refused calls must not have overwritten the known size"
        );
        assert!(relay.sync_size(30, 90), "a real change still gets through");
    }

    #[test]
    fn dropping_the_relay_closes_the_stream() {
        let pair = rustconn_pty_sys::open_pty_pair().expect("openpty");
        // Keep the slave open so the PTY does not end on its own: this test is
        // about the relay stopping, not about the child exiting.
        let mut slave = std::fs::File::from(pair.slave);
        let (relay, stream) = PtyRelay::start(pair.master, (24, 80)).expect("relay starts");

        slave.write_all(b"before-drop\n").expect("write");
        let seen = collect_until(&stream, "before-drop");
        assert!(seen.contains("before-drop"), "got {seen:?}");

        drop(relay);
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline && !stream.is_closed() {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            stream.is_closed(),
            "dropping the relay must end the stream so its consumer stops"
        );

        // The slave outliving the relay must not keep anything alive either.
        let mut buf = [0_u8; 8];
        let _ = slave.read(&mut buf);
    }

    #[test]
    fn dropping_the_relay_does_not_wait_for_a_consumer_that_stopped_draining() {
        // `yes` fills the queue; nothing reads it, as when a selection drag
        // parks the output loop. The reader is then blocked in `send_blocking`,
        // not in `poll`, and used to cost the closing thread SHUTDOWN_GRACE.
        let child = super::super::pty_spawn::spawn_on_pty(&["yes"], &[], None, (24, 80))
            .expect("yes should spawn");
        let pid = child.pid;
        let (relay, stream) = PtyRelay::start(child.master, (24, 80)).expect("relay starts");

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !stream.is_full() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            stream.is_full(),
            "the queue should fill with nothing reading"
        );

        let started = std::time::Instant::now();
        drop(relay);
        let elapsed = started.elapsed();
        reap(pid);
        assert!(
            elapsed < SHUTDOWN_GRACE / 2,
            "drop stalled for {elapsed:?} on a reader blocked in send"
        );
        assert!(stream.is_closed(), "the queue must be closed after drop");
    }

    #[test]
    fn input_after_the_child_exits_is_reported_rather_than_panicking() {
        let child = super::super::pty_spawn::spawn_on_pty(&["true"], &[], None, (24, 80))
            .expect("true should spawn");
        let pid = child.pid;
        let (relay, stream) = PtyRelay::start(child.master, (24, 80)).expect("relay starts");
        while stream.recv_blocking().is_ok() {}
        reap(pid);

        // The queue accepts the bytes even after the child is gone (the writer
        // thread discovers the closed PTY when it gets there); what matters is
        // that nothing panics and the session can be torn down.
        let _ = relay.write_input(b"ignored\n");
        drop(relay);
    }
}
