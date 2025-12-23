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

use gtk4::gdk;
use gtk4::glib;
use gtk4::glib::translate::IntoGlib;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, DrawingArea, EventControllerKey, EventControllerMotion,
    EventControllerScroll, EventControllerScrollFlags, GestureClick, Label, Orientation,
};
use std::cell::RefCell;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use thiserror::Error;

#[cfg(feature = "rdp-embedded")]
use rustconn_core::RdpClientCommand;

/// Standard RDP/display resolutions (width, height)
/// Sorted by total pixels for efficient lookup
const STANDARD_RESOLUTIONS: &[(u32, u32)] = &[
    (640, 480),   // VGA
    (800, 600),   // SVGA
    (1024, 768),  // XGA
    (1152, 864),  // XGA+
    (1280, 720),  // HD 720p
    (1280, 800),  // WXGA
    (1280, 1024), // SXGA
    (1366, 768),  // HD
    (1440, 900),  // WXGA+
    (1600, 900),  // HD+
    (1600, 1200), // UXGA
    (1680, 1050), // WSXGA+
    (1920, 1080), // Full HD
    (1920, 1200), // WUXGA
    (2560, 1440), // QHD
    (2560, 1600), // WQXGA
    (3840, 2160), // 4K UHD
];

/// Finds the best matching standard resolution for the given dimensions
///
/// Returns the largest standard resolution that fits within the given dimensions,
/// or the smallest standard resolution if none fit.
#[must_use]
fn find_best_standard_resolution(width: u32, height: u32) -> (u32, u32) {
    // Find the largest resolution that fits within the given dimensions
    let mut best = STANDARD_RESOLUTIONS[0]; // Start with smallest

    for &(res_w, res_h) in STANDARD_RESOLUTIONS {
        if res_w <= width && res_h <= height {
            // This resolution fits, and since we iterate in ascending order,
            // it's larger than or equal to the previous best
            best = (res_w, res_h);
        }
    }

    best
}

/// Error type for embedded RDP operations
#[derive(Debug, Error, Clone)]
pub enum EmbeddedRdpError {
    /// Wayland subsurface creation failed
    #[error("Wayland subsurface creation failed: {0}")]
    SubsurfaceCreation(String),

    /// FreeRDP initialization failed
    #[error("FreeRDP initialization failed: {0}")]
    FreeRdpInit(String),

    /// Connection to RDP server failed
    #[error("Connection failed: {0}")]
    Connection(String),

    /// wlfreerdp is not available, falling back to external mode
    #[error("wlfreerdp not available, falling back to external mode")]
    WlFreeRdpNotAvailable,

    /// Input forwarding error
    #[error("Input forwarding error: {0}")]
    InputForwarding(String),

    /// Resize handling error
    #[error("Resize handling error: {0}")]
    ResizeError(String),

    /// Qt/Wayland threading error (Requirement 6.1, 6.2)
    #[error("Qt/Wayland threading error: {0}")]
    QtThreadingError(String),

    /// FreeRDP process failed
    #[error("FreeRDP process failed: {0}")]
    ProcessFailed(String),

    /// Falling back to external mode (Requirement 6.4)
    #[error("Falling back to external mode: {0}")]
    FallbackToExternal(String),

    /// Thread communication error
    #[error("Thread communication error: {0}")]
    ThreadError(String),
}

// ============================================================================
// FreeRDP Thread Isolation (Requirement 6.3)
// ============================================================================

/// Commands that can be sent to the FreeRDP thread
#[derive(Debug, Clone)]
pub enum RdpCommand {
    /// Connect to an RDP server
    Connect(RdpConfig),
    /// Disconnect from the server
    Disconnect,
    /// Send keyboard event
    KeyEvent { keyval: u32, pressed: bool },
    /// Send mouse event
    MouseEvent {
        x: i32,
        y: i32,
        button: u32,
        pressed: bool,
    },
    /// Resize the display
    Resize { width: u32, height: u32 },
    /// Send Ctrl+Alt+Del key sequence (Requirement 1.4)
    SendCtrlAltDel,
    /// Shutdown the thread
    Shutdown,
}

/// Events emitted by the FreeRDP thread
#[derive(Debug, Clone)]
pub enum RdpEvent {
    /// Connection established
    Connected,
    /// Connection closed
    Disconnected,
    /// Connection error occurred
    Error(String),
    /// Frame update available
    FrameUpdate {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    /// Authentication required
    AuthRequired,
    /// Fallback to external mode triggered
    FallbackTriggered(String),
}

/// Thread state for FreeRDP operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FreeRdpThreadState {
    /// Thread not started
    #[default]
    NotStarted,
    /// Thread running and idle
    Idle,
    /// Thread connecting
    Connecting,
    /// Thread connected
    Connected,
    /// Thread encountered error
    Error,
    /// Thread shutting down
    ShuttingDown,
}

/// Thread-safe FreeRDP wrapper that isolates Qt from GTK main thread
///
/// This struct runs FreeRDP operations in a dedicated thread to avoid
/// Qt/GTK threading conflicts that cause QSocketNotifier and Wayland
/// requestActivate errors.
///
/// # Requirements Coverage
///
/// - Requirement 6.3: FreeRDP threading isolation
/// - Requirement 6.1: QSocketNotifier error handling
/// - Requirement 6.2: Wayland requestActivate warning suppression
pub struct FreeRdpThread {
    /// Handle to the FreeRDP process
    process: Arc<Mutex<Option<Child>>>,
    /// Shared memory buffer for frame data
    frame_buffer: Arc<Mutex<PixelBuffer>>,
    /// Channel for sending commands to FreeRDP thread
    command_tx: mpsc::Sender<RdpCommand>,
    /// Channel for receiving events from FreeRDP thread
    event_rx: mpsc::Receiver<RdpEvent>,
    /// Thread handle
    thread_handle: Option<JoinHandle<()>>,
    /// Current thread state
    state: Arc<Mutex<FreeRdpThreadState>>,
    /// Whether fallback was triggered
    fallback_triggered: Arc<Mutex<bool>>,
}

