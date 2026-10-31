//! Drawing setup for the embedded RDP widget
//!
//! Contains the `DrawingArea` draw function and status overlay rendering.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use super::types::{RdpConfig, RdpConnectionState};

/// Opacity of the wash drawn over a frame that may no longer reflect the server.
///
/// Enough that the change is unmistakable at a glance, light enough that the
/// user can still recognise what was on screen before the machine slept.
const STALE_FRAME_DIM_ALPHA: f64 = 0.55;

/// How far a magnification may stray from a whole number and still count as
/// one; absorbs the rounding in fractional compositor scales.
const WHOLE_MAGNIFICATION_TOLERANCE: f64 = 0.01;

/// The filter for drawing the framebuffer at `magnification` device pixels per
/// remote pixel.
///
/// Nearest at a whole magnification — every remote pixel becomes an even
/// block, so text stays sharp and the paint stays cheap. Bilinear otherwise,
/// because at 1.25× Nearest doubles every fourth row and column and makes
/// strokes uneven, which reads worse than the slight softness of bilinear.
fn upscale_filter(magnification: f64) -> gtk4::cairo::Filter {
    if (magnification - magnification.round()).abs() <= WHOLE_MAGNIFICATION_TOLERANCE {
        gtk4::cairo::Filter::Nearest
    } else {
        gtk4::cairo::Filter::Bilinear
    }
}

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
        let stale_frame = self.stale.clone();
        let is_embedded = self.is_embedded.clone();
        let config = self.config.clone();
        let rdp_width = self.rdp_width.clone();
        let rdp_height = self.rdp_height.clone();

        // Cache the last effective scale to avoid calling set_device_scale on
        // every draw frame when the scale has not changed. Cairo internally
        // invalidates pattern caches when device_scale is set, even to the
        // same value, so this avoids a measurable per-frame overhead.
        let cached_scale = std::cell::Cell::new(0.0_f64);

        self.drawing_area
            .set_draw_func(move |area, cr, width, height| {
                let current_state = *state.borrow();
                let embedded = *is_embedded.borrow();

                // Check if we should render the framebuffer
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
                        let display_scale = super::widget_fractional_scale(area);
                        let effective_scale = config
                            .borrow()
                            .as_ref()
                            .map_or(1.0, |c| c.scale_override.resolved_scale(display_scale));

                        // Only call set_device_scale when it actually changed — Cairo
                        // internally invalidates pattern caches on every call regardless
                        // of whether the value changed.
                        if (effective_scale - cached_scale.get()).abs() > f64::EPSILON {
                            surface.set_device_scale(effective_scale, effective_scale);
                            cached_scale.set(effective_scale);
                        }

                        // Scale to fit the drawing area while maintaining aspect ratio.
                        let css_buf_w = f64::from(buf_width) / effective_scale;
                        let css_buf_h = f64::from(buf_height) / effective_scale;
                        let scale_x = f64::from(width) / css_buf_w;
                        let scale_y = f64::from(height) / css_buf_h;
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

                        // Only paint a dark background if the framebuffer does not
                        // cover the full widget — avoids a redundant full-surface
                        // paint when the image fills the drawing area 1:1.
                        let covers_full_area = offset_x.abs() < 1.0
                            && offset_y.abs() < 1.0
                            && (scale - 1.0).abs() < 0.01;
                        if !covers_full_area {
                            cr.set_source_rgb(0.12, 0.12, 0.14);
                            let _ = cr.paint();
                        }

                        if let Err(e) = cr.save() {
                            tracing::warn!(error = %e, "Cairo save failed");
                        }

                        cr.translate(offset_x, offset_y);
                        cr.scale(scale, scale);
                        let _ = cr.set_source_surface(surface, 0.0, 0.0);

                        // Nearest keeps text razor-sharp when each remote pixel
                        // lands on a whole number of device pixels — 1:1 with
                        // Native, 2:1 with Auto on a 2× display. At a fractional
                        // magnification, such as Auto on a 125% or 150% display,
                        // it repeats every fourth or every second row and column,
                        // so strokes come out uneven; bilinear spreads the
                        // difference evenly instead.
                        cr.source()
                            .set_filter(upscale_filter(scale * display_scale / effective_scale));

                        let _ = cr.paint();

                        if let Err(e) = cr.restore() {
                            tracing::warn!(error = %e, "Cairo restore failed");
                        }

                        // The machine resumed from sleep and this frame may be
                        // minutes old. Wash it out so a frozen desktop is never
                        // mistaken for a live one (issue #248).
                        if stale_frame.get() {
                            cr.set_source_rgba(0.12, 0.12, 0.14, STALE_FRAME_DIM_ALPHA);
                            let _ = cr.paint();
                        }
                    }
                } else {
                    // Themed background for non-framebuffer status states (no
                    // video yet) — follows light/dark theme so it is not a black
                    // rectangle on a light desktop. (The letterbox fill above and
                    // the stale-frame wash stay fixed-dark on purpose: they frame
                    // or dim actual video, where a dark border is standard.)
                    let (bg, _) = crate::utils::placeholder_surface_colors();
                    cr.set_source_rgb(bg.0, bg.1, bg.2);
                    let _ = cr.paint();

                    // Show status overlay when not rendering framebuffer
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

#[cfg(test)]
mod tests {
    use gtk4::cairo::Filter;

    use super::upscale_filter;

    #[test]
    fn whole_magnifications_stay_nearest() {
        // Native on any display, Auto at 100% and at 200%
        for magnification in [1.0, 2.0, 3.0, 1.004, 1.996] {
            assert_eq!(
                upscale_filter(magnification),
                Filter::Nearest,
                "{magnification}"
            );
        }
    }

    #[test]
    fn fractional_magnifications_are_smoothed() {
        // Auto at 125%, 150% and 175%, and a letterboxed frame mid-resize
        for magnification in [1.25, 1.5, 1.75, 0.8, 1.1] {
            assert_eq!(
                upscale_filter(magnification),
                Filter::Bilinear,
                "{magnification}"
            );
        }
    }
}
