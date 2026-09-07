//! RDP client events and commands
//!
//! This module provides event and command types for the RDP client,
//! along with conversion functions for framebuffer data.

// cast_possible_truncation allowed at workspace level
#![allow(
    clippy::cast_sign_loss,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::missing_panics_doc,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::option_if_let_else,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::redundant_clone,
    reason = "module-wide override for legacy code; refactored case by case"
)]

/// Clipboard format information for RDP clipboard operations
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardFormatInfo {
    /// Format ID (standard Windows clipboard format or custom)
    pub id: u32,
    /// Format name (for custom formats)
    pub name: Option<String>,
}

/// Audio format information for RDP audio playback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioFormatInfo {
    /// Format tag (1 = PCM, 6 = A-Law, 7 = μ-Law, etc.)
    pub format_tag: u16,
    /// Number of channels (1 = mono, 2 = stereo)
    pub channels: u16,
    /// Samples per second (e.g., 44100, 48000)
    pub samples_per_sec: u32,
    /// Average bytes per second
    pub avg_bytes_per_sec: u32,
    /// Block alignment
    pub block_align: u16,
    /// Bits per sample (8, 16, 24, 32)
    pub bits_per_sample: u16,
}

impl AudioFormatInfo {
    /// PCM format tag
    pub const FORMAT_PCM: u16 = 1;
    /// A-Law format tag
    pub const FORMAT_ALAW: u16 = 6;
    /// μ-Law format tag
    pub const FORMAT_MULAW: u16 = 7;

    /// Creates a new audio format info
    #[must_use]
    pub const fn new(
        format_tag: u16,
        channels: u16,
        samples_per_sec: u32,
        bits_per_sample: u16,
    ) -> Self {
        let block_align = channels * (bits_per_sample / 8);
        let avg_bytes_per_sec = samples_per_sec * block_align as u32;
        Self {
            format_tag,
            channels,
            samples_per_sec,
            avg_bytes_per_sec,
            block_align,
            bits_per_sample,
        }
    }

    /// Creates a standard CD-quality PCM format (44100 Hz, 16-bit, stereo)
    #[must_use]
    pub const fn cd_quality() -> Self {
        Self::new(Self::FORMAT_PCM, 2, 44100, 16)
    }

    /// Creates a DVD-quality PCM format (48000 Hz, 16-bit, stereo)
    #[must_use]
    pub const fn dvd_quality() -> Self {
        Self::new(Self::FORMAT_PCM, 2, 48000, 16)
    }

    /// Returns true if this is a PCM format
    #[must_use]
    pub const fn is_pcm(&self) -> bool {
        self.format_tag == Self::FORMAT_PCM
    }

    /// Returns the number of bytes per sample (all channels)
    #[must_use]
    pub const fn bytes_per_sample(&self) -> u16 {
        self.block_align
    }
}

/// File information for clipboard file transfers (`CF_HDROP`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardFileInfo {
    /// File name (without path)
    pub name: String,
    /// File size in bytes
    pub size: u64,
    /// File attributes (Windows file attributes)
    pub attributes: u32,
    /// Last write time (Windows FILETIME)
    pub last_write_time: i64,
    /// Index in the file list (for requesting contents)
    pub index: u32,
}

impl ClipboardFileInfo {
    /// Windows file attribute: Directory
    pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    /// Windows file attribute: Read-only
    pub const FILE_ATTRIBUTE_READONLY: u32 = 0x01;
    /// Windows file attribute: Hidden
    pub const FILE_ATTRIBUTE_HIDDEN: u32 = 0x02;

    /// Creates a new file info
    #[must_use]
    pub const fn new(
        name: String,
        size: u64,
        attributes: u32,
        last_write_time: i64,
        index: u32,
    ) -> Self {
        Self {
            name,
            size,
            attributes,
            last_write_time,
            index,
        }
    }

    /// Returns true if this is a directory
    #[must_use]
    pub const fn is_directory(&self) -> bool {
        self.attributes & Self::FILE_ATTRIBUTE_DIRECTORY != 0
    }

    /// Returns true if this file is read-only
    #[must_use]
    pub const fn is_readonly(&self) -> bool {
        self.attributes & Self::FILE_ATTRIBUTE_READONLY != 0
    }

    /// Returns true if this file is hidden
    #[must_use]
    pub const fn is_hidden(&self) -> bool {
        self.attributes & Self::FILE_ATTRIBUTE_HIDDEN != 0
    }
}

