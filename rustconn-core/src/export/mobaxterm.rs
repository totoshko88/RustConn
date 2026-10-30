//! MobaXterm session file exporter.
//!
//! Exports `RustConn` connections to MobaXterm `.mxtsessions` format.
//! Supports SSH, SFTP, RDP, VNC, Telnet, Serial, MOSH and Web (Browser)
//! connection types; the rest (SPICE, Zero Trust, Kubernetes) are recorded as
//! skipped warnings rather than aborting the export.
//!
//! File format: INI-style with Windows-1252 encoding and CRLF line endings.

use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;

use uuid::Uuid;

use super::{
    ExportError, ExportFormat, ExportOperationResult, ExportOptions, ExportResult, ExportTarget,
};
use crate::models::{
    Connection, ConnectionGroup, ProtocolConfig, ProtocolType, RdpAudioMode, SerialDataBits,
    SerialFlowControl, SerialParity, SerialStopBits, SshAuthMethod,
};

/// Default icon numbers for each session type in MobaXterm.
const ICON_SSH: u16 = 109;
const ICON_SFTP: u16 = 140;
const ICON_RDP: u16 = 91;
const ICON_VNC: u16 = 128;
const ICON_TELNET: u16 = 98;
const ICON_SERIAL: u16 = 131;
const ICON_BROWSER: u16 = 313;
const ICON_MOSH: u16 = 145;
const ICON_FOLDER: u16 = 41;
const ICON_ROOT_FOLDER: u16 = 42;

/// MobaXterm session file exporter.
///
/// Exports connections to `.mxtsessions` format that can be imported into MobaXterm.
pub struct MobaXtermExporter;

impl MobaXtermExporter {
    /// Creates a new MobaXterm exporter.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Encodes special characters for MobaXterm format.
    fn encode_escapes(s: &str) -> String {
        s.replace('%', "__PERCENT__")
            .replace('#', "__DIEZE__")
            .replace(';', "__PTVIRG__")
            .replace('"', "__DBLQUO__")
            .replace('|', "__PIPE__")
    }

    /// Builds the terminal settings string (common for all session types).
    fn build_terminal_settings() -> String {
        // Default terminal settings
        "MobaFont%10%0%0%-1%15%236,236,236%30,30,30%180,180,192%0%-1%0%%xterm%-1%0%\
         _Std_Colors_0_%80%24%0%0%-1%<none>%%0%0%-1%-1"
            .to_string()
    }

    /// Exports a single SSH connection to MobaXterm session line format.
    fn export_ssh_session(connection: &Connection) -> Result<String, ExportError> {
        // Session type 0 = SSH, icon 109.
        Self::build_ssh_like_session(connection, "0", ICON_SSH)
    }

    /// Exports a single SFTP connection to MobaXterm session line format.
    ///
    /// MobaXterm's SFTP session type (7) shares the SSH field layout, and
    /// RustConn's SFTP config reuses `SshConfig`, so the SSH builder is
    /// reused verbatim with the SFTP session type and icon.
    fn export_sftp_session(connection: &Connection) -> Result<String, ExportError> {
        // Session type 7 = SFTP, icon 140.
        Self::build_ssh_like_session(connection, "7", ICON_SFTP)
    }

    /// Builds an SSH-shaped session line for SSH and SFTP.
    ///
    /// Both read their options from an `SshConfig` (SFTP reuses it), so the
    /// only differences are the session-type field and the icon number.
    fn build_ssh_like_session(
        connection: &Connection,
        session_type: &str,
        icon: u16,
    ) -> Result<String, ExportError> {
        let mut params = vec![String::new(); 35];

        // Session type
        params[0] = session_type.to_string();

        // Host
        params[1] = Self::encode_escapes(&connection.host);

        // Port
        params[2] = connection.port.to_string();

        // Username
        params[3] = connection
            .username
            .as_ref()
            .map(|u| Self::encode_escapes(u))
            .unwrap_or_default();

        // X11 forwarding (disabled by default)
        params[5] = "0".to_string();

        // Compression (disabled by default)
        params[6] = "0".to_string();

        // SSH-specific options. SFTP reuses the SSH config, so both variants
        // are read here.
        if let ProtocolConfig::Ssh(ref ssh_config) | ProtocolConfig::Sftp(ref ssh_config) =
            connection.protocol_config
        {
            // Startup command
            if let Some(ref cmd) = ssh_config.startup_command {
                params[7] = Self::encode_escapes(cmd);
            }

            // Private key path
            if let Some(ref key_path) = ssh_config.key_path {
                params[14] = Self::encode_escapes(&key_path.display().to_string());
            }

            // SSH agent
            params[33] = if matches!(ssh_config.auth_method, SshAuthMethod::Agent) {
                "-1".to_string()
            } else {
                "0".to_string()
            };

            // Agent forwarding
            params[34] = if ssh_config.agent_forwarding {
                "-1".to_string()
            } else {
                "0".to_string()
            };
        }

        let conn_params = params.join("%");
        let terminal_settings = Self::build_terminal_settings();

        // Build full session line: #icon#conn_params#terminal_settings#start_mode#comment#color
        Ok(format!("#{icon}#{conn_params}#{terminal_settings}#0# #-1"))
    }

