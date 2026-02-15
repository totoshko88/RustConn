//! Remmina connection file exporter.
//!
//! Exports `RustConn` connections to Remmina .remmina file format.

use std::fmt::Write;
use std::fs;
use std::path::Path;

use crate::models::{Connection, ConnectionGroup, ProtocolConfig, ProtocolType};

use super::{ExportError, ExportFormat, ExportOptions, ExportResult, ExportResult2, ExportTarget};

/// Remmina connection file exporter.
///
/// Exports connections to Remmina .remmina file format.
/// Each connection is exported to a separate file in the output directory.
pub struct RemminaExporter;

impl RemminaExporter {
    /// Creates a new Remmina exporter
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Exports a connection to .remmina file content.
    ///
    /// # Arguments
    ///
    /// * `connection` - The connection to export
    ///
    /// # Returns
    ///
    /// A string containing the .remmina file content.
    ///
    /// # Errors
    ///
    /// Returns an error if the protocol is not supported.
    pub fn export_connection(connection: &Connection) -> Result<String, ExportError> {
        let mut output = String::new();
        output.push_str("[remmina]\n");

        // Common fields
        let _ = writeln!(output, "name={}", connection.name);

        match connection.protocol {
            ProtocolType::Ssh => Self::write_ssh_fields(&mut output, connection),
            ProtocolType::Rdp => Self::write_rdp_fields(&mut output, connection),
            ProtocolType::Vnc => Self::write_vnc_fields(&mut output, connection),
            ProtocolType::Spice => {
                return Err(ExportError::UnsupportedProtocol("SPICE".to_string()));
            }
            ProtocolType::Telnet => {
                // Remmina supports TELNET protocol natively
                let _ = writeln!(output, "protocol=TELNET");
                let _ = writeln!(output, "server={}:{}", connection.host, connection.port);
            }
            ProtocolType::ZeroTrust => {
                return Err(ExportError::UnsupportedProtocol("ZeroTrust".to_string()));
            }
            ProtocolType::Serial => {
                return Err(ExportError::UnsupportedProtocol("Serial".to_string()));
            }
            ProtocolType::Sftp => {
                // Export SFTP as SSH with SFTP protocol marker
                Self::write_ssh_fields(&mut output, connection);
            }
            ProtocolType::Kubernetes => {
                return Err(ExportError::UnsupportedProtocol("Kubernetes".to_string()));
            }
        }

        Ok(output)
    }

    /// Writes SSH-specific fields to the output.
    fn write_ssh_fields(output: &mut String, connection: &Connection) {
        let _ = writeln!(output, "protocol=SSH");

        // Server with port
        if connection.port == 22 {
            let _ = writeln!(output, "server={}", connection.host);
        } else {
            let _ = writeln!(output, "server={}:{}", connection.host, connection.port);
        }

        // Username
        if let Some(ref username) = connection.username {
            let _ = writeln!(output, "username={username}");
        }

        // SSH-specific options
        if let ProtocolConfig::Ssh(ref ssh_config) = connection.protocol_config {
            // SSH private key
            if let Some(ref key_path) = ssh_config.key_path {
                let _ = writeln!(output, "ssh_privatekey={}", key_path.display());
            }

            // Auth method
            let auth_value = match ssh_config.auth_method {
                crate::models::SshAuthMethod::Password => "0",
                crate::models::SshAuthMethod::PublicKey
                | crate::models::SshAuthMethod::SecurityKey => "2",
                crate::models::SshAuthMethod::Agent => "3",
                crate::models::SshAuthMethod::KeyboardInteractive => "4",
            };
            let _ = writeln!(output, "ssh_auth={auth_value}");
        }
    }

    /// Writes RDP-specific fields to the output.
    fn write_rdp_fields(output: &mut String, connection: &Connection) {
        let _ = writeln!(output, "protocol=RDP");

        // Server with port
        if connection.port == 3389 {
            let _ = writeln!(output, "server={}", connection.host);
        } else {
            let _ = writeln!(output, "server={}:{}", connection.host, connection.port);
        }

        // Username
        if let Some(ref username) = connection.username {
            let _ = writeln!(output, "username={username}");
        }

        // Domain
        if let Some(ref domain) = connection.domain {
            let _ = writeln!(output, "domain={domain}");
        }

        // RDP-specific options
        if let ProtocolConfig::Rdp(ref rdp_config) = connection.protocol_config {
            // Resolution
            if let Some(ref resolution) = rdp_config.resolution {
                let _ = writeln!(
                    output,
                    "resolution={}x{}",
                    resolution.width, resolution.height
                );
            }

            // Color depth
            if let Some(color_depth) = rdp_config.color_depth {
                let _ = writeln!(output, "colordepth={color_depth}");
            }

            // Audio redirect
            if rdp_config.audio_redirect {
                let _ = writeln!(output, "sound=local");
            } else {
                let _ = writeln!(output, "sound=off");
            }
        }
    }

