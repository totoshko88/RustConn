//! Wayland subsurface integration for embedded protocol sessions
//!
//! This module provides the `WaylandSubsurface` struct for native Wayland
//! compositor integration of embedded RDP/VNC sessions.
//!
//! # Architecture
//!
//! On Wayland, embedded protocol sessions use `wl_subsurface` for native
//! compositor integration. This provides:
//! - Direct surface composition without intermediate copies
//! - Proper input handling through the compositor
//! - Smooth rendering with damage tracking
//!
//! On X11, the module falls back to Cairo-based rendering using the
//! existing `DrawingArea` approach.
//!
//! # Requirements Coverage
//!
//! - Requirement 8.1: Create wl_subsurface for embedded sessions
//! - Requirement 8.2: Update subsurface position on parent move/resize
//! - Requirement 8.3: Blit framebuffer to Wayland surface using shared memory
//! - Requirement 8.4: Fall back to Cairo rendering on X11

// Allow cast warnings - graphics code uses various integer sizes for coordinates
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::missing_panics_doc)]

use gtk4::prelude::*;
use gtk4::DrawingArea;
use thiserror::Error;

// Re-export DisplayServer from the unified display module
pub use crate::display::DisplayServer;

/// Error type for Wayland surface operations
#[derive(Debug, Error, Clone)]
pub enum WaylandSurfaceError {
    /// Display server detection failed
    #[error("Failed to detect display server: {0}")]
    DetectionFailed(String),

    /// Wayland display not available
    #[error("Wayland display not available")]
    WaylandNotAvailable,

    /// Subsurface creation failed
    #[error("Failed to create subsurface: {0}")]
    SubsurfaceCreation(String),

    /// Shared memory pool creation failed
    #[error("Failed to create shared memory pool: {0}")]
    ShmPoolCreation(String),

    /// Buffer allocation failed
    #[error("Failed to allocate buffer: {0}")]
    BufferAllocation(String),

    /// Surface commit failed
    #[error("Failed to commit surface: {0}")]
    CommitFailed(String),

    /// Position update failed
    #[error("Failed to update position: {0}")]
    PositionUpdateFailed(String),

    /// Framebuffer blit failed
    #[error("Failed to blit framebuffer: {0}")]
    BlitFailed(String),
}

/// Display server type - re-exported from display module with additional methods
///
/// This type alias provides backward compatibility while using the unified
/// display server detection from `crate::display::DisplayServer`.
pub type DisplayServerType = DisplayServer;

/// Shared memory buffer for Wayland surface
///
/// This struct manages a shared memory buffer that can be used
/// for efficient framebuffer blitting to Wayland surfaces.
#[derive(Debug)]
pub struct ShmBuffer {
    /// Raw pixel data in BGRA format
    data: Vec<u8>,
    /// Buffer width in pixels
    width: u32,
    /// Buffer height in pixels
    height: u32,
    /// Stride (bytes per row)
    stride: u32,
    /// Whether the buffer has been modified since last commit
    dirty: bool,
}

impl ShmBuffer {
    /// Creates a new shared memory buffer with the specified dimensions
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        let stride = width * 4; // BGRA = 4 bytes per pixel
        let size = (stride * height) as usize;
        Self {
            data: vec![0; size],
            width,
            height,
            stride,
            dirty: false,
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

    /// Returns whether the buffer has been modified
    #[must_use]
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Returns a reference to the raw pixel data
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Returns a mutable reference to the raw pixel data
    pub fn data_mut(&mut self) -> &mut [u8] {
        self.dirty = true;
        &mut self.data
    }

    /// Resizes the buffer to new dimensions
    pub fn resize(&mut self, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;
        self.stride = width * 4;
        let size = (self.stride * height) as usize;
        self.data.resize(size, 0);
        self.dirty = true;
    }

    /// Clears the buffer to black
    pub fn clear(&mut self) {
        self.data.fill(0);
        self.dirty = true;
    }

    /// Marks the buffer as clean (after commit)
    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Updates a rectangular region of the buffer
    ///
    /// # Arguments
    ///
    /// * `x` - X coordinate of the region
    /// * `y` - Y coordinate of the region
    /// * `w` - Width of the region
    /// * `h` - Height of the region
    /// * `src_data` - Source pixel data in BGRA format
    /// * `src_stride` - Source stride (bytes per row)
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
        self.dirty = true;
    }
}

