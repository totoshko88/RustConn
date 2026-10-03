//! RDP client configuration

// Allow struct with multiple bools - RDP has many boolean options
#![allow(
    clippy::struct_excessive_bools,
    reason = "module-wide override for legacy code; refactored case by case"
)]

use std::path::PathBuf;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use super::gateway::GatewayConfig;
use super::graphics::{GraphicsMode, GraphicsQuality};
use super::multimonitor::MonitorLayout;
use crate::models::RdpPerformanceMode;

/// Shared folder configuration for RDP drive redirection
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedFolder {
    /// Display name for the shared folder (visible in Windows Explorer)
    pub name: String,
    /// Local path to share
    pub path: PathBuf,
    /// Read-only access
    pub read_only: bool,
}

impl SharedFolder {
    /// Creates a new shared folder configuration
    #[must_use]
    pub fn new(name: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            path: path.into(),
            read_only: false,
        }
    }

    /// Sets read-only mode
    #[must_use]
    pub const fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }
}

/// Configuration for RDP client connection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RdpClientConfig {
    /// Target hostname or IP address
    pub host: String,

    /// Target port (default: 3389)
    pub port: u16,

    /// Username for authentication
    pub username: Option<String>,

    /// Password for authentication (stored securely, never serialized)
    #[serde(skip)]
    pub password: Option<SecretString>,

    /// Domain for authentication
    pub domain: Option<String>,

    /// Desired screen width
    pub width: u16,

    /// Desired screen height
    pub height: u16,

    /// Color depth (16, 24, or 32)
    pub color_depth: u8,

    /// Enable clipboard sharing
    pub clipboard_enabled: bool,

    /// Enable audio redirection
    pub audio_enabled: bool,

    /// Connection timeout in seconds
    pub timeout_secs: u64,

    /// Ignore certificate errors (insecure, for testing)
    pub ignore_certificate: bool,

    /// Enable NLA (Network Level Authentication)
    pub nla_enabled: bool,

    /// Security protocol to use
    pub security_protocol: RdpSecurityProtocol,

    /// Shared folders for drive redirection (RDPDR)
    #[serde(default)]
    pub shared_folders: Vec<SharedFolder>,

    /// Enable dynamic resolution changes (MS-RDPEDISP)
    #[serde(default = "default_true")]
    pub dynamic_resolution: bool,

    /// Scale factor for `HiDPI` displays (100 = 100%)
    #[serde(default = "default_scale_factor")]
    pub scale_factor: u32,

    /// Performance mode (Quality/Balanced/Speed)
    #[serde(default)]
    pub performance_mode: RdpPerformanceMode,

    // ========== New fields for enhanced functionality ==========
    /// Graphics mode selection
    #[serde(default)]
    pub graphics_mode: GraphicsMode,

    /// Graphics quality settings
    #[serde(default)]
    pub graphics_quality: GraphicsQuality,

    /// RD Gateway configuration
    #[serde(default)]
    pub gateway: GatewayConfig,

    /// Multi-monitor layout
    #[serde(default)]
    pub monitor_layout: MonitorLayout,

    /// Enable printer redirection
    #[serde(default)]
    pub printer_enabled: bool,

    /// Specific CUPS queue names to forward. When empty and `printer_enabled`
    /// is true, all local queues are forwarded.
    #[serde(default)]
    pub printers: Vec<String>,

    /// Enable smart card redirection.
    ///
    /// **Reserved / not yet implemented.** The embedded IronRDP client has no
    /// smart-card (scard) virtual channel — `RustConnRdpdrBackend::handle_scard_call`
    /// is a no-op — so setting this currently has no effect. The field is kept
    /// so a future scard implementation and existing persisted configs round-trip
    /// without a schema change; it is not surfaced in the GUI.
    #[serde(default)]
    pub smartcard_enabled: bool,

    /// Enable microphone redirection.
    ///
    /// **Reserved / not yet implemented.** The embedded IronRDP client has no
    /// audio-input (audin) virtual channel, so setting this currently has no
    /// effect. The field is kept for forward-compatibility and round-tripping;
    /// it is not surfaced in the GUI.
    #[serde(default)]
    pub microphone_enabled: bool,

    /// Remote application mode (RemoteApp)
    #[serde(default)]
    pub remote_app: Option<RemoteAppConfig>,

    /// Connection name/label for display
    #[serde(default)]
    pub connection_name: Option<String>,

    /// Windows keyboard layout identifier (KLID).
    /// `None` means auto-detect from system settings.
    /// Example: `0x0407` for German, `0x040C` for French.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard_layout: Option<u32>,

    /// Enable Multipath TCP for the embedded RDP connection.
    /// Uses multiple network paths for seamless mobility and bandwidth aggregation.
    /// Requires kernel MPTCP support (Linux 5.6+). Falls back to regular TCP.
    #[serde(default)]
    pub mptcp: bool,

    /// Authenticate NLA (CredSSP) with Kerberos instead of NTLM.
    ///
    /// When enabled and NLA is on, the client hands IronRDP a `KerberosConfig`,
    /// so sspi signs in with Kerberos — using [`Self::password`]; a ticket from
    /// `kinit` is never read. This is what an account in the AD "Protected Users"
    /// group requires: that group forbids NTLM, so an NTLM sign-in fails with
    /// `STATUS_ACCOUNT_RESTRICTION` (0xc000006e) (issue #351).
    ///
    /// There is no NTLM fallback once Kerberos is chosen: a KDC that cannot be
    /// found or reached fails the sign-in. The KDC comes from
    /// [`Self::kdc_proxy_url`], else sspi's own lookup, else the realm's DNS name
    /// — see [`crate::rdp_client::kerberos`]. [`Self::host`] must be the server's
    /// DNS name and [`Self::domain`] the DNS domain. Off by default, as a
    /// deliberate per-connection opt-in.
    #[serde(default)]
    pub kerberos_enabled: bool,

    /// KDC address for [`Self::kerberos_enabled`], put through
    /// [`crate::rdp_client::kerberos::normalize_kdc_url`] before use.
    ///
    /// `tcp://` or `udp://` for a domain controller, `http://` or `https://` for
    /// an MS-KKDCP KDC proxy such as `https://gateway.example.com/KdcProxy`,
    /// which reaches a KDC with no line of sight to port 88. `None` means sspi
    /// looks the KDC up (`SSPI_KDC_URL_<REALM>`, `SSPI_KDC_URL`, `krb5.conf`
    /// `[realms]`), and failing that the realm's own DNS name is tried.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kdc_proxy_url: Option<String>,
}

