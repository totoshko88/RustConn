//! Session lock / suspend listener for the SSH key passphrase cache (W3 phase 4).
//!
//! The passphrase cache
//! ([`rustconn_core::ssh_agent::passphrase_cache`]) holds, in process memory
//! only, the passphrases that unlocked encrypted SSH keys this session. When the
//! user locks their screen or the machine suspends, those passphrases should no
//! longer sit in memory — the same reasoning that makes an SSH agent drop keys on
//! lock. This module subscribes to the two D-Bus signals that announce those
//! events and calls [`passphrase_cache::clear`] when they fire.
//!
//! # Signals
//!
//! - `org.freedesktop.login1` → `PrepareForSleep(bool)` on the **system** bus.
//!   Emitted with `true` just before the system suspends/hibernates. On suspend
//!   we both clear the cache and ask the agent to drop its own keys (`ssh-add
//!   -D`), matching what a lock-on-suspend policy would do.
//! - `ScreenSaver` → `ActiveChanged(bool)` on the **session** bus. Emitted with
//!   `true` when the screensaver/lock engages. We subscribe to both the portable
//!   `org.freedesktop.ScreenSaver` name and the `org.gnome.ScreenSaver` name,
//!   because GNOME emits the signal on its own name and leaves the freedesktop
//!   one silent; a desktop that serves only one simply skips the other. On a
//!   plain lock we clear the cache but leave the agent alone: active sessions may
//!   still need it, and the user is expected to unlock shortly.
//!
//! # Threading
//!
//! zbus runs on a tokio reactor in this build, and the GTK main loop is not one,
//! so the listener owns a dedicated single-threaded tokio runtime on its own
//! thread. It never touches a GTK object, so there is no cross-thread widget
//! access: it only calls the process-global `passphrase_cache` and spawns
//! `ssh-add`. A failure to reach either bus is logged and that half is skipped —
//! the cache still works, the user is just prompted again after a lock.
//!
//! This module is Linux/BSD only (see the `cfg` on its declaration in
//! `main.rs`): D-Bus, login1 and the ScreenSaver interface do not exist on macOS.

use std::process::Command;

use rustconn_core::ssh_agent::passphrase_cache;
use zbus::Connection;

/// login1 `PrepareForSleep` on the system bus. The argument is `true` when the
/// system is about to sleep and `false` when it has woken; we act on `true`.
#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1Manager {
    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

/// freedesktop `ScreenSaver` `ActiveChanged` on the session bus. The argument is
/// `true` when the screensaver/lock becomes active. This is the portable name,
/// but on some desktops (notably GNOME) the freedesktop name is only a shim that
/// never emits the signal, so the GNOME `ScreenSaver` proxy is subscribed in
/// parallel (see the `gnome` module below).
#[zbus::proxy(
    interface = "org.freedesktop.ScreenSaver",
    default_service = "org.freedesktop.ScreenSaver",
    default_path = "/org/freedesktop/ScreenSaver"
)]
trait ScreenSaver {
    #[zbus(signal)]
    fn active_changed(&self, active: bool) -> zbus::Result<()>;
}

/// GNOME `ScreenSaver` `ActiveChanged` on the session bus — the name GNOME Shell
/// actually emits on lock/unlock. Same signal shape as the freedesktop one; both
/// are subscribed so a plain screen lock (no suspend) clears the cache under
/// GNOME, where the freedesktop shim stays silent. A desktop that implements
/// only one of the two simply has the other connect-fail and get skipped.
///
/// It lives in its own module because the `#[proxy]` macro derives its generated
/// type names (`ActiveChangedStream`, `ActiveChangedArgs`, …) from the wire
/// signal name, which is identical to the freedesktop one — so the two proxies
/// would collide in a shared namespace.
mod gnome {
    #[zbus::proxy(
        interface = "org.gnome.ScreenSaver",
        default_service = "org.gnome.ScreenSaver",
        default_path = "/org/gnome/ScreenSaver"
    )]
    pub(super) trait GnomeScreenSaver {
        #[zbus(signal)]
        fn active_changed(&self, active: bool) -> zbus::Result<()>;
    }
}

/// Starts the lock/suspend listener on a dedicated background thread.
///
/// Returns immediately; the listener runs for the lifetime of the process. Call
/// once at startup, after the window is presented. Any failure to connect to a
/// bus is logged and that signal source is skipped — the other still runs, and
/// the passphrase cache is simply never auto-cleared from that source.
pub fn start() {
    // A plain OS thread, not a GTK/glib one: the listener must not run on the
    // main loop (zbus needs a tokio reactor) and holds no GTK state.
    std::thread::Builder::new()
        .name("rustconn-lock-listener".to_owned())
        .spawn(run_thread)
        .map(|_| ())
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "could not spawn the lock/suspend listener thread");
        });
}