    /// Exports a single MOSH connection to MobaXterm session line format.
    ///
    /// MobaXterm's MOSH session type (12) shares the SSH field layout (MOSH
    /// connects over SSH for its handshake), so host, SSH port and username are
    /// written as for SSH. The SSH port prefers the MOSH config's `ssh_port`
    /// when set, falling back to the connection's port.
    fn export_mosh_session(connection: &Connection) -> Result<String, ExportError> {
        let mut params = vec![String::new(); 35];

        // Session type (12 = MOSH)
        params[0] = "12".to_string();

        // Host
        params[1] = Self::encode_escapes(&connection.host);

        // SSH port: the MOSH config's ssh_port overrides the connection port.
        let ssh_port = match connection.protocol_config {
            ProtocolConfig::Mosh(ref mosh_config) => {
                mosh_config.ssh_port.unwrap_or(connection.port)
            }
            _ => connection.port,
        };
        params[2] = ssh_port.to_string();

        // Username
        params[3] = connection
            .username
            .as_ref()
            .map(|u| Self::encode_escapes(u))
            .unwrap_or_default();

        let conn_params = params.join("%");
        let terminal_settings = Self::build_terminal_settings();

        Ok(format!(
            "#{ICON_MOSH}#{conn_params}#{terminal_settings}#0# #-1"
        ))
    }

    /// Exports a single Web connection to a MobaXterm Browser session line.
    ///
    /// RustConn stores a Web connection's URL in its host field, which maps
    /// directly to MobaXterm's Browser session type (11).
    fn export_web_session(connection: &Connection) -> Result<String, ExportError> {
        let mut params = vec![String::new(); 18];

        // Session type (11 = Browser)
        params[0] = "11".to_string();

        // URL (stored in the host field)
        params[1] = Self::encode_escapes(&connection.host);

        let conn_params = params.join("%");
        let terminal_settings = Self::build_terminal_settings();

        Ok(format!(
            "#{ICON_BROWSER}#{conn_params}#{terminal_settings}#0# #-1"
        ))
    }

    /// Exports a single Serial connection to MobaXterm session line format.
    ///
    /// MobaXterm's Serial session type (8) stores a device name, baud rate,
    /// data bits, parity, stop bits and flow control. The RustConn device path
    /// (for example `/dev/ttyUSB0`) is written verbatim; it is not a Windows
    /// COM port, so the user retargets it after importing into MobaXterm.
    fn export_serial_session(connection: &Connection) -> Result<String, ExportError> {
        let mut params = vec![String::new(); 10];

        // Session type (8 = Serial)
        params[0] = "8".to_string();

        if let ProtocolConfig::Serial(ref serial_config) = connection.protocol_config {
            // Device (port). Written verbatim; a Linux /dev path is not a COM
            // port, so the user retargets it in MobaXterm.
            params[1] = Self::encode_escapes(&serial_config.device);

            // Baud rate, e.g. "115200".
            params[2] = serial_config.baud_rate.display_name().to_string();

            // Data bits.
            params[3] = match serial_config.data_bits {
                SerialDataBits::Five => "5",
                SerialDataBits::Six => "6",
                SerialDataBits::Seven => "7",
                SerialDataBits::Eight => "8",
            }
            .to_string();

            // Parity (0=none, 1=odd, 2=even).
            params[4] = match serial_config.parity {
                SerialParity::None => "0",
                SerialParity::Odd => "1",
                SerialParity::Even => "2",
            }
            .to_string();

            // Stop bits (0=1, 2=2).
            params[5] = match serial_config.stop_bits {
                SerialStopBits::One => "0",
                SerialStopBits::Two => "2",
            }
            .to_string();

            // Flow control (0=none, 1=RTS/CTS, 2=XON/XOFF).
            params[6] = match serial_config.flow_control {
                SerialFlowControl::None => "0",
                SerialFlowControl::Hardware => "1",
                SerialFlowControl::Software => "2",
            }
            .to_string();
        }

        let conn_params = params.join("%");
        let terminal_settings = Self::build_terminal_settings();

        Ok(format!(
            "#{ICON_SERIAL}#{conn_params}#{terminal_settings}#0# #-1"
        ))
    }

