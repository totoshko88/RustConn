//! Protocol configuration types for SSH, RDP, and VNC connections.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Protocol type identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolType {
    /// SSH protocol
    Ssh,
    /// RDP protocol
    Rdp,
    /// VNC protocol
    Vnc,
    /// SPICE protocol
    Spice,
    /// Telnet protocol
    Telnet,
    /// Zero Trust connection (cloud-based secure access)
    ZeroTrust,
    /// Serial console protocol
    Serial,
    /// SFTP file transfer protocol (SSH-based)
    Sftp,
    /// Kubernetes pod shell (kubectl exec)
    Kubernetes,
}

impl ProtocolType {
    /// Returns the protocol identifier as a lowercase string
    ///
    /// This matches the protocol IDs used in the protocol registry.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Ssh => "ssh",
            Self::Rdp => "rdp",
            Self::Vnc => "vnc",
            Self::Spice => "spice",
            Self::Telnet => "telnet",
            Self::ZeroTrust => "zerotrust",
            Self::Serial => "serial",
            Self::Sftp => "sftp",
            Self::Kubernetes => "kubernetes",
        }
    }

    /// Returns the default port for this protocol type
    #[must_use]
    pub const fn default_port(&self) -> u16 {
        match self {
            Self::Ssh => 22,
            Self::Rdp => 3389,
            Self::Vnc | Self::Spice => 5900,
            Self::Telnet => 23,
            Self::ZeroTrust | Self::Serial => 0,
            Self::Sftp => 22,
            Self::Kubernetes => 0,
        }
    }
}

impl std::fmt::Display for ProtocolType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ssh => write!(f, "SSH"),
            Self::Rdp => write!(f, "RDP"),
            Self::Vnc => write!(f, "VNC"),
            Self::Spice => write!(f, "SPICE"),
            Self::Telnet => write!(f, "Telnet"),
            Self::ZeroTrust => write!(f, "Zero Trust"),
            Self::Serial => write!(f, "Serial"),
            Self::Sftp => write!(f, "SFTP"),
            Self::Kubernetes => write!(f, "Kubernetes"),
        }
    }
}

/// Protocol-specific configuration
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProtocolConfig {
    /// SSH protocol configuration
    Ssh(SshConfig),
    /// RDP protocol configuration
    Rdp(RdpConfig),
    /// VNC protocol configuration
    Vnc(VncConfig),
    /// SPICE protocol configuration
    Spice(SpiceConfig),
    /// Telnet protocol configuration
    Telnet(TelnetConfig),
    /// Zero Trust connection configuration
    ZeroTrust(ZeroTrustConfig),
    /// Serial console protocol configuration
    Serial(SerialConfig),
    /// SFTP file transfer configuration (reuses SSH config)
    Sftp(SshConfig),
    /// Kubernetes pod shell configuration
    Kubernetes(KubernetesConfig),
}

impl ProtocolConfig {
    /// Returns the protocol type for this configuration
    #[must_use]
    pub const fn protocol_type(&self) -> ProtocolType {
        match self {
            Self::Ssh(_) => ProtocolType::Ssh,
            Self::Rdp(_) => ProtocolType::Rdp,
            Self::Vnc(_) => ProtocolType::Vnc,
            Self::Spice(_) => ProtocolType::Spice,
            Self::Telnet(_) => ProtocolType::Telnet,
            Self::ZeroTrust(_) => ProtocolType::ZeroTrust,
            Self::Serial(_) => ProtocolType::Serial,
            Self::Sftp(_) => ProtocolType::Sftp,
            Self::Kubernetes(_) => ProtocolType::Kubernetes,
        }
    }
}

/// What the Backspace key sends in a Telnet session
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TelnetBackspaceSends {
    /// Automatic (use terminal default)
    #[default]
    Automatic,
    /// Send Backspace (^H, 0x08)
    Backspace,
    /// Send Delete (^?, 0x7F)
    Delete,
}

impl TelnetBackspaceSends {
    /// Returns all available options
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Automatic, Self::Backspace, Self::Delete]
    }

    /// Returns the display name for this option
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic",
            Self::Backspace => "Backspace (^H)",
            Self::Delete => "Delete (^?)",
        }
    }

    /// Returns the index of this option in the `all()` array
    #[must_use]
    pub const fn index(self) -> u32 {
        match self {
            Self::Automatic => 0,
            Self::Backspace => 1,
            Self::Delete => 2,
        }
    }

    /// Creates an option from an index
    #[must_use]
    pub const fn from_index(index: u32) -> Self {
        match index {
            1 => Self::Backspace,
            2 => Self::Delete,
            _ => Self::Automatic,
        }
    }
}

/// What the Delete key sends in a Telnet session
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TelnetDeleteSends {
    /// Automatic (use terminal default)
    #[default]
    Automatic,
    /// Send Backspace (^H, 0x08)
    Backspace,
    /// Send Delete (^?, 0x7F)
    Delete,
}

impl TelnetDeleteSends {
    /// Returns all available options
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Automatic, Self::Backspace, Self::Delete]
    }

    /// Returns the display name for this option
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic",
            Self::Backspace => "Backspace (^H)",
            Self::Delete => "Delete (^?)",
        }
    }

    /// Returns the index of this option in the `all()` array
    #[must_use]
    pub const fn index(self) -> u32 {
        match self {
            Self::Automatic => 0,
            Self::Backspace => 1,
            Self::Delete => 2,
        }
    }

    /// Creates an option from an index
    #[must_use]
    pub const fn from_index(index: u32) -> Self {
        match index {
            1 => Self::Backspace,
            2 => Self::Delete,
            _ => Self::Automatic,
        }
    }
}

/// Telnet protocol configuration
///
/// Configuration for Telnet connections including keyboard behavior.
/// Telnet sessions are spawned via VTE terminal using an external `telnet` client.
///
/// The backspace/delete key settings address a common issue where these keys
/// are inverted on some remote systems. Users can configure what each key sends
/// to match the remote system's expectations.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelnetConfig {
    /// Custom command-line arguments for the telnet client
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_args: Vec<String>,
    /// What the Backspace key sends
    #[serde(default)]
    pub backspace_sends: TelnetBackspaceSends,
    /// What the Delete key sends
    #[serde(default)]
    pub delete_sends: TelnetDeleteSends,
}

