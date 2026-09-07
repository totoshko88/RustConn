//! RDP Clipboard backend implementation
//!
//! This module implements the `CliprdrBackend` trait from `IronRDP`
//! to handle clipboard operations between client and server.
//!
//! # Bidirectional Clipboard Support
//!
//! The clipboard supports both directions:
//! - Server → Client: `on_remote_copy` → `on_format_data_response` → `ClipboardText` event
//! - Client → Server: `ClipboardCopy` command → `on_format_data_request` → `ClipboardData` command
//!
//! # Supported Formats
//!
//! - `CF_UNICODETEXT` (13): Unicode text (UTF-16LE)
//! - `CF_TEXT` (1): ANSI text
//! - `CF_DIB` (8): Device-independent bitmap (future)
//! - `CF_HDROP` (15): File list (future)

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::Sender;

use ironrdp::cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy, CliprdrBackend};
use ironrdp::cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsFlags,
    FileContentsRequest, FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
};
use ironrdp::core::impl_as_any;
use tracing::{debug, trace, warn};

use super::{ClipboardFileInfo, ClipboardFormatInfo, RdpClientEvent};

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
                // File transfer clipboard operations — not yet implemented.
                // IronRDP 0.15 adds file contents PDUs for clipboard file copy.
                trace!("Clipboard file contents message received (not implemented)");
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
}

impl_as_any!(RustConnClipboardBackend);

impl RustConnClipboardBackend {
    /// Creates a new clipboard backend
    #[must_use]
    pub fn new(event_tx: Sender<RdpClientEvent>) -> Self {
        Self {
            proxy: RustConnClipboardProxy::new(event_tx),
            ready: false,
            pending_paste_format: None,
            pending_copy_data: HashMap::new(),
            server_capabilities: ClipboardGeneralCapabilityFlags::empty(),
            local_file_paths: Vec::new(),
            pending_size_requests: HashSet::new(),
        }
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

        // Notify GUI about available formats (for UI display)
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
            self.pending_paste_format = Some(format.id);
            self.proxy
                .send_clipboard_message(ClipboardMessage::SendInitiatePaste(format.id));
        } else {
            debug!("No text format available in clipboard");
        }
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        let format_id = request.format.value();
        debug!("Server requested clipboard data for format {}", format_id);

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
        let data = response.data();
        let format_id = self.pending_paste_format.take();
        debug!(
            "on_format_data_response called: {} bytes, format: {:?}",
            data.len(),
            format_id
        );

        // Check for file list format (CF_HDROP = 15 or FileGroupDescriptorW)
        if format_id == Some(ClipboardFormatId::CF_HDROP)
            && let Some(files) = parse_file_group_descriptor(data)
        {
            debug!("Parsed {} files from clipboard", files.len());
            let _ = self
                .proxy
                .event_tx
                .send(RdpClientEvent::ClipboardFileList(files));
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

/// Parses a `FileGroupDescriptorW` structure from clipboard data
///
/// The structure format (MS-RDPECLIP 2.2.5.2.3.1):
/// - `cItems` (4 bytes): Number of `FILEDESCRIPTORW` entries
/// - `FILEDESCRIPTORW[]`: Array of file descriptors
///
/// Each `FILEDESCRIPTORW` (MS-RDPECLIP 2.2.5.2.3.1.1):
/// - `dwFlags` (4 bytes): Valid fields flags
/// - `clsid` (16 bytes): Reserved
/// - `sizelLow/High` (8 bytes): Reserved
/// - `pointl` (8 bytes): Reserved
/// - `dwFileAttributes` (4 bytes): File attributes
/// - `ftCreationTime` (8 bytes): Creation time
/// - `ftLastAccessTime` (8 bytes): Last access time
/// - `ftLastWriteTime` (8 bytes): Last write time
/// - `nFileSizeHigh` (4 bytes): High 32 bits of file size
/// - `nFileSizeLow` (4 bytes): Low 32 bits of file size
/// - `cFileName` (520 bytes): Null-terminated UTF-16LE filename (260 chars)
fn parse_file_group_descriptor(data: &[u8]) -> Option<Vec<ClipboardFileInfo>> {
    /// Size of each `FILEDESCRIPTORW` structure in bytes
    const FILEDESCRIPTOR_SIZE: usize = 592;

    // Minimum size: 4 bytes for count
    if data.len() < 4 {
        return None;
    }

    let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let expected_size = 4 + count * FILEDESCRIPTOR_SIZE;

    if data.len() < expected_size {
        tracing::warn!(
            "FileGroupDescriptorW too small: expected {}, got {}",
            expected_size,
            data.len()
        );
        return None;
    }

    let mut files = Vec::with_capacity(count);
    let mut offset = 4;

    for index in 0..count {
        if offset + FILEDESCRIPTOR_SIZE > data.len() {
            break;
        }

        let descriptor = &data[offset..offset + FILEDESCRIPTOR_SIZE];

        // dwFlags at offset 0
        let _flags =
            u32::from_le_bytes([descriptor[0], descriptor[1], descriptor[2], descriptor[3]]);

        // dwFileAttributes at offset 36
        let attributes = u32::from_le_bytes([
            descriptor[36],
            descriptor[37],
            descriptor[38],
            descriptor[39],
        ]);

        // ftLastWriteTime at offset 60 (FILETIME = 8 bytes)
        let last_write_time = i64::from_le_bytes([
            descriptor[60],
            descriptor[61],
            descriptor[62],
            descriptor[63],
            descriptor[64],
            descriptor[65],
            descriptor[66],
            descriptor[67],
        ]);

        // nFileSizeHigh at offset 68, nFileSizeLow at offset 72
        let size_high = u32::from_le_bytes([
            descriptor[68],
            descriptor[69],
            descriptor[70],
            descriptor[71],
        ]);
        let size_low = u32::from_le_bytes([
            descriptor[72],
            descriptor[73],
            descriptor[74],
            descriptor[75],
        ]);
        let size = (u64::from(size_high) << 32) | u64::from(size_low);

        // cFileName at offset 76 (520 bytes = 260 UTF-16 chars)
        let filename_bytes = &descriptor[76..76 + 520];
        let filename = string_from_utf16(filename_bytes).unwrap_or_default();

        if !filename.is_empty() {
            files.push(ClipboardFileInfo::new(
                filename,
                size,
                attributes,
                last_write_time,
                index as u32,
            ));
        }

        offset += FILEDESCRIPTOR_SIZE;
    }

    if files.is_empty() { None } else { Some(files) }
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
}
