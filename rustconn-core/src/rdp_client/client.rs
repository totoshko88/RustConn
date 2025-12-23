//! RDP client implementation using `IronRDP`
//!
//! This module provides the async RDP client that connects to RDP servers
//! and produces framebuffer events for the GUI to render.
//!
//! # Architecture
//!
//! The RDP client follows the same pattern as the `VncClient`:
//! - Runs in a background thread with its own Tokio runtime
//! - Communicates via `std::sync::mpsc` channels for cross-runtime compatibility
//! - Produces events for framebuffer updates, resolution changes, etc.
//! - Accepts commands for keyboard/mouse input, disconnect, etc.

// Allow clippy warnings for this file - RDP protocol uses various integer sizes
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::default_trait_access)]

use super::{RdpClientCommand, RdpClientConfig, RdpClientError, RdpClientEvent, RdpRect};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

/// Sender for commands to the RDP client (thread-safe, non-async)
pub type RdpCommandSender = std::sync::mpsc::Sender<RdpClientCommand>;

/// Receiver for events from the RDP client (thread-safe, non-async)
pub type RdpEventReceiver = std::sync::mpsc::Receiver<RdpClientEvent>;

/// RDP client state for tracking connection lifecycle
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RdpClientState {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Disconnecting,
    Error,
}

/// RDP client handle for managing connections
pub struct RdpClient {
    command_tx: Option<std::sync::mpsc::Sender<RdpClientCommand>>,
    event_rx: Option<std::sync::mpsc::Receiver<RdpClientEvent>>,
    connected: Arc<AtomicBool>,
    config: RdpClientConfig,
    thread_handle: Option<JoinHandle<()>>,
    shutdown_signal: Arc<AtomicBool>,
}

impl RdpClient {
    #[must_use]
    pub fn new(config: RdpClientConfig) -> Self {
        Self {
            command_tx: None,
            event_rx: None,
            connected: Arc::new(AtomicBool::new(false)),
            config,
            thread_handle: None,
            shutdown_signal: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Connects to the RDP server
    ///
    /// # Errors
    ///
    /// Returns `RdpClientError::AlreadyConnected` if already connected.
    pub fn connect(&mut self) -> Result<(), RdpClientError> {
        if self.connected.load(Ordering::SeqCst) {
            return Err(RdpClientError::AlreadyConnected);
        }

        self.shutdown_signal.store(false, Ordering::SeqCst);

        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let (command_tx, command_rx) = std::sync::mpsc::channel();

        self.event_rx = Some(event_rx);
        self.command_tx = Some(command_tx);

        let config = self.config.clone();
        let connected = self.connected.clone();
        let shutdown_signal = self.shutdown_signal.clone();

        self.connected.store(true, Ordering::SeqCst);

        let handle = std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = event_tx.send(RdpClientEvent::Error(format!(
                        "Failed to create Tokio runtime: {e}"
                    )));
                    connected.store(false, Ordering::SeqCst);
                    return;
                }
            };

            rt.block_on(async move {
                let result =
                    run_rdp_client(config, event_tx.clone(), command_rx, shutdown_signal).await;
                connected.store(false, Ordering::SeqCst);

                if let Err(e) = result {
                    let _ = event_tx.send(RdpClientEvent::Error(e.to_string()));
                }
                let _ = event_tx.send(RdpClientEvent::Disconnected);
            });
        });

        self.thread_handle = Some(handle);
        Ok(())
    }

    #[must_use]
    pub fn try_recv_event(&self) -> Option<RdpClientEvent> {
        self.event_rx.as_ref()?.try_recv().ok()
    }

    /// Sends a command to the RDP client.
    ///
    /// # Errors
    ///
    /// Returns `RdpClientError::NotConnected` if not connected,
    /// or `RdpClientError::ChannelError` if the channel is closed.
    pub fn send_command(&self, command: RdpClientCommand) -> Result<(), RdpClientError> {
        let tx = self
            .command_tx
            .as_ref()
            .ok_or(RdpClientError::NotConnected)?;
        tx.send(command)
            .map_err(|e| RdpClientError::ChannelError(e.to_string()))
    }

    /// Sends a key event to the RDP server.
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn send_key(&self, scancode: u16, pressed: bool) -> Result<(), RdpClientError> {
        self.send_command(RdpClientCommand::KeyEvent {
            scancode,
            pressed,
            extended: false,
        })
    }

    /// Sends a pointer/mouse event to the RDP server.
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn send_pointer(&self, x: u16, y: u16, buttons: u8) -> Result<(), RdpClientError> {
        self.send_command(RdpClientCommand::PointerEvent { x, y, buttons })
    }

    /// Sends Ctrl+Alt+Del key sequence to the RDP server.
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn send_ctrl_alt_del(&self) -> Result<(), RdpClientError> {
        self.send_command(RdpClientCommand::SendCtrlAltDel)
    }

    /// Requests a desktop size change.
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn set_desktop_size(&self, width: u16, height: u16) -> Result<(), RdpClientError> {
        self.send_command(RdpClientCommand::SetDesktopSize { width, height })
    }

    pub fn disconnect(&mut self) {
        self.shutdown_signal.store(true, Ordering::SeqCst);
        if let Some(tx) = &self.command_tx {
            let _ = tx.send(RdpClientCommand::Disconnect);
        }
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
        self.command_tx = None;
        self.event_rx = None;
        self.connected.store(false, Ordering::SeqCst);
    }

    #[must_use]
    pub fn is_cleaned_up(&self) -> bool {
        self.command_tx.is_none()
            && self.event_rx.is_none()
            && self.thread_handle.is_none()
            && !self.connected.load(Ordering::SeqCst)
    }

    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    #[must_use]
    pub const fn config(&self) -> &RdpClientConfig {
        &self.config
    }

    #[must_use]
    pub const fn event_receiver(&self) -> Option<&std::sync::mpsc::Receiver<RdpClientEvent>> {
        self.event_rx.as_ref()
    }

    #[must_use]
    pub fn command_sender(&self) -> Option<std::sync::mpsc::Sender<RdpClientCommand>> {
        self.command_tx.clone()
    }
}