impl ClipboardFormatInfo {
    /// Standard text format (`CF_TEXT`)
    pub const TEXT: u32 = 1;
    /// Unicode text format (`CF_UNICODETEXT`)
    pub const UNICODE_TEXT: u32 = 13;
    /// HTML format
    pub const HTML: u32 = 0xC0A0;
    /// File list format (`CF_HDROP`)
    pub const FILE_LIST: u32 = 15;

    /// `FileGroupDescriptorW`, the registered format that carries the
    /// `FILEGROUPDESCRIPTORW` blob for clipboard file transfer.
    ///
    /// Registered formats live in the 0xC000–0xFFFF range and are matched by
    /// name, not by id — the id is ours to choose (MS-RDPECLIP 2.2.3.1 shows
    /// 0xC079 in its own example). Announcing this blob under `CF_HDROP` (15)
    /// instead, as we did before 0.19.12, made Windows read it as a `DROPFILES`
    /// structure and fail the paste (issue #256).
    pub const FILE_GROUP_DESCRIPTOR_W: u32 = 0xC0C4;
    /// `FileContents`, the registered format the peer uses to stream file data
    /// via File Contents Request/Response. Must be advertised alongside
    /// `FileGroupDescriptorW` or the peer never asks for the bytes.
    pub const FILE_CONTENTS: u32 = 0xC0C5;

    /// Creates a new clipboard format info
    #[must_use]
    pub const fn new(id: u32, name: Option<String>) -> Self {
        Self { id, name }
    }

    /// Creates a Unicode text format
    #[must_use]
    pub const fn unicode_text() -> Self {
        Self {
            id: Self::UNICODE_TEXT,
            name: None,
        }
    }

    /// Returns true if this is a text format
    #[must_use]
    pub const fn is_text(&self) -> bool {
        matches!(self.id, Self::TEXT | Self::UNICODE_TEXT)
    }
}

/// Rectangle coordinates for RDP operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RdpRect {
    /// X coordinate
    pub x: u16,
    /// Y coordinate
    pub y: u16,
    /// Width
    pub width: u16,
    /// Height
    pub height: u16,
}

impl RdpRect {
    /// Creates a new rectangle
    #[must_use]
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Creates a rectangle covering the full screen
    #[must_use]
    pub const fn full_screen(width: u16, height: u16) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    /// Returns the area of the rectangle in pixels
    #[must_use]
    pub const fn area(&self) -> u32 {
        self.width as u32 * self.height as u32
    }

    /// Returns true if the rectangle has valid dimensions (non-zero width and height)
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.width > 0 && self.height > 0
    }

    /// Returns true if this rectangle is within the given bounds
    #[must_use]
    pub const fn is_within_bounds(&self, max_width: u16, max_height: u16) -> bool {
        let end_x = self.x as u32 + self.width as u32;
        let end_y = self.y as u32 + self.height as u32;
        end_x <= max_width as u32 && end_y <= max_height as u32
    }
}

/// Pixel format for framebuffer data
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PixelFormat {
    /// BGRA format (blue, green, red, alpha) - native for Cairo/GTK
    #[default]
    Bgra,
    /// RGBA format (red, green, blue, alpha)
    Rgba,
    /// RGB format (red, green, blue) - no alpha, 3 bytes per pixel
    Rgb,
    /// BGR format (blue, green, red) - no alpha, 3 bytes per pixel
    Bgr,
    /// RGB565 format - 16-bit color, 2 bytes per pixel
    Rgb565,
}

impl PixelFormat {
    /// Returns the number of bytes per pixel for this format
    #[must_use]
    pub const fn bytes_per_pixel(&self) -> usize {
        match self {
            Self::Bgra | Self::Rgba => 4,
            Self::Rgb | Self::Bgr => 3,
            Self::Rgb565 => 2,
        }
    }
}

use std::borrow::Cow;

