//! Type definitions for embedded RDP widget
//!
//! This module contains error types, enums, and configuration structs
//! used by the embedded RDP widget.

use rustconn_core::models::RdpPerformanceMode;
use std::path::PathBuf;
use thiserror::Error;

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
    pub local_path: PathBuf,
    /// Share name visible in the remote session
    pub share_name: String,
}

/// RDP connection configuration
#[derive(Debug, Clone)]
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
    /// Performance mode (Quality/Balanced/Speed)
    pub performance_mode: RdpPerformanceMode,
    /// Shared folders for drive redirection
    pub shared_folders: Vec<EmbeddedSharedFolder>,
    /// Additional FreeRDP arguments
    pub extra_args: Vec<String>,
    /// Window geometry for external mode (x, y, width, height)
    pub window_geometry: Option<(i32, i32, i32, i32)>,
    /// Whether to remember window position
    pub remember_window_position: bool,
    /// Event polling interval in milliseconds (default: 16ms = ~60 FPS)
    /// Lower values = smoother but more CPU usage
    /// Higher values = less CPU but choppier display
    pub polling_interval_ms: u32,
}

impl Default for RdpConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 3389,
            username: None,
            password: None,
            domain: None,
            width: 1920,
            height: 1080,
            clipboard_enabled: true,
            performance_mode: RdpPerformanceMode::default(),
            shared_folders: Vec::new(),
            extra_args: Vec::new(),
            window_geometry: None,
            remember_window_position: true,
            polling_interval_ms: 16, // ~60 FPS
        }
    }
}

impl RdpConfig {
    /// Creates a new RDP configuration with default settings
    #[must_use]
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            ..Default::default()
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

    /// Sets the performance mode (Quality/Balanced/Speed)
    #[must_use]
    pub const fn with_performance_mode(mut self, mode: RdpPerformanceMode) -> Self {
        self.performance_mode = mode;
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

    /// Whether to remember window position
    #[must_use]
    pub const fn with_remember_window_position(mut self, remember: bool) -> Self {
        self.remember_window_position = remember;
        self
    }

    /// Sets the event polling interval in milliseconds
    ///
    /// Default is 16ms (~60 FPS). Lower values give smoother display
    /// but use more CPU. Higher values save CPU but may appear choppy.
    ///
    /// Recommended values:
    /// - 16ms (~60 FPS) - smooth, good for interactive use
    /// - 33ms (~30 FPS) - balanced, good for most use cases
    /// - 50ms (~20 FPS) - low CPU, acceptable for static content
    #[must_use]
    pub const fn with_polling_interval(mut self, interval_ms: u32) -> Self {
        self.polling_interval_ms = interval_ms;
        self
    }
}

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

/// Callback type for state change notifications
pub type StateCallback = Box<dyn Fn(RdpConnectionState) + 'static>;

/// Callback type for error notifications
pub type ErrorCallback = Box<dyn Fn(&str) + 'static>;

/// Callback type for fallback notifications (Requirement 6.4)
pub type FallbackCallback = Box<dyn Fn(&str) + 'static>;

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
            .with_clipboard(true)
            .with_polling_interval(33);

        assert_eq!(config.host, "server.example.com");
        assert_eq!(config.port, 3390);
        assert_eq!(config.username, Some("admin".to_string()));
        assert_eq!(config.domain, Some("CORP".to_string()));
        assert_eq!(config.width, 1920);
        assert_eq!(config.height, 1080);
        assert!(config.clipboard_enabled);
        assert_eq!(config.polling_interval_ms, 33);
    }

    #[test]
    fn test_rdp_config_default_polling() {
        let config = RdpConfig::new("test.example.com");
        assert_eq!(config.polling_interval_ms, 16); // Default ~60 FPS
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
}
