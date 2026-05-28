//! UI construction and input handling for embedded VNC widget.
//!
//! Contains widget construction (`new()`), drawing setup, input handlers,
//! clipboard buttons, and coordinate transformation utilities.
//!
//! Extracted from `embedded_vnc.rs` as part of ARCH-5 decomposition.

use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, DrawingArea, EventControllerKey, EventControllerMotion, GestureClick,
    Label, Orientation,
};
use std::cell::RefCell;
use std::rc::Rc;

use crate::i18n::{i18n, i18n_f};

use super::EmbeddedVncWidget;
#[cfg(feature = "vnc-embedded")]
use super::VncClientCommand;
use super::find_best_standard_resolution;
use super::{VncConnectionState, VncPixelBuffer, VncWaylandSurface};

impl EmbeddedVncWidget {
    /// Creates a new embedded VNC widget
    #[must_use]
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        // Create toolbar with clipboard and Ctrl+Alt+Del buttons (right-aligned)
        let toolbar = GtkBox::new(Orientation::Horizontal, 4);
        toolbar.set_margin_start(6);
        toolbar.set_margin_end(6);
        toolbar.set_margin_top(6);
        toolbar.set_margin_bottom(6);
        toolbar.set_halign(gtk4::Align::End);

        // Status label for clipboard feedback (hidden by default)
        let status_label = Label::new(None);
        status_label.set_visible(false);
        status_label.set_margin_end(12);
        status_label.add_css_class("dim-label");
        toolbar.append(&status_label);

        // Copy button - copies from remote clipboard to local
        let copy_button = Button::with_label(&i18n("Copy"));
        copy_button.set_tooltip_text(Some(&i18n("Copy from remote session to local clipboard")));
        toolbar.append(&copy_button);

        // Paste button - pastes from local clipboard to remote
        let paste_button = Button::with_label(&i18n("Paste"));
        paste_button.set_tooltip_text(Some(&i18n("Paste from local clipboard to remote session")));
        toolbar.append(&paste_button);

        // Separator
        let separator = gtk4::Separator::new(Orientation::Vertical);
        separator.set_margin_start(6);
        separator.set_margin_end(6);
        toolbar.append(&separator);

        // Ctrl+Alt+Del button
        let ctrl_alt_del_button = Button::with_label(&i18n("Ctrl+Alt+Del"));
        ctrl_alt_del_button.add_css_class("suggested-action");
        ctrl_alt_del_button.set_tooltip_text(Some(&i18n("Send Ctrl+Alt+Del to remote session")));
        toolbar.append(&ctrl_alt_del_button);

        // Hide toolbar initially (show when connected)
        toolbar.set_visible(false);

        container.append(&toolbar);

        let drawing_area = DrawingArea::new();
        drawing_area.set_hexpand(true);
        drawing_area.set_vexpand(true);
        // Do NOT set content_width/content_height to the VNC resolution — this
        // inflates the widget's natural size and causes AdwTabOverview to warn
        // "exceeds AdwApplicationWindow size". The DrawingArea expands to fill
        // available space via hexpand/vexpand; the actual VNC resolution is
        // negotiated dynamically via SetDesktopSize.
        drawing_area.set_content_width(0);
        drawing_area.set_content_height(0);
        drawing_area.set_can_focus(true);
        drawing_area.set_focusable(true);

        container.append(&drawing_area);

        // Reconnect banner (shown when disconnected, at bottom like VTE sessions)
        let reconnect_banner = GtkBox::new(Orientation::Horizontal, 6);
        reconnect_banner.set_margin_start(12);
        reconnect_banner.set_margin_end(12);
        reconnect_banner.set_margin_top(6);
        reconnect_banner.set_margin_bottom(6);
        reconnect_banner.set_halign(gtk4::Align::Center);
        reconnect_banner.set_widget_name("reconnect-banner");
        reconnect_banner.set_visible(false);

        let reconnect_label = Label::new(Some(&i18n("Session disconnected")));
        reconnect_label.add_css_class("dim-label");