impl Drop for RdpClient {
    fn drop(&mut self) {
        self.disconnect();
    }
}

// ============================================================================
// IronRDP Integration
// ============================================================================

use ironrdp::connector::{ClientConnector, Config, ConnectionResult, Credentials, DesktopSize};
use ironrdp::graphics::image_processing::PixelFormat as IronPixelFormat;
use ironrdp::pdu::gcc::KeyboardType;
use ironrdp::pdu::input::fast_path::{FastPathInputEvent, KeyboardFlags};
use ironrdp::pdu::input::mouse::PointerFlags;
use ironrdp::pdu::input::MousePdu;
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp::pdu::rdp::client_info::PerformanceFlags;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageOutput};
use ironrdp_tokio::{split_tokio_framed, FramedWrite, TokioFramed};
use std::net::SocketAddr;
use tokio::net::TcpStream;

type UpgradedFramed = TokioFramed<ironrdp_tls::TlsStream<TcpStream>>;

/// Runs the RDP client protocol loop using `IronRDP`
async fn run_rdp_client(
    config: RdpClientConfig,
    event_tx: std::sync::mpsc::Sender<RdpClientEvent>,
    command_rx: std::sync::mpsc::Receiver<RdpClientCommand>,
    shutdown_signal: Arc<AtomicBool>,
) -> Result<(), RdpClientError> {
    use tokio::time::{timeout, Duration};

    let server_addr = config.server_address();
    let connect_timeout = Duration::from_secs(config.timeout_secs);

    // Phase 1: Establish TCP connection
    let tcp_result = timeout(connect_timeout, TcpStream::connect(&server_addr)).await;

    let stream = match tcp_result {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => {
            return Err(RdpClientError::ConnectionFailed(format!(
                "Failed to connect to {server_addr}: {e}"
            )));
        }
        Err(_) => {
            return Err(RdpClientError::Timeout);
        }
    };

    let client_addr = stream
        .local_addr()
        .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 0)));

    // Phase 2: Build IronRDP connector configuration
    let connector_config = build_connector_config(&config);
    let mut connector = ClientConnector::new(connector_config, client_addr);

    // Phase 3: Perform RDP connection sequence
    let mut framed = TokioFramed::new(stream);

    // Begin connection (X.224 negotiation)
    let should_upgrade = ironrdp_tokio::connect_begin(&mut framed, &mut connector)
        .await
        .map_err(|e| RdpClientError::ConnectionFailed(format!("Connection begin failed: {e}")))?;

    // TLS upgrade - returns stream and server public key directly
    let initial_stream = framed.into_inner_no_leftover();

    let (upgraded_stream, server_public_key) = ironrdp_tls::upgrade(initial_stream, &config.host)
        .await
        .map_err(|e| RdpClientError::ConnectionFailed(format!("TLS upgrade failed: {e}")))?;

    let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);

    let mut upgraded_framed = TokioFramed::new(upgraded_stream);

    // Complete connection (NLA, licensing, capabilities)
    // Note: We pass None for network_client since we don't need AAD/gateway
    let connection_result = ironrdp_tokio::connect_finalize(
        upgraded,
        &mut upgraded_framed,
        connector,
        (&config.host).into(),
        server_public_key,
        None,
        None,
    )
    .await
    .map_err(|e| RdpClientError::ConnectionFailed(format!("Connection finalize failed: {e}")))?;

    // Send connected event
    let _ = event_tx.send(RdpClientEvent::Connected {
        width: connection_result.desktop_size.width,
        height: connection_result.desktop_size.height,
    });

    // Phase 4: Active session loop
    run_active_session(
        upgraded_framed,
        connection_result,
        event_tx,
        command_rx,
        shutdown_signal,
    )
    .await
}

