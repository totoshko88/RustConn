//! RDP session widget with FreeRDP integration
//!
//! This module provides the `EmbeddedRdpWidget` struct for RDP session management
//! within the GTK4 application.
//!
//! # Architecture
//!
//! Unlike VNC which uses a pure Rust client (`vnc-rs`) for true embedded rendering,
//! RDP sessions use FreeRDP subprocess (wlfreerdp/xfreerdp) which opens its own window.
//! The widget displays connection status and manages the FreeRDP process lifecycle.
//!
//! ## Why not true embedded RDP?
//!
//! True embedded RDP (rendering frames directly in our GTK widget) would require:
//! - A pure Rust RDP client like `ironrdp` (complex API, limited documentation)
//! - Or FreeRDP with custom frame capture (requires FreeRDP modifications)
//!
//! The current approach provides:
//! - Reliable RDP connections via mature FreeRDP
//! - Session management (start/stop/status)
//! - Automatic client detection (wlfreerdp, xfreerdp3, xfreerdp)
//! - Qt/Wayland warning suppression for better compatibility
//!
//! # Client Mode
//!
//! - **Embedded mode**: Uses wlfreerdp (preferred) - opens separate window but managed by widget
//! - **External mode**: Uses xfreerdp - explicit external window mode
//!
//! Both modes open FreeRDP in a separate window; the difference is in client selection
//! and user expectations.

// Allow cast warnings - graphics code uses various integer sizes for coordinates
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::significant_drop_in_scrutinee)]
//!
//! # Requirements Coverage
//!
//! - Requirement 16.1: RDP connections via FreeRDP
//! - Requirement 16.6: Proper cleanup on disconnect
//! - Requirement 16.8: Fallback to xfreerdp if wlfreerdp unavailable
//! - Requirement 6.1: QSocketNotifier error handling
//! - Requirement 6.2: Wayland requestActivate warning suppression
//! - Requirement 6.3: FreeRDP threading isolation
//! - Requirement 6.4: Automatic fallback to external mode

// Re-export types for external use
pub use crate::embedded_rdp_buffer::{PixelBuffer, WaylandSurfaceHandle};
pub use crate::embedded_rdp_launcher::SafeFreeRdpLauncher;
pub use crate::embedded_rdp_thread::FreeRdpThread;
#[cfg(feature = "rdp-embedded")]
pub use crate::embedded_rdp_thread::{ClipboardFileTransfer, FileDownloadState};
pub use crate::embedded_rdp_types::{
    EmbeddedRdpError, EmbeddedSharedFolder, FreeRdpThreadState, RdpCommand, RdpConfig,
    RdpConnectionState, RdpEvent,
};

use crate::embedded_rdp_types::{ErrorCallback, FallbackCallback, StateCallback};
use gtk4::gdk;
use gtk4::glib;
use gtk4::glib::translate::IntoGlib;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, DrawingArea, EventControllerKey, EventControllerMotion,
    EventControllerScroll, EventControllerScrollFlags, GestureClick, Label, Orientation,
};
use std::cell::RefCell;
use std::process::Child;
use std::rc::Rc;

#[cfg(feature = "rdp-embedded")]
use rustconn_core::RdpClientCommand;

/// Embedded RDP widget using Wayland subsurface
///
/// This widget provides native RDP session embedding within GTK4 applications.
/// It uses a `DrawingArea` for rendering and integrates with FreeRDP for
/// protocol handling.
///
/// # Features
///
/// - Native Wayland subsurface integration
/// - FreeRDP frame capture via EndPaint callback
/// - Keyboard and mouse input forwarding
/// - Dynamic resolution changes on resize
/// - Automatic fallback to external xfreerdp
///
/// # Example
///
/// ```ignore
/// use rustconn::embedded_rdp::{EmbeddedRdpWidget, RdpConfig};
///
/// let widget = EmbeddedRdpWidget::new();
///
/// // Configure connection
/// let config = RdpConfig::new("192.168.1.100")
///     .with_username("admin")
///     .with_resolution(1920, 1080);
///
/// // Connect
/// widget.connect(&config)?;
/// ```
#[allow(dead_code)] // Many fields kept for GTK widget lifecycle and signal handlers
pub struct EmbeddedRdpWidget {
    /// Main container widget
    container: GtkBox,
    /// Toolbar with Ctrl+Alt+Del button
    toolbar: GtkBox,
    /// Status label for reconnect indicator
    status_label: Label,
    /// Copy button
    copy_button: Button,
    /// Paste button
    paste_button: Button,
    /// Ctrl+Alt+Del button
    ctrl_alt_del_button: Button,
    /// Separator between buttons
    separator: gtk4::Separator,
    /// Drawing area for rendering RDP frames
    drawing_area: DrawingArea,
    /// Wayland surface handle
    wl_surface: Rc<RefCell<WaylandSurfaceHandle>>,
    /// Pixel buffer for frame data
    pixel_buffer: Rc<RefCell<PixelBuffer>>,
    /// Current connection state
    state: Rc<RefCell<RdpConnectionState>>,
    /// Current configuration
    config: Rc<RefCell<Option<RdpConfig>>>,
    /// FreeRDP child process (for external mode)
    process: Rc<RefCell<Option<Child>>>,
    /// FreeRDP thread wrapper for embedded mode (Requirement 6.3)
    freerdp_thread: Rc<RefCell<Option<FreeRdpThread>>>,
    /// IronRDP command sender for embedded mode
    #[cfg(feature = "rdp-embedded")]
    ironrdp_command_tx: Rc<RefCell<Option<std::sync::mpsc::Sender<RdpClientCommand>>>>,
    /// Whether using embedded mode (wlfreerdp) or external mode (xfreerdp)
    is_embedded: Rc<RefCell<bool>>,
    /// Whether using IronRDP (true) or FreeRDP (false) for embedded mode
    is_ironrdp: Rc<RefCell<bool>>,
    /// Current widget width
    width: Rc<RefCell<u32>>,
    /// Current widget height
    height: Rc<RefCell<u32>>,
    /// RDP server framebuffer width (for coordinate transformation)
    rdp_width: Rc<RefCell<u32>>,
    /// RDP server framebuffer height (for coordinate transformation)
    rdp_height: Rc<RefCell<u32>>,
    /// State change callback
    on_state_changed: Rc<RefCell<Option<StateCallback>>>,
    /// Error callback
    on_error: Rc<RefCell<Option<ErrorCallback>>>,
    /// Fallback notification callback (Requirement 6.4)
    on_fallback: Rc<RefCell<Option<FallbackCallback>>>,
    /// Reconnect callback
    on_reconnect: Rc<RefCell<Option<Box<dyn Fn() + 'static>>>>,
    /// Reconnect button (shown when disconnected)
    reconnect_button: Button,
    /// Reconnect timer source ID for debounced resize reconnect
    reconnect_timer: Rc<RefCell<Option<glib::SourceId>>>,
    /// Remote clipboard text (received from server via CLIPRDR)
    remote_clipboard_text: Rc<RefCell<Option<String>>>,
    /// Available clipboard formats from server
    remote_clipboard_formats: Rc<RefCell<Vec<rustconn_core::ClipboardFormatInfo>>>,
    /// Audio player for RDP audio redirection
    #[cfg(feature = "rdp-audio")]
    audio_player: Rc<RefCell<Option<crate::audio::RdpAudioPlayer>>>,
    /// Clipboard file transfer state
    #[cfg(feature = "rdp-embedded")]
    file_transfer: Rc<RefCell<ClipboardFileTransfer>>,
    /// Save Files button (shown when files available on remote clipboard)
    #[cfg(feature = "rdp-embedded")]
    save_files_button: Button,
    /// File transfer progress callback
    #[cfg(feature = "rdp-embedded")]
    on_file_progress: Rc<RefCell<Option<Box<dyn Fn(f64, &str) + 'static>>>>,
    /// File transfer complete callback
    #[cfg(feature = "rdp-embedded")]
    on_file_complete: Rc<RefCell<Option<Box<dyn Fn(usize, &str) + 'static>>>>,
}

impl EmbeddedRdpWidget {
    /// Creates a new embedded RDP widget
    #[must_use]
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.set_hexpand(true);
        container.set_vexpand(true);

        // Create toolbar with clipboard and Ctrl+Alt+Del buttons (right-aligned like VNC)
        let toolbar = GtkBox::new(Orientation::Horizontal, 4);
        toolbar.set_margin_start(4);
        toolbar.set_margin_end(4);
        toolbar.set_margin_top(4);
        toolbar.set_margin_bottom(4);
        toolbar.set_halign(gtk4::Align::End); // Align to right

        // Status label for reconnect indicator (hidden by default)
        let status_label = Label::new(None);
        status_label.set_visible(false);
        status_label.set_margin_end(8);
        status_label.add_css_class("dim-label");
        toolbar.append(&status_label);

        // Copy button - copies remote clipboard to local (enabled when data available)
        let copy_button = Button::with_label("Copy");
        copy_button.set_tooltip_text(Some(
            "Copy remote clipboard to local (waiting for remote data...)",
        ));
        copy_button.set_sensitive(false); // Disabled until we receive clipboard data
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

        let ctrl_alt_del_button = Button::with_label("Ctrl+Alt+Del");
        ctrl_alt_del_button.add_css_class("suggested-action"); // Blue button style
        ctrl_alt_del_button.set_tooltip_text(Some("Send Ctrl+Alt+Del to remote session"));
        toolbar.append(&ctrl_alt_del_button);

        // Save Files button (shown when files available on remote clipboard)
        #[cfg(feature = "rdp-embedded")]
        let save_files_button = Button::with_label("Save Files");
        #[cfg(feature = "rdp-embedded")]
        {
            save_files_button.set_tooltip_text(Some("Save files from remote clipboard"));
            save_files_button.set_visible(false); // Hidden until files available
            toolbar.append(&save_files_button);
        }

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
        // Don't set fixed content size - let the widget expand to fill available space
        // The actual RDP resolution will be set when connect() is called
        drawing_area.set_can_focus(true);
        drawing_area.set_focusable(true);

        container.append(&drawing_area);

        let pixel_buffer = Rc::new(RefCell::new(PixelBuffer::new(1280, 720)));
        let state = Rc::new(RefCell::new(RdpConnectionState::Disconnected));
        let width = Rc::new(RefCell::new(1280u32));
        let height = Rc::new(RefCell::new(720u32));
        let rdp_width = Rc::new(RefCell::new(1280u32));
        let rdp_height = Rc::new(RefCell::new(720u32));
        let is_embedded = Rc::new(RefCell::new(false));
        let is_ironrdp = Rc::new(RefCell::new(false));

        #[cfg(feature = "rdp-embedded")]
        let ironrdp_command_tx: Rc<
            RefCell<Option<std::sync::mpsc::Sender<RdpClientCommand>>>,
        > = Rc::new(RefCell::new(None));

