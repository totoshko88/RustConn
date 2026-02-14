//! Asbru-CM YAML exporter.
//!
//! Exports `RustConn` connections to Asbru-CM YAML configuration format.

use std::collections::HashMap;

use uuid::Uuid;

use crate::models::{Connection, ConnectionGroup, ProtocolConfig, ProtocolType};

use super::{ExportFormat, ExportOptions, ExportResult, ExportResult2, ExportTarget};

/// Asbru-CM YAML exporter.
///
/// Exports connections to Asbru-CM YAML format, preserving group hierarchy
/// and connection properties.
pub struct AsbruExporter;

impl AsbruExporter {
    /// Creates a new Asbru exporter
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Exports connections to Asbru YAML format.
    ///
    /// # Arguments
    ///
    /// * `connections` - The connections to export
    /// * `groups` - The connection groups for hierarchy
    ///
    /// # Returns
    ///
    /// A string containing the YAML-formatted Asbru configuration.
    #[must_use]
    pub fn export(connections: &[Connection], groups: &[ConnectionGroup]) -> String {
        use std::fmt::Write;

        let mut output = String::new();
        output.push_str("---\n# Asbru-CM configuration exported from RustConn\n\n");

        // Build a map from RustConn group IDs to Asbru UUID strings
        let mut group_uuid_map: HashMap<Uuid, String> = HashMap::new();

        // Export groups first
        for group in groups {
            let asbru_uuid = generate_asbru_uuid();
            group_uuid_map.insert(group.id, asbru_uuid.clone());

            let entry = Self::group_to_entry(group, &group_uuid_map);
            let _ = writeln!(output, "{asbru_uuid}:");
            output.push_str(&entry);
            output.push('\n');
        }

        // Export connections
        for conn in connections {
            let asbru_uuid = generate_asbru_uuid();
            let entry = Self::connection_to_entry(conn, &group_uuid_map);
            let _ = writeln!(output, "{asbru_uuid}:");
            output.push_str(&entry);
            output.push('\n');
        }

        output
    }

    /// Converts a connection to an Asbru YAML entry.
    ///
    /// # Arguments
    ///
    /// * `connection` - The connection to convert
    /// * `group_uuid_map` - Map from `RustConn` group IDs to Asbru UUID strings
    ///
    /// # Returns
    ///
    /// A string containing the YAML entry for the connection.
    #[must_use]
    pub fn connection_to_entry(
        connection: &Connection,
        group_uuid_map: &HashMap<Uuid, String>,
    ) -> String {
        let mut lines = Vec::new();
        let name = escape_yaml_string(&connection.name);
        let host = escape_yaml_string(&connection.host);
        let port = connection.port;

        // _is_group: 0 for connections
        lines.push("  _is_group: 0".to_string());

        // name
        lines.push(format!("  name: \"{name}\""));

        // title (same as name for simplicity)
        lines.push(format!("  title: \"{name}\""));

        // ip (hostname)
        lines.push(format!("  ip: \"{host}\""));

        // port
        lines.push(format!("  port: {port}"));

        // user
        if let Some(ref user) = connection.username {
            let user = escape_yaml_string(user);
            lines.push(format!("  user: \"{user}\""));
        }

        // method (protocol type)
        let method = match connection.protocol {
            ProtocolType::Ssh | ProtocolType::ZeroTrust => "SSH", // ZeroTrust exported as SSH
            ProtocolType::Rdp => "RDP",
            ProtocolType::Vnc => "VNC",
            ProtocolType::Spice => "SPICE",
            ProtocolType::Telnet => "telnet",
            ProtocolType::Serial => "serial",
        };
        lines.push(format!("  method: \"{method}\""));

        // Protocol-specific fields
        match &connection.protocol_config {
            ProtocolConfig::Ssh(ssh_config) => {
                // auth_type
                let auth_type = match ssh_config.auth_method {
                    crate::models::SshAuthMethod::PublicKey
                    | crate::models::SshAuthMethod::SecurityKey => "publickey",
                    crate::models::SshAuthMethod::Agent => "agent",
                    crate::models::SshAuthMethod::KeyboardInteractive => "keyboard-interactive",
                    crate::models::SshAuthMethod::Password => "password",
                };
                lines.push(format!("  auth_type: \"{auth_type}\""));

                // public key path
                if let Some(ref key_path) = ssh_config.key_path {
                    let key = escape_yaml_string(&key_path.display().to_string());
                    lines.push(format!("  public key: \"{key}\""));
                }

                // custom options
                if !ssh_config.custom_options.is_empty() {
                    let opts: Vec<String> = ssh_config
                        .custom_options
                        .iter()
                        .map(|(k, v)| format!("-o \"{k}={v}\""))
                        .collect();
                    let opts_str = opts.join(" ");
                    lines.push(format!("  options: \"{opts_str}\""));
                }
            }
            ProtocolConfig::Rdp(rdp_config) => {
                // RDP-specific fields
                if let Some(ref domain) = connection.domain {
                    let domain = escape_yaml_string(domain);
                    lines.push(format!("  domain: \"{domain}\""));
                }
                if let Some(ref resolution) = rdp_config.resolution {
                    let width = resolution.width;
                    let height = resolution.height;
                    lines.push(format!("  resolution: \"{width}x{height}\""));
                }
            }
            ProtocolConfig::Vnc(_)
            | ProtocolConfig::Spice(_)
            | ProtocolConfig::Telnet(_)
            | ProtocolConfig::ZeroTrust(_)
            | ProtocolConfig::Serial(_) => {
                // VNC, SPICE, Telnet, and ZeroTrust don't have additional fields
            }
        }

        // parent (group membership)
        if let Some(group_id) = connection.group_id {
            if let Some(parent_uuid) = group_uuid_map.get(&group_id) {
                lines.push(format!("  parent: \"{parent_uuid}\""));
            }
        }

        // description - prefer direct field, fall back to tags
        if let Some(ref desc) = connection.description {
            if !desc.is_empty() {
                let desc = escape_yaml_string(desc);
                lines.push(format!("  description: \"{desc}\""));
            }
        } else {
            // Legacy: check for description in tags
            let desc_tags: Vec<&str> = connection
                .tags
                .iter()
                .filter_map(|t| t.strip_prefix("desc:"))
                .collect();
            if !desc_tags.is_empty() {
                let desc = escape_yaml_string(&desc_tags.join(", "));
                lines.push(format!("  description: \"{desc}\""));
            }
        }

        // children (empty for connections)
        lines.push("  children: {}".to_string());

        lines.join("\n")
    }

