//! Connection model representing a saved remote access configuration.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use std::collections::HashMap;

use super::custom_property::CustomProperty;
use super::protocol::{ProtocolConfig, ProtocolType};
use crate::automation::{ConnectionTask, ExpectRule, KeySequence};
use crate::session::LogConfig;
use crate::variables::Variable;
use crate::wol::WolConfig;

/// Automation configuration for a connection
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AutomationConfig {
    /// Expect rules for interactive prompts
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expect_rules: Vec<ExpectRule>,
    /// Post-login scripts to execute
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_login_scripts: Vec<String>,
}

/// Source of password/credentials for a connection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasswordSource {
    /// No password stored
    #[default]
    None,
    /// Password stored in connection config (encrypted)
    Stored,
    /// Password retrieved from `KeePass` database
    KeePass,
    /// Password retrieved from system keyring (libsecret)
    Keyring,
    /// Prompt user for password on each connection
    Prompt,
}

/// Window mode for connection display
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowMode {
    /// Embedded in main window (default)
    #[default]
    Embedded,
    /// Open in separate external window
    External,
    /// Open in fullscreen mode
    Fullscreen,
}

impl WindowMode {
    /// Returns all available window modes
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Embedded, Self::External, Self::Fullscreen]
    }

    /// Returns the display name for this window mode
    #[must_use]
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::Embedded => "Embedded",
            Self::External => "External Window",
            Self::Fullscreen => "Fullscreen",
        }
    }

    /// Returns the index of this window mode in the `all()` array
    #[must_use]
    pub const fn index(&self) -> u32 {
        match self {
            Self::Embedded => 0,
            Self::External => 1,
            Self::Fullscreen => 2,
        }
    }

    /// Creates a window mode from an index
    #[must_use]
    pub const fn from_index(index: u32) -> Self {
        match index {
            1 => Self::External,
            2 => Self::Fullscreen,
            _ => Self::Embedded,
        }
    }
}

/// Window geometry for external windows
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowGeometry {
    /// Window X position
    pub x: i32,
    /// Window Y position
    pub y: i32,
    /// Window width
    pub width: i32,
    /// Window height
    pub height: i32,
}

impl WindowGeometry {
    /// Creates a new window geometry
    #[must_use]
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Creates a default window geometry
    #[must_use]
    pub const fn default_geometry() -> Self {
        Self {
            x: 100,
            y: 100,
            width: 800,
            height: 600,
        }
    }

    /// Returns true if the geometry has valid dimensions
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.width > 0 && self.height > 0
    }
}

/// A saved remote connection configuration
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    /// Unique identifier for the connection
    pub id: Uuid,
    /// Human-readable name for the connection
    pub name: String,
    /// Protocol type (SSH, RDP, VNC)
    pub protocol: ProtocolType,
    /// Remote host address (hostname or IP)
    pub host: String,
    /// Remote port number
    pub port: u16,
    /// Username for authentication
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Group this connection belongs to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<Uuid>,
    /// Tags for organization and filtering
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Timestamp when the connection was created
    pub created_at: DateTime<Utc>,
    /// Timestamp when the connection was last modified
    pub updated_at: DateTime<Utc>,
    /// Protocol-specific configuration
    pub protocol_config: ProtocolConfig,
    /// Automation configuration
    #[serde(default)]
    pub automation: AutomationConfig,
    /// Sort order for manual ordering (lower values appear first)
    #[serde(default)]
    pub sort_order: i32,
    /// Timestamp when the connection was last used
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_connected: Option<DateTime<Utc>>,
    /// Source of password for this connection
    #[serde(default)]
    pub password_source: PasswordSource,
    /// Domain for RDP/Windows authentication
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Custom properties for additional metadata
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_properties: Vec<CustomProperty>,
    /// Pre-connect task to execute before establishing the connection
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_connect_task: Option<ConnectionTask>,
    /// Post-disconnect task to execute after the connection is terminated
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_disconnect_task: Option<ConnectionTask>,
    /// Wake On LAN configuration for waking sleeping machines
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wol_config: Option<WolConfig>,
    /// Local variables that override global variables for this connection
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub local_variables: HashMap<String, Variable>,
    /// Session logging configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_config: Option<LogConfig>,
    /// Key sequence to send after connection is established
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_sequence: Option<KeySequence>,
    /// Window mode for connection display (embedded, external, fullscreen)
    #[serde(default)]
    pub window_mode: WindowMode,
    /// Whether to remember window position for external windows
    #[serde(default)]
    pub remember_window_position: bool,
    /// Saved window geometry for external windows
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_geometry: Option<WindowGeometry>,
}

