//! SSH config file exporter.
//!
//! Exports `RustConn` connections to OpenSSH config format (~/.ssh/config).

use std::fmt::Write;
use std::fs;

use tracing::{debug, info_span};

use crate::models::{Connection, ConnectionGroup, ProtocolConfig, ProtocolType};
use crate::tracing::span_names;

use super::{ExportError, ExportFormat, ExportOptions, ExportResult, ExportResult2, ExportTarget};

/// SSH config file exporter.
///
/// Exports SSH connections to OpenSSH configuration file format.
/// Non-SSH connections are skipped with a warning.
pub struct SshConfigExporter;

impl SshConfigExporter {
    /// Creates a new SSH config exporter
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Exports connections to SSH config format.
    ///
    /// # Arguments
    ///
    /// * `connections` - The connections to export
    ///
    /// # Returns
    ///
    /// A string containing the SSH config formatted content.
    #[must_use]
    pub fn export(connections: &[Connection]) -> String {
        let _span = info_span!(
            span_names::EXPORT_EXECUTE,
            format = "ssh_config",
            connection_count = connections.len()
        )
        .entered();

        let mut output = String::new();
        output.push_str("# SSH config exported from RustConn\n\n");

        let mut exported_count = 0;
        for conn in connections {
            if conn.protocol == ProtocolType::Ssh {
                output.push_str(&Self::format_host_entry(conn));
                output.push('\n');
                exported_count += 1;
            }
        }

        debug!(exported = exported_count, "SSH config export completed");
        output
    }

    /// Formats a single Host entry for SSH config format.
    ///
    /// # Arguments
    ///
    /// * `connection` - The connection to format
    ///
    /// # Returns
    ///
    /// A string containing the SSH config Host block.
    #[must_use]
    pub fn format_host_entry(connection: &Connection) -> String {
        let mut output = String::new();

        // Host alias (use connection name, escaped if needed)
        let host_alias = escape_value(&connection.name);
        let _ = writeln!(output, "Host {host_alias}");

        // HostName (always include)
        let hostname = escape_value(&connection.host);
        let _ = writeln!(output, "    HostName {hostname}");

        // User (if set)
        if let Some(ref user) = connection.username {
            let escaped_user = escape_value(user);
            let _ = writeln!(output, "    User {escaped_user}");
        }

        // Port (only if not default)
        if connection.port != 22 {
            let _ = writeln!(output, "    Port {}", connection.port);
        }

        // SSH-specific options
        if let ProtocolConfig::Ssh(ref ssh_config) = connection.protocol_config {
            // IdentityFile
            if let Some(ref key_path) = ssh_config.key_path {
                let path_str = key_path.display().to_string();
                let escaped_path = escape_value(&path_str);
                let _ = writeln!(output, "    IdentityFile {escaped_path}");
            }

            // ProxyJump
            if let Some(ref proxy_jump) = ssh_config.proxy_jump {
                let escaped_proxy = escape_value(proxy_jump);
                let _ = writeln!(output, "    ProxyJump {escaped_proxy}");
            }

            // ControlMaster
            if ssh_config.use_control_master {
                let _ = writeln!(output, "    ControlMaster auto");
            }

            // Custom options
            for (key, value) in &ssh_config.custom_options {
                let escaped_value = escape_value(value);
                let _ = writeln!(output, "    {key} {escaped_value}");
            }
        }

        output
    }
}

impl Default for SshConfigExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ExportTarget for SshConfigExporter {
    fn format_id(&self) -> ExportFormat {
        ExportFormat::SshConfig
    }