        let reconnect_button = Button::with_label(&i18n("Reconnect"));
        reconnect_button.add_css_class("suggested-action");
        reconnect_button.set_tooltip_text(Some(&i18n("Reconnect to this session")));

        reconnect_banner.append(&reconnect_label);
        reconnect_banner.append(&reconnect_button);

        container.append(&reconnect_banner);

        let pixel_buffer = Rc::new(RefCell::new(VncPixelBuffer::new(1280, 720)));
        let cairo_buffer = Rc::new(RefCell::new(crate::cairo_buffer::CairoBackedBuffer::new(
            1280, 720,
        )));
        let state = Rc::new(RefCell::new(VncConnectionState::Disconnected));
        let width = Rc::new(RefCell::new(1280u32));
        let height = Rc::new(RefCell::new(720u32));
        let vnc_width = Rc::new(RefCell::new(1280u32));
        let vnc_height = Rc::new(RefCell::new(720u32));

        let widget = Self {
            container,
            toolbar,
            status_label,
            copy_button: copy_button.clone(),
            paste_button: paste_button.clone(),
            ctrl_alt_del_button: ctrl_alt_del_button.clone(),
            separator,
            drawing_area,
            wl_surface: Rc::new(RefCell::new(VncWaylandSurface::new())),
            pixel_buffer,
            cairo_buffer,
            state,
            config: Rc::new(RefCell::new(None)),
            process: Rc::new(RefCell::new(None)),
            is_embedded: Rc::new(RefCell::new(false)),
            width,
            height,
            vnc_width,
            vnc_height,
            on_state_changed: Rc::new(RefCell::new(None)),
            on_error: Rc::new(RefCell::new(None)),
            on_frame_update: Rc::new(RefCell::new(None)),
            on_reconnect: Rc::new(RefCell::new(None)),
            reconnect_banner,
            reconnect_button,
            #[cfg(feature = "vnc-embedded")]
            vnc_client: Rc::new(RefCell::new(None)),
            #[cfg(feature = "vnc-embedded")]
            command_sender: Rc::new(RefCell::new(None)),
        };

        widget.setup_drawing();
        widget.setup_input_handlers();
        widget.setup_resize_handler();
        widget.setup_clipboard_buttons(&copy_button, &paste_button);
        widget.setup_ctrl_alt_del_button(&ctrl_alt_del_button);
        widget.setup_reconnect_button();
        widget.setup_visibility_handler();

