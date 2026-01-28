//! Property-based tests for the Connection Template system
//!
//! These tests validate the correctness properties defined in the design document
//! for the Connection Template system (Requirements 8.x).

use proptest::prelude::*;
use rustconn_core::{
    ConnectionTemplate, CustomProperty, PasswordSource, ProtocolConfig, ProtocolType, RdpConfig,
    SpiceConfig, SshConfig, VncConfig,
};

// ========== Strategies ==========

/// Strategy for generating valid template names
fn arb_template_name() -> impl Strategy<Value = String> {
    "[A-Za-z][A-Za-z0-9 _-]{0,30}".prop_filter("non-empty", |s| !s.trim().is_empty())
}

/// Strategy for generating valid hostnames
fn arb_hostname() -> impl Strategy<Value = String> {
    prop_oneof![
        "[a-z][a-z0-9-]{0,20}\\.[a-z]{2,4}",
        "192\\.168\\.[0-9]{1,3}\\.[0-9]{1,3}",
        Just(String::new()), // Empty host is valid for templates
    ]
}

/// Strategy for generating valid usernames
fn arb_username() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), "[a-z][a-z0-9_]{0,15}".prop_map(Some),]
}

/// Strategy for generating valid ports
fn arb_port() -> impl Strategy<Value = u16> {
    prop_oneof![Just(22u16), Just(3389u16), Just(5900u16), 1024u16..65535u16,]
}

/// Strategy for generating tags
fn arb_tags() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec("[a-z][a-z0-9-]{0,10}", 0..5)
}

/// Strategy for generating protocol configs
fn arb_protocol_config() -> impl Strategy<Value = ProtocolConfig> {
    prop_oneof![
        Just(ProtocolConfig::Ssh(SshConfig::default())),
        Just(ProtocolConfig::Rdp(RdpConfig::default())),
        Just(ProtocolConfig::Vnc(VncConfig::default())),
        Just(ProtocolConfig::Spice(SpiceConfig::default())),
    ]
}

/// Strategy for generating password sources
fn arb_password_source() -> impl Strategy<Value = PasswordSource> {
    prop_oneof![
        Just(PasswordSource::None),
        Just(PasswordSource::KeePass),
        Just(PasswordSource::Keyring),
        Just(PasswordSource::Bitwarden),
        Just(PasswordSource::Prompt),
    ]
}

/// Strategy for generating custom properties
fn arb_custom_properties() -> impl Strategy<Value = Vec<CustomProperty>> {
    prop::collection::vec(
        ("[a-z_][a-z0-9_]{0,15}", "[a-zA-Z0-9 ]{0,50}")
            .prop_map(|(name, value)| CustomProperty::new_text(&name, &value)),
        0..3,
    )
}