/// Converts framebuffer data from one pixel format to BGRA
///
/// This function converts pixel data from various formats to BGRA,
/// which is the native format for Cairo/GTK rendering.
///
/// For the common BGRA case, returns a zero-copy borrowed slice.
///
/// # Arguments
///
/// * `data` - Source pixel data
/// * `format` - Source pixel format
/// * `width` - Width of the image in pixels
/// * `height` - Height of the image in pixels
///
/// # Returns
///
/// BGRA pixel data, or None if conversion fails
#[must_use]
pub fn convert_to_bgra(
    data: &[u8],
    format: PixelFormat,
    width: u16,
    height: u16,
) -> Option<Cow<'_, [u8]>> {
    let pixel_count = width as usize * height as usize;
    let expected_size = pixel_count * format.bytes_per_pixel();

    if data.len() < expected_size {
        return None;
    }

    match format {
        PixelFormat::Bgra => {
            // Already in BGRA format — zero-copy borrow
            Some(Cow::Borrowed(&data[..expected_size]))
        }
        PixelFormat::Rgba => {
            // Convert RGBA to BGRA (swap R and B)
            let mut result = Vec::with_capacity(pixel_count * 4);
            for chunk in data[..expected_size].as_chunks::<4>().0 {
                result.push(chunk[2]); // B
                result.push(chunk[1]); // G
                result.push(chunk[0]); // R
                result.push(chunk[3]); // A
            }
            Some(Cow::Owned(result))
        }
        PixelFormat::Rgb => {
            // Convert RGB to BGRA (swap R and B, add alpha)
            let mut result = Vec::with_capacity(pixel_count * 4);
            for chunk in data[..expected_size].as_chunks::<3>().0 {
                result.push(chunk[2]); // B
                result.push(chunk[1]); // G
                result.push(chunk[0]); // R
                result.push(255); // A (fully opaque)
            }
            Some(Cow::Owned(result))
        }
        PixelFormat::Bgr => {
            // Convert BGR to BGRA (add alpha)
            let mut result = Vec::with_capacity(pixel_count * 4);
            for chunk in data[..expected_size].as_chunks::<3>().0 {
                result.push(chunk[0]); // B
                result.push(chunk[1]); // G
                result.push(chunk[2]); // R
                result.push(255); // A (fully opaque)
            }
            Some(Cow::Owned(result))
        }
        PixelFormat::Rgb565 => {
            // Convert RGB565 to BGRA
            let mut result = Vec::with_capacity(pixel_count * 4);
            for chunk in data[..expected_size].as_chunks::<2>().0 {
                let pixel = u16::from_le_bytes([chunk[0], chunk[1]]);
                // RGB565: RRRRRGGGGGGBBBBB
                let r = ((pixel >> 11) & 0x1F) as u8;
                let g = ((pixel >> 5) & 0x3F) as u8;
                let b = (pixel & 0x1F) as u8;
                // Scale to 8-bit
                result.push((b << 3) | (b >> 2)); // B
                result.push((g << 2) | (g >> 4)); // G
                result.push((r << 3) | (r >> 2)); // R
                result.push(255); // A
            }
            Some(Cow::Owned(result))
        }
    }
}

/// Creates a `FrameUpdate` event from raw pixel data
///
/// This is a convenience function for creating framebuffer update events
/// with proper format conversion.
///
/// # Arguments
///
/// * `x` - X coordinate of the update region
/// * `y` - Y coordinate of the update region
/// * `width` - Width of the update region
/// * `height` - Height of the update region
/// * `data` - Pixel data (must be in BGRA format)
///
/// # Returns
///
/// A `RdpClientEvent::FrameUpdate` event, or `RdpClientEvent::Error` if validation fails
#[must_use]
pub fn create_frame_update(
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    data: Vec<u8>,
) -> RdpClientEvent {
    let rect = RdpRect::new(x, y, width, height);

    // Validate data size
    let expected_size = rect.area() as usize * 4; // BGRA = 4 bytes per pixel
    if data.len() < expected_size {
        return RdpClientEvent::Error(format!(
            "Invalid framebuffer data size: expected {} bytes, got {}",
            expected_size,
            data.len()
        ));
    }

    if !rect.is_valid() {
        return RdpClientEvent::Error("Invalid rectangle dimensions".to_string());
    }

    RdpClientEvent::FrameUpdate { rect, data }
}

/// Creates a `FrameUpdate` event with format conversion
///
/// This function converts pixel data from the source format to BGRA
/// before creating the event.
///
/// # Arguments
///
/// * `x` - X coordinate of the update region
/// * `y` - Y coordinate of the update region
/// * `width` - Width of the update region
/// * `height` - Height of the update region
/// * `data` - Pixel data in source format
/// * `format` - Source pixel format
///
/// # Returns
///
/// A `RdpClientEvent::FrameUpdate` event, or `RdpClientEvent::Error` if conversion fails
#[must_use]
pub fn create_frame_update_with_conversion(
    x: u16,
    y: u16,
    width: u16,
    height: u16,
    data: &[u8],
    format: PixelFormat,
) -> RdpClientEvent {
    match convert_to_bgra(data, format, width, height) {
        Some(bgra_data) => create_frame_update(x, y, width, height, bgra_data.into_owned()),
        None => RdpClientEvent::Error(format!(
            "Failed to convert framebuffer data from {format:?} format"
        )),
    }
}