    /// Writes VNC-specific fields to the output.
    fn write_vnc_fields(output: &mut String, connection: &Connection) {
        let _ = writeln!(output, "protocol=VNC");

        // Server always includes port for VNC
        let _ = writeln!(output, "server={}:{}", connection.host, connection.port);

        // Username (if set)
        if let Some(ref username) = connection.username {
            let _ = writeln!(output, "username={username}");
        }

        // VNC-specific options
        if let ProtocolConfig::Vnc(ref vnc_config) = connection.protocol_config {
            // Encoding
            if let Some(ref encoding) = vnc_config.encoding {
                let _ = writeln!(output, "encodings={encoding}");
            }

            // Quality
            if let Some(quality) = vnc_config.quality {
                let _ = writeln!(output, "quality={quality}");
            }
        }
    }

    /// Generates a filename for a connection.
    ///
    /// The filename is based on the connection ID to ensure uniqueness.
    ///
    /// # Arguments
    ///
    /// * `connection` - The connection to generate a filename for
    ///
    /// # Returns
    ///
    /// A string containing the filename (without path).
    #[must_use]
    pub fn generate_filename(connection: &Connection) -> String {
        // Use connection ID for uniqueness, sanitize name for readability
        let sanitized_name: String = connection
            .name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();

        format!("{}_{}.remmina", sanitized_name, connection.id)
    }

    /// Exports all connections to a directory.
    ///
    /// Each connection is exported to a separate .remmina file.
    ///
    /// # Arguments
    ///
    /// * `connections` - The connections to export
    /// * `output_dir` - The directory to write files to
    ///
    /// # Returns
    ///
    /// An `ExportResult` with counts and output file paths.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or files cannot be written.
    pub fn export_to_directory(
        connections: &[Connection],
        output_dir: &Path,
    ) -> ExportResult2<ExportResult> {
        let mut result = ExportResult::new();

        // Create output directory if it doesn't exist
        if !output_dir.exists() {
            fs::create_dir_all(output_dir).map_err(|e| {
                ExportError::WriteError(format!(
                    "Failed to create directory {}: {}",
                    output_dir.display(),
                    e
                ))
            })?;
        }

        for connection in connections {
            match Self::export_connection(connection) {
                Ok(content) => {
                    let filename = Self::generate_filename(connection);
                    let file_path = output_dir.join(&filename);

                    match super::write_export_file(&file_path, &content) {
                        Ok(()) => {
                            result.increment_exported();
                            result.add_output_file(file_path);
                        }
                        Err(e) => {
                            result.increment_skipped();
                            result.add_warning(format!(
                                "Failed to write '{}': {}",
                                connection.name, e
                            ));
                        }
                    }
                }
                Err(ExportError::UnsupportedProtocol(proto)) => {
                    result.increment_skipped();
                    result.add_warning(format!(
                        "Skipped '{}': unsupported protocol {}",
                        connection.name, proto
                    ));
                }
                Err(e) => {
                    result.increment_skipped();
                    result.add_warning(format!("Failed to export '{}': {}", connection.name, e));
                }
            }
        }

        Ok(result)
    }
}

impl Default for RemminaExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ExportTarget for RemminaExporter {
    fn format_id(&self) -> ExportFormat {
        ExportFormat::Remmina
    }

