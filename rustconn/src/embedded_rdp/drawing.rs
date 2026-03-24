//! Drawing setup for the embedded RDP widget
//!
//! Contains the `DrawingArea` draw function and status overlay rendering.

use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

use super::types::{RdpConfig, RdpConnectionState};

impl super::EmbeddedRdpWidget {
    /// Sets up the drawing function for the DrawingArea
    ///
    /// This function handles framebuffer rendering when IronRDP is available,
    /// or shows a status overlay when using FreeRDP external mode.
    ///
    /// # Framebuffer Rendering (Requirement 1.1)
    ///
    /// When in embedded mode with framebuffer data available:
    /// 1. Receives framebuffer updates via event channel
    /// 2. Blits pixel data to Cairo surface
    /// 3. Queues DrawingArea redraw on updates
    ///
    /// The pixel buffer is in BGRA format which matches Cairo's ARGB32 format.
    pub(super) fn setup_drawing(&self) {
        let pixel_buffer = self.pixel_buffer.clone();
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let config = self.config.clone();
        let rdp_width = self.rdp_width.clone();
        let rdp_height = self.rdp_height.clone();

        self.drawing_area
            .set_draw_func(move |area, cr, width, height| {
                let current_state = *state.borrow();
                let embedded = *is_embedded.borrow();

                // Dark background
                cr.set_source_rgb(0.12, 0.12, 0.14);
                let _ = cr.paint();

                // Check if we should render the framebuffer
                // This happens when:
                // 1. We're in embedded mode (IronRDP)
                // 2. We're connected
                // 3. The pixel buffer has valid data
                let should_render_framebuffer =
                    embedded && current_state == RdpConnectionState::Connected && {
                        let buffer = pixel_buffer.borrow();
                        buffer.width() > 0 && buffer.height() > 0 && buffer.has_data()
                    };

                if should_render_framebuffer {
                    // Render the pixel buffer to the DrawingArea
                    // This is the framebuffer rendering path for IronRDP
                    let buffer = pixel_buffer.borrow();
                    let buf_width = buffer.width();
                    let buf_height = buffer.height();

                    // Create a Cairo ImageSurface from the pixel buffer data
                    // The buffer is in BGRA format which matches Cairo's ARGB32
                    let data = buffer.data();
                    if let Ok(surface) = gtk4::cairo::ImageSurface::create_for_data(
                        data.to_vec(),
                        gtk4::cairo::Format::ARgb32,
                        crate::utils::dimension_to_i32(buf_width),
                        crate::utils::dimension_to_i32(buf_height),
                        crate::utils::stride_to_i32(buffer.stride()),
                    ) {
                        // HiDPI fix: The pixel buffer is in device pixels (e.g. 1920×1080
                        // on a 2× display where the widget is 960×540 CSS pixels).
                        // Tell Cairo the surface is already at device resolution so it
                        // doesn't double-scale (CSS→device) through bilinear interpolation,
                        // which causes blurry output.
                        let effective_scale = config.borrow().as_ref().map_or_else(
                            || f64::from(area.scale_factor().max(1)),
                            |c| c.scale_override.effective_scale(area.scale_factor()),
                        );
                        surface.set_device_scale(effective_scale, effective_scale);

                        // Scale to fit the drawing area while maintaining aspect ratio.
                        // After set_device_scale, Cairo treats the surface dimensions in
                        // CSS pixels (buf_width/scale × buf_height/scale), so we compute
                        // the scale ratio in CSS space directly.
                        let css_buf_w = f64::from(buf_width) / effective_scale;
                        let css_buf_h = f64::from(buf_height) / effective_scale;
                        let scale_x = f64::from(width) / css_buf_w;
                        let scale_y = f64::from(height) / css_buf_h;
                        let scale = scale_x.min(scale_y);

                        // Center the image
                        let offset_x = css_buf_w.mul_add(-scale, f64::from(width)) / 2.0;
                        let offset_y = css_buf_h.mul_add(-scale, f64::from(height)) / 2.0;

                        // Save the current transformation matrix
                        if let Err(e) = cr.save() {
                            tracing::warn!(error = %e, "Cairo save failed");
                        }

                        cr.translate(offset_x, offset_y);
                        cr.scale(scale, scale);
                        let _ = cr.set_source_surface(&surface, 0.0, 0.0);

                        // Use adaptive filtering: Nearest for 1:1 pixel mapping (sharp),
                        // Bilinear when actual scaling is needed (smooth).
                        let filter = if (scale - 1.0).abs() < 0.01 {
                            gtk4::cairo::Filter::Nearest
                        } else {
                            gtk4::cairo::Filter::Bilinear
                        };
                        cr.source().set_filter(filter);

                        let _ = cr.paint();

                        // Restore the transformation matrix
                        if let Err(e) = cr.restore() {
                            tracing::warn!(error = %e, "Cairo restore failed");
                        }
                    }
                } else {
                    // Show status overlay when not rendering framebuffer
                    // This is used for:
                    // - FreeRDP external mode (always)
                    // - IronRDP before connection is established
                    // - IronRDP when no framebuffer data is available
                    Self::draw_status_overlay(
                        cr,
                        width,
                        height,
                        current_state,
                        embedded,
                        &config,
                        &rdp_width,
                        &rdp_height,
                    );
                }
            });
    }

    /// Draws the status overlay when not rendering framebuffer
    ///
    /// This shows connection status, host information, and hints to the user.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_status_overlay(
        cr: &gtk4::cairo::Context,
        width: i32,
        height: i32,
        current_state: RdpConnectionState,
        embedded: bool,
        config: &Rc<RefCell<Option<RdpConfig>>>,
        rdp_width: &Rc<RefCell<u32>>,
        rdp_height: &Rc<RefCell<u32>>,
    ) {
        crate::embedded_rdp::ui::draw_status_overlay(
            cr,
            width,
            height,
            current_state,
            embedded,
            config,
            rdp_width,
            rdp_height,
        );
    }
}
