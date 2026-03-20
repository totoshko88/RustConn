//! Connection management for the embedded RDP widget
//!
//! Contains connect, disconnect, reconnect, and connection status methods
//! including IronRDP native client integration and FreeRDP fallback.

use gtk4::glib;
use gtk4::prelude::*;
use secrecy::ExposeSecret;
use std::cell::RefCell;
use std::rc::Rc;

use crate::i18n::{i18n, i18n_f};

use super::launcher::SafeFreeRdpLauncher;
use super::thread::FreeRdpThread;
use super::types::{
    EmbeddedRdpError, FreeRdpThreadState, RdpCommand, RdpConfig, RdpConnectionState, RdpEvent,
};

#[cfg(feature = "rdp-embedded")]
use rustconn_core::rdp_client::RdpClientCommand;

impl super::EmbeddedRdpWidget {
    /// Detects if wlfreerdp is available for embedded mode
    #[must_use]
    pub fn detect_wlfreerdp() -> bool {
        crate::embedded_rdp::detect::detect_wlfreerdp()
    }

    /// Detects if xfreerdp is available for external mode
    #[must_use]
    pub fn detect_xfreerdp() -> Option<String> {
        crate::embedded_rdp::detect::detect_xfreerdp()
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
        tracing::debug!(
            protocol = "rdp",
            widget_id = self.widget_id,
            generation = *self.connection_generation.borrow(),
            "connect() called"
        );

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
                    if let Some(ref thread) = *self.freerdp_thread.borrow()
                        && thread.fallback_triggered()
                    {
                        // Fallback was triggered, clean up and try external mode
                        self.cleanup_embedded_mode();
                        return self.connect_external_with_notification(config);
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
        crate::embedded_rdp::detect::is_ironrdp_available()
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
    pub(super) fn connect_ironrdp(&self, config: &RdpConfig) -> Result<(), EmbeddedRdpError> {
        use rustconn_core::rdp_client::{RdpClient, RdpClientConfig};

        // IronRDP 0.14 does not support RD Gateway (MS-TSGU). If gateway is
        // configured, bail out early so the caller falls back to external
        // xfreerdp which does support gateway connections.
        if config
            .gateway_hostname
            .as_ref()
            .is_some_and(|h| !h.is_empty())
        {
            tracing::warn!(
                protocol = "rdp",
                host = %config.host,
                gateway = ?config.gateway_hostname,
                "RD Gateway configured — IronRDP does not support gateway yet, \
                 falling back to external client"
            );
            return Err(EmbeddedRdpError::GatewayNotSupported);
        }

        // Increment connection generation to invalidate any stale polling loops
        let generation = {
            let mut counter = self.connection_generation.borrow_mut();
            *counter += 1;
            *counter
        };
        tracing::debug!(
            protocol = "rdp",
            generation,
            "Starting connection generation"
        );

        // Get actual widget size for initial resolution
        // This ensures the RDP session matches the current window size
        // Use scale override from config, falling back to system scale_factor
        let effective_scale = config
            .scale_override
            .effective_scale(self.drawing_area.scale_factor());
        let (actual_width, actual_height) = {
            let w = self.drawing_area.width();
            let h = self.drawing_area.height();
            if w > 100 && h > 100 {
                // Convert CSS pixels to device pixels using effective scale
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let device_w = (f64::from(w.unsigned_abs()) * effective_scale) as u32;
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let device_h = (f64::from(h.unsigned_abs()) * effective_scale) as u32;
                // Round down to multiple of 4 for RDP compatibility
                // Many RDP servers and codecs require dimensions divisible by 4
                let width = (device_w / 4) * 4;
                let height = (device_h / 4) * 4;
                // Clamp to reasonable maximum (8K) and ensure minimum size
                (width.clamp(640, 7680), height.clamp(480, 4320))
            } else {
                // Widget not yet realized, use config values
                (config.width, config.height)
            }
        };

        tracing::debug!(
            protocol = "rdp",
            host = %config.host,
            port = config.port,
            "Attempting IronRDP connection"
        );

        // Compute RDP desktop scale factor as percentage (e.g. 2.0 → 200)
        // This tells the Windows server what DPI scaling to use so UI elements
        // appear at the correct logical size on HiDPI displays.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let rdp_scale_percent = (effective_scale * 100.0) as u32;

        tracing::debug!(
            protocol = "rdp",
            width = actual_width,
            height = actual_height,
            "Using widget-size resolution"
        );
        tracing::debug!(
            protocol = "rdp",
            effective_scale = format_args!("{:.2}", effective_scale),
            desktop_scale_factor = rdp_scale_percent,
            "Scale configuration"
        );
        tracing::debug!(
            protocol = "rdp",
            has_username = config.username.is_some(),
            has_domain = config.domain.is_some(),
            has_password = config.password.is_some(),
            "Credential status"
        );

        // Log shared folders configuration
        if !config.shared_folders.is_empty() {
            tracing::debug!(
                protocol = "rdp",
                folder_count = config.shared_folders.len(),
                "Configuring shared folders via RDPDR"
            );
            for folder in &config.shared_folders {
                tracing::debug!(
                    protocol = "rdp",
                    share_name = %folder.share_name,
                    local_path = %folder.local_path.display(),
                    "Shared folder"
                );
            }
        }

        // Convert EmbeddedSharedFolder to SharedFolder for RdpClientConfig
        let shared_folders: Vec<rustconn_core::rdp_client::SharedFolder> = config
            .shared_folders
            .iter()
            .map(|f| rustconn_core::rdp_client::SharedFolder::new(&f.share_name, &f.local_path))
            .collect();

        // Convert GUI config to RdpClientConfig using actual widget size
        let mut client_config = RdpClientConfig::new(&config.host)
            .with_port(config.port)
            .with_resolution(
                crate::utils::dimension_to_u16(actual_width),
                crate::utils::dimension_to_u16(actual_height),
            )
            .with_clipboard(config.clipboard_enabled)
            .with_shared_folders(shared_folders)
            .with_performance_mode(config.performance_mode)
            .with_color_depth(config.performance_mode.color_depth())
            .with_scale_factor(rdp_scale_percent);

        if let Some(ref username) = config.username {
            client_config = client_config.with_username(username);
        }

        if let Some(ref password) = config.password {
            client_config = client_config.with_password(password.expose_secret());
        }

        if let Some(ref domain) = config.domain {
            client_config = client_config.with_domain(domain);
        }

        // Disable NLA (CredSSP) when credentials are incomplete — CredSSP
        // requires both username and password; empty identity causes
        // "Got empty identity" error. The server will prompt instead.
        if config.username.is_none() || config.password.is_none() {
            tracing::debug!(
                protocol = "rdp",
                has_username = config.username.is_some(),
                has_password = config.password.is_some(),
                "Disabling NLA: credentials incomplete"
            );
            client_config = client_config.with_nla(false);
        }

        if let Some(klid) = config.keyboard_layout {
            client_config = client_config.with_keyboard_layout(klid);
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

        // Hide local cursor if configured (avoids double cursor with remote)
        if !config.show_local_cursor {
            self.drawing_area.set_cursor_from_name(Some("none"));
        }

        // Initialize RDP dimensions from actual widget size (not config)
        *self.rdp_width.borrow_mut() = actual_width;
        *self.rdp_height.borrow_mut() = actual_height;

        // Resize and clear pixel buffer to match actual size
        {
            let mut buffer = self.pixel_buffer.borrow_mut();
            buffer.resize(actual_width, actual_height);
            buffer.clear();
        }

        // Set up event polling for IronRDP
        self.setup_ironrdp_polling(client, generation, effective_scale);

        self.set_state(RdpConnectionState::Connecting);
        Ok(())
    }

    /// Sets up the IronRDP event polling loop
    ///
    /// This is extracted from `connect_ironrdp` to keep the method manageable.
    #[cfg(feature = "rdp-embedded")]
    fn setup_ironrdp_polling(
        &self,
        client: rustconn_core::rdp_client::RdpClient,
        generation: u64,
        effective_scale: f64,
    ) {
        use rustconn_core::rdp_client::{RdpClientCommand, RdpClientEvent};

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
        let connection_generation = self.connection_generation.clone();
        #[cfg(feature = "rdp-audio")]
        let audio_player = self.audio_player.clone();
        let clipboard_handler_id = self.clipboard_handler_id.clone();

        // Flag to suppress clipboard change events when we set the clipboard
        // ourselves (Phase 2 auto-sync), preventing feedback loops.
        let clipboard_sync_suppressed: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));

        // Capture fallback-related state for auto-fallback on protocol errors
        // (e.g. xrdp ServerDemandActive incompatibility — IronRDP issue #139)
        let on_fallback = self.on_fallback.clone();
        let fallback_config = self.config.clone();
        let fallback_process = self.process.clone();

        // Capture effective scale for cursor size correction
        let cursor_scale = effective_scale;

        // Capture local cursor visibility preference
        let show_local_cursor = self
            .config
            .borrow()
            .as_ref()
            .is_none_or(|c| c.show_local_cursor);

        // Store client in a shared reference for the polling closure
        let client = std::rc::Rc::new(std::cell::RefCell::new(Some(client)));
        let client_ref = client.clone();
        let polling_interval = u64::from(
            self.config
                .borrow()
                .as_ref()
                .map_or(16, |c| c.polling_interval_ms),
        );

        glib::timeout_add_local(
            std::time::Duration::from_millis(polling_interval),
            move || {
                if client_ref.borrow().is_none() {
                    return glib::ControlFlow::Break;
                }

                // Check if this polling loop is stale (a newer connection was started)
                if *connection_generation.borrow() != generation {
                    tracing::debug!(
                        protocol = "rdp",
                        generation,
                        "Polling loop is stale, stopping"
                    );
                    // Clean up client without firing callbacks
                    if let Some(mut c) = client_ref.borrow_mut().take() {
                        c.disconnect();
                    }
                    // Clean up clipboard monitor
                    if let Some(handler_id) = clipboard_handler_id.borrow_mut().take() {
                        let display = drawing_area.display();
                        let cb = display.clipboard();
                        cb.disconnect(handler_id);
                    }
                    return glib::ControlFlow::Break;
                }

                // Check if we're still in embedded mode
                if !*is_embedded.borrow() || !*is_ironrdp.borrow() {
                    // Clean up client
                    if let Some(mut c) = client_ref.borrow_mut().take() {
                        c.disconnect();
                    }
                    *ironrdp_tx.borrow_mut() = None;
                    toolbar.set_visible(false);
                    // Clean up clipboard monitor
                    if let Some(handler_id) = clipboard_handler_id.borrow_mut().take() {
                        let display = drawing_area.display();
                        let cb = display.clipboard();
                        cb.disconnect(handler_id);
                    }
                    return glib::ControlFlow::Break;
                }

                // Track if we need to redraw
                let mut needs_redraw = false;
                let mut should_break = false;
                // Deferred error message — handle_ironrdp_error needs
                // client_ref.borrow_mut() which conflicts with the immutable
                // borrow held by the event polling loop (#57)
                let mut deferred_error: Option<String> = None;

                // Poll for events from IronRDP client
                if let Some(ref client) = *client_ref.borrow() {
                    while let Some(event) = client.try_recv_event() {
                        match event {
                            RdpClientEvent::Connected { width, height } => {
                                tracing::debug!(
                                    protocol = "rdp",
                                    width,
                                    height,
                                    "IronRDP connected"
                                );
                                *state.borrow_mut() = RdpConnectionState::Connected;

                                // Use server's resolution for the buffer
                                let server_w = u32::from(width);
                                let server_h = u32::from(height);
                                *rdp_width_ref.borrow_mut() = server_w;
                                *rdp_height_ref.borrow_mut() = server_h;
                                {
                                    let mut buffer = pixel_buffer.borrow_mut();
                                    buffer.resize(server_w, server_h);
                                    buffer.clear();
                                }

                                // Phase 3: Monitor local clipboard changes and
                                // announce to server via cliprdr
                                {
                                    let display = drawing_area.display();
                                    let clipboard = display.clipboard();
                                    let tx = ironrdp_tx.clone();
                                    let suppressed = clipboard_sync_suppressed.clone();
                                    let handler_id = clipboard.connect_changed(move |cb| {
                                        // Skip if this change was triggered by our own
                                        // server→client sync (Phase 2)
                                        if *suppressed.borrow() {
                                            return;
                                        }
                                        tracing::debug!(
                                            "[Clipboard] Local clipboard changed, \
                                             announcing to server"
                                        );
                                        // Read local clipboard text and send to server
                                        let tx_inner = tx.clone();
                                        cb.read_text_async(
                                            None::<&gtk4::gio::Cancellable>,
                                            move |result| {
                                                if let Ok(Some(text)) = result
                                                    && let Some(ref sender) = *tx_inner.borrow()
                                                {
                                                    let _ = sender.send(
                                                        RdpClientCommand::ClipboardText(
                                                            text.to_string(),
                                                        ),
                                                    );
                                                    tracing::debug!(
                                                        chars = text.len(),
                                                        "[Clipboard] Sent local clipboard \
                                                         to server"
                                                    );
                                                }
                                            },
                                        );
                                    });
                                    *clipboard_handler_id.borrow_mut() = Some(handler_id);
                                }

                                if let Some(ref callback) = *on_state_changed.borrow() {
                                    callback(RdpConnectionState::Connected);
                                }
                                needs_redraw = true;
                            }
                            RdpClientEvent::Disconnected => {
                                tracing::debug!(protocol = "rdp", generation, "Disconnected event");
                                // Clean up clipboard monitor
                                if let Some(handler_id) = clipboard_handler_id.borrow_mut().take() {
                                    let display = drawing_area.display();
                                    let cb = display.clipboard();
                                    cb.disconnect(handler_id);
                                }
                                // Check if this polling loop is still current before firing callback
                                if *connection_generation.borrow() == generation {
                                    *state.borrow_mut() = RdpConnectionState::Disconnected;
                                    toolbar.set_visible(false);
                                    if let Some(ref callback) = *on_state_changed.borrow() {
                                        callback(RdpConnectionState::Disconnected);
                                    }
                                    needs_redraw = true;
                                    should_break = true;
                                } else {
                                    tracing::debug!(
                                        protocol = "rdp",
                                        generation,
                                        "Ignoring Disconnected from stale generation"
                                    );
                                    should_break = true;
                                }
                            }
                            RdpClientEvent::Error(msg) => {
                                // Defer error handling — handle_ironrdp_error calls
                                // client_ref.borrow_mut().take() which would panic
                                // while client_ref.borrow() is held by this loop
                                deferred_error = Some(msg);
                                needs_redraw = true;
                                should_break = true;
                                break;
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
                                    protocol = "rdp",
                                    width,
                                    height,
                                    "Resolution changed"
                                );
                                *rdp_width_ref.borrow_mut() = u32::from(width);
                                *rdp_height_ref.borrow_mut() = u32::from(height);
                                {
                                    let mut buffer = pixel_buffer.borrow_mut();
                                    buffer.resize(u32::from(width), u32::from(height));
                                    for chunk in buffer.data_mut().chunks_exact_mut(4) {
                                        chunk[0] = 0x1E; // B
                                        chunk[1] = 0x1E; // G
                                        chunk[2] = 0x1E; // R
                                        chunk[3] = 0xFF; // A
                                    }
                                    buffer.set_has_data(true);
                                }
                                needs_redraw = true;
                            }
                            RdpClientEvent::AuthRequired => {
                                tracing::debug!(protocol = "rdp", "Authentication required");
                            }
                            RdpClientEvent::ClipboardText(text) => {
                                // Server sent clipboard text - store it, enable Copy button,
                                // and auto-sync to local GTK clipboard
                                tracing::debug!(
                                    protocol = "rdp",
                                    "Received clipboard text from server"
                                );
                                *remote_clipboard_text.borrow_mut() = Some(text.clone());
                                copy_button.set_sensitive(true);
                                copy_button.set_tooltip_text(Some(&i18n(
                                    "Copy remote clipboard to local",
                                )));

                                // Phase 2: Auto-sync server clipboard to local GTK clipboard
                                *clipboard_sync_suppressed.borrow_mut() = true;
                                let display = drawing_area.display();
                                let clipboard = display.clipboard();
                                clipboard.set_text(&text);
                                let suppressed = clipboard_sync_suppressed.clone();
                                glib::timeout_add_local_once(
                                    std::time::Duration::from_millis(100),
                                    move || {
                                        *suppressed.borrow_mut() = false;
                                    },
                                );
                                tracing::debug!(
                                    chars = text.len(),
                                    "[Clipboard] Auto-synced server text to local clipboard"
                                );
                            }
                            RdpClientEvent::ClipboardFormatsAvailable(formats) => {
                                tracing::debug!(
                                    protocol = "rdp",
                                    format_count = formats.len(),
                                    "Clipboard formats available"
                                );
                                *remote_clipboard_formats.borrow_mut() = formats;
                            }
                            RdpClientEvent::ClipboardInitiateCopy(formats) => {
                                if let Some(ref sender) = *ironrdp_tx.borrow() {
                                    let _ = sender.send(RdpClientCommand::ClipboardCopy(formats));
                                }
                            }
                            RdpClientEvent::ClipboardDataRequest(format) => {
                                tracing::debug!(
                                    format_id = format.id,
                                    "Server requests clipboard data"
                                );
                                let display = drawing_area.display();
                                let clipboard = display.clipboard();
                                let tx = ironrdp_tx.clone();
                                let format_id = format.id;

                                clipboard.read_text_async(
                                    None::<&gtk4::gio::Cancellable>,
                                    move |result| {
                                        if let Ok(Some(text)) = result {
                                            tracing::debug!(
                                                chars = text.len(),
                                                "Sending clipboard text to server"
                                            );
                                            if let Some(ref sender) = *tx.borrow() {
                                                if format_id == 13 {
                                                    // CF_UNICODETEXT
                                                    let data: Vec<u8> = text
                                                        .encode_utf16()
                                                        .flat_map(u16::to_le_bytes)
                                                        .chain([0, 0])
                                                        .collect();
                                                    let _ = sender.send(
                                                        RdpClientCommand::ClipboardData {
                                                            format_id,
                                                            data,
                                                        },
                                                    );
                                                } else {
                                                    let mut data = text.as_bytes().to_vec();
                                                    data.push(0);
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
                                if let Some(ref sender) = *ironrdp_tx.borrow() {
                                    let _ = sender.send(RdpClientCommand::RequestClipboardData {
                                        format_id: format.id,
                                    });
                                }
                            }
                            RdpClientEvent::CursorDefault => {
                                if show_local_cursor {
                                    drawing_area.set_cursor_from_name(Some("default"));
                                }
                                // When show_local_cursor is false, keep cursor hidden
                                // (server bitmap cursor from CursorUpdate is still shown)
                            }
                            RdpClientEvent::CursorHidden => {
                                drawing_area.set_cursor_from_name(Some("none"));
                            }
                            RdpClientEvent::CursorPosition { .. } => {
                                // Server-side cursor position update - handled client-side
                            }
                            RdpClientEvent::CursorUpdate {
                                hotspot_x,
                                hotspot_y,
                                width,
                                height,
                                data,
                            } => {
                                Self::handle_cursor_update(
                                    &drawing_area,
                                    cursor_scale,
                                    hotspot_x,
                                    hotspot_y,
                                    width,
                                    height,
                                    &data,
                                );
                            }
                            RdpClientEvent::ServerMessage(msg) => {
                                tracing::debug!(protocol = "rdp", message = %msg, "Server message");
                            }
                            #[cfg(feature = "rdp-audio")]
                            RdpClientEvent::AudioFormatChanged(format) => {
                                tracing::debug!(
                                    protocol = "rdp",
                                    sample_rate = format.samples_per_sec,
                                    channels = format.channels,
                                    "Audio format changed"
                                );
                                if let Ok(mut player_opt) = audio_player.try_borrow_mut() {
                                    if player_opt.is_none() {
                                        *player_opt = Some(crate::audio::RdpAudioPlayer::new());
                                    }
                                    if let Some(ref mut player) = *player_opt
                                        && let Err(e) = player.configure(format)
                                    {
                                        tracing::warn!(protocol = "rdp", error = %e, "Audio configure failed");
                                    }
                                }
                            }
                            #[cfg(feature = "rdp-audio")]
                            RdpClientEvent::AudioData { data, .. } => {
                                if let Ok(player_opt) = audio_player.try_borrow()
                                    && let Some(ref player) = *player_opt
                                {
                                    player.queue_data(&data);
                                }
                            }
                            #[cfg(feature = "rdp-audio")]
                            RdpClientEvent::AudioVolume { left, right } => {
                                if let Ok(player_opt) = audio_player.try_borrow()
                                    && let Some(ref player) = *player_opt
                                {
                                    player.set_volume(left, right);
                                }
                            }
                            #[cfg(feature = "rdp-audio")]
                            RdpClientEvent::AudioClose => {
                                tracing::debug!(protocol = "rdp", "Audio channel closed");
                                if let Ok(mut player_opt) = audio_player.try_borrow_mut()
                                    && let Some(ref mut player) = *player_opt
                                {
                                    player.stop();
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
                                tracing::debug!(
                                    protocol = "rdp",
                                    format_id,
                                    bytes = data.len(),
                                    "Clipboard data ready"
                                );
                                if let Some(ref sender) = *ironrdp_tx.borrow() {
                                    let _ = sender
                                        .send(RdpClientCommand::ClipboardData { format_id, data });
                                }
                            }
                            RdpClientEvent::ClipboardFileList(files) => {
                                tracing::info!(
                                    protocol = "rdp",
                                    file_count = files.len(),
                                    "Clipboard file list received"
                                );
                                for file in &files {
                                    tracing::debug!(
                                        protocol = "rdp",
                                        name = %file.name,
                                        size = file.size,
                                        is_dir = file.is_directory(),
                                        "Clipboard file entry"
                                    );
                                }
                                let file_count = files.len();
                                file_transfer.borrow_mut().set_available_files(files);
                                if file_count > 0 {
                                    save_files_button.set_label(&i18n_f(
                                        "Save {} Files",
                                        &[&file_count.to_string()],
                                    ));
                                    save_files_button.set_tooltip_text(Some(&i18n_f(
                                        "Save {} files from remote clipboard",
                                        &[&file_count.to_string()],
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
                                tracing::debug!(
                                    protocol = "rdp",
                                    stream_id,
                                    bytes = data.len(),
                                    is_last,
                                    "Clipboard file contents"
                                );
                                file_transfer
                                    .borrow_mut()
                                    .append_data(stream_id, &data, is_last);

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
                                        &i18n_f(
                                            "Downloaded {}/{} files",
                                            &[&completed.to_string(), &total.to_string()],
                                        ),
                                    );
                                }

                                if is_last {
                                    match file_transfer.borrow().save_download(stream_id) {
                                        Ok(path) => {
                                            tracing::info!(
                                                protocol = "rdp",
                                                path = %path.display(),
                                                "Saved clipboard file"
                                            );
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                protocol = "rdp",
                                                error = %e,
                                                "Failed to save clipboard file"
                                            );
                                        }
                                    }
                                }

                                if file_transfer.borrow().all_complete() {
                                    let count = file_transfer.borrow().completed_count;
                                    let target = file_transfer
                                        .borrow()
                                        .target_directory
                                        .as_ref()
                                        .map(|p| p.display().to_string())
                                        .unwrap_or_default();

                                    save_files_button.set_sensitive(true);
                                    let file_count = file_transfer.borrow().available_files.len();
                                    save_files_button.set_label(&i18n_f(
                                        "Save {} Files",
                                        &[&file_count.to_string()],
                                    ));

                                    status_label
                                        .set_text(&i18n_f("Saved {} files", &[&count.to_string()]));
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
                                tracing::debug!(
                                    protocol = "rdp",
                                    stream_id,
                                    size,
                                    "Clipboard file size"
                                );
                                file_transfer.borrow_mut().update_size(stream_id, size);
                            }
                        }
                    }
                }

                // Only redraw once after processing all events
                if needs_redraw {
                    drawing_area.queue_draw();
                }

                // Handle deferred error AFTER the client_ref.borrow() is dropped,
                // so handle_ironrdp_error can safely call client_ref.borrow_mut()
                if let Some(ref error_msg) = deferred_error {
                    Self::handle_ironrdp_error(
                        error_msg,
                        &state,
                        &drawing_area,
                        &toolbar,
                        &on_state_changed,
                        &on_error,
                        &on_fallback,
                        &is_embedded,
                        &is_ironrdp,
                        &ironrdp_tx,
                        &client_ref,
                        &fallback_config,
                        &fallback_process,
                        &clipboard_handler_id,
                    );
                }

                if should_break {
                    return glib::ControlFlow::Break;
                }

                glib::ControlFlow::Continue
            },
        );
    }

    /// Handles IronRDP protocol errors with auto-fallback to FreeRDP
    #[cfg(feature = "rdp-embedded")]
    #[allow(clippy::too_many_arguments)]
    fn handle_ironrdp_error(
        msg: &str,
        state: &Rc<RefCell<RdpConnectionState>>,
        drawing_area: &gtk4::DrawingArea,
        toolbar: &gtk4::Box,
        on_state_changed: &Rc<RefCell<Option<super::types::StateCallback>>>,
        on_error: &Rc<RefCell<Option<super::types::ErrorCallback>>>,
        on_fallback: &Rc<RefCell<Option<super::types::FallbackCallback>>>,
        is_embedded: &Rc<RefCell<bool>>,
        is_ironrdp: &Rc<RefCell<bool>>,
        ironrdp_tx: &Rc<RefCell<Option<std::sync::mpsc::Sender<RdpClientCommand>>>>,
        client_ref: &Rc<RefCell<Option<rustconn_core::rdp_client::RdpClient>>>,
        fallback_config: &Rc<RefCell<Option<RdpConfig>>>,
        fallback_process: &Rc<RefCell<Option<std::process::Child>>>,
        clipboard_handler_id: &Rc<RefCell<Option<glib::SignalHandlerId>>>,
    ) {
        tracing::error!(
            protocol = "rdp",
            error = %msg,
            "[IronRDP] Protocol error during session"
        );

        // Clean up clipboard monitor on any error
        if let Some(handler_id) = clipboard_handler_id.borrow_mut().take() {
            let display = drawing_area.display();
            let cb = display.clipboard();
            cb.disconnect(handler_id);
        }

        // Detect protocol-level errors that indicate server incompatibility
        let is_protocol_error = msg.contains("ServerDemandActive")
            || msg.contains("connect_finalize")
            || msg.contains("unexpected")
            || msg.contains("Unsupported")
            || msg.contains("negotiation");

        if is_protocol_error {
            tracing::warn!(
                protocol = "rdp",
                error = %msg,
                "[IronRDP] Protocol incompatibility — attempting fallback to FreeRDP"
            );

            // Clean up IronRDP state
            *is_embedded.borrow_mut() = false;
            *is_ironrdp.borrow_mut() = false;
            *ironrdp_tx.borrow_mut() = None;
            toolbar.set_visible(false);

            // Disconnect the IronRDP client
            if let Some(mut c) = client_ref.borrow_mut().take() {
                c.disconnect();
            }

            // Attempt FreeRDP external fallback via SafeFreeRdpLauncher
            // (uses /from-stdin to avoid exposing password in /proc/PID/cmdline)
            let fallback_ok = fallback_config.borrow().as_ref().cloned().and_then(|cfg| {
                let launcher = SafeFreeRdpLauncher::new();
                match launcher.launch(&cfg) {
                    Ok(child) => {
                        tracing::info!(
                            protocol = "rdp",
                            host = %cfg.host,
                            port = %cfg.port,
                            "[IronRDP] Fallback to external FreeRDP"
                        );
                        *fallback_process.borrow_mut() = Some(child);
                        Some(())
                    }
                    Err(e) => {
                        tracing::error!(
                            protocol = "rdp",
                            error = %e,
                            host = %cfg.host,
                            "[IronRDP] External FreeRDP fallback failed"
                        );
                        None
                    }
                }
            });

            if fallback_ok.is_some() {
                *state.borrow_mut() = RdpConnectionState::Connected;
                if let Some(ref cb) = *on_state_changed.borrow() {
                    cb(RdpConnectionState::Connected);
                }
                let fb_cb = on_fallback.borrow_mut().take();
                if let Some(cb) = fb_cb {
                    cb(&i18n("Using external RDP client (server incompatible)"));
                    *on_fallback.borrow_mut() = Some(cb);
                }
            } else {
                *state.borrow_mut() = RdpConnectionState::Error;
                if let Some(ref cb) = *on_error.borrow() {
                    cb(&i18n("RDP server incompatible. Install FreeRDP."));
                }
            }
        } else {
            // Non-protocol error — report normally
            *state.borrow_mut() = RdpConnectionState::Error;
            toolbar.set_visible(false);
            if let Some(ref callback) = *on_error.borrow() {
                callback(msg);
            }
        }
    }

    /// Handles cursor update events from IronRDP, with HiDPI downscaling
    #[cfg(feature = "rdp-embedded")]
    fn handle_cursor_update(
        drawing_area: &gtk4::DrawingArea,
        cursor_scale: f64,
        hotspot_x: u16,
        hotspot_y: u16,
        width: u16,
        height: u16,
        data: &[u8],
    ) {
        use gtk4::gdk;

        let scale = cursor_scale;
        if scale > 1.01 {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let logical_w = (f64::from(width) / scale).round().max(1.0) as u16;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let logical_h = (f64::from(height) / scale).round().max(1.0) as u16;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let hotspot_logical_x = (f64::from(hotspot_x) / scale).round() as i32;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let hotspot_logical_y = (f64::from(hotspot_y) / scale).round() as i32;

            // Nearest-neighbor downscale (BGRA, 4 bytes per pixel)
            let src_w = usize::from(width);
            let dst_w = usize::from(logical_w);
            let dst_h = usize::from(logical_h);
            let mut scaled = vec![0u8; dst_w * dst_h * 4];
            for dy in 0..dst_h {
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let sy = (dy as f64 * scale) as usize;
                for dx in 0..dst_w {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let sx = (dx as f64 * scale) as usize;
                    let src_off = (sy * src_w + sx) * 4;
                    let dst_off = (dy * dst_w + dx) * 4;
                    if src_off + 4 <= data.len() {
                        scaled[dst_off..dst_off + 4].copy_from_slice(&data[src_off..src_off + 4]);
                    }
                }
            }

            let bytes = glib::Bytes::from(&scaled);
            let texture = gdk::MemoryTexture::new(
                i32::from(logical_w),
                i32::from(logical_h),
                gdk::MemoryFormat::B8g8r8a8,
                &bytes,
                usize::from(logical_w) * 4,
            );
            let cursor =
                gdk::Cursor::from_texture(&texture, hotspot_logical_x, hotspot_logical_y, None);
            drawing_area.set_cursor(Some(&cursor));
        } else {
            // No scaling needed (1x display)
            let bytes = glib::Bytes::from(data);
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
    }

    /// Fallback when rdp-embedded feature is not enabled
    #[cfg(not(feature = "rdp-embedded"))]
    pub(super) fn connect_ironrdp(&self, _config: &RdpConfig) -> Result<(), EmbeddedRdpError> {
        Err(EmbeddedRdpError::FallbackToExternal(
            "IronRDP not available (rdp-embedded feature not enabled)".to_string(),
        ))
    }

    /// Cleans up embedded mode resources
    pub(super) fn cleanup_embedded_mode(&self) {
        if let Some(handler_id) = self.resize_handler_id.borrow_mut().take() {
            self.drawing_area.disconnect(handler_id);
        }
        #[cfg(feature = "rdp-embedded")]
        if let Some(handler_id) = self.clipboard_handler_id.borrow_mut().take() {
            let display = self.drawing_area.display();
            let clipboard = display.clipboard();
            clipboard.disconnect(handler_id);
            tracing::debug!(protocol = "rdp", "Disconnected local clipboard monitor");
        }
        if let Some(mut thread) = self.freerdp_thread.borrow_mut().take() {
            thread.shutdown();
        }
        self.wl_surface.borrow_mut().cleanup();
        *self.is_embedded.borrow_mut() = false;
    }

    /// Connects using external mode with user notification (Requirement 6.4)
    pub(super) fn connect_external_with_notification(
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
            protocol = "rdp",
            host = %config.host,
            port = config.port,
            "Attempting embedded FreeRDP connection"
        );

        // Initialize Wayland surface
        self.wl_surface
            .borrow_mut()
            .initialize()
            .map_err(|e| EmbeddedRdpError::SubsurfaceCreation(e.to_string()))?;

        // Spawn FreeRDP in a dedicated thread to isolate Qt/GTK conflicts (Requirement 6.3)
        let freerdp_thread = FreeRdpThread::spawn(config)?;

        // Send connect command to the thread
        freerdp_thread.send_command(RdpCommand::Connect(Box::new(config.clone())))?;

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
                            tracing::debug!(protocol = "rdp", "FreeRDP connected");
                            *state.borrow_mut() = RdpConnectionState::Connected;
                            if let Some(ref callback) = *on_state_changed.borrow() {
                                callback(RdpConnectionState::Connected);
                            }
                            drawing_area.queue_draw();
                        }
                        RdpEvent::Disconnected => {
                            tracing::debug!(protocol = "rdp", "FreeRDP disconnected");
                            *state.borrow_mut() = RdpConnectionState::Disconnected;
                            if let Some(ref callback) = *on_state_changed.borrow() {
                                callback(RdpConnectionState::Disconnected);
                            }
                            drawing_area.queue_draw();
                            return glib::ControlFlow::Break;
                        }
                        RdpEvent::Error(msg) => {
                            tracing::error!(protocol = "rdp", error = %msg, "FreeRDP error");
                            *state.borrow_mut() = RdpConnectionState::Error;
                            if let Some(ref callback) = *on_error.borrow() {
                                callback(&msg);
                            }
                            drawing_area.queue_draw();
                            return glib::ControlFlow::Break;
                        }
                        RdpEvent::FallbackTriggered(reason) => {
                            tracing::warn!(protocol = "rdp", reason = %reason, "Fallback triggered");
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
                            if width > 0 && height > 0 {
                                let current_w = *rdp_width_ref.borrow();
                                let current_h = *rdp_height_ref.borrow();
                                if width != current_w || height != current_h {
                                    tracing::debug!(
                                        protocol = "rdp",
                                        width,
                                        height,
                                        "FreeRDP resolution changed"
                                    );
                                    *rdp_width_ref.borrow_mut() = width;
                                    *rdp_height_ref.borrow_mut() = height;
                                    pixel_buffer.borrow_mut().resize(width, height);
                                }
                            }
                            drawing_area.queue_draw();
                            let _ = (x, y); // Suppress unused warnings
                        }
                        RdpEvent::AuthRequired => {
                            tracing::debug!(protocol = "rdp", "FreeRDP authentication required");
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
                let msg = if e.to_string().contains("not found")
                    || e.to_string().contains("No such file")
                {
                    "RDP connection failed. Install FreeRDP 3.x (xfreerdp3 or wlfreerdp3) for external mode.".to_string()
                } else {
                    format!("Failed to start FreeRDP: {e}")
                };
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
        // Increment connection generation to invalidate any active polling loops
        *self.connection_generation.borrow_mut() += 1;

        // Disconnect resize signal handler
        if let Some(handler_id) = self.resize_handler_id.borrow_mut().take() {
            self.drawing_area.disconnect(handler_id);
        }

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

    /// Reconnects with a new resolution
    ///
    /// This method disconnects and reconnects with the specified resolution.
    /// Used when Display Control is not available for dynamic resize.
    ///
    /// # Errors
    ///
    /// Returns an error if no previous configuration exists or if
    /// the connection fails.
    pub fn reconnect_with_resolution(
        &self,
        width: u32,
        height: u32,
    ) -> Result<(), EmbeddedRdpError> {
        let config = self.config.borrow().clone();
        if let Some(mut config) = config {
            tracing::info!(
                protocol = "rdp",
                width,
                height,
                "Reconnecting with new resolution"
            );
            config = config.with_resolution(width, height);
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
        if *self.is_embedded.borrow()
            && let Some(ref thread) = *self.freerdp_thread.borrow()
        {
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
