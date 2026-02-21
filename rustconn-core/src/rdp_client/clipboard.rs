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

use super::{ClipboardFileInfo, ClipboardFormatInfo, RdpClientEvent};
use ironrdp::cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy, CliprdrBackend};
use ironrdp::cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsFlags,
    FileContentsRequest, FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
};
use ironrdp::core::impl_as_any;
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use tracing::{debug, trace, warn};

/// Proxy for sending clipboard messages to the main event loop
#[derive(Clone, Debug)]
pub struct RustConnClipboardProxy {
    event_tx: Sender<RdpClientEvent>,
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

    /// Clears all pending copy data
    pub fn clear_pending_copy_data(&mut self) {
        self.pending_copy_data.clear();
    }

    /// Returns the server's negotiated capabilities
    #[must_use]
    pub const fn server_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        self.server_capabilities
    }

    /// Returns true if the server supports file clipboard
    #[must_use]
    pub const fn supports_file_clipboard(&self) -> bool {
        self.server_capabilities
            .contains(ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES)
    }
}

impl CliprdrBackend for RustConnClipboardBackend {
    #[allow(clippy::unnecessary_literal_bound)]
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
        ClipboardGeneralCapabilityFlags::empty()
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

        // Notify GUI about the file contents request
        // The GUI should respond with the file data via ClipboardData command
        let is_size_request = request.flags.contains(FileContentsFlags::SIZE);

        let _ = self
            .proxy
            .event_tx
            .send(RdpClientEvent::ServerMessage(format!(
                "Server requested file contents: index={}, {}",
                request.index,
                if is_size_request { "size" } else { "data" }
            )));
    }

    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        let stream_id = response.stream_id();
        let data = response.data();

        debug!(
            "File contents response: stream_id={}, data_len={}",
            stream_id,
            data.len()
        );

        // Check if this is a size response (8 bytes = u64 file size)
        if data.len() == 8 {
            let size = u64::from_le_bytes([
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ]);
            debug!("File size response: stream_id={}, size={}", stream_id, size);
            let _ = self
                .proxy
                .event_tx
                .send(RdpClientEvent::ClipboardFileSize { stream_id, size });
        } else {
            // This is file data
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
                    is_last: true, // For now, assume single chunk
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
        .chunks_exact(2)
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
}
