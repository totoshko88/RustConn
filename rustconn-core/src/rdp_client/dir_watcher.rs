//! Directory change notification watcher for RDPDR
//!
//! This module provides inotify-based directory monitoring for RDP shared folders.
//! When a directory changes, it sends notifications back to the RDP server.

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use tracing::{debug, trace, warn};

/// File action constants matching MS-FSCC `FILE_ACTION_*`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum FileAction {
    /// File was added
    Added = 0x0000_0001,
    /// File was removed
    Removed = 0x0000_0002,
    /// File was modified
    Modified = 0x0000_0003,
    /// File was renamed (old name)
    RenamedOldName = 0x0000_0004,
    /// File was renamed (new name)
    RenamedNewName = 0x0000_0005,
}

/// Completion filter flags matching MS-SMB2 `FILE_NOTIFY_CHANGE_*`
#[derive(Debug, Clone, Copy)]
pub struct CompletionFilter(pub u32);

impl CompletionFilter {
    /// File name changes (create, delete, rename)
    pub const FILE_NAME: u32 = 0x0000_0001;
    /// Directory name changes
    pub const DIR_NAME: u32 = 0x0000_0002;
    /// Attribute changes
    pub const ATTRIBUTES: u32 = 0x0000_0004;
    /// Size changes
    pub const SIZE: u32 = 0x0000_0008;
    /// Last write time changes
    pub const LAST_WRITE: u32 = 0x0000_0010;
    /// Last access time changes
    pub const LAST_ACCESS: u32 = 0x0000_0020;
    /// Creation time changes
    pub const CREATION: u32 = 0x0000_0040;
    /// Security descriptor changes
    pub const SECURITY: u32 = 0x0000_0100;

    /// Check if filter matches the given event kind
    #[must_use]
    pub fn matches(&self, kind: &EventKind) -> bool {
        match kind {
            EventKind::Create(_) => {
                self.0 & (Self::FILE_NAME | Self::DIR_NAME | Self::CREATION) != 0
            }
            EventKind::Remove(_) => self.0 & (Self::FILE_NAME | Self::DIR_NAME) != 0,
            EventKind::Modify(modify_kind) => {
                use notify::event::ModifyKind;
                match modify_kind {
                    ModifyKind::Data(_) => self.0 & (Self::SIZE | Self::LAST_WRITE) != 0,
                    ModifyKind::Metadata(_) => {
                        self.0 & (Self::ATTRIBUTES | Self::LAST_ACCESS | Self::SECURITY) != 0
                    }
                    ModifyKind::Name(_) => self.0 & (Self::FILE_NAME | Self::DIR_NAME) != 0,
                    _ => self.0 & Self::LAST_WRITE != 0,
                }
            }
            EventKind::Access(_) => self.0 & Self::LAST_ACCESS != 0,
            _ => false,
        }
    }
}

/// A directory change notification
#[derive(Debug, Clone)]
pub struct DirectoryChange {
    /// File ID that requested the watch
    pub file_id: u32,
    /// Action that occurred
    pub action: FileAction,
    /// Relative path of the changed file (from watched directory)
    pub file_name: String,
}

/// Watch request from RDPDR
#[derive(Debug, Clone)]
pub struct WatchRequest {
    /// File ID from the RDP request
    pub file_id: u32,
    /// Path to watch
    pub path: PathBuf,
    /// Watch subdirectories recursively
    pub watch_tree: bool,
    /// Completion filter flags
    pub completion_filter: u32,
}

/// Directory watcher for RDPDR notifications
pub struct DirectoryWatcher {
    /// The underlying notify watcher
    watcher: RecommendedWatcher,
    /// Map of watched paths to their watch requests
    watches: Arc<Mutex<HashMap<PathBuf, Vec<WatchRequest>>>>,
    /// Receiver for directory change events
    event_rx: Receiver<DirectoryChange>,
    /// Sender for internal notify events
    #[allow(dead_code)]
    notify_tx: Sender<Result<Event, notify::Error>>,
}

