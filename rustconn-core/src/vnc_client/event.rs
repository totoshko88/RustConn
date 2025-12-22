//! VNC client events and commands

/// Rectangle coordinates for VNC operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VncRect {
    /// X coordinate
    pub x: u16,
    /// Y coordinate
    pub y: u16,
    /// Width
    pub width: u16,
    /// Height
    pub height: u16,
}

impl VncRect {
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
}

/// Events emitted by the VNC client to the GUI
#[derive(Debug, Clone)]
pub enum VncClientEvent {
    /// Connection established successfully
    Connected,

    /// Connection closed
    Disconnected,

    /// Resolution changed (width, height)
    ResolutionChanged { width: u32, height: u32 },

    /// Raw framebuffer update (rect, BGRA pixel data)
    FrameUpdate { rect: VncRect, data: Vec<u8> },

    /// Copy rectangle from source to destination
    CopyRect { dst: VncRect, src: VncRect },

    /// Cursor shape update
    CursorUpdate { rect: VncRect, data: Vec<u8> },

    /// Server sent bell notification
    Bell,

    /// Server clipboard text
    ClipboardText(String),

    /// Authentication required
    AuthRequired,

    /// Error occurred
    Error(String),
}

/// Commands sent from GUI to VNC client
#[derive(Debug, Clone)]
pub enum VncClientCommand {
    /// Disconnect from server
    Disconnect,

    /// Send keyboard event (keysym, pressed)
    KeyEvent { keysym: u32, pressed: bool },

    /// Send pointer/mouse event (x, y, buttons)
    PointerEvent { x: u16, y: u16, buttons: u8 },

    /// Send clipboard text to server
    ClipboardText(String),

    /// Request full framebuffer refresh
    RefreshScreen,

    /// Provide authentication password
    Authenticate(String),

    /// Request desktop size change (requires server support for `ExtendedDesktopSize`)
    SetDesktopSize { width: u16, height: u16 },

    /// Send Ctrl+Alt+Del key sequence (for Windows login screens)
    SendCtrlAltDel,

    /// Type text by emulating key presses (for paste functionality)
    TypeText(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vnc_rect() {
        let rect = VncRect::new(10, 20, 100, 200);
        assert_eq!(rect.x, 10);
        assert_eq!(rect.y, 20);
        assert_eq!(rect.width, 100);
        assert_eq!(rect.height, 200);
    }

    #[test]
    fn test_event_variants() {
        let event = VncClientEvent::Connected;
        assert!(matches!(event, VncClientEvent::Connected));

        let event = VncClientEvent::ResolutionChanged {
            width: 1920,
            height: 1080,
        };
        if let VncClientEvent::ResolutionChanged { width, height } = event {
            assert_eq!(width, 1920);
            assert_eq!(height, 1080);
        }
    }

    #[test]
    fn test_command_variants() {
        let cmd = VncClientCommand::KeyEvent {
            keysym: 0x61,
            pressed: true,
        };
        if let VncClientCommand::KeyEvent { keysym, pressed } = cmd {
            assert_eq!(keysym, 0x61);
            assert!(pressed);
        }
    }
}
