//! Royal TS (.rtsz) XML exporter.
//!
//! Exports `RustConn` connections to Royal TS XML format (.rtsz).
//! Supports SSH, RDP, and VNC connections with folder hierarchy.

use std::collections::HashMap;
use std::fmt::Write;
use std::fs;

use tracing::{debug, info_span};
use uuid::Uuid;

use crate::models::{Connection, ConnectionGroup, ProtocolConfig, ProtocolType};
use crate::tracing::span_names;

use super::{ExportError, ExportFormat, ExportOptions, ExportResult, ExportResult2, ExportTarget};

/// Royal TS XML exporter.
///
/// Exports connections to Royal TS .rtsz XML format.
/// Supports SSH, RDP, and VNC protocols with folder hierarchy.
pub struct RoyalTsExporter;

impl RoyalTsExporter {
    /// Creates a new Royal TS exporter
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Exports connections to Royal TS XML format.
    ///
    /// # Arguments
    ///
    /// * `connections` - The connections to export
    /// * `groups` - The connection groups for hierarchy
    ///
    /// # Returns
    ///
    /// A string containing the Royal TS XML formatted content.
    #[must_use]
    pub fn export_to_xml(connections: &[Connection], groups: &[ConnectionGroup]) -> String {
        let _span = info_span!(
            span_names::EXPORT_EXECUTE,
            format = "royalts",
            connection_count = connections.len()
        )
        .entered();

        let mut output = String::new();
        output.push_str(r#"<?xml version="1.0" encoding="utf-8"?>"#);
        output.push('\n');
        output.push_str("<RoyalDocument>\n");

        // Build group ID mapping (RustConn UUID -> Royal TS ID)
        let group_map: HashMap<Uuid, String> = groups
            .iter()
            .map(|g| (g.id, Uuid::new_v4().to_string()))
            .collect();

        // Export folders (groups)
        for group in groups {
            Self::write_folder(&mut output, group, &group_map);
        }

        // Export connections
        let mut exported_count = 0;
        for conn in connections {
            match conn.protocol {
                ProtocolType::Ssh => {
                    Self::write_ssh_connection(&mut output, conn, &group_map);
                    exported_count += 1;
                }
                ProtocolType::Rdp => {
                    Self::write_rdp_connection(&mut output, conn, &group_map);
                    exported_count += 1;
                }
                ProtocolType::Vnc => {
                    Self::write_vnc_connection(&mut output, conn, &group_map);
                    exported_count += 1;
                }
                _ => {}
            }
        }

        output.push_str("</RoyalDocument>\n");

        debug!(exported = exported_count, "Royal TS export completed");
        output
    }

    fn write_folder(
        output: &mut String,
        group: &ConnectionGroup,
        group_map: &HashMap<Uuid, String>,
    ) {
        let id = group_map
            .get(&group.id)
            .map_or_else(|| Uuid::new_v4().to_string(), std::clone::Clone::clone);

        output.push_str("  <RoyalFolder>\n");
        let _ = writeln!(output, "    <ID>{id}</ID>");
        let _ = writeln!(output, "    <Name>{}</Name>", escape_xml(&group.name));

        if let Some(parent_id) = group.parent_id {
            if let Some(parent_royal_id) = group_map.get(&parent_id) {
                let _ = writeln!(output, "    <ParentID>{parent_royal_id}</ParentID>");
            }
        }

        output.push_str("  </RoyalFolder>\n");
    }

    fn write_ssh_connection(
        output: &mut String,
        conn: &Connection,
        group_map: &HashMap<Uuid, String>,
    ) {
        let id = Uuid::new_v4().to_string();

        output.push_str("  <RoyalSSHConnection>\n");
        let _ = writeln!(output, "    <ID>{id}</ID>");
        let _ = writeln!(output, "    <Name>{}</Name>", escape_xml(&conn.name));
        let _ = writeln!(output, "    <URI>{}</URI>", escape_xml(&conn.host));
        let _ = writeln!(output, "    <Port>{}</Port>", conn.port);

        if let Some(group_id) = conn.group_id {
            if let Some(royal_group_id) = group_map.get(&group_id) {
                let _ = writeln!(output, "    <ParentID>{royal_group_id}</ParentID>");
            }
        }

        if let Some(ref username) = conn.username {
            let _ = writeln!(
                output,
                "    <CredentialUsername>{}</CredentialUsername>",
                escape_xml(username)
            );
        }

        // SSH-specific options
        if let ProtocolConfig::Ssh(ref ssh_config) = conn.protocol_config {
            if let Some(ref key_path) = ssh_config.key_path {
                let _ = writeln!(
                    output,
                    "    <PrivateKeyFile>{}</PrivateKeyFile>",
                    escape_xml(&key_path.display().to_string())
                );
            }
        }

        output.push_str("  </RoyalSSHConnection>\n");
    }

    fn write_rdp_connection(
        output: &mut String,
        conn: &Connection,
        group_map: &HashMap<Uuid, String>,
    ) {
        let id = Uuid::new_v4().to_string();

        output.push_str("  <RoyalRDPConnection>\n");
        let _ = writeln!(output, "    <ID>{id}</ID>");
        let _ = writeln!(output, "    <Name>{}</Name>", escape_xml(&conn.name));
        let _ = writeln!(output, "    <URI>{}</URI>", escape_xml(&conn.host));
        let _ = writeln!(output, "    <Port>{}</Port>", conn.port);

        if let Some(group_id) = conn.group_id {
            if let Some(royal_group_id) = group_map.get(&group_id) {
                let _ = writeln!(output, "    <ParentID>{royal_group_id}</ParentID>");
            }
        }

        if let Some(ref username) = conn.username {
            let _ = writeln!(
                output,
                "    <CredentialUsername>{}</CredentialUsername>",
                escape_xml(username)
            );
        }

        if let Some(ref domain) = conn.domain {
            let _ = writeln!(
                output,
                "    <CredentialDomain>{}</CredentialDomain>",
                escape_xml(domain)
            );
        }

        // RDP-specific options
        if let ProtocolConfig::Rdp(ref rdp_config) = conn.protocol_config {
            if let Some(ref resolution) = rdp_config.resolution {
                let _ = writeln!(
                    output,
                    "    <DesktopWidth>{}</DesktopWidth>",
                    resolution.width
                );
                let _ = writeln!(
                    output,
                    "    <DesktopHeight>{}</DesktopHeight>",
                    resolution.height
                );
            }
            if let Some(ref gateway) = rdp_config.gateway {
                let _ = writeln!(
                    output,
                    "    <RDGatewayHost>{}</RDGatewayHost>",
                    escape_xml(&gateway.hostname)
                );
            }
        }

        output.push_str("  </RoyalRDPConnection>\n");
    }

    fn write_vnc_connection(
        output: &mut String,
        conn: &Connection,
        group_map: &HashMap<Uuid, String>,
    ) {
        let id = Uuid::new_v4().to_string();

        output.push_str("  <RoyalVNCConnection>\n");
        let _ = writeln!(output, "    <ID>{id}</ID>");
        let _ = writeln!(output, "    <Name>{}</Name>", escape_xml(&conn.name));
        let _ = writeln!(output, "    <URI>{}</URI>", escape_xml(&conn.host));
        let _ = writeln!(output, "    <VNCPort>{}</VNCPort>", conn.port);

        if let Some(group_id) = conn.group_id {
            if let Some(royal_group_id) = group_map.get(&group_id) {
                let _ = writeln!(output, "    <ParentID>{royal_group_id}</ParentID>");
            }
        }

        // VNC-specific options
        if let ProtocolConfig::Vnc(ref vnc_config) = conn.protocol_config {
            if vnc_config.view_only {
                output.push_str("    <ViewOnly>true</ViewOnly>\n");
            }
        }

        output.push_str("  </RoyalVNCConnection>\n");
    }
}

impl Default for RoyalTsExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ExportTarget for RoyalTsExporter {
    fn format_id(&self) -> ExportFormat {
        ExportFormat::RoyalTs
    }

