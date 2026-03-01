//! SPICE client events and commands
//!
//! This module provides event and command types for the SPICE client,
//! following the same pattern as VNC and RDP clients.

/// Rectangle coordinates for SPICE operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpiceRect {
    /// X coordinate
    pub x: u16,
    /// Y coordinate
    pub y: u16,
    /// Width
    pub width: u16,
    /// Height
    pub height: u16,
}

impl SpiceRect {
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

/// Events emitted by the SPICE client to the GUI
#[derive(Debug, Clone)]
pub enum SpiceClientEvent {
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
        rect: SpiceRect,
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

    /// Server clipboard text
    ClipboardText(String),

    /// Authentication required
    AuthRequired,

    /// Error occurred
    Error(String),

    /// Server sent a warning/info message
    ServerMessage(String),

    /// Channel opened (display, inputs, etc.)
    ChannelOpened(SpiceChannel),

    /// Channel closed
    ChannelClosed(SpiceChannel),

    /// USB device available for redirection
    UsbDeviceAvailable {
        /// Device ID
        device_id: u32,
        /// Device description
        description: String,
    },

    /// USB device redirected
    UsbDeviceRedirected {
        /// Device ID
        device_id: u32,
    },

    /// USB device disconnected
    UsbDeviceDisconnected {
        /// Device ID
        device_id: u32,
    },
}

/// SPICE channel types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpiceChannel {
    /// Main control channel
    Main,
    /// Display channel
    Display,
    /// Inputs channel (keyboard/mouse)
    Inputs,
    /// Cursor channel
    Cursor,
    /// Playback audio channel
    Playback,
    /// Record audio channel
    Record,
    /// USB redirection channel
    Usbredir,
    /// Smartcard channel
    Smartcard,
    /// Webdav (folder sharing) channel
    Webdav,
    /// Port channel
    Port,
}

impl std::fmt::Display for SpiceChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Main => write!(f, "Main"),
            Self::Display => write!(f, "Display"),
            Self::Inputs => write!(f, "Inputs"),
            Self::Cursor => write!(f, "Cursor"),
            Self::Playback => write!(f, "Playback"),
            Self::Record => write!(f, "Record"),
            Self::Usbredir => write!(f, "USB Redirection"),
            Self::Smartcard => write!(f, "Smartcard"),
            Self::Webdav => write!(f, "Folder Sharing"),
            Self::Port => write!(f, "Port"),
        }
    }
}

/// Commands sent from GUI to SPICE client
#[derive(Debug, Clone)]
pub enum SpiceClientCommand {
    /// Disconnect from server
    Disconnect,

    /// Send keyboard event
    KeyEvent {
        /// Scancode
        scancode: u32,
        /// Key pressed (true) or released (false)
        pressed: bool,
    },

    /// Send pointer/mouse event
    PointerEvent {
        /// X coordinate
        x: u16,
        /// Y coordinate
        y: u16,
        /// Button flags (bit 0: left, bit 1: middle, bit 2: right)
        buttons: u8,
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
        /// Password (stored securely, zeroized on drop)
        password: secrecy::SecretString,
    },

    /// Enable/disable USB redirection
    SetUsbRedirection {
        /// Enable or disable
        enabled: bool,
    },

    /// Redirect a specific USB device
    RedirectUsbDevice {
        /// Device ID
        device_id: u32,
    },

    /// Stop redirecting a USB device
    UnredirectUsbDevice {
        /// Device ID
        device_id: u32,
    },

    /// Enable/disable clipboard sharing
    SetClipboardEnabled {
        /// Enable or disable
        enabled: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spice_rect() {
        let rect = SpiceRect::new(10, 20, 100, 200);
        assert_eq!(rect.x, 10);
        assert_eq!(rect.y, 20);
        assert_eq!(rect.width, 100);
        assert_eq!(rect.height, 200);
    }

    #[test]
    fn test_full_screen_rect() {
        let rect = SpiceRect::full_screen(1920, 1080);
        assert_eq!(rect.x, 0);
        assert_eq!(rect.y, 0);
        assert_eq!(rect.width, 1920);
        assert_eq!(rect.height, 1080);
    }

    #[test]
    fn test_rect_area() {
        let rect = SpiceRect::new(0, 0, 100, 50);
        assert_eq!(rect.area(), 5000);
    }

    #[test]
    fn test_rect_is_valid() {
        assert!(SpiceRect::new(0, 0, 100, 100).is_valid());
        assert!(!SpiceRect::new(0, 0, 0, 100).is_valid());
        assert!(!SpiceRect::new(0, 0, 100, 0).is_valid());
    }

    #[test]
    fn test_rect_is_within_bounds() {
        let rect = SpiceRect::new(10, 10, 100, 100);
        assert!(rect.is_within_bounds(200, 200));
        assert!(rect.is_within_bounds(110, 110));
        assert!(!rect.is_within_bounds(100, 200));
        assert!(!rect.is_within_bounds(200, 100));
    }

    #[test]
    fn test_event_variants() {
        let event = SpiceClientEvent::Connected {
            width: 1920,
            height: 1080,
        };
        if let SpiceClientEvent::Connected { width, height } = event {
            assert_eq!(width, 1920);
            assert_eq!(height, 1080);
        }
    }

    #[test]
    fn test_command_variants() {
        let cmd = SpiceClientCommand::KeyEvent {
            scancode: 0x1E,
            pressed: true,
        };
        if let SpiceClientCommand::KeyEvent { scancode, pressed } = cmd {
            assert_eq!(scancode, 0x1E);
            assert!(pressed);
        }
    }

    #[test]
    fn test_channel_display() {
        assert_eq!(SpiceChannel::Main.to_string(), "Main");
        assert_eq!(SpiceChannel::Display.to_string(), "Display");
        assert_eq!(SpiceChannel::Inputs.to_string(), "Inputs");
        assert_eq!(SpiceChannel::Usbredir.to_string(), "USB Redirection");
        assert_eq!(SpiceChannel::Webdav.to_string(), "Folder Sharing");
    }
}
