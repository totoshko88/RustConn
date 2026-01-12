//! Embedded VNC widget using Wayland subsurface
//!
//! This module provides the `EmbeddedVncWidget` struct that enables native VNC
//! session embedding within the GTK4 application using Wayland subsurfaces.
//!
//! # Architecture
//!
//! The embedded VNC widget uses a `DrawingArea` as the rendering target and
//! integrates with a VNC client library for the actual VNC protocol handling.
//! On Wayland, it uses `wl_subsurface` for native compositor integration.
//!
//! # Requirements Coverage
//!
//! - Requirement 16.2: VNC connections embedded in main window
//! - Requirement 16.3: Wayland wl_subsurface for native compositor integration
//! - Requirement 16.4: Frame buffer handling and blit to wl_buffer
//! - Requirement 16.5: Keyboard and mouse input forwarding

// Allow cast warnings - graphics code uses various integer sizes for coordinates
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::missing_panics_doc)]

// Re-export types for external use
pub use crate::embedded_vnc_types::{
    find_best_standard_resolution, EmbeddedVncError, ErrorCallback, FrameCallback, StateCallback,
    VncConfig, VncConnectionState, VncPixelBuffer, VncWaylandSurface, STANDARD_RESOLUTIONS,
};

use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, DrawingArea, EventControllerKey, EventControllerMotion, GestureClick,
    Label, Orientation,
};
use rustconn_core::is_embedded_vnc_available;
#[cfg(feature = "vnc-embedded")]
use rustconn_core::{
    VncClient, VncClientCommand, VncClientConfig, VncClientEvent, VncCommandSender,
};
use std::cell::RefCell;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
#[cfg(feature = "vnc-embedded")]
use std::sync::{Arc, Mutex as StdMutex};

/// Embedded VNC widget using Wayland subsurface
///
/// This widget provides native VNC session embedding within GTK4 applications.
/// It uses a `DrawingArea` for rendering and integrates with a VNC client
/// library for protocol handling.
///
/// # Features
///
/// - Native Wayland subsurface integration
/// - Frame buffer capture and rendering
/// - Keyboard and mouse input forwarding
/// - Dynamic resolution changes on resize
/// - Automatic fallback to external vncviewer
///
/// # Example
///
/// ```ignore
/// use rustconn::embedded_vnc::{EmbeddedVncWidget, VncConfig};
///
/// let widget = EmbeddedVncWidget::new();
///
/// // Configure connection
/// let config = VncConfig::new("192.168.1.100")
///     .with_password("secret")
///     .with_resolution(1920, 1080);
///
/// // Connect
/// widget.connect(&config)?;
/// ```
#[allow(dead_code)] // Many fields kept for GTK widget lifecycle and signal handlers
pub struct EmbeddedVncWidget {
    /// Main container widget
    container: GtkBox,
    /// Toolbar with clipboard and Ctrl+Alt+Del buttons
    toolbar: GtkBox,
    /// Status label for clipboard feedback
    status_label: Label,
    /// Copy button
    copy_button: Button,
    /// Paste button
    paste_button: Button,
    /// Ctrl+Alt+Del button
    ctrl_alt_del_button: Button,
    /// Separator between buttons
    separator: gtk4::Separator,
    /// Drawing area for rendering VNC frames
    drawing_area: DrawingArea,
    /// Wayland surface handle
    wl_surface: Rc<RefCell<VncWaylandSurface>>,
    /// Pixel buffer for frame data
    pixel_buffer: Rc<RefCell<VncPixelBuffer>>,
    /// Current connection state
    state: Rc<RefCell<VncConnectionState>>,
    /// Current configuration
    config: Rc<RefCell<Option<VncConfig>>>,
    /// VNC viewer child process (for external mode)
    process: Rc<RefCell<Option<Child>>>,
    /// Whether using embedded mode or external mode
    is_embedded: Rc<RefCell<bool>>,
    /// Current widget width
    width: Rc<RefCell<u32>>,
    /// Current widget height
    height: Rc<RefCell<u32>>,
    /// VNC server framebuffer width (for coordinate transformation)
    vnc_width: Rc<RefCell<u32>>,
    /// VNC server framebuffer height (for coordinate transformation)
    vnc_height: Rc<RefCell<u32>>,
    /// State change callback
    on_state_changed: Rc<RefCell<Option<StateCallback>>>,
    /// Error callback
    on_error: Rc<RefCell<Option<ErrorCallback>>>,
    /// Frame update callback
    on_frame_update: Rc<RefCell<Option<FrameCallback>>>,
    /// Reconnect callback
    on_reconnect: Rc<RefCell<Option<Box<dyn Fn() + 'static>>>>,
    /// Reconnect button (shown when disconnected)
    reconnect_button: Button,
    /// Native VNC client (when vnc-embedded feature is enabled)
    #[cfg(feature = "vnc-embedded")]
    vnc_client: Rc<RefCell<Option<Arc<StdMutex<VncClient>>>>>,
    /// Command sender for the VNC client (when vnc-embedded feature is enabled)
    #[cfg(feature = "vnc-embedded")]
    command_sender: Rc<RefCell<Option<VncCommandSender>>>,
}

impl EmbeddedVncWidget {
    /// Creates a new embedded VNC widget
    #[must_use]
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        // Create toolbar with clipboard and Ctrl+Alt+Del buttons (right-aligned)
        let toolbar = GtkBox::new(Orientation::Horizontal, 4);
        toolbar.set_margin_start(4);
        toolbar.set_margin_end(4);
        toolbar.set_margin_top(4);
        toolbar.set_margin_bottom(4);
        toolbar.set_halign(gtk4::Align::End);