    /// Converts a group to an Asbru YAML entry.
    ///
    /// # Arguments
    ///
    /// * `group` - The group to convert
    /// * `group_uuid_map` - Map from `RustConn` group IDs to Asbru UUID strings
    ///
    /// # Returns
    ///
    /// A string containing the YAML entry for the group.
    #[must_use]
    pub fn group_to_entry(
        group: &ConnectionGroup,
        group_uuid_map: &HashMap<Uuid, String>,
    ) -> String {
        let mut lines = Vec::new();
        let name = escape_yaml_string(&group.name);

        // _is_group: 1 for groups
        lines.push("  _is_group: 1".to_string());

        // name
        lines.push(format!("  name: \"{name}\""));

        // parent (for nested groups)
        if let Some(parent_id) = group.parent_id {
            if let Some(parent_uuid) = group_uuid_map.get(&parent_id) {
                lines.push(format!("  parent: \"{parent_uuid}\""));
            }
        }

        // description
        if let Some(ref desc) = group.description {
            if !desc.is_empty() {
                let desc = escape_yaml_string(desc);
                lines.push(format!("  description: \"{desc}\""));
            }
        }

        // children (empty placeholder - actual children reference this group via parent)
        lines.push("  children: {}".to_string());

        lines.join("\n")
    }
}

impl Default for AsbruExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ExportTarget for AsbruExporter {
    fn format_id(&self) -> ExportFormat {
        ExportFormat::Asbru
    }

    fn display_name(&self) -> &'static str {
        "Asbru-CM"
    }

    fn export(
        &self,
        connections: &[Connection],
        groups: &[ConnectionGroup],
        options: &ExportOptions,
    ) -> ExportResult2<ExportResult> {
        let mut result = ExportResult::new();

        // Filter groups if not including them
        let filtered_groups = if options.include_groups {
            groups.to_vec()
        } else {
            Vec::new()
        };

        // Generate content
        let content = Self::export(connections, &filtered_groups);

        // Write to file
        super::write_export_file(&options.output_path, &content)?;

        result.exported_count = connections.len();
        result.add_output_file(options.output_path.clone());

        Ok(result)
    }

    fn export_connection(&self, connection: &Connection) -> ExportResult2<String> {
        let group_uuid_map = HashMap::new();
        let asbru_uuid = generate_asbru_uuid();
        let entry = Self::connection_to_entry(connection, &group_uuid_map);
        Ok(format!("{asbru_uuid}:\n{entry}"))
    }

    fn supports_protocol(&self, _protocol: &ProtocolType) -> bool {
        // Asbru supports SSH, RDP, and VNC
        true
    }
}

