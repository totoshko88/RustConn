//! Embedded SPICE widget for native SPICE session embedding
//!
//! This module provides the `EmbeddedSpiceWidget` struct that enables native
//! SPICE session embedding within GTK4 applications using the `spice-client` crate.
//!
//! # Architecture
//!
//! The widget uses a `DrawingArea` for rendering SPICE frames and handles:
//! - Connection lifecycle management
//! - Framebuffer rendering from SPICE client events
//! - Keyboard and mouse input forwarding
//! - Fallback to external viewer (remote-viewer) when native fails
//!
//! # Requirements Coverage
//!
//! - Requirement 9.1: Native SPICE embedding as GTK widget
//! - Requirement 9.2: Display rendering in embedded mode
//! - Requirement 9.3: Keyboard and mouse input forwarding
//! - Requirement 9.4: Fallback to external viewer

use crate::i18n::i18n;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, DrawingArea, Label, Orientation};
use std::cell::RefCell;
use std::process::Child;
use std::rc::Rc;

#[cfg(feature = "spice-embedded")]
use gtk4::glib;
#[cfg(feature = "spice-embedded")]
use rustconn_core::spice_client::{SpiceClient, SpiceClientCommand, SpiceClientEvent};
use rustconn_core::spice_client::{SpiceClientConfig, SpiceClientError};

/// Connection state for embedded SPICE widget
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpiceConnectionState {
    /// Not connected
    #[default]
    Disconnected,
    /// Connection in progress
    Connecting,
    /// Connected and rendering
    Connected,
    /// Connection error occurred
    Error,
}

impl std::fmt::Display for SpiceConnectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => write!(f, "Disconnected"),
            Self::Connecting => write!(f, "Connecting"),
            Self::Connected => write!(f, "Connected"),
            Self::Error => write!(f, "Error"),
        }
    }
}

/// Pixel buffer for SPICE frame data
#[derive(Debug)]
pub struct SpicePixelBuffer {
    /// Raw pixel data in BGRA format
    data: Vec<u8>,
    /// Buffer width in pixels
    width: u32,
    /// Buffer height in pixels
    height: u32,
    /// Stride (bytes per row)
    stride: u32,
}

impl SpicePixelBuffer {
    /// Creates a new pixel buffer with the specified dimensions
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        let stride = width * 4; // 4 bytes per pixel (BGRA)
        let size = (stride * height) as usize;
        Self {
            data: vec![0; size],
            width,
            height,
            stride,
        }
    }

    /// Returns the buffer width
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Returns the buffer height
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Returns a reference to the raw pixel data
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Resizes the buffer to new dimensions
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.stride = width * 4;
        let size = (self.stride * height) as usize;
        self.data.resize(size, 0);
    }

    /// Clears the buffer to black
    pub fn clear(&mut self) {
        self.data.fill(0);
    }

    /// Updates a region of the buffer with raw pixel data
    pub fn update_region(
        &mut self,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        src_data: &[u8],
        src_stride: u32,
    ) {
        let dst_stride = self.stride as usize;
        let src_stride = src_stride as usize;
        let bytes_per_pixel = 4;

        for row in 0..h {
            let dst_y = (y + row) as usize;
            if dst_y >= self.height as usize {
                break;
            }

            let dst_offset = dst_y * dst_stride + (x as usize * bytes_per_pixel);
            let src_offset = row as usize * src_stride;
            let copy_width =
                (w as usize * bytes_per_pixel).min(dst_stride - (x as usize * bytes_per_pixel));

            if src_offset + copy_width <= src_data.len()
                && dst_offset + copy_width <= self.data.len()
            {
                self.data[dst_offset..dst_offset + copy_width]
                    .copy_from_slice(&src_data[src_offset..src_offset + copy_width]);
            }
        }
    }
}

/// Callback type for state change notifications
type StateCallback = Box<dyn Fn(SpiceConnectionState) + 'static>;

/// Callback type for error notifications
type ErrorCallback = Box<dyn Fn(&str) + 'static>;

