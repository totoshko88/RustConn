//! RDP Clipboard backend implementation
//!
//! This module implements the `CliprdrBackend` trait from `IronRDP`
//! to handle clipboard operations between client and server.
//!
//! # Bidirectional Clipboard Support
//!
//! The clipboard supports both directions:
//! - Server → Client: `on_remote_copy` → `on_format_data_response` → `ClipboardText` event
//! - Server → Client files: `on_remote_copy` → `on_remote_file_list` → `ClipboardFileList` event
//! - Client → Server: `ClipboardCopy` command → `on_format_data_request` → `ClipboardData` command
//!
//! Each direction is gated by [`RdpClipboardDirection`]: the remote→local
//! direction is gated in `on_remote_copy` (the gate owner), and the local→remote
//! direction is gated in the three `commands.rs` clipboard arms
//! (`ClipboardText`, `ClipboardCopy`, `AnnounceClipboardFormats`), with
//! `on_format_data_request` as a backstop that never serves local data when
//! local→remote is disabled.
//!
//! # Supported Formats
//!
//! - `CF_UNICODETEXT` (13): Unicode text (UTF-16LE)
//! - `CF_TEXT` (1): ANSI text
//! - `CF_DIB` (8): Device-independent bitmap (future)
//! - `FileGroupDescriptorW` (registered, matched by name): file list, parsed by `IronRDP`

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Sender;

use ironrdp::cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy, CliprdrBackend};
use ironrdp::cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags,
    FileContentsFlags, FileContentsRequest, FileContentsResponse, FileDescriptor,
    FormatDataRequest, FormatDataResponse, LockDataId,
};
use ironrdp::core::impl_as_any;
use tracing::{debug, trace, warn};

use super::{ClipboardFileInfo, ClipboardFormatInfo, RdpClientEvent};
use crate::models::RdpClipboardDirection;

/// Proxy for sending clipboard messages to the main event loop
#[derive(Clone, Debug)]
pub struct RustConnClipboardProxy {
    pub(crate) event_tx: Sender<RdpClientEvent>,
}

impl RustConnClipboardProxy {
    /// Creates a new clipboard proxy
    #[must_use]
    pub const fn new(event_tx: Sender<RdpClientEvent>) -> Self {
        Self { event_tx }
    }
}

impl ClipboardMessageProxy for RustConnClipboardProxy {
    fn send_clipboard_message(&self, message: ClipboardMessage) {
        match message {
            ClipboardMessage::SendInitiateCopy(formats) => {
                // Backend wants to send format list to server (initiate copy)
                let format_infos: Vec<ClipboardFormatInfo> = formats
                    .iter()
                    .map(|f| {
                        let name = f.name.as_ref().map(|n| format!("{n:?}"));
                        ClipboardFormatInfo::new(f.id.value(), name)
                    })
                    .collect();
                trace!("Sending ClipboardCopy with {} formats", format_infos.len());
                let _ = self
                    .event_tx
                    .send(RdpClientEvent::ClipboardInitiateCopy(format_infos));
            }
            ClipboardMessage::SendInitiatePaste(format_id) => {
                trace!("Requesting clipboard data for format {}", format_id.value());
                let format_info = ClipboardFormatInfo::new(format_id.value(), None);
                let _ = self
                    .event_tx
                    .send(RdpClientEvent::ClipboardPasteRequest(format_info));
            }
            ClipboardMessage::SendFormatData(response) => {
                // This is called when IronRDP wants us to send data to server
                // But we also use it to extract received data
                let data = response.data();
                trace!(
                    "SendFormatData called with {} bytes (this is for sending TO server)",
                    data.len()
                );
            }
            ClipboardMessage::Error(err) => {
                warn!("Clipboard error: {}", err);
            }
            ClipboardMessage::SendFileContentsRequest(_)
            | ClipboardMessage::SendFileContentsResponse(_) => {
                // Unused proxy path. File-contents requests and responses are
                // handled directly on the `CliprdrBackend` impl below
                // (`on_file_contents_request` / `on_file_contents_response`),
                // which is where ironrdp actually delivers them; nothing emits
                // these `ClipboardMessage` variants, so this arm never fires.
                trace!("Clipboard file contents message received on unused proxy path");
            }
            ClipboardMessage::SendInitiateFileCopy(_file_descriptors) => {
                // ironrdp 0.17: backend signals that a local file list is ready
                // to be offered to the remote via CLIPRDR file copy. We already
                // handle file copy via StoreLocalFiles command path, so this is
                // a no-op for now.
                trace!("Clipboard SendInitiateFileCopy received (handled via StoreLocalFiles)");
            }
        }
    }
}

/// `RustConn` clipboard backend for `IronRDP`
#[derive(Debug)]
pub struct RustConnClipboardBackend {
    proxy: RustConnClipboardProxy,
    ready: bool,
    pending_paste_format: Option<ClipboardFormatId>,
    /// Pending data to send to server (`format_id` -> data)
    pending_copy_data: HashMap<u32, Vec<u8>>,
    /// Server's negotiated capabilities
    server_capabilities: ClipboardGeneralCapabilityFlags,
    /// Local file paths for client → server file transfer (DnD).
    /// Indexed by file_index as announced in `FileGroupDescriptorW`.
    local_file_paths: Vec<std::path::PathBuf>,
    /// Stream IDs of our own outstanding download requests that asked for a file
    /// *size* rather than data. The wire response carries no flag telling the two
    /// apart, so the request type is what disambiguates them — see
    /// [`Self::expect_size_response`].
    pending_size_requests: HashSet<u32>,
    /// The id the server gave `FileGroupDescriptorW` in its current format list.
    ///
    /// A registered format is matched by name and its id holds only until the
    /// next format list. Kept so a reply to our file-list request that `IronRDP`
    /// hands back unparsed is recognised as a broken descriptor, not as text.
    remote_file_list_format: Option<ClipboardFormatId>,
    /// Whether the file list still has to be requested once the text reply lands.
    ///
    /// CLIPRDR matches a Format Data Response to the one request outstanding, so
    /// a format list offering both text and files is fetched one at a time: text
    /// first, as it always was, then the file list.
    file_list_after_text: bool,
    /// File sizes from the server's current file list, by file index.
    ///
    /// `IronRDP` refuses a RANGE request that reaches past the size it parsed from
    /// the same list — see [`Self::range_request_length`].
    remote_file_sizes: Vec<Option<u64>>,
    /// Format Data Requests asked for the current format list and not answered.
    outstanding_requests: u32,
    /// Replies still due for requests made for a superseded format list.
    ///
    /// A Format Data Response names no format; CLIPRDR matches it to the
    /// requests in the order they were sent. When the server announces a new
    /// format list, `IronRDP` forgets what it was asked, so a late reply to the
    /// old file-list request reached `on_format_data_response` as an ordinary
    /// reply and was decoded as text — binary descriptor bytes put on the local
    /// clipboard. Replies are dropped while this is non-zero.
    stale_replies: u32,
    /// Which way the clipboard is allowed to synchronise. Gates the two announce
    /// directions: remote→local in [`Self::on_remote_copy`], local→remote in the
    /// `commands.rs` clipboard arms with [`Self::on_format_data_request`] as a
    /// backstop.
    direction: RdpClipboardDirection,
}