/// Builds `IronRDP` connector configuration from our config
fn build_connector_config(config: &RdpClientConfig) -> Config {
    // Always use UsernamePassword credentials
    // If username or password is missing, use empty strings
    // The server will prompt for credentials if needed
    let credentials = Credentials::UsernamePassword {
        username: config.username.clone().unwrap_or_default(),
        password: config.password.clone().unwrap_or_default(),
    };

    Config {
        credentials,
        domain: config.domain.clone(),
        enable_tls: true,
        enable_credssp: config.nla_enabled,
        keyboard_type: KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_functional_keys_count: 12,
        keyboard_layout: 0x0409, // US English
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: DesktopSize {
            width: config.width,
            height: config.height,
        },
        desktop_scale_factor: 0,
        bitmap: None,
        client_build: 0,
        client_name: String::from("RustConn"),
        client_dir: String::new(),
        platform: MajorPlatformType::UNIX,
        hardware_id: None,
        request_data: None,
        autologon: false,
        no_audio_playback: !config.audio_enabled,
        performance_flags: PerformanceFlags::default(),
        license_cache: None,
        no_server_pointer: false,
        pointer_software_rendering: true,
    }
}

/// Placeholder type for network client (not used, we pass None)

/// Runs the active RDP session, processing framebuffer updates and input
#[allow(clippy::too_many_lines)]
async fn run_active_session(
    framed: UpgradedFramed,
    connection_result: ConnectionResult,
    event_tx: std::sync::mpsc::Sender<RdpClientEvent>,
    command_rx: std::sync::mpsc::Receiver<RdpClientCommand>,
    shutdown_signal: Arc<AtomicBool>,
) -> Result<(), RdpClientError> {
    let (mut reader, mut writer) = split_tokio_framed(framed);

    // Create decoded image buffer
    let mut image = DecodedImage::new(
        IronPixelFormat::BgrA32,
        connection_result.desktop_size.width,
        connection_result.desktop_size.height,
    );

    let mut active_stage = ActiveStage::new(connection_result);

    loop {
        // Check shutdown signal
        if shutdown_signal.load(Ordering::SeqCst) {
            if let Ok(frames) = active_stage.graceful_shutdown() {
                for frame in frames {
                    if let ActiveStageOutput::ResponseFrame(data) = frame {
                        let _ = writer.write_all(&data).await;
                    }
                }
            }
            break;
        }

        // Process commands from GUI (non-blocking)
        while let Ok(cmd) = command_rx.try_recv() {
            match cmd {
                RdpClientCommand::Disconnect => {
                    if let Ok(frames) = active_stage.graceful_shutdown() {
                        for frame in frames {
                            if let ActiveStageOutput::ResponseFrame(data) = frame {
                                let _ = writer.write_all(&data).await;
                            }
                        }
                    }
                    return Ok(());
                }
                RdpClientCommand::KeyEvent {
                    scancode,
                    pressed,
                    extended,
                } => {
                    let event = create_keyboard_event(scancode, pressed, extended);
                    send_input_events(&mut active_stage, &mut image, &mut writer, &[event]).await;
                }
                RdpClientCommand::UnicodeEvent { character, pressed } => {
                    let event = create_unicode_event(character, pressed);
                    send_input_events(&mut active_stage, &mut image, &mut writer, &[event]).await;
                }
                RdpClientCommand::PointerEvent { x, y, buttons } => {
                    let event = create_pointer_event(x, y, buttons);
                    send_input_events(&mut active_stage, &mut image, &mut writer, &[event]).await;
                }
                RdpClientCommand::MouseButtonPress { x, y, button } => {
                    let event = create_button_press_event(x, y, button);
                    send_input_events(&mut active_stage, &mut image, &mut writer, &[event]).await;
                }
                RdpClientCommand::MouseButtonRelease { x, y, button } => {
                    let event = create_button_release_event(x, y, button);
                    send_input_events(&mut active_stage, &mut image, &mut writer, &[event]).await;
                }
                RdpClientCommand::SendCtrlAltDel => {
                    let events = create_ctrl_alt_del_sequence();
                    send_input_events(&mut active_stage, &mut image, &mut writer, &events).await;
                }
                RdpClientCommand::WheelEvent {
                    horizontal,
                    vertical,
                } => {
                    if vertical != 0 {
                        let event = create_wheel_event(vertical, false);
                        send_input_events(&mut active_stage, &mut image, &mut writer, &[event])
                            .await;
                    }
                    if horizontal != 0 {
                        let event = create_wheel_event(horizontal, true);
                        send_input_events(&mut active_stage, &mut image, &mut writer, &[event])
                            .await;
                    }
                }
                RdpClientCommand::SetDesktopSize { width, height } => {
                    // Request resolution change via Display Control Virtual Channel
                    // IronRDP supports this through the active stage
                    eprintln!("[IronRDP] Requesting resolution {width}x{height} for window");
                    // Note: IronRDP's ActiveStage doesn't directly expose resize API
                    // The server may support dynamic resolution via RDPEDISP
                    // For now, we log the request - full implementation would require
                    // sending Display Control PDUs
                }
                RdpClientCommand::RefreshScreen => {
                    // Request full screen refresh
                    // This is handled by sending a Refresh PDU or waiting for server updates
                    eprintln!("[IronRDP] Screen refresh requested");
                }
                RdpClientCommand::ClipboardText(text) => {
                    // Send text as Unicode key events (workaround for missing CLIPRDR)
                    // This simulates typing the text character by character
                    eprintln!(
                        "[IronRDP] Pasting {} chars via Unicode key events",
                        text.len()
                    );

                    for ch in text.chars() {
                        // Send key press and release for each character
                        let event_press = create_unicode_event(ch, true);
                        let event_release = create_unicode_event(ch, false);
                        send_input_events(
                            &mut active_stage,
                            &mut image,
                            &mut writer,
                            &[event_press],
                        )
                        .await;
                        send_input_events(
                            &mut active_stage,
                            &mut image,
                            &mut writer,
                            &[event_release],
                        )
                        .await;
                    }
                }
                RdpClientCommand::Authenticate { .. } => {}
            }
        }

        // Read and process RDP frames with timeout
        let read_result = tokio::time::timeout(
            std::time::Duration::from_millis(16), // ~60 FPS
            reader.read_pdu(),
        )
        .await;

        match read_result {
            Ok(Ok((action, payload))) => {
                match active_stage.process(&mut image, action, &payload) {
                    Ok(outputs) => {
                        for output in outputs {
                            match output {
                                ActiveStageOutput::ResponseFrame(data) => {
                                    if let Err(e) = writer.write_all(&data).await {
                                        return Err(RdpClientError::ConnectionFailed(format!(
                                            "Write error: {e}"
                                        )));
                                    }
                                }
                                ActiveStageOutput::GraphicsUpdate(region) => {
                                    let rect = RdpRect::new(
                                        region.left,
                                        region.top,
                                        region.right.saturating_sub(region.left),
                                        region.bottom.saturating_sub(region.top),
                                    );
                                    let data = extract_region_data(&image, &rect);
                                    let _ =
                                        event_tx.send(RdpClientEvent::FrameUpdate { rect, data });
                                }
                                ActiveStageOutput::PointerDefault => {
                                    let _ = event_tx.send(RdpClientEvent::CursorDefault);
                                }
                                ActiveStageOutput::PointerHidden => {
                                    let _ = event_tx.send(RdpClientEvent::CursorHidden);
                                }
                                ActiveStageOutput::PointerPosition { x, y } => {
                                    let _ = event_tx.send(RdpClientEvent::CursorPosition { x, y });
                                }
                                ActiveStageOutput::PointerBitmap(pointer) => {
                                    let _ = event_tx.send(RdpClientEvent::CursorUpdate {
                                        width: pointer.width,
                                        height: pointer.height,
                                        hotspot_x: pointer.hotspot_x,
                                        hotspot_y: pointer.hotspot_y,
                                        data: pointer.bitmap_data.clone(),
                                    });
                                }
                                ActiveStageOutput::Terminate(reason) => {
                                    tracing::info!("RDP session terminated: {reason:?}");
                                    return Ok(());
                                }
                                ActiveStageOutput::DeactivateAll(deactivate_info) => {
                                    // Server is deactivating the session, possibly for resize
                                    // Send a resolution changed event to notify GUI
                                    eprintln!(
                                        "[IronRDP] Received deactivate-all: {deactivate_info:?}"
                                    );
                                    // The session will be reactivated with new parameters
                                    // We should wait for new graphics updates
                                }
                            }
                        }
                    }
                    Err(e) => {
                        return Err(RdpClientError::ProtocolError(format!("Session error: {e}")));
                    }
                }
            }
            Ok(Err(e)) => {
                return Err(RdpClientError::ConnectionFailed(format!("Read error: {e}")));
            }
            Err(_) => {
                // Timeout - no data available, continue loop
            }
        }
    }

    Ok(())
}

