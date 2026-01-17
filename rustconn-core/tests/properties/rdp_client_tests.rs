//! Property-based tests for RDP client
//!
//! Tests for the native RDP client implementation using IronRDP.

use proptest::prelude::*;
use rustconn_core::{RdpClientConfig, RdpRect, RdpSecurityProtocol};

// Strategy for generating valid hostnames
fn arb_host() -> impl Strategy<Value = String> {
    prop_oneof![
        // Simple hostnames
        "[a-z][a-z0-9]{0,15}".prop_map(|s| s),
        // IP addresses
        (1u8..=254, 0u8..=255, 0u8..=255, 1u8..=254)
            .prop_map(|(a, b, c, d)| { format!("{a}.{b}.{c}.{d}") }),
        // Domain names
        "[a-z][a-z0-9]{0,10}\\.[a-z]{2,4}".prop_map(|s| s),
    ]
}

// Strategy for generating valid ports
fn arb_port() -> impl Strategy<Value = u16> {
    prop_oneof![
        Just(3389u16), // Default RDP port
        1024u16..=65535u16,
    ]
}

// Strategy for generating optional usernames
fn arb_username() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), "[a-zA-Z][a-zA-Z0-9_]{0,15}".prop_map(Some),]
}

// Strategy for generating optional domains
fn arb_domain() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), "[A-Z][A-Z0-9]{0,10}".prop_map(Some),]
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
        Just((3840, 2160)),
        (640u16..=7680, 480u16..=4320),
    ]
}

// Strategy for generating valid color depths
fn arb_color_depth() -> impl Strategy<Value = u8> {
    prop_oneof![Just(16u8), Just(24u8), Just(32u8),]
}

// Strategy for generating security protocols
fn arb_security_protocol() -> impl Strategy<Value = RdpSecurityProtocol> {
    prop_oneof![
        Just(RdpSecurityProtocol::Auto),
        Just(RdpSecurityProtocol::Rdp),
        Just(RdpSecurityProtocol::Tls),
        Just(RdpSecurityProtocol::Nla),
        Just(RdpSecurityProtocol::Ext),
    ]
}

// Strategy for generating RDP client config
fn arb_rdp_client_config() -> impl Strategy<Value = RdpClientConfig> {
    (
        (
            arb_host(),
            arb_port(),
            arb_username(),
            arb_domain(),
            arb_resolution(),
            arb_color_depth(),
        ),
        (
            any::<bool>(), // clipboard_enabled
            any::<bool>(), // audio_enabled
            1u64..=300,    // timeout_secs
            any::<bool>(), // ignore_certificate
            any::<bool>(), // nla_enabled
            arb_security_protocol(),
        ),
        (
            any::<bool>(),   // dynamic_resolution
            100u32..=300u32, // scale_factor
        ),
    )
        .prop_map(
            |(
                (host, port, username, domain, (width, height), color_depth),
                (
                    clipboard_enabled,
                    audio_enabled,
                    timeout_secs,
                    ignore_certificate,
                    nla_enabled,
                    security_protocol,
                ),
                (dynamic_resolution, scale_factor),
            )| {
                RdpClientConfig {
                    host,
                    port,
                    username,
                    password: None, // Password is skipped in serialization
                    domain,
                    width,
                    height,
                    color_depth,
                    clipboard_enabled,
                    audio_enabled,
                    timeout_secs,
                    ignore_certificate,
                    nla_enabled,
                    security_protocol,
                    shared_folders: Vec::new(),
                    dynamic_resolution,
                    scale_factor,
                    performance_mode: rustconn_core::models::RdpPerformanceMode::default(),
                }
            },
        )
}

