//! Ansible inventory exporter.
//!
//! Exports `RustConn` connections to Ansible inventory format (INI and YAML).

use std::collections::HashMap;
use std::fmt::Write;

use uuid::Uuid;

use crate::models::{Connection, ConnectionGroup, ProtocolConfig, ProtocolType};

use super::{
    ExportError, ExportFormat, ExportOperationResult, ExportOptions, ExportResult, ExportTarget,
};

/// Ansible inventory exporter.
///
/// Exports SSH connections to Ansible inventory format in both INI and YAML formats.
/// Non-SSH connections are skipped with a warning.
pub struct AnsibleExporter;

impl AnsibleExporter {
    /// Creates a new Ansible exporter
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Exports connections to INI format inventory.
    ///
    /// # Arguments
    ///
    /// * `connections` - The connections to export
    /// * `groups` - The connection groups for organization
    ///
    /// # Returns
    ///
    /// A string containing the INI-formatted Ansible inventory.
    #[must_use]
    pub fn export_ini(connections: &[Connection], groups: &[ConnectionGroup]) -> String {
        let mut output = String::new();
        output.push_str("# Ansible inventory exported from RustConn\n\n");

        // Build group lookup map
        let group_map: HashMap<Uuid, &ConnectionGroup> = groups.iter().map(|g| (g.id, g)).collect();

        // Group connections by their group_id
        let mut grouped: HashMap<Option<Uuid>, Vec<&Connection>> = HashMap::new();
        for conn in connections {
            if conn.protocol == ProtocolType::Ssh {
                grouped.entry(conn.group_id).or_default().push(conn);
            }
        }

        // Output ungrouped connections first (under [all] or no section)
        if let Some(ungrouped) = grouped.remove(&None) {
            if !ungrouped.is_empty() {
                output.push_str("[ungrouped]\n");
                for conn in ungrouped {
                    output.push_str(&Self::format_host_entry(conn));
                    output.push('\n');
                }
                output.push('\n');
            }
        }

        // Output grouped connections
        for (group_id, conns) in grouped {
            if let Some(group_id) = group_id {
                let group_name = group_map
                    .get(&group_id)
                    .map_or_else(|| "unknown".to_string(), |g| sanitize_group_name(&g.name));

                let _ = writeln!(output, "[{group_name}]");
                for conn in conns {
                    output.push_str(&Self::format_host_entry(conn));
                    output.push('\n');
                }
                output.push('\n');
            }
        }

        output
    }

    /// Exports connections to YAML format inventory.
    ///
    /// # Arguments
    ///
    /// * `connections` - The connections to export
    /// * `groups` - The connection groups for organization
    ///
    /// # Returns
    ///
    /// A string containing the YAML-formatted Ansible inventory.
    #[must_use]
    pub fn export_yaml(connections: &[Connection], groups: &[ConnectionGroup]) -> String {
        let mut output = String::new();
        output.push_str("---\n# Ansible inventory exported from RustConn\n\n");
        output.push_str("all:\n");

        // Build group lookup map
        let group_map: HashMap<Uuid, &ConnectionGroup> = groups.iter().map(|g| (g.id, g)).collect();

        // Group connections by their group_id
        let mut grouped: HashMap<Option<Uuid>, Vec<&Connection>> = HashMap::new();
        for conn in connections {
            if conn.protocol == ProtocolType::Ssh {
                grouped.entry(conn.group_id).or_default().push(conn);
            }
        }

        // Check if we have any grouped connections
        let has_groups = grouped.keys().any(Option::is_some);

        // Output ungrouped hosts directly under 'all'
        if let Some(ungrouped) = grouped.get(&None) {
            if !ungrouped.is_empty() {
                output.push_str("  hosts:\n");
                for conn in ungrouped {
                    output.push_str(&Self::format_yaml_host(conn, 4));
                }
            }
        }

        // Output grouped connections under 'children'
        if has_groups {
            output.push_str("  children:\n");
            for (group_id, conns) in &grouped {
                if let Some(group_id) = group_id {
                    let group_name = group_map
                        .get(group_id)
                        .map_or_else(|| "unknown".to_string(), |g| sanitize_group_name(&g.name));

                    let _ = writeln!(output, "    {group_name}:");
                    output.push_str("      hosts:\n");
                    for conn in conns {
                        output.push_str(&Self::format_yaml_host(conn, 8));
                    }
                }
            }
        }

        output
    }