/// Serial port baud rate
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SerialBaudRate {
    /// 9600 baud
    B9600,
    /// 19200 baud
    B19200,
    /// 38400 baud
    B38400,
    /// 57600 baud
    B57600,
    /// 115200 baud (default)
    #[default]
    B115200,
    /// 230400 baud
    B230400,
    /// 460800 baud
    B460800,
    /// 921600 baud
    B921600,
}

impl SerialBaudRate {
    /// Returns all available baud rates
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::B9600,
            Self::B19200,
            Self::B38400,
            Self::B57600,
            Self::B115200,
            Self::B230400,
            Self::B460800,
            Self::B921600,
        ]
    }

    /// Returns the display name for this baud rate
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::B9600 => "9600",
            Self::B19200 => "19200",
            Self::B38400 => "38400",
            Self::B57600 => "57600",
            Self::B115200 => "115200",
            Self::B230400 => "230400",
            Self::B460800 => "460800",
            Self::B921600 => "921600",
        }
    }

    /// Returns the index of this baud rate in the `all()` array
    #[must_use]
    pub const fn index(self) -> u32 {
        match self {
            Self::B9600 => 0,
            Self::B19200 => 1,
            Self::B38400 => 2,
            Self::B57600 => 3,
            Self::B115200 => 4,
            Self::B230400 => 5,
            Self::B460800 => 6,
            Self::B921600 => 7,
        }
    }

    /// Creates a baud rate from an index
    #[must_use]
    pub const fn from_index(index: u32) -> Self {
        match index {
            0 => Self::B9600,
            1 => Self::B19200,
            2 => Self::B38400,
            3 => Self::B57600,
            5 => Self::B230400,
            6 => Self::B460800,
            7 => Self::B921600,
            _ => Self::B115200,
        }
    }

    /// Returns the numeric baud rate value
    #[must_use]
    pub const fn value(self) -> u32 {
        match self {
            Self::B9600 => 9600,
            Self::B19200 => 19_200,
            Self::B38400 => 38_400,
            Self::B57600 => 57_600,
            Self::B115200 => 115_200,
            Self::B230400 => 230_400,
            Self::B460800 => 460_800,
            Self::B921600 => 921_600,
        }
    }
}

/// Serial port data bits
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SerialDataBits {
    /// 5 data bits
    Five,
    /// 6 data bits
    Six,
    /// 7 data bits
    Seven,
    /// 8 data bits (default)
    #[default]
    Eight,
}

impl SerialDataBits {
    /// Returns all available data bit options
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Five, Self::Six, Self::Seven, Self::Eight]
    }

    /// Returns the display name for this option
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Five => "5",
            Self::Six => "6",
            Self::Seven => "7",
            Self::Eight => "8",
        }
    }

    /// Returns the index of this option in the `all()` array
    #[must_use]
    pub const fn index(self) -> u32 {
        match self {
            Self::Five => 0,
            Self::Six => 1,
            Self::Seven => 2,
            Self::Eight => 3,
        }
    }

    /// Creates an option from an index
    #[must_use]
    pub const fn from_index(index: u32) -> Self {
        match index {
            0 => Self::Five,
            1 => Self::Six,
            2 => Self::Seven,
            _ => Self::Eight,
        }
    }

    /// Returns the numeric data bits value
    #[must_use]
    pub const fn value(self) -> u8 {
        match self {
            Self::Five => 5,
            Self::Six => 6,
            Self::Seven => 7,
            Self::Eight => 8,
        }
    }
}

/// Serial port stop bits
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SerialStopBits {
    /// 1 stop bit (default)
    #[default]
    One,
    /// 2 stop bits
    Two,
}

impl SerialStopBits {
    /// Returns all available stop bit options
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::One, Self::Two]
    }

    /// Returns the display name for this option
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::One => "1",
            Self::Two => "2",
        }
    }

    /// Returns the index of this option in the `all()` array
    #[must_use]
    pub const fn index(self) -> u32 {
        match self {
            Self::One => 0,
            Self::Two => 1,
        }
    }

    /// Creates an option from an index
    #[must_use]
    pub const fn from_index(index: u32) -> Self {
        match index {
            1 => Self::Two,
            _ => Self::One,
        }
    }
}

/// Serial port parity
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SerialParity {
    /// No parity (default)
    #[default]
    None,
    /// Odd parity
    Odd,
    /// Even parity
    Even,
}

impl SerialParity {
    /// Returns all available parity options
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::None, Self::Odd, Self::Even]
    }

    /// Returns the display name for this option
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Odd => "Odd",
            Self::Even => "Even",
        }
    }

    /// Returns the index of this option in the `all()` array
    #[must_use]
    pub const fn index(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Odd => 1,
            Self::Even => 2,
        }
    }

    /// Creates an option from an index
    #[must_use]
    pub const fn from_index(index: u32) -> Self {
        match index {
            1 => Self::Odd,
            2 => Self::Even,
            _ => Self::None,
        }
    }
}

/// Serial port flow control
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SerialFlowControl {
    /// No flow control (default)
    #[default]
    None,
    /// Hardware flow control (RTS/CTS)
    Hardware,
    /// Software flow control (XON/XOFF)
    Software,
}

impl SerialFlowControl {
    /// Returns all available flow control options
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::None, Self::Hardware, Self::Software]
    }

    /// Returns the display name for this option
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Hardware => "Hardware (RTS/CTS)",
            Self::Software => "Software (XON/XOFF)",
        }
    }

    /// Returns the index of this option in the `all()` array
    #[must_use]
    pub const fn index(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Hardware => 1,
            Self::Software => 2,
        }
    }

    /// Creates an option from an index
    #[must_use]
    pub const fn from_index(index: u32) -> Self {
        match index {
            1 => Self::Hardware,
            2 => Self::Software,
            _ => Self::None,
        }
    }
}

/// Serial console protocol configuration
///
/// Configuration for serial port connections. Serial sessions are
/// spawned via VTE terminal using an external serial client
/// (`picocom`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerialConfig {
    /// Serial device path (e.g., /dev/ttyUSB0, /dev/ttyACM0)
    pub device: String,
    /// Baud rate
    #[serde(default)]
    pub baud_rate: SerialBaudRate,
    /// Data bits
    #[serde(default)]
    pub data_bits: SerialDataBits,
    /// Stop bits
    #[serde(default)]
    pub stop_bits: SerialStopBits,
    /// Parity
    #[serde(default)]
    pub parity: SerialParity,
    /// Flow control
    #[serde(default)]
    pub flow_control: SerialFlowControl,
    /// Custom command-line arguments for the serial client
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_args: Vec<String>,
}