const fn default_true() -> bool {
    true
}

const fn default_scale_factor() -> u32 {
    100
}

/// RemoteApp configuration for running individual applications
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteAppConfig {
    /// Application alias or path
    pub program: String,
    /// Working directory
    pub working_dir: Option<String>,
    /// Command line arguments
    pub arguments: Option<String>,
    /// Expand environment variables in arguments
    pub expand_env: bool,
}

impl RemoteAppConfig {
    /// Creates a new RemoteApp configuration
    #[must_use]
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            working_dir: None,
            arguments: None,
            expand_env: false,
        }
    }

    /// Sets the working directory
    #[must_use]
    pub fn with_working_dir(mut self, dir: impl Into<String>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    /// Sets command line arguments
    #[must_use]
    pub fn with_arguments(mut self, args: impl Into<String>) -> Self {
        self.arguments = Some(args.into());
        self
    }
}

/// RDP security protocol options
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RdpSecurityProtocol {
    /// Automatic selection (server decides)
    #[default]
    Auto,
    /// Standard RDP security
    Rdp,
    /// TLS encryption
    Tls,
    /// Network Level Authentication
    Nla,
    /// Extended NLA (`CredSSP` with early user auth)
    Ext,
}

impl Default for RdpClientConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 3389,
            username: None,
            password: None,
            domain: None,
            width: 1280,
            height: 720,
            color_depth: 32,
            clipboard_enabled: true,
            audio_enabled: false,
            timeout_secs: 30,
            ignore_certificate: false,
            nla_enabled: true,
            security_protocol: RdpSecurityProtocol::default(),
            shared_folders: Vec::new(),
            dynamic_resolution: true,
            scale_factor: 100,
            performance_mode: RdpPerformanceMode::default(),
            // New fields
            graphics_mode: GraphicsMode::default(),
            graphics_quality: GraphicsQuality::default(),
            gateway: GatewayConfig::default(),
            monitor_layout: MonitorLayout::default(),
            printer_enabled: false,
            printers: Vec::new(),
            smartcard_enabled: false,
            microphone_enabled: false,
            remote_app: None,
            connection_name: None,
            keyboard_layout: None,
            mptcp: false,
            kerberos_enabled: false,
            kdc_proxy_url: None,
        }
    }
}

