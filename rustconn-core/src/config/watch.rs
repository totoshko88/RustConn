//! Notices when another process rewrites the connection files.
//!
//! A `rustconn-cli add` (or an editor, or a second instance) replaces
//! `connections.toml`, `groups.toml` or `trash.toml` underneath a running GUI.
//! [`ConfigWatcher`] records that, so the GUI can merge the files in with
//! `ConnectionManager::reload_from_disk` and show the new entries without a
//! restart. The merge on save (see [`super::merge`]) is what keeps those entries
//! from being overwritten; this only makes them visible.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::error::{ConfigError, ConfigResult};

/// The files a change to which matters. Temp files of an atomic write start with
/// a dot and never match.
const WATCHED_FILES: [&str; 3] = ["connections.toml", "groups.toml", "trash.toml"];

/// Watches the configuration directory for changes to the connection files.
///
/// Cheap by design: the notify callback only stamps the time of the latest
/// change. The owner polls [`Self::take_changed`] from wherever it wants to react
/// (the GTK main loop), which also debounces a burst into one reaction.
///
/// This process's own saves count too, since a file cannot tell who wrote it. A
/// reload after one finds nothing new and changes nothing.
// ponytail: a reload re-reads three small TOML files after each of this process's
// own saves; fine at a few hundred connections. Skip it by remembering the
// mtime this process wrote if the lists grow far past that.
pub struct ConfigWatcher {
    /// Kept alive to keep the watch.
    _watcher: RecommendedWatcher,
    /// When the latest relevant event arrived, until it is taken.
    last_event: Arc<Mutex<Option<Instant>>>,
}

impl ConfigWatcher {
    /// Starts watching `config_dir`.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Write`] when the directory cannot be watched, for
    /// instance when it does not exist or the inotify limit is reached. The
    /// caller can run without a watcher: saves still merge, only live updates are
    /// lost.
    pub fn new(config_dir: &Path) -> ConfigResult<Self> {
        let last_event: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
        let sink = Arc::clone(&last_event);

        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            let event = match res {
                Ok(event) => event,
                Err(e) => {
                    tracing::warn!(error = %e, "Config watcher error");
                    return;
                }
            };
            if matches!(event.kind, EventKind::Access(_)) {
                return;
            }
            let relevant = event.paths.iter().any(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| WATCHED_FILES.contains(&name))
            });
            if relevant {
                *sink
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Instant::now());
            }
        })
        .map_err(|e| ConfigError::Write(format!("Failed to start the config watcher: {e}")))?;

        watcher
            .watch(config_dir, RecursiveMode::NonRecursive)
            .map_err(|e| {
                ConfigError::Write(format!("Failed to watch {}: {e}", config_dir.display()))
            })?;

        Ok(Self {
            _watcher: watcher,
            last_event,
        })
    }

    /// Whether a connection file changed and has stayed quiet for `quiet`.
    ///
    /// Returns `true` once per burst of changes: a write is several events (temp
    /// file, rename), and a second process may follow within moments, so waiting
    /// for quiet turns them into one reload.
    #[must_use]
    pub fn take_changed(&self, quiet: Duration) -> bool {
        let mut last = self
            .last_event
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match *last {
            Some(at) if at.elapsed() >= quiet => {
                *last = None;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Polls `watcher` until it reports a change or `limit` passes.
    fn changed_within(watcher: &ConfigWatcher, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if watcher.take_changed(Duration::ZERO) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        false
    }

    #[test]
    fn reports_a_rewrite_of_a_connection_file() {
        let dir = tempfile::tempdir().unwrap();
        let watcher = ConfigWatcher::new(dir.path()).unwrap();

        crate::fs_atomic::atomic_write(
            &dir.path().join("connections.toml"),
            b"connections = []\n",
            crate::fs_atomic::MODE_PRIVATE,
        )
        .unwrap();

        assert!(changed_within(&watcher, Duration::from_secs(5)));
        // One write is several events (temp file, rename) and they do not all
        // land before the first is taken. Let the rest arrive and drain them.
        std::thread::sleep(Duration::from_millis(300));
        let _ = watcher.take_changed(Duration::ZERO);
        // Taken: nothing more to report until the next change.
        std::thread::sleep(Duration::from_millis(300));
        assert!(!watcher.take_changed(Duration::ZERO));
    }

    #[test]
    fn ignores_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let watcher = ConfigWatcher::new(dir.path()).unwrap();

        std::fs::write(dir.path().join("config.toml"), b"x = 1\n").unwrap();
        std::fs::write(dir.path().join(".connections.toml.1.2.3.tmp"), b"x\n").unwrap();

        assert!(!changed_within(&watcher, Duration::from_millis(500)));
    }

    #[test]
    fn waits_for_quiet() {
        let dir = tempfile::tempdir().unwrap();
        let watcher = ConfigWatcher::new(dir.path()).unwrap();

        std::fs::write(dir.path().join("groups.toml"), b"groups = []\n").unwrap();
        // Let the event arrive without taking it.
        std::thread::sleep(Duration::from_millis(300));

        assert!(!watcher.take_changed(Duration::from_secs(60)));
        assert!(watcher.take_changed(Duration::ZERO));
    }
}