        // Status label for clipboard feedback (hidden by default)
        let status_label = Label::new(None);
        status_label.set_visible(false);
        status_label.set_margin_end(8);
        status_label.add_css_class("dim-label");
        toolbar.append(&status_label);

        // Copy button - copies from remote clipboard to local
        let copy_button = Button::with_label("Copy");
        copy_button.set_tooltip_text(Some("Copy from remote session to local clipboard"));
        toolbar.append(&copy_button);

        // Paste button - pastes from local clipboard to remote
        let paste_button = Button::with_label("Paste");
        paste_button.set_tooltip_text(Some("Paste from local clipboard to remote session"));
        toolbar.append(&paste_button);

        // Separator
        let separator = gtk4::Separator::new(Orientation::Vertical);
        separator.set_margin_start(4);
        separator.set_margin_end(4);
        toolbar.append(&separator);

        // Ctrl+Alt+Del button
        let ctrl_alt_del_button = Button::with_label("Ctrl+Alt+Del");
        ctrl_alt_del_button.add_css_class("suggested-action");
        ctrl_alt_del_button.set_tooltip_text(Some("Send Ctrl+Alt+Del to remote session"));
        toolbar.append(&ctrl_alt_del_button);

        // Reconnect button (shown when disconnected)
        let reconnect_button = Button::with_label("Reconnect");
        reconnect_button.add_css_class("suggested-action");
        reconnect_button.set_tooltip_text(Some("Reconnect to the remote session"));
        reconnect_button.set_visible(false); // Hidden by default
        toolbar.append(&reconnect_button);

        // Hide toolbar initially (show when connected)
        toolbar.set_visible(false);

        container.append(&toolbar);

        let drawing_area = DrawingArea::new();
        drawing_area.set_hexpand(true);
        drawing_area.set_vexpand(true);
        drawing_area.set_content_width(1280);
        drawing_area.set_content_height(720);
        drawing_area.set_can_focus(true);
        drawing_area.set_focusable(true);

        container.append(&drawing_area);