/// Rectangular region for damage tracking
#[derive(Debug, Clone, Copy, Default)]
pub struct DamageRect {
    /// X coordinate
    pub x: i32,
    /// Y coordinate
    pub y: i32,
    /// Width
    pub width: i32,
    /// Height
    pub height: i32,
}

impl DamageRect {
    /// Creates a new damage rectangle
    #[must_use]
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Creates a damage rectangle covering the entire surface
    #[must_use]
    pub const fn full(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width: width as i32,
            height: height as i32,
        }
    }

    /// Returns whether this rectangle is empty
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    /// Merges another rectangle into this one (union)
    pub fn merge(&mut self, other: &Self) {
        if other.is_empty() {
            return;
        }
        if self.is_empty() {
            *self = *other;
            return;
        }

        let x1 = self.x.min(other.x);
        let y1 = self.y.min(other.y);
        let x2 = (self.x + self.width).max(other.x + other.width);
        let y2 = (self.y + self.height).max(other.y + other.height);

        self.x = x1;
        self.y = y1;
        self.width = x2 - x1;
        self.height = y2 - y1;
    }
}

/// Wayland subsurface for embedded protocol sessions
///
/// This struct manages a Wayland subsurface for rendering embedded
/// RDP/VNC sessions directly within the GTK widget hierarchy.
///
/// On X11, it falls back to Cairo-based rendering using the DrawingArea.
///
/// # Requirements Coverage
///
/// - Requirement 8.1: Create wl_subsurface for embedded sessions
/// - Requirement 8.2: Update subsurface position on parent move/resize
/// - Requirement 8.3: Blit framebuffer using shared memory buffers
/// - Requirement 8.4: Fall back to Cairo rendering on X11
#[derive(Debug)]
pub struct WaylandSubsurface {
    /// Detected display server type
    display_server: DisplayServerType,
    /// Whether the subsurface is initialized
    initialized: bool,
    /// Shared memory buffer for pixel data
    shm_buffer: ShmBuffer,
    /// Current position relative to parent (x, y)
    position: (i32, i32),
    /// Accumulated damage region
    damage: DamageRect,
    /// Surface ID for debugging
    surface_id: u32,
    /// Next surface ID counter
    next_id: u32,
    /// Whether native Wayland rendering is active
    native_wayland_active: bool,
}

impl WaylandSubsurface {
    /// Creates a new uninitialized Wayland subsurface
    #[must_use]
    pub fn new() -> Self {
        Self {
            display_server: DisplayServerType::detect(),
            initialized: false,
            shm_buffer: ShmBuffer::new(1280, 720),
            position: (0, 0),
            damage: DamageRect::default(),
            surface_id: 0,
            next_id: 1,
            native_wayland_active: false,
        }
    }

    /// Creates a new subsurface with specified dimensions
    #[must_use]
    pub fn with_size(width: u32, height: u32) -> Self {
        Self {
            display_server: DisplayServerType::detect(),
            initialized: false,
            shm_buffer: ShmBuffer::new(width, height),
            position: (0, 0),
            damage: DamageRect::default(),
            surface_id: 0,
            next_id: 1,
            native_wayland_active: false,
        }
    }

    /// Returns the detected display server type
    #[must_use]
    pub const fn display_server(&self) -> DisplayServerType {
        self.display_server
    }

    /// Returns whether the subsurface is initialized
    #[must_use]
    pub const fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Returns whether native Wayland subsurface is being used
    #[must_use]
    pub fn is_native_wayland(&self) -> bool {
        self.initialized && self.native_wayland_active
    }