/// Creates a keyboard `FastPath` event
fn create_keyboard_event(scancode: u16, pressed: bool, extended: bool) -> FastPathInputEvent {
    let mut flags = KeyboardFlags::empty();
    if !pressed {
        flags |= KeyboardFlags::RELEASE;
    }
    if extended {
        flags |= KeyboardFlags::EXTENDED;
    }
    // RDP scancodes are 8-bit, but we use u16 to preserve the value during transmission
    // The actual scancode is in the lower 8 bits
    #[allow(clippy::cast_possible_truncation)]
    FastPathInputEvent::KeyboardEvent(flags, scancode as u8)
}

/// Creates a Unicode keyboard `FastPath` event for non-ASCII characters
fn create_unicode_event(character: char, pressed: bool) -> FastPathInputEvent {
    let mut flags = KeyboardFlags::empty();
    if !pressed {
        flags |= KeyboardFlags::RELEASE;
    }
    // Unicode events use the character's code point as u16
    // Characters outside BMP (> 0xFFFF) are truncated, but most keyboard input is within BMP
    #[allow(clippy::cast_possible_truncation)]
    let code_point = character as u32 as u16;
    FastPathInputEvent::UnicodeKeyboardEvent(flags, code_point)
}

/// Creates a pointer/mouse motion `FastPath` event (no button state change)
const fn create_pointer_event(x: u16, y: u16, _buttons: u8) -> FastPathInputEvent {
    // For motion events, only send MOVE flag - no button state
    FastPathInputEvent::MouseEvent(MousePdu {
        flags: PointerFlags::MOVE,
        number_of_wheel_rotation_units: 0,
        x_position: x,
        y_position: y,
    })
}

