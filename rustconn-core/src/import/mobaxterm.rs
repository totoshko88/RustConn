//! MobaXterm session file importer.
//!
//! Parses `.mxtsessions` files exported from MobaXterm.
//! Maps SSH, SFTP, RDP, VNC, Telnet, Serial, MOSH, and Browser session types
//! to RustConn connections; session types RustConn has no equivalent for
//! (Rsh/Rlogin, XDMCP, FTP, file browser, local shell, AWS S3, WSL) are
//! recorded as skipped entries so the user sees what was left behind.
//!
//! File format: INI-style with Windows-1252 encoding and CRLF line endings.
//! Sessions are stored as key=value pairs where the value contains
//! `#`-separated fields with `%`-separated sub-fields.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use super::traits::{ImportResult, ImportSource, ImportWarning, SkippedEntry};
use crate::error::ImportError;
use crate::models::{
    Connection, ConnectionGroup, MoshConfig, ProtocolConfig, RdpAudioMode, RdpConfig, Resolution,
    SerialBaudRate, SerialConfig, SerialDataBits, SerialFlowControl, SerialParity, SerialStopBits,
    SshAuthMethod, SshConfig, SshKeySource, TelnetConfig, VncConfig, WebConfig,
};

/// MobaXterm session type identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MobaSessionType {
    Ssh = 0,
    Telnet = 1,
    Rsh = 2,
    Xdmcp = 3,
    Rdp = 4,
    Vnc = 5,
    Ftp = 6,
    Sftp = 7,
    Serial = 8,
    File = 9,
    Shell = 10,
    Browser = 11,
    Mosh = 12,
    AwsS3 = 13,
    Wsl = 14,
}

impl MobaSessionType {
    fn from_id(id: u8) -> Option<Self> {
        match id {
            0 => Some(Self::Ssh),
            1 => Some(Self::Telnet),
            2 => Some(Self::Rsh),
            3 => Some(Self::Xdmcp),
            4 => Some(Self::Rdp),
            5 => Some(Self::Vnc),
            6 => Some(Self::Ftp),
            7 => Some(Self::Sftp),
            8 => Some(Self::Serial),
            9 => Some(Self::File),
            10 => Some(Self::Shell),
            11 => Some(Self::Browser),
            12 => Some(Self::Mosh),
            13 => Some(Self::AwsS3),
            14 => Some(Self::Wsl),
            _ => None,
        }
    }

    /// Human-readable MobaXterm name for this session type.
    ///
    /// Used in the `SkippedEntry` reason for the session types RustConn has no
    /// equivalent for, so the user sees which kind of session was left behind
    /// rather than losing it silently.
    const fn display_name(self) -> &'static str {
        match self {
            Self::Ssh => "SSH",
            Self::Telnet => "Telnet",
            Self::Rsh => "Rsh/Rlogin",
            Self::Xdmcp => "XDMCP",
            Self::Rdp => "RDP",
            Self::Vnc => "VNC",
            Self::Ftp => "FTP",
            Self::Sftp => "SFTP",
            Self::Serial => "Serial",
            Self::File => "File browser",
            Self::Shell => "local shell",
            Self::Browser => "Browser",
            Self::Mosh => "MOSH",
            Self::AwsS3 => "AWS S3",
            Self::Wsl => "WSL",
        }
    }
}

/// Importer for MobaXterm `.mxtsessions` files.
///
/// MobaXterm stores sessions in INI format with a custom encoding scheme.
/// Each session is a single line with fields separated by `#` and sub-fields by `%`.
pub struct MobaXtermImporter {
    /// Custom path to import from
    custom_path: Option<PathBuf>,
}

impl MobaXtermImporter {
    /// Creates a new MobaXterm importer.
    #[must_use]
    pub const fn new() -> Self {
        Self { custom_path: None }
    }

    /// Creates a new importer with a custom path.
    #[must_use]
    pub const fn with_path(path: PathBuf) -> Self {
        Self {
            custom_path: Some(path),
        }
    }

    /// Decodes MobaXterm escape sequences in a string.
    fn decode_escapes(s: &str) -> String {
        s.replace("__PERCENT__", "%")
            .replace("__DIEZE__", "#")
            .replace("__PTVIRG__", ";")
            .replace("__DBLQUO__", "\"")
            .replace("__PIPE__", "|")
            .replace("_CurrentDrive_", "C:")
            .replace("_ProfileDir_", "~")
    }

    /// Parses the content of a `.mxtsessions` file.
    #[must_use]
    pub fn parse_content(&self, content: &str, source_path: &str) -> ImportResult {
        let mut result = ImportResult::new();
        // Maps a fully-qualified folder path (e.g. `Production\Web`) to the
        // UUID of the `ConnectionGroup` representing its leaf segment.
        let mut groups: HashMap<String, Uuid> = HashMap::new();
        let mut current_section: Option<String> = None;
        let mut current_subrep: Option<String> = None;

        for line in content.lines() {
            let line = line.trim();

            // Skip empty lines
            if line.is_empty() {
                continue;
            }

            // Check for section header
            if line.starts_with('[') && line.ends_with(']') {
                current_section = Some(line[1..line.len() - 1].to_string());
                current_subrep = None;
                continue;
            }

            // Parse key=value
            let Some(eq_pos) = line.find('=') else {
                continue;
            };

            let key = line[..eq_pos].trim();
            let value = line[eq_pos + 1..].trim();

            // Handle folder metadata
            if key == "SubRep" {
                current_subrep = if value.is_empty() {
                    None
                } else {
                    Some(Self::decode_escapes(value))
                };
                continue;
            }

            if key == "ImgNum" {
                continue; // Skip icon number
            }

            // This is a session line
            if let Some(ref section) = current_section
                && section.starts_with("Bookmarks")
            {
                // Resolve the (possibly nested) folder path into a leaf group,
                // creating intermediate groups for each path segment as needed.
                let group_id = current_subrep
                    .as_deref()
                    .and_then(|path| Self::resolve_subrep_path(path, &mut groups, &mut result));

                // Parse session
                match self.parse_session(key, value, source_path) {
                    Ok(Some((mut connection, warning))) => {
                        connection.group_id = group_id;
                        if let Some(warning) = warning {
                            result.add_warning(warning);
                        }
                        result.add_connection(connection);
                    }
                    Ok(None) => {
                        // Unsupported session type, already logged
                    }
                    Err(reason) => {
                        result.add_skipped(SkippedEntry::with_location(key, reason, source_path));
                    }
                }
            }
        }

        result
    }