        let pixel_buffer = Rc::new(RefCell::new(VncPixelBuffer::new(1280, 720)));
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
                    // In embedded mode, render the pixel buffer
                    let buffer = pixel_buffer.borrow();
                    let buf_width = buffer.width();
                    let buf_height = buffer.height();
                    if buf_width > 0 && buf_height > 0 {
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
                            // Scale to fit the drawing area while maintaining aspect ratio
                            let scale_x = f64::from(width) / f64::from(buf_width);
                            let scale_y = f64::from(height) / f64::from(buf_height);
                            let scale = scale_x.min(scale_y);

                            // Center the image
                            let offset_x =
                                f64::from(buf_width).mul_add(-scale, f64::from(width)) / 2.0;
                            let offset_y =
                                f64::from(buf_height).mul_add(-scale, f64::from(height)) / 2.0;

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
        use rustconn_core::VncClientCommand;

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

                let (vnc_x, vnc_y) = crate::embedded_vnc_ui::transform_widget_to_vnc(
                    x, y, widget_w, widget_h, vnc_w, vnc_h,
                );

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

                let (vnc_x, vnc_y) = crate::embedded_vnc_ui::transform_widget_to_vnc(
                    x, y, widget_w, widget_h, vnc_w, vnc_h,
                );

                let vnc_x = crate::utils::coord_to_u16(vnc_x);
                let vnc_y = crate::utils::coord_to_u16(vnc_y);

                // Convert GTK button to VNC button mask and update state
                let button_bit = crate::embedded_vnc_ui::gtk_button_to_vnc_mask(button);
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

                let (vnc_x, vnc_y) = crate::embedded_vnc_ui::transform_widget_to_vnc(
                    x, y, widget_w, widget_h, vnc_w, vnc_h,
                );

                let vnc_x = crate::utils::coord_to_u16(vnc_x);
                let vnc_y = crate::utils::coord_to_u16(vnc_y);

                // Clear the button bit from state
                let button_bit = crate::embedded_vnc_ui::gtk_button_to_vnc_mask(button);
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
        use rustconn_core::VncClientCommand;

        let width = self.width.clone();
        let height = self.height.clone();
        let vnc_width = self.vnc_width.clone();
        let vnc_height = self.vnc_height.clone();
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let command_sender = self.command_sender.clone();

        // Track last requested resolution to avoid duplicate requests
        let last_requested: Rc<RefCell<(u32, u32)>> = Rc::new(RefCell::new((0, 0)));

        self.drawing_area
            .connect_resize(move |_area, new_width, new_height| {
                let new_width = new_width.unsigned_abs();
                let new_height = new_height.unsigned_abs();

                *width.borrow_mut() = new_width;
                *height.borrow_mut() = new_height;

                let current_state = *state.borrow();
                let embedded = *is_embedded.borrow();

                if embedded && current_state == VncConnectionState::Connected {
                    // Find the best standard resolution that fits the window
                    let (best_w, best_h) = find_best_standard_resolution(new_width, new_height);

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
                            eprintln!(
                                "[VNC] Requesting standard resolution {}x{} for window {}x{}",
                                best_w, best_h, new_width, new_height
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

        self.drawing_area
            .connect_resize(move |_area, new_width, new_height| {
                let new_width = new_width.unsigned_abs();
                let new_height = new_height.unsigned_abs();

                *width.borrow_mut() = new_width;
                *height.borrow_mut() = new_height;

                // Resize the pixel buffer
                pixel_buffer.borrow_mut().resize(new_width, new_height);
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
                                status.set_text(&format!("Pasted {char_count} chars"));
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

    /// Returns the main container widget
    #[must_use]
    pub const fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Returns the drawing area widget
    #[must_use]
    pub const fn drawing_area(&self) -> &DrawingArea {
        &self.drawing_area
    }

    /// Returns the current connection state
    #[must_use]
    pub fn state(&self) -> VncConnectionState {
        *self.state.borrow()
    }

    /// Returns whether the widget is using embedded mode
    #[must_use]
    pub fn is_embedded(&self) -> bool {
        *self.is_embedded.borrow()
    }

    /// Returns the current width
    #[must_use]
    pub fn width(&self) -> u32 {
        *self.width.borrow()
    }

    /// Returns the current height
    #[must_use]
    pub fn height(&self) -> u32 {
        *self.height.borrow()
    }

    /// Connects a callback for state changes
    pub fn connect_state_changed<F>(&self, callback: F)
    where
        F: Fn(VncConnectionState) + 'static,
    {
        let reconnect_button = self.reconnect_button.clone();
        let copy_button = self.copy_button.clone();
        let paste_button = self.paste_button.clone();
        let ctrl_alt_del_button = self.ctrl_alt_del_button.clone();
        let separator = self.separator.clone();
        let toolbar = self.toolbar.clone();

        *self.on_state_changed.borrow_mut() = Some(Box::new(move |state| {
            // Update button visibility based on state
            let show_reconnect = matches!(
                state,
                VncConnectionState::Disconnected | VncConnectionState::Error
            );

            // When showing reconnect, hide other buttons
            reconnect_button.set_visible(show_reconnect);
            copy_button.set_visible(!show_reconnect);
            paste_button.set_visible(!show_reconnect);
            ctrl_alt_del_button.set_visible(!show_reconnect);
            separator.set_visible(!show_reconnect);

            // Show toolbar when reconnect button should be visible
            if show_reconnect {
                toolbar.set_visible(true);
            }
            // Call the user's callback
            callback(state);
        }));
    }

    /// Connects a callback for errors
    pub fn connect_error<F>(&self, callback: F)
    where
        F: Fn(&str) + 'static,
    {
        *self.on_error.borrow_mut() = Some(Box::new(callback));
    }

    /// Connects a callback for frame updates
    pub fn connect_frame_update<F>(&self, callback: F)
    where
        F: Fn(u32, u32, u32, u32) + 'static,
    {
        *self.on_frame_update.borrow_mut() = Some(Box::new(callback));
    }

    /// Sets the connection state and notifies listeners
    fn set_state(&self, new_state: VncConnectionState) {
        *self.state.borrow_mut() = new_state;
        self.drawing_area.queue_draw();

        if let Some(ref callback) = *self.on_state_changed.borrow() {
            callback(new_state);
        }
    }

    /// Reports an error and notifies listeners
    fn report_error(&self, message: &str) {
        self.set_state(VncConnectionState::Error);

        if let Some(ref callback) = *self.on_error.borrow() {
            callback(message);
        }
    }
}

impl Default for EmbeddedVncWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for EmbeddedVncWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddedVncWidget")
            .field("state", &self.state.borrow())
            .field("is_embedded", &self.is_embedded.borrow())
            .field("width", &self.width.borrow())
            .field("height", &self.height.borrow())
            .finish_non_exhaustive()
    }
}

// ============================================================================
// VNC Client Integration
// ============================================================================

impl EmbeddedVncWidget {
    /// Detects if a native VNC client library is available for embedded mode
    ///
    /// Returns true if the `vnc-embedded` feature is enabled in rustconn-core,
    /// which provides a pure Rust VNC client implementation.
    #[must_use]
    pub fn detect_native_vnc() -> bool {
        // Check if the vnc-embedded feature is available in rustconn-core
        is_embedded_vnc_available()
    }

    /// Detects available VNC viewer binaries for external mode
    #[must_use]
    pub fn detect_vnc_viewer() -> Option<String> {
        let candidates = [
            "vncviewer",   // TigerVNC, TightVNC
            "gvncviewer",  // GTK-VNC viewer
            "xvnc4viewer", // RealVNC
            "vinagre",     // GNOME Vinagre (deprecated but still available)
            "remmina",     // Remmina (supports VNC)
            "krdc",        // KDE Remote Desktop Client
        ];

        for candidate in candidates {
            if Command::new("which")
                .arg(candidate)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
            {
                return Some(candidate.to_string());
            }
        }
        None
    }

    /// Connects to a VNC server
    ///
    /// This method attempts to use native VNC embedding first.
    /// If native embedding is not available, it falls back to an external VNC viewer.
    ///
    /// # Arguments
    ///
    /// * `config` - The VNC connection configuration
    ///
    /// # Errors
    ///
    /// Returns error if connection fails or no VNC client is available
    pub fn connect(&self, config: &VncConfig) -> Result<(), EmbeddedVncError> {
        // Store configuration
        *self.config.borrow_mut() = Some(config.clone());

        // Update state
        self.set_state(VncConnectionState::Connecting);

        // Try embedded mode first (native VNC library)
        if Self::detect_native_vnc() {
            match self.connect_embedded(config) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    // Log the error and fall back to external mode
                    eprintln!("Embedded VNC failed, falling back to external: {e}");
                }
            }
        }

        // Fall back to external mode (vncviewer)
        self.connect_external(config)
    }

    /// Connects using embedded mode (native VNC library)
    #[cfg(feature = "vnc-embedded")]
    fn connect_embedded(&self, config: &VncConfig) -> Result<(), EmbeddedVncError> {
        tracing::debug!(
            "[EmbeddedVNC] Attempting embedded connection to {}:{}",
            config.host,
            config.port
        );

        // Initialize Wayland surface
        self.wl_surface
            .borrow_mut()
            .initialize()
            .map_err(|e| EmbeddedVncError::SubsurfaceCreation(e.to_string()))?;

        // Create VNC client configuration
        let vnc_config = VncClientConfig::new(&config.host)
            .with_port(config.port)
            .with_shared(true)
            .with_view_only(config.view_only);

        let vnc_config = if let Some(ref password) = config.password {
            vnc_config.with_password(password)
        } else {
            vnc_config
        };

        // Create the VNC client and connect (spawns background thread)
        let mut client = VncClient::new(vnc_config);
        match client.connect() {
            Ok(()) => {
                tracing::debug!("[EmbeddedVNC] VNC client started successfully");
            }
            Err(e) => {
                tracing::error!("[EmbeddedVNC] VNC connection failed: {}", e);
                return Err(EmbeddedVncError::Connection(e.to_string()));
            }
        }

        // Store the command sender for input handling
        if let Some(sender) = client.command_sender() {
            *self.command_sender.borrow_mut() = Some(sender);
        }

        // Store the client
        let client = Arc::new(StdMutex::new(client));
        *self.vnc_client.borrow_mut() = Some(client.clone());
        *self.is_embedded.borrow_mut() = true;

        // Resize pixel buffer to match config
        self.pixel_buffer
            .borrow_mut()
            .resize(config.width, config.height);

        // Clone references for the event polling timer
        let pixel_buffer = self.pixel_buffer.clone();
        let state = self.state.clone();
        let drawing_area = self.drawing_area.clone();
        let toolbar = self.toolbar.clone();
        let on_state_changed = self.on_state_changed.clone();
        let on_error = self.on_error.clone();
        let on_frame_update = self.on_frame_update.clone();
        let vnc_width_ref = self.vnc_width.clone();
        let vnc_height_ref = self.vnc_height.clone();
        let is_embedded = self.is_embedded.clone();
        let command_sender_ref = self.command_sender.clone();
        // Store desired resolution from config for SetDesktopSize request after connect
        let desired_width = config.width;
        let desired_height = config.height;

        // Set up a GLib timeout to poll for VNC events (~60 FPS)
        glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
            use rustconn_core::VncClientCommand;

            // Check if we're still in embedded mode
            if !*is_embedded.borrow() {
                return glib::ControlFlow::Break;
            }

            // Try to get events from the VNC client
            let client_guard = match client.lock() {
                Ok(guard) => guard,
                Err(_) => return glib::ControlFlow::Break,
            };

            // Poll all available events
            while let Some(event) = client_guard.try_recv_event() {
                match event {
                    VncClientEvent::Connected => {
                        tracing::debug!("[EmbeddedVNC] Connected!");
                        *state.borrow_mut() = VncConnectionState::Connected;
                        toolbar.set_visible(true);
                        if let Some(ref callback) = *on_state_changed.borrow() {
                            callback(VncConnectionState::Connected);
                        }
                        // Request desired resolution after connection
                        // (requires server support for ExtendedDesktopSize)
                        if let Some(ref sender) = *command_sender_ref.borrow() {
                            tracing::debug!(
                                "[VNC] Requesting initial resolution {}x{}",
                                desired_width,
                                desired_height
                            );
                            // Use try_send to avoid blocking GTK main thread
                            let _ = sender.try_send(VncClientCommand::SetDesktopSize {
                                width: crate::utils::dimension_to_u16(desired_width),
                                height: crate::utils::dimension_to_u16(desired_height),
                            });
                        }
                        drawing_area.queue_draw();
                    }
                    VncClientEvent::Disconnected => {
                        tracing::debug!("[EmbeddedVNC] Disconnected");
                        *state.borrow_mut() = VncConnectionState::Disconnected;
                        toolbar.set_visible(false);
                        if let Some(ref callback) = *on_state_changed.borrow() {
                            callback(VncConnectionState::Disconnected);
                        }
                        drawing_area.queue_draw();
                        return glib::ControlFlow::Break;
                    }
                    VncClientEvent::ResolutionChanged { width, height } => {
                        tracing::debug!("[EmbeddedVNC] Resolution changed: {}x{}", width, height);
                        // Store VNC server resolution for coordinate transformation
                        *vnc_width_ref.borrow_mut() = width;
                        *vnc_height_ref.borrow_mut() = height;
                        pixel_buffer.borrow_mut().resize(width, height);
                        // Don't change widget size - let it scale the VNC image
                        drawing_area.queue_draw();
                    }
                    VncClientEvent::FrameUpdate { rect, data } => {
                        // Update the pixel buffer with the new frame data
                        let stride = u32::from(rect.width) * 4;
                        pixel_buffer.borrow_mut().update_region(
                            u32::from(rect.x),
                            u32::from(rect.y),
                            u32::from(rect.width),
                            u32::from(rect.height),
                            &data,
                            stride,
                        );
                        if let Some(ref callback) = *on_frame_update.borrow() {
                            callback(
                                u32::from(rect.x),
                                u32::from(rect.y),
                                u32::from(rect.width),
                                u32::from(rect.height),
                            );
                        }
                        drawing_area.queue_draw();
                    }
                    VncClientEvent::CopyRect { dst, src } => {
                        pixel_buffer.borrow_mut().copy_rect(
                            u32::from(src.x),
                            u32::from(src.y),
                            u32::from(dst.x),
                            u32::from(dst.y),
                            u32::from(src.width),
                            u32::from(src.height),
                        );
                        drawing_area.queue_draw();
                    }
                    VncClientEvent::Error(msg) => {
                        tracing::error!("[EmbeddedVNC] Error: {}", msg);

                        // Handle "unexpected end of file" as a disconnect rather than a hard error
                        // This often happens when the server closes the connection cleanly but abruptly
                        if msg.contains("unexpected end of file") {
                            tracing::debug!("[EmbeddedVNC] Treating EOF as disconnect");
                            *state.borrow_mut() = VncConnectionState::Disconnected;
                            toolbar.set_visible(false);
                            if let Some(ref callback) = *on_state_changed.borrow() {
                                callback(VncConnectionState::Disconnected);
                            }
                        } else {
                            *state.borrow_mut() = VncConnectionState::Error;
                            toolbar.set_visible(false);
                            if let Some(ref callback) = *on_error.borrow() {
                                callback(&msg);
                            }
                        }

                        drawing_area.queue_draw();
                        return glib::ControlFlow::Break;
                    }
                    VncClientEvent::Bell => {
                        // Could play a sound or show notification
                    }
                    VncClientEvent::ClipboardText(_text) => {
                        // Could sync with system clipboard
                    }
                    VncClientEvent::CursorUpdate { .. } => {
                        // Could update cursor shape
                    }
                    VncClientEvent::AuthRequired => {
                        // Authentication is handled during connection
                    }
                }
            }

            glib::ControlFlow::Continue
        });

        // Set initial state
        self.set_state(VncConnectionState::Connecting);

        Ok(())
    }

    /// Connects using embedded mode (fallback when vnc-embedded feature is disabled)
    #[cfg(not(feature = "vnc-embedded"))]
    fn connect_embedded(&self, _config: &VncConfig) -> Result<(), EmbeddedVncError> {
        Err(EmbeddedVncError::NativeVncNotAvailable)
    }

    /// Connects using external mode (vncviewer)
    fn connect_external(&self, config: &VncConfig) -> Result<(), EmbeddedVncError> {
        let binary = Self::detect_vnc_viewer().ok_or_else(|| {
            EmbeddedVncError::VncClientInit(
                "No VNC viewer found. Install vncviewer, gvncviewer, or remmina.".to_string(),
            )
        })?;

        let mut cmd = Command::new(&binary);

        // Build server address based on port
        let server = if config.port == 5900 {
            format!("{}:0", config.host)
        } else if config.port > 5900 && config.port < 6000 {
            let display = config.port - 5900;
            format!("{}:{display}", config.host)
        } else {
            format!("{}::{}", config.host, config.port)
        };

        // Add viewer-specific arguments based on detected binary
        match binary.as_str() {
            "vncviewer" | "xvnc4viewer" => {
                // TigerVNC/TightVNC/RealVNC style arguments
                if let Some(ref encoding) = config.encoding {
                    cmd.arg("-PreferredEncoding");
                    cmd.arg(encoding);
                }

                if let Some(quality) = config.quality {
                    cmd.arg("-QualityLevel");
                    cmd.arg(quality.to_string());
                }

                if let Some(compression) = config.compression {
                    cmd.arg("-CompressLevel");
                    cmd.arg(compression.to_string());
                }

                if config.view_only {
                    cmd.arg("-ViewOnly");
                }

                // Password file handling would go here
                // For security, we don't pass password on command line

                cmd.arg(&server);
            }
            "gvncviewer" => {
                // GTK-VNC viewer arguments
                cmd.arg(&server);
            }
            "remmina" => {
                // Remmina uses a different connection format
                cmd.arg("-c");
                cmd.arg(format!("vnc://{}", server.replace(':', "/")));
            }
            "krdc" => {
                // KDE Remote Desktop Client
                cmd.arg(format!("vnc://{}", config.host));
            }
            _ => {
                // Generic fallback
                cmd.arg(&server);
            }
        }

        // Add extra arguments
        for arg in &config.extra_args {
            cmd.arg(arg);
        }

        // Spawn the process
        match cmd.spawn() {
            Ok(child) => {
                *self.process.borrow_mut() = Some(child);
                *self.is_embedded.borrow_mut() = false;
                self.set_state(VncConnectionState::Connected);
                Ok(())
            }
            Err(e) => {
                let msg = format!("Failed to start VNC viewer: {e}");
                self.report_error(&msg);
                Err(EmbeddedVncError::Connection(msg))
            }
        }
    }

    /// Disconnects from the VNC server
    #[cfg(feature = "vnc-embedded")]
    pub fn disconnect(&self) {
        // Clear command sender first to stop input forwarding
        *self.command_sender.borrow_mut() = None;

        // Disconnect native VNC client if running
        if let Some(client) = self.vnc_client.borrow_mut().take() {
            if let Ok(mut client_guard) = client.lock() {
                client_guard.disconnect();
            }
        }

        // Kill external process if running
        if let Some(mut child) = self.process.borrow_mut().take() {
            let _ = child.kill();
            let _ = child.wait();
        }

        // Clean up Wayland surface
        self.wl_surface.borrow_mut().cleanup();

        // Clear pixel buffer
        self.pixel_buffer.borrow_mut().clear();

        // Hide toolbar
        self.toolbar.set_visible(false);

        // Reset state (but keep config for potential reconnect)
        *self.is_embedded.borrow_mut() = false;
        self.set_state(VncConnectionState::Disconnected);
    }

    /// Disconnects from the VNC server (fallback when vnc-embedded is disabled)
    #[cfg(not(feature = "vnc-embedded"))]
    pub fn disconnect(&self) {
        // Kill external process if running
        if let Some(mut child) = self.process.borrow_mut().take() {
            let _ = child.kill();
            let _ = child.wait();
        }

        // Clean up Wayland surface
        self.wl_surface.borrow_mut().cleanup();

        // Clear pixel buffer
        self.pixel_buffer.borrow_mut().clear();

        // Hide toolbar
        self.toolbar.set_visible(false);

        // Reset state (but keep config for potential reconnect)
        *self.is_embedded.borrow_mut() = false;
        self.set_state(VncConnectionState::Disconnected);
    }

    /// Reconnects using the stored configuration
    ///
    /// This method attempts to reconnect to the VNC server using the
    /// configuration from the previous connection.
    ///
    /// # Errors
    ///
    /// Returns an error if no previous configuration exists or if
    /// the connection fails.
    pub fn reconnect(&self) -> Result<(), EmbeddedVncError> {
        let config = self.config.borrow().clone();
        if let Some(config) = config {
            self.connect(&config)
        } else {
            Err(EmbeddedVncError::Connection(
                "No previous configuration to reconnect".to_string(),
            ))
        }
    }

    /// Handles VNC frame buffer update
    ///
    /// This is called when the VNC server sends a frame buffer update.
    /// The pixel data is blitted to the Wayland surface.
    ///
    /// # Arguments
    ///
    /// * `x` - X coordinate of the updated region
    /// * `y` - Y coordinate of the updated region
    /// * `width` - Width of the updated region
    /// * `height` - Height of the updated region
    /// * `data` - Pixel data for the region
    /// * `stride` - Stride of the pixel data
    pub fn on_frame_update(
        &self,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        data: &[u8],
        stride: u32,
    ) {
        // Update the pixel buffer with the new frame data
        self.pixel_buffer
            .borrow_mut()
            .update_region(x, y, width, height, data, stride);

        // Damage the Wayland surface region
        self.wl_surface.borrow().damage(
            crate::utils::dimension_to_i32(x),
            crate::utils::dimension_to_i32(y),
            crate::utils::dimension_to_i32(width),
            crate::utils::dimension_to_i32(height),
        );

        // Commit the surface
        self.wl_surface.borrow().commit();

        // Queue a redraw of the GTK widget
        self.drawing_area.queue_draw();

        // Notify frame update callback
        if let Some(ref callback) = *self.on_frame_update.borrow() {
            callback(x, y, width, height);
        }
    }

    /// Handles VNC CopyRect update
    ///
    /// CopyRect is an efficient encoding where the server tells the client
    /// to copy a region from one location to another.
    ///
    /// # Arguments
    ///
    /// * `src_x` - Source X coordinate
    /// * `src_y` - Source Y coordinate
    /// * `dst_x` - Destination X coordinate
    /// * `dst_y` - Destination Y coordinate
    /// * `width` - Width of the region
    /// * `height` - Height of the region
    pub fn on_copy_rect(
        &self,
        src_x: u32,
        src_y: u32,
        dst_x: u32,
        dst_y: u32,
        width: u32,
        height: u32,
    ) {
        // Copy the region within the pixel buffer
        self.pixel_buffer
            .borrow_mut()
            .copy_rect(src_x, src_y, dst_x, dst_y, width, height);

        // Damage the destination region
        self.wl_surface.borrow().damage(
            crate::utils::dimension_to_i32(dst_x),
            crate::utils::dimension_to_i32(dst_y),
            crate::utils::dimension_to_i32(width),
            crate::utils::dimension_to_i32(height),
        );

        // Commit the surface
        self.wl_surface.borrow().commit();

        // Queue a redraw
        self.drawing_area.queue_draw();
    }

    /// Sends a keyboard event to the VNC server
    ///
    /// # Arguments
    ///
    /// * `keysym` - X11 keysym value
    /// * `pressed` - Whether the key is pressed or released
    pub fn send_key(&self, keysym: u32, pressed: bool) {
        if !*self.is_embedded.borrow() {
            return;
        }

        if *self.state.borrow() != VncConnectionState::Connected {
            return;
        }

        if self.config.borrow().as_ref().is_some_and(|c| c.view_only) {
            return;
        }

        // In a real implementation, this would:
        // 1. Send KeyEvent message to VNC server
        // rfb_send_key_event(keysym, pressed)

        let _keysym = keysym;
        let _pressed = pressed;
    }

    /// Sends Ctrl+Alt+Del key sequence to the VNC server
    ///
    /// This is useful for Windows login screens that require this key combination.
    #[cfg(feature = "vnc-embedded")]
    pub fn send_ctrl_alt_del(&self) {
        if !*self.is_embedded.borrow() {
            return;
        }

        if *self.state.borrow() != VncConnectionState::Connected {
            return;
        }

        if self.config.borrow().as_ref().is_some_and(|c| c.view_only) {
            return;
        }

        if let Some(ref sender) = *self.command_sender.borrow() {
            use rustconn_core::VncClientCommand;
            // Use try_send to avoid blocking GTK main thread
            let _ = sender.try_send(VncClientCommand::SendCtrlAltDel);
        }
    }

    /// Sends Ctrl+Alt+Del key sequence (no-op when vnc-embedded is disabled)
    #[cfg(not(feature = "vnc-embedded"))]
    pub fn send_ctrl_alt_del(&self) {
        // No-op when vnc-embedded feature is disabled
    }

    /// Sends a mouse/pointer event to the VNC server
    ///
    /// # Arguments
    ///
    /// * `x` - X coordinate
    /// * `y` - Y coordinate
    /// * `button_mask` - Button mask (bit 0 = left, bit 1 = middle, bit 2 = right)
    pub fn send_pointer(&self, x: u16, y: u16, button_mask: u8) {
        if !*self.is_embedded.borrow() {
            return;
        }

        if *self.state.borrow() != VncConnectionState::Connected {
            return;
        }

        if self.config.borrow().as_ref().is_some_and(|c| c.view_only) {
            return;
        }

        // In a real implementation, this would:
        // 1. Send PointerEvent message to VNC server
        // rfb_send_pointer_event(x, y, button_mask)

        let _x = x;
        let _y = y;
        let _button_mask = button_mask;
    }

    /// Sends a clipboard/cut text to the VNC server
    ///
    /// # Arguments
    ///
    /// * `text` - Text to send to the server clipboard
    pub fn send_clipboard(&self, text: &str) {
        if !*self.is_embedded.borrow() {
            return;
        }

        if *self.state.borrow() != VncConnectionState::Connected {
            return;
        }

        if !self
            .config
            .borrow()
            .as_ref()
            .is_some_and(|c| c.clipboard_enabled)
        {
            return;
        }

        // In a real implementation, this would:
        // 1. Send ClientCutText message to VNC server
        // rfb_send_client_cut_text(text)

        let _text = text;
    }

    /// Requests a full frame buffer update from the server
    pub fn request_full_update(&self) {
        if !*self.is_embedded.borrow() {
            return;
        }

        if *self.state.borrow() == VncConnectionState::Connected {
            // In a real implementation, this would:
            // 1. Send FramebufferUpdateRequest for the entire screen
            // rfb_send_framebuffer_update_request(0, 0, width, height, false)
        }
    }

    /// Notifies the VNC server of a resolution change request
    ///
    /// Note: Not all VNC servers support dynamic resolution changes.
    ///
    /// # Arguments
    ///
    /// * `width` - New width in pixels
    /// * `height` - New height in pixels
    pub fn notify_resize(&self, width: u32, height: u32) {
        if !*self.is_embedded.borrow() {
            return;
        }

        if *self.state.borrow() != VncConnectionState::Connected {
            return;
        }

        // Update internal dimensions
        *self.width.borrow_mut() = width;
        *self.height.borrow_mut() = height;

        // Resize pixel buffer
        self.pixel_buffer.borrow_mut().resize(width, height);

        // In a real implementation, this would:
        // 1. Send SetDesktopSize message if server supports it
        // rfb_send_set_desktop_size(width, height)
    }

    /// Returns whether the VNC session is connected
    #[must_use]
    pub fn is_connected(&self) -> bool {
        *self.state.borrow() == VncConnectionState::Connected
    }

    /// Returns the current configuration
    #[must_use]
    pub fn config(&self) -> Option<VncConfig> {
        self.config.borrow().clone()
    }
}

impl Drop for EmbeddedVncWidget {
    fn drop(&mut self) {
        self.disconnect();
    }
}

impl crate::embedded_trait::EmbeddedWidget for EmbeddedVncWidget {
    fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    fn state(&self) -> crate::embedded_trait::EmbeddedConnectionState {
        match *self.state.borrow() {
            VncConnectionState::Disconnected => {
                crate::embedded_trait::EmbeddedConnectionState::Disconnected
            }
            VncConnectionState::Connecting => {
                crate::embedded_trait::EmbeddedConnectionState::Connecting
            }
            VncConnectionState::Connected => {
                crate::embedded_trait::EmbeddedConnectionState::Connected
            }
            VncConnectionState::Error => crate::embedded_trait::EmbeddedConnectionState::Error,
        }
    }