impl RdpClientConfig {
    /// Creates a new configuration with the specified host
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
    pub const fn with_resolution(mut self, width: u16, height: u16) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Sets the color depth
    #[must_use]
    pub const fn with_color_depth(mut self, depth: u8) -> Self {
        self.color_depth = depth;
        self
    }

    /// Enables or disables clipboard sharing
    #[must_use]
    pub const fn with_clipboard(mut self, enabled: bool) -> Self {
        self.clipboard_enabled = enabled;
        self
    }

    /// Enables or disables NLA
    #[must_use]
    pub const fn with_nla(mut self, enabled: bool) -> Self {
        self.nla_enabled = enabled;
        self
    }

    /// Adds a shared folder for drive redirection
    #[must_use]
    pub fn with_shared_folder(mut self, folder: SharedFolder) -> Self {
        self.shared_folders.push(folder);
        self
    }

    /// Adds multiple shared folders
    #[must_use]
    pub fn with_shared_folders(mut self, folders: Vec<SharedFolder>) -> Self {
        self.shared_folders = folders;
        self
    }

    /// Enables or disables dynamic resolution
    #[must_use]
    pub const fn with_dynamic_resolution(mut self, enabled: bool) -> Self {
        self.dynamic_resolution = enabled;
        self
    }

    /// Sets the scale factor for `HiDPI` displays
    #[must_use]
    pub const fn with_scale_factor(mut self, factor: u32) -> Self {
        self.scale_factor = factor;
        self
    }

    /// Sets the performance mode (Quality/Balanced/Speed)
    #[must_use]
    pub const fn with_performance_mode(mut self, mode: RdpPerformanceMode) -> Self {
        self.performance_mode = mode;
        self
    }

    /// Sets the graphics mode
    #[must_use]
    pub const fn with_graphics_mode(mut self, mode: GraphicsMode) -> Self {
        self.graphics_mode = mode;
        self
    }

    /// Sets the graphics quality
    #[must_use]
    pub const fn with_graphics_quality(mut self, quality: GraphicsQuality) -> Self {
        self.graphics_quality = quality;
        self
    }

    /// Sets the RD Gateway configuration
    #[must_use]
    pub fn with_gateway(mut self, gateway: GatewayConfig) -> Self {
        self.gateway = gateway;
        self
    }

    /// Sets the monitor layout
    #[must_use]
    pub fn with_monitor_layout(mut self, layout: MonitorLayout) -> Self {
        self.monitor_layout = layout;
        self
    }

    /// Enables or disables audio
    #[must_use]
    pub const fn with_audio(mut self, enabled: bool) -> Self {
        self.audio_enabled = enabled;
        self
    }

    /// Enables or disables printer redirection
    #[must_use]
    pub const fn with_printer(mut self, enabled: bool) -> Self {
        self.printer_enabled = enabled;
        self
    }

    /// Restricts printer redirection to the named CUPS queues.
    ///
    /// An empty list (the default) forwards all local queues when
    /// `printer_enabled` is true.
    #[must_use]
    pub fn with_printers(mut self, queues: Vec<String>) -> Self {
        self.printers = queues;
        self
    }

    /// Sets the smart-card redirection flag.
    ///
    /// **Reserved / not yet implemented** — see [`RdpClientConfig::smartcard_enabled`].
    /// Setting it currently has no effect on the embedded client.
    #[must_use]
    pub const fn with_smartcard(mut self, enabled: bool) -> Self {
        self.smartcard_enabled = enabled;
        self
    }