/// Generates a UUID string in Asbru format (lowercase with hyphens).
fn generate_asbru_uuid() -> String {
    Uuid::new_v4().to_string()
}

/// Escapes special characters in a YAML string value.
fn escape_yaml_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn test_connection_to_entry_ssh() {
        let conn = create_ssh_connection("webserver", "192.168.1.100", 22);
        let group_map = HashMap::new();
        let entry = AsbruExporter::connection_to_entry(&conn, &group_map);

        assert!(entry.contains("_is_group: 0"));
        assert!(entry.contains("name: \"webserver\""));
        assert!(entry.contains("ip: \"192.168.1.100\""));
        assert!(entry.contains("port: 22"));
        assert!(entry.contains("method: \"SSH\""));
        assert!(entry.contains("children: {}"));
    }

    #[test]
    fn test_connection_to_entry_with_username() {
        let conn = create_ssh_connection("webserver", "192.168.1.100", 22).with_username("admin");
        let group_map = HashMap::new();
        let entry = AsbruExporter::connection_to_entry(&conn, &group_map);

        assert!(entry.contains("user: \"admin\""));
    }

    #[test]
    fn test_connection_to_entry_with_key_path() {
        let mut conn = create_ssh_connection("webserver", "192.168.1.100", 22);
        if let ProtocolConfig::Ssh(ref mut ssh_config) = conn.protocol_config {
            ssh_config.key_path = Some(PathBuf::from("/home/user/.ssh/id_rsa"));
        }
        let group_map = HashMap::new();
        let entry = AsbruExporter::connection_to_entry(&conn, &group_map);

        assert!(entry.contains("public key: \"/home/user/.ssh/id_rsa\""));
    }

    #[test]
    fn test_connection_to_entry_rdp() {
        let mut conn = create_rdp_connection("windows", "192.168.1.50", 3389);
        conn.domain = Some("DOMAIN".to_string());
        let group_map = HashMap::new();
        let entry = AsbruExporter::connection_to_entry(&conn, &group_map);

        assert!(entry.contains("_is_group: 0"));
        assert!(entry.contains("method: \"RDP\""));
        assert!(entry.contains("domain: \"DOMAIN\""));
    }

    #[test]
    fn test_connection_to_entry_vnc() {
        let conn = create_vnc_connection("vnc-server", "192.168.1.60", 5901);
        let group_map = HashMap::new();
        let entry = AsbruExporter::connection_to_entry(&conn, &group_map);

        assert!(entry.contains("_is_group: 0"));
        assert!(entry.contains("method: \"VNC\""));
        assert!(entry.contains("port: 5901"));
    }

    #[test]
    fn test_group_to_entry() {
        let group = ConnectionGroup::new("Production".to_string());
        let group_map = HashMap::new();
        let entry = AsbruExporter::group_to_entry(&group, &group_map);

        assert!(entry.contains("_is_group: 1"));
        assert!(entry.contains("name: \"Production\""));
        assert!(entry.contains("children: {}"));
    }

    #[test]
    fn test_group_to_entry_with_parent() {
        let parent_group = ConnectionGroup::new("Servers".to_string());
        let child_group = ConnectionGroup::with_parent("Web Servers".to_string(), parent_group.id);

        let mut group_map = HashMap::new();
        let parent_uuid = generate_asbru_uuid();
        group_map.insert(parent_group.id, parent_uuid.clone());

        let entry = AsbruExporter::group_to_entry(&child_group, &group_map);

        assert!(entry.contains("_is_group: 1"));
        assert!(entry.contains("name: \"Web Servers\""));
        assert!(entry.contains(&format!("parent: \"{parent_uuid}\"")));
    }

    #[test]
    fn test_connection_with_group() {
        let group = ConnectionGroup::new("Production".to_string());
        let group_id = group.id;
        let conn = create_ssh_connection("web1", "192.168.1.1", 22).with_group(group_id);

        let mut group_map = HashMap::new();
        let group_uuid = generate_asbru_uuid();
        group_map.insert(group_id, group_uuid.clone());

        let entry = AsbruExporter::connection_to_entry(&conn, &group_map);

        assert!(entry.contains(&format!("parent: \"{group_uuid}\"")));
    }

    #[test]
    fn test_export_simple() {
        let connections = vec![
            create_ssh_connection("web1", "192.168.1.1", 22),
            create_ssh_connection("web2", "192.168.1.2", 22),
        ];
        let output = AsbruExporter::export(&connections, &[]);

        assert!(output.contains("# Asbru-CM configuration exported from RustConn"));
        assert!(output.contains("name: \"web1\""));
        assert!(output.contains("name: \"web2\""));
        assert!(output.contains("_is_group: 0"));
    }

    #[test]
    fn test_export_with_groups() {
        let group = ConnectionGroup::new("webservers".to_string());
        let group_id = group.id;

        let connections = vec![
            create_ssh_connection("web1", "192.168.1.1", 22).with_group(group_id),
            create_ssh_connection("web2", "192.168.1.2", 22).with_group(group_id),
        ];

        let output = AsbruExporter::export(&connections, &[group]);

        assert!(output.contains("_is_group: 1"));
        assert!(output.contains("name: \"webservers\""));
        assert!(output.contains("name: \"web1\""));
        assert!(output.contains("name: \"web2\""));
        // Connections should have parent field
        assert!(output.contains("parent:"));
    }

    #[test]
    fn test_escape_yaml_string() {
        assert_eq!(escape_yaml_string("simple"), "simple");
        assert_eq!(escape_yaml_string("with\"quote"), "with\\\"quote");
        assert_eq!(escape_yaml_string("with\\backslash"), "with\\\\backslash");
        assert_eq!(escape_yaml_string("with\nnewline"), "with\\nnewline");
    }

    #[test]
    fn test_supports_protocol() {
        let exporter = AsbruExporter::new();
        assert!(exporter.supports_protocol(&ProtocolType::Ssh));
        assert!(exporter.supports_protocol(&ProtocolType::Rdp));
        assert!(exporter.supports_protocol(&ProtocolType::Vnc));
        assert!(exporter.supports_protocol(&ProtocolType::Spice));
    }

    #[test]
    fn test_connection_to_entry_with_description() {
        let mut conn = create_ssh_connection("webserver", "192.168.1.100", 22);
        conn.description = Some("Production web server for main site".to_string());
        let group_map = HashMap::new();
        let entry = AsbruExporter::connection_to_entry(&conn, &group_map);

        assert!(entry.contains("description: \"Production web server for main site\""));
    }

    #[test]
    fn test_connection_to_entry_description_from_tags_fallback() {
        let mut conn = create_ssh_connection("webserver", "192.168.1.100", 22);
        conn.tags
            .push("desc:Legacy description from tags".to_string());
        let group_map = HashMap::new();
        let entry = AsbruExporter::connection_to_entry(&conn, &group_map);

        assert!(entry.contains("description: \"Legacy description from tags\""));
    }

    #[test]
    fn test_connection_to_entry_description_prefers_field_over_tags() {
        let mut conn = create_ssh_connection("webserver", "192.168.1.100", 22);
        conn.description = Some("Direct description field".to_string());
        conn.tags.push("desc:Tag description".to_string());
        let group_map = HashMap::new();
        let entry = AsbruExporter::connection_to_entry(&conn, &group_map);

        // Should use direct field, not tags
        assert!(entry.contains("description: \"Direct description field\""));
        assert!(!entry.contains("Tag description"));
    }

    #[test]
    fn test_group_to_entry_with_description() {
        let mut group = ConnectionGroup::new("Production".to_string());
        group.description = Some("Production servers for main datacenter".to_string());
        let group_map = HashMap::new();
        let entry = AsbruExporter::group_to_entry(&group, &group_map);

        assert!(entry.contains("_is_group: 1"));
        assert!(entry.contains("name: \"Production\""));
        assert!(entry.contains("description: \"Production servers for main datacenter\""));
    }

    #[test]
    fn test_group_to_entry_empty_description_not_exported() {
        let mut group = ConnectionGroup::new("Production".to_string());
        group.description = Some(String::new());
        let group_map = HashMap::new();
        let entry = AsbruExporter::group_to_entry(&group, &group_map);

        // Empty description should not be exported
        assert!(!entry.contains("description:"));
    }
}
