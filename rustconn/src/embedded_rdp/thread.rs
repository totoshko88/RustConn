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
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStderr, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rustconn_core::protocol::FreeRdpSelection;
#[cfg(feature = "rdp-embedded")]
use rustconn_core::rdp_client::ClipboardFileInfo;
use secrecy::ExposeSecret;

use super::types::{
    EmbeddedClientExit, EmbeddedRdpError, FreeRdpThreadState, RdpCommand, RdpConfig, RdpEvent,
};

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
    // ponytail: the whole file is buffered in RAM before it is written, one
    // file at a time and released once saved, so a batch costs at most one
    // file's worth of memory, capped at MAX_DOWNLOAD_BYTES. Fine for the config
    // files and logs that are the realistic clipboard payload; stream straight
    // to a temp file (write each chunk in `append_data`, rename on Complete) if
    // the cap ever needs lifting.
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
    /// batch never reported its summary and the button never reset — even when
    /// every other file had saved. That is the common case rather than a rare
    /// one, since a server refuses any directory in the list.
    pub refused_count: usize,
    /// Positions in `available_files` that the batch has yet to start, in order.
    ///
    /// A batch fetches one file at a time. Asking for every file at once kept
    /// all of them in memory together — up to [`MAX_DOWNLOAD_BYTES`] each — and
    /// a large folder copy ran into IronRDP's limit of 1000 pending requests,
    /// so the files past it failed.
    pub pending: VecDeque<usize>,
    /// The "Save N Files" folder picker is open.
    ///
    /// Set from the moment Save is pressed, *before* the picker opens — not
    /// from when a batch starts. Under a portal (Flatpak, Snap) the picker is a
    /// separate Wayland client; returning focus to RustConn makes GTK report a
    /// local clipboard change before the callback with the chosen folder has
    /// even run. Holding from the button press rather than from `begin_batch`
    /// covers that ordering (issue #390).
    pub folder_pick_pending: bool,
    /// A local clipboard change was seen while [`Self::holds_local_announce`]
    /// was true, and still needs to be sent to the server once the hold is
    /// released.
    ///
    /// The change itself is never read or stored here — only that one happened
    /// — because the monitor announces *availability*, not data; the server
    /// pulls the actual text later with its own Format Data Request.
    pub announce_deferred: bool,
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
            pending: VecDeque::new(),
            folder_pick_pending: false,
            announce_deferred: false,
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
        // The remote just announced a clipboard of its own — a newer one than
        // whatever a local change deferred earlier was waiting to replace.
        // Sending that stale change once some later, unrelated hold came down
        // would override the list just received (issue #390, review finding:
        // deferral outliving a superseded batch).
        self.announce_deferred = false;
    }

    /// Withdraws the offer: the remote clipboard no longer holds these files.
    ///
    /// The list is dropped, so a folder picked after this saves nothing. A file
    /// already in flight is left to finish, but nothing new is asked for: the
    /// server answers a request from whatever its clipboard holds now, so a
    /// later request could return another file's bytes under this name. The
    /// files the batch had not started are settled as not delivered, so the
    /// summary still comes.
    pub fn withdraw_offer(&mut self) {
        self.available_files.clear();
        self.refused_count += self.pending.len();
        self.pending.clear();
        // Same reasoning as `set_available_files`: the remote clipboard just
        // changed to something that is not files, so a local change deferred
        // earlier must not be sent on top of it later (issue #390).
        self.announce_deferred = false;
    }

    /// Arms a fresh "Save N Files" run over the list offered right now.
    ///
    /// Returns how many files the run will fetch, zero when the offer was
    /// withdrawn while the folder dialog was open. The list is read here, when
    /// the folder has been picked, rather than when the button was pressed: a
    /// snapshot taken at the press counted files that a list arriving in
    /// between did not have, so the batch could never settle.
    ///
    /// Every counter is zeroed here rather than only in
    /// [`Self::set_available_files`], because a second click on the *same* list
    /// never re-announces it: the previous run's `save_failures` would then be
    /// added to the new summary, and its finished downloads would still be in
    /// the map. Takes the target directory so a run cannot be armed without one.
    pub fn begin_batch(&mut self, target_directory: PathBuf) -> usize {
        self.downloads.clear();
        self.reset_batch_counters();
        // The folder has been picked, so the hold the button press armed via
        // `begin_folder_pick` is no longer needed on its own account: from here
        // `holds_local_announce` is carried by the batch itself (`total_files`
        // below) until every file settles.
        self.folder_pick_pending = false;
        self.target_directory = Some(target_directory);
        self.total_files = self.available_files.len();
        self.pending = (0..self.total_files).collect();
        self.total_files
    }

    /// Starts the batch's next file: its stream id and the server's index for
    /// the size request, or `None` once every file has been started.
    ///
    /// A position the list no longer holds is settled as not delivered rather
    /// than skipped, or the batch would wait for it forever.
    pub fn start_next_download(&mut self) -> Option<(u32, u32)> {
        while let Some(position) = self.pending.pop_front() {
            let started = u32::try_from(position)
                .ok()
                .and_then(|position| self.start_download(position));
            if let Some(stream_id) = started
                && let Some(file_index) = self.file_index_of(stream_id)
            {
                return Some((stream_id, file_index));
            }
            self.refused_count += 1;
        }
        None
    }

    /// Zeroes the per-run tallies and queue, leaving the file list and the
    /// stream-id sequence.
    fn reset_batch_counters(&mut self) {
        self.total_files = 0;
        self.completed_count = 0;
        self.save_failures = 0;
        self.refused_count = 0;
        self.pending.clear();
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

    /// Drops a download whose size reply is over [`MAX_DOWNLOAD_BYTES`].
    ///
    /// Returns whether it was dropped. Such a file is settled as not delivered
    /// before any of it is fetched, instead of being pulled up to the cap only
    /// to be thrown away there.
    pub fn refuse_if_oversized(&mut self, stream_id: u32, size: u64) -> bool {
        if size <= MAX_DOWNLOAD_BYTES || self.downloads.remove(&stream_id).is_none() {
            return false;
        }
        self.refused_count += 1;
        true
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

    /// Saves a completed download to disk and releases its buffer.
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
    pub fn save_download(&mut self, stream_id: u32) -> Result<PathBuf, std::io::Error> {
        let not_found = || std::io::Error::new(std::io::ErrorKind::NotFound, "Download not found");
        let complete = self
            .downloads
            .get(&stream_id)
            .map(|state| state.complete)
            .ok_or_else(not_found)?;
        if !complete {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Download not complete",
            ));
        }
        // A finished download is written once and then released, whether the
        // write succeeds or not. Its bytes used to stay in memory until the next
        // batch, so every saved file stayed resident for the rest of the session.
        // A late chunk for it now finds no download, which `append_data` already
        // reports as `Unknown`.
        let state = self.downloads.remove(&stream_id).ok_or_else(not_found)?;

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

    /// Marks that the "Save N Files" folder picker is about to open.
    ///
    /// Called before `dialog.select_folder(...)`, not from its callback: focus
    /// can return to RustConn — and with it a spurious local clipboard
    /// `changed` signal — before the callback carrying the chosen path has run.
    /// The hold has to be in place for that window too (issue #390).
    pub const fn begin_folder_pick(&mut self) {
        self.folder_pick_pending = true;
    }

    /// Releases the hold [`Self::begin_folder_pick`] took, without a batch
    /// having started: the dialog was dismissed, cancelled, failed, or
    /// returned no path, or the remote clipboard no longer offered anything by
    /// the time a batch would have started.
    pub const fn cancel_folder_pick(&mut self) {
        self.folder_pick_pending = false;
    }

    /// Whether a local clipboard change must not be announced to the server
    /// right now.
    ///
    /// True while the folder picker is open, and while a batch has files that
    /// have not yet settled: announcing in either window replaces the
    /// server-side clipboard the in-flight download is reading from (MS-RDPECLIP
    /// Format List PDUs are not additive — each one replaces the last), so the
    /// server stops answering `FileContentsRequest`s for a file RustConn is
    /// still fetching and the transfer times out with nothing saved
    /// (issue #390).
    pub const fn holds_local_announce(&self) -> bool {
        self.folder_pick_pending || (self.total_files > 0 && !self.all_complete())
    }

    /// Records that a local clipboard change happened while
    /// [`Self::holds_local_announce`] was true, so it can be sent once the hold
    /// is released instead of being lost.
    pub const fn defer_announce(&mut self) {
        self.announce_deferred = true;
    }

    /// Takes and clears the deferred-announce flag, so it fires at most once
    /// per change it was set for.
    pub fn take_deferred_announce(&mut self) -> bool {
        std::mem::take(&mut self.announce_deferred)
    }

    /// Discards a deferred announce without sending it.
    ///
    /// Used when the remote clipboard itself changes to something new while an
    /// announce was deferred: the remote's own Format List PDU just became the
    /// shared clipboard's current content, and sending the stale local change
    /// afterwards would replace it — the same clobbering
    /// [`Self::holds_local_announce`] exists to prevent, just arriving from the
    /// other direction (issue #390, review finding: deferral outliving a
    /// superseded batch).
    pub const fn drop_deferred_announce(&mut self) {
        self.announce_deferred = false;
    }

    /// Clears all state
    pub fn clear(&mut self) {
        self.available_files.clear();
        self.downloads.clear();
        self.next_stream_id = 1;
        self.target_directory = None;
        self.reset_batch_counters();
        self.folder_pick_pending = false;
        self.announce_deferred = false;
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
///
/// Leading dots are stripped as well, the way a browser treats a download. The
/// name is the server's and the user never sees it — the button shows only a
/// count — so `.bash_profile` or `.zshenv` would land in the chosen folder as a
/// hidden file that the next login shell runs; `create_new` only refuses to
/// *overwrite* one that exists. Control and bidirectional-override characters
/// are dropped too, since they change what a file manager shows without being
/// visible themselves: `"\u{202E}fdp.exe"` reads as `exe.pdf`.
#[cfg(feature = "rdp-embedded")]
fn sanitized_file_name(raw: &str) -> Option<String> {
    let last = raw.rsplit(['/', '\\']).next()?;
    if last.contains('\0') {
        return None;
    }
    let visible: String = last.chars().filter(|&c| !is_hidden_name_char(c)).collect();
    // Dots and whitespace together, so `. .profile` cannot leave a dot behind.
    let name = visible
        .trim_start_matches(|c: char| c == '.' || c.is_whitespace())
        .trim_end();
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

/// Whether `c` changes how a filename displays without being visible in it: the
/// C0 and C1 controls, DEL, and the Unicode bidirectional formatting characters.
#[cfg(feature = "rdp-embedded")]
fn is_hidden_name_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
        )
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
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempt < MAX_ATTEMPTS => {
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
        (Some(stem), Some(ext)) => {
            format!("{} ({n}).{}", stem.to_string_lossy(), ext.to_string_lossy())
        }
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
// Embedded Client Exit Watch
// ============================================================================

/// Bytes of the embedded client's stderr kept to explain an early exit.
///
/// 16 KiB holds the last few dozen lines FreeRDP prints before it gives up,
/// which is where the reason is, while a client that logs for hours cannot
/// grow it.
pub const EMBEDDED_STDERR_TAIL_BYTES: usize = 16 * 1024;

/// How much of that tail goes into the debug log when the client exits.
pub const EMBEDDED_STDERR_LOG_BYTES: usize = 2 * 1024;

/// Longest stderr line kept; a longer one is cut so that a single runaway line
/// cannot push the rest of the tail out.
const STDERR_LINE_MAX_BYTES: usize = 1024;

/// How long after launch an exit of the embedded client means the connection
/// failed, rather than a session that ended.
///
/// A real session does not end within seconds of starting, while a client that
/// rejects its command line, fails authentication or cannot reach the host
/// exits well inside this window — the reasoning the external client's watchdog
/// applies as well. Five seconds also leaves room for a slow TLS handshake.
pub const EMBEDDED_EARLY_EXIT_WINDOW: Duration = Duration::from_secs(5);

/// How often the FreeRDP thread looks at its client while it waits for a
/// command. Commands are still taken the moment they arrive.
const CLIENT_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// How long an exit waits for the stderr reader to drain what the client wrote
/// just before it went.
const STDERR_DRAIN_TIMEOUT: Duration = Duration::from_millis(500);

/// Poll interval while waiting for the stderr reader to finish.
const STDERR_DRAIN_POLL: Duration = Duration::from_millis(10);

/// Whether an embedded client that exited after `running_for` failed to
/// connect, as opposed to a session that ended.
#[must_use]
pub fn embedded_exit_is_failure(running_for: Duration) -> bool {
    running_for < EMBEDDED_EARLY_EXIT_WINDOW
}

/// The last lines a client wrote to stderr, bounded in bytes.
///
/// Replaces the `/dev/null` the embedded launch used to send stderr to. That
/// redirect hid Qt chatter, and with it the one line saying why a client exited
/// right after launch; this keeps FreeRDP's lines and drops the chatter.
#[derive(Debug)]
pub struct StderrTail {
    lines: VecDeque<String>,
    bytes: usize,
    capacity: usize,
}

impl StderrTail {
    /// Creates an empty tail that keeps at most `capacity` bytes of lines.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            bytes: 0,
            capacity,
        }
    }

    /// Records one line, dropping blank lines, Qt platform noise and any line
    /// that repeats a password switch.
    pub fn push_line(&mut self, raw: &str) {
        let line = raw.trim();
        if line.is_empty() || is_qt_noise(line) || echoes_password_switch(line) {
            return;
        }
        let line = truncate_at_char_boundary(line, STDERR_LINE_MAX_BYTES);
        self.bytes += line.len();
        self.lines.push_back(line.to_owned());
        while self.bytes > self.capacity {
            let Some(oldest) = self.lines.pop_front() else {
                break;
            };
            self.bytes -= oldest.len();
        }
    }

    /// The kept lines, oldest first, one per line.
    #[must_use]
    pub fn joined(&self) -> String {
        let lines: Vec<&str> = self.lines.iter().map(String::as_str).collect();
        lines.join("\n")
    }
}

/// Whether a stderr line is Qt platform chatter rather than FreeRDP's own
/// output: the `qt.qpa.*` logging categories, `QObject`/`QSocketNotifier`
/// threading warnings and Qt's session-type notice.
fn is_qt_noise(line: &str) -> bool {
    line.starts_with("qt.")
        || line.starts_with("QObject::")
        || line.contains("QSocketNotifier")
        || line.contains("QT_QPA_PLATFORM")
}

/// Whether a stderr line repeats a password switch.
///
/// The password only ever travels in the `/args-from:` file. FreeRDP quotes an
/// argument it rejects, so a line naming one of these switches is dropped
/// rather than kept — and later logged.
fn echoes_password_switch(line: &str) -> bool {
    const PASSWORD_SWITCHES: [&str; 4] = ["/p:", "/password:", "/gp:", "/gateway-password:"];
    let lower = line.to_ascii_lowercase();
    PASSWORD_SWITCHES
        .iter()
        .any(|switch| lower.contains(switch))
}

/// The first `max_bytes` of `text`, cut back to a character boundary.
fn truncate_at_char_boundary(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The last `max_bytes` of `text`, moved forward to a character boundary.
#[must_use]
pub fn tail_excerpt(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut start = text.len() - max_bytes;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// Drains a client's stderr into `tail` until the pipe closes.
///
/// Reads raw lines rather than `String`s: `BufRead::lines` stops at the first
/// invalid UTF-8 byte, after which nothing reads the pipe and a chatty client
/// blocks on it once it is full.
fn spawn_stderr_reader(pipe: ChildStderr, tail: Arc<Mutex<StderrTail>>) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => lock_or_recover(&tail).push_line(&String::from_utf8_lossy(&line)),
            }
        }
    })
}