// Strategy for generating valid RDP rectangles
fn arb_rdp_rect() -> impl Strategy<Value = RdpRect> {
    (0u16..=7680, 0u16..=4320, 1u16..=7680, 1u16..=4320)
        .prop_map(|(x, y, width, height)| RdpRect::new(x, y, width, height))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: native-protocol-embedding, Property 21: RDP Config Round-Trip Serialization**
    /// **Validates: Requirements 10.4**
    ///
    /// For any valid RdpClientConfig, serializing to JSON and deserializing back
    /// should produce an equivalent configuration with all fields preserved
    /// (except password which is skipped in serialization).
    #[test]
    fn rdp_config_json_round_trip(config in arb_rdp_client_config()) {
        // Serialize to JSON
        let json_str = serde_json::to_string(&config)
            .expect("RdpClientConfig should serialize to JSON");

        // Deserialize back from JSON
        let deserialized: RdpClientConfig = serde_json::from_str(&json_str)
            .expect("JSON should deserialize back to RdpClientConfig");

        // Verify all serialized fields are preserved
        prop_assert_eq!(config.host, deserialized.host, "Host should be preserved");
        prop_assert_eq!(config.port, deserialized.port, "Port should be preserved");
        prop_assert_eq!(config.username, deserialized.username, "Username should be preserved");
        prop_assert_eq!(config.domain, deserialized.domain, "Domain should be preserved");
        prop_assert_eq!(config.width, deserialized.width, "Width should be preserved");
        prop_assert_eq!(config.height, deserialized.height, "Height should be preserved");
        prop_assert_eq!(config.color_depth, deserialized.color_depth, "Color depth should be preserved");
        prop_assert_eq!(config.clipboard_enabled, deserialized.clipboard_enabled, "Clipboard enabled should be preserved");
        prop_assert_eq!(config.audio_enabled, deserialized.audio_enabled, "Audio enabled should be preserved");
        prop_assert_eq!(config.timeout_secs, deserialized.timeout_secs, "Timeout should be preserved");
        prop_assert_eq!(config.ignore_certificate, deserialized.ignore_certificate, "Ignore certificate should be preserved");
        prop_assert_eq!(config.nla_enabled, deserialized.nla_enabled, "NLA enabled should be preserved");
        prop_assert_eq!(config.security_protocol, deserialized.security_protocol, "Security protocol should be preserved");

        // Password is intentionally skipped in serialization for security
        prop_assert!(deserialized.password.is_none(), "Password should not be serialized");
    }

    /// **Feature: native-protocol-embedding, Property 21: RDP Config Round-Trip Serialization**
    /// **Validates: Requirements 10.4**
    ///
    /// For any valid RdpClientConfig, serializing to TOML and deserializing back
    /// should produce an equivalent configuration.
    #[test]
    fn rdp_config_toml_round_trip(config in arb_rdp_client_config()) {
        // Serialize to TOML
        let toml_str = toml::to_string(&config)
            .expect("RdpClientConfig should serialize to TOML");

        // Deserialize back from TOML
        let deserialized: RdpClientConfig = toml::from_str(&toml_str)
            .expect("TOML should deserialize back to RdpClientConfig");

        // Verify all serialized fields are preserved
        prop_assert_eq!(config.host, deserialized.host, "Host should be preserved");
        prop_assert_eq!(config.port, deserialized.port, "Port should be preserved");
        prop_assert_eq!(config.username, deserialized.username, "Username should be preserved");
        prop_assert_eq!(config.domain, deserialized.domain, "Domain should be preserved");
        prop_assert_eq!(config.width, deserialized.width, "Width should be preserved");
        prop_assert_eq!(config.height, deserialized.height, "Height should be preserved");
        prop_assert_eq!(config.color_depth, deserialized.color_depth, "Color depth should be preserved");
        prop_assert_eq!(config.clipboard_enabled, deserialized.clipboard_enabled, "Clipboard enabled should be preserved");
        prop_assert_eq!(config.audio_enabled, deserialized.audio_enabled, "Audio enabled should be preserved");
        prop_assert_eq!(config.timeout_secs, deserialized.timeout_secs, "Timeout should be preserved");
        prop_assert_eq!(config.ignore_certificate, deserialized.ignore_certificate, "Ignore certificate should be preserved");
        prop_assert_eq!(config.nla_enabled, deserialized.nla_enabled, "NLA enabled should be preserved");
        prop_assert_eq!(config.security_protocol, deserialized.security_protocol, "Security protocol should be preserved");
    }

    /// Test that server_address() produces valid address strings
    #[test]
    fn rdp_config_server_address_format(config in arb_rdp_client_config()) {
        let addr = config.server_address();

        // Should contain host and port separated by colon
        prop_assert!(addr.contains(':'), "Address should contain colon separator");

        let parts: Vec<&str> = addr.rsplitn(2, ':').collect();
        prop_assert_eq!(parts.len(), 2, "Address should have host and port parts");

        // Port part should be parseable as u16
        let port_str = parts[0];
        let parsed_port: u16 = port_str.parse().expect("Port should be valid u16");
        prop_assert_eq!(parsed_port, config.port, "Port should match config");
    }

    /// Test RdpRect properties
    #[test]
    fn rdp_rect_properties(rect in arb_rdp_rect()) {
        // Rectangle should have valid dimensions
        prop_assert!(rect.width > 0, "Width should be positive");
        prop_assert!(rect.height > 0, "Height should be positive");

        // full_screen should create rect at origin
        let full = RdpRect::full_screen(rect.width, rect.height);
        prop_assert_eq!(full.x, 0, "Full screen x should be 0");
        prop_assert_eq!(full.y, 0, "Full screen y should be 0");
        prop_assert_eq!(full.width, rect.width, "Full screen width should match");
        prop_assert_eq!(full.height, rect.height, "Full screen height should match");
    }

    /// Test RdpClientConfig builder pattern
    #[test]
    fn rdp_config_builder_pattern(
        host in arb_host(),
        port in arb_port(),
        username in "[a-zA-Z][a-zA-Z0-9_]{0,15}",
        domain in "[A-Z][A-Z0-9]{0,10}",
        (width, height) in arb_resolution(),
        color_depth in arb_color_depth(),
    ) {
        let config = RdpClientConfig::new(&host)
            .with_port(port)
            .with_username(&username)
            .with_domain(&domain)
            .with_resolution(width, height)
            .with_color_depth(color_depth)
            .with_clipboard(true)
            .with_nla(false);

        prop_assert_eq!(config.host, host, "Host should match");
        prop_assert_eq!(config.port, port, "Port should match");
        prop_assert_eq!(config.username, Some(username), "Username should match");
        prop_assert_eq!(config.domain, Some(domain), "Domain should match");
        prop_assert_eq!(config.width, width, "Width should match");
        prop_assert_eq!(config.height, height, "Height should match");
        prop_assert_eq!(config.color_depth, color_depth, "Color depth should match");
        prop_assert!(config.clipboard_enabled, "Clipboard should be enabled");
        prop_assert!(!config.nla_enabled, "NLA should be disabled");
    }
}

// Import additional types for framebuffer tests
use rustconn_core::{
    convert_to_bgra, create_frame_update, create_frame_update_with_conversion, PixelFormat,
    RdpClientEvent,
};

// Strategy for generating pixel formats
fn arb_pixel_format() -> impl Strategy<Value = PixelFormat> {
    prop_oneof![
        Just(PixelFormat::Bgra),
        Just(PixelFormat::Rgba),
        Just(PixelFormat::Rgb),
        Just(PixelFormat::Bgr),
        Just(PixelFormat::Rgb565),
    ]
}