/// Thread body: build a current-thread tokio runtime and drive the listener.
fn run_thread() {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::warn!(error = %e, "lock listener: could not build tokio runtime");
            return;
        }
    };
    runtime.block_on(listen());
}

/// Connects to both buses and processes their signal streams until the process
/// exits. Each bus is independent: if one cannot be reached the other still runs.
async fn listen() {
    // Hold each `Connection` for as long as its stream is polled: dropping it
    // would close the bus and end the stream. The streams themselves are the
    // raw zbus signal streams (`Unpin`), decoded to a `bool` at the match below.
    let sleep = match connect_sleep_stream().await {
        Ok(pair) => Some(pair),
        Err(e) => {
            tracing::info!(error = %e, "lock listener: login1 PrepareForSleep unavailable");
            None
        }
    };

    // Both ScreenSaver names live on the session bus, so one connection serves
    // both proxies. Open it once; if it cannot be reached, neither lock source
    // is available (but login1 suspend may still be).
    let session = match Connection::session().await {
        Ok(conn) => Some(conn),
        Err(e) => {
            tracing::info!(error = %e, "lock listener: session bus unavailable; no lock source");
            None
        }
    };

    let fdo_saver = connect_fdo_saver_stream(session.as_ref()).await;
    let gnome_saver = connect_gnome_saver_stream(session.as_ref()).await;

    let (_sleep_conn, mut sleep_stream) = match sleep {
        Some((conn, stream)) => (Some(conn), Some(stream)),
        None => (None, None),
    };
    let mut fdo_stream = fdo_saver;
    let mut gnome_stream = gnome_saver;

    // `session` is held so both ScreenSaver streams stay open.
    let _session = session;

    if sleep_stream.is_none() && fdo_stream.is_none() && gnome_stream.is_none() {
        tracing::info!("lock listener: no lock/suspend signal source available; exiting");
        return;
    }

    loop {
        tokio::select! {
            // An absent source becomes a never-ready branch via `next_opt`.
            sleep = next_opt(sleep_stream.as_mut()) => {
                match sleep {
                    Some(signal) => {
                        if let Ok(args) = signal.args() {
                            on_prepare_for_sleep(args.start);
                        }
                    }
                    None => sleep_stream = None, // stream ended; stop polling it
                }
            }
            active = next_opt(fdo_stream.as_mut()) => {
                match active {
                    Some(signal) => {
                        if let Ok(args) = signal.args() {
                            on_screensaver_active("org.freedesktop.ScreenSaver", args.active);
                        }
                    }
                    None => fdo_stream = None,
                }
            }
            active = next_opt(gnome_stream.as_mut()) => {
                match active {
                    Some(signal) => {
                        if let Ok(args) = signal.args() {
                            on_screensaver_active("org.gnome.ScreenSaver", args.active);
                        }
                    }
                    None => gnome_stream = None,
                }
            }
        }

        if sleep_stream.is_none() && fdo_stream.is_none() && gnome_stream.is_none() {
            tracing::info!("lock listener: all signal sources ended; exiting");
            return;
        }
    }
}

/// Awaits the next item of an optional stream. When the stream is `None`
/// (unavailable), the future never completes, so `select!` ignores that branch.
async fn next_opt<S, T>(stream: Option<&mut S>) -> Option<T>
where
    S: futures_util::Stream<Item = T> + Unpin,
{
    use futures_util::StreamExt;
    match stream {
        Some(s) => s.next().await,
        None => std::future::pending().await,
    }
}

/// Opens the system bus and returns its connection plus the raw
/// `PrepareForSleep` signal stream. The caller keeps the connection alive.
async fn connect_sleep_stream() -> zbus::Result<(Connection, PrepareForSleepStream)> {
    let conn = Connection::system().await?;
    let proxy = Login1ManagerProxy::new(&conn).await?;
    let stream = proxy.receive_prepare_for_sleep().await?;
    Ok((conn, stream))
}

/// Subscribes to the freedesktop `ScreenSaver` `ActiveChanged` signal on the
/// shared session connection. Returns `None` if the session bus was unavailable
/// or the interface is not served here; that half is then simply skipped.
async fn connect_fdo_saver_stream(session: Option<&Connection>) -> Option<ActiveChangedStream> {
    let conn = session?;
    match ScreenSaverProxy::new(conn).await {
        Ok(proxy) => match proxy.receive_active_changed().await {
            Ok(stream) => Some(stream),
            Err(e) => {
                tracing::info!(error = %e, "lock listener: org.freedesktop.ScreenSaver signal unavailable");
                None
            }
        },
        Err(e) => {
            tracing::info!(error = %e, "lock listener: org.freedesktop.ScreenSaver unavailable");
            None
        }
    }
}