    /// Resolves a MobaXterm `SubRep` folder path into the UUID of its leaf group.
    ///
    /// MobaXterm encodes nested folders with backslash separators, e.g.
    /// `Production\Web`. Each path segment becomes a `ConnectionGroup`, chained
    /// via `parent_id` to reconstruct the original folder tree. Intermediate
    /// groups are created on demand and reused across sessions and sections.
    ///
    /// Returns `None` if the path is empty or contains only separators.
    fn resolve_subrep_path(
        path: &str,
        groups: &mut HashMap<String, Uuid>,
        result: &mut ImportResult,
    ) -> Option<Uuid> {
        let mut parent_id: Option<Uuid> = None;
        let mut accumulated = String::new();

        for segment in path.split('\\').map(str::trim).filter(|s| !s.is_empty()) {
            if !accumulated.is_empty() {
                accumulated.push('\\');
            }
            accumulated.push_str(segment);

            let group_id = *groups.entry(accumulated.clone()).or_insert_with(|| {
                let group = parent_id.map_or_else(
                    || ConnectionGroup::new(segment.to_string()),
                    |pid| ConnectionGroup::with_parent(segment.to_string(), pid),
                );
                let id = group.id;
                result.add_group(group);
                id
            });

            parent_id = Some(group_id);
        }

        parent_id
    }

    /// Parses a single session line.
    ///
    /// Returns the imported connection together with an optional
    /// [`ImportWarning`] the caller should surface (used for FTP, which is
    /// imported as SFTP because RustConn has no FTP client).
    fn parse_session(
        &self,
        name: &str,
        value: &str,
        source_path: &str,
    ) -> Result<Option<(Connection, Option<ImportWarning>)>, String> {
        let name = Self::decode_escapes(name);

        // Split by '#' to get main fields
        let fields: Vec<&str> = value.split('#').collect();
        if fields.len() < 3 {
            return Err("Invalid session format: too few fields".to_string());
        }

        // fields[0] = reconnect flag (";  logout" or empty)
        // fields[1] = icon number
        // fields[2] = connection parameters (%-separated)
        // fields[3] = terminal parameters (%-separated) - optional
        // fields[4] = start mode - optional
        // fields[5] = comment - optional
        // fields[6] = tab color - optional

        let conn_params: Vec<&str> = fields[2].split('%').collect();
        if conn_params.is_empty() {
            return Err("Invalid session format: no connection parameters".to_string());
        }

        // First param is session type
        let session_type_id: u8 = conn_params[0]
            .parse()
            .map_err(|_| format!("Invalid session type: {}", conn_params[0]))?;

        let Some(session_type) = MobaSessionType::from_id(session_type_id) else {
            return Err(format!("Unknown session type ID: {session_type_id}"));
        };

        // Every connection-producing branch pairs its result with an optional
        // warning; only FTP (imported as SFTP) carries one.
        let no_warning =
            |conn: Result<Option<Connection>, String>| conn.map(|c| c.map(|c| (c, None)));

        match session_type {
            MobaSessionType::Ssh => {
                no_warning(self.parse_ssh_session(&name, &conn_params, source_path, false))
            }
            // SFTP reuses the SSH field layout but must keep its own protocol
            // type so a round-trip through MobaXterm does not downgrade it to
            // SSH.
            MobaSessionType::Sftp => {
                no_warning(self.parse_ssh_session(&name, &conn_params, source_path, true))
            }
            MobaSessionType::Rdp => {
                no_warning(self.parse_rdp_session(&name, &conn_params, source_path))
            }
            MobaSessionType::Vnc => {
                no_warning(self.parse_vnc_session(&name, &conn_params, source_path))
            }
            MobaSessionType::Telnet => {
                no_warning(self.parse_telnet_session(&name, &conn_params, source_path))
            }
            // Rsh/Rlogin is a line protocol RustConn serves through its Telnet
            // client, the same mapping the mRemoteNG and PuTTY importers use.
            MobaSessionType::Rsh => {
                no_warning(self.parse_telnet_session(&name, &conn_params, source_path))
            }
            MobaSessionType::Serial => {
                no_warning(self.parse_serial_session(&name, &conn_params, source_path))
            }
            MobaSessionType::Mosh => {
                no_warning(self.parse_mosh_session(&name, &conn_params, source_path))
            }
            MobaSessionType::Browser => {
                no_warning(self.parse_browser_session(&name, &conn_params, source_path))
            }
            // FTP has no RustConn client; import it as SFTP and warn that the
            // transport, port and authentication differ.
            MobaSessionType::Ftp => self.parse_ftp_session(&name, &conn_params, source_path),
            // RustConn has no equivalent for these session types. Record them
            // as skipped so the user sees what was left behind instead of
            // losing them silently.
            MobaSessionType::Xdmcp
            | MobaSessionType::File
            | MobaSessionType::Shell
            | MobaSessionType::AwsS3
            | MobaSessionType::Wsl => Err(format!(
                "{} sessions are not supported by RustConn",
                session_type.display_name()
            )),
        }
    }

