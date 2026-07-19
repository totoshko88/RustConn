//! Drawing setup for the embedded RDP widget
//!
//! Contains the `DrawingArea` draw function and status overlay rendering.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use super::types::{RdpConfig, RdpConnectionState};

impl super::EmbeddedRdpWidget {
    /// Sets up the drawing function for the DrawingArea
    ///
    /// This function handles framebuffer rendering when IronRDP is available,
    /// or shows a status overlay when using FreeRDP external mode.
    ///
    /// # Framebuffer Rendering
    ///
    /// When in embedded mode with framebuffer data available:
    /// 1. Receives framebuffer updates via event channel
    /// 2. Blits pixel data to Cairo surface
    /// 3. Queues DrawingArea redraw on updates
    ///
    /// The pixel buffer is in BGRA format which matches Cairo's ARGB32 format.
    pub(super) fn setup_drawing(&self) {
        let cairo_buffer = self.cairo_buffer.clone();
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
                        let buffer = cairo_buffer.borrow();
                        buffer.width() > 0 && buffer.height() > 0 && buffer.has_data()
                    };

                if should_render_framebuffer {
                    // Fast path: use the persistent Cairo surface (zero-copy)
                    let buffer = cairo_buffer.borrow();
                    let buf_width = buffer.width();
                    let buf_height = buffer.height();

                    if let Some(surface) = buffer.surface() {
                        // HiDPI fix: The pixel buffer is in device pixels (e.g. 1920×1080
                        // on a 2× display where the widget is 960×540 CSS pixels).
                        // Tell Cairo the surface is already at device resolution so it
                        // doesn't double-scale (CSS→device) through bilinear interpolation,
                        // which causes blurry output.
                        let effective_scale = config.borrow().as_ref().map_or(1.0, |c| {
                            c.scale_override
                                .resolved_scale(super::widget_fractional_scale(area))
                        });
                        surface.set_device_scale(effective_scale, effective_scale);

                        // Scale to fit the drawing area while maintaining aspect ratio.
                        // After set_device_scale, Cairo treats the surface dimensions in
                        // CSS pixels (buf_width/scale × buf_height/scale), so we compute
                        // the scale ratio in CSS space directly.
                        let css_buf_w = f64::from(buf_width) / effective_scale;
                        let css_buf_h = f64::from(buf_height) / effective_scale;
                        let scale_x = f64::from(width) / css_buf_w;
                        let scale_y = f64::from(height) / css_buf_h;
                        // Within the match slack of the drawing area (the ≤1px
                        // even-rounding residual): blit 1:1 for a sharp border instead
                        // of a sub-pixel rescale. Larger mismatches (a resize in flight)
                        // still scale-to-fit with aspect preserved.
                        let slack = f64::from(super::DESKTOP_MATCH_SLACK_PX);
                        let scale = if (css_buf_w - f64::from(width)).abs() <= slack
                            && (css_buf_h - f64::from(height)).abs() <= slack
                        {
                            1.0
                        } else {
                            scale_x.min(scale_y)
                        };

                        // Center the image
                        let offset_x = css_buf_w.mul_add(-scale, f64::from(width)) / 2.0;
                        let offset_y = css_buf_h.mul_add(-scale, f64::from(height)) / 2.0;

                        // Save the current transformation matrix
                        if let Err(e) = cr.save() {
                            tracing::warn!(error = %e, "Cairo save failed");
                        }

                        cr.translate(offset_x, offset_y);
                        cr.scale(scale, scale);
                        let _ = cr.set_source_surface(surface, 0.0, 0.0);

                        // Use adaptive filtering: Nearest for 1:1 pixel mapping (sharp),
                        // Good for downscale (better quality than Bilinear when
                        // effective_scale > 1.0 shrinks the image), Bilinear otherwise.
                        let filter = if (scale - 1.0).abs() < 0.01 {
                            gtk4::cairo::Filter::Nearest
                        } else if scale < 1.0 {
                            // Downscale: use Good for sharper text at reduced size
                            gtk4::cairo::Filter::Good
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
    #[expect(
        clippy::too_many_arguments,
        reason = "function parameters mirror upstream API or struct fields 1:1; bundling into a struct only restates the field list"
    )]
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