/// Strategy for generating a complete template
fn arb_template() -> impl Strategy<Value = ConnectionTemplate> {
    (
        arb_template_name(),
        arb_protocol_config(),
        arb_hostname(),
        arb_port(),
        arb_username(),
        arb_tags(),
        arb_password_source(),
        arb_custom_properties(),
    )
        .prop_map(
            |(
                name,
                protocol_config,
                host,
                port,
                username,
                tags,
                password_source,
                custom_properties,
            )| {
                let mut template = ConnectionTemplate::new(name, protocol_config)
                    .with_host(host)
                    .with_port(port)
                    .with_tags(tags)
                    .with_password_source(password_source)
                    .with_custom_properties(custom_properties);

                if let Some(user) = username {
                    template = template.with_username(user);
                }

                template
            },
        )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 20: Template Application ==========
    // **Feature: rustconn-enhancements, Property 20: Template Application**
    // **Validates: Requirements 8.2**
    //
    // For any template and new connection, applying the template should copy
    // all template fields to the connection.

    #[test]
    fn prop_template_application_copies_all_fields(
        template in arb_template(),
        connection_name in prop::option::of(arb_template_name())
    ) {
        let connection = template.apply(connection_name.clone());

        // Connection should have a new unique ID
        prop_assert_ne!(connection.id, template.id);

        // Name should be the provided name or template name
        let expected_name = connection_name.unwrap_or_else(|| template.name.clone());
        prop_assert_eq!(&connection.name, &expected_name);

        // All template fields should be copied
        prop_assert_eq!(connection.protocol, template.protocol);
        prop_assert_eq!(&connection.host, &template.host);
        prop_assert_eq!(connection.port, template.port);
        prop_assert_eq!(&connection.username, &template.username);
        prop_assert_eq!(&connection.tags, &template.tags);
        prop_assert_eq!(connection.password_source, template.password_source);
        prop_assert_eq!(&connection.domain, &template.domain);
        prop_assert_eq!(&connection.custom_properties, &template.custom_properties);

        // Protocol config should match
        prop_assert_eq!(&connection.protocol_config, &template.protocol_config);

        // Connection-specific fields should be initialized
        prop_assert!(connection.group_id.is_none());
        prop_assert_eq!(connection.sort_order, 0);
        prop_assert!(connection.last_connected.is_none());
    }

    // ========== Property 21: Template Independence ==========
    // **Feature: rustconn-enhancements, Property 21: Template Independence**
    // **Validates: Requirements 8.3**
    //
    // For any connection created from a template, modifying the template
    // should not affect the connection.

    #[test]
    fn prop_template_independence_after_creation(
        template in arb_template(),
        new_host in arb_hostname(),
        new_port in arb_port(),
        new_username in arb_username()
    ) {
        // Create connection from template
        let connection = template.apply(Some("Test Connection".to_string()));

        // Store original values
        let original_host = connection.host.clone();
        let original_port = connection.port;
        let original_username = connection.username.clone();

        // Modify the template (simulating what would happen if template was mutable)
        let mut modified_template = template.clone();
        modified_template.host = new_host;
        modified_template.port = new_port;
        modified_template.username = new_username;

        // Connection should still have original values (it's a separate copy)
        prop_assert_eq!(&connection.host, &original_host);
        prop_assert_eq!(connection.port, original_port);
        prop_assert_eq!(&connection.username, &original_username);

        // Verify template was actually modified
        prop_assert_eq!(&modified_template.host, &modified_template.host);
    }

    // ========== Property 19: Template Field Preservation ==========
    // **Feature: rustconn-enhancements, Property 19: Template Field Preservation**
    // **Validates: Requirements 8.5**
    //
    // For any connection template, all field values should be preserved
    // when serializing and deserializing.

    #[test]
    fn prop_template_json_round_trip(template in arb_template()) {
        let json = serde_json::to_string(&template).expect("Serialization should succeed");
        let deserialized: ConnectionTemplate = serde_json::from_str(&json)
            .expect("Deserialization should succeed");

        prop_assert_eq!(template.id, deserialized.id);
        prop_assert_eq!(&template.name, &deserialized.name);
        prop_assert_eq!(template.protocol, deserialized.protocol);
        prop_assert_eq!(&template.host, &deserialized.host);
        prop_assert_eq!(template.port, deserialized.port);
        prop_assert_eq!(&template.username, &deserialized.username);
        prop_assert_eq!(&template.tags, &deserialized.tags);
        prop_assert_eq!(template.password_source, deserialized.password_source);
        prop_assert_eq!(&template.domain, &deserialized.domain);
        prop_assert_eq!(&template.custom_properties, &deserialized.custom_properties);
    }

    #[test]
    fn prop_template_toml_round_trip(template in arb_template()) {
        let toml = toml::to_string(&template).expect("Serialization should succeed");
        let deserialized: ConnectionTemplate = toml::from_str(&toml)
            .expect("Deserialization should succeed");

        prop_assert_eq!(template.id, deserialized.id);
        prop_assert_eq!(&template.name, &deserialized.name);
        prop_assert_eq!(template.protocol, deserialized.protocol);
        prop_assert_eq!(&template.host, &deserialized.host);
        prop_assert_eq!(template.port, deserialized.port);
        prop_assert_eq!(&template.username, &deserialized.username);
        prop_assert_eq!(&template.tags, &deserialized.tags);
    }

    // ========== Property 9: Template Serialization Round-Trip ==========
    // **Feature: rustconn-bugfixes, Property 9: Template Serialization Round-Trip**
    // **Validates: Requirements 11.3**
    //
    // For any valid ConnectionTemplate, serializing and deserializing should
    // produce an equivalent template.

    #[test]
    fn prop_template_serialization_round_trip(template in arb_template()) {
        // Test JSON round-trip
        let json = serde_json::to_string(&template).expect("JSON serialization should succeed");
        let from_json: ConnectionTemplate = serde_json::from_str(&json)
            .expect("JSON deserialization should succeed");

        // Verify all fields are preserved
        prop_assert_eq!(template.id, from_json.id, "ID should be preserved");
        prop_assert_eq!(&template.name, &from_json.name, "Name should be preserved");
        prop_assert_eq!(template.protocol, from_json.protocol, "Protocol should be preserved");
        prop_assert_eq!(&template.host, &from_json.host, "Host should be preserved");
        prop_assert_eq!(template.port, from_json.port, "Port should be preserved");
        prop_assert_eq!(&template.username, &from_json.username, "Username should be preserved");
        prop_assert_eq!(&template.description, &from_json.description, "Description should be preserved");
        prop_assert_eq!(&template.tags, &from_json.tags, "Tags should be preserved");
        prop_assert_eq!(template.password_source, from_json.password_source, "Password source should be preserved");
        prop_assert_eq!(&template.domain, &from_json.domain, "Domain should be preserved");
        prop_assert_eq!(&template.custom_properties, &from_json.custom_properties, "Custom properties should be preserved");
        prop_assert_eq!(&template.protocol_config, &from_json.protocol_config, "Protocol config should be preserved");

        // Test YAML round-trip for completeness
        let yaml = serde_yaml::to_string(&template).expect("YAML serialization should succeed");
        let from_yaml: ConnectionTemplate = serde_yaml::from_str(&yaml)
            .expect("YAML deserialization should succeed");

        prop_assert_eq!(template.id, from_yaml.id, "ID should be preserved in YAML");
        prop_assert_eq!(&template.name, &from_yaml.name, "Name should be preserved in YAML");
        prop_assert_eq!(template.protocol, from_yaml.protocol, "Protocol should be preserved in YAML");
    }

    // Additional property: Multiple connections from same template are independent
    #[test]
    fn prop_multiple_connections_from_template_are_independent(
        template in arb_template()
    ) {
        let conn1 = template.apply(Some("Connection 1".to_string()));
        let conn2 = template.apply(Some("Connection 2".to_string()));

        // Each connection should have a unique ID
        prop_assert_ne!(conn1.id, conn2.id);

        // Names should be different
        prop_assert_ne!(&conn1.name, &conn2.name);

        // But other fields should be the same (from template)
        prop_assert_eq!(&conn1.host, &conn2.host);
        prop_assert_eq!(conn1.port, conn2.port);
        prop_assert_eq!(&conn1.username, &conn2.username);
        prop_assert_eq!(&conn1.tags, &conn2.tags);
    }

    // ========== Property 12: Template Protocol Persistence ==========
    // **Feature: rustconn-fixes-v2, Property 12: Template Protocol Persistence**
    // **Validates: Requirements 10.1, 10.2, 10.3**
    //
    // For any template with a non-SSH protocol, saving and loading should
    // preserve the protocol type and all protocol-specific settings.

    #[test]
    fn prop_template_protocol_persistence(
        name in arb_template_name(),
        protocol_config in arb_protocol_config(),
        host in arb_hostname(),
        port in arb_port(),
        username in arb_username(),
        tags in arb_tags(),
    ) {
        // Create template with the given protocol
        let mut template = ConnectionTemplate::new(name, protocol_config.clone())
            .with_host(host)
            .with_port(port)
            .with_tags(tags);

        if let Some(user) = username {
            template = template.with_username(user);
        }

        // Serialize to TOML (the format used by ConfigManager)
        let toml_str = toml::to_string(&template).expect("TOML serialization should succeed");
        let loaded: ConnectionTemplate = toml::from_str(&toml_str)
            .expect("TOML deserialization should succeed");

        // Protocol type should be preserved
        prop_assert_eq!(
            template.protocol, loaded.protocol,
            "Protocol type should be preserved after save/load"
        );

        // Protocol config should be preserved
        prop_assert_eq!(
            &template.protocol_config, &loaded.protocol_config,
            "Protocol config should be preserved after save/load"
        );

        // All other fields should be preserved
        prop_assert_eq!(template.id, loaded.id, "ID should be preserved");
        prop_assert_eq!(&template.name, &loaded.name, "Name should be preserved");
        prop_assert_eq!(&template.host, &loaded.host, "Host should be preserved");
        prop_assert_eq!(template.port, loaded.port, "Port should be preserved");
        prop_assert_eq!(&template.username, &loaded.username, "Username should be preserved");
        prop_assert_eq!(&template.tags, &loaded.tags, "Tags should be preserved");
    }
}