/// SSH authentication method
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SshAuthMethod {
    /// Password authentication
    #[default]
    Password,
    /// Public key authentication
    PublicKey,
    /// Keyboard-interactive authentication
    KeyboardInteractive,
    /// SSH agent authentication
    Agent,
    /// FIDO2/Security Key authentication (sk-ssh-ed25519, sk-ecdsa)
    SecurityKey,
}

/// SSH protocol configuration
// Allow 6 bools - these are distinct SSH connection options that map directly to CLI flags
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshConfig {
    /// Authentication method
    #[serde(default)]
    pub auth_method: SshAuthMethod,
    /// Path to SSH private key file
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_path: Option<PathBuf>,
    /// Key source (file, agent, or default)
    #[serde(default, skip_serializing_if = "is_default_key_source")]
    pub key_source: SshKeySource,
    /// Agent key fingerprint (when using agent key source)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_key_fingerprint: Option<String>,
    /// Use only the specified identity file (prevents "Too many authentication failures")
    /// When enabled, adds `-o IdentitiesOnly=yes` to the SSH command
    #[serde(default)]
    pub identities_only: bool,
    /// `ProxyJump` configuration (host or user@host)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_jump: Option<String>,
    /// Enable SSH `ControlMaster` for connection multiplexing
    #[serde(default)]
    pub use_control_master: bool,
    /// Enable SSH agent forwarding (`-A` flag)
    /// Allows the remote host to use local SSH agent for authentication
    #[serde(default)]
    pub agent_forwarding: bool,
    /// Enable X11 forwarding (`-X` flag)
    /// Allows running graphical applications on the remote host
    #[serde(default)]
    pub x11_forwarding: bool,
    /// Enable compression (`-C` flag)
    /// Compresses all data for faster transfer over slow connections
    #[serde(default)]
    pub compression: bool,
    /// Custom SSH options (key-value pairs)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub custom_options: HashMap<String, String>,
    /// Command to execute on connection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub startup_command: Option<String>,
    /// ID of another connection to use as a Jump Host
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jump_host_id: Option<uuid::Uuid>,
    /// Enable SFTP file browser for this SSH connection
    /// (always true — SFTP is available for all SSH connections)
    #[serde(default = "default_true")]
    pub sftp_enabled: bool,
}

fn default_true() -> bool {
    true
}

impl SshConfig {
    /// Builds SSH command arguments based on the configuration
    ///
    /// Returns a vector of command-line arguments to pass to the SSH command.
    /// This includes options like `-o IdentitiesOnly=yes` when enabled.
    ///
    /// # Key Selection Behavior
    ///
    /// - **File auth method**: When `key_source` is `SshKeySource::File`, adds `-i <path>`
    ///   and `-o IdentitiesOnly=yes` to prevent SSH from trying other keys (avoiding
    ///   "Too many authentication failures" errors).
    /// - **Agent auth method**: When `key_source` is `SshKeySource::Agent`, uses the key
    ///   comment (which often contains the key file path) to specify the identity file.
    ///   SSH will match this to the corresponding key in the agent.
    /// - **Legacy behavior**: If `identities_only` is explicitly set to true, it will
    ///   still be honored for backward compatibility.
    #[must_use]
    pub fn build_command_args(&self) -> Vec<String> {
        let mut args = Vec::new();

        // Determine if we should add IdentitiesOnly based on key source
        // File auth method should always use IdentitiesOnly to prevent "Too many auth failures"
        // Agent auth method with a valid key path should also use IdentitiesOnly
        let mut should_use_identities_only =
            self.identities_only || matches!(self.key_source, SshKeySource::File { .. });

        // Add identity file if specified via key_source (preferred) or key_path (legacy)
        match &self.key_source {
            SshKeySource::File { path } if !path.as_os_str().is_empty() => {
                args.push("-i".to_string());
                args.push(path.display().to_string());
            }
            SshKeySource::Agent { comment, .. } => {
                // The comment often contains the key file path (e.g., "/home/user/.ssh/id_ed25519")
                // If it's a valid path, use it with -i flag - SSH will match it to the agent key
                if !comment.is_empty() {
                    let key_path = std::path::Path::new(comment);
                    // Check if comment looks like a file path and the public key exists
                    if comment.starts_with('/') || comment.starts_with('~') {
                        // Expand ~ to home directory
                        let expanded_path = if comment.starts_with('~') {
                            dirs::home_dir().map_or_else(
                                || key_path.to_path_buf(),
                                |home| home.join(comment.strip_prefix("~/").unwrap_or(comment)),
                            )
                        } else {
                            key_path.to_path_buf()
                        };

                        // Check if the key file or its .pub version exists
                        let pub_path =
                            expanded_path.with_extension(expanded_path.extension().map_or_else(
                                || "pub".to_string(),
                                |e| format!("{}.pub", e.to_string_lossy()),
                            ));

                        if expanded_path.exists() || pub_path.exists() {
                            args.push("-i".to_string());
                            args.push(expanded_path.display().to_string());
                            // Enable IdentitiesOnly to use only this specific key
                            should_use_identities_only = true;
                        }
                    }
                }
                // If comment is not a valid path, SSH will try all agent keys (no -i flag added)
            }
            SshKeySource::Default | SshKeySource::File { .. } => {
                // Default or File with empty path - check legacy key_path field
                if let Some(ref key_path) = self.key_path {
                    if !key_path.as_os_str().is_empty() {
                        args.push("-i".to_string());
                        args.push(key_path.display().to_string());
                    }
                }
            }
        }

        // Add IdentitiesOnly option if needed (after -i flag for proper ordering)
        // This prevents SSH from trying other keys when a specific key file is selected
        if should_use_identities_only {
            args.push("-o".to_string());
            args.push("IdentitiesOnly=yes".to_string());
        }

        // Add proxy jump if specified
        if let Some(ref proxy) = self.proxy_jump {
            args.push("-J".to_string());
            args.push(proxy.clone());
        }

        // Add control master options if enabled
        if self.use_control_master {
            args.push("-o".to_string());
            args.push("ControlMaster=auto".to_string());
            args.push("-o".to_string());
            args.push("ControlPersist=10m".to_string());
        }

        // Add agent forwarding if enabled
        if self.agent_forwarding {
            args.push("-A".to_string());
        }

        // Add X11 forwarding if enabled
        if self.x11_forwarding {
            args.push("-X".to_string());
        }

        // Add compression if enabled
        if self.compression {
            args.push("-C".to_string());
        }

        // Add custom options
        for (key, value) in &self.custom_options {
            args.push("-o".to_string());
            args.push(format!("{key}={value}"));
        }

        args
    }