    /// Parses an SSH/SFTP session.
    ///
    /// `is_sftp` selects the protocol variant: SFTP (session type 7) and SSH
    /// (type 0) share the exact same MobaXterm field layout and both map onto
    /// an [`SshConfig`], so only the wrapping `ProtocolConfig` variant differs.
    /// Keeping them distinct lets an SFTP session survive a round-trip through
    /// MobaXterm instead of being downgraded to SSH.
    #[expect(
        clippy::unused_self,
        reason = "method is part of a uniform helper API where most operations need &self; keeping &self preserves the consistent signature"
    )]
    fn parse_ssh_session(
        &self,
        name: &str,
        params: &[&str],
        _source_path: &str,
        is_sftp: bool,
    ) -> Result<Option<Connection>, String> {
        // SSH params layout:
        // [0] = session type (0 for SSH, 7 for SFTP)
        // [1] = host
        // [2] = port
        // [3] = username
        // [5] = X11 forwarding (-1=on, 0=off)
        // [6] = compression (-1=on, 0=off)
        // [7] = startup command
        // [14] = private key path
        // [33] = SSH agent (-1=on, 0=off)
        // [34] = agent forwarding (-1=on, 0=off)

        let host = params
            .get(1)
            .map(|s| Self::decode_escapes(s))
            .unwrap_or_default();
        if host.is_empty() {
            return Err("No host specified".to_string());
        }

        let port: u16 = params.get(2).and_then(|s| s.parse().ok()).unwrap_or(22);

        let username = params
            .get(3)
            .filter(|s| !s.is_empty() && **s != "<default>")
            .map(|s| Self::decode_escapes(s));

        // Parse SSH-specific options
        let key_path = params
            .get(14)
            .filter(|s| !s.is_empty())
            .map(|s| PathBuf::from(Self::decode_escapes(s)));

        let use_agent = params.get(33).map(|s| *s == "-1").unwrap_or(false);

        let agent_forwarding = params.get(34).map(|s| *s == "-1").unwrap_or(false);

        let startup_command = params
            .get(7)
            .filter(|s| !s.is_empty())
            .map(|s| Self::decode_escapes(s));

        // Determine auth method
        let auth_method = if key_path.is_some() {
            SshAuthMethod::PublicKey
        } else if use_agent {
            SshAuthMethod::Agent
        } else {
            SshAuthMethod::Password
        };

        // Parse X11 forwarding and compression from MobaXterm params
        let x11_forwarding = params.get(5).map(|s| *s == "-1").unwrap_or(false);
        let compression = params.get(6).map(|s| *s == "-1").unwrap_or(false);

        let ssh_config = SshConfig {
            auth_method,
            key_path,
            key_source: SshKeySource::Default,
            agent_key_fingerprint: None,
            identities_only: false,
            jump_host_id: None,
            proxy_jump: None,
            proxy_command: None,
            pkcs11_provider: None,
            use_control_master: false,
            agent_forwarding,
            x11_forwarding,
            compression,
            custom_options: HashMap::new(),
            startup_command,
            port_forwards: Vec::new(),
            waypipe: false,
            ssh_agent_socket: None,
            keep_alive_interval: None,
            keep_alive_count_max: None,
            verbose: false,
            mptcp: false,
            remote_path: None,
            // MobaXterm has no equivalent erase-key setting to carry over.
            ..Default::default()
        };

        let protocol_config = if is_sftp {
            ProtocolConfig::Sftp(ssh_config)
        } else {
            ProtocolConfig::Ssh(ssh_config)
        };

        let mut connection = Connection::new(name.to_string(), host, port, protocol_config);
        connection.username = username;

        Ok(Some(connection))
    }

    /// Parses an RDP session.
    #[expect(
        clippy::unused_self,
        reason = "method is part of a uniform helper API where most operations need &self; keeping &self preserves the consistent signature"
    )]
    fn parse_rdp_session(
        &self,
        name: &str,
        params: &[&str],
        _source_path: &str,
    ) -> Result<Option<Connection>, String> {
        // RDP params layout:
        // [0] = session type (4)
        // [1] = host
        // [2] = port
        // [3] = username
        // [10] = resolution (0=Fit, 1=Screen, 2=640x480, ..., 11=1920x1080, ...)
        // [16] = audio (0=none, 1=redirect, 2=remote)
        // [19] = clipboard (-1=on, 0=off)
        // [28] = color depth (0=auto, 1=8bit, 2=16bit, 3=24bit, 4=32bit)

        let host = params
            .get(1)
            .map(|s| Self::decode_escapes(s))
            .unwrap_or_default();
        if host.is_empty() {
            return Err("No host specified".to_string());
        }

        let port: u16 = params.get(2).and_then(|s| s.parse().ok()).unwrap_or(3389);

        let username = params
            .get(3)
            .filter(|s| !s.is_empty())
            .map(|s| Self::decode_escapes(s));

        // Parse resolution
        let resolution = params.get(10).and_then(|s| {
            match *s {
                "2" => Some(Resolution::new(640, 480)),
                "3" => Some(Resolution::new(800, 600)),
                "4" => Some(Resolution::new(1024, 768)),
                "5" => Some(Resolution::new(1152, 864)),
                "6" => Some(Resolution::new(1280, 720)),
                "7" => Some(Resolution::new(1280, 968)),
                "8" => Some(Resolution::new(1280, 1024)),
                "9" => Some(Resolution::new(1400, 1050)),
                "10" => Some(Resolution::new(1600, 1200)),
                "11" => Some(Resolution::new(1920, 1080)),
                "14" => Some(Resolution::new(1920, 1200)),
                "24" => Some(Resolution::new(2560, 1440)),
                "26" => Some(Resolution::new(3840, 2160)),
                _ => None, // Fit to terminal/screen - no fixed resolution
            }
        });

        // Parse color depth
        let color_depth = params.get(28).and_then(|s| {
            match *s {
                "1" => Some(8),
                "2" => Some(16),
                "3" => Some(24),
                "4" => Some(32),
                _ => None, // Auto
            }
        });

        let mut rdp_config = RdpConfig {
            resolution,
            color_depth,
            ..Default::default()
        };
        // Audio, field 16 (see the layout above). Only "1" used to be read, so
        // "2" — audio left on the remote computer — was imported as none.
        if let Some(audio) = params.get(16) {
            rdp_config.set_audio_mode(match *audio {
                "1" => RdpAudioMode::Local,
                "2" => RdpAudioMode::Remote,
                _ => RdpAudioMode::None,
            });
        }

        let mut connection = Connection::new(
            name.to_string(),
            host,
            port,
            ProtocolConfig::Rdp(rdp_config),
        );
        connection.username = username;

        Ok(Some(connection))
    }

    /// Parses a Telnet session.
    #[expect(
        clippy::unused_self,
        reason = "method is part of a uniform helper API where most operations need &self; keeping &self preserves the consistent signature"
    )]
    fn parse_telnet_session(
        &self,
        name: &str,
        params: &[&str],
        _source_path: &str,
    ) -> Result<Option<Connection>, String> {
        // Telnet params layout:
        // [0] = session type (1)
        // [1] = host
        // [2] = port

        let host = params
            .get(1)
            .map(|s| Self::decode_escapes(s))
            .unwrap_or_default();
        if host.is_empty() {
            return Err("No host specified".to_string());
        }

        let port: u16 = params.get(2).and_then(|s| s.parse().ok()).unwrap_or(23);

        let connection = Connection::new(
            name.to_string(),
            host,
            port,
            ProtocolConfig::Telnet(TelnetConfig::default()),
        );

        Ok(Some(connection))
    }

    /// Parses a VNC session.
    #[expect(
        clippy::unused_self,
        reason = "method is part of a uniform helper API where most operations need &self; keeping &self preserves the consistent signature"
    )]
    fn parse_vnc_session(
        &self,
        name: &str,
        params: &[&str],
        _source_path: &str,
    ) -> Result<Option<Connection>, String> {
        // VNC params layout:
        // [0] = session type (5)
        // [1] = host
        // [2] = port
        // [3] = auto scale (-1=on, 0=off)
        // [4] = view only (-1=on, 0=off)

        let host = params
            .get(1)
            .map(|s| Self::decode_escapes(s))
            .unwrap_or_default();
        if host.is_empty() {
            return Err("No host specified".to_string());
        }

        let port: u16 = params.get(2).and_then(|s| s.parse().ok()).unwrap_or(5900);

        // Parse VNC options
        let view_only = params.get(4).map(|s| *s == "-1").unwrap_or(false);

        let vnc_config = VncConfig {
            view_only,
            ..Default::default()
        };

        let connection = Connection::new(
            name.to_string(),
            host,
            port,
            ProtocolConfig::Vnc(vnc_config),
        );

        Ok(Some(connection))
    }

    /// Parses a Serial session.
    ///
    /// MobaXterm stores a Windows COM port (e.g. `COM3`) and a baud rate. The
    /// COM name has no Linux equivalent, so it is carried into the device field
    /// verbatim for the user to correct (to a `/dev/tty*` path); the baud rate,
    /// data bits, stop bits, parity and flow control map directly where the
    /// values are among the ones RustConn supports.
    #[expect(
        clippy::unused_self,
        reason = "method is part of a uniform helper API where most operations need &self; keeping &self preserves the consistent signature"
    )]
    fn parse_serial_session(
        &self,
        name: &str,
        params: &[&str],
        _source_path: &str,
    ) -> Result<Option<Connection>, String> {
        // Serial params layout:
        // [0] = session type (8)
        // [1] = COM port (e.g. "COM3")
        // [2] = baud rate (e.g. "115200")
        // [3] = data bits (e.g. "8")
        // [4] = parity ("0"=none, "1"=odd, "2"=even, "3"=mark, "4"=space)
        // [5] = stop bits ("0"=1, "1"=1.5, "2"=2)
        // [6] = flow control ("0"=none, "1"=RTS/CTS, "2"=XON/XOFF)

        let device = params
            .get(1)
            .map(|s| Self::decode_escapes(s))
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "No serial port specified".to_string())?;

        let baud_rate = params
            .get(2)
            .and_then(|s| s.parse::<u32>().ok())
            .map_or_else(SerialBaudRate::default, |rate| match rate {
                9600 => SerialBaudRate::B9600,
                19200 => SerialBaudRate::B19200,
                38400 => SerialBaudRate::B38400,
                57600 => SerialBaudRate::B57600,
                230_400 => SerialBaudRate::B230400,
                460_800 => SerialBaudRate::B460800,
                921_600 => SerialBaudRate::B921600,
                // 115200 and any rate RustConn does not list fall back to the
                // default (115200), the most common serial-console speed.
                _ => SerialBaudRate::B115200,
            });

        let data_bits = params.get(3).and_then(|s| match *s {
            "5" => Some(SerialDataBits::Five),
            "6" => Some(SerialDataBits::Six),
            "7" => Some(SerialDataBits::Seven),
            "8" => Some(SerialDataBits::Eight),
            _ => None,
        });

        let parity = params.get(4).and_then(|s| match *s {
            "0" => Some(SerialParity::None),
            "1" => Some(SerialParity::Odd),
            "2" => Some(SerialParity::Even),
            _ => None, // mark/space have no RustConn equivalent
        });

        let stop_bits = params.get(5).and_then(|s| match *s {
            "0" => Some(SerialStopBits::One),
            "2" => Some(SerialStopBits::Two),
            _ => None, // 1.5 stop bits has no RustConn equivalent
        });

        let flow_control = params.get(6).and_then(|s| match *s {
            "0" => Some(SerialFlowControl::None),
            "1" => Some(SerialFlowControl::Hardware),
            "2" => Some(SerialFlowControl::Software),
            _ => None,
        });

        let serial_config = SerialConfig {
            device,
            baud_rate,
            data_bits: data_bits.unwrap_or_default(),
            stop_bits: stop_bits.unwrap_or_default(),
            parity: parity.unwrap_or_default(),
            flow_control: flow_control.unwrap_or_default(),
            custom_args: Vec::new(),
        };

        // Serial has no host/port; store a placeholder host and port 0 like the
        // native serial-connection path does.
        let connection = Connection::new(
            name.to_string(),
            String::new(),
            0,
            ProtocolConfig::Serial(serial_config),
        );

        Ok(Some(connection))
    }

    /// Parses a MOSH session.
    ///
    /// MOSH shares the SSH field layout in MobaXterm: it connects over SSH for
    /// the initial handshake, so host, SSH port and username are read the same
    /// way as an SSH session.
    #[expect(
        clippy::unused_self,
        reason = "method is part of a uniform helper API where most operations need &self; keeping &self preserves the consistent signature"
    )]
    fn parse_mosh_session(
        &self,
        name: &str,
        params: &[&str],
        _source_path: &str,
    ) -> Result<Option<Connection>, String> {
        // MOSH params layout (same shape as SSH):
        // [0] = session type (12)
        // [1] = host
        // [2] = SSH port
        // [3] = username

        let host = params
            .get(1)
            .map(|s| Self::decode_escapes(s))
            .unwrap_or_default();
        if host.is_empty() {
            return Err("No host specified".to_string());
        }

        let port: u16 = params.get(2).and_then(|s| s.parse().ok()).unwrap_or(22);

        let username = params
            .get(3)
            .filter(|s| !s.is_empty() && **s != "<default>")
            .map(|s| Self::decode_escapes(s));

        let mosh_config = MoshConfig {
            ssh_port: (port != 22).then_some(port),
            ..Default::default()
        };

        let mut connection = Connection::new(
            name.to_string(),
            host,
            port,
            ProtocolConfig::Mosh(mosh_config),
        );
        connection.username = username;

        Ok(Some(connection))
    }

    /// Parses a Browser session.
    ///
    /// MobaXterm stores the target URL as the session's host field. RustConn
    /// opens it as a Web bookmark with its default browser mode (the embedded
    /// WebKit browser where available).
    #[expect(
        clippy::unused_self,
        reason = "method is part of a uniform helper API where most operations need &self; keeping &self preserves the consistent signature"
    )]
    fn parse_browser_session(
        &self,
        name: &str,
        params: &[&str],
        _source_path: &str,
    ) -> Result<Option<Connection>, String> {
        // Browser params layout:
        // [0] = session type (11)
        // [1] = URL / remote hostname

        let url = params
            .get(1)
            .map(|s| Self::decode_escapes(s))
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "No URL specified".to_string())?;

        // A Web connection's URL lives in the host field, as the mRemoteNG and
        // CSV importers also store it. The port is cosmetic for Web; use 443
        // for an https URL, 80 otherwise.
        let port = if url.trim_start().to_ascii_lowercase().starts_with("https") {
            443
        } else {
            80
        };

        let connection = Connection::new(
            name.to_string(),
            url,
            port,
            ProtocolConfig::Web(WebConfig::default()),
        );

        Ok(Some(connection))
    }

    /// Parses an FTP session.
    ///
    /// RustConn has no plain-FTP client, so an FTP session is imported as SFTP
    /// — the closest equivalent — and paired with an [`ImportWarning`] so the
    /// user knows the transport, port and authentication differ. The host and
    /// username are carried over; the port is kept as MobaXterm stored it (FTP
    /// default 21), which the user must change to an SFTP port before
    /// connecting.
    #[expect(
        clippy::unused_self,
        reason = "method is part of a uniform helper API where most operations need &self; keeping &self preserves the consistent signature"
    )]
    fn parse_ftp_session(
        &self,
        name: &str,
        params: &[&str],
        _source_path: &str,
    ) -> Result<Option<(Connection, Option<ImportWarning>)>, String> {
        // FTP params layout:
        // [0] = session type (6)
        // [1] = host
        // [2] = port
        // [3] = username

        let host = params
            .get(1)
            .map(|s| Self::decode_escapes(s))
            .unwrap_or_default();
        if host.is_empty() {
            return Err("No host specified".to_string());
        }

        let port: u16 = params.get(2).and_then(|s| s.parse().ok()).unwrap_or(21);

        let username = params
            .get(3)
            .filter(|s| !s.is_empty() && **s != "<default>")
            .map(|s| Self::decode_escapes(s));

        let mut connection = Connection::new(
            name.to_string(),
            host,
            port,
            // SFTP reuses the SSH config shape.
            ProtocolConfig::Sftp(SshConfig::default()),
        );
        connection.username = username;

        let warning = ImportWarning::FtpImportedAsSftp {
            connection_name: name.to_string(),
        };

        Ok(Some((connection, Some(warning))))
    }
}

