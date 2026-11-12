//! The D-Bus source of [`LockEvent`]s: login1 suspend and the screensaver.
//!
//! # Signals
//!
//! - `org.freedesktop.login1.Manager.PrepareForSleep(bool)` on the **system**
//!   bus, `true` just before the system suspends or hibernates and `false` after
//!   it wakes. `true` dispatches [`LockEvent::Suspending`].
//! - `ActiveChanged(bool)` on the **session** bus, `true` when the screensaver
//!   or lock engages, which dispatches [`LockEvent::ScreenLocked`]. Both the
//!   portable `org.freedesktop.ScreenSaver` name and `org.gnome.ScreenSaver` are
//!   watched: GNOME Shell emits the signal on its own name and leaves the
//!   freedesktop one silent. One lock can therefore arrive twice, so a handler
//!   must tolerate a repeat.
//!
//! # Delay lock
//!
//! `PrepareForSleep(true)` is sent when the suspend has already begun, and
//! without an inhibitor logind does not wait for anyone. So the listener takes a
//! `delay` inhibitor for `sleep` up front (`Inhibit("sleep", …, "delay")`),
//! releases it once every handler has returned from [`LockEvent::Suspending`],
//! and takes a new one after the system wakes. logind waits at most
//! `InhibitDelayMaxSec` (5 s by default) for the release.
//!
//! # Threading and failure
//!
//! zbus runs on tokio here and the GTK main loop is not a tokio reactor, so the
//! listener owns a single-threaded tokio runtime on its own thread. Handlers run
//! on tokio's blocking pool, never the GTK thread. A bus that cannot be reached,
//! a name nobody serves, or an inhibitor logind refuses (a Snap without the
//! `shutdown` interface) is logged and skipped; whatever else is available keeps
//! working. Nothing here spawns a process.

use std::sync::atomic::{AtomicBool, Ordering};

use futures::StreamExt as _;
use zbus::proxy::CacheProperties;
use zbus::{Connection, OwnedFd};

use super::LockEvent;

/// login1's manager: the delay inhibitor and `PrepareForSleep`.
#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1Manager {
    /// Takes an inhibitor lock; it lasts until the returned fd is closed.
    fn inhibit(&self, what: &str, who: &str, why: &str, mode: &str) -> zbus::Result<OwnedFd>;

    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

/// freedesktop `ScreenSaver` `ActiveChanged`, the portable name.
#[zbus::proxy(
    interface = "org.freedesktop.ScreenSaver",
    default_service = "org.freedesktop.ScreenSaver",
    default_path = "/org/freedesktop/ScreenSaver"
)]
trait ScreenSaver {
    #[zbus(signal)]
    fn active_changed(&self, active: bool) -> zbus::Result<()>;
}

/// GNOME `ScreenSaver` `ActiveChanged`, the name GNOME Shell emits on.
///
/// In its own module because the proxy macro names its generated types
/// (`ActiveChangedStream`, `ActiveChangedArgs`) after the wire signal, which is
/// the same as the freedesktop one.
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

/// Whether [`start`] has already spawned the listener.
static STARTED: AtomicBool = AtomicBool::new(false);

/// Starts the lock/suspend listener on its own thread.
///
/// Returns at once; the listener runs for the life of the process and calls the
/// handlers registered with [`super::subscribe`]. A second call does nothing,
/// so a consumer never ends up with two listeners and duplicate events. Call it
/// after the handlers that must see the first event have subscribed.
pub fn start() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("rustconn-lock-events".to_owned())
        .spawn(run_thread);
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "lock listener: could not spawn its thread");
    }
}

/// Thread body: a current-thread tokio runtime driving [`listen`].
fn run_thread() {
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime.block_on(listen()),
        Err(e) => tracing::warn!(error = %e, "lock listener: could not build its runtime"),
    }
}

/// Subscribes to every source it can reach and dispatches events until all of
/// them have ended.
async fn listen() {
    let login1 = connect_login1().await;
    let (login1, mut sleep_stream) = match login1 {
        Some((proxy, stream)) => (Some(proxy), Some(stream)),
        None => (None, None),
    };
    let mut inhibitor = match &login1 {
        Some(proxy) => take_inhibitor(proxy).await,
        None => None,
    };

    // Both ScreenSaver names are on the session bus; one connection serves
    // both, and is held for as long as their streams are polled.
    let session = match Connection::session().await {
        Ok(conn) => Some(conn),
        Err(e) => {
            tracing::info!(error = %e, "lock listener: session bus unavailable; no lock source");
            None
        }
    };
    let mut fdo_stream = match &session {
        Some(conn) => connect_fdo_saver(conn).await,
        None => None,
    };
    let mut gnome_stream = match &session {
        Some(conn) => connect_gnome_saver(conn).await,
        None => None,
    };

    loop {
        if sleep_stream.is_none() && fdo_stream.is_none() && gnome_stream.is_none() {
            tracing::info!("lock listener: no lock or suspend signal source left; stopping");
            return;
        }
        tokio::select! {
            signal = next_opt(sleep_stream.as_mut()) => match signal {
                Some(signal) => match signal.args().map(|args| args.start) {
                    Ok(true) => {
                        dispatch_blocking(LockEvent::Suspending).await;
                        // Closing the fd releases the delay lock: the handlers
                        // are done, the system may sleep.
                        inhibitor = None;
                        tracing::info!("lock listener: system suspending; handlers ran");
                    }
                    Ok(false) => {
                        if inhibitor.is_none()
                            && let Some(proxy) = &login1
                        {
                            inhibitor = take_inhibitor(proxy).await;
                        }
                    }
                    Err(e) => tracing::debug!(error = %e, "lock listener: unreadable PrepareForSleep"),
                },
                None => sleep_stream = None,
            },
            signal = next_opt(fdo_stream.as_mut()) => match signal {
                Some(signal) => {
                    if signal.args().is_ok_and(|args| args.active) {
                        tracing::info!(source = "org.freedesktop.ScreenSaver", "lock listener: screen locked");
                        dispatch_blocking(LockEvent::ScreenLocked).await;
                    }
                }
                None => fdo_stream = None,
            },
            signal = next_opt(gnome_stream.as_mut()) => match signal {
                Some(signal) => {
                    if signal.args().is_ok_and(|args| args.active) {
                        tracing::info!(source = "org.gnome.ScreenSaver", "lock listener: screen locked");
                        dispatch_blocking(LockEvent::ScreenLocked).await;
                    }
                }
                None => gnome_stream = None,
            },
        }
    }
}