// Strategy for generating small image dimensions (to keep test data manageable)
fn arb_small_dimensions() -> impl Strategy<Value = (u16, u16)> {
    (1u16..=64, 1u16..=64)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: native-protocol-embedding, Property 1: RDP Framebuffer Event Conversion**
    /// **Validates: Requirements 1.1, 10.3**
    ///
    /// For any valid framebuffer data in any supported pixel format,
    /// converting to BGRA should produce valid output with correct size.
    #[test]
    fn framebuffer_conversion_produces_valid_bgra(
        (width, height) in arb_small_dimensions(),
        format in arb_pixel_format(),
    ) {
        // Generate appropriately sized data for the format
        let input_size = (width as usize) * (height as usize) * format.bytes_per_pixel();
        let input_data: Vec<u8> = (0..input_size).map(|i| (i % 256) as u8).collect();

        let result = convert_to_bgra(&input_data, format, width, height);

        // Conversion should succeed
        prop_assert!(result.is_some(), "Conversion should succeed for valid input");

        let bgra_data = result.unwrap();

        // Output should be BGRA format (4 bytes per pixel)
        let expected_size = (width as usize) * (height as usize) * 4;
        prop_assert_eq!(
            bgra_data.len(),
            expected_size,
            "Output should be {} bytes for {}x{} BGRA image",
            expected_size,
            width,
            height
        );
    }

    /// **Feature: native-protocol-embedding, Property 1: RDP Framebuffer Event Conversion**
    /// **Validates: Requirements 1.1, 10.3**
    ///
    /// BGRA to BGRA conversion should be identity (passthrough).
    #[test]
    fn bgra_conversion_is_identity(
        (width, height) in arb_small_dimensions(),
    ) {
        let size = (width as usize) * (height as usize) * 4;
        let input_data: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();

        let result = convert_to_bgra(&input_data, PixelFormat::Bgra, width, height);

        prop_assert!(result.is_some(), "BGRA conversion should succeed");
        prop_assert_eq!(
            result.unwrap(),
            input_data,
            "BGRA to BGRA should be identity"
        );
    }

    /// **Feature: native-protocol-embedding, Property 1: RDP Framebuffer Event Conversion**
    /// **Validates: Requirements 1.1, 10.3**
    ///
    /// RGBA to BGRA conversion should swap R and B channels.
    #[test]
    fn rgba_to_bgra_swaps_rb_channels(
        r in any::<u8>(),
        g in any::<u8>(),
        b in any::<u8>(),
        a in any::<u8>(),
    ) {
        let rgba = vec![r, g, b, a];
        let result = convert_to_bgra(&rgba, PixelFormat::Rgba, 1, 1);

        prop_assert!(result.is_some(), "RGBA conversion should succeed");
        let bgra = result.unwrap();

        prop_assert_eq!(bgra[0], b, "B channel should be swapped from position 2");
        prop_assert_eq!(bgra[1], g, "G channel should remain in position 1");
        prop_assert_eq!(bgra[2], r, "R channel should be swapped from position 0");
        prop_assert_eq!(bgra[3], a, "A channel should remain in position 3");
    }

    /// **Feature: native-protocol-embedding, Property 1: RDP Framebuffer Event Conversion**
    /// **Validates: Requirements 1.1, 10.3**
    ///
    /// RGB to BGRA conversion should swap R and B and add full alpha.
    #[test]
    fn rgb_to_bgra_adds_alpha(
        r in any::<u8>(),
        g in any::<u8>(),
        b in any::<u8>(),
    ) {
        let rgb = vec![r, g, b];
        let result = convert_to_bgra(&rgb, PixelFormat::Rgb, 1, 1);

        prop_assert!(result.is_some(), "RGB conversion should succeed");
        let bgra = result.unwrap();

        prop_assert_eq!(bgra[0], b, "B channel should be swapped from position 2");
        prop_assert_eq!(bgra[1], g, "G channel should remain in position 1");
        prop_assert_eq!(bgra[2], r, "R channel should be swapped from position 0");
        prop_assert_eq!(bgra[3], 255, "A channel should be 255 (fully opaque)");
    }

    /// **Feature: native-protocol-embedding, Property 1: RDP Framebuffer Event Conversion**
    /// **Validates: Requirements 1.1, 10.3**
    ///
    /// create_frame_update should produce valid FrameUpdate events for valid input.
    #[test]
    fn create_frame_update_produces_valid_event(
        x in 0u16..1000,
        y in 0u16..1000,
        (width, height) in arb_small_dimensions(),
    ) {
        let size = (width as usize) * (height as usize) * 4;
        let data: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();

        let event = create_frame_update(x, y, width, height, data.clone());

        match event {
            RdpClientEvent::FrameUpdate { rect, data: event_data } => {
                prop_assert_eq!(rect.x, x, "X coordinate should match");
                prop_assert_eq!(rect.y, y, "Y coordinate should match");
                prop_assert_eq!(rect.width, width, "Width should match");
                prop_assert_eq!(rect.height, height, "Height should match");
                prop_assert_eq!(event_data.len(), size, "Data size should match");
            }
            _ => prop_assert!(false, "Expected FrameUpdate event"),
        }
    }

    /// **Feature: native-protocol-embedding, Property 1: RDP Framebuffer Event Conversion**
    /// **Validates: Requirements 1.1, 10.3**
    ///
    /// create_frame_update_with_conversion should handle all pixel formats.
    #[test]
    fn create_frame_update_with_conversion_handles_all_formats(
        (width, height) in arb_small_dimensions(),
        format in arb_pixel_format(),
    ) {
        let input_size = (width as usize) * (height as usize) * format.bytes_per_pixel();
        let input_data: Vec<u8> = (0..input_size).map(|i| (i % 256) as u8).collect();

        let event = create_frame_update_with_conversion(0, 0, width, height, &input_data, format);

        match event {
            RdpClientEvent::FrameUpdate { rect, data } => {
                prop_assert_eq!(rect.width, width, "Width should match");
                prop_assert_eq!(rect.height, height, "Height should match");
                let expected_size = (width as usize) * (height as usize) * 4;
                prop_assert_eq!(data.len(), expected_size, "Output should be BGRA format");
            }
            RdpClientEvent::Error(msg) => {
                prop_assert!(false, "Conversion should not fail: {}", msg);
            }
            _ => prop_assert!(false, "Expected FrameUpdate event"),
        }
    }

    /// **Feature: native-protocol-embedding, Property 1: RDP Framebuffer Event Conversion**
    /// **Validates: Requirements 1.1, 10.3**
    ///
    /// Conversion should fail gracefully for insufficient data.
    #[test]
    fn conversion_fails_for_insufficient_data(
        (width, height) in arb_small_dimensions(),
        format in arb_pixel_format(),
    ) {
        let required_size = (width as usize) * (height as usize) * format.bytes_per_pixel();
        // Provide less data than required (but at least 1 byte to avoid empty vec edge case)
        let insufficient_size = (required_size / 2).max(1);
        let input_data: Vec<u8> = (0..insufficient_size).map(|i| (i % 256) as u8).collect();

        // Only test if we actually have insufficient data
        if input_data.len() < required_size {
            let result = convert_to_bgra(&input_data, format, width, height);
            prop_assert!(result.is_none(), "Conversion should fail for insufficient data");
        }
    }

    /// **Feature: native-protocol-embedding, Property 1: RDP Framebuffer Event Conversion**
    /// **Validates: Requirements 1.1, 10.3**
    ///
    /// RdpRect validation should correctly identify valid and invalid rectangles.
    #[test]
    fn rdp_rect_validation_is_consistent(
        x in 0u16..1000,
        y in 0u16..1000,
        width in 0u16..1000,
        height in 0u16..1000,
    ) {
        let rect = RdpRect::new(x, y, width, height);

        // is_valid should return true only if both dimensions are non-zero
        let expected_valid = width > 0 && height > 0;
        prop_assert_eq!(
            rect.is_valid(),
            expected_valid,
            "is_valid should match expected for {}x{}",
            width,
            height
        );

        // area should be width * height
        prop_assert_eq!(
            rect.area(),
            width as u32 * height as u32,
            "area should be width * height"
        );
    }
}

