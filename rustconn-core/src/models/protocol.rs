//! Protocol configuration types for SSH, RDP, and VNC connections.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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
    /// MOSH protocol (mobile shell)
    Mosh,
    /// Web bookmark (opens URL in default browser)
    Web,
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
            Self::Mosh => "mosh",
            Self::Web => "web",
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
            Self::Mosh => 22,
            Self::Web => 443,
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
            Self::Mosh => write!(f, "MOSH"),
            Self::Web => write!(f, "Web"),
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
    /// MOSH protocol configuration
    Mosh(MoshConfig),
    /// Web bookmark configuration (opens URL in default browser)
    Web(WebConfig),
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
            Self::Mosh(_) => ProtocolType::Mosh,
            Self::Web(_) => ProtocolType::Web,
        }
    }

    /// Returns what Backspace and Delete send for this protocol.
    ///
    /// The one place the per-connection erase choice is derived from a stored
    /// configuration, so the terminal notebook, the SSH launcher and the MOSH
    /// launcher cannot drift apart on it (issue
    /// [#271](https://github.com/totoshko88/RustConn/issues/271)).
    ///
    /// Protocols with no VTE session of their own return the defaults rather
    /// than nothing, so a caller re-applying the modes always has a value to
    /// install. SFTP is deliberately among them: it shares [`SshConfig`] and so
    /// carries the two fields, but its session is a file-manager tab that never
    /// applies them, which is why the connection editor hides the choice when
    /// SFTP is selected.
    #[must_use]
    pub const fn erase_modes(&self) -> (BackspaceSends, DeleteSends) {
        match self {
            Self::Ssh(cfg) => (cfg.backspace_sends, cfg.delete_sends),
            Self::Telnet(cfg) => (cfg.backspace_sends, cfg.delete_sends),
            Self::Mosh(cfg) => (cfg.backspace_sends, cfg.delete_sends),
            _ => (BackspaceSends::Automatic, DeleteSends::Automatic),
        }
    }
}

/// What the Backspace key sends in a terminal session
///
/// Not protocol-specific: the remote side decides which byte erases the
/// character to the left, and it disagrees with the local default on both
/// Telnet hosts and SSH ones — network appliances and older Unix systems
/// commonly expect `^H` where a Linux host expects `^?`
/// (issue [#271](https://github.com/totoshko88/RustConn/issues/271)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BackspaceSends {
    /// Automatic (use terminal default)
    #[default]
    Automatic,
    /// Send Backspace (^H, 0x08)
    Backspace,
    /// Send Delete (^?, 0x7F)
    Delete,
}

impl BackspaceSends {
    /// Returns all available options
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Automatic, Self::Backspace, Self::Delete]
    }

    /// Returns the display name for this option
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic (^?)",
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

/// What the Delete key sends in a terminal session
///
/// Shares [`BackspaceSends`]' reason for existing, but stays a separate type so
/// the two keys cannot be configured with each other's value by accident:
/// `Automatic` means the VT220 sequence here and DEL there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DeleteSends {
    /// Automatic (use terminal default)
    #[default]
    Automatic,
    /// Send Backspace (^H, 0x08)
    Backspace,
    /// Send Delete (^?, 0x7F)
    Delete,
}

impl DeleteSends {
    /// Returns all available options
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Automatic, Self::Backspace, Self::Delete]
    }

    /// Returns the display name for this option
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic (\\e[3~)",
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
    pub backspace_sends: BackspaceSends,
    /// What the Delete key sends
    #[serde(default)]
    pub delete_sends: DeleteSends,
}

/// MOSH prediction mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoshPredictMode {
    /// Adaptive prediction (default)
    #[default]
    Adaptive,
    /// Always predict
    Always,
    /// Never predict
    Never,
}

/// MOSH protocol configuration
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoshConfig {
    /// SSH port for the initial handshake
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_port: Option<u16>,
    /// UDP port range for MOSH (e.g., "60000:60010")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port_range: Option<String>,
    /// Path to the mosh-server binary on the remote host
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_binary: Option<String>,
    /// Prediction mode
    #[serde(default)]
    pub predict_mode: MoshPredictMode,
    /// Custom command-line arguments for the mosh client
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_args: Vec<String>,
    /// What the Backspace key sends to this host.
    ///
    /// MOSH runs in the same VTE widget as SSH and Telnet, so the hosts that
    /// expect `^H` (`0x08`) where the default sends DEL (`0x7f`) expect it over
    /// MOSH too, and cannot be reconfigured from their end (issue
    /// [#271](https://github.com/totoshko88/RustConn/issues/271)).
    ///
    /// `Automatic` is the pre-existing behaviour, so stored connections keep
    /// working unchanged.
    #[serde(default)]
    pub backspace_sends: BackspaceSends,
    /// What the Delete key sends to this host.
    ///
    /// Counterpart to [`MoshConfig::backspace_sends`]: `Automatic` sends the
    /// VT220 sequence `\e[3~`, while hosts that treat Delete as an erase key
    /// need `^H` or `^?` named explicitly.
    #[serde(default)]
    pub delete_sends: DeleteSends,
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

/// Direction of an SSH port forward
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PortForwardDirection {
    /// Local port forwarding (`-L`): binds a local port and forwards traffic
    /// through the SSH tunnel to a remote destination
    #[default]
    Local,
    /// Remote port forwarding (`-R`): binds a port on the remote host and
    /// forwards traffic back through the tunnel to a local destination
    Remote,
    /// Dynamic port forwarding (`-D`): opens a local SOCKS proxy that routes
    /// traffic through the SSH tunnel
    Dynamic,
}

impl std::fmt::Display for PortForwardDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local => write!(f, "Local (-L)"),
            Self::Remote => write!(f, "Remote (-R)"),
            Self::Dynamic => write!(f, "Dynamic (-D)"),
        }
    }
}

/// A single SSH port forwarding rule
///
/// Supports local (`-L`), remote (`-R`), and dynamic (`-D`) forwarding.
/// For dynamic forwarding only `local_port` is used (SOCKS proxy).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortForward {
    /// Forwarding direction
    #[serde(default)]
    pub direction: PortForwardDirection,
    /// Local port to bind
    pub local_port: u16,
    /// Remote host to forward to (unused for dynamic)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub remote_host: String,
    /// Remote port to forward to (unused for dynamic)
    #[serde(default)]
    pub remote_port: u16,
}

impl PortForward {
    /// Builds the SSH command-line argument for this port forward rule
    #[must_use]
    pub fn to_ssh_arg(&self) -> Vec<String> {
        match self.direction {
            PortForwardDirection::Local => {
                vec![
                    "-L".to_string(),
                    format!(
                        "{}:{}:{}",
                        self.local_port, self.remote_host, self.remote_port
                    ),
                ]
            }
            PortForwardDirection::Remote => {
                vec![
                    "-R".to_string(),
                    format!(
                        "{}:{}:{}",
                        self.local_port, self.remote_host, self.remote_port
                    ),
                ]
            }
            PortForwardDirection::Dynamic => {
                vec!["-D".to_string(), self.local_port.to_string()]
            }
        }
    }

    /// Returns a human-readable summary of this forwarding rule
    #[must_use]
    pub fn display_summary(&self) -> String {
        match self.direction {
            PortForwardDirection::Local => {
                format!(
                    "L {} → {}:{}",
                    self.local_port, self.remote_host, self.remote_port
                )
            }
            PortForwardDirection::Remote => {
                format!(
                    "R {} → {}:{}",
                    self.local_port, self.remote_host, self.remote_port
                )
            }
            PortForwardDirection::Dynamic if self.local_port == 0 => "D auto (SOCKS)".to_string(),
            PortForwardDirection::Dynamic => {
                format!("D {} (SOCKS)", self.local_port)
            }
        }
    }

    /// Returns `true` for a dynamic SOCKS forward whose local port is to be
    /// chosen automatically at connection time.
    ///
    /// A `Dynamic` forward with `local_port == 0` is the sentinel for "pick a
    /// free port": zero is never a bindable listening port, so it cannot
    /// collide with a real request. The chosen port is exposed to the session
    /// as `${SOCKS5_PORT}`.
    #[must_use]
    pub const fn is_random_dynamic(&self) -> bool {
        matches!(self.direction, PortForwardDirection::Dynamic) && self.local_port == 0
    }
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
#[expect(
    clippy::struct_excessive_bools,
    reason = "settings/flags struct mirrors persisted config 1:1; bools represent independent toggles, not a state machine"
)]
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
    /// Port forwarding rules (local, remote, dynamic)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub port_forwards: Vec<PortForward>,
    /// Enable Wayland application forwarding via `waypipe`
    /// Wraps the SSH command with `waypipe ssh` for Wayland display forwarding
    #[serde(default)]
    pub waypipe: bool,
    /// Custom SSH agent socket path override for this connection.
    /// When set, overrides both the global setting and auto-detected socket.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_agent_socket: Option<String>,
    /// Custom `ProxyCommand` for connections that require a proxy (e.g., Tor `.onion` hosts).
    /// When set, SSH uses this command instead of a direct TCP connection.
    /// Example: `ncat --proxy 127.0.0.1:9050 --proxy-type socks5 %h %p`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_command: Option<String>,
    /// Path to a PKCS#11 provider library for hardware-token authentication
    /// (YubiKey/PIV/smart cards). Maps to `-o PKCS11Provider=<path>`.
    /// Example: `/usr/lib64/libykcs11.so.2`.
    ///
    /// Works alongside any auth method — the provider offers the token's keys.
    /// Empty/`None` means no token. The literal `none` disables an inherited
    /// provider (OpenSSH 8.1+).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pkcs11_provider: Option<String>,
    /// SSH keep-alive interval in seconds (`ServerAliveInterval`).
    /// Sends a keep-alive packet every N seconds to prevent idle disconnects.
    /// `None` means no keep-alive (SSH default behavior).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_alive_interval: Option<u32>,
    /// Maximum number of keep-alive messages without a response (`ServerAliveCountMax`).
    /// Connection is terminated after this many unanswered keep-alive packets.
    /// `None` uses SSH default (3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_alive_count_max: Option<u32>,
    /// Enable verbose/debug output for SSH connections (`-v` flag).
    /// Useful for diagnosing connection issues (e.g., reset by remote device).
    #[serde(default)]
    pub verbose: bool,
    /// Enable Multipath TCP via `mptcpize run` wrapper.
    /// Uses multiple network paths for seamless mobility and bandwidth aggregation.
    /// Requires `mptcpize` (mptcpd) and kernel MPTCP support (Linux 5.6+).
    /// Falls back to regular TCP transparently when unavailable.
    #[serde(default)]
    pub mptcp: bool,
    /// Initial remote directory for the SFTP file-browser URI.
    ///
    /// The GVFS sftp backend mounts at the server root `/`, so a bare
    /// `sftp://host` URI opens `/` — inaccessible on shared hosting (issue
    /// #212). When set, this absolute path is appended to the URI so the file
    /// manager opens where the user has access. When empty, the login home
    /// directory is resolved automatically (best-effort `ssh … pwd`), falling
    /// back to the server root. Only affects the file-manager path; `ssh`, the
    /// `sftp` CLI and `mc` already start in `$HOME`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_path: Option<String>,
    /// What the Backspace key sends to this host.
    ///
    /// Backspace normally sends DEL (`0x7f`), which is what a Linux host's
    /// `stty erase` agrees with. Network appliances and older Unix systems
    /// often expect `^H` (`0x08`) instead and echo `^?` or beep rather than
    /// erasing, with no way to fix it from the remote end (issue
    /// [#271](https://github.com/totoshko88/RustConn/issues/271)). Telnet
    /// sessions have had this switch since the same problem appeared there.
    ///
    /// `Automatic` is the pre-existing behaviour, so stored connections keep
    /// working unchanged.
    #[serde(default)]
    pub backspace_sends: BackspaceSends,
    /// What the Delete key sends to this host.
    ///
    /// Counterpart to [`SshConfig::backspace_sends`]: `Automatic` sends the
    /// VT220 sequence `\e[3~`, while hosts that treat Delete as an erase key
    /// need `^H` or `^?` named explicitly.
    #[serde(default)]
    pub delete_sends: DeleteSends,
}

fn default_true() -> bool {
    true
}

/// Default zoom level: 100%.
fn default_zoom() -> f64 {
    1.0
}

const fn default_jiggler_interval() -> u32 {
    60
}

