//! macOS-specific PTY spawn for VTE terminals.
//!
//! VTE's built-in `spawn_async` does not work on macOS (Homebrew build) —
//! the PTY is created but never connected to the child process output.
//! This module works around the issue by creating a native PTY via
//! `nix::pty::openpty()` and manually spawning the child process with
//! the slave fd as stdin/stdout/stderr, then handing the master fd to VTE.

use std::os::fd::AsFd;
use std::process::{Command, Stdio};

use gtk4::{gio, glib};
use vte4::prelude::*;
use vte4::{Pty, Terminal};

/// Essential environment variables that must be present for a child shell
/// to function correctly. If `envv` provides them, those values take priority;
/// otherwise we inherit from the parent process.
const ESSENTIAL_ENV_VARS: &[&str] = &["HOME", "USER", "LOGNAME", "SHELL", "LANG", "PATH"];

/// Checks whether `envv` already contains a variable with the given key.
fn envv_contains_key(envv: &[&str], key: &str) -> bool {
    let prefix_len = key.len();
    envv.iter()
        .any(|e| e.len() > prefix_len && e.starts_with(key) && e.as_bytes()[prefix_len] == b'=')
}

/// Spawns a command in a native macOS PTY and connects it to the VTE terminal.
///
/// Returns `Ok(child_pid)` on success, or an error string on failure.
pub fn spawn_native_pty(
    terminal: &Terminal,
    argv: &[&str],
    envv: &[&str],
    working_directory: Option<&str>,
) -> Result<u32, String> {
    if argv.is_empty() {
        return Err("argv is empty".to_string());
    }

    // 1. Create a PTY pair via openpty (safe wrapper around libc::openpty)
    let pty_pair = nix::pty::openpty(None, None).map_err(|e| format!("openpty failed: {e}"))?;

    let master_fd = pty_pair.master;
    let slave_fd = pty_pair.slave;

    // 2. Build the child process command
    let mut cmd = Command::new(argv[0]);
    if argv.len() > 1 {
        cmd.args(&argv[1..]);
    }

    // Set working directory
    if let Some(dir) = working_directory {
        cmd.current_dir(dir);
    }

    // Set environment: parse "KEY=VALUE" pairs
    cmd.env_clear();
    for env_str in envv {
        if let Some(eq_pos) = env_str.find('=') {
            let key = &env_str[..eq_pos];
            let value = &env_str[eq_pos + 1..];
            cmd.env(key, value);
        }
    }

    // If envv is empty, inherit full parent environment
    if envv.is_empty() {
        for (key, value) in std::env::vars() {
            cmd.env(&key, &value);
        }
    } else {
        // Ensure essential env vars are present even when envv is non-empty.
        // Without HOME/USER/SHELL the child shell may malfunction.
        for &var in ESSENTIAL_ENV_VARS {
            if !envv_contains_key(envv, var)
                && let Ok(value) = std::env::var(var)
            {
                cmd.env(var, value);
            }
        }
    }

    // Ensure TERM is set (overrides any inherited value)
    cmd.env("TERM", "xterm-256color");

    // 3. Connect slave fd as stdin/stdout/stderr for the child.
    //    nix 0.31 dup() returns OwnedFd which auto-closes on drop,
    //    so if a later dup fails the earlier OwnedFds are cleaned up.
    let stdin_fd =
        nix::unistd::dup(slave_fd.as_fd()).map_err(|e| format!("dup stdin failed: {e}"))?;
    let stdout_fd =
        nix::unistd::dup(slave_fd.as_fd()).map_err(|e| format!("dup stdout failed: {e}"))?;
    let stderr_fd =
        nix::unistd::dup(slave_fd.as_fd()).map_err(|e| format!("dup stderr failed: {e}"))?;

    // Stdio::from(OwnedFd) takes ownership — no unsafe needed.
    cmd.stdin(Stdio::from(stdin_fd));
    cmd.stdout(Stdio::from(stdout_fd));
    cmd.stderr(Stdio::from(stderr_fd));

    // 4. Spawn the child process.
    //
    // `set_controlling_terminal` registers a `pre_exec` hook that runs
    // setsid(2) + TIOCSCTTY so the slave PTY (fd 0) becomes the child's
    // controlling terminal. This is required for interactive programs such
    // as `ssh` to open /dev/tty and read a password prompt — without it,
    // a GUI process launched without a controlling terminal (e.g. from the
    // macOS Finder) makes ssh fail instantly with "Permission denied" (#175).
    //
    // setsid(2) also makes the child a session + process-group leader, which
    // provides basic job control (Ctrl-C → SIGINT to the foreground group),
    // so the previous `.process_group(0)` (setpgid) is intentionally gone —
    // and must stay gone, because setsid(2) fails with EPERM if the caller is
    // already a process-group leader.
    rustconn_pty_sys::set_controlling_terminal(&mut cmd);

    let child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;
    let child_pid = child.id();

    // Forget the Child handle — GLib's child_watch_add_local will reap the
    // process via waitpid(). If we let Child drop here, its Drop impl would
    // call waitpid() and race with GLib's watcher.
    std::mem::forget(child);

    tracing::info!(
        command = %argv[0],
        pid = child_pid,
        "macOS native PTY: child spawned"
    );

    // 5. Close slave fd in parent (child has its own copies)
    drop(slave_fd);

    // 6. Create VTE Pty from master fd and attach to terminal.
    // If this fails, kill the child and reap it to avoid a zombie process.
    let vte_pty = match Pty::foreign_sync(master_fd, gio::Cancellable::NONE) {
        Ok(pty) => pty,
        Err(e) => {
            let pid = nix::unistd::Pid::from_raw(child_pid as i32);
            let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
            // Reap the killed child to prevent zombie
            let _ = nix::sys::wait::waitpid(pid, None);
            return Err(format!("Failed to create VTE Pty from fd: {e}"));
        }
    };

    terminal.set_pty(Some(&vte_pty));

    // 7. Watch for child exit and notify VTE
    let terminal_weak = terminal.downgrade();
    glib::child_watch_add_local(glib::Pid(child_pid as i32), move |_pid, status| {
        tracing::debug!(status = status, "macOS native PTY: child exited");
        if let Some(terminal) = terminal_weak.upgrade() {
            // Emit child-exited signal so VTE/RustConn handles cleanup
            terminal.emit_by_name::<()>("child-exited", &[&status]);
        }
    });

    Ok(child_pid)
}
