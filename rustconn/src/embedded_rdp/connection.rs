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

use super::launcher::{SafeFreeRdpLauncher, StderrLines};
use super::thread::FreeRdpThread;
use super::types::{
    EmbeddedRdpError, FreeRdpThreadState, RdpCommand, RdpConfig, RdpConnectionState, RdpEvent,
};

#[cfg(feature = "rdp-embedded")]
use rustconn_core::rdp_client::RdpClientCommand;

/// Classifies FreeRDP stderr output into a user-friendly error message.
///
/// Scans accumulated stderr lines for known FreeRDP error patterns and returns
/// an appropriate localized message. Falls back to a generic message when the
/// failure reason is unrecognizable.
fn classify_freerdp_failure(stderr_lines: &StderrLines, status_str: &str) -> String {
    let lines = stderr_lines.lock().unwrap_or_else(|e| e.into_inner());
    let joined = lines.join(" ");

    if joined.contains("ERRCONNECT_LOGON_FAILURE")
        || joined.contains("LOGON_FAILURE")
        || joined.contains("ERRCONNECT_AUTHENTICATION_FAILED")
        || joined.contains("nla_recv_pdu")
            && (joined.contains("STATUS_LOGON_FAILURE") || joined.contains("0x00020014"))
    {
        i18n("Authentication failed: invalid username or password.")
    } else if joined.contains("ERRCONNECT_LOGON_TYPE_NOT_GRANTED") {
        i18n("Access denied: you do not have permission to log on to this server via RDP.")
    } else if joined.contains("ERRCONNECT_ACCOUNT_LOCKED_OUT") {
        i18n("Account is locked out. Wait a few minutes and try again.")
    } else if joined.contains("ERRCONNECT_ACCOUNT_DISABLED") {
        i18n("Account is disabled on the server.")
    } else if joined.contains("ERRCONNECT_PASSWORD_EXPIRED")
        || joined.contains("ERRCONNECT_PASSWORD_MUST_CHANGE")
    {
        i18n("Password expired. Change the password on the server and try again.")
    } else if joined.contains("ERRCONNECT_CONNECT_TRANSPORT_FAILED")
        || joined.contains("connect_failed")
    {
        i18n("Connection failed: server is unreachable. Check the host address and port.")
    } else if joined.contains("Certificate") && joined.contains("denied") {
        i18n("Connection rejected: server certificate was not accepted.")
    } else {
        i18n_f(
            "RDP connection failed ({}). Run with RUST_LOG=debug for details.",
            &[status_str],
        )
    }
}

