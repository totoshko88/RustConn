//! Resize handler for the embedded RDP widget
//!
//! Contains debounced resize logic that triggers resolution changes
//! after the widget stops being resized.

use gtk4::glib;
use gtk4::prelude::*;

use super::types::RdpConnectionState;

#[cfg(feature = "rdp-embedded")]
use rustconn_core::rdp_client::RdpClientCommand;

impl super::EmbeddedRdpWidget {
    /// Sets up the resize handler with debounced reconnect for resolution change
    ///
    /// When the widget is resized, we:
    /// 1. Immediately scale the current image to fit
    /// 2. After 2 seconds of no resize, reconnect with new resolution
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
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let device_width = (f64::from(css_width) * effective_scale) as u32;
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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

                // Schedule reconnect after 500ms of no resize
                let rdp_w = rdp_width.clone();
                let rdp_h = rdp_height.clone();
                let timer = reconnect_timer.clone();
                let cfg = config.clone();
                let tx = ironrdp_tx.clone();
                let sl = status_label.clone();
                let reconnect_cb = on_reconnect.clone();

                let source_id = glib::timeout_add_local_once(
                    std::time::Duration::from_millis(500),
                    move || {
                        // Clear the timer reference
                        timer.borrow_mut().take();

                        let current_rdp_w = *rdp_w.borrow();
                        let current_rdp_h = *rdp_h.borrow();

                        // Only reconnect if size actually changed significantly (>50px device)
                        let w_diff = (device_width as i32 - current_rdp_w as i32).unsigned_abs();
                        let h_diff = (device_height as i32 - current_rdp_h as i32).unsigned_abs();

                        if w_diff > 50 || h_diff > 50 {
                            // Round down to multiple of 4 for RDP compatibility
                            let rounded_width = (device_width / 4) * 4;
                            let rounded_height = (device_height / 4) * 4;

                            tracing::info!(
                                "[RDP Resize] Reconnecting with new resolution: {}x{} -> {}x{} \
                                 (rounded from {}x{})",
                                current_rdp_w,
                                current_rdp_h,
                                rounded_width,
                                rounded_height,
                                device_width,
                                device_height
                            );

                            // Update config with new resolution
                            {
                                let current_config = cfg.borrow().clone();
                                if let Some(mut config) = current_config {
                                    config = config.with_resolution(rounded_width, rounded_height);
                                    *cfg.borrow_mut() = Some(config);
                                }
                            }

                            // Disconnect current session
                            if let Some(ref sender) = *tx.borrow() {
                                let _ = sender.send(RdpClientCommand::Disconnect);
                            }

                            // Show reconnecting status
                            sl.set_text("Reconnecting...");
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