    fn display_name(&self) -> &'static str {
        "SSH Config"
    }

    fn export(
        &self,
        connections: &[Connection],
        _groups: &[ConnectionGroup],
        options: &ExportOptions,
    ) -> ExportResult2<ExportResult> {
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

        // Generate content
        let content = Self::export(&ssh_connections.iter().copied().cloned().collect::<Vec<_>>());

        // Write to file
        fs::write(&options.output_path, &content).map_err(|e| {
            ExportError::WriteError(format!(
                "Failed to write to {}: {}",
                options.output_path.display(),
                e
            ))
        })?;

        result.exported_count = ssh_connections.len();
        result.add_output_file(options.output_path.clone());

        Ok(result)
    }

    fn export_connection(&self, connection: &Connection) -> ExportResult2<String> {
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

/// Escapes special characters in SSH config values.
///
/// Values containing spaces or special characters are quoted.
/// Quotes within values are escaped.
fn escape_value(value: &str) -> String {
    // Check if value needs quoting
    let needs_quoting = value
        .chars()
        .any(|c| c.is_whitespace() || c == '"' || c == '\'' || c == '#' || c == '=' || c == '\\');

    if needs_quoting {
        // Escape internal quotes and wrap in quotes
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        value.to_string()
    }
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
        let conn = create_ssh_connection("myserver", "192.168.1.100", 22);
        let entry = SshConfigExporter::format_host_entry(&conn);

        assert!(entry.contains("Host myserver"));
        assert!(entry.contains("HostName 192.168.1.100"));
        assert!(!entry.contains("Port")); // Default port should not be included
    }

    #[test]
    fn test_format_host_entry_with_custom_port() {
        let conn = create_ssh_connection("myserver", "192.168.1.100", 2222);
        let entry = SshConfigExporter::format_host_entry(&conn);

        assert!(entry.contains("Port 2222"));
    }

    #[test]
    fn test_format_host_entry_with_username() {
        let conn = create_ssh_connection("myserver", "192.168.1.100", 22).with_username("admin");
        let entry = SshConfigExporter::format_host_entry(&conn);

        assert!(entry.contains("User admin"));
    }

    #[test]
    fn test_format_host_entry_with_key_path() {
        let mut conn = create_ssh_connection("myserver", "192.168.1.100", 22);
        if let ProtocolConfig::Ssh(ref mut ssh_config) = conn.protocol_config {
            ssh_config.key_path = Some(PathBuf::from("/home/user/.ssh/id_rsa"));
        }
        let entry = SshConfigExporter::format_host_entry(&conn);

        assert!(entry.contains("IdentityFile /home/user/.ssh/id_rsa"));
    }

    #[test]
    fn test_format_host_entry_with_proxy_jump() {
        let mut conn = create_ssh_connection("internal", "10.0.0.5", 22);
        if let ProtocolConfig::Ssh(ref mut ssh_config) = conn.protocol_config {
            ssh_config.proxy_jump = Some("bastion.example.com".to_string());
        }
        let entry = SshConfigExporter::format_host_entry(&conn);

        assert!(entry.contains("ProxyJump bastion.example.com"));
    }

    #[test]
    fn test_format_host_entry_with_control_master() {
        let mut conn = create_ssh_connection("myserver", "192.168.1.100", 22);
        if let ProtocolConfig::Ssh(ref mut ssh_config) = conn.protocol_config {
            ssh_config.use_control_master = true;
        }
        let entry = SshConfigExporter::format_host_entry(&conn);

        assert!(entry.contains("ControlMaster auto"));
    }

    #[test]
    fn test_format_host_entry_with_custom_options() {
        let mut conn = create_ssh_connection("myserver", "192.168.1.100", 22);
        if let ProtocolConfig::Ssh(ref mut ssh_config) = conn.protocol_config {
            ssh_config
                .custom_options
                .insert("ServerAliveInterval".to_string(), "60".to_string());
            ssh_config
                .custom_options
                .insert("ForwardAgent".to_string(), "yes".to_string());
        }
        let entry = SshConfigExporter::format_host_entry(&conn);

        assert!(entry.contains("ServerAliveInterval 60"));
        assert!(entry.contains("ForwardAgent yes"));
    }

    #[test]
    fn test_export_multiple_connections() {
        let connections = vec![
            create_ssh_connection("server1", "192.168.1.1", 22),
            create_ssh_connection("server2", "192.168.1.2", 2222),
        ];
        let output = SshConfigExporter::export(&connections);

        assert!(output.contains("Host server1"));
        assert!(output.contains("Host server2"));
        assert!(output.contains("HostName 192.168.1.1"));
        assert!(output.contains("HostName 192.168.1.2"));
        assert!(output.contains("Port 2222"));
    }

    #[test]
    fn test_escape_value_simple() {
        assert_eq!(escape_value("simple"), "simple");
        assert_eq!(escape_value("192.168.1.1"), "192.168.1.1");
    }

    #[test]
    fn test_escape_value_with_spaces() {
        assert_eq!(escape_value("path with spaces"), "\"path with spaces\"");
    }

    #[test]
    fn test_escape_value_with_quotes() {
        assert_eq!(
            escape_value("value\"with\"quotes"),
            "\"value\\\"with\\\"quotes\""
        );
    }

    #[test]
    fn test_escape_value_with_special_chars() {
        assert_eq!(escape_value("value#comment"), "\"value#comment\"");
        assert_eq!(escape_value("key=value"), "\"key=value\"");
    }

    #[test]
    fn test_supports_protocol() {
        let exporter = SshConfigExporter::new();
        assert!(exporter.supports_protocol(&ProtocolType::Ssh));
        assert!(!exporter.supports_protocol(&ProtocolType::Rdp));
        assert!(!exporter.supports_protocol(&ProtocolType::Vnc));
    }

    #[test]
    fn test_export_connection_non_ssh() {
        let exporter = SshConfigExporter::new();
        let conn = Connection::new_rdp("rdp-server".to_string(), "192.168.1.100".to_string(), 3389);
        let result = exporter.export_connection(&conn);

        assert!(result.is_err());
    }
}