impl Connection {
    /// Creates a new connection with the given parameters
    #[must_use]
    pub fn new(name: String, host: String, port: u16, protocol_config: ProtocolConfig) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            protocol: protocol_config.protocol_type(),
            host,
            port,
            username: None,
            group_id: None,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            protocol_config,
            sort_order: 0,
            last_connected: None,
            password_source: PasswordSource::None,
            domain: None,
            custom_properties: Vec::new(),
            pre_connect_task: None,
            post_disconnect_task: None,
            wol_config: None,
            local_variables: HashMap::new(),
            log_config: None,
            key_sequence: None,
            automation: AutomationConfig::default(),
            window_mode: WindowMode::default(),
            remember_window_position: false,
            window_geometry: None,
        }
    }

    /// Creates a new SSH connection with default configuration
    #[must_use]
    pub fn new_ssh(name: String, host: String, port: u16) -> Self {
        Self::new(
            name,
            host,
            port,
            ProtocolConfig::Ssh(super::protocol::SshConfig::default()),
        )
    }

    /// Creates a new RDP connection with default configuration
    #[must_use]
    pub fn new_rdp(name: String, host: String, port: u16) -> Self {
        Self::new(
            name,
            host,
            port,
            ProtocolConfig::Rdp(super::protocol::RdpConfig::default()),
        )
    }

    /// Creates a new VNC connection with default configuration
    #[must_use]
    pub fn new_vnc(name: String, host: String, port: u16) -> Self {
        Self::new(
            name,
            host,
            port,
            ProtocolConfig::Vnc(super::protocol::VncConfig::default()),
        )
    }

    /// Creates a new SPICE connection with default configuration
    #[must_use]
    pub fn new_spice(name: String, host: String, port: u16) -> Self {
        Self::new(
            name,
            host,
            port,
            ProtocolConfig::Spice(super::protocol::SpiceConfig::default()),
        )
    }

    /// Sets the username for this connection
    #[must_use]
    pub fn with_username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    /// Sets the group for this connection
    #[must_use]
    pub const fn with_group(mut self, group_id: Uuid) -> Self {
        self.group_id = Some(group_id);
        self
    }

    /// Adds tags to this connection
    #[must_use]
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Updates the `updated_at` timestamp to now
    pub fn touch(&mut self) {
        self.updated_at = Utc::now();
    }

    /// Returns the default port for this connection's protocol
    #[must_use]
    pub const fn default_port(&self) -> u16 {
        match self.protocol {
            ProtocolType::Ssh => 22,
            ProtocolType::Rdp => 3389,
            ProtocolType::Vnc | ProtocolType::Spice => 5900,
            ProtocolType::ZeroTrust => 0, // No default port for Zero Trust
        }
    }

    /// Gets a custom property by name
    ///
    /// # Arguments
    /// * `name` - The name of the property to retrieve
    ///
    /// # Returns
    /// A reference to the property if found, `None` otherwise
    #[must_use]
    pub fn get_custom_property(&self, name: &str) -> Option<&CustomProperty> {
        self.custom_properties.iter().find(|p| p.name == name)
    }

    /// Gets a mutable reference to a custom property by name
    ///
    /// # Arguments
    /// * `name` - The name of the property to retrieve
    ///
    /// # Returns
    /// A mutable reference to the property if found, `None` otherwise
    #[must_use]
    pub fn get_custom_property_mut(&mut self, name: &str) -> Option<&mut CustomProperty> {
        self.custom_properties.iter_mut().find(|p| p.name == name)
    }

    /// Sets a custom property, replacing any existing property with the same name
    ///
    /// # Arguments
    /// * `property` - The property to set
    pub fn set_custom_property(&mut self, property: CustomProperty) {
        if let Some(existing) = self.get_custom_property_mut(&property.name) {
            *existing = property;
        } else {
            self.custom_properties.push(property);
        }
        self.touch();
    }

    /// Removes a custom property by name
    ///
    /// # Arguments
    /// * `name` - The name of the property to remove
    ///
    /// # Returns
    /// `true` if a property was removed, `false` otherwise
    pub fn remove_custom_property(&mut self, name: &str) -> bool {
        let len_before = self.custom_properties.len();
        self.custom_properties.retain(|p| p.name != name);
        let removed = self.custom_properties.len() < len_before;
        if removed {
            self.touch();
        }
        removed
    }

    /// Adds custom properties to this connection (builder pattern)
    #[must_use]
    pub fn with_custom_properties(mut self, properties: Vec<CustomProperty>) -> Self {
        self.custom_properties = properties;
        self
    }

    /// Sets the pre-connect task for this connection
    #[must_use]
    pub fn with_pre_connect_task(mut self, task: ConnectionTask) -> Self {
        self.pre_connect_task = Some(task);
        self
    }

    /// Sets the post-disconnect task for this connection
    #[must_use]
    pub fn with_post_disconnect_task(mut self, task: ConnectionTask) -> Self {
        self.post_disconnect_task = Some(task);
        self
    }

    /// Returns true if this connection has a pre-connect task
    #[must_use]
    pub const fn has_pre_connect_task(&self) -> bool {
        self.pre_connect_task.is_some()
    }

    /// Returns true if this connection has a post-disconnect task
    #[must_use]
    pub const fn has_post_disconnect_task(&self) -> bool {
        self.post_disconnect_task.is_some()
    }

    /// Sets the Wake On LAN configuration for this connection
    #[must_use]
    pub fn with_wol_config(mut self, config: WolConfig) -> Self {
        self.wol_config = Some(config);
        self
    }

    /// Returns true if this connection has Wake On LAN configured
    #[must_use]
    pub const fn has_wol_config(&self) -> bool {
        self.wol_config.is_some()
    }

    /// Gets a reference to the WOL configuration if present
    #[must_use]
    pub const fn get_wol_config(&self) -> Option<&WolConfig> {
        self.wol_config.as_ref()
    }

    /// Sets the WOL configuration, updating the timestamp
    pub fn set_wol_config(&mut self, config: Option<WolConfig>) {
        self.wol_config = config;
        self.touch();
    }

    /// Gets a local variable by name
    ///
    /// # Arguments
    /// * `name` - The name of the variable to retrieve
    ///
    /// # Returns
    /// A reference to the variable if found, `None` otherwise
    #[must_use]
    pub fn get_local_variable(&self, name: &str) -> Option<&Variable> {
        self.local_variables.get(name)
    }

    /// Sets a local variable, replacing any existing variable with the same name
    ///
    /// # Arguments
    /// * `variable` - The variable to set
    pub fn set_local_variable(&mut self, variable: Variable) {
        self.local_variables.insert(variable.name.clone(), variable);
        self.touch();
    }

    /// Removes a local variable by name
    ///
    /// # Arguments
    /// * `name` - The name of the variable to remove
    ///
    /// # Returns
    /// The removed variable if it existed, `None` otherwise
    pub fn remove_local_variable(&mut self, name: &str) -> Option<Variable> {
        let removed = self.local_variables.remove(name);
        if removed.is_some() {
            self.touch();
        }
        removed
    }

    /// Returns true if this connection has local variables
    #[must_use]
    pub fn has_local_variables(&self) -> bool {
        !self.local_variables.is_empty()
    }

    /// Sets local variables for this connection (builder pattern)
    #[must_use]
    pub fn with_local_variables(mut self, variables: HashMap<String, Variable>) -> Self {
        self.local_variables = variables;
        self
    }

    /// Sets the session logging configuration for this connection
    #[must_use]
    pub fn with_log_config(mut self, config: LogConfig) -> Self {
        self.log_config = Some(config);
        self
    }

    /// Returns true if this connection has session logging configured
    #[must_use]
    pub const fn has_log_config(&self) -> bool {
        self.log_config.is_some()
    }

    /// Gets a reference to the log configuration if present
    #[must_use]
    pub const fn get_log_config(&self) -> Option<&LogConfig> {
        self.log_config.as_ref()
    }

    /// Sets the log configuration, updating the timestamp
    pub fn set_log_config(&mut self, config: Option<LogConfig>) {
        self.log_config = config;
        self.touch();
    }

    /// Returns true if session logging is enabled for this connection
    #[must_use]
    pub fn is_logging_enabled(&self) -> bool {
        self.log_config.as_ref().is_some_and(|c| c.enabled)
    }

    /// Sets the key sequence for this connection
    #[must_use]
    pub fn with_key_sequence(mut self, sequence: KeySequence) -> Self {
        self.key_sequence = Some(sequence);
        self
    }

    /// Returns true if this connection has a key sequence configured
    #[must_use]
    pub const fn has_key_sequence(&self) -> bool {
        self.key_sequence.is_some()
    }

    /// Gets a reference to the key sequence if present
    #[must_use]
    pub const fn get_key_sequence(&self) -> Option<&KeySequence> {
        self.key_sequence.as_ref()
    }

    /// Sets the key sequence, updating the timestamp
    pub fn set_key_sequence(&mut self, sequence: Option<KeySequence>) {
        self.key_sequence = sequence;
        self.touch();
    }

    /// Sets the expect rules for this connection
    #[must_use]
    pub fn with_expect_rules(mut self, rules: Vec<ExpectRule>) -> Self {
        self.automation.expect_rules = rules;
        self
    }

    /// Returns true if this connection has expect rules configured
    #[must_use]
    pub fn has_expect_rules(&self) -> bool {
        !self.automation.expect_rules.is_empty()
    }

    /// Gets a reference to the expect rules
    #[must_use]
    pub fn get_expect_rules(&self) -> &[ExpectRule] {
        &self.automation.expect_rules
    }

    /// Adds an expect rule to this connection
    pub fn add_expect_rule(&mut self, rule: ExpectRule) {
        self.automation.expect_rules.push(rule);
        self.touch();
    }

    /// Removes an expect rule by ID
    ///
    /// # Returns
    /// `true` if a rule was removed, `false` otherwise
    pub fn remove_expect_rule(&mut self, id: uuid::Uuid) -> bool {
        let len_before = self.automation.expect_rules.len();
        self.automation.expect_rules.retain(|r| r.id != id);
        let removed = self.automation.expect_rules.len() < len_before;
        if removed {
            self.touch();
        }
        removed
    }

    /// Sets the expect rules, updating the timestamp
    pub fn set_expect_rules(&mut self, rules: Vec<ExpectRule>) {
        self.automation.expect_rules = rules;
        self.touch();
    }

    /// Sets the window mode for this connection
    #[must_use]
    pub const fn with_window_mode(mut self, mode: WindowMode) -> Self {
        self.window_mode = mode;
        self
    }

    /// Gets the window mode for this connection
    #[must_use]
    pub const fn get_window_mode(&self) -> WindowMode {
        self.window_mode
    }

    /// Sets the window mode, updating the timestamp
    pub fn set_window_mode(&mut self, mode: WindowMode) {
        self.window_mode = mode;
        self.touch();
    }

    /// Returns true if this connection should open in an external window
    #[must_use]
    pub const fn is_external_window(&self) -> bool {
        matches!(self.window_mode, WindowMode::External)
    }

    /// Returns true if this connection should open in fullscreen mode
    #[must_use]
    pub const fn is_fullscreen(&self) -> bool {
        matches!(self.window_mode, WindowMode::Fullscreen)
    }

    /// Sets whether to remember window position for external windows
    #[must_use]
    pub const fn with_remember_window_position(mut self, remember: bool) -> Self {
        self.remember_window_position = remember;
        self
    }

    /// Gets whether to remember window position
    #[must_use]
    pub const fn should_remember_window_position(&self) -> bool {
        self.remember_window_position
    }

    /// Sets remember window position, updating the timestamp
    pub fn set_remember_window_position(&mut self, remember: bool) {
        self.remember_window_position = remember;
        self.touch();
    }

    /// Sets the window geometry for this connection
    #[must_use]
    pub const fn with_window_geometry(mut self, geometry: WindowGeometry) -> Self {
        self.window_geometry = Some(geometry);
        self
    }

    /// Gets the window geometry if set
    #[must_use]
    pub const fn get_window_geometry(&self) -> Option<&WindowGeometry> {
        self.window_geometry.as_ref()
    }

    /// Sets the window geometry, updating the timestamp
    pub fn set_window_geometry(&mut self, geometry: Option<WindowGeometry>) {
        self.window_geometry = geometry;
        self.touch();
    }

    /// Updates the window geometry from current window state
    pub fn update_window_geometry(&mut self, x: i32, y: i32, width: i32, height: i32) {
        if self.remember_window_position {
            self.window_geometry = Some(WindowGeometry::new(x, y, width, height));
            self.touch();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::custom_property::PropertyType;

    fn create_test_connection() -> Connection {
        Connection::new_ssh("Test Server".to_string(), "example.com".to_string(), 22)
    }

    #[test]
    fn test_get_custom_property_not_found() {
        let conn = create_test_connection();
        assert!(conn.get_custom_property("nonexistent").is_none());
    }

    #[test]
    fn test_set_and_get_custom_property() {
        let mut conn = create_test_connection();
        let prop = CustomProperty::new_text("notes", "Test notes");
        conn.set_custom_property(prop);

        let retrieved = conn.get_custom_property("notes");
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.name, "notes");
        assert_eq!(retrieved.value, "Test notes");
        assert_eq!(retrieved.property_type, PropertyType::Text);
    }

    #[test]
    fn test_set_custom_property_replaces_existing() {
        let mut conn = create_test_connection();

        // Set initial property
        conn.set_custom_property(CustomProperty::new_text("notes", "Initial"));
        assert_eq!(conn.custom_properties.len(), 1);

        // Replace with new value
        conn.set_custom_property(CustomProperty::new_text("notes", "Updated"));
        assert_eq!(conn.custom_properties.len(), 1);

        let retrieved = conn.get_custom_property("notes").unwrap();
        assert_eq!(retrieved.value, "Updated");
    }

    #[test]
    fn test_remove_custom_property() {
        let mut conn = create_test_connection();
        conn.set_custom_property(CustomProperty::new_text("notes", "Test"));

        assert!(conn.remove_custom_property("notes"));
        assert!(conn.get_custom_property("notes").is_none());
        assert!(conn.custom_properties.is_empty());
    }

    #[test]
    fn test_remove_nonexistent_property() {
        let mut conn = create_test_connection();
        assert!(!conn.remove_custom_property("nonexistent"));
    }

    #[test]
    fn test_with_custom_properties_builder() {
        let props = vec![
            CustomProperty::new_text("notes", "Some notes"),
            CustomProperty::new_url("docs", "https://example.com"),
            CustomProperty::new_protected("api_key", "secret"),
        ];

        let conn = create_test_connection().with_custom_properties(props);

        assert_eq!(conn.custom_properties.len(), 3);
        assert!(conn.get_custom_property("notes").is_some());
        assert!(conn.get_custom_property("docs").is_some());
        assert!(conn.get_custom_property("api_key").is_some());
    }

    #[test]
    fn test_all_property_types() {
        let mut conn = create_test_connection();

        // Test Text type
        conn.set_custom_property(CustomProperty::new_text("text_prop", "text value"));
        let text_prop = conn.get_custom_property("text_prop").unwrap();
        assert_eq!(text_prop.property_type, PropertyType::Text);
        assert!(!text_prop.is_protected());
        assert!(!text_prop.is_url());

        // Test URL type
        conn.set_custom_property(CustomProperty::new_url("url_prop", "https://example.com"));
        let url_prop = conn.get_custom_property("url_prop").unwrap();
        assert_eq!(url_prop.property_type, PropertyType::Url);
        assert!(!url_prop.is_protected());
        assert!(url_prop.is_url());

        // Test Protected type
        conn.set_custom_property(CustomProperty::new_protected("protected_prop", "secret"));
        let protected_prop = conn.get_custom_property("protected_prop").unwrap();
        assert_eq!(protected_prop.property_type, PropertyType::Protected);
        assert!(protected_prop.is_protected());
        assert!(!protected_prop.is_url());
    }

    #[test]
    fn test_get_custom_property_mut() {
        let mut conn = create_test_connection();
        conn.set_custom_property(CustomProperty::new_text("notes", "Initial"));

        // Modify through mutable reference
        if let Some(prop) = conn.get_custom_property_mut("notes") {
            prop.value = "Modified".to_string();
        }

        let retrieved = conn.get_custom_property("notes").unwrap();
        assert_eq!(retrieved.value, "Modified");
    }

    #[test]
    fn test_set_custom_property_updates_timestamp() {
        let mut conn = create_test_connection();
        let initial_updated_at = conn.updated_at;

        // Small delay to ensure timestamp difference
        std::thread::sleep(std::time::Duration::from_millis(10));

        conn.set_custom_property(CustomProperty::new_text("notes", "Test"));

        assert!(conn.updated_at > initial_updated_at);
    }

    #[test]
    fn test_remove_custom_property_updates_timestamp() {
        let mut conn = create_test_connection();
        conn.custom_properties
            .push(CustomProperty::new_text("notes", "Test"));
        let initial_updated_at = conn.updated_at;

        // Small delay to ensure timestamp difference
        std::thread::sleep(std::time::Duration::from_millis(10));

        conn.remove_custom_property("notes");

        assert!(conn.updated_at > initial_updated_at);
    }
}