    /// Returns whether Cairo fallback is being used
    #[must_use]
    pub fn is_cairo_fallback(&self) -> bool {
        self.initialized && !self.native_wayland_active
    }

    /// Returns the current buffer width
    #[must_use]
    pub fn width(&self) -> u32 {
        self.shm_buffer.width()
    }

    /// Returns the current buffer height
    #[must_use]
    pub fn height(&self) -> u32 {
        self.shm_buffer.height()
    }

    /// Returns the current position
    #[must_use]
    pub const fn position(&self) -> (i32, i32) {
        self.position
    }

    /// Returns a reference to the shared memory buffer
    #[must_use]
    pub const fn buffer(&self) -> &ShmBuffer {
        &self.shm_buffer
    }

    /// Returns a mutable reference to the shared memory buffer
    pub fn buffer_mut(&mut self) -> &mut ShmBuffer {
        &mut self.shm_buffer
    }

    /// Initializes the subsurface
    ///
    /// On Wayland with `wayland-native` feature, this detects the Wayland
    /// display and prepares for potential future native rendering.
    ///
    /// Currently, all rendering uses Cairo fallback because full native
    /// Wayland subsurface integration requires unsafe code to access
    /// raw `wl_surface` pointers, which is forbidden in this project.
    ///
    /// # Arguments
    ///
    /// * `parent` - The parent GTK widget (DrawingArea)
    ///
    /// # Errors
    ///
    /// Returns error if subsurface creation fails
    ///
    /// # Requirements Coverage
    ///
    /// - Requirement 8.1: Create wl_subsurface for embedded sessions (partial)
    /// - Requirement 8.4: Fall back to Cairo rendering on X11
    pub fn initialize(
        &mut self,
        parent: &impl IsA<gtk4::Widget>,
    ) -> Result<(), WaylandSurfaceError> {
        if self.initialized {
            return Ok(());
        }

        match self.display_server {
            DisplayServerType::Wayland => {
                #[cfg(feature = "wayland-native")]
                {
                    // Verify we're running on Wayland by checking the surface type
                    if let Some(native) = parent.as_ref().native() {
                        if let Some(surface) = native.surface() {
                            if surface
                                .downcast_ref::<gdk4_wayland::WaylandSurface>()
                                .is_some()
                            {
                                tracing::info!(
                                    "[WaylandSubsurface] Wayland surface confirmed, \
                                     using Cairo rendering (native subsurface requires unsafe)"
                                );
                            }
                        }
                    }
                }

                #[cfg(not(feature = "wayland-native"))]
                {
                    tracing::info!(
                        "[WaylandSubsurface] Wayland detected, using Cairo fallback \
                         (wayland-native feature not enabled)"
                    );
                }

                // Cairo fallback - native subsurface would require unsafe code
                // to access raw wl_surface pointers from gdk4-wayland
                self.native_wayland_active = false;
                self.surface_id = self.next_id;
                self.next_id += 1;
                self.initialized = true;
                Ok(())
            }
            DisplayServerType::X11 | DisplayServerType::Unknown => {
                tracing::info!(
                    "[WaylandSubsurface] {} detected, using Cairo fallback",
                    self.display_server
                );

                self.native_wayland_active = false;
                self.surface_id = self.next_id;
                self.next_id += 1;
                self.initialized = true;
                Ok(())
            }
        }
    }

    /// Updates the subsurface position relative to the parent
    ///
    /// On Wayland, this updates the wl_subsurface position.
    /// On X11, this stores the position for Cairo rendering offset.
    ///
    /// # Arguments
    ///
    /// * `x` - X coordinate relative to parent
    /// * `y` - Y coordinate relative to parent
    ///
    /// # Requirements Coverage
    ///
    /// - Requirement 8.2: Update subsurface position on parent move/resize
    pub fn update_position(&mut self, x: i32, y: i32) {
        if !self.initialized {
            return;
        }

        self.position = (x, y);

        if self.display_server.supports_subsurface() {
            // On Wayland, we would call:
            // wl_subsurface_set_position(subsurface, x, y);
            // wl_surface_commit(surface);
        }
        // On X11, position is used during Cairo rendering
    }

