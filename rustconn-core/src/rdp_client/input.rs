//! RDP input handling and coordinate transformation
//!
//! This module provides coordinate transformation for mouse input and
//! keyboard scancode conversion for RDP sessions.
//!
//! # Coordinate Transformation
//!
//! When rendering an RDP framebuffer in a GTK widget, the framebuffer may be
//! scaled and centered to maintain aspect ratio. Mouse coordinates from the
//! widget must be transformed to RDP server coordinates.
//!
//! # Requirements Coverage
//!
//! - Requirement 1.2: Mouse coordinate forwarding to RDP server
//! - Requirement 1.3: Keyboard event forwarding to RDP server
//! - Requirement 1.7: Dynamic resolution change on resize

// Allow clippy warnings for this file - input handling uses various integer sizes
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::float_cmp)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::unreadable_literal)]

use serde::{Deserialize, Serialize};

/// Represents the transformation parameters for coordinate conversion
///
/// This struct holds the scaling and offset values needed to transform
/// widget coordinates to RDP server coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CoordinateTransform {
    /// Scale factor applied to both X and Y (maintains aspect ratio)
    pub scale: f64,
    /// X offset for centering the framebuffer in the widget
    pub offset_x: f64,
    /// Y offset for centering the framebuffer in the widget
    pub offset_y: f64,
    /// RDP framebuffer width
    pub rdp_width: u32,
    /// RDP framebuffer height
    pub rdp_height: u32,
    /// Widget width
    pub widget_width: u32,
    /// Widget height
    pub widget_height: u32,
}

impl CoordinateTransform {
    /// Creates a new coordinate transform from widget and RDP dimensions
    ///
    /// The transform maintains aspect ratio by using the minimum scale factor
    /// and centers the framebuffer within the widget.
    ///
    /// # Arguments
    ///
    /// * `widget_width` - Width of the GTK widget in pixels
    /// * `widget_height` - Height of the GTK widget in pixels
    /// * `rdp_width` - Width of the RDP framebuffer in pixels
    /// * `rdp_height` - Height of the RDP framebuffer in pixels
    ///
    /// # Returns
    ///
    /// A `CoordinateTransform` with calculated scale and offset values.
    ///
    /// # Example
    ///
    /// ```
    /// use rustconn_core::rdp_client::input::CoordinateTransform;
    ///
    /// // Widget is 1920x1080, RDP framebuffer is 1280x720
    /// let transform = CoordinateTransform::new(1920, 1080, 1280, 720);
    ///
    /// // Scale should be 1.5 (1920/1280 = 1080/720 = 1.5)
    /// assert!((transform.scale - 1.5).abs() < 0.001);
    /// ```
    #[must_use]
    pub fn new(widget_width: u32, widget_height: u32, rdp_width: u32, rdp_height: u32) -> Self {
        // Handle edge cases with zero dimensions
        if widget_width == 0 || widget_height == 0 || rdp_width == 0 || rdp_height == 0 {
            return Self {
                scale: 1.0,
                offset_x: 0.0,
                offset_y: 0.0,
                rdp_width: rdp_width.max(1),
                rdp_height: rdp_height.max(1),
                widget_width: widget_width.max(1),
                widget_height: widget_height.max(1),
            };
        }

        let widget_w = f64::from(widget_width);
        let widget_h = f64::from(widget_height);
        let rdp_w = f64::from(rdp_width);
        let rdp_h = f64::from(rdp_height);

        // Calculate scale factors for each dimension
        let scale_x = widget_w / rdp_w;
        let scale_y = widget_h / rdp_h;

        // Use minimum scale to maintain aspect ratio (fit within widget)
        let scale = scale_x.min(scale_y);

        // Calculate offsets to center the framebuffer
        let offset_x = rdp_w.mul_add(-scale, widget_w) / 2.0;
        let offset_y = rdp_h.mul_add(-scale, widget_h) / 2.0;

        Self {
            scale,
            offset_x,
            offset_y,
            rdp_width,
            rdp_height,
            widget_width,
            widget_height,
        }
    }

    /// Transforms widget coordinates to RDP server coordinates
    ///
    /// This method converts mouse coordinates from the GTK widget space
    /// to the RDP framebuffer space, accounting for scaling and centering.
    ///
    /// # Arguments
    ///
    /// * `widget_x` - X coordinate in widget space
    /// * `widget_y` - Y coordinate in widget space
    ///
    /// # Returns
    ///
    /// A tuple `(rdp_x, rdp_y)` with coordinates clamped to valid RDP bounds.
    /// Returns `None` if the coordinates are outside the framebuffer area.
    ///
    /// # Example
    ///
    /// ```
    /// use rustconn_core::rdp_client::input::CoordinateTransform;
    ///
    /// let transform = CoordinateTransform::new(1920, 1080, 1280, 720);
    ///
    /// // Center of widget should map to center of RDP framebuffer
    /// let (rdp_x, rdp_y) = transform.transform(960.0, 540.0).unwrap();
    /// assert!((rdp_x - 640.0).abs() < 1.0);
    /// assert!((rdp_y - 360.0).abs() < 1.0);
    /// ```
    #[must_use]
    pub fn transform(&self, widget_x: f64, widget_y: f64) -> Option<(f64, f64)> {
        // Transform from widget space to RDP space
        let rdp_x = (widget_x - self.offset_x) / self.scale;
        let rdp_y = (widget_y - self.offset_y) / self.scale;

        // Check if coordinates are within the framebuffer bounds
        let rdp_w = f64::from(self.rdp_width);
        let rdp_h = f64::from(self.rdp_height);

        if rdp_x < 0.0 || rdp_x >= rdp_w || rdp_y < 0.0 || rdp_y >= rdp_h {
            return None;
        }

        Some((rdp_x, rdp_y))
    }