    /// Exports a single RDP connection to MobaXterm session line format.
    fn export_rdp_session(connection: &Connection) -> Result<String, ExportError> {
        let mut params = vec![String::new(); 32];

        // Session type
        params[0] = "4".to_string();

        // Host
        params[1] = Self::encode_escapes(&connection.host);

        // Port
        params[2] = connection.port.to_string();

        // Username
        params[3] = connection
            .username
            .as_ref()
            .map(|u| Self::encode_escapes(u))
            .unwrap_or_default();

        // RDP-specific options
        if let ProtocolConfig::Rdp(ref rdp_config) = connection.protocol_config {
            // Resolution
            params[10] = rdp_config
                .resolution
                .as_ref()
                .map(|r| Self::resolution_to_moba_id(r.width, r.height))
                .unwrap_or_else(|| "0".to_string()); // Fit to terminal

            // Audio, field 16: 0 = none, 1 = redirect here, 2 = leave on the
            // remote computer — the layout the importer documents and reads
            // back. Reading the legacy `audio_redirect` bool exported "remote"
            // as "none".
            params[16] = match rdp_config.effective_audio_mode() {
                RdpAudioMode::None => "0",
                RdpAudioMode::Local => "1",
                RdpAudioMode::Remote => "2",
            }
            .to_string();

            // Clipboard. MobaXterm has no direction concept, so this degrades to
            // on/off: any enabled direction exports `-1`, `Off` exports `0`.
            params[19] = if rdp_config.clipboard_enabled {
                "-1"
            } else {
                "0"
            }
            .to_string();

            // Color depth
            params[28] = rdp_config
                .color_depth
                .map(Self::color_depth_to_moba_id)
                .unwrap_or_else(|| "0".to_string()); // Auto
        }

        let conn_params = params.join("%");
        let terminal_settings = Self::build_terminal_settings();

        Ok(format!(
            "#{ICON_RDP}#{conn_params}#{terminal_settings}#0# #-1"
        ))
    }

    /// Exports a single Telnet connection to MobaXterm session line format.
    fn export_telnet_session(connection: &Connection) -> Result<String, ExportError> {
        let mut params = vec![String::new(); 18];

        // Session type (1 = Telnet)
        params[0] = "1".to_string();

        // Host
        params[1] = Self::encode_escapes(&connection.host);

        // Port
        params[2] = connection.port.to_string();

        let conn_params = params.join("%");
        let terminal_settings = Self::build_terminal_settings();

        Ok(format!(
            "#{ICON_TELNET}#{conn_params}#{terminal_settings}#0# #-1"
        ))
    }

    /// Exports a single VNC connection to MobaXterm session line format.
    fn export_vnc_session(connection: &Connection) -> Result<String, ExportError> {
        let mut params = vec![String::new(); 18];

        // Session type
        params[0] = "5".to_string();

        // Host
        params[1] = Self::encode_escapes(&connection.host);

        // Port
        params[2] = connection.port.to_string();

        // VNC-specific options
        if let ProtocolConfig::Vnc(ref vnc_config) = connection.protocol_config {
            // Auto scale
            params[3] = "-1".to_string(); // Enabled by default

            // View only
            params[4] = if vnc_config.view_only {
                "-1".to_string()
            } else {
                "0".to_string()
            };
        } else {
            params[3] = "-1".to_string();
            params[4] = "0".to_string();
        }

        let conn_params = params.join("%");
        let terminal_settings = Self::build_terminal_settings();

        Ok(format!(
            "#{ICON_VNC}#{conn_params}#{terminal_settings}#0# #-1"
        ))
    }

    /// Converts resolution to MobaXterm resolution ID.
    fn resolution_to_moba_id(width: u32, height: u32) -> String {
        match (width, height) {
            (640, 480) => "2",
            (800, 600) => "3",
            (1024, 768) => "4",
            (1152, 864) => "5",
            (1280, 720) => "6",
            (1280, 968) => "7",
            (1280, 1024) => "8",
            (1400, 1050) => "9",
            (1600, 1200) => "10",
            (1920, 1080) => "11",
            (1920, 1200) => "14",
            (2560, 1440) => "24",
            (3840, 2160) => "26",
            _ => "0", // Fit to terminal
        }
        .to_string()
    }

    /// Converts color depth to MobaXterm color depth ID.
    fn color_depth_to_moba_id(depth: u8) -> String {
        match depth {
            8 => "1",
            16 => "2",
            24 => "3",
            32 => "4",
            _ => "0", // Auto
        }
        .to_string()
    }

    /// Builds full backslash-separated folder paths for each group.
    ///
    /// Walks each group's parent chain to produce a MobaXterm `SubRep` path
    /// such as `Production\Web`. Unlike a single-pass approach, this correctly
    /// handles folders nested more than two levels deep regardless of the order
    /// groups appear in the input slice. A cycle guard prevents infinite loops
    /// on malformed `parent_id` references.
    fn build_group_hierarchy(groups: &[ConnectionGroup]) -> HashMap<Uuid, String> {
        let by_id: HashMap<Uuid, &ConnectionGroup> = groups.iter().map(|g| (g.id, g)).collect();
        let mut paths: HashMap<Uuid, String> = HashMap::with_capacity(groups.len());

        for group in groups {
            let mut segments = vec![group.name.as_str()];
            let mut current = group;
            let mut guard = 0;

            while let Some(parent_id) = current.parent_id {
                // Bound the walk by the group count to defend against cycles.
                if guard >= groups.len() {
                    break;
                }
                guard += 1;

                let Some(parent) = by_id.get(&parent_id) else {
                    break;
                };
                segments.push(parent.name.as_str());
                current = parent;
            }

            segments.reverse();
            paths.insert(group.id, segments.join("\\"));
        }

        paths
    }