    /// Formats a single host entry for INI format.
    ///
    /// # Arguments
    ///
    /// * `connection` - The connection to format
    ///
    /// # Returns
    ///
    /// A string containing the INI-formatted host entry.
    #[must_use]
    pub fn format_host_entry(connection: &Connection) -> String {
        let mut entry = sanitize_host_name(&connection.name);
        let mut vars = Vec::new();

        // ansible_host (always include if different from name)
        if connection.host != connection.name {
            vars.push(format!("ansible_host={}", connection.host));
        }

        // ansible_user
        if let Some(ref user) = connection.username {
            vars.push(format!("ansible_user={user}"));
        }

        // ansible_port (only if not default)
        if connection.port != 22 {
            vars.push(format!("ansible_port={}", connection.port));
        }

        // ansible_ssh_private_key_file
        if let ProtocolConfig::Ssh(ref ssh_config) = connection.protocol_config {
            if let Some(ref key_path) = ssh_config.key_path {
                vars.push(format!(
                    "ansible_ssh_private_key_file={}",
                    key_path.display()
                ));
            }
        }

        if !vars.is_empty() {
            entry.push(' ');
            entry.push_str(&vars.join(" "));
        }

        entry
    }

    /// Formats a single host entry for YAML format.
    fn format_yaml_host(connection: &Connection, indent: usize) -> String {
        let indent_str = " ".repeat(indent);
        let mut output = String::new();
        let _ = writeln!(
            output,
            "{indent_str}{}:",
            sanitize_host_name(&connection.name)
        );

        // ansible_host (always include if different from name)
        if connection.host != connection.name {
            let _ = writeln!(output, "{indent_str}  ansible_host: {}", connection.host);
        }

        // ansible_user
        if let Some(ref user) = connection.username {
            let _ = writeln!(output, "{indent_str}  ansible_user: {user}");
        }

        // ansible_port (only if not default)
        if connection.port != 22 {
            let _ = writeln!(output, "{indent_str}  ansible_port: {}", connection.port);
        }

        // ansible_ssh_private_key_file
        if let ProtocolConfig::Ssh(ref ssh_config) = connection.protocol_config {
            if let Some(ref key_path) = ssh_config.key_path {
                let _ = writeln!(
                    output,
                    "{indent_str}  ansible_ssh_private_key_file: {}",
                    key_path.display()
                );
            }
        }

        output
    }
}

impl Default for AnsibleExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ExportTarget for AnsibleExporter {
    fn format_id(&self) -> ExportFormat {
        ExportFormat::Ansible
    }

    fn display_name(&self) -> &'static str {
        "Ansible Inventory"
    }

    fn export(
        &self,
        connections: &[Connection],
        groups: &[ConnectionGroup],
        options: &ExportOptions,
    ) -> ExportOperationResult<ExportResult> {
        let mut result = ExportResult::new();

        // Filter SSH connections and count skipped
        let ssh_connections: Vec<&Connection> = connections
            .iter()
            .filter(|c| {
                if c.protocol == ProtocolType::Ssh {
                    true
                } else {
                    result.increment_skipped();
                    result.add_warning(format!(
                        "Skipped non-SSH connection '{}' (protocol: {})",
                        c.name, c.protocol
                    ));
                    false
                }
            })
            .collect();

        // Prepare filtered groups
        let filtered_groups = if options.include_groups {
            groups.to_vec()
        } else {
            Vec::new()
        };

        let connections_vec: Vec<_> = ssh_connections.iter().copied().cloned().collect();

        // Generate content based on file extension
        let content = if options
            .output_path
            .extension()
            .is_some_and(|ext| ext == "yml" || ext == "yaml")
        {
            Self::export_yaml(&connections_vec, &filtered_groups)
        } else {
            Self::export_ini(&connections_vec, &filtered_groups)
        };

        // Write to file
        super::write_export_file(&options.output_path, &content)?;

        result.exported_count = ssh_connections.len();
        result.add_output_file(options.output_path.clone());

        Ok(result)
    }

    fn export_connection(&self, connection: &Connection) -> ExportOperationResult<String> {
        if connection.protocol != ProtocolType::Ssh {
            return Err(ExportError::UnsupportedProtocol(format!(
                "{}",
                connection.protocol
            )));
        }

        Ok(Self::format_host_entry(connection))
    }

    fn supports_protocol(&self, protocol: &ProtocolType) -> bool {
        *protocol == ProtocolType::Ssh
    }
}