/// Creates a mouse button press `FastPath` event
#[allow(clippy::match_same_arms)]
fn create_button_press_event(x: u16, y: u16, button: u8) -> FastPathInputEvent {
    let button_flag = match button {
        1 => PointerFlags::LEFT_BUTTON,
        2 => PointerFlags::RIGHT_BUTTON,
        3 => PointerFlags::MIDDLE_BUTTON_OR_WHEEL,
        _ => PointerFlags::LEFT_BUTTON,
    };

    // Button press: button flag + DOWN, no MOVE
    FastPathInputEvent::MouseEvent(MousePdu {
        flags: button_flag | PointerFlags::DOWN,
        number_of_wheel_rotation_units: 0,
        x_position: x,
        y_position: y,
    })
}

/// Creates a mouse button release `FastPath` event
#[allow(clippy::match_same_arms)]
const fn create_button_release_event(x: u16, y: u16, button: u8) -> FastPathInputEvent {
    let button_flag = match button {
        1 => PointerFlags::LEFT_BUTTON,
        2 => PointerFlags::RIGHT_BUTTON,
        3 => PointerFlags::MIDDLE_BUTTON_OR_WHEEL,
        _ => PointerFlags::LEFT_BUTTON,
    };

    // Button release: only button flag, no DOWN, no MOVE
    FastPathInputEvent::MouseEvent(MousePdu {
        flags: button_flag,
        number_of_wheel_rotation_units: 0,
        x_position: x,
        y_position: y,
    })
}