    /// Checks if this SSH config uses File authentication method
    ///
    /// Returns true if `key_source` is `SshKeySource::File` with a non-empty path.
    #[must_use]
    pub fn uses_file_auth(&self) -> bool {
        matches!(&self.key_source, SshKeySource::File { path } if !path.as_os_str().is_empty())
    }

    /// Checks if this SSH config uses Agent authentication method
    ///
    /// Returns true if `key_source` is `SshKeySource::Agent`.
    #[must_use]
    pub const fn uses_agent_auth(&self) -> bool {
        matches!(&self.key_source, SshKeySource::Agent { .. })
    }
}

/// Key source for SSH connections
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum SshKeySource {
    /// Key from file path
    File {
        /// Path to the key file
        path: PathBuf,
    },
    /// Key from SSH agent (identified by fingerprint)
    Agent {
        /// Key fingerprint for identification
        fingerprint: String,
        /// Key comment for display
        comment: String,
    },
    /// No specific key (use default SSH behavior)
    #[default]
    Default,
}

/// Helper function for serde to skip serializing default key source
const fn is_default_key_source(source: &SshKeySource) -> bool {
    matches!(source, SshKeySource::Default)
}

/// Screen resolution
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    /// Width in pixels
    pub width: u32,
    /// Height in pixels
    pub height: u32,
}

impl Resolution {
    /// Creates a new resolution
    #[must_use]
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

/// RDP gateway configuration
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdpGateway {
    /// Gateway hostname
    pub hostname: String,
    /// Gateway port (default: 443)
    #[serde(default = "default_gateway_port")]
    pub port: u16,
    /// Gateway username (if different from connection username)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

/// A shared folder for RDP connections
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedFolder {
    /// Local directory path to share
    pub local_path: PathBuf,
    /// Share name visible in the remote session
    pub share_name: String,
}

const fn default_gateway_port() -> u16 {
    443
}

/// RDP performance mode for quality/speed tradeoff
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RdpPerformanceMode {
    /// Best quality - RemoteFX codec, lossless compression, all visual effects
    Quality,
    /// Balanced - RemoteFX codec, adaptive compression, font smoothing
    #[default]
    Balanced,
    /// Best speed - Legacy bitmap, maximum compression, no visual effects
    Speed,
}

impl RdpPerformanceMode {
    /// Returns all available performance modes
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Quality, Self::Balanced, Self::Speed]
    }

    /// Returns the display name for this mode
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Quality => "Quality (RemoteFX)",
            Self::Balanced => "Balanced (Adaptive)",
            Self::Speed => "Speed (Legacy)",
        }
    }

    /// Returns the index of this mode in the `all()` array
    #[must_use]
    pub const fn index(self) -> u32 {
        match self {
            Self::Quality => 0,
            Self::Balanced => 1,
            Self::Speed => 2,
        }
    }

    /// Creates a mode from an index
    #[must_use]
    pub const fn from_index(index: u32) -> Self {
        match index {
            0 => Self::Quality,
            2 => Self::Speed,
            _ => Self::Balanced,
        }
    }

    /// Returns the recommended color depth for this mode
    #[must_use]
    pub const fn color_depth(self) -> u8 {
        match self {
            Self::Quality => 32,
            Self::Balanced => 24,
            Self::Speed => 16,
        }
    }
}

/// RDP client mode selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RdpClientMode {
    /// Use embedded RDP viewer (default) with dynamic resolution
    #[default]
    Embedded,
    /// Use external RDP client (xfreerdp)
    External,
}

impl RdpClientMode {
    /// Returns all available RDP client modes
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Embedded, Self::External]
    }

    /// Returns the display name for this mode
    #[must_use]
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::Embedded => "Embedded",
            Self::External => "External RDP client",
        }
    }

    /// Returns the index of this mode in the `all()` array
    #[must_use]
    pub const fn index(&self) -> u32 {
        match self {
            Self::Embedded => 0,
            Self::External => 1,
        }
    }

    /// Creates a mode from an index
    #[must_use]
    pub const fn from_index(index: u32) -> Self {
        match index {
            1 => Self::External,
            _ => Self::Embedded,
        }
    }
}

/// RDP protocol configuration
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdpConfig {
    /// RDP client mode (embedded or external)
    #[serde(default)]
    pub client_mode: RdpClientMode,
    /// Performance mode (quality/balanced/speed)
    #[serde(default)]
    pub performance_mode: RdpPerformanceMode,
    /// Screen resolution
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<Resolution>,
    /// Color depth (8, 15, 16, 24, or 32) - overrides performance_mode if set
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_depth: Option<u8>,
    /// Enable audio redirection
    #[serde(default)]
    pub audio_redirect: bool,
    /// RDP gateway configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway: Option<RdpGateway>,
    /// Shared folders for drive redirection
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared_folders: Vec<SharedFolder>,
    /// Custom command-line arguments
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_args: Vec<String>,
    /// Keyboard layout override (Windows KLID). None = auto-detect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard_layout: Option<u32>,
}

impl RdpConfig {
    /// Returns the effective color depth based on performance mode and explicit setting
    #[must_use]
    pub fn effective_color_depth(&self) -> u8 {
        self.color_depth
            .unwrap_or_else(|| self.performance_mode.color_depth())
    }
}

/// VNC performance mode for quality/speed tradeoff
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VncPerformanceMode {
    /// Best quality - Tight encoding, no compression, max quality
    Quality,
    /// Balanced - Tight encoding, moderate compression/quality
    #[default]
    Balanced,
    /// Best speed - ZRLE encoding, max compression, low quality
    Speed,
}