const fn default_autotype_delay() -> u32 {
    20
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

        // Add verbose flag for debugging connection issues
        if self.verbose {
            args.push("-v".to_string());
        }

        // Determine if we should add IdentitiesOnly based on key source
        // File auth method should always use IdentitiesOnly to prevent "Too many auth failures"
        // Agent auth method with a valid key path should also use IdentitiesOnly
        let should_use_identities_only =
            self.identities_only || matches!(self.key_source, SshKeySource::File { .. });

        // Add identity file if specified via key_source (preferred) or key_path (legacy)
        match &self.key_source {
            SshKeySource::File { path } if !path.as_os_str().is_empty() => {
                args.push("-i".to_string());
                args.push(path.display().to_string());
            }
            SshKeySource::Agent { .. } => {
                // When using SSH agent authentication, do NOT pass -i <path> even if the
                // comment contains a file path. Passing -i causes SSH to first attempt
                // file-based auth (triggering an agent confirmation in Bitwarden/KeeAgent),
                // then fall back to agent auth (triggering a second confirmation).
                // Without -i, the agent offers keys naturally with a single prompt.
                // See: https://github.com/totoshko88/RustConn/issues/125
            }
            SshKeySource::Default | SshKeySource::File { .. } | SshKeySource::Inherit => {
                // Default or File with empty path - check legacy key_path field
                if let Some(ref key_path) = self.key_path
                    && !key_path.as_os_str().is_empty()
                {
                    args.push("-i".to_string());
                    args.push(key_path.display().to_string());
                }
            }
        }

        // Add IdentitiesOnly option if needed (after -i flag for proper ordering)
        // This prevents SSH from trying other keys when a specific key file is selected
        if should_use_identities_only {
            args.push("-o".to_string());
            args.push("IdentitiesOnly=yes".to_string());
        }

        // Add proxy jump if specified (skip when ProxyCommand is set — it takes precedence)
        if self.proxy_command.is_none()
            && let Some(ref proxy) = self.proxy_jump
        {
            args.push("-J".to_string());
            args.push(proxy.clone());
        }

        // Add ProxyCommand if specified (e.g., for Tor .onion hosts)
        if let Some(ref proxy_cmd) = self.proxy_command {
            args.push("-o".to_string());
            args.push(format!("ProxyCommand={proxy_cmd}"));
        }

        // Add PKCS#11 provider for hardware-token auth (YubiKey/PIV/smart cards).
        // The provider offers the token's keys independently of auth_method; no
        // -i and no IdentitiesOnly are forced (PKCS11Provider is its own
        // configured identity source, so its keys are offered regardless).
        if let Some(provider) = self.pkcs11_provider.as_deref()
            && !provider.trim().is_empty()
        {
            args.push("-o".to_string());
            args.push(format!("PKCS11Provider={}", provider.trim()));
        }

        // Add control master options if enabled
        if self.use_control_master {
            args.push("-o".to_string());
            args.push("ControlMaster=auto".to_string());
            args.push("-o".to_string());
            // ponytail: 60s persist — short enough to recover after network
            // changes (#217), long enough for monitoring multiplex.
            args.push("ControlPersist=60".to_string());
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

        // MPTCP (Multipath TCP) for SSH is handled at the command-launch level
        // by prefixing with `mptcpize run` — not via an SSH option.
        // See: protocol/ssh.rs build_command(), terminal/mod.rs spawn_ssh(),
        // and tunnel_manager.rs start_tunnel().

        // Add keep-alive options if configured
        // ServerAliveInterval sends a keep-alive packet every N seconds
        // ServerAliveCountMax terminates after N unanswered packets
        if let Some(interval) = self.keep_alive_interval {
            // Only add if user hasn't already set it via custom_options
            if !self
                .custom_options
                .keys()
                .any(|k| k.eq_ignore_ascii_case("ServerAliveInterval"))
            {
                args.push("-o".to_string());
                args.push(format!("ServerAliveInterval={interval}"));
            }
        }
        if let Some(count) = self.keep_alive_count_max
            && !self
                .custom_options
                .keys()
                .any(|k| k.eq_ignore_ascii_case("ServerAliveCountMax"))
        {
            args.push("-o".to_string());
            args.push(format!("ServerAliveCountMax={count}"));
        }

        // Add custom options (filter out dangerous directives)
        for (key, value) in &self.custom_options {
            // Block directives that could execute arbitrary commands
            let key_lower = key.to_lowercase();
            if matches!(
                key_lower.as_str(),
                "proxycommand" | "localcommand" | "permitlocalcommand" | "remotecommand" | "match"
            ) {
                tracing::warn!(
                    option = %key,
                    "Skipping dangerous SSH custom option"
                );
                continue;
            }
            args.push("-o".to_string());
            args.push(format!("{key}={value}"));
        }

        // Add port forwarding rules
        for pf in &self.port_forwards {
            args.extend(pf.to_ssh_arg());
        }

        args
    }

    /// Returns `true` if any port forward is a random dynamic SOCKS forward
    /// (a `Dynamic` rule with `local_port == 0`).
    #[must_use]
    pub fn has_random_socks_forward(&self) -> bool {
        self.port_forwards
            .iter()
            .any(PortForward::is_random_dynamic)
    }

    /// Assigns a concrete local port to every random dynamic SOCKS forward,
    /// returning the first port assigned (the value for `${SOCKS5_PORT}`).
    ///
    /// A `Dynamic` forward stored with `local_port == 0` means "pick a free
    /// port at connection time" (see [`PortForward::is_random_dynamic`]). The
    /// port itself is chosen by `pick`, which the caller supplies — typically
    /// [`crate::ssh_tunnel::find_free_port`] — so this stays deterministic and
    /// testable without opening a socket. Forwards with an explicit port and
    /// non-dynamic forwards are left untouched.
    ///
    /// Returns `None` when there is no random dynamic forward, or when `pick`
    /// yields no port (so the caller can fall back or report the failure).
    ///
    /// # Errors
    ///
    /// Propagates whatever error type `pick` returns on the first failure; the
    /// config is then left with any ports already assigned before the failure.
    pub fn assign_random_socks_ports<E>(
        &mut self,
        mut pick: impl FnMut() -> Result<u16, E>,
    ) -> Result<Option<u16>, E> {
        let mut first_assigned = None;
        for pf in &mut self.port_forwards {
            if pf.is_random_dynamic() {
                let port = pick()?;
                pf.local_port = port;
                if first_assigned.is_none() {
                    first_assigned = Some(port);
                }
            }
        }
        Ok(first_assigned)
    }

    /// Returns the effective SOCKS proxy port, if a dynamic forward is
    /// configured with a concrete (already-assigned) port.
    ///
    /// Returns `None` when no dynamic forward exists or its port is still the
    /// `0` "pick at connection time" sentinel.
    #[must_use]
    pub fn socks_proxy_port(&self) -> Option<u16> {
        self.port_forwards
            .iter()
            .find(|pf| matches!(pf.direction, PortForwardDirection::Dynamic) && pf.local_port != 0)
            .map(|pf| pf.local_port)
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
    /// Inherit SSH key from parent group chain
    Inherit,
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

    /// Parses a `WIDTHxHEIGHT` resolution such as `2560x1440`.
    ///
    /// The separator may be `x` or `X`. Returns `None` for anything that is not
    /// two positive integers, so a caller can report the bad value rather than
    /// silently substituting a default.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (width, height) = text
            .trim()
            .split_once(['x', 'X'])
            .map(|(width, height)| (width.trim(), height.trim()))?;
        let width: u32 = width.parse().ok()?;
        let height: u32 = height.parse().ok()?;
        (width > 0 && height > 0).then_some(Self::new(width, height))
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
    #[default]
    Quality,
    /// Balanced - RemoteFX codec, adaptive compression, font smoothing
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

/// How the external RDP client sizes its window.
///
/// Read only by the external FreeRDP client. The embedded viewer has no use for
/// it: it is drawn into a widget whose size it already knows and renegotiates
/// over MS-RDPEDISP on every resize, so there is no monitor for it to size
/// against.
///
/// [`Self::FitScreen`] is the default because the previous behaviour — a fixed
/// resolution taken from a spin button the connection editor hides in embedded
/// mode — produced a `1920x1080` window on every display, including 4K ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RdpDisplayMode {
    /// Cover the monitor, keeping the window decorations (`/size:100%`).
    #[default]
    FitScreen,
    /// Take over the monitor completely (`/f`).
    Fullscreen,
    /// Use the resolution stored in [`RdpConfig::resolution`] (`/w:` + `/h:`).
    Custom,
    /// Span every connected monitor (`/multimon`).
    AllMonitors,
}

impl RdpDisplayMode {
    /// Returns all available display modes, in dropdown order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::FitScreen,
            Self::Fullscreen,
            Self::Custom,
            Self::AllMonitors,
        ]
    }

    /// Returns the untranslated display name for this mode.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::FitScreen => "Fit to screen",
            Self::Fullscreen => "Fullscreen",
            Self::Custom => "Custom resolution",
            Self::AllMonitors => "All monitors",
        }
    }

    /// Returns the dropdown index for this mode.
    #[must_use]
    pub const fn index(self) -> u32 {
        match self {
            Self::FitScreen => 0,
            Self::Fullscreen => 1,
            Self::Custom => 2,
            Self::AllMonitors => 3,
        }
    }

    /// Creates a mode from a dropdown index.
    #[must_use]
    pub const fn from_index(index: u32) -> Self {
        match index {
            1 => Self::Fullscreen,
            2 => Self::Custom,
            3 => Self::AllMonitors,
            _ => Self::FitScreen,
        }
    }

    /// Parses the `rustconn-cli` spelling of a display mode.
    ///
    /// Returns `None` for an unknown name so the caller can report it. The CLI's
    /// `value_parser` already restricts the input, which makes `None` a
    /// programming mismatch between the two lists rather than user error.
    #[must_use]
    pub fn from_cli_name(name: &str) -> Option<Self> {
        match name {
            "fit" => Some(Self::FitScreen),
            "fullscreen" => Some(Self::Fullscreen),
            "custom" => Some(Self::Custom),
            "multimon" => Some(Self::AllMonitors),
            _ => None,
        }
    }

    /// Returns the `rustconn-cli` spelling of this mode.
    #[must_use]
    pub const fn cli_name(self) -> &'static str {
        match self {
            Self::FitScreen => "fit",
            Self::Fullscreen => "fullscreen",
            Self::Custom => "custom",
            Self::AllMonitors => "multimon",
        }
    }

    /// Whether this mode sizes the session from a stored resolution.
    ///
    /// The connection editor uses this to decide whether the resolution row is
    /// worth showing, and the config builder to decide whether to store one.
    #[must_use]
    pub const fn uses_stored_resolution(self) -> bool {
        matches!(self, Self::Custom)
    }

    /// Returns the FreeRDP arguments that size the session for this mode.
    ///
    /// `resolution` is only read for [`Self::Custom`]; a `Custom` mode with no
    /// stored resolution falls back to filling the screen rather than letting
    /// FreeRDP apply its own `1024x768` default, which no display has.
    #[must_use]
    pub fn freerdp_args(self, resolution: Option<&Resolution>) -> Vec<String> {
        match self {
            // `/size:<p>%` with no `w`/`h` suffix applies the percentage to both
            // dimensions, so this is "as large as the monitor" (FreeRDP #5171).
            Self::FitScreen => vec!["/size:100%".to_string()],
            Self::Fullscreen => vec!["/f".to_string()],
            Self::Custom => resolution.map_or_else(
                || vec!["/size:100%".to_string()],
                |res| vec![format!("/w:{}", res.width), format!("/h:{}", res.height)],
            ),
            Self::AllMonitors => vec!["/multimon".to_string()],
        }
    }
}

/// Smallest desktop scale factor MS-RDPEDISP accepts, and FreeRDP's own default.
const FREERDP_MIN_SCALE_PERCENT: u16 = 100;

/// Largest desktop scale factor MS-RDPEDISP accepts.
const FREERDP_MAX_SCALE_PERCENT: u16 = 500;

/// The only three device scale factors MS-RDPEDISP accepts.
///
/// A desktop scale factor is discarded outright when the device scale factor is
/// not one of these, which is why [`ScaleOverride::freerdp_scale_args`] always
/// emits the pair.
const FREERDP_DEVICE_SCALE_STEPS: [u16; 3] = [100, 140, 180];

/// Returns the accepted device scale factor closest to `percent`.
fn nearest_freerdp_device_scale(percent: u16) -> u16 {
    FREERDP_DEVICE_SCALE_STEPS
        .into_iter()
        .min_by_key(|step| step.abs_diff(percent))
        .unwrap_or(FREERDP_MIN_SCALE_PERCENT)
}

/// Display scale override for embedded protocol viewers.
///
/// Controls the scale factor used to convert CSS pixels to device pixels
/// when negotiating resolution with the remote server. `Auto` uses the
/// system-reported scale factor; explicit values override it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScaleOverride {
    /// Request the widget's logical resolution and upscale locally (default,
    /// bandwidth-saving)
    #[default]
    Auto,
    /// Follow the display's HiDPI scale factor for a full-resolution
    /// ("retina") remote desktop that adapts across monitors
    Native,
    /// 1.25× scale
    Scale125,
    /// 1.5× scale
    Scale150,
    /// 2× scale
    Scale200,
    /// 3× scale
    Scale300,
    /// 4× scale
    Scale400,
}

