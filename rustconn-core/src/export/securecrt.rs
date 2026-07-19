//! SecureCRT session file exporter.
//!
//! Exports `RustConn` connections to SecureCRT `.ini` session file format.
//! Each connection becomes a separate `.ini` file in a directory tree
//! that mirrors the SecureCRT `Config/Sessions/` structure.
//!
//! Supports SSH, Telnet, RDP, and VNC connection types.

use std::collections::HashMap;
use std::fmt::Write;
use std::path::Path;

use uuid::Uuid;

use super::{
    ExportError, ExportFormat, ExportOperationResult, ExportOptions, ExportResult, ExportTarget,
};
use crate::models::{Connection, ConnectionGroup, ProtocolConfig, ProtocolType, SshAuthMethod};

/// SecureCRT session file exporter.
///
/// Exports connections as individual `.ini` files in a directory hierarchy
/// matching SecureCRT's `Config/Sessions/` layout.
pub struct SecureCrtExporter;

impl SecureCrtExporter {
    /// Creates a new SecureCRT exporter.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Formats a DWORD value in SecureCRT hex format (e.g., 22 → "00000016").
    fn format_dword(value: u32) -> String {
        format!("{value:08x}")
    }

    /// Sanitizes a session name for use as a filename.
    /// SecureCRT doesn't allow certain characters in session names.
    fn sanitize_filename(name: &str) -> String {
        name.chars()
            .map(|c| match c {
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                _ => c,
            })
            .collect()
    }