/// Events emitted by the RDP client to the GUI
#[derive(Debug, Clone)]
pub enum RdpClientEvent {
    /// Connection established successfully
    Connected {
        /// Server-negotiated width
        width: u16,
        /// Server-negotiated height
        height: u16,
    },

    /// Connection closed
    Disconnected,

    /// Resolution changed
    ResolutionChanged {
        /// New width
        width: u16,
        /// New height
        height: u16,
    },

    /// Framebuffer update (rect, BGRA pixel data)
    FrameUpdate {
        /// Rectangle being updated
        rect: RdpRect,
        /// BGRA pixel data
        data: Vec<u8>,
    },

    /// Full framebuffer update (entire screen)
    FullFrameUpdate {
        /// Screen width
        width: u16,
        /// Screen height
        height: u16,
        /// BGRA pixel data for entire screen
        data: Vec<u8>,
    },

    /// Cursor shape update
    CursorUpdate {
        /// Cursor hotspot X
        hotspot_x: u16,
        /// Cursor hotspot Y
        hotspot_y: u16,
        /// Cursor width
        width: u16,
        /// Cursor height
        height: u16,
        /// BGRA cursor image data
        data: Vec<u8>,
    },

    /// Cursor position update
    CursorPosition {
        /// X coordinate
        x: u16,
        /// Y coordinate
        y: u16,
    },

    /// Reset cursor to default
    CursorDefault,

    /// Hide cursor
    CursorHidden,

    /// Server clipboard text
    ClipboardText(String),

    /// Server clipboard data available (formats list)
    ClipboardFormatsAvailable(Vec<ClipboardFormatInfo>),

    /// Client wants to send format list to server (initiate copy)
    /// This is triggered by the backend during initialization or when local clipboard changes
    ClipboardInitiateCopy(Vec<ClipboardFormatInfo>),

    /// Server requests clipboard data from client
    ClipboardDataRequest(ClipboardFormatInfo),

    /// Clipboard data is ready to send to server (internal event)
    /// This is emitted when pending data is available for a format request
    ClipboardDataReady {
        /// Format ID
        format_id: u32,
        /// Data bytes
        data: Vec<u8>,
    },

    /// Request to fetch clipboard data from server (internal, triggers `initiate_paste`)
    ClipboardPasteRequest(ClipboardFormatInfo),

    /// File list available on server clipboard (`CF_HDROP`)
    ClipboardFileList(Vec<ClipboardFileInfo>),

    /// File contents received from server
    ClipboardFileContents {
        /// Stream ID for matching request/response
        stream_id: u32,
        /// File data for this range. Whether it is the last chunk is decided by
        /// the GUI from the known file size, not signalled here — the wire gives
        /// no reliable per-chunk "last" flag.
        data: Vec<u8>,
    },

    /// File size information received from server
    ClipboardFileSize {
        /// Stream ID for matching request/response
        stream_id: u32,
        /// File size in bytes
        size: u64,
    },

    /// The server refused a file-contents request (`CB_FILECONTENTS_RESPONSE`
    /// with the fail flag). The matching download must be abandoned rather than
    /// left waiting for data that will never arrive.
    ClipboardFileError {
        /// Stream ID of the request the server rejected
        stream_id: u32,
    },

    /// Authentication required (for NLA)
    AuthRequired,

    /// Error occurred
    Error(String),

    /// Server sent a warning/info message
    ServerMessage(String),

    // ========== Audio Events ==========
    /// Audio format changed (server selected a format)
    AudioFormatChanged(AudioFormatInfo),

    /// Audio data received from server
    AudioData {
        /// Format index (into supported formats list)
        format_index: usize,
        /// Timestamp for synchronization
        timestamp: u32,
        /// PCM audio data
        data: Vec<u8>,
    },

    /// Audio volume changed
    AudioVolume {
        /// Left channel volume (0-65535)
        left: u16,
        /// Right channel volume (0-65535)
        right: u16,
    },

    /// Audio channel closed
    AudioClose,

    /// Round-trip time measured via Echo DVC (MS-RDPEECO).
    ///
    /// Emitted periodically when the Echo virtual channel is active.
    /// The GUI can display this in the toolbar as a latency indicator.
    ///
    /// The active graphics mode is reported separately by
    /// [`Self::GraphicsModeActive`]; it used to travel with this event as a
    /// compile-time constant, which meant the status bar showed "GFX + H.264"
    /// even for sessions that never negotiated the GFX channel (issue #262).
    Rtt {
        /// Round-trip time in milliseconds
        rtt_ms: u32,
    },

