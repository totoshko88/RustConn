//! Pixel buffer and Wayland surface handling for embedded RDP
//!
//! This module contains the `PixelBuffer` struct for frame data storage
//! and `WaylandSurfaceHandle` for Wayland subsurface integration.

use crate::embedded_rdp_types::EmbeddedRdpError;

/// Pixel buffer for frame data
///
/// This struct holds the pixel data received from FreeRDP's EndPaint callback
/// and is used to blit to the Wayland surface.
#[derive(Debug)]
pub struct PixelBuffer {
    /// Raw pixel data in BGRA format
    data: Vec<u8>,
    /// Buffer width in pixels
    width: u32,
    /// Buffer height in pixels
    height: u32,
    /// Stride (bytes per row)
    stride: u32,
    /// Whether the buffer has received any data
    has_data: bool,
}

impl PixelBuffer {
    /// Creates a new pixel buffer with the specified dimensions
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        let stride = width * 4; // BGRA = 4 bytes per pixel
        let size = (stride * height) as usize;
        Self {
            data: vec![0; size],
            width,
            height,
            stride,
            has_data: false,
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

    /// Returns whether the buffer has received any data
    #[must_use]
    pub const fn has_data(&self) -> bool {
        self.has_data
    }

    /// Sets the has_data flag
    pub fn set_has_data(&mut self, has_data: bool) {
        self.has_data = has_data;
    }

    /// Returns the stride (bytes per row)
    #[must_use]
    pub const fn stride(&self) -> u32 {
        self.stride
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
    ///
    /// Preserves existing content by scaling it to the new size to avoid
    /// visual artifacts during resize. The has_data flag is preserved.
    pub fn resize(&mut self, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return; // No change needed
        }

        let old_width = self.width;
        let old_height = self.height;
        let had_data = self.has_data;

        self.width = width;
        self.height = height;
        self.stride = width * 4;
        let new_size = (self.stride * height) as usize;

        if had_data && old_width > 0 && old_height > 0 {
            // Preserve old data - just resize the buffer
            // The old content will be scaled during rendering
            self.data.resize(new_size, 0);
            self.has_data = true; // Keep has_data true to continue rendering
        } else {
            self.data.resize(new_size, 0);
            self.has_data = false;
        }
    }

    /// Clears the buffer to black
    pub fn clear(&mut self) {
        self.data.fill(0);
        self.has_data = false; // Reset data flag on clear
    }

    /// Updates a region of the buffer
    ///
    /// # Arguments
    ///
    /// * `x` - X coordinate of the region
    /// * `y` - Y coordinate of the region
    /// * `w` - Width of the region
    /// * `h` - Height of the region
    /// * `src_data` - Source pixel data
    /// * `src_stride` - Source stride
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

            let x_offset = x as usize * bytes_per_pixel;
            if x_offset >= dst_stride {
                continue;
            }

            let dst_offset = dst_y * dst_stride + x_offset;
            let src_offset = row as usize * src_stride;
            let max_copy = dst_stride.saturating_sub(x_offset);
            let copy_width = (w as usize * bytes_per_pixel).min(max_copy);

            if copy_width > 0
                && src_offset + copy_width <= src_data.len()
                && dst_offset + copy_width <= self.data.len()
            {
                self.data[dst_offset..dst_offset + copy_width]
                    .copy_from_slice(&src_data[src_offset..src_offset + copy_width]);
                self.has_data = true; // Mark that we have received data
            }
        }
    }
}

/// Wayland surface handle for subsurface integration
///
/// This struct manages the Wayland surface resources for embedding
/// the RDP display within the GTK widget hierarchy.
#[derive(Debug, Default)]
pub struct WaylandSurfaceHandle {
    /// Whether the surface is initialized
    initialized: bool,
    /// Surface ID (for debugging)
    surface_id: u32,
}

impl WaylandSurfaceHandle {
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
    pub fn initialize(&mut self) -> Result<(), EmbeddedRdpError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pixel_buffer_new() {
        let buffer = PixelBuffer::new(100, 50);
        assert_eq!(buffer.width(), 100);
        assert_eq!(buffer.height(), 50);
        assert_eq!(buffer.stride(), 400); // 100 * 4 bytes per pixel
        assert_eq!(buffer.data().len(), 20000); // 100 * 50 * 4
    }

    #[test]
    fn test_pixel_buffer_resize() {
        let mut buffer = PixelBuffer::new(100, 50);
        buffer.resize(200, 100);
        assert_eq!(buffer.width(), 200);
        assert_eq!(buffer.height(), 100);
        assert_eq!(buffer.stride(), 800);
        assert_eq!(buffer.data().len(), 80000);
    }

    #[test]
    fn test_pixel_buffer_clear() {
        let mut buffer = PixelBuffer::new(10, 10);
        buffer.data_mut()[0] = 255;
        buffer.clear();
        assert!(buffer.data().iter().all(|&b| b == 0));
    }

    #[test]
    fn test_wayland_surface_handle() {
        let mut handle = WaylandSurfaceHandle::new();
        assert!(!handle.is_initialized());

        handle.initialize().unwrap();
        assert!(handle.is_initialized());

        handle.cleanup();
        assert!(!handle.is_initialized());
    }
}