/// Subscribes to the GNOME `ScreenSaver` `ActiveChanged` signal on the shared
/// session connection. Returns `None` on a non-GNOME desktop (the name is not
/// served), leaving the freedesktop subscription to carry the lock event.
async fn connect_gnome_saver_stream(
    session: Option<&Connection>,
) -> Option<gnome::ActiveChangedStream> {
    use gnome::GnomeScreenSaverProxy;
    let conn = session?;
    match GnomeScreenSaverProxy::new(conn).await {
        Ok(proxy) => match proxy.receive_active_changed().await {
            Ok(stream) => Some(stream),
            Err(e) => {
                tracing::info!(error = %e, "lock listener: org.gnome.ScreenSaver signal unavailable");
                None
            }
        },
        Err(e) => {
            tracing::info!(error = %e, "lock listener: org.gnome.ScreenSaver unavailable");
            None
        }
    }
}

/// login1 `PrepareForSleep`: `true` just before sleep, `false` on resume.
/// On the way into sleep, clear the cache and drop the agent's keys.
fn on_prepare_for_sleep(going_to_sleep: bool) {
    if !going_to_sleep {
        return;
    }
    let cleared = passphrase_cache::len();
    passphrase_cache::clear();
    remove_all_agent_keys();
    tracing::info!(
        cleared_entries = cleared,
        "system suspending: cleared SSH passphrase cache and dropped agent keys"
    );
}

/// ScreenSaver `ActiveChanged`: `true` on lock. Clear the cache but leave the
/// agent's keys in place — an active session may still need them, and the user
/// is expected back shortly. `source` is the D-Bus name that emitted it, for the
/// log; when both the freedesktop and GNOME names fire for one lock, the second
/// call simply clears an already-empty cache.
fn on_screensaver_active(source: &str, active: bool) {
    if !active {
        return;
    }
    let cleared = passphrase_cache::len();
    passphrase_cache::clear();
    tracing::info!(
        source,
        cleared_entries = cleared,
        "screen locked: cleared SSH passphrase cache"
    );
}

/// Best-effort `ssh-add -D`: removes every identity from the running agent so a
/// suspended machine holds no decrypted keys either. A missing agent or a
/// non-zero exit is logged at debug level and ignored — this is a hardening step,
/// not a correctness requirement.
fn remove_all_agent_keys() {
    match Command::new("ssh-add").arg("-D").output() {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            tracing::debug!(
                status = ?out.status.code(),
                "ssh-add -D returned non-zero (likely no agent or no keys)"
            );
        }
        Err(e) => {
            tracing::debug!(error = %e, "ssh-add -D could not be run");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rustconn_core::ssh_agent::passphrase_cache;
    use secrecy::SecretString;

    use super::*;

    /// A unique key path per test so the process-global cache does not let one
    /// test observe another's entry. Mirrors the pattern in `passphrase_cache`.
    fn seed_one(tag: &str) -> PathBuf {
        let dir = std::env::var("KIROCREW_SCRATCH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir());
        let path = dir.join(format!(
            "rustconn-lock-listener-test-{tag}-{}",
            std::process::id()
        ));
        passphrase_cache::remember(&path, &SecretString::from("secret".to_string()));
        path
    }

    /// One test for all handler edges. The passphrase cache is process-global
    /// and `on_screensaver_active(true)` calls `clear()`, which would race a
    /// sibling test's seed if these ran in parallel — so the edges are checked
    /// in sequence inside a single test. The `false` edges are asserted BEFORE
    /// the `true` edge so an early `clear()` cannot mask them, and neither
    /// `false` edge shells out to `ssh-add` (both return early).
    #[test]
    fn lock_suspend_handlers_clear_only_on_lock_and_sleep() {
        // Resume edge: start=false must not touch the cache.
        let resume_path = seed_one("sleep-resume");
        on_prepare_for_sleep(false);
        assert!(
            passphrase_cache::get(&resume_path).is_some(),
            "a resume (start=false) does not touch the cache"
        );

        // Unlock edge: active=false must not touch the cache.
        let unlock_path = seed_one("saver-inactive");
        on_screensaver_active("org.gnome.ScreenSaver", false);
        assert!(
            passphrase_cache::get(&unlock_path).is_some(),
            "an unlock (active=false) does not touch the cache"
        );

        // Lock edge: active=true clears the whole cache.
        on_screensaver_active("org.gnome.ScreenSaver", true);
        assert!(
            passphrase_cache::get(&resume_path).is_none()
                && passphrase_cache::get(&unlock_path).is_none(),
            "a screen lock drops every cached passphrase"
        );
    }
}
