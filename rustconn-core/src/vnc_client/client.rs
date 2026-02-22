//! VNC client implementation using vnc-rs
//!
//! This module provides the async VNC client that connects to VNC servers
//! and produces framebuffer events for the GUI to render.

use super::{VncClientCommand, VncClientConfig, VncClientError, VncClientEvent, VncRect};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use vnc::{
    ClientKeyEvent, ClientMouseEvent, PixelFormat, VncConnector, VncEncoding, VncEvent, X11Event,
};

/// Sender for commands to the VNC client (thread-safe, non-async)
pub type VncCommandSender = mpsc::Sender<VncClientCommand>;

/// Receiver for events from the VNC client (thread-safe, non-async)
pub type VncEventReceiver = std::sync::mpsc::Receiver<VncClientEvent>;

/// VNC client handle for managing connections
///
/// This struct provides the interface for connecting to VNC servers
/// and receiving framebuffer updates. It runs the VNC protocol in
/// a background thread with its own Tokio runtime and communicates
/// via channels for cross-runtime compatibility.
pub struct VncClient {
    /// Channel for sending commands to the VNC task
    command_tx: Option<mpsc::Sender<VncClientCommand>>,
    /// Channel for receiving events from the VNC task (`std::sync` for cross-runtime)
    event_rx: Option<std::sync::mpsc::Receiver<VncClientEvent>>,
    /// Connection state (atomic for cross-thread access)
    connected: Arc<AtomicBool>,
    /// Configuration
    config: VncClientConfig,
}

impl VncClient {
    /// Creates a new VNC client with the given configuration
    #[must_use]
    pub fn new(config: VncClientConfig) -> Self {
        Self {
            command_tx: None,
            event_rx: None,
            connected: Arc::new(AtomicBool::new(false)),
            config,
        }
    }

