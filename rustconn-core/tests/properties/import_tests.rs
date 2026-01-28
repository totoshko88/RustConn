//! Property-based tests for import functionality
//!
//! Tests correctness properties for importing connections from various sources.

use proptest::prelude::*;
use rustconn_core::import::{
    AnsibleInventoryImporter, AsbruImporter, RemminaImporter, SshConfigImporter,
};
use rustconn_core::models::{ProtocolConfig, SshAuthMethod};

/// Generates a valid SSH config hostname (no wildcards, valid characters)
fn arb_hostname() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-z][a-z0-9-]{0,20}(\\.[a-z][a-z0-9-]{0,10})*")
        .unwrap()
        .prop_filter("hostname must not be empty", |s| !s.is_empty())
}

/// Generates a valid SSH config host alias
fn arb_host_alias() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-zA-Z][a-zA-Z0-9_-]{0,30}")
        .unwrap()
        .prop_filter("alias must not be empty", |s| !s.is_empty())
}

/// Generates a valid username
fn arb_username() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-z_][a-z0-9_-]{0,30}")
        .unwrap()
        .prop_filter("username must not be empty", |s| !s.is_empty())
}

/// Generates a valid port number
fn arb_port() -> impl Strategy<Value = u16> {
    1u16..65535
}

/// Generates a valid identity file path
fn arb_identity_file() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-z0-9_]{1,20}")
        .unwrap()
        .prop_filter("name must not be empty", |s| !s.is_empty())
        .prop_map(|name| format!("~/.ssh/id_{}", name))
}

/// Generates a valid proxy jump host
fn arb_proxy_jump() -> impl Strategy<Value = String> {
    (arb_username(), arb_hostname()).prop_map(|(user, host)| format!("{}@{}", user, host))
}

/// Represents a generated SSH config entry for testing
#[derive(Debug, Clone)]
struct SshConfigEntry {
    host_alias: String,
    hostname: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    identity_file: Option<String>,
    proxy_jump: Option<String>,
}

impl SshConfigEntry {
    /// Converts to SSH config file format
    fn to_config_string(&self) -> String {
        let mut lines = vec![format!("Host {}", self.host_alias)];

        if let Some(ref hostname) = self.hostname {
            lines.push(format!("    HostName {}", hostname));
        }
        if let Some(port) = self.port {
            lines.push(format!("    Port {}", port));
        }
        if let Some(ref user) = self.user {
            lines.push(format!("    User {}", user));
        }
        if let Some(ref identity_file) = self.identity_file {
            lines.push(format!("    IdentityFile {}", identity_file));
        }
        if let Some(ref proxy_jump) = self.proxy_jump {
            lines.push(format!("    ProxyJump {}", proxy_jump));
        }

        lines.join("\n")
    }
}

/// Strategy for generating SSH config entries
fn arb_ssh_config_entry() -> impl Strategy<Value = SshConfigEntry> {
    (
        arb_host_alias(),
        prop::option::of(arb_hostname()),
        prop::option::of(arb_port()),
        prop::option::of(arb_username()),
        prop::option::of(arb_identity_file()),
        prop::option::of(arb_proxy_jump()),
    )
        .prop_map(
            |(host_alias, hostname, port, user, identity_file, proxy_jump)| SshConfigEntry {
                host_alias,
                hostname,
                port,
                user,
                identity_file,
                proxy_jump,
            },
        )
}

