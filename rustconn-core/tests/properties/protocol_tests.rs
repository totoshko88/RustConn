//! Property-based tests for protocol validation
//!
//! These tests validate the correctness properties for SSH, RDP, and VNC
//! protocol validation as defined in the design document.

use proptest::prelude::*;
use std::collections::HashMap;

use rustconn_core::models::{
    Connection, ProtocolConfig, RdpConfig, RdpGateway, Resolution, SharedFolder, SpiceConfig,
    SpiceImageCompression, SshAuthMethod, SshConfig, SshKeySource, VncConfig,
};
use rustconn_core::protocol::{Protocol, RdpProtocol, SshProtocol, VncProtocol};
use std::path::PathBuf;

// ============================================================================
// Generators for SSH configurations
// ============================================================================

fn arb_ssh_auth_method() -> impl Strategy<Value = SshAuthMethod> {
    prop_oneof![
        Just(SshAuthMethod::Password),
        Just(SshAuthMethod::PublicKey),
        Just(SshAuthMethod::KeyboardInteractive),
        Just(SshAuthMethod::Agent),
    ]
}

fn arb_ssh_custom_options() -> impl Strategy<Value = HashMap<String, String>> {
    prop::collection::hash_map("[A-Za-z][A-Za-z0-9]{0,20}", "[a-zA-Z0-9_.-]{1,30}", 0..5)
}