        widget
    }

    /// Sets up visibility handler to redraw when widget becomes visible again
    /// This fixes the issue where the image disappears when switching tabs
    fn setup_visibility_handler(&self) {
        let drawing_area = self.drawing_area.clone();

        // Redraw when the widget becomes visible (e.g., switching back to this tab)
        self.container.connect_map(move |_| {
            drawing_area.queue_draw();
        });
    }

    /// Sets up the reconnect button click handler
    fn setup_reconnect_button(&self) {
        let on_reconnect = self.on_reconnect.clone();

        self.reconnect_button.connect_clicked(move |_| {
            if let Some(ref callback) = *on_reconnect.borrow() {
                callback();
            }
        });
    }

    /// Connects a callback for reconnect button clicks
    ///
    /// The callback is invoked when the user clicks the Reconnect button
    /// after a session has disconnected or encountered an error.
    pub fn connect_reconnect<F>(&self, callback: F)
    where
        F: Fn() + 'static,
    {
        *self.on_reconnect.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets up the drawing function for the DrawingArea
    fn setup_drawing(&self) {
        let pixel_buffer = self.pixel_buffer.clone();
        let cairo_buffer = self.cairo_buffer.clone();
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let config = self.config.clone();

        self.drawing_area
            .set_draw_func(move |_area, cr, width, height| {
                let current_state = *state.borrow();
                let embedded = *is_embedded.borrow();

                // Dark background
                cr.set_source_rgb(0.12, 0.12, 0.14);
                let _ = cr.paint();

                if embedded && current_state == VncConnectionState::Connected {
                    // Fast path: use the persistent Cairo surface (zero-copy)
                    let buffer = cairo_buffer.borrow();
                    let buf_width = buffer.width();
                    let buf_height = buffer.height();

                    if buf_width > 0
                        && buf_height > 0
                        && buffer.has_data()
                        && let Some(surface) = buffer.surface()
                    {
                        let scale_x = f64::from(width) / f64::from(buf_width);
                        let scale_y = f64::from(height) / f64::from(buf_height);
                        let scale = scale_x.min(scale_y);

                        let offset_x = f64::from(buf_width).mul_add(-scale, f64::from(width)) / 2.0;
                        let offset_y =
                            f64::from(buf_height).mul_add(-scale, f64::from(height)) / 2.0;

                        cr.translate(offset_x, offset_y);
                        cr.scale(scale, scale);
                        let _ = cr.set_source_surface(surface, 0.0, 0.0);
                        let _ = cr.paint();
                        return;
                    }

                    // Fallback: old VncPixelBuffer path (to_vec copy)
                    #[expect(
    clippy::items_after_statements,
    reason = "local helper introduced inline next to its only call site; hoisting would scatter related logic"
)]
                    static WARN_ONCE: std::sync::Once = std::sync::Once::new();
                    WARN_ONCE.call_once(|| {
                        tracing::warn!("VNC: using fallback VncPixelBuffer with per-frame to_vec() copy — consider migrating to CairoBackedBuffer");
                    });
                    let fb = pixel_buffer.borrow();
                    let fb_w = fb.width();
                    let fb_h = fb.height();
                    if fb_w > 0 && fb_h > 0 {
                        let data = fb.data();
                        if let Ok(surface) = gtk4::cairo::ImageSurface::create_for_data(
                            data.to_vec(),
                            gtk4::cairo::Format::ARgb32,
                            crate::utils::dimension_to_i32(fb_w),
                            crate::utils::dimension_to_i32(fb_h),
                            crate::utils::stride_to_i32(fb.stride()),
                        ) {
                            let scale_x = f64::from(width) / f64::from(fb_w);
                            let scale_y = f64::from(height) / f64::from(fb_h);
                            let scale = scale_x.min(scale_y);

                            let offset_x = f64::from(fb_w).mul_add(-scale, f64::from(width)) / 2.0;
                            let offset_y = f64::from(fb_h).mul_add(-scale, f64::from(height)) / 2.0;

                            cr.translate(offset_x, offset_y);
                            cr.scale(scale, scale);
                            let _ = cr.set_source_surface(&surface, 0.0, 0.0);
                            let _ = cr.paint();
                        }
                    }
                } else {
                    // Show status overlay
                    cr.select_font_face(
                        "Sans",
                        gtk4::cairo::FontSlant::Normal,
                        gtk4::cairo::FontWeight::Normal,
                    );

                    let center_y = f64::from(height) / 2.0 - 40.0;

                    // Protocol icon (circle with "V" for VNC)
                    cr.set_source_rgb(0.5, 0.3, 0.7);
                    cr.arc(
                        f64::from(width) / 2.0,
                        center_y,
                        40.0,
                        0.0,
                        2.0 * std::f64::consts::PI,
                    );
                    let _ = cr.fill();

                    cr.set_source_rgb(1.0, 1.0, 1.0);
                    cr.set_font_size(32.0);
                    if let Ok(extents) = cr.text_extents("V") {
                        cr.move_to(
                            f64::from(width) / 2.0 - extents.width() / 2.0,
                            center_y + extents.height() / 2.0,
                        );
                        let _ = cr.show_text("V");
                    }

                    // Connection info
                    let config_ref = config.borrow();
                    let host = config_ref.as_ref().map_or("Not configured", |c| &c.host);

                    cr.set_source_rgb(0.9, 0.9, 0.9);
                    cr.set_font_size(18.0);
                    if let Ok(extents) = cr.text_extents(host) {
                        cr.move_to((f64::from(width) - extents.width()) / 2.0, center_y + 70.0);
                        let _ = cr.show_text(host);
                    }

                    // Status message
                    cr.set_font_size(13.0);
                    let status_text = match current_state {
                        VncConnectionState::Disconnected => "Disconnected",
                        VncConnectionState::Connecting => "Connecting...",
                        VncConnectionState::Connected if !embedded => {
                            "Session running in external window"
                        }
                        VncConnectionState::Connected => "Connected",
                        VncConnectionState::Error => "Connection error",
                    };

                    let color = match current_state {
                        VncConnectionState::Connected => (0.6, 0.8, 0.6),
                        VncConnectionState::Connecting => (0.8, 0.8, 0.6),
                        VncConnectionState::Error => (0.8, 0.4, 0.4),
                        VncConnectionState::Disconnected => (0.5, 0.5, 0.5),
                    };
                    cr.set_source_rgb(color.0, color.1, color.2);

                    if let Ok(extents) = cr.text_extents(status_text) {
                        cr.move_to((f64::from(width) - extents.width()) / 2.0, center_y + 100.0);
                        let _ = cr.show_text(status_text);
                    }
                }
            });
    }

    /// Sets up keyboard and mouse input handlers
    #[cfg(feature = "vnc-embedded")]
    fn setup_input_handlers(&self) {
        use glib::translate::IntoGlib;
        use rustconn_core::vnc_client::VncClientCommand;

        // Keyboard input handler
        let key_controller = EventControllerKey::new();
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let config = self.config.clone();
        let command_sender = self.command_sender.clone();

        key_controller.connect_key_pressed(move |_controller, keyval, _keycode, _modifier| {
            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();
            let view_only = config.borrow().as_ref().is_some_and(|c| c.view_only);

            if embedded && current_state == VncConnectionState::Connected && !view_only {
                // GDK Key values are compatible with X11 keysyms
                let keysym = keyval.into_glib();
                if let Some(ref sender) = *command_sender.borrow() {
                    // Use try_send to avoid blocking GTK main thread
                    let _ = sender.try_send(VncClientCommand::KeyEvent {
                        keysym,
                        pressed: true,
                    });
                }
            }

            gdk::glib::Propagation::Proceed
        });

        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let config = self.config.clone();
        let command_sender = self.command_sender.clone();

        key_controller.connect_key_released(move |_controller, keyval, _keycode, _modifier| {
            use glib::translate::IntoGlib;
            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();
            let view_only = config.borrow().as_ref().is_some_and(|c| c.view_only);

            if embedded && current_state == VncConnectionState::Connected && !view_only {
                let keysym = keyval.into_glib();
                if let Some(ref sender) = *command_sender.borrow() {
                    // Use try_send to avoid blocking GTK main thread
                    let _ = sender.try_send(VncClientCommand::KeyEvent {
                        keysym,
                        pressed: false,
                    });
                }
            }
        });

        self.drawing_area.add_controller(key_controller);

        // Track current button state for motion events
        let button_state = Rc::new(RefCell::new(0u8));

        // Mouse motion handler
        let motion_controller = EventControllerMotion::new();
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let config = self.config.clone();
        let command_sender = self.command_sender.clone();
        let button_state_motion = button_state.clone();
        let width_motion = self.width.clone();
        let height_motion = self.height.clone();
        let vnc_width_motion = self.vnc_width.clone();
        let vnc_height_motion = self.vnc_height.clone();

        motion_controller.connect_motion(move |_controller, x, y| {
            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();
            let view_only = config.borrow().as_ref().is_some_and(|c| c.view_only);

            if embedded && current_state == VncConnectionState::Connected && !view_only {
                // Transform widget coordinates to VNC server coordinates
                let widget_w = f64::from(*width_motion.borrow());
                let widget_h = f64::from(*height_motion.borrow());
                let vnc_w = f64::from(*vnc_width_motion.borrow());
                let vnc_h = f64::from(*vnc_height_motion.borrow());

                let (vnc_x, vnc_y) =
                    transform_widget_to_vnc(x, y, widget_w, widget_h, vnc_w, vnc_h);

                let vnc_x = crate::utils::coord_to_u16(vnc_x);
                let vnc_y = crate::utils::coord_to_u16(vnc_y);
                let buttons = *button_state_motion.borrow();

                if let Some(ref sender) = *command_sender.borrow() {
                    // Use try_send to avoid blocking GTK main thread
                    let _ = sender.try_send(VncClientCommand::PointerEvent {
                        x: vnc_x,
                        y: vnc_y,
                        buttons,
                    });
                }
            }
        });

        self.drawing_area.add_controller(motion_controller);

        // Mouse click handler
        let click_controller = GestureClick::new();
        click_controller.set_button(0); // Listen to all buttons
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let config = self.config.clone();
        let command_sender = self.command_sender.clone();
        let button_state_press = button_state.clone();
        let width_press = self.width.clone();
        let height_press = self.height.clone();
        let vnc_width_press = self.vnc_width.clone();
        let vnc_height_press = self.vnc_height.clone();

        click_controller.connect_pressed(move |gesture, _n_press, x, y| {
            // Grab focus on click so keyboard events are received
            if let Some(widget) = gesture.widget() {
                widget.grab_focus();
            }

            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();
            let view_only = config.borrow().as_ref().is_some_and(|c| c.view_only);

            if embedded && current_state == VncConnectionState::Connected && !view_only {
                let button = gesture.current_button();

                // Transform widget coordinates to VNC server coordinates
                let widget_w = f64::from(*width_press.borrow());
                let widget_h = f64::from(*height_press.borrow());
                let vnc_w = f64::from(*vnc_width_press.borrow());
                let vnc_h = f64::from(*vnc_height_press.borrow());

                let (vnc_x, vnc_y) =
                    transform_widget_to_vnc(x, y, widget_w, widget_h, vnc_w, vnc_h);

                let vnc_x = crate::utils::coord_to_u16(vnc_x);
                let vnc_y = crate::utils::coord_to_u16(vnc_y);

                // Convert GTK button to VNC button mask and update state
                let button_bit = gtk_button_to_vnc_mask(button);
                let buttons = *button_state_press.borrow() | button_bit;
                *button_state_press.borrow_mut() = buttons;

                if let Some(ref sender) = *command_sender.borrow() {
                    // Use try_send to avoid blocking GTK main thread
                    let _ = sender.try_send(VncClientCommand::PointerEvent {
                        x: vnc_x,
                        y: vnc_y,
                        buttons,
                    });
                }
            }
        });

        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let config = self.config.clone();
        let command_sender = self.command_sender.clone();
        let button_state_release = button_state;
        let width_release = self.width.clone();
        let height_release = self.height.clone();
        let vnc_width_release = self.vnc_width.clone();
        let vnc_height_release = self.vnc_height.clone();

        click_controller.connect_released(move |gesture, _n_press, x, y| {
            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();
            let view_only = config.borrow().as_ref().is_some_and(|c| c.view_only);

            if embedded && current_state == VncConnectionState::Connected && !view_only {
                let button = gesture.current_button();

                // Transform widget coordinates to VNC server coordinates
                let widget_w = f64::from(*width_release.borrow());
                let widget_h = f64::from(*height_release.borrow());
                let vnc_w = f64::from(*vnc_width_release.borrow());
                let vnc_h = f64::from(*vnc_height_release.borrow());

                let (vnc_x, vnc_y) =
                    transform_widget_to_vnc(x, y, widget_w, widget_h, vnc_w, vnc_h);

                let vnc_x = crate::utils::coord_to_u16(vnc_x);
                let vnc_y = crate::utils::coord_to_u16(vnc_y);

                // Clear the button bit from state
                let button_bit = gtk_button_to_vnc_mask(button);
                let buttons = *button_state_release.borrow() & !button_bit;
                *button_state_release.borrow_mut() = buttons;

                if let Some(ref sender) = *command_sender.borrow() {
                    // Use try_send to avoid blocking GTK main thread
                    let _ = sender.try_send(VncClientCommand::PointerEvent {
                        x: vnc_x,
                        y: vnc_y,
                        buttons,
                    });
                }
            }
        });

        self.drawing_area.add_controller(click_controller);
    }

    /// Sets up keyboard and mouse input handlers (fallback when vnc-embedded is disabled)
    #[cfg(not(feature = "vnc-embedded"))]
    fn setup_input_handlers(&self) {
        // No-op when vnc-embedded feature is disabled
        // Input is handled by external VNC viewer
    }

    /// Sets up the resize handler
    #[cfg(feature = "vnc-embedded")]
    fn setup_resize_handler(&self) {
        use rustconn_core::vnc_client::VncClientCommand;

        let width = self.width.clone();
        let height = self.height.clone();
        let vnc_width = self.vnc_width.clone();
        let vnc_height = self.vnc_height.clone();
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let command_sender = self.command_sender.clone();
        let config = self.config.clone();

        // Track last requested resolution to avoid duplicate requests
        let last_requested: Rc<RefCell<(u32, u32)>> = Rc::new(RefCell::new((0, 0)));

        self.drawing_area
            .connect_resize(move |area, new_width, new_height| {
                // Apply scale override from config, falling back to system scale_factor
                let effective_scale = config.borrow().as_ref().map_or_else(
                    || f64::from(area.scale_factor().max(1)),
                    |c| c.scale_override.effective_scale(area.scale_factor()),
                );
                #[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "value range fits the target type and is non-negative by construction in this code path"
)]
                let scaled_width = (f64::from(new_width.unsigned_abs()) * effective_scale) as u32;
                #[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "value range fits the target type and is non-negative by construction in this code path"
)]
                let scaled_height = (f64::from(new_height.unsigned_abs()) * effective_scale) as u32;

                *width.borrow_mut() = new_width.unsigned_abs();
                *height.borrow_mut() = new_height.unsigned_abs();

                let current_state = *state.borrow();
                let embedded = *is_embedded.borrow();

                if embedded && current_state == VncConnectionState::Connected {
                    // Find the best standard resolution that fits the scaled window
                    let (best_w, best_h) =
                        find_best_standard_resolution(scaled_width, scaled_height);

                    // Only request if different from current VNC resolution and last request
                    let current_vnc_w = *vnc_width.borrow();
                    let current_vnc_h = *vnc_height.borrow();
                    let last = *last_requested.borrow();

                    if (best_w, best_h) != (current_vnc_w, current_vnc_h)
                        && (best_w, best_h) != last
                    {
                        *last_requested.borrow_mut() = (best_w, best_h);

                        if let Some(ref sender) = *command_sender.borrow() {
                            // Use try_send to avoid blocking GTK main thread
                            let _ = sender.try_send(VncClientCommand::SetDesktopSize {
                                width: crate::utils::dimension_to_u16(best_w),
                                height: crate::utils::dimension_to_u16(best_h),
                            });
                            tracing::debug!(
                                "[VNC] Requesting resolution {}x{} for window {}x{} \
                                 (scale: {:.2})",
                                best_w,
                                best_h,
                                new_width,
                                new_height,
                                effective_scale
                            );
                        }
                    }
                }
            });
    }

    #[cfg(not(feature = "vnc-embedded"))]
    fn setup_resize_handler(&self) {
        let width = self.width.clone();
        let height = self.height.clone();
        let pixel_buffer = self.pixel_buffer.clone();
        let cairo_buffer = self.cairo_buffer.clone();

        self.drawing_area
            .connect_resize(move |_area, new_width, new_height| {
                let new_width = new_width.unsigned_abs();
                let new_height = new_height.unsigned_abs();

                *width.borrow_mut() = new_width;
                *height.borrow_mut() = new_height;

                // Resize the pixel buffer
                pixel_buffer.borrow_mut().resize(new_width, new_height);
                cairo_buffer.borrow_mut().resize(new_width, new_height);
            });
    }

    /// Sets up the clipboard Copy/Paste button handlers
    #[cfg(feature = "vnc-embedded")]
    fn setup_clipboard_buttons(&self, copy_btn: &Button, paste_btn: &Button) {
        // Copy button - get text from remote clipboard and copy to local
        {
            let drawing_area = self.drawing_area.clone();
            let state = self.state.clone();
            let is_embedded = self.is_embedded.clone();

            copy_btn.connect_clicked(move |_| {
                let current_state = *state.borrow();
                let embedded = *is_embedded.borrow();

                if current_state != VncConnectionState::Connected || !embedded {
                    return;
                }

                // For VNC, clipboard sync happens via ServerCutText messages
                // This button shows a hint that clipboard is synced
                tracing::debug!(
                    "[VNC] Clipboard sync: VNC clipboard is automatically synchronized"
                );

                // Get GTK clipboard and show notification
                let display = drawing_area.display();
                let clipboard = display.clipboard();
                clipboard.read_text_async(
                    None::<&gtk4::gio::Cancellable>,
                    move |result: Result<Option<glib::GString>, glib::Error>| {
                        if let Ok(Some(text)) = result {
                            tracing::debug!("[VNC] Local clipboard has {} chars", text.len());
                        }
                    },
                );
            });
        }

        // Paste button - send local clipboard text to remote
        {
            let command_sender = self.command_sender.clone();
            let drawing_area = self.drawing_area.clone();
            let state = self.state.clone();
            let is_embedded = self.is_embedded.clone();
            let status_label = self.status_label.clone();

            paste_btn.connect_clicked(move |_| {
                let current_state = *state.borrow();
                let embedded = *is_embedded.borrow();

                if current_state != VncConnectionState::Connected || !embedded {
                    return;
                }

                // Get text from local clipboard and send to remote
                let display = drawing_area.display();
                let clipboard = display.clipboard();
                let tx = command_sender.clone();
                let status = status_label.clone();

                clipboard.read_text_async(
                    None::<&gtk4::gio::Cancellable>,
                    move |result: Result<Option<glib::GString>, glib::Error>| {
                        if let Ok(Some(text)) = result {
                            let char_count = text.len();
                            tracing::debug!("[VNC] Pasting {char_count} chars to remote");

                            // Send text as key presses via VNC client
                            // (ClipboardText only syncs clipboard, doesn't paste)
                            if let Some(ref sender) = *tx.borrow() {
                                // Use try_send to avoid blocking GTK main thread
                                let _ =
                                    sender.try_send(VncClientCommand::TypeText(text.to_string()));
                                // Show brief feedback
                                status.set_text(&i18n_f(
                                    "Pasted {} chars",
                                    &[&char_count.to_string()],
                                ));
                                status.set_visible(true);
                                // Hide after 2 seconds
                                let status_hide = status.clone();
                                glib::timeout_add_local_once(
                                    std::time::Duration::from_secs(2),
                                    move || {
                                        status_hide.set_visible(false);
                                    },
                                );
                            }
                        }
                    },
                );
            });
        }
    }

    /// Sets up the clipboard Copy/Paste button handlers (no-op when vnc-embedded is disabled)
    #[cfg(not(feature = "vnc-embedded"))]
    fn setup_clipboard_buttons(&self, _copy_btn: &Button, _paste_btn: &Button) {
        // No-op when vnc-embedded feature is disabled
    }

    /// Sets up the Ctrl+Alt+Del button handler
    #[cfg(feature = "vnc-embedded")]
    fn setup_ctrl_alt_del_button(&self, button: &Button) {
        let command_sender = self.command_sender.clone();
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();

        button.connect_clicked(move |_| {
            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();

            if current_state != VncConnectionState::Connected || !embedded {
                return;
            }

            // Send Ctrl+Alt+Del via VNC client
            if let Some(ref sender) = *command_sender.borrow() {
                // Use try_send to avoid blocking GTK main thread
                let _ = sender.try_send(VncClientCommand::SendCtrlAltDel);
                tracing::debug!("[VNC] Sent Ctrl+Alt+Del");
            }
        });
    }

    /// Sets up the Ctrl+Alt+Del button handler (no-op when vnc-embedded is disabled)
    #[cfg(not(feature = "vnc-embedded"))]
    fn setup_ctrl_alt_del_button(&self, _button: &Button) {
        // No-op when vnc-embedded feature is disabled
    }
}