impl ScaleOverride {
    /// Returns all available scale override options
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Auto,
            Self::Native,
            Self::Scale125,
            Self::Scale150,
            Self::Scale200,
            Self::Scale300,
            Self::Scale400,
        ]
    }

    /// Returns the display name for this scale override
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Auto => "Auto (system)",
            Self::Native => "Native (full HiDPI)",
            Self::Scale125 => "125%",
            Self::Scale150 => "150%",
            Self::Scale200 => "200%",
            Self::Scale300 => "300%",
            Self::Scale400 => "400%",
        }
    }

    /// Returns the dropdown index for this scale override
    #[must_use]
    pub const fn index(self) -> u32 {
        match self {
            Self::Auto => 0,
            Self::Native => 1,
            Self::Scale125 => 2,
            Self::Scale150 => 3,
            Self::Scale200 => 4,
            Self::Scale300 => 5,
            Self::Scale400 => 6,
        }
    }

    /// Creates a scale override from a dropdown index
    #[must_use]
    pub const fn from_index(index: u32) -> Self {
        match index {
            1 => Self::Native,
            2 => Self::Scale125,
            3 => Self::Scale150,
            4 => Self::Scale200,
            5 => Self::Scale300,
            6 => Self::Scale400,
            _ => Self::Auto,
        }
    }

    /// Returns the resolution multiplier applied to the logical (CSS) size,
    /// using the live display `system_scale` (compositor scale factor) for
    /// [`Self::Native`].
    ///
    /// `Auto` returns `1.0`: the remote desktop is requested at the widget's
    /// logical resolution, minimising network traffic. On fractional-scaling
    /// displays (125%, 150%) this means the compositor applies bilinear upscale
    /// — slightly softer text but much faster over WAN. For pixel-perfect
    /// sharpness on fractional displays, use `Native`.
    ///
    /// `Native` returns the real compositor scale (including fractional values
    /// like 1.25), so the RDP framebuffer matches device pixels exactly —
    /// the compositor renders 1:1 with no interpolation. Costs more bandwidth
    /// (≈56% more pixels at 125%) but eliminates blur.
    ///
    /// The explicit steps (125%–400%) request a fixed multiplier regardless of
    /// the display.
    #[must_use]
    pub fn resolved_scale(self, system_scale: f64) -> f64 {
        match self {
            Self::Auto => 1.0,
            Self::Native => system_scale.max(1.0),
            Self::Scale125 => 1.25,
            Self::Scale150 => 1.5,
            Self::Scale200 => 2.0,
            Self::Scale300 => 3.0,
            Self::Scale400 => 4.0,
        }
    }

    /// Returns the FreeRDP DPI arguments for this scale override.
    ///
    /// `system_scale_percent` is the live compositor scale as a percentage (for
    /// example `200` on a 2× display) and is only read for [`Self::Native`].
    ///
    /// Emits `/scale-desktop:` — the desktop scale factor sent to the server,
    /// which MS-RDPEDISP accepts between
    /// [`FREERDP_MIN_SCALE_PERCENT`] and [`FREERDP_MAX_SCALE_PERCENT`] — paired
    /// with the nearest accepted `/scale-device:`. The pair is deliberate: a
    /// desktop scale factor is ignored when the device scale factor is not one
    /// of [`FREERDP_DEVICE_SCALE_STEPS`], so the desktop value alone changes
    /// nothing.
    ///
    /// [`Self::Auto`] returns an empty vector, matching FreeRDP's own 100%
    /// default: the session is requested at the window's own size with no DPI
    /// override, which is what "Auto" means for the embedded viewer too.
    #[must_use]
    pub fn freerdp_scale_args(self, system_scale_percent: u16) -> Vec<String> {
        let requested = match self {
            Self::Auto => return Vec::new(),
            Self::Native => system_scale_percent,
            Self::Scale125 => 125,
            Self::Scale150 => 150,
            Self::Scale200 => 200,
            Self::Scale300 => 300,
            Self::Scale400 => 400,
        };
        let desktop = requested.clamp(FREERDP_MIN_SCALE_PERCENT, FREERDP_MAX_SCALE_PERCENT);
        // A 100% desktop scale factor is FreeRDP's default; saying it explicitly
        // adds an argument that changes nothing. This is the `Native` case on a
        // display that is not scaled at all.
        if desktop == FREERDP_MIN_SCALE_PERCENT {
            return Vec::new();
        }
        vec![
            format!("/scale-desktop:{desktop}"),
            format!("/scale-device:{}", nearest_freerdp_device_scale(desktop)),
        ]
    }
}

/// RDP security layer selection for FreeRDP connections.
///
/// Controls which security protocol is used during the RDP handshake.
/// Legacy servers (Windows Server 2012 / Windows 7) may require `Rdp` or `Tls`
/// instead of the default `Negotiate`.
///
/// **Note:** `Rdp` and `Tls` modes are incompatible with IronRDP (which requires
/// TLS 1.2+ via `rustls`). When these modes are selected, the connection
/// automatically falls back to external FreeRDP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RdpSecurityLayer {
    /// Server negotiates the best available method (default)
    #[default]
    Negotiate,
    /// RDP Security Layer only (legacy, no TLS — for very old servers)
    Rdp,
    /// TLS encryption without NLA
    Tls,
    /// Network Level Authentication (CredSSP over TLS)
    Nla,
}

impl RdpSecurityLayer {
    /// Returns all available security layers
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Negotiate, Self::Rdp, Self::Tls, Self::Nla]
    }

    /// Returns the display name for this security layer
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Negotiate => "Negotiate (Auto)",
            Self::Rdp => "RDP (Legacy)",
            Self::Tls => "TLS",
            Self::Nla => "NLA",
        }
    }

    /// Returns the index of this layer in the `all()` array
    #[must_use]
    pub const fn index(self) -> u32 {
        match self {
            Self::Negotiate => 0,
            Self::Rdp => 1,
            Self::Tls => 2,
            Self::Nla => 3,
        }
    }

    /// Creates a security layer from a dropdown index
    #[must_use]
    pub const fn from_index(index: u32) -> Self {
        match index {
            1 => Self::Rdp,
            2 => Self::Tls,
            3 => Self::Nla,
            _ => Self::Negotiate,
        }
    }

    /// Returns the FreeRDP `/sec:` argument for this security layer.
    ///
    /// `Negotiate` returns `None` (FreeRDP default behavior).
    #[must_use]
    pub const fn freerdp_arg(self) -> Option<&'static str> {
        match self {
            Self::Negotiate => None,
            Self::Rdp => Some("/sec:rdp"),
            Self::Tls => Some("/sec:tls"),
            Self::Nla => Some("/sec:nla"),
        }
    }

    /// Whether this security layer requires FreeRDP (incompatible with IronRDP).
    ///
    /// IronRDP uses `rustls` which only supports TLS 1.2+, so `Rdp` (no TLS)
    /// cannot work. `Tls` without NLA also requires OpenSSL-level control
    /// that `rustls` doesn't provide.
    #[must_use]
    pub const fn requires_freerdp(self) -> bool {
        matches!(self, Self::Rdp | Self::Tls)
    }
}

/// Where the remote session's audio is played.
///
/// Mirrors the three RDP audio modes defined by MS-RDPBCGR and offered by
/// `mstsc`: bring the sound to the client, leave it on the remote machine's
/// own output, or disable session audio entirely. The wire representation is
/// two independent Client Info flags (`INFO_NOAUDIOPLAYBACK` and
/// `INFO_REMOTECONSOLEAUDIO`), which is why a plain bool cannot express it —
/// "not redirected" and "played on the server" are different states, and
/// neither of them is the absence of the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RdpAudioMode {
    /// Do not play session audio anywhere (`/audio-mode:2`).
    ///
    /// Default, because it matches what RustConn has always done and what the
    /// previous `audio_redirect: false` meant. Windows reports no audio device
    /// inside the session in this mode.
    #[default]
    None,
    /// Redirect the remote audio to this computer (`/sound`).
    Local,
    /// Leave the audio on the remote computer's own output
    /// (`/audio-mode:1`).
    Remote,
}

impl RdpAudioMode {
    /// Returns all available audio modes, in dropdown order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::None, Self::Local, Self::Remote]
    }

    /// Returns the untranslated display name for this mode.
    ///
    /// Callers in the GUI wrap the result in `i18n()`.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::None => "Do not play",
            Self::Local => "Play on this computer",
            Self::Remote => "Play on the remote computer",
        }
    }

    /// Returns the dropdown index for this mode.
    #[must_use]
    pub const fn index(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Local => 1,
            Self::Remote => 2,
        }
    }

    /// Creates a mode from a dropdown index.
    #[must_use]
    pub const fn from_index(index: u32) -> Self {
        match index {
            1 => Self::Local,
            2 => Self::Remote,
            _ => Self::None,
        }
    }

    /// Returns the FreeRDP argument that selects this mode.
    ///
    /// Always returns an argument: leaving the mode implicit makes FreeRDP
    /// apply its own default (`AudioPlayback` and `RemoteConsoleAudio` both
    /// false, i.e. no audio at all), which is what issue #245 was about.
    /// Numeric `/audio-mode:` values are used rather than the `redirect`,
    /// `server` and `none` aliases because the aliases only exist in recent
    /// FreeRDP 3.x, while the numbers are accepted by FreeRDP 2 as well.
    #[must_use]
    pub const fn freerdp_arg(self) -> &'static str {
        match self {
            // `/sound` also selects the platform audio backend, which a bare
            // `/audio-mode:0` does not do.
            Self::Local => "/sound",
            Self::Remote => "/audio-mode:1",
            Self::None => "/audio-mode:2",
        }
    }

    /// Whether this mode requires FreeRDP instead of IronRDP.
    ///
    /// `ironrdp-connector` only exposes `enable_audio_playback`, which maps to
    /// `INFO_NOAUDIOPLAYBACK`. It never sets `INFO_REMOTECONSOLEAUDIO`, so
    /// "play on the remote computer" cannot be expressed by the embedded
    /// client and has to go through FreeRDP.
    #[must_use]
    pub const fn requires_freerdp(self) -> bool {
        matches!(self, Self::Remote)
    }

    /// Whether the remote audio stream is played by this client.
    #[must_use]
    pub const fn is_local_playback(self) -> bool {
        matches!(self, Self::Local)
    }

    /// Returns the CLI token for this mode.
    #[must_use]
    pub const fn as_cli_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }

    /// Parses a mode from its CLI token, case-insensitively.
    ///
    /// Returns `None` for anything else, so the caller can report the invalid
    /// value rather than silently picking a default.
    #[must_use]
    pub fn from_cli_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" | "off" => Some(Self::None),
            "local" | "client" => Some(Self::Local),
            "remote" | "server" => Some(Self::Remote),
            _ => None,
        }
    }
}

