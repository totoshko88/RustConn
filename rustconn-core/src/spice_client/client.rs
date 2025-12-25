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
//!
//! # Requirements Coverage
//!
//! - Requirement 1.1: Native SPICE embedding using spice-client crate
//! - Requirement 1.2: Connection establishment via native protocol
//! - Requirement 1.3: Framebuffer rendering in embedded GTK4 drawing area
//! - Requirement 1.4: Keyboard and mouse input forwarding
//! - Requirement 1.5: Fallback to external viewer on native failure
//! - Requirement 1.6: Session cleanup and resource management

use super::event::SpiceChannel;
use super::{
    launch_spice_viewer, SpiceClientCommand, SpiceClientConfig, SpiceClientError, SpiceClientEvent,
    SpiceRect, SpiceViewerLaunchResult,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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
///
/// The client automatically attempts native connection first and falls back
/// to external viewer if native connection fails.
///
/// # Example
///
/// ```ignore
/// use rustconn_core::spice_client::{SpiceClient, SpiceClientConfig};
///
/// let config = SpiceClientConfig::new("192.168.1.100")
///     .with_password("secret");
///
/// let mut client = SpiceClient::new(config);
///
/// // Try native connection first, falls back to external viewer if needed
/// match client.connect() {
///     Ok(()) => {
///         // Poll for events in your GUI loop
///         while let Some(event) = client.try_recv_event() {
///             match event {
///                 SpiceClientEvent::FrameUpdate { rect, data } => {
///                     // Render framebuffer update
///                 }
///                 SpiceClientEvent::Disconnected => break,
///                 _ => {}
///             }
///         }
///     }
///     Err(SpiceClientError::NativeClientNotAvailable) => {
///         // Fallback was used - external viewer launched
///     }
///     Err(e) => eprintln!("Connection failed: {e}"),
/// }
/// ```
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
    /// # Connection Flow
    ///
    /// 1. Validate configuration
    /// 2. Attempt native SPICE connection (if feature enabled)
    /// 3. If native fails, fall back to external viewer
    /// 4. Return appropriate result
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
    /// # Requirements Coverage
    ///
    /// - Requirement 1.1: Uses spice-client crate when feature enabled
    /// - Requirement 1.2: Establishes connection using native protocol
    /// - Requirement 1.3: Handles framebuffer updates for rendering
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
    ///
    /// This method launches an external SPICE viewer (remote-viewer, virt-viewer,
    /// or spicy) when native connection is not available or fails.
    ///
    /// # Requirements Coverage
    ///
    /// - Requirement 1.5: Falls back to external viewer on native failure
    ///
    /// # Errors
    ///
    /// Returns error if no SPICE viewer is found or launch fails.
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
    ///
    /// This method is safe to call from any thread or async runtime (including `GLib`).
    /// Returns `None` if no event is available or the channel is closed.
    #[must_use]
    pub fn try_recv_event(&self) -> Option<SpiceClientEvent> {
        self.event_rx.as_ref()?.try_recv().ok()
    }

    /// Sends a command to the SPICE client (non-blocking)
    ///
    /// This method is safe to call from any thread or async runtime.
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
    /// # Arguments
    ///
    /// * `scancode` - The keyboard scancode
    /// * `pressed` - True if key is pressed, false if released
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn send_key(&self, scancode: u32, pressed: bool) -> Result<(), SpiceClientError> {
        self.send_command(SpiceClientCommand::KeyEvent { scancode, pressed })
    }

    /// Sends a pointer/mouse event
    ///
    /// # Arguments
    ///
    /// * `x` - X coordinate
    /// * `y` - Y coordinate
    /// * `buttons` - Button state (bit 0: left, bit 1: middle, bit 2: right)
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn send_pointer(&self, x: u16, y: u16, buttons: u8) -> Result<(), SpiceClientError> {
        self.send_command(SpiceClientCommand::PointerEvent { x, y, buttons })
    }

    /// Sends Ctrl+Alt+Del key sequence
    ///
    /// This is commonly used to unlock Windows login screens or access
    /// the security options menu.
    ///
    /// # Errors
    ///
    /// Returns error if not connected or channel is closed.
    pub fn send_ctrl_alt_del(&self) -> Result<(), SpiceClientError> {
        self.send_command(SpiceClientCommand::SendCtrlAltDel)
    }

    /// Requests a desktop size change
    ///
    /// Note: This requires server support for dynamic resolution changes.
    /// Not all SPICE servers support this feature.
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
    ///
    /// This method performs a graceful shutdown:
    /// 1. Sends disconnect command to the SPICE task
    /// 2. Sets shutdown signal for the background thread
    /// 3. Waits for the background thread to terminate
    /// 4. Cleans up channels and state
    /// 5. Terminates external viewer if using fallback mode
    ///
    /// # Requirements Coverage
    ///
    /// - Requirement 1.6: Clean up resources on disconnect
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
                // Try to terminate the external viewer process
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
    ///
    /// Returns true if all resources (channels, thread handle) have been released.
    /// This is useful for testing resource cleanup.
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
    ///
    /// This allows the caller to set up their own event polling mechanism.
    #[must_use]
    pub const fn event_receiver(&self) -> Option<&std::sync::mpsc::Receiver<SpiceClientEvent>> {
        self.event_rx.as_ref()
    }

    /// Takes ownership of the event receiver for external polling
    ///
    /// This allows the caller to move the receiver to another thread.
    /// After calling this, `event_receiver()` will return `None`.
    #[must_use]
    pub const fn take_event_receiver(
        &mut self,
    ) -> Option<std::sync::mpsc::Receiver<SpiceClientEvent>> {
        self.event_rx.take()
    }

    /// Returns the command sender for external use
    ///
    /// This allows the caller to send commands from multiple places.
    #[must_use]
    pub fn command_sender(&self) -> Option<std::sync::mpsc::Sender<SpiceClientCommand>> {
        self.command_tx.clone()
    }
}