/// Strategy for generating multiple SSH config entries with unique host aliases
fn arb_ssh_config_entries() -> impl Strategy<Value = Vec<SshConfigEntry>> {
    prop::collection::vec(arb_ssh_config_entry(), 1..10).prop_map(|entries| {
        entries
            .into_iter()
            .enumerate()
            .map(|(i, mut entry)| {
                // Ensure unique host aliases by appending index
                entry.host_alias = format!("{}_{}", entry.host_alias, i);
                entry
            })
            .collect()
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn, Property 7: SSH Config Import Parsing**
    /// **Validates: Requirements 6.2, 6.3**
    ///
    /// For any valid SSH config file content, parsing should extract all Host entries
    /// with their corresponding parameters (hostname, port, user, identity file, proxy jump)
    /// correctly mapped to Connection objects.
    #[test]
    fn prop_ssh_config_import_extracts_all_hosts(entries in arb_ssh_config_entries()) {
        let importer = SshConfigImporter::new();

        // Generate SSH config content
        let config_content = entries
            .iter()
            .map(|e| e.to_config_string())
            .collect::<Vec<_>>()
            .join("\n\n");

        // Parse the config
        let result = importer.parse_config(&config_content, "test");

        // Property: All non-wildcard hosts should be imported
        // (entries without hostname use the alias as hostname)
        prop_assert_eq!(
            result.connections.len(),
            entries.len(),
            "Expected {} connections, got {}. Config:\n{}",
            entries.len(),
            result.connections.len(),
            config_content
        );

        // Property: Each connection should have correct parameters
        for entry in &entries {
            let conn = result
                .connections
                .iter()
                .find(|c| c.name == entry.host_alias)
                .expect(&format!("Connection '{}' not found", entry.host_alias));

            // Hostname should be HostName if specified, otherwise the alias
            let expected_host = entry.hostname.as_ref().unwrap_or(&entry.host_alias);
            prop_assert_eq!(
                &conn.host,
                expected_host,
                "Host mismatch for '{}'",
                entry.host_alias
            );

            // Port should match (default 22)
            let expected_port = entry.port.unwrap_or(22);
            prop_assert_eq!(
                conn.port,
                expected_port,
                "Port mismatch for '{}'",
                entry.host_alias
            );

            // Username should match
            prop_assert_eq!(
                conn.username.as_ref(),
                entry.user.as_ref(),
                "Username mismatch for '{}'",
                entry.host_alias
            );

            // Check SSH-specific config
            if let ProtocolConfig::Ssh(ssh_config) = &conn.protocol_config {
                // Identity file should set auth method to PublicKey
                if entry.identity_file.is_some() {
                    prop_assert_eq!(
                        &ssh_config.auth_method,
                        &SshAuthMethod::PublicKey,
                        "Auth method should be PublicKey when IdentityFile is set"
                    );
                    prop_assert!(
                        ssh_config.key_path.is_some(),
                        "Key path should be set when IdentityFile is specified"
                    );
                }

                // ProxyJump should be preserved
                prop_assert_eq!(
                    ssh_config.proxy_jump.as_ref(),
                    entry.proxy_jump.as_ref(),
                    "ProxyJump mismatch for '{}'",
                    entry.host_alias
                );
            } else {
                prop_assert!(false, "Expected SSH protocol config");
            }
        }
    }

    /// **Feature: rustconn, Property 7: SSH Config Import Parsing (Wildcards)**
    /// **Validates: Requirements 6.2, 6.3**
    ///
    /// Wildcard patterns should be skipped during import.
    #[test]
    fn prop_ssh_config_import_skips_wildcards(
        valid_entries in arb_ssh_config_entries(),
        wildcard_count in 1usize..5
    ) {
        let importer = SshConfigImporter::new();

        // Generate wildcard entries
        let wildcards: Vec<String> = (0..wildcard_count)
            .map(|i| format!("Host *.domain{}.com\n    User admin", i))
            .collect();

        // Generate valid entries
        let valid_configs: Vec<String> = valid_entries
            .iter()
            .map(|e| e.to_config_string())
            .collect();

        // Combine all entries
        let mut all_configs = wildcards.clone();
        all_configs.extend(valid_configs);

        let config_content = all_configs.join("\n\n");

        // Parse the config
        let result = importer.parse_config(&config_content, "test");

        // Property: Only valid (non-wildcard) entries should be imported
        prop_assert_eq!(
            result.connections.len(),
            valid_entries.len(),
            "Should import only non-wildcard entries"
        );

        // Property: Wildcards should be in skipped list
        prop_assert!(
            result.skipped.len() >= wildcard_count,
            "Wildcards should be skipped"
        );
    }
}

// ============================================================================
// Asbru-CM Import Property Tests
// ============================================================================

/// Represents a generated Asbru-CM connection entry for testing
#[derive(Debug, Clone)]
struct AsbruConnectionEntry {
    key: String,
    name: String,
    host: String,
    port: Option<u16>,
    user: Option<String>,
    protocol: AsbruProtocol,
    auth_type: Option<String>,
    public_key: Option<String>,
}

#[derive(Debug, Clone)]
enum AsbruProtocol {
    Ssh,
    Rdp,
    Vnc,
}

impl AsbruProtocol {
    fn to_string(&self) -> &'static str {
        match self {
            AsbruProtocol::Ssh => "ssh",
            AsbruProtocol::Rdp => "rdp",
            AsbruProtocol::Vnc => "vnc",
        }
    }

    fn default_port(&self) -> u16 {
        match self {
            AsbruProtocol::Ssh => 22,
            AsbruProtocol::Rdp => 3389,
            AsbruProtocol::Vnc => 5900,
        }
    }
}

impl AsbruConnectionEntry {
    /// Converts to Asbru YAML format (flat format with _is_group field)
    fn to_yaml_string(&self) -> String {
        self.to_yaml_string_with_parent(None)
    }

    /// Converts to Asbru YAML format with optional parent UUID
    fn to_yaml_string_with_parent(&self, parent: Option<&str>) -> String {
        let mut lines = vec![format!("{}:", self.key)];
        lines.push("  _is_group: 0".to_string());
        lines.push(format!("  name: \"{}\"", self.name));
        lines.push(format!("  ip: \"{}\"", self.host));
        lines.push(format!(
            "  method: \"{}\"",
            self.protocol.to_string().to_uppercase()
        ));

        if let Some(port) = self.port {
            lines.push(format!("  port: {}", port));
        }
        if let Some(ref user) = self.user {
            lines.push(format!("  user: \"{}\"", user));
        }
        if let Some(ref auth_type) = self.auth_type {
            lines.push(format!("  auth_type: \"{}\"", auth_type));
        }
        if let Some(ref public_key) = self.public_key {
            lines.push(format!("  public key: \"{}\"", public_key));
        }
        if let Some(parent_uuid) = parent {
            lines.push(format!("  parent: \"{}\"", parent_uuid));
        }
        lines.push("  children: {}".to_string());

        lines.join("\n")
    }
}

/// Strategy for generating Asbru protocol types
fn arb_asbru_protocol() -> impl Strategy<Value = AsbruProtocol> {
    prop_oneof![
        Just(AsbruProtocol::Ssh),
        Just(AsbruProtocol::Rdp),
        Just(AsbruProtocol::Vnc),
    ]
}

/// Strategy for generating Asbru connection entries
fn arb_asbru_connection_entry() -> impl Strategy<Value = AsbruConnectionEntry> {
    (
        prop::string::string_regex("[a-z][a-z0-9_]{0,20}").unwrap(),
        prop::string::string_regex("[A-Za-z][A-Za-z0-9 _-]{0,30}").unwrap(),
        arb_hostname(),
        prop::option::of(arb_port()),
        prop::option::of(arb_username()),
        arb_asbru_protocol(),
    )
        .prop_filter(
            "key and name must not be empty",
            |(key, name, _, _, _, _)| !key.is_empty() && !name.is_empty(),
        )
        .prop_map(|(key, name, host, port, user, protocol)| {
            // For SSH, optionally add auth settings
            let (auth_type, public_key) = if matches!(protocol, AsbruProtocol::Ssh) {
                (Some("password".to_string()), None)
            } else {
                (None, None)
            };

            AsbruConnectionEntry {
                key,
                name,
                host,
                port,
                user,
                protocol,
                auth_type,
                public_key,
            }
        })
}

/// Strategy for generating multiple Asbru connection entries with unique keys and names
fn arb_asbru_connection_entries() -> impl Strategy<Value = Vec<AsbruConnectionEntry>> {
    prop::collection::vec(arb_asbru_connection_entry(), 1..10).prop_map(|entries| {
        // Ensure unique keys and names by appending index
        // Names must be unique because the test looks up connections by name
        entries
            .into_iter()
            .enumerate()
            .map(|(i, mut e)| {
                e.key = format!("{}_{}", e.key, i);
                e.name = format!("{}_{}", e.name, i);
                e
            })
            .collect()
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn, Property 8: Asbru-CM Import Parsing**
    /// **Validates: Requirements 6.1**
    ///
    /// For any valid Asbru-CM configuration (YAML), parsing should create Connection
    /// objects with correctly mapped protocol, host, port, and authentication settings.
    #[test]
    fn prop_asbru_import_extracts_all_connections(entries in arb_asbru_connection_entries()) {
        let importer = AsbruImporter::new();

        // Generate Asbru YAML content
        let yaml_content = entries
            .iter()
            .map(|e| e.to_yaml_string())
            .collect::<Vec<_>>()
            .join("\n");

        // Parse the config
        let result = importer.parse_config(&yaml_content, "test");

        // Property: All valid entries should be imported
        prop_assert_eq!(
            result.connections.len(),
            entries.len(),
            "Expected {} connections, got {}. YAML:\n{}",
            entries.len(),
            result.connections.len(),
            yaml_content
        );

        // Property: Each connection should have correct parameters
        for entry in &entries {
            let conn = result
                .connections
                .iter()
                .find(|c| c.name == entry.name)
                .expect(&format!("Connection '{}' not found", entry.name));

            // Host should match
            prop_assert_eq!(
                &conn.host,
                &entry.host,
                "Host mismatch for '{}'",
                entry.name
            );

            // Port should match (or default for protocol)
            let expected_port = entry.port.unwrap_or(entry.protocol.default_port());
            prop_assert_eq!(
                conn.port,
                expected_port,
                "Port mismatch for '{}'",
                entry.name
            );

            // Username should match
            prop_assert_eq!(
                conn.username.as_ref(),
                entry.user.as_ref(),
                "Username mismatch for '{}'",
                entry.name
            );

            // Protocol type should match
            match (&conn.protocol_config, &entry.protocol) {
                (ProtocolConfig::Ssh(_), AsbruProtocol::Ssh) => {}
                (ProtocolConfig::Rdp(_), AsbruProtocol::Rdp) => {}
                (ProtocolConfig::Vnc(_), AsbruProtocol::Vnc) => {}
                _ => prop_assert!(
                    false,
                    "Protocol mismatch for '{}': expected {:?}",
                    entry.name,
                    entry.protocol
                ),
            }
        }
    }

    /// **Feature: rustconn, Property 8: Asbru-CM Import Parsing (Flat Groups)**
    /// **Validates: Requirements 6.1**
    ///
    /// Groups in Asbru-CM (flat format with _is_group and parent fields) should be
    /// correctly parsed and connections should be assigned to their parent groups.
    #[test]
    fn prop_asbru_import_handles_nested_groups(
        group_name in prop::string::string_regex("[A-Za-z][A-Za-z0-9 _-]{0,20}").unwrap(),
        entries in arb_asbru_connection_entries()
    ) {
        prop_assume!(!group_name.is_empty());

        let importer = AsbruImporter::new();

        // Generate flat YAML with a group and connections referencing it via parent
        let group_uuid = "group-uuid-12345";
        let group_yaml = format!(
            "{}:\n  _is_group: 1\n  name: \"{}\"\n  children: {{}}",
            group_uuid, group_name
        );

        // Generate connections with parent reference
        let connections_yaml = entries
            .iter()
            .map(|e| e.to_yaml_string_with_parent(Some(group_uuid)))
            .collect::<Vec<_>>()
            .join("\n");

        let yaml_content = format!("{}\n{}", group_yaml, connections_yaml);

        // Parse the config
        let result = importer.parse_config(&yaml_content, "test");

        // Property: Group should be created
        prop_assert_eq!(
            result.groups.len(),
            1,
            "Expected 1 group, got {}. YAML:\n{}",
            result.groups.len(),
            yaml_content
        );

        // Property: All connections should be imported
        prop_assert_eq!(
            result.connections.len(),
            entries.len(),
            "Expected {} connections, got {}",
            entries.len(),
            result.connections.len()
        );

        // Property: All connections should have group_id set
        let group_id = result.groups[0].id;
        for conn in &result.connections {
            prop_assert_eq!(
                conn.group_id,
                Some(group_id),
                "Connection '{}' should have group_id set",
                conn.name
            );
        }
    }

    /// **Feature: rustconn-bugfixes, Property 2: Asbru Hostname Extraction**
    /// **Validates: Requirements 2.1, 2.2**
    ///
    /// For any Asbru entry with hostname in ip, host, name, or title field,
    /// the importer SHALL extract a valid hostname using the fallback chain.
    /// Note: For name/title fields, the hostname must "look like" a hostname
    /// (contain dots or be IP-like) to be extracted as a fallback.
    #[test]
    fn prop_asbru_hostname_extraction_fallback(
        key in prop::string::string_regex("[a-z][a-z0-9_]{0,20}").unwrap(),
        display_name in prop::string::string_regex("[A-Za-z][A-Za-z0-9 _-]{0,30}").unwrap(),
        hostname in arb_hostname(),
        // Which field contains the hostname (0=ip, 1=host, 2=name, 3=title)
        hostname_field in 0u8..4,
    ) {
        prop_assume!(!key.is_empty() && !display_name.is_empty());
        // For name/title fields (2, 3), hostname must contain dots to be recognized
        // as a hostname-like value (this is by design - we don't want arbitrary names)
        if hostname_field >= 2 {
            prop_assume!(hostname.contains('.'));
        }

        let importer = AsbruImporter::new();

        // Build YAML with hostname in different fields based on hostname_field
        // Other fields are set to invalid values ("tmp" or empty)
        let (ip_val, host_val, name_val, title_val) = match hostname_field {
            0 => (hostname.clone(), String::new(), display_name.clone(), String::new()),
            1 => ("tmp".to_string(), hostname.clone(), display_name.clone(), String::new()),
            2 => ("tmp".to_string(), String::new(), hostname.clone(), String::new()),
            3 => ("tmp".to_string(), String::new(), display_name.clone(), hostname.clone()),
            _ => unreachable!(),
        };

        let yaml = format!(
            r#"{}:
  _is_group: 0
  name: "{}"
  title: "{}"
  ip: "{}"
  host: "{}"
  method: "SSH"
"#,
            key, name_val, title_val, ip_val, host_val
        );

        let result = importer.parse_config(&yaml, "test");

        // Property: Connection should be imported (not skipped)
        prop_assert_eq!(
            result.connections.len(),
            1,
            "Expected 1 connection, got {}. Skipped: {:?}. YAML:\n{}",
            result.connections.len(),
            result.skipped,
            yaml
        );

        // Property: Extracted hostname should match the expected hostname
        prop_assert_eq!(
            &result.connections[0].host,
            &hostname,
            "Hostname mismatch. Expected '{}', got '{}'. Field index: {}",
            hostname,
            result.connections[0].host,
            hostname_field
        );
    }

    /// **Feature: rustconn-bugfixes, Property 2: Asbru Hostname Extraction (Dynamic Variables)**
    /// **Validates: Requirements 2.1, 2.2, 2.3**
    ///
    /// For any Asbru entry with dynamic variables (${VAR} syntax) in hostname fields,
    /// the importer SHALL preserve the variable syntax and extract it as hostname.
    #[test]
    fn prop_asbru_hostname_extraction_preserves_dynamic_variables(
        key in prop::string::string_regex("[a-z][a-z0-9_]{0,20}").unwrap(),
        var_name in prop::string::string_regex("[A-Z][A-Z0-9_]{0,15}").unwrap(),
        // Which field contains the variable (0=ip, 1=host, 2=name, 3=title)
        var_field in 0u8..4,
    ) {
        prop_assume!(!key.is_empty() && !var_name.is_empty());

        let importer = AsbruImporter::new();
        let dynamic_var = format!("${{{}}}", var_name);

        // Build YAML with dynamic variable in different fields
        let (ip_val, host_val, name_val, title_val) = match var_field {
            0 => (dynamic_var.clone(), String::new(), "Server Name".to_string(), String::new()),
            1 => ("tmp".to_string(), dynamic_var.clone(), "Server Name".to_string(), String::new()),
            2 => ("tmp".to_string(), String::new(), dynamic_var.clone(), String::new()),
            3 => ("tmp".to_string(), String::new(), "Server Name".to_string(), dynamic_var.clone()),
            _ => unreachable!(),
        };

        let yaml = format!(
            r#"{}:
  _is_group: 0
  name: "{}"
  title: "{}"
  ip: "{}"
  host: "{}"
  method: "SSH"
"#,
            key, name_val, title_val, ip_val, host_val
        );

        let result = importer.parse_config(&yaml, "test");

        // Property: Connection should be imported (not skipped)
        prop_assert_eq!(
            result.connections.len(),
            1,
            "Expected 1 connection with dynamic variable, got {}. Skipped: {:?}. YAML:\n{}",
            result.connections.len(),
            result.skipped,
            yaml
        );

        // Property: Dynamic variable should be preserved as-is
        prop_assert_eq!(
            &result.connections[0].host,
            &dynamic_var,
            "Dynamic variable not preserved. Expected '{}', got '{}'",
            dynamic_var,
            result.connections[0].host
        );
    }

    /// **Feature: rustconn-bugfixes, Property 2: Asbru Hostname Extraction (Skip Invalid)**
    /// **Validates: Requirements 2.1, 2.2**
    ///
    /// For any Asbru entry with no valid hostname in any field (all "tmp" or empty),
    /// the importer SHALL skip the entry with a descriptive message.
    #[test]
    fn prop_asbru_hostname_extraction_skips_invalid(
        key in prop::string::string_regex("[a-z][a-z0-9_]{0,20}").unwrap(),
        name in prop::string::string_regex("[A-Za-z][A-Za-z0-9 _-]{0,30}").unwrap(),
    ) {
        prop_assume!(!key.is_empty() && !name.is_empty());
        // Ensure name doesn't look like a hostname (no dots, not an IP)
        prop_assume!(!name.contains('.'));

        let importer = AsbruImporter::new();

        // All hostname fields are invalid
        let yaml = format!(
            r#"{}:
  _is_group: 0
  name: "{}"
  title: ""
  ip: "tmp"
  host: ""
  method: "SSH"
"#,
            key, name
        );

        let result = importer.parse_config(&yaml, "test");

        // Property: No connections should be imported
        prop_assert_eq!(
            result.connections.len(),
            0,
            "Expected 0 connections (should be skipped), got {}. YAML:\n{}",
            result.connections.len(),
            yaml
        );

        // Property: Entry should be in skipped list
        prop_assert_eq!(
            result.skipped.len(),
            1,
            "Expected 1 skipped entry, got {}",
            result.skipped.len()
        );
    }
}

// ============================================================================
// Remmina Import Property Tests
// ============================================================================

/// Represents a generated Remmina connection entry for testing
#[derive(Debug, Clone)]
struct RemminaEntry {
    name: String,
    protocol: RemminaProtocol,
    server: String,
    port: Option<u16>,
    username: Option<String>,
    resolution: Option<(u32, u32)>,
    color_depth: Option<u8>,
}

#[derive(Debug, Clone)]
enum RemminaProtocol {
    Ssh,
    Rdp,
    Vnc,
}

impl RemminaProtocol {
    fn to_string(&self) -> &'static str {
        match self {
            RemminaProtocol::Ssh => "SSH",
            RemminaProtocol::Rdp => "RDP",
            RemminaProtocol::Vnc => "VNC",
        }
    }

    fn default_port(&self) -> u16 {
        match self {
            RemminaProtocol::Ssh => 22,
            RemminaProtocol::Rdp => 3389,
            RemminaProtocol::Vnc => 5900,
        }
    }
}

impl RemminaEntry {
    /// Converts to Remmina INI format
    fn to_remmina_string(&self) -> String {
        let mut lines = vec!["[remmina]".to_string()];
        lines.push(format!("name={}", self.name));
        lines.push(format!("protocol={}", self.protocol.to_string()));

        // Server with optional port
        if let Some(port) = self.port {
            lines.push(format!("server={}:{}", self.server, port));
        } else {
            lines.push(format!("server={}", self.server));
        }

        if let Some(ref username) = self.username {
            lines.push(format!("username={}", username));
        }

        // RDP-specific settings
        if matches!(self.protocol, RemminaProtocol::Rdp) {
            if let Some((width, height)) = self.resolution {
                lines.push(format!("resolution={}x{}", width, height));
            }
            if let Some(depth) = self.color_depth {
                lines.push(format!("colordepth={}", depth));
            }
        }

        lines.join("\n")
    }
}

/// Strategy for generating Remmina protocol types
fn arb_remmina_protocol() -> impl Strategy<Value = RemminaProtocol> {
    prop_oneof![
        Just(RemminaProtocol::Ssh),
        Just(RemminaProtocol::Rdp),
        Just(RemminaProtocol::Vnc),
    ]
}

/// Strategy for generating Remmina entry names (no leading/trailing whitespace)
fn arb_remmina_name() -> impl Strategy<Value = String> {
    prop::string::string_regex("[A-Za-z][A-Za-z0-9_-]{0,30}")
        .unwrap()
        .prop_filter("name must not be empty", |s| !s.is_empty())
}

/// Strategy for generating Remmina entries
fn arb_remmina_entry() -> impl Strategy<Value = RemminaEntry> {
    (
        arb_remmina_name(),
        arb_remmina_protocol(),
        arb_hostname(),
        prop::option::of(arb_port()),
        prop::option::of(arb_username()),
    )
        .prop_map(|(name, protocol, server, port, username)| {
            // Add RDP-specific settings for RDP protocol
            let (resolution, color_depth) = if matches!(protocol, RemminaProtocol::Rdp) {
                (Some((1920, 1080)), Some(32))
            } else {
                (None, None)
            };

            RemminaEntry {
                name,
                protocol,
                server,
                port,
                username,
                resolution,
                color_depth,
            }
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn, Property 9: Remmina Import Parsing**
    /// **Validates: Requirements 6.4**
    ///
    /// For any valid .remmina file, parsing should create a Connection object
    /// with correctly mapped protocol type, server, port, and protocol-specific settings.
    #[test]
    fn prop_remmina_import_extracts_connection(entry in arb_remmina_entry()) {
        let importer = RemminaImporter::new();

        // Generate Remmina file content
        let remmina_content = entry.to_remmina_string();

        // Parse the file
        let result = importer.parse_remmina_file(&remmina_content, "test.remmina", &mut std::collections::HashMap::new());

        // Property: One connection should be imported
        prop_assert_eq!(
            result.connections.len(),
            1,
            "Expected 1 connection, got {}. Content:\n{}",
            result.connections.len(),
            remmina_content
        );

        let conn = &result.connections[0];

        // Property: Name should match
        prop_assert_eq!(
            &conn.name,
            &entry.name,
            "Name mismatch"
        );

        // Property: Host should match
        prop_assert_eq!(
            &conn.host,
            &entry.server,
            "Host mismatch"
        );

        // Property: Port should match (from server string or default)
        let expected_port = entry.port.unwrap_or(entry.protocol.default_port());
        prop_assert_eq!(
            conn.port,
            expected_port,
            "Port mismatch"
        );

        // Property: Username should match
        prop_assert_eq!(
            conn.username.as_ref(),
            entry.username.as_ref(),
            "Username mismatch"
        );

        // Property: Protocol type should match
        match (&conn.protocol_config, &entry.protocol) {
            (ProtocolConfig::Ssh(_), RemminaProtocol::Ssh) => {}
            (ProtocolConfig::Rdp(rdp_config), RemminaProtocol::Rdp) => {
                // Check RDP-specific settings
                if let Some((width, height)) = entry.resolution {
                    prop_assert!(
                        rdp_config.resolution.is_some(),
                        "RDP resolution should be set"
                    );
                    if let Some(ref res) = rdp_config.resolution {
                        prop_assert_eq!(res.width, width, "RDP width mismatch");
                        prop_assert_eq!(res.height, height, "RDP height mismatch");
                    }
                }
                if entry.color_depth.is_some() {
                    prop_assert_eq!(
                        rdp_config.color_depth,
                        entry.color_depth,
                        "RDP color depth mismatch"
                    );
                }
            }
            (ProtocolConfig::Vnc(_), RemminaProtocol::Vnc) => {}
            _ => prop_assert!(
                false,
                "Protocol mismatch: expected {:?}",
                entry.protocol
            ),
        }
    }

    /// **Feature: rustconn, Property 9: Remmina Import Parsing (Unsupported Protocols)**
    /// **Validates: Requirements 6.4**
    ///
    /// Unsupported protocols should be skipped during import.
    /// Note: SPICE is now supported, so it's not in this list.
    #[test]
    fn prop_remmina_import_skips_unsupported_protocols(
        name in prop::string::string_regex("[A-Za-z][A-Za-z0-9 _-]{0,30}").unwrap(),
        server in arb_hostname(),
        unsupported_protocol in prop::string::string_regex("(NX|XDMCP|EXEC)").unwrap()
    ) {
        prop_assume!(!name.is_empty());

        let importer = RemminaImporter::new();

        let remmina_content = format!(
            "[remmina]\nname={}\nprotocol={}\nserver={}",
            name, unsupported_protocol, server
        );

        let result = importer.parse_remmina_file(&remmina_content, "test.remmina", &mut std::collections::HashMap::new());

        // Property: No connections should be imported
        prop_assert_eq!(
            result.connections.len(),
            0,
            "Unsupported protocol should not create connection"
        );

        // Property: Entry should be in skipped list
        prop_assert_eq!(
            result.skipped.len(),
            1,
            "Unsupported protocol should be skipped"
        );
    }
}

// ============================================================================
// Ansible Inventory Import Property Tests
// ============================================================================

/// Represents a generated Ansible host entry for testing
#[derive(Debug, Clone)]
struct AnsibleHostEntry {
    name: String,
    ansible_host: Option<String>,
    ansible_port: Option<u16>,
    ansible_user: Option<String>,
}

impl AnsibleHostEntry {
    /// Converts to INI format host line
    fn to_ini_line(&self) -> String {
        let mut parts = vec![self.name.clone()];

        if let Some(ref host) = self.ansible_host {
            parts.push(format!("ansible_host={}", host));
        }
        if let Some(port) = self.ansible_port {
            parts.push(format!("ansible_port={}", port));
        }
        if let Some(ref user) = self.ansible_user {
            parts.push(format!("ansible_user={}", user));
        }

        parts.join(" ")
    }

    /// Converts to YAML format
    fn to_yaml_string(&self) -> String {
        let mut lines = vec![format!("        {}:", self.name)];

        if let Some(ref host) = self.ansible_host {
            lines.push(format!("          ansible_host: {}", host));
        }
        if let Some(port) = self.ansible_port {
            lines.push(format!("          ansible_port: {}", port));
        }
        if let Some(ref user) = self.ansible_user {
            lines.push(format!("          ansible_user: {}", user));
        }

        // If no vars, add empty mapping
        if self.ansible_host.is_none() && self.ansible_port.is_none() && self.ansible_user.is_none()
        {
            lines[0] = format!("        {}:", self.name);
        }

        lines.join("\n")
    }
}

/// Strategy for generating Ansible host names (valid hostnames without patterns)
fn arb_ansible_hostname() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-z][a-z0-9-]{0,20}(\\.[a-z][a-z0-9-]{0,10})*")
        .unwrap()
        .prop_filter("hostname must not be empty", |s| !s.is_empty())
}

/// Strategy for generating Ansible host entries
fn arb_ansible_host_entry() -> impl Strategy<Value = AnsibleHostEntry> {
    (
        arb_ansible_hostname(),
        prop::option::of(arb_hostname()),
        prop::option::of(arb_port()),
        prop::option::of(arb_username()),
    )
        .prop_map(
            |(name, ansible_host, ansible_port, ansible_user)| AnsibleHostEntry {
                name,
                ansible_host,
                ansible_port,
                ansible_user,
            },
        )
}

/// Strategy for generating multiple Ansible host entries with unique names
fn arb_ansible_host_entries() -> impl Strategy<Value = Vec<AnsibleHostEntry>> {
    prop::collection::vec(arb_ansible_host_entry(), 1..10).prop_map(|entries| {
        // Ensure unique names by appending index
        entries
            .into_iter()
            .enumerate()
            .map(|(i, mut e)| {
                e.name = format!("{}{}", e.name, i);
                e
            })
            .collect()
    })
}

/// Strategy for generating group names
fn arb_group_name() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-z][a-z0-9_]{0,20}")
        .unwrap()
        .prop_filter("group name must not be empty", |s| !s.is_empty())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn, Property 10: Ansible Inventory Import Parsing (INI)**
    /// **Validates: Requirements 6.5**
    ///
    /// For any valid Ansible INI inventory, parsing should create Connection objects
    /// for each host with correctly mapped hostname, port, and ansible_user.
    #[test]
    fn prop_ansible_ini_import_extracts_all_hosts(
        group_name in arb_group_name(),
        entries in arb_ansible_host_entries()
    ) {
        let importer = AnsibleInventoryImporter::new();

        // Generate INI inventory content
        let host_lines = entries
            .iter()
            .map(|e| e.to_ini_line())
            .collect::<Vec<_>>()
            .join("\n");

        let ini_content = format!("[{}]\n{}", group_name, host_lines);

        // Parse the inventory
        let result = importer.parse_ini_inventory(&ini_content, "test");

        // Property: All hosts should be imported
        prop_assert_eq!(
            result.connections.len(),
            entries.len(),
            "Expected {} connections, got {}. INI:\n{}",
            entries.len(),
            result.connections.len(),
            ini_content
        );

        // Property: Group should be created
        prop_assert_eq!(
            result.groups.len(),
            1,
            "Expected 1 group"
        );

        // Property: Each connection should have correct parameters
        for entry in &entries {
            let conn = result
                .connections
                .iter()
                .find(|c| c.name == entry.name)
                .expect(&format!("Connection '{}' not found", entry.name));

            // Host should be ansible_host if specified, otherwise the name
            let expected_host = entry.ansible_host.as_ref().unwrap_or(&entry.name);
            prop_assert_eq!(
                &conn.host,
                expected_host,
                "Host mismatch for '{}'",
                entry.name
            );

            // Port should match (default 22)
            let expected_port = entry.ansible_port.unwrap_or(22);
            prop_assert_eq!(
                conn.port,
                expected_port,
                "Port mismatch for '{}'",
                entry.name
            );

            // Username should match
            prop_assert_eq!(
                conn.username.as_ref(),
                entry.ansible_user.as_ref(),
                "Username mismatch for '{}'",
                entry.name
            );

            // Should be SSH protocol
            prop_assert!(
                matches!(conn.protocol_config, ProtocolConfig::Ssh(_)),
                "Expected SSH protocol for Ansible host"
            );

            // Should have group_id set
            prop_assert!(
                conn.group_id.is_some(),
                "Connection should have group_id set"
            );
        }
    }

    /// **Feature: rustconn, Property 10: Ansible Inventory Import Parsing (YAML)**
    /// **Validates: Requirements 6.5**
    ///
    /// For any valid Ansible YAML inventory, parsing should create Connection objects
    /// for each host with correctly mapped hostname, port, and ansible_user.
    #[test]
    fn prop_ansible_yaml_import_extracts_all_hosts(
        group_name in arb_group_name(),
        entries in arb_ansible_host_entries()
    ) {
        let importer = AnsibleInventoryImporter::new();

        // Generate YAML inventory content
        let host_yaml = entries
            .iter()
            .map(|e| e.to_yaml_string())
            .collect::<Vec<_>>()
            .join("\n");

        let yaml_content = format!(
            "all:\n  children:\n    {}:\n      hosts:\n{}",
            group_name, host_yaml
        );

        // Parse the inventory
        let result = importer.parse_yaml_inventory(&yaml_content, "test");

        // Property: All hosts should be imported
        prop_assert_eq!(
            result.connections.len(),
            entries.len(),
            "Expected {} connections, got {}. YAML:\n{}",
            entries.len(),
            result.connections.len(),
            yaml_content
        );

        // Property: Each connection should have correct parameters
        for entry in &entries {
            let conn = result
                .connections
                .iter()
                .find(|c| c.name == entry.name)
                .expect(&format!("Connection '{}' not found", entry.name));

            // Host should be ansible_host if specified, otherwise the name
            let expected_host = entry.ansible_host.as_ref().unwrap_or(&entry.name);
            prop_assert_eq!(
                &conn.host,
                expected_host,
                "Host mismatch for '{}'",
                entry.name
            );

            // Port should match (default 22)
            let expected_port = entry.ansible_port.unwrap_or(22);
            prop_assert_eq!(
                conn.port,
                expected_port,
                "Port mismatch for '{}'",
                entry.name
            );

            // Username should match
            prop_assert_eq!(
                conn.username.as_ref(),
                entry.ansible_user.as_ref(),
                "Username mismatch for '{}'",
                entry.name
            );
        }
    }

    /// **Feature: rustconn, Property 10: Ansible Inventory Import Parsing (Host Ranges)**
    /// **Validates: Requirements 6.5**
    ///
    /// Host ranges should be skipped during import.
    #[test]
    fn prop_ansible_import_skips_host_ranges(
        group_name in arb_group_name(),
        valid_entries in arb_ansible_host_entries(),
        range_start in 1u32..10,
        range_end in 11u32..20
    ) {
        let importer = AnsibleInventoryImporter::new();

        // Generate INI with both valid hosts and a range pattern
        let valid_lines = valid_entries
            .iter()
            .map(|e| e.to_ini_line())
            .collect::<Vec<_>>()
            .join("\n");

        let range_pattern = format!("web[{}:{}].example.com", range_start, range_end);

        let ini_content = format!("[{}]\n{}\n{}", group_name, range_pattern, valid_lines);

        // Parse the inventory
        let result = importer.parse_ini_inventory(&ini_content, "test");

        // Property: Only valid hosts should be imported (range should be skipped)
        prop_assert_eq!(
            result.connections.len(),
            valid_entries.len(),
            "Should import only non-range hosts"
        );

        // Property: Range should be in skipped list
        prop_assert!(
            result.skipped.len() >= 1,
            "Host range should be skipped"
        );
    }
}