/// RDP protocol configuration
// Allow 4 bools - these are distinct RDP connection options
#[expect(
    clippy::struct_excessive_bools,
    reason = "settings/flags struct mirrors persisted config 1:1; bools represent independent toggles, not a state machine"
)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdpConfig {
    /// RDP client mode (embedded or external)
    #[serde(default)]
    pub client_mode: RdpClientMode,
    /// Performance mode (quality/balanced/speed)
    #[serde(default)]
    pub performance_mode: RdpPerformanceMode,
    /// Graphics pipeline mode for embedded IronRDP client.
    /// Auto (default) negotiates GFX/H.264; Legacy skips GFX entirely.
    /// Only relevant for Embedded mode. (Issue #218)
    #[serde(default)]
    pub graphics_mode: crate::rdp_client::graphics::GraphicsMode,
    /// How the external client sizes its window.
    ///
    /// Only read when the session runs in an external FreeRDP window — either
    /// because [`Self::client_mode`] says so, or because the embedded client
    /// handed the connection over. The embedded viewer sizes itself from the
    /// widget it is drawn into.
    #[serde(default)]
    pub external_display_mode: RdpDisplayMode,
    /// Screen resolution, used when [`Self::external_display_mode`] is
    /// [`RdpDisplayMode::Custom`].
    ///
    /// `None` means "no fixed resolution was chosen". It used to be written
    /// unconditionally from a spin button the editor hides in embedded mode, so
    /// every profile carried the spin button's `1920x1080` default whether the
    /// user had ever seen the row or not — and that value then sized every
    /// external window, including on 4K displays.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<Resolution>,
    /// Color depth (8, 15, 16, 24, or 32) - overrides performance_mode if set
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_depth: Option<u8>,
    /// Enable audio redirection to this computer.
    ///
    /// Superseded by [`RdpConfig::audio_mode`], kept for compatibility: older
    /// RustConn versions and the import/export formats (Remmina, `MobaXterm`,
    /// `.rdp`) only understand this boolean. It is written on save so that
    /// downgrading does not lose the setting, and it is the fallback when
    /// `audio_mode` is absent. Read it through
    /// [`RdpConfig::effective_audio_mode`] rather than directly.
    #[serde(default)]
    pub audio_redirect: bool,
    /// Where the session audio is played. `None` means "not migrated yet" —
    /// fall back to `audio_redirect`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_mode: Option<RdpAudioMode>,
    /// Enable printer redirection (maps local CUPS printer into the session)
    #[serde(default)]
    pub printer_enabled: bool,
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
    /// Display scale override for embedded mode
    #[serde(default)]
    pub scale_override: ScaleOverride,
    /// Disable Network Level Authentication
    #[serde(default)]
    pub disable_nla: bool,
    /// Security layer selection (Negotiate/RDP/TLS/NLA).
    /// Legacy servers (Windows 2012/Win7) may need `Rdp` or `Tls`.
    /// `Rdp` and `Tls` force FreeRDP fallback (incompatible with IronRDP).
    #[serde(default)]
    pub security_layer: RdpSecurityLayer,
    /// TLS security level for FreeRDP (0–5). `None` = FreeRDP default.
    /// Level 0 enables compatibility with legacy servers (TLS 1.0).
    /// Only effective with FreeRDP; IronRDP uses `rustls` (TLS 1.2+ only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls_security_level: Option<u8>,
    /// Skip TLS certificate verification (use `/cert:ignore` instead of `/cert:tofu`).
    /// Default: false (TOFU — trust-on-first-use, like SSH known_hosts).
    #[serde(default)]
    pub ignore_certificate: bool,
    /// Enable clipboard sharing between local and remote
    #[serde(default = "default_true")]
    pub clipboard_enabled: bool,
    /// Show local mouse cursor over embedded viewer (disable to avoid double cursor)
    #[serde(default = "default_true")]
    pub show_local_cursor: bool,
    /// Remove the floating session toolbar and its reveal handle from the
    /// embedded viewer (issue #260).
    ///
    /// Stated negatively so that `false` — the value `Default::default()` and a
    /// stored profile without the key both produce — keeps the toolbar. Every
    /// importer, template and wizard builds its protocol config from
    /// [`Default`], so a positive `default_true` field would have arrived
    /// switched off through all of them.
    ///
    /// Scoped to the viewer's own chrome. The split view's panel corner buttons
    /// stay put: they are the only discoverable way to close or detach a pane,
    /// and suppressing them left RDP, VNC and Web panes with no way out.
    #[serde(default)]
    pub hide_floating_toolbar: bool,
    /// Enable mouse jiggler to prevent idle disconnect
    #[serde(default)]
    pub jiggler_enabled: bool,
    /// Mouse jiggler interval in seconds (10–600, default: 60)
    #[serde(default = "default_jiggler_interval")]
    pub jiggler_interval_secs: u32,
    /// ID of an SSH connection to use as a jump host (SSH tunnel).
    /// The RDP connection is tunnelled through this SSH host via local
    /// port forwarding (`ssh -L`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump_host_id: Option<uuid::Uuid>,

    /// Inter-character delay for autotype in milliseconds.
    /// Controls how fast characters are sent when using "Type Clipboard" or
    /// "Type Text" features. Higher values needed for Citrix/slow gateways.
    /// Range: 5–200ms. Default: 20ms.
    #[serde(default = "default_autotype_delay")]
    pub autotype_delay_ms: u32,

    /// Initial delay before autotype starts in milliseconds.
    /// Gives the user time to focus the target input field on the remote desktop.
    /// Range: 0–5000ms. Default: 0ms.
    #[serde(default)]
    pub autotype_initial_delay_ms: u32,

    /// Force full reconnect on window resize instead of using Display Control Channel.
    /// Useful for legacy RDP servers that don't support MS-RDPEDISP or when the
    /// server ignores dynamic resolution changes.
    /// Default: false (use Display Control for seamless resize).
    #[serde(default)]
    pub reconnect_on_resize: bool,

    /// Send scripts via clipboard paste (Ctrl+V) instead of character-by-character
    /// autotype. Clipboard paste is instant regardless of script length, while
    /// autotype at 5ms/char takes ~10s for a 2000-char script.
    /// Default: true (use clipboard paste for speed).
    #[serde(default = "default_true")]
    pub script_paste_via_clipboard: bool,

    /// RemoteApp program path or alias.
    /// When set, the RDP session launches a single application instead of a full desktop.
    /// Forces FreeRDP fallback (IronRDP does not support RAIL protocol).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_app_program: Option<String>,

    /// RemoteApp command-line arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_app_args: Option<String>,

    /// RemoteApp display name (shown in taskbar/window title).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_app_name: Option<String>,

    /// Enable Multipath TCP for the embedded RDP connection.
    /// Uses multiple network paths for seamless mobility and bandwidth aggregation.
    /// Requires kernel MPTCP support (Linux 5.6+). Falls back to regular TCP.
    /// Only applies to Embedded mode; External FreeRDP handles its own sockets.
    #[serde(default)]
    pub mptcp: bool,
}

/// Written out by hand rather than derived, so that it agrees with the serde
/// defaults above.
///
/// `#[derive(Default)]` does not read `#[serde(default = "…")]`, so the two
/// disagreed on five fields: `clipboard_enabled`, `show_local_cursor` and
/// `script_paste_via_clipboard` came back `false` where a stored profile without
/// the key deserialises to `true`, `jiggler_interval_secs` came back `0` instead
/// of 60, and `autotype_delay_ms` `0` instead of 20 — fast enough that autotype
/// drops characters on a Citrix or gateway session. That mattered because
/// `RdpConfig::default()` is not a corner case: every importer (Remmina,
/// RoyalTS, RDM, SecureCRT, CSV, Ásbrú, libvirt), `models::template`,
/// `sync::inventory`, the connection wizard and RDP quick-connect all build
/// their config from it. Adding `hide_floating_toolbar` (issue #260) is what
/// surfaced this: the field is negative precisely so it cannot be caught by the
/// same trap, and then the trap turned out to be worth closing.
///
/// `SpiceConfig` already did this; `default_agrees_with_serde` in
/// `default_consistency_tests` now checks all three so the next added field
/// cannot re-open the gap silently.
impl Default for RdpConfig {
    fn default() -> Self {
        Self {
            client_mode: RdpClientMode::default(),
            performance_mode: RdpPerformanceMode::default(),
            graphics_mode: crate::rdp_client::graphics::GraphicsMode::default(),
            external_display_mode: RdpDisplayMode::default(),
            resolution: None,
            color_depth: None,
            audio_redirect: false,
            audio_mode: None,
            printer_enabled: false,
            gateway: None,
            shared_folders: Vec::new(),
            custom_args: Vec::new(),
            keyboard_layout: None,
            scale_override: ScaleOverride::default(),
            disable_nla: false,
            security_layer: RdpSecurityLayer::default(),
            tls_security_level: None,
            ignore_certificate: false,
            clipboard_enabled: default_true(),
            show_local_cursor: default_true(),
            hide_floating_toolbar: false,
            jiggler_enabled: false,
            jiggler_interval_secs: default_jiggler_interval(),
            jump_host_id: None,
            autotype_delay_ms: default_autotype_delay(),
            autotype_initial_delay_ms: 0,
            reconnect_on_resize: false,
            script_paste_via_clipboard: default_true(),
            remote_app_program: None,
            remote_app_args: None,
            remote_app_name: None,
            mptcp: false,
        }
    }
}

impl RdpConfig {
    /// Returns the effective color depth based on performance mode and explicit setting
    #[must_use]
    pub fn effective_color_depth(&self) -> u8 {
        self.color_depth
            .unwrap_or_else(|| self.performance_mode.color_depth())
    }

    /// Returns where the session audio should be played.
    ///
    /// Prefers the three-state [`RdpConfig::audio_mode`] and falls back to the
    /// legacy `audio_redirect` boolean for profiles written before the mode
    /// existed: `true` becomes [`RdpAudioMode::Local`], `false` becomes
    /// [`RdpAudioMode::None`], which is what that boolean actually meant on
    /// the wire.
    #[must_use]
    pub fn effective_audio_mode(&self) -> RdpAudioMode {
        self.audio_mode.unwrap_or({
            if self.audio_redirect {
                RdpAudioMode::Local
            } else {
                RdpAudioMode::None
            }
        })
    }

    /// Sets the audio mode, keeping the legacy `audio_redirect` bool in sync.
    ///
    /// Writing both means an older RustConn reading the same profile still
    /// sees local redirection turned on or off correctly, and the Remmina /
    /// `MobaXterm` exporters keep working unchanged.
    pub const fn set_audio_mode(&mut self, mode: RdpAudioMode) {
        self.audio_mode = Some(mode);
        self.audio_redirect = mode.is_local_playback();
    }

    /// Whether this configuration requires FreeRDP instead of IronRDP.
    ///
    /// Returns `true` when the security layer or TLS level is incompatible
    /// with IronRDP's `rustls` backend (TLS 1.2+ only), when RemoteApp
    /// is configured (IronRDP does not support RAIL protocol), or when the
    /// audio mode is one IronRDP cannot signal.
    #[must_use]
    pub fn requires_freerdp_fallback(&self) -> bool {
        // RemoteApp requires RAIL protocol — not supported by IronRDP
        if self.is_remote_app() {
            return true;
        }
        // Security layer incompatible with IronRDP
        if self.security_layer.requires_freerdp() {
            return true;
        }
        // TLS security level < 2 requires OpenSSL (legacy TLS 1.0/1.1)
        if let Some(level) = self.tls_security_level
            && level < 2
        {
            return true;
        }
        // "Play on the remote computer" needs INFO_REMOTECONSOLEAUDIO, which
        // ironrdp-connector never sets (issue #245)
        if self.effective_audio_mode().requires_freerdp() {
            return true;
        }
        false
    }

    /// Returns whether this is a RemoteApp session.
    #[must_use]
    pub fn is_remote_app(&self) -> bool {
        self.remote_app_program
            .as_ref()
            .is_some_and(|p| !p.is_empty())
    }

    /// Builds FreeRDP command-line arguments for RemoteApp (RAIL) mode.
    ///
    /// Returns an empty `Vec` if no RemoteApp program is configured.
    /// The returned args use FreeRDP 3.x syntax: `/app:`, `/app-cmd:`, `/app-name:`.
    /// Values containing spaces are quoted for correct FreeRDP parsing.
    #[must_use]
    pub fn remote_app_freerdp_args(&self) -> Vec<String> {
        build_remote_app_freerdp_args(
            self.remote_app_program.as_deref(),
            self.remote_app_args.as_deref(),
            self.remote_app_name.as_deref(),
        )
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
// Allow 4 bools - these are distinct VNC connection options
#[expect(
    clippy::struct_excessive_bools,
    reason = "settings/flags struct mirrors persisted config 1:1; bools represent independent toggles, not a state machine"
)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Display scale override for embedded mode
    #[serde(default)]
    pub scale_override: ScaleOverride,
    /// Show local mouse cursor over embedded viewer (disable to avoid double cursor)
    #[serde(default = "default_true")]
    pub show_local_cursor: bool,
    /// Remove the floating session toolbar and its reveal handle from the
    /// embedded viewer (issue #260).
    ///
    /// See [`RdpConfig::hide_floating_toolbar`] for why this reads as "hide"
    /// rather than "show".
    #[serde(default)]
    pub hide_floating_toolbar: bool,
    /// ID of an SSH connection to use as a jump host (SSH tunnel).
    /// The VNC connection is tunnelled through this SSH host via local
    /// port forwarding (`ssh -L`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump_host_id: Option<uuid::Uuid>,
    /// Accept untrusted/self-signed TLS certificates without prompting.
    /// When true, the external VNC viewer will be instructed to skip
    /// certificate verification (similar to RDP's `ignore_certificate`).
    /// Default: false (strict verification — connection fails on untrusted cert).
    #[serde(default)]
    pub accept_certificate: bool,

    /// Enable Multipath TCP for the embedded VNC connection.
    /// Uses multiple network paths for seamless mobility and bandwidth aggregation.
    /// Requires kernel MPTCP support (Linux 5.6+). Falls back to regular TCP.
    /// Only applies to Embedded mode; External viewers handle their own sockets.
    #[serde(default)]
    pub mptcp: bool,
}

/// Written out by hand rather than derived, for the reason given on
/// [`RdpConfig`]'s `Default`: `scaling`, `clipboard_enabled` and
/// `show_local_cursor` all deserialise to `true` from a stored profile that
/// omits them, and a derived `Default` handed back `false`.
impl Default for VncConfig {
    fn default() -> Self {
        Self {
            client_mode: VncClientMode::default(),
            performance_mode: VncPerformanceMode::default(),
            encoding: None,
            compression: None,
            quality: None,
            view_only: false,
            scaling: default_true(),
            clipboard_enabled: default_true(),
            custom_args: Vec::new(),
            scale_override: ScaleOverride::default(),
            show_local_cursor: default_true(),
            hide_floating_toolbar: false,
            jump_host_id: None,
            accept_certificate: false,
            mptcp: false,
        }
    }
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
#[expect(
    clippy::struct_excessive_bools,
    reason = "settings/flags struct mirrors persisted config 1:1; bools represent independent toggles, not a state machine"
)]
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
    /// Show local mouse cursor over embedded viewer (disable to avoid double cursor)
    #[serde(default = "default_true")]
    pub show_local_cursor: bool,
    /// ID of an SSH connection to use as a jump host (SSH tunnel).
    /// The SPICE connection is tunnelled through this SSH host via local
    /// port forwarding (`ssh -L`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump_host_id: Option<uuid::Uuid>,
    /// Path to a SPICE unix socket (e.g. `/run/libvirt/qemu/vm-spice.sock`).
    /// When set, the connection uses `spice+unix://` URI instead of host:port.
    /// Mutually exclusive with host:port — jump_host_id is ignored when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unix_socket_path: Option<PathBuf>,
}

