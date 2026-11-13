//! Type definitions for embedded RDP widget
//!
//! This module contains error types, enums, and configuration structs
//! used by the embedded RDP widget.

use std::path::PathBuf;

use rustconn_core::models::{RdpClipboardDirection, RdpPerformanceMode};
use rustconn_core::rdp_client::graphics::GraphicsMode;
use secrecy::SecretString;
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

    /// No FreeRDP 3 client is installed. Carries the version of the FreeRDP
    /// that was refused — FreeRDP 2 — when one was found (issue #351).
    #[error("No supported FreeRDP client is installed (RustConn needs FreeRDP 3)")]
    NoSupportedFreeRdp(Option<rustconn_core::protocol::FreeRdpVersion>),

    /// Input forwarding error
    #[error("Input forwarding error: {0}")]
    InputForwarding(String),

    /// Resize handling error
    #[error("Resize handling error: {0}")]
    ResizeError(String),

    /// Qt/Wayland threading error
    #[error("Qt/Wayland threading error: {0}")]
    QtThreadingError(String),

    /// FreeRDP process failed
    #[error("FreeRDP process failed: {0}")]
    ProcessFailed(String),

    /// Falling back to external mode
    #[error("Falling back to external mode: {0}")]
    FallbackToExternal(String),

    /// RD Gateway configured, but this connection cannot use the embedded
    /// MS-TSGU tunnel (feature disabled, or a target port other than 3389).
    #[error("RD Gateway requires the external RDP client for this connection")]
    GatewayNotSupported,

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
#[expect(
    clippy::struct_excessive_bools,
    reason = "settings/flags struct mirrors persisted config 1:1; bools represent independent toggles, not a state machine"
)]
pub struct RdpConfig {
    /// Target hostname or IP address
    pub host: String,
    /// Target port (default: 3389)
    pub port: u16,
    /// Username for authentication
    pub username: Option<String>,
    /// Password for authentication
    pub password: Option<SecretString>,
    /// Domain for authentication
    pub domain: Option<String>,
    /// Desired width in pixels, negotiated by the embedded viewer
    pub width: u32,
    /// Desired height in pixels, negotiated by the embedded viewer
    pub height: u32,
    /// How an external FreeRDP window is sized.
    ///
    /// Separate from `width`/`height` on purpose: those are the embedded
    /// viewer's own geometry, measured from the `DrawingArea` in *logical*
    /// pixels. Handing them to a standalone FreeRDP window is what opened a
    /// ~1670x990 client on a 4K display — the widget size, unscaled, in a window
    /// that has nothing to do with the widget.
    pub external_display_mode: rustconn_core::models::RdpDisplayMode,
    /// FreeRDP monitor ids for the external client (`/monitors:`).
    pub external_monitor_ids: Vec<u32>,
    /// Fixed resolution for [`rustconn_core::models::RdpDisplayMode::Custom`]
    pub external_resolution: Option<rustconn_core::models::Resolution>,
    /// Session colour depth for the external client (`/bpp:`)
    pub color_depth: Option<u8>,
    /// Live compositor scale as a percentage, read for
    /// [`rustconn_core::models::ScaleOverride::Native`] on the external path
    pub system_scale_percent: u16,
    /// Which way the clipboard synchronises
    pub clipboard_direction: RdpClipboardDirection,
    /// Performance mode (Quality/Balanced/Speed)
    pub performance_mode: RdpPerformanceMode,
    /// Shared folders for drive redirection
    pub shared_folders: Vec<EmbeddedSharedFolder>,
    /// Enable printer redirection (maps local CUPS printer into the session)
    pub printer_enabled: bool,
    /// Where the session audio is played. `Remote` forces FreeRDP fallback —
    /// IronRDP cannot signal `INFO_REMOTECONSOLEAUDIO` (issue #245).
    pub audio_mode: rustconn_core::models::RdpAudioMode,
    /// What OS the server runs (issue #366). It tailors the toolbar — `Linux`
    /// hides the Windows-only admin tools and shell launchers — and decides how
    /// a resize reaches the server: `Windows` always uses Display Control,
    /// `Linux` always reconnects, and `Auto` uses Display Control until the
    /// server ignores a layout, then reconnects (see
    /// [`Self::resize_needs_reconnect`]).
    pub server_os: rustconn_core::models::RdpServerOs,
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
    /// Keyboard layout override (Windows KLID). None = auto-detect.
    pub keyboard_layout: Option<u32>,
    /// Explicit FreeRDP client binary for external mode. `None` auto-detects.
    /// An unavailable choice falls back to auto-detection (issue #340).
    pub freerdp_client_override: Option<String>,
    /// Display scale override for embedded mode
    pub scale_override: rustconn_core::models::ScaleOverride,
    /// Show local mouse cursor over embedded viewer (disable to avoid double cursor)
    pub show_local_cursor: bool,
    /// Gateway hostname. When set, the embedded client tunnels the session
    /// through it (MS-TSGU); targets on a port other than 3389 and builds
    /// without the `rd-gateway` feature use the external FreeRDP client.
    pub gateway_hostname: Option<String>,
    /// Gateway port (default: 443)
    pub gateway_port: u16,
    /// Gateway username (if different from connection username)
    pub gateway_username: Option<String>,
    /// Whether to ignore TLS certificate validation (default: false).
    /// When false, uses TOFU (trust-on-first-use) like SSH known_hosts.
    pub ignore_certificate: bool,
    /// Security layer selection (Negotiate/RDP/TLS/NLA).
    /// `Rdp` and `Tls` force FreeRDP fallback (incompatible with IronRDP).
    pub security_layer: rustconn_core::models::RdpSecurityLayer,
    /// Disable Network Level Authentication while leaving other methods.
    ///
    /// FreeRDP-only (`/sec:nla:off`), so it takes effect on an external launch
    /// or on the fallback. IronRDP decides NLA from whether the credentials are
    /// complete and does not read this.
    pub disable_nla: bool,
    /// Request the console/admin session (`/admin`) on the external client.
    ///
    /// FreeRDP-only (issue #366): bypasses RDS CAL licensing by connecting to
    /// the server's console/administrative session. The embedded IronRDP client
    /// cannot request this yet (IronRDP #1629), so it only affects an external
    /// launch or the FreeRDP fallback.
    pub admin_session: bool,
    /// TLS security level for FreeRDP (0–5). Level 0 = legacy TLS 1.0 compat.
    /// Only effective with FreeRDP; IronRDP uses rustls (TLS 1.2+ only).
    pub tls_security_level: Option<u8>,
    /// Enable mouse jiggler to prevent idle disconnect
    pub jiggler_enabled: bool,
    /// Mouse jiggler interval in seconds (10–600, default: 60)
    pub jiggler_interval_secs: u32,
    /// Inter-character delay for autotype in milliseconds (default: 20ms)
    pub autotype_delay_ms: u32,
    /// Initial delay before autotype starts in milliseconds (default: 0ms)
    pub autotype_initial_delay_ms: u32,
    /// Send scripts via clipboard paste instead of autotype (default: true)
    pub script_paste_via_clipboard: bool,
    /// Force full reconnect on resize instead of Display Control Channel.
    /// Useful for legacy servers that don't support MS-RDPEDISP.
    pub reconnect_on_resize: bool,
    /// Set at runtime, never saved: this server accepted a Display Control
    /// monitor layout and did not apply it (xrdp on `Auto`). From then on the
    /// session resizes by reconnecting, like a `Linux` server. Kept in the
    /// runtime config so it outlives the reconnect that follows.
    pub layout_resize_ignored: bool,
    /// Reconnect by itself when an established session's connection drops
    /// (`RdpConfig::auto_reconnect`). Read by the IronRDP path only.
    pub auto_reconnect: bool,
    /// Request dynamic desktop resizing on the external client
    /// (`/dynamic-resolution`). Default true; turn off for legacy servers.
    /// Mutually exclusive with [`Self::smart_sizing`] (issue #341).
    pub dynamic_resolution: bool,
    /// Scale the remote framebuffer to the external window (`/smart-sizing`).
    /// Mutually exclusive with [`Self::dynamic_resolution`] (issue #341).
    pub smart_sizing: bool,
    /// RemoteApp program path or alias (forces FreeRDP fallback).
    pub remote_app_program: Option<String>,
    /// RemoteApp command-line arguments.
    pub remote_app_args: Option<String>,
    /// RemoteApp display name.
    pub remote_app_name: Option<String>,
    /// User-selected graphics pipeline mode for the embedded IronRDP client.
    /// - Auto (default): negotiate GFX/H.264 when available
    /// - Legacy: skip GFX pipeline entirely (most compatible, fixes #218)
    /// - RemoteFx: use RemoteFX codec without GFX pipeline
    ///
    /// Only relevant for Embedded mode; External (FreeRDP) handles this itself.
    pub graphics_mode: GraphicsMode,
    /// Force Legacy/RemoteFX graphics path (skip GFX/EGFX pipeline).
    /// Set to `true` on automatic retry when GFX decode fails (issue #218).
    /// This is an internal retry flag — the user-facing choice is `graphics_mode`.
    // ponytail: single bool covers one retry level (GFX→Legacy); if a
    // multi-step cascade is ever needed (Avc444→H264→RemoteFx→Legacy→FreeRDP),
    // replace with a `GfxFallbackState` enum and a state machine.
    pub force_legacy_graphics: bool,
    /// Enable Multipath TCP for the embedded RDP connection.
    /// Uses multiple network paths for seamless mobility and bandwidth aggregation.
    /// Requires kernel MPTCP support (Linux 5.6+). Falls back to regular TCP.
    pub mptcp: bool,
    /// Enable FIDO2/WebAuthn device redirection.
    /// Allows using local FIDO2 security keys for authentication in the remote session.
    /// Requires FreeRDP 3.x with `/fido` support. Only applies to External mode.
    pub fido2_enabled: bool,
    /// Redirect the local microphone: the `AUDIO_INPUT` channel on the
    /// embedded client (with `rdp-audio`), `/microphone` on FreeRDP.
    pub microphone_enabled: bool,
    /// Redirect local smart cards (`/smartcard`). External FreeRDP only.
    pub smartcard_enabled: bool,
    /// RD Connection Broker routing value, sent as the routing token by the
    /// embedded client and as `/load-balance-info:` by FreeRDP.
    pub load_balance_info: Option<String>,
    /// Authenticate NLA with Kerberos instead of NTLM (issue #351).
    ///
    /// On the embedded IronRDP path (CredSSP) this chooses Kerberos: it signs in
    /// with the saved password, not a `kinit` ticket, and has no NTLM fallback
    /// when the KDC cannot be found or reached. Needed for accounts in the AD
    /// "Protected Users" group, which may not use NTLM. On the FreeRDP fallback
    /// it lifts the `RemoteApp` NTLM restriction and gates the KDC Address.
    pub kerberos_enabled: bool,
    /// KDC address for the Kerberos exchange, used only when
    /// [`Self::kerberos_enabled`] is set: a domain controller or an MS-KKDCP
    /// proxy URL. `None` lets sspi look the KDC up, then tries the realm's own
    /// DNS name. The FreeRDP fallback can use only an `https://…/KdcProxy`
    /// proxy (see `rustconn_core::protocol::freerdp_kerberos_arg`).
    pub kdc_proxy_url: Option<String>,
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
            external_display_mode: rustconn_core::models::RdpDisplayMode::default(),
            external_monitor_ids: Vec::new(),
            external_resolution: None,
            color_depth: None,
            system_scale_percent: 100,
            clipboard_direction: RdpClipboardDirection::Both,
            performance_mode: RdpPerformanceMode::default(),
            shared_folders: Vec::new(),
            printer_enabled: false,
            audio_mode: rustconn_core::models::RdpAudioMode::default(),
            server_os: rustconn_core::models::RdpServerOs::default(),
            extra_args: Vec::new(),
            window_geometry: None,
            remember_window_position: true,
            polling_interval_ms: 16, // ~60 FPS
            keyboard_layout: None,
            freerdp_client_override: None,
            scale_override: rustconn_core::models::ScaleOverride::default(),
            show_local_cursor: true,
            gateway_hostname: None,
            gateway_port: 443,
            gateway_username: None,
            ignore_certificate: false,
            security_layer: rustconn_core::models::RdpSecurityLayer::default(),
            tls_security_level: None,
            disable_nla: false,
            admin_session: false,
            jiggler_enabled: false,
            jiggler_interval_secs: 60,
            autotype_delay_ms: 20,
            autotype_initial_delay_ms: 0,
            script_paste_via_clipboard: true,
            reconnect_on_resize: false,
            layout_resize_ignored: false,
            auto_reconnect: true,
            dynamic_resolution: true,
            smart_sizing: false,
            remote_app_program: None,
            remote_app_args: None,
            remote_app_name: None,
            graphics_mode: GraphicsMode::Auto,
            force_legacy_graphics: false,
            mptcp: false,
            fido2_enabled: false,
            microphone_enabled: false,
            smartcard_enabled: false,
            load_balance_info: None,
            kerberos_enabled: false,
            kdc_proxy_url: None,
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

