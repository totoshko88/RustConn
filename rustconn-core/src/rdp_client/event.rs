//! RDP client events and commands
//!
//! This module provides event and command types for the RDP client,
//! along with conversion functions for framebuffer data.

// Allow clippy warnings for this file - RDP protocol uses various integer sizes
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::option_if_let_else)]
#![allow(clippy::redundant_clone)]

/// Clipboard format information for RDP clipboard operations
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardFormatInfo {
    /// Format ID (standard Windows clipboard format or custom)
    pub id: u32,
    /// Format name (for custom formats)
    pub name: Option<String>,
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

/// Converts framebuffer data from one pixel format to BGRA
///
/// This function converts pixel data from various formats to BGRA,
/// which is the native format for Cairo/GTK rendering.
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
) -> Option<Vec<u8>> {
    let pixel_count = width as usize * height as usize;
    let expected_size = pixel_count * format.bytes_per_pixel();

    if data.len() < expected_size {
        return None;
    }

    match format {
        PixelFormat::Bgra => {
            // Already in BGRA format, just clone
            Some(data[..expected_size].to_vec())
        }
        PixelFormat::Rgba => {
            // Convert RGBA to BGRA (swap R and B)
            let mut result = Vec::with_capacity(pixel_count * 4);
            for chunk in data[..expected_size].chunks_exact(4) {
                result.push(chunk[2]); // B
                result.push(chunk[1]); // G
                result.push(chunk[0]); // R
                result.push(chunk[3]); // A
            }
            Some(result)
        }
        PixelFormat::Rgb => {
            // Convert RGB to BGRA (swap R and B, add alpha)
            let mut result = Vec::with_capacity(pixel_count * 4);
            for chunk in data[..expected_size].chunks_exact(3) {
                result.push(chunk[2]); // B
                result.push(chunk[1]); // G
                result.push(chunk[0]); // R
                result.push(255); // A (fully opaque)
            }
            Some(result)
        }
        PixelFormat::Bgr => {
            // Convert BGR to BGRA (add alpha)
            let mut result = Vec::with_capacity(pixel_count * 4);
            for chunk in data[..expected_size].chunks_exact(3) {
                result.push(chunk[0]); // B
                result.push(chunk[1]); // G
                result.push(chunk[2]); // R
                result.push(255); // A (fully opaque)
            }
            Some(result)
        }
        PixelFormat::Rgb565 => {
            // Convert RGB565 to BGRA
            let mut result = Vec::with_capacity(pixel_count * 4);
            for chunk in data[..expected_size].chunks_exact(2) {
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
            Some(result)
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
        Some(bgra_data) => create_frame_update(x, y, width, height, bgra_data),
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

    /// Server requests clipboard data from client
    ClipboardDataRequest(ClipboardFormatInfo),

    /// Request to fetch clipboard data from server (internal, triggers `initiate_paste`)
    ClipboardPasteRequest(ClipboardFormatInfo),

    /// Authentication required (for NLA)
    AuthRequired,

    /// Error occurred
    Error(String),

    /// Server sent a warning/info message
    ServerMessage(String),
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

    /// Request clipboard data from server (triggers `initiate_paste`)
    RequestClipboardData {
        /// Format ID to request
        format_id: u32,
    },

    /// Request screen refresh
    RefreshScreen,

    /// Request resolution change (if server supports)
    SetDesktopSize {
        /// Desired width
        width: u16,
        /// Desired height
        height: u16,
    },

    /// Send Ctrl+Alt+Del key sequence
    SendCtrlAltDel,

    /// Provide authentication credentials
    Authenticate {
        /// Username
        username: String,
        /// Password
        password: String,
        /// Domain (optional)
        domain: Option<String>,
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
        // BGRA should pass through unchanged
        let data = vec![0, 1, 2, 3, 4, 5, 6, 7]; // 2 pixels
        let result = convert_to_bgra(&data, PixelFormat::Bgra, 2, 1);
        assert_eq!(result, Some(data));
    }

    #[test]
    fn test_convert_rgba_to_bgra() {
        // RGBA: R=255, G=128, B=64, A=200
        let rgba = vec![255, 128, 64, 200];
        let result = convert_to_bgra(&rgba, PixelFormat::Rgba, 1, 1);
        // BGRA: B=64, G=128, R=255, A=200
        assert_eq!(result, Some(vec![64, 128, 255, 200]));
    }

    #[test]
    fn test_convert_rgb_to_bgra() {
        // RGB: R=255, G=128, B=64
        let rgb = vec![255, 128, 64];
        let result = convert_to_bgra(&rgb, PixelFormat::Rgb, 1, 1);
        // BGRA: B=64, G=128, R=255, A=255
        assert_eq!(result, Some(vec![64, 128, 255, 255]));
    }

    #[test]
    fn test_convert_bgr_to_bgra() {
        // BGR: B=64, G=128, R=255
        let bgr = vec![64, 128, 255];
        let result = convert_to_bgra(&bgr, PixelFormat::Bgr, 1, 1);
        // BGRA: B=64, G=128, R=255, A=255
        assert_eq!(result, Some(vec![64, 128, 255, 255]));
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