impl_as_any!(RustConnClipboardBackend);

impl RustConnClipboardBackend {
    /// Creates a new clipboard backend with bidirectional sync.
    ///
    /// Equivalent to [`Self::new_with_direction`] with
    /// [`RdpClipboardDirection::Both`] — the historical always-bidirectional
    /// behaviour.
    #[must_use]
    pub fn new(event_tx: Sender<RdpClientEvent>) -> Self {
        Self::new_with_direction(event_tx, RdpClipboardDirection::Both)
    }

    /// Creates a new clipboard backend honouring `direction`.
    #[must_use]
    pub fn new_with_direction(
        event_tx: Sender<RdpClientEvent>,
        direction: RdpClipboardDirection,
    ) -> Self {
        Self {
            proxy: RustConnClipboardProxy::new(event_tx),
            ready: false,
            pending_paste_format: None,
            pending_copy_data: HashMap::new(),
            server_capabilities: ClipboardGeneralCapabilityFlags::empty(),
            local_file_paths: Vec::new(),
            pending_size_requests: HashSet::new(),
            remote_file_list_format: None,
            file_list_after_text: false,
            remote_file_sizes: Vec::new(),
            outstanding_requests: 0,
            stale_replies: 0,
            direction,
        }
    }

    /// Whether the remote clipboard may flow into the local one.
    #[must_use]
    pub const fn allows_remote_to_local(&self) -> bool {
        self.direction.allows_remote_to_local()
    }

    /// Whether the local clipboard may flow to the remote.
    #[must_use]
    pub const fn allows_local_to_remote(&self) -> bool {
        self.direction.allows_local_to_remote()
    }

    /// Returns true if the clipboard is ready
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
    }

    /// Sets pending copy data for a format
    ///
    /// This should be called when the GUI has clipboard data ready to send.
    /// The data will be sent when the server requests it via `on_format_data_request`.
    pub fn set_pending_copy_data(&mut self, format_id: u32, data: Vec<u8>) {
        debug!(
            "Setting pending copy data for format {}: {} bytes",
            format_id,
            data.len()
        );
        self.pending_copy_data.insert(format_id, data);
    }

    /// Drops the parked payload for `format_id`, if any.
    ///
    /// Used when announcing that the local clipboard has new content in a
    /// format whose data will be supplied on demand. Whatever was parked for
    /// that format belongs to the previous clipboard owner, and
    /// [`Self::on_format_data_request`] answers from `pending_copy_data`
    /// before it asks the GUI — so without this the server would be served
    /// stale text (issue #261). Only the announced format is cleared — the
    /// file-clipboard entries parked by `StoreLocalFiles` are left alone.
    pub fn clear_pending_format(&mut self, format_id: u32) {
        if self.pending_copy_data.remove(&format_id).is_some() {
            debug!("Dropped stale pending copy data for format {format_id}");
        }
    }

    /// Sets the local file paths for client → server file transfer.
    ///
    /// Called by the GUI when files are dropped onto the RDP widget.
    /// The paths are indexed by position matching the `FileGroupDescriptorW`
    /// announcement order.
    pub fn set_local_file_paths(&mut self, paths: Vec<std::path::PathBuf>) {
        debug!("Storing {} local file paths for DnD transfer", paths.len());
        self.local_file_paths = paths;
    }

    /// Returns the stored local file paths (for external access).
    #[must_use]
    pub fn local_file_paths(&self) -> &[std::path::PathBuf] {
        &self.local_file_paths
    }

    /// Records that the download request on `stream_id` asked for a file size.
    ///
    /// Called from the session loop when it emits a SIZE File Contents Request,
    /// so [`Self::on_file_contents_response`] can classify the reply by what was
    /// asked rather than by guessing from the payload length — an 8-byte *data*
    /// chunk is otherwise indistinguishable from an 8-byte size field.
    pub fn expect_size_response(&mut self, stream_id: u32) {
        self.pending_size_requests.insert(stream_id);
    }

    /// Returns whether `stream_id` was a size request, consuming the record.
    ///
    /// Consuming it keeps the set bounded to genuinely outstanding size requests
    /// and means a stream id reused for a later data request is not mistaken for
    /// a size request.
    fn take_size_expectation(&mut self, stream_id: u32) -> bool {
        self.pending_size_requests.remove(&stream_id)
    }

    /// Reports that a download request never reached the server.
    ///
    /// When the session loop cannot build or submit the File Contents Request —
    /// the file clipboard was not negotiated, the CLIPRDR channel is gone — no
    /// size or data reply will ever arrive for `stream_id`. Without this the
    /// batch's "Save N Files" button sits on "Downloading…" forever for that
    /// file. Clears any size expectation first so a reused id starts clean, then
    /// raises the same [`RdpClientEvent::ClipboardFileError`] a server refusal
    /// does, which the GUI already handles by dropping the download and freeing
    /// the button.
    pub fn emit_download_failed(&mut self, stream_id: u32) {
        self.pending_size_requests.remove(&stream_id);
        let _ = self
            .proxy
            .event_tx
            .send(RdpClientEvent::ClipboardFileError { stream_id });
    }

    /// Trims a RANGE download request to the bytes the server's file list says remain.
    ///
    /// `IronRDP` refuses a RANGE request that reaches past the size it parsed from
    /// the same file list, and the download loop asks in fixed 1 MiB slices. So
    /// untrimmed, the last slice of every file never went out — and for a file
    /// under 1 MiB that is the only slice. Returns `requested` when the size is
    /// unknown, and 0 when nothing is left to fetch.
    #[must_use]
    pub fn range_request_length(&self, file_index: u32, offset: u64, requested: u32) -> u32 {
        let Some(Some(size)) = self.remote_file_sizes.get(file_index as usize) else {
            return requested;
        };
        let remaining = size.saturating_sub(offset);
        u32::try_from(remaining).map_or(requested, |remaining| remaining.min(requested))
    }

    /// Reports the end of a download that has nothing left to fetch.
    ///
    /// A RANGE request past the last byte cannot go out, since `IronRDP` refuses a
    /// zero-length range, yet the download loop is waiting for a reply. An empty
    /// chunk is what a server answers at the end of a file, and the loop already
    /// treats it as the end — so a zero-byte file completes the same way.
    pub fn emit_end_of_file(&self, stream_id: u32) {
        let _ = self
            .proxy
            .event_tx
            .send(RdpClientEvent::ClipboardFileContents {
                stream_id,
                data: Vec::new(),
            });
    }

    /// Asks the server for its clipboard data in `format`.
    ///
    /// Recorded in `pending_paste_format`, so the reply is decoded as what was
    /// asked for.
    fn request_remote_format(&mut self, format: ClipboardFormatId) {
        self.pending_paste_format = Some(format);
        self.outstanding_requests = self.outstanding_requests.saturating_add(1);
        self.proxy
            .send_clipboard_message(ClipboardMessage::SendInitiatePaste(format));
    }

    /// Records that a Format Data Request this backend asked for never went out.
    ///
    /// The session loop calls it when `initiate_paste` or the write fails. No
    /// reply will come, so the request must not be counted as outstanding —
    /// otherwise the next format list would expect a stale reply and drop a
    /// genuine one. A file list that was to follow a failed text request is
    /// asked for now, since its turn would never come.
    pub fn paste_request_not_sent(&mut self) {
        self.outstanding_requests = self.outstanding_requests.saturating_sub(1);
        self.pending_paste_format = None;
        if std::mem::take(&mut self.file_list_after_text)
            && let Some(format) = self.remote_file_list_format
        {
            self.request_remote_format(format);
        }
    }

    /// Consumes the next reply's slot, returning `false` when it answers a
    /// request made for a superseded format list and must be dropped.
    fn take_reply_slot(&mut self) -> bool {
        if 0 < self.stale_replies {
            self.stale_replies -= 1;
            debug!("Dropping a clipboard reply to a request for a superseded format list");
            return false;
        }
        self.outstanding_requests = self.outstanding_requests.saturating_sub(1);
        true
    }

    /// Returns the server's negotiated capabilities
    #[must_use]
    pub const fn server_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        self.server_capabilities
    }

    /// Returns true if the peer negotiated stream-based file copy.
    ///
    /// Checks `STREAM_FILECLIP_ENABLED`, the flag MS-RDPECLIP 2.2.2.1 actually
    /// gates File Contents Request/Response on. This used to test
    /// `USE_LONG_FORMAT_NAMES`, which says nothing about file transfer and so
    /// reported support on servers that had none.
    #[must_use]
    pub const fn supports_file_clipboard(&self) -> bool {
        self.server_capabilities
            .contains(ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED)
    }
}

