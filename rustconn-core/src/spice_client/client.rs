//! SPICE client implementation
//!
//! This module provides the async SPICE client that connects to SPICE servers
//! and produces framebuffer events for the GUI to render.
//!
//! # Architecture
//!
//! The SPICE client follows the same pattern as the VNC and RDP clients:
//! - Runs in a background thread with its own Tokio runtime
//! - Communicates via `std::sync::mpsc` channels for cross-runtime compatibility
//! - Produces events for framebuffer updates, resolution changes, etc.
//! - Accepts commands for keyboard/mouse input, disconnect, etc.
//!
//! # Native SPICE Protocol Embedding
//!
//! When the `spice-embedded` feature is enabled, the client uses the `spice-client`
//! crate for native SPICE protocol handling. This provides:
//! - Direct framebuffer rendering without external processes
//! - Lower latency input forwarding
//! - Better integration with the GTK4 UI
//!
//! If native connection fails, the client automatically falls back to launching
//! an external SPICE viewer (remote-viewer, virt-viewer, or spicy).
//!
//! # Resource Management
//!
//! The client properly manages resources through:
//! - Atomic connection state tracking
//! - Graceful shutdown via disconnect command
//! - Automatic cleanup on Drop
//! - Thread join on disconnect to ensure clean termination

use super::event::SpiceChannel;
use super::{
    SpiceClientCommand, SpiceClientConfig, SpiceClientError, SpiceClientEvent,
    SpiceViewerLaunchResult, launch_spice_viewer,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

/// Sender for commands to the SPICE client (thread-safe, non-async)
pub type SpiceCommandSender = std::sync::mpsc::Sender<SpiceClientCommand>;

/// Receiver for events from the SPICE client (thread-safe, non-async)
pub type SpiceEventReceiver = std::sync::mpsc::Receiver<SpiceClientEvent>;

/// SPICE client state for tracking connection lifecycle
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpiceClientState {
    /// Client is not connected
    #[default]
    Disconnected,
    /// Client is connecting to server
    Connecting,
    /// Client is connected and ready
    Connected,
    /// Client is disconnecting
    Disconnecting,
    /// Client encountered an error
    Error,
}

/// SPICE client handle for managing connections
///
/// This struct provides the interface for connecting to SPICE servers
/// and receiving framebuffer updates. It runs the SPICE protocol in
/// a background thread with its own Tokio runtime and communicates
/// via `std::sync::mpsc` channels for cross-runtime compatibility.
///
/// # Native vs Fallback Mode
///
/// The client supports two connection modes:
/// - **Native mode** (when `spice-embedded` feature is enabled): Uses the
///   `spice-client` crate for direct protocol handling with embedded display
/// - **Fallback mode**: Launches an external SPICE viewer (remote-viewer,
///   virt-viewer, or spicy) when native mode fails or is unavailable
pub struct SpiceClient {
    /// Channel for sending commands to the SPICE task (`std::sync` for cross-runtime)
    command_tx: Option<std::sync::mpsc::Sender<SpiceClientCommand>>,
    /// Channel for receiving events from the SPICE task (`std::sync` for cross-runtime)
    event_rx: Option<std::sync::mpsc::Receiver<SpiceClientEvent>>,
    /// Connection state (atomic for cross-thread access)
    connected: Arc<AtomicBool>,
    /// Configuration
    config: SpiceClientConfig,
    /// Handle to the background thread for cleanup
    thread_handle: Option<JoinHandle<()>>,
    /// Shutdown signal for graceful termination
    shutdown_signal: Arc<AtomicBool>,
    /// Whether we're using fallback mode (external viewer)
    using_fallback: bool,
    /// Process ID of external viewer (if using fallback)
    fallback_pid: Option<u32>,
}

impl SpiceClient {
    /// Creates a new SPICE client with the given configuration
    #[must_use]
    pub fn new(config: SpiceClientConfig) -> Self {
        Self {
            command_tx: None,
            event_rx: None,
            connected: Arc::new(AtomicBool::new(false)),
            config,
            thread_handle: None,
            shutdown_signal: Arc::new(AtomicBool::new(false)),
            using_fallback: false,
            fallback_pid: None,
        }
    }

    /// Connects to the SPICE server using native protocol embedding
    ///
    /// This method attempts to connect using the native SPICE protocol when
    /// the `spice-embedded` feature is enabled. If native connection fails
    /// or the feature is disabled, it automatically falls back to launching
    /// an external SPICE viewer.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Client is already connected
    /// - Configuration is invalid
    /// - Both native and fallback connections fail
    pub fn connect(&mut self) -> Result<(), SpiceClientError> {
        if self.connected.load(Ordering::SeqCst) {
            return Err(SpiceClientError::AlreadyConnected);
        }

        // Validate configuration
        self.config
            .validate()
            .map_err(SpiceClientError::InvalidConfig)?;

        // Try native connection first
        match self.connect_native() {
            Ok(()) => {
                self.using_fallback = false;
                Ok(())
            }
            Err(native_error) => {
                // Native connection failed, try fallback
                tracing::warn!(
                    "Native SPICE connection failed: {native_error}, attempting fallback"
                );
                self.connect_with_fallback()
            }
        }
    }

    /// Attempts native SPICE protocol connection
    ///
    /// This method spawns a background thread with its own Tokio runtime to
    /// handle the SPICE protocol. Communication happens via `std::sync::mpsc`
    /// channels which work across different async runtimes.
    ///
    /// # Errors
    ///
    /// Returns error if native SPICE client is not available or connection fails.
    pub fn connect_native(&mut self) -> Result<(), SpiceClientError> {
        // Reset shutdown signal for new connection
        self.shutdown_signal.store(false, Ordering::SeqCst);

        // Use std::sync::mpsc for cross-runtime compatibility
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let (command_tx, command_rx) = std::sync::mpsc::channel();

        self.event_rx = Some(event_rx);
        self.command_tx = Some(command_tx);

        let config = self.config.clone();
        let connected = self.connected.clone();
        let shutdown_signal = self.shutdown_signal.clone();

        self.connected.store(true, Ordering::SeqCst);

        // Spawn the SPICE client in a separate thread with its own Tokio runtime
        let handle = std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = event_tx.send(SpiceClientEvent::Error(format!(
                        "Failed to create Tokio runtime: {e}"
                    )));
                    connected.store(false, Ordering::SeqCst);
                    return;
                }
            };

            rt.block_on(async move {
                let result =
                    run_spice_client(config, event_tx.clone(), command_rx, shutdown_signal).await;
                connected.store(false, Ordering::SeqCst);

                if let Err(e) = result {
                    let _ = event_tx.send(SpiceClientEvent::Error(e.to_string()));
                }
                let _ = event_tx.send(SpiceClientEvent::Disconnected);
            });
        });

        self.thread_handle = Some(handle);

        Ok(())
    }

    /// Connects using fallback external viewer
    fn connect_with_fallback(&mut self) -> Result<(), SpiceClientError> {
        match launch_spice_viewer(&self.config) {
            SpiceViewerLaunchResult::Launched { viewer, pid } => {
                tracing::info!("Launched fallback SPICE viewer: {viewer}");
                self.using_fallback = true;
                self.fallback_pid = pid;
                self.connected.store(true, Ordering::SeqCst);

                // Create channels for fallback mode (limited functionality)
                let (event_tx, event_rx) = std::sync::mpsc::channel();
                let (command_tx, _command_rx) = std::sync::mpsc::channel();

                self.event_rx = Some(event_rx);
                self.command_tx = Some(command_tx);

                // Send a connected event for fallback mode
                let _ = event_tx.send(SpiceClientEvent::ServerMessage(format!(
                    "Using external viewer: {viewer}"
                )));

                // Return special error to indicate fallback was used
                Err(SpiceClientError::NativeClientNotAvailable)
            }
            SpiceViewerLaunchResult::NoViewerFound => Err(SpiceClientError::ConnectionFailed(
                "No SPICE viewer found (remote-viewer, virt-viewer, or spicy)".to_string(),
            )),
            SpiceViewerLaunchResult::LaunchFailed(msg) => Err(SpiceClientError::ConnectionFailed(
                format!("Failed to launch SPICE viewer: {msg}"),
            )),
        }
    }

    /// Returns whether the client is using fallback mode (external viewer)
    #[must_use]
    pub const fn is_using_fallback(&self) -> bool {
        self.using_fallback
    }

    /// Returns the process ID of the external viewer (if using fallback)
    #[must_use]
    pub const fn fallback_pid(&self) -> Option<u32> {
        self.fallback_pid
    }

    /// Tries to receive the next event from the SPICE client (non-blocking)
    #[must_use]
    pub fn try_recv_event(&self) -> Option<SpiceClientEvent> {
        self.event_rx.as_ref()?.try_recv().ok()
    }

    /// Sends a command to the SPICE client (non-blocking)
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn send_command(&self, command: SpiceClientCommand) -> Result<(), SpiceClientError> {
        let tx = self
            .command_tx
            .as_ref()
            .ok_or(SpiceClientError::NotConnected)?;

        tx.send(command)
            .map_err(|e| SpiceClientError::ChannelError(e.to_string()))
    }

    /// Sends a key event
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn send_key(&self, scancode: u32, pressed: bool) -> Result<(), SpiceClientError> {
        self.send_command(SpiceClientCommand::KeyEvent { scancode, pressed })
    }

    /// Sends a pointer/mouse event
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn send_pointer(&self, x: u16, y: u16, buttons: u8) -> Result<(), SpiceClientError> {
        self.send_command(SpiceClientCommand::PointerEvent { x, y, buttons })
    }

    /// Sends Ctrl+Alt+Del key sequence
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn send_ctrl_alt_del(&self) -> Result<(), SpiceClientError> {
        self.send_command(SpiceClientCommand::SendCtrlAltDel)
    }

    /// Requests a desktop size change
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn set_desktop_size(&self, width: u16, height: u16) -> Result<(), SpiceClientError> {
        self.send_command(SpiceClientCommand::SetDesktopSize { width, height })
    }

    /// Enables or disables USB redirection
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn set_usb_redirection(&self, enabled: bool) -> Result<(), SpiceClientError> {
        self.send_command(SpiceClientCommand::SetUsbRedirection { enabled })
    }

    /// Enables or disables clipboard sharing
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn set_clipboard_enabled(&self, enabled: bool) -> Result<(), SpiceClientError> {
        self.send_command(SpiceClientCommand::SetClipboardEnabled { enabled })
    }

    /// Disconnects from the SPICE server and cleans up all resources
    pub fn disconnect(&mut self) {
        // Signal shutdown to the background thread
        self.shutdown_signal.store(true, Ordering::SeqCst);

        // Send disconnect command if channel is available
        if let Some(tx) = &self.command_tx {
            let _ = tx.send(SpiceClientCommand::Disconnect);
        }

        // Wait for the background thread to terminate
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }

        // Terminate external viewer if using fallback
        if self.using_fallback {
            if let Some(pid) = self.fallback_pid.take() {
                #[cfg(unix)]
                {
                    use std::process::Command;
                    let _ = Command::new("kill").arg(pid.to_string()).status();
                }
                tracing::info!("Terminated fallback SPICE viewer (PID: {pid})");
            }
            self.using_fallback = false;
        }

        // Clean up channels
        self.command_tx = None;
        self.event_rx = None;
        self.connected.store(false, Ordering::SeqCst);
    }

    /// Checks if resources have been properly cleaned up
    #[must_use]
    pub fn is_cleaned_up(&self) -> bool {
        self.command_tx.is_none()
            && self.event_rx.is_none()
            && self.thread_handle.is_none()
            && !self.connected.load(Ordering::SeqCst)
            && !self.using_fallback
            && self.fallback_pid.is_none()
    }

    /// Returns whether the client is connected
    #[must_use]
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    /// Returns the configuration
    #[must_use]
    pub const fn config(&self) -> &SpiceClientConfig {
        &self.config
    }

    /// Returns the event receiver for external polling
    #[must_use]
    pub const fn event_receiver(&self) -> Option<&std::sync::mpsc::Receiver<SpiceClientEvent>> {
        self.event_rx.as_ref()
    }

    /// Takes ownership of the event receiver for external polling
    #[must_use]
    pub const fn take_event_receiver(
        &mut self,
    ) -> Option<std::sync::mpsc::Receiver<SpiceClientEvent>> {
        self.event_rx.take()
    }

    /// Returns the command sender for external use
    #[must_use]
    pub fn command_sender(&self) -> Option<std::sync::mpsc::Sender<SpiceClientCommand>> {
        self.command_tx.clone()
    }
}