    /// Sets RemoteApp configuration
    #[must_use]
    pub fn with_remote_app(mut self, app: RemoteAppConfig) -> Self {
        self.remote_app = Some(app);
        self
    }

    /// Sets the connection name
    #[must_use]
    pub fn with_connection_name(mut self, name: impl Into<String>) -> Self {
        self.connection_name = Some(name.into());
        self
    }

    /// Sets the keyboard layout (Windows KLID).
    /// If not set, the layout is auto-detected at connection time.
    #[must_use]
    pub const fn with_keyboard_layout(mut self, klid: u32) -> Self {
        self.keyboard_layout = Some(klid);
        self
    }

    /// Enables or disables Kerberos authentication for NLA (issue #351).
    ///
    /// Only takes effect when NLA is also enabled; with NLA off there is no
    /// CredSSP exchange to negotiate.
    #[must_use]
    pub const fn with_kerberos(mut self, enabled: bool) -> Self {
        self.kerberos_enabled = enabled;
        self
    }

    /// Sets the KDC address used when Kerberos is enabled: a domain controller
    /// (`tcp://`, `udp://`) or an MS-KKDCP KDC proxy URL. An empty string clears it.
    #[must_use]
    pub fn with_kdc_proxy_url(mut self, url: impl Into<String>) -> Self {
        let url = url.into();
        self.kdc_proxy_url = if url.is_empty() { None } else { Some(url) };
        self
    }

    /// Returns the server address as "host:port"
    #[must_use]
    pub fn server_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Returns whether this connection uses a gateway
    #[must_use]
    pub fn uses_gateway(&self) -> bool {
        self.gateway.enabled && !self.gateway.should_bypass(&self.host)
    }

    /// Returns whether this is a RemoteApp session
    #[must_use]
    pub const fn is_remote_app(&self) -> bool {
        self.remote_app.is_some()
    }

    /// Returns whether multi-monitor is enabled
    #[must_use]
    pub fn is_multimonitor(&self) -> bool {
        self.monitor_layout.is_multimonitor()
    }

    /// Validates the configuration
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid.
    pub fn validate(&self) -> Result<(), ConfigValidationError> {
        if self.host.is_empty() {
            return Err(ConfigValidationError::MissingHost);
        }

        if self.port == 0 {
            return Err(ConfigValidationError::InvalidPort);
        }

        if !matches!(self.color_depth, 8 | 15 | 16 | 24 | 32) {
            return Err(ConfigValidationError::InvalidColorDepth(self.color_depth));
        }

        if self.width == 0 || self.height == 0 {
            return Err(ConfigValidationError::InvalidResolution);
        }

        self.gateway
            .validate()
            .map_err(|e| ConfigValidationError::GatewayError(e.to_string()))?;

        Ok(())
    }
}

// Manual PartialEq implementation since SecretString doesn't implement it
impl PartialEq for RdpClientConfig {
    fn eq(&self, other: &Self) -> bool {
        self.host == other.host
            && self.port == other.port
            && self.username == other.username
            && match (&self.password, &other.password) {
                (Some(a), Some(b)) => a.expose_secret() == b.expose_secret(),
                (None, None) => true,
                _ => false,
            }
            && self.domain == other.domain
            && self.width == other.width
            && self.height == other.height
            && self.color_depth == other.color_depth
            && self.clipboard_enabled == other.clipboard_enabled
            && self.audio_enabled == other.audio_enabled
            && self.timeout_secs == other.timeout_secs
            && self.ignore_certificate == other.ignore_certificate
            && self.nla_enabled == other.nla_enabled
            && self.security_protocol == other.security_protocol
            && self.shared_folders == other.shared_folders
            && self.dynamic_resolution == other.dynamic_resolution
            && self.scale_factor == other.scale_factor
            && self.performance_mode == other.performance_mode
            && self.graphics_mode == other.graphics_mode
            && self.graphics_quality == other.graphics_quality
            && self.gateway == other.gateway
            && self.monitor_layout == other.monitor_layout
            && self.printer_enabled == other.printer_enabled
            && self.printers == other.printers
            && self.smartcard_enabled == other.smartcard_enabled
            && self.microphone_enabled == other.microphone_enabled
            && self.remote_app == other.remote_app
            && self.connection_name == other.connection_name
            && self.keyboard_layout == other.keyboard_layout
            && self.kerberos_enabled == other.kerberos_enabled
            && self.kdc_proxy_url == other.kdc_proxy_url
    }
}

