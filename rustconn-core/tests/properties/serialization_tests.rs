//! Property-based tests for serialization round-trip
//!
//! **Feature: rustconn, Property 6: Connection Serialization Round-Trip**
//! **Validates: Requirements 10.5, 10.6**

use proptest::prelude::*;
use rustconn_core::models::SharedFolder;
use rustconn_core::{
    Connection, ProtocolConfig, RdpConfig, RdpGateway, Resolution, SpiceConfig,
    SpiceImageCompression, SshAuthMethod, SshConfig, SshKeySource, VncConfig,
};
use std::collections::HashMap;
use std::path::PathBuf;

// Strategy for generating valid connection names
fn arb_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_-]{0,31}".prop_map(|s| s)
}

// Strategy for generating valid hostnames
fn arb_host() -> impl Strategy<Value = String> {
    "[a-z0-9]([a-z0-9-]{0,15}[a-z0-9])?(\\.[a-z0-9]([a-z0-9-]{0,15}[a-z0-9])?)*".prop_map(|s| s)
}

// Strategy for generating valid ports
fn arb_port() -> impl Strategy<Value = u16> {
    1u16..=65535u16
}

// Strategy for generating optional usernames
fn arb_username() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), "[a-z][a-z0-9_]{0,15}".prop_map(Some),]
}

// Strategy for generating tags
fn arb_tags() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec("[a-z]{1,10}", 0..5)
}

// Strategy for SSH auth method
fn arb_ssh_auth_method() -> impl Strategy<Value = SshAuthMethod> {
    prop_oneof![
        Just(SshAuthMethod::Password),
        Just(SshAuthMethod::PublicKey),
        Just(SshAuthMethod::KeyboardInteractive),
        Just(SshAuthMethod::Agent),
    ]
}

// Strategy for optional PathBuf
fn arb_optional_path() -> impl Strategy<Value = Option<PathBuf>> {
    prop_oneof![
        Just(None),
        "[a-z]{1,10}(/[a-z]{1,10}){0,3}".prop_map(|s| Some(PathBuf::from(s))),
    ]
}

// Strategy for optional string
fn arb_optional_string() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), "[a-zA-Z0-9_-]{1,20}".prop_map(Some),]
}

// Strategy for custom SSH options
fn arb_custom_options() -> impl Strategy<Value = HashMap<String, String>> {
    prop::collection::hash_map("[A-Za-z]{1,20}", "[a-zA-Z0-9]{1,10}", 0..3)
}

// Strategy for SSH config
fn arb_ssh_config() -> impl Strategy<Value = SshConfig> {
    (
        arb_ssh_auth_method(),
        arb_optional_path(),
        arb_optional_string(),
        any::<bool>(),
        arb_custom_options(),
        arb_optional_string(),
    )
        .prop_map(
            |(
                auth_method,
                key_path,
                proxy_jump,
                use_control_master,
                custom_options,
                startup_command,
            )| {
                SshConfig {
                    auth_method,
                    key_path,
                    key_source: SshKeySource::Default,
                    agent_key_fingerprint: None,
                    identities_only: false,
                    proxy_jump,
                    use_control_master,
                    agent_forwarding: false,
                    custom_options,
                    startup_command,
                    jump_host_id: None,
                }
            },
        )
}

// Strategy for optional resolution
fn arb_optional_resolution() -> impl Strategy<Value = Option<Resolution>> {
    prop_oneof![
        Just(None),
        (640u32..4096u32, 480u32..2160u32).prop_map(|(w, h)| Some(Resolution::new(w, h))),
    ]
}

// Strategy for optional color depth
fn arb_optional_color_depth() -> impl Strategy<Value = Option<u8>> {
    prop_oneof![
        Just(None),
        prop_oneof![Just(8u8), Just(15u8), Just(16u8), Just(24u8), Just(32u8)].prop_map(Some),
    ]
}

// Strategy for optional RDP gateway
fn arb_optional_gateway() -> impl Strategy<Value = Option<RdpGateway>> {
    prop_oneof![
        Just(None),
        (arb_host(), 1u16..65535u16, arb_optional_string()).prop_map(
            |(hostname, port, username)| {
                Some(RdpGateway {
                    hostname,
                    port,
                    username,
                })
            }
        ),
    ]
}

// Strategy for custom args
fn arb_custom_args() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec("[a-zA-Z0-9_=-]{1,20}", 0..3)
}