impl CliprdrBackend for RustConnClipboardBackend {
    #[expect(
        clippy::unnecessary_literal_bound,
        reason = "explicit 'static bound documents that the trait object outlives all callers, even when the type system can infer it"
    )]
    fn temporary_directory(&self) -> &str {
        ".cliprdr"
    }

    fn on_ready(&mut self) {
        debug!("Clipboard channel ready");
        self.ready = true;
    }

    fn on_request_format_list(&mut self) {
        trace!("Server requested format list - sending empty list to complete initialization");
        // Send an empty format list to complete the initialization handshake
        self.proxy
            .send_clipboard_message(ClipboardMessage::SendInitiateCopy(Vec::new()));
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        // USE_LONG_FORMAT_NAMES: the server sends full format names, which the
        // file clipboard needs (`FileGroupDescriptorW` is a registered format,
        // reachable only by name) and which some Windows Server 2016+ builds
        // require before they announce a format list at all.
        //
        // STREAM_FILECLIP_ENABLED: without it the peer never issues a File
        // Contents Request, so copying or dragging a file into the session
        // silently produced nothing (issue #256). MS-RDPECLIP 2.2.2.1 gates
        // stream-based file copy on this flag.
        //
        // FILECLIP_NO_FILE_PATHS: we describe files by name only and never
        // hand out local paths, which is what this flag promises the peer.
        // FreeRDP advertises the same trio.
        ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
            | ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS
    }

    fn on_process_negotiated_capabilities(
        &mut self,
        capabilities: ClipboardGeneralCapabilityFlags,
    ) {
        trace!(?capabilities, "Negotiated clipboard capabilities");
        self.server_capabilities = capabilities;

        // Log useful capability info
        if capabilities.contains(ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES) {
            debug!("Server supports long format names (file clipboard possible)");
        }
        if capabilities.contains(ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED) {
            debug!("Server supports file stream clipboard");
        } else {
            // Server does not support file clipboard — notify GUI to disable file DnD
            debug!("Server does NOT support file stream clipboard — disabling file DnD");
            let _ = self
                .proxy
                .event_tx
                .send(RdpClientEvent::FileClipboardUnsupported);
        }
        if capabilities.contains(ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS) {
            debug!("Server prefers file clipboard without paths");
        }
        if capabilities.contains(ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA) {
            debug!("Server supports clipboard data locking");
        }
        if capabilities.contains(ClipboardGeneralCapabilityFlags::HUGE_FILE_SUPPORT_ENABLED) {
            debug!("Server supports huge file transfers");
        }
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        debug!(
            "on_remote_copy called with {} formats: {:?}",
            available_formats.len(),
            available_formats
                .iter()
                .map(|f| f.id.value())
                .collect::<Vec<_>>()
        );

        // Notify GUI about available formats (for UI display), unless the
        // direction suppresses remote→local — advertising formats the user
        // cannot paste is pointless.
        if self.allows_remote_to_local() {
            let format_infos: Vec<ClipboardFormatInfo> = available_formats
                .iter()
                .map(|f| {
                    let name = f.name.as_ref().map(|n| format!("{n:?}"));
                    ClipboardFormatInfo::new(f.id.value(), name)
                })
                .collect();
            let _ = self
                .proxy
                .event_tx
                .send(RdpClientEvent::ClipboardFormatsAvailable(format_infos));
        }

        // A new format list replaces everything the server offered before, so the
        // previous file offer is withdrawn here. Otherwise "Save N Files" stays up
        // after the remote user copies text, and pressing it asks for files the
        // server no longer holds. A list that does carry files offers them again
        // once their descriptor arrives, in `on_remote_file_list`.
        self.remote_file_sizes.clear();
        self.file_list_after_text = false;
        // Whatever was asked for the previous list is still owed a reply, which
        // must not be read as an answer about this one.
        self.stale_replies = self
            .stale_replies
            .saturating_add(std::mem::take(&mut self.outstanding_requests));
        self.pending_paste_format = None;
        self.remote_file_list_format = available_formats
            .iter()
            .find(|f| f.name.as_ref() == Some(&ClipboardFormatName::FILE_LIST))
            .map(|f| f.id);
        let _ = self
            .proxy
            .event_tx
            .send(RdpClientEvent::ClipboardFileList(Vec::new()));

        // Remote→local disabled: the bookkeeping above still runs so internal
        // state stays consistent, but nothing is pulled from the remote.
        if !self.allows_remote_to_local() {
            trace!("clipboard remote→local suppressed by direction");
            return;
        }

        // Check if text format is available and auto-request it
        let text_format = available_formats
            .iter()
            .find(|f| f.id == ClipboardFormatId::CF_UNICODETEXT)
            .or_else(|| {
                available_formats
                    .iter()
                    .find(|f| f.id == ClipboardFormatId::CF_TEXT)
            });

        if let Some(format) = text_format {
            debug!(
                "Text format available (id={}), requesting paste",
                format.id.value()
            );
            self.file_list_after_text = self.remote_file_list_format.is_some();
            self.request_remote_format(format.id);
        } else if let Some(format) = self.remote_file_list_format {
            // IronRDP only records the file-list format (MS-RDPECLIP delayed
            // rendering) and leaves asking for it to the backend. Nothing asked,
            // so the descriptor never arrived and "Save N Files" never appeared.
            debug!("File list available (id={}), requesting it", format.value());
            self.request_remote_format(format);
        } else {
            debug!("No text or file list available in clipboard");
        }
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        let format_id = request.format.value();
        debug!("Server requested clipboard data for format {}", format_id);

        // Local→remote disabled: never serve local clipboard data to the server,
        // even if a command arm forgot to gate the announce (defence in depth).
        if !self.allows_local_to_remote() {
            trace!("clipboard local→remote data request suppressed by direction");
            return;
        }

        // Check if we have pending data for this format
        if let Some(data) = self.pending_copy_data.get(&format_id) {
            debug!(
                "Sending {} bytes of pending data for format {}",
                data.len(),
                format_id
            );
            // Data is ready, send it via the proxy
            // The actual sending happens through the command channel
            let _ = self
                .proxy
                .event_tx
                .send(RdpClientEvent::ClipboardDataReady {
                    format_id,
                    data: data.clone(),
                });
        } else {
            // Request data from GUI
            debug!(
                "No pending data for format {}, requesting from GUI",
                format_id
            );
            let format_info = ClipboardFormatInfo::new(format_id, None);
            let _ = self
                .proxy
                .event_tx
                .send(RdpClientEvent::ClipboardDataRequest(format_info));
        }
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        if !self.take_reply_slot() {
            return;
        }
        let data = response.data();
        let format_id = self.pending_paste_format.take();
        debug!(
            "on_format_data_response called: {} bytes, format: {:?}",
            data.len(),
            format_id
        );

        // The text request went first; with its reply in, the file list the same
        // format list offered is next. Asked even when the text was refused, as
        // the two are independent.
        if std::mem::take(&mut self.file_list_after_text)
            && let Some(format) = self.remote_file_list_format
        {
            self.request_remote_format(format);
        }

        // A refusal carries no data. Decoding its empty payload as text would
        // blank the local clipboard.
        if response.is_error() {
            debug!(?format_id, "Server refused the clipboard data request");
            return;
        }

        // IronRDP consumes a well-formed reply to the file-list request itself
        // (see `on_remote_file_list`) and forwards only one it could not parse.
        // That is a broken descriptor, not text for the local clipboard.
        if format_id.is_some() && format_id == self.remote_file_list_format {
            warn!("Server sent a clipboard file list that could not be parsed");
            return;
        }

        match format_id {
            Some(ClipboardFormatId::CF_UNICODETEXT) | None => {
                if let Ok(text) = string_from_utf16(data) {
                    debug!("Clipboard text decoded (UTF-16): {} chars", text.len());
                    let _ = self
                        .proxy
                        .event_tx
                        .send(RdpClientEvent::ClipboardText(text));
                } else {
                    warn!("Failed to decode clipboard data as UTF-16");
                }
            }
            Some(ClipboardFormatId::CF_TEXT) => {
                if let Ok(text) = String::from_utf8(data.to_vec()) {
                    debug!("Clipboard text decoded (ANSI): {} chars", text.len());
                    let _ = self
                        .proxy
                        .event_tx
                        .send(RdpClientEvent::ClipboardText(text));
                } else {
                    let text: String = data.iter().map(|&b| b as char).collect();
                    debug!("Clipboard text decoded (Latin-1): {} chars", text.len());
                    let _ = self
                        .proxy
                        .event_tx
                        .send(RdpClientEvent::ClipboardText(text));
                }
            }
            Some(_) => {
                if let Ok(text) = string_from_utf16(data) {
                    debug!("Clipboard text decoded (auto): {} chars", text.len());
                    let _ = self
                        .proxy
                        .event_tx
                        .send(RdpClientEvent::ClipboardText(text));
                } else if let Ok(text) = String::from_utf8(data.to_vec()) {
                    debug!("Clipboard text decoded (UTF-8): {} chars", text.len());
                    let _ = self
                        .proxy
                        .event_tx
                        .send(RdpClientEvent::ClipboardText(text));
                } else {
                    warn!("Failed to decode clipboard data");
                }
            }
        }
    }

    fn on_remote_file_list(&mut self, files: &[FileDescriptor], _clip_data_id: Option<u32>) {
        // IronRDP parsed the reply to our file-list request itself (and sanitised
        // the names), so `on_format_data_response` never sees it. The request is
        // settled here instead — unless the reply answers a superseded list,
        // whose files the server no longer holds.
        if !self.take_reply_slot() {
            return;
        }
        self.pending_paste_format = None;

        // A new file list supersedes the previous batch. A size expectation still
        // parked from it is consumed by stream id, and a stale one would classify a
        // later data chunk as a size reply. Drop them first, so even an empty list
        // supersedes the old batch.
        if !self.pending_size_requests.is_empty() {
            debug!(
                "Dropping {} stale size expectation(s) on a new file list",
                self.pending_size_requests.len()
            );
            self.pending_size_requests.clear();
        }

        self.remote_file_sizes = files.iter().map(|file| file.file_size).collect();
        // The index is the position in the server's list: it is what a File
        // Contents Request names, and what IronRDP checks the request against.
        let infos: Vec<ClipboardFileInfo> = files
            .iter()
            .enumerate()
            .filter_map(|(index, file)| {
                Some(ClipboardFileInfo::new(
                    file.name.clone(),
                    file.file_size.unwrap_or(0),
                    file.attributes.map_or(0, |attributes| attributes.bits()),
                    file.last_write_time
                        .and_then(|time| i64::try_from(time).ok())
                        .unwrap_or(0),
                    u32::try_from(index).ok()?,
                ))
            })
            .collect();
        debug!("Received {} files from the remote clipboard", infos.len());
        let _ = self
            .proxy
            .event_tx
            .send(RdpClientEvent::ClipboardFileList(infos));
    }

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        debug!(
            ?request,
            "File contents request: stream_id={}, index={}, flags={:?}",
            request.stream_id,
            request.index,
            request.flags
        );

        let is_size_request = request.flags.contains(FileContentsFlags::SIZE);

        let _ = self
            .proxy
            .event_tx
            .send(RdpClientEvent::FileContentsRequested {
                stream_id: request.stream_id,
                file_index: request.index as u32,
                is_size_request,
                offset: request.position,
                requested_size: request.requested_size,
            });
    }

    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        let stream_id = response.stream_id();

        // A rejected request carries the fail flag and no usable payload. The
        // download waiting on this stream must be told, or it hangs forever;
        // clear any size expectation so a later reuse of the id starts clean.
        if response.is_error() {
            self.pending_size_requests.remove(&stream_id);
            warn!("File contents request rejected by server: stream_id={stream_id}");
            let _ = self
                .proxy
                .event_tx
                .send(RdpClientEvent::ClipboardFileError { stream_id });
            return;
        }

        let data = response.data();
        debug!(
            "File contents response: stream_id={}, data_len={}",
            stream_id,
            data.len()
        );

        // A size response and an 8-byte data chunk look identical on the wire —
        // both are eight bytes. What tells them apart is which kind of request we
        // sent, tracked in `pending_size_requests`, not the length (the old
        // `data.len() == 8` guess corrupted any 8-byte file).
        if self.take_size_expectation(stream_id) {
            match response.data_as_size() {
                Ok(size) => {
                    debug!("File size response: stream_id={stream_id}, size={size}");
                    let _ = self
                        .proxy
                        .event_tx
                        .send(RdpClientEvent::ClipboardFileSize { stream_id, size });
                }
                Err(e) => {
                    warn!("Malformed file size response on stream_id={stream_id}: {e}");
                    let _ = self
                        .proxy
                        .event_tx
                        .send(RdpClientEvent::ClipboardFileError { stream_id });
                }
            }
        } else {
            debug!(
                "File data response: stream_id={}, bytes={}",
                stream_id,
                data.len()
            );
            let _ = self
                .proxy
                .event_tx
                .send(RdpClientEvent::ClipboardFileContents {
                    stream_id,
                    data: data.to_vec(),
                });
        }
    }

    fn on_lock(&mut self, data_id: LockDataId) {
        debug!(?data_id, "Clipboard lock");
    }

    fn on_unlock(&mut self, data_id: LockDataId) {
        debug!(?data_id, "Clipboard unlock");
    }
}