impl VncPerformanceMode {
    /// Returns all available performance modes
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Quality, Self::Balanced, Self::Speed]
    }

    /// Returns the display name for this mode
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Quality => "Quality",
            Self::Balanced => "Balanced",
            Self::Speed => "Speed",
        }
    }

    /// Returns the index of this mode in the `all()` array
    #[must_use]
    pub const fn index(self) -> u32 {
        match self {
            Self::Quality => 0,
            Self::Balanced => 1,
            Self::Speed => 2,
        }
    }

    /// Creates a mode from an index
    #[must_use]
    pub const fn from_index(index: u32) -> Self {
        match index {
            0 => Self::Quality,
            2 => Self::Speed,
            _ => Self::Balanced,
        }
    }

    /// Returns the recommended encoding for this mode
    #[must_use]
    pub const fn encoding(self) -> &'static str {
        match self {
            Self::Quality | Self::Balanced => "tight",
            Self::Speed => "zrle",
        }
    }

    /// Returns the recommended compression level (0-9) for this mode
    #[must_use]
    pub const fn compression(self) -> u8 {
        match self {
            Self::Quality => 0,
            Self::Balanced => 5,
            Self::Speed => 9,
        }
    }

    /// Returns the recommended quality level (0-9) for this mode
    #[must_use]
    pub const fn quality(self) -> u8 {
        match self {
            Self::Quality => 9,
            Self::Balanced => 5,
            Self::Speed => 1,
        }
    }
}

/// VNC client mode selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VncClientMode {
    /// Use embedded VNC viewer (default) with dynamic resolution
    #[default]
    Embedded,
    /// Use external VNC viewer application
    External,
}

impl VncClientMode {
    /// Returns all available VNC client modes
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Embedded, Self::External]
    }

    /// Returns the display name for this mode
    #[must_use]
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::Embedded => "Embedded",
            Self::External => "External VNC client",
        }
    }

    /// Returns the index of this mode in the `all()` array
    #[must_use]
    pub const fn index(&self) -> u32 {
        match self {
            Self::Embedded => 0,
            Self::External => 1,
        }
    }

    /// Creates a mode from an index
    #[must_use]
    pub const fn from_index(index: u32) -> Self {
        match index {
            1 => Self::External,
            _ => Self::Embedded,
        }
    }
}

/// VNC protocol configuration
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VncConfig {
    /// VNC client mode (embedded or external)
    #[serde(default)]
    pub client_mode: VncClientMode,
    /// Performance mode (quality/balanced/speed)
    #[serde(default)]
    pub performance_mode: VncPerformanceMode,
    /// Preferred encoding (e.g., "tight", "zrle", "hextile") - overrides performance_mode if set
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    /// Compression level (0-9) - overrides performance_mode if set
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compression: Option<u8>,
    /// Quality level (0-9) - overrides performance_mode if set
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<u8>,
    /// View-only mode (no input)
    #[serde(default)]
    pub view_only: bool,
    /// Scale display to fit window (for embedded mode)
    #[serde(default = "default_true")]
    pub scaling: bool,
    /// Enable clipboard sharing
    #[serde(default = "default_true")]
    pub clipboard_enabled: bool,
    /// Custom command-line arguments (for external client)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_args: Vec<String>,
}

impl VncConfig {
    /// Returns the effective encoding based on performance mode and explicit setting
    #[must_use]
    pub fn effective_encoding(&self) -> &str {
        self.encoding
            .as_deref()
            .unwrap_or_else(|| self.performance_mode.encoding())
    }

    /// Returns the effective compression level based on performance mode and explicit setting
    #[must_use]
    pub fn effective_compression(&self) -> u8 {
        self.compression
            .unwrap_or_else(|| self.performance_mode.compression())
    }

    /// Returns the effective quality level based on performance mode and explicit setting
    #[must_use]
    pub fn effective_quality(&self) -> u8 {
        self.quality
            .unwrap_or_else(|| self.performance_mode.quality())
    }
}

/// SPICE image compression mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpiceImageCompression {
    /// Automatic compression selection
    #[default]
    Auto,
    /// No compression
    Off,
    /// GLZ compression
    Glz,
    /// LZ compression
    Lz,
    /// QUIC compression
    Quic,
}

/// SPICE protocol configuration
// Allow 4 bools - these are distinct configuration options for SPICE protocol
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpiceConfig {
    /// Enable TLS encryption
    #[serde(default)]
    pub tls_enabled: bool,
    /// CA certificate path for TLS verification
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ca_cert_path: Option<PathBuf>,
    /// Skip certificate verification (insecure)
    #[serde(default)]
    pub skip_cert_verify: bool,
    /// Enable USB redirection
    #[serde(default)]
    pub usb_redirection: bool,
    /// Shared folders for folder sharing
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared_folders: Vec<SharedFolder>,
    /// Enable clipboard sharing
    #[serde(default = "default_true")]
    pub clipboard_enabled: bool,
    /// Preferred image compression mode
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_compression: Option<SpiceImageCompression>,
    /// SPICE proxy URL (e.g. `http://proxy:3128`) for Proxmox VE tunnelled connections
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
}

impl Default for SpiceConfig {
    fn default() -> Self {
        Self {
            tls_enabled: false,
            ca_cert_path: None,
            skip_cert_verify: false,
            usb_redirection: false,
            shared_folders: Vec::new(),
            clipboard_enabled: true, // Clipboard enabled by default
            image_compression: None,
            proxy: None,
        }
    }
}

/// Zero Trust provider type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ZeroTrustProvider {
    /// AWS Systems Manager Session Manager
    #[default]
    AwsSsm,
    /// Google Cloud Identity-Aware Proxy (IAP)
    GcpIap,
    /// Azure Bastion with AAD authentication
    AzureBastion,
    /// Azure SSH with AAD authentication
    AzureSsh,
    /// Oracle Cloud Infrastructure Bastion
    OciBastion,
    /// Cloudflare Access
    CloudflareAccess,
    /// Teleport
    Teleport,
    /// Tailscale SSH
    TailscaleSsh,
    /// `HashiCorp` Boundary
    Boundary,
    /// Generic custom command
    Generic,
}