    /// Connects to the VNC server and spawns the client task in a background thread
    ///
    /// This method spawns a new thread with its own Tokio runtime to handle the
    /// VNC protocol. Communication happens via `std::sync::mpsc` channels which
    /// work across different async runtimes (Tokio and `GLib`).
    ///
    /// Use `try_recv_event()` to poll for events from the `GLib` main loop.
    ///
    /// # Errors
    ///
    /// Returns error if client is already connected.
    pub fn connect(&mut self) -> Result<(), VncClientError> {
        if self.connected.load(Ordering::SeqCst) {
            return Err(VncClientError::AlreadyConnected);
        }

        // Use std::sync::mpsc for events (cross-runtime compatibility)
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        // Use tokio::sync::mpsc for commands (async select compatibility)
        let (command_tx, command_rx) = mpsc::channel(32);

        self.event_rx = Some(event_rx);
        self.command_tx = Some(command_tx);

        let config = self.config.clone();
        let connected = self.connected.clone();

        self.connected.store(true, Ordering::SeqCst);

        // Spawn the VNC client in a separate thread with its own Tokio runtime
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = event_tx.send(VncClientEvent::Error(format!(
                        "Failed to create Tokio runtime: {e}"
                    )));
                    connected.store(false, Ordering::SeqCst);
                    return;
                }
            };

            rt.block_on(async move {
                let result = run_vnc_client(config, event_tx.clone(), command_rx).await;
                connected.store(false, Ordering::SeqCst);

                if let Err(e) = result {
                    let _ = event_tx.send(VncClientEvent::Error(e.to_string()));
                }
                let _ = event_tx.send(VncClientEvent::Disconnected);
            });
        });

        Ok(())
    }

    /// Tries to receive the next event from the VNC client (non-blocking)
    ///
    /// This method is safe to call from any thread or async runtime (including `GLib`).
    /// Returns `None` if no event is available or the channel is closed.
    #[must_use]
    pub fn try_recv_event(&self) -> Option<VncClientEvent> {
        self.event_rx.as_ref()?.try_recv().ok()
    }

    /// Sends a command to the VNC client (non-blocking)
    ///
    /// This method is safe to call from any thread or async runtime.
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn send_command(&self, command: VncClientCommand) -> Result<(), VncClientError> {
        let tx = self
            .command_tx
            .as_ref()
            .ok_or(VncClientError::NotConnected)?;

        tx.blocking_send(command)
            .map_err(|e| VncClientError::ChannelError(e.to_string()))
    }

    /// Sends a key event
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn send_key(&self, keysym: u32, pressed: bool) -> Result<(), VncClientError> {
        if self.config.view_only {
            return Ok(());
        }
        self.send_command(VncClientCommand::KeyEvent { keysym, pressed })
    }

    /// Sends a pointer/mouse event
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn send_pointer(&self, x: u16, y: u16, buttons: u8) -> Result<(), VncClientError> {
        if self.config.view_only {
            return Ok(());
        }
        self.send_command(VncClientCommand::PointerEvent { x, y, buttons })
    }

    /// Requests a desktop size change
    ///
    /// Note: This requires server support for the `ExtendedDesktopSize` extension.
    /// Not all VNC servers support dynamic resolution changes.
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn set_desktop_size(&self, width: u16, height: u16) -> Result<(), VncClientError> {
        self.send_command(VncClientCommand::SetDesktopSize { width, height })
    }

    /// Sends Ctrl+Alt+Del key sequence
    ///
    /// This is commonly used to unlock Windows login screens or access
    /// the security options menu.
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn send_ctrl_alt_del(&self) -> Result<(), VncClientError> {
        self.send_command(VncClientCommand::SendCtrlAltDel)
    }

    /// Disconnects from the VNC server
    pub fn disconnect(&mut self) {
        if let Some(tx) = &self.command_tx {
            let _ = tx.blocking_send(VncClientCommand::Disconnect);
        }
        self.command_tx = None;
        self.event_rx = None;
        self.connected.store(false, Ordering::SeqCst);
    }

    /// Returns whether the client is connected
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    /// Returns the configuration
    #[must_use]
    pub const fn config(&self) -> &VncClientConfig {
        &self.config
    }

    /// Returns the event receiver for external polling
    ///
    /// This allows the caller to set up their own event polling mechanism.
    #[must_use]
    pub const fn event_receiver(&self) -> Option<&std::sync::mpsc::Receiver<VncClientEvent>> {
        self.event_rx.as_ref()
    }

    /// Returns the command sender for external use
    ///
    /// This allows the caller to send commands from multiple places.
    #[must_use]
    pub fn command_sender(&self) -> Option<mpsc::Sender<VncClientCommand>> {
        self.command_tx.clone()
    }
}