impl std::fmt::Debug for DirectoryWatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectoryWatcher")
            .field(
                "watches_count",
                &self.watches.lock().map(|w| w.len()).unwrap_or(0),
            )
            .finish_non_exhaustive()
    }
}

impl DirectoryWatcher {
    /// Creates a new directory watcher
    ///
    /// # Errors
    ///
    /// Returns error if the watcher cannot be created
    #[allow(clippy::similar_names)]
    pub fn new() -> Result<Self, DirectoryWatcherError> {
        let (notify_tx, notify_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let watches: Arc<Mutex<HashMap<PathBuf, Vec<WatchRequest>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let watches_clone = watches.clone();

        // Create the watcher with a callback that processes events
        let fs_watcher = RecommendedWatcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Err(e) = notify_tx.send(res) {
                    warn!("Failed to send notify event: {}", e);
                }
            },
            Config::default(),
        )
        .map_err(|e| DirectoryWatcherError::InitFailed(e.to_string()))?;

        // Spawn a thread to process notify events and convert them to DirectoryChange
        std::thread::spawn(move || {
            process_notify_events(notify_rx, &event_tx, &watches_clone);
        });

        Ok(Self {
            watcher: fs_watcher,
            watches,
            event_rx,
            notify_tx: mpsc::channel().0, // Placeholder, actual sender is in closure
        })
    }

    /// Adds a watch for a directory
    ///
    /// # Errors
    ///
    /// Returns error if the watch cannot be added
    #[allow(clippy::significant_drop_tightening)]
    pub fn add_watch(&mut self, request: WatchRequest) -> Result<(), DirectoryWatcherError> {
        let path = request.path.clone();
        let mode = if request.watch_tree {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };

        debug!(
            "Adding directory watch: path={:?}, file_id={}, recursive={}, filter={:#x}",
            path, request.file_id, request.watch_tree, request.completion_filter
        );

        // Add to notify watcher
        self.watcher
            .watch(&path, mode)
            .map_err(|e| DirectoryWatcherError::WatchFailed(path.clone(), e.to_string()))?;

        // Store the watch request
        let mut watches = self.watches.lock().map_err(|_| {
            DirectoryWatcherError::WatchFailed(path.clone(), "Lock poisoned".to_string())
        })?;

        watches.entry(path).or_default().push(request);

        Ok(())
    }

    /// Removes a watch for a file ID
    #[allow(clippy::significant_drop_tightening)]
    pub fn remove_watch(&mut self, file_id: u32) {
        let Ok(mut watches) = self.watches.lock() else {
            return;
        };

        // Find and remove watches for this file_id
        let mut paths_to_check = Vec::new();
        for (path, requests) in watches.iter_mut() {
            requests.retain(|r| r.file_id != file_id);
            if requests.is_empty() {
                paths_to_check.push(path.clone());
            }
        }

        // Remove paths with no more watches
        for path in paths_to_check {
            watches.remove(&path);
            if let Err(e) = self.watcher.unwatch(&path) {
                trace!("Failed to unwatch {:?}: {}", path, e);
            }
        }
    }

    /// Tries to receive the next directory change event (non-blocking)
    #[must_use]
    pub fn try_recv(&self) -> Option<DirectoryChange> {
        self.event_rx.try_recv().ok()
    }

    /// Receives all pending directory change events
    #[must_use]
    pub fn recv_all(&self) -> Vec<DirectoryChange> {
        let mut changes = Vec::new();
        while let Ok(change) = self.event_rx.try_recv() {
            changes.push(change);
        }
        changes
    }
}