impl Default for SpiceConfig {
    fn default() -> Self {
        Self {
            tls_enabled: false,
            ca_cert_path: None,
            skip_cert_verify: false,
            usb_redirection: false,
            shared_folders: Vec::new(),
            clipboard_enabled: true,
            image_compression: None,
            proxy: None,
            show_local_cursor: true,
            jump_host_id: None,
            unix_socket_path: None,
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
    /// Hoop.dev zero-trust access gateway
    #[serde(rename = "hoop_dev")]
    HoopDev,
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
            Self::HoopDev => "Hoop.dev",
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
            Self::Teleport => "preferences-system-symbolic", // Teleport - system/gear
            Self::TailscaleSsh => "network-vpn-symbolic", // Tailscale - VPN
            Self::Boundary => "dialog-password-symbolic", // Boundary - password/lock
            Self::HoopDev => "network-transmit-symbolic", // Hoop.dev - network transmit
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
            Self::HoopDev => "hoop",
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
            Self::HoopDev,
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
}

impl Default for ZeroTrustConfig {
    fn default() -> Self {
        Self {
            provider: ZeroTrustProvider::default(),
            provider_config: ZeroTrustProviderConfig::AwsSsm(AwsSsmConfig::default()),
            custom_args: Vec::new(),
        }
    }
}

impl ZeroTrustConfig {
    /// Validates provider-specific configuration fields.
    ///
    /// Returns `Ok(())` if the configuration is valid, or a `ProtocolError`
    /// describing which required field is missing or invalid.
    ///
    /// # Errors
    ///
    /// Returns `ProtocolError::InvalidConfig` if required fields are empty.
    #[expect(
        clippy::too_many_lines,
        reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
    )] // Single match over 10 provider variants
    pub fn validate(&self) -> crate::error::ProtocolResult<()> {
        use crate::error::ProtocolError;

        match &self.provider_config {
            ZeroTrustProviderConfig::AwsSsm(cfg) => {
                if cfg.target.trim().is_empty() {
                    return Err(ProtocolError::InvalidConfig(
                        "AWS SSM target cannot be empty".into(),
                    ));
                }
            }
            ZeroTrustProviderConfig::GcpIap(cfg) => {
                if cfg.instance.trim().is_empty() {
                    return Err(ProtocolError::InvalidConfig(
                        "GCP IAP instance cannot be empty".into(),
                    ));
                }
                if cfg.zone.trim().is_empty() {
                    return Err(ProtocolError::InvalidConfig(
                        "GCP IAP zone cannot be empty".into(),
                    ));
                }
            }
            ZeroTrustProviderConfig::AzureBastion(cfg) => {
                if cfg.target_resource_id.trim().is_empty() {
                    return Err(ProtocolError::InvalidConfig(
                        "Azure Bastion target resource ID cannot be empty".into(),
                    ));
                }
                if cfg.resource_group.trim().is_empty() {
                    return Err(ProtocolError::InvalidConfig(
                        "Azure Bastion resource group cannot be empty".into(),
                    ));
                }
                if cfg.bastion_name.trim().is_empty() {
                    return Err(ProtocolError::InvalidConfig(
                        "Azure Bastion name cannot be empty".into(),
                    ));
                }
            }
            ZeroTrustProviderConfig::AzureSsh(cfg) => {
                if cfg.vm_name.trim().is_empty() {
                    return Err(ProtocolError::InvalidConfig(
                        "Azure SSH VM name cannot be empty".into(),
                    ));
                }
                if cfg.resource_group.trim().is_empty() {
                    return Err(ProtocolError::InvalidConfig(
                        "Azure SSH resource group cannot be empty".into(),
                    ));
                }
            }
            ZeroTrustProviderConfig::OciBastion(cfg) => {
                if cfg.bastion_id.trim().is_empty() {
                    return Err(ProtocolError::InvalidConfig(
                        "OCI Bastion ID cannot be empty".into(),
                    ));
                }
                if cfg.target_resource_id.trim().is_empty() {
                    return Err(ProtocolError::InvalidConfig(
                        "OCI target resource ID cannot be empty".into(),
                    ));
                }
                if cfg.target_private_ip.trim().is_empty() {
                    return Err(ProtocolError::InvalidConfig(
                        "OCI target private IP cannot be empty".into(),
                    ));
                }
            }
            ZeroTrustProviderConfig::CloudflareAccess(cfg) => {
                if cfg.hostname.trim().is_empty() {
                    return Err(ProtocolError::InvalidConfig(
                        "Cloudflare Access hostname cannot be empty".into(),
                    ));
                }
            }
            ZeroTrustProviderConfig::Teleport(cfg) => {
                if cfg.host.trim().is_empty() {
                    return Err(ProtocolError::InvalidConfig(
                        "Teleport host cannot be empty".into(),
                    ));
                }
            }
            ZeroTrustProviderConfig::TailscaleSsh(cfg) => {
                if cfg.host.trim().is_empty() {
                    return Err(ProtocolError::InvalidConfig(
                        "Tailscale SSH host cannot be empty".into(),
                    ));
                }
            }
            ZeroTrustProviderConfig::Boundary(cfg) => {
                if cfg.target.trim().is_empty() {
                    return Err(ProtocolError::InvalidConfig(
                        "Boundary target cannot be empty".into(),
                    ));
                }
            }
            ZeroTrustProviderConfig::HoopDev(cfg) => {
                if cfg.connection_name.trim().is_empty() {
                    return Err(ProtocolError::InvalidConfig(
                        "Hoop.dev connection name cannot be empty".into(),
                    ));
                }
            }
            ZeroTrustProviderConfig::Generic(cfg) => {
                if cfg.command_template.trim().is_empty() {
                    return Err(ProtocolError::InvalidConfig(
                        "Generic ZeroTrust command template cannot be empty".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Builds the command and arguments for this Zero Trust connection
    ///
    /// Returns a tuple of (program, arguments) that can be used to spawn the process.
    /// The `username` parameter is used for providers that support it.
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
    )]
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
                // In Flatpak, ~/.ssh/ is read-only so gcloud cannot
                // generate its SSH key pair there. Redirect to the
                // writable sandbox SSH directory and copy existing
                // host keys if available.
                if let Some(ssh_dir) = crate::flatpak::get_flatpak_ssh_dir() {
                    let key_path = ssh_dir.join("google_compute_engine");
                    // Copy existing gcloud SSH keys from host if not
                    // yet present in the writable directory.
                    if !key_path.exists()
                        && let Ok(home) = std::env::var("HOME")
                    {
                        let host_key =
                            std::path::PathBuf::from(&home).join(".ssh/google_compute_engine");
                        if host_key.exists() {
                            let _ = std::fs::copy(&host_key, &key_path);
                            // Also copy the public key
                            let host_pub = host_key.with_extension("pub");
                            let sandbox_pub = key_path.with_extension("pub");
                            if host_pub.exists() {
                                let _ = std::fs::copy(&host_pub, &sandbox_pub);
                            }
                        }
                    }
                    a.push("--ssh-key-file".to_string());
                    a.push(key_path.display().to_string());
                    // gcloud also writes google_compute_known_hosts
                    // to ~/.ssh/ which is read-only. The file is written
                    // by gcloud's own Python code (not ssh), so --ssh-flag
                    // alone doesn't help. Use --strict-host-key-checking=no
                    // to skip gcloud's known_hosts write (IAP tunnel already
                    // authenticates via Google infrastructure), and redirect
                    // ssh's own UserKnownHostsFile to the writable dir.
                    let known_hosts = ssh_dir.join("google_compute_known_hosts");
                    a.push("--strict-host-key-checking=no".to_string());
                    a.push("--ssh-flag=-o".to_string());
                    a.push(format!(
                        "--ssh-flag=UserKnownHostsFile={}",
                        known_hosts.display()
                    ));
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
                let mut a = vec![
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
                if cfg.ssh_public_key_file.as_os_str() != "" {
                    a.push("--ssh-public-key-file".to_string());
                    a.push(cfg.ssh_public_key_file.display().to_string());
                }
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
            ZeroTrustProviderConfig::HoopDev(cfg) => {
                let mut a = vec!["connect".to_string(), cfg.connection_name.clone()];
                if let Some(ref url) = cfg.gateway_url
                    && !url.is_empty()
                {
                    a.push("--api-url".to_string());
                    a.push(url.clone());
                }
                if let Some(ref url) = cfg.grpc_url
                    && !url.is_empty()
                {
                    a.push("--grpc-url".to_string());
                    a.push(url.clone());
                }
                ("hoop".to_string(), a)
            }
            ZeroTrustProviderConfig::Generic(cfg) => {
                // Parse the command template
                let mut cmd = cfg.command_template.clone();
                // Embed custom_args into the shell command (appending after -c
                // would make them positional parameters $0/$1 which are ignored
                // unless the template explicitly references them)
                if !self.custom_args.is_empty() {
                    cmd.push(' ');
                    cmd.push_str(&self.custom_args.join(" "));
                }
                // Simple shell execution
                let a = vec!["-c".to_string(), cmd];
                ("sh".to_string(), a)
            }
        };

        // Append custom args (skip for Generic — already embedded above)
        if !matches!(self.provider_config, ZeroTrustProviderConfig::Generic(_)) {
            args.1.extend(self.custom_args.clone());
        }

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
    /// Hoop.dev zero-trust access gateway configuration
    HoopDev(HoopDevConfig),
    /// Generic custom command configuration
    Generic(GenericZeroTrustConfig),
}

impl ZeroTrustProviderConfig {
    /// Build a `ZeroTrustProviderConfig` from wizard-style positional fields.
    ///
    /// Maps the provider enum + up to 3 string fields into the correct config struct.
    /// Used by the Connection Wizard and property tests.
    ///
    /// # Field mapping per provider
    ///
    /// | Provider | field1 | field2 | field3 |
    /// |----------|--------|--------|--------|
    /// | Generic | — (uses `command`) | — | — |
    /// | AwsSsm | target | region | profile |
    /// | GcpIap | instance | zone | project |
    /// | AzureBastion | resource_id | resource_group | bastion_name |
    /// | AzureSsh | vm_name | resource_group | — |
    /// | CloudflareAccess | hostname | — | — |
    /// | Teleport | host | cluster | — |
    /// | TailscaleSsh | host | — | — |
    /// | Boundary | target | addr | — |
    /// | HoopDev | connection_name | gateway_url | — |
    /// | OciBastion | — (uses `command` fallback) | — | — |
    #[must_use]
    pub fn from_wizard_fields(
        provider: ZeroTrustProvider,
        command: Option<&str>,
        field1: Option<&str>,
        field2: Option<&str>,
        field3: Option<&str>,
    ) -> Self {
        match provider {
            ZeroTrustProvider::Generic => Self::Generic(GenericZeroTrustConfig {
                command_template: command.unwrap_or_default().to_string(),
            }),
            ZeroTrustProvider::AwsSsm => Self::AwsSsm(AwsSsmConfig {
                target: field1.unwrap_or_default().to_string(),
                region: field2.map(String::from),
                profile: field3.unwrap_or("default").to_string(),
            }),
            ZeroTrustProvider::GcpIap => Self::GcpIap(GcpIapConfig {
                instance: field1.unwrap_or_default().to_string(),
                zone: field2.unwrap_or_default().to_string(),
                project: field3.map(String::from),
            }),
            ZeroTrustProvider::AzureBastion => Self::AzureBastion(AzureBastionConfig {
                target_resource_id: field1.unwrap_or_default().to_string(),
                resource_group: field2.unwrap_or_default().to_string(),
                bastion_name: field3.unwrap_or_default().to_string(),
            }),
            ZeroTrustProvider::AzureSsh => Self::AzureSsh(AzureSshConfig {
                vm_name: field1.unwrap_or_default().to_string(),
                resource_group: field2.unwrap_or_default().to_string(),
            }),
            ZeroTrustProvider::CloudflareAccess => Self::CloudflareAccess(CloudflareAccessConfig {
                hostname: field1.unwrap_or_default().to_string(),
                username: None,
            }),
            ZeroTrustProvider::Teleport => Self::Teleport(TeleportConfig {
                host: field1.unwrap_or_default().to_string(),
                username: None,
                cluster: field2.map(String::from),
            }),
            ZeroTrustProvider::TailscaleSsh => Self::TailscaleSsh(TailscaleSshConfig {
                host: field1.unwrap_or_default().to_string(),
                username: None,
            }),
            ZeroTrustProvider::Boundary => Self::Boundary(BoundaryConfig {
                target: field1.unwrap_or_default().to_string(),
                addr: field2.map(String::from),
            }),
            ZeroTrustProvider::HoopDev => Self::HoopDev(HoopDevConfig {
                connection_name: field1.unwrap_or_default().to_string(),
                gateway_url: field2.map(String::from),
                grpc_url: None,
            }),
            ZeroTrustProvider::OciBastion => {
                // OCI Bastion not in wizard (too many fields) — use Generic fallback
                Self::Generic(GenericZeroTrustConfig {
                    command_template: command.unwrap_or_default().to_string(),
                })
            }
        }
    }
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

/// Hoop.dev zero-trust access gateway configuration
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoopDevConfig {
    /// Connection name identifier in Hoop.dev (passed as `hoop connect <connection_name>`)
    pub connection_name: String,
    /// Gateway API URL (optional, passed as `--api-url`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_url: Option<String>,
    /// gRPC server URL (optional, passed as `--grpc-url`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grpc_url: Option<String>,
}

/// Generic Zero Trust command configuration
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenericZeroTrustConfig {
    /// Full command template, executed verbatim through `sh -c`.
    ///
    /// Not processed for RustConn placeholders (`{host}`, `{user}`, `{port}`);
    /// shell syntax (variables, pipes, quoting) applies as-is.
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

/// Builds FreeRDP command-line arguments for RemoteApp (RAIL) mode.
///
/// This is a shared implementation used by both `rustconn_core::models::RdpConfig`
/// and `rustconn::embedded_rdp::types::RdpConfig` to avoid code duplication.
///
/// Returns an empty `Vec` if `program` is `None` or empty.
/// The returned args use FreeRDP 3.x syntax: `/app:`, `/app-cmd:`, `/app-name:`.
/// Values containing spaces are quoted for correct FreeRDP parsing.
#[must_use]
pub fn build_remote_app_freerdp_args(
    program: Option<&str>,
    cmd_args: Option<&str>,
    name: Option<&str>,
) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(program) = program
        && !program.is_empty()
    {
        // FreeRDP 3.x syntax: /app:program:<path>,cmd:<args>,name:<name>
        // FreeRDP 2.x used separate /app: /app-cmd: /app-name: arguments,
        // but 3.x uses a single /app: with comma-separated key:value pairs.
        let mut app_parts = vec![format!("program:{program}")];
        if let Some(cmd_args) = cmd_args
            && !cmd_args.is_empty()
        {
            app_parts.push(format!("cmd:{cmd_args}"));
        }
        if let Some(name) = name
            && !name.is_empty()
        {
            app_parts.push(format!("name:{name}"));
        }
        args.push(format!("/app:{}", app_parts.join(",")));
    }
    args
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

    // ====================================================================
    // HoopDev unit tests
    // ====================================================================

    #[test]
    fn test_hoop_dev_serde_rename() {
        let json = serde_json::to_string(&ZeroTrustProvider::HoopDev)
            .expect("serialize ZeroTrustProvider::HoopDev");
        assert!(
            json.contains("hoop_dev"),
            "HoopDev serde rename must be 'hoop_dev', got: {json}"
        );
    }

    #[test]
    fn test_hoop_dev_display_name() {
        assert_eq!(ZeroTrustProvider::HoopDev.display_name(), "Hoop.dev");
    }

    #[test]
    fn test_hoop_dev_cli_command() {
        assert_eq!(ZeroTrustProvider::HoopDev.cli_command(), "hoop");
    }

    #[test]
    fn test_hoop_dev_in_all() {
        let all = ZeroTrustProvider::all();
        let hoop_pos = all.iter().position(|p| *p == ZeroTrustProvider::HoopDev);
        let generic_pos = all.iter().position(|p| *p == ZeroTrustProvider::Generic);
        assert!(hoop_pos.is_some(), "HoopDev must be in all()");
        assert!(generic_pos.is_some(), "Generic must be in all()");
        assert!(
            hoop_pos.expect("checked") < generic_pos.expect("checked"),
            "HoopDev must appear before Generic in all()"
        );
    }

    #[test]
    fn test_hoop_dev_validate_empty_name() {
        let config = ZeroTrustConfig {
            provider: ZeroTrustProvider::HoopDev,
            provider_config: ZeroTrustProviderConfig::HoopDev(HoopDevConfig {
                connection_name: String::new(),
                gateway_url: None,
                grpc_url: None,
            }),
            custom_args: vec![],
        };
        assert!(
            config.validate().is_err(),
            "Empty connection_name must be rejected"
        );
    }

    #[test]
    fn test_hoop_dev_validate_whitespace_name() {
        let config = ZeroTrustConfig {
            provider: ZeroTrustProvider::HoopDev,
            provider_config: ZeroTrustProviderConfig::HoopDev(HoopDevConfig {
                connection_name: "   ".to_string(),
                gateway_url: None,
                grpc_url: None,
            }),
            custom_args: vec![],
        };
        assert!(
            config.validate().is_err(),
            "Whitespace-only connection_name must be rejected"
        );
    }

    #[test]
    fn test_hoop_dev_validate_valid() {
        let config = ZeroTrustConfig {
            provider: ZeroTrustProvider::HoopDev,
            provider_config: ZeroTrustProviderConfig::HoopDev(HoopDevConfig {
                connection_name: "my-database".to_string(),
                gateway_url: Some("https://app.hoop.dev".to_string()),
                grpc_url: Some("grpc.hoop.dev:8443".to_string()),
            }),
            custom_args: vec![],
        };
        assert!(
            config.validate().is_ok(),
            "Valid HoopDevConfig must pass validation"
        );
    }

    #[test]
    fn test_hoop_dev_build_command_basic() {
        let config = ZeroTrustConfig {
            provider: ZeroTrustProvider::HoopDev,
            provider_config: ZeroTrustProviderConfig::HoopDev(HoopDevConfig {
                connection_name: "my-db".to_string(),
                gateway_url: None,
                grpc_url: None,
            }),
            custom_args: vec![],
        };
        let (program, args) = config.build_command(None);
        assert_eq!(program, "hoop");
        assert_eq!(args, vec!["connect", "my-db"]);
    }

    #[test]
    fn test_hoop_dev_build_command_with_urls() {
        let config = ZeroTrustConfig {
            provider: ZeroTrustProvider::HoopDev,
            provider_config: ZeroTrustProviderConfig::HoopDev(HoopDevConfig {
                connection_name: "prod-server".to_string(),
                gateway_url: Some("https://app.hoop.dev".to_string()),
                grpc_url: Some("grpc.hoop.dev:8443".to_string()),
            }),
            custom_args: vec![],
        };
        let (program, args) = config.build_command(None);
        assert_eq!(program, "hoop");
        assert_eq!(
            args,
            vec![
                "connect",
                "prod-server",
                "--api-url",
                "https://app.hoop.dev",
                "--grpc-url",
                "grpc.hoop.dev:8443"
            ]
        );
    }

    #[test]
    fn test_hoop_dev_build_command_with_custom_args() {
        let config = ZeroTrustConfig {
            provider: ZeroTrustProvider::HoopDev,
            provider_config: ZeroTrustProviderConfig::HoopDev(HoopDevConfig {
                connection_name: "staging".to_string(),
                gateway_url: None,
                grpc_url: None,
            }),
            custom_args: vec!["--debug".to_string(), "--verbose".to_string()],
        };
        let (program, args) = config.build_command(None);
        assert_eq!(program, "hoop");
        assert_eq!(args, vec!["connect", "staging", "--debug", "--verbose"]);
    }
}

/// Browser mode selection for Web connections.
///
/// Determines how a Web connection URL is opened: embedded inside the tab,
/// in the system default browser, or via a custom command.
///
/// Every variant exists in every build, `web-embedded` or not. It is the feature
/// that decides whether `Embedded` can be *run*, not whether it can be
/// *represented* — and the difference is the whole point. `Embedded` used to be
/// `#[cfg]`-gated, so a build without the feature parsed
/// `browser_mode = "embedded"` as `System`; the connection then sat in memory as
/// `System`, and the next save of *any* connection — updating `last_connected`
/// on connect is enough — rewrote the file with `browser_mode = "system"` and
/// destroyed the user's choice for good, silently, at debug log level. That is
/// not a hypothetical mixed install: `rustconn-cli` takes `rustconn-core` with
/// `default-features = false` and never enables the feature, so a CLI built on
/// its own did it too, and every distribution build without WebKitGTK 6.0 does
/// it to a config written by a Flatpak build that has it. See
/// [`crate::protocol::web`] and the GUI's web connect path for where the
/// feature is honoured instead: at the point of use, where falling back to the
/// system browser costs the user nothing permanent.
///
/// Uses a manual `Deserialize` so an unrecognised mode from a newer release
/// falls back to `System` instead of failing the whole config parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WebBrowserMode {
    /// Embedded WebKitGTK 6.0 WebView inside the tab.
    ///
    /// Requires the `web-embedded` feature to actually open a WebView; builds
    /// without it fall back to the system browser at launch time and leave the
    /// stored value alone.
    Embedded,
    /// System default browser (xdg-open / UriLauncher)
    System,
    /// Custom browser command
    Custom,
}

impl<'de> serde::Deserialize<'de> for WebBrowserMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "embedded" => Ok(Self::Embedded),
            "system" => Ok(Self::System),
            "custom" => Ok(Self::Custom),
            other => {
                tracing::warn!(
                    browser_mode = other,
                    "Unknown browser_mode value; falling back to System"
                );
                Ok(Self::System)
            }
        }
    }
}

/// Unlike the enum itself, the *default* stays feature-conditional: a brand-new
/// Web connection created on a build that cannot open a WebView should not start
/// out asking for one. Nothing is lost either way — a default only applies where
/// no stored value exists.
///
/// This doc comment is also what keeps `clippy::derivable_impls` quiet, which is
/// why the `#[expect]` that used to sit here is gone: clippy skips a `Default`
/// impl that carries documentation, on the reasoning that a documented manual
/// impl is deliberate. Delete the comment and the lint comes back.
impl Default for WebBrowserMode {
    fn default() -> Self {
        #[cfg(feature = "web-embedded")]
        {
            Self::Embedded
        }
        #[cfg(not(feature = "web-embedded"))]
        {
            Self::System
        }
    }
}

/// Web connection configuration.
///
/// Configuration for web bookmark connections. These connections open a URL
/// in the user's default browser or an embedded WebView. Credentials
/// (username/password) are stored in the configured secret backend and can
/// be used for autofill in embedded mode.
///
/// `Serialize` is derived, but `Deserialize` is implemented manually to
/// validate `user_agent` length (max 512 chars) and clamp `zoom_level`
/// to the [0.3, 3.0] range at parse time.
#[expect(
    clippy::struct_excessive_bools,
    reason = "settings/flags struct mirrors persisted config 1:1; bools represent independent toggles, not a state machine"
)]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WebConfig {
    /// Custom browser command (None = system default via xdg-open / portal)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<String>,
    /// Open in private/incognito mode
    #[serde(default)]
    pub private_mode: bool,
    /// Browser mode: Embedded, System, or Custom
    pub browser_mode: WebBrowserMode,
    /// Whether JavaScript is enabled in the embedded WebView
    pub javascript_enabled: bool,
    /// Custom user agent string (None = WebKitGTK default, max 512 chars)
    pub user_agent: Option<String>,
    /// Persisted zoom level (1.0 = 100%, range 0.3–3.0)
    #[serde(default = "default_zoom")]
    pub zoom_level: f64,
    /// Accept invalid TLS certificates (self-signed, expired, wrong host).
    /// Useful for local services like Cockpit, Proxmox, or dev environments.
    #[serde(default)]
    pub accept_invalid_certs: bool,
    /// Remove the floating navigation toolbar and its reveal handle from the
    /// embedded browser (issue #260).
    ///
    /// See [`RdpConfig::hide_floating_toolbar`] for why this reads as "hide"
    /// rather than "show". Only the embedded browser has a toolbar to remove;
    /// the System and Custom modes hand the URL to another program.
    #[serde(default)]
    pub hide_floating_toolbar: bool,
}