/// Creates Ctrl+Alt+Del key sequence
fn create_ctrl_alt_del_sequence() -> [FastPathInputEvent; 6] {
    [
        // Ctrl down
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x1D),
        // Alt down
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x38),
        // Delete down (extended)
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::EXTENDED, 0x53),
        // Delete up
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE | KeyboardFlags::EXTENDED, 0x53),
        // Alt up
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x38),
        // Ctrl up
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x1D),
    ]
}

/// Creates a mouse wheel event
const fn create_wheel_event(delta: i16, horizontal: bool) -> FastPathInputEvent {
    let flags = if horizontal {
        PointerFlags::HORIZONTAL_WHEEL
    } else {
        PointerFlags::VERTICAL_WHEEL
    };

    FastPathInputEvent::MouseEvent(MousePdu {
        flags,
        number_of_wheel_rotation_units: delta,
        x_position: 0,
        y_position: 0,
    })
}

/// Sends input events to the RDP server
async fn send_input_events<W: FramedWrite>(
    active_stage: &mut ActiveStage,
    image: &mut DecodedImage,
    writer: &mut W,
    events: &[FastPathInputEvent],
) {
    if let Ok(outputs) = active_stage.process_fastpath_input(image, events) {
        for output in outputs {
            if let ActiveStageOutput::ResponseFrame(data) = output {
                let _ = writer.write_all(&data).await;
            }
        }
    }
}

/// Extracts pixel data for a specific region from the decoded image
/// Converts from `IronRDP`'s BGRA format to Cairo's ARGB32 format by swapping R and B channels
#[allow(clippy::many_single_char_names)]
#[allow(clippy::trivially_copy_pass_by_ref)]
fn extract_region_data(image: &DecodedImage, rect: &RdpRect) -> Vec<u8> {
    let img_width = image.width();
    let img_height = image.height();
    let data = image.data();

    let region_x = rect.x.min(img_width);
    let region_y = rect.y.min(img_height);
    let region_w = rect.width.min(img_width.saturating_sub(region_x));
    let region_h = rect.height.min(img_height.saturating_sub(region_y));

    if region_w == 0 || region_h == 0 {
        return Vec::new();
    }

    let bytes_per_pixel = 4;
    let stride = img_width as usize * bytes_per_pixel;
    let result_size = region_w as usize * region_h as usize * bytes_per_pixel;
    let mut result = vec![0u8; result_size];

    for row in 0..region_h as usize {
        let src_row = (region_y as usize + row) * stride + region_x as usize * bytes_per_pixel;
        let dst_row = row * region_w as usize * bytes_per_pixel;

        for col in 0..region_w as usize {
            let src_idx = src_row + col * bytes_per_pixel;
            let dst_idx = dst_row + col * bytes_per_pixel;

            if src_idx + 4 <= data.len() {
                // IronRDP outputs BGRA, Cairo expects ARGB32 (which is BGRA on little-endian)
                // But IronRDP actually outputs in a different order, so swap R and B
                result[dst_idx] = data[src_idx + 2]; // R -> B position
                result[dst_idx + 1] = data[src_idx + 1]; // G stays
                result[dst_idx + 2] = data[src_idx]; // B -> R position
                result[dst_idx + 3] = data[src_idx + 3]; // A stays
            }
        }
    }

    result
}
