//! FreeRDP thread isolation and clipboard file transfer
//!
//! This module provides thread-safe FreeRDP wrapper and clipboard file transfer
//! state management for RDP sessions.
//!
//! # Safety Notes
//!
//! Mutex locks in this module protect simple state flags and process handles.
//! They are held briefly. If a mutex is poisoned (indicating a thread panic while
//! holding the lock), we recover gracefully by extracting the inner value and
//! setting an error state rather than propagating the panic.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};

#[cfg(feature = "rdp-embedded")]
use rustconn_core::rdp_client::ClipboardFileInfo;
use secrecy::ExposeSecret;

use super::types::{EmbeddedRdpError, FreeRdpThreadState, RdpCommand, RdpConfig, RdpEvent};

// ============================================================================
// Clipboard File Transfer State (for rdp-embedded feature)
// ============================================================================

/// Size of each RANGE request when downloading a clipboard file.
///
/// A clipboard download is pulled in fixed pieces rather than asking for the
/// whole file at once: a single `u32::MAX` request made the server's reply
/// size its own choice, so a large file arrived truncated. 1 MiB keeps the
/// round-trips few while staying well inside what a server will return in one
/// File Contents Response.
#[cfg(feature = "rdp-embedded")]
pub const FILE_DOWNLOAD_CHUNK_SIZE: u32 = 1024 * 1024;

/// Hard ceiling on the bytes accepted for a single clipboard file.
///
/// Completion is decided from the server's own numbers — its announced size, or
/// a chunk shorter than requested. A server that keeps answering with full
/// chunks past the end of the file therefore satisfies neither condition, and
/// the loop would request forever while `FileDownloadState::data` grew without
/// bound. This is the backstop that does not depend on the peer behaving: the
/// download is abandoned once it exceeds the cap.
///
/// 512 MiB is far above the config files and logs that are the realistic
/// clipboard payload, and low enough that the whole-file RAM buffer cannot
/// exhaust a desktop. Raising it only makes sense together with streaming
/// straight to a temp file — see the note on `FileDownloadState::data`.
#[cfg(feature = "rdp-embedded")]
pub const MAX_DOWNLOAD_BYTES: u64 = 512 * 1024 * 1024;

/// What [`ClipboardFileTransfer::append_data`] decided after a chunk.
#[cfg(feature = "rdp-embedded")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkOutcome {
    /// The file is fully received and ready to save.
    Complete,
    /// More data is needed; request the next range from this offset.
    NeedMore {
        /// Byte offset the next RANGE request must start at.
        next_offset: u64,
    },
    /// The download passed [`MAX_DOWNLOAD_BYTES`] and was abandoned.
    ///
    /// Treated like a server refusal: the file is dropped and the rest of the
    /// batch continues.
    TooLarge,
    /// No download is tracked under this stream id (already finished, cancelled,
    /// or the chunk belongs to a superseded batch).
    Unknown,
}

#[cfg(feature = "rdp-embedded")]
#[derive(Debug, Clone)]
pub struct FileDownloadState {
    /// File information from server
    pub file_info: ClipboardFileInfo,
    /// Total file size (may be updated after size request)
    pub total_size: u64,
    /// Bytes received so far
    pub bytes_received: u64,
    /// Accumulated data chunks.
    // ponytail: the whole file is buffered in RAM before it is written, so a
    // multi-gigabyte clipboard file costs that much memory. Fine for the
    // config files and logs that are the realistic clipboard payload; stream
    // straight to a temp file (write each chunk in `append_data`, rename on
    // Complete) if a size limit ever needs lifting.
    pub data: Vec<u8>,
    /// Whether download is complete
    pub complete: bool,
    /// Local path where file will be saved
    pub local_path: Option<PathBuf>,
}

#[cfg(feature = "rdp-embedded")]
impl FileDownloadState {
    /// Creates a new file download state
    pub fn new(file_info: ClipboardFileInfo) -> Self {
        let total_size = file_info.size;
        Self {
            file_info,
            total_size,
            bytes_received: 0,
            data: Vec::new(),
            complete: false,
            local_path: None,
        }
    }

    /// Returns download progress as fraction (0.0 to 1.0)
    pub fn progress(&self) -> f64 {
        if self.total_size == 0 {
            return if self.complete { 1.0 } else { 0.0 };
        }
        crate::utils::progress_fraction(self.bytes_received, self.total_size)
    }
}

/// Manages clipboard file transfer state
#[cfg(feature = "rdp-embedded")]
#[derive(Debug, Default)]
pub struct ClipboardFileTransfer {
    /// Available files from server clipboard
    pub available_files: Vec<ClipboardFileInfo>,
    /// Active downloads keyed by stream_id
    pub downloads: HashMap<u32, FileDownloadState>,
    /// Next stream ID to use for requests
    pub next_stream_id: u32,
    /// Target directory for saving files
    pub target_directory: Option<PathBuf>,
    /// Total files to download
    pub total_files: usize,
    /// Completed downloads count (bytes fully received, before the disk write)
    pub completed_count: usize,
    /// Files that were received in full but could not be written to disk.
    ///
    /// Separate from `completed_count` because the download succeeding and the
    /// save succeeding are two different things: a full disk or a read-only
    /// target fails the write after every byte has arrived. Without this the
    /// batch reported "Saved N files" even when the write threw the file away.
    pub save_failures: usize,
    /// Files that will never deliver: refused by the server, abandoned for
    /// exceeding [`MAX_DOWNLOAD_BYTES`], or failed before a request went out.
    ///
    /// A refused file is settled but not completed, and [`Self::all_complete`]
    /// asks whether every file is *settled*. Without this counter a single
    /// refusal left `completed_count` permanently short of `total_files`, so the
    /// batch never reported a summary and `on_file_complete` never fired — even
    /// when every other file had saved. That is the common case rather than a
    /// rare one, since a server refuses any directory in the list.
    pub refused_count: usize,
}