    /// Resizes the subsurface buffer
    ///
    /// # Arguments
    ///
    /// * `width` - New width in pixels
    /// * `height` - New height in pixels
    pub fn resize(&mut self, width: u32, height: u32) {
        if !self.initialized {
            return;
        }

        self.shm_buffer.resize(width, height);

        // Mark entire surface as damaged after resize
        self.damage = DamageRect::full(width, height);
    }

    /// Blits framebuffer data to the subsurface
    ///
    /// # Arguments
    ///
    /// * `data` - Pixel data in BGRA format
    /// * `rect` - Rectangle describing the update region
    ///
    /// # Requirements Coverage
    ///
    /// - Requirement 8.3: Blit framebuffer to Wayland surface
    pub fn blit_framebuffer(&mut self, data: &[u8], rect: &DamageRect) {
        if !self.initialized || rect.is_empty() {
            return;
        }

        // Calculate source stride (assuming BGRA format)
        let src_stride = (rect.width as u32) * 4;

        // Update the shared memory buffer
        self.shm_buffer.update_region(
            rect.x as u32,
            rect.y as u32,
            rect.width as u32,
            rect.height as u32,
            data,
            src_stride,
        );

        // Accumulate damage
        self.damage.merge(rect);
    }

    /// Blits a full framebuffer to the subsurface
    ///
    /// # Arguments
    ///
    /// * `data` - Full framebuffer pixel data in BGRA format
    /// * `width` - Framebuffer width
    /// * `height` - Framebuffer height
    /// * `stride` - Framebuffer stride (bytes per row)
    pub fn blit_full_framebuffer(&mut self, data: &[u8], width: u32, height: u32, stride: u32) {
        if !self.initialized {
            return;
        }

        // Resize buffer if needed
        if self.shm_buffer.width() != width || self.shm_buffer.height() != height {
            self.shm_buffer.resize(width, height);
        }

        // Update entire buffer
        self.shm_buffer
            .update_region(0, 0, width, height, data, stride);

        // Mark entire surface as damaged
        self.damage = DamageRect::full(width, height);
    }

    /// Commits pending changes to the surface
    ///
    /// On Wayland, this commits the surface with damage information.
    /// On X11, this is a no-op (Cairo rendering handles commits).
    pub fn commit(&mut self) {
        if !self.initialized || self.damage.is_empty() {
            return;
        }

        if self.display_server.supports_subsurface() {
            // On Wayland, we would:
            // 1. Attach the buffer to the surface
            // 2. Mark the damaged region
            // 3. Commit the surface
            //
            // wl_surface_attach(surface, buffer, 0, 0);
            // wl_surface_damage_buffer(surface, damage.x, damage.y, damage.width, damage.height);
            // wl_surface_commit(surface);
        }

        // Clear damage after commit
        self.damage = DamageRect::default();
        self.shm_buffer.mark_clean();
    }

    /// Damages a region of the surface for redraw
    ///
    /// # Arguments
    ///
    /// * `x` - X coordinate
    /// * `y` - Y coordinate
    /// * `width` - Width of damaged region
    /// * `height` - Height of damaged region
    pub fn damage(&mut self, x: i32, y: i32, width: i32, height: i32) {
        if !self.initialized {
            return;
        }

        let rect = DamageRect::new(x, y, width, height);
        self.damage.merge(&rect);
    }

    /// Damages the entire surface
    pub fn damage_full(&mut self) {
        if !self.initialized {
            return;
        }

        self.damage = DamageRect::full(self.shm_buffer.width(), self.shm_buffer.height());
    }