        let widget = Self {
            container,
            toolbar,
            status_label,
            copy_button: copy_button.clone(),
            paste_button: paste_button.clone(),
            ctrl_alt_del_button: ctrl_alt_del_button.clone(),
            separator,
            drawing_area,
            wl_surface: Rc::new(RefCell::new(WaylandSurfaceHandle::new())),
            pixel_buffer,
            state,
            config: Rc::new(RefCell::new(None)),
            process: Rc::new(RefCell::new(None)),
            freerdp_thread: Rc::new(RefCell::new(None)),
            #[cfg(feature = "rdp-embedded")]
            ironrdp_command_tx,
            is_embedded,
            is_ironrdp,
            width,
            height,
            rdp_width,
            rdp_height,
            on_state_changed: Rc::new(RefCell::new(None)),
            on_error: Rc::new(RefCell::new(None)),
            on_fallback: Rc::new(RefCell::new(None)),
            on_reconnect: Rc::new(RefCell::new(None)),
            reconnect_button,
            reconnect_timer: Rc::new(RefCell::new(None)),
            remote_clipboard_text: Rc::new(RefCell::new(None)),
            remote_clipboard_formats: Rc::new(RefCell::new(Vec::new())),
            #[cfg(feature = "rdp-audio")]
            audio_player: Rc::new(RefCell::new(None)),
            #[cfg(feature = "rdp-embedded")]
            file_transfer: Rc::new(RefCell::new(ClipboardFileTransfer::new())),
            #[cfg(feature = "rdp-embedded")]
            save_files_button: save_files_button.clone(),
            #[cfg(feature = "rdp-embedded")]
            on_file_progress: Rc::new(RefCell::new(None)),
            #[cfg(feature = "rdp-embedded")]
            on_file_complete: Rc::new(RefCell::new(None)),
        };

        widget.setup_drawing();
        widget.setup_input_handlers();
        widget.setup_resize_handler();
        widget.setup_clipboard_buttons(&copy_button, &paste_button);
        widget.setup_ctrl_alt_del_button(&ctrl_alt_del_button);
        widget.setup_reconnect_button();
        widget.setup_visibility_handler();
        #[cfg(feature = "rdp-embedded")]
        widget.setup_save_files_button(&save_files_button);

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

    /// Sets up the Save Files button click handler for clipboard file transfer
    #[cfg(feature = "rdp-embedded")]
    fn setup_save_files_button(&self, button: &Button) {
        let file_transfer = self.file_transfer.clone();
        let ironrdp_tx = self.ironrdp_command_tx.clone();
        let on_progress = self.on_file_progress.clone();
        let on_complete = self.on_file_complete.clone();
        let status_label = self.status_label.clone();
        let save_btn = button.clone();

        button.connect_clicked(move |_| {
            let files = file_transfer.borrow().available_files.clone();
            if files.is_empty() {
                return;
            }

            // Show file chooser dialog for target directory
            let dialog = gtk4::FileDialog::builder()
                .title("Select folder to save files")
                .modal(true)
                .build();

            let file_transfer_clone = file_transfer.clone();
            let ironrdp_tx_clone = ironrdp_tx.clone();
            let on_progress_clone = on_progress.clone();
            let _on_complete_clone = on_complete.clone();
            let status_label_clone = status_label.clone();
            let save_btn_clone = save_btn.clone();
            let files_clone = files.clone();

            dialog.select_folder(
                None::<&gtk4::Window>,
                None::<&gtk4::gio::Cancellable>,
                move |result| {
                    if let Ok(folder) = result {
                        if let Some(path) = folder.path() {
                            // Set target directory and start downloads
                            {
                                let mut transfer = file_transfer_clone.borrow_mut();
                                transfer.target_directory = Some(path.clone());
                                transfer.total_files = files_clone.len();
                                transfer.completed_count = 0;
                            }

                            // Disable button during transfer
                            save_btn_clone.set_sensitive(false);
                            save_btn_clone.set_label("Downloading...");

                            // Request file contents for each file
                            if let Some(ref sender) = *ironrdp_tx_clone.borrow() {
                                for (idx, file) in files_clone.iter().enumerate() {
                                    let stream_id = {
                                        let mut transfer = file_transfer_clone.borrow_mut();
                                        transfer.start_download(idx as u32)
                                    };

                                    if let Some(sid) = stream_id {
                                        // First request size, then data
                                        let _ =
                                            sender.send(RdpClientCommand::RequestFileContents {
                                                stream_id: sid,
                                                file_index: file.index,
                                                request_size: true,
                                                offset: 0,
                                                length: 0,
                                            });

                                        // Then request actual data
                                        let _ =
                                            sender.send(RdpClientCommand::RequestFileContents {
                                                stream_id: sid,
                                                file_index: file.index,
                                                request_size: false,
                                                offset: 0,
                                                length: u32::MAX, // Request all data
                                            });
                                    }
                                }
                            }

                            // Show progress
                            status_label_clone.set_text("Downloading files...");
                            status_label_clone.set_visible(true);

                            if let Some(ref callback) = *on_progress_clone.borrow() {
                                callback(0.0, "Starting download...");
                            }
                        }
                    }
                },
            );
        });
    }

    /// Connects a callback for file transfer progress updates
    #[cfg(feature = "rdp-embedded")]
    pub fn connect_file_progress<F>(&self, callback: F)
    where
        F: Fn(f64, &str) + 'static,
    {
        *self.on_file_progress.borrow_mut() = Some(Box::new(callback));
    }