/// What the FreeRDP thread keeps about the client it launched.
struct ClientWatch {
    /// When the client was spawned
    launched_at: Instant,
    /// The bounded tail of its stderr
    stderr: Arc<Mutex<StderrTail>>,
    /// The thread draining its stderr
    stderr_reader: Option<JoinHandle<()>>,
}

impl ClientWatch {
    /// Gives the stderr reader a moment to finish, so the lines the client
    /// wrote just before it exited are in the tail.
    fn drain_stderr(&self) {
        let Some(reader) = self.stderr_reader.as_ref() else {
            return;
        };
        let deadline = Instant::now() + STDERR_DRAIN_TIMEOUT;
        while !reader.is_finished() && Instant::now() < deadline {
            thread::sleep(STDERR_DRAIN_POLL);
        }
    }
}

/// Collects the client's exit, if it has exited since the last look.
fn reap_exited_client(
    shared: &Arc<Mutex<FreeRdpSharedState>>,
    watch: &ClientWatch,
) -> Option<EmbeddedClientExit> {
    let status = {
        let mut state = lock_or_recover(shared);
        let status = match state.process.as_mut()?.try_wait() {
            Ok(Some(status)) => status,
            Ok(None) => return None,
            Err(error) => {
                tracing::debug!(
                    protocol = "rdp",
                    %error,
                    "Could not poll the embedded FreeRDP client"
                );
                return None;
            }
        };
        state.process = None;
        state.state = FreeRdpThreadState::Idle;
        status
    };
    let running_for = watch.launched_at.elapsed();
    watch.drain_stderr();
    let stderr_tail = lock_or_recover(&watch.stderr).joined();
    Some(EmbeddedClientExit {
        status,
        running_for,
        stderr_tail,
    })
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
        // The client this thread launched, while it runs.
        let mut watch: Option<ClientWatch> = None;

        loop {
            let command = match cmd_rx.recv_timeout(CLIENT_EXIT_POLL_INTERVAL) {
                Ok(command) => command,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // The client going away is an event too. Nothing used to
                    // look, so a client that exited right after launch left the
                    // session showing "Connected" over nothing (issue #351).
                    if let Some(exit) = watch
                        .as_ref()
                        .and_then(|running| reap_exited_client(&shared, running))
                    {
                        watch = None;
                        let _ = evt_tx.send(RdpEvent::ClientExited(exit));
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    lock_or_recover(&shared).cleanup_process();
                    break;
                }
            };

            match command {
                RdpCommand::Connect(config) => {
                    lock_or_recover(&shared).state = FreeRdpThreadState::Connecting;
                    current_config = Some(*config.clone());

                    match Self::launch_freerdp(&config, &shared) {
                        Ok(launched) => {
                            watch = Some(launched);
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
                RdpCommand::Disconnect => {
                    watch = None;
                    let mut s = lock_or_recover(&shared);
                    s.cleanup_process();
                    s.state = FreeRdpThreadState::Idle;
                    drop(s);
                    let _ = evt_tx.send(RdpEvent::Disconnected);
                }
                RdpCommand::KeyEvent {
                    keyval: _,
                    pressed: _,
                } => {
                    // Forward keyboard event to FreeRDP process
                }
                RdpCommand::MouseEvent {
                    x: _,
                    y: _,
                    button: _,
                    pressed: _,
                } => {
                    // Forward mouse event to FreeRDP process
                }
                RdpCommand::Resize { width, height } => {
                    if let Some(ref mut config) = current_config {
                        config.width = width;
                        config.height = height;
                    }
                }
                RdpCommand::SendCtrlAltDel => {
                    tracing::debug!("[FreeRDP] Ctrl+Alt+Del requested");
                }
                RdpCommand::Shutdown => {
                    let mut s = lock_or_recover(&shared);
                    s.state = FreeRdpThreadState::ShuttingDown;
                    s.cleanup_process();
                    break;
                }
            }
        }
    }

    /// Launches FreeRDP with Qt error suppression
    ///
    /// Uses mutex poisoning recovery for safe process handle storage. Returns
    /// what the thread needs to notice the client exiting and say why.
    fn launch_freerdp(
        config: &RdpConfig,
        shared: &Arc<Mutex<FreeRdpSharedState>>,
    ) -> Result<ClientWatch, EmbeddedRdpError> {
        // `wlfreerdp3`, or a `wlfreerdp` that reports FreeRDP 3. Debian and
        // Ubuntu ship FreeRDP 2 as `wlfreerdp`, which rejects the `/args-from:`
        // command line below and prints its usage banner instead of
        // connecting (issue #351). `detect_wlfreerdp` asks the same question
        // before this mode is chosen.
        let binary = match super::detect::select_embedded_wlfreerdp() {
            FreeRdpSelection::Supported(binary) => binary,
            selection => {
                tracing::info!(
                    protocol = "rdp",
                    unsupported_version = ?selection.unsupported_version(),
                    "No FreeRDP 3 Wayland client for embedded mode"
                );
                return Err(EmbeddedRdpError::WlFreeRdpNotAvailable);
            }
        };

        let mut cmd = Command::new(&binary);

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

        // Security layer and NLA, mirroring `push_security_args` on the external
        // path. Without this the embedded wlfreerdp subsurface always ran the
        // FreeRDP default (Negotiate → NLA), so a TLS-only server such as KDE
        // KRdp or an NLA-less xrdp was met with CredSSP and rejected it with
        // ERRCONNECT_LOGON_FAILURE / exit 255 (issue #366) — exactly the servers
        // this embedded fallback exists to reach. `Negotiate` emits nothing
        // (FreeRDP's own default); `Tls`/`Rdp`/`Nla` pin `/sec:…`, and
        // `disable_nla` adds `/sec:nla:off`.
        if let Some(security) = config.security_layer.freerdp_arg() {
            plain_args.push(security.to_string());
        }
        if config.disable_nla {
            plain_args.push("/sec:nla:off".to_string());
        }

        // Dynamic resolution is always requested here, unlike the external
        // client, which reads `config.dynamic_resolution` / `config.smart_sizing`
        // (issue #341). This is the embedded wlfreerdp widget: its size follows
        // the DrawingArea geometry and the Display Control Channel, so the
        // external-window sizing switches do not apply, and only `wlfreerdp3`
        // and `wlfreerdp` are candidates above because only a Wayland-native
        // client embeds as a subsurface — the SDL3 client that is now preferred
        // for external launches (#340) cannot. The one exception is a custom `/smart-sizing`
        // argument: FreeRDP refuses it beside `/dynamic-resolution`, so the
        // shared resolver leaves the switch out and the custom one stands.
        let sizing =
            rustconn_core::protocol::FreeRdpSizing::resolve(true, false, &config.extra_args);
        if let Some(flag) = sizing.flag {
            plain_args.push(flag.to_string());
        }

        if let Some(arg) = config.clipboard_direction.freerdp_arg() {
            plain_args.push(arg);
        }

        // State the audio routing explicitly — FreeRDP's implicit default is
        // "no audio at all" (issue #245). Before extra_args so a hand-written
        // override still wins.
        plain_args.push(config.audio_mode.freerdp_arg().to_string());

        // The same filter as the external launcher: a custom argument carrying a
        // secret field, or selecting a shell or proxy, is dropped here too.
        plain_args.extend(rustconn_core::protocol::filter_extra_args(
            &config.extra_args,
            sizing,
        ));

        if config.port == 3389 {
            plain_args.push(format!("/v:{}", config.host));
        } else {
            plain_args.push(format!("/v:{}:{}", config.host, config.port));
        }

        // Write all arguments (plain + secret) to the ephemeral args file; the
        // password travels in it and nowhere else. The shared helper picks the
        // `/args-from:` spelling this client's version accepts.
        let prepared_args = super::launcher::SafeFreeRdpLauncher::prepare_args_file(
            &binary,
            &plain_args,
            &secret_args,
        )?;
        cmd.arg(prepared_args.argument());

        // Nobody can answer a prompt here. With stdin at /dev/null FreeRDP
        // declines a certificate question and exits — which the exit watch then
        // reports — instead of blocking on the terminal RustConn started from.
        cmd.stdin(Stdio::null());

        // Keep stderr instead of discarding it: when the client exits right
        // after launch it is the only record of why. The tail drops the Qt noise
        // the old /dev/null redirect was there to hide.
        cmd.stderr(Stdio::piped());

        match cmd.spawn() {
            Ok(mut child) => {
                // FreeRDP opens the args file after exec, not before `spawn`
                // returns, so the file has to outlive this call for a moment;
                // dropping it here raced the client's own argument parsing.
                prepared_args.retain_for_post_spawn_parse();
                let stderr = Arc::new(Mutex::new(StderrTail::new(EMBEDDED_STDERR_TAIL_BYTES)));
                let stderr_reader = child
                    .stderr
                    .take()
                    .map(|pipe| spawn_stderr_reader(pipe, Arc::clone(&stderr)));
                lock_or_recover(shared).process = Some(child);
                Ok(ClientWatch {
                    launched_at: Instant::now(),
                    stderr,
                    stderr_reader,
                })
            }
            // `prepared_args` drops here and removes the file at once.
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

#[cfg(test)]
mod client_watch_tests {
    use std::time::Duration;

    use super::{
        EMBEDDED_EARLY_EXIT_WINDOW, EMBEDDED_STDERR_TAIL_BYTES, StderrTail,
        embedded_exit_is_failure, tail_excerpt,
    };

    #[test]
    fn the_tail_keeps_only_the_last_lines() {
        let mut tail = StderrTail::new(10);
        for line in ["aaaa", "bbbb", "cccc"] {
            tail.push_line(line);
        }
        assert_eq!(tail.joined(), "bbbb\ncccc");
    }

    /// What the old `/dev/null` redirect existed to hide goes; FreeRDP's own
    /// error line, which says why the client exited, stays.
    #[test]
    fn the_tail_drops_qt_noise_and_blank_lines() {
        let mut tail = StderrTail::new(EMBEDDED_STDERR_TAIL_BYTES);
        for line in [
            "qt.qpa.wayland: Wayland does not support QWindow::requestActivate()\n",
            "QSocketNotifier: Can only be used with threads started with QThread",
            "Warning: Ignoring XDG_SESSION_TYPE=wayland on Gnome. \
             Use QT_QPA_PLATFORM=wayland to run on Wayland anyway.",
            "QObject::connect: invalid nullptr parameter",
            "",
            "   \r\n",
            "[ERROR][com.freerdp.core] - ERRCONNECT_LOGON_FAILURE [0x00020014]\r\n",
        ] {
            tail.push_line(line);
        }
        assert_eq!(
            tail.joined(),
            "[ERROR][com.freerdp.core] - ERRCONNECT_LOGON_FAILURE [0x00020014]"
        );
    }

    #[test]
    fn a_line_quoting_a_password_switch_is_not_kept() {
        let mut tail = StderrTail::new(EMBEDDED_STDERR_TAIL_BYTES);
        tail.push_line("Failed at index 3 [/p:hunter2]: Unexpected keyword");
        tail.push_line("Failed at index 4 [/GP:hunter2]: Unexpected keyword");
        assert_eq!(tail.joined(), "");
    }

    #[test]
    fn a_runaway_line_is_cut_at_a_character_boundary() {
        let mut tail = StderrTail::new(EMBEDDED_STDERR_TAIL_BYTES);
        tail.push_line(&"€".repeat(1000));
        let kept = tail.joined();
        assert!(
            !kept.is_empty() && kept.len() <= 1024,
            "kept {} bytes",
            kept.len()
        );
        assert!(kept.chars().all(|c| c == '€'));
    }

    #[test]
    fn the_log_excerpt_is_the_end_of_the_tail() {
        assert_eq!(tail_excerpt("abcdef", 3), "def");
        assert_eq!(tail_excerpt("abc", 10), "abc");
        // A cut inside a character moves forward to the next one.
        assert_eq!(tail_excerpt("aéb", 2), "b");
    }

    /// An exit right after launch is a failed connection; one after the window
    /// is a session that ended, which keeps showing Connected until it does.
    #[test]
    fn only_an_exit_inside_the_window_is_a_failure() {
        assert!(embedded_exit_is_failure(Duration::ZERO));
        assert!(embedded_exit_is_failure(Duration::from_millis(400)));
        assert!(!embedded_exit_is_failure(EMBEDDED_EARLY_EXIT_WINDOW));
        assert!(!embedded_exit_is_failure(Duration::from_hours(1)));
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
        assert_eq!(t.begin_batch(std::path::PathBuf::from("/tmp")), 2);
        let first = t.start_download(0).expect("first download starts");
        let second = t.start_download(1).expect("second download starts");
        t.update_size(first, 10);
        t.update_size(second, 10);

        assert_eq!(
            t.append_data(first, &[0u8; 10], false),
            ChunkOutcome::Complete
        );
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
        assert_eq!(
            t.append_data(sid, &[0u8; 10], false),
            ChunkOutcome::Complete
        );
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
        assert_eq!(t.begin_batch(std::path::PathBuf::from("/tmp")), 2);
        let first = t.start_download(0).expect("starts");
        let second = t.start_download(1).expect("starts");
        t.update_size(first, 10);
        let _ = t.append_data(first, &[0u8; 10], false);
        t.cancel_download(second);
        assert!((t.overall_progress() - 1.0).abs() < f64::EPSILON);
    }

    fn three_files() -> Vec<ClipboardFileInfo> {
        ["a.bin", "b.bin", "c.bin"]
            .into_iter()
            .zip(10u32..)
            .map(|(name, index)| ClipboardFileInfo::new(name.to_string(), 4, 0, 0, index))
            .collect()
    }

    /// Asking for every file at once held all of them in memory together and
    /// hit IronRDP's cap of 1000 pending requests on a large folder copy. The
    /// next file starts only when the previous one has settled.
    #[test]
    fn a_batch_fetches_one_file_at_a_time() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut t = ClipboardFileTransfer::new();
        t.set_available_files(three_files());
        assert_eq!(t.begin_batch(dir.path().to_path_buf()), 3);

        let mut server_indices = Vec::new();
        while let Some((sid, index)) = t.start_next_download() {
            assert_eq!(t.downloads.len(), 1, "only one file is in flight");
            server_indices.push(index);
            t.update_size(sid, 4);
            assert_eq!(t.append_data(sid, b"data", false), ChunkOutcome::Complete);
            t.save_download(sid).expect("the write succeeds");
        }
        assert_eq!(server_indices, vec![10, 11, 12], "the server's own indices");
        assert!(t.all_complete());
        assert_eq!(t.saved_count(), 3);
    }

    /// A saved file's bytes used to stay in memory until the next batch.
    #[test]
    fn saving_releases_the_download() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (mut t, sid) = completed_download_named(dir.path(), "notes.txt");
        t.save_download(sid).expect("the write succeeds");

        assert!(!t.downloads.contains_key(&sid), "the buffer is gone");
        assert!(t.all_complete(), "the file still counts as saved");
        assert_eq!(t.append_data(sid, b"late", false), ChunkOutcome::Unknown);
    }

    /// A file the server says is over the cap is settled on its size reply
    /// instead of being fetched up to the cap only to be thrown away.
    #[test]
    fn an_oversized_file_is_refused_on_its_size_reply() {
        use super::MAX_DOWNLOAD_BYTES;

        let mut t = ClipboardFileTransfer::new();
        t.set_available_files(three_files());
        t.begin_batch(std::path::PathBuf::from("/tmp"));
        let (sid, _) = t.start_next_download().expect("starts");

        assert!(
            !t.refuse_if_oversized(sid, MAX_DOWNLOAD_BYTES),
            "at the cap is fine"
        );
        assert!(t.refuse_if_oversized(sid, MAX_DOWNLOAD_BYTES + 1));
        assert!(!t.downloads.contains_key(&sid));
        assert_eq!(t.refused_count(), 1);
        assert!(
            !t.refuse_if_oversized(sid, MAX_DOWNLOAD_BYTES + 1),
            "a second reply for it settles nothing"
        );
    }

    /// When the remote clipboard changes mid-batch the server no longer holds
    /// the files, so nothing new is asked for; the file in flight may finish,
    /// and the rest settle as not delivered so the summary still comes.
    #[test]
    fn withdrawing_the_offer_settles_the_files_not_started() {
        let mut t = ClipboardFileTransfer::new();
        t.set_available_files(three_files());
        t.begin_batch(std::path::PathBuf::from("/tmp"));
        let (sid, _) = t.start_next_download().expect("starts");

        t.withdraw_offer();
        assert!(t.start_next_download().is_none(), "nothing new is started");
        assert_eq!(t.refused_count(), 2);
        assert!(!t.all_complete(), "the file in flight is still open");

        t.update_size(sid, 4);
        assert_eq!(t.append_data(sid, b"data", false), ChunkOutcome::Complete);
        assert!(t.all_complete());
    }

    /// The batch is built from the list offered when the folder is picked. A
    /// snapshot from the button press could count files a newer list did not
    /// have, and the batch never settled.
    #[test]
    fn a_batch_uses_the_list_offered_when_the_folder_is_picked() {
        let mut t = ClipboardFileTransfer::new();
        t.set_available_files(three_files());
        // The remote clipboard changes while the folder dialog is open.
        t.set_available_files(vec![ClipboardFileInfo::new(
            "only.bin".to_string(),
            4,
            0,
            0,
            7,
        )]);
        assert_eq!(t.begin_batch(std::path::PathBuf::from("/tmp")), 1);
        assert_eq!(t.start_next_download().map(|(_, index)| index), Some(7));
        assert!(t.start_next_download().is_none());

        // And an offer withdrawn before the folder was picked fetches nothing.
        t.withdraw_offer();
        assert_eq!(t.begin_batch(std::path::PathBuf::from("/tmp")), 0);
        assert!(t.start_next_download().is_none());
        assert!(!t.all_complete(), "an empty batch reports nothing");
    }

    // ------------------------------------------------------------------
    // Local-clipboard announce hold (issue #390)
    // ------------------------------------------------------------------

    /// From the moment the folder picker opens until the batch it starts has
    /// settled, a local clipboard change must not be announced. Covers both
    /// halves: the picker being open with no batch yet, and a batch running
    /// after the picker closed.
    #[test]
    fn a_folder_pick_holds_local_announce_until_the_batch_settles() {
        let mut t = ClipboardFileTransfer::new();
        t.set_available_files(vec![ClipboardFileInfo::new(
            "a.bin".to_string(),
            4,
            0,
            0,
            0,
        )]);

        assert!(!t.holds_local_announce(), "nothing is happening yet");

        t.begin_folder_pick();
        assert!(t.holds_local_announce(), "the picker is open");

        assert_eq!(t.begin_batch(std::path::PathBuf::from("/tmp")), 1);
        assert!(
            t.holds_local_announce(),
            "a file is in flight after the picker closed"
        );

        let (sid, _) = t.start_next_download().expect("starts");
        t.update_size(sid, 4);
        assert_eq!(t.append_data(sid, &[0u8; 4], true), ChunkOutcome::Complete);
        assert!(
            !t.holds_local_announce(),
            "the batch settled; nothing to hold for anymore"
        );
    }

    /// Dismissing or cancelling the folder dialog must release the hold the
    /// button press took, even though no batch ever started.
    #[test]
    fn a_cancelled_folder_pick_releases_the_hold() {
        let mut t = ClipboardFileTransfer::new();
        t.begin_folder_pick();
        assert!(t.holds_local_announce());

        t.cancel_folder_pick();
        assert!(
            !t.holds_local_announce(),
            "no batch started, so nothing else holds the announce"
        );
    }

    /// A file the server refuses must still release the hold once it was the
    /// only thing outstanding, the same way it settles the batch summary.
    #[test]
    fn a_refused_file_releases_the_hold() {
        let mut t = ClipboardFileTransfer::new();
        t.set_available_files(vec![ClipboardFileInfo::new(
            "a.bin".to_string(),
            4,
            0,
            0,
            0,
        )]);
        t.begin_batch(std::path::PathBuf::from("/tmp"));
        let (sid, _) = t.start_next_download().expect("starts");
        assert!(t.holds_local_announce());

        assert!(t.cancel_download(sid));
        assert!(
            !t.holds_local_announce(),
            "the refusal settled the only file in the batch"
        );
    }

    /// The remote clipboard withdrawing its file offer must release the hold
    /// once nothing is left in flight, the same as any other settlement path.
    #[test]
    fn a_withdrawn_offer_releases_the_hold_once_nothing_is_in_flight() {
        let mut t = ClipboardFileTransfer::new();
        t.set_available_files(three_files());
        t.begin_batch(std::path::PathBuf::from("/tmp"));
        let (sid, _) = t.start_next_download().expect("starts");
        assert!(t.holds_local_announce());

        t.withdraw_offer();
        assert!(
            t.holds_local_announce(),
            "the file already in flight can still finish"
        );

        t.update_size(sid, 4);
        assert_eq!(t.append_data(sid, b"data", false), ChunkOutcome::Complete);
        assert!(
            !t.holds_local_announce(),
            "nothing is left in flight and the picker is not open"
        );
    }

    /// A deferred announce fires once and only once: taking it clears it, so a
    /// second release of the hold with no new change in between does not
    /// re-announce.
    #[test]
    fn a_deferred_announce_is_taken_once() {
        let mut t = ClipboardFileTransfer::new();
        assert!(!t.take_deferred_announce(), "nothing was deferred yet");

        t.defer_announce();
        assert!(t.take_deferred_announce());
        assert!(
            !t.take_deferred_announce(),
            "taking it must clear it, not just read it"
        );
    }

    /// `clear()` must drop both the fold-pick hold and a pending deferral, or a
    /// widget reused for a new connection would start out already holding.
    #[test]
    fn clear_drops_the_hold_and_the_deferral() {
        let mut t = ClipboardFileTransfer::new();
        t.begin_folder_pick();
        t.defer_announce();

        t.clear();

        assert!(!t.holds_local_announce());
        assert!(!t.take_deferred_announce());
    }

    /// `connect_ironrdp` resets this state at the start of every embedded
    /// session (capture `folder_pick_pending`, `clear()`, restore it) so that
    /// a batch a server-dropped session cut off mid-download does not keep
    /// holding the local-clipboard announce forever once the Reconnect
    /// button, a network-change reconnect, a cert-accept reconnect or the
    /// IronRDP-error fallback starts the next session — none of which route
    /// through `disconnect()` (issue #390, review finding: hold surviving
    /// reconnect).
    #[test]
    fn a_session_start_reset_drops_a_batch_hold_left_by_the_old_session() {
        let (mut t, _sid) = transfer_with_file(10);
        assert!(
            t.holds_local_announce(),
            "an in-flight download holds the announce"
        );

        let folder_pick_pending = t.folder_pick_pending;
        t.clear();
        t.folder_pick_pending = folder_pick_pending;

        assert!(
            !t.holds_local_announce(),
            "the reconnected session must not inherit the old batch's hold"
        );
    }

    /// The same reset must leave an *open* folder picker's hold in place: its
    /// callback holds the same `Rc` and is the only thing that can resolve
    /// it, and it is the flag protecting the window between focus returning
    /// from the picker and that callback running — exactly the window a
    /// reconnect must not reopen (issue #390, review finding: hold surviving
    /// reconnect — the safer option named in the fix).
    #[test]
    fn a_session_start_reset_keeps_an_open_folder_picks_hold() {
        let mut t = ClipboardFileTransfer::new();
        t.begin_folder_pick();

        let folder_pick_pending = t.folder_pick_pending;
        t.clear();
        t.folder_pick_pending = folder_pick_pending;

        assert!(
            t.holds_local_announce(),
            "an open picker's hold must survive a reconnect"
        );
    }

    /// `drop_deferred_announce` discards the flag outright: unlike
    /// `take_deferred_announce`, nothing is meant to be sent afterwards.
    #[test]
    fn drop_deferred_announce_discards_it_without_sending() {
        let mut t = ClipboardFileTransfer::new();
        t.defer_announce();

        t.drop_deferred_announce();

        assert!(
            !t.take_deferred_announce(),
            "the deferral was discarded, not merely read"
        );
    }

    /// A new remote file list is a newer clipboard arriving from the other
    /// direction: a local change deferred against an earlier, now-superseded
    /// batch must not survive to be announced later and override it.
    ///
    /// Before this, `set_available_files` only reset the batch counters, so
    /// `announce_deferred` passed through it untouched and fired at the end
    /// of some later, unrelated hold (issue #390, review finding: deferral
    /// outliving a superseded batch).
    #[test]
    fn a_new_file_list_drops_a_deferral_left_over_from_the_superseded_batch() {
        let mut t = ClipboardFileTransfer::new();
        t.set_available_files(three_files());
        t.begin_batch(std::path::PathBuf::from("/tmp"));
        // A local change happens while this batch is still running.
        t.defer_announce();

        // The remote clipboard changes again before the batch settled.
        t.set_available_files(vec![ClipboardFileInfo::new(
            "new.bin".to_string(),
            4,
            0,
            0,
            0,
        )]);

        assert!(
            !t.take_deferred_announce(),
            "the stale deferral must not survive the new list"
        );
    }

    /// The remote clipboard withdrawing its file offer (e.g. a remote text
    /// copy) is the same kind of "newer clipboard from the other side" as a
    /// new file list, and must drop a stale deferral the same way.
    #[test]
    fn withdrawing_the_offer_drops_a_pending_deferral() {
        let mut t = ClipboardFileTransfer::new();
        t.set_available_files(three_files());
        t.begin_batch(std::path::PathBuf::from("/tmp"));
        t.defer_announce();

        t.withdraw_offer();

        assert!(!t.take_deferred_announce());
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
        assert_eq!(
            t.append_data(sid, &[1u8; 10], false),
            ChunkOutcome::Complete
        );
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
        t.begin_batch(std::path::PathBuf::from("/tmp"));
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
        assert_eq!(
            t.append_data(first, &[0u8; 1], false),
            ChunkOutcome::Unknown
        );
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

        assert_eq!(t.begin_batch(std::path::PathBuf::from("/tmp")), 1);
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

    /// The name is the server's and the user never sees it, so a leading dot
    /// would plant a hidden file — `.bash_profile`, `.zshenv` — that the next
    /// login shell runs. `create_new` only refuses to overwrite one.
    #[test]
    fn a_leading_dot_is_stripped_so_no_hidden_file_is_created() {
        for (raw, expected) in [
            (".bash_profile", "bash_profile"),
            (r"..\..\.zshenv", "zshenv"),
            ("...hidden", "hidden"),
            (" . .profile", "profile"),
            ("report.v2.txt", "report.v2.txt"),
        ] {
            assert_eq!(
                super::sanitized_file_name(raw).as_deref(),
                Some(expected),
                "{raw:?}"
            );
        }
    }

    /// Control and bidirectional-override characters change what a file
    /// manager shows without being visible: `U+202E` makes `fdp.exe` read as
    /// `exe.pdf`.
    #[test]
    fn control_and_bidi_characters_are_removed() {
        for (raw, expected) in [
            ("\u{202E}fdp.exe", "fdp.exe"),
            ("in\u{2066}voice\u{2069}.pdf", "invoice.pdf"),
            ("line\nbreak.txt", "linebreak.txt"),
            ("tab\there\u{7f}.log", "tabhere.log"),
        ] {
            assert_eq!(
                super::sanitized_file_name(raw).as_deref(),
                Some(expected),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn names_with_no_usable_component_are_refused() {
        for raw in [
            "",
            "   ",
            ".",
            "..",
            "...",
            ". .",
            "\u{202E}",
            "a/b/",
            "with\0nul",
            "sub/..",
        ] {
            assert!(
                super::sanitized_file_name(raw).is_none(),
                "{raw:?} must be refused"
            );
        }
    }

    #[test]
    fn the_collision_suffix_goes_before_the_extension() {
        assert_eq!(super::suffixed_name("notes.txt", 1), "notes (1).txt");
        assert_eq!(
            super::suffixed_name("archive.tar.gz", 2),
            "archive.tar (2).gz"
        );
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
        t.set_available_files(vec![ClipboardFileInfo::new(name.to_string(), 4, 0, 0, 0)]);
        t.begin_batch(dir.to_path_buf());
        let sid = t.start_download(0).expect("starts");
        t.update_size(sid, 4);
        assert_eq!(t.append_data(sid, b"data", false), ChunkOutcome::Complete);
        (t, sid)
    }

    #[test]
    fn a_download_is_written_into_the_chosen_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (mut t, sid) = completed_download_named(dir.path(), "notes.txt");

        let path = t.save_download(sid).expect("the write succeeds");
        assert_eq!(path, dir.path().join("notes.txt"));
        assert_eq!(std::fs::read(&path).expect("readable"), b"data");
    }

    /// End to end: a server offering `.bash_profile` gets a visible file.
    #[test]
    fn a_server_dotfile_is_saved_as_a_visible_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (mut t, sid) = completed_download_named(dir.path(), ".bash_profile");

        let path = t.save_download(sid).expect("the write succeeds");
        assert_eq!(path, dir.path().join("bash_profile"));
        assert!(!dir.path().join(".bash_profile").exists());
    }

    /// The security case: an absolute name from the server must not escape the
    /// directory the user picked.
    #[test]
    fn an_absolute_server_name_cannot_escape_the_chosen_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let escape = dir.path().join("outside.txt");
        let (mut t, sid) = completed_download_named(
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
        let (mut t, sid) = completed_download_named(&inner, "../escaped.txt");

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
        let (mut t, sid) = completed_download_named(dir.path(), "..");

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

        let (mut t, sid) = completed_download_named(dir.path(), "notes.txt");
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
        t.begin_batch(dir.path().to_path_buf());
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
