//! Resize handler for the embedded RDP widget
//!
//! Contains debounced resize logic that triggers dynamic resolution changes
//! via the Display Control Channel (MS-RDPEDISP) without reconnecting.
//!
//! ## How it works
//!
//! When the widget is resized:
//! 1. The current image is immediately scaled to fit (visual feedback)
//! 2. After 500ms of no further resize, a `SetDesktopSize` command is sent
//!    via the Display Control Channel (DVC)
//! 3. The server responds with a new resolution and the session continues
//!    seamlessly — no disconnect/reconnect cycle
//!
//! If the server does not support Display Control (e.g. Windows Server 2008),
//! `encode_resize` returns `None` and we fall back to a full reconnect.

use gtk4::glib;
use gtk4::prelude::*;

use super::types::RdpConnectionState;

use crate::i18n::i18n;

/// Minimum pixel difference (in device pixels) before triggering an RDP
/// resolution change on widget resize. Prevents unnecessary resize requests
/// from minor layout adjustments.
const RESIZE_THRESHOLD_PX: u32 = 50;

/// Minimum change in the widget's *logical* (CSS) size, in pixels, before a
/// settled resize issues a new remote-resolution request.
///
/// A resolution change can nudge the widget's allocation by a handful of pixels
/// (toolbar reflow, tab re-measure). Without this dead zone that nudge would be
/// read as a fresh resize and request another resolution, which nudges again —
/// an endless ping-pong (observed as the window "breathing" on small sizes).
/// Comfortably larger than the observed few-pixel jitter, small enough that a
/// real user drag still registers.
const RESIZE_HYSTERESIS_CSS_PX: u32 = 48;

#[cfg(feature = "rdp-embedded")]
use rustconn_core::rdp_client::RdpClientCommand;