    fn display_name(&self) -> &'static str {
        "Remmina"
    }

    fn export(
        &self,
        connections: &[Connection],
        _groups: &[ConnectionGroup],
        options: &ExportOptions,
    ) -> ExportResult2<ExportResult> {
        Self::export_to_directory(connections, &options.output_path)
    }

    fn export_connection(&self, connection: &Connection) -> ExportResult2<String> {
        Self::export_connection(connection)
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
    use crate::models::{Resolution, SshAuthMethod};
    use std::path::PathBuf;

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
    fn test_export_ssh_connection_basic() {
        let conn = create_ssh_connection("myserver", "192.168.1.100", 22);
        let content = RemminaExporter::export_connection(&conn).unwrap();

        assert!(content.contains("[remmina]"));
        assert!(content.contains("name=myserver"));
        assert!(content.contains("protocol=SSH"));
        assert!(content.contains("server=192.168.1.100"));
    }

    #[test]
    fn test_export_ssh_connection_with_custom_port() {
        let conn = create_ssh_connection("myserver", "192.168.1.100", 2222);
        let content = RemminaExporter::export_connection(&conn).unwrap();

        assert!(content.contains("server=192.168.1.100:2222"));
    }

    #[test]
    fn test_export_ssh_connection_with_username() {
        let conn = create_ssh_connection("myserver", "192.168.1.100", 22).with_username("admin");
        let content = RemminaExporter::export_connection(&conn).unwrap();

        assert!(content.contains("username=admin"));
    }

    #[test]
    fn test_export_ssh_connection_with_key() {
        let mut conn = create_ssh_connection("myserver", "192.168.1.100", 22);
        if let ProtocolConfig::Ssh(ref mut ssh_config) = conn.protocol_config {
            ssh_config.key_path = Some(PathBuf::from("/home/user/.ssh/id_rsa"));
            ssh_config.auth_method = SshAuthMethod::PublicKey;
        }
        let content = RemminaExporter::export_connection(&conn).unwrap();

        assert!(content.contains("ssh_privatekey=/home/user/.ssh/id_rsa"));
        assert!(content.contains("ssh_auth=2"));
    }

    #[test]
    fn test_export_rdp_connection_basic() {
        let conn = create_rdp_connection("windows", "192.168.1.50", 3389);
        let content = RemminaExporter::export_connection(&conn).unwrap();

        assert!(content.contains("[remmina]"));
        assert!(content.contains("name=windows"));
        assert!(content.contains("protocol=RDP"));
        assert!(content.contains("server=192.168.1.50"));
    }

    #[test]
    fn test_export_rdp_connection_with_domain() {
        let mut conn = create_rdp_connection("windows", "192.168.1.50", 3389);
        conn.domain = Some("MYDOMAIN".to_string());
        let content = RemminaExporter::export_connection(&conn).unwrap();

        assert!(content.contains("domain=MYDOMAIN"));
    }

    #[test]
    fn test_export_rdp_connection_with_resolution() {
        let mut conn = create_rdp_connection("windows", "192.168.1.50", 3389);
        if let ProtocolConfig::Rdp(ref mut rdp_config) = conn.protocol_config {
            rdp_config.resolution = Some(Resolution::new(1920, 1080));
            rdp_config.color_depth = Some(32);
        }
        let content = RemminaExporter::export_connection(&conn).unwrap();

        assert!(content.contains("resolution=1920x1080"));
        assert!(content.contains("colordepth=32"));
    }

    #[test]
    fn test_export_vnc_connection_basic() {
        let conn = create_vnc_connection("vnc-desktop", "192.168.1.75", 5900);
        let content = RemminaExporter::export_connection(&conn).unwrap();

        assert!(content.contains("[remmina]"));
        assert!(content.contains("name=vnc-desktop"));
        assert!(content.contains("protocol=VNC"));
        assert!(content.contains("server=192.168.1.75:5900"));
    }

    #[test]
    fn test_export_vnc_connection_with_custom_port() {
        let conn = create_vnc_connection("vnc-desktop", "192.168.1.75", 5901);
        let content = RemminaExporter::export_connection(&conn).unwrap();

        assert!(content.contains("server=192.168.1.75:5901"));
    }

    #[test]
    fn test_export_spice_connection_fails() {
        let conn = Connection::new_spice("spice-vm".to_string(), "192.168.1.100".to_string(), 5900);
        let result = RemminaExporter::export_connection(&conn);

        assert!(result.is_err());
        if let Err(ExportError::UnsupportedProtocol(proto)) = result {
            assert_eq!(proto, "SPICE");
        } else {
            panic!("Expected UnsupportedProtocol error");
        }
    }

    #[test]
    fn test_generate_filename() {
        let conn = create_ssh_connection("My Server", "192.168.1.100", 22);
        let filename = RemminaExporter::generate_filename(&conn);

        assert!(filename.starts_with("My_Server_"));
        assert!(filename.ends_with(".remmina"));
        assert!(filename.contains(&conn.id.to_string()));
    }

    #[test]
    fn test_generate_filename_special_chars() {
        let conn = create_ssh_connection("Server@Home!", "192.168.1.100", 22);
        let filename = RemminaExporter::generate_filename(&conn);

        assert!(filename.starts_with("Server_Home_"));
        assert!(!filename.contains('@'));
        assert!(!filename.contains('!'));
    }

    #[test]
    fn test_supports_protocol() {
        let exporter = RemminaExporter::new();
        assert!(exporter.supports_protocol(&ProtocolType::Ssh));
        assert!(exporter.supports_protocol(&ProtocolType::Rdp));
        assert!(exporter.supports_protocol(&ProtocolType::Vnc));
        assert!(!exporter.supports_protocol(&ProtocolType::Spice));
    }

    #[test]
    fn test_export_to_directory() {
        let temp_dir = tempfile::tempdir().unwrap();
        let connections = vec![
            create_ssh_connection("ssh-server", "192.168.1.1", 22),
            create_rdp_connection("rdp-server", "192.168.1.2", 3389),
            create_vnc_connection("vnc-server", "192.168.1.3", 5900),
        ];

        let result = RemminaExporter::export_to_directory(&connections, temp_dir.path()).unwrap();

        assert_eq!(result.exported_count, 3);
        assert_eq!(result.skipped_count, 0);
        assert_eq!(result.output_files.len(), 3);

        // Verify files exist
        for file_path in &result.output_files {
            assert!(file_path.exists());
        }
    }

    #[test]
    fn test_export_to_directory_skips_spice() {
        let temp_dir = tempfile::tempdir().unwrap();
        let connections = vec![
            create_ssh_connection("ssh-server", "192.168.1.1", 22),
            Connection::new_spice("spice-vm".to_string(), "192.168.1.2".to_string(), 5900),
        ];

        let result = RemminaExporter::export_to_directory(&connections, temp_dir.path()).unwrap();

        assert_eq!(result.exported_count, 1);
        assert_eq!(result.skipped_count, 1);
        assert!(result.has_warnings());
    }
}
