//! Connection template model for creating connections with predefined settings.
//!
//! Templates allow users to define default settings for new connections,
//! making it easy to create similar connections without repetitive configuration.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::connection::{AutomationConfig, Connection, PasswordSource};
use super::custom_property::CustomProperty;
use super::protocol::{ProtocolConfig, ProtocolType, RdpConfig, SpiceConfig, SshConfig, VncConfig};
use crate::automation::ConnectionTask;
use crate::wol::WolConfig;

/// Error type for template operations
#[derive(Debug, Clone, thiserror::Error)]
pub enum TemplateError {
    /// Template not found
    #[error("Template not found: {0}")]
    NotFound(Uuid),
    /// Invalid template configuration
    #[error("Invalid template: {0}")]
    Invalid(String),
}

/// A connection template with default settings
///
/// Templates mirror the Connection structure but are used as blueprints
/// for creating new connections. When a connection is created from a template,
/// all template fields are copied to the new connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionTemplate {
    /// Unique identifier for the template
    pub id: Uuid,
    /// Human-readable name for the template
    pub name: String,
    /// Description of what this template is for
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Protocol type (SSH, RDP, VNC, SPICE)
    pub protocol: ProtocolType,
    /// Default remote host address (can be empty for user to fill in)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub host: String,
    /// Default remote port number
    pub port: u16,
    /// Default username for authentication
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Default tags for organization
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Protocol-specific configuration
    pub protocol_config: ProtocolConfig,
    /// Default password source
    #[serde(default)]
    pub password_source: PasswordSource,
    /// Default domain for RDP/Windows authentication
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Default custom properties
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_properties: Vec<CustomProperty>,
    /// Default pre-connect task
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_connect_task: Option<ConnectionTask>,
    /// Default post-disconnect task
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_disconnect_task: Option<ConnectionTask>,
    /// Default Wake On LAN configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wol_config: Option<WolConfig>,
    /// Timestamp when the template was created
    pub created_at: DateTime<Utc>,
    /// Timestamp when the template was last modified
    pub updated_at: DateTime<Utc>,
}