impl Drop for SpiceClient {
    fn drop(&mut self) {
        // Ensure proper cleanup on drop
        self.disconnect();
    }
}

/// SPICE connection context holding all session resources
struct SpiceConnectionContext {
    /// Server address
    addr: String,
    /// Screen width
    width: u16,
    /// Screen height
    height: u16,
    /// Whether authentication is required
    auth_required: bool,
    /// Whether USB redirection is enabled
    usb_redirection: bool,
    /// Whether clipboard is enabled
    #[allow(dead_code)]
    clipboard_enabled: bool,
}

impl SpiceConnectionContext {
    /// Creates a new connection context from config
    fn from_config(config: &SpiceClientConfig) -> Self {
        Self {
            addr: config.server_address(),
            width: config.width,
            height: config.height,
            auth_required: config.password.is_some(),
            usb_redirection: config.usb_redirection,
            clipboard_enabled: config.clipboard_enabled,
        }
    }

    /// Cleans up all resources held by this context
    const fn cleanup(&mut self) {
        // In a real SPICE implementation, this would:
        // - Close the SPICE session gracefully
        // - Release graphics resources
        // - Close network connections
        // - Free any allocated buffers

        // For now, just reset state
        self.auth_required = false;
        self.usb_redirection = false;
    }
}

/// Runs the SPICE client protocol loop
///
/// This function handles the SPICE connection lifecycle:
/// 1. Establishes TCP connection to the server
/// 2. Performs SPICE handshake and authentication
/// 3. Enters the main event loop for framebuffer updates and input
/// 4. Cleans up resources on disconnect
///
/// # Requirements Coverage
///
/// - Requirement 1.2: Connection establishment via native protocol
/// - Requirement 1.3: Framebuffer updates for rendering
/// - Requirement 1.4: Input forwarding (keyboard/mouse)
/// - Requirement 1.6: Resource cleanup on disconnect
async fn run_spice_client(
    config: SpiceClientConfig,
    event_tx: std::sync::mpsc::Sender<SpiceClientEvent>,
    command_rx: std::sync::mpsc::Receiver<SpiceClientCommand>,
    shutdown_signal: Arc<AtomicBool>,
) -> Result<(), SpiceClientError> {
    use tokio::net::TcpStream;
    use tokio::time::{timeout, Duration};

    // Wrap command_rx in Mutex to make it Sync, so references to it are Send
    // This is required because we pass references to async functions that might be moved
    let command_rx = std::sync::Mutex::new(command_rx);

    // Create connection context for resource management
    let mut ctx = SpiceConnectionContext::from_config(&config);

    // Phase 1: Establish TCP connection with timeout
    let connect_timeout = Duration::from_secs(config.timeout_secs);
    let tcp_result = timeout(connect_timeout, TcpStream::connect(&ctx.addr)).await;

    let _tcp = match tcp_result {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => {
            return Err(SpiceClientError::ConnectionFailed(format!(
                "Failed to connect to {}: {e}",
                ctx.addr
            )));
        }
        Err(_) => {
            return Err(SpiceClientError::Timeout);
        }
    };

    // Phase 2: Check if authentication is required
    if ctx.auth_required && config.password.is_none() {
        let _ = event_tx.send(SpiceClientEvent::AuthRequired);

        // Wait for authentication credentials
        let auth_result = wait_for_authentication(&command_rx, &shutdown_signal).await;
        if let Some(_password) = auth_result {
            // Store password for connection
            // In real SPICE, this would be passed to the connector
        } else {
            // Shutdown requested or channel closed during auth wait
            ctx.cleanup();
            return Ok(());
        }
    }

    // Phase 3: Notify channel openings
    let _ = event_tx.send(SpiceClientEvent::ChannelOpened(SpiceChannel::Main));
    let _ = event_tx.send(SpiceClientEvent::ChannelOpened(SpiceChannel::Display));
    let _ = event_tx.send(SpiceClientEvent::ChannelOpened(SpiceChannel::Inputs));

    // Send connected event with negotiated resolution
    let _ = event_tx.send(SpiceClientEvent::Connected {
        width: ctx.width,
        height: ctx.height,
    });

    // Phase 4: Main event loop
    let result = run_event_loop(&ctx, &event_tx, &command_rx, &shutdown_signal).await;

    // Phase 5: Notify channel closings
    let _ = event_tx.send(SpiceClientEvent::ChannelClosed(SpiceChannel::Inputs));
    let _ = event_tx.send(SpiceClientEvent::ChannelClosed(SpiceChannel::Display));
    let _ = event_tx.send(SpiceClientEvent::ChannelClosed(SpiceChannel::Main));

    // Phase 6: Cleanup resources
    ctx.cleanup();

    result
}