// Strategy for RDP config
fn arb_rdp_config() -> impl Strategy<Value = RdpConfig> {
    (
        arb_optional_resolution(),
        arb_optional_color_depth(),
        any::<bool>(),
        arb_optional_gateway(),
        arb_custom_args(),
    )
        .prop_map(
            |(resolution, color_depth, audio_redirect, gateway, custom_args)| RdpConfig {
                resolution,
                color_depth,
                audio_redirect,
                gateway,
                shared_folders: Vec::new(),
                custom_args,
                client_mode: Default::default(),
                performance_mode: Default::default(),
            },
        )
}

// Strategy for optional encoding
fn arb_optional_encoding() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        Just(None),
        prop_oneof![
            Just("tight".to_string()),
            Just("zrle".to_string()),
            Just("hextile".to_string()),
        ]
        .prop_map(Some),
    ]
}

// Strategy for optional compression/quality (0-9)
fn arb_optional_level() -> impl Strategy<Value = Option<u8>> {
    prop_oneof![Just(None), (0u8..=9u8).prop_map(Some),]
}

// Strategy for VNC config
fn arb_vnc_config() -> impl Strategy<Value = VncConfig> {
    (
        arb_optional_encoding(),
        arb_optional_level(),
        arb_optional_level(),
        arb_custom_args(),
    )
        .prop_map(|(encoding, compression, quality, custom_args)| VncConfig {
            client_mode: Default::default(),
            performance_mode: Default::default(),
            encoding,
            compression,
            quality,
            view_only: false,
            scaling: true,
            clipboard_enabled: true,
            custom_args,
        })
}

// Strategy for SPICE image compression
fn arb_spice_image_compression() -> impl Strategy<Value = Option<SpiceImageCompression>> {
    prop_oneof![
        Just(None),
        Just(Some(SpiceImageCompression::Auto)),
        Just(Some(SpiceImageCompression::Off)),
        Just(Some(SpiceImageCompression::Glz)),
        Just(Some(SpiceImageCompression::Lz)),
        Just(Some(SpiceImageCompression::Quic)),
    ]
}

// Strategy for shared folders
fn arb_shared_folders() -> impl Strategy<Value = Vec<SharedFolder>> {
    prop::collection::vec(
        (
            "/[a-z]{1,10}(/[a-z]{1,10}){0,2}",
            "[A-Za-z][A-Za-z0-9_]{0,10}",
        )
            .prop_map(|(path, name)| SharedFolder {
                local_path: PathBuf::from(path),
                share_name: name,
            }),
        0..3,
    )
}

// Strategy for SPICE config
fn arb_spice_config() -> impl Strategy<Value = SpiceConfig> {
    (
        any::<bool>(),                 // tls_enabled
        arb_optional_path(),           // ca_cert_path
        any::<bool>(),                 // skip_cert_verify
        any::<bool>(),                 // usb_redirection
        arb_shared_folders(),          // shared_folders
        any::<bool>(),                 // clipboard_enabled
        arb_spice_image_compression(), // image_compression
    )
        .prop_map(
            |(
                tls_enabled,
                ca_cert_path,
                skip_cert_verify,
                usb_redirection,
                shared_folders,
                clipboard_enabled,
                image_compression,
            )| SpiceConfig {
                tls_enabled,
                ca_cert_path,
                skip_cert_verify,
                usb_redirection,
                shared_folders,
                clipboard_enabled,
                image_compression,
            },
        )
}

// Strategy for protocol config
fn arb_protocol_config() -> impl Strategy<Value = ProtocolConfig> {
    prop_oneof![
        arb_ssh_config().prop_map(ProtocolConfig::Ssh),
        arb_rdp_config().prop_map(ProtocolConfig::Rdp),
        arb_vnc_config().prop_map(ProtocolConfig::Vnc),
        arb_spice_config().prop_map(ProtocolConfig::Spice),
    ]
}