/// Embedded SPICE widget using native spice-client
///
/// This widget provides native SPICE session embedding within GTK4 applications.
/// It uses a `DrawingArea` for rendering and integrates with the SPICE client
/// from `rustconn-core`.
#[allow(dead_code)] // Many fields kept for GTK widget lifecycle and signal handlers
pub struct EmbeddedSpiceWidget {
    /// Main container widget
    container: GtkBox,
    /// Toolbar with clipboard and special key buttons
    toolbar: GtkBox,
    /// Status label for feedback
    status_label: Label,
    /// Copy button
    copy_button: Button,
    /// Paste button
    paste_button: Button,
    /// Ctrl+Alt+Del button
    ctrl_alt_del_button: Button,
    /// Separator between buttons
    separator: gtk4::Separator,
    /// Drawing area for rendering SPICE frames
    drawing_area: DrawingArea,
    /// Pixel buffer for frame data
    pixel_buffer: Rc<RefCell<SpicePixelBuffer>>,
    /// Current connection state
    state: Rc<RefCell<SpiceConnectionState>>,
    /// Current configuration
    config: Rc<RefCell<Option<SpiceClientConfig>>>,
    /// External viewer child process (for fallback mode)
    process: Rc<RefCell<Option<Child>>>,
    /// Whether using embedded mode or external mode
    is_embedded: Rc<RefCell<bool>>,
    /// Current widget width
    width: Rc<RefCell<u32>>,
    /// Current widget height
    height: Rc<RefCell<u32>>,
    /// SPICE server framebuffer width
    spice_width: Rc<RefCell<u32>>,
    /// SPICE server framebuffer height
    spice_height: Rc<RefCell<u32>>,
    /// State change callback
    on_state_changed: Rc<RefCell<Option<StateCallback>>>,
    /// Error callback
    on_error: Rc<RefCell<Option<ErrorCallback>>>,
    /// Reconnect callback
    on_reconnect: Rc<RefCell<Option<Box<dyn Fn() + 'static>>>>,
    /// Reconnect button (shown when disconnected)
    reconnect_button: Button,
    /// Native SPICE client (when spice-embedded feature is enabled)
    #[cfg(feature = "spice-embedded")]
    spice_client: Rc<RefCell<Option<SpiceClient>>>,
    /// Command sender for the SPICE client
    #[cfg(feature = "spice-embedded")]
    command_sender: Rc<RefCell<Option<rustconn_core::SpiceCommandSender>>>,
}

impl EmbeddedSpiceWidget {
    /// Creates a new embedded SPICE widget
    #[must_use]
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        // Create toolbar with clipboard and Ctrl+Alt+Del buttons
        let toolbar = GtkBox::new(Orientation::Horizontal, 4);
        toolbar.set_margin_start(4);
        toolbar.set_margin_end(4);
        toolbar.set_margin_top(4);
        toolbar.set_margin_bottom(4);
        toolbar.set_halign(gtk4::Align::End);

        // Status label for feedback
        let status_label = Label::new(None);
        status_label.set_visible(false);
        status_label.set_margin_end(8);
        status_label.add_css_class("dim-label");
        toolbar.append(&status_label);

        // Copy button
        let copy_button = Button::with_label(&i18n("Copy"));
        copy_button.set_tooltip_text(Some(&i18n("Copy from remote session to local clipboard")));
        toolbar.append(&copy_button);

        // Paste button
        let paste_button = Button::with_label(&i18n("Paste"));
        paste_button.set_tooltip_text(Some(&i18n("Paste from local clipboard to remote session")));
        toolbar.append(&paste_button);

        // Separator
        let separator = gtk4::Separator::new(Orientation::Vertical);
        separator.set_margin_start(4);
        separator.set_margin_end(4);
        toolbar.append(&separator);

        // Ctrl+Alt+Del button
        let ctrl_alt_del_button = Button::with_label("Ctrl+Alt+Del");
        ctrl_alt_del_button.add_css_class("suggested-action");
        ctrl_alt_del_button.set_tooltip_text(Some(&i18n("Send Ctrl+Alt+Del to remote session")));
        toolbar.append(&ctrl_alt_del_button);

        // Reconnect button (shown when disconnected)
        let reconnect_button = Button::with_label(&i18n("Reconnect"));
        reconnect_button.add_css_class("suggested-action");
        reconnect_button.set_tooltip_text(Some(&i18n("Reconnect to the remote session")));
        reconnect_button.set_visible(false); // Hidden by default
        toolbar.append(&reconnect_button);