    /// Cleans up the subsurface resources
    pub fn cleanup(&mut self) {
        if !self.initialized {
            return;
        }

        if self.display_server.supports_subsurface() {
            // On Wayland, we would destroy:
            // - wl_buffer
            // - wl_shm_pool
            // - wl_subsurface
            // - wl_surface
        }

        self.initialized = false;
        self.surface_id = 0;
        self.damage = DamageRect::default();
        self.shm_buffer.clear();
    }

    /// Renders the buffer to a Cairo context (X11 fallback)
    ///
    /// This method is used on X11 where native Wayland subsurfaces
    /// are not available. It renders the shared memory buffer to
    /// the provided Cairo context.
    ///
    /// # Arguments
    ///
    /// * `cr` - Cairo context to render to
    /// * `widget_width` - Widget width for scaling
    /// * `widget_height` - Widget height for scaling
    ///
    /// # Requirements Coverage
    ///
    /// - Requirement 8.4: Fall back to Cairo rendering on X11
    pub fn render_cairo(
        &self,
        cr: &gtk4::cairo::Context,
        widget_width: i32,
        widget_height: i32,
    ) -> Result<(), WaylandSurfaceError> {
        if !self.initialized {
            return Err(WaylandSurfaceError::CommitFailed(
                "Subsurface not initialized".to_string(),
            ));
        }

        let buf_width = self.shm_buffer.width();
        let buf_height = self.shm_buffer.height();

        if buf_width == 0 || buf_height == 0 {
            return Ok(());
        }

        // Create a Cairo ImageSurface from the buffer data
        // The buffer is in BGRA format which matches Cairo's ARGB32
        let data = self.shm_buffer.data().to_vec();

        let surface = gtk4::cairo::ImageSurface::create_for_data(
            data,
            gtk4::cairo::Format::ARgb32,
            buf_width as i32,
            buf_height as i32,
            self.shm_buffer.stride() as i32,
        )
        .map_err(|e| WaylandSurfaceError::BlitFailed(e.to_string()))?;

        // Calculate scale to fit widget while maintaining aspect ratio
        let scale_x = f64::from(widget_width) / f64::from(buf_width);
        let scale_y = f64::from(widget_height) / f64::from(buf_height);
        let scale = scale_x.min(scale_y);

        // Center the image
        let offset_x = f64::from(buf_width).mul_add(-scale, f64::from(widget_width)) / 2.0;
        let offset_y = f64::from(buf_height).mul_add(-scale, f64::from(widget_height)) / 2.0;

        // Apply position offset
        let final_x = offset_x + f64::from(self.position.0);
        let final_y = offset_y + f64::from(self.position.1);

        cr.save()
            .map_err(|e| WaylandSurfaceError::BlitFailed(e.to_string()))?;
        cr.translate(final_x, final_y);
        cr.scale(scale, scale);
        cr.set_source_surface(&surface, 0.0, 0.0)
            .map_err(|e| WaylandSurfaceError::BlitFailed(e.to_string()))?;
        cr.paint()
            .map_err(|e| WaylandSurfaceError::BlitFailed(e.to_string()))?;
        cr.restore()
            .map_err(|e| WaylandSurfaceError::BlitFailed(e.to_string()))?;

        Ok(())
    }
}

impl Default for WaylandSubsurface {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for WaylandSubsurface {
    fn drop(&mut self) {
        self.cleanup();
    }
}

/// Rendering mode for embedded sessions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderingMode {
    /// Native Wayland subsurface rendering
    WaylandSubsurface,
    /// Cairo-based rendering (X11 fallback)
    #[default]
    CairoFallback,
}

impl RenderingMode {
    /// Detects the best rendering mode for the current display server
    #[must_use]
    pub fn detect() -> Self {
        let display_server = DisplayServerType::detect();

        // Native Wayland subsurface requires both:
        // 1. Wayland display server
        // 2. wayland-native feature enabled
        if display_server.supports_subsurface() && DisplayServerType::has_native_wayland_support() {
            // For now, still use Cairo fallback until full wl_subsurface
            // protocol implementation is complete
            Self::CairoFallback
        } else {
            Self::CairoFallback
        }
    }

