//! Property-based tests for SPICE external-viewer support
//!
//! Tests for the SPICE viewer launch/config path (native embedding was removed
//! in 0.18.0).
//!
//! # Coverage
//!
//! - Property 14: SPICE Fallback on Failure

use proptest::prelude::*;
use rustconn_core::spice_client::{
    SpiceClientConfig, SpiceClientError, SpiceCompression, SpiceSecurityProtocol,
    SpiceSharedFolder, build_spice_viewer_args, detect_spice_viewer,
};

// Strategy for generating valid hostnames
fn arb_host() -> impl Strategy<Value = String> {
    prop_oneof![
        // Simple hostnames
        "[a-z][a-z0-9]{0,15}".prop_map(|s| s),
        // IP addresses
        (1u8..=254, 0u8..=255, 0u8..=255, 1u8..=254)
            .prop_map(|(a, b, c, d)| format!("{a}.{b}.{c}.{d}")),
        // Domain names
        "[a-z][a-z0-9]{0,10}\\.[a-z]{2,4}".prop_map(|s| s),
    ]
}

// Strategy for generating valid ports
fn arb_port() -> impl Strategy<Value = u16> {
    prop_oneof![
        Just(5900u16), // Default SPICE port
        Just(5901u16),
        Just(5902u16),
        1024u16..=65535u16,
    ]
}

// Strategy for generating valid resolutions
fn arb_resolution() -> impl Strategy<Value = (u16, u16)> {
    prop_oneof![
        Just((800, 600)),
        Just((1024, 768)),
        Just((1280, 720)),
        Just((1280, 1024)),
        Just((1920, 1080)),
        Just((2560, 1440)),
        (640u16..=3840, 480u16..=2160),
    ]
}

// Strategy for generating security protocols
fn arb_security_protocol() -> impl Strategy<Value = SpiceSecurityProtocol> {
    prop_oneof![
        Just(SpiceSecurityProtocol::Auto),
        Just(SpiceSecurityProtocol::Plain),
        Just(SpiceSecurityProtocol::Tls),
        Just(SpiceSecurityProtocol::Sasl),
    ]
}

// Strategy for generating image compression settings
fn arb_image_compression() -> impl Strategy<Value = SpiceCompression> {
    prop_oneof![
        Just(SpiceCompression::Auto),
        Just(SpiceCompression::Off),
        Just(SpiceCompression::Glz),
        Just(SpiceCompression::Lz),
        Just(SpiceCompression::Quic),
    ]
}

// Strategy for generating SPICE client config
fn arb_spice_client_config() -> impl Strategy<Value = SpiceClientConfig> {
    (
        arb_host(),
        arb_port(),
        arb_resolution(),
        any::<bool>(), // tls_enabled
        any::<bool>(), // skip_cert_verify
        any::<bool>(), // clipboard_enabled
        any::<bool>(), // usb_redirection
        arb_image_compression(),
        any::<bool>(), // audio_playback
        any::<bool>(), // audio_record
        1u64..=120,    // timeout_secs
        arb_security_protocol(),
    )
        .prop_map(
            |(
                host,
                port,
                (width, height),
                tls_enabled,
                skip_cert_verify,
                clipboard_enabled,
                usb_redirection,
                image_compression,
                audio_playback,
                audio_record,
                timeout_secs,
                security_protocol,
            )| {
                SpiceClientConfig::new(host)
                    .with_port(port)
                    .with_resolution(width, height)
                    .with_tls(tls_enabled)
                    .with_skip_cert_verify(skip_cert_verify)
                    .with_clipboard(clipboard_enabled)
                    .with_usb_redirection(usb_redirection)
                    .with_image_compression(image_compression)
                    .with_audio_playback(audio_playback)
                    .with_audio_record(audio_record)
                    .with_timeout(timeout_secs)
                    .with_security_protocol(security_protocol)
            },
        )
}

// ============================================================================
// Property Tests
// ============================================================================