// Manual Eq: zoom_level is always clamped to [0.3, 3.0] (finite, no NaN),
// so total equality is safe. Required because ProtocolConfig derives Eq.
impl Eq for WebConfig {}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            browser: None,
            private_mode: false,
            browser_mode: WebBrowserMode::default(),
            javascript_enabled: true,
            user_agent: None,
            zoom_level: 1.0,
            accept_invalid_certs: false,
            hide_floating_toolbar: false,
        }
    }
}

impl<'de> Deserialize<'de> for WebConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        /// Internal helper for raw deserialization before validation.
        #[expect(
            clippy::struct_excessive_bools,
            reason = "mirrors WebConfig field for field; the whole point is to be a 1:1 wire shape"
        )]
        #[derive(Deserialize)]
        struct WebConfigRaw {
            #[serde(default, skip_serializing_if = "Option::is_none")]
            browser: Option<String>,
            #[serde(default)]
            private_mode: bool,
            #[serde(default)]
            browser_mode: WebBrowserMode,
            #[serde(default = "default_true")]
            javascript_enabled: bool,
            #[serde(default)]
            user_agent: Option<String>,
            #[serde(default = "default_zoom")]
            zoom_level: f64,
            #[serde(default)]
            accept_invalid_certs: bool,
            #[serde(default)]
            hide_floating_toolbar: bool,
        }

        let raw = WebConfigRaw::deserialize(deserializer)?;

        // Validate user_agent length (max 512 Unicode characters)
        if let Some(ref ua) = raw.user_agent
            && ua.chars().count() > 512
        {
            return Err(serde::de::Error::custom(
                "user_agent exceeds maximum allowed length of 512 characters",
            ));
        }

        Ok(Self {
            browser: raw.browser,
            private_mode: raw.private_mode,
            browser_mode: raw.browser_mode,
            javascript_enabled: raw.javascript_enabled,
            user_agent: raw.user_agent,
            zoom_level: raw.zoom_level.clamp(0.3, 3.0),
            accept_invalid_certs: raw.accept_invalid_certs,
            hide_floating_toolbar: raw.hide_floating_toolbar,
        })
    }
}