#[cfg(feature = "rdp-embedded")]
impl ClipboardFileTransfer {
    /// Creates a new file transfer manager
    pub fn new() -> Self {
        Self {
            available_files: Vec::new(),
            downloads: HashMap::new(),
            next_stream_id: 1,
            target_directory: None,
            total_files: 0,
            completed_count: 0,
            save_failures: 0,
            refused_count: 0,
        }
    }

    /// Sets available files from server clipboard
    ///
    /// Does **not** reset `next_stream_id`: ids stay monotonic for the lifetime
    /// of the widget. Restarting them at 1 for each list meant a late reply from
    /// a superseded batch matched a fresh download holding the same id, and its
    /// bytes were appended to the wrong file with nothing to detect it. A
    /// never-reused id makes that stale reply land on no download at all, which
    /// [`Self::append_data`] already reports as [`ChunkOutcome::Unknown`].
    pub fn set_available_files(&mut self, files: Vec<ClipboardFileInfo>) {
        self.available_files = files;
        self.downloads.clear();
        self.reset_batch_counters();
    }

    /// Arms a fresh "Save N Files" run over the current file list.
    ///
    /// Every counter is zeroed here rather than only in
    /// [`Self::set_available_files`], because a second click on the *same* list
    /// never re-announces it: the previous run's `save_failures` would then be
    /// added to the new summary, and its finished downloads would still be in
    /// the map. Takes the target directory so a run cannot be armed without one.
    pub fn begin_batch(&mut self, target_directory: PathBuf, total_files: usize) {
        self.downloads.clear();
        self.reset_batch_counters();
        self.target_directory = Some(target_directory);
        self.total_files = total_files;
    }

    /// Zeroes the per-run tallies, leaving the file list and stream-id sequence.
    fn reset_batch_counters(&mut self) {
        self.total_files = 0;
        self.completed_count = 0;
        self.save_failures = 0;
        self.refused_count = 0;
    }

    /// Starts download for a file, returns stream_id
    pub fn start_download(&mut self, file_index: u32) -> Option<u32> {
        let file_info = self.available_files.get(file_index as usize)?.clone();
        let stream_id = self.next_stream_id;
        // Saturating rather than wrapping: at the ceiling ids would start
        // colliding with live downloads, which is the very thing monotonic ids
        // exist to prevent. Reaching it needs 4 billion downloads in one
        // session, so stalling the sequence is the safer end state.
        self.next_stream_id = self.next_stream_id.saturating_add(1);
        self.downloads
            .insert(stream_id, FileDownloadState::new(file_info));
        Some(stream_id)
    }

    /// Updates file size for a download
    pub fn update_size(&mut self, stream_id: u32, size: u64) {
        if let Some(state) = self.downloads.get_mut(&stream_id) {
            state.total_size = size;
        }
    }

    /// Drops a download the server refused, so the batch is not left waiting on
    /// a stream that will never deliver data. Returns whether one was removed.
    ///
    /// Counts the file as settled-but-not-delivered so [`Self::all_complete`]
    /// can still become true for the batch. A refusal for a stream that is not
    /// tracked — a duplicate error, or one for a superseded batch — is ignored
    /// rather than counted, otherwise it would settle a file that is still in
    /// flight.
    pub fn cancel_download(&mut self, stream_id: u32) -> bool {
        let removed = self.downloads.remove(&stream_id);
        if let Some(state) = removed {
            // A download already counted as complete must not be counted again
            // under a different tally, or the batch would over-settle.
            if !state.complete {
                self.refused_count += 1;
            }
            return true;
        }
        false
    }

    /// The announced file index for an active download, for building the next
    /// RANGE request on the same file.
    pub fn file_index_of(&self, stream_id: u32) -> Option<u32> {
        self.downloads.get(&stream_id).map(|d| d.file_info.index)
    }

    /// Appends a received data chunk and reports what to do next.
    ///
    /// The server delivers a file in one or more chunks, so completion is decided
    /// by comparing bytes received against the known size — not by trusting a
    /// per-chunk "last" flag (there is no reliable one on the wire). A short chunk
    /// (fewer bytes than asked) also means end of file: a server that has no more
    /// to give returns less rather than signalling separately.
    pub fn append_data(&mut self, stream_id: u32, data: &[u8], short_chunk: bool) -> ChunkOutcome {
        let Some(state) = self.downloads.get_mut(&stream_id) else {
            return ChunkOutcome::Unknown;
        };

        // A download that already finished keeps its state in the map until the
        // save step has read it, so a duplicate or late chunk can still arrive
        // here. Appending it would corrupt the buffer, and counting it again
        // would push `completed_count` past `total_files` — settling the batch
        // while other files are still in flight and re-writing this one.
        if state.complete {
            return ChunkOutcome::Unknown;
        }

        state.data.extend_from_slice(data);
        state.bytes_received += data.len() as u64;

        // Neither completion condition below depends on us rather than the peer,
        // so check the one that does first. See `MAX_DOWNLOAD_BYTES`.
        if state.bytes_received > MAX_DOWNLOAD_BYTES {
            // Drop the buffer now rather than at removal: the point of the cap
            // is to stop holding these bytes.
            state.data = Vec::new();
            self.downloads.remove(&stream_id);
            self.refused_count += 1;
            return ChunkOutcome::TooLarge;
        }

        // total_size 0 means the size request has not landed yet; treat any
        // short chunk as the end, otherwise wait for the byte count to catch up.
        let done =
            short_chunk || (state.total_size > 0 && state.bytes_received >= state.total_size);
        if done {
            state.complete = true;
            self.completed_count += 1;
            ChunkOutcome::Complete
        } else {
            ChunkOutcome::NeedMore {
                next_offset: state.bytes_received,
            }
        }
    }

