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

#[cfg(feature = "rdp-embedded")]
use rustconn_core::rdp_client::RdpClientCommand;

impl super::EmbeddedRdpWidget {
    /// Sets up the resize handler with debounced dynamic resolution change
    ///
    /// When the widget is resized, we:
    /// 1. Immediately scale the current image to fit
    /// 2. After 500ms of no resize, send `SetDesktopSize` via Display Control Channel
    /// 3. If Display Control is unavailable, fall back to reconnect
    ///
    /// # Requirements Coverage
    ///
    /// - Requirement 1.7: Dynamic resolution change on resize
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

        let handler_id = self
            .drawing_area
            .connect_resize(move |area, new_width, new_height| {
                // Store CSS pixel dimensions for mouse coordinate transform.
                // GTK mouse events use CSS coordinates, and the draw function
                // also operates in CSS space, so self.width/height must match.
                let css_width = new_width.unsigned_abs();
                let css_height = new_height.unsigned_abs();

                // Compute device pixels for RDP resolution requests
                let effective_scale = config.borrow().as_ref().map_or_else(
                    || f64::from(area.scale_factor().max(1)),
                    |c| c.scale_override.effective_scale(area.scale_factor()),
                );
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

                        // Only resize if size actually changed significantly (>50px device)
                        let w_diff = (device_width as i32 - current_rdp_w as i32).unsigned_abs();
                        let h_diff = (device_height as i32 - current_rdp_h as i32).unsigned_abs();

                        if w_diff > RESIZE_THRESHOLD_PX || h_diff > RESIZE_THRESHOLD_PX {
                            // Round down to multiple of 4 for RDP compatibility
                            let rounded_width = (device_width / 4) * 4;
                            let rounded_height = (device_height / 4) * 4;

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
                                #[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "value range fits the target type and is non-negative by construction in this code path"
)]
                                let w = rounded_width as u16;
                                #[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "value range fits the target type and is non-negative by construction in this code path"
)]
                                let h = rounded_height as u16;

                                if let Some(ref sender) = *tx.borrow() {
                                    let _ = sender.send(RdpClientCommand::SetDesktopSize {
                                        width: w,
                                        height: h,
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

    /// Sets up the resize handler (fallback when rdp-embedded is disabled)
    #[cfg(not(feature = "rdp-embedded"))]
    pub(super) fn setup_resize_handler(&self) {
        let width = self.width.clone();
        let height = self.height.clone();
        let pixel_buffer = self.pixel_buffer.clone();

        let handler_id = self
            .drawing_area
            .connect_resize(move |area, new_width, new_height| {
                let new_width = new_width.unsigned_abs();
                let new_height = new_height.unsigned_abs();

                *width.borrow_mut() = new_width;
                *height.borrow_mut() = new_height;

                // Resize pixel buffer
                pixel_buffer.borrow_mut().resize(new_width, new_height);
                area.queue_draw();
            });
        *self.resize_handler_id.borrow_mut() = Some(handler_id);
    }
}