/// Converts UTF-16LE bytes to a Rust String
fn string_from_utf16(data: &[u8]) -> Result<String, std::string::FromUtf16Error> {
    let u16_data: Vec<u16> = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .take_while(|&c| c != 0)
        .collect();

    String::from_utf16(&u16_data)
}

/// Converts a Rust String to UTF-16LE bytes with null terminator
#[must_use]
pub fn string_to_utf16(text: &str) -> Vec<u8> {
    let mut result: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    result.extend_from_slice(&[0, 0]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_from_utf16() {
        let data = [
            0x48, 0x00, // H
            0x65, 0x00, // e
            0x6C, 0x00, // l
            0x6C, 0x00, // l
            0x6F, 0x00, // o
            0x00, 0x00, // null
        ];
        let result = string_from_utf16(&data).unwrap();
        assert_eq!(result, "Hello");
    }

    #[test]
    fn test_string_to_utf16() {
        let text = "Hi";
        let result = string_to_utf16(text);
        assert_eq!(result, vec![0x48, 0x00, 0x69, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_clipboard_format_info() {
        let format = ClipboardFormatInfo::unicode_text();
        assert!(format.is_text());
        assert_eq!(format.id, ClipboardFormatInfo::UNICODE_TEXT);
    }

    #[test]
    fn direction_predicates() {
        use RdpClipboardDirection::{Both, LocalToRemote, Off, RemoteToLocal};
        let (tx, _rx) = std::sync::mpsc::channel();

        let off = RustConnClipboardBackend::new_with_direction(tx.clone(), Off);
        assert!(!off.allows_remote_to_local() && !off.allows_local_to_remote());

        let r2l = RustConnClipboardBackend::new_with_direction(tx.clone(), RemoteToLocal);
        assert!(r2l.allows_remote_to_local() && !r2l.allows_local_to_remote());

        let l2r = RustConnClipboardBackend::new_with_direction(tx.clone(), LocalToRemote);
        assert!(!l2r.allows_remote_to_local() && l2r.allows_local_to_remote());

        let both = RustConnClipboardBackend::new_with_direction(tx, Both);
        assert!(both.allows_remote_to_local() && both.allows_local_to_remote());
    }

    /// With remote→local disabled, a remote copy announcement must not advertise
    /// formats to the GUI nor request the remote data.
    #[test]
    fn remote_to_local_suppresses_remote_copy_request() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut backend =
            RustConnClipboardBackend::new_with_direction(tx, RdpClipboardDirection::LocalToRemote);

        backend.on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]);

        // The only events emitted are the bookkeeping file-list clear; no
        // ClipboardFormatsAvailable (the remote→local advertisement).
        let saw_formats_available = std::iter::from_fn(|| rx.try_recv().ok())
            .any(|e| matches!(e, RdpClientEvent::ClipboardFormatsAvailable(_)));
        assert!(
            !saw_formats_available,
            "remote→local disabled must not advertise remote formats"
        );
        assert!(
            backend.pending_paste_format.is_none(),
            "no remote format should have been requested"
        );
    }

    /// With both directions, a remote copy announcement advertises formats and
    /// requests the text (sanity complement).
    #[test]
    fn both_requests_remote_copy() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut backend =
            RustConnClipboardBackend::new_with_direction(tx, RdpClipboardDirection::Both);

        backend.on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)]);

        let saw_formats_available = std::iter::from_fn(|| rx.try_recv().ok())
            .any(|e| matches!(e, RdpClientEvent::ClipboardFormatsAvailable(_)));
        assert!(
            saw_formats_available,
            "both directions must advertise remote formats"
        );
        assert_eq!(
            backend.pending_paste_format,
            Some(ClipboardFormatId::CF_UNICODETEXT),
            "both directions must request the remote text"
        );
    }

    /// Announcing new text must drop the parked text payload — otherwise
    /// `on_format_data_request` serves the previous clipboard owner's content —
    /// while leaving the file-clipboard entries intact (issue #261).
    #[test]
    fn clear_pending_format_only_drops_that_format() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut backend = RustConnClipboardBackend::new(tx);

        backend.set_pending_copy_data(ClipboardFormatInfo::UNICODE_TEXT, b"stale".to_vec());
        backend.set_pending_copy_data(
            ClipboardFormatInfo::FILE_GROUP_DESCRIPTOR_W,
            b"descriptor".to_vec(),
        );

        backend.clear_pending_format(ClipboardFormatInfo::UNICODE_TEXT);

        assert!(
            !backend
                .pending_copy_data
                .contains_key(&ClipboardFormatInfo::UNICODE_TEXT),
            "stale text must not survive an announcement"
        );
        assert_eq!(
            backend
                .pending_copy_data
                .get(&ClipboardFormatInfo::FILE_GROUP_DESCRIPTOR_W)
                .map(Vec::as_slice),
            Some(b"descriptor".as_slice()),
            "a pending file descriptor is unrelated to a text announcement"
        );

        // Removing an absent format is a no-op, not a panic.
        backend.clear_pending_format(ClipboardFormatInfo::UNICODE_TEXT);
    }

    /// An 8-byte reply to a SIZE request is a file size, not file data — the
    /// request type decides, not the length (the old `data.len() == 8` guess
    /// corrupted any 8-byte file).
    #[test]
    fn size_expectation_classifies_an_eight_byte_reply_as_size() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut backend = RustConnClipboardBackend::new(tx);

        backend.expect_size_response(42);
        backend.on_file_contents_response(FileContentsResponse::new_size_response(42, 4096));

        match rx.try_recv() {
            Ok(RdpClientEvent::ClipboardFileSize { stream_id, size }) => {
                assert_eq!(stream_id, 42);
                assert_eq!(size, 4096);
            }
            other => panic!("expected ClipboardFileSize, got {other:?}"),
        }
    }

    /// The same eight bytes, with no size request outstanding, are file data —
    /// a file whose contents happen to be eight bytes long.
    #[test]
    fn eight_byte_data_without_expectation_is_treated_as_data() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut backend = RustConnClipboardBackend::new(tx);

        let payload = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        backend
            .on_file_contents_response(FileContentsResponse::new_data_response(9, payload.clone()));

        match rx.try_recv() {
            Ok(RdpClientEvent::ClipboardFileContents { stream_id, data }) => {
                assert_eq!(stream_id, 9);
                assert_eq!(data, payload);
            }
            other => panic!("expected ClipboardFileContents, got {other:?}"),
        }
    }

    /// A size expectation is consumed once, so a stream id reused for a later
    /// data request is not mistaken for another size reply.
    #[test]
    fn size_expectation_is_consumed_once() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut backend = RustConnClipboardBackend::new(tx);

        backend.expect_size_response(5);
        backend.on_file_contents_response(FileContentsResponse::new_size_response(5, 16));
        // Second reply on the same id, now a data chunk.
        backend.on_file_contents_response(FileContentsResponse::new_data_response(5, vec![0u8; 8]));

        assert!(matches!(
            rx.try_recv(),
            Ok(RdpClientEvent::ClipboardFileSize { .. })
        ));
        assert!(
            matches!(
                rx.try_recv(),
                Ok(RdpClientEvent::ClipboardFileContents { .. })
            ),
            "a reused stream id must fall through to data, not size"
        );
    }

    /// A rejected request must surface as an error, never a phantom size or an
    /// empty data chunk that leaves the download waiting forever.
    #[test]
    fn error_response_emits_file_error_and_clears_expectation() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut backend = RustConnClipboardBackend::new(tx);

        backend.expect_size_response(7);
        backend.on_file_contents_response(FileContentsResponse::new_error(7));

        assert!(matches!(
            rx.try_recv(),
            Ok(RdpClientEvent::ClipboardFileError { stream_id: 7 })
        ));
        // The expectation is gone, so a later reuse of id 7 is a clean data path.
        assert!(!backend.pending_size_requests.contains(&7));
    }

    /// A download that never reached the server must surface as a file error,
    /// so the GUI stops waiting on a stream id no reply is coming for, and the
    /// size expectation is cleared so a reused id starts clean.
    #[test]
    fn emit_download_failed_reports_the_error_and_clears_the_expectation() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut backend = RustConnClipboardBackend::new(tx);

        backend.expect_size_response(3);
        backend.emit_download_failed(3);

        assert!(matches!(
            rx.try_recv(),
            Ok(RdpClientEvent::ClipboardFileError { stream_id: 3 })
        ));
        assert!(!backend.pending_size_requests.contains(&3));
    }

    /// A new file list supersedes the previous batch, so any size expectation
    /// left over from it must go: an expectation is consumed by stream id, and a
    /// stale one would misclassify a later data chunk as a size reply. The GUI
    /// now also keeps stream ids monotonic across batches, which closes the same
    /// hole from the other side — this remains the backstop, since the two sides
    /// are separate crates and only this one owns the expectation set.
    #[test]
    fn a_new_file_list_drops_stale_size_expectations() {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut backend = RustConnClipboardBackend::new(tx);

        // An outstanding size request from a previous "Save N Files" batch.
        backend.expect_size_response(1);
        assert!(backend.pending_size_requests.contains(&1));

        // The server's fresh file list, as IronRDP delivers it. Even an empty
        // list must supersede the old batch just as a populated one does.
        backend.on_remote_file_list(&[], None);

        assert!(
            backend.pending_size_requests.is_empty(),
            "a new file list must clear expectations from the superseded batch"
        );
    }

    /// The part of the session loop these tests stand in for.
    ///
    /// The backend asks for work through events and IronRDP's `Cliprdr` does the
    /// protocol, so a test that exercises only one of them cannot see the gap
    /// between them — which is where the file list got lost (issue #74). These
    /// helpers run both: PDUs are encoded as they arrive on the channel, and the
    /// commands the session loop would issue are made directly.
    mod session {
        use std::sync::mpsc::Receiver;

        use ironrdp::cliprdr::CliprdrClient;
        use ironrdp::cliprdr::pdu::{
            Capabilities, ClipboardPdu, ClipboardProtocolVersion, FormatList, FormatListResponse,
        };
        use ironrdp::svc::SvcProcessor as _;

        use super::super::*;

        /// Id this test server gives `FileGroupDescriptorW`; Windows picks one
        /// from the registered range and the client must match it by name.
        pub(super) const FILE_LIST_ID: u32 = 0xC0FE;

        /// A client through the initialization a Windows server performs, with
        /// its startup events drained.
        pub(super) fn ready_client() -> (CliprdrClient, Receiver<RdpClientEvent>) {
            let (tx, rx) = std::sync::mpsc::channel();
            let mut cliprdr: CliprdrClient =
                ironrdp::cliprdr::Cliprdr::new(Box::new(RustConnClipboardBackend::new(tx)));
            let server = Capabilities::new(
                ClipboardProtocolVersion::V2,
                ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
                    | ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
                    | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS,
            );
            feed(&mut cliprdr, &ClipboardPdu::Capabilities(server));
            feed(&mut cliprdr, &ClipboardPdu::MonitorReady);
            // The session loop answers the backend's initial empty format list.
            cliprdr.initiate_copy(&[]).expect("initial format list");
            feed(
                &mut cliprdr,
                &ClipboardPdu::FormatListResponse(FormatListResponse::Ok),
            );
            drain(&rx);
            (cliprdr, rx)
        }

        /// Hands `pdu` to the client exactly as it would arrive on the channel.
        pub(super) fn feed(cliprdr: &mut CliprdrClient, pdu: &ClipboardPdu<'_>) {
            let bytes = ironrdp::core::encode_vec(pdu).expect("encode PDU");
            cliprdr.process(&bytes).expect("client accepts PDU");
        }

        /// The server takes clipboard ownership with `formats`.
        pub(super) fn announce(cliprdr: &mut CliprdrClient, formats: &[ClipboardFormat]) {
            let list = FormatList::new_unicode(formats, true).expect("format list");
            feed(cliprdr, &ClipboardPdu::FormatList(list));
        }

        /// What Windows Explorer announces for Ctrl+C on a file, minus the
        /// shell formats a client never asks for.
        pub(super) fn file_copy_formats() -> Vec<ClipboardFormat> {
            vec![
                ClipboardFormat::new(ClipboardFormatId::new(0xC0FD))
                    .with_name(ClipboardFormatName::new("Shell IDList Array")),
                ClipboardFormat::new(ClipboardFormatId::new(FILE_LIST_ID))
                    .with_name(ClipboardFormatName::FILE_LIST),
                ClipboardFormat::new(ClipboardFormatId::new(0xC0FF))
                    .with_name(ClipboardFormatName::new("FileContents")),
            ]
        }

        /// Every event the backend has raised since the last drain.
        pub(super) fn drain(rx: &Receiver<RdpClientEvent>) -> Vec<RdpClientEvent> {
            rx.try_iter().collect()
        }

        /// The formats the backend asked the session loop to fetch, in order.
        pub(super) fn requested(events: &[RdpClientEvent]) -> Vec<u32> {
            events
                .iter()
                .filter_map(|event| match event {
                    RdpClientEvent::ClipboardPasteRequest(format) => Some(format.id),
                    _ => None,
                })
                .collect()
        }

        /// The session loop's answer to a paste request: send it to the server.
        pub(super) fn fetch(cliprdr: &mut CliprdrClient, format_id: u32) {
            cliprdr
                .initiate_paste(ClipboardFormatId::new(format_id))
                .expect("format data request");
        }

        /// The server answers the file-list request with `files`.
        pub(super) fn reply_file_list(cliprdr: &mut CliprdrClient, files: Vec<FileDescriptor>) {
            use ironrdp::cliprdr::pdu::PackedFileList;
            let reply = FormatDataResponse::new_file_list(&PackedFileList { files })
                .expect("file list reply");
            feed(cliprdr, &ClipboardPdu::FormatDataResponse(reply));
        }

        /// The non-empty file lists the backend handed the GUI.
        pub(super) fn offered(events: &[RdpClientEvent]) -> Vec<Vec<ClipboardFileInfo>> {
            events
                .iter()
                .filter_map(|event| match event {
                    RdpClientEvent::ClipboardFileList(files) if !files.is_empty() => {
                        Some(files.clone())
                    }
                    _ => None,
                })
                .collect()
        }

        /// Whether the backend withdrew the previous file offer.
        pub(super) fn withdrew(events: &[RdpClientEvent]) -> bool {
            events.iter().any(
                |event| matches!(event, RdpClientEvent::ClipboardFileList(files) if files.is_empty()),
            )
        }

        /// The text the backend handed the GUI for the local clipboard.
        pub(super) fn texts(events: &[RdpClientEvent]) -> Vec<String> {
            events
                .iter()
                .filter_map(|event| match event {
                    RdpClientEvent::ClipboardText(text) => Some(text.clone()),
                    _ => None,
                })
                .collect()
        }
    }

    /// Ctrl+C on a file in the remote session must end with "Save N Files".
    ///
    /// IronRDP leaves requesting the file list to the backend, and this backend
    /// requested only text, so the descriptor was never fetched; had it been,
    /// IronRDP delivers it through `on_remote_file_list`, which was not
    /// implemented. The button could not appear against a real server (issue #74).
    #[test]
    fn a_remote_file_copy_reaches_the_gui_as_a_file_list() {
        use ironrdp::cliprdr::pdu::ClipboardFileAttributes;

        let (mut cliprdr, rx) = session::ready_client();

        session::announce(&mut cliprdr, &session::file_copy_formats());
        let events = session::drain(&rx);
        assert_eq!(
            session::requested(&events),
            vec![session::FILE_LIST_ID],
            "the backend must ask for the file list, matched by name"
        );

        session::fetch(&mut cliprdr, session::FILE_LIST_ID);
        session::reply_file_list(
            &mut cliprdr,
            vec![
                FileDescriptor::new("serverBak.py")
                    .with_file_size(5_000)
                    .with_attributes(ClipboardFileAttributes::ARCHIVE),
            ],
        );

        let offered = session::offered(&session::drain(&rx));
        assert_eq!(offered.len(), 1, "exactly one file list reaches the GUI");
        let file = &offered[0][0];
        assert_eq!(file.name, "serverBak.py");
        assert_eq!(file.size, 5_000);
        assert_eq!(file.index, 0, "the index a File Contents Request names");
        assert!(!file.is_directory());
    }

    /// A text-only copy must keep working exactly as before the file list was
    /// fetched at all: text requested at once and delivered to the local
    /// clipboard. It also withdraws a file offer the server no longer holds.
    #[test]
    fn a_text_copy_still_fetches_text_and_withdraws_the_file_offer() {
        let (mut cliprdr, rx) = session::ready_client();

        session::announce(
            &mut cliprdr,
            &[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)],
        );
        let events = session::drain(&rx);
        assert!(session::withdrew(&events), "the previous offer must go");
        assert_eq!(
            session::requested(&events),
            vec![ClipboardFormatId::CF_UNICODETEXT.value()]
        );

        session::fetch(&mut cliprdr, ClipboardFormatId::CF_UNICODETEXT.value());
        session::feed(
            &mut cliprdr,
            &ironrdp::cliprdr::pdu::ClipboardPdu::FormatDataResponse(
                FormatDataResponse::new_unicode_string("hello"),
            ),
        );
        let events = session::drain(&rx);
        assert_eq!(session::texts(&events), vec!["hello".to_string()]);
        assert!(
            session::requested(&events).is_empty(),
            "nothing else to fetch"
        );
    }

    /// CLIPRDR matches a reply to the one request outstanding, so a list with
    /// both text and files is fetched in turn: text first, then the file list,
    /// and each reply lands where it belongs.
    #[test]
    fn a_copy_with_text_and_files_fetches_text_then_the_file_list() {
        let (mut cliprdr, rx) = session::ready_client();

        let mut formats = session::file_copy_formats();
        formats.push(ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT));
        session::announce(&mut cliprdr, &formats);
        assert_eq!(
            session::requested(&session::drain(&rx)),
            vec![ClipboardFormatId::CF_UNICODETEXT.value()],
            "text first, one request at a time"
        );

        session::fetch(&mut cliprdr, ClipboardFormatId::CF_UNICODETEXT.value());
        session::feed(
            &mut cliprdr,
            &ironrdp::cliprdr::pdu::ClipboardPdu::FormatDataResponse(
                FormatDataResponse::new_unicode_string("serverBak.py"),
            ),
        );
        let events = session::drain(&rx);
        assert_eq!(session::texts(&events), vec!["serverBak.py".to_string()]);
        assert_eq!(session::requested(&events), vec![session::FILE_LIST_ID]);

        session::fetch(&mut cliprdr, session::FILE_LIST_ID);
        session::reply_file_list(
            &mut cliprdr,
            vec![FileDescriptor::new("serverBak.py").with_file_size(5_000)],
        );
        let events = session::drain(&rx);
        assert_eq!(session::offered(&events).len(), 1);
        assert!(session::texts(&events).is_empty());
    }

    /// A refused file-list request carries no data. Decoding it as text would
    /// blank the local clipboard, so it must produce nothing at all.
    #[test]
    fn a_refused_file_list_is_not_read_as_text() {
        let (mut cliprdr, rx) = session::ready_client();

        session::announce(&mut cliprdr, &session::file_copy_formats());
        session::fetch(&mut cliprdr, session::FILE_LIST_ID);
        session::drain(&rx);
        session::feed(
            &mut cliprdr,
            &ironrdp::cliprdr::pdu::ClipboardPdu::FormatDataResponse(
                FormatDataResponse::new_error(),
            ),
        );

        let events = session::drain(&rx);
        assert!(session::texts(&events).is_empty());
        assert!(session::offered(&events).is_empty());
    }

    /// A refused text request carries no data. Decoding its empty payload as
    /// text used to blank the local clipboard.
    #[test]
    fn a_refused_text_request_leaves_the_local_clipboard_alone() {
        let (mut cliprdr, rx) = session::ready_client();

        session::announce(
            &mut cliprdr,
            &[ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)],
        );
        session::fetch(&mut cliprdr, ClipboardFormatId::CF_UNICODETEXT.value());
        session::drain(&rx);
        session::feed(
            &mut cliprdr,
            &ironrdp::cliprdr::pdu::ClipboardPdu::FormatDataResponse(
                FormatDataResponse::new_error(),
            ),
        );

        assert!(session::texts(&session::drain(&rx)).is_empty());
    }

    /// The remote clipboard changes while the file-list request is in flight.
    /// IronRDP forgets the request on the new format list and hands the late
    /// reply over as an ordinary one, which was decoded as text: descriptor
    /// bytes on the local clipboard.
    #[test]
    fn a_late_reply_for_a_superseded_list_is_not_read_as_text() {
        let (mut cliprdr, rx) = session::ready_client();

        session::announce(&mut cliprdr, &session::file_copy_formats());
        session::fetch(&mut cliprdr, session::FILE_LIST_ID);
        // An image copy: neither text nor files, so nothing new is asked for.
        session::announce(
            &mut cliprdr,
            &[ClipboardFormat::new(ClipboardFormatId::new(8))],
        );
        session::drain(&rx);
        session::reply_file_list(
            &mut cliprdr,
            vec![FileDescriptor::new("serverBak.py").with_file_size(5_000)],
        );

        let events = session::drain(&rx);
        assert!(
            session::texts(&events).is_empty(),
            "no descriptor bytes on the local clipboard"
        );
        assert!(
            session::offered(&events).is_empty(),
            "the files are no longer on offer"
        );
    }

    /// CLIPRDR answers in request order, so after a stale reply is dropped the
    /// reply for the current list is still decoded.
    #[test]
    fn the_current_reply_still_arrives_after_a_stale_one() {
        let (mut cliprdr, rx) = session::ready_client();
        let text = [ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)];

        session::announce(&mut cliprdr, &text);
        session::fetch(&mut cliprdr, ClipboardFormatId::CF_UNICODETEXT.value());
        session::announce(&mut cliprdr, &text);
        session::fetch(&mut cliprdr, ClipboardFormatId::CF_UNICODETEXT.value());
        session::drain(&rx);
        for reply in ["old", "new"] {
            session::feed(
                &mut cliprdr,
                &ironrdp::cliprdr::pdu::ClipboardPdu::FormatDataResponse(
                    FormatDataResponse::new_unicode_string(reply),
                ),
            );
        }

        assert_eq!(
            session::texts(&session::drain(&rx)),
            vec!["new".to_string()]
        );
    }

    /// A text request that never reached the server gets no reply. The file
    /// list that was to follow it is asked for at once, and the next reply is
    /// not taken for a stale one.
    #[test]
    fn a_text_request_that_never_went_out_still_leads_to_the_file_list() {
        let (mut cliprdr, rx) = session::ready_client();

        let mut formats = session::file_copy_formats();
        formats.push(ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT));
        session::announce(&mut cliprdr, &formats);
        assert_eq!(
            session::requested(&session::drain(&rx)),
            vec![ClipboardFormatId::CF_UNICODETEXT.value()]
        );

        cliprdr
            .downcast_backend_mut::<RustConnClipboardBackend>()
            .expect("backend")
            .paste_request_not_sent();
        assert_eq!(
            session::requested(&session::drain(&rx)),
            vec![session::FILE_LIST_ID]
        );

        session::fetch(&mut cliprdr, session::FILE_LIST_ID);
        session::reply_file_list(
            &mut cliprdr,
            vec![FileDescriptor::new("serverBak.py").with_file_size(5_000)],
        );
        assert_eq!(session::offered(&session::drain(&rx)).len(), 1);
    }

    /// IronRDP refuses a RANGE reaching past the file size it parsed, and the
    /// download loop asks in 1 MiB slices — so the one slice of a small file was
    /// refused before it went out. The trimmed request must pass, and at the end
    /// of the file there must be nothing left to ask for.
    #[test]
    fn range_requests_are_trimmed_to_the_remote_file_size() {
        const SLICE: u32 = 1024 * 1024;

        let (mut cliprdr, rx) = session::ready_client();
        session::announce(&mut cliprdr, &session::file_copy_formats());
        session::fetch(&mut cliprdr, session::FILE_LIST_ID);
        session::reply_file_list(
            &mut cliprdr,
            vec![FileDescriptor::new("serverBak.py").with_file_size(5_000)],
        );
        session::drain(&rx);

        let range = |stream_id, length| FileContentsRequest {
            stream_id,
            index: 0,
            flags: FileContentsFlags::RANGE,
            position: 0,
            requested_size: length,
            data_id: None,
        };
        assert!(
            cliprdr.request_file_contents(range(1, SLICE)).is_err(),
            "the untrimmed slice is what IronRDP refuses"
        );

        let backend = cliprdr
            .downcast_backend::<RustConnClipboardBackend>()
            .expect("backend");
        let trimmed = backend.range_request_length(0, 0, SLICE);
        assert_eq!(trimmed, 5_000);
        assert_eq!(backend.range_request_length(0, 4_000, SLICE), 1_000);
        assert_eq!(
            backend.range_request_length(0, 5_000, SLICE),
            0,
            "at the end"
        );
        assert_eq!(
            backend.range_request_length(7, 0, SLICE),
            SLICE,
            "an index the list does not know is left for IronRDP to judge"
        );
        assert!(cliprdr.request_file_contents(range(2, trimmed)).is_ok());
    }

    /// A zero-length range cannot be sent, so the end of a file is reported the
    /// way a server reports it: an empty chunk on the download's stream.
    #[test]
    fn end_of_file_is_an_empty_chunk() {
        let (tx, rx) = std::sync::mpsc::channel();
        let backend = RustConnClipboardBackend::new(tx);

        backend.emit_end_of_file(4);

        match rx.try_recv() {
            Ok(RdpClientEvent::ClipboardFileContents { stream_id, data }) => {
                assert_eq!(stream_id, 4);
                assert!(data.is_empty());
            }
            other => panic!("expected an empty ClipboardFileContents, got {other:?}"),
        }
    }
}