    /// Records that a fully received file could not be written to disk.
    ///
    /// The bytes arrived, so `completed_count` already counted it; this notes
    /// that the save step failed so the batch summary can say "saved N, M
    /// failed" instead of a bare "Saved N files".
    pub const fn record_save_failure(&mut self) {
        self.save_failures += 1;
    }

    /// Number of files that downloaded in full but could not be saved.
    pub const fn save_failures(&self) -> usize {
        self.save_failures
    }

    /// Files actually written to disk: everything that completed, minus the
    /// ones whose disk write failed.
    pub const fn saved_count(&self) -> usize {
        self.completed_count.saturating_sub(self.save_failures)
    }

    /// Files the server never delivered, for the batch summary.
    pub const fn refused_count(&self) -> usize {
        self.refused_count
    }

    /// Files whose fate is decided: delivered, or known never to arrive.
    const fn settled_count(&self) -> usize {
        self.completed_count + self.refused_count
    }

    /// Saves a completed download to disk.
    ///
    /// The name comes from the server, so it is reduced to a single safe
    /// component first and the write refuses to clobber an existing file — see
    /// [`sanitized_file_name`] and [`create_new_in`].
    ///
    /// # Errors
    /// Returns [`std::io::ErrorKind::NotFound`] if no download is tracked under
    /// `stream_id` or no target directory is set, [`std::io::ErrorKind::InvalidData`]
    /// if the download has not finished, [`std::io::ErrorKind::InvalidInput`] if
    /// the server's filename cannot be made safe, and any underlying I/O error
    /// from creating or writing the file.
    pub fn save_download(&self, stream_id: u32) -> Result<PathBuf, std::io::Error> {
        let state = self.downloads.get(&stream_id).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "Download not found")
        })?;

        if !state.complete {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Download not complete",
            ));
        }

        let target_dir = self.target_directory.as_ref().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "Target directory not set")
        })?;

        let name = sanitized_file_name(&state.file_info.name).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "server sent a filename that cannot be written safely",
            )
        })?;

        let (file_path, mut file) = create_new_in(target_dir, &name)?;
        file.write_all(&state.data)?;
        Ok(file_path)
    }

    /// Returns overall progress (0.0 to 1.0)
    ///
    /// Counts settled files, not completed ones, so a batch containing a file
    /// the server refused still reaches 1.0 instead of stalling just short of it.
    pub fn overall_progress(&self) -> f64 {
        if self.total_files == 0 {
            return 0.0;
        }
        crate::utils::progress_fraction(self.settled_count() as u64, self.total_files as u64)
    }

    /// Returns true once every file in the batch is settled.
    ///
    /// Settled means delivered *or* known never to arrive. Asking only about
    /// completed files meant one refusal held the batch open forever, so the
    /// summary and the completion callback never came.
    pub const fn all_complete(&self) -> bool {
        self.total_files > 0 && self.settled_count() >= self.total_files
    }

    /// Clears all state
    pub fn clear(&mut self) {
        self.available_files.clear();
        self.downloads.clear();
        self.next_stream_id = 1;
        self.target_directory = None;
        self.reset_batch_counters();
    }
}

/// Reduces a server-supplied clipboard filename to one safe path component.
///
/// `cFileName` in the file descriptor is whatever the peer chose to send, and
/// `Path::join` *replaces* its base when handed an absolute path — so an
/// unchecked name let a malicious or compromised RDP server write anywhere the
/// user could, the moment a target folder was picked. `..` traversed out of the
/// folder for the same reason.
///
/// Only the final component survives, split on both separators because a
/// Windows server sends Windows paths for a file inside a copied folder (a
/// nested path is flattened to its filename rather than recreating the tree —
/// this code does not walk a directory hierarchy). Returns `None` for anything
/// with no usable component left: empty, whitespace, `.`, `..`, or a name
/// carrying an interior NUL.
#[cfg(feature = "rdp-embedded")]
fn sanitized_file_name(raw: &str) -> Option<String> {
    let last = raw.rsplit(['/', '\\']).next()?.trim();
    if last.is_empty() || last == "." || last == ".." || last.contains('\0') {
        return None;
    }
    Some(last.to_string())
}

/// Creates `name` inside `dir` without ever overwriting an existing file.
///
/// Uses `create_new`, so the check and the create are one atomic step — testing
/// with `exists()` first would leave a window in which the file appears. On a
/// collision the stem gains a ` (1)`, ` (2)` … suffix, the convention a browser
/// download uses. Two descriptors in one batch can legitimately carry the same
/// name, and silently truncating the first file was the old behaviour.
///
/// # Errors
/// Any I/O error other than a collision, and
/// [`std::io::ErrorKind::AlreadyExists`] once the suffix budget is exhausted.
#[cfg(feature = "rdp-embedded")]
fn create_new_in(
    dir: &std::path::Path,
    name: &str,
) -> Result<(PathBuf, std::fs::File), std::io::Error> {
    /// Enough suffixes to cover any realistic clipboard batch while still
    /// terminating if something is generating names in a loop.
    const MAX_ATTEMPTS: u32 = 999;

    let mut attempt = 0u32;
    loop {
        let candidate = dir.join(if attempt == 0 {
            name.to_string()
        } else {
            suffixed_name(name, attempt)
        });
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(e)
                if e.kind() == std::io::ErrorKind::AlreadyExists && attempt < MAX_ATTEMPTS =>
            {
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Inserts ` (n)` before the extension: `notes.txt` → `notes (1).txt`.
///
/// A name with no extension, or a dotfile like `.bashrc` (all stem, no
/// extension as far as `Path` is concerned), gets the suffix appended.
#[cfg(feature = "rdp-embedded")]
fn suffixed_name(name: &str, n: u32) -> String {
    let path = std::path::Path::new(name);
    match (path.file_stem(), path.extension()) {
        (Some(stem), Some(ext)) => format!(
            "{} ({n}).{}",
            stem.to_string_lossy(),
            ext.to_string_lossy()
        ),
        _ => format!("{name} ({n})"),
    }
}

// ============================================================================
// Mutex Poisoning Recovery Helpers
// ============================================================================

/// Safely locks a mutex, recovering from poisoning by extracting the inner value.
///
/// If the mutex is poisoned (a thread panicked while holding the lock),
/// we recover by extracting the inner value. This is safe because our
/// mutex-protected values are simple state flags that can be reset.
fn lock_or_recover<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("Mutex was poisoned, recovering inner value");
            poisoned.into_inner()
        }
    }
}