impl ZeroTrustProvider {
    /// Returns the display name for this provider
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::AwsSsm => "AWS Session Manager",
            Self::GcpIap => "GCP IAP Tunnel",
            Self::AzureBastion => "Azure Bastion",
            Self::AzureSsh => "Azure SSH (AAD)",
            Self::OciBastion => "OCI Bastion",
            Self::CloudflareAccess => "Cloudflare Access",
            Self::Teleport => "Teleport",
            Self::TailscaleSsh => "Tailscale SSH",
            Self::Boundary => "HashiCorp Boundary",
            Self::Generic => "Generic Command",
        }
    }

    /// Returns the GTK symbolic icon name for this provider
    ///
    /// Uses standard Adwaita icons that are guaranteed to exist in all GTK themes.
    /// Each provider has a unique icon - no duplicates with SSH or other protocols.
    ///
    /// Icons must match sidebar.rs `get_protocol_icon()` for consistency.
    #[must_use]
    pub const fn icon_name(self) -> &'static str {
        match self {
            Self::AwsSsm => "network-workgroup-symbolic", // AWS - workgroup
            Self::GcpIap => "weather-overcast-symbolic",  // GCP - cloud
            Self::AzureBastion => "weather-few-clouds-symbolic", // Azure - clouds
            Self::AzureSsh => "weather-showers-symbolic", // Azure SSH - showers
            Self::OciBastion => "drive-harddisk-symbolic", // OCI - harddisk
            Self::CloudflareAccess => "security-high-symbolic", // Cloudflare - security
            Self::Teleport => "emblem-system-symbolic",   // Teleport - system/gear
            Self::TailscaleSsh => "network-vpn-symbolic", // Tailscale - VPN
            Self::Boundary => "dialog-password-symbolic", // Boundary - password/lock
            Self::Generic => "system-run-symbolic",       // Generic - run command
        }
    }

    /// Returns the CLI command name for this provider
    #[must_use]
    pub const fn cli_command(self) -> &'static str {
        match self {
            Self::AwsSsm => "aws",
            Self::GcpIap => "gcloud",
            Self::AzureBastion | Self::AzureSsh => "az",
            Self::OciBastion => "oci",
            Self::CloudflareAccess => "cloudflared",
            Self::Teleport => "tsh",
            Self::TailscaleSsh => "tailscale",
            Self::Boundary => "boundary",
            Self::Generic => "",
        }
    }

    /// Returns all available providers
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::AwsSsm,
            Self::GcpIap,
            Self::AzureBastion,
            Self::AzureSsh,
            Self::OciBastion,
            Self::CloudflareAccess,
            Self::Teleport,
            Self::TailscaleSsh,
            Self::Boundary,
            Self::Generic,
        ]
    }
}

impl std::fmt::Display for ZeroTrustProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Zero Trust connection configuration
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZeroTrustConfig {
    /// Zero Trust provider
    pub provider: ZeroTrustProvider,
    /// Provider-specific configuration
    #[serde(flatten)]
    pub provider_config: ZeroTrustProviderConfig,
    /// Custom command-line arguments (appended to generated command)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_args: Vec<String>,
    /// Cached detected provider for consistent icon display
    /// This is auto-detected from the command and persisted for consistent display
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detected_provider: Option<String>,
}

impl Default for ZeroTrustConfig {
    fn default() -> Self {
        Self {
            provider: ZeroTrustProvider::default(),
            provider_config: ZeroTrustProviderConfig::AwsSsm(AwsSsmConfig::default()),
            custom_args: Vec::new(),
            detected_provider: None,
        }
    }
}

impl ZeroTrustConfig {
    /// Builds the command and arguments for this Zero Trust connection
    ///
    /// Returns a tuple of (program, arguments) that can be used to spawn the process.
    /// The `username` parameter is used for providers that support it.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn build_command(&self, username: Option<&str>) -> (String, Vec<String>) {
        let mut args = match &self.provider_config {
            ZeroTrustProviderConfig::AwsSsm(cfg) => {
                let mut a = vec![
                    "ssm".to_string(),
                    "start-session".to_string(),
                    "--target".to_string(),
                    cfg.target.clone(),
                ];
                if cfg.profile != "default" {
                    a.push("--profile".to_string());
                    a.push(cfg.profile.clone());
                }
                if let Some(ref region) = cfg.region {
                    a.push("--region".to_string());
                    a.push(region.clone());
                }
                ("aws".to_string(), a)
            }
            ZeroTrustProviderConfig::GcpIap(cfg) => {
                let mut a = vec![
                    "compute".to_string(),
                    "ssh".to_string(),
                    cfg.instance.clone(),
                    "--zone".to_string(),
                    cfg.zone.clone(),
                    "--tunnel-through-iap".to_string(),
                ];
                if let Some(ref project) = cfg.project {
                    a.push("--project".to_string());
                    a.push(project.clone());
                }
                ("gcloud".to_string(), a)
            }
            ZeroTrustProviderConfig::AzureBastion(cfg) => {
                let a = vec![
                    "network".to_string(),
                    "bastion".to_string(),
                    "ssh".to_string(),
                    "--name".to_string(),
                    cfg.bastion_name.clone(),
                    "--resource-group".to_string(),
                    cfg.resource_group.clone(),
                    "--target-resource-id".to_string(),
                    cfg.target_resource_id.clone(),
                    "--auth-type".to_string(),
                    "AAD".to_string(),
                ];
                ("az".to_string(), a)
            }
            ZeroTrustProviderConfig::AzureSsh(cfg) => {
                let a = vec![
                    "ssh".to_string(),
                    "vm".to_string(),
                    "--name".to_string(),
                    cfg.vm_name.clone(),
                    "--resource-group".to_string(),
                    cfg.resource_group.clone(),
                ];
                ("az".to_string(), a)
            }
            ZeroTrustProviderConfig::OciBastion(cfg) => {
                let a = vec![
                    "bastion".to_string(),
                    "session".to_string(),
                    "create-managed-ssh".to_string(),
                    "--bastion-id".to_string(),
                    cfg.bastion_id.clone(),
                    "--target-resource-id".to_string(),
                    cfg.target_resource_id.clone(),
                    "--target-private-ip".to_string(),
                    cfg.target_private_ip.clone(),
                    "--session-ttl".to_string(),
                    cfg.session_ttl.to_string(),
                ];
                ("oci".to_string(), a)
            }
            ZeroTrustProviderConfig::CloudflareAccess(cfg) => {
                let mut a = vec![
                    "access".to_string(),
                    "ssh".to_string(),
                    "--hostname".to_string(),
                    cfg.hostname.clone(),
                ];
                let user = cfg.username.as_deref().or(username);
                if let Some(u) = user {
                    a.push("--user".to_string());
                    a.push(u.to_string());
                }
                ("cloudflared".to_string(), a)
            }
            ZeroTrustProviderConfig::Teleport(cfg) => {
                let mut a = vec!["ssh".to_string()];
                if let Some(ref cluster) = cfg.cluster {
                    a.push("--cluster".to_string());
                    a.push(cluster.clone());
                }
                let user = cfg.username.as_deref().or(username);
                let target = user.map_or_else(|| cfg.host.clone(), |u| format!("{u}@{}", cfg.host));
                a.push(target);
                ("tsh".to_string(), a)
            }
            ZeroTrustProviderConfig::TailscaleSsh(cfg) => {
                let user = cfg.username.as_deref().or(username);
                let target = user.map_or_else(|| cfg.host.clone(), |u| format!("{u}@{}", cfg.host));
                let a = vec!["ssh".to_string(), target];
                ("tailscale".to_string(), a)
            }
            ZeroTrustProviderConfig::Boundary(cfg) => {
                let mut a = vec![
                    "connect".to_string(),
                    "ssh".to_string(),
                    "-target-id".to_string(),
                    cfg.target.clone(),
                ];
                if let Some(ref addr) = cfg.addr {
                    a.push("-addr".to_string());
                    a.push(addr.clone());
                }
                ("boundary".to_string(), a)
            }
            ZeroTrustProviderConfig::Generic(cfg) => {
                // Parse the command template
                let cmd = cfg.command_template.clone();
                // Simple shell execution
                let a = vec!["-c".to_string(), cmd];
                ("sh".to_string(), a)
            }
        };