// ============================================================================
// Import Error Handling Property Tests
// ============================================================================

/// Represents a mix of valid and invalid SSH config entries for error handling tests
#[derive(Debug, Clone)]
enum MixedSshEntry {
    Valid(SshConfigEntry),
    Wildcard(String),
    InvalidSyntax(String),
}

impl MixedSshEntry {
    fn to_config_string(&self) -> String {
        match self {
            MixedSshEntry::Valid(entry) => entry.to_config_string(),
            MixedSshEntry::Wildcard(pattern) => format!("Host {}\n    User admin", pattern),
            MixedSshEntry::InvalidSyntax(line) => line.clone(),
        }
    }

    fn is_valid(&self) -> bool {
        matches!(self, MixedSshEntry::Valid(_))
    }

    fn is_skippable(&self) -> bool {
        matches!(self, MixedSshEntry::Wildcard(_))
    }
}

/// Strategy for generating mixed SSH config entries
fn arb_mixed_ssh_entry() -> impl Strategy<Value = MixedSshEntry> {
    prop_oneof![
        // Valid entries (70% weight)
        7 => arb_ssh_config_entry().prop_map(MixedSshEntry::Valid),
        // Wildcard patterns (20% weight)
        2 => prop::string::string_regex("\\*\\.[a-z]{3,10}\\.(com|org|net)")
            .unwrap()
            .prop_map(MixedSshEntry::Wildcard),
        // Invalid syntax (10% weight) - lines that will be skipped
        1 => Just(MixedSshEntry::InvalidSyntax("# This is a comment".to_string())),
    ]
}