// ============================================================================
// FreeRDP Thread Isolation
// ============================================================================

/// Consolidated shared state for the FreeRDP thread.
///
/// Groups process handle, thread state, and fallback flag into a single
/// mutex-protected struct to reduce lock contention and simplify reasoning
/// about concurrent access.
struct FreeRdpSharedState {
    /// Handle to the FreeRDP child process
    process: Option<Child>,
    /// Current thread state
    state: FreeRdpThreadState,
    /// Whether fallback to external client was triggered
    fallback_triggered: bool,
}

impl FreeRdpSharedState {
    /// Creates a new shared state with default values
    fn new() -> Self {
        Self {
            process: None,
            state: FreeRdpThreadState::NotStarted,
            fallback_triggered: false,
        }
    }

    /// Kills and waits for the child process if running
    fn cleanup_process(&mut self) {
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Thread-safe FreeRDP wrapper that isolates Qt from GTK main thread
///
/// This struct runs FreeRDP operations in a dedicated thread to avoid
/// Qt/GTK threading conflicts that cause QSocketNotifier and Wayland
/// requestActivate errors.
pub struct FreeRdpThread {
    /// Consolidated process, state, and fallback flag (single lock)
    shared: Arc<Mutex<FreeRdpSharedState>>,
    /// Channel for sending commands to FreeRDP thread
    command_tx: mpsc::Sender<RdpCommand>,
    /// Channel for receiving events from FreeRDP thread
    event_rx: mpsc::Receiver<RdpEvent>,
    /// Thread handle
    thread_handle: Option<JoinHandle<()>>,
}

impl FreeRdpThread {
    /// Spawns FreeRDP in a dedicated thread to avoid Qt/GTK conflicts
    pub fn spawn(config: &RdpConfig) -> Result<Self, EmbeddedRdpError> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<RdpCommand>();
        let (evt_tx, evt_rx) = mpsc::channel::<RdpEvent>();

        let shared = Arc::new(Mutex::new(FreeRdpSharedState::new()));

        let shared_clone = Arc::clone(&shared);
        let config_clone = config.clone();

        let thread_handle = thread::spawn(move || {
            Self::run_freerdp_loop(cmd_rx, evt_tx, shared_clone, config_clone);
        });

        // Initialize state - safe because thread just started and mutex is not poisoned
        lock_or_recover(&shared).state = FreeRdpThreadState::Idle;

        Ok(Self {
            shared,
            command_tx: cmd_tx,
            event_rx: evt_rx,
            thread_handle: Some(thread_handle),
        })
    }

    /// Main loop for FreeRDP operations running in dedicated thread
    ///
    /// Uses mutex poisoning recovery to gracefully handle thread panics.
    fn run_freerdp_loop(
        cmd_rx: mpsc::Receiver<RdpCommand>,
        evt_tx: mpsc::Sender<RdpEvent>,
        shared: Arc<Mutex<FreeRdpSharedState>>,
        initial_config: RdpConfig,
    ) {
        // Note: Qt/Wayland env vars are set per-process via Command::env()
        // in launch_freerdp() to avoid data races from std::env::set_var
        // in multi-threaded context (unsafe since Rust 1.66+).

        let mut current_config = Some(initial_config);

        loop {
            match cmd_rx.recv() {
                Ok(RdpCommand::Connect(config)) => {
                    lock_or_recover(&shared).state = FreeRdpThreadState::Connecting;
                    current_config = Some(*config.clone());

                    match Self::launch_freerdp(&config, &shared) {
                        Ok(()) => {
                            lock_or_recover(&shared).state = FreeRdpThreadState::Connected;
                            let _ = evt_tx.send(RdpEvent::Connected);
                        }
                        Err(e) => {
                            let mut s = lock_or_recover(&shared);
                            s.fallback_triggered = true;
                            s.state = FreeRdpThreadState::Error;
                            drop(s);
                            let _ = evt_tx.send(RdpEvent::FallbackTriggered(e.to_string()));
                        }
                    }
                }
                Ok(RdpCommand::Disconnect) => {
                    let mut s = lock_or_recover(&shared);
                    s.cleanup_process();
                    s.state = FreeRdpThreadState::Idle;
                    drop(s);
                    let _ = evt_tx.send(RdpEvent::Disconnected);
                }
                Ok(RdpCommand::KeyEvent {
                    keyval: _,
                    pressed: _,
                }) => {
                    // Forward keyboard event to FreeRDP process
                }
                Ok(RdpCommand::MouseEvent {
                    x: _,
                    y: _,
                    button: _,
                    pressed: _,
                }) => {
                    // Forward mouse event to FreeRDP process
                }
                Ok(RdpCommand::Resize { width, height }) => {
                    if let Some(ref mut config) = current_config {
                        config.width = width;
                        config.height = height;
                    }
                }
                Ok(RdpCommand::SendCtrlAltDel) => {
                    tracing::debug!("[FreeRDP] Ctrl+Alt+Del requested");
                }
                Ok(RdpCommand::Shutdown) => {
                    let mut s = lock_or_recover(&shared);
                    s.state = FreeRdpThreadState::ShuttingDown;
                    s.cleanup_process();
                    break;
                }
                Err(_) => {
                    lock_or_recover(&shared).cleanup_process();
                    break;
                }
            }
        }
    }

    /// Launches FreeRDP with Qt error suppression
    ///
    /// Uses mutex poisoning recovery for safe process handle storage.
    fn launch_freerdp(
        config: &RdpConfig,
        shared: &Arc<Mutex<FreeRdpSharedState>>,
    ) -> Result<(), EmbeddedRdpError> {
        // Try wlfreerdp first for embedded mode
        let binary = "wlfreerdp";
        if !rustconn_core::which::is_available(binary) {
            return Err(EmbeddedRdpError::WlFreeRdpNotAvailable);
        }

        let mut cmd = Command::new(binary);

        // Set environment to suppress Qt warnings
        cmd.env("QT_LOGGING_RULES", "qt.qpa.wayland=false;qt.qpa.*=false");
        // Do NOT set QT_QPA_PLATFORM — allow wlfreerdp to use native Wayland backend

        // Build connection arguments as a Vec<String> for the args file.
        // FreeRDP requires `/args-from:` to be the ONLY CLI argument — it
        // cannot be combined with other arguments.
        let mut plain_args: Vec<String> = Vec::new();

        if let Some(ref domain) = config.domain
            && !domain.is_empty()
        {
            plain_args.push(format!("/d:{domain}"));
        }

        if let Some(ref username) = config.username {
            plain_args.push(format!("/u:{username}"));
        }

        // Session password is passed as a secret arg via the ephemeral file.
        let session_password = config
            .password
            .as_ref()
            .filter(|p| !p.expose_secret().is_empty());

        let mut secret_args: Vec<(&str, &secrecy::SecretString)> = Vec::new();
        if let Some(p) = session_password {
            secret_args.push(("p", p));
        }

        plain_args.push(format!("/w:{}", config.width));
        plain_args.push(format!("/h:{}", config.height));
        if config.ignore_certificate {
            plain_args.push("/cert:ignore".to_string());
        } else {
            plain_args.push("/cert:tofu".to_string());
        }
        plain_args.push("/dynamic-resolution".to_string());

        if config.clipboard_enabled {
            plain_args.push("+clipboard".to_string());
        }

        // State the audio routing explicitly — FreeRDP's implicit default is
        // "no audio at all" (issue #245). Before extra_args so a hand-written
        // override still wins.
        plain_args.push(config.audio_mode.freerdp_arg().to_string());

        for arg in &config.extra_args {
            plain_args.push(arg.clone());
        }

        if config.port == 3389 {
            plain_args.push(format!("/v:{}", config.host));
        } else {
            plain_args.push(format!("/v:{}:{}", config.host, config.port));
        }

        // Write all arguments (plain + secret) to the ephemeral args file
        let _args_guard =
            match super::ephemeral_args::EphemeralRdpArgs::write_all(&plain_args, &secret_args) {
                Ok(guard) => {
                    cmd.arg(super::detect::args_from_argument(binary, guard.path()));
                    guard
                }
                Err(e) => {
                    return Err(EmbeddedRdpError::FreeRdpInit(format!(
                        "could not prepare RDP args file: {e}"
                    )));
                }
            };

        // Redirect stderr to suppress Qt warnings
        cmd.stderr(Stdio::null());

        match cmd.spawn() {
            Ok(child) => {
                lock_or_recover(shared).process = Some(child);
                // _args_guard is dropped here after FreeRDP has consumed
                // the file during argument parsing (synchronous before fork).
                Ok(())
            }
            Err(e) => Err(EmbeddedRdpError::FreeRdpInit(e.to_string())),
        }
    }

    /// Sends a command to the FreeRDP thread
    pub fn send_command(&self, cmd: RdpCommand) -> Result<(), EmbeddedRdpError> {
        self.command_tx
            .send(cmd)
            .map_err(|e| EmbeddedRdpError::ThreadError(e.to_string()))
    }

    /// Tries to receive an event from the FreeRDP thread (non-blocking)
    pub fn try_recv_event(&self) -> Option<RdpEvent> {
        self.event_rx.try_recv().ok()
    }

    /// Returns the current thread state
    ///
    /// Uses mutex poisoning recovery for safe state access.
    pub fn state(&self) -> FreeRdpThreadState {
        lock_or_recover(&self.shared).state
    }

    /// Returns whether fallback was triggered
    ///
    /// Uses mutex poisoning recovery for safe flag access.
    pub fn fallback_triggered(&self) -> bool {
        lock_or_recover(&self.shared).fallback_triggered
    }

    /// Shuts down the FreeRDP thread
    pub fn shutdown(&mut self) {
        let _ = self.command_tx.send(RdpCommand::Shutdown);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for FreeRdpThread {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(all(test, feature = "rdp-embedded"))]
mod tests {
    use rustconn_core::rdp_client::ClipboardFileInfo;

    use super::{ChunkOutcome, ClipboardFileTransfer};

    fn transfer_with_file(size: u64) -> (ClipboardFileTransfer, u32) {
        let mut t = ClipboardFileTransfer::new();
        t.set_available_files(vec![ClipboardFileInfo::new(
            "big.bin".to_string(),
            size,
            0,
            0,
            0,
        )]);
        t.total_files = 1;
        let stream_id = t.start_download(0).expect("download starts");
        t.update_size(stream_id, size);
        (t, stream_id)
    }

    #[test]
    fn a_file_larger_than_one_chunk_requests_the_next_range() {
        let (mut t, sid) = transfer_with_file(3000);
        // First 1000-byte chunk of a 3000-byte file: not done, ask for more from
        // offset 1000.
        let outcome = t.append_data(sid, &vec![0u8; 1000], false);
        assert_eq!(outcome, ChunkOutcome::NeedMore { next_offset: 1000 });
        assert!(!t.all_complete());
    }

    #[test]
    fn reaching_the_known_size_completes_the_download() {
        let (mut t, sid) = transfer_with_file(2000);
        assert_eq!(
            t.append_data(sid, &vec![0u8; 1000], false),
            ChunkOutcome::NeedMore { next_offset: 1000 }
        );
        // Second chunk brings bytes_received to the full size → complete.
        assert_eq!(
            t.append_data(sid, &vec![0u8; 1000], false),
            ChunkOutcome::Complete
        );
        assert!(t.all_complete());
    }

    #[test]
    fn a_short_chunk_ends_the_download_even_below_the_estimated_size() {
        // The server claims 5000 bytes but returns fewer than requested: treat
        // the short chunk as end of file rather than looping forever.
        let (mut t, sid) = transfer_with_file(5000);
        assert_eq!(
            t.append_data(sid, &vec![0u8; 800], true),
            ChunkOutcome::Complete
        );
        assert!(t.all_complete());
    }

    #[test]
    fn a_chunk_for_an_unknown_stream_is_reported() {
        let mut t = ClipboardFileTransfer::new();
        assert_eq!(t.append_data(999, &[1, 2, 3], false), ChunkOutcome::Unknown);
    }

    #[test]
    fn cancel_and_file_index_lookup() {
        let (mut t, sid) = transfer_with_file(100);
        assert_eq!(t.file_index_of(sid), Some(0));
        assert!(t.cancel_download(sid));
        assert_eq!(t.file_index_of(sid), None);
        assert!(!t.cancel_download(sid));
    }

    /// A file whose bytes all arrived but whose disk write failed counts as
    /// completed (the download did finish) yet not as saved, so the batch
    /// summary can say "N saved, M failed" instead of claiming success.
    #[test]
    fn a_save_failure_is_counted_apart_from_a_saved_file() {
        let (mut t, sid) = transfer_with_file(1000);
        assert_eq!(
            t.append_data(sid, &vec![0u8; 1000], false),
            ChunkOutcome::Complete
        );
        // Downloaded, but the write to disk failed.
        t.record_save_failure();

        assert_eq!(t.completed_count, 1, "the download itself did complete");
        assert_eq!(t.save_failures(), 1);
        assert_eq!(t.saved_count(), 0, "nothing actually reached the disk");
        // The batch is still settled, so the button is freed and the summary
        // shown — it just reports the failure rather than a phantom success.
        assert!(t.all_complete());
    }

    /// With no write failures, every completed file counts as saved.
    #[test]
    fn without_a_failure_saved_equals_completed() {
        let (mut t, sid) = transfer_with_file(500);
        assert_eq!(
            t.append_data(sid, &vec![0u8; 500], false),
            ChunkOutcome::Complete
        );
        assert_eq!(t.save_failures(), 0);
        assert_eq!(t.saved_count(), 1);
    }

    /// A fresh file list resets the failure tally along with the rest, so a
    /// later batch does not inherit an earlier one's failures.
    #[test]
    fn a_new_file_list_resets_the_save_failure_tally() {
        let (mut t, sid) = transfer_with_file(100);
        let _ = t.append_data(sid, &[0u8; 100], false);
        t.record_save_failure();
        assert_eq!(t.save_failures(), 1);

        t.set_available_files(vec![ClipboardFileInfo::new(
            "next.bin".to_string(),
            10,
            0,
            0,
            0,
        )]);
        assert_eq!(t.save_failures(), 0);
        assert_eq!(t.saved_count(), 0);
    }

    // ------------------------------------------------------------------
    // Batch settlement
    // ------------------------------------------------------------------

    /// A file the server refuses must still settle the batch. Counting only
    /// completed files left `completed_count` permanently short of
    /// `total_files`, so the summary and the completion callback never came.
    #[test]
    fn a_refused_file_still_settles_the_batch() {
        let mut t = ClipboardFileTransfer::new();
        t.set_available_files(vec![
            ClipboardFileInfo::new("a.bin".to_string(), 10, 0, 0, 0),
            ClipboardFileInfo::new("b.bin".to_string(), 10, 0, 0, 1),
        ]);
        t.begin_batch(std::path::PathBuf::from("/tmp"), 2);
        let first = t.start_download(0).expect("first download starts");
        let second = t.start_download(1).expect("second download starts");
        t.update_size(first, 10);
        t.update_size(second, 10);

        assert_eq!(t.append_data(first, &[0u8; 10], false), ChunkOutcome::Complete);
        assert!(!t.all_complete(), "one file is still outstanding");

        // The server refuses the second file.
        assert!(t.cancel_download(second));
        assert!(
            t.all_complete(),
            "a refusal settles the file, so the batch is done"
        );
        assert_eq!(t.saved_count(), 1);
        assert_eq!(t.refused_count(), 1);
    }

    /// A refusal for a stream that is not tracked — a duplicate error, or one
    /// for a superseded batch — must not settle a file that is still in flight.
    #[test]
    fn a_refusal_for_an_unknown_stream_settles_nothing() {
        let (mut t, _sid) = transfer_with_file(10);
        assert!(!t.cancel_download(4242));
        assert_eq!(t.refused_count(), 0);
        assert!(!t.all_complete());
    }

    /// Cancelling an already-completed download must not count it twice, which
    /// would over-settle the batch.
    #[test]
    fn cancelling_a_completed_download_does_not_double_count() {
        let (mut t, sid) = transfer_with_file(10);
        assert_eq!(t.append_data(sid, &[0u8; 10], false), ChunkOutcome::Complete);
        assert!(t.cancel_download(sid));
        assert_eq!(t.refused_count(), 0, "it completed; it was not refused");
        assert_eq!(t.completed_count, 1);
    }

    /// Progress must reach 1.0 even when part of the batch was refused,
    /// otherwise the bar stalls just short of the end.
    #[test]
    fn progress_completes_even_with_a_refusal() {
        let mut t = ClipboardFileTransfer::new();
        t.set_available_files(vec![
            ClipboardFileInfo::new("a.bin".to_string(), 10, 0, 0, 0),
            ClipboardFileInfo::new("b.bin".to_string(), 10, 0, 0, 1),
        ]);
        t.begin_batch(std::path::PathBuf::from("/tmp"), 2);
        let first = t.start_download(0).expect("starts");
        let second = t.start_download(1).expect("starts");
        t.update_size(first, 10);
        let _ = t.append_data(first, &[0u8; 10], false);
        t.cancel_download(second);
        assert!((t.overall_progress() - 1.0).abs() < f64::EPSILON);
    }

    // ------------------------------------------------------------------
    // Loop safety
    // ------------------------------------------------------------------

    /// A chunk arriving after the download completed must be ignored. It used to
    /// be appended and counted again, which pushed `completed_count` past
    /// `total_files` and re-wrote the file.
    #[test]
    fn a_chunk_after_completion_is_ignored() {
        let (mut t, sid) = transfer_with_file(10);
        assert_eq!(t.append_data(sid, &[1u8; 10], false), ChunkOutcome::Complete);
        assert_eq!(t.completed_count, 1);

        // A duplicate of the same chunk.
        assert_eq!(t.append_data(sid, &[1u8; 10], false), ChunkOutcome::Unknown);
        assert_eq!(t.completed_count, 1, "the file was not counted twice");
        assert_eq!(
            t.downloads.get(&sid).map(|d| d.bytes_received),
            Some(10),
            "the duplicate was not appended"
        );
    }

    /// A server that never answers short and overstates the size would loop
    /// forever. The byte cap is the backstop that does not rely on the peer.
    #[test]
    fn passing_the_byte_cap_abandons_the_download() {
        use super::MAX_DOWNLOAD_BYTES;

        let mut t = ClipboardFileTransfer::new();
        t.set_available_files(vec![ClipboardFileInfo::new(
            "endless.bin".to_string(),
            u64::MAX,
            0,
            0,
            0,
        )]);
        t.begin_batch(std::path::PathBuf::from("/tmp"), 1);
        let sid = t.start_download(0).expect("starts");
        t.update_size(sid, u64::MAX);

        // Force the accounting past the cap without allocating half a gigabyte.
        if let Some(state) = t.downloads.get_mut(&sid) {
            state.bytes_received = MAX_DOWNLOAD_BYTES;
        }
        assert_eq!(t.append_data(sid, &[0u8; 1], false), ChunkOutcome::TooLarge);
        assert_eq!(t.refused_count(), 1);
        assert!(!t.downloads.contains_key(&sid), "the buffer was released");
        assert!(t.all_complete(), "the batch settles rather than hanging");
    }

    /// Stream ids must not restart with each batch: a late reply carrying an old
    /// id would otherwise match a fresh download and corrupt the wrong file.
    #[test]
    fn stream_ids_stay_monotonic_across_batches() {
        let mut t = ClipboardFileTransfer::new();
        t.set_available_files(vec![ClipboardFileInfo::new(
            "a.bin".to_string(),
            1,
            0,
            0,
            0,
        )]);
        let first = t.start_download(0).expect("starts");

        t.set_available_files(vec![ClipboardFileInfo::new(
            "b.bin".to_string(),
            1,
            0,
            0,
            0,
        )]);
        let second = t.start_download(0).expect("starts");

        assert_ne!(first, second, "a new batch must not reuse a stream id");
        // A stale chunk for the retired id lands on nothing.
        assert_eq!(t.append_data(first, &[0u8; 1], false), ChunkOutcome::Unknown);
    }

    /// A second click on the same file list never re-announces it, so the run
    /// counters have to be reset when the batch is armed rather than only when a
    /// list arrives.
    #[test]
    fn re_arming_a_batch_clears_the_previous_run() {
        let (mut t, sid) = transfer_with_file(10);
        let _ = t.append_data(sid, &[0u8; 10], false);
        t.record_save_failure();
        assert_eq!(t.save_failures(), 1);

        t.begin_batch(std::path::PathBuf::from("/tmp"), 1);
        assert_eq!(t.save_failures(), 0);
        assert_eq!(t.completed_count, 0);
        assert_eq!(t.refused_count(), 0);
        assert!(t.downloads.is_empty(), "the old downloads are gone");
        assert_eq!(t.total_files, 1);
    }

    // ------------------------------------------------------------------
    // Filename safety
    // ------------------------------------------------------------------

    #[test]
    fn a_safe_name_survives_unchanged() {
        assert_eq!(
            super::sanitized_file_name("notes.txt").as_deref(),
            Some("notes.txt")
        );
    }

    /// `Path::join` *replaces* its base when given an absolute path, so an
    /// unchecked name let the server write anywhere the user could.
    #[test]
    fn an_absolute_name_is_reduced_to_its_last_component() {
        assert_eq!(
            super::sanitized_file_name("/etc/cron.d/evil").as_deref(),
            Some("evil")
        );
    }

    #[test]
    fn a_traversing_name_is_reduced_to_its_last_component() {
        assert_eq!(
            super::sanitized_file_name("../../.ssh/authorized_keys").as_deref(),
            Some("authorized_keys")
        );
    }

    /// A Windows server sends Windows separators for a file inside a copied
    /// folder, and `Path::file_name` does not split on those under Unix.
    #[test]
    fn a_windows_subpath_is_reduced_to_its_last_component() {
        assert_eq!(
            super::sanitized_file_name(r"sub\deeper\file.txt").as_deref(),
            Some("file.txt")
        );
        assert_eq!(
            super::sanitized_file_name(r"C:\Users\bob\secret.doc").as_deref(),
            Some("secret.doc")
        );
    }

    #[test]
    fn names_with_no_usable_component_are_refused() {
        for raw in ["", "   ", ".", "..", "a/b/", "with\0nul", "sub/.."] {
            assert!(
                super::sanitized_file_name(raw).is_none(),
                "{raw:?} must be refused"
            );
        }
    }

    #[test]
    fn the_collision_suffix_goes_before_the_extension() {
        assert_eq!(super::suffixed_name("notes.txt", 1), "notes (1).txt");
        assert_eq!(super::suffixed_name("archive.tar.gz", 2), "archive.tar (2).gz");
        assert_eq!(super::suffixed_name("README", 3), "README (3)");
        assert_eq!(super::suffixed_name(".bashrc", 1), ".bashrc (1)");
    }

    // ------------------------------------------------------------------
    // save_download
    // ------------------------------------------------------------------

    /// Builds a transfer holding one completed download whose server-supplied
    /// name is `name`, targeted at `dir`.
    fn completed_download_named(dir: &std::path::Path, name: &str) -> (ClipboardFileTransfer, u32) {
        let mut t = ClipboardFileTransfer::new();
        t.set_available_files(vec![ClipboardFileInfo::new(
            name.to_string(),
            4,
            0,
            0,
            0,
        )]);
        t.begin_batch(dir.to_path_buf(), 1);
        let sid = t.start_download(0).expect("starts");
        t.update_size(sid, 4);
        assert_eq!(t.append_data(sid, b"data", false), ChunkOutcome::Complete);
        (t, sid)
    }

    #[test]
    fn a_download_is_written_into_the_chosen_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (t, sid) = completed_download_named(dir.path(), "notes.txt");

        let path = t.save_download(sid).expect("the write succeeds");
        assert_eq!(path, dir.path().join("notes.txt"));
        assert_eq!(std::fs::read(&path).expect("readable"), b"data");
    }

    /// The security case: an absolute name from the server must not escape the
    /// directory the user picked.
    #[test]
    fn an_absolute_server_name_cannot_escape_the_chosen_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let escape = dir.path().join("outside.txt");
        let (t, sid) = completed_download_named(
            &dir.path().join("inside"),
            escape.to_str().expect("utf-8 temp path"),
        );
        std::fs::create_dir_all(dir.path().join("inside")).expect("target dir");

        let path = t.save_download(sid).expect("the write succeeds");
        assert_eq!(
            path,
            dir.path().join("inside").join("outside.txt"),
            "the name was reduced to its last component"
        );
        assert!(!escape.exists(), "nothing was written outside the target");
    }

    #[test]
    fn a_traversing_server_name_cannot_escape_the_chosen_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let inner = dir.path().join("inner");
        std::fs::create_dir_all(&inner).expect("target dir");
        let (t, sid) = completed_download_named(&inner, "../escaped.txt");

        let path = t.save_download(sid).expect("the write succeeds");
        assert_eq!(path, inner.join("escaped.txt"));
        assert!(
            !dir.path().join("escaped.txt").exists(),
            "the parent directory was not touched"
        );
    }