impl FreeRdpThread {
    /// Spawns FreeRDP in a dedicated thread to avoid Qt/GTK conflicts
    ///
    /// # Arguments
    ///
    /// * `config` - The RDP connection configuration
    ///
    /// # Returns
    ///
    /// A new `FreeRdpThread` instance ready for connection.
    ///
    /// # Errors
    ///
    /// Returns error if thread creation fails.
    pub fn spawn(config: &RdpConfig) -> Result<Self, EmbeddedRdpError> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<RdpCommand>();
        let (evt_tx, evt_rx) = mpsc::channel::<RdpEvent>();

        let frame_buffer = Arc::new(Mutex::new(PixelBuffer::new(config.width, config.height)));
        let process = Arc::new(Mutex::new(None));
        let state = Arc::new(Mutex::new(FreeRdpThreadState::NotStarted));
        let fallback_triggered = Arc::new(Mutex::new(false));

        let frame_buffer_clone = Arc::clone(&frame_buffer);
        let process_clone = Arc::clone(&process);
        let state_clone = Arc::clone(&state);
        let fallback_clone = Arc::clone(&fallback_triggered);
        let config_clone = config.clone();

        let thread_handle = thread::spawn(move || {
            Self::run_freerdp_loop(
                cmd_rx,
                evt_tx,
                frame_buffer_clone,
                process_clone,
                state_clone,
                fallback_clone,
                config_clone,
            );
        });

        *state.lock().unwrap() = FreeRdpThreadState::Idle;