    /// The graphics pipeline the server actually confirmed.
    ///
    /// Emitted once per session from the EGFX capability-confirm callback, and
    /// therefore only when the GFX channel really came up. Sessions that stay
    /// on the RemoteFX/bitmap path never emit it, which is itself the signal
    /// that no GFX pipeline is in use.
    GraphicsModeActive {
        /// Graphics pipeline derived from the confirmed EGFX capability set
        mode: super::graphics::GraphicsMode,
    },

    /// Display Control Channel (MS-RDPEDISP) is ready for dynamic resize.
    ///
    /// Emitted once the server's Display Control capabilities arrive over
    /// DRDYNVC, i.e. when `ActiveStage::encode_resize` will succeed. The GUI
    /// uses this to run the initial "snap to settled size" over Display
    /// Control instead of firing it right after connect (when the channel is
    /// not yet negotiated, which would fail over to a disruptive reconnect).
    DisplayControlReady,

    /// Display Control Channel is not available on this server.
    ///
    /// Emitted when `encode_resize` returns `None`, indicating the server
    /// does not support MS-RDPEDISP dynamic resolution changes. The GUI
    /// should fall back to a full reconnect with the new resolution.
    DisplayControlUnavailable {
        /// Requested width that could not be applied
        width: u16,
        /// Requested height that could not be applied
        height: u16,
    },

    /// Server does not support file clipboard (`STREAM_FILECLIP_ENABLED`).
    ///
    /// Emitted during capability negotiation when the server's CLIPRDR
    /// general capabilities do not include the file stream flag. The GUI
    /// should disable file drag-and-drop for this session.
    FileClipboardUnsupported,

    /// GFX H.264 pipeline persistent decode failure.
    ///
    /// Emitted once when 10+ consecutive bitmap updates deliver empty data,
    /// indicating the H.264 decoder is failing persistently. The GUI may
    /// display a degraded-quality warning or suggest reconnecting with
    /// a different graphics mode.
    ///
    /// This covers only the "decoder ran and produced nothing" shape. Content
    /// the pipeline cannot even attempt to decode is reported by
    /// [`Self::GfxUnsupportedCodec`] instead.
    GfxDecodeFailure {
        /// Number of consecutive empty frames observed
        consecutive_failures: u32,
    },

    /// The server sent surface content in a codec the GFX pipeline cannot decode.
    ///
    /// The upstream `ironrdp-egfx` client forwards such `WireToSurface1` PDUs to
    /// its catch-all callback instead of decoding them, so the pixels are lost.
    /// Before this event existed the loss was completely silent: no bitmap
    /// update ever reached the handler, so the empty-frame counter behind
    /// [`Self::GfxDecodeFailure`] stayed at zero and the session simply looked
    /// frozen (issue #262).
    GfxUnsupportedCodec {
        /// Codec the server used, as reported by `ironrdp-egfx`
        codec: String,
        /// Number of surface updates dropped because of it
        dropped_frames: u32,
    },

    /// Server requested file contents from us (client → server file transfer).
    ///
    /// Emitted by the clipboard backend when the server requests file data
    /// after we announced files via `FileGroupDescriptorW`. The session loop
    /// should read the file and respond via `submit_file_contents`.
    FileContentsRequested {
        /// Stream ID for matching request/response
        stream_id: u32,
        /// File index in the announced file list
        file_index: u32,
        /// Whether the server wants the file size (true) or data (false)
        is_size_request: bool,
        /// Byte offset for data requests
        offset: u64,
        /// Number of bytes requested (for data requests)
        requested_size: u32,
    },
}

/// Commands sent from GUI to RDP client
#[derive(Debug, Clone)]
pub enum RdpClientCommand {
    /// Disconnect from server
    Disconnect,

    /// Send keyboard event
    KeyEvent {
        /// Scancode
        scancode: u16,
        /// Key pressed (true) or released (false)
        pressed: bool,
        /// Extended key flag
        extended: bool,
    },

    /// Send Unicode character
    UnicodeEvent {
        /// Unicode character
        character: char,
        /// Key pressed (true) or released (false)
        pressed: bool,
    },

    /// Send pointer/mouse motion event (no button state change)
    PointerEvent {
        /// X coordinate
        x: u16,
        /// Y coordinate
        y: u16,
        /// Button flags (bit 0: left, bit 1: right, bit 2: middle) - current state for reference
        buttons: u8,
    },