    /// Transforms widget coordinates to RDP server coordinates with clamping
    ///
    /// Unlike `transform()`, this method always returns valid coordinates
    /// by clamping to the framebuffer bounds.
    ///
    /// # Arguments
    ///
    /// * `widget_x` - X coordinate in widget space
    /// * `widget_y` - Y coordinate in widget space
    ///
    /// # Returns
    ///
    /// A tuple `(rdp_x, rdp_y)` with coordinates clamped to valid RDP bounds.
    ///
    /// # Example
    ///
    /// ```
    /// use rustconn_core::rdp_client::input::CoordinateTransform;
    ///
    /// let transform = CoordinateTransform::new(1920, 1080, 1280, 720);
    ///
    /// // Coordinates outside the framebuffer are clamped
    /// let (rdp_x, rdp_y) = transform.transform_clamped(-100.0, -100.0);
    /// assert_eq!(rdp_x, 0.0);
    /// assert_eq!(rdp_y, 0.0);
    /// ```
    #[must_use]
    pub fn transform_clamped(&self, widget_x: f64, widget_y: f64) -> (f64, f64) {
        let rdp_x = (widget_x - self.offset_x) / self.scale;
        let rdp_y = (widget_y - self.offset_y) / self.scale;

        let rdp_w = f64::from(self.rdp_width);
        let rdp_h = f64::from(self.rdp_height);

        // Clamp to valid bounds (0 to width-1, 0 to height-1)
        let clamped_x = rdp_x.clamp(0.0, (rdp_w - 1.0).max(0.0));
        let clamped_y = rdp_y.clamp(0.0, (rdp_h - 1.0).max(0.0));

        (clamped_x, clamped_y)
    }

    /// Transforms widget coordinates to integer RDP coordinates
    ///
    /// This is a convenience method that returns u16 coordinates suitable
    /// for sending to the RDP server.
    ///
    /// # Arguments
    ///
    /// * `widget_x` - X coordinate in widget space
    /// * `widget_y` - Y coordinate in widget space
    ///
    /// # Returns
    ///
    /// A tuple `(rdp_x, rdp_y)` as u16 values clamped to valid RDP bounds.
    #[must_use]
    pub fn transform_to_u16(&self, widget_x: f64, widget_y: f64) -> (u16, u16) {
        let (rdp_x, rdp_y) = self.transform_clamped(widget_x, widget_y);

        // Convert to u16, clamping to u16::MAX just in case
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let x = (rdp_x.round() as u32).min(u32::from(u16::MAX)) as u16;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let y = (rdp_y.round() as u32).min(u32::from(u16::MAX)) as u16;

        (x, y)
    }

    /// Checks if widget coordinates are within the framebuffer display area
    ///
    /// # Arguments
    ///
    /// * `widget_x` - X coordinate in widget space
    /// * `widget_y` - Y coordinate in widget space
    ///
    /// # Returns
    ///
    /// `true` if the coordinates are within the displayed framebuffer area.
    #[must_use]
    pub fn is_within_framebuffer(&self, widget_x: f64, widget_y: f64) -> bool {
        self.transform(widget_x, widget_y).is_some()
    }

    /// Returns the display bounds of the framebuffer within the widget
    ///
    /// # Returns
    ///
    /// A tuple `(x, y, width, height)` representing the framebuffer's
    /// position and size within the widget.
    #[must_use]
    pub fn framebuffer_bounds(&self) -> (f64, f64, f64, f64) {
        let width = f64::from(self.rdp_width) * self.scale;
        let height = f64::from(self.rdp_height) * self.scale;
        (self.offset_x, self.offset_y, width, height)
    }
}

impl Default for CoordinateTransform {
    fn default() -> Self {
        Self::new(1280, 720, 1280, 720)
    }
}

/// Standard RDP/display resolutions (width, height)
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

/// Maximum RDP resolution (per RDP specification)
pub const MAX_RDP_WIDTH: u16 = 8192;
pub const MAX_RDP_HEIGHT: u16 = 8192;

/// Minimum RDP resolution
pub const MIN_RDP_WIDTH: u16 = 200;
pub const MIN_RDP_HEIGHT: u16 = 200;

/// Finds the best matching standard resolution for the given dimensions
///
/// Returns the largest standard resolution that fits within the given dimensions,
/// or the smallest standard resolution if none fit.
///
/// # Arguments
///
/// * `width` - Available width in pixels
/// * `height` - Available height in pixels
///
/// # Returns
///
/// A tuple `(width, height)` of the best matching standard resolution.
///
/// # Example
///
/// ```
/// use rustconn_core::rdp_client::input::find_best_standard_resolution;
///
/// // For a 1920x1080 window, should return 1920x1080
/// let (w, h) = find_best_standard_resolution(1920, 1080);
/// assert_eq!((w, h), (1920, 1080));
///
/// // For a 1900x1000 window, should return 1680x1050 or similar
/// let (w, h) = find_best_standard_resolution(1900, 1000);
/// assert!(w <= 1900 && h <= 1000);
/// ```
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

