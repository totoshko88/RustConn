//! Configuration manager for TOML file operations
//!
//! This module provides the `ConfigManager` which handles loading and saving
//! configuration files for connections, groups, snippets, and application settings.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use fs2::FileExt;

use super::settings::AppSettings;
use super::version_skew::{self, Existing, RUNNING_VERSION, is_newer_than_running};
use crate::cluster::Cluster;
use crate::error::{ConfigError, ConfigResult};
use crate::models::{
    Connection, ConnectionGroup, ConnectionHistoryEntry, ConnectionTemplate, Snippet,
    WorkspaceProfile,
};
use crate::sync::tombstone::Tombstone;

/// File names for configuration files
const CONNECTIONS_FILE: &str = "connections.toml";
const GROUPS_FILE: &str = "groups.toml";
const SNIPPETS_FILE: &str = "snippets.toml";
const CLUSTERS_FILE: &str = "clusters.toml";
const TEMPLATES_FILE: &str = "templates.toml";
const HISTORY_FILE: &str = "history.toml";
const TRASH_FILE: &str = "trash.toml";
const WORKSPACE_PROFILES_FILE: &str = "workspace_profiles.toml";
const TOMBSTONES_FILE: &str = "tombstones.toml";
const CONFIG_FILE: &str = "config.toml";

/// How long a config write waits for the `.lock` another *process* holds.
///
/// Bounded on purpose. `save_settings` runs synchronously on the GTK main
/// thread, so an unbounded `flock(LOCK_EX)` there is an unbounded UI freeze —
/// and a stale lock is easy to come by: a `rustconn-cli` stopped in a debugger,
/// a second instance wedged on a hung network filesystem. Generous next to the
/// work it protects (serialize a few hundred KB, fsync, rename).
const LOCK_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Poll interval while waiting for the lock.
///
/// `fs2` offers no timed acquire, so the wait is a poll. Short enough that the
/// ordinary case — a lock held for the length of one fsync — is barely delayed.
const LOCK_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(25);

/// Serializes config writes inside this process.
///
/// `flock(2)` is held per *open file description*, so two `acquire_lock()` calls
/// contend even on the same thread — and the app has four independent writers:
/// the three debounce workers in [`crate::connection::ConnectionManager`]
/// (connections, groups, trash), the history flusher on its own thread, and the
/// synchronous `save_settings` calls from GTK callbacks. A single connect starts
/// two of those 2-second debounces at the same instant, so they woke together
/// and one found the lock taken — which is what produced a steady stream of
/// "waiting for another rustconn instance" with no other instance running.
///
/// Taking this first means the in-process writers queue instead of racing, and
/// the `flock` below is left doing the job it is actually for: keeping *other
/// processes* out. Held only inside [`ConfigManager::write_locked`], which does
/// not call itself, so it cannot deadlock against itself.
static CONFIG_WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Marked config files a newer `RustConn` wrote, by path, with the version that wrote each.
type NewerFiles = std::collections::BTreeMap<PathBuf, String>;

/// Wrapper for serializing a list of connections
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct ConnectionsFile {
    /// Version of the `RustConn` that wrote the file; see [`AppSettings::written_by`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    written_by: Option<String>,
    #[serde(default)]
    connections: Vec<Connection>,
}

/// Wrapper for serializing a list of groups
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct GroupsFile {
    /// Version of the `RustConn` that wrote the file; see [`AppSettings::written_by`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    written_by: Option<String>,
    #[serde(default)]
    groups: Vec<ConnectionGroup>,
}

/// A file wrapper that records which `RustConn` version wrote it.
trait Marked {
    /// The `written_by` marker as loaded.
    fn marker(&self) -> Option<&str>;
}

impl Marked for ConnectionsFile {
    fn marker(&self) -> Option<&str> {
        self.written_by.as_deref()
    }
}

impl Marked for GroupsFile {
    fn marker(&self) -> Option<&str> {
        self.written_by.as_deref()
    }
}

impl Marked for SnippetsFile {
    fn marker(&self) -> Option<&str> {
        self.written_by.as_deref()
    }
}

impl Marked for ClustersFile {
    fn marker(&self) -> Option<&str> {
        self.written_by.as_deref()
    }
}

impl Marked for TemplatesFile {
    fn marker(&self) -> Option<&str> {
        self.written_by.as_deref()
    }
}

impl Marked for WorkspaceProfilesFile {
    fn marker(&self) -> Option<&str> {
        self.written_by.as_deref()
    }
}

impl Marked for HistoryFile {
    fn marker(&self) -> Option<&str> {
        self.written_by.as_deref()
    }
}

impl Marked for TombstonesFile {
    fn marker(&self) -> Option<&str> {
        self.written_by.as_deref()
    }
}

impl Marked for TrashFile {
    fn marker(&self) -> Option<&str> {
        self.written_by.as_deref()
    }
}

/// Wrapper for serializing a list of snippets
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct SnippetsFile {
    /// Version of the `RustConn` that wrote the file; see [`AppSettings::written_by`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    written_by: Option<String>,
    #[serde(default)]
    snippets: Vec<Snippet>,
}

/// Wrapper for serializing a list of clusters
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct ClustersFile {
    /// Version of the `RustConn` that wrote the file; see [`AppSettings::written_by`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    written_by: Option<String>,
    #[serde(default)]
    clusters: Vec<Cluster>,
}

/// Wrapper for serializing a list of templates
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct TemplatesFile {
    /// Version of the `RustConn` that wrote the file; see [`AppSettings::written_by`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    written_by: Option<String>,
    #[serde(default)]
    templates: Vec<ConnectionTemplate>,
}

/// Wrapper for serializing connection history
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct HistoryFile {
    /// Version of the `RustConn` that wrote the file; see [`AppSettings::written_by`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    written_by: Option<String>,
    #[serde(default)]
    entries: Vec<ConnectionHistoryEntry>,
}

/// Wrapper for serializing Simple Sync tombstones
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct TombstonesFile {
    /// Version of the `RustConn` that wrote the file; see [`AppSettings::written_by`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    written_by: Option<String>,
    #[serde(default)]
    tombstones: Vec<Tombstone>,
}

/// Wrapper for serializing trash (deleted items)
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct TrashFile {
    /// Version of the `RustConn` that wrote the file; see [`AppSettings::written_by`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    written_by: Option<String>,
    #[serde(default)]
    pub connections: Vec<(Connection, chrono::DateTime<chrono::Utc>)>,
    #[serde(default)]
    pub groups: Vec<(ConnectionGroup, chrono::DateTime<chrono::Utc>)>,
}

/// Wrapper for serializing workspace profiles
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct WorkspaceProfilesFile {
    /// Version of the `RustConn` that wrote the file; see [`AppSettings::written_by`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    written_by: Option<String>,
    #[serde(default)]
    profiles: Vec<WorkspaceProfile>,
}

/// Configuration manager for `RustConn`
///
/// Handles loading and saving configuration files in TOML format.
/// Configuration is stored in `~/.config/rustconn/` by default.
#[derive(Debug, Clone)]
pub struct ConfigManager {
    /// Base directory for configuration files
    config_dir: PathBuf,
    /// Whether `ensure_config_dir()` has already succeeded (avoids repeated syscalls)
    dir_ensured: std::sync::Arc<AtomicBool>,
    /// Marked files this process loaded that a newer `RustConn` wrote and that
    /// have not been backed up yet.
    ///
    /// Shared across clones like `dir_ensured`, because a file is rarely saved
    /// through the handle that loaded it: `ConnectionManager` saves on debounce
    /// workers that hold clones of their own. [`Self::write_locked`] takes a file
    /// out of here the first time it overwrites it, after copying it aside.
    newer_files: std::sync::Arc<std::sync::Mutex<NewerFiles>>,
    /// Files whose `written_by` marker this process has already read, by a load
    /// or by [`Self::probe_unseen_marker`].
    ///
    /// Shared across clones for the same reason as `newer_files`. A save of a
    /// path not in here reads the marker on disk first, once, so a file that was
    /// never loaded still gets its backup.
    marker_seen: std::sync::Arc<std::sync::Mutex<std::collections::BTreeSet<PathBuf>>>,
}

impl ConfigManager {
    /// File name of the application settings, for [`Self::quarantine_unreadable`].
    pub const SETTINGS_FILE_NAME: &str = CONFIG_FILE;
    /// File name of the saved clusters, for [`Self::quarantine_unreadable`].
    pub const CLUSTERS_FILE_NAME: &str = CLUSTERS_FILE;
    /// File name of the connection history, for [`Self::quarantine_unreadable`].
    pub const HISTORY_FILE_NAME: &str = HISTORY_FILE;
    /// File name of the Simple Sync tombstones, for [`Self::quarantine_unreadable`].
    pub const TOMBSTONES_FILE_NAME: &str = TOMBSTONES_FILE;