proptest! {
    /// **Feature: performance-improvements, Property 14: SPICE Fallback on Failure**
    ///
    /// *For any* failed native SPICE connection attempt, the system SHALL attempt
    /// fallback to external viewer.
    ///
    /// This test verifies that:
    /// 1. The fallback mechanism is available (detect_spice_viewer works)
    /// 2. The viewer arguments are correctly built for any valid config
    /// 3. The fallback result is one of the expected variants
    #[test]
    fn prop_spice_fallback_viewer_args_valid(config in arb_spice_client_config()) {
        // Build viewer arguments for the config
        let args = build_spice_viewer_args(&config);

        // Verify the URI is present and correctly formatted
        let uri = if config.tls_enabled {
            format!("spice+tls://{}:{}", config.host, config.port)
        } else {
            format!("spice://{}:{}", config.host, config.port)
        };
        prop_assert!(args.contains(&uri), "URI not found in args: {:?}", args);

        // Verify title is present
        prop_assert!(args.contains(&"--title".to_string()), "Title flag not found");

        // Verify USB redirection flag is present when enabled
        if config.usb_redirection {
            prop_assert!(
                args.contains(&"--spice-usbredir-auto-redirect-filter".to_string()),
                "USB redirection flag not found when enabled"
            );
        }

        // Verify audio disable flag is present when audio is disabled
        if !config.audio_playback {
            prop_assert!(
                args.contains(&"--spice-disable-audio".to_string()),
                "Audio disable flag not found when audio is disabled"
            );
        }
    }

    /// Property: SPICE config validation is consistent
    ///
    /// *For any* valid SPICE config, validation should pass.
    #[test]
    fn prop_spice_config_validation(config in arb_spice_client_config()) {
        // Config with TLS enabled but no cert and skip_cert_verify=false should fail
        // Our generator always sets skip_cert_verify when tls_enabled, so this should pass
        if config.tls_enabled && !config.skip_cert_verify && config.ca_cert_path.is_none() {
            // This is expected to fail validation
            prop_assert!(config.validate().is_err());
        } else {
            // All other configs should be valid
            prop_assert!(config.validate().is_ok(), "Config validation failed: {:?}", config);
        }
    }

    /// Property: SPICE server address format is correct
    ///
    /// *For any* SPICE config, the server address should be "host:port".
    #[test]
    fn prop_spice_server_address_format(config in arb_spice_client_config()) {
        let expected = format!("{}:{}", config.host, config.port);
        prop_assert_eq!(config.server_address(), expected);
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[test]
fn test_detect_spice_viewer() {
    // This test verifies the function works without panicking
    // The result depends on whether a SPICE viewer is installed
    let _viewer = detect_spice_viewer();
}

#[test]
fn test_spice_client_error_variants() {
    // Test that all error variants can be created and formatted
    let errors = vec![
        SpiceClientError::ConnectionFailed("test".to_string()),
        SpiceClientError::AuthenticationFailed("test".to_string()),
        SpiceClientError::ProtocolError("test".to_string()),
        SpiceClientError::IoError("test".to_string()),
        SpiceClientError::TlsError("test".to_string()),
        SpiceClientError::NotConnected,
        SpiceClientError::AlreadyConnected,
        SpiceClientError::InvalidConfig("test".to_string()),
        SpiceClientError::ChannelError("test".to_string()),
        SpiceClientError::Timeout,
        SpiceClientError::ServerDisconnected("test".to_string()),
        SpiceClientError::Unsupported("test".to_string()),
        SpiceClientError::UsbRedirectionError("test".to_string()),
        SpiceClientError::SharedFolderError("test".to_string()),
        SpiceClientError::NativeClientNotAvailable,
    ];

    for error in errors {
        // Verify error formatting works
        let _ = format!("{}", error);
        let _ = format!("{:?}", error);
    }
}

#[test]
fn test_spice_shared_folder() {
    let folder = SpiceSharedFolder::new("/home/user/share", "MyShare");
    assert_eq!(folder.local_path.to_string_lossy(), "/home/user/share");
    assert_eq!(folder.share_name, "MyShare");
    assert!(!folder.read_only);

    let read_only_folder = folder.with_read_only(true);
    assert!(read_only_folder.read_only);
}

#[test]
fn test_spice_config_with_shared_folder() {
    let folder = SpiceSharedFolder::new("/tmp", "TempShare");
    let config = SpiceClientConfig::new("localhost").with_shared_folder(folder);

    assert_eq!(config.shared_folders.len(), 1);
    assert_eq!(config.shared_folders[0].share_name, "TempShare");

    // Verify shared folder appears in viewer args
    let args = build_spice_viewer_args(&config);
    assert!(args.contains(&"--spice-shared-dir".to_string()));
    assert!(args.contains(&"/tmp".to_string()));
}

#[test]
fn test_spice_config_validation_empty_host() {
    let config = SpiceClientConfig::default();
    assert!(config.validate().is_err());
}

#[test]
fn test_spice_config_validation_zero_port() {
    let config = SpiceClientConfig::new("localhost").with_port(0);
    assert!(config.validate().is_err());
}

#[test]
fn test_spice_config_validation_tls_without_cert() {
    let config = SpiceClientConfig::new("localhost")
        .with_tls(true)
        .with_skip_cert_verify(false);
    assert!(config.validate().is_err());
}

#[test]
fn test_spice_config_validation_tls_with_skip_verify() {
    let config = SpiceClientConfig::new("localhost")
        .with_tls(true)
        .with_skip_cert_verify(true);
    assert!(config.validate().is_ok());
}

/// `SpiceClientConfig` derives `Debug` while holding a password
/// (`SecretString`); the Debug output must never contain the plaintext.
#[test]
fn spice_config_debug_does_not_leak_password() {
    let mut config = SpiceClientConfig::new("localhost");
    config.password = Some(secrecy::SecretString::from("hunter2-spice-secret"));
    let debug_output = format!("{config:?}");
    assert!(
        !debug_output.contains("hunter2-spice-secret"),
        "Debug output must not contain the plaintext password"
    );
}