/// Polls a freshly launched external FreeRDP client for an early exit.
///
/// A real RDP session never terminates within the first few seconds. If the
/// external client exits that quickly it almost always failed to connect
/// (authentication failure, rejected certificate, unsupported codec, or the
/// wrong display backend). Without this watchdog the widget stayed in a
/// phantom `Connected` state while the user only saw a window flash and close.
///
/// Surfaces the exit as an `Error` (with the process status) instead. The real
/// failure reason is captured separately from the client's stderr by
/// [`SafeFreeRdpLauncher::launch`]. (Fixes #177 follow-up: "it closes automatically")
fn arm_external_exit_watchdog(
    process: Rc<RefCell<Option<std::process::Child>>>,
    state: Rc<RefCell<RdpConnectionState>>,
    on_state_changed: Rc<RefCell<Option<super::types::StateCallback>>>,
    on_error: Rc<RefCell<Option<super::types::ErrorCallback>>>,
    drawing_area: gtk4::DrawingArea,
    stderr_lines: StderrLines,
) {
    // Poll every 500 ms for ~3 s. Long enough to catch an immediate auth/cert
    // rejection, short enough not to delay reporting a genuine failure.
    const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
    const MAX_POLLS: u32 = 6;

    let polls = Rc::new(RefCell::new(0u32));
    glib::timeout_add_local(POLL_INTERVAL, move || {
        // Stop once we're no longer in the external-connected state (e.g. the
        // user disconnected, or an error was already reported elsewhere).
        if *state.borrow() != RdpConnectionState::Connected {
            return glib::ControlFlow::Break;
        }

        let exit_status = match process.borrow_mut().as_mut() {
            Some(child) => child.try_wait().ok().flatten(),
            None => return glib::ControlFlow::Break,
        };

        if let Some(status) = exit_status {
            // Reap the dead child so disconnect() doesn't try to wait on it again.
            *process.borrow_mut() = None;
            *state.borrow_mut() = RdpConnectionState::Error;
            drawing_area.queue_draw();

            let status_str = status.to_string();
            tracing::error!(
                protocol = "rdp",
                status = %status_str,
                "[FreeRDP] External client exited shortly after launch — connection failed"
            );

            let msg = classify_freerdp_failure(&stderr_lines, &status_str);

            // take-invoke-restore: the callbacks may close the tab and re-enter
            // these cells, which would otherwise panic with BorrowMutError.
            let scb = on_state_changed.borrow_mut().take();
            if let Some(ref cb) = scb {
                cb(RdpConnectionState::Error);
            }
            *on_state_changed.borrow_mut() = scb;

            let ecb = on_error.borrow_mut().take();
            if let Some(ref cb) = ecb {
                cb(&msg);
            }
            *on_error.borrow_mut() = ecb;

            return glib::ControlFlow::Break;
        }

        let mut count = polls.borrow_mut();
        *count += 1;
        if *count >= MAX_POLLS {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

/// Groups the shared state references needed by `handle_ironrdp_error`.
///
/// Replaces the 13-parameter function signature with a single context struct,
/// improving readability and reducing clippy `too_many_arguments` warnings.
#[cfg(feature = "rdp-embedded")]
pub(super) struct RdpConnectionContext {
    pub state: Rc<RefCell<RdpConnectionState>>,
    pub drawing_area: gtk4::DrawingArea,
    pub toolbar: gtk4::Box,
    pub on_state_changed: Rc<RefCell<Option<super::types::StateCallback>>>,
    pub on_error: Rc<RefCell<Option<super::types::ErrorCallback>>>,
    pub on_fallback: Rc<RefCell<Option<super::types::FallbackCallback>>>,
    pub is_embedded: Rc<RefCell<bool>>,
    pub is_ironrdp: Rc<RefCell<bool>>,
    pub ironrdp_tx: Rc<RefCell<Option<tokio::sync::mpsc::UnboundedSender<RdpClientCommand>>>>,
    pub client_ref: Rc<RefCell<Option<rustconn_core::rdp_client::RdpClient>>>,
    pub fallback_config: Rc<RefCell<Option<RdpConfig>>>,
    pub fallback_process: Rc<RefCell<Option<std::process::Child>>>,
    pub clipboard_handler_id: Rc<RefCell<Option<glib::SignalHandlerId>>>,
    /// Whether we already attempted a retry without GFX pipeline.
    /// Prevents infinite retry loops. (Issue #218)
    pub gfx_retry_attempted: Rc<RefCell<bool>>,
    /// Reconnect callback — triggers a fresh `connect()` with the (now-modified)
    /// stored config. Shared with the reconnect button.
    pub on_reconnect: Rc<RefCell<Option<Box<dyn Fn() + 'static>>>>,
}

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

        // Check if IronRDP embedded mode is available
        // This is determined at compile time via the rdp-embedded feature flag
        if Self::is_ironrdp_available() {
            // Skip IronRDP if security settings require FreeRDP
            // (RDP Security Layer, TLS-only, low TLS security level, or RemoteApp)
            let force_freerdp = config.security_layer.requires_freerdp()
                || config.tls_security_level.is_some_and(|l| l < 2)
                || config
                    .remote_app_program
                    .as_ref()
                    .is_some_and(|p| !p.is_empty());

            if force_freerdp {
                let reason = format!(
                    "Security layer {:?} / TLS level {:?} requires FreeRDP \
                     (IronRDP only supports TLS 1.2+)",
                    config.security_layer, config.tls_security_level
                );
                tracing::info!(protocol = "rdp", %reason, "Skipping IronRDP for legacy security");
                self.report_fallback(&reason);
            } else {
                // Try IronRDP embedded mode first
                match self.connect_ironrdp(config) {
                    Ok(()) => {
                        return Ok(());
                    }
                    Err(e) => {
                        // Log the error and fall back to FreeRDP
                        let reason = format!("IronRDP connection failed: {e}");
                        self.report_fallback(&reason);
                        self.cleanup_embedded_mode();
                    }
                }
            }
        } else {
            // IronRDP not available, notify user
            self.report_fallback("Native RDP client not available, using FreeRDP external mode");
        }

        // Try wlfreerdp for embedded-like experience
        // Skip embedded mode for RemoteApp — RAIL requires its own window management
        // which is incompatible with Wayland subsurface embedding.
        let is_remote_app = config
            .remote_app_program
            .as_ref()
            .is_some_and(|p| !p.is_empty());

        // Skip embedded wlfreerdp when an RD Gateway is configured. The embedded
        // thread (see `thread.rs`) does not emit `/g:` gateway arguments, so it
        // would connect straight to the gateway host on 3389 without tunnelling
        // and render a broken session. Only the external launcher
        // (`launcher::add_connection_args`) wires up gateway routing.
        let has_gateway = config
            .gateway_hostname
            .as_ref()
            .is_some_and(|h| !h.is_empty());

        if Self::detect_wlfreerdp() && !is_remote_app && !has_gateway {
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
                    // Log the error and fall back to external mode
                    let reason = format!("Embedded RDP failed: {e}");
                    self.report_fallback(&reason);
                    self.cleanup_embedded_mode();
                }
            }
        }

        // Fall back to external mode (xfreerdp)
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
    #[cfg(feature = "rdp-embedded")]
    pub(super) fn connect_ironrdp(&self, config: &RdpConfig) -> Result<(), EmbeddedRdpError> {
        use rustconn_core::rdp_client::{RdpClient, RdpClientConfig};

        // When rd-gateway feature is disabled and gateway is configured,
        // fall back to external xfreerdp which supports gateway.
        #[cfg(not(feature = "rd-gateway"))]
        if config
            .gateway_hostname
            .as_ref()
            .is_some_and(|h| !h.is_empty())
        {
            tracing::warn!(
                protocol = "rdp",
                host = %config.host,
                gateway = ?config.gateway_hostname,
                "RD Gateway configured but rd-gateway feature disabled — \
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

        // Get actual widget size for initial resolution. The remote desktop is
        // requested at the widget's LOGICAL size (Auto = 1.0×) so we don't push
        // scale-factor-inflated device resolutions over the network; the
        // framebuffer is upscaled locally for HiDPI. Explicit Display Scale
        // values raise the remote resolution for a sharper image.
        let effective_scale = config
            .scale_override
            .resolved_scale(super::widget_fractional_scale(&self.drawing_area));
        // Base DPI scale as a percentage (e.g. 2.0 → 200). With Display Scale =
        // Auto this is 100 (native rendering on the logical-sized desktop).
        #[expect(
            clippy::cast_possible_truncation,
            reason = "RDP scale percent is a small value (100–300) that fits u16"
        )]
        let base_scale_percent = super::rdp_scale_percent(effective_scale) as u16;
        // Resolve the initial resolution AND the DPI scale through the shared
        // core helper, so the connect path matches the resize / Fit-resolution
        // paths. For a small window (logical < 640x480) it requests a >=minimum
        // desktop at a fixed 100% DPI and lets the viewer downscale the frame —
        // so the cursor/UI stay normal-sized and content is dense, instead of a
        // huge 200%-DPI cursor on a cramped ~373x270 logical desktop.
        let (actual_width, actual_height, rdp_scale_percent) = {
            let w = self.drawing_area.width();
            let h = self.drawing_area.height();
            if w > 100 && h > 100 {
                // Convert CSS pixels to device pixels using effective scale
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "value range fits the target type and is non-negative by construction in this code path"
                )]
                let device_w = (f64::from(w.unsigned_abs()) * effective_scale) as u32;
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "value range fits the target type and is non-negative by construction in this code path"
                )]
                let device_h = (f64::from(h.unsigned_abs()) * effective_scale) as u32;
                let req = rustconn_core::display_geometry::desktop_request_for_area(
                    device_w,
                    device_h,
                    640,
                    480,
                    base_scale_percent,
                );
                // Even dimensions + resolution ceiling (see round_rdp_desktop);
                // the helper already guarantees >= 640x480.
                let (width, height) = super::round_rdp_desktop(req.width, req.height);
                (width, height, u32::from(req.scale_percent))
            } else {
                // Widget not yet realized, use config values at the base scale.
                (config.width, config.height, u32::from(base_scale_percent))
            }
        };

        tracing::debug!(
            protocol = "rdp",
            host = %config.host,
            port = config.port,
            "Attempting IronRDP connection"
        );

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
            .with_printer(config.printer_enabled)
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

        // Apply user-selected graphics mode for the embedded IronRDP client.
        // Auto (default) lets IronRDP negotiate GFX/H.264; Legacy/RemoteFx skip
        // the EGFX pipeline entirely. (Issue #218 — user workaround)
        if !matches!(
            config.graphics_mode,
            rustconn_core::rdp_client::graphics::GraphicsMode::Auto
        ) {
            tracing::info!(
                protocol = "rdp",
                graphics_mode = ?config.graphics_mode,
                "User-selected graphics mode applied"
            );
            client_config = client_config.with_graphics_mode(config.graphics_mode);
        }

        // When GFX pipeline previously failed (e.g. decode errors, no first
        // frame), retry with Legacy graphics mode — this skips the EGFX DVC
        // registration entirely and forces RemoteFX/bitmap path. (Issue #218)
        // This overrides the user's graphics_mode for the retry attempt only.
        if config.force_legacy_graphics {
            tracing::info!(
                protocol = "rdp",
                "Retrying with Legacy graphics (GFX pipeline disabled)"
            );
            client_config = client_config
                .with_graphics_mode(rustconn_core::rdp_client::graphics::GraphicsMode::Legacy);
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
        // (may already be visible via show_toolbar() called before measure — idempotent)
        self.toolbar.set_visible(true);

        // Hide local cursor if configured (avoids double cursor with remote)
        if !config.show_local_cursor {
            self.drawing_area.set_cursor_from_name(Some("none"));
        }

        // Initialize RDP dimensions from actual widget size (not config)
        *self.rdp_width.borrow_mut() = actual_width;
        *self.rdp_height.borrow_mut() = actual_height;

        // Resize and clear Cairo-backed buffer to match actual size
        {
            let mut cbuf = self.cairo_buffer.borrow_mut();
            cbuf.resize(actual_width, actual_height);
            cbuf.clear();
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

        /// How long to wait for the first displayable frame after the server
        /// reports the session as connected before falling back to the external
        /// FreeRDP client. Servers that only offer GFX/H.264 (which IronRDP cannot
        /// decode yet) connect successfully but never produce a frame.
        ///
        /// 15 s accommodates Windows Server 2019 with AD auth + login scripts
        /// where the desktop may take 10+ seconds to render the first frame
        /// through the GFX pipeline. (Fixes #177, #218)
        const FIRST_FRAME_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

        let state = self.state.clone();
        let drawing_area = self.drawing_area.clone();
        let toolbar = self.toolbar.clone();
        let on_state_changed = self.on_state_changed.clone();
        let on_error = self.on_error.clone();
        let rdp_width_ref = self.rdp_width.clone();
        let rdp_height_ref = self.rdp_height.clone();
        let cairo_buffer = self.cairo_buffer.clone();
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

        // Use the struct-level suppression flag so both the Copy button handler
        // and the Phase 2 auto-sync can suppress the clipboard-changed callback.
        let clipboard_sync_suppressed = self.clipboard_sync_suppressed.clone();

        // Capture fallback-related state for auto-fallback on protocol errors
        // (e.g. xrdp ServerDemandActive incompatibility — IronRDP issue #139)
        let on_fallback = self.on_fallback.clone();
        let fallback_config = self.config.clone();
        let fallback_process = self.process.clone();

        // Capture reconnect callback and file DnD circuit breaker for event handling
        let on_reconnect = self.on_reconnect.clone();
        let config = self.config.clone();
        let file_dnd_cb = self.file_dnd_circuit_breaker.clone();

        // Initial "snap to settled size". The connect resolution is measured
        // before the permanent session toolbar has laid out, leaving the server
        // desktop a few dozen px too tall for the drawing area — a mismatch
        // below RESIZE_THRESHOLD_PX that the debounced resize handler ignores,
        // so the first frame is softly rescaled. This corrects it once over
        // Display Control (MS-RDPEDISP) for a 1:1 map. It reads the live server
        // size and sends at most one SetDesktopSize (guarded by snap_attempted);
        // a no-op when the size already matches (e.g. reconnect). It is fired
        // ONLY by DisplayControlReady — the channel is not ready right after
        // connect, so firing it earlier would fail encode_resize and fall over
        // to a disruptive reconnect. If the server never negotiates Display
        // Control the snap simply never runs and the frame is scaled to fit.
        let snap_attempted = std::rc::Rc::new(std::cell::Cell::new(false));
        let snap_to_settled: std::rc::Rc<dyn Fn()> = {
            let config = config.clone();
            let ironrdp_tx = ironrdp_tx.clone();
            let drawing_area = drawing_area.clone();
            let rdp_width_ref = rdp_width_ref.clone();
            let rdp_height_ref = rdp_height_ref.clone();
            let snap_attempted = snap_attempted.clone();
            std::rc::Rc::new(move || {
                if snap_attempted.get() {
                    return;
                }
                let server_w = *rdp_width_ref.borrow();
                let server_h = *rdp_height_ref.borrow();
                let effective_scale = config.borrow().as_ref().map_or(1.0, |c| {
                    c.scale_override
                        .resolved_scale(super::widget_fractional_scale(&drawing_area))
                });
                let css_w = drawing_area.width().unsigned_abs();
                let css_h = drawing_area.height().unsigned_abs();
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "value range fits the target type and is non-negative by construction in this code path"
                )]
                let dev_w = (f64::from(css_w) * effective_scale) as u32;
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "value range fits the target type and is non-negative by construction in this code path"
                )]
                let dev_h = (f64::from(css_h) * effective_scale) as u32;
                // Adaptive request through the shared core helper (same path as
                // connect / resize / Fit): a small window gets a >=minimum
                // desktop at 100% DPI, downscaled locally (dense, normal cursor).
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "RDP scale percent is a small value (100–300) that fits u16"
                )]
                let snap_base_scale = super::rdp_scale_percent(effective_scale) as u16;
                let snap_req = rustconn_core::display_geometry::desktop_request_for_area(
                    dev_w,
                    dev_h,
                    640,
                    480,
                    snap_base_scale,
                );
                // Even dimensions + ceiling (see round_rdp_desktop).
                let (settled_w, settled_h) =
                    super::round_rdp_desktop(snap_req.width, snap_req.height);
                let settled_scale = u32::from(snap_req.scale_percent);

                // Only when realized, a sane size, and actually different (slack
                // absorbs the ≤1px even-rounding residual).
                if css_w > 100
                    && css_h > 100
                    && settled_w >= 640
                    && settled_h >= 480
                    && (settled_w.abs_diff(server_w) > super::DESKTOP_MATCH_SLACK_PX
                        || settled_h.abs_diff(server_h) > super::DESKTOP_MATCH_SLACK_PX)
                {
                    snap_attempted.set(true);
                    // Keep the stored config in sync
                    let current_config = config.borrow().clone();
                    if let Some(mut c) = current_config {
                        c = c.with_resolution(settled_w, settled_h);
                        *config.borrow_mut() = Some(c);
                    }
                    if let Some(ref sender) = *ironrdp_tx.borrow() {
                        #[expect(
                            clippy::cast_possible_truncation,
                            reason = "RDP resolution is clamped well below u16::MAX in this code path"
                        )]
                        let sw = settled_w as u16;
                        #[expect(
                            clippy::cast_possible_truncation,
                            reason = "RDP resolution is clamped well below u16::MAX in this code path"
                        )]
                        let sh = settled_h as u16;
                        let _ = sender.send(RdpClientCommand::SetDesktopSize {
                            width: sw,
                            height: sh,
                            scale_percent: Some(settled_scale),
                        });
                    }
                    tracing::info!(
                        protocol = "rdp",
                        server_w,
                        server_h,
                        settled_w,
                        settled_h,
                        "[RDP] Snapping desktop to settled drawing-area size (toolbar layout)"
                    );
                }
            })
        };

        // Mouse jiggler handles — armed on Connected here because the embedded
        // connection path sets the state directly and never calls set_state (#185).
        let jiggler = self.jiggler_handles();

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

        // First-frame watchdog state: tracks when the session became connected
        // and whether any real frame has been blitted yet.
        let first_frame_received = std::rc::Rc::new(std::cell::RefCell::new(false));
        let connected_at: std::rc::Rc<std::cell::RefCell<Option<std::time::Instant>>> =
            std::rc::Rc::new(std::cell::RefCell::new(None));

        // GFX retry state: when the GFX pipeline fails (decode errors, no first
        // frame), we retry once with Legacy graphics mode before falling back to
        // external FreeRDP. Avoids the double-connection race on single-session
        // servers where FreeRDP fails with NLA because IronRDP's session hasn't
        // fully torn down yet. (Issue #218)
        let gfx_retry_attempted = std::rc::Rc::new(std::cell::RefCell::new(false));

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
                                // Arm the first-frame watchdog (see FIRST_FRAME_TIMEOUT)
                                *connected_at.borrow_mut() = Some(std::time::Instant::now());

                                // Use server's resolution for the buffer
                                let server_w = u32::from(width);
                                let server_h = u32::from(height);
                                *rdp_width_ref.borrow_mut() = server_w;
                                *rdp_height_ref.borrow_mut() = server_h;
                                {
                                    let mut cbuf = cairo_buffer.borrow_mut();
                                    cbuf.resize(server_w, server_h);
                                    cbuf.clear();
                                }

                                // The initial "snap to settled size" is triggered
                                // only by DisplayControlReady (see snap_to_settled) —
                                // never on a timer. Real servers can take much longer
                                // than a couple of seconds to negotiate the Display
                                // Control channel, and forcing the snap before it is
                                // ready makes encode_resize fail over to a full
                                // reconnect (the visible connect → reconnect flicker,
                                // and connection resets from the churn). If the channel
                                // never becomes ready the desktop simply stays at the
                                // server size and is scaled to fit — a slightly softer
                                // frame, but no reconnect and no dropped session.

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

                                // Arm the mouse jiggler now: embedded mode never
                                // routes a Connected transition through set_state,
                                // so this is the only place it can start (#185).
                                if let Some(interval) = config
                                    .borrow()
                                    .as_ref()
                                    .filter(|c| c.jiggler_enabled)
                                    .map(|c| c.jiggler_interval_secs)
                                {
                                    jiggler.start(interval);
                                }
                                needs_redraw = true;
                            }
                            RdpClientEvent::Disconnected => {
                                tracing::debug!(protocol = "rdp", generation, "Disconnected event");
                                jiggler.stop();
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
                                jiggler.stop();
                                deferred_error = Some(msg);
                                needs_redraw = true;
                                should_break = true;
                                break;
                            }
                            RdpClientEvent::FrameUpdate { rect, data } => {
                                // Update Cairo-backed buffer (zero-copy path)
                                let mut cbuf = cairo_buffer.borrow_mut();
                                cbuf.update_region(
                                    u32::from(rect.x),
                                    u32::from(rect.y),
                                    u32::from(rect.width),
                                    u32::from(rect.height),
                                    &data,
                                    u32::from(rect.width) * 4,
                                );
                                // NOTE: The legacy PixelBuffer is only read by the
                                // FreeRDP fallback path (which populates it via
                                // on_end_paint); on the IronRDP path the Cairo buffer
                                // above is authoritative, so a second per-frame copy
                                // here was pure overhead (~33 MB/frame at 4K).
                                if !*first_frame_received.borrow()
                                    && let Some(t) = *connected_at.borrow()
                                {
                                    tracing::info!(
                                        protocol = "rdp",
                                        elapsed_ms = u64::try_from(t.elapsed().as_millis())
                                            .unwrap_or(u64::MAX),
                                        "[IronRDP] First displayable frame received"
                                    );
                                }
                                *first_frame_received.borrow_mut() = true;
                                needs_redraw = true;
                            }
                            RdpClientEvent::FullFrameUpdate {
                                width,
                                height,
                                data,
                            } => {
                                // Full screen update — resize + blit into Cairo buffer
                                {
                                    let mut cbuf = cairo_buffer.borrow_mut();
                                    if cbuf.width() != u32::from(width)
                                        || cbuf.height() != u32::from(height)
                                    {
                                        cbuf.resize(u32::from(width), u32::from(height));
                                        *rdp_width_ref.borrow_mut() = u32::from(width);
                                        *rdp_height_ref.borrow_mut() = u32::from(height);
                                    }
                                    cbuf.update_region(
                                        0,
                                        0,
                                        u32::from(width),
                                        u32::from(height),
                                        &data,
                                        u32::from(width) * 4,
                                    );
                                }
                                // Legacy PixelBuffer intentionally not updated here —
                                // see the FrameUpdate handler above (IronRDP renders
                                // from the Cairo buffer; PixelBuffer is FreeRDP-only).
                                if !*first_frame_received.borrow()
                                    && let Some(t) = *connected_at.borrow()
                                {
                                    tracing::info!(
                                        protocol = "rdp",
                                        elapsed_ms = u64::try_from(t.elapsed().as_millis())
                                            .unwrap_or(u64::MAX),
                                        "[IronRDP] First displayable frame received"
                                    );
                                }
                                *first_frame_received.borrow_mut() = true;
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
                                    let mut cbuf = cairo_buffer.borrow_mut();
                                    cbuf.resize(u32::from(width), u32::from(height));
                                    cbuf.fill_solid(0x1E, 0x1E, 0x1E, 0xFF);
                                }
                                // The reactivated desktop is repainted by the server
                                // only where content changed, leaving the gray fill
                                // above as a seam on untouched regions. Request a full
                                // repaint so the whole desktop is resent (no-op if the
                                // server ignores the Refresh Rect).
                                if let Some(ref sender) = *ironrdp_tx.borrow() {
                                    let _ = sender.send(RdpClientCommand::RefreshScreen);
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
                                    chars = text.len(),
                                    "Received clipboard text from server"
                                );
                                *remote_clipboard_text.borrow_mut() = Some(text.clone());
                                copy_button.set_sensitive(true);
                                copy_button.set_tooltip_text(Some(&i18n(
                                    "Copy remote clipboard to local",
                                )));

                                // Phase 2: Auto-sync server clipboard to local GTK clipboard.
                                // Use root native surface for reliable Wayland clipboard ownership.
                                *clipboard_sync_suppressed.borrow_mut() = true;
                                let clipboard = if let Some(root) = drawing_area.root()
                                    && let Some(window) = root.downcast_ref::<gtk4::Window>()
                                {
                                    gtk4::prelude::WidgetExt::display(window).clipboard()
                                } else {
                                    drawing_area.display().clipboard()
                                };
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
                            RdpClientEvent::FileContentsRequested { .. } => {
                                // File contents requests are handled directly in the
                                // session thread via handle_file_contents_request().
                                // This event is only emitted for observability; no
                                // GUI action needed.
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
                            RdpClientEvent::DisplayControlReady => {
                                // The Display Control channel is negotiated → run the
                                // initial snap now, so the resize goes over MS-RDPEDISP
                                // (smooth) instead of failing over to a reconnect.
                                let snap: &dyn Fn() = &*snap_to_settled;
                                snap();
                            }
                            RdpClientEvent::DisplayControlUnavailable { width, height } => {
                                // Server does not support Display Control Channel
                                // (e.g. Windows Server without RDPEDISP). The only
                                // way to change resolution is a full reconnect.
                                // We always reconnect here — the "Reconnect on Resize"
                                // toggle controls whether resize.rs sends the initial
                                // SetDesktopSize attempt at all (force path), but once
                                // we already tried dynamic resize and the server said
                                // "no", reconnect is the correct fallback.
                                tracing::info!(
                                    protocol = "rdp",
                                    width,
                                    height,
                                    "Display Control Channel unavailable — \
                                     reconnecting with new resolution"
                                );
                                // Update config with the requested resolution
                                {
                                    let current_config = config.borrow().clone();
                                    if let Some(mut cfg) = current_config {
                                        cfg = cfg
                                            .with_resolution(u32::from(width), u32::from(height));
                                        *config.borrow_mut() = Some(cfg);
                                    }
                                }
                                // Disconnect current session
                                if let Some(ref sender) = *ironrdp_tx.borrow() {
                                    let _ = sender.send(RdpClientCommand::Disconnect);
                                }
                                // Trigger reconnect via callback
                                let reconnect_cb_clone = on_reconnect.clone();
                                glib::timeout_add_local_once(
                                    std::time::Duration::from_millis(500),
                                    move || {
                                        if let Some(ref callback) = *reconnect_cb_clone.borrow() {
                                            callback();
                                        }
                                    },
                                );
                            }
                            RdpClientEvent::Rtt {
                                rtt_ms,
                                active_graphics_mode,
                            } => {
                                // Display RTT and active graphics mode in the status label
                                status_label.set_text(&i18n_f(
                                    "RTT: {} ms | {}",
                                    &[&rtt_ms.to_string(), active_graphics_mode.display_name()],
                                ));
                                status_label.set_visible(true);
                                tracing::debug!(
                                    protocol = "rdp",
                                    rtt_ms,
                                    graphics_mode = active_graphics_mode.display_name(),
                                    "RTT measurement from server Auto-Detect"
                                );
                            }
                            RdpClientEvent::FileClipboardUnsupported => {
                                tracing::info!(
                                    protocol = "rdp",
                                    "Server does not support file clipboard — disabling file DnD"
                                );
                                file_dnd_cb
                                    .borrow_mut()
                                    .disable("Server does not support file clipboard");
                            }
                            RdpClientEvent::GfxDecodeFailure {
                                consecutive_failures,
                            } => {
                                // GFX H.264 pipeline is failing persistently.
                                // This means the server chose a codec we cannot
                                // decode (e.g. AVC444 on a misconfigured OpenH264).
                                // Trigger immediate fallback instead of waiting for
                                // the no-frame-watchdog timeout. (Fixes #218)
                                tracing::warn!(
                                    protocol = "rdp",
                                    consecutive_failures,
                                    "GFX H.264 persistent decode failure — triggering fallback"
                                );
                                if !*first_frame_received.borrow() {
                                    deferred_error = Some(
                                        "no-frame-watchdog: GFX pipeline decode failure \
                                         (server codec incompatible with client)"
                                            .to_string(),
                                    );
                                    should_break = true;
                                    break;
                                }
                            }
                        }
                    }
                }

                // Only redraw once after processing all events
                if needs_redraw {
                    drawing_area.queue_draw();
                }

                // First-frame watchdog: if the server reported the session as
                // Connected but never produced a displayable frame within
                // FIRST_FRAME_TIMEOUT, it almost certainly uses a graphics
                // pipeline IronRDP cannot decode yet (GFX/H.264/AVC444). Inject a
                // synthetic protocol error so handle_ironrdp_error falls back to
                // the external FreeRDP client, which supports those codecs.
                // (Fixes #177 — "connected but desktop not showing".)
                if deferred_error.is_none()
                    && !*first_frame_received.borrow()
                    && let Some(connected_instant) = *connected_at.borrow()
                    && connected_instant.elapsed() >= FIRST_FRAME_TIMEOUT
                {
                    tracing::warn!(
                        protocol = "rdp",
                        timeout_secs = FIRST_FRAME_TIMEOUT.as_secs(),
                        "[IronRDP] Connected but no frame received — falling back to \
                         external client (likely GFX/H.264-only server)"
                    );
                    deferred_error = Some(
                        "no-frame-watchdog: server connected but sent no displayable frames"
                            .to_string(),
                    );
                    should_break = true;
                }

                // Handle deferred error AFTER the client_ref.borrow() is dropped,
                // so handle_ironrdp_error can safely call client_ref.borrow_mut()
                if let Some(ref error_msg) = deferred_error {
                    let ctx = RdpConnectionContext {
                        state: state.clone(),
                        drawing_area: drawing_area.clone(),
                        toolbar: toolbar.clone(),
                        on_state_changed: on_state_changed.clone(),
                        on_error: on_error.clone(),
                        on_fallback: on_fallback.clone(),
                        is_embedded: is_embedded.clone(),
                        is_ironrdp: is_ironrdp.clone(),
                        ironrdp_tx: ironrdp_tx.clone(),
                        client_ref: client_ref.clone(),
                        fallback_config: fallback_config.clone(),
                        fallback_process: fallback_process.clone(),
                        clipboard_handler_id: clipboard_handler_id.clone(),
                        gfx_retry_attempted: gfx_retry_attempted.clone(),
                        on_reconnect: on_reconnect.clone(),
                    };
                    Self::handle_ironrdp_error(error_msg, &ctx);
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
    fn handle_ironrdp_error(msg: &str, ctx: &RdpConnectionContext) {
        tracing::error!(
            protocol = "rdp",
            error = %msg,
            "[IronRDP] Protocol error during session"
        );

        // Clean up clipboard monitor on any error
        if let Some(handler_id) = ctx.clipboard_handler_id.borrow_mut().take() {
            let display = ctx.drawing_area.display();
            let cb = display.clipboard();
            cb.disconnect(handler_id);
        }

        // Detect protocol-level errors that indicate server incompatibility
        // Known issue: IronRDP connector 0.8.0 does not handle ServerDeactivateAll
        // PDU during CapabilitiesExchange phase. GNOME Remote Desktop (grd) and
        // some other servers send this PDU before ServerDemandActive, causing
        // "unexpected Share Control Pdu (expected ServerDemandActive)" error.
        // See: https://github.com/Devolutions/IronRDP — upstream limitation.
        //
        // GNOME Remote Desktop also trips the connector's internal state machine
        // during connect_finalize: NLA/CredSSP succeeds, then the capabilities /
        // finalization phase returns `general_err!("invalid state (this is a bug)")`
        // (ironrdp-connector connection.rs). Our core wraps this as
        // "Connection finalize failed: …". Match both the wrapper prefix and the
        // upstream signature so the connection falls back to FreeRDP instead of
        // surfacing a dead-end error. See https://github.com/totoshko88/RustConn/issues/199.
        //
        // IMPORTANT: CredSSP authentication failures (wrong password, locked
        // account, etc.) also arrive as "Connection finalize failed: … CredSSP"
        // but are NOT protocol incompatibilities — retrying with FreeRDP using
        // the same credentials is pointless. Exclude them from fallback.
        let is_auth_failure = msg.contains("0xc000006d")
            || msg.contains("0xc000006e")
            || msg.contains("0xc000006a")
            || msg.contains("0xc0000070")
            || msg.contains("0xc0000071")
            || msg.contains("0xc0000072")
            || msg.contains("0xc000015b")
            || msg.contains("STATUS_LOGON_FAILURE")
            || msg.contains("STATUS_ACCOUNT_DISABLED")
            || msg.contains("STATUS_ACCOUNT_LOCKED_OUT")
            || msg.contains("STATUS_PASSWORD_EXPIRED");

        let is_protocol_error = !is_auth_failure
            && (msg.contains("ServerDemandActive")
                || msg.contains("ServerDeactivateAll")
                || msg.contains("connect_finalize")
                || msg.contains("Connection finalize failed")
                || msg.contains("invalid state (this is a bug)")
                || msg.contains("unexpected Share Control Pdu")
                || msg.contains("Unsupported PDU")
                || msg.contains("Unsupported security protocol")
                || msg.contains("negotiation failed")
                || msg.contains("NegotiationError")
                || msg.contains("decode error")
                || msg.contains("unsupported fast-path update code")
                // First-frame watchdog: server connected but never sent a decodable
                // frame (GFX/H.264-only). Treated as incompatibility → FreeRDP fallback.
                || msg.contains("no-frame-watchdog"));

        if is_protocol_error {
            // Distinguish GFX-specific errors (decode failure, no-frame-watchdog)
            // from true protocol incompatibilities (ServerDemandActive, NLA negotiation).
            // GFX errors get a retry with Legacy graphics before falling back to FreeRDP.
            let is_gfx_error =
                msg.contains("no-frame-watchdog") || msg.contains("GFX pipeline decode failure");

            let should_retry_without_gfx = is_gfx_error && !*ctx.gfx_retry_attempted.borrow()
                // Skip retry if user already selected a non-GFX graphics mode
                // (Legacy/RemoteFx) — there's nothing to fall back to.
                && ctx
                    .fallback_config
                    .borrow()
                    .as_ref()
                    .is_none_or(|cfg| {
                        matches!(cfg.graphics_mode, rustconn_core::rdp_client::graphics::GraphicsMode::Auto)
                    });

            if should_retry_without_gfx {
                // Mark retry as attempted to prevent infinite loops
                *ctx.gfx_retry_attempted.borrow_mut() = true;

                tracing::info!(
                    protocol = "rdp",
                    error = %msg,
                    "[IronRDP] GFX pipeline failed — retrying with Legacy graphics \
                     (skipping EGFX channel)"
                );

                // Disconnect the current IronRDP session
                *ctx.ironrdp_tx.borrow_mut() = None;
                if let Some(mut c) = ctx.client_ref.borrow_mut().take() {
                    c.disconnect();
                }

                // Modify the stored config to force Legacy graphics on retry
                if let Some(ref mut cfg) = *ctx.fallback_config.borrow_mut() {
                    cfg.force_legacy_graphics = true;
                }

                // Schedule reconnect after a brief delay (1s) to allow the
                // server to fully tear down the previous session. This avoids
                // NLA rejection on single-session servers. (Issue #218)
                let on_reconnect = ctx.on_reconnect.clone();
                let state = ctx.state.clone();
                let on_state_changed = ctx.on_state_changed.clone();
                glib::timeout_add_local_once(std::time::Duration::from_secs(1), move || {
                    *state.borrow_mut() = RdpConnectionState::Connecting;
                    if let Some(ref cb) = *on_state_changed.borrow() {
                        cb(RdpConnectionState::Connecting);
                    }
                    if let Some(ref callback) = *on_reconnect.borrow() {
                        callback();
                    }
                });
                return;
            }

            tracing::warn!(
                protocol = "rdp",
                error = %msg,
                "[IronRDP] Protocol incompatibility — attempting fallback to FreeRDP"
            );

            // Clean up IronRDP state
            *ctx.is_embedded.borrow_mut() = false;
            *ctx.is_ironrdp.borrow_mut() = false;
            *ctx.ironrdp_tx.borrow_mut() = None;
            ctx.toolbar.set_visible(false);

            // Disconnect the IronRDP client
            if let Some(mut c) = ctx.client_ref.borrow_mut().take() {
                c.disconnect();
            }

            // Attempt FreeRDP external fallback via SafeFreeRdpLauncher
            // (uses /from-stdin to avoid exposing password in /proc/PID/cmdline)
            let fallback_result = ctx
                .fallback_config
                .borrow()
                .as_ref()
                .cloned()
                .and_then(|cfg| {
                    let launcher = SafeFreeRdpLauncher::new();
                    match launcher.launch(&cfg) {
                        Ok((child, stderr_buf)) => {
                            tracing::info!(
                                protocol = "rdp",
                                host = %cfg.host,
                                port = %cfg.port,
                                "[IronRDP] Fallback to external FreeRDP"
                            );
                            *ctx.fallback_process.borrow_mut() = Some(child);
                            Some(stderr_buf)
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

            if let Some(stderr_buf) = fallback_result {
                *ctx.state.borrow_mut() = RdpConnectionState::Connected;
                if let Some(ref cb) = *ctx.on_state_changed.borrow() {
                    cb(RdpConnectionState::Connected);
                }
                let fb_cb = ctx.on_fallback.borrow_mut().take();
                if let Some(cb) = fb_cb {
                    cb(&i18n("Using external RDP client (server incompatible)"));
                    *ctx.on_fallback.borrow_mut() = Some(cb);
                }
                // The external client may itself fail to connect (auth, cert,
                // codec). Detect an immediate exit and surface it as an error
                // instead of leaving a phantom "Connected" state.
                arm_external_exit_watchdog(
                    ctx.fallback_process.clone(),
                    ctx.state.clone(),
                    ctx.on_state_changed.clone(),
                    ctx.on_error.clone(),
                    ctx.drawing_area.clone(),
                    stderr_buf,
                );
            } else {
                *ctx.state.borrow_mut() = RdpConnectionState::Error;
                if let Some(ref cb) = *ctx.on_error.borrow() {
                    cb(&i18n(
                        "RDP server sent unsupported data. Install FreeRDP (xfreerdp3) for compatibility.",
                    ));
                }
            }
        } else {
            // Non-protocol error — report normally with user-friendly message
            *ctx.state.borrow_mut() = RdpConnectionState::Error;
            ctx.toolbar.set_visible(false);

            // Parse IronRDP error into user-friendly message
            let user_msg = Self::parse_ironrdp_error(msg);

            // Use take-invoke-restore to avoid RefCell re-entrancy panic:
            // the state_changed callback may close the tab, which fires
            // Disconnected and tries to borrow the same cell again.
            let state_cb = ctx.on_state_changed.borrow_mut().take();
            if let Some(ref callback) = state_cb {
                callback(RdpConnectionState::Error);
            }
            *ctx.on_state_changed.borrow_mut() = state_cb;

            let error_cb = ctx.on_error.borrow_mut().take();
            if let Some(ref callback) = error_cb {
                callback(&user_msg);
            }
            *ctx.on_error.borrow_mut() = error_cb;
        }
    }

    /// Parses IronRDP error messages into user-friendly descriptions.
    ///
    /// Maps known NTSTATUS codes and error patterns to localized messages
    /// that help users understand what went wrong.
    #[cfg(feature = "rdp-embedded")]
    fn parse_ironrdp_error(msg: &str) -> String {
        // CredSSP / NLA authentication failures
        // STATUS_LOGON_FAILURE (0xc000006d) — wrong username or password
        if msg.contains("0xc000006d") || msg.contains("STATUS_LOGON_FAILURE") {
            return i18n("Authentication failed: invalid username or password.");
        }
        // STATUS_WRONG_PASSWORD (0xc000006a)
        if msg.contains("0xc000006a") {
            return i18n("Authentication failed: invalid username or password.");
        }
        // STATUS_ACCOUNT_RESTRICTION (0xc000006e) — logon restrictions apply
        if msg.contains("0xc000006e") {
            return i18n("Authentication failed: account restrictions prevent this login.");
        }
        // STATUS_PASSWORD_MUST_CHANGE (0xc0000070)
        if msg.contains("0xc0000070") {
            return i18n("Authentication failed: password must be changed before first login.");
        }
        // STATUS_ACCOUNT_DISABLED (0xc0000072)
        if msg.contains("0xc0000072") {
            return i18n("Authentication failed: account is disabled.");
        }
        // STATUS_ACCOUNT_LOCKED_OUT (0xc0000234)
        if msg.contains("0xc0000234") {
            return i18n("Authentication failed: account is locked out.");
        }
        // STATUS_PASSWORD_EXPIRED (0xc0000071)
        if msg.contains("0xc0000071") {
            return i18n("Authentication failed: password has expired.");
        }
        // STATUS_ACCOUNT_EXPIRED (0xc0000193)
        if msg.contains("0xc0000193") {
            return i18n("Authentication failed: account has expired.");
        }
        // STATUS_LOGON_TYPE_NOT_GRANTED (0xc000015b)
        if msg.contains("0xc000015b") {
            return i18n("Authentication failed: user is not allowed to log on to this computer.");
        }
        // Generic CredSSP error
        if msg.contains("CredSSP") || msg.contains("Credssp") {
            return i18n("NLA authentication failed. Check username and password.");
        }
        // TLS errors
        if msg.contains("TLS") || msg.contains("tls") {
            return i18n("TLS connection failed. The server may not support this security level.");
        }
        // Connection refused / unreachable
        if msg.contains("Connection refused") || msg.contains("connection refused") {
            return i18n("Connection refused. Check host and port.");
        }
        if msg.contains("timed out") || msg.contains("Timeout") {
            return i18n("Connection timed out. Check that the host is reachable.");
        }
        // Fallback: return original message (already formatted by EmbeddedClientError)
        msg.to_string()
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

        let expected = usize::from(width) * usize::from(height) * 4;
        if data.len() < expected {
            tracing::warn!(
                expected,
                actual = data.len(),
                "Cursor bitmap data too small, skipping"
            );
            return;
        }

        // The server renders the pointer at the session DPI, which now matches
        // our display scale — so a 200% session sends a 2× (device-pixel) cursor
        // bitmap. GDK interprets a cursor texture's dimensions as *logical*
        // pixels (there is no HiDPI/scale hint on `from_texture`), so handing it
        // the raw device bitmap yields a cursor ~`scale`× too large. We therefore
        // downscale device→logical here.
        //
        // The downscale is an area-average (box filter), NOT nearest-neighbor:
        // NN samples one source pixel per destination pixel and drops the rest,
        // which erased the thin 1px strokes of HiDPI cursors (the "half missing"
        // pointer). Averaging every covered source pixel preserves them. At
        // Display Scale = Auto the session runs at 100%, so `scale` is 1.0 and
        // this is an identity copy (plus premultiply + R↔B swap for GDK).
        let w = usize::from(width);
        let h = usize::from(height);
        let bpp = 4;
        let scale = cursor_scale.max(1.0);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "cursor dimensions are small and non-negative; logical size fits usize"
        )]
        let dst_w = ((w as f64 / scale).round() as usize).max(1);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "cursor dimensions are small and non-negative; logical size fits usize"
        )]
        let dst_h = ((h as f64 / scale).round() as usize).max(1);

        // Contiguous source spans: sx1 of column N equals sx0 of column N+1, so
        // every source pixel contributes to exactly one destination pixel.
        let span = |d: usize, src_max: usize| {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "cursor dimensions are small and non-negative; span fits usize"
            )]
            let lo = ((d as f64) * scale).round() as usize;
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "cursor dimensions are small and non-negative; span fits usize"
            )]
            let hi = (((d + 1) as f64) * scale).round() as usize;
            (lo.min(src_max), hi.min(src_max).max(lo + 1).min(src_max))
        };

        // Output is B8g8r8a8 premultiplied: server data is straight-alpha RGBA,
        // so we premultiply before averaging (correct edge blending) and swap
        // R↔B on write.
        let mut out = vec![0u8; dst_w * dst_h * bpp];
        for dy in 0..dst_h {
            let (sy0, sy1) = span(dy, h);
            for dx in 0..dst_w {
                let (sx0, sx1) = span(dx, w);
                let (mut acc_r, mut acc_g, mut acc_b, mut acc_a, mut count) =
                    (0u32, 0u32, 0u32, 0u32, 0u32);
                for sy in sy0..sy1 {
                    for sx in sx0..sx1 {
                        let o = (sy * w + sx) * bpp;
                        let a = u32::from(data[o + 3]);
                        acc_r += u32::from(data[o]) * a / 255;
                        acc_g += u32::from(data[o + 1]) * a / 255;
                        acc_b += u32::from(data[o + 2]) * a / 255;
                        acc_a += a;
                        count += 1;
                    }
                }
                let count = count.max(1);
                let d = (dy * dst_w + dx) * bpp;
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "averaged 8-bit channel stays within u8"
                )]
                {
                    out[d] = (acc_b / count) as u8; // B
                    out[d + 1] = (acc_g / count) as u8; // G
                    out[d + 2] = (acc_r / count) as u8; // R
                    out[d + 3] = (acc_a / count) as u8; // A
                }
            }
        }

        let hotspot_logical_x = (f64::from(hotspot_x) / scale).round() as i32;
        let hotspot_logical_y = (f64::from(hotspot_y) / scale).round() as i32;

        let bytes = glib::Bytes::from(&out);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_possible_wrap,
            reason = "logical cursor dimensions are small and fit i32 texture args"
        )]
        let texture = gdk::MemoryTexture::new(
            dst_w as i32,
            dst_h as i32,
            gdk::MemoryFormat::B8g8r8a8Premultiplied,
            &bytes,
            dst_w * bpp,
        );
        let cursor =
            gdk::Cursor::from_texture(&texture, hotspot_logical_x, hotspot_logical_y, None);
        drawing_area.set_cursor(Some(&cursor));
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
        *self.is_embedded.borrow_mut() = false;
    }

    /// Connects using external mode with user notification
    pub(super) fn connect_external_with_notification(
        &self,
        config: &RdpConfig,
    ) -> Result<(), EmbeddedRdpError> {
        // Notify user about fallback
        self.report_fallback("RDP session will open in external window");

        // Connect using external mode
        self.connect_external(config)
    }

    /// Connects using embedded mode (wlfreerdp) with thread isolation
    fn connect_embedded(&self, config: &RdpConfig) -> Result<(), EmbeddedRdpError> {
        tracing::debug!(
            protocol = "rdp",
            host = %config.host,
            port = config.port,
            "Attempting embedded FreeRDP connection"
        );

        // Spawn FreeRDP in a dedicated thread to isolate Qt/GTK conflicts
        let freerdp_thread = FreeRdpThread::spawn(config)?;

        // Send connect command to the thread
        freerdp_thread.send_command(RdpCommand::Connect(Box::new(config.clone())))?;

        // Store the thread handle
        *self.freerdp_thread.borrow_mut() = Some(freerdp_thread);
        *self.is_embedded.borrow_mut() = true;

        // Initialize RDP dimensions from config
        *self.rdp_width.borrow_mut() = config.width;
        *self.rdp_height.borrow_mut() = config.height;

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
        let is_embedded = self.is_embedded.clone();
        let freerdp_thread_ref = self.freerdp_thread.clone();

        // Mouse jiggler handles + config — armed on Connected here because this
        // event path sets the state directly, bypassing set_state (#185).
        let jiggler = self.jiggler_handles();
        let jiggler_config = self.config.clone();

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
                            if let Some(interval) = jiggler_config
                                .borrow()
                                .as_ref()
                                .filter(|c| c.jiggler_enabled)
                                .map(|c| c.jiggler_interval_secs)
                            {
                                jiggler.start(interval);
                            }
                            drawing_area.queue_draw();
                        }
                        RdpEvent::Disconnected => {
                            tracing::debug!(protocol = "rdp", "FreeRDP disconnected");
                            jiggler.stop();
                            *state.borrow_mut() = RdpConnectionState::Disconnected;
                            if let Some(ref callback) = *on_state_changed.borrow() {
                                callback(RdpConnectionState::Disconnected);
                            }
                            drawing_area.queue_draw();
                            return glib::ControlFlow::Break;
                        }
                        RdpEvent::Error(msg) => {
                            tracing::error!(protocol = "rdp", error = %msg, "FreeRDP error");
                            jiggler.stop();
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
    fn connect_external(&self, config: &RdpConfig) -> Result<(), EmbeddedRdpError> {
        // Use SafeFreeRdpLauncher for Qt error suppression
        let launcher = SafeFreeRdpLauncher::new();

        match launcher.launch(config) {
            Ok((child, stderr_buf)) => {
                *self.process.borrow_mut() = Some(child);
                *self.stderr_lines.borrow_mut() = Some(stderr_buf);
                *self.is_embedded.borrow_mut() = false;
                self.set_state(RdpConnectionState::Connected);
                // Trigger redraw to show "Session running in external window"
                self.drawing_area.queue_draw();
                // Detect an immediate exit (auth/cert/codec failure) so the user
                // sees an error instead of a window that flashed and closed.
                self.arm_external_exit_watchdog();
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
    pub fn disconnect(&self) {
        // Increment connection generation to invalidate any active polling loops
        *self.connection_generation.borrow_mut() += 1;

        // Disconnect resize signal handler
        if let Some(handler_id) = self.resize_handler_id.borrow_mut().take() {
            self.drawing_area.disconnect(handler_id);
        }

        // Shutdown FreeRDP thread if running
        if let Some(mut thread) = self.freerdp_thread.borrow_mut().take() {
            thread.shutdown();
        }

        // Kill external process if running
        self.terminate_external_process();

        // Clear Cairo-backed buffer
        self.cairo_buffer.borrow_mut().clear();

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
        if let Some(mut config) = config {
            // Reset force_legacy_graphics so the GFX pipeline gets another
            // chance on user-initiated reconnect. If GFX fails again, the
            // automatic retry will re-enable Legacy mode. (Issue #218)
            config.force_legacy_graphics = false;
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

    /// Arms a short-lived watchdog detecting an external client that exits
    /// immediately after launch (auth/certificate/codec failure).
    ///
    /// See [`arm_external_exit_watchdog`] for the rationale.
    fn arm_external_exit_watchdog(&self) {
        use std::sync::{Arc, Mutex};

        let stderr_buf = self
            .stderr_lines
            .borrow()
            .clone()
            .unwrap_or_else(|| Arc::new(Mutex::new(Vec::new())));
        arm_external_exit_watchdog(
            self.process.clone(),
            self.state.clone(),
            self.on_state_changed.clone(),
            self.on_error.clone(),
            self.drawing_area.clone(),
            stderr_buf,
        );
    }

    /// Terminates the external FreeRDP process if running
    ///
    /// This method gracefully terminates the process, waiting for it to exit.
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