        // Append custom args
        args.1.extend(self.custom_args.clone());

        args
    }
}

/// Provider-specific Zero Trust configuration
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "provider_type", rename_all = "snake_case")]
pub enum ZeroTrustProviderConfig {
    /// AWS SSM configuration
    AwsSsm(AwsSsmConfig),
    /// GCP IAP configuration
    GcpIap(GcpIapConfig),
    /// Azure Bastion configuration
    AzureBastion(AzureBastionConfig),
    /// Azure SSH configuration
    AzureSsh(AzureSshConfig),
    /// OCI Bastion configuration
    OciBastion(OciBastionConfig),
    /// Cloudflare Access configuration
    CloudflareAccess(CloudflareAccessConfig),
    /// Teleport configuration
    Teleport(TeleportConfig),
    /// Tailscale SSH configuration
    TailscaleSsh(TailscaleSshConfig),
    /// `HashiCorp` Boundary configuration
    Boundary(BoundaryConfig),
    /// Generic custom command configuration
    Generic(GenericZeroTrustConfig),
}

/// AWS Systems Manager Session Manager configuration
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AwsSsmConfig {
    /// EC2 instance ID (e.g., i-0123456789abcdef0)
    pub target: String,
    /// AWS profile name (default: "default")
    #[serde(default = "default_aws_profile")]
    pub profile: String,
    /// AWS region (optional, uses profile default if not set)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}

fn default_aws_profile() -> String {
    "default".to_string()
}

/// GCP Identity-Aware Proxy configuration
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcpIapConfig {
    /// Instance name
    pub instance: String,
    /// GCP zone (e.g., us-central1-a)
    pub zone: String,
    /// GCP project (optional, uses gcloud default if not set)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

/// Azure Bastion configuration
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AzureBastionConfig {
    /// Target resource ID
    pub target_resource_id: String,
    /// Resource group name
    pub resource_group: String,
    /// Bastion host name
    pub bastion_name: String,
}

/// Azure SSH (AAD) configuration
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AzureSshConfig {
    /// VM name
    pub vm_name: String,
    /// Resource group name
    pub resource_group: String,
}

/// OCI Bastion configuration
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OciBastionConfig {
    /// Bastion OCID
    pub bastion_id: String,
    /// Target resource OCID
    pub target_resource_id: String,
    /// Target private IP
    pub target_private_ip: String,
    /// SSH public key file path
    #[serde(default = "default_ssh_pub_key")]
    pub ssh_public_key_file: PathBuf,
    /// Session TTL in seconds (default: 1800)
    #[serde(default = "default_session_ttl")]
    pub session_ttl: u32,
}

fn default_ssh_pub_key() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".ssh/id_rsa.pub")
}

const fn default_session_ttl() -> u32 {
    1800
}