impl super::EmbeddedRdpWidget {
    /// Sets up the resize handler with debounced dynamic resolution change
    ///
    /// When the widget is resized, we:
    /// 1. Immediately scale the current image to fit
    /// 2. After 500ms of no resize, send `SetDesktopSize` via Display Control Channel
    /// 3. If Display Control is unavailable, fall back to reconnect
    #[cfg(feature = "rdp-embedded")]
    pub(super) fn setup_resize_handler(&self) {
        let width = self.width.clone();
        let height = self.height.clone();
        let rdp_width = self.rdp_width.clone();
        let rdp_height = self.rdp_height.clone();
        let state = self.state.clone();
        let reconnect_timer = self.reconnect_timer.clone();
        let config = self.config.clone();
        let ironrdp_tx = self.ironrdp_command_tx.clone();
        let status_label = self.status_label.clone();
        let on_reconnect = self.on_reconnect.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        let last_request_css = self.last_resize_request_css.clone();

        let handler_id = self
            .drawing_area
            .connect_resize(move |area, new_width, new_height| {
                // Store CSS pixel dimensions for mouse coordinate transform.
                // GTK mouse events use CSS coordinates, and the draw function
                // also operates in CSS space, so self.width/height must match.
                let css_width = new_width.unsigned_abs();
                let css_height = new_height.unsigned_abs();

                // Requested resolution = logical widget size × scale multiplier
                // (Auto = 1.0×, i.e. logical — keeps the network payload small;
                // Native follows the display scale for a full-resolution image).
                let effective_scale = config.borrow().as_ref().map_or(1.0, |c| {
                    c.scale_override
                        .resolved_scale(f64::from(area.scale_factor()))
                });
                #[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "value range fits the target type and is non-negative by construction in this code path"
)]
                let device_width = (f64::from(css_width) * effective_scale) as u32;
                #[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "value range fits the target type and is non-negative by construction in this code path"
)]
                let device_height = (f64::from(css_height) * effective_scale) as u32;

                tracing::debug!(
                    "[RDP Resize] Widget resized to {}x{} CSS ({}x{} device) (RDP: {}x{})",
                    css_width,
                    css_height,
                    device_width,
                    device_height,
                    *rdp_width.borrow(),
                    *rdp_height.borrow()
                );

                // Store CSS dimensions for coordinate transform
                *width.borrow_mut() = css_width;
                *height.borrow_mut() = css_height;

                // Queue redraw for scaling - the draw function handles aspect ratio
                area.queue_draw();

                // Only request resolution change if connected
                let current_state = *state.borrow();
                if current_state != RdpConnectionState::Connected {
                    return;
                }

                // Cancel any pending resize timer
                if let Some(source_id) = reconnect_timer.borrow_mut().take() {
                    source_id.remove();
                }

                // Schedule resolution change after 500ms of no resize
                let rdp_w = rdp_width.clone();
                let rdp_h = rdp_height.clone();
                let timer = reconnect_timer.clone();
                let cfg = config.clone();
                let tx = ironrdp_tx.clone();
                let sl = status_label.clone();
                let reconnect_cb = on_reconnect.clone();
                let last_req = last_request_css.clone();
                let using_ironrdp = *is_ironrdp.borrow();
                let force_reconnect = config
                    .borrow()
                    .as_ref()
                    .is_some_and(|c| c.reconnect_on_resize);

                let source_id = glib::timeout_add_local_once(
                    std::time::Duration::from_millis(500),
                    move || {
                        // Clear the timer reference
                        timer.borrow_mut().take();

                        let current_rdp_w = *rdp_w.borrow();
                        let current_rdp_h = *rdp_h.borrow();

                        // Adaptive resolution (R13.1, R13.2): ask the pure core
                        // helper for a >=min, aspect-preserving, even-dimensioned
                        // request. For a small window (logical < 640x480) it
                        // returns a larger desktop at a fixed 100% DPI so the
                        // viewer downscales the frame locally — dense content,
                        // normal-sized cursor.
                        #[expect(
                            clippy::cast_possible_truncation,
                            reason = "RDP scale percent is a small value (100–300) that fits u16"
                        )]
                        let base_scale_percent = super::rdp_scale_percent(effective_scale) as u16;
                        let req = rustconn_core::display_geometry::desktop_request_for_area(
                            device_width,
                            device_height,
                            640,
                            480,
                            base_scale_percent,
                        );

                        // Preserve the MS-RDPEDISP max-desktop ceiling
                        // (round_rdp_desktop also re-affirms even dimensions).
                        let (rounded_width, rounded_height) =
                            super::round_rdp_desktop(req.width, req.height);

                        // Feedback-loop guard: act only when the *logical* widget
                        // size actually moved beyond the hysteresis since our last
                        // request AND the resulting resolution really differs from
                        // the current one. Comparing the request (not the raw
                        // widget size) to the current resolution is essential:
                        // a small window requests a 2×/3× desktop, so the old
                        // "widget vs resolution" check was always above threshold
                        // and re-fired on every layout nudge — an endless loop.
                        let logical_moved = last_req.borrow().is_none_or(|(lw, lh)| {
                            css_width.abs_diff(lw) >= RESIZE_HYSTERESIS_CSS_PX
                                || css_height.abs_diff(lh) >= RESIZE_HYSTERESIS_CSS_PX
                        });
                        let resolution_changed = rounded_width.abs_diff(current_rdp_w)
                            > RESIZE_THRESHOLD_PX
                            || rounded_height.abs_diff(current_rdp_h) > RESIZE_THRESHOLD_PX;

                        if logical_moved && resolution_changed {
                            *last_req.borrow_mut() = Some((css_width, css_height));

                            // Update config with new resolution
                            {
                                let current_config = cfg.borrow().clone();
                                if let Some(mut config) = current_config {
                                    config = config.with_resolution(rounded_width, rounded_height);
                                    *cfg.borrow_mut() = Some(config);
                                }
                            }

                            if using_ironrdp && !force_reconnect {
                                // IronRDP path: use Display Control Channel for
                                // seamless resize without reconnect (MS-RDPEDISP)
                                let w = rounded_width as u16;
                                let h = rounded_height as u16;

                                if let Some(ref sender) = *tx.borrow() {
                                    let _ = sender.send(RdpClientCommand::SetDesktopSize {
                                        width: w,
                                        height: h,
                                        // Helper DPI: 100% for a small window
                                        // (dense, normal cursor), display DPI otherwise.
                                        scale_percent: Some(u32::from(req.scale_percent)),
                                    });
                                }

                                tracing::info!(
                                    "[RDP Resize] Dynamic resize via Display Control: \
                                     {}x{} -> {}x{} (rounded from {}x{})",
                                    current_rdp_w,
                                    current_rdp_h,
                                    rounded_width,
                                    rounded_height,
                                    device_width,
                                    device_height
                                );

                                // Brief status indicator
                                sl.set_text(&i18n("Resizing…"));
                                sl.set_visible(true);
                                let sl_hide = sl.clone();
                                glib::timeout_add_local_once(
                                    std::time::Duration::from_secs(2),
                                    move || {
                                        sl_hide.set_visible(false);
                                    },
                                );
                            } else {
                                // FreeRDP external path: must reconnect (no DVC access)
                                tracing::info!(
                                    "[RDP Resize] Reconnecting (FreeRDP) with new resolution: \
                                     {}x{} -> {}x{} (rounded from {}x{})",
                                    current_rdp_w,
                                    current_rdp_h,
                                    rounded_width,
                                    rounded_height,
                                    device_width,
                                    device_height
                                );

                                // Disconnect current session
                                if let Some(ref sender) = *tx.borrow() {
                                    let _ = sender.send(RdpClientCommand::Disconnect);
                                }

                                // Show reconnecting status
                                sl.set_text(&i18n("Reconnecting..."));
                                sl.set_visible(true);

                                // Trigger reconnect via callback after short delay
                                let reconnect_cb_clone = reconnect_cb.clone();
                                glib::timeout_add_local_once(
                                    std::time::Duration::from_millis(500),
                                    move || {
                                        if let Some(ref callback) = *reconnect_cb_clone.borrow() {
                                            callback();
                                        }
                                    },
                                );
                            }
                        }
                    },
                );

                *reconnect_timer.borrow_mut() = Some(source_id);
            });
        *self.resize_handler_id.borrow_mut() = Some(handler_id);
    }

    /// Forces an immediate RDP resolution change to match the current widget
    /// size, bypassing the debounced resize handler.
    ///
    /// This is the action behind the toolbar "Fit resolution to window" button.
    /// It covers the edge case where the window was resized between connection
    /// init and the session becoming active (so the desktop is not using the
    /// whole window), or any time the user wants to re-request the resolution.
    ///
    /// Uses the Display Control Channel (MS-RDPEDISP) for a seamless change
    /// when available; otherwise falls back to a full reconnect.
    #[cfg(feature = "rdp-embedded")]
    pub fn request_resolution_sync(&self) {
        Self::apply_resolution_sync(
            &self.drawing_area,
            &self.state,
            &self.config,
            &self.is_ironrdp,
            &self.ironrdp_command_tx,
            &self.status_label,
            &self.on_reconnect,
        );
    }

    /// Connects the toolbar "Fit resolution to window" button to the
    /// resolution-sync logic.
    #[cfg(feature = "rdp-embedded")]
    pub(super) fn setup_fit_resolution_button(&self, button: &gtk4::Button) {
        let drawing_area = self.drawing_area.clone();
        let state = self.state.clone();
        let config = self.config.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        let ironrdp_tx = self.ironrdp_command_tx.clone();
        let status_label = self.status_label.clone();
        let on_reconnect = self.on_reconnect.clone();
        button.connect_clicked(move |_| {
            Self::apply_resolution_sync(
                &drawing_area,
                &state,
                &config,
                &is_ironrdp,
                &ironrdp_tx,
                &status_label,
                &on_reconnect,
            );
        });
    }

    /// Fallback button setup when `rdp-embedded` is disabled (no IronRDP channel).
    #[cfg(not(feature = "rdp-embedded"))]
    pub(super) fn setup_fit_resolution_button(&self, _button: &gtk4::Button) {}

    /// Core resolution-sync logic shared by the toolbar button and the public
    /// [`request_resolution_sync`](Self::request_resolution_sync) API.
    #[cfg(feature = "rdp-embedded")]
    fn apply_resolution_sync(
        drawing_area: &gtk4::DrawingArea,
        state: &std::rc::Rc<std::cell::RefCell<RdpConnectionState>>,
        config: &std::rc::Rc<std::cell::RefCell<Option<super::types::RdpConfig>>>,
        is_ironrdp: &std::rc::Rc<std::cell::RefCell<bool>>,
        ironrdp_tx: &std::rc::Rc<
            std::cell::RefCell<Option<std::sync::mpsc::Sender<RdpClientCommand>>>,
        >,
        status_label: &gtk4::Label,
        on_reconnect: &std::rc::Rc<std::cell::RefCell<Option<Box<dyn Fn() + 'static>>>>,
    ) {
        if *state.borrow() != RdpConnectionState::Connected {
            return;
        }

        let css_width = drawing_area.width().unsigned_abs();
        let css_height = drawing_area.height().unsigned_abs();

        let effective_scale = config.borrow().as_ref().map_or(1.0, |c| {
            c.scale_override
                .resolved_scale(f64::from(drawing_area.scale_factor()))
        });
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        let device_width = (f64::from(css_width) * effective_scale) as u32;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "value range fits the target type and is non-negative by construction in this code path"
        )]
        let device_height = (f64::from(css_height) * effective_scale) as u32;

        // Adaptive resolution (R13.1, R13.2): the pure core helper returns a
        // >=min, aspect-preserving, even-dimensioned request. A small window
        // (logical < 640x480) gets a larger desktop at a fixed 100% DPI,
        // downscaled locally by the viewer — dense content, normal-sized cursor.
        #[expect(
            clippy::cast_possible_truncation,
            reason = "RDP scale percent is a small value (100–300) that fits u16"
        )]
        let base_scale_percent = super::rdp_scale_percent(effective_scale) as u16;
        let req = rustconn_core::display_geometry::desktop_request_for_area(
            device_width,
            device_height,
            640,
            480,
            base_scale_percent,
        );

        // Preserve the MS-RDPEDISP max-desktop ceiling (round_rdp_desktop also
        // re-affirms even dimensions).
        let (rounded_width, rounded_height) = super::round_rdp_desktop(req.width, req.height);

        // Persist the new resolution in the config
        {
            let current_config = config.borrow().clone();
            if let Some(mut cfg) = current_config {
                cfg = cfg.with_resolution(rounded_width, rounded_height);
                *config.borrow_mut() = Some(cfg);
            }
        }

        let using_ironrdp = *is_ironrdp.borrow();
        let force_reconnect = config
            .borrow()
            .as_ref()
            .is_some_and(|c| c.reconnect_on_resize);

        if using_ironrdp && !force_reconnect {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "RDP resolution is clamped well below u16::MAX in this code path"
            )]
            let w = rounded_width as u16;
            #[expect(
                clippy::cast_possible_truncation,
                reason = "RDP resolution is clamped well below u16::MAX in this code path"
            )]
            let h = rounded_height as u16;

            if let Some(ref sender) = *ironrdp_tx.borrow() {
                let _ = sender.send(RdpClientCommand::SetDesktopSize {
                    width: w,
                    height: h,
                    // Helper DPI: 100% for a small window (dense, normal
                    // cursor), display DPI otherwise.
                    scale_percent: Some(u32::from(req.scale_percent)),
                });
            }

            tracing::info!(
                protocol = "rdp",
                width = rounded_width,
                height = rounded_height,
                "[RDP Resize] Manual resolution sync via Display Control"
            );

            status_label.set_text(&i18n("Resizing…"));
            status_label.set_visible(true);
            let sl_hide = status_label.clone();
            glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
                sl_hide.set_visible(false);
            });
        } else {
            tracing::info!(
                protocol = "rdp",
                width = rounded_width,
                height = rounded_height,
                "[RDP Resize] Manual resolution sync via reconnect (FreeRDP/forced)"
            );

            if let Some(ref sender) = *ironrdp_tx.borrow() {
                let _ = sender.send(RdpClientCommand::Disconnect);
            }

            status_label.set_text(&i18n("Reconnecting..."));
            status_label.set_visible(true);

            let reconnect_cb = on_reconnect.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
                if let Some(ref callback) = *reconnect_cb.borrow() {
                    callback();
                }
            });
        }
    }

    /// Sets up the resize handler (fallback when rdp-embedded is disabled)
    #[cfg(not(feature = "rdp-embedded"))]
    pub(super) fn setup_resize_handler(&self) {
        let width = self.width.clone();
        let height = self.height.clone();

        let handler_id = self
            .drawing_area
            .connect_resize(move |area, new_width, new_height| {
                let new_width = new_width.unsigned_abs();
                let new_height = new_height.unsigned_abs();

                *width.borrow_mut() = new_width;
                *height.borrow_mut() = new_height;

                area.queue_draw();
            });
        *self.resize_handler_id.borrow_mut() = Some(handler_id);
    }

    /// Forces an RDP resolution change to match the current widget size.
    ///
    /// No-op fallback when the `rdp-embedded` feature is disabled (no IronRDP
    /// command channel is available in that build).
    #[cfg(not(feature = "rdp-embedded"))]
    pub fn request_resolution_sync(&self) {
        self.drawing_area.queue_draw();
    }
}
