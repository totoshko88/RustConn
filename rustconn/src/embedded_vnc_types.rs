//! Type definitions for embedded VNC widget
//!
//! This module contains types, enums, and helper structs used by the embedded VNC widget.

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
pub struct VncConfig {
    /// Target hostname or IP address
    pub host: String,
    /// Target port (default: 5900)
    pub port: u16,
    /// Password for authentication
    pub password: Option<String>,
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
    /// Additional VNC viewer arguments
    pub extra_args: Vec<String>,
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
            extra_args: Vec::new(),
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
        self.password = Some(password.into());
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

/// Pixel buffer for VNC frame data
///
/// This struct holds the pixel data received from the VNC server
/// and is used to blit to the Wayland surface.
#[derive(Debug)]
pub struct VncPixelBuffer {
    /// Raw pixel data in BGRA format
    data: Vec<u8>,
    /// Buffer width in pixels
    width: u32,
    /// Buffer height in pixels
    height: u32,
    /// Stride (bytes per row)
    stride: u32,
    /// Bits per pixel
    bpp: u8,
}

impl VncPixelBuffer {
    /// Creates a new pixel buffer with the specified dimensions
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        let bpp = 32; // BGRA = 32 bits per pixel
        let stride = width * 4; // 4 bytes per pixel
        let size = (stride * height) as usize;
        Self {
            data: vec![0; size],
            width,
            height,
            stride,
            bpp,
        }
    }

    /// Returns the buffer width
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the buffer height
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the stride (bytes per row)
    #[must_use]
    pub const fn stride(&self) -> u32 {
        self.stride
    }

    /// Returns the bits per pixel
    #[must_use]
    pub const fn bpp(&self) -> u8 {
        self.bpp
    }

    /// Returns a reference to the raw pixel data
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Returns a mutable reference to the raw pixel data
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Resizes the buffer to new dimensions
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.stride = width * 4;
        let size = (self.stride * height) as usize;
        self.data.resize(size, 0);
    }

    /// Clears the buffer to black
    pub fn clear(&mut self) {
        self.data.fill(0);
    }

    /// Updates a region of the buffer with raw pixel data
    ///
    /// # Arguments
    ///
    /// * `x` - X coordinate of the region
    /// * `y` - Y coordinate of the region
    /// * `w` - Width of the region
    /// * `h` - Height of the region
    /// * `src_data` - Source pixel data
    /// * `src_stride` - Source stride
    #[allow(clippy::cast_possible_truncation)]
    pub fn update_region(
        &mut self,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        src_data: &[u8],
        src_stride: u32,
    ) {
        let dst_stride = self.stride as usize;
        let src_stride = src_stride as usize;
        let bytes_per_pixel = 4;

        for row in 0..h {
            let dst_y = (y + row) as usize;
            if dst_y >= self.height as usize {
                break;
            }

            let dst_offset = dst_y * dst_stride + (x as usize * bytes_per_pixel);
            let src_offset = row as usize * src_stride;
            let copy_width =
                (w as usize * bytes_per_pixel).min(dst_stride - (x as usize * bytes_per_pixel));

            if src_offset + copy_width <= src_data.len()
                && dst_offset + copy_width <= self.data.len()
            {
                self.data[dst_offset..dst_offset + copy_width]
                    .copy_from_slice(&src_data[src_offset..src_offset + copy_width]);
            }
        }
    }

    /// Copies a rectangular region within the buffer (for CopyRect encoding)
    #[allow(clippy::cast_possible_truncation)]
    pub fn copy_rect(&mut self, src_x: u32, src_y: u32, dst_x: u32, dst_y: u32, w: u32, h: u32) {
        let stride = self.stride as usize;
        let bytes_per_pixel = 4;

        // Create a temporary buffer for the source region
        let mut temp = vec![0u8; (w * h * 4) as usize];

        // Copy source region to temp
        for row in 0..h {
            let src_offset = ((src_y + row) as usize * stride) + (src_x as usize * bytes_per_pixel);
            let temp_offset = row as usize * (w as usize * bytes_per_pixel);
            let copy_width = w as usize * bytes_per_pixel;

            if src_offset + copy_width <= self.data.len() {
                temp[temp_offset..temp_offset + copy_width]
                    .copy_from_slice(&self.data[src_offset..src_offset + copy_width]);
            }
        }

        // Copy temp to destination
        for row in 0..h {
            let dst_offset = ((dst_y + row) as usize * stride) + (dst_x as usize * bytes_per_pixel);
            let temp_offset = row as usize * (w as usize * bytes_per_pixel);
            let copy_width = w as usize * bytes_per_pixel;

            if dst_offset + copy_width <= self.data.len() {
                self.data[dst_offset..dst_offset + copy_width]
                    .copy_from_slice(&temp[temp_offset..temp_offset + copy_width]);
            }
        }
    }
}

/// Wayland surface handle for VNC subsurface integration
///
/// This struct manages the Wayland surface resources for embedding
/// the VNC display within the GTK widget hierarchy.
#[derive(Debug, Default)]
pub struct VncWaylandSurface {
    /// Whether the surface is initialized
    initialized: bool,
    /// Surface ID (for debugging)
    surface_id: u32,
}

impl VncWaylandSurface {
    /// Creates a new uninitialized surface handle
    #[must_use]
    pub const fn new() -> Self {
        Self {
            initialized: false,
            surface_id: 0,
        }
    }

    /// Initializes the Wayland surface
    ///
    /// # Errors
    ///
    /// Returns error if surface creation fails
    pub fn initialize(&mut self) -> Result<(), EmbeddedVncError> {
        // In a real implementation, this would:
        // 1. Get the wl_display from GTK
        // 2. Create a wl_surface
        // 3. Create a wl_subsurface attached to the parent
        // 4. Set up shared memory buffers

        // For now, we mark as initialized for the fallback path
        self.initialized = true;
        self.surface_id = 1;
        Ok(())
    }

    /// Returns whether the surface is initialized
    #[must_use]
    pub const fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Commits pending changes to the surface
    pub fn commit(&self) {
        // In a real implementation, this would call wl_surface_commit
    }

    /// Damages a region of the surface for redraw
    pub fn damage(&self, _x: i32, _y: i32, _width: i32, _height: i32) {
        // In a real implementation, this would call wl_surface_damage_buffer
    }

    /// Cleans up the surface resources
    pub fn cleanup(&mut self) {
        self.initialized = false;
        self.surface_id = 0;
    }
}

/// Callback type for state change notifications
pub type StateCallback = Box<dyn Fn(VncConnectionState) + 'static>;

/// Callback type for error notifications
pub type ErrorCallback = Box<dyn Fn(&str) + 'static>;

/// Callback type for frame update notifications
pub type FrameCallback = Box<dyn Fn(u32, u32, u32, u32) + 'static>;