    /// Send mouse button press event
    MouseButtonPress {
        /// X coordinate
        x: u16,
        /// Y coordinate
        y: u16,
        /// Button: 1=left, 2=right, 3=middle
        button: u8,
    },

    /// Send mouse button release event
    MouseButtonRelease {
        /// X coordinate
        x: u16,
        /// Y coordinate
        y: u16,
        /// Button: 1=left, 2=right, 3=middle
        button: u8,
    },

    /// Send mouse wheel event
    WheelEvent {
        /// Horizontal scroll (negative = left, positive = right)
        horizontal: i16,
        /// Vertical scroll (negative = down, positive = up)
        vertical: i16,
    },

    /// Send clipboard text to server
    ClipboardText(String),

    /// Send clipboard data to server (response to `ClipboardDataRequest`)
    ClipboardData {
        /// Format ID
        format_id: u32,
        /// Data bytes
        data: Vec<u8>,
    },

    /// Notify server that client clipboard has new data
    ClipboardCopy(Vec<ClipboardFormatInfo>),

    /// Announce that the local clipboard changed, without supplying the data.
    ///
    /// Unlike [`Self::ClipboardCopy`], any payload parked for the announced
    /// formats is dropped first, so the server cannot be served the previous
    /// clipboard owner's content. The data itself is fetched lazily: the peer
    /// answers with a Format Data Request, which surfaces as
    /// [`RdpClientEvent::ClipboardDataRequest`].
    ///
    /// This exists so the GUI never has to read the local selection just to
    /// announce it — on X11 that read can take down the process inside GTK
    /// (issue #261).
    AnnounceClipboardFormats(Vec<ClipboardFormatInfo>),

    /// Request clipboard data from server (triggers `initiate_paste`)
    RequestClipboardData {
        /// Format ID to request
        format_id: u32,
    },

    /// Download file contents *from* the server clipboard (server → client).
    ///
    /// Sent by the "Save N Files" button after the server announced a file list
    /// via `FileGroupDescriptorW`. The session loop turns this into a CLIPRDR
    /// File Contents *Request* PDU (`Cliprdr::request_file_contents`); the reply
    /// arrives asynchronously as [`RdpClientEvent::ClipboardFileSize`] or
    /// [`RdpClientEvent::ClipboardFileContents`].
    RequestFileContents {
        /// Stream ID for matching request/response
        stream_id: u32,
        /// File index in the file list
        file_index: u32,
        /// Request type: true = size, false = data
        request_size: bool,
        /// Offset for data requests
        offset: u64,
        /// Number of bytes to request (for data requests)
        length: u32,
    },

    /// Answer the server's file-contents request *with* our local file
    /// (client → server), for a file we announced via `FileGroupDescriptorW`.
    ///
    /// Sent by the GUI in response to [`RdpClientEvent::FileContentsRequested`].
    /// The session loop reads the local file and replies with a CLIPRDR File
    /// Contents *Response* PDU (`Cliprdr::submit_file_contents`). This is the
    /// mirror image of [`Self::RequestFileContents`]: a request answered, not a
    /// download initiated.
    ProvideFileContents {
        /// Stream ID copied from the server's request
        stream_id: u32,
        /// Index into our announced local file list
        file_index: u32,
        /// Whether the server asked for the size (true) or the data (false)
        request_size: bool,
        /// Byte offset for a data request
        offset: u64,
        /// Number of bytes the server asked for
        length: u32,
    },

    /// Request screen refresh
    RefreshScreen,

    /// Request resolution change (if server supports)
    SetDesktopSize {
        /// Desired width
        width: u16,
        /// Desired height
        height: u16,
        /// Desktop scale factor as a percentage (e.g. `200` for 200%), forwarded
        /// to the server in the MS-RDPEDISP monitor layout. `None` leaves the
        /// scale factor unset, which the server interprets as 100%.
        scale_percent: Option<u32>,
    },

    /// Send Ctrl+Alt+Del key sequence
    SendCtrlAltDel,

    /// Send a predefined key sequence for Windows admin quick actions.
    ///
    /// Each step is a `(scancode, pressed, extended)` tuple. The client
    /// inserts a small delay between steps so the remote OS can process
    /// each keystroke.
    SendKeySequence {
        /// Ordered list of `(scancode, pressed, extended)` key events
        keys: Vec<(u16, bool, bool)>,
    },

    /// Provide authentication credentials
    Authenticate {
        /// Username
        username: String,
        /// Password (stored securely, zeroized on drop)
        password: secrecy::SecretString,
        /// Domain (optional)
        domain: Option<String>,
    },