/// Processes notify events and converts them to `DirectoryChange`
fn process_notify_events(
    notify_rx: Receiver<Result<Event, notify::Error>>,
    event_tx: &Sender<DirectoryChange>,
    watches: &Arc<Mutex<HashMap<PathBuf, Vec<WatchRequest>>>>,
) {
    for result in notify_rx {
        let event = match result {
            Ok(e) => e,
            Err(e) => {
                warn!("Notify error: {}", e);
                continue;
            }
        };

        trace!("Notify event: {:?}", event);

        // Convert notify event kind to FileAction
        let action = match &event.kind {
            EventKind::Create(_) => FileAction::Added,
            EventKind::Remove(_) => FileAction::Removed,
            EventKind::Modify(modify_kind) => {
                use notify::event::ModifyKind;
                match modify_kind {
                    ModifyKind::Name(_) => FileAction::RenamedNewName,
                    _ => FileAction::Modified,
                }
            }
            _ => continue, // Ignore other events
        };

        // Find matching watches for each affected path
        let Ok(watches_guard) = watches.lock() else {
            continue;
        };

        for event_path in &event.paths {
            // Find the watched directory that contains this path
            for (watched_path, requests) in watches_guard.iter() {
                let is_match = if event_path.starts_with(watched_path) {
                    // Check if it's a direct child or recursive watch
                    let relative = event_path.strip_prefix(watched_path).ok();
                    relative.is_some()
                } else {
                    false
                };

                if !is_match {
                    continue;
                }

                // Get relative path for the notification
                let file_name = event_path
                    .strip_prefix(watched_path)
                    .ok()
                    .and_then(|p| p.to_str())
                    .map(|s| s.replace('/', "\\")) // Convert to Windows path
                    .unwrap_or_default();

                // Send notification for each matching watch request
                for request in requests {
                    // Check completion filter
                    let filter = CompletionFilter(request.completion_filter);
                    if !filter.matches(&event.kind) {
                        continue;
                    }

                    // Check recursive flag
                    if !request.watch_tree && file_name.contains('\\') {
                        continue; // Skip subdirectory changes for non-recursive watches
                    }

                    let change = DirectoryChange {
                        file_id: request.file_id,
                        action,
                        file_name: file_name.clone(),
                    };

                    if event_tx.send(change).is_err() {
                        return; // Channel closed, exit thread
                    }
                }
            }
        }
    }
}

/// Errors that can occur with directory watching
#[derive(Debug, thiserror::Error)]
pub enum DirectoryWatcherError {
    /// Failed to initialize watcher
    #[error("Failed to initialize directory watcher: {0}")]
    InitFailed(String),

    /// Failed to add watch
    #[error("Failed to watch directory {0:?}: {1}")]
    WatchFailed(PathBuf, String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_action_values() {
        assert_eq!(FileAction::Added as u32, 0x0000_0001);
        assert_eq!(FileAction::Removed as u32, 0x0000_0002);
        assert_eq!(FileAction::Modified as u32, 0x0000_0003);
        assert_eq!(FileAction::RenamedOldName as u32, 0x0000_0004);
        assert_eq!(FileAction::RenamedNewName as u32, 0x0000_0005);
    }

    #[test]
    fn test_completion_filter_matches_create() {
        let filter = CompletionFilter(CompletionFilter::FILE_NAME);
        assert!(filter.matches(&EventKind::Create(notify::event::CreateKind::File)));

        let filter_no_match = CompletionFilter(CompletionFilter::SIZE);
        assert!(!filter_no_match.matches(&EventKind::Create(notify::event::CreateKind::File)));
    }

    #[test]
    fn test_completion_filter_matches_modify() {
        let filter = CompletionFilter(CompletionFilter::LAST_WRITE);
        assert!(
            filter.matches(&EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content
            )))
        );
    }

    #[test]
    fn test_watch_request_creation() {
        let request = WatchRequest {
            file_id: 42,
            path: PathBuf::from("/tmp/test"),
            watch_tree: true,
            completion_filter: CompletionFilter::FILE_NAME | CompletionFilter::DIR_NAME,
        };
        assert_eq!(request.file_id, 42);
        assert!(request.watch_tree);
    }
}