/// Sanitizes a group name for use in Ansible inventory.
///
/// Replaces spaces and special characters with underscores.
fn sanitize_group_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Sanitizes a host name for use in Ansible inventory.
///
/// Replaces spaces with underscores and removes invalid characters.
fn sanitize_host_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn create_ssh_connection(name: &str, host: &str, port: u16) -> Connection {
        Connection::new_ssh(name.to_string(), host.to_string(), port)
    }

    #[test]
    fn test_format_host_entry_simple() {
        let conn = create_ssh_connection("webserver", "webserver", 22);
        let entry = AnsibleExporter::format_host_entry(&conn);
        assert_eq!(entry, "webserver");
    }

    #[test]
    fn test_format_host_entry_with_different_host() {
        let conn = create_ssh_connection("webserver", "192.168.1.100", 22);
        let entry = AnsibleExporter::format_host_entry(&conn);
        assert!(entry.contains("ansible_host=192.168.1.100"));
    }

    #[test]
    fn test_format_host_entry_with_custom_port() {
        let conn = create_ssh_connection("webserver", "webserver", 2222);
        let entry = AnsibleExporter::format_host_entry(&conn);
        assert!(entry.contains("ansible_port=2222"));
    }

    #[test]
    fn test_format_host_entry_with_username() {
        let conn = create_ssh_connection("webserver", "webserver", 22).with_username("admin");
        let entry = AnsibleExporter::format_host_entry(&conn);
        assert!(entry.contains("ansible_user=admin"));
    }

    #[test]
    fn test_format_host_entry_with_key_path() {
        let mut conn = create_ssh_connection("webserver", "webserver", 22);
        if let ProtocolConfig::Ssh(ref mut ssh_config) = conn.protocol_config {
            ssh_config.key_path = Some(PathBuf::from("/home/user/.ssh/id_rsa"));
        }
        let entry = AnsibleExporter::format_host_entry(&conn);
        assert!(entry.contains("ansible_ssh_private_key_file=/home/user/.ssh/id_rsa"));
    }

    #[test]
    fn test_export_ini_simple() {
        let connections = vec![
            create_ssh_connection("web1", "192.168.1.1", 22),
            create_ssh_connection("web2", "192.168.1.2", 22),
        ];
        let output = AnsibleExporter::export_ini(&connections, &[]);
        assert!(output.contains("[ungrouped]"));
        assert!(output.contains("web1"));
        assert!(output.contains("web2"));
    }

    #[test]
    fn test_export_ini_with_groups() {
        let group = ConnectionGroup::new("webservers".to_string());
        let group_id = group.id;

        let connections = vec![
            create_ssh_connection("web1", "192.168.1.1", 22).with_group(group_id),
            create_ssh_connection("web2", "192.168.1.2", 22).with_group(group_id),
        ];

        let output = AnsibleExporter::export_ini(&connections, &[group]);
        assert!(output.contains("[webservers]"));
        assert!(output.contains("web1"));
        assert!(output.contains("web2"));
    }

    #[test]
    fn test_export_yaml_simple() {
        let connections = vec![
            create_ssh_connection("web1", "192.168.1.1", 22),
            create_ssh_connection("web2", "192.168.1.2", 22),
        ];
        let output = AnsibleExporter::export_yaml(&connections, &[]);
        assert!(output.contains("all:"));
        assert!(output.contains("hosts:"));
        assert!(output.contains("web1:"));
        assert!(output.contains("web2:"));
    }

    #[test]
    fn test_export_yaml_with_groups() {
        let group = ConnectionGroup::new("webservers".to_string());
        let group_id = group.id;

        let connections = vec![
            create_ssh_connection("web1", "192.168.1.1", 22).with_group(group_id),
            create_ssh_connection("web2", "192.168.1.2", 22).with_group(group_id),
        ];

        let output = AnsibleExporter::export_yaml(&connections, &[group]);
        assert!(output.contains("children:"));
        assert!(output.contains("webservers:"));
    }

    #[test]
    fn test_sanitize_group_name() {
        assert_eq!(sanitize_group_name("web servers"), "web_servers");
        assert_eq!(sanitize_group_name("web-servers"), "web-servers");
        assert_eq!(sanitize_group_name("web_servers"), "web_servers");
        assert_eq!(sanitize_group_name("web@servers!"), "web_servers_");
    }

    #[test]
    fn test_sanitize_host_name() {
        assert_eq!(sanitize_host_name("web server"), "web_server");
        assert_eq!(sanitize_host_name("web.example.com"), "web.example.com");
        assert_eq!(sanitize_host_name("web-server"), "web-server");
    }

    #[test]
    fn test_supports_protocol() {
        let exporter = AnsibleExporter::new();
        assert!(exporter.supports_protocol(&ProtocolType::Ssh));
        assert!(!exporter.supports_protocol(&ProtocolType::Rdp));
        assert!(!exporter.supports_protocol(&ProtocolType::Vnc));
    }
}