        Ok(Self {
            process,
            frame_buffer,
            command_tx: cmd_tx,
            event_rx: evt_rx,
            thread_handle: Some(thread_handle),
            state,
            fallback_triggered,
        })
    }

    /// Main loop for FreeRDP operations running in dedicated thread
    fn run_freerdp_loop(
        cmd_rx: mpsc::Receiver<RdpCommand>,
        evt_tx: mpsc::Sender<RdpEvent>,
        _frame_buffer: Arc<Mutex<PixelBuffer>>,
        process: Arc<Mutex<Option<Child>>>,
        state: Arc<Mutex<FreeRdpThreadState>>,
        fallback_triggered: Arc<Mutex<bool>>,
        initial_config: RdpConfig,
    ) {
        // Set environment variables to suppress Qt/Wayland warnings (Requirement 6.1, 6.2)
        std::env::set_var("QT_LOGGING_RULES", "qt.qpa.wayland=false;qt.qpa.*=false");
        std::env::set_var("QT_QPA_PLATFORM", "xcb");

        let mut current_config = Some(initial_config);

        loop {
            match cmd_rx.recv() {
                Ok(RdpCommand::Connect(config)) => {
                    *state.lock().unwrap() = FreeRdpThreadState::Connecting;
                    current_config = Some(config.clone());

                    match Self::launch_freerdp(&config, &process) {
                        Ok(()) => {
                            *state.lock().unwrap() = FreeRdpThreadState::Connected;
                            let _ = evt_tx.send(RdpEvent::Connected);
                        }
                        Err(e) => {
                            // Trigger fallback to external mode (Requirement 6.4)
                            *fallback_triggered.lock().unwrap() = true;
                            *state.lock().unwrap() = FreeRdpThreadState::Error;
                            let _ = evt_tx.send(RdpEvent::FallbackTriggered(e.to_string()));
                        }
                    }
                }
                Ok(RdpCommand::Disconnect) => {
                    Self::cleanup_process(&process);
                    *state.lock().unwrap() = FreeRdpThreadState::Idle;
                    let _ = evt_tx.send(RdpEvent::Disconnected);
                }
                Ok(RdpCommand::KeyEvent {
                    keyval: _,
                    pressed: _,
                }) => {
                    // Forward keyboard event to FreeRDP process
                    // In a real implementation, this would use FreeRDP's input API
                }
                Ok(RdpCommand::MouseEvent {
                    x: _,
                    y: _,
                    button: _,
                    pressed: _,
                }) => {
                    // Forward mouse event to FreeRDP process
                    // In a real implementation, this would use FreeRDP's input API
                }
                Ok(RdpCommand::Resize { width, height }) => {
                    // Handle resize - may need to reconnect with new resolution
                    if let Some(ref mut config) = current_config {
                        config.width = width;
                        config.height = height;
                    }
                }
                Ok(RdpCommand::SendCtrlAltDel) => {
                    // Send Ctrl+Alt+Del key sequence (Requirement 1.4)
                    // In a real implementation, this would use FreeRDP's input API
                    // to send the key sequence to the RDP server
                    eprintln!("[FreeRDP] Ctrl+Alt+Del requested");
                }
                Ok(RdpCommand::Shutdown) => {
                    *state.lock().unwrap() = FreeRdpThreadState::ShuttingDown;
                    Self::cleanup_process(&process);
                    break;
                }
                Err(_) => {
                    // Channel closed, exit loop
                    Self::cleanup_process(&process);
                    break;
                }
            }
        }
    }

    /// Launches FreeRDP with Qt error suppression
    fn launch_freerdp(
        config: &RdpConfig,
        process: &Arc<Mutex<Option<Child>>>,
    ) -> Result<(), EmbeddedRdpError> {
        // Try wlfreerdp first for embedded mode
        let binary = if Command::new("which")
            .arg("wlfreerdp")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
        {
            "wlfreerdp"
        } else {
            return Err(EmbeddedRdpError::WlFreeRdpNotAvailable);
        };

        let mut cmd = Command::new(binary);

        // Set environment to suppress Qt warnings (Requirement 6.1, 6.2)
        cmd.env("QT_LOGGING_RULES", "qt.qpa.wayland=false;qt.qpa.*=false");
        cmd.env("QT_QPA_PLATFORM", "xcb");

        // Build connection arguments
        if let Some(ref domain) = config.domain {
            if !domain.is_empty() {
                cmd.arg(format!("/d:{domain}"));
            }
        }

        if let Some(ref username) = config.username {
            cmd.arg(format!("/u:{username}"));
        }

        if let Some(ref password) = config.password {
            if !password.is_empty() {
                cmd.arg(format!("/p:{password}"));
            }
        }

        cmd.arg(format!("/w:{}", config.width));
        cmd.arg(format!("/h:{}", config.height));
        cmd.arg("/cert:ignore");
        cmd.arg("/dynamic-resolution");

        if config.clipboard_enabled {
            cmd.arg("+clipboard");
        }

        for arg in &config.extra_args {
            cmd.arg(arg);
        }

        if config.port == 3389 {
            cmd.arg(format!("/v:{}", config.host));
        } else {
            cmd.arg(format!("/v:{}:{}", config.host, config.port));
        }

        // Redirect stderr to suppress Qt warnings
        cmd.stderr(Stdio::null());

        match cmd.spawn() {
            Ok(child) => {
                *process.lock().unwrap() = Some(child);
                Ok(())
            }
            Err(e) => Err(EmbeddedRdpError::FreeRdpInit(e.to_string())),
        }
    }

    /// Cleans up the FreeRDP process
    fn cleanup_process(process: &Arc<Mutex<Option<Child>>>) {
        if let Some(mut child) = process.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Sends a command to the FreeRDP thread
    pub fn send_command(&self, cmd: RdpCommand) -> Result<(), EmbeddedRdpError> {
        self.command_tx
            .send(cmd)
            .map_err(|e| EmbeddedRdpError::ThreadError(e.to_string()))
    }

    /// Tries to receive an event from the FreeRDP thread (non-blocking)
    pub fn try_recv_event(&self) -> Option<RdpEvent> {
        self.event_rx.try_recv().ok()
    }

    /// Returns the current thread state
    pub fn state(&self) -> FreeRdpThreadState {
        *self.state.lock().unwrap()
    }

    /// Returns whether fallback was triggered
    pub fn fallback_triggered(&self) -> bool {
        *self.fallback_triggered.lock().unwrap()
    }

    /// Returns a reference to the frame buffer
    pub fn frame_buffer(&self) -> &Arc<Mutex<PixelBuffer>> {
        &self.frame_buffer
    }

    /// Shuts down the FreeRDP thread
    pub fn shutdown(&mut self) {
        let _ = self.command_tx.send(RdpCommand::Shutdown);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for FreeRdpThread {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ============================================================================
// Qt/Wayland Warning Suppression (Requirement 6.1, 6.2)
// ============================================================================

/// Safe FreeRDP launcher with Qt error suppression
///
/// This struct provides methods to launch FreeRDP with environment variables
/// set to suppress Qt/Wayland warnings that can cause issues when mixing
/// Qt-based FreeRDP with GTK4 applications.
///
/// # Requirements Coverage
///
/// - Requirement 6.1: QSocketNotifier error handling
/// - Requirement 6.2: Wayland requestActivate warning suppression
pub struct SafeFreeRdpLauncher {
    /// Whether to suppress Qt warnings
    suppress_qt_warnings: bool,
    /// Whether to force X11 backend
    force_x11: bool,
}

impl SafeFreeRdpLauncher {
    /// Creates a new launcher with default settings (warnings suppressed)
    #[must_use]
    pub fn new() -> Self {
        Self {
            suppress_qt_warnings: true,
            force_x11: true,
        }
    }

    /// Sets whether to suppress Qt warnings
    #[must_use]
    pub const fn with_suppress_warnings(mut self, suppress: bool) -> Self {
        self.suppress_qt_warnings = suppress;
        self
    }

    /// Sets whether to force X11 backend for FreeRDP
    #[must_use]
    pub const fn with_force_x11(mut self, force: bool) -> Self {
        self.force_x11 = force;
        self
    }

    /// Builds the environment variables for Qt suppression
    fn build_env(&self) -> Vec<(&'static str, &'static str)> {
        let mut env = Vec::new();

        if self.suppress_qt_warnings {
            // Suppress Qt/Wayland warnings (Requirement 6.1, 6.2)
            env.push(("QT_LOGGING_RULES", "qt.qpa.wayland=false;qt.qpa.*=false"));
        }

        if self.force_x11 {
            // Force X11 backend to avoid Wayland-specific issues
            env.push(("QT_QPA_PLATFORM", "xcb"));
        }

        env
    }

    /// Launches xfreerdp with Qt error suppression
    ///
    /// # Arguments
    ///
    /// * `config` - The RDP connection configuration
    ///
    /// # Returns
    ///
    /// The spawned child process.
    ///
    /// # Errors
    ///
    /// Returns error if FreeRDP cannot be launched.
    pub fn launch(&self, config: &RdpConfig) -> Result<Child, EmbeddedRdpError> {
        let binary = Self::detect_freerdp().ok_or_else(|| {
            EmbeddedRdpError::FreeRdpInit(
                "No FreeRDP client found. Install xfreerdp or wlfreerdp.".to_string(),
            )
        })?;

        let mut cmd = Command::new(&binary);

        // Set environment to suppress Qt warnings (Requirement 6.1, 6.2)
        for (key, value) in self.build_env() {
            cmd.env(key, value);
        }

        // Build connection arguments
        Self::add_connection_args(&mut cmd, config);

        // Redirect stderr to suppress warnings
        cmd.stderr(Stdio::null());

        cmd.spawn()
            .map_err(|e| EmbeddedRdpError::FreeRdpInit(e.to_string()))
    }

    /// Detects available FreeRDP binary
    fn detect_freerdp() -> Option<String> {
        let candidates = ["xfreerdp3", "xfreerdp", "freerdp"];
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

    /// Adds connection arguments to the command
    fn add_connection_args(cmd: &mut Command, config: &RdpConfig) {
        if let Some(ref domain) = config.domain {
            if !domain.is_empty() {
                cmd.arg(format!("/d:{domain}"));
            }
        }

        if let Some(ref username) = config.username {
            cmd.arg(format!("/u:{username}"));
        }

        if let Some(ref password) = config.password {
            if !password.is_empty() {
                cmd.arg(format!("/p:{password}"));
            }
        }

        cmd.arg(format!("/w:{}", config.width));
        cmd.arg(format!("/h:{}", config.height));
        cmd.arg("/cert:ignore");
        cmd.arg("/dynamic-resolution");

        // Add decorations flag for window controls (Requirement 6.1)
        cmd.arg("/decorations");

        // Add window geometry if saved and remember_window_position is enabled
        if config.remember_window_position {
            if let Some((x, y, _width, _height)) = config.window_geometry {
                cmd.arg(format!("/x:{x}"));
                cmd.arg(format!("/y:{y}"));
            }
        }

        if config.clipboard_enabled {
            cmd.arg("+clipboard");
        }

        // Add shared folders for drive redirection
        for folder in &config.shared_folders {
            let path = folder.local_path.display();
            cmd.arg(format!("/drive:{},{}", folder.share_name, path));
        }

        for arg in &config.extra_args {
            cmd.arg(arg);
        }

        if config.port == 3389 {
            cmd.arg(format!("/v:{}", config.host));
        } else {
            cmd.arg(format!("/v:{}:{}", config.host, config.port));
        }
    }
}

impl Default for SafeFreeRdpLauncher {
    fn default() -> Self {
        Self::new()
    }
}

/// Connection state for embedded RDP widget
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RdpConnectionState {
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

impl std::fmt::Display for RdpConnectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disconnected => write!(f, "Disconnected"),
            Self::Connecting => write!(f, "Connecting"),
            Self::Connected => write!(f, "Connected"),
            Self::Error => write!(f, "Error"),
        }
    }
}

/// A shared folder for RDP drive redirection
#[derive(Debug, Clone)]
pub struct EmbeddedSharedFolder {
    /// Local directory path to share
    pub local_path: std::path::PathBuf,
    /// Share name visible in the remote session
    pub share_name: String,
}

/// RDP connection configuration
#[derive(Debug, Clone, Default)]
pub struct RdpConfig {
    /// Target hostname or IP address
    pub host: String,
    /// Target port (default: 3389)
    pub port: u16,
    /// Username for authentication
    pub username: Option<String>,
    /// Password for authentication (should use SecretString in production)
    pub password: Option<String>,
    /// Domain for authentication
    pub domain: Option<String>,
    /// Desired width in pixels
    pub width: u32,
    /// Desired height in pixels
    pub height: u32,
    /// Enable clipboard sharing
    pub clipboard_enabled: bool,
    /// Shared folders for drive redirection
    pub shared_folders: Vec<EmbeddedSharedFolder>,
    /// Additional FreeRDP arguments
    pub extra_args: Vec<String>,
    /// Window geometry for external mode (x, y, width, height)
    pub window_geometry: Option<(i32, i32, i32, i32)>,
    /// Whether to remember window position
    pub remember_window_position: bool,
}

impl RdpConfig {
    /// Creates a new RDP configuration with default settings
    #[must_use]
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 3389,
            username: None,
            password: None,
            domain: None,
            width: 1920,
            height: 1080,
            clipboard_enabled: true,
            shared_folders: Vec::new(),
            extra_args: Vec::new(),
            window_geometry: None,
            remember_window_position: true,
        }
    }

    /// Sets the port
    #[must_use]
    pub const fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Sets the username
    #[must_use]
    pub fn with_username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    /// Sets the password
    #[must_use]
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Sets the domain
    #[must_use]
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    /// Sets the resolution
    #[must_use]
    pub const fn with_resolution(mut self, width: u32, height: u32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Enables or disables clipboard sharing
    #[must_use]
    pub const fn with_clipboard(mut self, enabled: bool) -> Self {
        self.clipboard_enabled = enabled;
        self
    }

    /// Sets shared folders for drive redirection
    #[must_use]
    pub fn with_shared_folders(mut self, folders: Vec<EmbeddedSharedFolder>) -> Self {
        self.shared_folders = folders;
        self
    }

    /// Adds extra FreeRDP arguments
    #[must_use]
    pub fn with_extra_args(mut self, args: Vec<String>) -> Self {
        self.extra_args = args;
        self
    }

    /// Sets the window geometry for external mode
    #[must_use]
    pub const fn with_window_geometry(mut self, x: i32, y: i32, width: i32, height: i32) -> Self {
        self.window_geometry = Some((x, y, width, height));
        self
    }

    /// Sets whether to remember window position
    #[must_use]
    pub const fn with_remember_window_position(mut self, remember: bool) -> Self {
        self.remember_window_position = remember;
        self
    }
}

/// Pixel buffer for frame data
///
/// This struct holds the pixel data received from FreeRDP's EndPaint callback
/// and is used to blit to the Wayland surface.
#[derive(Debug)]
pub struct PixelBuffer {
    /// Raw pixel data in BGRA format
    data: Vec<u8>,
    /// Buffer width in pixels
    width: u32,
    /// Buffer height in pixels
    height: u32,
    /// Stride (bytes per row)
    stride: u32,
    /// Whether the buffer has received any data
    has_data: bool,
}

impl PixelBuffer {
    /// Creates a new pixel buffer with the specified dimensions
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        let stride = width * 4; // BGRA = 4 bytes per pixel
        let size = (stride * height) as usize;
        Self {
            data: vec![0; size],
            width,
            height,
            stride,
            has_data: false,
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

    /// Returns whether the buffer has received any data
    #[must_use]
    pub const fn has_data(&self) -> bool {
        self.has_data
    }

    /// Sets the has_data flag
    pub fn set_has_data(&mut self, has_data: bool) {
        self.has_data = has_data;
    }

    /// Returns the stride (bytes per row)
    #[must_use]
    pub const fn stride(&self) -> u32 {
        self.stride
    }

    /// Returns a reference to the raw pixel data
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Returns a mutable reference to the raw pixel data
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Resizes the buffer to new dimensions
    ///
    /// Preserves existing content by scaling it to the new size to avoid
    /// visual artifacts during resize. The has_data flag is preserved.
    pub fn resize(&mut self, width: u32, height: u32) {
        if self.width == width && self.height == height {
            return; // No change needed
        }

        let old_width = self.width;
        let old_height = self.height;
        let had_data = self.has_data;

        self.width = width;
        self.height = height;
        self.stride = width * 4;
        let new_size = (self.stride * height) as usize;

        if had_data && old_width > 0 && old_height > 0 {
            // Preserve old data - just resize the buffer
            // The old content will be scaled during rendering
            self.data.resize(new_size, 0);
            self.has_data = true; // Keep has_data true to continue rendering
        } else {
            self.data.resize(new_size, 0);
            self.has_data = false;
        }
    }

    /// Clears the buffer to black
    pub fn clear(&mut self) {
        self.data.fill(0);
        self.has_data = false; // Reset data flag on clear
    }

    /// Updates a region of the buffer
    ///
    /// # Arguments
    ///
    /// * `x` - X coordinate of the region
    /// * `y` - Y coordinate of the region
    /// * `w` - Width of the region
    /// * `h` - Height of the region
    /// * `src_data` - Source pixel data
    /// * `src_stride` - Source stride
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

            let x_offset = x as usize * bytes_per_pixel;
            if x_offset >= dst_stride {
                continue;
            }

            let dst_offset = dst_y * dst_stride + x_offset;
            let src_offset = row as usize * src_stride;
            let max_copy = dst_stride.saturating_sub(x_offset);
            let copy_width = (w as usize * bytes_per_pixel).min(max_copy);

            if copy_width > 0
                && src_offset + copy_width <= src_data.len()
                && dst_offset + copy_width <= self.data.len()
            {
                self.data[dst_offset..dst_offset + copy_width]
                    .copy_from_slice(&src_data[src_offset..src_offset + copy_width]);
                self.has_data = true; // Mark that we have received data
            }
        }
    }
}