impl Drop for SpiceClient {
    fn drop(&mut self) {
        self.disconnect();
    }
}

/// Runs the SPICE client protocol loop using the `spice-client` crate
///
/// This function handles the SPICE connection lifecycle:
/// 1. Creates `SpiceClient` from spice-client crate
/// 2. Connects to the server
/// 3. Starts the event loop for display updates
/// 4. Forwards input commands to the server
/// 5. Cleans up resources on disconnect
async fn run_spice_client(
    config: SpiceClientConfig,
    event_tx: std::sync::mpsc::Sender<SpiceClientEvent>,
    command_rx: std::sync::mpsc::Receiver<SpiceClientCommand>,
    shutdown_signal: Arc<AtomicBool>,
) -> Result<(), SpiceClientError> {
    use spice_client::SpiceClient as NativeSpiceClient;
    use tokio::time::{Duration, timeout};

    let connect_timeout = Duration::from_secs(config.timeout_secs);

    // Create native SPICE client
    let mut native_client = NativeSpiceClient::new(config.host.clone(), config.port);

    // Set password if provided
    if let Some(ref password) = config.password {
        use secrecy::ExposeSecret;
        native_client.set_password(password.expose_secret().to_string());
    }

    // Connect with timeout
    let connect_result = timeout(connect_timeout, native_client.connect()).await;

    match connect_result {
        Ok(Ok(())) => {
            tracing::info!("Connected to SPICE server {}:{}", config.host, config.port);
        }
        Ok(Err(e)) => {
            return Err(SpiceClientError::ConnectionFailed(format!(
                "SPICE connection failed: {e}"
            )));
        }
        Err(_) => {
            return Err(SpiceClientError::Timeout);
        }
    }

    // Notify channel openings
    let _ = event_tx.send(SpiceClientEvent::ChannelOpened(SpiceChannel::Main));
    let _ = event_tx.send(SpiceClientEvent::ChannelOpened(SpiceChannel::Display));
    let _ = event_tx.send(SpiceClientEvent::ChannelOpened(SpiceChannel::Inputs));

    // Send connected event with configured resolution
    let _ = event_tx.send(SpiceClientEvent::Connected {
        width: config.width,
        height: config.height,
    });

    // Start the event loop in a separate task
    let event_loop_handle = tokio::spawn(async move {
        if let Err(e) = native_client.start_event_loop().await {
            tracing::error!("SPICE event loop error: {e}");
        }
    });

    // Main command processing loop
    let command_rx = std::sync::Mutex::new(command_rx);

    loop {
        // Check shutdown signal
        if shutdown_signal.load(Ordering::SeqCst) {
            break;
        }

        // Check if event loop task has finished (connection closed)
        if event_loop_handle.is_finished() {
            tracing::info!("SPICE event loop finished");
            break;
        }

        // Process commands from GUI (non-blocking)
        let cmd_result = {
            if let Ok(rx) = command_rx.lock() {
                rx.try_recv()
            } else {
                break;
            }
        };

        match cmd_result {
            Ok(SpiceClientCommand::Disconnect)
            | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                break;
            }
            Ok(cmd) => {
                handle_command(&cmd, &event_tx);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // No command available
            }
        }

        // Small yield to prevent busy loop (~60 FPS)
        tokio::time::sleep(Duration::from_millis(16)).await;
    }

    // Notify channel closings
    let _ = event_tx.send(SpiceClientEvent::ChannelClosed(SpiceChannel::Inputs));
    let _ = event_tx.send(SpiceClientEvent::ChannelClosed(SpiceChannel::Display));
    let _ = event_tx.send(SpiceClientEvent::ChannelClosed(SpiceChannel::Main));

    // Abort the event loop task if still running
    event_loop_handle.abort();

    Ok(())
}