impl Default for MobaXtermImporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportSource for MobaXtermImporter {
    fn source_id(&self) -> &'static str {
        "mobaxterm"
    }

    fn display_name(&self) -> &'static str {
        "MobaXterm"
    }

    fn is_available(&self) -> bool {
        self.custom_path.as_ref().is_some_and(|p| p.exists())
    }

    fn default_paths(&self) -> Vec<PathBuf> {
        self.custom_path
            .as_ref()
            .map(|p| vec![p.clone()])
            .unwrap_or_default()
    }

    fn import(&self) -> Result<ImportResult, ImportError> {
        let Some(ref path) = self.custom_path else {
            return Err(ImportError::FileNotFound(PathBuf::from(
                "No MobaXterm session file specified",
            )));
        };

        self.import_from_path(path)
    }

    fn import_from_path(&self, path: &Path) -> Result<ImportResult, ImportError> {
        // Read file as bytes first to handle Windows-1252 encoding
        let bytes = fs::read(path).map_err(|e| ImportError::ParseError {
            source_name: "MobaXterm".to_string(),
            reason: format!("Failed to read {}: {}", path.display(), e),
        })?;

        // Try UTF-8 first, fall back to lossy conversion for Windows-1252
        let content = match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => {
                let bytes = e.into_bytes();
                String::from_utf8_lossy(&bytes).into_owned()
            }
        };

        Ok(self.parse_content(&content, &path.display().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_escapes() {
        assert_eq!(
            MobaXtermImporter::decode_escapes("test__PERCENT__value"),
            "test%value"
        );
        assert_eq!(
            MobaXtermImporter::decode_escapes("test__DIEZE__value"),
            "test#value"
        );
        assert_eq!(
            MobaXtermImporter::decode_escapes("test__PIPE__value"),
            "test|value"
        );
        assert_eq!(
            MobaXtermImporter::decode_escapes("_CurrentDrive_\\path"),
            "C:\\path"
        );
    }

    #[test]
    fn test_parse_ssh_session() {
        let importer = MobaXtermImporter::new();
        let content = "[Bookmarks]
SubRep=
ImgNum=42
My SSH Server=#109#0%192.168.1.100%22%admin%%-1%-1%%%%%0%0%0%%%-1%0%0%0%%1080%%0%0%1%#MobaFont%10%0%0%-1%15%236,236,236%30,30,30%180,180,192%0%-1%0%%xterm%-1%0%_Std_Colors_0_%80%24%0%0%-1%<none>%%0%0%-1%-1#0# #-1";

        let result = importer.parse_content(content, "test.mxtsessions");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert_eq!(conn.name, "My SSH Server");
        assert_eq!(conn.host, "192.168.1.100");
        assert_eq!(conn.port, 22);
        assert_eq!(conn.username, Some("admin".to_string()));
        assert!(matches!(conn.protocol_config, ProtocolConfig::Ssh(_)));
    }

    #[test]
    fn test_parse_sftp_session_keeps_sftp_type() {
        let importer = MobaXtermImporter::new();
        // Session type 7 = SFTP, same field layout as SSH (type 0). It must be
        // imported as SFTP, not downgraded to SSH.
        let content = "[Bookmarks]
SubRep=
ImgNum=140
My SFTP=#140#7%192.168.1.200%22%admin%%-1%-1%%%%%0%0%0%%%-1%0%0%0%%1080%%0%0%1%#MobaFont%10#0# #-1";

        let result = importer.parse_content(content, "test.mxtsessions");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert_eq!(conn.name, "My SFTP");
        assert_eq!(conn.host, "192.168.1.200");
        assert_eq!(conn.port, 22);
        assert_eq!(conn.username, Some("admin".to_string()));
        assert!(
            matches!(conn.protocol_config, ProtocolConfig::Sftp(_)),
            "MobaXterm session type 7 must import as SFTP"
        );
    }

    #[test]
    fn test_parse_rdp_session() {
        let importer = MobaXtermImporter::new();
        // RDP session with resolution at index 10 (value "11" = 1920x1080) and color depth at index 28 (value "4" = 32bit)
        // Format: type%host%port%user%...%resolution(10)%...%audio(16)%...%clipboard(19)%...%colordepth(28)%...
        let content = "[Bookmarks]
SubRep=
ImgNum=42
Windows Server=#91#4%192.168.1.50%3389%Administrator%0%0%0%0%-1%0%11%-1%%%%%-1%0%0%0%0%0%0%0%0%0%0%0%4%0%0%0%#MobaFont%10#0# #-1";

        let result = importer.parse_content(content, "test.mxtsessions");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert_eq!(conn.name, "Windows Server");
        assert_eq!(conn.host, "192.168.1.50");
        assert_eq!(conn.port, 3389);
        assert!(matches!(conn.protocol_config, ProtocolConfig::Rdp(_)));

        if let ProtocolConfig::Rdp(rdp_config) = &conn.protocol_config {
            // Resolution 11 = 1920x1080
            assert!(rdp_config.resolution.is_some());
            let res = rdp_config.resolution.as_ref().unwrap();
            assert_eq!(res.width, 1920);
            assert_eq!(res.height, 1080);
            // Color depth 4 = 32bit
            assert_eq!(rdp_config.color_depth, Some(32));
        }
    }

    #[test]
    fn test_parse_vnc_session() {
        let importer = MobaXtermImporter::new();
        let content = "[Bookmarks]
SubRep=
ImgNum=42
VNC Desktop=#128#5%192.168.1.75%5901%-1%0%%%%%0%0%0%%%%%#MobaFont%10%0%0%-1%15%236,236,236%30,30,30%180,180,192%0%-1%0%%xterm%-1%0%_Std_Colors_0_%80%24%0%0%-1%<none>%%0%0%-1%-1#0# #-1";

        let result = importer.parse_content(content, "test.mxtsessions");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert_eq!(conn.name, "VNC Desktop");
        assert_eq!(conn.host, "192.168.1.75");
        assert_eq!(conn.port, 5901);
        assert!(matches!(conn.protocol_config, ProtocolConfig::Vnc(_)));
    }

    #[test]
    fn test_parse_with_folders() {
        let importer = MobaXtermImporter::new();
        let content = "[Bookmarks]
SubRep=
ImgNum=42
Root Server=#109#0%10.0.0.1%22%root%%-1%-1%%%%%0%0%0%%%-1%0%0%0%%1080%%0%0%1%#MobaFont%10#0# #-1
[Bookmarks_1]
SubRep=Production
ImgNum=41
Prod Server=#109#0%10.0.1.1%22%admin%%-1%-1%%%%%0%0%0%%%-1%0%0%0%%1080%%0%0%1%#MobaFont%10#0# #-1
[Bookmarks_2]
SubRep=Production\\Web
ImgNum=41
Web Server=#109#0%10.0.1.2%22%www%%-1%-1%%%%%0%0%0%%%-1%0%0%0%%1080%%0%0%1%#MobaFont%10#0# #-1";

        let result = importer.parse_content(content, "test.mxtsessions");
        assert_eq!(result.connections.len(), 3);
        assert_eq!(result.groups.len(), 2);

        // Nested SubRep `Production\Web` must form a tree, not a flat group
        // named literally "Production\Web".
        let production = result
            .groups
            .iter()
            .find(|g| g.name == "Production")
            .expect("Production group should exist");
        assert!(production.parent_id.is_none(), "Production is a root group");

        let web = result
            .groups
            .iter()
            .find(|g| g.name == "Web")
            .expect("Web group should exist as a nested folder");
        assert_eq!(
            web.parent_id,
            Some(production.id),
            "Web should be nested under Production"
        );

        // Check root connection has no group
        let root_conn = result
            .connections
            .iter()
            .find(|c| c.name == "Root Server")
            .unwrap();
        assert!(root_conn.group_id.is_none());

        // Check Production connection has group
        let prod_conn = result
            .connections
            .iter()
            .find(|c| c.name == "Prod Server")
            .unwrap();
        assert_eq!(prod_conn.group_id, Some(production.id));

        // Check the deeply nested connection lands in the leaf group
        let web_conn = result
            .connections
            .iter()
            .find(|c| c.name == "Web Server")
            .unwrap();
        assert_eq!(web_conn.group_id, Some(web.id));
    }

    #[test]
    fn test_skip_unsupported_session_type() {
        let importer = MobaXtermImporter::new();
        // XDMCP session (type 3) - no RustConn equivalent
        let content = "[Bookmarks]
SubRep=
ImgNum=42
X Desktop=#88#3%192.168.1.100%177%%%#MobaFont%10#0# #-1";

        let result = importer.parse_content(content, "test.mxtsessions");
        assert_eq!(result.connections.len(), 0);
        // RustConn has no XDMCP equivalent, so the session is recorded as
        // skipped rather than lost silently.
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(result.skipped[0].identifier, "X Desktop");
        assert!(
            result.skipped[0].reason.contains("XDMCP"),
            "skip reason should name the MobaXterm session type, got: {}",
            result.skipped[0].reason
        );
    }

    #[test]
    fn test_parse_rsh_session_maps_to_telnet() {
        let importer = MobaXtermImporter::new();
        // Rsh/Rlogin (type 2) is served through RustConn's Telnet client, like
        // the mRemoteNG and PuTTY importers do.
        let content = "[Bookmarks]
SubRep=
ImgNum=42
Rlogin Host=#100#2%legacy.example.com%513%%%#MobaFont%10#0# #-1";

        let result = importer.parse_content(content, "test.mxtsessions");
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.skipped.len(), 0);

        let conn = &result.connections[0];
        assert_eq!(conn.name, "Rlogin Host");
        assert_eq!(conn.host, "legacy.example.com");
        assert_eq!(conn.port, 513);
        assert!(matches!(conn.protocol_config, ProtocolConfig::Telnet(_)));
    }

    #[test]
    fn test_parse_ftp_session_maps_to_sftp_with_warning() {
        let importer = MobaXtermImporter::new();
        // FTP (type 6) has no RustConn client; it is imported as SFTP and the
        // result carries a warning that the transport/port/auth differ.
        let content = "[Bookmarks]
SubRep=
ImgNum=42
File Server=#130#6%ftp.example.com%21%ftpuser%%#MobaFont%10#0# #-1";

        let result = importer.parse_content(content, "test.mxtsessions");
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.skipped.len(), 0);

        let conn = &result.connections[0];
        assert_eq!(conn.name, "File Server");
        assert_eq!(conn.host, "ftp.example.com");
        // The FTP port is kept as stored; the user retargets it to SFTP.
        assert_eq!(conn.port, 21);
        assert_eq!(conn.username, Some("ftpuser".to_string()));
        assert!(matches!(conn.protocol_config, ProtocolConfig::Sftp(_)));

        assert_eq!(result.warnings.len(), 1);
        assert!(matches!(
            result.warnings[0],
            ImportWarning::FtpImportedAsSftp { .. }
        ));
    }

    #[test]
    fn test_parse_serial_session() {
        let importer = MobaXtermImporter::new();
        // Serial params: type(8) % port % baud % data % parity % stop % flow
        let content = "[Bookmarks]
SubRep=
ImgNum=42
Console=#131#8%COM3%9600%8%0%0%1%#MobaFont%10#0# #-1";

        let result = importer.parse_content(content, "test.mxtsessions");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert_eq!(conn.name, "Console");
        if let ProtocolConfig::Serial(cfg) = &conn.protocol_config {
            // The Windows COM name is carried verbatim for the user to correct.
            assert_eq!(cfg.device, "COM3");
            assert_eq!(cfg.baud_rate, SerialBaudRate::B9600);
            assert_eq!(cfg.data_bits, SerialDataBits::Eight);
            assert_eq!(cfg.parity, SerialParity::None);
            assert_eq!(cfg.stop_bits, SerialStopBits::One);
            assert_eq!(cfg.flow_control, SerialFlowControl::Hardware);
        } else {
            panic!("Expected Serial config");
        }
    }

    #[test]
    fn test_parse_serial_unknown_baud_falls_back_to_default() {
        let importer = MobaXtermImporter::new();
        // 110 baud is not among RustConn's supported rates; expect the 115200
        // default rather than an error.
        let content = "[Bookmarks]
SubRep=
ImgNum=42
Odd Serial=#131#8%COM1%110%8%0%0%0%#MobaFont%10#0# #-1";

        let result = importer.parse_content(content, "test.mxtsessions");
        assert_eq!(result.connections.len(), 1);
        if let ProtocolConfig::Serial(cfg) = &result.connections[0].protocol_config {
            assert_eq!(cfg.baud_rate, SerialBaudRate::B115200);
        } else {
            panic!("Expected Serial config");
        }
    }

    #[test]
    fn test_parse_mosh_session() {
        let importer = MobaXtermImporter::new();
        // MOSH shares the SSH layout: type(12) % host % port % user
        let content = "[Bookmarks]
SubRep=
ImgNum=42
Mobile Shell=#145#12%10.0.0.5%2222%deploy%%-1%-1%%%%%0%0%0%%%-1%0%0%0%%1080%%0%0%1%#MobaFont%10#0# #-1";

        let result = importer.parse_content(content, "test.mxtsessions");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert_eq!(conn.name, "Mobile Shell");
        assert_eq!(conn.host, "10.0.0.5");
        assert_eq!(conn.port, 2222);
        assert_eq!(conn.username, Some("deploy".to_string()));
        if let ProtocolConfig::Mosh(cfg) = &conn.protocol_config {
            // A non-default SSH port is carried into the MOSH config.
            assert_eq!(cfg.ssh_port, Some(2222));
        } else {
            panic!("Expected MOSH config");
        }
    }

    #[test]
    fn test_parse_mosh_default_port_not_stored() {
        let importer = MobaXtermImporter::new();
        let content = "[Bookmarks]
SubRep=
ImgNum=42
Plain Mosh=#145#12%host.example%22%root%%-1%-1%%%%%0%0%0%%%-1%0%0%0%%1080%%0%0%1%#MobaFont%10#0# #-1";

        let result = importer.parse_content(content, "test.mxtsessions");
        assert_eq!(result.connections.len(), 1);
        if let ProtocolConfig::Mosh(cfg) = &result.connections[0].protocol_config {
            // Port 22 is the default, so it is left unset in the MOSH config.
            assert_eq!(cfg.ssh_port, None);
        } else {
            panic!("Expected MOSH config");
        }
    }

    #[test]
    fn test_parse_browser_session() {
        let importer = MobaXtermImporter::new();
        // Browser params: type(11) % URL
        let content = "[Bookmarks]
SubRep=
ImgNum=42
Dashboard=#313#11%https://grafana.example.com%-1%-1%-1%-1%#MobaFont%10#0# #-1";

        let result = importer.parse_content(content, "test.mxtsessions");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert_eq!(conn.name, "Dashboard");
        // The URL lives in the host field, like the other Web importers.
        assert_eq!(conn.host, "https://grafana.example.com");
        assert_eq!(conn.port, 443);
        assert!(matches!(conn.protocol_config, ProtocolConfig::Web(_)));
    }

    #[test]
    fn test_parse_browser_http_url_uses_port_80() {
        let importer = MobaXtermImporter::new();
        let content = "[Bookmarks]
SubRep=
ImgNum=42
Intranet=#313#11%http://wiki.internal%-1%-1%-1%-1%#MobaFont%10#0# #-1";

        let result = importer.parse_content(content, "test.mxtsessions");
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.connections[0].port, 80);
    }

    #[test]
    fn test_session_with_private_key() {
        let importer = MobaXtermImporter::new();
        // SSH session with private key at index 14
        // Indices: 0=type, 1=host, 2=port, 3=user, 4, 5=x11, 6=compress, 7=cmd, 8-10=gateway, 11, 12, 13, 14=key
        let content = "[Bookmarks]
SubRep=
ImgNum=42
Key Auth Server=#109#0%192.168.1.100%22%admin%%0%0%%%%%0%0%0%~/.ssh/id_rsa%#MobaFont%10#0# #-1";

        let result = importer.parse_content(content, "test.mxtsessions");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        if let ProtocolConfig::Ssh(ssh_config) = &conn.protocol_config {
            assert!(ssh_config.key_path.is_some(), "key_path should be Some");
            assert_eq!(ssh_config.auth_method, SshAuthMethod::PublicKey);
        } else {
            panic!("Expected SSH config");
        }
    }
}