/// Generates a resize request for the RDP server
///
/// This function takes the new widget dimensions and generates appropriate
/// resize parameters for the RDP server, respecting RDP resolution limits.
///
/// # Arguments
///
/// * `widget_width` - New widget width in pixels
/// * `widget_height` - New widget height in pixels
/// * `use_standard_resolution` - If true, snap to standard resolutions
///
/// # Returns
///
/// A tuple `(width, height)` as u16 values suitable for RDP resize request.
///
/// # Requirements Coverage
///
/// - Requirement 1.7: Dynamic resolution change on resize
#[must_use]
pub fn generate_resize_request(
    widget_width: u32,
    widget_height: u32,
    use_standard_resolution: bool,
) -> (u16, u16) {
    let (width, height) = if use_standard_resolution {
        find_best_standard_resolution(widget_width, widget_height)
    } else {
        (widget_width, widget_height)
    };

    // Clamp to RDP limits
    let width = width.clamp(u32::from(MIN_RDP_WIDTH), u32::from(MAX_RDP_WIDTH));
    let height = height.clamp(u32::from(MIN_RDP_HEIGHT), u32::from(MAX_RDP_HEIGHT));

    #[allow(clippy::cast_possible_truncation)]
    (width as u16, height as u16)
}

/// Checks if a resize request should be sent based on dimension changes
///
/// This function implements hysteresis to avoid sending too many resize
/// requests for small changes.
///
/// # Arguments
///
/// * `current_width` - Current RDP width
/// * `current_height` - Current RDP height
/// * `new_width` - Proposed new width
/// * `new_height` - Proposed new height
/// * `threshold` - Minimum change in pixels to trigger resize
///
/// # Returns
///
/// `true` if a resize request should be sent.
#[must_use]
pub fn should_resize(
    current_width: u16,
    current_height: u16,
    new_width: u16,
    new_height: u16,
    threshold: u16,
) -> bool {
    let width_diff = (i32::from(new_width) - i32::from(current_width)).unsigned_abs();
    let height_diff = (i32::from(new_height) - i32::from(current_height)).unsigned_abs();

    width_diff >= u32::from(threshold) || height_diff >= u32::from(threshold)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coordinate_transform_identity() {
        // When widget and RDP have same dimensions, transform should be identity
        let transform = CoordinateTransform::new(1280, 720, 1280, 720);

        assert!((transform.scale - 1.0).abs() < 0.001);
        assert!((transform.offset_x - 0.0).abs() < 0.001);
        assert!((transform.offset_y - 0.0).abs() < 0.001);

        // Center should map to center
        let (x, y) = transform.transform_clamped(640.0, 360.0);
        assert!((x - 640.0).abs() < 0.001);
        assert!((y - 360.0).abs() < 0.001);
    }

    #[test]
    fn test_coordinate_transform_scaled_up() {
        // Widget is 2x the RDP size
        let transform = CoordinateTransform::new(2560, 1440, 1280, 720);

        assert!((transform.scale - 2.0).abs() < 0.001);

        // Center of widget should map to center of RDP
        let (x, y) = transform.transform_clamped(1280.0, 720.0);
        assert!((x - 640.0).abs() < 0.001);
        assert!((y - 360.0).abs() < 0.001);
    }

    #[test]
    fn test_coordinate_transform_with_letterboxing() {
        // Widget is wider than RDP aspect ratio (will have horizontal letterboxing)
        let transform = CoordinateTransform::new(1920, 720, 1280, 720);

        // Scale should be 1.0 (limited by height)
        assert!((transform.scale - 1.0).abs() < 0.001);

        // Should have horizontal offset
        assert!(transform.offset_x > 0.0);
        assert!((transform.offset_y - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_coordinate_transform_clamping() {
        let transform = CoordinateTransform::new(1280, 720, 1280, 720);

        // Negative coordinates should clamp to 0
        let (x, y) = transform.transform_clamped(-100.0, -100.0);
        assert_eq!(x, 0.0);
        assert_eq!(y, 0.0);

        // Coordinates beyond bounds should clamp to max
        let (x, y) = transform.transform_clamped(2000.0, 2000.0);
        assert!((x - 1279.0).abs() < 0.001);
        assert!((y - 719.0).abs() < 0.001);
    }

    #[test]
    fn test_find_best_standard_resolution() {
        // Exact match
        assert_eq!(find_best_standard_resolution(1920, 1080), (1920, 1080));

        // Slightly smaller should return next smaller standard
        assert_eq!(find_best_standard_resolution(1900, 1000), (1600, 900));

        // Very small should return smallest standard
        assert_eq!(find_best_standard_resolution(100, 100), (640, 480));
    }

    #[test]
    fn test_generate_resize_request() {
        // Standard resolution
        let (w, h) = generate_resize_request(1920, 1080, true);
        assert_eq!((w, h), (1920, 1080));

        // Non-standard with snapping
        let (w, h) = generate_resize_request(1900, 1000, true);
        assert!(w <= 1900 && h <= 1000);

        // Non-standard without snapping
        let (w, h) = generate_resize_request(1900, 1000, false);
        assert_eq!((w, h), (1900, 1000));
    }

    #[test]
    fn test_should_resize() {
        // No change
        assert!(!should_resize(1920, 1080, 1920, 1080, 50));

        // Small change below threshold
        assert!(!should_resize(1920, 1080, 1930, 1080, 50));

        // Change above threshold
        assert!(should_resize(1920, 1080, 1980, 1080, 50));
    }
}

// ============================================================================
// Keyboard Scancode Conversion (Requirement 1.3)
// ============================================================================

/// RDP scancode for a key event
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RdpScancode {
    /// The scancode value
    pub code: u16,
    /// Whether this is an extended key (E0 prefix)
    pub extended: bool,
}

impl RdpScancode {
    /// Creates a new scancode
    #[must_use]
    pub const fn new(code: u16, extended: bool) -> Self {
        Self { code, extended }
    }

    /// Creates a standard (non-extended) scancode
    #[must_use]
    pub const fn standard(code: u16) -> Self {
        Self {
            code,
            extended: false,
        }
    }

    /// Creates an extended scancode (E0 prefix)
    #[must_use]
    pub const fn extended(code: u16) -> Self {
        Self {
            code,
            extended: true,
        }
    }
}

/// GTK/GDK keyval to RDP scancode mapping
///
/// This module provides conversion from GTK keyval values to RDP scancodes.
/// RDP uses IBM PC AT scancodes (Set 1).
///
/// # Scancode Format
///
/// - Standard keys: 8-bit scancode (0x00-0x7F for make, 0x80-0xFF for break)
/// - Extended keys: E0 prefix followed by scancode
///
/// # Requirements Coverage
///
/// - Requirement 1.3: Keyboard event forwarding to RDP server
///
/// Converts a GTK keyval to an RDP scancode
///
/// # Arguments
///
/// * `keyval` - GTK/GDK keyval (e.g., from `gdk::Key::into_glib()`)
///
/// # Returns
///
/// An `Option<RdpScancode>` containing the scancode if the key is mappable.
///
/// # Example
///
/// ```
/// use rustconn_core::rdp_client::input::keyval_to_scancode;
///
/// // 'a' key
/// let scancode = keyval_to_scancode(0x61); // GDK_KEY_a
/// assert!(scancode.is_some());
/// ```
#[must_use]
#[allow(clippy::too_many_lines)]
pub const fn keyval_to_scancode(keyval: u32) -> Option<RdpScancode> {
    // GDK keyval constants (from gdk/gdkkeysyms.h)
    // These are the most common keys used in RDP sessions

    match keyval {
        // Letters (lowercase and uppercase map to same scancode)
        0x61 | 0x41 => Some(RdpScancode::standard(0x1E)), // a/A
        0x62 | 0x42 => Some(RdpScancode::standard(0x30)), // b/B
        0x63 | 0x43 => Some(RdpScancode::standard(0x2E)), // c/C
        0x64 | 0x44 => Some(RdpScancode::standard(0x20)), // d/D
        0x65 | 0x45 => Some(RdpScancode::standard(0x12)), // e/E
        0x66 | 0x46 => Some(RdpScancode::standard(0x21)), // f/F
        0x67 | 0x47 => Some(RdpScancode::standard(0x22)), // g/G
        0x68 | 0x48 => Some(RdpScancode::standard(0x23)), // h/H
        0x69 | 0x49 => Some(RdpScancode::standard(0x17)), // i/I
        0x6A | 0x4A => Some(RdpScancode::standard(0x24)), // j/J
        0x6B | 0x4B => Some(RdpScancode::standard(0x25)), // k/K
        0x6C | 0x4C => Some(RdpScancode::standard(0x26)), // l/L
        0x6D | 0x4D => Some(RdpScancode::standard(0x32)), // m/M
        0x6E | 0x4E => Some(RdpScancode::standard(0x31)), // n/N
        0x6F | 0x4F => Some(RdpScancode::standard(0x18)), // o/O
        0x70 | 0x50 => Some(RdpScancode::standard(0x19)), // p/P
        0x71 | 0x51 => Some(RdpScancode::standard(0x10)), // q/Q
        0x72 | 0x52 => Some(RdpScancode::standard(0x13)), // r/R
        0x73 | 0x53 => Some(RdpScancode::standard(0x1F)), // s/S
        0x74 | 0x54 => Some(RdpScancode::standard(0x14)), // t/T
        0x75 | 0x55 => Some(RdpScancode::standard(0x16)), // u/U
        0x76 | 0x56 => Some(RdpScancode::standard(0x2F)), // v/V
        0x77 | 0x57 => Some(RdpScancode::standard(0x11)), // w/W
        0x78 | 0x58 => Some(RdpScancode::standard(0x2D)), // x/X
        0x79 | 0x59 => Some(RdpScancode::standard(0x15)), // y/Y
        0x7A | 0x5A => Some(RdpScancode::standard(0x2C)), // z/Z

        // Numbers (top row)
        0x30 => Some(RdpScancode::standard(0x0B)), // 0
        0x31 => Some(RdpScancode::standard(0x02)), // 1
        0x32 => Some(RdpScancode::standard(0x03)), // 2
        0x33 => Some(RdpScancode::standard(0x04)), // 3
        0x34 => Some(RdpScancode::standard(0x05)), // 4
        0x35 => Some(RdpScancode::standard(0x06)), // 5
        0x36 => Some(RdpScancode::standard(0x07)), // 6
        0x37 => Some(RdpScancode::standard(0x08)), // 7
        0x38 => Some(RdpScancode::standard(0x09)), // 8
        0x39 => Some(RdpScancode::standard(0x0A)), // 9

        // Function keys (GDK_KEY_F1 = 0xFFBE)
        0xFFBE => Some(RdpScancode::standard(0x3B)), // F1
        0xFFBF => Some(RdpScancode::standard(0x3C)), // F2
        0xFFC0 => Some(RdpScancode::standard(0x3D)), // F3
        0xFFC1 => Some(RdpScancode::standard(0x3E)), // F4
        0xFFC2 => Some(RdpScancode::standard(0x3F)), // F5
        0xFFC3 => Some(RdpScancode::standard(0x40)), // F6
        0xFFC4 => Some(RdpScancode::standard(0x41)), // F7
        0xFFC5 => Some(RdpScancode::standard(0x42)), // F8
        0xFFC6 => Some(RdpScancode::standard(0x43)), // F9
        0xFFC7 => Some(RdpScancode::standard(0x44)), // F10
        0xFFC8 => Some(RdpScancode::standard(0x57)), // F11
        0xFFC9 => Some(RdpScancode::standard(0x58)), // F12

        // Special keys
        0xFF1B => Some(RdpScancode::standard(0x01)), // Escape
        0xFF08 => Some(RdpScancode::standard(0x0E)), // BackSpace
        0xFF09 => Some(RdpScancode::standard(0x0F)), // Tab
        0xFF0D => Some(RdpScancode::standard(0x1C)), // Return/Enter
        0x20 => Some(RdpScancode::standard(0x39)),   // Space

        // Modifier keys
        0xFFE1 => Some(RdpScancode::standard(0x2A)), // Shift_L
        0xFFE2 => Some(RdpScancode::standard(0x36)), // Shift_R
        0xFFE3 => Some(RdpScancode::standard(0x1D)), // Control_L
        0xFFE4 => Some(RdpScancode::extended(0x1D)), // Control_R (extended)
        0xFFE9 => Some(RdpScancode::standard(0x38)), // Alt_L
        0xFFEA => Some(RdpScancode::extended(0x38)), // Alt_R (extended)
        0xFFEB => Some(RdpScancode::extended(0x5B)), // Super_L (Windows key)
        0xFFEC => Some(RdpScancode::extended(0x5C)), // Super_R (Windows key)
        0xFFE5 => Some(RdpScancode::standard(0x3A)), // Caps_Lock
        0xFF7F => Some(RdpScancode::standard(0x45)), // Num_Lock
        0xFF14 => Some(RdpScancode::standard(0x46)), // Scroll_Lock

        // Navigation keys (extended)
        0xFF50 => Some(RdpScancode::extended(0x47)), // Home
        0xFF51 => Some(RdpScancode::extended(0x4B)), // Left
        0xFF52 => Some(RdpScancode::extended(0x48)), // Up
        0xFF53 => Some(RdpScancode::extended(0x4D)), // Right
        0xFF54 => Some(RdpScancode::extended(0x50)), // Down
        0xFF55 => Some(RdpScancode::extended(0x49)), // Page_Up
        0xFF56 => Some(RdpScancode::extended(0x51)), // Page_Down
        0xFF57 => Some(RdpScancode::extended(0x4F)), // End
        0xFF63 => Some(RdpScancode::extended(0x52)), // Insert
        0xFFFF => Some(RdpScancode::extended(0x53)), // Delete

        // Punctuation and symbols
        0x2D => Some(RdpScancode::standard(0x0C)), // minus
        0x3D => Some(RdpScancode::standard(0x0D)), // equal
        0x5B => Some(RdpScancode::standard(0x1A)), // bracketleft
        0x5D => Some(RdpScancode::standard(0x1B)), // bracketright
        0x5C => Some(RdpScancode::standard(0x2B)), // backslash
        0x3B => Some(RdpScancode::standard(0x27)), // semicolon
        0x27 => Some(RdpScancode::standard(0x28)), // apostrophe
        0x60 => Some(RdpScancode::standard(0x29)), // grave
        0x2C => Some(RdpScancode::standard(0x33)), // comma
        0x2E => Some(RdpScancode::standard(0x34)), // period
        0x2F => Some(RdpScancode::standard(0x35)), // slash

        // Numpad keys
        0xFFB0 => Some(RdpScancode::standard(0x52)), // KP_0
        0xFFB1 => Some(RdpScancode::standard(0x4F)), // KP_1
        0xFFB2 => Some(RdpScancode::standard(0x50)), // KP_2
        0xFFB3 => Some(RdpScancode::standard(0x51)), // KP_3
        0xFFB4 => Some(RdpScancode::standard(0x4B)), // KP_4
        0xFFB5 => Some(RdpScancode::standard(0x4C)), // KP_5
        0xFFB6 => Some(RdpScancode::standard(0x4D)), // KP_6
        0xFFB7 => Some(RdpScancode::standard(0x47)), // KP_7
        0xFFB8 => Some(RdpScancode::standard(0x48)), // KP_8
        0xFFB9 => Some(RdpScancode::standard(0x49)), // KP_9
        0xFFAA => Some(RdpScancode::standard(0x37)), // KP_Multiply
        0xFFAB => Some(RdpScancode::standard(0x4E)), // KP_Add
        0xFFAD => Some(RdpScancode::standard(0x4A)), // KP_Subtract
        0xFFAE => Some(RdpScancode::standard(0x53)), // KP_Decimal
        0xFFAF => Some(RdpScancode::extended(0x35)), // KP_Divide (extended)
        0xFF8D => Some(RdpScancode::extended(0x1C)), // KP_Enter (extended)

        // Print Screen, Pause, Break
        0xFF61 => Some(RdpScancode::extended(0x37)), // Print
        0xFF13 => Some(RdpScancode::standard(0x45)), // Pause (special handling needed)
        0xFF6B => Some(RdpScancode::extended(0x46)), // Break

        // Menu key
        0xFF67 => Some(RdpScancode::extended(0x5D)), // Menu

        _ => None,
    }
}

/// Converts a hardware keycode to an RDP scancode
///
/// This is an alternative to keyval conversion that uses the raw hardware
/// keycode, which may be more reliable for some keyboard layouts.
///
/// # Arguments
///
/// * `keycode` - Hardware keycode (evdev on Linux)
///
/// # Returns
///
/// An `Option<RdpScancode>` containing the scancode if mappable.
#[must_use]
pub const fn keycode_to_scancode(keycode: u32) -> Option<RdpScancode> {
    // Linux evdev keycodes are offset by 8 from X11 keycodes
    // and roughly correspond to AT scancodes

    // For most keys, evdev keycode - 8 gives the AT scancode
    // Extended keys need special handling

    match keycode {
        // Extended keys (navigation, etc.)
        97 => Some(RdpScancode::extended(0x1D)), // Right Ctrl
        100 => Some(RdpScancode::extended(0x38)), // Right Alt
        102 => Some(RdpScancode::extended(0x47)), // Home
        103 => Some(RdpScancode::extended(0x48)), // Up
        104 => Some(RdpScancode::extended(0x49)), // Page Up
        105 => Some(RdpScancode::extended(0x4B)), // Left
        106 => Some(RdpScancode::extended(0x4D)), // Right
        107 => Some(RdpScancode::extended(0x4F)), // End
        108 => Some(RdpScancode::extended(0x50)), // Down
        109 => Some(RdpScancode::extended(0x51)), // Page Down
        110 => Some(RdpScancode::extended(0x52)), // Insert
        111 => Some(RdpScancode::extended(0x53)), // Delete
        125 => Some(RdpScancode::extended(0x5B)), // Left Super
        126 => Some(RdpScancode::extended(0x5C)), // Right Super
        127 => Some(RdpScancode::extended(0x5D)), // Menu

        // Numpad Enter and Divide
        96 => Some(RdpScancode::extended(0x1C)), // KP Enter
        98 => Some(RdpScancode::extended(0x35)), // KP Divide

        // Standard keys: evdev keycode - 8 = AT scancode (approximately)
        9..=88 => {
            // This is a simplification; real mapping is more complex
            let scancode = (keycode - 8) as u16;
            if scancode <= 0x58 {
                Some(RdpScancode::standard(scancode))
            } else {
                None
            }
        }

        _ => None,
    }
}

/// Scancodes for Ctrl+Alt+Del sequence
pub const SCANCODE_CTRL: RdpScancode = RdpScancode::standard(0x1D);
pub const SCANCODE_ALT: RdpScancode = RdpScancode::standard(0x38);
pub const SCANCODE_DELETE: RdpScancode = RdpScancode::extended(0x53);

/// Generates the key sequence for Ctrl+Alt+Del
///
/// Returns a vector of (scancode, extended, pressed) tuples representing
/// the key events to send.
///
/// # Requirements Coverage
///
/// - Requirement 1.4: Ctrl+Alt+Del support
#[must_use]
pub fn ctrl_alt_del_sequence() -> Vec<(u16, bool, bool)> {
    vec![
        // Press Ctrl
        (SCANCODE_CTRL.code, SCANCODE_CTRL.extended, true),
        // Press Alt
        (SCANCODE_ALT.code, SCANCODE_ALT.extended, true),
        // Press Delete
        (SCANCODE_DELETE.code, SCANCODE_DELETE.extended, true),
        // Release Delete
        (SCANCODE_DELETE.code, SCANCODE_DELETE.extended, false),
        // Release Alt
        (SCANCODE_ALT.code, SCANCODE_ALT.extended, false),
        // Release Ctrl
        (SCANCODE_CTRL.code, SCANCODE_CTRL.extended, false),
    ]
}

/// Checks if a keyval represents a printable character
#[must_use]
pub fn is_printable_keyval(keyval: u32) -> bool {
    // ASCII printable range
    (0x20..=0x7E).contains(&keyval)
}

/// Checks if a keyval represents a modifier key
#[must_use]
pub const fn is_modifier_keyval(keyval: u32) -> bool {
    matches!(
        keyval,
        0xFFE1 | 0xFFE2 | // Shift
        0xFFE3 | 0xFFE4 | // Control
        0xFFE9 | 0xFFEA | // Alt
        0xFFEB | 0xFFEC | // Super
        0xFFE5 | 0xFF7F | 0xFF14 // Caps, Num, Scroll Lock
    )
}

#[cfg(test)]
mod keyboard_tests {
    use super::*;

    #[test]
    fn test_letter_scancodes() {
        // Test lowercase letters
        assert_eq!(keyval_to_scancode(0x61), Some(RdpScancode::standard(0x1E))); // a
        assert_eq!(keyval_to_scancode(0x7A), Some(RdpScancode::standard(0x2C))); // z

        // Test uppercase letters (same scancode)
        assert_eq!(keyval_to_scancode(0x41), Some(RdpScancode::standard(0x1E))); // A
        assert_eq!(keyval_to_scancode(0x5A), Some(RdpScancode::standard(0x2C)));
        // Z
    }

    #[test]
    fn test_number_scancodes() {
        assert_eq!(keyval_to_scancode(0x30), Some(RdpScancode::standard(0x0B))); // 0
        assert_eq!(keyval_to_scancode(0x31), Some(RdpScancode::standard(0x02))); // 1
        assert_eq!(keyval_to_scancode(0x39), Some(RdpScancode::standard(0x0A)));
        // 9
    }

    #[test]
    fn test_function_key_scancodes() {
        assert_eq!(
            keyval_to_scancode(0xFFBE),
            Some(RdpScancode::standard(0x3B))
        ); // F1
        assert_eq!(
            keyval_to_scancode(0xFFC9),
            Some(RdpScancode::standard(0x58))
        ); // F12
    }

    #[test]
    fn test_extended_key_scancodes() {
        // Navigation keys should be extended
        let home = keyval_to_scancode(0xFF50).unwrap();
        assert!(home.extended);

        let delete = keyval_to_scancode(0xFFFF).unwrap();
        assert!(delete.extended);

        // Right Ctrl should be extended
        let right_ctrl = keyval_to_scancode(0xFFE4).unwrap();
        assert!(right_ctrl.extended);
    }

    #[test]
    fn test_ctrl_alt_del_sequence() {
        let sequence = ctrl_alt_del_sequence();
        assert_eq!(sequence.len(), 6);

        // First three should be presses
        assert!(sequence[0].2); // Ctrl press
        assert!(sequence[1].2); // Alt press
        assert!(sequence[2].2); // Delete press

        // Last three should be releases
        assert!(!sequence[3].2); // Delete release
        assert!(!sequence[4].2); // Alt release
        assert!(!sequence[5].2); // Ctrl release
    }

    #[test]
    fn test_is_printable() {
        assert!(is_printable_keyval(0x20)); // Space
        assert!(is_printable_keyval(0x41)); // A
        assert!(is_printable_keyval(0x7E)); // ~
        assert!(!is_printable_keyval(0x1B)); // Escape (control char)
        assert!(!is_printable_keyval(0xFFBE)); // F1
    }

    #[test]
    fn test_is_modifier() {
        assert!(is_modifier_keyval(0xFFE1)); // Shift_L
        assert!(is_modifier_keyval(0xFFE3)); // Control_L
        assert!(is_modifier_keyval(0xFFE9)); // Alt_L
        assert!(!is_modifier_keyval(0x61)); // 'a'
        assert!(!is_modifier_keyval(0xFFBE)); // F1
    }
}

// ============================================================================
// GTK Keyval to Unicode Conversion
// ============================================================================

/// Converts a GTK keyval to a Unicode character
///
/// GTK keyvals for non-Latin characters (like Cyrillic) are not direct Unicode
/// code points. This function handles the conversion.
///
/// # GTK Keyval Ranges
///
/// - 0x0020-0x007E: ASCII printable (direct Unicode)
/// - 0x00A0-0x00FF: Latin-1 supplement (direct Unicode)
/// - 0x0100-0x01FF: Latin Extended-A (direct Unicode)
/// - 0x0400-0x04FF: Cyrillic in Unicode
/// - 0x06xx: GTK Cyrillic keyvals (need conversion)
///
/// # Arguments
///
/// * `keyval` - GTK/GDK keyval
///
/// # Returns
///
/// The corresponding Unicode character, or None if not convertible.
#[must_use]
pub fn keyval_to_unicode(keyval: u32) -> Option<char> {
    // ASCII printable range - direct mapping
    if (0x0020..=0x007E).contains(&keyval) {
        return char::from_u32(keyval);
    }

    // Latin-1 supplement - direct mapping
    if (0x00A0..=0x00FF).contains(&keyval) {
        return char::from_u32(keyval);
    }

    // GTK Cyrillic keyvals (0x6xx range) need conversion to Unicode (0x4xx range)
    // GDK_KEY_Cyrillic_* constants are in 0x6A0-0x6FF range
    // Unicode Cyrillic is in 0x400-0x4FF range
    if (0x06A1..=0x06FF).contains(&keyval) {
        // Map GTK Cyrillic keyval to Unicode
        // The mapping is: GTK 0x6xx -> Unicode 0x4xx with specific offsets
        let unicode = match keyval {
            // Ukrainian specific
            0x06A4 => 0x0404, // Є (Ukrainian Ie)
            0x06A6 => 0x0406, // І (Ukrainian I)
            0x06A7 => 0x0407, // Ї (Ukrainian Yi)
            0x06AD => 0x0490, // Ґ (Ukrainian Ghe with upturn)
            0x06B4 => 0x0454, // є (Ukrainian ie)
            0x06B6 => 0x0456, // і (Ukrainian i)
            0x06B7 => 0x0457, // ї (Ukrainian yi)
            0x06BD => 0x0491, // ґ (Ukrainian ghe with upturn)

            // Russian/Common Cyrillic uppercase (0x6A0-0x6BF -> 0x410-0x42F)
            0x06E1 => 0x0410, // А
            0x06E2 => 0x0411, // Б
            0x06F7 => 0x0412, // В
            0x06E7 => 0x0413, // Г
            0x06E4 => 0x0414, // Д
            0x06E5 => 0x0415, // Е
            0x06F6 => 0x0416, // Ж
            0x06FA => 0x0417, // З
            0x06E9 => 0x0418, // И
            0x06EA => 0x0419, // Й
            0x06EB => 0x041A, // К
            0x06EC => 0x041B, // Л
            0x06ED => 0x041C, // М
            0x06EE => 0x041D, // Н
            0x06EF => 0x041E, // О
            0x06F0 => 0x041F, // П
            0x06F2 => 0x0420, // Р
            0x06F3 => 0x0421, // С
            0x06F4 => 0x0422, // Т
            0x06F5 => 0x0423, // У
            0x06E6 => 0x0424, // Ф
            0x06E8 => 0x0425, // Х
            0x06E3 => 0x0426, // Ц
            0x06FE => 0x0427, // Ч
            0x06FB => 0x0428, // Ш
            0x06FD => 0x0429, // Щ
            0x06FF => 0x042A, // Ъ
            0x06F9 => 0x042B, // Ы
            0x06F8 => 0x042C, // Ь
            0x06FC => 0x042D, // Э
            0x06E0 => 0x042E, // Ю
            0x06F1 => 0x042F, // Я

            // Russian/Common Cyrillic lowercase (0x6C0-0x6DF -> 0x430-0x44F)
            0x06C1 => 0x0430, // а
            0x06C2 => 0x0431, // б
            0x06D7 => 0x0432, // в
            0x06C7 => 0x0433, // г
            0x06C4 => 0x0434, // д
            0x06C5 => 0x0435, // е
            0x06D6 => 0x0436, // ж
            0x06DA => 0x0437, // з
            0x06C9 => 0x0438, // и
            0x06CA => 0x0439, // й
            0x06CB => 0x043A, // к
            0x06CC => 0x043B, // л
            0x06CD => 0x043C, // м
            0x06CE => 0x043D, // н
            0x06CF => 0x043E, // о
            0x06D0 => 0x043F, // п
            0x06D2 => 0x0440, // р
            0x06D3 => 0x0441, // с
            0x06D4 => 0x0442, // т
            0x06D5 => 0x0443, // у
            0x06C6 => 0x0444, // ф
            0x06C8 => 0x0445, // х
            0x06C3 => 0x0446, // ц
            0x06DE => 0x0447, // ч
            0x06DB => 0x0448, // ш
            0x06DD => 0x0449, // щ
            0x06DF => 0x044A, // ъ
            0x06D9 => 0x044B, // ы
            0x06D8 => 0x044C, // ь
            0x06DC => 0x044D, // э
            0x06C0 => 0x044E, // ю
            0x06D1 => 0x044F, // я

            // Ё
            0x06A3 => 0x0401, // Ё (uppercase)
            0x06B3 => 0x0451, // ё (lowercase)

            _ => return None,
        };
        return char::from_u32(unicode);
    }

    // Direct Unicode range (for characters already in Unicode)
    if keyval <= 0x0010_FFFF {
        return char::from_u32(keyval);
    }

    None
}

#[cfg(test)]
mod unicode_tests {
    use super::*;

    #[test]
    fn test_ascii_conversion() {
        assert_eq!(keyval_to_unicode(0x41), Some('A'));
        assert_eq!(keyval_to_unicode(0x61), Some('a'));
        assert_eq!(keyval_to_unicode(0x20), Some(' '));
    }

    #[test]
    fn test_cyrillic_lowercase() {
        // Ukrainian і
        assert_eq!(keyval_to_unicode(0x06B6), Some('і'));
        // Russian а
        assert_eq!(keyval_to_unicode(0x06C1), Some('а'));
    }

    #[test]
    fn test_cyrillic_uppercase() {
        // Ukrainian І
        assert_eq!(keyval_to_unicode(0x06A6), Some('І'));
        // Russian А
        assert_eq!(keyval_to_unicode(0x06E1), Some('А'));
    }
}
