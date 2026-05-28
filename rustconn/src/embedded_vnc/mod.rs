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

// cast_possible_truncation, cast_precision_loss allowed at workspace level
#![allow(
    clippy::cast_sign_loss,
    reason = "module-wide override for legacy code; refactored case by case"
)]
#![allow(
    clippy::missing_panics_doc,
    reason = "module-wide override for legacy code; refactored case by case"
)]

// Re-export types for external use
pub use crate::embedded_vnc_types::{
    EmbeddedVncError, ErrorCallback, FrameCallback, STANDARD_RESOLUTIONS, StateCallback, VncConfig,
    VncConnectionState, VncPixelBuffer, VncWaylandSurface, find_best_standard_resolution,
};

mod ui;
pub use ui::{gtk_button_to_vnc_mask, transform_widget_to_vnc};

use crate::i18n::i18n;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Button, DrawingArea, Label};
use rustconn_core::vnc_client::is_embedded_vnc_available;
#[cfg(feature = "vnc-embedded")]
use rustconn_core::vnc_client::{
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
#[allow(
    dead_code,
    reason = "Many fields kept for GTK widget lifecycle and signal handlers"
)]
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
    /// Persistent Cairo-backed pixel buffer for zero-copy rendering
    cairo_buffer: Rc<RefCell<crate::cairo_buffer::CairoBackedBuffer>>,
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
    /// Reconnect banner (shown when disconnected, at bottom of container)
    reconnect_banner: GtkBox,
    /// Reconnect button inside the banner
    reconnect_button: Button,
    /// Native VNC client (when vnc-embedded feature is enabled)
    #[cfg(feature = "vnc-embedded")]
    vnc_client: Rc<RefCell<Option<Arc<StdMutex<VncClient>>>>>,
    /// Command sender for the VNC client (when vnc-embedded feature is enabled)
    #[cfg(feature = "vnc-embedded")]
    command_sender: Rc<RefCell<Option<VncCommandSender>>>,
}

impl EmbeddedVncWidget {
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
        let reconnect_banner = self.reconnect_banner.clone();
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

            // Show/hide reconnect banner at bottom
            reconnect_banner.set_visible(show_reconnect);

            // When disconnected, hide toolbar buttons
            copy_button.set_visible(!show_reconnect);
            paste_button.set_visible(!show_reconnect);
            ctrl_alt_del_button.set_visible(!show_reconnect);
            separator.set_visible(!show_reconnect);

            // Hide toolbar when disconnected (no reconnect button there anymore)
            if show_reconnect {
                toolbar.set_visible(false);
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
                    tracing::warn!(%e, "Embedded VNC failed, falling back to external");
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
            use secrecy::ExposeSecret;
            vnc_config.with_password(password.expose_secret())
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

        // Hide local cursor if configured (avoids double cursor with remote)
        if !config.show_local_cursor {
            self.drawing_area.set_cursor_from_name(Some("none"));
        }

        // Clone references for the event polling timer
        let pixel_buffer = self.pixel_buffer.clone();
        let cairo_buffer = self.cairo_buffer.clone();
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
        // Capture config and process for auto-fallback to external viewer
        let fallback_config = self.config.clone();
        let fallback_process = self.process.clone();

        // Set up a GLib timeout to poll for VNC events (~60 FPS)
        glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
            use rustconn_core::vnc_client::VncClientCommand;

            // Check if we're still in embedded mode
            if !*is_embedded.borrow() {
                return glib::ControlFlow::Break;
            }

            // Try to get events from the VNC client
            let client_guard = match client.try_lock() {
                Ok(guard) => guard,
                Err(std::sync::TryLockError::WouldBlock) => {
                    return glib::ControlFlow::Continue; // skip this frame
                }
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    tracing::error!("[EmbeddedVNC] Client mutex poisoned");
                    return glib::ControlFlow::Break;
                }
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
                        *vnc_width_ref.borrow_mut() = width;
                        *vnc_height_ref.borrow_mut() = height;
                        pixel_buffer.borrow_mut().resize(width, height);
                        cairo_buffer.borrow_mut().resize(width, height);
                        drawing_area.queue_draw();
                    }
                    VncClientEvent::FrameUpdate { rect, data } => {
                        let stride = u32::from(rect.width) * 4;
                        pixel_buffer.borrow_mut().update_region(
                            u32::from(rect.x),
                            u32::from(rect.y),
                            u32::from(rect.width),
                            u32::from(rect.height),
                            &data,
                            stride,
                        );
                        cairo_buffer.borrow_mut().update_region(
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
                        } else if msg.contains("Unsupported security type")
                            || msg.contains("Unknown VNC security type")
                            || msg.contains("unknown security type")
                        {
                            // Unsupported security type (e.g. RSA-AES type 129)
                            // Auto-fallback to external VNC viewer which may support it
                            tracing::warn!(
                                "[EmbeddedVNC] {msg} — attempting fallback to external viewer"
                            );
                            *is_embedded.borrow_mut() = false;

                            let no_support_msg =
                                i18n("VNC encryption not supported. Install TigerVNC.");

                            // Try to launch external viewer with stored config
                            let fallback_ok = fallback_config
                                .borrow()
                                .as_ref()
                                .and_then(|cfg| {
                                    let viewer = Self::detect_vnc_viewer()?;
                                    let server = if cfg.port == 5900 {
                                        format!("{}:0", cfg.host)
                                    } else if cfg.port > 5900 && cfg.port < 6000 {
                                        let display = cfg.port - 5900;
                                        format!("{}:{display}", cfg.host)
                                    } else {
                                        format!("{}::{}", cfg.host, cfg.port)
                                    };
                                    Some((viewer, server))
                                })
                                .and_then(|(viewer, server)| {
                                    match Command::new(&viewer).arg(&server).spawn() {
                                        Ok(child) => {
                                            tracing::info!(
                                                viewer = %viewer,
                                                "[EmbeddedVNC] Fallback to external viewer"
                                            );
                                            *fallback_process.borrow_mut() = Some(child);
                                            Some(())
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                %e,
                                                "[EmbeddedVNC] External viewer fallback failed"
                                            );
                                            None
                                        }
                                    }
                                });

                            if fallback_ok.is_some() {
                                *state.borrow_mut() = VncConnectionState::Connected;
                                if let Some(ref cb) = *on_state_changed.borrow() {
                                    cb(VncConnectionState::Connected);
                                }
                                if let Some(ref cb) = *on_error.borrow() {
                                    cb(&i18n("Using external viewer (unsupported encryption)"));
                                }
                            } else {
                                *state.borrow_mut() = VncConnectionState::Error;
                                toolbar.set_visible(false);
                                if let Some(ref cb) = *on_error.borrow() {
                                    cb(&no_support_msg);
                                }
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
        if let Some(client) = self.vnc_client.borrow_mut().take()
            && let Ok(mut client_guard) = client.lock()
        {
            client_guard.disconnect();
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
            use rustconn_core::vnc_client::VncClientCommand;
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
        {
            use secrecy::ExposeSecret;
            assert_eq!(
                config
                    .password
                    .as_ref()
                    .map(|p| p.expose_secret().to_string()),
                Some("secret".to_string())
            );
        }
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