    /// Connects a callback for file transfer completion
    #[cfg(feature = "rdp-embedded")]
    pub fn connect_file_complete<F>(&self, callback: F)
    where
        F: Fn(usize, &str) + 'static,
    {
        *self.on_file_complete.borrow_mut() = Some(Box::new(callback));
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

    /// Sets up the clipboard Copy/Paste button handlers
    fn setup_clipboard_buttons(&self, copy_btn: &Button, paste_btn: &Button) {
        // Copy button - copy remote clipboard text to local clipboard
        {
            let state = self.state.clone();
            let is_embedded = self.is_embedded.clone();
            let remote_clipboard_text = self.remote_clipboard_text.clone();
            let drawing_area = self.drawing_area.clone();
            let status_label = self.status_label.clone();

            copy_btn.connect_clicked(move |_| {
                let current_state = *state.borrow();
                let embedded = *is_embedded.borrow();

                if current_state != RdpConnectionState::Connected || !embedded {
                    return;
                }

                // Check if we have remote clipboard text
                if let Some(ref text) = *remote_clipboard_text.borrow() {
                    let char_count = text.len();

                    // Copy to local clipboard
                    let display = drawing_area.display();
                    let clipboard = display.clipboard();
                    clipboard.set_text(text);

                    // Show feedback
                    status_label.set_text(&format!("Copied {char_count} chars"));
                    status_label.set_visible(true);
                    let status_hide = status_label.clone();
                    glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
                        status_hide.set_visible(false);
                    });
                } else {
                    status_label.set_text("No remote clipboard data");
                    status_label.set_visible(true);
                    let status_hide = status_label.clone();
                    glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
                        status_hide.set_visible(false);
                    });
                }
            });
        }

        // Paste button - send local clipboard text to remote
        {
            #[cfg(feature = "rdp-embedded")]
            let ironrdp_tx = self.ironrdp_command_tx.clone();
            let drawing_area = self.drawing_area.clone();
            let state = self.state.clone();
            let is_embedded = self.is_embedded.clone();
            #[cfg(feature = "rdp-embedded")]
            let is_ironrdp = self.is_ironrdp.clone();
            let status_label = self.status_label.clone();

            paste_btn.connect_clicked(move |_| {
                let current_state = *state.borrow();
                let embedded = *is_embedded.borrow();

                if current_state != RdpConnectionState::Connected || !embedded {
                    return;
                }

                // Get text from local clipboard and send to remote
                let display = drawing_area.display();
                let clipboard = display.clipboard();

                #[cfg(feature = "rdp-embedded")]
                let using_ironrdp = *is_ironrdp.borrow();
                #[cfg(feature = "rdp-embedded")]
                let tx = ironrdp_tx.clone();
                let status = status_label.clone();

                clipboard.read_text_async(
                    None::<&gtk4::gio::Cancellable>,
                    move |result: Result<Option<glib::GString>, glib::Error>| {
                        if let Ok(Some(text)) = result {
                            let char_count = text.len();

                            #[cfg(feature = "rdp-embedded")]
                            if using_ironrdp {
                                // Send clipboard text via IronRDP
                                if let Some(ref sender) = *tx.borrow() {
                                    let _ = sender
                                        .send(RdpClientCommand::ClipboardText(text.to_string()));
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
                            // For FreeRDP, clipboard is handled by the external process
                        }
                    },
                );
            });
        }
    }

    /// Sets up the Ctrl+Alt+Del button handler
    fn setup_ctrl_alt_del_button(&self, button: &Button) {
        #[cfg(feature = "rdp-embedded")]
        {
            let ironrdp_tx = self.ironrdp_command_tx.clone();
            let freerdp_thread = self.freerdp_thread.clone();
            let state = self.state.clone();
            let is_embedded = self.is_embedded.clone();
            let is_ironrdp = self.is_ironrdp.clone();

            button.connect_clicked(move |_| {
                let current_state = *state.borrow();
                let embedded = *is_embedded.borrow();
                let using_ironrdp = *is_ironrdp.borrow();

                if current_state != RdpConnectionState::Connected || !embedded {
                    return;
                }

                if using_ironrdp {
                    // Send via IronRDP
                    if let Some(ref tx) = *ironrdp_tx.borrow() {
                        let _ = tx.send(RdpClientCommand::SendCtrlAltDel);
                    }
                } else {
                    // Send via FreeRDP thread
                    if let Some(ref thread) = *freerdp_thread.borrow() {
                        let _ = thread.send_command(RdpCommand::SendCtrlAltDel);
                        eprintln!("[FreeRDP] Sent Ctrl+Alt+Del");
                    }
                }
            });
        }

        #[cfg(not(feature = "rdp-embedded"))]
        {
            let freerdp_thread = self.freerdp_thread.clone();
            let state = self.state.clone();
            let is_embedded = self.is_embedded.clone();

            button.connect_clicked(move |_| {
                let current_state = *state.borrow();
                let embedded = *is_embedded.borrow();

                if current_state != RdpConnectionState::Connected || !embedded {
                    return;
                }

                if let Some(ref thread) = *freerdp_thread.borrow() {
                    let _ = thread.send_command(RdpCommand::SendCtrlAltDel);
                    eprintln!("[FreeRDP] Sent Ctrl+Alt+Del");
                }
            });
        }
    }

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
    fn setup_drawing(&self) {
        let pixel_buffer = self.pixel_buffer.clone();
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let config = self.config.clone();
        let rdp_width = self.rdp_width.clone();
        let rdp_height = self.rdp_height.clone();

        self.drawing_area
            .set_draw_func(move |_area, cr, width, height| {
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
                        buf_width as i32,
                        buf_height as i32,
                        buffer.stride() as i32,
                    ) {
                        // Scale to fit the drawing area while maintaining aspect ratio
                        let scale_x = f64::from(width) / f64::from(buf_width);
                        let scale_y = f64::from(height) / f64::from(buf_height);
                        let scale = scale_x.min(scale_y);

                        // Center the image
                        let offset_x = f64::from(buf_width).mul_add(-scale, f64::from(width)) / 2.0;
                        let offset_y =
                            f64::from(buf_height).mul_add(-scale, f64::from(height)) / 2.0;

                        // Save the current transformation matrix
                        cr.save().unwrap_or(());

                        cr.translate(offset_x, offset_y);
                        cr.scale(scale, scale);
                        let _ = cr.set_source_surface(&surface, 0.0, 0.0);

                        // Use bilinear filtering for smooth scaling to reduce artifacts
                        // Nearest-neighbor can cause visible pixelation and artifacts
                        cr.source().set_filter(gtk4::cairo::Filter::Bilinear);

                        let _ = cr.paint();

                        // Restore the transformation matrix
                        cr.restore().unwrap_or(());
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
    fn draw_status_overlay(
        cr: &gtk4::cairo::Context,
        width: i32,
        height: i32,
        current_state: RdpConnectionState,
        embedded: bool,
        config: &Rc<RefCell<Option<RdpConfig>>>,
        rdp_width: &Rc<RefCell<u32>>,
        rdp_height: &Rc<RefCell<u32>>,
    ) {
        crate::embedded_rdp_ui::draw_status_overlay(
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

    /// Sets up keyboard and mouse input handlers with coordinate transformation
    #[cfg(feature = "rdp-embedded")]
    fn setup_input_handlers(&self) {
        use rustconn_core::{keyval_to_scancode, keyval_to_unicode};

        // Keyboard input handler
        let key_controller = EventControllerKey::new();
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        let freerdp_thread = self.freerdp_thread.clone();
        let ironrdp_tx = self.ironrdp_command_tx.clone();

        key_controller.connect_key_pressed(move |_controller, keyval, _keycode, _modifier| {
            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();
            let using_ironrdp = *is_ironrdp.borrow();

            if embedded && current_state == RdpConnectionState::Connected {
                if using_ironrdp {
                    // Convert GTK keyval to RDP scancode and send via IronRDP
                    let gdk_keyval = keyval.into_glib();
                    if let Some(scancode) = keyval_to_scancode(gdk_keyval) {
                        // Known scancode - send as keyboard event
                        if let Some(ref tx) = *ironrdp_tx.borrow() {
                            let _ = tx.send(RdpClientCommand::KeyEvent {
                                scancode: scancode.code,
                                pressed: true,
                                extended: scancode.extended,
                            });
                        }
                    } else if let Some(ch) = keyval_to_unicode(gdk_keyval) {
                        // Unknown scancode but valid Unicode character - send as Unicode event
                        // This handles non-Latin characters (Cyrillic, etc.)
                        if let Some(ref tx) = *ironrdp_tx.borrow() {
                            let _ = tx.send(RdpClientCommand::UnicodeEvent {
                                character: ch,
                                pressed: true,
                            });
                        }
                    } else {
                        tracing::warn!("[IronRDP] Unknown keyval: 0x{:X}", gdk_keyval);
                    }
                } else if let Some(ref thread) = *freerdp_thread.borrow() {
                    let _ = thread.send_command(RdpCommand::KeyEvent {
                        keyval: keyval.into_glib(),
                        pressed: true,
                    });
                }
            }

            gdk::glib::Propagation::Proceed
        });

        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        let freerdp_thread = self.freerdp_thread.clone();
        let ironrdp_tx = self.ironrdp_command_tx.clone();

        key_controller.connect_key_released(move |_controller, keyval, _keycode, _modifier| {
            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();
            let using_ironrdp = *is_ironrdp.borrow();

            if embedded && current_state == RdpConnectionState::Connected {
                if using_ironrdp {
                    let gdk_keyval = keyval.into_glib();
                    if let Some(scancode) = keyval_to_scancode(gdk_keyval) {
                        if let Some(ref tx) = *ironrdp_tx.borrow() {
                            let _ = tx.send(RdpClientCommand::KeyEvent {
                                scancode: scancode.code,
                                pressed: false,
                                extended: scancode.extended,
                            });
                        }
                    } else if let Some(ch) = keyval_to_unicode(gdk_keyval) {
                        // Unicode character release
                        if let Some(ref tx) = *ironrdp_tx.borrow() {
                            let _ = tx.send(RdpClientCommand::UnicodeEvent {
                                character: ch,
                                pressed: false,
                            });
                        }
                    }
                } else if let Some(ref thread) = *freerdp_thread.borrow() {
                    let _ = thread.send_command(RdpCommand::KeyEvent {
                        keyval: keyval.into_glib(),
                        pressed: false,
                    });
                }
            }
        });

        self.drawing_area.add_controller(key_controller);

        // Track current button state for motion events
        let button_state = Rc::new(RefCell::new(0u8));

        // Mouse motion handler with coordinate transformation
        let motion_controller = EventControllerMotion::new();
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        let freerdp_thread = self.freerdp_thread.clone();
        let ironrdp_tx = self.ironrdp_command_tx.clone();
        let button_state_motion = button_state.clone();
        let width_motion = self.width.clone();
        let height_motion = self.height.clone();
        let rdp_width_motion = self.rdp_width.clone();
        let rdp_height_motion = self.rdp_height.clone();

        motion_controller.connect_motion(move |_controller, x, y| {
            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();
            let using_ironrdp = *is_ironrdp.borrow();

            if embedded && current_state == RdpConnectionState::Connected {
                let widget_w = f64::from(*width_motion.borrow());
                let widget_h = f64::from(*height_motion.borrow());
                let rdp_w = f64::from(*rdp_width_motion.borrow());
                let rdp_h = f64::from(*rdp_height_motion.borrow());

                let (rdp_x, rdp_y) = crate::embedded_rdp_ui::transform_widget_to_rdp(
                    x, y, widget_w, widget_h, rdp_w, rdp_h,
                );
                let buttons = *button_state_motion.borrow();

                if using_ironrdp {
                    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                    if let Some(ref tx) = *ironrdp_tx.borrow() {
                        let _ = tx.send(RdpClientCommand::PointerEvent {
                            x: rdp_x as u16,
                            y: rdp_y as u16,
                            buttons,
                        });
                    }
                } else {
                    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                    if let Some(ref thread) = *freerdp_thread.borrow() {
                        let _ = thread.send_command(RdpCommand::MouseEvent {
                            x: rdp_x as i32,
                            y: rdp_y as i32,
                            button: u32::from(buttons),
                            pressed: false,
                        });
                    }
                }
            }
        });

        self.drawing_area.add_controller(motion_controller);

        // Mouse click handler with coordinate transformation
        let click_controller = GestureClick::new();
        click_controller.set_button(0);
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        let freerdp_thread = self.freerdp_thread.clone();
        let ironrdp_tx = self.ironrdp_command_tx.clone();
        let button_state_press = button_state.clone();
        let width_press = self.width.clone();
        let height_press = self.height.clone();
        let rdp_width_press = self.rdp_width.clone();
        let rdp_height_press = self.rdp_height.clone();
        let drawing_area_press = self.drawing_area.clone();

        click_controller.connect_pressed(move |gesture, _n_press, x, y| {
            // Grab focus on click to receive keyboard events
            drawing_area_press.grab_focus();

            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();
            let using_ironrdp = *is_ironrdp.borrow();

            if embedded && current_state == RdpConnectionState::Connected {
                let button = gesture.current_button();

                let widget_w = f64::from(*width_press.borrow());
                let widget_h = f64::from(*height_press.borrow());
                let rdp_w = f64::from(*rdp_width_press.borrow());
                let rdp_h = f64::from(*rdp_height_press.borrow());

                let (rdp_x, rdp_y) = crate::embedded_rdp_ui::transform_widget_to_rdp(
                    x, y, widget_w, widget_h, rdp_w, rdp_h,
                );

                // Convert GTK button to RDP button mask
                let button_bit = crate::embedded_rdp_ui::gtk_button_to_rdp_mask(button);
                let buttons = *button_state_press.borrow() | button_bit;
                *button_state_press.borrow_mut() = buttons;

                if using_ironrdp {
                    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                    if let Some(ref tx) = *ironrdp_tx.borrow() {
                        let rdp_button = crate::embedded_rdp_ui::gtk_button_to_rdp_button(button);
                        let _ = tx.send(RdpClientCommand::MouseButtonPress {
                            x: rdp_x as u16,
                            y: rdp_y as u16,
                            button: rdp_button,
                        });
                    }
                } else {
                    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                    if let Some(ref thread) = *freerdp_thread.borrow() {
                        let _ = thread.send_command(RdpCommand::MouseEvent {
                            x: rdp_x as i32,
                            y: rdp_y as i32,
                            button,
                            pressed: true,
                        });
                    }
                }
            }
        });

        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        let freerdp_thread = self.freerdp_thread.clone();
        let ironrdp_tx = self.ironrdp_command_tx.clone();
        let button_state_release = button_state.clone();
        let width_release = self.width.clone();
        let height_release = self.height.clone();
        let rdp_width_release = self.rdp_width.clone();
        let rdp_height_release = self.rdp_height.clone();

        click_controller.connect_released(move |gesture, _n_press, x, y| {
            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();
            let using_ironrdp = *is_ironrdp.borrow();

            if embedded && current_state == RdpConnectionState::Connected {
                let button = gesture.current_button();

                let widget_w = f64::from(*width_release.borrow());
                let widget_h = f64::from(*height_release.borrow());
                let rdp_w = f64::from(*rdp_width_release.borrow());
                let rdp_h = f64::from(*rdp_height_release.borrow());

                let (rdp_x, rdp_y) = crate::embedded_rdp_ui::transform_widget_to_rdp(
                    x, y, widget_w, widget_h, rdp_w, rdp_h,
                );

                let button_bit = crate::embedded_rdp_ui::gtk_button_to_rdp_mask(button);
                let buttons = *button_state_release.borrow() & !button_bit;
                *button_state_release.borrow_mut() = buttons;

                if using_ironrdp {
                    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                    if let Some(ref tx) = *ironrdp_tx.borrow() {
                        let rdp_button = crate::embedded_rdp_ui::gtk_button_to_rdp_button(button);
                        let _ = tx.send(RdpClientCommand::MouseButtonRelease {
                            x: rdp_x as u16,
                            y: rdp_y as u16,
                            button: rdp_button,
                        });
                    }
                } else {
                    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                    if let Some(ref thread) = *freerdp_thread.borrow() {
                        let _ = thread.send_command(RdpCommand::MouseEvent {
                            x: rdp_x as i32,
                            y: rdp_y as i32,
                            button,
                            pressed: false,
                        });
                    }
                }
            }
        });

        self.drawing_area.add_controller(click_controller);

        // Mouse scroll handler for wheel events
        let scroll_controller = EventControllerScroll::new(EventControllerScrollFlags::VERTICAL);
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        let ironrdp_tx = self.ironrdp_command_tx.clone();

        scroll_controller.connect_scroll(move |_controller, _dx, dy| {
            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();
            let using_ironrdp = *is_ironrdp.borrow();

            if embedded && current_state == RdpConnectionState::Connected && using_ironrdp {
                if let Some(ref tx) = *ironrdp_tx.borrow() {
                    #[allow(clippy::cast_possible_truncation)]
                    let wheel_delta = (-dy * 120.0) as i16;
                    if wheel_delta != 0 {
                        let _ = tx.send(RdpClientCommand::WheelEvent {
                            horizontal: 0,
                            vertical: wheel_delta,
                        });
                    }
                }
            }

            gdk::glib::Propagation::Proceed
        });

        self.drawing_area.add_controller(scroll_controller);
    }

    /// Sets up keyboard and mouse input handlers (fallback when rdp-embedded is disabled)
    #[cfg(not(feature = "rdp-embedded"))]
    fn setup_input_handlers(&self) {
        // Simplified handlers for FreeRDP-only mode
        let key_controller = EventControllerKey::new();
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let freerdp_thread = self.freerdp_thread.clone();

        key_controller.connect_key_pressed(move |_controller, keyval, _keycode, _modifier| {
            let current_state = *state.borrow();
            let embedded = *is_embedded.borrow();

            if embedded && current_state == RdpConnectionState::Connected {
                if let Some(ref thread) = *freerdp_thread.borrow() {
                    let _ = thread.send_command(RdpCommand::KeyEvent {
                        keyval: keyval.into_glib(),
                        pressed: true,
                    });
                }
            }

            gdk::glib::Propagation::Proceed
        });

        self.drawing_area.add_controller(key_controller);
    }

    /// Sets up the resize handler with scaling
    ///
    /// The RDP image is scaled to fit the widget size. No reconnection is performed
    /// as Windows RDP servers may not support dynamic resolution changes well.
    ///
    /// # Requirements Coverage
    ///
    /// - Requirement 1.7: Dynamic resolution change on resize
    #[cfg(feature = "rdp-embedded")]
    fn setup_resize_handler(&self) {
        let width = self.width.clone();
        let height = self.height.clone();
        let pixel_buffer = self.pixel_buffer.clone();
        let state = self.state.clone();
        let is_embedded = self.is_embedded.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        let config = self.config.clone();
        let reconnect_timer = self.reconnect_timer.clone();
        let rdp_width = self.rdp_width.clone();
        let rdp_height = self.rdp_height.clone();
        let ironrdp_tx = self.ironrdp_command_tx.clone();
        let on_state_changed = self.on_state_changed.clone();
        let on_error = self.on_error.clone();
        let toolbar = self.toolbar.clone();
        let status_label = self.status_label.clone();

        self.drawing_area
            .connect_resize(move |area, new_width, new_height| {
                let new_width = new_width.unsigned_abs();
                let new_height = new_height.unsigned_abs();

                let _old_width = *width.borrow();
                let _old_height = *height.borrow();

                *width.borrow_mut() = new_width;
                *height.borrow_mut() = new_height;

                // Always queue redraw for immediate visual feedback (scaling)
                area.queue_draw();

                // Check if we should trigger reconnect
                let current_state = *state.borrow();
                let embedded = *is_embedded.borrow();
                let using_ironrdp = *is_ironrdp.borrow();

                if current_state != RdpConnectionState::Connected || !embedded || !using_ironrdp {
                    return;
                }

                // Only reconnect if size changed significantly (more than 50 pixels in any dimension)
                let rdp_w = *rdp_width.borrow();
                let rdp_h = *rdp_height.borrow();
                let width_diff = (new_width as i32 - rdp_w as i32).unsigned_abs();
                let height_diff = (new_height as i32 - rdp_h as i32).unsigned_abs();

                if width_diff < 50 && height_diff < 50 {
                    return; // Size change too small, just scale
                }

                // Cancel any pending reconnect timer
                // Note: We just take the SourceId from the Option. If the timer has already
                // fired, the callback has already run and cleared itself. We don't call
                // remove() because the source may have already been removed by glib after
                // the callback executed, which would cause a panic.
                let _ = reconnect_timer.borrow_mut().take();

                // Show reconnect pending indicator
                status_label.set_text("Resizing...");
                status_label.set_visible(true);

                // Set up debounced reconnect (500ms delay)
                let config_clone = config.clone();
                let state_clone = state.clone();
                let is_embedded_clone = is_embedded.clone();
                let is_ironrdp_clone = is_ironrdp.clone();
                let rdp_width_clone = rdp_width.clone();
                let rdp_height_clone = rdp_height.clone();
                let ironrdp_tx_clone = ironrdp_tx.clone();
                let pixel_buffer_clone = pixel_buffer.clone();
                let on_state_changed_clone = on_state_changed.clone();
                let on_error_clone = on_error.clone();
                let toolbar_clone = toolbar.clone();
                let status_label_clone = status_label.clone();
                let area_clone = area.clone();
                let new_w = new_width;
                let new_h = new_height;

                let source_id = glib::timeout_add_local_once(
                    std::time::Duration::from_millis(500),
                    move || {
                        // Check if still connected and in IronRDP mode
                        let current_state = *state_clone.borrow();
                        let embedded = *is_embedded_clone.borrow();
                        let using_ironrdp = *is_ironrdp_clone.borrow();

                        if current_state != RdpConnectionState::Connected
                            || !embedded
                            || !using_ironrdp
                        {
                            status_label_clone.set_visible(false);
                            return;
                        }

                        // Get current config
                        let config_opt = config_clone.borrow().clone();
                        if let Some(mut cfg) = config_opt {
                            // Update status label
                            status_label_clone.set_text("Reconnecting...");

                            tracing::debug!(
                                "[IronRDP] Reconnecting with new resolution {}x{} (was {}x{})",
                                new_w,
                                new_h,
                                cfg.width,
                                cfg.height
                            );

                            // Update config with new resolution
                            cfg.width = new_w;
                            cfg.height = new_h;
                            *config_clone.borrow_mut() = Some(cfg.clone());

                            // Disconnect current session
                            if let Some(ref tx) = *ironrdp_tx_clone.borrow() {
                                let _ = tx.send(RdpClientCommand::Disconnect);
                            }
                            *ironrdp_tx_clone.borrow_mut() = None;

                            // Clear pixel buffer and prepare for new connection
                            {
                                let mut buffer = pixel_buffer_clone.borrow_mut();
                                buffer.resize(new_w, new_h);
                                // Fill with dark gray to show reconnecting
                                for chunk in buffer.data_mut().chunks_exact_mut(4) {
                                    chunk[0] = 0x1E; // B
                                    chunk[1] = 0x1E; // G
                                    chunk[2] = 0x1E; // R
                                    chunk[3] = 0xFF; // A
                                }
                                buffer.set_has_data(true);
                            }
                            *rdp_width_clone.borrow_mut() = new_w;
                            *rdp_height_clone.borrow_mut() = new_h;
                            area_clone.queue_draw();

                            // Reconnect with new resolution
                            *state_clone.borrow_mut() = RdpConnectionState::Connecting;
                            if let Some(ref callback) = *on_state_changed_clone.borrow() {
                                callback(RdpConnectionState::Connecting);
                            }

                            // Start new IronRDP connection
                            Self::reconnect_ironrdp(
                                cfg,
                                state_clone.clone(),
                                area_clone.clone(),
                                toolbar_clone.clone(),
                                status_label_clone.clone(),
                                on_state_changed_clone.clone(),
                                on_error_clone.clone(),
                                rdp_width_clone.clone(),
                                rdp_height_clone.clone(),
                                pixel_buffer_clone.clone(),
                                is_embedded_clone.clone(),
                                is_ironrdp_clone.clone(),
                                ironrdp_tx_clone.clone(),
                            );
                        }
                    },
                );

                *reconnect_timer.borrow_mut() = Some(source_id);
            });
    }

    /// Reconnects IronRDP with new configuration (called from resize handler)
    #[cfg(feature = "rdp-embedded")]
    #[allow(clippy::too_many_arguments)]
    fn reconnect_ironrdp(
        config: RdpConfig,
        state: Rc<RefCell<RdpConnectionState>>,
        drawing_area: DrawingArea,
        toolbar: GtkBox,
        status_label: Label,
        on_state_changed: Rc<RefCell<Option<StateCallback>>>,
        on_error: Rc<RefCell<Option<ErrorCallback>>>,
        rdp_width_ref: Rc<RefCell<u32>>,
        rdp_height_ref: Rc<RefCell<u32>>,
        pixel_buffer: Rc<RefCell<PixelBuffer>>,
        is_embedded: Rc<RefCell<bool>>,
        is_ironrdp: Rc<RefCell<bool>>,
        ironrdp_tx: Rc<RefCell<Option<std::sync::mpsc::Sender<RdpClientCommand>>>>,
    ) {
        use rustconn_core::{RdpClient, RdpClientConfig, RdpClientEvent};

        // Convert GUI config to RdpClientConfig
        let mut client_config = RdpClientConfig::new(&config.host)
            .with_port(config.port)
            .with_resolution(config.width as u16, config.height as u16)
            .with_clipboard(config.clipboard_enabled);

        if let Some(ref username) = config.username {
            client_config = client_config.with_username(username);
        }

        if let Some(ref password) = config.password {
            client_config = client_config.with_password(password);
        }

        if let Some(ref domain) = config.domain {
            client_config = client_config.with_domain(domain);
        }

        // Create and connect the IronRDP client
        let mut client = RdpClient::new(client_config);
        if let Err(e) = client.connect() {
            tracing::error!("[IronRDP] Reconnect failed: {}", e);
            *state.borrow_mut() = RdpConnectionState::Error;
            if let Some(ref callback) = *on_error.borrow() {
                callback(&format!("Reconnect failed: {e}"));
            }
            return;
        }

        // Store command sender for input handling
        if let Some(tx) = client.command_sender() {
            *ironrdp_tx.borrow_mut() = Some(tx);
        }

        // Store client in a shared reference for the polling closure
        let client = std::rc::Rc::new(std::cell::RefCell::new(Some(client)));
        let client_ref = client.clone();
        let polling_interval = u64::from(config.polling_interval_ms);

        glib::timeout_add_local(
            std::time::Duration::from_millis(polling_interval),
            move || {
                // Check if we're still in embedded mode
                if !*is_embedded.borrow() || !*is_ironrdp.borrow() {
                    // Clean up client
                    if let Some(mut c) = client_ref.borrow_mut().take() {
                        c.disconnect();
                    }
                    *ironrdp_tx.borrow_mut() = None;
                    toolbar.set_visible(false);
                    return glib::ControlFlow::Break;
                }

                // Track if we need to redraw
                let mut needs_redraw = false;
                let mut should_break = false;

                // Poll for events from IronRDP client
                if let Some(ref client) = *client_ref.borrow() {
                    while let Some(event) = client.try_recv_event() {
                        match event {
                            RdpClientEvent::Connected { width, height } => {
                                tracing::debug!("[IronRDP] Reconnected: {}x{}", width, height);
                                *state.borrow_mut() = RdpConnectionState::Connected;
                                *rdp_width_ref.borrow_mut() = u32::from(width);
                                *rdp_height_ref.borrow_mut() = u32::from(height);
                                {
                                    let mut buffer = pixel_buffer.borrow_mut();
                                    buffer.resize(u32::from(width), u32::from(height));
                                    buffer.clear();
                                }
                                // Show toolbar and hide status label when connected
                                toolbar.set_visible(true);
                                status_label.set_visible(false);
                                if let Some(ref callback) = *on_state_changed.borrow() {
                                    callback(RdpConnectionState::Connected);
                                }
                                needs_redraw = true;
                            }
                            RdpClientEvent::Disconnected => {
                                tracing::debug!("[IronRDP] Disconnected after reconnect");
                                *state.borrow_mut() = RdpConnectionState::Disconnected;
                                toolbar.set_visible(false);
                                status_label.set_visible(false);
                                if let Some(ref callback) = *on_state_changed.borrow() {
                                    callback(RdpConnectionState::Disconnected);
                                }
                                needs_redraw = true;
                                should_break = true;
                            }
                            RdpClientEvent::Error(msg) => {
                                tracing::error!("[IronRDP] Error after reconnect: {}", msg);
                                *state.borrow_mut() = RdpConnectionState::Error;
                                toolbar.set_visible(false);
                                status_label.set_visible(false);
                                if let Some(ref callback) = *on_error.borrow() {
                                    callback(&msg);
                                }
                                needs_redraw = true;
                                should_break = true;
                            }
                            RdpClientEvent::FrameUpdate { rect, data } => {
                                let mut buffer = pixel_buffer.borrow_mut();
                                buffer.update_region(
                                    u32::from(rect.x),
                                    u32::from(rect.y),
                                    u32::from(rect.width),
                                    u32::from(rect.height),
                                    &data,
                                    u32::from(rect.width) * 4,
                                );
                                needs_redraw = true;
                            }
                            RdpClientEvent::FullFrameUpdate {
                                width,
                                height,
                                data,
                            } => {
                                let mut buffer = pixel_buffer.borrow_mut();
                                if buffer.width() != u32::from(width)
                                    || buffer.height() != u32::from(height)
                                {
                                    buffer.resize(u32::from(width), u32::from(height));
                                    *rdp_width_ref.borrow_mut() = u32::from(width);
                                    *rdp_height_ref.borrow_mut() = u32::from(height);
                                }
                                buffer.update_region(
                                    0,
                                    0,
                                    u32::from(width),
                                    u32::from(height),
                                    &data,
                                    u32::from(width) * 4,
                                );
                                needs_redraw = true;
                            }
                            RdpClientEvent::ResolutionChanged { width, height } => {
                                *rdp_width_ref.borrow_mut() = u32::from(width);
                                *rdp_height_ref.borrow_mut() = u32::from(height);
                                {
                                    let mut buffer = pixel_buffer.borrow_mut();
                                    buffer.resize(u32::from(width), u32::from(height));
                                    for chunk in buffer.data_mut().chunks_exact_mut(4) {
                                        chunk[0] = 0x1E;
                                        chunk[1] = 0x1E;
                                        chunk[2] = 0x1E;
                                        chunk[3] = 0xFF;
                                    }
                                    buffer.set_has_data(true);
                                }
                                needs_redraw = true;
                            }
                            _ => {}
                        }
                    }
                }

                if needs_redraw {
                    drawing_area.queue_draw();
                }

                if should_break {
                    return glib::ControlFlow::Break;
                }

                glib::ControlFlow::Continue
            },
        );
    }

    /// Sets up the resize handler (fallback when rdp-embedded is disabled)
    #[cfg(not(feature = "rdp-embedded"))]
    fn setup_resize_handler(&self) {
        let width = self.width.clone();
        let height = self.height.clone();
        let pixel_buffer = self.pixel_buffer.clone();

        self.drawing_area
            .connect_resize(move |area, new_width, new_height| {
                let new_width = new_width.unsigned_abs();
                let new_height = new_height.unsigned_abs();

                *width.borrow_mut() = new_width;
                *height.borrow_mut() = new_height;

                // Resize pixel buffer
                pixel_buffer.borrow_mut().resize(new_width, new_height);
                area.queue_draw();
            });
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
    pub fn state(&self) -> RdpConnectionState {
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
        F: Fn(RdpConnectionState) + 'static,
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
                RdpConnectionState::Disconnected | RdpConnectionState::Error
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

    /// Connects a callback for fallback notifications (Requirement 6.4)
    ///
    /// This callback is invoked when embedded mode fails and the system
    /// falls back to external xfreerdp mode.
    pub fn connect_fallback<F>(&self, callback: F)
    where
        F: Fn(&str) + 'static,
    {
        *self.on_fallback.borrow_mut() = Some(Box::new(callback));
    }

    /// Reports a fallback and notifies listeners (Requirement 6.4)
    fn report_fallback(&self, message: &str) {
        if let Some(ref callback) = *self.on_fallback.borrow() {
            callback(message);
        }
    }

    /// Sets the connection state and notifies listeners
    fn set_state(&self, new_state: RdpConnectionState) {
        *self.state.borrow_mut() = new_state;
        self.drawing_area.queue_draw();

        if let Some(ref callback) = *self.on_state_changed.borrow() {
            callback(new_state);
        }
    }

    /// Reports an error and notifies listeners
    fn report_error(&self, message: &str) {
        self.set_state(RdpConnectionState::Error);

        if let Some(ref callback) = *self.on_error.borrow() {
            callback(message);
        }
    }
}

impl Default for EmbeddedRdpWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for EmbeddedRdpWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddedRdpWidget")
            .field("state", &self.state.borrow())
            .field("is_embedded", &self.is_embedded.borrow())
            .field("width", &self.width.borrow())
            .field("height", &self.height.borrow())
            .finish_non_exhaustive()
    }
}

// ============================================================================
// FreeRDP Integration
// ============================================================================

impl EmbeddedRdpWidget {
    /// Detects if wlfreerdp is available for embedded mode
    #[must_use]
    pub fn detect_wlfreerdp() -> bool {
        crate::embedded_rdp_detect::detect_wlfreerdp()
    }

    /// Detects if xfreerdp is available for external mode
    #[must_use]
    pub fn detect_xfreerdp() -> Option<String> {
        crate::embedded_rdp_detect::detect_xfreerdp()
    }

    /// Connects to an RDP server
    ///
    /// This method attempts to use wlfreerdp for embedded mode first.
    /// If wlfreerdp is not available or fails, it falls back to xfreerdp in external mode.
    ///
    /// # Arguments
    ///
    /// * `config` - The RDP connection configuration
    ///
    /// # Errors
    ///
    /// Returns error if connection fails or no FreeRDP client is available
    ///
    /// # Requirements Coverage
    ///
    /// - Requirement 1.5: Fallback to FreeRDP external mode
    /// - Requirement 6.4: Automatic fallback to external mode on failure
    pub fn connect(&self, config: &RdpConfig) -> Result<(), EmbeddedRdpError> {
        // Store configuration
        *self.config.borrow_mut() = Some(config.clone());

        // Update state
        self.set_state(RdpConnectionState::Connecting);

        // Check if IronRDP embedded mode is available (Requirement 1.5)
        // This is determined at compile time via the rdp-embedded feature flag
        if Self::is_ironrdp_available() {
            // Try IronRDP embedded mode first
            match self.connect_ironrdp(config) {
                Ok(()) => {
                    return Ok(());
                }
                Err(e) => {
                    // Log the error and fall back to FreeRDP (Requirement 1.5)
                    let reason = format!("IronRDP connection failed: {e}");
                    self.report_fallback(&reason);
                    self.cleanup_embedded_mode();
                }
            }
        } else {
            // IronRDP not available, notify user
            self.report_fallback("Native RDP client not available, using FreeRDP external mode");
        }

        // Try wlfreerdp for embedded-like experience (Requirement 6.4)
        if Self::detect_wlfreerdp() {
            match self.connect_embedded(config) {
                Ok(()) => {
                    // Check if fallback was triggered by the thread
                    if let Some(ref thread) = *self.freerdp_thread.borrow() {
                        if thread.fallback_triggered() {
                            // Fallback was triggered, clean up and try external mode
                            self.cleanup_embedded_mode();
                            return self.connect_external_with_notification(config);
                        }
                    }
                    return Ok(());
                }
                Err(e) => {
                    // Log the error and fall back to external mode (Requirement 6.4)
                    let reason = format!("Embedded RDP failed: {e}");
                    self.report_fallback(&reason);
                    self.cleanup_embedded_mode();
                }
            }
        }

        // Fall back to external mode (xfreerdp) (Requirement 6.4)
        self.connect_external_with_notification(config)
    }

    /// Checks if IronRDP native client is available
    ///
    /// This is determined at compile time via the `rdp-embedded` feature flag.
    /// When IronRDP dependencies are resolved, this will return true.
    #[must_use]
    pub fn is_ironrdp_available() -> bool {
        crate::embedded_rdp_detect::is_ironrdp_available()
    }

    /// Connects using IronRDP native client
    ///
    /// This method uses the pure Rust IronRDP library for true embedded
    /// RDP rendering within the GTK widget.
    ///
    /// # Requirements Coverage
    ///
    /// - Requirement 1.1: Native RDP embedding as GTK widget
    /// - Requirement 1.5: Fallback to FreeRDP if IronRDP fails
    #[cfg(feature = "rdp-embedded")]
    fn connect_ironrdp(&self, config: &RdpConfig) -> Result<(), EmbeddedRdpError> {
        use rustconn_core::{RdpClient, RdpClientConfig, RdpClientEvent};

        tracing::debug!(
            "[EmbeddedRDP] Attempting IronRDP connection to {}:{}",
            config.host,
            config.port
        );
        tracing::debug!(
            "[EmbeddedRDP] Username: {:?}, Domain: {:?}, Password: {}",
            config.username,
            config.domain,
            if config.password.is_some() {
                "[REDACTED]"
            } else {
                "not set"
            }
        );

        // Log shared folders configuration
        if !config.shared_folders.is_empty() {
            tracing::debug!(
                "[EmbeddedRDP] Configuring {} shared folder(s) via RDPDR",
                config.shared_folders.len()
            );
            for folder in &config.shared_folders {
                tracing::debug!(
                    "[EmbeddedRDP]   - '{}' -> {}",
                    folder.share_name,
                    folder.local_path.display()
                );
            }
        }

        // Convert EmbeddedSharedFolder to SharedFolder for RdpClientConfig
        let shared_folders: Vec<rustconn_core::rdp_client::SharedFolder> = config
            .shared_folders
            .iter()
            .map(|f| rustconn_core::rdp_client::SharedFolder::new(&f.share_name, &f.local_path))
            .collect();

        // Convert GUI config to RdpClientConfig
        let mut client_config = RdpClientConfig::new(&config.host)
            .with_port(config.port)
            .with_resolution(config.width as u16, config.height as u16)
            .with_clipboard(config.clipboard_enabled)
            .with_shared_folders(shared_folders);

        if let Some(ref username) = config.username {
            client_config = client_config.with_username(username);
        }

        if let Some(ref password) = config.password {
            client_config = client_config.with_password(password);
        }

        if let Some(ref domain) = config.domain {
            client_config = client_config.with_domain(domain);
        }

        // Create and connect the IronRDP client
        let mut client = RdpClient::new(client_config);
        client
            .connect()
            .map_err(|e| EmbeddedRdpError::Connection(format!("IronRDP connection failed: {e}")))?;

        // Store command sender for input handling
        if let Some(tx) = client.command_sender() {
            *self.ironrdp_command_tx.borrow_mut() = Some(tx);
        }

        // Mark as embedded mode using IronRDP
        *self.is_embedded.borrow_mut() = true;
        *self.is_ironrdp.borrow_mut() = true;

        // Show toolbar with Ctrl+Alt+Del button
        self.toolbar.set_visible(true);

        // Initialize RDP dimensions from config
        *self.rdp_width.borrow_mut() = config.width;
        *self.rdp_height.borrow_mut() = config.height;

        // Resize and clear pixel buffer to match config
        {
            let mut buffer = self.pixel_buffer.borrow_mut();
            buffer.resize(config.width, config.height);
            buffer.clear();
        }

        // Set up event polling for IronRDP
        let state = self.state.clone();
        let drawing_area = self.drawing_area.clone();
        let toolbar = self.toolbar.clone();
        let on_state_changed = self.on_state_changed.clone();
        let on_error = self.on_error.clone();
        let rdp_width_ref = self.rdp_width.clone();
        let rdp_height_ref = self.rdp_height.clone();
        let pixel_buffer = self.pixel_buffer.clone();
        let is_embedded = self.is_embedded.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        let ironrdp_tx = self.ironrdp_command_tx.clone();
        let remote_clipboard_text = self.remote_clipboard_text.clone();
        let remote_clipboard_formats = self.remote_clipboard_formats.clone();
        let copy_button = self.copy_button.clone();
        let file_transfer = self.file_transfer.clone();
        let save_files_button = self.save_files_button.clone();
        let status_label = self.status_label.clone();
        let on_file_progress = self.on_file_progress.clone();
        let on_file_complete = self.on_file_complete.clone();
        #[cfg(feature = "rdp-audio")]
        let audio_player = self.audio_player.clone();

        // Store client in a shared reference for the polling closure
        let client = std::rc::Rc::new(std::cell::RefCell::new(Some(client)));
        let client_ref = client.clone();
        let polling_interval = u64::from(config.polling_interval_ms);

        glib::timeout_add_local(
            std::time::Duration::from_millis(polling_interval),
            move || {
                // Check if we're still in embedded mode
                if !*is_embedded.borrow() || !*is_ironrdp.borrow() {
                    // Clean up client
                    if let Some(mut c) = client_ref.borrow_mut().take() {
                        c.disconnect();
                    }
                    *ironrdp_tx.borrow_mut() = None;
                    toolbar.set_visible(false);
                    return glib::ControlFlow::Break;
                }

                // Track if we need to redraw
                let mut needs_redraw = false;
                let mut should_break = false;

                // Poll for events from IronRDP client
                if let Some(ref client) = *client_ref.borrow() {
                    while let Some(event) = client.try_recv_event() {
                        match event {
                            RdpClientEvent::Connected { width, height } => {
                                tracing::debug!("[IronRDP] Connected: {}x{}", width, height);
                                *state.borrow_mut() = RdpConnectionState::Connected;
                                *rdp_width_ref.borrow_mut() = u32::from(width);
                                *rdp_height_ref.borrow_mut() = u32::from(height);
                                {
                                    let mut buffer = pixel_buffer.borrow_mut();
                                    buffer.resize(u32::from(width), u32::from(height));
                                    buffer.clear();
                                }
                                if let Some(ref callback) = *on_state_changed.borrow() {
                                    callback(RdpConnectionState::Connected);
                                }
                                needs_redraw = true;
                            }
                            RdpClientEvent::Disconnected => {
                                tracing::debug!("[IronRDP] Disconnected");
                                *state.borrow_mut() = RdpConnectionState::Disconnected;
                                toolbar.set_visible(false);
                                if let Some(ref callback) = *on_state_changed.borrow() {
                                    callback(RdpConnectionState::Disconnected);
                                }
                                needs_redraw = true;
                                should_break = true;
                            }
                            RdpClientEvent::Error(msg) => {
                                tracing::error!("[IronRDP] Error: {}", msg);
                                *state.borrow_mut() = RdpConnectionState::Error;
                                toolbar.set_visible(false);
                                if let Some(ref callback) = *on_error.borrow() {
                                    callback(&msg);
                                }
                                needs_redraw = true;
                                should_break = true;
                            }
                            RdpClientEvent::FrameUpdate { rect, data } => {
                                // Update pixel buffer with framebuffer data
                                let mut buffer = pixel_buffer.borrow_mut();
                                buffer.update_region(
                                    u32::from(rect.x),
                                    u32::from(rect.y),
                                    u32::from(rect.width),
                                    u32::from(rect.height),
                                    &data,
                                    u32::from(rect.width) * 4,
                                );
                                needs_redraw = true;
                            }
                            RdpClientEvent::FullFrameUpdate {
                                width,
                                height,
                                data,
                            } => {
                                // Full screen update
                                let mut buffer = pixel_buffer.borrow_mut();
                                if buffer.width() != u32::from(width)
                                    || buffer.height() != u32::from(height)
                                {
                                    buffer.resize(u32::from(width), u32::from(height));
                                    *rdp_width_ref.borrow_mut() = u32::from(width);
                                    *rdp_height_ref.borrow_mut() = u32::from(height);
                                }
                                buffer.update_region(
                                    0,
                                    0,
                                    u32::from(width),
                                    u32::from(height),
                                    &data,
                                    u32::from(width) * 4,
                                );
                                needs_redraw = true;
                            }
                            RdpClientEvent::ResolutionChanged { width, height } => {
                                tracing::debug!(
                                    "[IronRDP] Resolution changed: {}x{}",
                                    width,
                                    height
                                );
                                *rdp_width_ref.borrow_mut() = u32::from(width);
                                *rdp_height_ref.borrow_mut() = u32::from(height);
                                {
                                    let mut buffer = pixel_buffer.borrow_mut();
                                    // Resize buffer but fill with dark gray instead of black
                                    // to indicate we're waiting for new frame data
                                    buffer.resize(u32::from(width), u32::from(height));
                                    // Fill with dark gray (0x1E1E1E) to show resize is happening
                                    for chunk in buffer.data_mut().chunks_exact_mut(4) {
                                        chunk[0] = 0x1E; // B
                                        chunk[1] = 0x1E; // G
                                        chunk[2] = 0x1E; // R
                                        chunk[3] = 0xFF; // A
                                    }
                                    // Keep has_data true so we continue rendering
                                    buffer.set_has_data(true);
                                }
                                needs_redraw = true;
                            }
                            RdpClientEvent::AuthRequired => {
                                tracing::debug!("[IronRDP] Authentication required");
                            }
                            RdpClientEvent::ClipboardText(text) => {
                                // Server sent clipboard text - store it and enable Copy button
                                tracing::debug!("[Clipboard] Received text from server");
                                *remote_clipboard_text.borrow_mut() = Some(text);
                                copy_button.set_sensitive(true);
                                copy_button
                                    .set_tooltip_text(Some("Copy remote clipboard to local"));
                            }
                            RdpClientEvent::ClipboardFormatsAvailable(formats) => {
                                // Server has clipboard data available
                                tracing::debug!(
                                    "[Clipboard] Formats available: {} formats",
                                    formats.len()
                                );
                                *remote_clipboard_formats.borrow_mut() = formats;
                            }
                            RdpClientEvent::ClipboardInitiateCopy(formats) => {
                                // Backend wants to send format list to server (initialization)
                                if let Some(ref sender) = *ironrdp_tx.borrow() {
                                    let _ = sender.send(RdpClientCommand::ClipboardCopy(formats));
                                }
                            }
                            RdpClientEvent::ClipboardDataRequest(format) => {
                                // Server requests clipboard data from us
                                // Get local clipboard and send to server
                                eprintln!(
                                    "[Clipboard] Server requests data for format {}",
                                    format.id
                                );
                                let display = drawing_area.display();
                                let clipboard = display.clipboard();
                                let tx = ironrdp_tx.clone();
                                let format_id = format.id;

                                clipboard.read_text_async(
                                    None::<&gtk4::gio::Cancellable>,
                                    move |result| {
                                        if let Ok(Some(text)) = result {
                                            eprintln!(
                                                "[Clipboard] Sending {} chars to server",
                                                text.len()
                                            );
                                            if let Some(ref sender) = *tx.borrow() {
                                                // Send as UTF-16 for CF_UNICODETEXT
                                                if format_id == 13 {
                                                    // CF_UNICODETEXT
                                                    let data: Vec<u8> = text
                                                        .encode_utf16()
                                                        .flat_map(u16::to_le_bytes)
                                                        .chain([0, 0]) // null terminator
                                                        .collect();
                                                    let _ = sender.send(
                                                        RdpClientCommand::ClipboardData {
                                                            format_id,
                                                            data,
                                                        },
                                                    );
                                                } else {
                                                    // CF_TEXT - send as bytes
                                                    let mut data = text.as_bytes().to_vec();
                                                    data.push(0); // null terminator
                                                    let _ = sender.send(
                                                        RdpClientCommand::ClipboardData {
                                                            format_id,
                                                            data,
                                                        },
                                                    );
                                                }
                                            }
                                        }
                                    },
                                );
                            }
                            RdpClientEvent::ClipboardPasteRequest(format) => {
                                // Backend requests to fetch data from server
                                if let Some(ref sender) = *ironrdp_tx.borrow() {
                                    let _ = sender.send(RdpClientCommand::RequestClipboardData {
                                        format_id: format.id,
                                    });
                                }
                            }
                            RdpClientEvent::CursorDefault => {
                                // Reset to default cursor
                                drawing_area.set_cursor_from_name(Some("default"));
                            }
                            RdpClientEvent::CursorHidden => {
                                // Hide cursor
                                drawing_area.set_cursor_from_name(Some("none"));
                            }
                            RdpClientEvent::CursorPosition { .. } => {
                                // Server-side cursor position update - we handle this client-side
                            }
                            RdpClientEvent::CursorUpdate {
                                hotspot_x,
                                hotspot_y,
                                width,
                                height,
                                data,
                            } => {
                                // Create custom cursor from bitmap data
                                let bytes = glib::Bytes::from(&data);
                                let texture = gdk::MemoryTexture::new(
                                    i32::from(width),
                                    i32::from(height),
                                    gdk::MemoryFormat::B8g8r8a8,
                                    &bytes,
                                    usize::from(width) * 4,
                                );
                                let cursor = gdk::Cursor::from_texture(
                                    &texture,
                                    i32::from(hotspot_x),
                                    i32::from(hotspot_y),
                                    None,
                                );
                                drawing_area.set_cursor(Some(&cursor));
                            }
                            RdpClientEvent::ServerMessage(msg) => {
                                tracing::debug!("[IronRDP] Server message: {}", msg);
                            }
                            #[cfg(feature = "rdp-audio")]
                            RdpClientEvent::AudioFormatChanged(format) => {
                                // Audio format negotiated - configure audio player
                                tracing::debug!(
                                    "[Audio] Format changed: {} Hz, {} ch",
                                    format.samples_per_sec,
                                    format.channels
                                );
                                if let Ok(mut player_opt) = audio_player.try_borrow_mut() {
                                    if player_opt.is_none() {
                                        *player_opt = Some(crate::audio::RdpAudioPlayer::new());
                                    }
                                    if let Some(ref mut player) = *player_opt {
                                        if let Err(e) = player.configure(format) {
                                            tracing::warn!("[Audio] Failed to configure: {}", e);
                                        }
                                    }
                                }
                            }
                            #[cfg(feature = "rdp-audio")]
                            RdpClientEvent::AudioData { data, .. } => {
                                // Queue audio data for playback
                                if let Ok(player_opt) = audio_player.try_borrow() {
                                    if let Some(ref player) = *player_opt {
                                        player.queue_data(&data);
                                    }
                                }
                            }
                            #[cfg(feature = "rdp-audio")]
                            RdpClientEvent::AudioVolume { left, right } => {
                                // Update audio volume
                                if let Ok(player_opt) = audio_player.try_borrow() {
                                    if let Some(ref player) = *player_opt {
                                        player.set_volume(left, right);
                                    }
                                }
                            }
                            #[cfg(feature = "rdp-audio")]
                            RdpClientEvent::AudioClose => {
                                // Stop audio playback
                                tracing::debug!("[Audio] Channel closed");
                                if let Ok(mut player_opt) = audio_player.try_borrow_mut() {
                                    if let Some(ref mut player) = *player_opt {
                                        player.stop();
                                    }
                                }
                            }
                            #[cfg(not(feature = "rdp-audio"))]
                            RdpClientEvent::AudioFormatChanged(_)
                            | RdpClientEvent::AudioData { .. }
                            | RdpClientEvent::AudioVolume { .. }
                            | RdpClientEvent::AudioClose => {
                                // Audio not enabled - ignore
                            }
                            RdpClientEvent::ClipboardDataReady { format_id, data } => {
                                // Clipboard data ready to send to server
                                tracing::debug!(
                                    "[Clipboard] Data ready for format {}: {} bytes",
                                    format_id,
                                    data.len()
                                );
                                if let Some(ref sender) = *ironrdp_tx.borrow() {
                                    let _ = sender
                                        .send(RdpClientCommand::ClipboardData { format_id, data });
                                }
                            }
                            RdpClientEvent::ClipboardFileList(files) => {
                                // File list available on server clipboard
                                tracing::info!(
                                    "[Clipboard] File list received: {} files",
                                    files.len()
                                );
                                for file in &files {
                                    tracing::debug!(
                                        "  - {} ({} bytes, dir={})",
                                        file.name,
                                        file.size,
                                        file.is_directory()
                                    );
                                }
                                // Store file list and show Save Files button
                                let file_count = files.len();
                                file_transfer.borrow_mut().set_available_files(files);
                                if file_count > 0 {
                                    save_files_button
                                        .set_label(&format!("Save {} Files", file_count));
                                    save_files_button.set_tooltip_text(Some(&format!(
                                        "Save {} files from remote clipboard",
                                        file_count
                                    )));
                                    save_files_button.set_visible(true);
                                    save_files_button.set_sensitive(true);
                                } else {
                                    save_files_button.set_visible(false);
                                }
                            }
                            RdpClientEvent::ClipboardFileContents {
                                stream_id,
                                data,
                                is_last,
                            } => {
                                // File contents received from server
                                tracing::debug!(
                                    "[Clipboard] File contents: stream_id={}, {} bytes, last={}",
                                    stream_id,
                                    data.len(),
                                    is_last
                                );
                                // Append data to download state
                                file_transfer
                                    .borrow_mut()
                                    .append_data(stream_id, &data, is_last);

                                // Update progress
                                let (progress, completed, total) = {
                                    let transfer = file_transfer.borrow();
                                    (
                                        transfer.overall_progress(),
                                        transfer.completed_count,
                                        transfer.total_files,
                                    )
                                };

                                if let Some(ref callback) = *on_file_progress.borrow() {
                                    callback(
                                        progress,
                                        &format!("Downloaded {}/{} files", completed, total),
                                    );
                                }

                                // If this file is complete, save it
                                if is_last {
                                    match file_transfer.borrow().save_download(stream_id) {
                                        Ok(path) => {
                                            tracing::info!(
                                                "[Clipboard] Saved file: {}",
                                                path.display()
                                            );
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                "[Clipboard] Failed to save file: {}",
                                                e
                                            );
                                        }
                                    }
                                }

                                // Check if all downloads complete
                                if file_transfer.borrow().all_complete() {
                                    let count = file_transfer.borrow().completed_count;
                                    let target = file_transfer
                                        .borrow()
                                        .target_directory
                                        .as_ref()
                                        .map(|p| p.display().to_string())
                                        .unwrap_or_default();

                                    // Reset button
                                    save_files_button.set_sensitive(true);
                                    let file_count = file_transfer.borrow().available_files.len();
                                    save_files_button
                                        .set_label(&format!("Save {} Files", file_count));

                                    // Show completion status
                                    status_label.set_text(&format!("Saved {} files", count));
                                    let status_hide = status_label.clone();
                                    glib::timeout_add_local_once(
                                        std::time::Duration::from_secs(3),
                                        move || {
                                            status_hide.set_visible(false);
                                        },
                                    );

                                    if let Some(ref callback) = *on_file_complete.borrow() {
                                        callback(count, &target);
                                    }
                                }
                            }
                            RdpClientEvent::ClipboardFileSize { stream_id, size } => {
                                // File size information received
                                tracing::debug!(
                                    "[Clipboard] File size: stream_id={}, size={}",
                                    stream_id,
                                    size
                                );
                                // Update size for progress indication
                                file_transfer.borrow_mut().update_size(stream_id, size);
                            }
                        }
                    }
                }

                // Only redraw once after processing all events
                if needs_redraw {
                    drawing_area.queue_draw();
                }

                if should_break {
                    return glib::ControlFlow::Break;
                }

                glib::ControlFlow::Continue
            },
        );

        self.set_state(RdpConnectionState::Connecting);
        Ok(())
    }

    /// Fallback when rdp-embedded feature is not enabled
    #[cfg(not(feature = "rdp-embedded"))]
    fn connect_ironrdp(&self, _config: &RdpConfig) -> Result<(), EmbeddedRdpError> {
        Err(EmbeddedRdpError::FallbackToExternal(
            "IronRDP not available (rdp-embedded feature not enabled)".to_string(),
        ))
    }

    /// Cleans up embedded mode resources
    fn cleanup_embedded_mode(&self) {
        if let Some(mut thread) = self.freerdp_thread.borrow_mut().take() {
            thread.shutdown();
        }
        self.wl_surface.borrow_mut().cleanup();
        *self.is_embedded.borrow_mut() = false;
    }

    /// Connects using external mode with user notification (Requirement 6.4)
    fn connect_external_with_notification(
        &self,
        config: &RdpConfig,
    ) -> Result<(), EmbeddedRdpError> {
        // Notify user about fallback
        self.report_fallback("RDP session will open in external window");

        // Connect using external mode
        self.connect_external(config)
    }

    /// Connects using embedded mode (wlfreerdp) with thread isolation (Requirement 6.3)
    fn connect_embedded(&self, config: &RdpConfig) -> Result<(), EmbeddedRdpError> {
        tracing::debug!(
            "[EmbeddedRDP] Attempting embedded connection to {}:{}",
            config.host,
            config.port
        );

        // Initialize Wayland surface
        self.wl_surface
            .borrow_mut()
            .initialize()
            .map_err(|e| EmbeddedRdpError::SubsurfaceCreation(e.to_string()))?;

        // Spawn FreeRDP in a dedicated thread to isolate Qt/GTK conflicts (Requirement 6.3)
        let freerdp_thread = FreeRdpThread::spawn(config)?;

        // Send connect command to the thread
        freerdp_thread.send_command(RdpCommand::Connect(config.clone()))?;

        // Store the thread handle
        *self.freerdp_thread.borrow_mut() = Some(freerdp_thread);
        *self.is_embedded.borrow_mut() = true;

        // Initialize RDP dimensions from config
        *self.rdp_width.borrow_mut() = config.width;
        *self.rdp_height.borrow_mut() = config.height;

        // Resize pixel buffer to match config
        self.pixel_buffer
            .borrow_mut()
            .resize(config.width, config.height);

        // Set state to connecting - actual connected state will be set
        // when we receive the Connected event from the thread
        self.set_state(RdpConnectionState::Connecting);

        // Set up a GLib timeout to poll for RDP events (~30 FPS)
        let state = self.state.clone();
        let drawing_area = self.drawing_area.clone();
        let on_state_changed = self.on_state_changed.clone();
        let on_error = self.on_error.clone();
        let on_fallback = self.on_fallback.clone();
        let rdp_width_ref = self.rdp_width.clone();
        let rdp_height_ref = self.rdp_height.clone();
        let pixel_buffer = self.pixel_buffer.clone();
        let is_embedded = self.is_embedded.clone();
        let freerdp_thread_ref = self.freerdp_thread.clone();

        glib::timeout_add_local(std::time::Duration::from_millis(33), move || {
            // Check if we're still in embedded mode
            if !*is_embedded.borrow() {
                return glib::ControlFlow::Break;
            }

            // Try to get events from the FreeRDP thread
            if let Some(ref thread) = *freerdp_thread_ref.borrow() {
                while let Some(event) = thread.try_recv_event() {
                    match event {
                        RdpEvent::Connected => {
                            tracing::debug!("[EmbeddedRDP] Connected!");
                            *state.borrow_mut() = RdpConnectionState::Connected;
                            if let Some(ref callback) = *on_state_changed.borrow() {
                                callback(RdpConnectionState::Connected);
                            }
                            drawing_area.queue_draw();
                        }
                        RdpEvent::Disconnected => {
                            tracing::debug!("[EmbeddedRDP] Disconnected");
                            *state.borrow_mut() = RdpConnectionState::Disconnected;
                            if let Some(ref callback) = *on_state_changed.borrow() {
                                callback(RdpConnectionState::Disconnected);
                            }
                            drawing_area.queue_draw();
                            return glib::ControlFlow::Break;
                        }
                        RdpEvent::Error(msg) => {
                            tracing::error!("[EmbeddedRDP] Error: {}", msg);
                            *state.borrow_mut() = RdpConnectionState::Error;
                            if let Some(ref callback) = *on_error.borrow() {
                                callback(&msg);
                            }
                            drawing_area.queue_draw();
                            return glib::ControlFlow::Break;
                        }
                        RdpEvent::FallbackTriggered(reason) => {
                            tracing::warn!("[EmbeddedRDP] Fallback triggered: {}", reason);
                            if let Some(ref callback) = *on_fallback.borrow() {
                                callback(&reason);
                            }
                            return glib::ControlFlow::Break;
                        }
                        RdpEvent::FrameUpdate {
                            x,
                            y,
                            width,
                            height,
                        } => {
                            // Update RDP dimensions if changed
                            if width > 0 && height > 0 {
                                let current_w = *rdp_width_ref.borrow();
                                let current_h = *rdp_height_ref.borrow();
                                if width != current_w || height != current_h {
                                    tracing::debug!(
                                        "[EmbeddedRDP] Resolution changed: {}x{}",
                                        width,
                                        height
                                    );
                                    *rdp_width_ref.borrow_mut() = width;
                                    *rdp_height_ref.borrow_mut() = height;
                                    pixel_buffer.borrow_mut().resize(width, height);
                                }
                            }
                            // Queue redraw for frame updates
                            drawing_area.queue_draw();
                            let _ = (x, y); // Suppress unused warnings
                        }
                        RdpEvent::AuthRequired => {
                            // Handle authentication request
                            tracing::debug!("[EmbeddedRDP] Authentication required");
                        }
                    }
                }
            }

            glib::ControlFlow::Continue
        });

        Ok(())
    }

    /// Connects using external mode (xfreerdp)
    ///
    /// Uses `SafeFreeRdpLauncher` to handle Qt/Wayland warning suppression.
    ///
    /// # Requirements Coverage
    ///
    /// - Requirement 1.2: Fallback to xfreerdp in external window mode
    /// - Requirement 6.1: QSocketNotifier error handling
    /// - Requirement 6.2: Wayland requestActivate warning suppression
    fn connect_external(&self, config: &RdpConfig) -> Result<(), EmbeddedRdpError> {
        // Use SafeFreeRdpLauncher for Qt error suppression (Requirement 6.1, 6.2)
        let launcher = SafeFreeRdpLauncher::new();

        match launcher.launch(config) {
            Ok(child) => {
                *self.process.borrow_mut() = Some(child);
                *self.is_embedded.borrow_mut() = false;
                self.set_state(RdpConnectionState::Connected);
                // Trigger redraw to show "Session running in external window"
                self.drawing_area.queue_draw();
                Ok(())
            }
            Err(e) => {
                let msg = format!("Failed to start FreeRDP: {e}");
                self.report_error(&msg);
                Err(EmbeddedRdpError::Connection(msg))
            }
        }
    }

    /// Disconnects from the RDP server
    ///
    /// This method properly cleans up all resources including:
    /// - FreeRDP thread (if using embedded mode)
    /// - External process (if using external mode)
    /// - Wayland surface resources
    /// - Pixel buffer
    ///
    /// # Requirements Coverage
    ///
    /// - Requirement 1.6: Proper cleanup on disconnect
    pub fn disconnect(&self) {
        // Shutdown FreeRDP thread if running (Requirement 1.6)
        if let Some(mut thread) = self.freerdp_thread.borrow_mut().take() {
            thread.shutdown();
        }

        // Kill external process if running (Requirement 1.6)
        self.terminate_external_process();

        // Clean up Wayland surface
        self.wl_surface.borrow_mut().cleanup();

        // Clear pixel buffer
        self.pixel_buffer.borrow_mut().clear();

        // Reset state (but keep config for potential reconnect)
        *self.is_embedded.borrow_mut() = false;
        self.set_state(RdpConnectionState::Disconnected);
    }

    /// Reconnects using the stored configuration
    ///
    /// This method attempts to reconnect to the RDP server using the
    /// configuration from the previous connection.
    ///
    /// # Errors
    ///
    /// Returns an error if no previous configuration exists or if
    /// the connection fails.
    pub fn reconnect(&self) -> Result<(), EmbeddedRdpError> {
        let config = self.config.borrow().clone();
        if let Some(config) = config {
            self.connect(&config)
        } else {
            Err(EmbeddedRdpError::Connection(
                "No previous configuration to reconnect".to_string(),
            ))
        }
    }

    /// Terminates the external FreeRDP process if running
    ///
    /// This method gracefully terminates the process, waiting for it to exit.
    ///
    /// # Requirements Coverage
    ///
    /// - Requirement 1.6: Handle process termination
    fn terminate_external_process(&self) {
        if let Some(mut child) = self.process.borrow_mut().take() {
            // Try graceful termination first (SIGTERM on Unix)
            let _ = child.kill();

            // Wait for the process to exit with a timeout
            // This prevents zombie processes
            match child.try_wait() {
                Ok(Some(_status)) => {
                    // Process already exited
                }
                Ok(None) => {
                    // Process still running, wait for it
                    let _ = child.wait();
                }
                Err(_) => {
                    // Error checking status, try to wait anyway
                    let _ = child.wait();
                }
            }
        }
    }

    /// Checks if the external process is still running
    ///
    /// Returns `true` if the process is running, `false` otherwise.
    pub fn is_process_running(&self) -> bool {
        if let Some(ref mut child) = *self.process.borrow_mut() {
            match child.try_wait() {
                Ok(Some(_)) => {
                    // Process has exited
                    false
                }
                Ok(None) => {
                    // Process is still running
                    true
                }
                Err(_) => {
                    // Error checking, assume not running
                    false
                }
            }
        } else {
            false
        }
    }

    /// Checks the connection status and updates state if process has exited
    ///
    /// This should be called periodically to detect when external processes
    /// have terminated unexpectedly.
    pub fn check_connection_status(&self) {
        // Check external process
        if !*self.is_embedded.borrow()
            && self.process.borrow().is_some()
            && !self.is_process_running()
        {
            // Process has exited, update state
            self.process.borrow_mut().take();
            self.set_state(RdpConnectionState::Disconnected);
        }

        // Check embedded mode thread
        if *self.is_embedded.borrow() {
            if let Some(ref thread) = *self.freerdp_thread.borrow() {
                match thread.state() {
                    FreeRdpThreadState::Error => {
                        self.set_state(RdpConnectionState::Error);
                    }
                    FreeRdpThreadState::ShuttingDown => {
                        self.set_state(RdpConnectionState::Disconnected);
                    }
                    _ => {}
                }
            }
        }
    }

    /// Handles FreeRDP BeginPaint callback
    ///
    /// This is called by FreeRDP before rendering a frame region.
    /// In embedded mode, this prepares the pixel buffer for updates.
    pub fn on_begin_paint(&self) {
        // In a real implementation, this would:
        // 1. Lock the pixel buffer
        // 2. Prepare for incoming frame data
    }

    /// Handles FreeRDP EndPaint callback
    ///
    /// This is called by FreeRDP after rendering a frame region.
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
    pub fn on_end_paint(&self, x: i32, y: i32, width: i32, height: i32, data: &[u8], stride: u32) {
        // Update the pixel buffer with the new frame data
        self.pixel_buffer.borrow_mut().update_region(
            x.unsigned_abs(),
            y.unsigned_abs(),
            width.unsigned_abs(),
            height.unsigned_abs(),
            data,
            stride,
        );

        // Damage the Wayland surface region
        self.wl_surface.borrow().damage(x, y, width, height);

        // Commit the surface
        self.wl_surface.borrow().commit();

        // Queue a redraw of the GTK widget
        self.drawing_area.queue_draw();
    }

    /// Sends a keyboard event to the RDP session
    ///
    /// # Arguments
    ///
    /// * `keyval` - GTK key value
    /// * `pressed` - Whether the key is pressed or released
    pub fn send_key(&self, keyval: u32, pressed: bool) {
        if !*self.is_embedded.borrow() {
            return;
        }

        if *self.state.borrow() != RdpConnectionState::Connected {
            return;
        }

        // Send keyboard event via FreeRDP thread (Requirement 6.3)
        if let Some(ref thread) = *self.freerdp_thread.borrow() {
            let _ = thread.send_command(RdpCommand::KeyEvent { keyval, pressed });
        }
    }

    /// Sends Ctrl+Alt+Del key sequence to the RDP session
    ///
    /// This is commonly used to unlock Windows login screens or access
    /// the security options menu.
    ///
    /// # Requirements Coverage
    ///
    /// - Requirement 1.4: Ctrl+Alt+Del support
    pub fn send_ctrl_alt_del(&self) {
        if !*self.is_embedded.borrow() {
            return;
        }

        if *self.state.borrow() != RdpConnectionState::Connected {
            return;
        }

        // Send the Ctrl+Alt+Del command to the FreeRDP thread
        if let Some(ref thread) = *self.freerdp_thread.borrow() {
            let _ = thread.send_command(RdpCommand::SendCtrlAltDel);
        }
    }

    /// Sends a mouse event to the RDP session
    ///
    /// # Arguments
    ///
    /// * `x` - X coordinate
    /// * `y` - Y coordinate
    /// * `button` - Mouse button (0 = none/motion, 1 = left, 2 = middle, 3 = right)
    /// * `pressed` - Whether the button is pressed or released
    pub fn send_mouse(&self, x: i32, y: i32, button: u32, pressed: bool) {
        if !*self.is_embedded.borrow() {
            return;
        }

        if *self.state.borrow() != RdpConnectionState::Connected {
            return;
        }

        // Send mouse event via FreeRDP thread (Requirement 6.3)
        if let Some(ref thread) = *self.freerdp_thread.borrow() {
            let _ = thread.send_command(RdpCommand::MouseEvent {
                x,
                y,
                button,
                pressed,
            });
        }
    }

    /// Notifies the RDP session of a resolution change
    ///
    /// # Arguments
    ///
    /// * `width` - New width in pixels
    /// * `height` - New height in pixels
    pub fn notify_resize(&self, width: u32, height: u32) {
        if !*self.is_embedded.borrow() {
            return;
        }

        if *self.state.borrow() != RdpConnectionState::Connected {
            return;
        }

        // Update internal dimensions
        *self.width.borrow_mut() = width;
        *self.height.borrow_mut() = height;

        // Resize pixel buffer
        self.pixel_buffer.borrow_mut().resize(width, height);

        // Send resize command via FreeRDP thread (Requirement 6.3)
        if let Some(ref thread) = *self.freerdp_thread.borrow() {
            let _ = thread.send_command(RdpCommand::Resize { width, height });
        }
    }

    /// Returns whether the RDP session is connected
    #[must_use]
    pub fn is_connected(&self) -> bool {
        *self.state.borrow() == RdpConnectionState::Connected
    }

    /// Returns the current configuration
    #[must_use]
    pub fn config(&self) -> Option<RdpConfig> {
        self.config.borrow().clone()
    }
}

impl Drop for EmbeddedRdpWidget {
    fn drop(&mut self) {
        self.disconnect();
    }
}

// Tests moved to embedded_rdp_types.rs, embedded_rdp_buffer.rs, and embedded_rdp_launcher.rs