/// Runs the handlers on tokio's blocking pool and waits for all of them.
async fn dispatch_blocking(event: LockEvent) {
    // `dispatch` catches a handler's panic itself, so a `JoinError` here means
    // the task was cancelled: the runtime shut down before the handlers ran.
    if let Err(e) = tokio::task::spawn_blocking(move || super::dispatch(event)).await {
        tracing::warn!(error = %e, ?event, "lock listener: the handler task was cancelled");
    }
}

/// The next item of an optional stream; never ready when the stream is absent,
/// so `select!` simply ignores that branch.
async fn next_opt<S>(stream: Option<&mut S>) -> Option<S::Item>
where
    S: futures::Stream + Unpin,
{
    match stream {
        Some(stream) => stream.next().await,
        None => std::future::pending().await,
    }
}

/// The login1 proxy and its `PrepareForSleep` stream, or `None` when the system
/// bus or login1 is not reachable.
async fn connect_login1() -> Option<(Login1ManagerProxy<'static>, PrepareForSleepStream)> {
    let result = async {
        let conn = Connection::system().await?;
        let proxy = Login1ManagerProxy::builder(&conn)
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        let stream = proxy.receive_prepare_for_sleep().await?;
        zbus::Result::Ok((proxy, stream))
    }
    .await;
    match result {
        Ok(pair) => Some(pair),
        Err(e) => {
            tracing::info!(error = %e, "lock listener: login1 PrepareForSleep unavailable");
            None
        }
    }
}

/// A `delay` inhibitor for `sleep`, or `None` when logind refuses it. Without
/// one the handlers still run on suspend, but the system may not wait for them.
async fn take_inhibitor(proxy: &Login1ManagerProxy<'_>) -> Option<OwnedFd> {
    match proxy
        .inhibit(
            "sleep",
            "RustConn",
            "Remove the SSH keys RustConn added before the system sleeps",
            "delay",
        )
        .await
    {
        Ok(fd) => Some(fd),
        Err(e) => {
            tracing::info!(error = %e, "lock listener: no login1 delay lock; suspend will not wait");
            None
        }
    }
}

/// The freedesktop `ActiveChanged` stream, or `None` where it is not served.
async fn connect_fdo_saver(conn: &Connection) -> Option<ActiveChangedStream> {
    let result = async {
        let proxy = ScreenSaverProxy::builder(conn)
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        proxy.receive_active_changed().await
    }
    .await;
    match result {
        Ok(stream) => Some(stream),
        Err(e) => {
            tracing::info!(error = %e, "lock listener: org.freedesktop.ScreenSaver unavailable");
            None
        }
    }
}

/// The GNOME `ActiveChanged` stream, or `None` off GNOME.
async fn connect_gnome_saver(conn: &Connection) -> Option<gnome::ActiveChangedStream> {
    let result = async {
        let proxy = gnome::GnomeScreenSaverProxy::builder(conn)
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        proxy.receive_active_changed().await
    }
    .await;
    match result {
        Ok(stream) => Some(stream),
        Err(e) => {
            tracing::info!(error = %e, "lock listener: org.gnome.ScreenSaver unavailable");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An absent source must never be ready, or `select!` would spin on it.
    #[tokio::test]
    async fn next_opt_on_an_absent_stream_never_resolves() {
        let absent: Option<&mut futures::stream::Iter<std::vec::IntoIter<u8>>> = None;
        let waited =
            tokio::time::timeout(std::time::Duration::from_millis(20), next_opt(absent)).await;
        assert!(waited.is_err(), "an absent stream resolved: {waited:?}");
    }

    /// A present source yields its items, then `None` once it ends, which is
    /// what makes the listener drop that branch.
    #[tokio::test]
    async fn next_opt_yields_then_reports_the_end() {
        let mut stream = futures::stream::iter(vec![true, false]);
        assert_eq!(next_opt(Some(&mut stream)).await, Some(true));
        assert_eq!(next_opt(Some(&mut stream)).await, Some(false));
        assert_eq!(next_opt(Some(&mut stream)).await, None);
    }
}