/// Strategy for generating a mix of valid and invalid entries
fn arb_mixed_ssh_entries() -> impl Strategy<Value = Vec<MixedSshEntry>> {
    prop::collection::vec(arb_mixed_ssh_entry(), 2..15)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn, Property 11: Import Error Handling (SSH Config)**
    /// **Validates: Requirements 6.6, 6.7**
    ///
    /// For any import source containing a mix of valid and invalid entries,
    /// the import result should contain all valid entries as connections,
    /// all invalid entries in the skipped list.
    #[test]
    fn prop_ssh_config_import_handles_mixed_entries(entries in arb_mixed_ssh_entries()) {
        let importer = SshConfigImporter::new();

        // Generate SSH config content
        let config_content = entries
            .iter()
            .map(|e| e.to_config_string())
            .collect::<Vec<_>>()
            .join("\n\n");

        // Parse the config
        let result = importer.parse_config(&config_content, "test");

        // Count expected valid and skippable entries
        let expected_valid = entries.iter().filter(|e| e.is_valid()).count();
        let expected_skipped = entries.iter().filter(|e| e.is_skippable()).count();

        // Property: All valid entries should be imported
        prop_assert_eq!(
            result.connections.len(),
            expected_valid,
            "Expected {} valid connections, got {}",
            expected_valid,
            result.connections.len()
        );

        // Property: Wildcards should be skipped
        prop_assert!(
            result.skipped.len() >= expected_skipped,
            "Expected at least {} skipped entries, got {}",
            expected_skipped,
            result.skipped.len()
        );
    }

    /// **Feature: rustconn, Property 11: Import Error Handling (Asbru)**
    /// **Validates: Requirements 6.6, 6.7**
    ///
    /// For Asbru imports with mixed valid/invalid entries, valid entries should
    /// be imported and invalid entries should be skipped.
    #[test]
    fn prop_asbru_import_handles_mixed_entries(
        valid_count in 1usize..5,
        invalid_count in 1usize..3
    ) {
        let importer = AsbruImporter::new();

        // Generate valid entries
        let valid_entries: Vec<String> = (0..valid_count)
            .map(|i| {
                format!(
                    "valid{}:\n  name: \"Valid Server {}\"\n  ip: \"192.168.1.{}\"\n  type: \"ssh\"",
                    i, i, i + 1
                )
            })
            .collect();

        // Generate invalid entries (no host)
        let invalid_entries: Vec<String> = (0..invalid_count)
            .map(|i| {
                format!(
                    "invalid{}:\n  name: \"Invalid Server {}\"\n  type: \"ssh\"",
                    i, i
                )
            })
            .collect();

        // Combine all entries
        let mut all_entries = valid_entries.clone();
        all_entries.extend(invalid_entries.clone());

        let yaml_content = all_entries.join("\n");

        // Parse the config
        let result = importer.parse_config(&yaml_content, "test");

        // Property: All valid entries should be imported
        prop_assert_eq!(
            result.connections.len(),
            valid_count,
            "Expected {} valid connections, got {}. YAML:\n{}",
            valid_count,
            result.connections.len(),
            yaml_content
        );

        // Property: Invalid entries should be skipped
        prop_assert_eq!(
            result.skipped.len(),
            invalid_count,
            "Expected {} skipped entries, got {}",
            invalid_count,
            result.skipped.len()
        );

        // Property: Total processed should equal input count
        // (connections + skipped = total entries, no errors for valid YAML)
        prop_assert_eq!(
            result.connections.len() + result.skipped.len(),
            valid_count + invalid_count,
            "Total processed should equal input count"
        );
    }

    /// **Feature: rustconn, Property 11: Import Error Handling (Summary)**
    /// **Validates: Requirements 6.6, 6.7**
    ///
    /// Import result summary should accurately reflect the counts.
    #[test]
    fn prop_import_result_summary_is_accurate(
        conn_count in 0usize..10,
        skip_count in 0usize..5
    ) {
        use rustconn_core::import::ImportResult;
        use rustconn_core::models::Connection;

        let mut result = ImportResult::new();

        // Add connections
        for i in 0..conn_count {
            let conn = Connection::new(
                format!("conn{}", i),
                format!("host{}.example.com", i),
                22,
                ProtocolConfig::Ssh(Default::default()),
            );
            result.add_connection(conn);
        }

        // Add skipped entries
        for i in 0..skip_count {
            result.add_skipped(rustconn_core::import::SkippedEntry::new(
                format!("skipped{}", i),
                "Test skip reason",
            ));
        }

        // Property: Counts should be accurate
        prop_assert_eq!(result.connections.len(), conn_count);
        prop_assert_eq!(result.skipped.len(), skip_count);

        // Property: Summary should contain accurate counts
        let summary = result.summary();
        prop_assert!(
            summary.contains(&format!("Imported: {}", conn_count)),
            "Summary should contain connection count"
        );
        prop_assert!(
            summary.contains(&format!("Skipped: {}", skip_count)),
            "Summary should contain skipped count"
        );
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn test_ssh_config_entry_to_string() {
        let entry = SshConfigEntry {
            host_alias: "myserver".to_string(),
            hostname: Some("192.168.1.100".to_string()),
            port: Some(2222),
            user: Some("admin".to_string()),
            identity_file: Some("~/.ssh/id_ed25519".to_string()),
            proxy_jump: None,
        };

        let config = entry.to_config_string();
        assert!(config.contains("Host myserver"));
        assert!(config.contains("HostName 192.168.1.100"));
        assert!(config.contains("Port 2222"));
        assert!(config.contains("User admin"));
        assert!(config.contains("IdentityFile ~/.ssh/id_ed25519"));
    }

    #[test]
    fn test_asbru_connection_entry_to_yaml() {
        let entry = AsbruConnectionEntry {
            key: "server1".to_string(),
            name: "My Server".to_string(),
            host: "192.168.1.100".to_string(),
            port: Some(22),
            user: Some("admin".to_string()),
            protocol: AsbruProtocol::Ssh,
            auth_type: Some("password".to_string()),
            public_key: None,
        };

        let yaml = entry.to_yaml_string();
        assert!(yaml.contains("server1:"));
        assert!(yaml.contains("_is_group: 0"));
        assert!(yaml.contains("name: \"My Server\""));
        assert!(yaml.contains("ip: \"192.168.1.100\""));
        assert!(yaml.contains("method: \"SSH\""));
        assert!(yaml.contains("port: 22"));
        assert!(yaml.contains("user: \"admin\""));
    }

    #[test]
    fn test_remmina_entry_to_string() {
        let entry = RemminaEntry {
            name: "My Server".to_string(),
            protocol: RemminaProtocol::Ssh,
            server: "192.168.1.100".to_string(),
            port: Some(22),
            username: Some("admin".to_string()),
            resolution: None,
            color_depth: None,
        };

        let content = entry.to_remmina_string();
        assert!(content.contains("[remmina]"));
        assert!(content.contains("name=My Server"));
        assert!(content.contains("protocol=SSH"));
        assert!(content.contains("server=192.168.1.100:22"));
        assert!(content.contains("username=admin"));
    }
}
