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

pub mod buffer;
pub mod detect;
pub mod launcher;
pub mod thread;
pub mod types;
pub mod ui;

mod clipboard;
mod connection;
mod drawing;
mod input;
mod resize;

// Re-export types for external use
pub use buffer::{CairoBackedBuffer, PixelBuffer, WaylandSurfaceHandle};
pub use launcher::SafeFreeRdpLauncher;
pub use thread::FreeRdpThread;
#[cfg(feature = "rdp-embedded")]
pub use thread::{ClipboardFileTransfer, FileDownloadState};
pub use types::{
    EmbeddedRdpError, EmbeddedSharedFolder, FreeRdpThreadState, RdpCommand, RdpConfig,
    RdpConnectionState, RdpEvent,
};

use types::{ErrorCallback, FallbackCallback, StateCallback};

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, DrawingArea, Label, Orientation};
use std::cell::RefCell;
use std::process::Child;
use std::rc::Rc;

use crate::i18n::i18n;

#[cfg(feature = "rdp-embedded")]
use rustconn_core::rdp_client::RdpClientCommand;

/// Invokes a callback stored in a `RefCell<Option<T>>` using the take-invoke-restore
/// pattern. This prevents `BorrowMutError` panics when the callback re-enters and
/// borrows the same cell.
fn with_callback<T>(cell: &RefCell<Option<T>>, f: impl FnOnce(&T)) {
    let cb = cell.borrow_mut().take();
    if let Some(ref callback) = cb {
        f(callback);
        *cell.borrow_mut() = cb;
    }
}

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
    /// Persistent Cairo-backed pixel buffer for zero-copy rendering.
    /// Used by IronRDP embedded mode to avoid 33MB copies per frame at 4K.
    cairo_buffer: Rc<RefCell<CairoBackedBuffer>>,
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
    /// Connection generation counter to track stale callbacks
    /// Incremented on each connect() call to invalidate old polling loops
    connection_generation: Rc<RefCell<u64>>,
    /// Unique widget ID for debugging
    widget_id: u64,
    /// Signal handler ID for the drawing area resize handler
    resize_handler_id: Rc<RefCell<Option<glib::SignalHandlerId>>>,
    /// Signal handler ID for local clipboard change monitoring (Phase 3)
    #[cfg(feature = "rdp-embedded")]
    clipboard_handler_id: Rc<RefCell<Option<glib::SignalHandlerId>>>,
    /// Mouse jiggler timer source ID (sends periodic mouse moves to prevent idle disconnect)
    jiggler_timer: Rc<RefCell<Option<glib::SourceId>>>,
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
        let copy_button = Button::with_label(&i18n("Copy"));
        copy_button.set_tooltip_text(Some(&i18n(
            "Copy remote clipboard to local (waiting for remote data...)",
        )));
        copy_button.set_sensitive(false); // Disabled until we receive clipboard data
        toolbar.append(&copy_button);

        // Paste button - pastes from local clipboard to remote
        let paste_button = Button::with_label(&i18n("Paste"));
        paste_button.set_tooltip_text(Some(&i18n("Paste from local clipboard to remote session")));
        toolbar.append(&paste_button);

        // Separator
        let separator = gtk4::Separator::new(Orientation::Vertical);
        separator.set_margin_start(4);
        separator.set_margin_end(4);
        toolbar.append(&separator);

        let ctrl_alt_del_button = Button::with_label("Ctrl+Alt+Del");
        ctrl_alt_del_button.add_css_class("suggested-action"); // Blue button style
        ctrl_alt_del_button.set_tooltip_text(Some(&i18n("Send Ctrl+Alt+Del to remote session")));
        toolbar.append(&ctrl_alt_del_button);

        // Windows Admin quick actions dropdown menu
        let quick_actions_button = gtk4::MenuButton::new();
        quick_actions_button.set_icon_name("view-more-symbolic");
        quick_actions_button.set_tooltip_text(Some(&i18n("Windows admin tools")));
        quick_actions_button.add_css_class("flat");
        {
            let menu = gtk4::gio::Menu::new();
            for action in rustconn_core::QUICK_ACTIONS {
                menu.append(
                    Some(&i18n(action.label)),
                    Some(&format!("rdp.{}", action.id)),
                );
            }
            quick_actions_button.set_menu_model(Some(&menu));
        }
        toolbar.append(&quick_actions_button);

        // Save Files button (shown when files available on remote clipboard)
        #[cfg(feature = "rdp-embedded")]
        let save_files_button = Button::with_label(&i18n("Save Files"));
        #[cfg(feature = "rdp-embedded")]
        {
            save_files_button.set_tooltip_text(Some(&i18n("Save files from remote clipboard")));
            save_files_button.set_visible(false); // Hidden until files available
            toolbar.append(&save_files_button);
        }

        // Reconnect button (shown when disconnected)
        let reconnect_button = Button::with_label(&i18n("Reconnect"));
        reconnect_button.add_css_class("suggested-action");
        reconnect_button.set_tooltip_text(Some(&i18n("Reconnect to the remote session")));
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
        let cairo_buffer = Rc::new(RefCell::new(CairoBackedBuffer::new(1280, 720)));
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
            cairo_buffer,
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
            connection_generation: Rc::new(RefCell::new(0)),
            widget_id: {
                use std::sync::atomic::{AtomicU64, Ordering};
                static WIDGET_COUNTER: AtomicU64 = AtomicU64::new(1);
                WIDGET_COUNTER.fetch_add(1, Ordering::SeqCst)
            },
            resize_handler_id: Rc::new(RefCell::new(None)),
            #[cfg(feature = "rdp-embedded")]
            clipboard_handler_id: Rc::new(RefCell::new(None)),
            jiggler_timer: Rc::new(RefCell::new(None)),
        };

        widget.setup_drawing();
        widget.setup_input_handlers();
        widget.setup_resize_handler();
        widget.setup_clipboard_buttons(&copy_button, &paste_button);
        widget.setup_ctrl_alt_del_button(&ctrl_alt_del_button);
        widget.setup_quick_actions();
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

    /// Sets up the Windows admin quick actions menu.
    ///
    /// Registers a GIO action for each entry in [`rustconn_core::QUICK_ACTIONS`].
    /// When triggered, the action builds the corresponding key sequence and
    /// sends it to the IronRDP client via `SendKeySequence`.
    fn setup_quick_actions(&self) {
        use gtk4::gio;

        let action_group = gio::SimpleActionGroup::new();

        for action_def in rustconn_core::QUICK_ACTIONS {
            let action = gio::SimpleAction::new(action_def.id, None);

            #[cfg(feature = "rdp-embedded")]
            {
                let tx = self.ironrdp_command_tx.clone();
                let action_id = action_def.id;
                action.connect_activate(move |_, _| {
                    if let Some(keys) = rustconn_core::build_rdp_quick_action(action_id)
                        && let Some(ref sender) = *tx.borrow()
                    {
                        let _ =
                            sender.send(rustconn_core::RdpClientCommand::SendKeySequence { keys });
                        tracing::info!(
                            protocol = "rdp",
                            action = action_id,
                            "Quick action triggered"
                        );
                    }
                });
            }

            action_group.add_action(&action);
        }

        self.container
            .insert_action_group("rdp", Some(&action_group));
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
                .title(i18n("Select folder to save files"))
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
                    if let Ok(folder) = result
                        && let Some(path) = folder.path()
                    {
                        // Set target directory and start downloads
                        {
                            let mut transfer = file_transfer_clone.borrow_mut();
                            transfer.target_directory = Some(path.clone());
                            transfer.total_files = files_clone.len();
                            transfer.completed_count = 0;
                        }

                        // Disable button during transfer
                        save_btn_clone.set_sensitive(false);
                        save_btn_clone.set_label(&i18n("Downloading..."));

                        // Request file contents for each file
                        if let Some(ref sender) = *ironrdp_tx_clone.borrow() {
                            for (idx, file) in files_clone.iter().enumerate() {
                                let stream_id = {
                                    let mut transfer = file_transfer_clone.borrow_mut();
                                    transfer.start_download(idx as u32)
                                };

                                if let Some(sid) = stream_id {
                                    // First request size, then data
                                    let _ = sender.send(RdpClientCommand::RequestFileContents {
                                        stream_id: sid,
                                        file_index: file.index,
                                        request_size: true,
                                        offset: 0,
                                        length: 0,
                                    });

                                    // Then request actual data
                                    let _ = sender.send(RdpClientCommand::RequestFileContents {
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
                        status_label_clone.set_text(&i18n("Downloading files..."));
                        status_label_clone.set_visible(true);

                        if let Some(ref callback) = *on_progress_clone.borrow() {
                            callback(0.0, "Starting download...");
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

    /// Queues a redraw of the drawing area
    pub fn queue_draw(&self) {
        self.drawing_area.queue_draw();
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
        with_callback(&self.on_fallback, |cb| cb(message));
    }

    /// Sets the connection state and notifies listeners
    fn set_state(&self, new_state: RdpConnectionState) {
        *self.state.borrow_mut() = new_state;
        self.drawing_area.queue_draw();

        // Auto-manage jiggler based on connection state
        match new_state {
            RdpConnectionState::Connected => {
                // Check if jiggler is enabled in config
                if let Some(ref config) = *self.config.borrow()
                    && config.jiggler_enabled
                {
                    self.start_jiggler(config.jiggler_interval_secs);
                }
            }
            RdpConnectionState::Disconnected | RdpConnectionState::Error => {
                self.stop_jiggler();
            }
            RdpConnectionState::Connecting => {}
        }

        with_callback(&self.on_state_changed, |cb| cb(new_state));
    }

    /// Starts the mouse jiggler timer
    ///
    /// Sends a tiny ±1px mouse movement every `interval_secs` seconds to
    /// prevent the remote session from going idle and disconnecting.
    /// Uses a toggling offset so the cursor oscillates in place without
    /// drifting or jumping to the screen center.
    pub fn start_jiggler(&self, interval_secs: u32) {
        self.stop_jiggler();

        let interval = interval_secs.clamp(10, 600);
        let state = self.state.clone();
        let is_ironrdp = self.is_ironrdp.clone();
        #[cfg(feature = "rdp-embedded")]
        let ironrdp_tx = self.ironrdp_command_tx.clone();
        let freerdp_thread = self.freerdp_thread.clone();
        let rdp_width = self.rdp_width.clone();
        let rdp_height = self.rdp_height.clone();
        // Toggle between +1 and -1 so the cursor oscillates in place
        let jiggle_toggle = Rc::new(std::cell::Cell::new(false));

        let source_id = glib::timeout_add_seconds_local(interval, move || {
            let current_state = *state.borrow();
            if current_state != RdpConnectionState::Connected {
                return glib::ControlFlow::Break;
            }

            // Use center as a safe reference point but only move ±1px
            let cx = *rdp_width.borrow() / 2;
            let cy = *rdp_height.borrow() / 2;
            let toggle = jiggle_toggle.get();
            jiggle_toggle.set(!toggle);
            let offset: i32 = if toggle { 1 } else { -1 };

            let using_ironrdp = *is_ironrdp.borrow();
            if using_ironrdp {
                #[cfg(feature = "rdp-embedded")]
                if let Some(ref tx) = *ironrdp_tx.borrow() {
                    let _ = tx.send(RdpClientCommand::PointerEvent {
                        x: (cx as i32 + offset).max(0) as u16,
                        y: cy as u16,
                        buttons: 0,
                    });
                }
            } else if let Some(ref thread) = *freerdp_thread.borrow() {
                let _ = thread.send_command(RdpCommand::MouseEvent {
                    x: cx as i32 + offset,
                    y: cy as i32,
                    button: 0,
                    pressed: false,
                });
            }

            tracing::trace!("Mouse jiggler tick");
            glib::ControlFlow::Continue
        });

        *self.jiggler_timer.borrow_mut() = Some(source_id);
        tracing::debug!(interval_secs = interval, "Mouse jiggler started");
    }

    /// Stops the mouse jiggler timer
    pub fn stop_jiggler(&self) {
        if let Some(source_id) = self.jiggler_timer.borrow_mut().take() {
            source_id.remove();
            tracing::debug!("Mouse jiggler stopped");
        }
    }

    /// Reports an error and notifies listeners
    fn report_error(&self, message: &str) {
        self.set_state(RdpConnectionState::Error);

        with_callback(&self.on_error, |cb| cb(message));
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
// Paint callbacks and direct input API
// ============================================================================

impl EmbeddedRdpWidget {
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

impl crate::embedded_trait::EmbeddedWidget for EmbeddedRdpWidget {
    fn widget(&self) -> &gtk4::Box {
        &self.container
    }

    fn state(&self) -> crate::embedded_trait::EmbeddedConnectionState {
        match *self.state.borrow() {
            RdpConnectionState::Disconnected => {
                crate::embedded_trait::EmbeddedConnectionState::Disconnected
            }
            RdpConnectionState::Connecting => {
                crate::embedded_trait::EmbeddedConnectionState::Connecting
            }
            RdpConnectionState::Connected => {
                crate::embedded_trait::EmbeddedConnectionState::Connected
            }
            RdpConnectionState::Error => crate::embedded_trait::EmbeddedConnectionState::Error,
        }
    }

    fn is_embedded(&self) -> bool {
        *self.is_embedded.borrow()
    }

    fn disconnect(&self) -> Result<(), crate::embedded_trait::EmbeddedError> {
        // Call the existing disconnect method (returns ())
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
        "RDP"
    }
}

// Tests moved to types.rs, buffer.rs, and launcher.rs submodules