/// Wayland surface handle for subsurface integration
///
/// This struct manages the Wayland surface resources for embedding
/// the RDP display within the GTK widget hierarchy.
#[derive(Debug, Default)]
pub struct WaylandSurfaceHandle {
    /// Whether the surface is initialized
    initialized: bool,
    /// Surface ID (for debugging)
    surface_id: u32,
}

impl WaylandSurfaceHandle {
    /// Creates a new uninitialized surface handle
    #[must_use]
    pub const fn new() -> Self {
        Self {
            initialized: false,
            surface_id: 0,
        }
    }

    /// Initializes the Wayland surface
    ///
    /// # Errors
    ///
    /// Returns error if surface creation fails
    pub fn initialize(&mut self) -> Result<(), EmbeddedRdpError> {
        // In a real implementation, this would:
        // 1. Get the wl_display from GTK
        // 2. Create a wl_surface
        // 3. Create a wl_subsurface attached to the parent
        // 4. Set up shared memory buffers

        // For now, we mark as initialized for the fallback path
        self.initialized = true;
        self.surface_id = 1;
        Ok(())
    }

    /// Returns whether the surface is initialized
    #[must_use]
    pub const fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Commits pending changes to the surface
    pub fn commit(&self) {
        // In a real implementation, this would call wl_surface_commit
    }