    /// Writes the session lines for a set of connections into the output buffer.
    ///
    /// Unsupported protocols are recorded as skipped warnings rather than
    /// aborting the export.
    fn write_connection_lines(
        output: &mut String,
        connections: &[&Connection],
        result: &mut ExportResult,
    ) {
        for conn in connections {
            match Self::export_connection_line(conn) {
                Ok(line) => {
                    let name = Self::encode_escapes(&conn.name);
                    let _ = writeln!(output, "{name}={line}");
                    result.increment_exported();
                }
                Err(ExportError::UnsupportedProtocol(proto)) => {
                    result.increment_skipped();
                    result.add_warning(format!(
                        "Skipped '{}': unsupported protocol {}",
                        conn.name, proto
                    ));
                }
                Err(e) => {
                    result.increment_skipped();
                    result.add_warning(format!("Failed to export '{}': {}", conn.name, e));
                }
            }
        }
    }

    /// Exports all connections to MobaXterm format.
    ///
    /// # Errors
    ///
    /// Returns an error if the output file cannot be written.
    pub fn export_to_file(
        connections: &[Connection],
        groups: &[ConnectionGroup],
        output_path: &Path,
    ) -> ExportOperationResult<ExportResult> {
        let mut result = ExportResult::new();
        let mut output = String::new();

        // Build group paths
        let group_paths = Self::build_group_hierarchy(groups);

        // Group connections by their group path
        let mut connections_by_group: HashMap<Option<String>, Vec<&Connection>> = HashMap::new();

        for conn in connections {
            let group_path = conn.group_id.and_then(|id| group_paths.get(&id).cloned());
            connections_by_group
                .entry(group_path)
                .or_default()
                .push(conn);
        }

        // Write root bookmarks section
        let _ = writeln!(output, "[Bookmarks]");
        let _ = writeln!(output, "SubRep=");
        let _ = writeln!(output, "ImgNum={ICON_ROOT_FOLDER}");

        // Write root-level connections
        if let Some(root_connections) = connections_by_group.get(&None) {
            Self::write_connection_lines(&mut output, root_connections, &mut result);
        }

        // Write one section per group, sorted by full folder path so parent
        // folders always precede their children. Every group gets a section —
        // including intermediate folders with no direct sessions — so MobaXterm
        // can reconstruct the complete folder tree on import.
        let mut sorted_groups: Vec<&ConnectionGroup> = groups.iter().collect();
        sorted_groups.sort_by(|a, b| {
            group_paths
                .get(&a.id)
                .map(String::as_str)
                .cmp(&group_paths.get(&b.id).map(String::as_str))
        });

        for (offset, group) in sorted_groups.iter().enumerate() {
            let Some(path) = group_paths.get(&group.id) else {
                continue;
            };

            let _ = writeln!(output, "[Bookmarks_{}]", offset + 1);
            let _ = writeln!(output, "SubRep={}", Self::encode_escapes(path));
            let _ = writeln!(output, "ImgNum={ICON_FOLDER}");

            if let Some(group_connections) = connections_by_group.get(&Some(path.clone())) {
                Self::write_connection_lines(&mut output, group_connections, &mut result);
            }
        }

        // Convert to Windows line endings (CRLF)
        let output_crlf = output.replace('\n', "\r\n");

        // Write to file
        super::write_export_file(output_path, &output_crlf)?;

        result.add_output_file(output_path.to_path_buf());
        Ok(result)
    }

    /// Exports a single connection to a session line.
    fn export_connection_line(connection: &Connection) -> Result<String, ExportError> {
        match connection.protocol {
            ProtocolType::Ssh => Self::export_ssh_session(connection),
            ProtocolType::Sftp => Self::export_sftp_session(connection),
            ProtocolType::Rdp => Self::export_rdp_session(connection),
            ProtocolType::Vnc => Self::export_vnc_session(connection),
            ProtocolType::Telnet => Self::export_telnet_session(connection),
            ProtocolType::Serial => Self::export_serial_session(connection),
            ProtocolType::Mosh => Self::export_mosh_session(connection),
            ProtocolType::Web => Self::export_web_session(connection),
            ProtocolType::Spice => Err(ExportError::UnsupportedProtocol("SPICE".to_string())),
            ProtocolType::ZeroTrust => {
                Err(ExportError::UnsupportedProtocol("ZeroTrust".to_string()))
            }
            ProtocolType::Kubernetes => {
                Err(ExportError::UnsupportedProtocol("Kubernetes".to_string()))
            }
        }
    }
}

impl Default for MobaXtermExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ExportTarget for MobaXtermExporter {
    fn format_id(&self) -> ExportFormat {
        ExportFormat::MobaXterm
    }

