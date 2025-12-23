//! RDP Clipboard backend implementation
//!
//! This module implements the `CliprdrBackend` trait from `IronRDP`
//! to handle clipboard operations between client and server.

use super::{ClipboardFormatInfo, RdpClientEvent};
use ironrdp::cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy, CliprdrBackend};
use ironrdp::cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest,
    FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
    OwnedFormatDataResponse,
};
use ironrdp::core::impl_as_any;
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
                let format_infos: Vec<ClipboardFormatInfo> = formats
                    .iter()
                    .map(|f| {
                        let name = f.name.as_ref().map(|n| format!("{n:?}"));
                        ClipboardFormatInfo::new(f.id.value(), name)
                    })
                    .collect();
                let _ = self
                    .event_tx
                    .send(RdpClientEvent::ClipboardFormatsAvailable(format_infos));
            }
            ClipboardMessage::SendInitiatePaste(format_id) => {
                // Request to fetch data from server - send as event to trigger initiate_paste
                let format_info = ClipboardFormatInfo::new(format_id.value(), None);
                let _ = self
                    .event_tx
                    .send(RdpClientEvent::ClipboardPasteRequest(format_info));
            }
            ClipboardMessage::SendFormatData(response) => {
                let data = response.data();
                if let Ok(text) = string_from_utf16(data) {
                    let _ = self.event_tx.send(RdpClientEvent::ClipboardText(text));
                }
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
}

impl_as_any!(RustConnClipboardBackend);

impl RustConnClipboardBackend {
    /// Creates a new clipboard backend
    #[must_use]
    pub const fn new(event_tx: Sender<RdpClientEvent>) -> Self {
        Self {
            proxy: RustConnClipboardProxy::new(event_tx),
            ready: false,
            pending_paste_format: None,
        }
    }

    /// Returns true if the clipboard is ready
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
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
        debug!("Server requested format list");
        // Server wants to know what formats we have available
        // For now, we don't have any local clipboard data to offer
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::empty()
    }

    fn on_process_negotiated_capabilities(
        &mut self,
        capabilities: ClipboardGeneralCapabilityFlags,
    ) {
        trace!(?capabilities, "Negotiated clipboard capabilities");
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        trace!(?available_formats, "Remote copy - formats available");

        // Notify GUI about available formats
        self.proxy
            .send_clipboard_message(ClipboardMessage::SendInitiateCopy(
                available_formats.to_vec(),
            ));

        // Auto-request text data if available (CF_UNICODETEXT preferred, then CF_TEXT)
        let text_format = available_formats
            .iter()
            .find(|f| f.id == ClipboardFormatId::CF_UNICODETEXT)
            .or_else(|| {
                available_formats
                    .iter()
                    .find(|f| f.id == ClipboardFormatId::CF_TEXT)
            });

        if let Some(format) = text_format {
            debug!("Auto-requesting clipboard text in format {:?}", format.id);
            self.pending_paste_format = Some(format.id);
            // Request the data - this will trigger on_format_data_response when server responds
            self.proxy
                .send_clipboard_message(ClipboardMessage::SendInitiatePaste(format.id));
        }
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        trace!(?request, "Format data request from server");
        self.proxy
            .send_clipboard_message(ClipboardMessage::SendInitiatePaste(request.format));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        trace!("Format data response received");

        let data = response.data();

        if let Some(format_id) = self.pending_paste_format.take() {
            match format_id {
                ClipboardFormatId::CF_UNICODETEXT => {
                    if let Ok(text) = string_from_utf16(data) {
                        let owned = OwnedFormatDataResponse::new_data(text.into_bytes());
                        self.proxy
                            .send_clipboard_message(ClipboardMessage::SendFormatData(owned));
                    } else {
                        warn!("Failed to decode Unicode clipboard text");
                    }
                }
                ClipboardFormatId::CF_TEXT => {
                    if let Ok(text) = String::from_utf8(data.to_vec()) {
                        let owned = OwnedFormatDataResponse::new_data(text.into_bytes());
                        self.proxy
                            .send_clipboard_message(ClipboardMessage::SendFormatData(owned));
                    }
                }
                _ => {
                    let owned = OwnedFormatDataResponse::new_data(data.to_vec());
                    self.proxy
                        .send_clipboard_message(ClipboardMessage::SendFormatData(owned));
                }
            }
        } else {
            let owned = OwnedFormatDataResponse::new_data(data.to_vec());
            self.proxy
                .send_clipboard_message(ClipboardMessage::SendFormatData(owned));
        }
    }

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        debug!(?request, "File contents request (not implemented)");
    }

    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        debug!(?response, "File contents response (not implemented)");
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
    let mut result: Vec<u8> = text
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
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
}
