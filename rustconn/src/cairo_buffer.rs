//! Persistent Cairo-backed pixel buffer for zero-copy rendering.
//!
//! Instead of cloning pixel data on every draw call, this struct owns
//! the underlying byte buffer via Cairo's `ImageSurface::create_for_data()`
//! and provides mutable access through `surface.data()` for in-place updates.
//!
//! The Cairo surface is created once and reused across frames.
//! Only `surface.mark_dirty_rectangle()` is needed to tell Cairo
//! which regions changed.
//!
//! Used by embedded RDP, VNC, and SPICE widgets.

/// A pixel buffer backed by a persistent Cairo `ImageSurface`.
pub struct CairoBackedBuffer {
    surface: Option<gtk4::cairo::ImageSurface>,
    width: u32,
    height: u32,
    stride: u32,
    has_data: bool,
}

impl CairoBackedBuffer {
    /// Creates a new Cairo-backed buffer with the specified dimensions.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        let stride = width * 4;
        let mut buf = Self {
            surface: None,
            width,
            height,
            stride,
            has_data: false,
        };
        buf.ensure_surface();
        buf
    }

    /// Lazily creates the Cairo `ImageSurface` if it doesn't exist yet.
    fn ensure_surface(&mut self) {
        if self.surface.is_some() || self.width == 0 || self.height == 0 {
            return;
        }
        let size = (self.stride * self.height) as usize;
        let data = vec![0u8; size];
        match gtk4::cairo::ImageSurface::create_for_data(
            data,
            gtk4::cairo::Format::ARgb32,
            crate::utils::dimension_to_i32(self.width),
            crate::utils::dimension_to_i32(self.height),
            crate::utils::stride_to_i32(self.stride),
        ) {
            Ok(s) => {
                self.surface = Some(s);
            }
            Err(e) => {
                tracing::warn!("Failed to create Cairo surface: {e}");
            }
        }
    }

    /// Returns the buffer width.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the buffer height.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns the stride (bytes per row).
    #[must_use]
    pub const fn stride(&self) -> u32 {
        self.stride
    }

    /// Returns whether the buffer has received any frame data.
    #[must_use]
    pub const fn has_data(&self) -> bool {
        self.has_data
    }

    /// Returns a reference to the underlying `ImageSurface`, if available.
    #[must_use]
    pub fn surface(&self) -> Option<&gtk4::cairo::ImageSurface> {
        self.surface.as_ref()
    }

    /// Updates a rectangular region of the surface's pixel data in-place.
    ///
    /// After writing, calls `mark_dirty_rectangle` so Cairo knows which
    /// area needs to be re-composited.
    #[expect(
        clippy::many_single_char_names,
        reason = "matrix/vector arithmetic uses canonical short names from the linear algebra literature"
    )]
    pub fn update_region(
        &mut self,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        src_data: &[u8],
        src_stride: u32,
    ) {
        let Some(ref mut surface) = self.surface else {
            return;
        };

        let mut data = match surface.data() {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("Failed to lock surface data: {e}");
                return;
            }
        };

        let dst_stride = self.stride as usize;
        let src_stride_usize = src_stride as usize;
        let bpp = 4;

        for row in 0..h {
            let dst_y = (y + row) as usize;
            if dst_y >= self.height as usize {
                break;
            }

            let x_off = x as usize * bpp;
            if x_off >= dst_stride {
                continue;
            }

            let dst_off = dst_y * dst_stride + x_off;
            let src_off = row as usize * src_stride_usize;
            let copy_w = (w as usize * bpp).min(dst_stride - x_off);

            if copy_w > 0 && src_off + copy_w <= src_data.len() && dst_off + copy_w <= data.len() {
                data[dst_off..dst_off + copy_w]
                    .copy_from_slice(&src_data[src_off..src_off + copy_w]);
            }
        }

        drop(data);
        surface.mark_dirty_rectangle(x as i32, y as i32, w as i32, h as i32);
        self.has_data = true;
    }

    /// Recreates the surface when dimensions change.
    pub fn resize(&mut self, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return;
        }
        self.width = width;
        self.height = height;
        self.stride = width * 4;
        self.has_data = false;
        self.surface = None;
        self.ensure_surface();
    }

    /// Clears the buffer to black (zeros) and marks the entire surface dirty.
    pub fn clear(&mut self) {
        if let Some(ref mut surface) = self.surface {
            {
                if let Ok(mut data) = surface.data() {
                    data.fill(0);
                }
            }
            surface.mark_dirty();
        }
        self.has_data = false;
    }

    /// Fills the buffer with a solid colour (used for resize placeholder).
    ///
    /// Each pixel is written as `[b, g, r, a]` in BGRA order.
    pub fn fill_solid(&mut self, b: u8, g: u8, r: u8, a: u8) {
        if let Some(ref mut surface) = self.surface {
            {
                if let Ok(mut data) = surface.data() {
                    for chunk in data.chunks_exact_mut(4) {
                        chunk[0] = b;
                        chunk[1] = g;
                        chunk[2] = r;
                        chunk[3] = a;
                    }
                }
            }
            surface.mark_dirty();
        }
        self.has_data = true;
    }
}