    /// Send a string as individual Unicode keystroke events with configurable delay.
    ///
    /// Bypasses keyboard layout issues by using `TS_UNICODE_KEYBOARD_EVENT` PDU.
    /// Each grapheme cluster is sent as press+release events for all its chars.
    /// This is the RDP equivalent of KeePassXC/Remmina "auto-type" feature.
    AutotypeText {
        /// Text to type (iterated by grapheme clusters via `unicode-segmentation`)
        text: String,
        /// Delay between characters in milliseconds (default: 20ms)
        inter_char_delay_ms: u32,
        /// Initial delay before typing starts in milliseconds (default: 0)
        initial_delay_ms: u32,
    },

    /// Offer dropped files to the server via CLIPRDR file copy.
    ///
    /// Called by the GUI after files are dropped onto the RDP widget. The
    /// session loop stores the local paths (so a later File Contents Request can
    /// be answered by reading them by index) and then hands the file list to
    /// IronRDP's `Cliprdr::initiate_file_copy`.
    ///
    /// The descriptor is deliberately **not** built here. IronRDP keeps its own
    /// `local_file_list` and only forwards a File Contents Request to the backend
    /// when that list is populated — which happens only through
    /// `initiate_file_copy`. Hand-building a `FILEGROUPDESCRIPTORW` and parking it
    /// under a private format id left IronRDP's list empty, so it answered the
    /// server's contents request with an error PDU before the backend ever saw
    /// it (the file appeared offered, then failed with "Unspecified error").
    InitiateFileCopy {
        /// Local file paths, in the same order as `files`. Read by index when
        /// the server requests contents.
        paths: Vec<std::path::PathBuf>,
        /// File metadata (name, size, attributes) used to build the CLIPRDR file
        /// descriptors. Order matches `paths`.
        files: Vec<ClipboardFileInfo>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rdp_rect() {
        let rect = RdpRect::new(10, 20, 100, 200);
        assert_eq!(rect.x, 10);
        assert_eq!(rect.y, 20);
        assert_eq!(rect.width, 100);
        assert_eq!(rect.height, 200);
    }

    #[test]
    fn test_full_screen_rect() {
        let rect = RdpRect::full_screen(1920, 1080);
        assert_eq!(rect.x, 0);
        assert_eq!(rect.y, 0);
        assert_eq!(rect.width, 1920);
        assert_eq!(rect.height, 1080);
    }

    #[test]
    fn test_rect_area() {
        let rect = RdpRect::new(0, 0, 100, 50);
        assert_eq!(rect.area(), 5000);
    }

    #[test]
    fn test_rect_is_valid() {
        assert!(RdpRect::new(0, 0, 100, 100).is_valid());
        assert!(!RdpRect::new(0, 0, 0, 100).is_valid());
        assert!(!RdpRect::new(0, 0, 100, 0).is_valid());
    }

    #[test]
    fn test_rect_is_within_bounds() {
        let rect = RdpRect::new(10, 10, 100, 100);
        assert!(rect.is_within_bounds(200, 200));
        assert!(rect.is_within_bounds(110, 110));
        assert!(!rect.is_within_bounds(100, 200));
        assert!(!rect.is_within_bounds(200, 100));
    }

    #[test]
    fn test_event_variants() {
        let event = RdpClientEvent::Connected {
            width: 1920,
            height: 1080,
        };
        if let RdpClientEvent::Connected { width, height } = event {
            assert_eq!(width, 1920);
            assert_eq!(height, 1080);
        }
    }

    #[test]
    fn test_command_variants() {
        let cmd = RdpClientCommand::KeyEvent {
            scancode: 0x1E,
            pressed: true,
            extended: false,
        };
        if let RdpClientCommand::KeyEvent {
            scancode,
            pressed,
            extended,
        } = cmd
        {
            assert_eq!(scancode, 0x1E);
            assert!(pressed);
            assert!(!extended);
        }
    }

    #[test]
    fn test_pixel_format_bytes_per_pixel() {
        assert_eq!(PixelFormat::Bgra.bytes_per_pixel(), 4);
        assert_eq!(PixelFormat::Rgba.bytes_per_pixel(), 4);
        assert_eq!(PixelFormat::Rgb.bytes_per_pixel(), 3);
        assert_eq!(PixelFormat::Bgr.bytes_per_pixel(), 3);
        assert_eq!(PixelFormat::Rgb565.bytes_per_pixel(), 2);
    }

    #[test]
    fn test_convert_bgra_passthrough() {
        // BGRA should pass through unchanged (zero-copy Cow::Borrowed)
        let data = vec![0, 1, 2, 3, 4, 5, 6, 7]; // 2 pixels
        let result = convert_to_bgra(&data, PixelFormat::Bgra, 2, 1);
        assert_eq!(result.as_deref(), Some(data.as_slice()));
    }

    #[test]
    fn test_convert_rgba_to_bgra() {
        // RGBA: R=255, G=128, B=64, A=200
        let rgba = vec![255, 128, 64, 200];
        let result = convert_to_bgra(&rgba, PixelFormat::Rgba, 1, 1);
        // BGRA: B=64, G=128, R=255, A=200
        assert_eq!(result.as_deref(), Some(vec![64, 128, 255, 200].as_slice()));
    }

    #[test]
    fn test_convert_rgb_to_bgra() {
        // RGB: R=255, G=128, B=64
        let rgb = vec![255, 128, 64];
        let result = convert_to_bgra(&rgb, PixelFormat::Rgb, 1, 1);
        // BGRA: B=64, G=128, R=255, A=255
        assert_eq!(result.as_deref(), Some(vec![64, 128, 255, 255].as_slice()));
    }

    #[test]
    fn test_convert_bgr_to_bgra() {
        // BGR: B=64, G=128, R=255
        let bgr = vec![64, 128, 255];
        let result = convert_to_bgra(&bgr, PixelFormat::Bgr, 1, 1);
        // BGRA: B=64, G=128, R=255, A=255
        assert_eq!(result.as_deref(), Some(vec![64, 128, 255, 255].as_slice()));
    }

    #[test]
    fn test_convert_rgb565_to_bgra() {
        // RGB565: Pure red (R=31, G=0, B=0) = 0xF800
        let rgb565 = vec![0x00, 0xF8]; // Little endian
        let result = convert_to_bgra(&rgb565, PixelFormat::Rgb565, 1, 1);
        // Should be close to BGRA: B=0, G=0, R=255, A=255
        let bgra = result.unwrap();
        assert_eq!(bgra[0], 0); // B
        assert_eq!(bgra[1], 0); // G
        assert!(bgra[2] > 240); // R (should be ~248)
        assert_eq!(bgra[3], 255); // A
    }

    #[test]
    fn test_convert_insufficient_data() {
        let data = vec![0, 1, 2]; // Only 3 bytes, need 4 for 1 BGRA pixel
        let result = convert_to_bgra(&data, PixelFormat::Bgra, 1, 1);
        assert_eq!(result, None);
    }

    #[test]
    fn test_create_frame_update_valid() {
        let data = vec![0u8; 400]; // 10x10 BGRA = 400 bytes
        let event = create_frame_update(0, 0, 10, 10, data.clone());
        if let RdpClientEvent::FrameUpdate {
            rect,
            data: event_data,
        } = event
        {
            assert_eq!(rect.x, 0);
            assert_eq!(rect.y, 0);
            assert_eq!(rect.width, 10);
            assert_eq!(rect.height, 10);
            assert_eq!(event_data.len(), 400);
        } else {
            panic!("Expected FrameUpdate event");
        }
    }

    #[test]
    fn test_create_frame_update_invalid_size() {
        let data = vec![0u8; 100]; // Too small for 10x10
        let event = create_frame_update(0, 0, 10, 10, data);
        assert!(matches!(event, RdpClientEvent::Error(_)));
    }

    #[test]
    fn test_create_frame_update_invalid_rect() {
        let data = vec![0u8; 400];
        let event = create_frame_update(0, 0, 0, 10, data);
        assert!(matches!(event, RdpClientEvent::Error(_)));
    }

    #[test]
    fn test_create_frame_update_with_conversion() {
        // RGB data for 2x2 image
        let rgb = vec![
            255, 0, 0, // Red
            0, 255, 0, // Green
            0, 0, 255, // Blue
            255, 255, 0, // Yellow
        ];
        let event = create_frame_update_with_conversion(0, 0, 2, 2, &rgb, PixelFormat::Rgb);
        if let RdpClientEvent::FrameUpdate { rect, data } = event {
            assert_eq!(rect.width, 2);
            assert_eq!(rect.height, 2);
            assert_eq!(data.len(), 16); // 4 pixels * 4 bytes
            // First pixel should be red in BGRA: B=0, G=0, R=255, A=255
            assert_eq!(data[0], 0);
            assert_eq!(data[1], 0);
            assert_eq!(data[2], 255);
            assert_eq!(data[3], 255);
        } else {
            panic!("Expected FrameUpdate event");
        }
    }
}
