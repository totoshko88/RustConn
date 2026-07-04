//! Pixel buffer and Wayland surface handling for embedded RDP
//!
//! This module re-exports `CairoBackedBuffer` (the zero-copy render surface
//! used by IronRDP embedded mode) and provides `WaylandSurfaceHandle` for
//! Wayland subsurface integration.

use super::types::EmbeddedRdpError;

/// A pixel buffer backed by a persistent Cairo `ImageSurface`.
///
/// Instead of cloning 33MB of pixel data on every draw call (at 4K),
/// this struct owns the underlying byte buffer via Cairo's
/// `ImageSurface::create_for_data()` and provides mutable access
/// through `surface.data()` for in-place updates.
///
/// The Cairo surface is created once and reused across frames.
/// Only `surface.mark_dirty_rectangle()` is needed to tell Cairo
/// which regions changed.
///
/// Re-exported from [`crate::cairo_buffer::CairoBackedBuffer`].
pub use crate::cairo_buffer::CairoBackedBuffer;

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
    fn test_wayland_surface_handle() {
        let mut handle = WaylandSurfaceHandle::new();
        assert!(!handle.is_initialized());

        handle.initialize().unwrap();
        assert!(handle.is_initialized());

        handle.cleanup();
        assert!(!handle.is_initialized());
    }
}