    /// Returns the graphics pipeline this configuration will end up on if the
    /// EGFX channel never negotiates.
    ///
    /// Used as the initial status-bar value; a real GFX session overrides it
    /// with the confirmed capability set. When GFX is off, the pipeline follows
    /// the bitmap codecs we advertise, which are driven by the performance
    /// mode: Speed advertises no codecs at all (plain bitmap updates), while
    /// Quality and Balanced advertise RemoteFX.
    #[must_use]
    pub const fn assumed_graphics_mode(&self) -> GraphicsMode {
        if self.force_legacy_graphics || matches!(self.graphics_mode, GraphicsMode::Legacy) {
            return GraphicsMode::Legacy;
        }
        match self.performance_mode {
            RdpPerformanceMode::Speed => GraphicsMode::Legacy,
            RdpPerformanceMode::Quality | RdpPerformanceMode::Balanced => GraphicsMode::RemoteFx,
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
        self.password = Some(SecretString::from(password.into()));
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

    /// Enables or disables clipboard sharing.
    ///
    /// Thin shim over [`Self::with_clipboard_direction`]: `true` maps to
    /// [`RdpClipboardDirection::Both`], `false` to
    /// [`RdpClipboardDirection::Off`].
    #[must_use]
    pub const fn with_clipboard(mut self, enabled: bool) -> Self {
        self.clipboard_direction = if enabled {
            RdpClipboardDirection::Both
        } else {
            RdpClipboardDirection::Off
        };
        self
    }

    /// Sets which way the clipboard synchronises.
    #[must_use]
    pub const fn with_clipboard_direction(mut self, dir: RdpClipboardDirection) -> Self {
        self.clipboard_direction = dir;
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

    /// Enables or disables printer redirection
    #[must_use]
    pub const fn with_printer(mut self, enabled: bool) -> Self {
        self.printer_enabled = enabled;
        self
    }

    /// Sets what OS the server runs (tailors the toolbar and the resize path;
    /// issue #366)
    #[must_use]
    pub const fn with_server_os(mut self, os: rustconn_core::models::RdpServerOs) -> Self {
        self.server_os = os;
        self
    }

    /// Whether a widget resize must go through a full reconnect rather than the
    /// Display Control Channel (MS-RDPEDISP).
    ///
    /// True when the user forced it (`reconnect_on_resize`) or the server is a
    /// Linux RDP server. A Linux server (xrdp with xorgxrdp) accepts the
    /// MS-RDPEDISP monitor layout and resizes its RANDR screen, but — unlike
    /// Windows — does not follow it with a Deactivation-Reactivation Sequence,
    /// so the embedded client never rebuilds its framebuffer for the new size
    /// and the desktop stays at the old resolution with black bars. A reconnect
    /// starts a fresh session at the requested resolution, which is how a Linux
    /// server reaches the new size reliably.
    ///
    /// Also true once an `Auto` server has ignored a layout
    /// ([`Self::layout_resize_ignored`]): `Auto` cannot tell xrdp from Windows
    /// up front, so the first resize tries Display Control and the session
    /// client reports when nothing happened.
    #[must_use]
    pub fn resize_needs_reconnect(&self) -> bool {
        self.reconnect_on_resize
            || self.layout_resize_ignored
            || matches!(self.server_os, rustconn_core::models::RdpServerOs::Linux)
    }

    /// Whether a layout the server ignored should fall back to a reconnect.
    ///
    /// `Windows` keeps the seamless path: Windows applies layouts, so a slow
    /// answer is not a reason to tear the session down. `Auto` and `Linux`
    /// reconnect (a `Linux` session sends no layouts, so only `Auto` gets here
    /// in practice).
    #[must_use]
    pub fn falls_back_when_layout_ignored(&self) -> bool {
        !matches!(self.server_os, rustconn_core::models::RdpServerOs::Windows)
    }

    /// The config to reconnect with after the session client reported
    /// `DisplayControlUnavailable` for a `width` × `height` layout, or `None`
    /// when the session keeps Display Control instead
    /// ([`Self::falls_back_when_layout_ignored`]).
    ///
    /// An ignored layout also sets [`Self::layout_resize_ignored`]. The
    /// reconnect reuses this runtime config, so the flag lasts for the rest of
    /// the session: once it is set no layout is sent again, and nothing could
    /// show that a later one would be applied.
    #[must_use]
    pub fn after_display_control_unavailable(
        &self,
        width: u16,
        height: u16,
        layout_ignored: bool,
    ) -> Option<Self> {
        if layout_ignored && !self.falls_back_when_layout_ignored() {
            return None;
        }
        let mut next = self
            .clone()
            .with_resolution(u32::from(width), u32::from(height));
        next.layout_resize_ignored |= layout_ignored;
        Some(next)
    }

    /// Sets where the session audio is played
    #[must_use]
    pub const fn with_audio_mode(mut self, mode: rustconn_core::models::RdpAudioMode) -> Self {
        self.audio_mode = mode;
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
    Connect(Box<RdpConfig>),
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
    /// Send Ctrl+Alt+Del key sequence
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
    /// The embedded client's process exited
    ClientExited(EmbeddedClientExit),
}

/// How the embedded FreeRDP client's process ended.
///
/// Reported as an event so the widget stops showing "Connected" over a client
/// that is gone: an exit right after launch is a failed connection, a later
/// one is the session ending (issue #351).
#[derive(Debug, Clone)]
pub struct EmbeddedClientExit {
    /// Exit status as the operating system reported it
    pub status: std::process::ExitStatus,
    /// Time from the launch until the exit was noticed
    pub running_for: std::time::Duration,
    /// The last lines of the client's stderr, Qt noise removed
    pub stderr_tail: String,
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

/// Callback type for fallback notifications
pub type FallbackCallback = Box<dyn Fn(&str) + 'static>;

/// Resolves an explicit legacy-security decision exactly once.
pub type LegacySecurityDecision = Box<dyn FnOnce(bool) + 'static>;

/// Callback invoked when Standard RDP Security requires explicit consent.
pub type LegacySecurityCallback = Box<dyn Fn(LegacySecurityDecision) + 'static>;

/// Callback type for certificate change notifications.
///
/// Called when FreeRDP detects a server certificate mismatch. The callback
/// receives `(host, port, message)` and should show a confirmation dialog.
pub type CertChangedCallback = Box<dyn Fn(&str, u16, &str) + 'static>;

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
        assert_eq!(config.clipboard_direction, RdpClipboardDirection::Both);
        assert_eq!(config.polling_interval_ms, 33);
    }

    #[test]
    fn test_rdp_config_default_polling() {
        let config = RdpConfig::new("test.example.com");
        assert_eq!(config.polling_interval_ms, 16); // Default ~60 FPS
    }

    #[test]
    fn resize_needs_reconnect_follows_server_os_and_toggle() {
        use rustconn_core::models::RdpServerOs;

        // Default (Auto) with the toggle off uses the seamless MS-RDPEDISP path.
        let base = RdpConfig::new("host");
        assert!(!base.resize_needs_reconnect());

        // Windows keeps the seamless path — Windows honours MS-RDPEDISP.
        assert!(
            !base
                .clone()
                .with_server_os(RdpServerOs::Windows)
                .resize_needs_reconnect()
        );

        // Linux forces reconnect: xrdp resizes RANDR but sends no
        // Deactivation-Reactivation, so the seamless path leaves black bars.
        assert!(
            base.clone()
                .with_server_os(RdpServerOs::Linux)
                .resize_needs_reconnect()
        );

        // The explicit user toggle still forces reconnect regardless of OS.
        let mut forced = base.clone().with_server_os(RdpServerOs::Auto);
        forced.reconnect_on_resize = true;
        assert!(forced.resize_needs_reconnect());

        // An Auto server that ignored a layout reconnects from then on.
        let mut ignored = base.with_server_os(RdpServerOs::Auto);
        ignored.layout_resize_ignored = true;
        assert!(ignored.resize_needs_reconnect());
    }

    #[test]
    fn only_windows_keeps_display_control_when_a_layout_is_ignored() {
        use rustconn_core::models::RdpServerOs;

        let base = RdpConfig::new("host");
        assert!(base.falls_back_when_layout_ignored(), "Auto is the default");
        assert!(
            base.clone()
                .with_server_os(RdpServerOs::Linux)
                .falls_back_when_layout_ignored()
        );
        assert!(
            !base
                .with_server_os(RdpServerOs::Windows)
                .falls_back_when_layout_ignored()
        );
    }

    #[test]
    fn an_ignored_layout_on_auto_reconnects_at_the_new_size_for_good() {
        let auto = RdpConfig::new("host").with_resolution(1280, 800);
        let next = auto
            .after_display_control_unavailable(1600, 900, true)
            .expect("Auto falls back to a reconnect");
        assert_eq!((next.width, next.height), (1600, 900));
        assert!(next.layout_resize_ignored);
        assert!(next.resize_needs_reconnect());

        // The reconnect and every later resize start from this runtime config
        // (`reconnect`, `reconnect_with_resolution` and the resize handler
        // clone it and change the resolution), so the session keeps resizing
        // by reconnecting.
        let after_reconnect = next.clone().with_resolution(1920, 1080);
        assert!(after_reconnect.resize_needs_reconnect());

        // A second report keeps the flag.
        let again = after_reconnect
            .after_display_control_unavailable(1024, 768, false)
            .expect("an unavailable channel reconnects");
        assert!(again.layout_resize_ignored);
    }

    #[test]
    fn windows_keeps_display_control_when_a_layout_is_ignored() {
        let windows =
            RdpConfig::new("host").with_server_os(rustconn_core::models::RdpServerOs::Windows);
        assert!(
            windows
                .after_display_control_unavailable(1600, 900, true)
                .is_none()
        );
        assert!(!windows.resize_needs_reconnect());
    }

    #[test]
    fn an_unavailable_channel_reconnects_without_marking_the_layout_ignored() {
        use rustconn_core::models::RdpServerOs;

        for os in [RdpServerOs::Auto, RdpServerOs::Windows] {
            let next = RdpConfig::new("host")
                .with_server_os(os)
                .after_display_control_unavailable(1600, 900, false)
                .expect("no Display Control channel always reconnects");
            assert_eq!((next.width, next.height), (1600, 900));
            assert!(!next.layout_resize_ignored, "{os:?}");
            assert!(!next.resize_needs_reconnect(), "{os:?}");
        }
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

        let err = EmbeddedRdpError::NoSupportedFreeRdp(Some(
            rustconn_core::protocol::FreeRdpVersion::new(2, 11, 5),
        ));
        assert!(err.to_string().contains("FreeRDP 3"));
    }

    #[test]
    fn test_rdp_command_variants() {
        let config = RdpConfig::new("test.example.com");
        let cmd = RdpCommand::Connect(Box::new(config));
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