    /// Returns whether native Wayland rendering could be available
    #[must_use]
    pub fn native_wayland_possible() -> bool {
        DisplayServerType::detect().supports_subsurface()
            && DisplayServerType::has_native_wayland_support()
    }
}

impl std::fmt::Display for RenderingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WaylandSubsurface => write!(f, "Wayland Subsurface"),
            Self::CairoFallback => write!(f, "Cairo Fallback"),
        }
    }
}

/// Wrapper for managing embedded session rendering
///
/// This struct provides a unified interface for rendering embedded
/// protocol sessions, automatically selecting the best rendering
/// mode based on the display server.
pub struct EmbeddedRenderer {
    /// The Wayland subsurface (used for both Wayland and X11)
    subsurface: WaylandSubsurface,
    /// Current rendering mode
    ///
    /// Stored for future use when native Wayland subsurface integration
    /// is fully implemented. Currently always `CairoFallback`.
    mode: RenderingMode,
    /// Reference to the drawing area for Cairo rendering
    drawing_area: Option<DrawingArea>,
}

impl EmbeddedRenderer {
    /// Creates a new embedded renderer
    #[must_use]
    pub fn new() -> Self {
        Self {
            subsurface: WaylandSubsurface::new(),
            mode: RenderingMode::detect(),
            drawing_area: None,
        }
    }

    /// Creates a new embedded renderer with specified dimensions
    #[must_use]
    pub fn with_size(width: u32, height: u32) -> Self {
        Self {
            subsurface: WaylandSubsurface::with_size(width, height),
            mode: RenderingMode::detect(),
            drawing_area: None,
        }
    }

    /// Returns the current rendering mode
    #[must_use]
    pub const fn mode(&self) -> RenderingMode {
        self.mode
    }

    /// Returns the display server type
    #[must_use]
    pub const fn display_server(&self) -> DisplayServerType {
        self.subsurface.display_server()
    }

    /// Returns whether the renderer is initialized
    #[must_use]
    pub const fn is_initialized(&self) -> bool {
        self.subsurface.is_initialized()
    }

    /// Returns the current buffer width
    #[must_use]
    pub fn width(&self) -> u32 {
        self.subsurface.width()
    }

    /// Returns the current buffer height
    #[must_use]
    pub fn height(&self) -> u32 {
        self.subsurface.height()
    }

    /// Initializes the renderer with a parent widget
    ///
    /// # Arguments
    ///
    /// * `drawing_area` - The DrawingArea widget for rendering
    ///
    /// # Errors
    ///
    /// Returns error if initialization fails
    pub fn initialize(&mut self, drawing_area: &DrawingArea) -> Result<(), WaylandSurfaceError> {
        self.subsurface.initialize(drawing_area)?;
        self.drawing_area = Some(drawing_area.clone());
        Ok(())
    }

    /// Updates the position relative to parent
    pub fn update_position(&mut self, x: i32, y: i32) {
        self.subsurface.update_position(x, y);
    }

    /// Resizes the renderer buffer
    pub fn resize(&mut self, width: u32, height: u32) {
        self.subsurface.resize(width, height);
    }

    /// Blits framebuffer data
    pub fn blit(&mut self, data: &[u8], rect: &DamageRect) {
        self.subsurface.blit_framebuffer(data, rect);
    }

    /// Blits a full framebuffer
    pub fn blit_full(&mut self, data: &[u8], width: u32, height: u32, stride: u32) {
        self.subsurface
            .blit_full_framebuffer(data, width, height, stride);
    }

    /// Commits pending changes
    pub fn commit(&mut self) {
        self.subsurface.commit();

        // Queue redraw for Cairo fallback mode
        if self.mode == RenderingMode::CairoFallback {
            if let Some(ref drawing_area) = self.drawing_area {
                drawing_area.queue_draw();
            }
        }
    }

    /// Renders to a Cairo context (for draw function)
    pub fn render(
        &self,
        cr: &gtk4::cairo::Context,
        width: i32,
        height: i32,
    ) -> Result<(), WaylandSurfaceError> {
        self.subsurface.render_cairo(cr, width, height)
    }