    fn is_embedded(&self) -> bool {
        *self.is_embedded.borrow()
    }

    fn disconnect(&self) -> Result<(), crate::embedded_trait::EmbeddedError> {
        Self::disconnect(self);
        Ok(())
    }

    fn reconnect(&self) -> Result<(), crate::embedded_trait::EmbeddedError> {
        Self::reconnect(self)
            .map_err(|e| crate::embedded_trait::EmbeddedError::ConnectionFailed(e.to_string()))
    }

    fn send_ctrl_alt_del(&self) {
        Self::send_ctrl_alt_del(self);
    }

    fn protocol_name(&self) -> &'static str {
        "VNC"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vnc_config_builder() {
        let config = VncConfig::new("server.example.com")
            .with_port(5901)
            .with_password("secret")
            .with_resolution(1920, 1080)
            .with_encoding("tight")
            .with_quality(8)
            .with_compression(6)
            .with_clipboard(true)
            .with_view_only(false);

        assert_eq!(config.host, "server.example.com");
        assert_eq!(config.port, 5901);
        assert_eq!(config.password, Some("secret".to_string()));
        assert_eq!(config.width, 1920);
        assert_eq!(config.height, 1080);
        assert_eq!(config.encoding, Some("tight".to_string()));
        assert_eq!(config.quality, Some(8));
        assert_eq!(config.compression, Some(6));
        assert!(config.clipboard_enabled);
        assert!(!config.view_only);
    }