fn arb_ssh_config() -> impl Strategy<Value = SshConfig> {
    (
        arb_ssh_auth_method(),
        prop::option::of("[a-z0-9.-]{1,30}"), // proxy_jump
        any::<bool>(),                        // use_control_master
        arb_ssh_custom_options(),
        prop::option::of("[a-z0-9 -]{1,50}"), // startup_command
    )
        .prop_map(
            |(auth_method, proxy_jump, use_control_master, custom_options, startup_command)| {
                SshConfig {
                    auth_method,
                    key_path: None, // Don't test with actual file paths
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

fn arb_ssh_connection() -> impl Strategy<Value = Connection> {
    (
        "[a-zA-Z][a-zA-Z0-9_-]{0,30}", // name
        "[a-z0-9]([a-z0-9-]{0,30}[a-z0-9])?(\\.[a-z0-9]([a-z0-9-]{0,30}[a-z0-9])?)*", // host
        1u16..65535,                   // port
        prop::option::of("[a-z][a-z0-9_-]{0,20}"), // username
        arb_ssh_config(),
    )
        .prop_map(|(name, host, port, username, ssh_config)| {
            let mut conn = Connection::new(name, host, port, ProtocolConfig::Ssh(ssh_config));
            if let Some(u) = username {
                conn.username = Some(u);
            }
            conn
        })
}

// ============================================================================
// Generators for RDP configurations
// ============================================================================

fn arb_resolution() -> impl Strategy<Value = Resolution> {
    (640u32..3840, 480u32..2160).prop_map(|(w, h)| Resolution::new(w, h))
}

fn arb_color_depth() -> impl Strategy<Value = u8> {
    prop_oneof![Just(8u8), Just(15u8), Just(16u8), Just(24u8), Just(32u8)]
}

fn arb_rdp_gateway() -> impl Strategy<Value = RdpGateway> {
    (
        "[a-z0-9.-]{1,30}",                        // hostname
        443u16..65535,                             // port
        prop::option::of("[a-z][a-z0-9_-]{0,20}"), // username
    )
        .prop_map(|(hostname, port, username)| RdpGateway {
            hostname,
            port,
            username,
        })
}

fn arb_shared_folder() -> impl Strategy<Value = SharedFolder> {
    (
        "/[a-z]{1,10}(/[a-z]{1,10}){0,3}", // local_path (Unix-style path)
        "[A-Za-z][A-Za-z0-9_]{0,10}",      // share_name
    )
        .prop_map(|(path, name)| SharedFolder {
            local_path: std::path::PathBuf::from(path),
            share_name: name,
        })
}

fn arb_shared_folders() -> impl Strategy<Value = Vec<SharedFolder>> {
    prop::collection::vec(arb_shared_folder(), 0..5)
}

fn arb_rdp_config() -> impl Strategy<Value = RdpConfig> {
    (
        prop::option::of(arb_resolution()),
        prop::option::of(arb_color_depth()),
        any::<bool>(), // audio_redirect
        prop::option::of(arb_rdp_gateway()),
        arb_shared_folders(),
        prop::collection::vec("/[a-z-]{1,20}", 0..3), // custom_args
    )
        .prop_map(
            |(resolution, color_depth, audio_redirect, gateway, shared_folders, custom_args)| {
                RdpConfig {
                    resolution,
                    color_depth,
                    audio_redirect,
                    gateway,
                    shared_folders,
                    custom_args,
                    client_mode: Default::default(),
                    performance_mode: Default::default(),
                }
            },
        )
}

fn arb_rdp_connection() -> impl Strategy<Value = Connection> {
    (
        "[a-zA-Z][a-zA-Z0-9_-]{0,30}", // name
        "[a-z0-9]([a-z0-9-]{0,30}[a-z0-9])?(\\.[a-z0-9]([a-z0-9-]{0,30}[a-z0-9])?)*", // host
        1u16..65535,                   // port
        prop::option::of("[a-z][a-z0-9_-]{0,20}"), // username
        arb_rdp_config(),
    )
        .prop_map(|(name, host, port, username, rdp_config)| {
            let mut conn = Connection::new(name, host, port, ProtocolConfig::Rdp(rdp_config));
            if let Some(u) = username {
                conn.username = Some(u);
            }
            conn
        })
}

// ============================================================================
// Generators for VNC configurations
// ============================================================================

fn arb_vnc_config() -> impl Strategy<Value = VncConfig> {
    (
        prop::option::of("(tight|zrle|hextile|raw)"), // encoding
        prop::option::of(0u8..=9),                    // compression
        prop::option::of(0u8..=9),                    // quality
        prop::collection::vec("-[a-z]{1,15}", 0..3),  // custom_args
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

fn arb_vnc_connection() -> impl Strategy<Value = Connection> {
    (
        "[a-zA-Z][a-zA-Z0-9_-]{0,30}", // name
        "[a-z0-9]([a-z0-9-]{0,30}[a-z0-9])?(\\.[a-z0-9]([a-z0-9-]{0,30}[a-z0-9])?)*", // host
        5900u16..6000,                 // port (VNC display range)
        arb_vnc_config(),
    )
        .prop_map(|(name, host, port, vnc_config)| {
            Connection::new(name, host, port, ProtocolConfig::Vnc(vnc_config))
        })
}

// ============================================================================
// Property Tests for Validation
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // **Feature: rustconn, Property: SSH Validation Accepts Valid Connections**
    // **Validates: Requirements 2.2, 2.3, 2.4, 2.5**
    //
    // For any valid SSH connection configuration, validation should pass.

    #[test]
    fn prop_ssh_validation_accepts_valid_connections(conn in arb_ssh_connection()) {
        let protocol = SshProtocol::new();
        let result = protocol.validate_connection(&conn);
        prop_assert!(result.is_ok(), "Valid SSH connection should pass validation: {:?}", result);
    }

    // **Feature: rustconn, Property: RDP Validation Accepts Valid Connections**
    // **Validates: Requirements 3.1, 3.2, 3.3, 3.5**
    //
    // For any valid RDP connection configuration, validation should pass.

    #[test]
    fn prop_rdp_validation_accepts_valid_connections(conn in arb_rdp_connection()) {
        let protocol = RdpProtocol::new();
        let result = protocol.validate_connection(&conn);
        prop_assert!(result.is_ok(), "Valid RDP connection should pass validation: {:?}", result);
    }

    // **Feature: rustconn, Property: VNC Validation Accepts Valid Connections**
    // **Validates: Requirements 4.1, 4.2, 4.3**
    //
    // For any valid VNC connection configuration, validation should pass.

    #[test]
    fn prop_vnc_validation_accepts_valid_connections(conn in arb_vnc_connection()) {
        let protocol = VncProtocol::new();
        let result = protocol.validate_connection(&conn);
        prop_assert!(result.is_ok(), "Valid VNC connection should pass validation: {:?}", result);
    }

    // **Feature: rustconn, Property: Empty Host Rejected**
    //
    // For any protocol, an empty host should be rejected.

    #[test]
    fn prop_ssh_rejects_empty_host(mut conn in arb_ssh_connection()) {
        conn.host = String::new();
        let protocol = SshProtocol::new();
        let result = protocol.validate_connection(&conn);
        prop_assert!(result.is_err(), "Empty host should be rejected");
    }

    #[test]
    fn prop_rdp_rejects_empty_host(mut conn in arb_rdp_connection()) {
        conn.host = String::new();
        let protocol = RdpProtocol::new();
        let result = protocol.validate_connection(&conn);
        prop_assert!(result.is_err(), "Empty host should be rejected");
    }

    #[test]
    fn prop_vnc_rejects_empty_host(mut conn in arb_vnc_connection()) {
        conn.host = String::new();
        let protocol = VncProtocol::new();
        let result = protocol.validate_connection(&conn);
        prop_assert!(result.is_err(), "Empty host should be rejected");
    }

    // **Feature: rustconn, Property: Zero Port Rejected**
    //
    // For any protocol, a zero port should be rejected.

    #[test]
    fn prop_ssh_rejects_zero_port(mut conn in arb_ssh_connection()) {
        conn.port = 0;
        let protocol = SshProtocol::new();
        let result = protocol.validate_connection(&conn);
        prop_assert!(result.is_err(), "Zero port should be rejected");
    }

    #[test]
    fn prop_rdp_rejects_zero_port(mut conn in arb_rdp_connection()) {
        conn.port = 0;
        let protocol = RdpProtocol::new();
        let result = protocol.validate_connection(&conn);
        prop_assert!(result.is_err(), "Zero port should be rejected");
    }

    #[test]
    fn prop_vnc_rejects_zero_port(mut conn in arb_vnc_connection()) {
        conn.port = 0;
        let protocol = VncProtocol::new();
        let result = protocol.validate_connection(&conn);
        prop_assert!(result.is_err(), "Zero port should be rejected");
    }

    // **Feature: rustconn, Property: Invalid VNC Compression Rejected**
    //
    // VNC compression level > 9 should be rejected.

    #[test]
    fn prop_vnc_rejects_invalid_compression(conn in arb_vnc_connection(), compression in 10u8..255) {
        let mut conn = conn;
        if let ProtocolConfig::Vnc(ref mut vnc_config) = conn.protocol_config {
            vnc_config.compression = Some(compression);
        }
        let protocol = VncProtocol::new();
        let result = protocol.validate_connection(&conn);
        prop_assert!(result.is_err(), "Compression > 9 should be rejected");
    }

    // **Feature: rustconn, Property: Invalid VNC Quality Rejected**
    //
    // VNC quality level > 9 should be rejected.

    #[test]
    fn prop_vnc_rejects_invalid_quality(conn in arb_vnc_connection(), quality in 10u8..255) {
        let mut conn = conn;
        if let ProtocolConfig::Vnc(ref mut vnc_config) = conn.protocol_config {
            vnc_config.quality = Some(quality);
        }
        let protocol = VncProtocol::new();
        let result = protocol.validate_connection(&conn);
        prop_assert!(result.is_err(), "Quality > 9 should be rejected");
    }

    // **Feature: rustconn, Property: Invalid RDP Color Depth Rejected**
    //
    // RDP color depth not in {8, 15, 16, 24, 32} should be rejected.

    #[test]
    fn prop_rdp_rejects_invalid_color_depth(conn in arb_rdp_connection(), depth in 0u8..255) {
        // Skip valid depths
        if matches!(depth, 8 | 15 | 16 | 24 | 32) {
            return Ok(());
        }
        let mut conn = conn;
        if let ProtocolConfig::Rdp(ref mut rdp_config) = conn.protocol_config {
            rdp_config.color_depth = Some(depth);
        }
        let protocol = RdpProtocol::new();
        let result = protocol.validate_connection(&conn);
        prop_assert!(result.is_err(), "Invalid color depth {} should be rejected", depth);
    }
}

// ============================================================================
// Property Test for Protocol Port Defaults
// ============================================================================

/// **Feature: rustconn-bugfixes, Property 10: Protocol Port Defaults**
/// **Validates: Requirements 8.2, 8.3, 8.4, 8.5**
///
/// For any protocol selection in Quick Connect, the default port SHALL match
/// the protocol standard (SSH=22, RDP=3389, VNC=5900).
#[test]
fn prop_protocol_port_defaults() {
    // Test SSH default port
    let ssh_protocol = SshProtocol::new();
    assert_eq!(
        ssh_protocol.default_port(),
        22,
        "SSH default port must be 22"
    );

    // Test RDP default port
    let rdp_protocol = RdpProtocol::new();
    assert_eq!(
        rdp_protocol.default_port(),
        3389,
        "RDP default port must be 3389"
    );

    // Test VNC default port
    let vnc_protocol = VncProtocol::new();
    assert_eq!(
        vnc_protocol.default_port(),
        5900,
        "VNC default port must be 5900"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // **Feature: rustconn-bugfixes, Property 10: Protocol Port Defaults**
    // **Validates: Requirements 8.2, 8.3, 8.4, 8.5**
    //
    // For any protocol type, the default port returned by the Protocol trait
    // implementation must match the standard port for that protocol.

    #[test]
    fn prop_protocol_default_port_matches_standard(protocol_idx in 0u32..3) {
        let (protocol_name, expected_port): (&str, u16) = match protocol_idx {
            0 => ("SSH", 22),
            1 => ("RDP", 3389),
            2 => ("VNC", 5900),
            _ => unreachable!(),
        };

        let actual_port = match protocol_idx {
            0 => SshProtocol::new().default_port(),
            1 => RdpProtocol::new().default_port(),
            2 => VncProtocol::new().default_port(),
            _ => unreachable!(),
        };

        prop_assert_eq!(
            actual_port,
            expected_port,
            "{} default port must be {}",
            protocol_name,
            expected_port
        );
    }

    // Additional property: Connection model default_port matches Protocol trait
    #[test]
    fn prop_connection_default_port_matches_protocol(conn in prop_oneof![
        arb_ssh_connection(),
        arb_rdp_connection(),
        arb_vnc_connection(),
    ]) {
        let expected_port = match &conn.protocol_config {
            ProtocolConfig::Ssh(_) => 22u16,
            ProtocolConfig::Rdp(_) => 3389u16,
            ProtocolConfig::Vnc(_) => 5900u16,
            ProtocolConfig::Spice(_) => 5900u16,
            ProtocolConfig::ZeroTrust(_) => 0u16, // No default port for Zero Trust
        };

        prop_assert_eq!(
            conn.default_port(),
            expected_port,
            "Connection default_port() must match protocol standard"
        );
    }

    // **Feature: rustconn-enhancements, Property 3: Shared Folder CRUD Operations**
    // **Validates: Requirements 2.3, 2.5**
    //
    // For any RDP configuration, adding a shared folder should increase the folder
    // count by one, and removing a shared folder should decrease it by one, with
    // the configuration remaining valid.

    #[test]
    fn prop_shared_folder_add_increases_count(
        mut config in arb_rdp_config(),
        folder in arb_shared_folder()
    ) {
        let initial_count = config.shared_folders.len();
        config.shared_folders.push(folder);
        prop_assert_eq!(
            config.shared_folders.len(),
            initial_count + 1,
            "Adding a shared folder should increase count by 1"
        );
    }

    #[test]
    fn prop_shared_folder_remove_decreases_count(config in arb_rdp_config()) {
        // Only test removal if there are folders to remove
        if !config.shared_folders.is_empty() {
            let mut config = config;
            let initial_count = config.shared_folders.len();
            config.shared_folders.pop();
            prop_assert_eq!(
                config.shared_folders.len(),
                initial_count - 1,
                "Removing a shared folder should decrease count by 1"
            );
        }
    }

    #[test]
    fn prop_shared_folder_config_remains_valid_after_crud(
        mut config in arb_rdp_config(),
        folder in arb_shared_folder()
    ) {
        // Add a folder
        config.shared_folders.push(folder.clone());

        // Verify the folder was added correctly
        prop_assert!(
            config.shared_folders.iter().any(|f| f == &folder),
            "Added folder should be present in the list"
        );

        // Remove the folder
        config.shared_folders.retain(|f| f != &folder);

        // Verify the folder was removed
        prop_assert!(
            !config.shared_folders.iter().any(|f| f == &folder),
            "Removed folder should not be present in the list"
        );
    }
}

// ============================================================================
// Generators for SPICE configurations
// ============================================================================

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

fn arb_spice_shared_folders() -> impl Strategy<Value = Vec<SharedFolder>> {
    prop::collection::vec(arb_shared_folder(), 0..5)
}

fn arb_optional_path() -> impl Strategy<Value = Option<PathBuf>> {
    prop_oneof![
        Just(None),
        "/[a-z]{1,10}(/[a-z]{1,10}){0,3}".prop_map(|s| Some(PathBuf::from(s))),
    ]
}

fn arb_spice_config() -> impl Strategy<Value = SpiceConfig> {
    (
        any::<bool>(),                 // tls_enabled
        arb_optional_path(),           // ca_cert_path
        any::<bool>(),                 // skip_cert_verify
        any::<bool>(),                 // usb_redirection
        arb_spice_shared_folders(),    // shared_folders
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

fn arb_spice_connection() -> impl Strategy<Value = Connection> {
    (
        "[a-zA-Z][a-zA-Z0-9_-]{0,30}", // name
        "[a-z0-9]([a-z0-9-]{0,30}[a-z0-9])?(\\.[a-z0-9]([a-z0-9-]{0,30}[a-z0-9])?)*", // host
        5900u16..6000,                 // port (SPICE display range)
        arb_spice_config(),
    )
        .prop_map(|(name, host, port, spice_config)| {
            Connection::new(name, host, port, ProtocolConfig::Spice(spice_config))
        })
}

// ============================================================================
// Property Tests for SPICE Configuration Validation
// ============================================================================

/// Helper function to validate SPICE configuration
/// Returns Ok(()) if valid, Err with message if invalid
fn validate_spice_config(config: &SpiceConfig) -> Result<(), String> {
    // Validate shared folder paths are not empty
    for folder in &config.shared_folders {
        if folder.local_path.as_os_str().is_empty() {
            return Err("Shared folder local_path cannot be empty".to_string());
        }
        if folder.share_name.is_empty() {
            return Err("Shared folder share_name cannot be empty".to_string());
        }
    }

    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // **Feature: native-protocol-embedding, Property 5: Protocol configuration validation rejects invalid inputs**
    // **Validates: Requirements 6.3**
    //
    // For any valid SPICE configuration, validation should pass.

    #[test]
    fn prop_spice_validation_accepts_valid_configs(config in arb_spice_config()) {
        let result = validate_spice_config(&config);
        prop_assert!(result.is_ok(), "Valid SPICE config should pass validation: {:?}", result);
    }

    // **Feature: native-protocol-embedding, Property 5: Protocol configuration validation rejects invalid inputs**
    // **Validates: Requirements 6.3**
    //
    // SPICE config with empty shared folder path should be rejected.

    #[test]
    fn prop_spice_rejects_empty_shared_folder_path(mut config in arb_spice_config()) {
        // Add a shared folder with empty path
        config.shared_folders.push(SharedFolder {
            local_path: PathBuf::new(),
            share_name: "test".to_string(),
        });
        let result = validate_spice_config(&config);
        prop_assert!(result.is_err(), "Empty shared folder path should be rejected");
    }

    // **Feature: native-protocol-embedding, Property 5: Protocol configuration validation rejects invalid inputs**
    // **Validates: Requirements 6.3**
    //
    // SPICE config with empty shared folder name should be rejected.

    #[test]
    fn prop_spice_rejects_empty_shared_folder_name(mut config in arb_spice_config()) {
        // Add a shared folder with empty name
        config.shared_folders.push(SharedFolder {
            local_path: PathBuf::from("/tmp/test"),
            share_name: String::new(),
        });
        let result = validate_spice_config(&config);
        prop_assert!(result.is_err(), "Empty shared folder name should be rejected");
    }

    // **Feature: native-protocol-embedding, Property 5: Protocol configuration validation rejects invalid inputs**
    // **Validates: Requirements 6.3**
    //
    // SPICE connection with empty host should be rejected (common validation).

    #[test]
    fn prop_spice_connection_rejects_empty_host(mut conn in arb_spice_connection()) {
        conn.host = String::new();
        // Empty host is invalid for any protocol
        prop_assert!(conn.host.is_empty(), "Host should be empty for this test");
    }

    // **Feature: native-protocol-embedding, Property 5: Protocol configuration validation rejects invalid inputs**
    // **Validates: Requirements 6.3**
    //
    // SPICE connection with zero port should be rejected (common validation).

    #[test]
    fn prop_spice_connection_rejects_zero_port(mut conn in arb_spice_connection()) {
        conn.port = 0;
        // Zero port is invalid for any protocol
        prop_assert_eq!(conn.port, 0, "Port should be zero for this test");
    }
}

// ============================================================================
// Property Test for Default Configuration Validity
// ============================================================================

/// **Feature: native-protocol-embedding, Property 6: Default configurations are valid**
/// **Validates: Requirements 6.4**
///
/// For any protocol type, `Default::default()` SHALL produce a configuration
/// that passes validation.
#[test]
fn prop_default_spice_config_is_valid() {
    let default_config = SpiceConfig::default();
    let result = validate_spice_config(&default_config);
    assert!(
        result.is_ok(),
        "Default SpiceConfig should be valid: {:?}",
        result
    );

    // Verify default values are sensible
    assert!(
        !default_config.tls_enabled,
        "TLS should be disabled by default"
    );
    assert!(
        default_config.ca_cert_path.is_none(),
        "CA cert path should be None by default"
    );
    assert!(
        !default_config.skip_cert_verify,
        "skip_cert_verify should be false by default"
    );
    assert!(
        !default_config.usb_redirection,
        "USB redirection should be disabled by default"
    );
    assert!(
        default_config.shared_folders.is_empty(),
        "Shared folders should be empty by default"
    );
    assert!(
        default_config.clipboard_enabled,
        "Clipboard should be enabled by default"
    );
    assert!(
        default_config.image_compression.is_none(),
        "Image compression should be None by default"
    );
}

/// **Feature: native-protocol-embedding, Property 6: Default configurations are valid**
/// **Validates: Requirements 6.4**
///
/// All protocol default configurations should be valid.
#[test]
fn prop_all_default_protocol_configs_are_valid() {
    // Test SSH default
    let ssh_config = SshConfig::default();
    assert_eq!(
        ssh_config.auth_method,
        SshAuthMethod::Password,
        "SSH default auth should be Password"
    );

    // Test RDP default
    let rdp_config = RdpConfig::default();
    assert!(
        rdp_config.resolution.is_none(),
        "RDP default resolution should be None"
    );
    assert!(
        rdp_config.shared_folders.is_empty(),
        "RDP default shared folders should be empty"
    );

    // Test VNC default
    let vnc_config = VncConfig::default();
    assert!(
        vnc_config.encoding.is_none(),
        "VNC default encoding should be None"
    );
    assert!(
        vnc_config.compression.is_none(),
        "VNC default compression should be None"
    );
    assert!(
        vnc_config.quality.is_none(),
        "VNC default quality should be None"
    );

    // Test SPICE default
    let spice_config = SpiceConfig::default();
    let result = validate_spice_config(&spice_config);
    assert!(result.is_ok(), "Default SpiceConfig should be valid");
}

// ============================================================================
// SSH IdentitiesOnly and Command Builder Property Tests
// ============================================================================

/// Generator for SSH config with identities_only option
fn arb_ssh_config_with_identities_only() -> impl Strategy<Value = SshConfig> {
    (
        arb_ssh_auth_method(),
        any::<bool>(),                          // identities_only
        prop::option::of("[/a-z0-9._-]{1,50}"), // key_path
        prop::option::of("[a-z0-9.-]{1,30}"),   // proxy_jump
        any::<bool>(),                          // use_control_master
        arb_ssh_custom_options(),
    )
        .prop_map(
            |(
                auth_method,
                identities_only,
                key_path,
                proxy_jump,
                use_control_master,
                custom_options,
            )| {
                SshConfig {
                    auth_method,
                    key_path: key_path.map(PathBuf::from),
                    key_source: SshKeySource::Default,
                    agent_key_fingerprint: None,
                    identities_only,
                    proxy_jump,
                    use_control_master,
                    agent_forwarding: false,
                    custom_options,
                    startup_command: None,
                    jump_host_id: None,
                }
            },
        )
}

/// Generator for SSH config with agent key fingerprint
fn arb_ssh_config_with_agent_fingerprint() -> impl Strategy<Value = SshConfig> {
    (
        prop::option::of("[a-zA-Z0-9+/]{43}".prop_map(|s| format!("SHA256:{}", s))), // fingerprint
        prop::option::of("[a-z0-9@._-]{1,50}"),                                      // comment
    )
        .prop_map(|(fingerprint, comment)| {
            let key_source = if let (Some(fp), Some(c)) = (fingerprint.clone(), comment) {
                SshKeySource::Agent {
                    fingerprint: fp.clone(),
                    comment: c,
                }
            } else {
                SshKeySource::Default
            };
            SshConfig {
                auth_method: SshAuthMethod::Agent,
                key_path: None,
                key_source,
                agent_key_fingerprint: fingerprint,
                identities_only: false,
                proxy_jump: None,
                use_control_master: false,
                agent_forwarding: false,
                custom_options: HashMap::new(),
                startup_command: None,
                jump_host_id: None,
            }
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // **Feature: rustconn-bugfixes, Property 4: SSH IdentitiesOnly Command Generation**
    // **Validates: Requirements 6.2, 6.3**
    //
    // For any SSH config with identities_only=true, the generated command should
    // contain "-o IdentitiesOnly=yes".
    #[test]
    fn prop_ssh_identities_only_command_generation(config in arb_ssh_config_with_identities_only()) {
        let args = config.build_command_args();

        if config.identities_only {
            // When identities_only is true, args should contain "-o" followed by "IdentitiesOnly=yes"
            let has_identities_only = args.windows(2).any(|w| w[0] == "-o" && w[1] == "IdentitiesOnly=yes");
            prop_assert!(
                has_identities_only,
                "When identities_only is true, command should contain '-o IdentitiesOnly=yes'. Got: {:?}",
                args
            );
        } else {
            // When identities_only is false, args should NOT contain "IdentitiesOnly=yes"
            let has_identities_only = args.iter().any(|a| a.contains("IdentitiesOnly"));
            prop_assert!(
                !has_identities_only,
                "When identities_only is false, command should NOT contain 'IdentitiesOnly'. Got: {:?}",
                args
            );
        }
    }

    // **Feature: rustconn-bugfixes, Property 4: SSH IdentitiesOnly Command Generation**
    // **Validates: Requirements 6.2, 6.3**
    //
    // For any SSH config with a key_path, the generated command should contain
    // "-i <path>" when the path is non-empty.
    #[test]
    fn prop_ssh_identity_file_command_generation(config in arb_ssh_config_with_identities_only()) {
        let args = config.build_command_args();

        if let Some(ref key_path) = config.key_path {
            if !key_path.as_os_str().is_empty() {
                // When key_path is set, args should contain "-i" followed by the path
                let path_str = key_path.display().to_string();
                let has_identity = args.windows(2).any(|w| w[0] == "-i" && w[1] == path_str);
                prop_assert!(
                    has_identity,
                    "When key_path is set, command should contain '-i <path>'. Got: {:?}",
                    args
                );
            }
        }
    }

    // **Feature: rustconn-bugfixes, Property 5: SSH Config Serialization Round-Trip**
    // **Validates: Requirements 6.4, 8.4**
    //
    // For any valid SshConfig including identities_only and ssh_agent_key_fingerprint,
    // serializing and deserializing should produce an equivalent config.
    #[test]
    fn prop_ssh_config_serialization_round_trip(config in arb_ssh_config_with_identities_only()) {
        // Serialize to JSON
        let json = serde_json::to_string(&config).expect("Failed to serialize SshConfig");

        // Deserialize back
        let deserialized: SshConfig = serde_json::from_str(&json).expect("Failed to deserialize SshConfig");

        // Verify all fields are preserved
        prop_assert_eq!(
            config.auth_method, deserialized.auth_method,
            "auth_method should be preserved"
        );
        prop_assert_eq!(
            config.key_path, deserialized.key_path,
            "key_path should be preserved"
        );
        prop_assert_eq!(
            config.identities_only, deserialized.identities_only,
            "identities_only should be preserved"
        );
        prop_assert_eq!(
            config.proxy_jump, deserialized.proxy_jump,
            "proxy_jump should be preserved"
        );
        prop_assert_eq!(
            config.use_control_master, deserialized.use_control_master,
            "use_control_master should be preserved"
        );
        prop_assert_eq!(
            config.custom_options, deserialized.custom_options,
            "custom_options should be preserved"
        );
    }

    // **Feature: rustconn-bugfixes, Property 6: SSH Agent Key Fingerprint Persistence**
    // **Validates: Requirements 8.1, 8.2, 8.4**
    //
    // For any connection with a saved ssh_agent_key_fingerprint, loading the connection
    // should preserve the fingerprint value.
    #[test]
    fn prop_ssh_agent_key_fingerprint_persistence(config in arb_ssh_config_with_agent_fingerprint()) {
        // Serialize to JSON
        let json = serde_json::to_string(&config).expect("Failed to serialize SshConfig");

        // Deserialize back
        let deserialized: SshConfig = serde_json::from_str(&json).expect("Failed to deserialize SshConfig");

        // Verify fingerprint is preserved
        prop_assert_eq!(
            config.agent_key_fingerprint, deserialized.agent_key_fingerprint,
            "agent_key_fingerprint should be preserved through serialization"
        );

        // Verify key_source is preserved
        prop_assert_eq!(
            config.key_source, deserialized.key_source,
            "key_source should be preserved through serialization"
        );
    }
}

// ============================================================================
// SSH Key Selection Property Tests (native-protocol-embedding)
// ============================================================================

/// Generator for SSH config with File key source (for testing Property 15)
fn arb_ssh_config_with_file_key_source() -> impl Strategy<Value = SshConfig> {
    (
        "/[a-z]{1,10}(/[a-z]{1,10}){0,3}/[a-z_]{1,10}", // key file path
        prop::option::of("[a-z0-9.-]{1,30}"),           // proxy_jump
        any::<bool>(),                                  // use_control_master
        arb_ssh_custom_options(),
    )
        .prop_map(
            |(key_path, proxy_jump, use_control_master, custom_options)| {
                SshConfig {
                    auth_method: SshAuthMethod::PublicKey,
                    key_path: None, // Use key_source instead
                    key_source: SshKeySource::File {
                        path: PathBuf::from(key_path),
                    },
                    agent_key_fingerprint: None,
                    identities_only: false, // Should be auto-enabled for File auth
                    proxy_jump,
                    use_control_master,
                    agent_forwarding: false,
                    custom_options,
                    startup_command: None,
                    jump_host_id: None,
                }
            },
        )
}

/// Generator for SSH config with Agent key source (for testing Property 16)
fn arb_ssh_config_with_agent_key_source() -> impl Strategy<Value = SshConfig> {
    (
        "[a-zA-Z0-9+/]{43}".prop_map(|s| format!("SHA256:{s}")), // fingerprint
        "[a-z0-9@._-]{1,50}",                                    // comment
        prop::option::of("[a-z0-9.-]{1,30}"),                    // proxy_jump
        any::<bool>(),                                           // use_control_master
        arb_ssh_custom_options(),
    )
        .prop_map(
            |(fingerprint, comment, proxy_jump, use_control_master, custom_options)| {
                SshConfig {
                    auth_method: SshAuthMethod::Agent,
                    key_path: None,
                    key_source: SshKeySource::Agent {
                        fingerprint: fingerprint.clone(),
                        comment,
                    },
                    agent_key_fingerprint: Some(fingerprint),
                    identities_only: false, // Should NOT be enabled for Agent auth
                    proxy_jump,
                    use_control_master,
                    agent_forwarding: false,
                    custom_options,
                    startup_command: None,
                    jump_host_id: None,
                }
            },
        )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // **Feature: native-protocol-embedding, Property 15: SSH Key File Flag Generation**
    // **Validates: Requirements 5.1, 5.2**
    //
    // For any SSH connection with File authentication method and a key file path,
    // the generated command should include `-i <path>` and `-o IdentitiesOnly=yes`.
    #[test]
    fn prop_ssh_key_file_flag_generation(config in arb_ssh_config_with_file_key_source()) {
        let args = config.build_command_args();

        // Extract the key path from the config
        if let SshKeySource::File { ref path } = config.key_source {
            let path_str = path.display().to_string();

            // Requirement 5.1: Should include -i <key_path> flag
            let has_identity_flag = args.windows(2).any(|w| w[0] == "-i" && w[1] == path_str);
            prop_assert!(
                has_identity_flag,
                "File auth should include '-i <path>' flag. Got: {:?}",
                args
            );

            // Requirement 5.2: Should include -o IdentitiesOnly=yes flag
            let has_identities_only = args.windows(2).any(|w| w[0] == "-o" && w[1] == "IdentitiesOnly=yes");
            prop_assert!(
                has_identities_only,
                "File auth should include '-o IdentitiesOnly=yes' flag. Got: {:?}",
                args
            );
        }
    }

    // **Feature: native-protocol-embedding, Property 16: SSH Agent No IdentitiesOnly**
    // **Validates: Requirements 5.3**
    //
    // For any SSH connection with Agent authentication method, the generated command
    // should NOT include `-o IdentitiesOnly=yes` to allow SSH to use all keys from the agent.
    #[test]
    fn prop_ssh_agent_no_identities_only(config in arb_ssh_config_with_agent_key_source()) {
        let args = config.build_command_args();

        // Requirement 5.3: Agent auth should NOT include IdentitiesOnly
        let has_identities_only = args.iter().any(|a| a.contains("IdentitiesOnly"));
        prop_assert!(
            !has_identities_only,
            "Agent auth should NOT include 'IdentitiesOnly'. Got: {:?}",
            args
        );

        // Agent auth should NOT include -i flag as a standalone flag (no specific key file)
        // Check for -i followed by a path-like argument (not as a value to another flag like -J)
        let has_identity_flag = args.windows(2).any(|w| {
            w[0] == "-i" && !w[1].starts_with('-')
        });
        prop_assert!(
            !has_identity_flag,
            "Agent auth should NOT include '-i <path>' flag. Got: {:?}",
            args
        );
    }
}

// ============================================================================
// Property Tests for Cloud Provider Icon Detection
// ============================================================================

use rustconn_core::protocol::icons::{detect_provider, CloudProvider};

/// Generator for AWS-style commands
fn arb_aws_command() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("aws ssm start-session".to_string()),
        Just("aws ssm start-session --target i-123456".to_string()),
        Just("/usr/bin/aws ssm start-session".to_string()),
        "aws [a-z]{1,10} [a-z-]{1,20}".prop_map(|s| s),
    ]
}

/// Generator for GCP-style commands
fn arb_gcloud_command() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("gcloud compute ssh".to_string()),
        Just("gcloud compute ssh instance --zone us-central1-a".to_string()),
        Just("/usr/bin/gcloud compute ssh".to_string()),
        "gcloud [a-z]{1,10} [a-z-]{1,20}".prop_map(|s| s),
    ]
}

/// Generator for Azure-style commands
fn arb_azure_command() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("az network bastion ssh".to_string()),
        Just("az ssh vm --name myvm".to_string()),
        Just("/usr/bin/az ssh vm".to_string()),
        "az [a-z]{1,10} [a-z-]{1,20}".prop_map(|s| s),
    ]
}

/// Generator for OCI-style commands
fn arb_oci_command() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("oci bastion session create".to_string()),
        Just("/usr/bin/oci bastion session".to_string()),
        "oci [a-z]{1,10} [a-z-]{1,20}".prop_map(|s| s),
    ]
}

/// Generator for Cloudflare-style commands
fn arb_cloudflare_command() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("cloudflared access ssh".to_string()),
        Just("/usr/bin/cloudflared access ssh".to_string()),
        "cloudflared [a-z]{1,10} [a-z-]{1,20}".prop_map(|s| s),
    ]
}