    /// Exports a single SSH connection to SecureCRT INI format.
    fn export_ssh_ini(connection: &Connection) -> String {
        let mut output = String::new();

        let _ = writeln!(output, r#"D:"Is Session"=00000001"#);
        let _ = writeln!(output, r#"S:"Protocol Name"=SSH2"#);
        let _ = writeln!(output, r#"S:"Hostname"={}"#, connection.host);
        let _ = writeln!(
            output,
            r#"D:"[SSH2] Port"={}"#,
            Self::format_dword(u32::from(connection.port))
        );

        if let Some(ref username) = connection.username {
            let _ = writeln!(output, r#"S:"Username"={username}"#);
        } else {
            let _ = writeln!(output, r#"S:"Username"="#);
        }

        if let ProtocolConfig::Ssh(ref ssh_config) = connection.protocol_config {
            // Identity file
            if let Some(ref key_path) = ssh_config.key_path {
                let _ = writeln!(output, r#"S:"Identity Filename V2"={}"#, key_path.display());
            }

            // Auth methods
            let auth_str = match ssh_config.auth_method {
                SshAuthMethod::PublicKey => "publickey,password,keyboard-interactive",
                SshAuthMethod::Password => "password,publickey,keyboard-interactive",
                SshAuthMethod::Agent => "publickey,password,keyboard-interactive,gssapi",
                SshAuthMethod::KeyboardInteractive => "keyboard-interactive,password,publickey",
                SshAuthMethod::SecurityKey => "password,publickey,keyboard-interactive",
            };
            let _ = writeln!(output, r#"S:"SSH2 Authentications V2"={auth_str}"#);

            // X11 forwarding
            let _ = writeln!(
                output,
                r#"D:"Forward X11"={}"#,
                Self::format_dword(u32::from(ssh_config.x11_forwarding))
            );

            // Agent forwarding (1=enabled, 2=use global)
            let agent_val = if ssh_config.agent_forwarding { 1 } else { 2 };
            let _ = writeln!(
                output,
                r#"D:"Enable Agent Forwarding"={}"#,
                Self::format_dword(agent_val)
            );

            // Compression
            let compression_str = if ssh_config.compression {
                "zlib@openssh.com,zlib,none"
            } else {
                "none"
            };
            let _ = writeln!(output, r#"S:"Compression List"={compression_str}"#);
        }

        // Description
        if let Some(ref desc) = connection.description {
            let escaped = desc.replace('\n', "\\r");
            let _ = writeln!(output, r#"S:"Description"={escaped}"#);
        }

        // Default emulation
        let _ = writeln!(output, r#"S:"Emulation"=XTerm"#);

        // Common defaults
        let _ = writeln!(output, r#"D:"ANSI Color"=00000001"#);
        let _ = writeln!(output, r#"D:"Scrollback"=00001000"#);

        output
    }

    /// Exports a single Telnet connection to SecureCRT INI format.
    fn export_telnet_ini(connection: &Connection) -> String {
        let mut output = String::new();

        let _ = writeln!(output, r#"D:"Is Session"=00000001"#);
        let _ = writeln!(output, r#"S:"Protocol Name"=Telnet"#);
        let _ = writeln!(output, r#"S:"Hostname"={}"#, connection.host);
        let _ = writeln!(
            output,
            r#"D:"Port"={}"#,
            Self::format_dword(u32::from(connection.port))
        );

        if let Some(ref username) = connection.username {
            let _ = writeln!(output, r#"S:"Username"={username}"#);
        }

        if let Some(ref desc) = connection.description {
            let escaped = desc.replace('\n', "\\r");
            let _ = writeln!(output, r#"S:"Description"={escaped}"#);
        }

        let _ = writeln!(output, r#"S:"Emulation"=VT100"#);
        let _ = writeln!(output, r#"D:"ANSI Color"=00000001"#);

        output
    }

    /// Exports a single RDP connection to SecureCRT INI format.
    fn export_rdp_ini(connection: &Connection) -> String {
        let mut output = String::new();

        let _ = writeln!(output, r#"D:"Is Session"=00000001"#);
        let _ = writeln!(output, r#"S:"Protocol Name"=RDP"#);
        let _ = writeln!(output, r#"S:"Hostname"={}"#, connection.host);
        let _ = writeln!(
            output,
            r#"D:"Port"={}"#,
            Self::format_dword(u32::from(connection.port))
        );

        if let Some(ref username) = connection.username {
            let _ = writeln!(output, r#"S:"Username"={username}"#);
        }

        if let Some(ref desc) = connection.description {
            let escaped = desc.replace('\n', "\\r");
            let _ = writeln!(output, r#"S:"Description"={escaped}"#);
        }

        output
    }

    /// Exports a single VNC connection to SecureCRT INI format.
    fn export_vnc_ini(connection: &Connection) -> String {
        let mut output = String::new();

        let _ = writeln!(output, r#"D:"Is Session"=00000001"#);
        let _ = writeln!(output, r#"S:"Protocol Name"=VNC"#);
        let _ = writeln!(output, r#"S:"Hostname"={}"#, connection.host);
        let _ = writeln!(
            output,
            r#"D:"Port"={}"#,
            Self::format_dword(u32::from(connection.port))
        );

        if let Some(ref desc) = connection.description {
            let escaped = desc.replace('\n', "\\r");
            let _ = writeln!(output, r#"S:"Description"={escaped}"#);
        }

        output
    }

    /// Builds full directory paths for each group by walking the parent chain.
    ///
    /// Produces a forward-slash separated path (e.g. `Production/Web`) with each
    /// segment individually sanitized for the filesystem. Walking the parent
    /// chain (rather than a single-pass update) keeps paths correct for folders
    /// nested more than two levels deep regardless of the order groups appear in
    /// the input slice. A cycle guard prevents infinite loops on malformed
    /// `parent_id` references.
    fn build_group_hierarchy(groups: &[ConnectionGroup]) -> HashMap<Uuid, String> {
        let by_id: HashMap<Uuid, &ConnectionGroup> = groups.iter().map(|g| (g.id, g)).collect();
        let mut paths: HashMap<Uuid, String> = HashMap::with_capacity(groups.len());

        for group in groups {
            let mut segments = vec![Self::sanitize_filename(&group.name)];
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
                segments.push(Self::sanitize_filename(&parent.name));
                current = parent;
            }

            segments.reverse();
            paths.insert(group.id, segments.join("/"));
        }

        paths
    }

    /// Exports all connections to SecureCRT directory format.
    ///
    /// # Errors
    ///
    /// Returns an error if the output directory cannot be created or files cannot be written.
    pub fn export_to_directory(
        connections: &[Connection],
        groups: &[ConnectionGroup],
        output_path: &Path,
    ) -> ExportOperationResult<ExportResult> {
        let mut result = ExportResult::new();

        // Create output directory
        std::fs::create_dir_all(output_path).map_err(|e| {
            ExportError::WriteError(format!(
                "Failed to create directory {}: {}",
                output_path.display(),
                e
            ))
        })?;

        // Build group paths
        let group_paths = Self::build_group_hierarchy(groups);

        // Create a directory for every group, including intermediate folders that
        // hold no direct sessions, so the full folder tree round-trips even when
        // some folders are empty.
        for path in group_paths.values() {
            if path.is_empty() {
                continue;
            }
            let group_dir = output_path.join(path);
            if !group_dir.exists() {
                std::fs::create_dir_all(&group_dir).map_err(|e| {
                    ExportError::WriteError(format!(
                        "Failed to create directory {}: {}",
                        group_dir.display(),
                        e
                    ))
                })?;
            }
        }

        for conn in connections {
            let ini_content = match conn.protocol {
                ProtocolType::Ssh => Self::export_ssh_ini(conn),
                ProtocolType::Telnet => Self::export_telnet_ini(conn),
                ProtocolType::Rdp => Self::export_rdp_ini(conn),
                ProtocolType::Vnc => Self::export_vnc_ini(conn),
                _ => {
                    result.increment_skipped();
                    result.add_warning(format!(
                        "Skipped '{}': unsupported protocol {:?}",
                        conn.name, conn.protocol
                    ));
                    continue;
                }
            };

            // Determine output path based on group
            let relative_dir = conn
                .group_id
                .and_then(|id| group_paths.get(&id).cloned())
                .unwrap_or_default();

            let session_dir = if relative_dir.is_empty() {
                output_path.to_path_buf()
            } else {
                output_path.join(&relative_dir)
            };

            // Create subdirectory if needed
            if !session_dir.exists() {
                std::fs::create_dir_all(&session_dir).map_err(|e| {
                    ExportError::WriteError(format!(
                        "Failed to create directory {}: {}",
                        session_dir.display(),
                        e
                    ))
                })?;
            }

            // Write .ini file
            let filename = format!("{}.ini", Self::sanitize_filename(&conn.name));
            let file_path = session_dir.join(&filename);

            super::write_export_file(&file_path, &ini_content)?;

            result.increment_exported();
            result.add_output_file(file_path);
        }

        Ok(result)
    }

    /// Exports a single connection to INI string.
    fn export_connection_ini(connection: &Connection) -> Result<String, ExportError> {
        match connection.protocol {
            ProtocolType::Ssh => Ok(Self::export_ssh_ini(connection)),
            ProtocolType::Telnet => Ok(Self::export_telnet_ini(connection)),
            ProtocolType::Rdp => Ok(Self::export_rdp_ini(connection)),
            ProtocolType::Vnc => Ok(Self::export_vnc_ini(connection)),
            _ => Err(ExportError::UnsupportedProtocol(format!(
                "{:?}",
                connection.protocol
            ))),
        }
    }
}

impl Default for SecureCrtExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ExportTarget for SecureCrtExporter {
    fn format_id(&self) -> ExportFormat {
        ExportFormat::SecureCrt
    }

    fn display_name(&self) -> &'static str {
        "SecureCRT"
    }

    fn export(
        &self,
        connections: &[Connection],
        groups: &[ConnectionGroup],
        options: &ExportOptions,
    ) -> ExportOperationResult<ExportResult> {
        Self::export_to_directory(connections, groups, &options.output_path)
    }

    fn export_connection(&self, connection: &Connection) -> ExportOperationResult<String> {
        Self::export_connection_ini(connection)
    }

    fn supports_protocol(&self, protocol: &ProtocolType) -> bool {
        matches!(
            protocol,
            ProtocolType::Ssh | ProtocolType::Telnet | ProtocolType::Rdp | ProtocolType::Vnc
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProtocolConfig, RdpConfig, SshConfig, TelnetConfig, VncConfig};

    #[test]
    fn test_format_dword() {
        assert_eq!(SecureCrtExporter::format_dword(22), "00000016");
        assert_eq!(SecureCrtExporter::format_dword(3389), "00000d3d");
        assert_eq!(SecureCrtExporter::format_dword(23), "00000017");
        assert_eq!(SecureCrtExporter::format_dword(5900), "0000170c");
        assert_eq!(SecureCrtExporter::format_dword(0), "00000000");
        assert_eq!(SecureCrtExporter::format_dword(1), "00000001");
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(SecureCrtExporter::sanitize_filename("normal"), "normal");
        assert_eq!(
            SecureCrtExporter::sanitize_filename("has/slash"),
            "has_slash"
        );
        assert_eq!(
            SecureCrtExporter::sanitize_filename("has:colon"),
            "has_colon"
        );
        assert_eq!(
            SecureCrtExporter::sanitize_filename("a*b?c\"d<e>f|g"),
            "a_b_c_d_e_f_g"
        );
    }

    #[test]
    fn test_export_ssh_connection() {
        let ssh_config = SshConfig {
            auth_method: SshAuthMethod::PublicKey,
            key_path: Some(std::path::PathBuf::from("/home/user/.ssh/id_rsa")),
            x11_forwarding: true,
            agent_forwarding: true,
            compression: true,
            ..Default::default()
        };

        let mut conn = Connection::new(
            "Test Server".to_string(),
            "192.168.1.100".to_string(),
            22,
            ProtocolConfig::Ssh(ssh_config),
        );
        conn.username = Some("admin".to_string());
        conn.description = Some("Production\nserver".to_string());

        let ini = SecureCrtExporter::export_ssh_ini(&conn);

        assert!(ini.contains(r#"S:"Protocol Name"=SSH2"#));
        assert!(ini.contains(r#"S:"Hostname"=192.168.1.100"#));
        assert!(ini.contains(r#"D:"[SSH2] Port"=00000016"#));
        assert!(ini.contains(r#"S:"Username"=admin"#));
        assert!(ini.contains(r#"S:"Identity Filename V2"=/home/user/.ssh/id_rsa"#));
        assert!(ini.contains(r#"D:"Forward X11"=00000001"#));
        assert!(ini.contains(r#"D:"Enable Agent Forwarding"=00000001"#));
        assert!(ini.contains("zlib"));
        assert!(ini.contains(r#"S:"Description"=Production\rserver"#));
    }

    #[test]
    fn test_export_telnet_connection() {
        let conn = Connection::new(
            "Switch".to_string(),
            "10.0.0.1".to_string(),
            23,
            ProtocolConfig::Telnet(TelnetConfig::default()),
        );

        let ini = SecureCrtExporter::export_telnet_ini(&conn);

        assert!(ini.contains(r#"S:"Protocol Name"=Telnet"#));
        assert!(ini.contains(r#"S:"Hostname"=10.0.0.1"#));
        assert!(ini.contains(r#"D:"Port"=00000017"#));
        assert!(ini.contains(r#"S:"Emulation"=VT100"#));
    }

    #[test]
    fn test_export_rdp_connection() {
        let mut conn = Connection::new(
            "Windows".to_string(),
            "win.local".to_string(),
            3389,
            ProtocolConfig::Rdp(RdpConfig::default()),
        );
        conn.username = Some("Administrator".to_string());

        let ini = SecureCrtExporter::export_rdp_ini(&conn);

        assert!(ini.contains(r#"S:"Protocol Name"=RDP"#));
        assert!(ini.contains(r#"S:"Hostname"=win.local"#));
        assert!(ini.contains(r#"D:"Port"=00000d3d"#));
        assert!(ini.contains(r#"S:"Username"=Administrator"#));
    }

    #[test]
    fn test_export_vnc_connection() {
        let conn = Connection::new(
            "Desktop".to_string(),
            "vnc.host".to_string(),
            5900,
            ProtocolConfig::Vnc(VncConfig::default()),
        );

        let ini = SecureCrtExporter::export_vnc_ini(&conn);

        assert!(ini.contains(r#"S:"Protocol Name"=VNC"#));
        assert!(ini.contains(r#"S:"Hostname"=vnc.host"#));
        assert!(ini.contains(r#"D:"Port"=0000170c"#));
    }

    #[test]
    fn test_export_to_directory() {
        let dir = tempfile::tempdir().unwrap();
        let output_path = dir.path().join("Sessions");

        let group = ConnectionGroup::new("Production".to_string());
        let group_id = group.id;

        let mut conn1 = Connection::new(
            "Server1".to_string(),
            "10.0.0.1".to_string(),
            22,
            ProtocolConfig::Ssh(SshConfig::default()),
        );
        conn1.group_id = Some(group_id);

        let conn2 = Connection::new(
            "Router".to_string(),
            "10.0.0.2".to_string(),
            23,
            ProtocolConfig::Telnet(TelnetConfig::default()),
        );

        let result =
            SecureCrtExporter::export_to_directory(&[conn1, conn2], &[group], &output_path)
                .unwrap();

        assert_eq!(result.exported_count, 2);
        assert_eq!(result.skipped_count, 0);

        // Check files exist
        assert!(output_path.join("Production").join("Server1.ini").exists());
        assert!(output_path.join("Router.ini").exists());
    }

    #[test]
    fn test_export_deeply_nested_folders() {
        let dir = tempfile::tempdir().unwrap();
        let output_path = dir.path().join("Sessions");

        // Three levels deep: Production -> Web -> Frontend
        let production = ConnectionGroup::new("Production".to_string());
        let web = ConnectionGroup::with_parent("Web".to_string(), production.id);
        let frontend = ConnectionGroup::with_parent("Frontend".to_string(), web.id);

        let mut conn = Connection::new(
            "web01".to_string(),
            "10.0.1.2".to_string(),
            22,
            ProtocolConfig::Ssh(SshConfig::default()),
        );
        conn.group_id = Some(frontend.id);

        // Pass groups deepest-first to expose order-dependent path building.
        let groups = vec![frontend, web, production];

        let result =
            SecureCrtExporter::export_to_directory(&[conn], &groups, &output_path).unwrap();

        assert_eq!(result.exported_count, 1);

        // The session must land in the full path, not a truncated one.
        let full = output_path.join("Production").join("Web").join("Frontend");
        assert!(
            full.join("web01.ini").exists(),
            "connection should be written under the full nested path"
        );
        // Truncated path must NOT exist.
        assert!(
            !output_path
                .join("Web")
                .join("Frontend")
                .join("web01.ini")
                .exists(),
            "no truncated path should be created"
        );
    }

    #[test]
    fn test_export_creates_empty_intermediate_folders() {
        let dir = tempfile::tempdir().unwrap();
        let output_path = dir.path().join("Sessions");

        // "Staging" has no direct sessions but is an intermediate folder.
        let staging = ConnectionGroup::new("Staging".to_string());
        let db = ConnectionGroup::with_parent("DB".to_string(), staging.id);

        let mut conn = Connection::new(
            "db01".to_string(),
            "10.0.2.2".to_string(),
            22,
            ProtocolConfig::Ssh(SshConfig::default()),
        );
        conn.group_id = Some(db.id);

        SecureCrtExporter::export_to_directory(&[conn], &[staging, db], &output_path).unwrap();

        // Both the intermediate and leaf directories must exist.
        assert!(output_path.join("Staging").is_dir());
        assert!(output_path.join("Staging").join("DB").is_dir());
    }

    #[test]
    fn test_unsupported_protocol_skipped() {
        use crate::models::SpiceConfig;

        let dir = tempfile::tempdir().unwrap();
        let output_path = dir.path().join("Sessions");

        let conn = Connection::new(
            "SPICE VM".to_string(),
            "vm.host".to_string(),
            5900,
            ProtocolConfig::Spice(SpiceConfig::default()),
        );

        let result = SecureCrtExporter::export_to_directory(&[conn], &[], &output_path).unwrap();

        assert_eq!(result.exported_count, 0);
        assert_eq!(result.skipped_count, 1);
        assert!(result.warnings[0].contains("unsupported protocol"));
    }

    #[test]
    fn test_supports_protocol() {
        let exporter = SecureCrtExporter::new();
        assert!(exporter.supports_protocol(&ProtocolType::Ssh));
        assert!(exporter.supports_protocol(&ProtocolType::Telnet));
        assert!(exporter.supports_protocol(&ProtocolType::Rdp));
        assert!(exporter.supports_protocol(&ProtocolType::Vnc));
        assert!(!exporter.supports_protocol(&ProtocolType::Spice));
        assert!(!exporter.supports_protocol(&ProtocolType::ZeroTrust));
    }
}
