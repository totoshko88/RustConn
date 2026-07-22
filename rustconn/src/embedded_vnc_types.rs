//! Type definitions for embedded VNC widget
//!
//! This module contains types, enums, and helper structs used by the embedded VNC widget.

use rustconn_core::models::ScaleOverride;
use thiserror::Error;

/// Standard VNC/display resolutions (width, height)
/// Sorted by total pixels for efficient lookup
pub const STANDARD_RESOLUTIONS: &[(u32, u32)] = &[
    (640, 480),   // VGA
    (800, 600),   // SVGA
    (1024, 768),  // XGA
    (1152, 864),  // XGA+
    (1280, 720),  // HD 720p
    (1280, 800),  // WXGA
    (1280, 1024), // SXGA
    (1366, 768),  // HD
    (1440, 900),  // WXGA+
    (1600, 900),  // HD+
    (1600, 1200), // UXGA
    (1680, 1050), // WSXGA+
    (1920, 1080), // Full HD
    (1920, 1200), // WUXGA
    (2560, 1440), // QHD
    (2560, 1600), // WQXGA
    (3840, 2160), // 4K UHD
];

/// Finds the best matching standard resolution for the given dimensions
///
/// Returns the largest standard resolution that fits within the given dimensions,
/// or the smallest standard resolution if none fit.
#[must_use]
pub fn find_best_standard_resolution(width: u32, height: u32) -> (u32, u32) {
    // Find the largest resolution that fits within the given dimensions
    let mut best = STANDARD_RESOLUTIONS[0]; // Start with smallest

    for &(res_w, res_h) in STANDARD_RESOLUTIONS {
        if res_w <= width && res_h <= height {
            // This resolution fits, and since we iterate in ascending order,
            // it's larger than or equal to the previous best
            best = (res_w, res_h);
        }
    }

    best
}

/// Error type for embedded VNC operations
#[derive(Debug, Error, Clone)]
pub enum EmbeddedVncError {
    /// Wayland subsurface creation failed
    #[error("Wayland subsurface creation failed: {0}")]
    SubsurfaceCreation(String),

    /// VNC client initialization failed
    #[error("VNC client initialization failed: {0}")]
    VncClientInit(String),

    /// Connection to VNC server failed
    #[error("Connection failed: {0}")]
    Connection(String),

    /// Native VNC client is not available, falling back to external mode
    #[error("Native VNC client not available, falling back to external mode")]
    NativeVncNotAvailable,

    /// Input forwarding error
    #[error("Input forwarding error: {0}")]
    InputForwarding(String),

    /// Resize handling error
    #[error("Resize handling error: {0}")]
    ResizeError(String),

    /// Authentication failed
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
}

/// Connection state for embedded VNC widget
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VncConnectionState {
    /// Not connected
    #[default]
    Disconnected,
    /// Connection in progress
    Connecting,
    /// Connected and rendering
    Connected,
    /// Connection error occurred
    Error,
}

impl std::fmt::Display for VncConnectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => write!(f, "Disconnected"),
            Self::Connecting => write!(f, "Connecting"),
            Self::Connected => write!(f, "Connected"),
            Self::Error => write!(f, "Error"),
        }
    }
}

/// VNC connection configuration
#[derive(Debug, Clone, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "VNC protocol config has natural boolean flags"
)]
pub struct VncConfig {
    /// Target hostname or IP address
    pub host: String,
    /// Target port (default: 5900)
    pub port: u16,
    /// Password for authentication (stored securely, zeroized on drop)
    pub password: Option<secrecy::SecretString>,
    /// Desired width in pixels
    pub width: u32,
    /// Desired height in pixels
    pub height: u32,
    /// Encoding preference (e.g., "tight", "zrle", "raw")
    pub encoding: Option<String>,
    /// Quality level (0-9, higher is better quality)
    pub quality: Option<u8>,
    /// Compression level (0-9, higher is more compression)
    pub compression: Option<u8>,
    /// Enable clipboard sharing
    pub clipboard_enabled: bool,
    /// View only mode (no input forwarding)
    pub view_only: bool,
    /// Display scale override for embedded mode
    pub scale_override: ScaleOverride,
    /// Additional VNC viewer arguments
    pub extra_args: Vec<String>,
    /// Show local mouse cursor over embedded viewer (disable to avoid double cursor)
    pub show_local_cursor: bool,
    /// Accept untrusted TLS certificates (for VeNCrypt connections)
    pub accept_certificate: bool,
    /// Enable Multipath TCP for the embedded VNC connection.
    /// Uses multiple network paths for seamless mobility and bandwidth aggregation.
    /// Requires kernel MPTCP support (Linux 5.6+). Falls back to regular TCP.
    pub mptcp: bool,
}

impl VncConfig {
    /// Creates a new VNC configuration with default settings
    #[must_use]
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 5900,
            password: None,
            width: 1280,
            height: 720,
            encoding: None,
            quality: None,
            compression: None,
            clipboard_enabled: true,
            view_only: false,
            scale_override: ScaleOverride::default(),
            extra_args: Vec::new(),
            show_local_cursor: true,
            accept_certificate: false,
            mptcp: false,
        }
    }

    /// Sets the port
    #[must_use]
    pub const fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Sets the password
    #[must_use]
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(secrecy::SecretString::new(password.into().into()));
        self
    }

    /// Sets the resolution
    #[must_use]
    pub const fn with_resolution(mut self, width: u32, height: u32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Sets the encoding preference
    #[must_use]
    pub fn with_encoding(mut self, encoding: impl Into<String>) -> Self {
        self.encoding = Some(encoding.into());
        self
    }

    /// Sets the quality level (0-9)
    #[must_use]
    pub const fn with_quality(mut self, quality: u8) -> Self {
        self.quality = Some(if quality > 9 { 9 } else { quality });
        self
    }

    /// Sets the compression level (0-9)
    #[must_use]
    pub const fn with_compression(mut self, compression: u8) -> Self {
        self.compression = Some(if compression > 9 { 9 } else { compression });
        self
    }

    /// Enables or disables clipboard sharing
    #[must_use]
    pub const fn with_clipboard(mut self, enabled: bool) -> Self {
        self.clipboard_enabled = enabled;
        self
    }

    /// Enables or disables view-only mode
    #[must_use]
    pub const fn with_view_only(mut self, view_only: bool) -> Self {
        self.view_only = view_only;
        self
    }

    /// Adds extra VNC viewer arguments
    #[must_use]
    pub fn with_extra_args(mut self, args: Vec<String>) -> Self {
        self.extra_args = args;
        self
    }

    /// Returns the VNC display number (port - 5900)
    #[must_use]
    pub fn display_number(&self) -> i32 {
        if self.port >= 5900 && self.port < 6000 {
            i32::from(self.port) - 5900
        } else {
            -1 // Use raw port
        }
    }
}

/// Callback type for state change notifications
pub type StateCallback = Box<dyn Fn(VncConnectionState) + 'static>;

/// Callback type for error notifications
pub type ErrorCallback = Box<dyn Fn(&str) + 'static>;

/// Callback type for frame update notifications
pub type FrameCallback = Box<dyn Fn(u32, u32, u32, u32) + 'static>;