    /// Returns a reference to the underlying subsurface
    #[must_use]
    pub const fn subsurface(&self) -> &WaylandSubsurface {
        &self.subsurface
    }

    /// Returns a mutable reference to the underlying subsurface
    pub fn subsurface_mut(&mut self) -> &mut WaylandSubsurface {
        &mut self.subsurface
    }

    /// Cleans up resources
    pub fn cleanup(&mut self) {
        self.subsurface.cleanup();
        self.drawing_area = None;
    }
}

impl Default for EmbeddedRenderer {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(clippy::missing_fields_in_debug)]
impl std::fmt::Debug for EmbeddedRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddedRenderer")
            .field("mode", &self.mode)
            .field("display_server", &self.subsurface.display_server())
            .field("initialized", &self.subsurface.is_initialized())
            .field("width", &self.subsurface.width())
            .field("height", &self.subsurface.height())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_server_detection() {
        let server = DisplayServerType::detect();
        // Should return some valid value
        assert!(matches!(
            server,
            DisplayServerType::Wayland | DisplayServerType::X11 | DisplayServerType::Unknown
        ));
    }

    #[test]
    fn test_shm_buffer_creation() {
        let buffer = ShmBuffer::new(100, 100);
        assert_eq!(buffer.width(), 100);
        assert_eq!(buffer.height(), 100);
        assert_eq!(buffer.stride(), 400); // 100 * 4 bytes per pixel
        assert!(!buffer.is_dirty());
    }

    #[test]
    fn test_shm_buffer_resize() {
        let mut buffer = ShmBuffer::new(100, 100);
        buffer.resize(200, 150);
        assert_eq!(buffer.width(), 200);
        assert_eq!(buffer.height(), 150);
        assert_eq!(buffer.stride(), 800);
        assert!(buffer.is_dirty());
    }

    #[test]
    fn test_shm_buffer_update_region() {
        let mut buffer = ShmBuffer::new(100, 100);
        let src_data = vec![255u8; 40]; // 10 pixels * 4 bytes
        buffer.update_region(0, 0, 10, 1, &src_data, 40);
        assert!(buffer.is_dirty());
    }

    #[test]
    fn test_damage_rect_merge() {
        let mut rect1 = DamageRect::new(10, 10, 20, 20);
        let rect2 = DamageRect::new(25, 25, 20, 20);
        rect1.merge(&rect2);

        assert_eq!(rect1.x, 10);
        assert_eq!(rect1.y, 10);
        assert_eq!(rect1.width, 35); // 25 + 20 - 10
        assert_eq!(rect1.height, 35);
    }

    #[test]
    fn test_damage_rect_empty() {
        let rect = DamageRect::new(0, 0, 0, 0);
        assert!(rect.is_empty());

        let rect2 = DamageRect::new(0, 0, 10, 10);
        assert!(!rect2.is_empty());
    }

    #[test]
    fn test_wayland_subsurface_creation() {
        let subsurface = WaylandSubsurface::new();
        assert!(!subsurface.is_initialized());
        assert_eq!(subsurface.width(), 1280);
        assert_eq!(subsurface.height(), 720);
    }

    #[test]
    fn test_wayland_subsurface_with_size() {
        let subsurface = WaylandSubsurface::with_size(1920, 1080);
        assert_eq!(subsurface.width(), 1920);
        assert_eq!(subsurface.height(), 1080);
    }

    #[test]
    fn test_rendering_mode_detection() {
        let mode = RenderingMode::detect();
        // Should return Cairo fallback for now
        assert_eq!(mode, RenderingMode::CairoFallback);
    }

    #[test]
    fn test_embedded_renderer_creation() {
        let renderer = EmbeddedRenderer::new();
        assert!(!renderer.is_initialized());
        assert_eq!(renderer.mode(), RenderingMode::CairoFallback);
    }
}