    #[test]
    fn test_vnc_config_display_number() {
        let config = VncConfig::new("host").with_port(5900);
        assert_eq!(config.display_number(), 0);

        let config = VncConfig::new("host").with_port(5901);
        assert_eq!(config.display_number(), 1);

        let config = VncConfig::new("host").with_port(5910);
        assert_eq!(config.display_number(), 10);

        // Raw port (outside 5900-5999 range)
        let config = VncConfig::new("host").with_port(6000);
        assert_eq!(config.display_number(), -1);

        let config = VncConfig::new("host").with_port(5800);
        assert_eq!(config.display_number(), -1);
    }

    #[test]
    fn test_vnc_config_quality_clamping() {
        let config = VncConfig::new("host").with_quality(15);
        assert_eq!(config.quality, Some(9)); // Clamped to max 9

        let config = VncConfig::new("host").with_compression(20);
        assert_eq!(config.compression, Some(9)); // Clamped to max 9
    }

    #[test]
    fn test_pixel_buffer_new() {
        let buffer = VncPixelBuffer::new(100, 50);
        assert_eq!(buffer.width(), 100);
        assert_eq!(buffer.height(), 50);
        assert_eq!(buffer.stride(), 400); // 100 * 4 bytes per pixel
        assert_eq!(buffer.bpp(), 32);
        assert_eq!(buffer.data().len(), 20000); // 100 * 50 * 4
    }