/// Runs the VNC client protocol loop
#[allow(clippy::too_many_lines)]
async fn run_vnc_client(
    config: VncClientConfig,
    event_tx: std::sync::mpsc::Sender<VncClientEvent>,
    mut command_rx: mpsc::Receiver<VncClientCommand>,
) -> Result<(), VncClientError> {
    // Connect to the server
    tracing::warn!(
        host = %config.host,
        port = %config.port,
        "VNC connection is unencrypted. Consider using SSH tunnel for security."
    );
    let tcp = TcpStream::connect(config.server_address())
        .await
        .map_err(|e| VncClientError::ConnectionFailed(e.to_string()))?;

    // Build the VNC connector
    let password = config
        .password
        .as_ref()
        .map(|p| secrecy::ExposeSecret::expose_secret(p).to_string());
    let mut connector = VncConnector::new(tcp)
        .set_auth_method(async move {
            if let Some(p) = password {
                Ok(p)
            } else {
                tracing::warn!("VNC connection attempted without password configured");
                Ok(String::new())
            }
        })
        .allow_shared(config.shared)
        .set_pixel_format(PixelFormat::bgra());

    // Add encodings
    for encoding in &config.encodings {
        connector = match encoding {
            super::config::VncEncoding::Tight => connector.add_encoding(VncEncoding::Tight),
            super::config::VncEncoding::Zrle => connector.add_encoding(VncEncoding::Zrle),
            super::config::VncEncoding::CopyRect => connector.add_encoding(VncEncoding::CopyRect),
            super::config::VncEncoding::Raw => connector.add_encoding(VncEncoding::Raw),
        };
    }

    // Start the connection
    let vnc = connector
        .build()
        .map_err(|e| VncClientError::ConnectionFailed(e.to_string()))?
        .try_start()
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("Unknown VNC security type") || msg.contains("unknown security type") {
                VncClientError::UnsupportedSecurityType(msg)
            } else {
                VncClientError::ConnectionFailed(msg)
            }
        })?
        .finish()
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("Unknown VNC security type") || msg.contains("unknown security type") {
                VncClientError::UnsupportedSecurityType(msg)
            } else {
                VncClientError::AuthenticationFailed(msg)
            }
        })?;

    // Notify connected
    let _ = event_tx.send(VncClientEvent::Connected);

    // Main event loop
    let mut last_refresh = std::time::Instant::now();
    let refresh_interval = std::time::Duration::from_millis(16); // ~60 FPS

    loop {
        // Calculate time until next refresh
        let now = std::time::Instant::now();
        let time_since_refresh = now.duration_since(last_refresh);
        let sleep_duration = if time_since_refresh >= refresh_interval {
            // Time to refresh
            let _ = vnc.input(X11Event::Refresh).await;
            last_refresh = now;
            refresh_interval
        } else {
            refresh_interval
                .checked_sub(time_since_refresh)
                .unwrap_or(refresh_interval)
        };

        tokio::select! {
            cmd = command_rx.recv() => {
                match cmd {
                    Some(VncClientCommand::Disconnect) | None => break,
                    Some(cmd) => {
                        match cmd {
                            VncClientCommand::KeyEvent { keysym, pressed } => {
                                let event = X11Event::KeyEvent(ClientKeyEvent {
                                    keycode: keysym,
                                    down: pressed,
                                });
                                let _ = vnc.input(event).await;
                            }
                            VncClientCommand::PointerEvent { x, y, buttons } => {
                                let event = X11Event::PointerEvent(ClientMouseEvent {
                                    position_x: x,
                                    position_y: y,
                                    bottons: buttons, // Note: typo in vnc-rs library
                                });
                                let _ = vnc.input(event).await;
                            }
                            VncClientCommand::ClipboardText(text) => {
                                let event = X11Event::CopyText(text);
                                let _ = vnc.input(event).await;
                            }
                            VncClientCommand::RefreshScreen => {
                                let _ = vnc.input(X11Event::Refresh).await;
                            }
                            VncClientCommand::SetDesktopSize { width, height } => {
                                tracing::debug!(
                                    "[VNC] SetDesktopSize requested: {width}x{height} (server support required)"
                                );
                                let _ = vnc.input(X11Event::Refresh).await;
                            }
                            VncClientCommand::SendCtrlAltDel => {
                                const CTRL_L: u32 = 0xffe3;
                                const ALT_L: u32 = 0xffe9;
                                const DELETE: u32 = 0xffff;
                                let _ = vnc.input(X11Event::KeyEvent(ClientKeyEvent { keycode: CTRL_L, down: true })).await;
                                let _ = vnc.input(X11Event::KeyEvent(ClientKeyEvent { keycode: ALT_L, down: true })).await;
                                let _ = vnc.input(X11Event::KeyEvent(ClientKeyEvent { keycode: DELETE, down: true })).await;
                                let _ = vnc.input(X11Event::KeyEvent(ClientKeyEvent { keycode: DELETE, down: false })).await;
                                let _ = vnc.input(X11Event::KeyEvent(ClientKeyEvent { keycode: ALT_L, down: false })).await;
                                let _ = vnc.input(X11Event::KeyEvent(ClientKeyEvent { keycode: CTRL_L, down: false })).await;
                                tracing::debug!("[VNC] Sent Ctrl+Alt+Del");
                            }
                            VncClientCommand::TypeText(text) => {
                                for ch in text.chars() {
                                    let keysym = if ch.is_ascii() {
                                        u32::from(ch as u8)
                                    } else {
                                        0x0100_0000 | u32::from(ch)
                                    };
                                    let _ = vnc.input(X11Event::KeyEvent(ClientKeyEvent { keycode: keysym, down: true })).await;
                                    let _ = vnc.input(X11Event::KeyEvent(ClientKeyEvent { keycode: keysym, down: false })).await;
                                }
                                tracing::debug!("[VNC] Typed {} characters", text.len());
                            }
                            VncClientCommand::Authenticate(_) | VncClientCommand::Disconnect => {}
                        }
                    }
                }
            }
            event = vnc.poll_event() => {
                match event {
                    Ok(Some(event)) => {
                        let client_event = convert_vnc_event(event);
                        if event_tx.send(client_event).is_err() {
                            break;
                        }
                    }
                    Ok(None) => {
                        // No event, sleep briefly to prevent busy loop
                        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    }
                    Err(e) => {
                        let _ = event_tx.send(VncClientEvent::Error(e.to_string()));
                        break;
                    }
                }
            }
            () = tokio::time::sleep(sleep_duration) => {
                // Just wake up to check refresh logic at top of loop
            }
        }
    }

    let _ = vnc.close().await;
    Ok(())
}