// --- Coordinate transformation utilities (merged from embedded_vnc_ui.rs) ---

/// Transforms widget coordinates to VNC framebuffer coordinates
///
/// This function handles the coordinate transformation needed when the widget
/// size differs from the VNC framebuffer size. It maintains aspect ratio and
/// centers the content.
///
/// # Arguments
/// * `x`, `y` - Widget coordinates (from mouse event)
/// * `widget_w`, `widget_h` - Current widget dimensions
/// * `vnc_w`, `vnc_h` - VNC framebuffer dimensions
///
/// # Returns
/// Tuple of (vnc_x, vnc_y) clamped to valid framebuffer coordinates
#[must_use]
pub fn transform_widget_to_vnc(
    x: f64,
    y: f64,
    widget_w: f64,
    widget_h: f64,
    vnc_w: f64,
    vnc_h: f64,
) -> (f64, f64) {
    // Calculate scale factor maintaining aspect ratio
    let scale = (widget_w / vnc_w).min(widget_h / vnc_h);

    // Calculate centering offsets
    let offset_x = vnc_w.mul_add(-scale, widget_w) / 2.0;
    let offset_y = vnc_h.mul_add(-scale, widget_h) / 2.0;

    // Transform and clamp to valid range
    let vnc_x = ((x - offset_x) / scale).clamp(0.0, vnc_w - 1.0);
    let vnc_y = ((y - offset_y) / scale).clamp(0.0, vnc_h - 1.0);

    (vnc_x, vnc_y)
}