/// Configuration validation errors
#[derive(Debug, Clone, thiserror::Error)]
pub enum ConfigValidationError {
    /// Host is missing
    #[error("Host is required")]
    MissingHost,

    /// Invalid port
    #[error("Port cannot be 0")]
    InvalidPort,

    /// Invalid color depth
    #[error("Invalid color depth: {0}. Must be 8, 15, 16, 24, or 32")]
    InvalidColorDepth(u8),

    /// Invalid resolution
    #[error("Resolution cannot be 0x0")]
    InvalidResolution,

    /// Gateway configuration error
    #[error("Gateway configuration error: {0}")]
    GatewayError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_builder() {
        let config = RdpClientConfig::new("192.168.1.100")
            .with_port(3390)
            .with_username("admin")
            .with_password("secret")
            .with_domain("CORP")
            .with_resolution(1920, 1080)
            .with_color_depth(24);

        assert_eq!(config.host, "192.168.1.100");
        assert_eq!(config.port, 3390);
        assert_eq!(config.username, Some("admin".to_string()));
        assert_eq!(
            config
                .password
                .as_ref()
                .map(|p| p.expose_secret().to_string()),
            Some("secret".to_string())
        );
        assert_eq!(config.domain, Some("CORP".to_string()));
        assert_eq!(config.width, 1920);
        assert_eq!(config.height, 1080);
        assert_eq!(config.color_depth, 24);
    }

    #[test]
    fn test_server_address() {
        let config = RdpClientConfig::new("localhost").with_port(3389);
        assert_eq!(config.server_address(), "localhost:3389");
    }

    #[test]
    fn test_default_values() {
        let config = RdpClientConfig::default();
        assert_eq!(config.port, 3389);
        assert_eq!(config.color_depth, 32);
        assert!(config.clipboard_enabled);
        assert!(config.nla_enabled);
    }

    #[test]
    fn gateway_is_used_for_a_target_that_only_exists_behind_it() {
        // Regression guard for issue #246: the target of a gateway connection is
        // typically an internal name or a private address, and the connect path
        // must tunnel instead of resolving it locally.
        for host in ["host.internal", "10.10.0.5", "192.168.1.20"] {
            let config = RdpClientConfig::new(host).with_gateway(GatewayConfig::always_for_target(
                "gw.example.com",
                443,
                None,
            ));
            assert!(config.uses_gateway(), "host must use the gateway: {host}");
        }
    }

    #[test]
    fn no_gateway_configured_means_direct_connect() {
        let config = RdpClientConfig::new("host.example.com");
        assert!(!config.uses_gateway());
    }

    #[test]
    fn mptcp_opt_in_is_carried_on_the_client_config() {
        // Regression guard for the MPTCP toggle being a silent no-op. The
        // connect path branches on `config.mptcp` (rdp_client/client/connection
        // .rs: `if config.mptcp { connect_mptcp_async(..) }`), and the GUI
        // model->embedded mapper (rustconn/src/window/rdp_vnc.rs) used to drop
        // this field entirely. It must default off, round-trip, and persist
        // when the user opts in.
        let default_config = RdpClientConfig::new("rdp.example.com");
        assert!(
            !default_config.mptcp,
            "MPTCP must stay off until the user opts in"
        );

        let mut config = RdpClientConfig::new("rdp.example.com");
        config.mptcp = true;
        assert!(config.mptcp, "an explicit opt-in must be honoured");

        // Survives a serde round-trip (the stored per-connection value).
        let json = serde_json::to_string(&config).expect("serialize");
        let restored: RdpClientConfig = serde_json::from_str(&json).expect("deserialize");
        assert!(
            restored.mptcp,
            "MPTCP opt-in must survive serialization round-trip"
        );
    }
}