/// Cloudflare Access configuration
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudflareAccessConfig {
    /// Target hostname
    pub hostname: String,
    /// SSH username
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

/// Teleport configuration
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeleportConfig {
    /// Target host
    pub host: String,
    /// SSH username
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Teleport cluster (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster: Option<String>,
}

/// Tailscale SSH configuration
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TailscaleSshConfig {
    /// Target host (Tailscale hostname or IP)
    pub host: String,
    /// SSH username
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

/// `HashiCorp` Boundary configuration
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundaryConfig {
    /// Target ID or name
    pub target: String,
    /// Boundary address (optional, uses `BOUNDARY_ADDR` env if not set)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addr: Option<String>,
}

/// Generic Zero Trust command configuration
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenericZeroTrustConfig {
    /// Full command template with placeholders
    /// Supported placeholders: {host}, {user}, {port}
    pub command_template: String,
}

/// Default shell for Kubernetes connections
fn default_shell() -> String {
    "/bin/sh".to_string()
}

/// Default busybox image for temporary pods
fn default_busybox_image() -> String {
    "busybox:latest".to_string()
}

/// Kubernetes pod shell configuration (kubectl exec)
///
/// Each connection stores its own kubeconfig, context, namespace,
/// pod, container, shell, and busybox settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KubernetesConfig {
    /// Path to kubeconfig file (uses default if None)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kubeconfig: Option<PathBuf>,
    /// Kubernetes context to use (uses current-context if None)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Namespace (uses default namespace if None)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Pod name to exec into
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pod: Option<String>,
    /// Container name within the pod (optional for single-container pods)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    /// Shell to use inside the container
    #[serde(default = "default_shell")]
    pub shell: String,
    /// Whether to use a temporary busybox pod instead of exec
    #[serde(default)]
    pub use_busybox: bool,
    /// Busybox image to use for temporary pods
    #[serde(default = "default_busybox_image")]
    pub busybox_image: String,
    /// Additional kubectl arguments
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_args: Vec<String>,
}

impl Default for KubernetesConfig {
    fn default() -> Self {
        Self {
            kubeconfig: None,
            context: None,
            namespace: None,
            pod: None,
            container: None,
            shell: default_shell(),
            use_busybox: false,
            busybox_image: default_busybox_image(),
            custom_args: Vec::new(),
        }
    }
}

#[cfg(test)]
mod zerotrust_tests {
    use super::*;

    #[test]
    fn test_aws_ssm_build_command() {
        let config = ZeroTrustConfig {
            provider: ZeroTrustProvider::AwsSsm,
            provider_config: ZeroTrustProviderConfig::AwsSsm(AwsSsmConfig {
                target: "i-0123456789abcdef0".to_string(),
                profile: "production".to_string(),
                region: Some("us-west-2".to_string()),
            }),
            custom_args: vec![],
            detected_provider: None,
        };

        let (program, args) = config.build_command(None);
        assert_eq!(program, "aws");
        assert!(args.contains(&"ssm".to_string()));
        assert!(args.contains(&"start-session".to_string()));
        assert!(args.contains(&"--target".to_string()));
        assert!(args.contains(&"i-0123456789abcdef0".to_string()));
        assert!(args.contains(&"--profile".to_string()));
        assert!(args.contains(&"production".to_string()));
        assert!(args.contains(&"--region".to_string()));
        assert!(args.contains(&"us-west-2".to_string()));
    }

    #[test]
    fn test_gcp_iap_build_command() {
        let config = ZeroTrustConfig {
            provider: ZeroTrustProvider::GcpIap,
            provider_config: ZeroTrustProviderConfig::GcpIap(GcpIapConfig {
                instance: "my-instance".to_string(),
                zone: "us-central1-a".to_string(),
                project: Some("my-project".to_string()),
            }),
            custom_args: vec![],
            detected_provider: None,
        };

        let (program, args) = config.build_command(None);
        assert_eq!(program, "gcloud");
        assert!(args.contains(&"compute".to_string()));
        assert!(args.contains(&"ssh".to_string()));
        assert!(args.contains(&"my-instance".to_string()));
        assert!(args.contains(&"--zone".to_string()));
        assert!(args.contains(&"us-central1-a".to_string()));
        assert!(args.contains(&"--project".to_string()));
        assert!(args.contains(&"my-project".to_string()));
    }

    #[test]
    fn test_teleport_build_command_with_username() {
        let config = ZeroTrustConfig {
            provider: ZeroTrustProvider::Teleport,
            provider_config: ZeroTrustProviderConfig::Teleport(TeleportConfig {
                host: "server.example.com".to_string(),
                username: None,
                cluster: Some("production".to_string()),
            }),
            custom_args: vec![],
            detected_provider: None,
        };

        let (program, args) = config.build_command(Some("admin"));
        assert_eq!(program, "tsh");
        assert!(args.contains(&"ssh".to_string()));
        assert!(args.contains(&"--cluster".to_string()));
        assert!(args.contains(&"production".to_string()));
        assert!(args.contains(&"admin@server.example.com".to_string()));
    }

    #[test]
    fn test_tailscale_build_command() {
        let config = ZeroTrustConfig {
            provider: ZeroTrustProvider::TailscaleSsh,
            provider_config: ZeroTrustProviderConfig::TailscaleSsh(TailscaleSshConfig {
                host: "my-server".to_string(),
                username: Some("root".to_string()),
            }),
            custom_args: vec![],
            detected_provider: None,
        };

        let (program, args) = config.build_command(None);
        assert_eq!(program, "tailscale");
        assert!(args.contains(&"ssh".to_string()));
        assert!(args.contains(&"root@my-server".to_string()));
    }

    #[test]
    fn test_generic_build_command() {
        let config = ZeroTrustConfig {
            provider: ZeroTrustProvider::Generic,
            provider_config: ZeroTrustProviderConfig::Generic(GenericZeroTrustConfig {
                command_template: "ssh -o ProxyCommand='nc -x proxy:1080 %h %p' user@host"
                    .to_string(),
            }),
            custom_args: vec![],
            detected_provider: None,
        };

        let (program, args) = config.build_command(None);
        assert_eq!(program, "sh");
        assert_eq!(args[0], "-c");
        assert!(args[1].contains("ProxyCommand"));
    }

    #[test]
    fn test_custom_args_appended() {
        let config = ZeroTrustConfig {
            provider: ZeroTrustProvider::AwsSsm,
            provider_config: ZeroTrustProviderConfig::AwsSsm(AwsSsmConfig {
                target: "i-123".to_string(),
                profile: "default".to_string(),
                region: None,
            }),
            custom_args: vec!["--debug".to_string(), "--verbose".to_string()],
            detected_provider: None,
        };

        let (_, args) = config.build_command(None);
        assert!(args.contains(&"--debug".to_string()));
        assert!(args.contains(&"--verbose".to_string()));
    }

    #[test]
    fn test_zerotrust_config_serialization() {
        let config = ZeroTrustConfig {
            provider: ZeroTrustProvider::AwsSsm,
            provider_config: ZeroTrustProviderConfig::AwsSsm(AwsSsmConfig {
                target: "i-123".to_string(),
                profile: "default".to_string(),
                region: None,
            }),
            custom_args: vec![],
            detected_provider: Some("aws".to_string()),
        };

        let json = serde_json::to_string(&config).unwrap();
        let parsed: ZeroTrustConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, parsed);
    }

    #[test]
    fn test_zerotrust_provider_display() {
        assert_eq!(
            ZeroTrustProvider::AwsSsm.display_name(),
            "AWS Session Manager"
        );
        assert_eq!(ZeroTrustProvider::GcpIap.display_name(), "GCP IAP Tunnel");
        assert_eq!(ZeroTrustProvider::Teleport.display_name(), "Teleport");
        assert_eq!(ZeroTrustProvider::Generic.display_name(), "Generic Command");
    }

    #[test]
    fn test_zerotrust_provider_cli_command() {
        assert_eq!(ZeroTrustProvider::AwsSsm.cli_command(), "aws");
        assert_eq!(ZeroTrustProvider::GcpIap.cli_command(), "gcloud");
        assert_eq!(ZeroTrustProvider::Teleport.cli_command(), "tsh");
        assert_eq!(ZeroTrustProvider::Generic.cli_command(), "");
    }
}