// Strategy for generating a complete Connection
fn arb_connection() -> impl Strategy<Value = Connection> {
    (
        arb_name(),
        arb_host(),
        arb_port(),
        arb_protocol_config(),
        arb_username(),
        arb_tags(),
    )
        .prop_map(|(name, host, port, protocol_config, username, tags)| {
            let mut conn = Connection::new(name, host, port, protocol_config);
            if let Some(u) = username {
                conn = conn.with_username(u);
            }
            if !tags.is_empty() {
                conn = conn.with_tags(tags);
            }
            conn
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn, Property 6: Connection Serialization Round-Trip**
    /// **Validates: Requirements 10.5, 10.6**
    ///
    /// For any valid Connection object, serializing to TOML and then deserializing
    /// should produce an equivalent Connection object with all fields preserved.
    #[test]
    fn connection_toml_round_trip(conn in arb_connection()) {
        // Serialize to TOML
        let toml_str = toml::to_string(&conn)
            .expect("Connection should serialize to TOML");

        // Deserialize back from TOML
        let deserialized: Connection = toml::from_str(&toml_str)
            .expect("TOML should deserialize back to Connection");

        // Verify all fields are preserved
        prop_assert_eq!(conn.id, deserialized.id, "ID should be preserved");
        prop_assert_eq!(conn.name, deserialized.name, "Name should be preserved");
        prop_assert_eq!(conn.protocol, deserialized.protocol, "Protocol type should be preserved");
        prop_assert_eq!(conn.host, deserialized.host, "Host should be preserved");
        prop_assert_eq!(conn.port, deserialized.port, "Port should be preserved");
        prop_assert_eq!(conn.username, deserialized.username, "Username should be preserved");
        prop_assert_eq!(conn.group_id, deserialized.group_id, "Group ID should be preserved");
        prop_assert_eq!(conn.tags, deserialized.tags, "Tags should be preserved");
        prop_assert_eq!(conn.protocol_config, deserialized.protocol_config, "Protocol config should be preserved");

        // Timestamps may have nanosecond precision loss in TOML, so compare at second precision
        prop_assert_eq!(
            conn.created_at.timestamp(),
            deserialized.created_at.timestamp(),
            "Created timestamp should be preserved (second precision)"
        );
        prop_assert_eq!(
            conn.updated_at.timestamp(),
            deserialized.updated_at.timestamp(),
            "Updated timestamp should be preserved (second precision)"
        );
    }

    /// Additional test: Connection JSON round-trip for completeness
    /// This validates that the serde implementation works across formats
    #[test]
    fn connection_json_round_trip(conn in arb_connection()) {
        // Serialize to JSON
        let json_str = serde_json::to_string(&conn)
            .expect("Connection should serialize to JSON");

        // Deserialize back from JSON
        let deserialized: Connection = serde_json::from_str(&json_str)
            .expect("JSON should deserialize back to Connection");

        // Verify equality (JSON preserves nanosecond precision)
        prop_assert_eq!(conn, deserialized, "Connection should round-trip through JSON");
    }

    /// **Feature: native-protocol-embedding, Property 4: Protocol configuration round-trip serialization**
    /// **Validates: Requirements 6.2**
    ///
    /// For any valid SpiceConfig, serializing to TOML/JSON and deserializing back
    /// should produce an equivalent configuration with all fields preserved.
    #[test]
    fn spice_config_toml_round_trip(config in arb_spice_config()) {
        // Wrap in ProtocolConfig for proper serialization with type tag
        let protocol_config = ProtocolConfig::Spice(config.clone());

        // Serialize to TOML
        let toml_str = toml::to_string(&protocol_config)
            .expect("SpiceConfig should serialize to TOML");

        // Deserialize back from TOML
        let deserialized: ProtocolConfig = toml::from_str(&toml_str)
            .expect("TOML should deserialize back to ProtocolConfig");

        // Verify the config is preserved
        if let ProtocolConfig::Spice(deserialized_config) = deserialized {
            prop_assert_eq!(config.tls_enabled, deserialized_config.tls_enabled, "tls_enabled should be preserved");
            prop_assert_eq!(config.ca_cert_path, deserialized_config.ca_cert_path, "ca_cert_path should be preserved");
            prop_assert_eq!(config.skip_cert_verify, deserialized_config.skip_cert_verify, "skip_cert_verify should be preserved");
            prop_assert_eq!(config.usb_redirection, deserialized_config.usb_redirection, "usb_redirection should be preserved");
            prop_assert_eq!(config.shared_folders, deserialized_config.shared_folders, "shared_folders should be preserved");
            prop_assert_eq!(config.clipboard_enabled, deserialized_config.clipboard_enabled, "clipboard_enabled should be preserved");
            prop_assert_eq!(config.image_compression, deserialized_config.image_compression, "image_compression should be preserved");
        } else {
            prop_assert!(false, "Deserialized config should be Spice variant");
        }
    }

    /// **Feature: native-protocol-embedding, Property 4: Protocol configuration round-trip serialization**
    /// **Validates: Requirements 6.2**
    ///
    /// For any valid SpiceConfig, JSON round-trip should preserve all fields.
    #[test]
    fn spice_config_json_round_trip(config in arb_spice_config()) {
        // Wrap in ProtocolConfig for proper serialization with type tag
        let protocol_config = ProtocolConfig::Spice(config.clone());

        // Serialize to JSON
        let json_str = serde_json::to_string(&protocol_config)
            .expect("SpiceConfig should serialize to JSON");

        // Deserialize back from JSON
        let deserialized: ProtocolConfig = serde_json::from_str(&json_str)
            .expect("JSON should deserialize back to ProtocolConfig");

        // Verify equality
        prop_assert_eq!(protocol_config, deserialized, "SpiceConfig should round-trip through JSON");
    }
}
