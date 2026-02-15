//! MobaXterm session file exporter.
//!
//! Exports `RustConn` connections to MobaXterm `.mxtsessions` format.
//! Supports SSH, RDP, and VNC connection types.
//!
//! File format: INI-style with Windows-1252 encoding and CRLF line endings.

use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;

use uuid::Uuid;

use crate::models::{Connection, ConnectionGroup, ProtocolConfig, ProtocolType, SshAuthMethod};

use super::{ExportError, ExportFormat, ExportOptions, ExportResult, ExportResult2, ExportTarget};

/// Default icon numbers for each session type in MobaXterm.
const ICON_SSH: u16 = 109;
const ICON_RDP: u16 = 91;
const ICON_VNC: u16 = 128;
const ICON_TELNET: u16 = 98;
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
        let mut params = vec![String::new(); 35];

        // Session type
        params[0] = "0".to_string();

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

        // SSH-specific options
        if let ProtocolConfig::Ssh(ref ssh_config) = connection.protocol_config {
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

        // Fill remaining params with defaults
        for param in &mut params {
            if param.is_empty() {
                *param = String::new();
            }
        }

        let conn_params = params.join("%");
        let terminal_settings = Self::build_terminal_settings();

        // Build full session line: #icon#conn_params#terminal_settings#start_mode#comment#color
        Ok(format!(
            "#{ICON_SSH}#{conn_params}#{terminal_settings}#0# #-1"
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

            // Audio redirect
            params[16] = if rdp_config.audio_redirect {
                "1".to_string()
            } else {
                "0".to_string()
            };

            // Clipboard (always enabled by default in MobaXterm)
            params[19] = "-1".to_string();

            // Color depth
            params[28] = rdp_config
                .color_depth
                .map(|d| Self::color_depth_to_moba_id(d))
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

    /// Builds group hierarchy from flat group list.
    fn build_group_hierarchy(groups: &[ConnectionGroup]) -> HashMap<Uuid, String> {
        let mut paths: HashMap<Uuid, String> = HashMap::new();

        for group in groups {
            paths.insert(group.id, group.name.clone());
        }

        // Build full paths for nested groups
        for group in groups {
            if let Some(parent_id) = group.parent_id {
                if let Some(parent_path) = paths.get(&parent_id).cloned() {
                    let full_path = format!("{}\\{}", parent_path, group.name);
                    paths.insert(group.id, full_path);
                }
            }
        }

        paths
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
    ) -> ExportResult2<ExportResult> {
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
            for conn in root_connections {
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

        // Write grouped connections
        let mut section_index = 1;
        let mut sorted_groups: Vec<_> = connections_by_group
            .iter()
            .filter(|(k, _)| k.is_some())
            .collect();
        sorted_groups.sort_by(|(a, _), (b, _)| a.cmp(b));

        for (group_path, group_connections) in sorted_groups {
            if let Some(path) = group_path {
                let _ = writeln!(output, "[Bookmarks_{section_index}]");
                let _ = writeln!(output, "SubRep={}", Self::encode_escapes(path));
                let _ = writeln!(output, "ImgNum={ICON_FOLDER}");

                for conn in group_connections {
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

                section_index += 1;
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
            ProtocolType::Rdp => Self::export_rdp_session(connection),
            ProtocolType::Vnc => Self::export_vnc_session(connection),
            ProtocolType::Telnet => Self::export_telnet_session(connection),
            ProtocolType::Spice => Err(ExportError::UnsupportedProtocol("SPICE".to_string())),
            ProtocolType::ZeroTrust => {
                Err(ExportError::UnsupportedProtocol("ZeroTrust".to_string()))
            }
            ProtocolType::Serial => Err(ExportError::UnsupportedProtocol("Serial".to_string())),
            ProtocolType::Sftp => Err(ExportError::UnsupportedProtocol("SFTP".to_string())),
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
    ) -> ExportResult2<ExportResult> {
        Self::export_to_file(connections, groups, &options.output_path)
    }

    fn export_connection(&self, connection: &Connection) -> ExportResult2<String> {
        Self::export_connection_line(connection)
    }

    fn supports_protocol(&self, protocol: &ProtocolType) -> bool {
        matches!(
            protocol,
            ProtocolType::Ssh | ProtocolType::Rdp | ProtocolType::Vnc | ProtocolType::Telnet
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Resolution;
    use std::fs;

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
        assert!(exporter.supports_protocol(&ProtocolType::Rdp));
        assert!(exporter.supports_protocol(&ProtocolType::Vnc));
        assert!(!exporter.supports_protocol(&ProtocolType::Spice));
        assert!(!exporter.supports_protocol(&ProtocolType::ZeroTrust));
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