    #[test]
    fn test_pixel_buffer_resize() {
        let mut buffer = VncPixelBuffer::new(100, 50);
        buffer.resize(200, 100);
        assert_eq!(buffer.width(), 200);
        assert_eq!(buffer.height(), 100);
        assert_eq!(buffer.stride(), 800);
        assert_eq!(buffer.data().len(), 80000);
    }

    #[test]
    fn test_pixel_buffer_clear() {
        let mut buffer = VncPixelBuffer::new(10, 10);
        buffer.data_mut()[0] = 255;
        buffer.data_mut()[100] = 128;
        buffer.clear();
        assert!(buffer.data().iter().all(|&b| b == 0));
    }

    #[test]
    fn test_pixel_buffer_update_region() {
        let mut buffer = VncPixelBuffer::new(10, 10);

        // Create a 2x2 red region (BGRA format: B=0, G=0, R=255, A=255)
        let src_data = vec![
            0, 0, 255, 255, // Pixel (0,0)
            0, 0, 255, 255, // Pixel (1,0)
            0, 0, 255, 255, // Pixel (0,1)
            0, 0, 255, 255, // Pixel (1,1)
        ];

        buffer.update_region(2, 2, 2, 2, &src_data, 8);

        // Check that the region was updated
        let stride = buffer.stride() as usize;
        let offset = 2 * stride + 2 * 4; // Row 2, Column 2
        assert_eq!(buffer.data()[offset + 2], 255); // Red channel
    }