    /// Creates a new `ConfigManager` with the default configuration directory
    ///
    /// The default directory is `~/.config/rustconn/`
    ///
    /// # Errors
    ///
    /// Returns an error if the home directory cannot be determined.
    pub fn new() -> ConfigResult<Self> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| ConfigError::NotFound(PathBuf::from("~/.config")))?
            .join("rustconn");
        Ok(Self {
            config_dir,
            dir_ensured: std::sync::Arc::new(AtomicBool::new(false)),
            newer_files: std::sync::Arc::default(),
            marker_seen: std::sync::Arc::default(),
        })
    }

    /// Creates a new `ConfigManager` with a custom configuration directory
    ///
    /// This is useful for testing or non-standard configurations.
    #[must_use]
    pub fn with_config_dir(config_dir: PathBuf) -> Self {
        Self {
            config_dir,
            dir_ensured: std::sync::Arc::new(AtomicBool::new(false)),
            newer_files: std::sync::Arc::default(),
            marker_seen: std::sync::Arc::default(),
        }
    }

    /// Returns the configuration directory path
    #[must_use]
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Ensures the configuration directory exists
    ///
    /// Creates the directory and any parent directories if they don't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created.
    pub fn ensure_config_dir(&self) -> ConfigResult<()> {
        // Fast path: directory already ensured in this process lifetime
        if self.dir_ensured.load(Ordering::Relaxed) {
            return Ok(());
        }

        if !self.config_dir.exists() {
            fs::create_dir_all(&self.config_dir).map_err(|e| {
                ConfigError::Write(format!(
                    "Failed to create config directory {}: {}",
                    self.config_dir.display(),
                    e
                ))
            })?;
        }

        // Restrict directory permissions to owner-only (0700)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.config_dir, fs::Permissions::from_mode(0o700)).map_err(
                |e| {
                    ConfigError::Write(format!(
                        "Failed to set permissions on {}: {}",
                        self.config_dir.display(),
                        e
                    ))
                },
            )?;
        }

        self.dir_ensured.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Acquires an exclusive advisory lock on the configuration directory.
    ///
    /// Returns the lock file handle, which holds the lock until dropped. When the
    /// lock is held elsewhere this waits for it, but only up to
    /// [`LOCK_WAIT_TIMEOUT`] — it used to block forever, which on the GTK main
    /// thread means a frozen window with no way out.
    ///
    /// Callers that write should go through [`Self::write_locked`], which also
    /// takes [`CONFIG_WRITE_LOCK`] so this process's own writers do not contend
    /// here.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Lock`] if the lock file cannot be created, if
    /// locking fails outright, or if the lock is still held after
    /// [`LOCK_WAIT_TIMEOUT`].
    pub fn acquire_lock(&self) -> ConfigResult<fs::File> {
        self.ensure_config_dir()?;
        let lock_path = self.config_dir.join(".lock");
        let lock_file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|e| {
                ConfigError::Lock(format!(
                    "Failed to open lock file {}: {}",
                    lock_path.display(),
                    e
                ))
            })?;

        if lock_file.try_lock_exclusive().is_ok() {
            return Ok(lock_file);
        }

        // Busy. Poll to a deadline rather than blocking indefinitely. The message
        // no longer claims another *instance* holds it: with CONFIG_WRITE_LOCK in
        // front of every write, reaching here does mean another process, but this
        // function is public and says only what it can actually observe.
        tracing::info!(
            lock = %lock_path.display(),
            timeout_secs = LOCK_WAIT_TIMEOUT.as_secs(),
            "Config lock is held elsewhere; waiting"
        );
        let deadline = std::time::Instant::now() + LOCK_WAIT_TIMEOUT;
        loop {
            std::thread::sleep(LOCK_POLL_INTERVAL);
            if lock_file.try_lock_exclusive().is_ok() {
                return Ok(lock_file);
            }
            if std::time::Instant::now() >= deadline {
                return Err(ConfigError::Lock(format!(
                    "timed out after {}s waiting for the config lock on {}; \
                     another process may be holding it",
                    LOCK_WAIT_TIMEOUT.as_secs(),
                    lock_path.display()
                )));
            }
        }
    }

    /// Writes `content` to `path` atomically, holding both write locks.
    ///
    /// The single place config bytes reach the disk: temp file, owner-only
    /// permissions, fsync, rename. [`Self::save_toml_file`] and
    /// [`Self::save_toml_file_async`] both funnel through here, which is how the
    /// two stay in step — they used to be separate copies of this sequence, each
    /// commented "matches the other". Being the one place is also why the backup
    /// of a file a newer `RustConn` wrote happens here: see
    /// [`Self::back_up_newer_file`].
    fn write_locked(&self, path: &Path, content: &str) -> ConfigResult<()> {
        // In-process writers queue here; see CONFIG_WRITE_LOCK. The guard holds
        // `()`, so a poisoned mutex carries no invalid state and recovering is
        // strictly better than propagating a panic from an unrelated writer.
        let _serialized = CONFIG_WRITE_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // Advisory lock against other processes (released on drop)
        let _lock = self.acquire_lock()?;

        // A file a newer RustConn wrote is copied aside before this process first
        // replaces it. Under both locks, so the copy holds exactly the bytes the
        // rename below replaces; a copy that fails fails this write.
        self.back_up_newer_file(path)?;

        let temp_path = path.with_extension("tmp");

        fs::write(&temp_path, content).map_err(|e| {
            ConfigError::Write(format!("Failed to write {}: {}", temp_path.display(), e))
        })?;

        // Restrict file permissions to owner-only (0600)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o600)).map_err(|e| {
                ConfigError::Write(format!(
                    "Failed to set permissions on {}: {}",
                    temp_path.display(),
                    e
                ))
            })?;
        }

        // Sync data to disk before rename
        {
            let file = fs::File::open(&temp_path).map_err(|e| {
                ConfigError::Write(format!(
                    "Failed to open {} for sync: {}",
                    temp_path.display(),
                    e
                ))
            })?;
            file.sync_all().map_err(|e| {
                ConfigError::Write(format!("Failed to sync {}: {}", temp_path.display(), e))
            })?;
        }

        fs::rename(&temp_path, path).map_err(|e| {
            ConfigError::Write(format!(
                "Failed to rename {} to {}: {}",
                temp_path.display(),
                path.display(),
                e
            ))
        })?;

        Ok(())
    }

    // ========== Version Skew ==========

    /// Returns the marked files this process loaded that a newer `RustConn` wrote.
    ///
    /// Each entry is a file's path and the version that wrote it, in path order.
    /// A file drops out once this process has backed it up on the way to its
    /// first overwrite, or has loaded it again with a marker that is not newer.
    #[must_use]
    pub fn newer_version_files(&self) -> Vec<(PathBuf, String)> {
        self.lock_newer_files()
            .iter()
            .map(|(path, version)| (path.clone(), version.clone()))
            .collect()
    }

    /// Copies an unreadable config file aside before a fallback lets a save replace it.
    ///
    /// `file_name` names a file in the configuration directory, such as
    /// [`Self::SETTINGS_FILE_NAME`]. The copy is `<file>.unreadable-<UTC
    /// timestamp>` beside it, byte for byte and owner-only. Call this before
    /// falling back to defaults after a [`ConfigError::Deserialize`]: the next
    /// routine save writes that fallback over the file. An earlier copy holding
    /// the same bytes is returned instead of a new one.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Validation`] if `file_name` is not a plain file
    /// name, [`ConfigError::Parse`] if the file cannot be read, and
    /// [`ConfigError::Write`] if the copy cannot be written.
    pub fn quarantine_unreadable(&self, file_name: &str) -> ConfigResult<PathBuf> {
        let mut components = Path::new(file_name).components();
        let plain = matches!(
            (components.next(), components.next()),
            (Some(std::path::Component::Normal(_)), None)
        );
        if !plain {
            return Err(ConfigError::Validation {
                field: "file_name".to_string(),
                reason: format!("{file_name:?} is not a plain file name"),
            });
        }
        version_skew::quarantine_file(&self.config_dir.join(file_name))
    }

    /// Locks the newer-version flags; see the `newer_files` field.
    ///
    /// A poisoned lock is recovered: every change to the map is one insert or
    /// one remove, so a panic elsewhere cannot leave it half-updated.
    fn lock_newer_files(&self) -> std::sync::MutexGuard<'_, NewerFiles> {
        self.newer_files
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Records whether the file at `path` was written by a newer `RustConn`.
    ///
    /// A newer marker flags the file for [`Self::back_up_newer_file`] and is
    /// logged once. Anything else — this version, an older one, no marker, or a
    /// marker that is not a plain release — clears a flag left by an earlier load
    /// of the same file: what is on disk now holds nothing newer.
    fn note_written_by(&self, path: &Path, written_by: Option<&str>) {
        self.lock_marker_seen().insert(path.to_path_buf());
        let Some(version) = written_by.filter(|v| is_newer_than_running(v)) else {
            self.lock_newer_files().remove(path);
            return;
        };
        let first_sighting = self
            .lock_newer_files()
            .insert(path.to_path_buf(), version.to_owned())
            .is_none();
        if first_sighting {
            tracing::warn!(
                file = %path.display(),
                written_by = version,
                running = RUNNING_VERSION,
                "Config file was written by a newer RustConn; backed up before the first save"
            );
        }
    }

    /// Locks the set of files whose marker was read; see the `marker_seen` field.
    ///
    /// A poisoned lock is recovered for the same reason as
    /// [`Self::lock_newer_files`]: every change is one insert.
    fn lock_marker_seen(&self) -> std::sync::MutexGuard<'_, std::collections::BTreeSet<PathBuf>> {
        self.marker_seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Reads the marker of a file this process is about to save without having loaded it.
    ///
    /// The backup used to depend on a load in the same process having flagged
    /// the file, so a save with no load before it — `rustconn-cli history
    /// clear` writes an empty history without reading the old one — replaced a
    /// newer `RustConn`'s file with no copy. Runs once per path per process:
    /// after it, either the marker has been noted or this process's own write
    /// has replaced it. A file that is missing, unreadable or not TOML has no
    /// marker to go by and is left unflagged, as a load would leave it.
    fn probe_unseen_marker(&self, path: &Path) {
        if !self.lock_marker_seen().insert(path.to_path_buf()) {
            return;
        }
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                tracing::warn!(
                    file = %path.display(),
                    error = %e,
                    "Could not read a config file's version marker before replacing it"
                );
                return;
            }
        };
        if let Ok(written_by) = version_skew::probe_written_by(&bytes) {
            self.note_written_by(path, written_by.as_deref());
        }
    }

    /// Copies a flagged file to `<file>.<version>.bak` before its first overwrite here.
    ///
    /// Runs inside [`Self::write_locked`] with both locks held, so the copy holds
    /// exactly the bytes about to be replaced, and only for a file flagged as
    /// written by a newer `RustConn` — by one of this process's loads or, for a
    /// file it never loaded, by [`Self::probe_unseen_marker`]. Once the copy is
    /// on disk the flag is cleared: one backup per file per process, however many
    /// saves follow. An existing backup of the same name is replaced; it holds an
    /// older state of what that same version wrote.
    ///
    /// The version is read again from the bytes on disk. Another process may have
    /// rewritten the file since it was loaded; if this or an older version did,
    /// nothing newer is left to protect, and copying it would overwrite a good
    /// backup with a worse one.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Write`] if the file cannot be read or the copy
    /// cannot be written. The flag stays set, so the next save tries again: a
    /// newer version's file is never overwritten without its copy.
    fn back_up_newer_file(&self, path: &Path) -> ConfigResult<()> {
        self.probe_unseen_marker(path);

        // Bound on its own line so the guard is dropped before anything below
        // takes the lock again.
        let flagged = self.lock_newer_files().get(path).cloned();
        let Some(flagged) = flagged else {
            return Ok(());
        };

        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            // Gone since it was loaded: there is nothing left to copy.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.lock_newer_files().remove(path);
                return Ok(());
            }
            Err(e) => {
                return Err(ConfigError::Write(format!(
                    "Failed to read {} to back it up: {e}",
                    path.display()
                )));
            }
        };

        let version = match version_skew::probe_written_by(&bytes) {
            Ok(Some(on_disk)) if is_newer_than_running(&on_disk) => on_disk,
            // Rewritten since by this version or an older one.
            Ok(_) => {
                self.lock_newer_files().remove(path);
                return Ok(());
            }
            // No longer TOML at all: keep it anyway, under the version flagged.
            Err(_) => flagged,
        };

        let mut backup_name = path.file_name().unwrap_or_default().to_os_string();
        backup_name.push(format!(".{version}.bak"));
        let backup = path.with_file_name(backup_name);

        version_skew::write_owner_only(&backup, &bytes, Existing::Replace).map_err(|e| {
            ConfigError::Write(format!(
                "Failed to back up {} to {}: {e}",
                path.display(),
                backup.display()
            ))
        })?;
        self.lock_newer_files().remove(path);

        tracing::info!(
            file = %path.display(),
            backup = %backup.display(),
            written_by = %version,
            "Backed up a config file written by a newer RustConn before changing it"
        );
        Ok(())
    }

    /// Ensures the logs directory exists
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created.
    pub fn ensure_logs_dir(&self) -> ConfigResult<PathBuf> {
        let logs_dir = self.config_dir.join("logs");
        if !logs_dir.exists() {
            fs::create_dir_all(&logs_dir).map_err(|e| {
                ConfigError::Write(format!(
                    "Failed to create logs directory {}: {}",
                    logs_dir.display(),
                    e
                ))
            })?;
        }
        Ok(logs_dir)
    }

    // ========== Connections ==========

    /// Loads connections from the configuration file
    ///
    /// Returns an empty vector if the file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load_connections(&self) -> ConfigResult<Vec<Connection>> {
        let path = self.config_dir.join(CONNECTIONS_FILE);
        let file: ConnectionsFile = self.load_marked_toml_file(&path)?;
        Ok(file.connections)
    }

    /// Saves connections to the configuration file
    ///
    /// Creates the configuration directory if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save_connections(&self, connections: &[Connection]) -> ConfigResult<()> {
        self.ensure_config_dir()?;
        let path = self.config_dir.join(CONNECTIONS_FILE);
        let file = ConnectionsFile {
            written_by: Some(RUNNING_VERSION.to_owned()),
            connections: connections.to_vec(),
        };
        self.save_toml_file(&path, &file)
    }

    /// Saves connections to the configuration file asynchronously
    ///
    /// Creates the configuration directory if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub async fn save_connections_async(&self, connections: &[Connection]) -> ConfigResult<()> {
        self.ensure_config_dir()?;
        let path = self.config_dir.join(CONNECTIONS_FILE);
        let file = ConnectionsFile {
            written_by: Some(RUNNING_VERSION.to_owned()),
            connections: connections.to_vec(),
        };
        self.save_toml_file_async(&path, &file).await
    }

    // ========== Groups ==========

    /// Loads connection groups from the configuration file
    ///
    /// Returns an empty vector if the file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load_groups(&self) -> ConfigResult<Vec<ConnectionGroup>> {
        let path = self.config_dir.join(GROUPS_FILE);
        let file: GroupsFile = self.load_marked_toml_file(&path)?;
        Ok(file.groups)
    }

    /// Saves connection groups to the configuration file
    ///
    /// Creates the configuration directory if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save_groups(&self, groups: &[ConnectionGroup]) -> ConfigResult<()> {
        self.ensure_config_dir()?;
        let path = self.config_dir.join(GROUPS_FILE);
        let file = GroupsFile {
            written_by: Some(RUNNING_VERSION.to_owned()),
            groups: groups.to_vec(),
        };
        self.save_toml_file(&path, &file)
    }

    /// Saves connection groups to the configuration file asynchronously
    ///
    /// Creates the configuration directory if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub async fn save_groups_async(&self, groups: &[ConnectionGroup]) -> ConfigResult<()> {
        self.ensure_config_dir()?;
        let path = self.config_dir.join(GROUPS_FILE);
        let file = GroupsFile {
            written_by: Some(RUNNING_VERSION.to_owned()),
            groups: groups.to_vec(),
        };
        self.save_toml_file_async(&path, &file).await
    }

    // ========== Snippets ==========

    /// Loads snippets from the configuration file
    ///
    /// Returns an empty vector if the file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load_snippets(&self) -> ConfigResult<Vec<Snippet>> {
        let path = self.config_dir.join(SNIPPETS_FILE);
        self.load_marked_toml_file::<SnippetsFile>(&path)
            .map(|f| f.snippets)
    }

    /// Saves snippets to the configuration file
    ///
    /// Creates the configuration directory if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save_snippets(&self, snippets: &[Snippet]) -> ConfigResult<()> {
        self.ensure_config_dir()?;
        let path = self.config_dir.join(SNIPPETS_FILE);
        let file = SnippetsFile {
            written_by: Some(RUNNING_VERSION.to_owned()),
            snippets: snippets.to_vec(),
        };
        self.save_toml_file(&path, &file)
    }

    // ========== Clusters ==========

    /// Loads clusters from the configuration file
    ///
    /// Returns an empty vector if the file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load_clusters(&self) -> ConfigResult<Vec<Cluster>> {
        let path = self.config_dir.join(CLUSTERS_FILE);
        self.load_marked_toml_file::<ClustersFile>(&path)
            .map(|f| f.clusters)
    }

    /// Saves clusters to the configuration file
    ///
    /// Creates the configuration directory if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save_clusters(&self, clusters: &[Cluster]) -> ConfigResult<()> {
        self.ensure_config_dir()?;
        let path = self.config_dir.join(CLUSTERS_FILE);
        let file = ClustersFile {
            written_by: Some(RUNNING_VERSION.to_owned()),
            clusters: clusters.to_vec(),
        };
        self.save_toml_file(&path, &file)
    }

    // ========== Templates ==========

    /// Loads templates from the configuration file
    ///
    /// Returns an empty vector if the file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load_templates(&self) -> ConfigResult<Vec<ConnectionTemplate>> {
        let path = self.config_dir.join(TEMPLATES_FILE);
        self.load_marked_toml_file::<TemplatesFile>(&path)
            .map(|f| f.templates)
    }

    /// Saves templates to the configuration file
    ///
    /// Creates the configuration directory if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save_templates(&self, templates: &[ConnectionTemplate]) -> ConfigResult<()> {
        self.ensure_config_dir()?;
        let path = self.config_dir.join(TEMPLATES_FILE);
        let file = TemplatesFile {
            written_by: Some(RUNNING_VERSION.to_owned()),
            templates: templates.to_vec(),
        };
        self.save_toml_file(&path, &file)
    }

    // ========== Workspace Profiles ==========

    /// Loads workspace profiles from the configuration file
    ///
    /// Returns an empty vector if the file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load_workspace_profiles(&self) -> ConfigResult<Vec<WorkspaceProfile>> {
        let path = self.config_dir.join(WORKSPACE_PROFILES_FILE);
        self.load_marked_toml_file::<WorkspaceProfilesFile>(&path)
            .map(|f| f.profiles)
    }

    /// Saves workspace profiles to the configuration file
    ///
    /// Creates the configuration directory if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save_workspace_profiles(&self, profiles: &[WorkspaceProfile]) -> ConfigResult<()> {
        self.ensure_config_dir()?;
        let path = self.config_dir.join(WORKSPACE_PROFILES_FILE);
        let file = WorkspaceProfilesFile {
            written_by: Some(RUNNING_VERSION.to_owned()),
            profiles: profiles.to_vec(),
        };
        self.save_toml_file(&path, &file)
    }

    // ========== Connection History ==========

    /// Loads connection history from the configuration file
    ///
    /// Returns an empty list if the file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load_history(&self) -> ConfigResult<Vec<ConnectionHistoryEntry>> {
        let path = self.config_dir.join(HISTORY_FILE);
        self.load_marked_toml_file::<HistoryFile>(&path)
            .map(|f| f.entries)
    }

    /// Saves connection history to the configuration file
    ///
    /// Creates the configuration directory if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save_history(&self, entries: &[ConnectionHistoryEntry]) -> ConfigResult<()> {
        self.ensure_config_dir()?;
        let path = self.config_dir.join(HISTORY_FILE);
        let file = HistoryFile {
            written_by: Some(RUNNING_VERSION.to_owned()),
            entries: entries.to_vec(),
        };
        self.save_toml_file(&path, &file)
    }

    // ========== Simple Sync Tombstones ==========

    /// Loads Simple Sync tombstones from the configuration file.
    ///
    /// Returns an empty list if the file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load_tombstones(&self) -> ConfigResult<Vec<Tombstone>> {
        let path = self.config_dir.join(TOMBSTONES_FILE);
        self.load_marked_toml_file::<TombstonesFile>(&path)
            .map(|f| f.tombstones)
    }

    /// Saves Simple Sync tombstones to the configuration file.
    ///
    /// Creates the configuration directory if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save_tombstones(&self, tombstones: &[Tombstone]) -> ConfigResult<()> {
        self.ensure_config_dir()?;
        let path = self.config_dir.join(TOMBSTONES_FILE);
        let file = TombstonesFile {
            written_by: Some(RUNNING_VERSION.to_owned()),
            tombstones: tombstones.to_vec(),
        };
        self.save_toml_file(&path, &file)
    }

    // ========== Trash ==========

    /// Loads trash (deleted items) from the configuration file
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    #[expect(
        clippy::type_complexity,
        reason = "internal helper signature documents the exact tuple layout used by the caller; aliasing would obscure the data flow"
    )]
    pub fn load_trash(
        &self,
    ) -> ConfigResult<(
        Vec<(Connection, chrono::DateTime<chrono::Utc>)>,
        Vec<(ConnectionGroup, chrono::DateTime<chrono::Utc>)>,
    )> {
        let path = self.config_dir.join(TRASH_FILE);
        let file = self.load_marked_toml_file::<TrashFile>(&path)?;
        Ok((file.connections, file.groups))
    }

    /// Saves trash items to the configuration file asynchronously
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub async fn save_trash_async(
        &self,
        connections: &[(Connection, chrono::DateTime<chrono::Utc>)],
        groups: &[(ConnectionGroup, chrono::DateTime<chrono::Utc>)],
    ) -> ConfigResult<()> {
        self.ensure_config_dir()?;
        let path = self.config_dir.join(TRASH_FILE);
        let file = TrashFile {
            written_by: Some(RUNNING_VERSION.to_owned()),
            connections: connections.to_vec(),
            groups: groups.to_vec(),
        };
        self.save_toml_file_async(&path, &file).await
    }

    // ========== Application Settings ==========

    /// Loads application settings from the configuration file
    ///
    /// Returns default settings if the file doesn't exist. A file a newer
    /// `RustConn` wrote is flagged, so the first save backs it up before
    /// replacing it.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load_settings(&self) -> ConfigResult<AppSettings> {
        let path = self.config_dir.join(CONFIG_FILE);
        if !path.exists() {
            return Ok(AppSettings::default());
        }
        let settings: AppSettings = Self::load_toml_file(&path)?;
        self.note_written_by(&path, settings.written_by.as_deref());
        Ok(settings)
    }

    /// Saves application settings to the configuration file
    ///
    /// Creates the configuration directory if it doesn't exist. The file records
    /// this version in [`AppSettings::written_by`]; `settings` itself is left
    /// untouched.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save_settings(&self, settings: &AppSettings) -> ConfigResult<()> {
        self.ensure_config_dir()?;
        let path = self.config_dir.join(CONFIG_FILE);
        // Stamped on a copy, never on `settings`: the caller's value keeps the
        // marker it was loaded with, so a save cannot make two in-memory settings
        // compare unequal and send a "settings changed" check round again.
        let stamped = AppSettings {
            written_by: Some(RUNNING_VERSION.to_owned()),
            ..settings.clone()
        };
        self.save_toml_file(&path, &stamped)
    }

    // ========== Global Variables ==========

    /// Loads global variables from the settings file
    ///
    /// Returns an empty vector if no variables are configured.
    ///
    /// # Errors
    ///
    /// Returns an error if the settings file cannot be read.
    pub fn load_variables(&self) -> ConfigResult<Vec<crate::variables::Variable>> {
        let settings = self.load_settings()?;
        Ok(settings.global_variables)
    }

    /// Saves global variables to the settings file
    ///
    /// # Errors
    ///
    /// Returns an error if the settings file cannot be written.
    pub fn save_variables(&self, variables: &[crate::variables::Variable]) -> ConfigResult<()> {
        let mut settings = self.load_settings()?;
        settings.global_variables = variables.to_vec();
        self.save_settings(&settings)
    }

    // ========== Generic TOML Operations ==========

    /// Loads and parses a TOML file
    ///
    /// Returns the default value if the file doesn't exist.
    fn load_toml_file<T>(path: &Path) -> ConfigResult<T>
    where
        T: serde::de::DeserializeOwned + Default,
    {
        if !path.exists() {
            return Ok(T::default());
        }

        let content = fs::read_to_string(path)
            .map_err(|e| ConfigError::Parse(format!("Failed to read {}: {}", path.display(), e)))?;

        Self::parse_toml(&content, path)
    }

    /// Loads a TOML file that records which `RustConn` wrote it, flagging a newer one.
    ///
    /// The marker comes from the parsed file. When the full parse fails, a probe
    /// that reads nothing but the marker runs on the same text, so a file this
    /// version cannot read is still recognised as a newer one's — and the caller
    /// still gets the full parse's error, which says what is wrong, not the
    /// probe's.
    fn load_marked_toml_file<T>(&self, path: &Path) -> ConfigResult<T>
    where
        T: serde::de::DeserializeOwned + Default + Marked,
    {
        if !path.exists() {
            return Ok(T::default());
        }

        let content = fs::read_to_string(path)
            .map_err(|e| ConfigError::Parse(format!("Failed to read {}: {}", path.display(), e)))?;

        match Self::parse_toml::<T>(&content, path) {
            Ok(file) => {
                self.note_written_by(path, file.marker());
                Ok(file)
            }
            Err(e) => {
                if let Ok(written_by) = version_skew::probe_written_by(content.as_bytes()) {
                    self.note_written_by(path, written_by.as_deref());
                }
                Err(e)
            }
        }
    }

    /// Parses TOML content with validation
    fn parse_toml<T>(content: &str, path: &Path) -> ConfigResult<T>
    where
        T: serde::de::DeserializeOwned,
    {
        toml::from_str(content).map_err(|e| {
            ConfigError::Deserialize(format!("Failed to parse {}: {}", path.display(), e))
        })
    }

    /// Saves data to a TOML file with atomic write (temp file + rename).
    ///
    /// Acquires an exclusive advisory lock before writing to prevent
    /// concurrent modifications from other processes (GUI + CLI).
    fn save_toml_file<T>(&self, path: &Path, data: &T) -> ConfigResult<()>
    where
        T: serde::Serialize,
    {
        let content = toml::to_string_pretty(data)
            .map_err(|e| ConfigError::Serialize(format!("Failed to serialize: {e}")))?;

        self.write_locked(path, &content)
    }

    /// Saves data to a TOML file from async context, without blocking the runtime.
    ///
    /// The write itself is [`Self::write_locked`] on a blocking-pool thread. It
    /// used to be a second, hand-maintained copy of that sequence built out of
    /// `tokio::fs`, which looked asynchronous but opened with a synchronous
    /// `flock(LOCK_EX)` — parking a runtime worker for as long as another writer's
    /// fsync took, and making the caller's `tokio::time::timeout` useless: a timer
    /// only fires when the future yields, and a future stuck in a syscall never
    /// does. `spawn_blocking` puts the blocking work where blocking work belongs
    /// and restores the yield point the timeout needs.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Serialize`] if `data` cannot be rendered as TOML,
    /// [`ConfigError::Write`] if the blocking task could not be joined, or
    /// whatever [`Self::write_locked`] reports.
    async fn save_toml_file_async<T>(&self, path: &Path, data: &T) -> ConfigResult<()>
    where
        T: serde::Serialize + Sync,
    {
        let content = toml::to_string_pretty(data)
            .map_err(|e| ConfigError::Serialize(format!("Failed to serialize: {e}")))?;

        // Owned copies: `spawn_blocking` needs 'static + Send.
        let path = path.to_path_buf();
        let manager = self.clone();
        tokio::task::spawn_blocking(move || manager.write_locked(&path, &content))
            .await
            .map_err(|e| ConfigError::Write(format!("Config write task failed: {e}")))?
    }

    // ========== Validation ==========

    /// Validates a connection configuration
    ///
    /// # Errors
    ///
    /// Returns an error if the connection is invalid.
    pub fn validate_connection(connection: &Connection) -> ConfigResult<()> {
        use crate::models::ProtocolConfig;

        if connection.name.trim().is_empty() {
            return Err(ConfigError::Validation {
                field: "name".to_string(),
                reason: "Connection name cannot be empty".to_string(),
            });
        }

        // Host and port are optional for Zero Trust connections
        // (the target is defined in the provider config), Serial connections
        // (the target is a local device path, not a network host), and Kubernetes
        // connections (the target is a pod/container, not a network host).
        let is_zerotrust = matches!(connection.protocol_config, ProtocolConfig::ZeroTrust(_));
        let is_serial = matches!(connection.protocol_config, ProtocolConfig::Serial(_));
        let is_kubernetes = matches!(connection.protocol_config, ProtocolConfig::Kubernetes(_));
        let skip_host_port = is_zerotrust || is_serial || is_kubernetes;

        if !skip_host_port && connection.host.trim().is_empty() {
            return Err(ConfigError::Validation {
                field: "host".to_string(),
                reason: "Host cannot be empty".to_string(),
            });
        }

        if !skip_host_port && connection.port == 0 {
            return Err(ConfigError::Validation {
                field: "port".to_string(),
                reason: "Port must be greater than 0".to_string(),
            });
        }

        Ok(())
    }

    /// Validates a connection group
    ///
    /// # Errors
    ///
    /// Returns an error if the group is invalid.
    pub fn validate_group(group: &ConnectionGroup) -> ConfigResult<()> {
        if group.name.trim().is_empty() {
            return Err(ConfigError::Validation {
                field: "name".to_string(),
                reason: "Group name cannot be empty".to_string(),
            });
        }

        Ok(())
    }

    /// Validates a snippet
    ///
    /// # Errors
    ///
    /// Returns an error if the snippet is invalid.
    pub fn validate_snippet(snippet: &Snippet) -> ConfigResult<()> {
        if snippet.name.trim().is_empty() {
            return Err(ConfigError::Validation {
                field: "name".to_string(),
                reason: "Snippet name cannot be empty".to_string(),
            });
        }

        if snippet.command.trim().is_empty() {
            return Err(ConfigError::Validation {
                field: "command".to_string(),
                reason: "Snippet command cannot be empty".to_string(),
            });
        }

        Ok(())
    }

    /// Validates a cluster
    ///
    /// # Errors
    ///
    /// Returns an error if the cluster is invalid.
    pub fn validate_cluster(cluster: &Cluster) -> ConfigResult<()> {
        if cluster.name.trim().is_empty() {
            return Err(ConfigError::Validation {
                field: "name".to_string(),
                reason: "Cluster name cannot be empty".to_string(),
            });
        }

        Ok(())
    }

    /// Validates all connections and returns errors for invalid ones
    #[must_use]
    pub fn validate_connections(connections: &[Connection]) -> Vec<(usize, ConfigError)> {
        connections
            .iter()
            .enumerate()
            .filter_map(|(i, conn)| Self::validate_connection(conn).err().map(|e| (i, e)))
            .collect()
    }

    /// Validates all groups and returns errors for invalid ones
    #[must_use]
    pub fn validate_groups(groups: &[ConnectionGroup]) -> Vec<(usize, ConfigError)> {
        groups
            .iter()
            .enumerate()
            .filter_map(|(i, group)| Self::validate_group(group).err().map(|e| (i, e)))
            .collect()
    }

    /// Validates all snippets and returns errors for invalid ones
    #[must_use]
    pub fn validate_snippets(snippets: &[Snippet]) -> Vec<(usize, ConfigError)> {
        snippets
            .iter()
            .enumerate()
            .filter_map(|(i, snippet)| Self::validate_snippet(snippet).err().map(|e| (i, e)))
            .collect()
    }

    /// Validates all clusters and returns errors for invalid ones
    #[must_use]
    pub fn validate_clusters(clusters: &[Cluster]) -> Vec<(usize, ConfigError)> {
        clusters
            .iter()
            .enumerate()
            .filter_map(|(i, cluster)| Self::validate_cluster(cluster).err().map(|e| (i, e)))
            .collect()
    }

    /// Validates a template
    ///
    /// # Errors
    ///
    /// Returns an error if the template is invalid.
    pub fn validate_template(template: &ConnectionTemplate) -> ConfigResult<()> {
        if template.name.trim().is_empty() {
            return Err(ConfigError::Validation {
                field: "name".to_string(),
                reason: "Template name cannot be empty".to_string(),
            });
        }

        Ok(())
    }

    /// Validates all templates and returns errors for invalid ones
    #[must_use]
    pub fn validate_templates(templates: &[ConnectionTemplate]) -> Vec<(usize, ConfigError)> {
        templates
            .iter()
            .enumerate()
            .filter_map(|(i, template)| Self::validate_template(template).err().map(|e| (i, e)))
            .collect()
    }

    // ========== Backup / Restore ==========

    /// Files included in a settings backup archive.
    const BACKUP_FILES: &[&str] = &[
        CONNECTIONS_FILE,
        GROUPS_FILE,
        SNIPPETS_FILE,
        CLUSTERS_FILE,
        TEMPLATES_FILE,
        HISTORY_FILE,
        CONFIG_FILE,
    ];

    /// Creates a ZIP backup of all configuration files.
    ///
    /// Only files that exist on disk are included. The archive can be
    /// restored with [`Self::restore_from_archive`].
    ///
    /// # Errors
    ///
    /// Returns an error if the archive cannot be created or written.
    pub fn backup_to_archive(&self, dest: &Path) -> ConfigResult<u32> {
        let file = fs::File::create(dest).map_err(|e| {
            ConfigError::Write(format!(
                "Failed to create backup file {}: {e}",
                dest.display()
            ))
        })?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        let mut count = 0u32;
        for name in Self::BACKUP_FILES {
            let path = self.config_dir.join(name);
            if path.exists() {
                let content = fs::read(&path).map_err(|e| {
                    ConfigError::Parse(format!("Failed to read {}: {e}", path.display()))
                })?;
                zip.start_file(*name, options).map_err(|e| {
                    ConfigError::Write(format!("Failed to add {name} to archive: {e}"))
                })?;
                std::io::Write::write_all(&mut zip, &content).map_err(|e| {
                    ConfigError::Write(format!("Failed to write {name} to archive: {e}"))
                })?;
                count += 1;
            }
        }

        zip.finish()
            .map_err(|e| ConfigError::Write(format!("Failed to finalize backup archive: {e}")))?;

        tracing::info!(path = %dest.display(), files = count, "Settings backup created");
        Ok(count)
    }

    /// Restores configuration files from a ZIP backup archive.
    ///
    /// Only known configuration file names are extracted; unknown entries
    /// are silently skipped. Existing files are overwritten.
    ///
    /// # Errors
    ///
    /// Returns an error if the archive cannot be read or files cannot be written.
    pub fn restore_from_archive(&self, src: &Path) -> ConfigResult<u32> {
        self.ensure_config_dir()?;

        let file = fs::File::open(src).map_err(|e| {
            ConfigError::Parse(format!("Failed to open backup file {}: {e}", src.display()))
        })?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| {
            ConfigError::Deserialize(format!("Invalid backup archive {}: {e}", src.display()))
        })?;

        let allowed: std::collections::HashSet<&str> = Self::BACKUP_FILES.iter().copied().collect();

        let mut count = 0u32;
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).map_err(|e| {
                ConfigError::Parse(format!("Failed to read archive entry {i}: {e}"))
            })?;
            let Some(name) = entry.enclosed_name() else {
                continue;
            };
            let name_str = name.to_string_lossy();
            if !allowed.contains(name_str.as_ref()) {
                continue;
            }
            let dest_path = self.config_dir.join(&*name_str);
            let mut content = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut content).map_err(|e| {
                ConfigError::Parse(format!("Failed to read {name_str} from archive: {e}"))
            })?;
            // The backup files are all RustConn's own TOML config files (text),
            // so decode to UTF-8 and route through the atomic write path
            // (temp file + owner-only perms + fsync + rename) instead of a bare
            // fs::write. A raw write left a half-written config on disk if the
            // process died mid-restore, and skipped the 0600 permissioning that
            // every other config write gets.
            let text = String::from_utf8(content).map_err(|e| {
                ConfigError::Deserialize(format!("Backup entry {name_str} is not valid UTF-8: {e}"))
            })?;
            self.write_locked(&dest_path, &text)?;
            count += 1;
        }

        tracing::info!(path = %src.display(), files = count, "Settings restored from backup");
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::models::{ProtocolConfig, SshConfig};

    fn create_test_manager() -> (ConfigManager, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());
        (manager, temp_dir)
    }

    #[test]
    fn test_ensure_config_dir() {
        let (manager, _temp) = create_test_manager();
        assert!(manager.ensure_config_dir().is_ok());
        assert!(manager.config_dir().exists());
    }

    #[test]
    fn test_load_empty_connections() {
        let (manager, _temp) = create_test_manager();
        let connections = manager.load_connections().unwrap();
        assert!(connections.is_empty());
    }

    #[test]
    fn test_save_and_load_connections() {
        let (manager, _temp) = create_test_manager();

        let conn = Connection::new(
            "Test Server".to_string(),
            "example.com".to_string(),
            22,
            ProtocolConfig::Ssh(SshConfig::default()),
        );

        manager
            .save_connections(std::slice::from_ref(&conn))
            .unwrap();
        let loaded = manager.load_connections().unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, conn.name);
        assert_eq!(loaded[0].host, conn.host);
        assert_eq!(loaded[0].port, conn.port);
    }

    #[tokio::test]
    async fn test_save_connections_async() {
        let (manager, _temp) = create_test_manager();

        let conn = Connection::new(
            "Test Async".to_string(),
            "async.example.com".to_string(),
            22,
            ProtocolConfig::Ssh(SshConfig::default()),
        );

        manager
            .save_connections_async(std::slice::from_ref(&conn))
            .await
            .unwrap();
        let loaded = manager.load_connections().unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "Test Async");
    }

    #[test]
    fn test_save_and_load_groups() {
        let (manager, _temp) = create_test_manager();

        let group = ConnectionGroup::new("Production".to_string());

        manager.save_groups(std::slice::from_ref(&group)).unwrap();
        let loaded = manager.load_groups().unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, group.name);
    }

    #[test]
    fn test_save_and_load_snippets() {
        let (manager, _temp) = create_test_manager();

        let snippet = Snippet::new("List files".to_string(), "ls -la".to_string());

        manager
            .save_snippets(std::slice::from_ref(&snippet))
            .unwrap();
        let loaded = manager.load_snippets().unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, snippet.name);
        assert_eq!(loaded[0].command, snippet.command);
    }

    #[test]
    fn test_save_and_load_settings() {
        let (manager, _temp) = create_test_manager();

        let mut settings = AppSettings::default();
        settings.terminal.font_size = 14;
        settings.logging.enabled = true;

        manager.save_settings(&settings).unwrap();
        let loaded = manager.load_settings().unwrap();

        assert_eq!(loaded.terminal.font_size, 14);
        assert!(loaded.logging.enabled);
    }

    #[test]
    fn test_validate_connection_empty_name() {
        let conn = Connection::new(
            String::new(),
            "example.com".to_string(),
            22,
            ProtocolConfig::Ssh(SshConfig::default()),
        );

        let result = ConfigManager::validate_connection(&conn);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_connection_empty_host() {
        let conn = Connection::new(
            "Test".to_string(),
            String::new(),
            22,
            ProtocolConfig::Ssh(SshConfig::default()),
        );

        let result = ConfigManager::validate_connection(&conn);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_group_empty_name() {
        let mut group = ConnectionGroup::new("Test".to_string());
        group.name = String::new();

        let result = ConfigManager::validate_group(&group);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_snippet_empty_command() {
        let mut snippet = Snippet::new("Test".to_string(), "ls".to_string());
        snippet.command = String::new();

        let result = ConfigManager::validate_snippet(&snippet);
        assert!(result.is_err());
    }

    #[test]
    fn test_save_and_load_clusters() {
        use uuid::Uuid;

        use crate::cluster::Cluster;

        let (manager, _temp) = create_test_manager();

        let mut cluster = Cluster::new("Production Servers".to_string());
        cluster.add_connection(Uuid::new_v4());
        cluster.add_connection(Uuid::new_v4());
        cluster.broadcast_enabled = true;

        manager
            .save_clusters(std::slice::from_ref(&cluster))
            .unwrap();
        let loaded = manager.load_clusters().unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, cluster.name);
        assert_eq!(loaded[0].id, cluster.id);
        assert_eq!(loaded[0].connection_ids.len(), 2);
        assert!(loaded[0].broadcast_enabled);
    }

    #[test]
    fn test_save_and_load_tombstones() {
        use uuid::Uuid;

        use crate::sync::tombstone::{SyncEntityType, Tombstone};

        let (manager, _temp) = create_test_manager();

        // Empty when no file exists.
        assert!(manager.load_tombstones().unwrap().is_empty());

        let conn_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let tombstones = vec![
            Tombstone::new(SyncEntityType::Connection, conn_id),
            Tombstone::new(SyncEntityType::Group, group_id),
        ];

        manager.save_tombstones(&tombstones).unwrap();
        let loaded = manager.load_tombstones().unwrap();

        assert_eq!(loaded.len(), 2);
        assert!(
            loaded
                .iter()
                .any(|t| t.entity_type == SyncEntityType::Connection && t.id == conn_id)
        );
        assert!(
            loaded
                .iter()
                .any(|t| t.entity_type == SyncEntityType::Group && t.id == group_id)
        );
    }

    #[test]
    fn test_validate_cluster_empty_name() {
        use crate::cluster::Cluster;

        let mut cluster = Cluster::new("Test".to_string());
        cluster.name = String::new();

        let result = ConfigManager::validate_cluster(&cluster);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_cluster_whitespace_name() {
        use crate::cluster::Cluster;

        let mut cluster = Cluster::new("Test".to_string());
        cluster.name = "   ".to_string();

        let result = ConfigManager::validate_cluster(&cluster);
        assert!(result.is_err());
    }

    #[test]
    fn test_acquire_lock_exclusive() {
        let (manager, _temp) = create_test_manager();
        manager.ensure_config_dir().unwrap();

        // First lock should succeed
        let lock1 = manager.acquire_lock();
        assert!(lock1.is_ok());

        // Second lock from same process should block or fail with try_lock
        let lock_path = manager.config_dir().join(".lock");
        let lock_file2 = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .unwrap();
        // try_lock_exclusive should fail because lock1 is held
        assert!(fs2::FileExt::try_lock_exclusive(&lock_file2).is_err());

        // Drop lock1 — now lock2 should succeed
        drop(lock1);
        assert!(fs2::FileExt::try_lock_exclusive(&lock_file2).is_ok());
    }

    #[test]
    fn test_concurrent_save_with_lock() {
        use std::sync::Arc;
        use std::thread;

        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().to_path_buf();

        let manager1 = ConfigManager::with_config_dir(config_dir.clone());
        let manager2 = ConfigManager::with_config_dir(config_dir);

        manager1.ensure_config_dir().unwrap();

        let m1 = Arc::new(manager1);
        let m2 = Arc::new(manager2);

        let m1_clone = Arc::clone(&m1);
        let m2_clone = Arc::clone(&m2);

        // Two threads saving connections concurrently — no lost updates
        let handle1 = thread::spawn(move || {
            let conn = Connection::new(
                "Server A".to_string(),
                "a.example.com".to_string(),
                22,
                ProtocolConfig::Ssh(SshConfig::default()),
            );
            m1_clone
                .save_connections(std::slice::from_ref(&conn))
                .unwrap();
        });

        let handle2 = thread::spawn(move || {
            let conn = Connection::new(
                "Server B".to_string(),
                "b.example.com".to_string(),
                22,
                ProtocolConfig::Ssh(SshConfig::default()),
            );
            m2_clone
                .save_connections(std::slice::from_ref(&conn))
                .unwrap();
        });

        handle1.join().unwrap();
        handle2.join().unwrap();

        // One of the two writes wins — file is valid TOML with exactly 1 connection
        let loaded = m1.load_connections().unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(loaded[0].name == "Server A" || loaded[0].name == "Server B");
    }

    #[test]
    fn test_backup_restore_round_trip() {
        let (manager, temp) = create_test_manager();

        let conn = Connection::new(
            "Backup Me".to_string(),
            "backup.example.com".to_string(),
            2222,
            ProtocolConfig::Ssh(SshConfig::default()),
        );
        manager
            .save_connections(std::slice::from_ref(&conn))
            .unwrap();

        // Back up, then delete the on-disk connections file.
        let archive = temp.path().join("backup.zip");
        let backed_up = manager.backup_to_archive(&archive).unwrap();
        assert!(backed_up >= 1, "at least the connections file is backed up");

        let conn_file = manager.config_dir().join(CONNECTIONS_FILE);
        std::fs::remove_file(&conn_file).unwrap();
        assert!(manager.load_connections().unwrap().is_empty());

        // Restore goes through the atomic write path; content must come back
        // intact and no leftover .tmp file must remain.
        let restored = manager.restore_from_archive(&archive).unwrap();
        assert!(restored >= 1);
        let loaded = manager.load_connections().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "Backup Me");
        assert!(
            !conn_file.with_extension("tmp").exists(),
            "atomic restore must not leave a .tmp file behind"
        );
    }

    // ========== Version skew ==========

    /// Names of the `.bak` files in `dir`, sorted.
    fn backups_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| Path::new(name).extension().is_some_and(|ext| ext == "bak"))
            .collect();
        names.sort();
        names
    }

    /// A file from before the marker existed is not "newer", and saving it backs
    /// nothing up.
    #[test]
    fn an_unmarked_file_loads_unflagged_and_saves_without_a_backup() {
        let (manager, _temp) = create_test_manager();
        let path = manager.config_dir().join(CONFIG_FILE);
        fs::write(&path, "[terminal]\nfont_size = 13\n").unwrap();

        let settings = manager.load_settings().unwrap();
        assert_eq!(settings.written_by, None);
        assert_eq!(settings.terminal.font_size, 13);
        assert!(manager.newer_version_files().is_empty());

        manager.save_settings(&settings).unwrap();
        assert!(backups_in(manager.config_dir()).is_empty());
    }

    /// A newer version's file is copied byte for byte before the first save, and
    /// only before the first.
    #[test]
    fn a_newer_file_is_backed_up_once_before_the_first_save() {
        let (manager, _temp) = create_test_manager();
        let path = manager.config_dir().join(CONFIG_FILE);
        let original = "written_by = \"99.0.0\"\n\n[terminal]\nfont_size = 13\n";
        fs::write(&path, original).unwrap();

        let settings = manager.load_settings().unwrap();
        let flagged = vec![(path, "99.0.0".to_string())];
        assert_eq!(manager.newer_version_files(), flagged);

        // Through a clone, the way ConnectionManager's debounce workers save.
        let worker = manager.clone();
        worker.save_settings(&settings).unwrap();
        let backup = manager.config_dir().join("config.toml.99.0.0.bak");
        assert_eq!(fs::read(&backup).unwrap(), original.as_bytes());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&backup).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(manager.newer_version_files().is_empty());

        manager.save_settings(&settings).unwrap();
        let backups = backups_in(manager.config_dir());
        assert_eq!(backups, ["config.toml.99.0.0.bak"]);
        assert_eq!(fs::read(&backup).unwrap(), original.as_bytes());
    }

    /// A save stamps the running version on the file, not on the caller's value,
    /// and a marker from this version or an older one is not flagged.
    #[test]
    fn saves_stamp_the_running_version_and_older_markers_are_not_flagged() {
        let (manager, _temp) = create_test_manager();
        let path = manager.config_dir().join(CONFIG_FILE);
        let settings = AppSettings::default();

        manager.save_settings(&settings).unwrap();
        assert_eq!(settings.written_by, None);
        let reloaded = manager.load_settings().unwrap();
        assert_eq!(reloaded.written_by.as_deref(), Some(RUNNING_VERSION));

        for marker in ["0.0.1", RUNNING_VERSION] {
            fs::write(&path, format!("written_by = \"{marker}\"\n")).unwrap();
            let loaded = manager.load_settings().unwrap();
            assert_eq!(loaded.written_by.as_deref(), Some(marker));
            assert!(manager.newer_version_files().is_empty(), "{marker}");
        }
        manager.save_settings(&settings).unwrap();
        assert!(backups_in(manager.config_dir()).is_empty());
    }

    /// The marker line is the only change to what a save writes.
    #[test]
    fn a_save_writes_what_it_did_before_plus_the_marker_line() {
        let (manager, _temp) = create_test_manager();
        let mut settings = AppSettings::default();
        settings.terminal.font_size = 15;
        settings.logging.enabled = true;
        // `written_by` is `None` here, so it is skipped: the file as written
        // before the marker existed.
        let unmarked = toml::to_string_pretty(&settings).unwrap();

        manager.save_settings(&settings).unwrap();

        let saved = fs::read_to_string(manager.config_dir().join(CONFIG_FILE)).unwrap();
        let marker_line = format!("written_by = \"{RUNNING_VERSION}\"\n");
        assert_eq!(saved.matches(&marker_line).count(), 1, "{saved}");
        assert_eq!(saved.replacen(&marker_line, "", 1), unmarked);
    }

    /// A value this version does not know fails the load, and the file can be
    /// kept aside, byte for byte and owner-only, before defaults replace it.
    #[test]
    fn an_unknown_enum_value_fails_the_load_and_the_file_can_be_kept_aside() {
        let (manager, _temp) = create_test_manager();
        let path = manager.config_dir().join(CONFIG_FILE);
        let content = "[secrets]\npreferred_backend = \"future_backend\"\n";
        fs::write(&path, content).unwrap();

        let error = manager.load_settings().unwrap_err();
        assert!(matches!(error, ConfigError::Deserialize(_)), "{error}");

        let name = ConfigManager::SETTINGS_FILE_NAME;
        let kept = manager.quarantine_unreadable(name).unwrap();
        assert_eq!(kept.parent(), Some(manager.config_dir()));
        let kept_name = kept.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            kept_name.starts_with("config.toml.unreadable-"),
            "{kept_name}"
        );
        assert_eq!(fs::read(&kept).unwrap(), content.as_bytes());
        assert_eq!(fs::read(&path).unwrap(), content.as_bytes());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&kept).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        // The same bytes on the next start reuse the copy instead of adding one.
        let again = manager.quarantine_unreadable(name).unwrap();
        assert_eq!(again, kept);
    }

    /// `quarantine_unreadable` only ever copies a file in the config directory.
    #[test]
    fn quarantine_takes_only_a_plain_file_name() {
        let (manager, _temp) = create_test_manager();
        for name in ["../config.toml", "/etc/passwd", "sub/config.toml", ""] {
            let result = manager.quarantine_unreadable(name);
            assert!(
                matches!(result, Err(ConfigError::Validation { .. })),
                "{name:?}"
            );
        }
    }

    /// `connections.toml` is flagged by its marker even when this version cannot
    /// parse it, and the probe never hides the full parse's error.
    #[test]
    fn a_newer_connections_file_is_flagged_and_an_unknown_protocol_still_fails() {
        let (manager, _temp) = create_test_manager();
        let path = manager.config_dir().join(CONNECTIONS_FILE);
        let conn = Connection::new(
            "Probe".to_string(),
            "probe.example.com".to_string(),
            22,
            ProtocolConfig::Ssh(SshConfig::default()),
        );
        manager
            .save_connections(std::slice::from_ref(&conn))
            .unwrap();
        let ours = fs::read_to_string(&path).unwrap();
        let our_marker = format!("written_by = \"{RUNNING_VERSION}\"");
        assert!(ours.contains(&our_marker), "{ours}");

        // Readable, from a newer version: it loads, and it is flagged.
        let newer = ours.replacen(&our_marker, "written_by = \"99.0.0\"", 1);
        fs::write(&path, &newer).unwrap();
        assert_eq!(manager.load_connections().unwrap().len(), 1);
        let flagged = vec![(path.clone(), "99.0.0".to_string())];
        assert_eq!(manager.newer_version_files(), flagged);

        // Not readable by this version: still flagged, and the load fails with
        // the full parse's error.
        let unknown = newer.replacen("type = \"Ssh\"", "type = \"Teleport\"", 1);
        assert_ne!(unknown, newer, "the fixture must carry the protocol tag");
        fs::write(&path, &unknown).unwrap();
        let fresh = ConfigManager::with_config_dir(manager.config_dir().to_path_buf());
        let error = fresh.load_connections().unwrap_err();
        assert!(matches!(error, ConfigError::Deserialize(_)), "{error}");
        assert!(error.to_string().contains("Teleport"), "{error}");
        assert_eq!(fresh.newer_version_files(), flagged);
    }

    /// A flagged file that another process has since rewritten from this version
    /// holds nothing newer, so it is not copied under the newer version's name.
    #[test]
    fn a_file_rewritten_since_by_this_version_is_not_backed_up() {
        let (manager, _temp) = create_test_manager();
        let path = manager.config_dir().join(CONFIG_FILE);
        fs::write(&path, "written_by = \"99.0.0\"\n").unwrap();
        let settings = manager.load_settings().unwrap();

        fs::write(&path, format!("written_by = \"{RUNNING_VERSION}\"\n")).unwrap();
        manager.save_settings(&settings).unwrap();

        assert!(backups_in(manager.config_dir()).is_empty());
        assert!(manager.newer_version_files().is_empty());
    }

    /// A backup that cannot be written stops the save: the newer file stays as
    /// it was, and stays flagged for the next attempt.
    #[test]
    fn a_failed_backup_aborts_the_write() {
        let (manager, _temp) = create_test_manager();
        let path = manager.config_dir().join(CONFIG_FILE);
        let original = "written_by = \"99.0.0\"\n";
        fs::write(&path, original).unwrap();
        let settings = manager.load_settings().unwrap();
        // A directory where the backup has to go makes the copy fail.
        fs::create_dir(manager.config_dir().join("config.toml.99.0.0.bak")).unwrap();

        let error = manager.save_settings(&settings).unwrap_err();

        assert!(matches!(error, ConfigError::Write(_)), "{error}");
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        assert_eq!(manager.newer_version_files().len(), 1);
    }

    /// A save with no load before it — `rustconn-cli history clear` writes an
    /// empty history straight away — still backs up a newer version's file,
    /// once: the marker on disk is read before the first overwrite.
    #[test]
    fn a_save_without_a_load_still_backs_up_a_newer_file() {
        let (manager, _temp) = create_test_manager();
        let path = manager.config_dir().join(HISTORY_FILE);
        let original = "written_by = \"99.0.0\"\n\n[[entries]]\nfuture_field = 1\n";
        fs::write(&path, original).unwrap();

        manager.save_history(&[]).unwrap();

        let backup = manager.config_dir().join("history.toml.99.0.0.bak");
        assert_eq!(fs::read_to_string(&backup).unwrap(), original);
        assert!(manager.newer_version_files().is_empty());
        let ours = fs::read_to_string(&path).unwrap();
        assert!(
            ours.contains(&format!("written_by = \"{RUNNING_VERSION}\"")),
            "{ours}"
        );

        // The next save overwrites this version's own file: no second copy, and
        // the first one still holds what the newer version wrote.
        manager.save_history(&[]).unwrap();
        assert_eq!(
            backups_in(manager.config_dir()),
            ["history.toml.99.0.0.bak"]
        );
        assert_eq!(fs::read_to_string(&backup).unwrap(), original);
    }

    /// The probe flags only a newer marker: a file this or an older version
    /// wrote, or one with no marker, is replaced without a copy.
    #[test]
    fn a_save_without_a_load_copies_nothing_that_is_not_newer() {
        for existing in [
            format!("written_by = \"{RUNNING_VERSION}\"\n"),
            "written_by = \"0.0.1\"\n".to_string(),
            String::new(),
            "not toml [".to_string(),
        ] {
            let (manager, _temp) = create_test_manager();
            fs::write(manager.config_dir().join(HISTORY_FILE), &existing).unwrap();

            manager.save_history(&[]).unwrap();

            assert!(backups_in(manager.config_dir()).is_empty(), "{existing:?}");
            assert!(manager.newer_version_files().is_empty(), "{existing:?}");
        }
    }

    /// Restoring writes the archived bytes as they are; the marker is not
    /// stamped again.
    #[test]
    fn restore_writes_the_archived_bytes_as_they_are() {
        let (manager, temp) = create_test_manager();
        let path = manager.config_dir().join(CONFIG_FILE);
        let archived = "written_by = \"0.0.1\"\n\n[terminal]\nfont_size = 13\n";
        fs::write(&path, archived).unwrap();
        let archive = temp.path().join("backup.zip");
        manager.backup_to_archive(&archive).unwrap();
        fs::remove_file(&path).unwrap();

        manager.restore_from_archive(&archive).unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), archived);
    }

    /// The forward-compat marker now covers the secondary collection files, not
    /// just connections/groups/settings. Clusters stands in for the group:
    /// SnippetsFile, ClustersFile, TemplatesFile and WorkspaceProfilesFile all
    /// gained the same `written_by` field, stamp and marked loader.
    #[test]
    fn clusters_carry_the_version_marker_and_a_newer_one_is_flagged() {
        use crate::cluster::Cluster;

        let (manager, _temp) = create_test_manager();
        let path = manager.config_dir().join(CLUSTERS_FILE);

        // A save stamps the running version.
        manager
            .save_clusters(&[Cluster::new("DC Fleet".to_string())])
            .unwrap();
        let on_disk = fs::read_to_string(&path).unwrap();
        assert!(
            on_disk.contains(&format!("written_by = \"{RUNNING_VERSION}\"")),
            "save must stamp the running version: {on_disk}"
        );

        // An older/same marker is not flagged, and the file still loads.
        for marker in ["0.0.1", RUNNING_VERSION] {
            fs::write(&path, format!("written_by = \"{marker}\"\nclusters = []\n")).unwrap();
            assert!(manager.load_clusters().unwrap().is_empty());
            assert!(
                manager.newer_version_files().is_empty(),
                "marker {marker} must not be flagged as newer"
            );
        }

        // A newer marker is flagged on load and backed up before the next save
        // overwrites it — the whole point of the forward-compat coverage.
        let newer = "written_by = \"99.0.0\"\nclusters = []\n";
        fs::write(&path, newer).unwrap();
        let _ = manager.load_clusters().unwrap();
        assert_eq!(
            manager.newer_version_files(),
            vec![(path.clone(), "99.0.0".to_string())]
        );
        manager.save_clusters(&[]).unwrap();
        let backup = manager.config_dir().join("clusters.toml.99.0.0.bak");
        assert_eq!(
            fs::read_to_string(&backup).unwrap(),
            newer,
            "the newer file must be backed up before being overwritten"
        );

        // A legacy file with no marker at all still loads (serde-default).
        fs::write(&path, "clusters = []\n").unwrap();
        assert!(manager.load_clusters().unwrap().is_empty());
    }

    /// The bookkeeping files (history, tombstones, trash) gained the marker too.
    /// Tombstones stands in for the group.
    #[test]
    fn tombstones_carry_the_version_marker_and_a_newer_one_is_flagged() {
        let (manager, _temp) = create_test_manager();
        let path = manager.config_dir().join(TOMBSTONES_FILE);

        // A save stamps the running version.
        manager.save_tombstones(&[]).unwrap();
        let on_disk = fs::read_to_string(&path).unwrap();
        assert!(
            on_disk.contains(&format!("written_by = \"{RUNNING_VERSION}\"")),
            "save must stamp the running version: {on_disk}"
        );

        // A newer marker is flagged on load and backed up before the next save.
        let newer = "written_by = \"99.0.0\"\ntombstones = []\n";
        fs::write(&path, newer).unwrap();
        let _ = manager.load_tombstones().unwrap();
        assert_eq!(
            manager.newer_version_files(),
            vec![(path.clone(), "99.0.0".to_string())]
        );
        manager.save_tombstones(&[]).unwrap();
        let backup = manager.config_dir().join("tombstones.toml.99.0.0.bak");
        assert_eq!(
            fs::read_to_string(&backup).unwrap(),
            newer,
            "the newer file must be backed up before being overwritten"
        );

        // A legacy file with no marker at all still loads.
        fs::write(&path, "tombstones = []\n").unwrap();
        assert!(manager.load_tombstones().unwrap().is_empty());
    }
}