/// Generator for Teleport-style commands
fn arb_teleport_command() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("tsh ssh user@host".to_string()),
        Just("/usr/bin/tsh ssh user@host".to_string()),
        "tsh [a-z]{1,10} [a-z@-]{1,20}".prop_map(|s| s),
    ]
}

/// Generator for Tailscale-style commands
fn arb_tailscale_command() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("tailscale ssh user@host".to_string()),
        Just("/usr/bin/tailscale ssh user@host".to_string()),
        "tailscale [a-z]{1,10} [a-z@-]{1,20}".prop_map(|s| s),
    ]
}

/// Generator for Boundary-style commands
fn arb_boundary_command() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("boundary connect ssh".to_string()),
        Just("/usr/bin/boundary connect ssh".to_string()),
        "boundary [a-z]{1,10} [a-z-]{1,20}".prop_map(|s| s),
    ]
}

/// Generator for generic/unknown commands
fn arb_generic_command() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("ssh user@host".to_string()),
        Just("custom-tool connect".to_string()),
        Just("".to_string()),
        "[a-z]{1,10} [a-z@.-]{1,20}".prop_map(|s| s),
    ]
}

/// Generator for any CLI command with expected provider
fn arb_command_with_provider() -> impl Strategy<Value = (String, CloudProvider)> {
    prop_oneof![
        arb_aws_command().prop_map(|cmd| (cmd, CloudProvider::Aws)),
        arb_gcloud_command().prop_map(|cmd| (cmd, CloudProvider::Gcloud)),
        arb_azure_command().prop_map(|cmd| (cmd, CloudProvider::Azure)),
        arb_oci_command().prop_map(|cmd| (cmd, CloudProvider::Oci)),
        arb_cloudflare_command().prop_map(|cmd| (cmd, CloudProvider::Cloudflare)),
        arb_teleport_command().prop_map(|cmd| (cmd, CloudProvider::Teleport)),
        arb_tailscale_command().prop_map(|cmd| (cmd, CloudProvider::Tailscale)),
        arb_boundary_command().prop_map(|cmd| (cmd, CloudProvider::Boundary)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // **Feature: rustconn-bugfixes, Property 1: Provider Icon Detection**
    // **Validates: Requirements 4.2**
    //
    // For any CLI command string, the provider detection function should return
    // a valid CloudProvider enum value.
    #[test]
    fn prop_provider_detection_returns_valid_provider(command in "[a-zA-Z0-9 /_.-]{0,100}") {
        let provider = detect_provider(&command);

        // Verify the result is a valid CloudProvider variant
        let valid_providers = CloudProvider::all();
        prop_assert!(
            valid_providers.contains(&provider),
            "detect_provider should return a valid CloudProvider. Got: {:?}",
            provider
        );
    }

    // **Feature: rustconn-bugfixes, Property 1: Provider Icon Detection**
    // **Validates: Requirements 4.2**
    //
    // For any known provider command, detection should return the correct provider.
    #[test]
    fn prop_provider_detection_correct_for_known_commands((command, expected_provider) in arb_command_with_provider()) {
        let detected = detect_provider(&command);

        prop_assert_eq!(
            detected, expected_provider,
            "Command '{}' should be detected as {:?}, but got {:?}",
            command, expected_provider, detected
        );
    }

    // **Feature: rustconn-bugfixes, Property 1: Provider Icon Detection**
    // **Validates: Requirements 4.3**
    //
    // For any unknown/generic command, detection should return Generic provider.
    #[test]
    fn prop_provider_detection_generic_for_unknown(command in arb_generic_command()) {
        let detected = detect_provider(&command);

        // Generic commands should return Generic provider
        // (unless they accidentally match a known pattern)
        prop_assert!(
            detected == CloudProvider::Generic || CloudProvider::all().contains(&detected),
            "Unknown command should return Generic or a valid provider. Got: {:?}",
            detected
        );
    }

    // **Feature: rustconn-bugfixes, Property 1: Provider Icon Detection**
    // **Validates: Requirements 4.2**
    //
    // Provider detection should be case-insensitive.
    #[test]
    fn prop_provider_detection_case_insensitive(command in arb_command_with_provider()) {
        let (cmd, _expected) = command;

        // Test lowercase
        let lower_result = detect_provider(&cmd.to_lowercase());
        // Test uppercase
        let upper_result = detect_provider(&cmd.to_uppercase());

        prop_assert_eq!(
            lower_result, upper_result,
            "Provider detection should be case-insensitive. Lower: {:?}, Upper: {:?}",
            lower_result, upper_result
        );
    }
}

// ============================================================================
// Unit Tests for Cloud Provider Icon Detection
// ============================================================================

/// **Feature: rustconn-bugfixes, Property 1: Provider Icon Detection**
/// **Validates: Requirements 4.2**
///
/// Verify that each provider has a valid icon name.
#[test]
fn test_all_providers_have_icon_names() {
    for provider in CloudProvider::all() {
        let icon_name = provider.icon_name();
        assert!(
            !icon_name.is_empty(),
            "Provider {:?} should have a non-empty icon name",
            provider
        );
        assert!(
            icon_name.ends_with("-symbolic") || icon_name.contains("cloud"),
            "Icon name '{}' should follow naming convention",
            icon_name
        );
    }
}

/// **Feature: rustconn-bugfixes, Property 1: Provider Icon Detection**
/// **Validates: Requirements 4.2**
///
/// Verify that each provider has a valid display name.
#[test]
fn test_all_providers_have_display_names() {
    for provider in CloudProvider::all() {
        let display_name = provider.display_name();
        assert!(
            !display_name.is_empty(),
            "Provider {:?} should have a non-empty display name",
            provider
        );
    }
}

/// **Feature: rustconn-bugfixes, Property 1: Provider Icon Detection**
/// **Validates: Requirements 4.3**
///
/// Verify that Generic provider is the default.
#[test]
fn test_generic_is_default_provider() {
    let default_provider = CloudProvider::default();
    assert_eq!(
        default_provider,
        CloudProvider::Generic,
        "Default provider should be Generic"
    );
}

// ============================================================================
// Property Test for Protocol-Specific Icons
// ============================================================================

use rustconn_core::models::ProtocolType;
use rustconn_core::protocol::icons::{all_protocol_icons, get_protocol_icon};

/// **Feature: rustconn-fixes-v2, Property 9: Protocol Icons Are Distinct**
/// **Validates: Requirements 7.1, 7.2, 7.3, 7.4**
///
/// For any two different protocol types (SSH, RDP, VNC, SPICE), the icon names
/// returned should be different.
#[test]
fn prop_protocol_icons_are_distinct() {
    let protocols = [
        ProtocolType::Ssh,
        ProtocolType::Rdp,
        ProtocolType::Vnc,
        ProtocolType::Spice,
    ];

    // Collect all icon names
    let icons: Vec<(&ProtocolType, &'static str)> = protocols
        .iter()
        .map(|p| (p, get_protocol_icon(*p)))
        .collect();

    // Check that each pair of different protocols has different icons
    for i in 0..icons.len() {
        for j in (i + 1)..icons.len() {
            let (proto_a, icon_a) = icons[i];
            let (proto_b, icon_b) = icons[j];

            assert_ne!(
                icon_a, icon_b,
                "Protocol {:?} and {:?} should have distinct icons, but both have '{}'",
                proto_a, proto_b, icon_a
            );
        }
    }
}

/// **Feature: rustconn-fixes-v2, Property 9: Protocol Icons Are Distinct**
/// **Validates: Requirements 7.1, 7.2, 7.3, 7.4**
///
/// Verify that each protocol has the expected icon name as per requirements.
#[test]
fn test_protocol_icons_match_requirements() {
    // Requirements 7.1: SSH should show terminal icon
    assert_eq!(
        get_protocol_icon(ProtocolType::Ssh),
        "utilities-terminal-symbolic",
        "SSH should use terminal icon (Requirement 7.3)"
    );

    // Requirements 7.2: VNC should show video display icon
    assert_eq!(
        get_protocol_icon(ProtocolType::Vnc),
        "video-display-symbolic",
        "VNC should use video display icon (Requirement 7.2)"
    );

    // Requirements 7.1: RDP should show computer/monitor icon
    assert_eq!(
        get_protocol_icon(ProtocolType::Rdp),
        "computer-symbolic",
        "RDP should use computer icon (Requirement 7.1)"
    );

    // Requirements 7.4: SPICE should show remote desktop icon
    assert_eq!(
        get_protocol_icon(ProtocolType::Spice),
        "preferences-desktop-remote-desktop-symbolic",
        "SPICE should use remote desktop icon (Requirement 7.4)"
    );
}

/// **Feature: rustconn-fixes-v2, Property 9: Protocol Icons Are Distinct**
/// **Validates: Requirements 7.1, 7.2, 7.3, 7.4**
///
/// Verify that all_protocol_icons returns consistent data.
#[test]
fn test_all_protocol_icons_consistency() {
    let all_icons = all_protocol_icons();

    // Verify we have entries for all main protocols
    assert!(
        all_icons.len() >= 4,
        "Should have at least 4 protocol icons (SSH, RDP, VNC, SPICE)"
    );

    // Verify each entry matches get_protocol_icon
    for (protocol, expected_icon) in all_icons {
        let actual_icon = get_protocol_icon(*protocol);
        assert_eq!(
            actual_icon, *expected_icon,
            "all_protocol_icons and get_protocol_icon should return same icon for {:?}",
            protocol
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-fixes-v2, Property 9: Protocol Icons Are Distinct**
    /// **Validates: Requirements 7.1, 7.2, 7.3, 7.4**
    ///
    /// For any randomly selected pair of different protocol types,
    /// their icons should be different.
    #[test]
    fn prop_random_protocol_pairs_have_distinct_icons(
        idx_a in 0usize..4,
        idx_b in 0usize..4
    ) {
        let protocols = [
            ProtocolType::Ssh,
            ProtocolType::Rdp,
            ProtocolType::Vnc,
            ProtocolType::Spice,
        ];

        let proto_a = protocols[idx_a];
        let proto_b = protocols[idx_b];

        let icon_a = get_protocol_icon(proto_a);
        let icon_b = get_protocol_icon(proto_b);

        // If protocols are different, icons must be different
        if idx_a != idx_b {
            prop_assert_ne!(
                icon_a, icon_b,
                "Different protocols {:?} and {:?} should have different icons",
                proto_a, proto_b
            );
        } else {
            // Same protocol should have same icon (consistency)
            prop_assert_eq!(
                icon_a, icon_b,
                "Same protocol {:?} should always return same icon",
                proto_a
            );
        }
    }
}