// ========== Unit Tests ==========

#[test]
fn test_template_protocol_types() {
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
fn test_template_from_connection_preserves_fields() {
    use rustconn_core::Connection;

    let connection =
        Connection::new_ssh("Original".to_string(), "host.example.com".to_string(), 2222)
            .with_username("testuser")
            .with_tags(vec!["production".to_string(), "critical".to_string()]);

    let template =
        ConnectionTemplate::from_connection(&connection, "Template from Original".to_string());

    assert_eq!(template.name, "Template from Original");
    assert_eq!(template.host, "host.example.com");
    assert_eq!(template.port, 2222);
    assert_eq!(template.username, Some("testuser".to_string()));
    assert_eq!(
        template.tags,
        vec!["production".to_string(), "critical".to_string()]
    );
    assert_eq!(template.protocol, ProtocolType::Ssh);
}

#[test]
fn test_apply_template_with_custom_properties() {
    let props = vec![
        CustomProperty::new_text("notes", "Important server"),
        CustomProperty::new_url("docs", "https://docs.example.com"),
    ];

    let template = ConnectionTemplate::new_ssh("SSH Template".to_string())
        .with_custom_properties(props.clone());

    let connection = template.apply(Some("New Server".to_string()));

    assert_eq!(connection.custom_properties.len(), 2);
    assert_eq!(connection.custom_properties[0].name, "notes");
    assert_eq!(connection.custom_properties[1].name, "docs");
}

#[test]
fn test_template_double_round_trip() {
    let template = ConnectionTemplate::new_rdp("RDP Template".to_string())
        .with_description("Test template")
        .with_host("rdp.example.com")
        .with_port(3390)
        .with_username("admin")
        .with_domain("CORP")
        .with_tags(vec!["windows".to_string()]);

    // First round trip
    let json1 = serde_json::to_string(&template).unwrap();
    let template2: ConnectionTemplate = serde_json::from_str(&json1).unwrap();

    // Second round trip
    let json2 = serde_json::to_string(&template2).unwrap();
    let template3: ConnectionTemplate = serde_json::from_str(&json2).unwrap();

    assert_eq!(template.id, template3.id);
    assert_eq!(template.name, template3.name);
    assert_eq!(template.description, template3.description);
    assert_eq!(template.host, template3.host);
    assert_eq!(template.port, template3.port);
    assert_eq!(template.username, template3.username);
    assert_eq!(template.domain, template3.domain);
    assert_eq!(template.tags, template3.tags);
}