// ============================================================================
// Resource Cleanup Tests (Requirement 1.6)
// ============================================================================

// These tests require the rdp-embedded feature to access RdpClient
#[cfg(feature = "rdp-embedded")]
mod resource_cleanup_tests {
    use proptest::prelude::*;
    use rustconn_core::{RdpClient, RdpClientConfig};

    // Strategy for generating valid RDP client configs for cleanup testing
    fn arb_cleanup_config() -> impl Strategy<Value = RdpClientConfig> {
        (
            // Use localhost to avoid actual network connections
            Just("127.0.0.1".to_string()),
            // Use a port that's unlikely to have an RDP server
            Just(13389u16),
            // Random resolution
            (640u16..=1920, 480u16..=1080),
            // Random timeout (short for testing)
            1u64..=5,
        )
            .prop_map(|(host, port, (width, height), _timeout)| {
                RdpClientConfig::new(host)
                    .with_port(port)
                    .with_resolution(width, height)
                    .with_nla(false) // Disable NLA to avoid auth complexity
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// **Feature: native-protocol-embedding, Property 4: Resource Cleanup on Disconnect**
        /// **Validates: Requirements 1.6**
        ///
        /// For any RDP client configuration, after creating a client and calling disconnect,
        /// all resources (channels, thread handle) should be properly released.
        /// The client should report is_cleaned_up() == true.
        #[test]
        fn resource_cleanup_on_disconnect(config in arb_cleanup_config()) {
            // Create a new client
            let mut client = RdpClient::new(config);

            // Initially, client should be in cleaned up state (not connected)
            prop_assert!(
                client.is_cleaned_up(),
                "New client should be in cleaned up state"
            );
            prop_assert!(
                !client.is_connected(),
                "New client should not be connected"
            );

            // Call disconnect (should be safe even without connecting)
            client.disconnect();

            // After disconnect, client should be in cleaned up state
            prop_assert!(
                client.is_cleaned_up(),
                "Client should be cleaned up after disconnect"
            );
            prop_assert!(
                !client.is_connected(),
                "Client should not be connected after disconnect"
            );
        }

        /// **Feature: native-protocol-embedding, Property 4: Resource Cleanup on Disconnect**
        /// **Validates: Requirements 1.6**
        ///
        /// For any RDP client, calling disconnect multiple times should be safe
        /// and should not cause panics or resource leaks.
        #[test]
        fn multiple_disconnect_calls_are_safe(config in arb_cleanup_config()) {
            let mut client = RdpClient::new(config);

            // Call disconnect multiple times
            client.disconnect();
            client.disconnect();
            client.disconnect();

            // Client should still be in cleaned up state
            prop_assert!(
                client.is_cleaned_up(),
                "Client should be cleaned up after multiple disconnects"
            );
        }

        /// **Feature: native-protocol-embedding, Property 4: Resource Cleanup on Disconnect**
        /// **Validates: Requirements 1.6**
        ///
        /// For any RDP client, dropping the client should automatically clean up resources.
        /// This tests the Drop implementation.
        #[test]
        fn drop_cleans_up_resources(config in arb_cleanup_config()) {
            // Create client in a scope
            {
                let _client = RdpClient::new(config);
                // Client will be dropped at end of scope
            }
            // If we get here without panic, Drop worked correctly
            prop_assert!(true, "Drop should complete without panic");
        }
    }
}

// ============================================================================
// Coordinate Transformation Tests (Requirement 1.2)
// ============================================================================

use rustconn_core::{
    find_best_standard_resolution, generate_resize_request, should_resize, CoordinateTransform,
    MAX_RDP_HEIGHT, MAX_RDP_WIDTH, MIN_RDP_HEIGHT, MIN_RDP_WIDTH, STANDARD_RESOLUTIONS,
};

// Strategy for generating valid widget dimensions
fn arb_widget_dimensions() -> impl Strategy<Value = (u32, u32)> {
    (100u32..=4000, 100u32..=4000)
}

// Strategy for generating valid RDP framebuffer dimensions
fn arb_rdp_dimensions() -> impl Strategy<Value = (u32, u32)> {
    (200u32..=3840, 200u32..=2160)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: native-protocol-embedding, Property 2: Mouse Coordinate Transformation**
    /// **Validates: Requirements 1.2**
    ///
    /// For any widget coordinates within the embedded RDP widget bounds,
    /// the transformation to RDP server coordinates should produce valid
    /// coordinates within the RDP framebuffer dimensions.
    #[test]
    fn coordinate_transform_produces_valid_rdp_coords(
        (widget_w, widget_h) in arb_widget_dimensions(),
        (rdp_w, rdp_h) in arb_rdp_dimensions(),
    ) {
        let transform = CoordinateTransform::new(widget_w, widget_h, rdp_w, rdp_h);

        // Get the framebuffer bounds within the widget
        let (fb_x, fb_y, fb_w, fb_h) = transform.framebuffer_bounds();

        // Generate a point within the framebuffer display area
        let widget_x = fb_x + fb_w / 2.0;
        let widget_y = fb_y + fb_h / 2.0;

        // Transform should succeed for points within framebuffer
        let result = transform.transform(widget_x, widget_y);
        prop_assert!(result.is_some(), "Transform should succeed for point within framebuffer");

        let (rdp_x, rdp_y) = result.unwrap();

        // Result should be within RDP bounds
        prop_assert!(rdp_x >= 0.0, "RDP X should be >= 0, got {}", rdp_x);
        prop_assert!(rdp_y >= 0.0, "RDP Y should be >= 0, got {}", rdp_y);
        prop_assert!(rdp_x < rdp_w as f64, "RDP X should be < {}, got {}", rdp_w, rdp_x);
        prop_assert!(rdp_y < rdp_h as f64, "RDP Y should be < {}, got {}", rdp_h, rdp_y);
    }

    /// **Feature: native-protocol-embedding, Property 2: Mouse Coordinate Transformation**
    /// **Validates: Requirements 1.2**
    ///
    /// For any widget coordinates, transform_clamped should always produce
    /// valid RDP coordinates within bounds.
    #[test]
    fn coordinate_transform_clamped_always_valid(
        (widget_w, widget_h) in arb_widget_dimensions(),
        (rdp_w, rdp_h) in arb_rdp_dimensions(),
        widget_x in -1000.0f64..5000.0,
        widget_y in -1000.0f64..5000.0,
    ) {
        let transform = CoordinateTransform::new(widget_w, widget_h, rdp_w, rdp_h);
        let (rdp_x, rdp_y) = transform.transform_clamped(widget_x, widget_y);

        // Result should always be within valid RDP bounds
        prop_assert!(rdp_x >= 0.0, "Clamped RDP X should be >= 0, got {}", rdp_x);
        prop_assert!(rdp_y >= 0.0, "Clamped RDP Y should be >= 0, got {}", rdp_y);
        prop_assert!(
            rdp_x <= (rdp_w - 1) as f64,
            "Clamped RDP X should be <= {}, got {}",
            rdp_w - 1,
            rdp_x
        );
        prop_assert!(
            rdp_y <= (rdp_h - 1) as f64,
            "Clamped RDP Y should be <= {}, got {}",
            rdp_h - 1,
            rdp_y
        );
    }

    /// **Feature: native-protocol-embedding, Property 2: Mouse Coordinate Transformation**
    /// **Validates: Requirements 1.2**
    ///
    /// For any coordinate transform, the center of the widget should map
    /// to approximately the center of the RDP framebuffer.
    #[test]
    fn coordinate_transform_center_maps_to_center(
        (widget_w, widget_h) in arb_widget_dimensions(),
        (rdp_w, rdp_h) in arb_rdp_dimensions(),
    ) {
        let transform = CoordinateTransform::new(widget_w, widget_h, rdp_w, rdp_h);

        // Get the center of the framebuffer display area in widget coords
        let (fb_x, fb_y, fb_w, fb_h) = transform.framebuffer_bounds();
        let widget_center_x = fb_x + fb_w / 2.0;
        let widget_center_y = fb_y + fb_h / 2.0;

        // Transform to RDP coords
        let (rdp_x, rdp_y) = transform.transform_clamped(widget_center_x, widget_center_y);

        // Should map to center of RDP framebuffer
        let rdp_center_x = rdp_w as f64 / 2.0;
        let rdp_center_y = rdp_h as f64 / 2.0;

        // Allow small tolerance for floating point
        prop_assert!(
            (rdp_x - rdp_center_x).abs() < 1.0,
            "RDP X {} should be close to center {}",
            rdp_x,
            rdp_center_x
        );
        prop_assert!(
            (rdp_y - rdp_center_y).abs() < 1.0,
            "RDP Y {} should be close to center {}",
            rdp_y,
            rdp_center_y
        );
    }

    /// **Feature: native-protocol-embedding, Property 2: Mouse Coordinate Transformation**
    /// **Validates: Requirements 1.2**
    ///
    /// When widget and RDP have the same dimensions, transform should be identity.
    #[test]
    fn coordinate_transform_identity_when_same_size(
        (w, h) in arb_rdp_dimensions(),
        x in 0.0f64..1000.0,
        y in 0.0f64..1000.0,
    ) {
        let transform = CoordinateTransform::new(w, h, w, h);

        // Scale should be 1.0
        prop_assert!(
            (transform.scale - 1.0).abs() < 0.001,
            "Scale should be 1.0 for same dimensions, got {}",
            transform.scale
        );

        // Offsets should be 0
        prop_assert!(
            transform.offset_x.abs() < 0.001,
            "Offset X should be 0, got {}",
            transform.offset_x
        );
        prop_assert!(
            transform.offset_y.abs() < 0.001,
            "Offset Y should be 0, got {}",
            transform.offset_y
        );

        // Clamp input to valid range
        let clamped_x = x.min((w - 1) as f64);
        let clamped_y = y.min((h - 1) as f64);

        // Transform should be identity
        if let Some((rdp_x, rdp_y)) = transform.transform(clamped_x, clamped_y) {
            prop_assert!(
                (rdp_x - clamped_x).abs() < 0.001,
                "X should be unchanged: {} vs {}",
                rdp_x,
                clamped_x
            );
            prop_assert!(
                (rdp_y - clamped_y).abs() < 0.001,
                "Y should be unchanged: {} vs {}",
                rdp_y,
                clamped_y
            );
        }
    }

    /// **Feature: native-protocol-embedding, Property 2: Mouse Coordinate Transformation**
    /// **Validates: Requirements 1.2**
    ///
    /// transform_to_u16 should produce valid u16 coordinates.
    #[test]
    fn coordinate_transform_to_u16_valid(
        (widget_w, widget_h) in arb_widget_dimensions(),
        (rdp_w, rdp_h) in arb_rdp_dimensions(),
        widget_x in -1000.0f64..5000.0,
        widget_y in -1000.0f64..5000.0,
    ) {
        let transform = CoordinateTransform::new(widget_w, widget_h, rdp_w, rdp_h);
        let (rdp_x, rdp_y) = transform.transform_to_u16(widget_x, widget_y);

        // Result should be within valid u16 range and RDP bounds
        prop_assert!(rdp_x < rdp_w as u16, "u16 X {} should be < {}", rdp_x, rdp_w);
        prop_assert!(rdp_y < rdp_h as u16, "u16 Y {} should be < {}", rdp_y, rdp_h);
    }

    /// **Feature: native-protocol-embedding, Property 2: Mouse Coordinate Transformation**
    /// **Validates: Requirements 1.2**
    ///
    /// Aspect ratio should be preserved (scale is uniform).
    #[test]
    fn coordinate_transform_preserves_aspect_ratio(
        (widget_w, widget_h) in arb_widget_dimensions(),
        (rdp_w, rdp_h) in arb_rdp_dimensions(),
    ) {
        let transform = CoordinateTransform::new(widget_w, widget_h, rdp_w, rdp_h);

        // The scale should be the minimum of the two possible scales
        let scale_x = widget_w as f64 / rdp_w as f64;
        let scale_y = widget_h as f64 / rdp_h as f64;
        let expected_scale = scale_x.min(scale_y);

        prop_assert!(
            (transform.scale - expected_scale).abs() < 0.001,
            "Scale {} should equal min({}, {}) = {}",
            transform.scale,
            scale_x,
            scale_y,
            expected_scale
        );
    }
}

// ============================================================================
// Resize Request Tests (Requirement 1.7)
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: native-protocol-embedding, Property 5: Resize Request Generation**
    /// **Validates: Requirements 1.7**
    ///
    /// For any resize event on the embedded RDP widget, the system should
    /// generate a resolution change request with dimensions that fit within
    /// standard RDP resolution limits.
    #[test]
    fn resize_request_within_rdp_limits(
        widget_w in 100u32..10000,
        widget_h in 100u32..10000,
        use_standard in any::<bool>(),
    ) {
        let (width, height) = generate_resize_request(widget_w, widget_h, use_standard);

        // Result should be within RDP limits
        prop_assert!(
            width >= MIN_RDP_WIDTH,
            "Width {} should be >= {}",
            width,
            MIN_RDP_WIDTH
        );
        prop_assert!(
            height >= MIN_RDP_HEIGHT,
            "Height {} should be >= {}",
            height,
            MIN_RDP_HEIGHT
        );
        prop_assert!(
            width <= MAX_RDP_WIDTH,
            "Width {} should be <= {}",
            width,
            MAX_RDP_WIDTH
        );
        prop_assert!(
            height <= MAX_RDP_HEIGHT,
            "Height {} should be <= {}",
            height,
            MAX_RDP_HEIGHT
        );
    }

    /// **Feature: native-protocol-embedding, Property 5: Resize Request Generation**
    /// **Validates: Requirements 1.7**
    ///
    /// When using standard resolutions, the result should be a known standard resolution.
    #[test]
    fn resize_request_uses_standard_resolution(
        widget_w in 200u32..4000,
        widget_h in 200u32..4000,
    ) {
        let (width, height) = generate_resize_request(widget_w, widget_h, true);

        // Result should be one of the standard resolutions
        let is_standard = STANDARD_RESOLUTIONS
            .iter()
            .any(|&(w, h)| w == u32::from(width) && h == u32::from(height));

        prop_assert!(
            is_standard,
            "Resolution {}x{} should be a standard resolution",
            width,
            height
        );
    }

    /// **Feature: native-protocol-embedding, Property 5: Resize Request Generation**
    /// **Validates: Requirements 1.7**
    ///
    /// Standard resolution should fit within the widget dimensions when widget
    /// is large enough to contain at least the smallest standard resolution.
    #[test]
    fn standard_resolution_fits_widget(
        // Use dimensions >= smallest standard resolution (640x480)
        widget_w in 640u32..4000,
        widget_h in 480u32..4000,
    ) {
        let (res_w, res_h) = find_best_standard_resolution(widget_w, widget_h);

        prop_assert!(
            res_w <= widget_w,
            "Standard width {} should fit in widget width {}",
            res_w,
            widget_w
        );
        prop_assert!(
            res_h <= widget_h,
            "Standard height {} should fit in widget height {}",
            res_h,
            widget_h
        );
    }

    /// **Feature: native-protocol-embedding, Property 5: Resize Request Generation**
    /// **Validates: Requirements 1.7**
    ///
    /// should_resize correctly detects when resize is needed.
    #[test]
    fn should_resize_detects_changes(
        current_w in 200u16..4000,
        current_h in 200u16..4000,
        delta_w in 0i32..200,
        delta_h in 0i32..200,
        threshold in 10u16..100,
    ) {
        let new_w = (current_w as i32 + delta_w).clamp(200, 8000) as u16;
        let new_h = (current_h as i32 + delta_h).clamp(200, 8000) as u16;

        let result = should_resize(current_w, current_h, new_w, new_h, threshold);

        // Calculate expected result
        let width_diff = (new_w as i32 - current_w as i32).unsigned_abs();
        let height_diff = (new_h as i32 - current_h as i32).unsigned_abs();
        let expected = width_diff >= threshold as u32 || height_diff >= threshold as u32;

        prop_assert_eq!(
            result,
            expected,
            "should_resize({}, {}, {}, {}, {}) = {} but expected {}",
            current_w,
            current_h,
            new_w,
            new_h,
            threshold,
            result,
            expected
        );
    }
}

// ============================================================================
// Keyboard Event Forwarding Tests (Requirement 1.3)
// ============================================================================

use rustconn_core::{
    ctrl_alt_del_sequence, is_modifier_keyval, is_printable_keyval, keyval_to_scancode,
    SCANCODE_ALT, SCANCODE_CTRL, SCANCODE_DELETE,
};

// GDK keyval constants for testing
const GDK_KEY_A: u32 = 0x41;
const GDK_KEY_Z: u32 = 0x5A;
const GDK_KEY_A_LOWER: u32 = 0x61;
const GDK_KEY_Z_LOWER: u32 = 0x7A;
const GDK_KEY_0: u32 = 0x30;
const GDK_KEY_9: u32 = 0x39;
const GDK_KEY_F1: u32 = 0xFFBE;
const GDK_KEY_F12: u32 = 0xFFC9;
const GDK_KEY_ESCAPE: u32 = 0xFF1B;
const GDK_KEY_RETURN: u32 = 0xFF0D;
const GDK_KEY_SPACE: u32 = 0x20;
const GDK_KEY_BACKSPACE: u32 = 0xFF08;
const GDK_KEY_TAB: u32 = 0xFF09;
const GDK_KEY_SHIFT_L: u32 = 0xFFE1;
const GDK_KEY_CONTROL_L: u32 = 0xFFE3;
const GDK_KEY_ALT_L: u32 = 0xFFE9;
const GDK_KEY_HOME: u32 = 0xFF50;
const GDK_KEY_END: u32 = 0xFF57;
const GDK_KEY_LEFT: u32 = 0xFF51;
const GDK_KEY_RIGHT: u32 = 0xFF53;
const GDK_KEY_UP: u32 = 0xFF52;
const GDK_KEY_DOWN: u32 = 0xFF54;
const GDK_KEY_DELETE: u32 = 0xFFFF;
const GDK_KEY_INSERT: u32 = 0xFF63;

// Strategy for generating common keyvals
fn arb_common_keyval() -> impl Strategy<Value = u32> {
    prop_oneof![
        // Letters (lowercase)
        (GDK_KEY_A_LOWER..=GDK_KEY_Z_LOWER),
        // Letters (uppercase)
        (GDK_KEY_A..=GDK_KEY_Z),
        // Numbers
        (GDK_KEY_0..=GDK_KEY_9),
        // Function keys
        (GDK_KEY_F1..=GDK_KEY_F12),
        // Special keys
        Just(GDK_KEY_ESCAPE),
        Just(GDK_KEY_RETURN),
        Just(GDK_KEY_SPACE),
        Just(GDK_KEY_BACKSPACE),
        Just(GDK_KEY_TAB),
        // Modifiers
        Just(GDK_KEY_SHIFT_L),
        Just(GDK_KEY_CONTROL_L),
        Just(GDK_KEY_ALT_L),
        // Navigation
        Just(GDK_KEY_HOME),
        Just(GDK_KEY_END),
        Just(GDK_KEY_LEFT),
        Just(GDK_KEY_RIGHT),
        Just(GDK_KEY_UP),
        Just(GDK_KEY_DOWN),
        Just(GDK_KEY_DELETE),
        Just(GDK_KEY_INSERT),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: native-protocol-embedding, Property 3: Key Event Forwarding**
    /// **Validates: Requirements 1.3**
    ///
    /// For any GTK key event with a valid keyval, the system should produce
    /// a corresponding RDP scancode that can be sent to the server.
    #[test]
    fn keyval_produces_valid_scancode(keyval in arb_common_keyval()) {
        let result = keyval_to_scancode(keyval);

        // All common keyvals should produce a scancode
        prop_assert!(
            result.is_some(),
            "Common keyval {:#x} should produce a scancode",
            keyval
        );

        let scancode = result.unwrap();

        // Scancode should be in valid range (0x00-0x7F for standard keys)
        prop_assert!(
            scancode.code <= 0x7F || scancode.extended,
            "Scancode {:#x} should be <= 0x7F or extended",
            scancode.code
        );
    }

    /// **Feature: native-protocol-embedding, Property 3: Key Event Forwarding**
    /// **Validates: Requirements 1.3**
    ///
    /// Lowercase and uppercase letters should produce the same scancode.
    #[test]
    fn case_insensitive_letter_scancodes(letter in 0u32..26) {
        let lowercase = GDK_KEY_A_LOWER + letter;
        let uppercase = GDK_KEY_A + letter;

        let lower_scancode = keyval_to_scancode(lowercase);
        let upper_scancode = keyval_to_scancode(uppercase);

        prop_assert!(lower_scancode.is_some(), "Lowercase letter should have scancode");
        prop_assert!(upper_scancode.is_some(), "Uppercase letter should have scancode");

        prop_assert_eq!(
            lower_scancode,
            upper_scancode,
            "Lowercase and uppercase should have same scancode"
        );
    }

    /// **Feature: native-protocol-embedding, Property 3: Key Event Forwarding**
    /// **Validates: Requirements 1.3**
    ///
    /// Navigation keys should produce extended scancodes.
    #[test]
    fn navigation_keys_are_extended(
        keyval in prop_oneof![
            Just(GDK_KEY_HOME),
            Just(GDK_KEY_END),
            Just(GDK_KEY_LEFT),
            Just(GDK_KEY_RIGHT),
            Just(GDK_KEY_UP),
            Just(GDK_KEY_DOWN),
            Just(GDK_KEY_DELETE),
            Just(GDK_KEY_INSERT),
        ]
    ) {
        let scancode = keyval_to_scancode(keyval);

        prop_assert!(scancode.is_some(), "Navigation key should have scancode");
        prop_assert!(
            scancode.unwrap().extended,
            "Navigation key {:#x} should be extended",
            keyval
        );
    }

    /// **Feature: native-protocol-embedding, Property 3: Key Event Forwarding**
    /// **Validates: Requirements 1.3**
    ///
    /// All scancodes should be unique for different key groups.
    #[test]
    fn letter_scancodes_are_unique(
        letter1 in 0u32..26,
        letter2 in 0u32..26,
    ) {
        prop_assume!(letter1 != letter2);

        let keyval1 = GDK_KEY_A_LOWER + letter1;
        let keyval2 = GDK_KEY_A_LOWER + letter2;

        let scancode1 = keyval_to_scancode(keyval1);
        let scancode2 = keyval_to_scancode(keyval2);

        prop_assert!(scancode1.is_some());
        prop_assert!(scancode2.is_some());

        prop_assert_ne!(
            scancode1.unwrap().code,
            scancode2.unwrap().code,
            "Different letters should have different scancodes"
        );
    }

    /// **Feature: native-protocol-embedding, Property 3: Key Event Forwarding**
    /// **Validates: Requirements 1.3**
    ///
    /// Number key scancodes should be sequential.
    #[test]
    fn number_scancodes_are_sequential(num in 1u32..9) {
        let keyval1 = GDK_KEY_0 + num;
        let keyval2 = GDK_KEY_0 + num + 1;

        let scancode1 = keyval_to_scancode(keyval1);
        let scancode2 = keyval_to_scancode(keyval2);

        prop_assert!(scancode1.is_some());
        prop_assert!(scancode2.is_some());

        // Number scancodes should be sequential (1-9 are 0x02-0x0A, 0 is 0x0B)
        // This is the AT keyboard layout
        let s1 = scancode1.unwrap().code;
        let s2 = scancode2.unwrap().code;

        // For numbers 1-9, scancodes are sequential
        if num < 9 {
            prop_assert_eq!(
                s2,
                s1 + 1,
                "Number scancodes should be sequential: {} + 1 != {}",
                s1,
                s2
            );
        }
    }

    /// **Feature: native-protocol-embedding, Property 3: Key Event Forwarding**
    /// **Validates: Requirements 1.3**
    ///
    /// Function key scancodes should be sequential.
    #[test]
    fn function_key_scancodes_are_sequential(fkey in 0u32..11) {
        let keyval1 = GDK_KEY_F1 + fkey;
        let keyval2 = GDK_KEY_F1 + fkey + 1;

        let scancode1 = keyval_to_scancode(keyval1);
        let scancode2 = keyval_to_scancode(keyval2);

        prop_assert!(scancode1.is_some(), "F{} should have scancode", fkey + 1);
        prop_assert!(scancode2.is_some(), "F{} should have scancode", fkey + 2);

        let s1 = scancode1.unwrap().code;
        let s2 = scancode2.unwrap().code;

        // F1-F10 are sequential (0x3B-0x44), F11-F12 are 0x57-0x58
        if fkey < 9 {
            prop_assert_eq!(
                s2,
                s1 + 1,
                "Function key scancodes should be sequential: F{} ({:#x}) + 1 != F{} ({:#x})",
                fkey + 1,
                s1,
                fkey + 2,
                s2
            );
        }
    }
}

// Non-property tests for Ctrl+Alt+Del sequence
#[test]
fn test_ctrl_alt_del_sequence_structure() {
    let sequence = ctrl_alt_del_sequence();

    // Should have 6 events: 3 presses + 3 releases
    assert_eq!(sequence.len(), 6);

    // First event should be Ctrl press
    assert_eq!(sequence[0].0, SCANCODE_CTRL.code);
    assert_eq!(sequence[0].1, SCANCODE_CTRL.extended);
    assert!(sequence[0].2); // pressed

    // Second event should be Alt press
    assert_eq!(sequence[1].0, SCANCODE_ALT.code);
    assert_eq!(sequence[1].1, SCANCODE_ALT.extended);
    assert!(sequence[1].2); // pressed

    // Third event should be Delete press
    assert_eq!(sequence[2].0, SCANCODE_DELETE.code);
    assert_eq!(sequence[2].1, SCANCODE_DELETE.extended);
    assert!(sequence[2].2); // pressed

    // Fourth event should be Delete release
    assert_eq!(sequence[3].0, SCANCODE_DELETE.code);
    assert!(!sequence[3].2); // released

    // Fifth event should be Alt release
    assert_eq!(sequence[4].0, SCANCODE_ALT.code);
    assert!(!sequence[4].2); // released

    // Sixth event should be Ctrl release
    assert_eq!(sequence[5].0, SCANCODE_CTRL.code);
    assert!(!sequence[5].2); // released
}

#[test]
fn test_is_printable_keyval() {
    // Printable ASCII
    assert!(is_printable_keyval(0x20)); // Space
    assert!(is_printable_keyval(0x41)); // A
    assert!(is_printable_keyval(0x7E)); // ~

    // Non-printable
    assert!(!is_printable_keyval(0x1B)); // Escape (control char)
    assert!(!is_printable_keyval(0x7F)); // DEL
    assert!(!is_printable_keyval(GDK_KEY_F1)); // Function key
    assert!(!is_printable_keyval(GDK_KEY_RETURN)); // Return
}

#[test]
fn test_is_modifier_keyval() {
    // Modifiers
    assert!(is_modifier_keyval(GDK_KEY_SHIFT_L));
    assert!(is_modifier_keyval(GDK_KEY_CONTROL_L));
    assert!(is_modifier_keyval(GDK_KEY_ALT_L));

    // Non-modifiers
    assert!(!is_modifier_keyval(0x41)); // A
    assert!(!is_modifier_keyval(GDK_KEY_F1)); // F1
    assert!(!is_modifier_keyval(GDK_KEY_RETURN)); // Return
}