    fn display_name(&self) -> &'static str {
        "Royal TS"
    }

    fn export(
        &self,
        connections: &[Connection],
        groups: &[ConnectionGroup],
        options: &ExportOptions,
    ) -> ExportResult2<ExportResult> {
        let mut result = ExportResult::new();

        // Filter supported connections and count skipped
        let supported_connections: Vec<&Connection> = connections
            .iter()
            .filter(|c| {
                if matches!(
                    c.protocol,
                    ProtocolType::Ssh | ProtocolType::Rdp | ProtocolType::Vnc
                ) {
                    true
                } else {
                    result.increment_skipped();
                    result.add_warning(format!(
                        "Skipped unsupported connection '{}' (protocol: {})",
                        c.name, c.protocol
                    ));
                    false
                }
            })
            .collect();

        // Generate content
        let content = Self::export_to_xml(
            &supported_connections
                .iter()
                .copied()
                .cloned()
                .collect::<Vec<_>>(),
            groups,
        );

        // Write to file
        fs::write(&options.output_path, &content).map_err(|e| {
            ExportError::WriteError(format!(
                "Failed to write to {}: {}",
                options.output_path.display(),
                e
            ))
        })?;

        result.exported_count = supported_connections.len();
        result.add_output_file(options.output_path.clone());

        Ok(result)
    }

