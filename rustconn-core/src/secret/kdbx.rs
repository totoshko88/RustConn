//! KDBX export functionality
//!
//! This module provides functionality to export credentials to a KeePassXC-compatible
//! KDBX file format. Since implementing full KDBX encryption is complex, this module
//! exports to an XML format that can be imported into `KeePassXC`.

use std::io::Write;
use std::path::Path;

use chrono::Utc;

use crate::error::{SecretError, SecretResult};
use crate::models::{Connection, Credentials};

/// Entry for KDBX export
#[derive(Debug, Clone)]
pub struct KdbxEntry {
    /// Entry title (connection name)
    pub title: String,
    /// Username
    pub username: Option<String>,
    /// Password
    pub password: Option<String>,
    /// URL (connection URL)
    pub url: Option<String>,
    /// Notes
    pub notes: Option<String>,
    /// Group path (e.g., "RustConn/SSH")
    pub group: String,
}

impl KdbxEntry {
    /// Creates a new KDBX entry from a connection and credentials
    #[must_use]
    pub fn from_connection(connection: &Connection, credentials: &Credentials) -> Self {
        let protocol_name = match &connection.protocol_config {
            crate::models::ProtocolConfig::Ssh(_) => "SSH",
            crate::models::ProtocolConfig::Rdp(_) => "RDP",
            crate::models::ProtocolConfig::Vnc(_) => "VNC",
            crate::models::ProtocolConfig::Spice(_) => "SPICE",
            crate::models::ProtocolConfig::ZeroTrust(_) => "ZeroTrust",
            crate::models::ProtocolConfig::Telnet(_) => "Telnet",
        };

        let url = format!(
            "{}://{}:{}",
            protocol_name.to_lowercase(),
            connection.host,
            connection.port
        );

        let notes = format!(
            "Connection ID: {}\nProtocol: {}\nHost: {}\nPort: {}",
            connection.id, protocol_name, connection.host, connection.port
        );

        Self {
            title: connection.name.clone(),
            username: credentials.username.clone(),
            password: credentials.expose_password().map(String::from),
            url: Some(url),
            notes: Some(notes),
            group: format!("RustConn/{protocol_name}"),
        }
    }
}

/// KDBX exporter for credential export
///
/// This exporter creates a `KeePass` XML file that can be imported into
/// `KeePassXC` or other `KeePass`-compatible applications.
pub struct KdbxExporter {
    /// Entries to export
    entries: Vec<KdbxEntry>,
    /// Database name
    database_name: String,
}

impl KdbxExporter {
    /// Creates a new KDBX exporter
    #[must_use]
    pub fn new(database_name: impl Into<String>) -> Self {
        Self {
            entries: Vec::new(),
            database_name: database_name.into(),
        }
    }

    /// Adds an entry to the export
    pub fn add_entry(&mut self, entry: KdbxEntry) {
        self.entries.push(entry);
    }

    /// Adds a connection with credentials to the export
    pub fn add_connection(&mut self, connection: &Connection, credentials: &Credentials) {
        self.entries
            .push(KdbxEntry::from_connection(connection, credentials));
    }

    /// Exports to `KeePass` XML format
    ///
    /// # Arguments
    /// * `path` - Path to write the XML file
    ///
    /// # Errors
    /// Returns `SecretError` if writing fails
    pub fn export_xml(&self, path: impl AsRef<Path>) -> SecretResult<()> {
        let xml = self.generate_xml();

        std::fs::write(path.as_ref(), xml)
            .map_err(|e| SecretError::KeePassXC(format!("Failed to write KDBX XML file: {e}")))?;

        Ok(())
    }

    /// Exports to a writer
    ///
    /// # Arguments
    /// * `writer` - Writer to write the XML to
    ///
    /// # Errors
    /// Returns `SecretError` if writing fails
    pub fn export_to_writer<W: Write>(&self, mut writer: W) -> SecretResult<()> {
        let xml = self.generate_xml();

        writer
            .write_all(xml.as_bytes())
            .map_err(|e| SecretError::KeePassXC(format!("Failed to write KDBX XML: {e}")))?;

        Ok(())
    }