#[cfg(test)]
mod web_browser_mode_tests {
    use super::*;

    /// The regression that made this file's `#[cfg]` on `Embedded` a bug: a build
    /// without `web-embedded` read `"embedded"` as `System`, and the next save of
    /// any connection wrote `"system"` back. The round trip has to hold in *both*
    /// feature configurations, which is why this test carries no `#[cfg]` of its
    /// own — that is the entire assertion.
    #[test]
    fn embedded_browser_mode_round_trips() {
        let config: WebConfig = toml::from_str("browser_mode = \"embedded\"\n")
            .expect("a stored embedded browser_mode must parse");
        assert_eq!(config.browser_mode, WebBrowserMode::Embedded);

        let rendered = toml::to_string(&config).expect("WebConfig must serialize");
        assert!(
            rendered.contains("browser_mode = \"embedded\""),
            "saving must not downgrade the stored mode, got:\n{rendered}"
        );
    }

    #[test]
    fn system_and_custom_browser_modes_round_trip() {
        for (stored, expected) in [
            ("system", WebBrowserMode::System),
            ("custom", WebBrowserMode::Custom),
        ] {
            let config: WebConfig = toml::from_str(&format!("browser_mode = \"{stored}\"\n"))
                .expect("a stored browser_mode must parse");
            assert_eq!(config.browser_mode, expected);
            let rendered = toml::to_string(&config).expect("WebConfig must serialize");
            assert!(rendered.contains(&format!("browser_mode = \"{stored}\"")));
        }
    }

    /// A mode written by a future release must not fail the whole config parse —
    /// one unknown value would otherwise take every connection in the file down
    /// with it.
    #[test]
    fn unknown_browser_mode_falls_back_to_system() {
        let config: WebConfig = toml::from_str("browser_mode = \"holographic\"\n")
            .expect("an unknown browser_mode must not fail the parse");
        assert_eq!(config.browser_mode, WebBrowserMode::System);
    }
}

#[cfg(test)]
mod erase_mode_tests {
    use super::*;

    /// A stored connection predating issue #271 has neither key in its config.
    /// The CHANGELOG promises those keep behaving exactly as before, which only
    /// holds while both fields deserialize to `Automatic`.
    #[test]
    fn ssh_config_without_erase_fields_deserializes_to_automatic() {
        let json = r#"{"auth_method":"public_key","host":"example.com"}"#;
        let config: SshConfig =
            serde_json::from_str(json).expect("legacy SSH JSON must still deserialize");
        assert_eq!(config.backspace_sends, BackspaceSends::Automatic);
        assert_eq!(config.delete_sends, DeleteSends::Automatic);
    }

    #[test]
    fn ssh_config_toml_without_erase_fields_deserializes_to_automatic() {
        let config: SshConfig = toml::from_str("auth_method = \"public_key\"\n")
            .expect("legacy SSH TOML must still deserialize");
        assert_eq!(config.backspace_sends, BackspaceSends::Automatic);
        assert_eq!(config.delete_sends, DeleteSends::Automatic);
    }

    #[test]
    fn mosh_config_without_erase_fields_deserializes_to_automatic() {
        let json = r#"{"ssh_port":2222,"predict_mode":"always"}"#;
        let config: MoshConfig =
            serde_json::from_str(json).expect("legacy MOSH JSON must still deserialize");
        assert_eq!(config.backspace_sends, BackspaceSends::Automatic);
        assert_eq!(config.delete_sends, DeleteSends::Automatic);
    }

    #[test]
    fn mosh_config_toml_without_erase_fields_deserializes_to_automatic() {
        let config: MoshConfig =
            toml::from_str("ssh_port = 2222\n").expect("legacy MOSH TOML must still deserialize");
        assert_eq!(config.backspace_sends, BackspaceSends::Automatic);
        assert_eq!(config.delete_sends, DeleteSends::Automatic);
    }

    /// The dropdown stores nothing but a row index, so `index()`/`from_index()`
    /// being inverse is what keeps the editor from saving a different value than
    /// the one on screen.
    #[test]
    fn backspace_sends_index_round_trips() {
        for mode in BackspaceSends::all() {
            assert_eq!(BackspaceSends::from_index(mode.index()), *mode);
        }
    }

    #[test]
    fn delete_sends_index_round_trips() {
        for mode in DeleteSends::all() {
            assert_eq!(DeleteSends::from_index(mode.index()), *mode);
        }
    }

    /// An index past the end of `all()` cannot come from the dropdown, so it can
    /// only mean the model and the UI disagree — fall back to the pre-#271
    /// behaviour rather than to an arbitrary variant.
    #[test]
    fn out_of_range_index_falls_back_to_automatic() {
        let past_end = u32::try_from(BackspaceSends::all().len()).unwrap_or(u32::MAX);
        assert_eq!(
            BackspaceSends::from_index(past_end),
            BackspaceSends::Automatic
        );
        assert_eq!(
            BackspaceSends::from_index(u32::MAX),
            BackspaceSends::Automatic
        );
        let past_end = u32::try_from(DeleteSends::all().len()).unwrap_or(u32::MAX);
        assert_eq!(DeleteSends::from_index(past_end), DeleteSends::Automatic);
        assert_eq!(DeleteSends::from_index(u32::MAX), DeleteSends::Automatic);
    }

    /// The labels are in the translation catalogue, so changing them silently
    /// orphans every existing `po` entry.
    #[test]
    fn erase_mode_display_names_are_stable() {
        assert_eq!(BackspaceSends::Automatic.display_name(), "Automatic (^?)");
        assert_eq!(BackspaceSends::Backspace.display_name(), "Backspace (^H)");
        assert_eq!(BackspaceSends::Delete.display_name(), "Delete (^?)");
        assert_eq!(DeleteSends::Automatic.display_name(), "Automatic (\\e[3~)");
        assert_eq!(DeleteSends::Backspace.display_name(), "Backspace (^H)");
        assert_eq!(DeleteSends::Delete.display_name(), "Delete (^?)");
    }

    #[test]
    fn erase_modes_reads_ssh_config() {
        let config = ProtocolConfig::Ssh(SshConfig {
            backspace_sends: BackspaceSends::Backspace,
            delete_sends: DeleteSends::Delete,
            ..SshConfig::default()
        });
        assert_eq!(
            config.erase_modes(),
            (BackspaceSends::Backspace, DeleteSends::Delete)
        );
    }

    #[test]
    fn erase_modes_reads_telnet_config() {
        let config = ProtocolConfig::Telnet(TelnetConfig {
            backspace_sends: BackspaceSends::Backspace,
            delete_sends: DeleteSends::Backspace,
            ..TelnetConfig::default()
        });
        assert_eq!(
            config.erase_modes(),
            (BackspaceSends::Backspace, DeleteSends::Backspace)
        );
    }

    #[test]
    fn erase_modes_reads_mosh_config() {
        let config = ProtocolConfig::Mosh(MoshConfig {
            backspace_sends: BackspaceSends::Delete,
            delete_sends: DeleteSends::Backspace,
            ..MoshConfig::default()
        });
        assert_eq!(
            config.erase_modes(),
            (BackspaceSends::Delete, DeleteSends::Backspace)
        );
    }

    /// A protocol without the setting still has to answer, so callers that
    /// re-apply the modes over a whole window always have something to install.
    #[test]
    fn erase_modes_falls_back_to_automatic_without_the_setting() {
        let defaults = (BackspaceSends::Automatic, DeleteSends::Automatic);
        assert_eq!(
            ProtocolConfig::Rdp(RdpConfig::default()).erase_modes(),
            defaults
        );
        assert_eq!(
            ProtocolConfig::Serial(SerialConfig::default()).erase_modes(),
            defaults
        );
        // SFTP shares SshConfig, so it carries the fields — but its session is a
        // file-manager tab that never applies them, and the editor hides the
        // choice accordingly.
        let sftp = ProtocolConfig::Sftp(SshConfig {
            backspace_sends: BackspaceSends::Backspace,
            delete_sends: DeleteSends::Backspace,
            ..SshConfig::default()
        });
        assert_eq!(sftp.erase_modes(), defaults);
    }
}

#[cfg(test)]
mod floating_toolbar_tests {
    use super::*;

    /// Every stored profile written before issue #260 lacks the key, and all of
    /// them must keep their toolbar. This is the whole reason the field reads as
    /// "hide" rather than "show".
    #[test]
    fn rdp_config_without_the_key_keeps_the_toolbar() {
        let json = r#"{"client_mode":"embedded","performance_mode":"balanced"}"#;
        let config: RdpConfig =
            serde_json::from_str(json).expect("legacy RDP JSON must still deserialize");
        assert!(!config.hide_floating_toolbar);

        let config: RdpConfig = toml::from_str("performance_mode = \"balanced\"\n")
            .expect("legacy RDP TOML must still deserialize");
        assert!(!config.hide_floating_toolbar);
    }

    #[test]
    fn vnc_config_without_the_key_keeps_the_toolbar() {
        let json = r#"{"client_mode":"embedded","view_only":false}"#;
        let config: VncConfig =
            serde_json::from_str(json).expect("legacy VNC JSON must still deserialize");
        assert!(!config.hide_floating_toolbar);

        let config: VncConfig =
            toml::from_str("view_only = false\n").expect("legacy VNC TOML must still deserialize");
        assert!(!config.hide_floating_toolbar);
    }

    /// `WebConfig` has a hand-written `Deserialize` going through `WebConfigRaw`,
    /// so a new field has to be threaded through four places and a miss here is
    /// silent rather than a compile error.
    #[test]
    fn web_config_without_the_key_keeps_the_toolbar() {
        let json = r#"{"browser_mode":"embedded","javascript_enabled":true}"#;
        let config: WebConfig =
            serde_json::from_str(json).expect("legacy Web JSON must still deserialize");
        assert!(!config.hide_floating_toolbar);

        let config: WebConfig = toml::from_str("javascript_enabled = true\n")
            .expect("legacy Web TOML must still deserialize");
        assert!(!config.hide_floating_toolbar);
    }

    /// The importers, templates, wizard and `sync::inventory` all build their
    /// protocol config from `Default`, so the derived default and the serde
    /// default have to agree — for this field they do, by construction.
    #[test]
    fn default_agrees_with_the_serde_default() {
        assert!(!RdpConfig::default().hide_floating_toolbar);
        assert!(!VncConfig::default().hide_floating_toolbar);
        assert!(!WebConfig::default().hide_floating_toolbar);
    }

    #[test]
    fn hidden_toolbar_survives_a_round_trip() {
        let rdp = RdpConfig {
            hide_floating_toolbar: true,
            ..RdpConfig::default()
        };
        let restored: RdpConfig =
            serde_json::from_str(&serde_json::to_string(&rdp).expect("RdpConfig must serialize"))
                .expect("RdpConfig must round-trip");
        assert!(restored.hide_floating_toolbar);

        let vnc = VncConfig {
            hide_floating_toolbar: true,
            ..VncConfig::default()
        };
        let restored: VncConfig =
            serde_json::from_str(&serde_json::to_string(&vnc).expect("VncConfig must serialize"))
                .expect("VncConfig must round-trip");
        assert!(restored.hide_floating_toolbar);

        let web = WebConfig {
            hide_floating_toolbar: true,
            ..WebConfig::default()
        };
        let restored: WebConfig =
            serde_json::from_str(&serde_json::to_string(&web).expect("WebConfig must serialize"))
                .expect("WebConfig must round-trip");
        assert!(restored.hide_floating_toolbar);
    }
}