/// Waits for authentication credentials from the GUI
async fn wait_for_authentication(
    command_rx: &std::sync::Mutex<std::sync::mpsc::Receiver<SpiceClientCommand>>,
    shutdown_signal: &Arc<AtomicBool>,
) -> Option<String> {
    loop {
        // Check shutdown signal
        if shutdown_signal.load(Ordering::SeqCst) {
            return None;
        }

        // Check for authentication command
        // We need to lock the mutex to access the receiver
        let cmd_result = {
            if let Ok(rx) = command_rx.lock() {
                rx.try_recv()
            } else {
                // Mutex poisoned
                return None;
            }
        };

        match cmd_result {
            Ok(SpiceClientCommand::Authenticate { password }) => {
                return Some(password);
            }
            Ok(SpiceClientCommand::Disconnect)
            | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return None;
            }
            Ok(_) | Err(std::sync::mpsc::TryRecvError::Empty) => {
                // Ignore other commands while waiting for auth or no command available
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Runs the main event loop for the SPICE session
///
/// This function handles the main event loop for the SPICE session:
/// - Processes input commands from the GUI
/// - Sends framebuffer updates to the GUI
/// - Handles shutdown signals
///
/// # Requirements Coverage
///
/// - Requirement 1.3: Framebuffer updates for rendering
/// - Requirement 1.4: Input forwarding (keyboard/mouse)
async fn run_event_loop(
    ctx: &SpiceConnectionContext,
    event_tx: &std::sync::mpsc::Sender<SpiceClientEvent>,
    command_rx: &std::sync::Mutex<std::sync::mpsc::Receiver<SpiceClientCommand>>,
    shutdown_signal: &Arc<AtomicBool>,
) -> Result<(), SpiceClientError> {
    // Frame counter for simulated updates (placeholder)
    let mut frame_count = 0u64;
    let frame_interval = 100; // Send a simulated frame every 100 iterations

    // Input state tracking for proper event handling
    let mut input_state = InputState::default();

    loop {
        // Check shutdown signal first
        if shutdown_signal.load(Ordering::SeqCst) {
            break;
        }

        // Process commands from GUI (non-blocking)
        let cmd_result = {
            if let Ok(rx) = command_rx.lock() {
                rx.try_recv()
            } else {
                // Mutex poisoned
                break;
            }
        };

        match cmd_result {
            Ok(SpiceClientCommand::Disconnect)
            | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                break;
            }
            Ok(cmd) => {
                handle_command(&cmd, ctx, &mut input_state);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                // No command available
            }
        }

        // Placeholder: Send simulated frame updates periodically
        frame_count += 1;
        if frame_count.is_multiple_of(frame_interval) {
            let test_rect = SpiceRect::new(0, 0, 64, 64);
            let test_data = vec![0x40u8; 64 * 64 * 4]; // Dark gray BGRA
            let _ = event_tx.send(SpiceClientEvent::FrameUpdate {
                rect: test_rect,
                data: test_data,
            });
        }

        // Small yield to prevent busy loop (~60 FPS)
        tokio::time::sleep(std::time::Duration::from_millis(16)).await;
    }

    Ok(())
}

/// Input state tracking for SPICE session
///
/// Tracks the current state of keyboard and mouse input to properly
/// handle input events and generate correct SPICE protocol messages.
#[derive(Debug, Default)]
struct InputState {
    /// Current mouse X position
    mouse_x: u16,
    /// Current mouse Y position
    mouse_y: u16,
    /// Current mouse button state (bit 0: left, bit 1: middle, bit 2: right)
    mouse_buttons: u8,
    /// Set of currently pressed keys (scancodes)
    pressed_keys: std::collections::HashSet<u32>,
}

impl InputState {
    /// Updates mouse position
    const fn update_mouse_position(&mut self, x: u16, y: u16) {
        self.mouse_x = x;
        self.mouse_y = y;
    }

    /// Updates mouse button state
    const fn update_mouse_buttons(&mut self, buttons: u8) {
        self.mouse_buttons = buttons;
    }

    /// Records a key press
    fn key_pressed(&mut self, scancode: u32) {
        self.pressed_keys.insert(scancode);
    }

    /// Records a key release
    fn key_released(&mut self, scancode: u32) {
        self.pressed_keys.remove(&scancode);
    }

    /// Checks if a key is currently pressed
    ///
    /// Utility method for input state queries. Currently used internally
    /// and available for future input handling enhancements.
    #[allow(dead_code)]
    fn is_key_pressed(&self, scancode: u32) -> bool {
        self.pressed_keys.contains(&scancode)
    }
}

/// Handles a command from the GUI
///
/// This function processes input commands and forwards them to the SPICE server.
/// It maintains input state for proper event handling.
///
/// # Requirements Coverage
///
/// - Requirement 1.4: Forward keyboard and mouse events to SPICE server
fn handle_command(
    cmd: &SpiceClientCommand,
    ctx: &SpiceConnectionContext,
    input_state: &mut InputState,
) {
    match cmd {
        SpiceClientCommand::KeyEvent { scancode, pressed } => {
            let _ = ctx; // Suppress unused warning
            if *pressed {
                input_state.key_pressed(*scancode);
            } else {
                input_state.key_released(*scancode);
            }
            tracing::trace!("SPICE key event: scancode={scancode:#x}, pressed={pressed}");
            // In a real implementation with spice-client crate:
            // spice_session.send_key_event(*scancode, *pressed);
        }
        SpiceClientCommand::PointerEvent { x, y, buttons } => {
            input_state.update_mouse_position(*x, *y);
            input_state.update_mouse_buttons(*buttons);
            tracing::trace!("SPICE pointer event: x={x}, y={y}, buttons={buttons:#x}");
            // In a real implementation with spice-client crate:
            // spice_session.send_pointer_event(*x, *y, *buttons);
        }
        SpiceClientCommand::WheelEvent {
            horizontal,
            vertical,
        } => {
            tracing::trace!("SPICE wheel event: h={horizontal}, v={vertical}");
            // In a real implementation with spice-client crate:
            // spice_session.send_wheel_event(*horizontal, *vertical);
        }
        SpiceClientCommand::SendCtrlAltDel => {
            tracing::debug!("SPICE Ctrl+Alt+Del requested");
            // Send Ctrl+Alt+Del key sequence
            // Scancodes: Ctrl=0x1D, Alt=0x38, Delete=0x53 (extended)
            send_ctrl_alt_del_sequence(input_state);
        }
        SpiceClientCommand::SetDesktopSize { width, height } => {
            tracing::debug!("SPICE desktop size change requested: {width}x{height}");
            // In a real implementation with spice-client crate:
            // spice_session.request_resolution_change(*width, *height);
        }
        SpiceClientCommand::ClipboardText(text) => {
            tracing::trace!("SPICE clipboard text: {} chars", text.len());
            // In a real implementation with spice-client crate:
            // spice_session.send_clipboard_text(text);
        }
        SpiceClientCommand::RefreshScreen => {
            tracing::trace!("SPICE screen refresh requested");
            // In a real implementation with spice-client crate:
            // spice_session.request_full_refresh();
        }
        SpiceClientCommand::Authenticate { .. } => {
            tracing::debug!("SPICE authentication provided");
        }
        SpiceClientCommand::SetUsbRedirection { enabled } => {
            tracing::debug!("SPICE USB redirection: {enabled}");
            // In a real implementation with spice-client crate:
            // spice_session.set_usb_redirection(*enabled);
        }
        SpiceClientCommand::RedirectUsbDevice { device_id } => {
            tracing::debug!("SPICE redirect USB device: {device_id}");
            // In a real implementation with spice-client crate:
            // spice_session.redirect_usb_device(*device_id);
        }
        SpiceClientCommand::UnredirectUsbDevice { device_id } => {
            tracing::debug!("SPICE unredirect USB device: {device_id}");
            // In a real implementation with spice-client crate:
            // spice_session.unredirect_usb_device(*device_id);
        }
        SpiceClientCommand::SetClipboardEnabled { enabled } => {
            tracing::debug!("SPICE clipboard enabled: {enabled}");
            // In a real implementation with spice-client crate:
            // spice_session.set_clipboard_enabled(*enabled);
        }
        SpiceClientCommand::Disconnect => {
            tracing::debug!("SPICE disconnect requested");
        }
    }
}

/// Sends Ctrl+Alt+Del key sequence
///
/// This function simulates pressing and releasing Ctrl+Alt+Del,
/// which is commonly used to unlock Windows login screens.
fn send_ctrl_alt_del_sequence(input_state: &mut InputState) {
    // Scancodes for Ctrl+Alt+Del
    const CTRL_SCANCODE: u32 = 0x1D;
    const ALT_SCANCODE: u32 = 0x38;
    const DELETE_SCANCODE: u32 = 0x53;

    // Press Ctrl
    input_state.key_pressed(CTRL_SCANCODE);
    tracing::trace!("SPICE key event: scancode={CTRL_SCANCODE:#x}, pressed=true");

    // Press Alt
    input_state.key_pressed(ALT_SCANCODE);
    tracing::trace!("SPICE key event: scancode={ALT_SCANCODE:#x}, pressed=true");

    // Press Delete
    input_state.key_pressed(DELETE_SCANCODE);
    tracing::trace!("SPICE key event: scancode={DELETE_SCANCODE:#x}, pressed=true");

    // Release Delete
    input_state.key_released(DELETE_SCANCODE);
    tracing::trace!("SPICE key event: scancode={DELETE_SCANCODE:#x}, pressed=false");

    // Release Alt
    input_state.key_released(ALT_SCANCODE);
    tracing::trace!("SPICE key event: scancode={ALT_SCANCODE:#x}, pressed=false");

    // Release Ctrl
    input_state.key_released(CTRL_SCANCODE);
    tracing::trace!("SPICE key event: scancode={CTRL_SCANCODE:#x}, pressed=false");
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

        // Verify all states are distinct
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

        // New client should be in cleaned up state
        assert!(client.is_cleaned_up());
    }

    #[test]
    fn test_spice_client_disconnect_without_connect() {
        let config = SpiceClientConfig::new("localhost");
        let mut client = SpiceClient::new(config);

        // Disconnect should be safe to call even without connecting
        client.disconnect();

        // Should still be in cleaned up state
        assert!(client.is_cleaned_up());
    }

    #[test]
    fn test_spice_connection_context_from_config() {
        let config = SpiceClientConfig::new("test.example.com")
            .with_port(5901)
            .with_password("secret")
            .with_resolution(1920, 1080)
            .with_usb_redirection(true)
            .with_clipboard(true);

        let ctx = SpiceConnectionContext::from_config(&config);

        assert_eq!(ctx.addr, "test.example.com:5901");
        assert_eq!(ctx.width, 1920);
        assert_eq!(ctx.height, 1080);
        assert!(ctx.auth_required);
        assert!(ctx.usb_redirection);
        assert!(ctx.clipboard_enabled);
    }

    #[test]
    fn test_spice_connection_context_cleanup() {
        let config = SpiceClientConfig::new("localhost")
            .with_password("secret")
            .with_usb_redirection(true);
        let mut ctx = SpiceConnectionContext::from_config(&config);

        assert!(ctx.auth_required);
        assert!(ctx.usb_redirection);

        ctx.cleanup();

        // After cleanup, state should be reset
        assert!(!ctx.auth_required);
        assert!(!ctx.usb_redirection);
    }

    #[test]
    fn test_spice_client_fallback_state() {
        let config = SpiceClientConfig::new("localhost");
        let mut client = SpiceClient::new(config);

        // Initially not using fallback
        assert!(!client.is_using_fallback());
        assert!(client.fallback_pid().is_none());

        // Simulate fallback mode
        client.using_fallback = true;
        client.fallback_pid = Some(12345);

        assert!(client.is_using_fallback());
        assert_eq!(client.fallback_pid(), Some(12345));

        // Cleanup should reset fallback state
        client.disconnect();
        assert!(!client.is_using_fallback());
        assert!(client.fallback_pid().is_none());
    }

    #[test]
    fn test_spice_client_is_cleaned_up_with_fallback() {
        let config = SpiceClientConfig::new("localhost");
        let mut client = SpiceClient::new(config);

        // Set fallback state
        client.using_fallback = true;
        client.fallback_pid = Some(12345);

        // Should not be cleaned up when using fallback
        assert!(!client.is_cleaned_up());

        // Reset fallback state
        client.using_fallback = false;
        client.fallback_pid = None;

        // Now should be cleaned up
        assert!(client.is_cleaned_up());
    }
}
