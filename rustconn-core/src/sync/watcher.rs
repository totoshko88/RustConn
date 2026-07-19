//! File system watcher for Cloud Sync.
//!
//! Monitors the sync directory for `.rcn` file changes using the [`notify`]
//! crate (inotify on Linux, kqueue on macOS). Includes a 3-second debounce
//! to handle partial writes from cloud sync clients (Google Drive, Dropbox,
//! Syncthing, etc.).
//!
//! Master group files are filtered out to prevent circular export→import loops.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tracing::{debug, warn};

use super::group_export::SyncError;

/// Debounce interval for file change events.
///
/// Cloud sync clients often write files in multiple chunks, so we wait 3
/// seconds after the last event before triggering the callback.
const DEBOUNCE_SECS: u64 = 3;

/// Watches the sync directory for `.rcn` file changes.
///
/// Uses the `notify` crate's [`RecommendedWatcher`] (inotify on Linux) with
/// a 3-second debounce. Master group files can be excluded via
/// [`add_master_file`] to prevent circular export→import.
pub struct SyncFileWatcher {
    /// The underlying notify watcher. Kept alive to maintain the watch.
    _watcher: RecommendedWatcher,
    /// Filenames of Master group files to ignore (prevents circular sync).
    master_files: Arc<Mutex<HashSet<String>>>,
    /// Handle to the debounce thread so we can signal it to stop.
    stop_flag: Arc<Mutex<bool>>,
    /// Join handle for the debounce thread — joined on drop for clean shutdown.
    debounce_thread: Option<std::thread::JoinHandle<()>>,
}

impl SyncFileWatcher {
    /// Creates a new file watcher on the given sync directory.
    ///
    /// The `callback` is invoked (on a background thread) for each `.rcn`
    /// file that changes, after the 3-second debounce window expires.
    /// Files registered as Master via [`add_master_file`] are filtered out.
    ///
    /// # Errors
    ///
    /// Returns [`SyncError::Io`] if the directory cannot be watched.
    pub fn new(
        sync_dir: &Path,
        callback: impl Fn(PathBuf) + Send + 'static,
    ) -> Result<Self, SyncError> {
        let master_files: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let stop_flag = Arc::new(Mutex::new(false));

        // Pending events: filename → last event time
        let pending: Arc<Mutex<HashMap<PathBuf, Instant>>> = Arc::new(Mutex::new(HashMap::new()));

        let master_files_clone = Arc::clone(&master_files);
        let pending_clone = Arc::clone(&pending);

        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            let Ok(event) = res else {
                if let Err(ref e) = res {
                    warn!("File watcher error: {e}");
                }
                return;
            };

            // Only care about create/modify events
            if !matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                return;
            }

            for path in &event.paths {
                // Only .rcn files
                let Some(ext) = path.extension() else {
                    continue;
                };
                if ext != "rcn" {
                    continue;
                }

                // Skip temp files from atomic writes
                if path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(".rcn.tmp"))
                {
                    continue;
                }

                // Filter Master group files
                if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
                    let masters = master_files_clone
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if masters.contains(filename) {
                        debug!(file = %filename, "Ignoring Master group file change");
                        continue;
                    }
                }

                // Record/update pending event
                let mut map = pending_clone
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                map.insert(path.clone(), Instant::now());
            }
        })
        .map_err(|e| SyncError::Io(std::io::Error::other(e)))?;

        watcher
            .watch(sync_dir, RecursiveMode::NonRecursive)
            .map_err(|e| SyncError::Io(std::io::Error::other(e)))?;

        // Spawn debounce thread
        let pending_debounce = Arc::clone(&pending);
        let stop_debounce = Arc::clone(&stop_flag);
        let debounce_handle = std::thread::Builder::new()
            .name("sync-file-watcher-debounce".into())
            .spawn(move || {
                let debounce = Duration::from_secs(DEBOUNCE_SECS);
                let tick = Duration::from_millis(500);

                loop {
                    std::thread::sleep(tick);

                    // Check stop flag
                    if *stop_debounce.lock().unwrap_or_else(|e| {
                        warn!("Debounce stop_flag mutex poisoned, recovering");
                        e.into_inner()
                    }) {
                        break;
                    }

                    // Collect ready paths
                    let now = Instant::now();
                    let mut ready = Vec::new();
                    {
                        let mut map =
                            pending_debounce.lock().unwrap_or_else(|e| {
                                warn!("Debounce pending mutex poisoned, recovering");
                                e.into_inner()
                            });
                        map.retain(|path, last_event| {
                            if now.duration_since(*last_event) >= debounce {
                                ready.push(path.clone());
                                false // remove from pending
                            } else {
                                true // keep waiting
                            }
                        });
                    }

                    // Fire callbacks
                    for path in ready {
                        debug!(file = %path.display(), "Debounced file change — triggering callback");
                        callback(path);
                    }
                }
            })
            .map_err(SyncError::Io)?;

        Ok(Self {
            _watcher: watcher,
            master_files,
            stop_flag,
            debounce_thread: Some(debounce_handle),
        })
    }

    /// Registers a filename as a Master group file.
    ///
    /// Changes to this file will be ignored by the watcher, preventing
    /// circular export→import loops.
    pub fn add_master_file(&self, filename: &str) {
        let mut masters = self.master_files.lock().unwrap_or_else(|e| {
            tracing::warn!("Master files mutex poisoned, recovering");
            e.into_inner()
        });
        masters.insert(filename.to_owned());
    }

    /// Removes a filename from the Master group filter.
    pub fn remove_master_file(&self, filename: &str) {
        let mut masters = self.master_files.lock().unwrap_or_else(|e| {
            tracing::warn!("Master files mutex poisoned, recovering");
            e.into_inner()
        });
        masters.remove(filename);
    }

    /// Stops the file watcher and its debounce thread.
    pub fn stop(&mut self) {
        {
            let mut flag = self.stop_flag.lock().unwrap_or_else(|e| {
                tracing::warn!("Stop flag mutex poisoned, recovering");
                e.into_inner()
            });
            *flag = true;
        }
        // Join the debounce thread for clean shutdown
        if let Some(handle) = self.debounce_thread.take()
            && let Err(e) = handle.join()
        {
            tracing::warn!("Debounce thread panicked during join: {e:?}");
        }
    }
}

impl Drop for SyncFileWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn master_file_filtering() {
        let dir = tempfile::TempDir::new().unwrap();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let watcher = SyncFileWatcher::new(dir.path(), move |_path| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();

        watcher.add_master_file("production.rcn");

        // Write a master file — should be ignored
        std::fs::write(dir.path().join("production.rcn"), "{}").unwrap();

        // Write a non-master file — should trigger callback after debounce
        std::fs::write(dir.path().join("staging.rcn"), "{}").unwrap();

        // Wait for debounce (3s) + some margin
        std::thread::sleep(Duration::from_secs(4));

        // Only the non-master file should have triggered
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn ignores_non_rcn_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = Arc::clone(&counter);

        let _watcher = SyncFileWatcher::new(dir.path(), move |_path| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();

        // Write non-.rcn files
        std::fs::write(dir.path().join("notes.txt"), "hello").unwrap();
        std::fs::write(dir.path().join("data.json"), "{}").unwrap();

        std::thread::sleep(Duration::from_secs(4));

        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn stop_terminates_cleanly() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut watcher = SyncFileWatcher::new(dir.path(), |_| {}).unwrap();
        watcher.stop();
        // Should not panic or hang
    }
}