    #[test]
    fn test_pixel_buffer_copy_rect() {
        let mut buffer = VncPixelBuffer::new(10, 10);

        // Set a pixel at (1, 1) to red
        let stride = buffer.stride() as usize;
        let src_offset = stride + 4; // row 1, col 1
        buffer.data_mut()[src_offset] = 0; // B
        buffer.data_mut()[src_offset + 1] = 0; // G
        buffer.data_mut()[src_offset + 2] = 255; // R
        buffer.data_mut()[src_offset + 3] = 255; // A

        // Copy 1x1 region from (1,1) to (5,5)
        buffer.copy_rect(1, 1, 5, 5, 1, 1);

        // Check destination
        let dst_offset = 5 * stride + 5 * 4;
        assert_eq!(buffer.data()[dst_offset + 2], 255); // Red channel
    }

    #[test]
    fn test_wayland_surface_handle() {
        let mut handle = VncWaylandSurface::new();
        assert!(!handle.is_initialized());

        handle.initialize().unwrap();
        assert!(handle.is_initialized());

        handle.cleanup();
        assert!(!handle.is_initialized());
    }

    #[test]
    fn test_vnc_connection_state_display() {
        assert_eq!(VncConnectionState::Disconnected.to_string(), "Disconnected");
        assert_eq!(VncConnectionState::Connecting.to_string(), "Connecting");
        assert_eq!(VncConnectionState::Connected.to_string(), "Connected");
        assert_eq!(VncConnectionState::Error.to_string(), "Error");
    }

    #[test]
    fn test_embedded_vnc_error_display() {
        let err = EmbeddedVncError::NativeVncNotAvailable;
        assert!(err.to_string().contains("Native VNC client not available"));

        let err = EmbeddedVncError::Connection("timeout".to_string());
        assert!(err.to_string().contains("timeout"));

        let err = EmbeddedVncError::AuthenticationFailed("wrong password".to_string());
        assert!(err.to_string().contains("wrong password"));
    }

    #[test]
    fn test_vnc_config_default() {
        let config = VncConfig::default();
        assert!(config.host.is_empty());
        assert_eq!(config.port, 0);
        assert!(config.password.is_none());
        assert_eq!(config.width, 0);
        assert_eq!(config.height, 0);
    }

    #[test]
    fn test_vnc_config_extra_args() {
        let config = VncConfig::new("host")
            .with_extra_args(vec!["-FullScreen".to_string(), "-Shared".to_string()]);

        assert_eq!(config.extra_args.len(), 2);
        assert_eq!(config.extra_args[0], "-FullScreen");
        assert_eq!(config.extra_args[1], "-Shared");
    }
}