impl ConnectionTemplate {
    /// Creates a new template with the given name and protocol configuration
    #[must_use]
    pub fn new(name: String, protocol_config: ProtocolConfig) -> Self {
        let now = Utc::now();
        let protocol = protocol_config.protocol_type();
        let port = protocol.default_port();

        Self {
            id: Uuid::new_v4(),
            name,
            description: None,
            protocol,
            host: String::new(),
            port,
            username: None,
            tags: Vec::new(),
            protocol_config,
            password_source: PasswordSource::None,
            domain: None,
            custom_properties: Vec::new(),
            pre_connect_task: None,
            post_disconnect_task: None,
            wol_config: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// Creates a new SSH template
    #[must_use]
    pub fn new_ssh(name: String) -> Self {
        Self::new(name, ProtocolConfig::Ssh(SshConfig::default()))
    }

    /// Creates a new RDP template
    #[must_use]
    pub fn new_rdp(name: String) -> Self {
        Self::new(name, ProtocolConfig::Rdp(RdpConfig::default()))
    }

    /// Creates a new VNC template
    #[must_use]
    pub fn new_vnc(name: String) -> Self {
        Self::new(name, ProtocolConfig::Vnc(VncConfig::default()))
    }

    /// Creates a new SPICE template
    #[must_use]
    pub fn new_spice(name: String) -> Self {
        Self::new(name, ProtocolConfig::Spice(SpiceConfig::default()))
    }

    /// Sets the description for this template
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Sets the default host for this template
    #[must_use]
    pub fn with_host(mut self, host: impl Into<String>) -> Self {
        self.host = host.into();
        self
    }

    /// Sets the default port for this template
    #[must_use]
    pub const fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Sets the default username for this template
    #[must_use]
    pub fn with_username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    /// Sets the default tags for this template
    #[must_use]
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Sets the default password source for this template
    #[must_use]
    pub const fn with_password_source(mut self, source: PasswordSource) -> Self {
        self.password_source = source;
        self
    }

    /// Sets the default domain for this template
    #[must_use]
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    /// Sets the default custom properties for this template
    #[must_use]
    pub fn with_custom_properties(mut self, properties: Vec<CustomProperty>) -> Self {
        self.custom_properties = properties;
        self
    }

    /// Sets the default pre-connect task for this template
    #[must_use]
    pub fn with_pre_connect_task(mut self, task: ConnectionTask) -> Self {
        self.pre_connect_task = Some(task);
        self
    }

    /// Sets the default post-disconnect task for this template
    #[must_use]
    pub fn with_post_disconnect_task(mut self, task: ConnectionTask) -> Self {
        self.post_disconnect_task = Some(task);
        self
    }

    /// Sets the default WOL configuration for this template
    #[must_use]
    pub fn with_wol_config(mut self, config: WolConfig) -> Self {
        self.wol_config = Some(config);
        self
    }

    /// Updates the `updated_at` timestamp to now
    pub fn touch(&mut self) {
        self.updated_at = Utc::now();
    }

    /// Applies this template to create a new connection
    ///
    /// The new connection will have:
    /// - A new unique ID
    /// - The provided name (or template name if empty)
    /// - All template fields copied over
    /// - Fresh timestamps
    /// - A reference to this template's ID
    #[must_use]
    pub fn apply(&self, name: Option<String>) -> Connection {
        let now = Utc::now();
        let connection_name = name.unwrap_or_else(|| self.name.clone());

        Connection {
            id: Uuid::new_v4(),
            name: connection_name,
            protocol: self.protocol,
            host: self.host.clone(),
            port: self.port,
            username: self.username.clone(),
            group_id: None,
            tags: self.tags.clone(),
            created_at: now,
            updated_at: now,
            protocol_config: self.protocol_config.clone(),
            automation: AutomationConfig::default(),
            sort_order: 0,
            last_connected: None,
            password_source: self.password_source,
            domain: self.domain.clone(),
            custom_properties: self.custom_properties.clone(),
            pre_connect_task: self.pre_connect_task.clone(),
            post_disconnect_task: self.post_disconnect_task.clone(),
            wol_config: self.wol_config.clone(),
            local_variables: std::collections::HashMap::new(),
            log_config: None,
            key_sequence: None,
            window_mode: super::connection::WindowMode::default(),
            remember_window_position: false,
            window_geometry: None,
        }
    }

    /// Creates a template from an existing connection
    ///
    /// This is useful for creating a template based on a well-configured connection.
    #[must_use]
    pub fn from_connection(connection: &Connection, template_name: String) -> Self {
        let now = Utc::now();

        Self {
            id: Uuid::new_v4(),
            name: template_name,
            description: None,
            protocol: connection.protocol,
            host: connection.host.clone(),
            port: connection.port,
            username: connection.username.clone(),
            tags: connection.tags.clone(),
            protocol_config: connection.protocol_config.clone(),
            password_source: connection.password_source,
            domain: connection.domain.clone(),
            custom_properties: connection.custom_properties.clone(),
            pre_connect_task: connection.pre_connect_task.clone(),
            post_disconnect_task: connection.post_disconnect_task.clone(),
            wol_config: connection.wol_config.clone(),
            created_at: now,
            updated_at: now,
        }
    }
}

/// Groups templates by their protocol type
///
/// Returns a map where keys are protocol types and values are vectors of templates.
#[must_use]
pub fn group_templates_by_protocol(
    templates: &[ConnectionTemplate],
) -> std::collections::HashMap<ProtocolType, Vec<&ConnectionTemplate>> {
    let mut grouped: std::collections::HashMap<ProtocolType, Vec<&ConnectionTemplate>> =
        std::collections::HashMap::new();

    for template in templates {
        grouped.entry(template.protocol).or_default().push(template);
    }

    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_template_creation() {
        let template = ConnectionTemplate::new_ssh("SSH Server".to_string());

        assert_eq!(template.name, "SSH Server");
        assert_eq!(template.protocol, ProtocolType::Ssh);
        assert_eq!(template.port, 22);
        assert!(template.host.is_empty());
        assert!(template.username.is_none());
    }

    #[test]
    fn test_template_builders() {
        let template = ConnectionTemplate::new_rdp("RDP Server".to_string())
            .with_description("Default RDP template")
            .with_host("server.example.com")
            .with_port(3390)
            .with_username("admin")
            .with_domain("CORP")
            .with_tags(vec!["production".to_string()]);

        assert_eq!(template.name, "RDP Server");
        assert_eq!(
            template.description,
            Some("Default RDP template".to_string())
        );
        assert_eq!(template.host, "server.example.com");
        assert_eq!(template.port, 3390);
        assert_eq!(template.username, Some("admin".to_string()));
        assert_eq!(template.domain, Some("CORP".to_string()));
        assert_eq!(template.tags, vec!["production".to_string()]);
    }

    #[test]
    fn test_apply_template() {
        let template = ConnectionTemplate::new_ssh("SSH Template".to_string())
            .with_host("template.example.com")
            .with_port(2222)
            .with_username("user")
            .with_tags(vec!["dev".to_string()]);

        let connection = template.apply(Some("My Server".to_string()));

        assert_ne!(connection.id, template.id);
        assert_eq!(connection.name, "My Server");
        assert_eq!(connection.host, "template.example.com");
        assert_eq!(connection.port, 2222);
        assert_eq!(connection.username, Some("user".to_string()));
        assert_eq!(connection.tags, vec!["dev".to_string()]);
        assert_eq!(connection.protocol, ProtocolType::Ssh);
    }

    #[test]
    fn test_apply_template_uses_template_name_when_none() {
        let template = ConnectionTemplate::new_vnc("VNC Template".to_string());
        let connection = template.apply(None);

        assert_eq!(connection.name, "VNC Template");
    }

    #[test]
    fn test_from_connection() {
        let connection = Connection::new_ssh("Original".to_string(), "host.com".to_string(), 22)
            .with_username("testuser")
            .with_tags(vec!["tag1".to_string()]);

        let template = ConnectionTemplate::from_connection(&connection, "New Template".to_string());

        assert_ne!(template.id, connection.id);
        assert_eq!(template.name, "New Template");
        assert_eq!(template.host, "host.com");
        assert_eq!(template.port, 22);
        assert_eq!(template.username, Some("testuser".to_string()));
        assert_eq!(template.tags, vec!["tag1".to_string()]);
    }

    #[test]
    fn test_template_serialization() {
        let template = ConnectionTemplate::new_ssh("Test".to_string())
            .with_description("Test template")
            .with_host("example.com");

        let json = serde_json::to_string(&template).unwrap();
        let deserialized: ConnectionTemplate = serde_json::from_str(&json).unwrap();

        assert_eq!(template.id, deserialized.id);
        assert_eq!(template.name, deserialized.name);
        assert_eq!(template.description, deserialized.description);
        assert_eq!(template.host, deserialized.host);
    }

    #[test]
    fn test_template_touch() {
        let mut template = ConnectionTemplate::new_ssh("Test".to_string());
        let initial = template.updated_at;

        std::thread::sleep(std::time::Duration::from_millis(10));
        template.touch();

        assert!(template.updated_at > initial);
    }

    #[test]
    fn test_all_protocol_templates() {
        let ssh = ConnectionTemplate::new_ssh("SSH".to_string());
        assert_eq!(ssh.protocol, ProtocolType::Ssh);
        assert_eq!(ssh.port, 22);

        let rdp = ConnectionTemplate::new_rdp("RDP".to_string());
        assert_eq!(rdp.protocol, ProtocolType::Rdp);
        assert_eq!(rdp.port, 3389);

        let vnc = ConnectionTemplate::new_vnc("VNC".to_string());
        assert_eq!(vnc.protocol, ProtocolType::Vnc);
        assert_eq!(vnc.port, 5900);

        let spice = ConnectionTemplate::new_spice("SPICE".to_string());
        assert_eq!(spice.protocol, ProtocolType::Spice);
        assert_eq!(spice.port, 5900);
    }

    #[test]
    fn test_group_templates_by_protocol() {
        let templates = vec![
            ConnectionTemplate::new_ssh("SSH 1".to_string()),
            ConnectionTemplate::new_ssh("SSH 2".to_string()),
            ConnectionTemplate::new_rdp("RDP 1".to_string()),
            ConnectionTemplate::new_vnc("VNC 1".to_string()),
            ConnectionTemplate::new_spice("SPICE 1".to_string()),
        ];

        let grouped = group_templates_by_protocol(&templates);

        assert_eq!(grouped.get(&ProtocolType::Ssh).map(Vec::len), Some(2));
        assert_eq!(grouped.get(&ProtocolType::Rdp).map(Vec::len), Some(1));
        assert_eq!(grouped.get(&ProtocolType::Vnc).map(Vec::len), Some(1));
        assert_eq!(grouped.get(&ProtocolType::Spice).map(Vec::len), Some(1));
    }

    #[test]
    fn test_group_templates_empty() {
        let templates: Vec<ConnectionTemplate> = vec![];
        let grouped = group_templates_by_protocol(&templates);
        assert!(grouped.is_empty());
    }
}