        // Hide toolbar initially
        toolbar.set_visible(false);

        container.append(&toolbar);

        let drawing_area = DrawingArea::new();
        drawing_area.set_hexpand(true);
        drawing_area.set_vexpand(true);
        drawing_area.set_can_focus(true);
        drawing_area.set_focusable(true);

        container.append(&drawing_area);

        let pixel_buffer = Rc::new(RefCell::new(SpicePixelBuffer::new(1280, 720)));
        let state = Rc::new(RefCell::new(SpiceConnectionState::Disconnected));
        let width = Rc::new(RefCell::new(1280u32));
        let height = Rc::new(RefCell::new(720u32));
        let spice_width = Rc::new(RefCell::new(1280u32));
        let spice_height = Rc::new(RefCell::new(720u32));

        let widget = Self {
            container,
            toolbar,
            status_label,
            copy_button: copy_button.clone(),
            paste_button: paste_button.clone(),
            ctrl_alt_del_button: ctrl_alt_del_button.clone(),
            separator,
            drawing_area,
            pixel_buffer,
            state,
            config: Rc::new(RefCell::new(None)),
            process: Rc::new(RefCell::new(None)),
            is_embedded: Rc::new(RefCell::new(false)),
            width,
            height,
            spice_width,
            spice_height,
            on_state_changed: Rc::new(RefCell::new(None)),
            on_error: Rc::new(RefCell::new(None)),
            on_reconnect: Rc::new(RefCell::new(None)),
            reconnect_button,
            #[cfg(feature = "spice-embedded")]
            spice_client: Rc::new(RefCell::new(None)),
            #[cfg(feature = "spice-embedded")]
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

    /// Sets up visibility handler to redraw when widget becomes visible
    fn setup_visibility_handler(&self) {
        let drawing_area = self.drawing_area.clone();
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

        self.drawing_area.set_draw_func(move |_area, cr, w, h| {
            use gtk4::cairo;

            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();

            // Fill background
            cr.set_source_rgb(0.1, 0.1, 0.1);
            let _ = cr.paint();

            if embedded && current_state == SpiceConnectionState::Connected {
                // Render the pixel buffer
                let buffer = pixel_buffer.borrow();
                let buf_w = crate::utils::dimension_to_i32(buffer.width());
                let buf_h = crate::utils::dimension_to_i32(buffer.height());

                if buf_w > 0 && buf_h > 0 && !buffer.data().is_empty() {
                    // Calculate scaling to fit widget while maintaining aspect ratio
                    let scale_x = f64::from(w) / f64::from(buf_w);
                    let scale_y = f64::from(h) / f64::from(buf_h);
                    let scale = scale_x.min(scale_y);

                    let scaled_w = (f64::from(buf_w) * scale) as i32;
                    let scaled_h = (f64::from(buf_h) * scale) as i32;
                    let offset_x = (w - scaled_w) / 2;
                    let offset_y = (h - scaled_h) / 2;

                    // Create image surface from pixel buffer
                    let stride = cairo::Format::ARgb32
                        .stride_for_width(buffer.width())
                        .unwrap_or(buf_w * 4);

                    if let Ok(surface) = cairo::ImageSurface::create_for_data(
                        buffer.data().to_vec(),
                        cairo::Format::ARgb32,
                        buf_w,
                        buf_h,
                        stride,
                    ) {
                        cr.translate(f64::from(offset_x), f64::from(offset_y));
                        cr.scale(scale, scale);
                        let _ = cr.set_source_surface(&surface, 0.0, 0.0);
                        let _ = cr.paint();
                    }
                }
            } else {
                // Show status text
                cr.set_source_rgb(0.7, 0.7, 0.7);
                cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
                cr.set_font_size(13.0);

                let status_text = match current_state {
                    SpiceConnectionState::Disconnected => "Session ended",
                    SpiceConnectionState::Connecting => "Connecting...",
                    SpiceConnectionState::Connected if !embedded => {
                        "Session running in external window"
                    }
                    SpiceConnectionState::Connected => "Connected",
                    SpiceConnectionState::Error => "Connection error",
                };

                let color = match current_state {
                    SpiceConnectionState::Connected => (0.6, 0.8, 0.6),
                    SpiceConnectionState::Connecting => (0.8, 0.8, 0.6),
                    SpiceConnectionState::Error => (0.8, 0.4, 0.4),
                    SpiceConnectionState::Disconnected => (0.8, 0.4, 0.4),
                };
                cr.set_source_rgb(color.0, color.1, color.2);

                if let Ok(extents) = cr.text_extents(status_text) {
                    let x = (f64::from(w) - extents.width()) / 2.0;
                    let y = f64::midpoint(f64::from(h), extents.height());
                    cr.move_to(x, y);
                    let _ = cr.show_text(status_text);
                }
            }
        });
    }

    /// Sets up keyboard and mouse input handlers
    fn setup_input_handlers(&self) {
        #[cfg(feature = "spice-embedded")]
        {
            let command_sender = self.command_sender.clone();
            let state = self.state.clone();
            let is_embedded = self.is_embedded.clone();

            // Keyboard event controller
            let key_controller = gtk4::EventControllerKey::new();
            let cmd_sender_key = command_sender.clone();
            let state_key = state.clone();
            let is_embedded_key = is_embedded.clone();

            key_controller.connect_key_pressed(move |_, _keyval, keycode, _| {
                let current_state = *state_key.borrow();
                let embedded = *is_embedded_key.borrow();

                if embedded && current_state == SpiceConnectionState::Connected {
                    if let Some(ref sender) = *cmd_sender_key.borrow() {
                        let _ = sender.send(SpiceClientCommand::KeyEvent {
                            scancode: keycode,
                            pressed: true,
                        });
                    }
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });

            let cmd_sender_release = command_sender.clone();
            let state_release = state.clone();
            let is_embedded_release = is_embedded.clone();

            key_controller.connect_key_released(move |_, _keyval, keycode, _| {
                let current_state = *state_release.borrow();
                let embedded = *is_embedded_release.borrow();

                if embedded && current_state == SpiceConnectionState::Connected {
                    if let Some(ref sender) = *cmd_sender_release.borrow() {
                        let _ = sender.send(SpiceClientCommand::KeyEvent {
                            scancode: keycode,
                            pressed: false,
                        });
                    }
                }
            });

            self.drawing_area.add_controller(key_controller);

            // Mouse motion controller
            let motion_controller = gtk4::EventControllerMotion::new();
            let cmd_sender_motion = command_sender.clone();
            let state_motion = state.clone();
            let is_embedded_motion = is_embedded.clone();
            let width_motion = self.width.clone();
            let height_motion = self.height.clone();
            let spice_width_motion = self.spice_width.clone();
            let spice_height_motion = self.spice_height.clone();

            motion_controller.connect_motion(move |_, x, y| {
                let current_state = *state_motion.borrow();
                let embedded = *is_embedded_motion.borrow();

                if embedded && current_state == SpiceConnectionState::Connected {
                    let widget_w = f64::from(*width_motion.borrow());
                    let widget_h = f64::from(*height_motion.borrow());
                    let spice_w = f64::from(*spice_width_motion.borrow());
                    let spice_h = f64::from(*spice_height_motion.borrow());

                    if widget_w > 0.0 && widget_h > 0.0 && spice_w > 0.0 && spice_h > 0.0 {
                        let scale_x = widget_w / spice_w;
                        let scale_y = widget_h / spice_h;
                        let scale = scale_x.min(scale_y);

                        let scaled_w = spice_w * scale;
                        let scaled_h = spice_h * scale;
                        let offset_x = (widget_w - scaled_w) / 2.0;
                        let offset_y = (widget_h - scaled_h) / 2.0;

                        let rel_x = (x - offset_x) / scale;
                        let rel_y = (y - offset_y) / scale;

                        #[allow(clippy::cast_sign_loss)]
                        let spice_x = rel_x.clamp(0.0, spice_w - 1.0) as u16;
                        #[allow(clippy::cast_sign_loss)]
                        let spice_y = rel_y.clamp(0.0, spice_h - 1.0) as u16;

                        if let Some(ref sender) = *cmd_sender_motion.borrow() {
                            let _ = sender.send(SpiceClientCommand::PointerEvent {
                                x: spice_x,
                                y: spice_y,
                                buttons: 0,
                            });
                        }
                    }
                }
            });

            self.drawing_area.add_controller(motion_controller);

            // Mouse click controller
            let click_controller = gtk4::GestureClick::new();
            click_controller.set_button(0); // All buttons
            let cmd_sender_click = command_sender.clone();
            let state_click = state.clone();
            let is_embedded_click = is_embedded.clone();

            click_controller.connect_pressed(move |gesture, _n_press, _x, _y| {
                let current_state = *state_click.borrow();
                let embedded = *is_embedded_click.borrow();

                if embedded && current_state == SpiceConnectionState::Connected {
                    let button = gesture.current_button();
                    let button_mask = match button {
                        1 => 1, // Left
                        2 => 2, // Middle
                        3 => 4, // Right
                        _ => 0,
                    };

                    if let Some(ref sender) = *cmd_sender_click.borrow() {
                        let _ = sender.send(SpiceClientCommand::PointerEvent {
                            x: 0,
                            y: 0,
                            buttons: button_mask,
                        });
                    }
                }
            });

            let cmd_sender_release = command_sender.clone();
            let state_release = state.clone();
            let is_embedded_release = is_embedded.clone();

            click_controller.connect_released(move |_gesture, _n_press, _x, _y| {
                let current_state = *state_release.borrow();
                let embedded = *is_embedded_release.borrow();

                if embedded && current_state == SpiceConnectionState::Connected {
                    if let Some(ref sender) = *cmd_sender_release.borrow() {
                        let _ = sender.send(SpiceClientCommand::PointerEvent {
                            x: 0,
                            y: 0,
                            buttons: 0,
                        });
                    }
                }
            });

            self.drawing_area.add_controller(click_controller);

            // Scroll controller
            let scroll_controller = gtk4::EventControllerScroll::new(
                gtk4::EventControllerScrollFlags::VERTICAL
                    | gtk4::EventControllerScrollFlags::HORIZONTAL,
            );
            let cmd_sender_scroll = command_sender;
            let state_scroll = state;
            let is_embedded_scroll = is_embedded;

            scroll_controller.connect_scroll(move |_, dx, dy| {
                let current_state = *state_scroll.borrow();
                let embedded = *is_embedded_scroll.borrow();

                if embedded && current_state == SpiceConnectionState::Connected {
                    if let Some(ref sender) = *cmd_sender_scroll.borrow() {
                        let _ = sender.send(SpiceClientCommand::WheelEvent {
                            horizontal: (dx * 120.0) as i16,
                            vertical: (-dy * 120.0) as i16,
                        });
                    }
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });

            self.drawing_area.add_controller(scroll_controller);
        }
    }

    /// Sets up resize handler
    fn setup_resize_handler(&self) {
        let width = self.width.clone();
        let height = self.height.clone();

        self.drawing_area.connect_resize(move |_, w, h| {
            if w >= 0 && h >= 0 {
                if let Ok(w_u32) = u32::try_from(w) {
                    *width.borrow_mut() = w_u32;
                }
                if let Ok(h_u32) = u32::try_from(h) {
                    *height.borrow_mut() = h_u32;
                }
            }
        });
    }

    /// Sets up clipboard buttons
    fn setup_clipboard_buttons(&self, copy_btn: &Button, paste_btn: &Button) {
        #[cfg(feature = "spice-embedded")]
        {
            let command_sender = self.command_sender.clone();
            let status_label = self.status_label.clone();

            // Paste button - send local clipboard to remote
            let cmd_sender_paste = command_sender.clone();
            let status_paste = status_label.clone();
            paste_btn.connect_clicked(move |_| {
                if let Some(ref sender) = *cmd_sender_paste.borrow() {
                    // Get clipboard text and send to remote
                    let display = gtk4::gdk::Display::default();
                    if let Some(display) = display {
                        let clipboard = display.clipboard();
                        let sender_clone = sender.clone();
                        let status_clone = status_paste.clone();
                        clipboard.read_text_async(None::<&gtk4::gio::Cancellable>, move |result| {
                            if let Ok(Some(text)) = result {
                                let _ = sender_clone
                                    .send(SpiceClientCommand::ClipboardText(text.to_string()));
                                status_clone.set_text("Pasted to remote");
                                status_clone.set_visible(true);
                                glib::timeout_add_seconds_local_once(2, move || {
                                    status_clone.set_visible(false);
                                });
                            }
                        });
                    }
                }
            });

            // Copy button - request clipboard from remote (handled via events)
            copy_btn.connect_clicked(move |_| {
                status_label.set_text("Copy requested");
                status_label.set_visible(true);
                let status_clone = status_label.clone();
                glib::timeout_add_seconds_local_once(2, move || {
                    status_clone.set_visible(false);
                });
            });
        }

        #[cfg(not(feature = "spice-embedded"))]
        {
            let _ = copy_btn;
            let _ = paste_btn;
        }
    }

    /// Sets up Ctrl+Alt+Del button
    fn setup_ctrl_alt_del_button(&self, btn: &Button) {
        #[cfg(feature = "spice-embedded")]
        {
            let command_sender = self.command_sender.clone();

            btn.connect_clicked(move |_| {
                if let Some(ref sender) = *command_sender.borrow() {
                    let _ = sender.send(SpiceClientCommand::SendCtrlAltDel);
                }
            });
        }

        #[cfg(not(feature = "spice-embedded"))]
        {
            let _ = btn;
        }
    }

    /// Returns the main container widget
    #[must_use]
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Returns the current connection state
    #[must_use]
    pub fn state(&self) -> SpiceConnectionState {
        *self.state.borrow()
    }

    /// Connects a callback for state changes
    pub fn connect_state_changed<F>(&self, callback: F)
    where
        F: Fn(SpiceConnectionState) + 'static,
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
                SpiceConnectionState::Disconnected | SpiceConnectionState::Error
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

    /// Sets the connection state and notifies listeners
    fn set_state(&self, new_state: SpiceConnectionState) {
        *self.state.borrow_mut() = new_state;
        self.drawing_area.queue_draw();

        if let Some(ref callback) = *self.on_state_changed.borrow() {
            callback(new_state);
        }
    }

    /// Reports an error and notifies listeners
    fn report_error(&self, message: &str) {
        self.set_state(SpiceConnectionState::Error);

        if let Some(ref callback) = *self.on_error.borrow() {
            callback(message);
        }
    }

    /// Connects to a SPICE server
    ///
    /// Attempts native embedded connection first, falls back to external viewer.
    pub fn connect(&self, config: &SpiceClientConfig) -> Result<(), SpiceClientError> {
        *self.config.borrow_mut() = Some(config.clone());
        self.set_state(SpiceConnectionState::Connecting);

        // Try native embedded mode first
        #[cfg(feature = "spice-embedded")]
        {
            if rustconn_core::is_embedded_spice_available() {
                match self.connect_native(config) {
                    Ok(()) => return Ok(()),
                    Err(e) => {
                        tracing::warn!(%e, "Native SPICE connection failed, trying fallback");
                    }
                }
            }
        }

        // Fallback to external viewer
        self.connect_external(config)
    }

    /// Connects using native SPICE client
    #[cfg(feature = "spice-embedded")]
    fn connect_native(&self, config: &SpiceClientConfig) -> Result<(), SpiceClientError> {
        let mut client = SpiceClient::new(config.clone());

        // Connect and get channels
        client.connect()?;

        let event_rx = client
            .take_event_receiver()
            .ok_or_else(|| SpiceClientError::ConnectionFailed("No event receiver".to_string()))?;

        let command_tx = client
            .command_sender()
            .ok_or_else(|| SpiceClientError::ConnectionFailed("No command sender".to_string()))?;

        *self.command_sender.borrow_mut() = Some(command_tx);
        *self.spice_client.borrow_mut() = Some(client);
        *self.is_embedded.borrow_mut() = true;

        // Show toolbar for embedded mode
        self.toolbar.set_visible(true);

        // Start event polling
        self.start_event_polling(event_rx);

        Ok(())
    }

    /// Starts polling for SPICE client events
    #[cfg(feature = "spice-embedded")]
    fn start_event_polling(&self, event_rx: rustconn_core::SpiceEventReceiver) {
        let pixel_buffer = self.pixel_buffer.clone();
        let state = self.state.clone();
        let drawing_area = self.drawing_area.clone();
        let toolbar = self.toolbar.clone();
        let spice_width = self.spice_width.clone();
        let spice_height = self.spice_height.clone();
        let on_state_changed = self.on_state_changed.clone();
        let on_error = self.on_error.clone();

        glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
            // Poll for events
            while let Ok(event) = event_rx.try_recv() {
                match event {
                    SpiceClientEvent::Connected { width, height } => {
                        tracing::info!(width, height, "SPICE connected");
                        *state.borrow_mut() = SpiceConnectionState::Connected;
                        *spice_width.borrow_mut() = u32::from(width);
                        *spice_height.borrow_mut() = u32::from(height);
                        pixel_buffer
                            .borrow_mut()
                            .resize(u32::from(width), u32::from(height));
                        toolbar.set_visible(true);
                        if let Some(ref callback) = *on_state_changed.borrow() {
                            callback(SpiceConnectionState::Connected);
                        }
                        drawing_area.queue_draw();
                    }
                    SpiceClientEvent::Disconnected => {
                        tracing::info!("SPICE disconnected");
                        *state.borrow_mut() = SpiceConnectionState::Disconnected;
                        toolbar.set_visible(false);
                        if let Some(ref callback) = *on_state_changed.borrow() {
                            callback(SpiceConnectionState::Disconnected);
                        }
                        drawing_area.queue_draw();
                        return glib::ControlFlow::Break;
                    }
                    SpiceClientEvent::ResolutionChanged { width, height } => {
                        tracing::debug!(width, height, "SPICE resolution changed");
                        *spice_width.borrow_mut() = u32::from(width);
                        *spice_height.borrow_mut() = u32::from(height);
                        pixel_buffer
                            .borrow_mut()
                            .resize(u32::from(width), u32::from(height));
                        drawing_area.queue_draw();
                    }
                    SpiceClientEvent::FrameUpdate { rect, data } => {
                        let stride = u32::from(rect.width) * 4;
                        pixel_buffer.borrow_mut().update_region(
                            u32::from(rect.x),
                            u32::from(rect.y),
                            u32::from(rect.width),
                            u32::from(rect.height),
                            &data,
                            stride,
                        );
                        drawing_area.queue_draw();
                    }
                    SpiceClientEvent::FullFrameUpdate {
                        width,
                        height,
                        data,
                    } => {
                        *spice_width.borrow_mut() = u32::from(width);
                        *spice_height.borrow_mut() = u32::from(height);
                        let mut buffer = pixel_buffer.borrow_mut();
                        buffer.resize(u32::from(width), u32::from(height));
                        let stride = u32::from(width) * 4;
                        buffer.update_region(
                            0,
                            0,
                            u32::from(width),
                            u32::from(height),
                            &data,
                            stride,
                        );
                        drop(buffer);
                        drawing_area.queue_draw();
                    }
                    SpiceClientEvent::Error(msg) => {
                        tracing::error!(error = %msg, "SPICE client error");
                        *state.borrow_mut() = SpiceConnectionState::Error;
                        toolbar.set_visible(false);
                        if let Some(ref callback) = *on_error.borrow() {
                            callback(&msg);
                        }
                        drawing_area.queue_draw();
                    }
                    SpiceClientEvent::ClipboardText(text) => {
                        // Copy to local clipboard
                        if let Some(display) = gtk4::gdk::Display::default() {
                            let clipboard = display.clipboard();
                            clipboard.set_text(&text);
                        }
                    }
                    _ => {}
                }
            }

            glib::ControlFlow::Continue
        });
    }

    /// Connects using external SPICE viewer (fallback)
    fn connect_external(&self, config: &SpiceClientConfig) -> Result<(), SpiceClientError> {
        use rustconn_core::spice_client::{SpiceViewerLaunchResult, launch_spice_viewer};

        match launch_spice_viewer(config) {
            SpiceViewerLaunchResult::Launched { viewer, pid } => {
                tracing::info!(%viewer, ?pid, "Launched external SPICE viewer");
                *self.is_embedded.borrow_mut() = false;
                self.set_state(SpiceConnectionState::Connected);
                // Hide toolbar for external mode
                self.toolbar.set_visible(false);
                Ok(())
            }
            SpiceViewerLaunchResult::NoViewerFound => {
                self.report_error("No SPICE viewer found (install remote-viewer or virt-viewer)");
                Err(SpiceClientError::ConnectionFailed(
                    "No SPICE viewer found".to_string(),
                ))
            }
            SpiceViewerLaunchResult::LaunchFailed(msg) => {
                self.report_error(&format!("Failed to launch viewer: {msg}"));
                Err(SpiceClientError::ConnectionFailed(msg))
            }
        }
    }

    /// Disconnects from the SPICE server
    pub fn disconnect(&self) {
        #[cfg(feature = "spice-embedded")]
        {
            if let Some(ref sender) = *self.command_sender.borrow() {
                let _ = sender.send(SpiceClientCommand::Disconnect);
            }
            *self.spice_client.borrow_mut() = None;
            *self.command_sender.borrow_mut() = None;
        }

        // Kill external process if any
        if let Some(mut process) = self.process.borrow_mut().take() {
            let _ = process.kill();
        }

        *self.is_embedded.borrow_mut() = false;
        self.toolbar.set_visible(false);
        self.set_state(SpiceConnectionState::Disconnected);
    }

    /// Reconnects using the stored configuration
    ///
    /// This method attempts to reconnect to the SPICE server using the
    /// configuration from the previous connection.
    ///
    /// # Errors
    ///
    /// Returns an error if no previous configuration exists or if
    /// the connection fails.
    pub fn reconnect(&self) -> Result<(), SpiceClientError> {
        let config = self.config.borrow().clone();
        if let Some(config) = config {
            self.connect(&config)
        } else {
            Err(SpiceClientError::ConnectionFailed(
                "No previous configuration to reconnect".to_string(),
            ))
        }
    }

    /// Returns whether the widget is connected
    #[must_use]
    pub fn is_connected(&self) -> bool {
        *self.state.borrow() == SpiceConnectionState::Connected
    }

    /// Returns whether using embedded mode
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
}

impl Default for EmbeddedSpiceWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::embedded_trait::EmbeddedWidget for EmbeddedSpiceWidget {
    fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    fn state(&self) -> crate::embedded_trait::EmbeddedConnectionState {
        match *self.state.borrow() {
            SpiceConnectionState::Disconnected => {
                crate::embedded_trait::EmbeddedConnectionState::Disconnected
            }
            SpiceConnectionState::Connecting => {
                crate::embedded_trait::EmbeddedConnectionState::Connecting
            }
            SpiceConnectionState::Connected => {
                crate::embedded_trait::EmbeddedConnectionState::Connected
            }
            SpiceConnectionState::Error => crate::embedded_trait::EmbeddedConnectionState::Error,
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
        #[cfg(feature = "spice-embedded")]
        {
            if let Some(ref sender) = *self.command_sender.borrow() {
                let _ = sender.send(SpiceClientCommand::SendCtrlAltDel);
            }
        }
    }

    fn protocol_name(&self) -> &'static str {
        "SPICE"
    }
}

impl Drop for EmbeddedSpiceWidget {
    fn drop(&mut self) {
        Self::disconnect(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spice_connection_state_display() {
        assert_eq!(
            SpiceConnectionState::Disconnected.to_string(),
            "Disconnected"
        );
        assert_eq!(SpiceConnectionState::Connecting.to_string(), "Connecting");
        assert_eq!(SpiceConnectionState::Connected.to_string(), "Connected");
        assert_eq!(SpiceConnectionState::Error.to_string(), "Error");
    }

    #[test]
    fn test_pixel_buffer_new() {
        let buffer = SpicePixelBuffer::new(100, 50);
        assert_eq!(buffer.width(), 100);
        assert_eq!(buffer.height(), 50);
        assert_eq!(buffer.data().len(), 100 * 50 * 4);
    }

    #[test]
    fn test_pixel_buffer_resize() {
        let mut buffer = SpicePixelBuffer::new(100, 50);
        buffer.resize(200, 100);
        assert_eq!(buffer.width(), 200);
        assert_eq!(buffer.height(), 100);
        assert_eq!(buffer.data().len(), 200 * 100 * 4);
    }
}