    fn display_name(&self) -> &'static str {
        "MobaXterm"
    }

    fn export(
        &self,
        connections: &[Connection],
        groups: &[ConnectionGroup],
        options: &ExportOptions,
    ) -> ExportOperationResult<ExportResult> {
        Self::export_to_file(connections, groups, &options.output_path)
    }

    fn export_connection(&self, connection: &Connection) -> ExportOperationResult<String> {
        Self::export_connection_line(connection)
    }

    fn supports_protocol(&self, protocol: &ProtocolType) -> bool {
        matches!(
            protocol,
            ProtocolType::Ssh
                | ProtocolType::Sftp
                | ProtocolType::Rdp
                | ProtocolType::Vnc
                | ProtocolType::Telnet
                | ProtocolType::Serial
                | ProtocolType::Mosh
                | ProtocolType::Web
        )
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::models::Resolution;

    fn create_ssh_connection(name: &str, host: &str, port: u16) -> Connection {
        Connection::new_ssh(name.to_string(), host.to_string(), port)
    }

    fn create_rdp_connection(name: &str, host: &str, port: u16) -> Connection {
        Connection::new_rdp(name.to_string(), host.to_string(), port)
    }

    fn create_vnc_connection(name: &str, host: &str, port: u16) -> Connection {
        Connection::new_vnc(name.to_string(), host.to_string(), port)
    }

    #[test]
    fn test_encode_escapes() {
        assert_eq!(
            MobaXtermExporter::encode_escapes("test%value"),
            "test__PERCENT__value"
        );
        assert_eq!(
            MobaXtermExporter::encode_escapes("test#value"),
            "test__DIEZE__value"
        );
        assert_eq!(
            MobaXtermExporter::encode_escapes("test|value"),
            "test__PIPE__value"
        );
    }

    #[test]
    fn test_export_ssh_session() {
        let conn = create_ssh_connection("myserver", "192.168.1.100", 22);
        let line = MobaXtermExporter::export_ssh_session(&conn).unwrap();

        assert!(line.starts_with("#109#"));
        assert!(line.contains("192.168.1.100"));
        assert!(line.contains("%22%"));
    }

    #[test]
    fn test_export_ssh_session_with_username() {
        let conn = create_ssh_connection("myserver", "192.168.1.100", 22).with_username("admin");
        let line = MobaXtermExporter::export_ssh_session(&conn).unwrap();

        assert!(line.contains("%admin%"));
    }

    #[test]
    fn test_export_rdp_session() {
        let conn = create_rdp_connection("windows", "192.168.1.50", 3389);
        let line = MobaXtermExporter::export_rdp_session(&conn).unwrap();

        assert!(line.starts_with("#91#"));
        assert!(line.contains("192.168.1.50"));
        assert!(line.contains("%3389%"));
    }

    #[test]
    fn test_export_rdp_session_with_resolution() {
        let mut conn = create_rdp_connection("windows", "192.168.1.50", 3389);
        if let ProtocolConfig::Rdp(ref mut rdp_config) = conn.protocol_config {
            rdp_config.resolution = Some(Resolution::new(1920, 1080));
            rdp_config.color_depth = Some(32);
        }
        let line = MobaXtermExporter::export_rdp_session(&conn).unwrap();

        // Resolution 1920x1080 = ID 11
        assert!(line.contains("%11%"));
        // Color depth 32 = ID 4
        assert!(line.contains("%4%"));
    }

    /// MobaXterm has no clipboard direction concept, so the export degrades to
    /// on/off: an enabled direction writes `-1`, `Off` writes `0` (field 19).
    #[test]
    fn mobaxterm_export_clipboard_off_writes_zero() {
        use crate::models::RdpClipboardDirection;

        fn clipboard_field(line: &str) -> String {
            // Line is `#91#<params joined by %>#<terminal>#0# #-1`.
            let conn_params = line.splitn(3, '#').nth(2).expect("params segment");
            let conn_params = conn_params
                .split('#')
                .next()
                .expect("params before terminal");
            conn_params
                .split('%')
                .nth(19)
                .expect("field 19")
                .to_string()
        }

        let mut both = create_rdp_connection("windows", "192.168.1.50", 3389);
        if let ProtocolConfig::Rdp(ref mut rdp) = both.protocol_config {
            rdp.set_clipboard_direction(RdpClipboardDirection::Both);
        }
        let both_line = MobaXtermExporter::export_rdp_session(&both).unwrap();
        assert_eq!(clipboard_field(&both_line), "-1");

        let mut off = create_rdp_connection("windows", "192.168.1.50", 3389);
        if let ProtocolConfig::Rdp(ref mut rdp) = off.protocol_config {
            rdp.set_clipboard_direction(RdpClipboardDirection::Off);
        }
        let off_line = MobaXtermExporter::export_rdp_session(&off).unwrap();
        assert_eq!(clipboard_field(&off_line), "0");
    }

    #[test]
    fn test_export_vnc_session() {
        let conn = create_vnc_connection("vnc-desktop", "192.168.1.75", 5900);
        let line = MobaXtermExporter::export_vnc_session(&conn).unwrap();

        assert!(line.starts_with("#128#"));
        assert!(line.contains("192.168.1.75"));
        assert!(line.contains("%5900%"));
    }

    #[test]
    fn test_export_spice_fails() {
        let conn = Connection::new_spice("spice-vm".to_string(), "192.168.1.100".to_string(), 5900);
        let result = MobaXtermExporter::export_connection_line(&conn);

        assert!(result.is_err());
        if let Err(ExportError::UnsupportedProtocol(proto)) = result {
            assert_eq!(proto, "SPICE");
        } else {
            panic!("Expected UnsupportedProtocol error");
        }
    }

    #[test]
    fn test_resolution_to_moba_id() {
        assert_eq!(MobaXtermExporter::resolution_to_moba_id(1920, 1080), "11");
        assert_eq!(MobaXtermExporter::resolution_to_moba_id(1024, 768), "4");
        assert_eq!(MobaXtermExporter::resolution_to_moba_id(3840, 2160), "26");
        assert_eq!(MobaXtermExporter::resolution_to_moba_id(999, 999), "0"); // Unknown -> Fit
    }

    #[test]
    fn test_color_depth_to_moba_id() {
        assert_eq!(MobaXtermExporter::color_depth_to_moba_id(8), "1");
        assert_eq!(MobaXtermExporter::color_depth_to_moba_id(16), "2");
        assert_eq!(MobaXtermExporter::color_depth_to_moba_id(24), "3");
        assert_eq!(MobaXtermExporter::color_depth_to_moba_id(32), "4");
        assert_eq!(MobaXtermExporter::color_depth_to_moba_id(15), "0"); // Unknown -> Auto
    }

    #[test]
    fn test_supports_protocol() {
        let exporter = MobaXtermExporter::new();
        assert!(exporter.supports_protocol(&ProtocolType::Ssh));
        assert!(exporter.supports_protocol(&ProtocolType::Sftp));
        assert!(exporter.supports_protocol(&ProtocolType::Rdp));
        assert!(exporter.supports_protocol(&ProtocolType::Vnc));
        assert!(exporter.supports_protocol(&ProtocolType::Telnet));
        assert!(exporter.supports_protocol(&ProtocolType::Serial));
        assert!(exporter.supports_protocol(&ProtocolType::Mosh));
        assert!(exporter.supports_protocol(&ProtocolType::Web));
        assert!(!exporter.supports_protocol(&ProtocolType::Spice));
        assert!(!exporter.supports_protocol(&ProtocolType::ZeroTrust));
        assert!(!exporter.supports_protocol(&ProtocolType::Kubernetes));
    }

    #[test]
    fn test_export_sftp_session() {
        let conn = Connection::new_sftp("files".to_string(), "192.168.1.9".to_string(), 22);
        let line = MobaXtermExporter::export_connection_line(&conn).unwrap();

        // SFTP session type 7, icon 140.
        assert!(line.starts_with("#140#"));
        assert!(line.contains("192.168.1.9"));
        // params[0] is "7" (session type), immediately after the icon fence.
        let conn_params = line.splitn(3, '#').nth(2).unwrap();
        assert!(conn_params.starts_with("7%"), "SFTP session type must be 7");
    }

    #[test]
    fn test_export_mosh_session() {
        let mut conn = Connection::new_mosh("mobile".to_string(), "10.0.0.5".to_string(), 22);
        conn.username = Some("deploy".to_string());
        if let ProtocolConfig::Mosh(ref mut cfg) = conn.protocol_config {
            cfg.ssh_port = Some(2222);
        }
        let line = MobaXtermExporter::export_connection_line(&conn).unwrap();

        // MOSH icon 145, session type 12, SSH port taken from the config.
        assert!(line.starts_with("#145#"));
        let conn_params = line.splitn(3, '#').nth(2).unwrap();
        assert!(
            conn_params.starts_with("12%"),
            "MOSH session type must be 12"
        );
        assert!(
            line.contains("%2222%"),
            "MOSH ssh_port should override port"
        );
        assert!(line.contains("%deploy%"));
    }

    #[test]
    fn test_export_web_session() {
        let conn = Connection::new(
            "dash".to_string(),
            "https://grafana.example.com".to_string(),
            443,
            ProtocolConfig::Web(crate::models::WebConfig::default()),
        );
        let line = MobaXtermExporter::export_connection_line(&conn).unwrap();

        // Browser icon 313, session type 11, URL in the host field.
        assert!(line.starts_with("#313#"));
        let conn_params = line.splitn(3, '#').nth(2).unwrap();
        assert!(
            conn_params.starts_with("11%"),
            "Browser session type must be 11"
        );
        assert!(line.contains("https://grafana.example.com"));
    }

    #[test]
    fn test_export_serial_session() {
        use crate::models::{SerialBaudRate, SerialConfig};

        let mut conn = Connection::new_serial("console".to_string(), "/dev/ttyUSB0".to_string());
        if let ProtocolConfig::Serial(ref mut cfg) = conn.protocol_config {
            *cfg = SerialConfig {
                device: "/dev/ttyUSB0".to_string(),
                baud_rate: SerialBaudRate::B9600,
                data_bits: SerialDataBits::Seven,
                parity: SerialParity::Even,
                stop_bits: SerialStopBits::Two,
                flow_control: SerialFlowControl::Software,
                custom_args: Vec::new(),
            };
        }
        let line = MobaXtermExporter::export_connection_line(&conn).unwrap();

        // Serial icon 131, session type 8.
        assert!(line.starts_with("#131#"));
        let conn_params = line.splitn(3, '#').nth(2).unwrap();
        let fields: Vec<&str> = conn_params.split('#').next().unwrap().split('%').collect();
        assert_eq!(fields[0], "8", "Serial session type");
        assert_eq!(fields[1], "/dev/ttyUSB0");
        assert_eq!(fields[2], "9600");
        assert_eq!(fields[3], "7"); // data bits
        assert_eq!(fields[4], "2"); // parity even
        assert_eq!(fields[5], "2"); // stop bits 2
        assert_eq!(fields[6], "2"); // flow control XON/XOFF
    }

    #[test]
    fn serial_round_trips_through_mobaxterm() {
        use crate::import::MobaXtermImporter;
        use crate::models::{SerialBaudRate, SerialConfig};

        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("serial.mxtsessions");

        let mut conn = Connection::new_serial("console".to_string(), "/dev/ttyUSB0".to_string());
        if let ProtocolConfig::Serial(ref mut cfg) = conn.protocol_config {
            *cfg = SerialConfig {
                device: "/dev/ttyUSB0".to_string(),
                baud_rate: SerialBaudRate::B9600,
                data_bits: SerialDataBits::Seven,
                parity: SerialParity::Even,
                stop_bits: SerialStopBits::Two,
                flow_control: SerialFlowControl::Software,
                custom_args: Vec::new(),
            };
        }

        MobaXtermExporter::export_to_file(&[conn], &[], &output_path).unwrap();
        let content = fs::read_to_string(&output_path).unwrap();
        let result = MobaXtermImporter::new().parse_content(&content, "serial.mxtsessions");

        assert_eq!(result.connections.len(), 1);
        let ProtocolConfig::Serial(ref imported) = result.connections[0].protocol_config else {
            panic!("expected a Serial connection");
        };
        assert_eq!(imported.device, "/dev/ttyUSB0");
        assert_eq!(imported.baud_rate, SerialBaudRate::B9600);
        assert_eq!(imported.data_bits, SerialDataBits::Seven);
        assert_eq!(imported.parity, SerialParity::Even);
        assert_eq!(imported.stop_bits, SerialStopBits::Two);
        assert_eq!(imported.flow_control, SerialFlowControl::Software);
    }

    #[test]
    fn sftp_round_trips_through_mobaxterm() {
        use crate::import::MobaXtermImporter;

        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("sftp.mxtsessions");

        let mut conn = Connection::new_sftp("files".to_string(), "192.168.1.9".to_string(), 22);
        conn.username = Some("admin".to_string());

        MobaXtermExporter::export_to_file(&[conn], &[], &output_path).unwrap();
        let content = fs::read_to_string(&output_path).unwrap();
        let result = MobaXtermImporter::new().parse_content(&content, "sftp.mxtsessions");

        assert_eq!(result.connections.len(), 1);
        let imported = &result.connections[0];
        // Session type 7 must round-trip back as SFTP, not be downgraded to SSH.
        assert!(
            matches!(imported.protocol_config, ProtocolConfig::Sftp(_)),
            "SFTP must survive the round-trip, got {:?}",
            imported.protocol_config
        );
        assert_eq!(imported.host, "192.168.1.9");
        assert_eq!(imported.port, 22);
        assert_eq!(imported.username, Some("admin".to_string()));
    }

    #[test]
    fn web_and_mosh_round_trip_through_mobaxterm() {
        use crate::import::MobaXtermImporter;

        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("mixed.mxtsessions");

        let web = Connection::new(
            "dash".to_string(),
            "https://grafana.example.com".to_string(),
            443,
            ProtocolConfig::Web(crate::models::WebConfig::default()),
        );
        let mut mosh = Connection::new_mosh("mobile".to_string(), "10.0.0.5".to_string(), 2222);
        mosh.username = Some("deploy".to_string());

        MobaXtermExporter::export_to_file(&[web, mosh], &[], &output_path).unwrap();
        let content = fs::read_to_string(&output_path).unwrap();
        let result = MobaXtermImporter::new().parse_content(&content, "mixed.mxtsessions");

        assert_eq!(result.connections.len(), 2);

        let web = result
            .connections
            .iter()
            .find(|c| c.name == "dash")
            .expect("web connection");
        assert!(matches!(web.protocol_config, ProtocolConfig::Web(_)));
        assert_eq!(web.host, "https://grafana.example.com");

        let mosh = result
            .connections
            .iter()
            .find(|c| c.name == "mobile")
            .expect("mosh connection");
        assert!(matches!(mosh.protocol_config, ProtocolConfig::Mosh(_)));
        assert_eq!(mosh.host, "10.0.0.5");
        assert_eq!(mosh.port, 2222);
        assert_eq!(mosh.username, Some("deploy".to_string()));
    }

    #[test]
    fn test_export_to_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("sessions.mxtsessions");

        let connections = vec![
            create_ssh_connection("ssh-server", "192.168.1.1", 22),
            create_rdp_connection("rdp-server", "192.168.1.2", 3389),
            create_vnc_connection("vnc-server", "192.168.1.3", 5900),
        ];

        let result = MobaXtermExporter::export_to_file(&connections, &[], &output_path).unwrap();

        assert_eq!(result.exported_count, 3);
        assert_eq!(result.skipped_count, 0);
        assert!(output_path.exists());

        // Verify file content
        let content = fs::read_to_string(&output_path).unwrap();
        assert!(content.contains("[Bookmarks]"));
        assert!(content.contains("ssh-server="));
        assert!(content.contains("rdp-server="));
        assert!(content.contains("vnc-server="));
        // Verify CRLF line endings
        assert!(content.contains("\r\n"));
    }

    #[test]
    fn test_export_with_groups() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("sessions.mxtsessions");

        let group = ConnectionGroup::new("Production".to_string());
        let group_id = group.id;

        let mut conn = create_ssh_connection("prod-server", "10.0.0.1", 22);
        conn.group_id = Some(group_id);

        let connections = vec![
            create_ssh_connection("root-server", "192.168.1.1", 22),
            conn,
        ];
        let groups = vec![group];

        let result =
            MobaXtermExporter::export_to_file(&connections, &groups, &output_path).unwrap();

        assert_eq!(result.exported_count, 2);

        let content = fs::read_to_string(&output_path).unwrap();
        assert!(content.contains("[Bookmarks]"));
        assert!(content.contains("[Bookmarks_1]"));
        assert!(content.contains("SubRep=Production"));
    }

    #[test]
    fn test_export_nested_folders_preserves_hierarchy() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("sessions.mxtsessions");

        // Production -> Web -> web01 (intermediate "Web" holds no direct session)
        let production = ConnectionGroup::new("Production".to_string());
        let web = ConnectionGroup::with_parent("Web".to_string(), production.id);

        let mut conn = create_ssh_connection("web01", "10.0.1.2", 22);
        conn.group_id = Some(web.id);

        let connections = vec![conn];
        let groups = vec![production, web];

        MobaXtermExporter::export_to_file(&connections, &groups, &output_path).unwrap();

        let content = fs::read_to_string(&output_path).unwrap();

        // Both the intermediate and leaf folders must be emitted as sections,
        // with full backslash-separated paths, so MobaXterm rebuilds the tree.
        assert!(
            content.contains("SubRep=Production\r\n"),
            "intermediate Production folder must be present"
        );
        assert!(
            content.contains("SubRep=Production\\Web\r\n"),
            "nested Web folder must use the full path"
        );
        assert!(content.contains("web01="));

        // Parent folder section must precede the child folder section.
        let prod_idx = content.find("SubRep=Production\r\n").unwrap();
        let web_idx = content.find("SubRep=Production\\Web\r\n").unwrap();
        assert!(prod_idx < web_idx, "parent folder must come before child");
    }

    #[test]
    fn test_roundtrip_nested_folders() {
        use crate::import::MobaXtermImporter;

        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("sessions.mxtsessions");

        let production = ConnectionGroup::new("Production".to_string());
        let web = ConnectionGroup::with_parent("Web".to_string(), production.id);

        let mut conn = create_ssh_connection("web01", "10.0.1.2", 22);
        conn.group_id = Some(web.id);

        let connections = vec![conn];
        let groups = vec![production, web];

        MobaXtermExporter::export_to_file(&connections, &groups, &output_path).unwrap();

        // Re-import the exported file and verify the hierarchy survives.
        let importer = MobaXtermImporter::new();
        let content = fs::read_to_string(&output_path).unwrap();
        let result = importer.parse_content(&content, "roundtrip.mxtsessions");

        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.groups.len(), 2);

        let imported_prod = result
            .groups
            .iter()
            .find(|g| g.name == "Production")
            .expect("Production group should round-trip");
        let imported_web = result
            .groups
            .iter()
            .find(|g| g.name == "Web")
            .expect("Web group should round-trip");

        assert!(imported_prod.parent_id.is_none());
        assert_eq!(imported_web.parent_id, Some(imported_prod.id));
        assert_eq!(result.connections[0].group_id, Some(imported_web.id));
    }

    /// Every audio mode survives a MobaXterm export and import, "remote"
    /// included, which the exporter used to write as "none".
    #[test]
    fn audio_mode_round_trips_through_mobaxterm() {
        use crate::import::MobaXtermImporter;

        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("audio.mxtsessions");
        for mode in RdpAudioMode::all().iter().copied() {
            let mut conn = create_rdp_connection("windows", "192.168.1.50", 3389);
            if let ProtocolConfig::Rdp(ref mut rdp_config) = conn.protocol_config {
                rdp_config.set_audio_mode(mode);
            }

            MobaXtermExporter::export_to_file(&[conn], &[], &output_path).unwrap();
            let content = fs::read_to_string(&output_path).unwrap();
            let result = MobaXtermImporter::new().parse_content(&content, "audio.mxtsessions");

            let ProtocolConfig::Rdp(ref imported) = result.connections[0].protocol_config else {
                panic!("expected an RDP connection");
            };
            assert_eq!(imported.effective_audio_mode(), mode, "{content}");
        }
    }

    #[test]
    fn test_export_skips_spice() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("sessions.mxtsessions");

        let connections = vec![
            create_ssh_connection("ssh-server", "192.168.1.1", 22),
            Connection::new_spice("spice-vm".to_string(), "192.168.1.2".to_string(), 5900),
        ];

        let result = MobaXtermExporter::export_to_file(&connections, &[], &output_path).unwrap();

        assert_eq!(result.exported_count, 1);
        assert_eq!(result.skipped_count, 1);
        assert!(result.has_warnings());
    }
}