    fn export_connection(&self, connection: &Connection) -> ExportResult2<String> {
        if !matches!(
            connection.protocol,
            ProtocolType::Ssh | ProtocolType::Rdp | ProtocolType::Vnc
        ) {
            return Err(ExportError::UnsupportedProtocol(format!(
                "{}",
                connection.protocol
            )));
        }

        let mut output = String::new();
        let group_map = HashMap::new();

        match connection.protocol {
            ProtocolType::Ssh => Self::write_ssh_connection(&mut output, connection, &group_map),
            ProtocolType::Rdp => Self::write_rdp_connection(&mut output, connection, &group_map),
            ProtocolType::Vnc => Self::write_vnc_connection(&mut output, connection, &group_map),
            _ => {}
        }

        Ok(output)
    }

    fn supports_protocol(&self, protocol: &ProtocolType) -> bool {
        matches!(
            protocol,
            ProtocolType::Ssh | ProtocolType::Rdp | ProtocolType::Vnc
        )
    }
}

/// Escapes special XML characters in a string.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_export_ssh_connection() {
        let conn = create_ssh_connection("myserver", "192.168.1.100", 22);
        let output = RoyalTsExporter::export_to_xml(&[conn], &[]);

        assert!(output.contains("<RoyalSSHConnection>"));
        assert!(output.contains("<Name>myserver</Name>"));
        assert!(output.contains("<URI>192.168.1.100</URI>"));
        assert!(output.contains("<Port>22</Port>"));
    }

    #[test]
    fn test_export_rdp_connection() {
        let conn = create_rdp_connection("rdp-server", "192.168.1.200", 3389);
        let output = RoyalTsExporter::export_to_xml(&[conn], &[]);

        assert!(output.contains("<RoyalRDPConnection>"));
        assert!(output.contains("<Name>rdp-server</Name>"));
        assert!(output.contains("<URI>192.168.1.200</URI>"));
        assert!(output.contains("<Port>3389</Port>"));
    }

    #[test]
    fn test_export_vnc_connection() {
        let conn = create_vnc_connection("vnc-server", "192.168.1.150", 5900);
        let output = RoyalTsExporter::export_to_xml(&[conn], &[]);

        assert!(output.contains("<RoyalVNCConnection>"));
        assert!(output.contains("<Name>vnc-server</Name>"));
        assert!(output.contains("<URI>192.168.1.150</URI>"));
        assert!(output.contains("<VNCPort>5900</VNCPort>"));
    }

    #[test]
    fn test_export_with_username() {
        let conn = create_ssh_connection("myserver", "192.168.1.100", 22).with_username("admin");
        let output = RoyalTsExporter::export_to_xml(&[conn], &[]);

        assert!(output.contains("<CredentialUsername>admin</CredentialUsername>"));
    }

    #[test]
    fn test_export_with_groups() {
        let group = ConnectionGroup::new("Production".to_string());
        let group_id = group.id;

        let mut conn = create_ssh_connection("server1", "192.168.1.1", 22);
        conn.group_id = Some(group_id);

        let output = RoyalTsExporter::export_to_xml(&[conn], &[group]);

        assert!(output.contains("<RoyalFolder>"));
        assert!(output.contains("<Name>Production</Name>"));
        assert!(output.contains("<ParentID>"));
    }

    #[test]
    fn test_escape_xml() {
        assert_eq!(escape_xml("test"), "test");
        assert_eq!(escape_xml("<script>"), "&lt;script&gt;");
        assert_eq!(escape_xml("a & b"), "a &amp; b");
        assert_eq!(escape_xml("\"quoted\""), "&quot;quoted&quot;");
    }

    #[test]
    fn test_supports_protocol() {
        let exporter = RoyalTsExporter::new();
        assert!(exporter.supports_protocol(&ProtocolType::Ssh));
        assert!(exporter.supports_protocol(&ProtocolType::Rdp));
        assert!(exporter.supports_protocol(&ProtocolType::Vnc));
        assert!(!exporter.supports_protocol(&ProtocolType::Spice));
    }

    #[test]
    fn test_export_multiple_connections() {
        let connections = vec![
            create_ssh_connection("ssh1", "192.168.1.1", 22),
            create_rdp_connection("rdp1", "192.168.1.2", 3389),
            create_vnc_connection("vnc1", "192.168.1.3", 5900),
        ];
        let output = RoyalTsExporter::export_to_xml(&connections, &[]);

        assert!(output.contains("<RoyalSSHConnection>"));
        assert!(output.contains("<RoyalRDPConnection>"));
        assert!(output.contains("<RoyalVNCConnection>"));
    }
}