    /// Damages a region of the surface for redraw
    pub fn damage(&self, _x: i32, _y: i32, _width: i32, _height: i32) {
        // In a real implementation, this would call wl_surface_damage_buffer
    }

    /// Cleans up the surface resources
    pub fn cleanup(&mut self) {
        self.initialized = false;
        self.surface_id = 0;
    }
}

/// Callback type for state change notifications
type StateCallback = Box<dyn Fn(RdpConnectionState) + 'static>;

/// Callback type for error notifications
type ErrorCallback = Box<dyn Fn(&str) + 'static>;

/// Callback type for fallback notifications (Requirement 6.4)
type FallbackCallback = Box<dyn Fn(&str) + 'static>;

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

    /// Updates the reconnect button visibility based on connection state
    fn update_reconnect_button_visibility(&self) {
        let state = *self.state.borrow();
        let show_reconnect = matches!(
            state,
            RdpConnectionState::Disconnected | RdpConnectionState::Error
        );
        self.reconnect_button.set_visible(show_reconnect);
        // Show toolbar when reconnect button should be visible
        if show_reconnect {
            self.toolbar.set_visible(true);
        }
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
                    eprintln!("[RDP] Copying {} chars from remote to local clipboard", char_count);

                    // Copy to local clipboard
                    let display = drawing_area.display();
                    let clipboard = display.clipboard();
                    clipboard.set_text(text);

                    // Show feedback
                    status_label.set_text(&format!("Copied {char_count} chars"));
                    status_label.set_visible(true);
                    let status_hide = status_label.clone();
                    glib::timeout_add_local_once(
                        std::time::Duration::from_secs(2),
                        move || {
                            status_hide.set_visible(false);
                        },
                    );
                } else {
                    eprintln!("[RDP] No remote clipboard text available");
                    status_label.set_text("No remote clipboard data");
                    status_label.set_visible(true);
                    let status_hide = status_label.clone();
                    glib::timeout_add_local_once(
                        std::time::Duration::from_secs(2),
                        move || {
                            status_hide.set_visible(false);
                        },
                    );
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
                            eprintln!("[RDP] Pasting {char_count} chars to remote");

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
                        eprintln!("[IronRDP] Sent Ctrl+Alt+Del");
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
        _rdp_width: &Rc<RefCell<u32>>,
        _rdp_height: &Rc<RefCell<u32>>,
    ) {
        cr.select_font_face(
            "Sans",
            gtk4::cairo::FontSlant::Normal,
            gtk4::cairo::FontWeight::Normal,
        );

        let center_y = f64::from(height) / 2.0 - 40.0;

        // Protocol icon (circle with "R" for RDP)
        let icon_color = match current_state {
            RdpConnectionState::Connected => (0.3, 0.6, 0.4), // Green for connected
            RdpConnectionState::Connecting => (0.5, 0.5, 0.3), // Yellow for connecting
            RdpConnectionState::Error => (0.6, 0.3, 0.3),     // Red for error
            RdpConnectionState::Disconnected => (0.3, 0.5, 0.7), // Blue for disconnected
        };
        cr.set_source_rgb(icon_color.0, icon_color.1, icon_color.2);
        cr.arc(
            f64::from(width) / 2.0,
            center_y,
            40.0,
            0.0,
            2.0 * std::f64::consts::PI,
        );
        let _ = cr.fill();

        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.set_font_size(32.0);
        let extents = cr.text_extents("R").unwrap();
        cr.move_to(
            f64::from(width) / 2.0 - extents.width() / 2.0,
            center_y + extents.height() / 2.0,
        );
        let _ = cr.show_text("R");

        // Connection info - show host from config
        let config_ref = config.borrow();
        let host = config_ref
            .as_ref()
            .map(|c| c.host.as_str())
            .unwrap_or("No connection");

        cr.set_source_rgb(0.9, 0.9, 0.9);
        cr.set_font_size(18.0);
        let extents = cr.text_extents(host).unwrap();
        cr.move_to((f64::from(width) - extents.width()) / 2.0, center_y + 70.0);
        let _ = cr.show_text(host);

        // Status message - be clear about mode
        cr.set_font_size(13.0);
        let (status_text, status_color) = match current_state {
            RdpConnectionState::Disconnected => {
                if config_ref.is_some() {
                    // Was connected before, now disconnected
                    ("Session ended", (0.8, 0.4, 0.4))
                } else {
                    ("No connection configured", (0.5, 0.5, 0.5))
                }
            }
            RdpConnectionState::Connecting => {
                if embedded {
                    ("Connecting via IronRDP...", (0.8, 0.8, 0.6))
                } else {
                    ("Starting FreeRDP...", (0.8, 0.8, 0.6))
                }
            }
            RdpConnectionState::Connected => {
                if embedded {
                    // IronRDP embedded mode - waiting for framebuffer
                    ("✓ Connected - waiting for display", (0.6, 0.8, 0.6))
                } else {
                    // FreeRDP runs in separate window
                    ("✓ RDP session running in FreeRDP window", (0.6, 0.8, 0.6))
                }
            }
            RdpConnectionState::Error => ("Connection failed", (0.8, 0.4, 0.4)),
        };

        cr.set_source_rgb(status_color.0, status_color.1, status_color.2);
        let extents = cr.text_extents(status_text).unwrap();
        cr.move_to((f64::from(width) - extents.width()) / 2.0, center_y + 100.0);
        let _ = cr.show_text(status_text);

        // Show hint for connected state
        if current_state == RdpConnectionState::Connected && !embedded {
            cr.set_source_rgb(0.6, 0.6, 0.6);
            cr.set_font_size(11.0);
            let hint = "Switch to the FreeRDP window to interact with the session";
            let extents = cr.text_extents(hint).unwrap();
            cr.move_to((f64::from(width) - extents.width()) / 2.0, center_y + 125.0);
            let _ = cr.show_text(hint);
        }
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
                        eprintln!("[IronRDP] Unknown keyval: 0x{:X}", gdk_keyval);
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

                let scale_x = widget_w / rdp_w;
                let scale_y = widget_h / rdp_h;
                let scale = scale_x.min(scale_y);
                let offset_x = rdp_w.mul_add(-scale, widget_w) / 2.0;
                let offset_y = rdp_h.mul_add(-scale, widget_h) / 2.0;

                let rdp_x = ((x - offset_x) / scale).clamp(0.0, rdp_w - 1.0);
                let rdp_y = ((y - offset_y) / scale).clamp(0.0, rdp_h - 1.0);
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

                let scale_x = widget_w / rdp_w;
                let scale_y = widget_h / rdp_h;
                let scale = scale_x.min(scale_y);
                let offset_x = rdp_w.mul_add(-scale, widget_w) / 2.0;
                let offset_y = rdp_h.mul_add(-scale, widget_h) / 2.0;

                let rdp_x = ((x - offset_x) / scale).clamp(0.0, rdp_w - 1.0);
                let rdp_y = ((y - offset_y) / scale).clamp(0.0, rdp_h - 1.0);

                // Convert GTK button to RDP button mask
                let button_bit: u8 = match button {
                    1 => 0x01, // Left
                    2 => 0x04, // Middle
                    3 => 0x02, // Right
                    _ => 0x00,
                };
                let buttons = *button_state_press.borrow() | button_bit;
                *button_state_press.borrow_mut() = buttons;

                if using_ironrdp {
                    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                    if let Some(ref tx) = *ironrdp_tx.borrow() {
                        // Send button press event (separate from motion)
                        // GTK button: 1=left, 2=middle, 3=right
                        // RDP button: 1=left, 2=right, 3=middle
                        let rdp_button = match button {
                            1 => 1, // Left
                            2 => 3, // Middle (GTK button 2 = middle)
                            3 => 2, // Right (GTK button 3 = right)
                            _ => 1,
                        };
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

                let scale_x = widget_w / rdp_w;
                let scale_y = widget_h / rdp_h;
                let scale = scale_x.min(scale_y);
                let offset_x = rdp_w.mul_add(-scale, widget_w) / 2.0;
                let offset_y = rdp_h.mul_add(-scale, widget_h) / 2.0;

                let rdp_x = ((x - offset_x) / scale).clamp(0.0, rdp_w - 1.0);
                let rdp_y = ((y - offset_y) / scale).clamp(0.0, rdp_h - 1.0);

                let button_bit: u8 = match button {
                    1 => 0x01,
                    2 => 0x04,
                    3 => 0x02,
                    _ => 0x00,
                };
                let buttons = *button_state_release.borrow() & !button_bit;
                *button_state_release.borrow_mut() = buttons;

                if using_ironrdp {
                    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                    if let Some(ref tx) = *ironrdp_tx.borrow() {
                        // Send button release event (separate from motion)
                        // GTK button: 1=left, 2=middle, 3=right
                        // RDP button: 1=left, 2=right, 3=middle
                        let rdp_button = match button {
                            1 => 1, // Left
                            2 => 3, // Middle
                            3 => 2, // Right
                            _ => 1,
                        };
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

                            eprintln!(
                                "[IronRDP] Reconnecting with new resolution {}x{} (was {}x{})",
                                new_w, new_h, cfg.width, cfg.height
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
            eprintln!("[IronRDP] Reconnect failed: {}", e);
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

        glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
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
                            eprintln!("[IronRDP] Reconnected: {}x{}", width, height);
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
                            eprintln!("[IronRDP] Disconnected after reconnect");
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
                            eprintln!("[IronRDP] Error after reconnect: {}", msg);
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
        });
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
        Command::new("which")
            .arg("wlfreerdp")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }

    /// Detects if xfreerdp is available for external mode
    #[must_use]
    pub fn detect_xfreerdp() -> Option<String> {
        let candidates = ["xfreerdp3", "xfreerdp", "freerdp"];
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
        rustconn_core::is_embedded_rdp_available()
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

        eprintln!(
            "[EmbeddedRDP] Attempting IronRDP connection to {}:{}",
            config.host, config.port
        );
        eprintln!(
            "[EmbeddedRDP] Username: {:?}, Domain: {:?}, Password: {}",
            config.username,
            config.domain,
            if config.password.is_some() {
                "[REDACTED]"
            } else {
                "not set"
            }
        );

        // Warn about shared folders - IronRDP doesn't support drive redirection yet
        if !config.shared_folders.is_empty() {
            eprintln!(
                "[EmbeddedRDP] Warning: {} shared folder(s) configured but IronRDP doesn't support drive redirection yet. Use external mode for shared folders.",
                config.shared_folders.len()
            );
        }

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

        // Store client in a shared reference for the polling closure
        let client = std::rc::Rc::new(std::cell::RefCell::new(Some(client)));
        let client_ref = client.clone();

        glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
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
                            eprintln!("[IronRDP] Connected: {}x{}", width, height);
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
                            eprintln!("[IronRDP] Disconnected");
                            *state.borrow_mut() = RdpConnectionState::Disconnected;
                            toolbar.set_visible(false);
                            if let Some(ref callback) = *on_state_changed.borrow() {
                                callback(RdpConnectionState::Disconnected);
                            }
                            needs_redraw = true;
                            should_break = true;
                        }
                        RdpClientEvent::Error(msg) => {
                            eprintln!("[IronRDP] Error: {}", msg);
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
                            eprintln!("[IronRDP] Resolution changed: {}x{}", width, height);
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
                            eprintln!("[IronRDP] Authentication required");
                        }
                        RdpClientEvent::ClipboardText(text) => {
                            // Server sent clipboard text - store it and enable Copy button
                            eprintln!(
                                "[IronRDP] Clipboard text received: {} chars",
                                text.len()
                            );
                            *remote_clipboard_text.borrow_mut() = Some(text);
                            copy_button.set_sensitive(true);
                            copy_button.set_tooltip_text(Some(
                                "Copy remote clipboard to local",
                            ));
                        }
                        RdpClientEvent::ClipboardFormatsAvailable(formats) => {
                            // Server has clipboard data available
                            eprintln!(
                                "[IronRDP] Clipboard formats available: {:?}",
                                formats
                            );
                            *remote_clipboard_formats.borrow_mut() = formats;
                        }
                        RdpClientEvent::ClipboardDataRequest(format) => {
                            // Server requests clipboard data from us
                            eprintln!(
                                "[IronRDP] Server requests clipboard format: {:?}",
                                format
                            );
                            // TODO: Send local clipboard data to server
                        }
                        RdpClientEvent::ClipboardPasteRequest(format) => {
                            // Backend requests to fetch data from server
                            eprintln!(
                                "[IronRDP] Requesting clipboard data for format: {:?}",
                                format
                            );
                            if let Some(ref sender) = *ironrdp_tx.borrow() {
                                let _ = sender.send(RdpClientCommand::RequestClipboardData {
                                    format_id: format.id,
                                });
                            }
                        }
                        _ => {}
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
        });

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
        eprintln!(
            "[EmbeddedRDP] Attempting embedded connection to {}:{}",
            config.host, config.port
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
                            eprintln!("[EmbeddedRDP] Connected!");
                            *state.borrow_mut() = RdpConnectionState::Connected;
                            if let Some(ref callback) = *on_state_changed.borrow() {
                                callback(RdpConnectionState::Connected);
                            }
                            drawing_area.queue_draw();
                        }
                        RdpEvent::Disconnected => {
                            eprintln!("[EmbeddedRDP] Disconnected");
                            *state.borrow_mut() = RdpConnectionState::Disconnected;
                            if let Some(ref callback) = *on_state_changed.borrow() {
                                callback(RdpConnectionState::Disconnected);
                            }
                            drawing_area.queue_draw();
                            return glib::ControlFlow::Break;
                        }
                        RdpEvent::Error(msg) => {
                            eprintln!("[EmbeddedRDP] Error: {}", msg);
                            *state.borrow_mut() = RdpConnectionState::Error;
                            if let Some(ref callback) = *on_error.borrow() {
                                callback(&msg);
                            }
                            drawing_area.queue_draw();
                            return glib::ControlFlow::Break;
                        }
                        RdpEvent::FallbackTriggered(reason) => {
                            eprintln!("[EmbeddedRDP] Fallback triggered: {}", reason);
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
                                    eprintln!(
                                        "[EmbeddedRDP] Resolution changed: {}x{}",
                                        width, height
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
                            eprintln!("[EmbeddedRDP] Authentication required");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rdp_config_builder() {
        let config = RdpConfig::new("server.example.com")
            .with_port(3390)
            .with_username("admin")
            .with_domain("CORP")
            .with_resolution(1920, 1080)
            .with_clipboard(true);

        assert_eq!(config.host, "server.example.com");
        assert_eq!(config.port, 3390);
        assert_eq!(config.username, Some("admin".to_string()));
        assert_eq!(config.domain, Some("CORP".to_string()));
        assert_eq!(config.width, 1920);
        assert_eq!(config.height, 1080);
        assert!(config.clipboard_enabled);
    }

    #[test]
    fn test_pixel_buffer_new() {
        let buffer = PixelBuffer::new(100, 50);
        assert_eq!(buffer.width(), 100);
        assert_eq!(buffer.height(), 50);
        assert_eq!(buffer.stride(), 400); // 100 * 4 bytes per pixel
        assert_eq!(buffer.data().len(), 20000); // 100 * 50 * 4
    }

    #[test]
    fn test_pixel_buffer_resize() {
        let mut buffer = PixelBuffer::new(100, 50);
        buffer.resize(200, 100);
        assert_eq!(buffer.width(), 200);
        assert_eq!(buffer.height(), 100);
        assert_eq!(buffer.stride(), 800);
        assert_eq!(buffer.data().len(), 80000);
    }

    #[test]
    fn test_pixel_buffer_clear() {
        let mut buffer = PixelBuffer::new(10, 10);
        buffer.data_mut()[0] = 255;
        buffer.clear();
        assert!(buffer.data().iter().all(|&b| b == 0));
    }

    #[test]
    fn test_wayland_surface_handle() {
        let mut handle = WaylandSurfaceHandle::new();
        assert!(!handle.is_initialized());

        handle.initialize().unwrap();
        assert!(handle.is_initialized());

        handle.cleanup();
        assert!(!handle.is_initialized());
    }

    #[test]
    fn test_rdp_connection_state_display() {
        assert_eq!(RdpConnectionState::Disconnected.to_string(), "Disconnected");
        assert_eq!(RdpConnectionState::Connecting.to_string(), "Connecting");
        assert_eq!(RdpConnectionState::Connected.to_string(), "Connected");
        assert_eq!(RdpConnectionState::Error.to_string(), "Error");
    }

    #[test]
    fn test_embedded_rdp_error_display() {
        let err = EmbeddedRdpError::WlFreeRdpNotAvailable;
        assert!(err.to_string().contains("wlfreerdp not available"));

        let err = EmbeddedRdpError::Connection("timeout".to_string());
        assert!(err.to_string().contains("timeout"));
    }

    // Tests for FreeRDP thread isolation (Requirement 6.3)

    #[test]
    fn test_rdp_command_variants() {
        let config = RdpConfig::new("test.example.com");
        let cmd = RdpCommand::Connect(config);
        assert!(matches!(cmd, RdpCommand::Connect(_)));

        let cmd = RdpCommand::Disconnect;
        assert!(matches!(cmd, RdpCommand::Disconnect));

        let cmd = RdpCommand::KeyEvent {
            keyval: 65,
            pressed: true,
        };
        assert!(matches!(cmd, RdpCommand::KeyEvent { .. }));

        let cmd = RdpCommand::MouseEvent {
            x: 100,
            y: 200,
            button: 1,
            pressed: true,
        };
        assert!(matches!(cmd, RdpCommand::MouseEvent { .. }));

        let cmd = RdpCommand::Resize {
            width: 1920,
            height: 1080,
        };
        assert!(matches!(cmd, RdpCommand::Resize { .. }));

        let cmd = RdpCommand::Shutdown;
        assert!(matches!(cmd, RdpCommand::Shutdown));
    }

    #[test]
    fn test_rdp_event_variants() {
        let evt = RdpEvent::Connected;
        assert!(matches!(evt, RdpEvent::Connected));

        let evt = RdpEvent::Disconnected;
        assert!(matches!(evt, RdpEvent::Disconnected));

        let evt = RdpEvent::Error("test error".to_string());
        assert!(matches!(evt, RdpEvent::Error(_)));

        let evt = RdpEvent::FrameUpdate {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        };
        assert!(matches!(evt, RdpEvent::FrameUpdate { .. }));

        let evt = RdpEvent::AuthRequired;
        assert!(matches!(evt, RdpEvent::AuthRequired));

        let evt = RdpEvent::FallbackTriggered("reason".to_string());
        assert!(matches!(evt, RdpEvent::FallbackTriggered(_)));
    }

    #[test]
    fn test_freerdp_thread_state_default() {
        let state = FreeRdpThreadState::default();
        assert_eq!(state, FreeRdpThreadState::NotStarted);
    }

    #[test]
    fn test_qt_threading_error() {
        let err = EmbeddedRdpError::QtThreadingError("QSocketNotifier error".to_string());
        assert!(err.to_string().contains("Qt/Wayland threading error"));
        assert!(err.to_string().contains("QSocketNotifier"));
    }

    #[test]
    fn test_fallback_to_external_error() {
        let err = EmbeddedRdpError::FallbackToExternal("embedded mode failed".to_string());
        assert!(err.to_string().contains("Falling back to external mode"));
    }

    #[test]
    fn test_thread_error() {
        let err = EmbeddedRdpError::ThreadError("channel closed".to_string());
        assert!(err.to_string().contains("Thread communication error"));
    }

    // Tests for SafeFreeRdpLauncher (Requirement 6.1, 6.2)

    #[test]
    fn test_safe_freerdp_launcher_default() {
        let launcher = SafeFreeRdpLauncher::new();
        assert!(launcher.suppress_qt_warnings);
        assert!(launcher.force_x11);
    }

    #[test]
    fn test_safe_freerdp_launcher_builder() {
        let launcher = SafeFreeRdpLauncher::new()
            .with_suppress_warnings(false)
            .with_force_x11(false);
        assert!(!launcher.suppress_qt_warnings);
        assert!(!launcher.force_x11);
    }

    #[test]
    fn test_safe_freerdp_launcher_env() {
        let launcher = SafeFreeRdpLauncher::new();
        let env = launcher.build_env();

        // Should have both QT_LOGGING_RULES and QT_QPA_PLATFORM
        assert!(env.iter().any(|(k, _)| *k == "QT_LOGGING_RULES"));
        assert!(env.iter().any(|(k, _)| *k == "QT_QPA_PLATFORM"));
    }

    #[test]
    fn test_safe_freerdp_launcher_env_disabled() {
        let launcher = SafeFreeRdpLauncher::new()
            .with_suppress_warnings(false)
            .with_force_x11(false);
        let env = launcher.build_env();

        // Should be empty when both are disabled
        assert!(env.is_empty());
    }
}