/// Converts GTK button number to VNC button mask bit
///
/// GTK buttons: 1=left, 2=middle, 3=right
/// VNC mask bits: 0x01=left, 0x02=middle, 0x04=right
#[must_use]
pub const fn gtk_button_to_vnc_mask(gtk_button: u32) -> u8 {
    match gtk_button {
        1 => 0x01, // Left
        2 => 0x02, // Middle
        3 => 0x04, // Right
        _ => 0x00,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transform_widget_to_vnc_centered() {
        // Widget and VNC same size - no transformation needed
        let (x, y) = transform_widget_to_vnc(100.0, 100.0, 1920.0, 1080.0, 1920.0, 1080.0);
        assert!((x - 100.0).abs() < 0.001);
        assert!((y - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_transform_widget_to_vnc_scaled() {
        // Widget is 2x larger than VNC
        let (x, y) = transform_widget_to_vnc(200.0, 200.0, 3840.0, 2160.0, 1920.0, 1080.0);
        assert!((x - 100.0).abs() < 0.001);
        assert!((y - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_transform_widget_to_vnc_clamped() {
        // Coordinates outside VNC area should be clamped
        let (x, y) = transform_widget_to_vnc(-100.0, -100.0, 1920.0, 1080.0, 1920.0, 1080.0);
        assert!(x >= 0.0);
        assert!(y >= 0.0);

        let (x, y) = transform_widget_to_vnc(10000.0, 10000.0, 1920.0, 1080.0, 1920.0, 1080.0);
        assert!(x <= 1919.0);
        assert!(y <= 1079.0);
    }

    #[test]
    fn test_gtk_button_to_vnc_mask() {
        assert_eq!(gtk_button_to_vnc_mask(1), 0x01); // Left
        assert_eq!(gtk_button_to_vnc_mask(2), 0x02); // Middle
        assert_eq!(gtk_button_to_vnc_mask(3), 0x04); // Right
        assert_eq!(gtk_button_to_vnc_mask(4), 0x00); // Unknown
    }
}