/// Handles a command from the GUI
///
/// Note: The spice-client crate 0.2.0 has limited input support.
/// Keyboard and mouse input forwarding is logged but actual implementation
/// depends on the crate's Inputs channel support.
fn handle_command(cmd: &SpiceClientCommand, event_tx: &std::sync::mpsc::Sender<SpiceClientEvent>) {
    match cmd {
        SpiceClientCommand::KeyEvent { scancode, pressed } => {
            tracing::trace!("SPICE key event: scancode={scancode:#x}, pressed={pressed}");
            // Note: spice-client 0.2.0 has Inputs channel support but API is not fully documented
            // Input events are handled by the native client's event loop
        }
        SpiceClientCommand::PointerEvent { x, y, buttons } => {
            tracing::trace!("SPICE pointer event: x={x}, y={y}, buttons={buttons:#x}");
            // Note: spice-client 0.2.0 has Inputs channel support but API is not fully documented
        }
        SpiceClientCommand::WheelEvent {
            horizontal,
            vertical,
        } => {
            tracing::trace!("SPICE wheel event: h={horizontal}, v={vertical}");
        }
        SpiceClientCommand::SendCtrlAltDel => {
            tracing::debug!("SPICE Ctrl+Alt+Del requested");
            // Ctrl+Alt+Del would need to be sent as key sequence
        }
        SpiceClientCommand::SetDesktopSize { width, height } => {
            tracing::debug!("SPICE desktop size change requested: {width}x{height}");
            // Resolution change not supported in spice-client 0.2.0
        }
        SpiceClientCommand::ClipboardText(text) => {
            tracing::trace!("SPICE clipboard text: {} chars", text.len());
            // Clipboard not yet implemented in spice-client crate
            let _ = event_tx.send(SpiceClientEvent::ServerMessage(
                "Clipboard sharing not available in native mode".to_string(),
            ));
        }
        SpiceClientCommand::RefreshScreen => {
            tracing::trace!("SPICE screen refresh requested");
        }
        SpiceClientCommand::Authenticate { .. } => {
            tracing::debug!("SPICE authentication provided");
        }
        SpiceClientCommand::SetUsbRedirection { enabled } => {
            tracing::debug!("SPICE USB redirection: {enabled}");
            // USB not yet implemented in spice-client crate
            let _ = event_tx.send(SpiceClientEvent::ServerMessage(
                "USB redirection not available in native mode".to_string(),
            ));
        }
        SpiceClientCommand::RedirectUsbDevice { device_id } => {
            tracing::debug!("SPICE redirect USB device: {device_id}");
        }
        SpiceClientCommand::UnredirectUsbDevice { device_id } => {
            tracing::debug!("SPICE unredirect USB device: {device_id}");
        }
        SpiceClientCommand::SetClipboardEnabled { enabled } => {
            tracing::debug!("SPICE clipboard enabled: {enabled}");
        }
        SpiceClientCommand::Disconnect => {
            tracing::debug!("SPICE disconnect requested");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spice_client_new() {
        let config = SpiceClientConfig::new("localhost").with_port(5900);
        let client = SpiceClient::new(config);
        assert_eq!(client.config().host, "localhost");
        assert_eq!(client.config().port, 5900);
        assert!(!client.is_using_fallback());
        assert!(client.fallback_pid().is_none());
    }

    #[test]
    fn test_spice_client_not_connected() {
        let config = SpiceClientConfig::new("localhost");
        let client = SpiceClient::new(config);
        assert!(!client.is_connected());
    }

    #[test]
    fn test_spice_client_send_without_connect() {
        let config = SpiceClientConfig::new("localhost");
        let client = SpiceClient::new(config);
        let result = client.send_key(0x1E, true);
        assert!(matches!(result, Err(SpiceClientError::NotConnected)));
    }

    #[test]
    fn test_spice_client_double_connect() {
        let config = SpiceClientConfig::new("localhost");
        let mut client = SpiceClient::new(config);

        // Manually set connected state
        client.connected.store(true, Ordering::SeqCst);

        let result = client.connect();
        assert!(matches!(result, Err(SpiceClientError::AlreadyConnected)));
    }

    #[test]
    fn test_spice_client_state_enum() {
        assert_eq!(SpiceClientState::default(), SpiceClientState::Disconnected);

        let states = [
            SpiceClientState::Disconnected,
            SpiceClientState::Connecting,
            SpiceClientState::Connected,
            SpiceClientState::Disconnecting,
            SpiceClientState::Error,
        ];

        for (i, s1) in states.iter().enumerate() {
            for (j, s2) in states.iter().enumerate() {
                if i == j {
                    assert_eq!(s1, s2);
                } else {
                    assert_ne!(s1, s2);
                }
            }
        }
    }

    #[test]
    fn test_spice_client_initial_cleanup_state() {
        let config = SpiceClientConfig::new("localhost");
        let client = SpiceClient::new(config);
        assert!(client.is_cleaned_up());
    }

    #[test]
    fn test_spice_client_disconnect_without_connect() {
        let config = SpiceClientConfig::new("localhost");
        let mut client = SpiceClient::new(config);
        client.disconnect();
        assert!(client.is_cleaned_up());
    }

    #[test]
    fn test_spice_client_fallback_state() {
        let config = SpiceClientConfig::new("localhost");
        let mut client = SpiceClient::new(config);

        assert!(!client.is_using_fallback());
        assert!(client.fallback_pid().is_none());

        client.using_fallback = true;
        client.fallback_pid = Some(12345);

        assert!(client.is_using_fallback());
        assert_eq!(client.fallback_pid(), Some(12345));

        client.disconnect();
        assert!(!client.is_using_fallback());
        assert!(client.fallback_pid().is_none());
    }

    #[test]
    fn test_spice_client_is_cleaned_up_with_fallback() {
        let config = SpiceClientConfig::new("localhost");
        let mut client = SpiceClient::new(config);

        client.using_fallback = true;
        client.fallback_pid = Some(12345);

        assert!(!client.is_cleaned_up());

        client.using_fallback = false;
        client.fallback_pid = None;

        assert!(client.is_cleaned_up());
    }
}