#[cfg(test)]
mod default_consistency_tests {
    use super::*;

    /// `Default::default()` and "deserialise an empty object" are two answers to
    /// the same question, and every importer, template and wizard asks the first
    /// one while every stored profile answers the second. They diverged for
    /// `RdpConfig` and `VncConfig` until the hand-written `Default` impls above,
    /// because `#[derive(Default)]` cannot see `#[serde(default = "…")]`.
    ///
    /// Comparing the two whole structs rather than naming fields is the point:
    /// a field added later with a non-`Default` serde default fails here without
    /// anyone remembering to extend the test.
    #[test]
    fn default_agrees_with_serde() {
        let from_serde: RdpConfig =
            serde_json::from_str("{}").expect("RdpConfig must deserialise from an empty object");
        assert_eq!(RdpConfig::default(), from_serde);

        let from_serde: VncConfig =
            serde_json::from_str("{}").expect("VncConfig must deserialise from an empty object");
        assert_eq!(VncConfig::default(), from_serde);

        let from_serde: SpiceConfig =
            serde_json::from_str("{}").expect("SpiceConfig must deserialise from an empty object");
        assert_eq!(SpiceConfig::default(), from_serde);

        let from_serde: WebConfig =
            serde_json::from_str("{}").expect("WebConfig must deserialise from an empty object");
        assert_eq!(WebConfig::default(), from_serde);
    }

    /// The values a Remmina or RoyalTS import used to arrive with. Named
    /// separately from the struct comparison above so a regression says which
    /// behaviour broke rather than printing two thirty-field structs.
    #[test]
    fn imported_rdp_connection_keeps_its_usable_defaults() {
        let config = RdpConfig::default();
        assert!(config.clipboard_enabled, "clipboard was silently off");
        assert!(config.show_local_cursor, "local cursor was silently off");
        assert!(
            config.script_paste_via_clipboard,
            "scripts fell back to character-by-character autotype"
        );
        assert_eq!(
            config.autotype_delay_ms, 20,
            "a 0 ms inter-character delay drops characters on slow gateways"
        );
        assert_eq!(config.jiggler_interval_secs, 60);
        // The one this release added, stated negatively on purpose.
        assert!(!config.hide_floating_toolbar);
    }

    #[test]
    fn imported_vnc_connection_keeps_its_usable_defaults() {
        let config = VncConfig::default();
        assert!(config.scaling, "scaling to fit the window was silently off");
        assert!(config.clipboard_enabled, "clipboard was silently off");
        assert!(config.show_local_cursor, "local cursor was silently off");
        assert!(!config.hide_floating_toolbar);
    }
}

#[cfg(test)]
mod external_display_tests {
    use super::*;

    /// Every mode must produce at least one argument that decides the session
    /// size. A mode that emits nothing inherits FreeRDP's `1024x768` default,
    /// which is the shape of the bug this enum exists to fix.
    #[test]
    fn every_mode_sizes_the_session() {
        for mode in RdpDisplayMode::all() {
            let args = mode.freerdp_args(Some(&Resolution::new(1280, 1024)));
            assert!(
                !args.is_empty(),
                "{mode:?} left the session size to FreeRDP's default"
            );
        }
    }

    #[test]
    fn fit_to_screen_asks_for_the_whole_monitor() {
        // `/size:<p>%` with no `w`/`h` suffix applies to both dimensions.
        assert_eq!(
            RdpDisplayMode::FitScreen.freerdp_args(None),
            vec!["/size:100%".to_string()]
        );
    }

    #[test]
    fn custom_mode_uses_the_stored_resolution() {
        let args = RdpDisplayMode::Custom.freerdp_args(Some(&Resolution::new(3840, 2160)));
        assert_eq!(args, vec!["/w:3840".to_string(), "/h:2160".to_string()]);
    }

    /// A profile can carry `Custom` with no resolution — the CLI can write one,
    /// and so can a hand-edited `connections.toml`. Falling through to FreeRDP's
    /// own default would reintroduce a fixed small window.
    #[test]
    fn custom_mode_without_a_resolution_falls_back_to_the_screen() {
        assert_eq!(
            RdpDisplayMode::Custom.freerdp_args(None),
            vec!["/size:100%".to_string()]
        );
    }

    #[test]
    fn only_custom_mode_reads_a_stored_resolution() {
        for mode in RdpDisplayMode::all() {
            assert_eq!(
                mode.uses_stored_resolution(),
                *mode == RdpDisplayMode::Custom,
                "{mode:?} disagrees with its own freerdp_args"
            );
        }
    }

    /// The dropdown wraps these in `i18n()` at the call site, so the literals
    /// live here while the translation markers live in
    /// `rustconn/src/i18n_markers.rs` — `po/update-pot.sh` does not scan this
    /// crate. Renaming a label here without updating that file leaves the row
    /// untranslated in every locale, silently. Change them here first, then
    /// there.
    #[test]
    fn display_mode_labels_are_stable() {
        assert_eq!(RdpDisplayMode::FitScreen.display_name(), "Fit to screen");
        assert_eq!(RdpDisplayMode::Fullscreen.display_name(), "Fullscreen");
        assert_eq!(RdpDisplayMode::Custom.display_name(), "Custom resolution");
        assert_eq!(RdpDisplayMode::AllMonitors.display_name(), "All monitors");
    }

    #[test]
    fn display_mode_index_round_trips() {
        for mode in RdpDisplayMode::all() {
            assert_eq!(RdpDisplayMode::from_index(mode.index()), *mode);
        }
    }

    /// An out-of-range dropdown index must land on the default rather than on
    /// whichever variant happens to be first in the match.
    #[test]
    fn unknown_display_mode_index_is_the_default() {
        assert_eq!(RdpDisplayMode::from_index(99), RdpDisplayMode::default());
        assert_eq!(RdpDisplayMode::default(), RdpDisplayMode::FitScreen);
    }

    #[test]
    fn auto_scale_sends_no_dpi_override() {
        assert!(ScaleOverride::Auto.freerdp_scale_args(200).is_empty());
    }

    /// `Native` on an unscaled display resolves to 100%, which is already
    /// FreeRDP's default — saying it adds an argument that changes nothing.
    #[test]
    fn native_scale_on_an_unscaled_display_sends_nothing() {
        assert!(ScaleOverride::Native.freerdp_scale_args(100).is_empty());
    }

    #[test]
    fn native_scale_follows_the_compositor() {
        assert_eq!(
            ScaleOverride::Native.freerdp_scale_args(200),
            vec![
                "/scale-desktop:200".to_string(),
                "/scale-device:180".to_string()
            ]
        );
    }

    /// MS-RDPEDISP discards the desktop scale factor unless the device scale
    /// factor is exactly 100, 140 or 180, so the pair is always emitted together
    /// and the device value is always one of the three.
    #[test]
    fn device_scale_is_always_an_accepted_step() {
        for scale in ScaleOverride::all() {
            let args = scale.freerdp_scale_args(250);
            if args.is_empty() {
                continue;
            }
            let device = args
                .iter()
                .find_map(|arg| arg.strip_prefix("/scale-device:"))
                .expect("a desktop scale factor was sent without its device pair");
            let device: u16 = device.parse().expect("device scale must be numeric");
            assert!(
                FREERDP_DEVICE_SCALE_STEPS.contains(&device),
                "{scale:?} produced device scale {device}, which the server ignores"
            );
        }
    }

    /// The protocol caps the desktop scale factor at 500%; `Native` on an
    /// extreme compositor scale must be clamped rather than rejected wholesale.
    #[test]
    fn desktop_scale_is_clamped_to_the_protocol_ceiling() {
        let args = ScaleOverride::Native.freerdp_scale_args(900);
        assert!(
            args.contains(&format!("/scale-desktop:{FREERDP_MAX_SCALE_PERCENT}")),
            "expected the ceiling, got {args:?}"
        );
    }

    #[test]
    fn fixed_scale_steps_ignore_the_compositor() {
        // 125% is closer to the 140 step than to 100.
        assert_eq!(
            ScaleOverride::Scale125.freerdp_scale_args(100),
            vec![
                "/scale-desktop:125".to_string(),
                "/scale-device:140".to_string()
            ]
        );
    }
}

#[cfg(test)]
mod resolution_parse_tests {
    use super::*;

    #[test]
    fn parses_a_plain_resolution() {
        assert_eq!(
            Resolution::parse("2560x1440"),
            Some(Resolution::new(2560, 1440))
        );
    }

    #[test]
    fn accepts_an_uppercase_separator_and_surrounding_space() {
        assert_eq!(
            Resolution::parse(" 3840X2160 "),
            Some(Resolution::new(3840, 2160))
        );
    }

    /// A zero dimension is not a resolution any server can allocate, and it is
    /// what `"x1080"` and `"0x0"` would otherwise produce.
    #[test]
    fn rejects_values_that_are_not_two_positive_integers() {
        for bad in [
            "",
            "1920",
            "1920x",
            "x1080",
            "0x0",
            "1920x0",
            "-1x5",
            "1920*1080",
            "axb",
        ] {
            assert_eq!(Resolution::parse(bad), None, "{bad:?} was accepted");
        }
    }

    #[test]
    fn cli_display_mode_names_round_trip() {
        for mode in RdpDisplayMode::all() {
            assert_eq!(
                RdpDisplayMode::from_cli_name(mode.cli_name()),
                Some(*mode),
                "{mode:?} does not survive its own CLI name"
            );
        }
    }

    #[test]
    fn unknown_cli_display_mode_is_rejected() {
        assert_eq!(RdpDisplayMode::from_cli_name("fit-to-screen"), None);
    }
}

#[cfg(test)]
mod port_forward_socks_tests {
    use super::*;

    fn dynamic(local_port: u16) -> PortForward {
        PortForward {
            direction: PortForwardDirection::Dynamic,
            local_port,
            remote_host: String::new(),
            remote_port: 0,
        }
    }

    fn local(local_port: u16) -> PortForward {
        PortForward {
            direction: PortForwardDirection::Local,
            local_port,
            remote_host: "db".to_string(),
            remote_port: 5432,
        }
    }

    #[test]
    fn a_dynamic_forward_with_port_zero_is_a_random_socks_forward() {
        assert!(dynamic(0).is_random_dynamic());
        assert!(!dynamic(1080).is_random_dynamic());
        assert!(!local(0).is_random_dynamic());
    }

    #[test]
    fn display_summary_marks_an_auto_port() {
        assert_eq!(dynamic(0).display_summary(), "D auto (SOCKS)");
        assert_eq!(dynamic(1080).display_summary(), "D 1080 (SOCKS)");
    }

    #[test]
    fn assign_random_socks_ports_fills_only_the_zero_dynamic_forward() {
        let mut cfg = SshConfig {
            port_forwards: vec![local(5432), dynamic(0), dynamic(1080)],
            ..SshConfig::default()
        };
        assert!(cfg.has_random_socks_forward());

        // Deterministic picker standing in for find_free_port().
        let mut next = 40000u16;
        let assigned = cfg
            .assign_random_socks_ports::<std::convert::Infallible>(|| {
                let p = next;
                next += 1;
                Ok(p)
            })
            .unwrap();

        // The first (and here only zero) dynamic forward gets the first port.
        assert_eq!(assigned, Some(40000));
        // The explicit local and explicit dynamic ports are untouched.
        assert_eq!(cfg.port_forwards[0].local_port, 5432);
        assert_eq!(cfg.port_forwards[1].local_port, 40000);
        assert_eq!(cfg.port_forwards[2].local_port, 1080);
        // After assignment nothing is random anymore.
        assert!(!cfg.has_random_socks_forward());
    }

    #[test]
    fn assign_returns_none_when_there_is_no_random_forward() {
        let mut cfg = SshConfig {
            port_forwards: vec![local(5432), dynamic(1080)],
            ..SshConfig::default()
        };
        let assigned = cfg
            .assign_random_socks_ports::<std::convert::Infallible>(|| {
                unreachable!("no random forward")
            })
            .unwrap();
        assert_eq!(assigned, None);
    }

    #[test]
    fn socks_proxy_port_reports_a_concrete_port_only() {
        // Still the sentinel — no concrete port yet.
        let unresolved = SshConfig {
            port_forwards: vec![dynamic(0)],
            ..SshConfig::default()
        };
        assert_eq!(unresolved.socks_proxy_port(), None);

        // Explicit / already-assigned port is reported.
        let resolved = SshConfig {
            port_forwards: vec![dynamic(1080)],
            ..SshConfig::default()
        };
        assert_eq!(resolved.socks_proxy_port(), Some(1080));

        // No dynamic forward at all.
        let none = SshConfig {
            port_forwards: vec![local(5432)],
            ..SshConfig::default()
        };
        assert_eq!(none.socks_proxy_port(), None);
    }

    #[test]
    fn to_ssh_arg_renders_a_bare_d_flag_for_dynamic() {
        assert_eq!(dynamic(1080).to_ssh_arg(), vec!["-D", "1080"]);
    }
}