    /// Generates the `KeePass` XML content
    #[allow(clippy::format_push_string)]
    fn generate_xml(&self) -> String {
        let now = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        xml.push_str("<KeePassFile>\n");
        xml.push_str("\t<Root>\n");
        xml.push_str("\t\t<Group>\n");
        xml.push_str(&format!(
            "\t\t\t<Name>{}</Name>\n",
            escape_xml(&self.database_name)
        ));
        xml.push_str("\t\t\t<IsExpanded>True</IsExpanded>\n");

        // Group entries by their group path
        let mut groups: std::collections::HashMap<String, Vec<&KdbxEntry>> =
            std::collections::HashMap::new();

        for entry in &self.entries {
            groups.entry(entry.group.clone()).or_default().push(entry);
        }

        // Write groups and entries
        for (group_path, entries) in &groups {
            xml.push_str("\t\t\t<Group>\n");
            xml.push_str(&format!(
                "\t\t\t\t<Name>{}</Name>\n",
                escape_xml(group_path)
            ));
            xml.push_str("\t\t\t\t<IsExpanded>True</IsExpanded>\n");

            for entry in entries {
                xml.push_str("\t\t\t\t<Entry>\n");
                xml.push_str(&format!(
                    "\t\t\t\t\t<String><Key>Title</Key><Value>{}</Value></String>\n",
                    escape_xml(&entry.title)
                ));

                if let Some(username) = &entry.username {
                    xml.push_str(&format!(
                        "\t\t\t\t\t<String><Key>UserName</Key><Value>{}</Value></String>\n",
                        escape_xml(username)
                    ));
                }

                if let Some(password) = &entry.password {
                    xml.push_str(&format!(
                        "\t\t\t\t\t<String><Key>Password</Key><Value Protected=\"True\">{}</Value></String>\n",
                        escape_xml(password)
                    ));
                }

                if let Some(url) = &entry.url {
                    xml.push_str(&format!(
                        "\t\t\t\t\t<String><Key>URL</Key><Value>{}</Value></String>\n",
                        escape_xml(url)
                    ));
                }

                if let Some(notes) = &entry.notes {
                    xml.push_str(&format!(
                        "\t\t\t\t\t<String><Key>Notes</Key><Value>{}</Value></String>\n",
                        escape_xml(notes)
                    ));
                }

                xml.push_str(&format!(
                    "\t\t\t\t\t<Times><CreationTime>{now}</CreationTime></Times>\n"
                ));
                xml.push_str("\t\t\t\t</Entry>\n");
            }

            xml.push_str("\t\t\t</Group>\n");
        }

        xml.push_str("\t\t</Group>\n");
        xml.push_str("\t</Root>\n");
        xml.push_str("</KeePassFile>\n");

        xml
    }

    /// Returns the number of entries
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if there are no entries
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for KdbxExporter {
    fn default() -> Self {
        Self::new("RustConn Export")
    }
}

/// Escapes special XML characters
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

    #[test]
    fn test_escape_xml() {
        assert_eq!(escape_xml("hello"), "hello");
        assert_eq!(escape_xml("<test>"), "&lt;test&gt;");
        assert_eq!(escape_xml("a & b"), "a &amp; b");
        assert_eq!(escape_xml("\"quoted\""), "&quot;quoted&quot;");
    }

    #[test]
    fn test_kdbx_entry_creation() {
        let entry = KdbxEntry {
            title: "Test Server".to_string(),
            username: Some("admin".to_string()),
            password: Some("secret".to_string()),
            url: Some("ssh://test.example.com:22".to_string()),
            notes: None,
            group: "RustConn/SSH".to_string(),
        };

        assert_eq!(entry.title, "Test Server");
        assert_eq!(entry.username, Some("admin".to_string()));
    }

    #[test]
    fn test_exporter_generates_xml() {
        let mut exporter = KdbxExporter::new("Test DB");
        exporter.add_entry(KdbxEntry {
            title: "Server 1".to_string(),
            username: Some("user1".to_string()),
            password: Some("pass1".to_string()),
            url: Some("ssh://server1:22".to_string()),
            notes: None,
            group: "RustConn/SSH".to_string(),
        });

        let xml = exporter.generate_xml();
        assert!(xml.contains("<?xml version=\"1.0\""));
        assert!(xml.contains("<KeePassFile>"));
        assert!(xml.contains("Server 1"));
        assert!(xml.contains("user1"));
    }
}