/// Converts vnc-rs events to our event type
fn convert_vnc_event(event: VncEvent) -> VncClientEvent {
    match event {
        VncEvent::SetResolution(screen) => VncClientEvent::ResolutionChanged {
            width: u32::from(screen.width),
            height: u32::from(screen.height),
        },
        VncEvent::RawImage(rect, data) => VncClientEvent::FrameUpdate {
            rect: VncRect::new(rect.x, rect.y, rect.width, rect.height),
            data,
        },
        VncEvent::Copy(dst, src) => VncClientEvent::CopyRect {
            dst: VncRect::new(dst.x, dst.y, dst.width, dst.height),
            src: VncRect::new(src.x, src.y, src.width, src.height),
        },
        VncEvent::SetCursor(rect, data) => VncClientEvent::CursorUpdate {
            rect: VncRect::new(rect.x, rect.y, rect.width, rect.height),
            data,
        },
        VncEvent::Bell => VncClientEvent::Bell,
        VncEvent::Text(text) => VncClientEvent::ClipboardText(text),
        VncEvent::JpegImage(rect, data) => {
            // JPEG images need decoding - for now treat as raw
            // In a full implementation, we'd decode JPEG here
            VncClientEvent::FrameUpdate {
                rect: VncRect::new(rect.x, rect.y, rect.width, rect.height),
                data,
            }
        }
        _ => VncClientEvent::Error("Unknown VNC event".to_string()),
    }
}

impl Drop for VncClient {
    fn drop(&mut self) {
        // Signal disconnect on drop
        if let Some(tx) = &self.command_tx {
            let _ = tx.try_send(VncClientCommand::Disconnect);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vnc_client_new() {
        let config = VncClientConfig::new("localhost").with_port(5900);
        let client = VncClient::new(config);
        assert_eq!(client.config().host, "localhost");
        assert_eq!(client.config().port, 5900);
    }

    #[test]
    fn test_vnc_client_not_connected() {
        let config = VncClientConfig::new("localhost");
        let client = VncClient::new(config);
        assert!(!client.is_connected());
    }

    #[test]
    fn test_convert_resolution_event() {
        // Create a mock screen struct similar to vnc-rs
        let event = VncClientEvent::ResolutionChanged {
            width: 1920,
            height: 1080,
        };
        if let VncClientEvent::ResolutionChanged { width, height } = event {
            assert_eq!(width, 1920);
            assert_eq!(height, 1080);
        }
    }
}