    #[test]
    fn a_name_with_no_usable_component_is_refused_rather_than_written() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (t, sid) = completed_download_named(dir.path(), "..");

        let err = t.save_download(sid).expect_err("must refuse");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(
            std::fs::read_dir(dir.path()).expect("readable").count(),
            0,
            "nothing was created"
        );
    }

    /// Two descriptors in one batch can carry the same name. Truncating the
    /// first file was the old behaviour.
    #[test]
    fn an_existing_file_is_not_clobbered() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("notes.txt"), b"original").expect("seed file");

        let (t, sid) = completed_download_named(dir.path(), "notes.txt");
        let path = t.save_download(sid).expect("the write succeeds");

        assert_eq!(path, dir.path().join("notes (1).txt"));
        assert_eq!(
            std::fs::read(dir.path().join("notes.txt")).expect("readable"),
            b"original",
            "the existing file is untouched"
        );
        assert_eq!(std::fs::read(&path).expect("readable"), b"data");
    }

    #[test]
    fn an_incomplete_download_is_not_written() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut t = ClipboardFileTransfer::new();
        t.set_available_files(vec![ClipboardFileInfo::new(
            "partial.bin".to_string(),
            100,
            0,
            0,
            0,
        )]);
        t.begin_batch(dir.path().to_path_buf(), 1);
        let sid = t.start_download(0).expect("starts");
        t.update_size(sid, 100);
        assert_eq!(
            t.append_data(sid, &[0u8; 10], false),
            ChunkOutcome::NeedMore { next_offset: 10 }
        );

        let err = t.save_download(sid).expect_err("must refuse");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }
}
