//! Property tests for VNC client module

use proptest::prelude::*;
use rustconn_core::vnc_client::is_embedded_vnc_available;

// Note: VncClientConfig, VncRect, etc. are only available with vnc-embedded feature
// These tests focus on the always-available functionality

// ============================================================================
// Feature Detection Tests
// ============================================================================

#[test]
fn is_embedded_vnc_available_returns_bool() {
    // This should compile and return a boolean
    let _available = is_embedded_vnc_available();
}

#[test]
fn is_embedded_vnc_available_is_const() {
    // Verify it can be used in const context
    const AVAILABLE: bool = is_embedded_vnc_available();
    let _ = AVAILABLE;
}

// ============================================================================
// VNC Config Tests (when feature enabled)
// ============================================================================

#[cfg(feature = "vnc-embedded")]
mod vnc_config_tests {
    use super::*;
    use rustconn_core::vnc_client::{VncClientConfig, VncClientEvent, VncRect};

    fn arb_host() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("localhost".to_string()),
            Just("127.0.0.1".to_string()),
            "[a-z]{3,10}\\.[a-z]{2,4}".prop_map(|s| s),
            "192\\.168\\.[0-9]{1,3}\\.[0-9]{1,3}".prop_map(|s| s),
        ]
    }

    fn arb_port() -> impl Strategy<Value = u16> {
        prop_oneof![
            Just(5900u16),
            Just(5901u16),
            5900u16..5999u16,
            1024u16..65535u16,
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(50))]

        #[test]
        fn config_new_preserves_host(host in arb_host()) {
            let config = VncClientConfig::new(&host);
            prop_assert_eq!(config.host, host);
        }

        #[test]
        fn config_with_port_preserves_port(host in arb_host(), port in arb_port()) {
            let config = VncClientConfig::new(&host).with_port(port);
            prop_assert_eq!(config.port, port);
        }

        #[test]
        fn config_server_address_format(host in arb_host(), port in arb_port()) {
            let config = VncClientConfig::new(&host).with_port(port);
            let expected = format!("{}:{}", host, port);
            prop_assert_eq!(config.server_address(), expected);
        }

        #[test]
        fn config_with_view_only_preserves_flag(host in arb_host(), view_only in any::<bool>()) {
            let config = VncClientConfig::new(&host).with_view_only(view_only);
            prop_assert_eq!(config.view_only, view_only);
        }

        #[test]
        fn config_with_shared_preserves_flag(host in arb_host(), shared in any::<bool>()) {
            let config = VncClientConfig::new(&host).with_shared(shared);
            prop_assert_eq!(config.shared, shared);
        }

        #[test]
        fn config_with_password_sets_password(host in arb_host(), password in "[a-zA-Z0-9]{4,20}") {
            let config = VncClientConfig::new(&host).with_password(&password);
            let stored = config
                .password
                .as_ref()
                .map(|p| secrecy::ExposeSecret::expose_secret(p).to_string());
            prop_assert_eq!(stored, Some(password));
        }
    }

    // ============================================================================
    // VncRect Tests
    // ============================================================================

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn vnc_rect_preserves_values(x in any::<u16>(), y in any::<u16>(), w in any::<u16>(), h in any::<u16>()) {
            let rect = VncRect::new(x, y, w, h);
            prop_assert_eq!(rect.x, x);
            prop_assert_eq!(rect.y, y);
            prop_assert_eq!(rect.width, w);
            prop_assert_eq!(rect.height, h);
        }

        #[test]
        fn vnc_rect_equality(x in any::<u16>(), y in any::<u16>(), w in any::<u16>(), h in any::<u16>()) {
            let rect1 = VncRect::new(x, y, w, h);
            let rect2 = VncRect::new(x, y, w, h);
            prop_assert_eq!(rect1, rect2);
        }
    }

    #[test]
    fn vnc_rect_clone() {
        let rect1 = VncRect::new(10, 20, 100, 200);
        let rect2 = rect1;
        assert_eq!(rect1, rect2);
    }

    #[test]
    fn vnc_rect_debug() {
        let rect = VncRect::new(0, 0, 1920, 1080);
        let debug = format!("{:?}", rect);
        assert!(debug.contains("1920"));
        assert!(debug.contains("1080"));
    }

    // ============================================================================
    // VncClientEvent Tests
    // ============================================================================

    #[test]
    fn vnc_event_connected_debug() {
        let event = VncClientEvent::Connected;
        let debug = format!("{:?}", event);
        assert!(debug.contains("Connected"));
    }

    #[test]
    fn vnc_event_disconnected_debug() {
        let event = VncClientEvent::Disconnected;
        let debug = format!("{:?}", event);
        assert!(debug.contains("Disconnected"));
    }

    #[test]
    fn vnc_event_resolution_changed() {
        let event = VncClientEvent::ResolutionChanged {
            width: 1920,
            height: 1080,
        };
        if let VncClientEvent::ResolutionChanged { width, height } = event {
            assert_eq!(width, 1920);
            assert_eq!(height, 1080);
        } else {
            panic!("Expected ResolutionChanged");
        }
    }

    #[test]
    fn vnc_event_frame_update() {
        let rect = VncRect::new(0, 0, 100, 100);
        let data = vec![0u8; 100 * 100 * 4]; // BGRA
        let event = VncClientEvent::FrameUpdate {
            rect,
            data: data.clone(),
        };
        if let VncClientEvent::FrameUpdate { rect: r, data: d } = event {
            assert_eq!(r.width, 100);
            assert_eq!(d.len(), data.len());
        } else {
            panic!("Expected FrameUpdate");
        }
    }

    #[test]
    fn vnc_event_bell() {
        let event = VncClientEvent::Bell;
        assert!(matches!(event, VncClientEvent::Bell));
    }

    #[test]
    fn vnc_event_clipboard_text() {
        let event = VncClientEvent::ClipboardText("test".to_string());
        if let VncClientEvent::ClipboardText(text) = event {
            assert_eq!(text, "test");
        } else {
            panic!("Expected ClipboardText");
        }
    }

    #[test]
    fn vnc_event_error() {
        let event = VncClientEvent::Error("connection failed".to_string());
        if let VncClientEvent::Error(msg) = event {
            assert!(msg.contains("connection"));
        } else {
            panic!("Expected Error");
        }
    }

    // ============================================================================
    // VncClientConfig Default Tests
    // ============================================================================

    #[test]
    fn config_default_port_is_5900() {
        let config = VncClientConfig::default();
        assert_eq!(config.port, 5900);
    }

    #[test]
    fn config_default_shared_is_true() {
        let config = VncClientConfig::default();
        assert!(config.shared);
    }

    #[test]
    fn config_default_view_only_is_false() {
        let config = VncClientConfig::default();
        assert!(!config.view_only);
    }

    #[test]
    fn config_default_password_is_none() {
        let config = VncClientConfig::default();
        assert!(config.password.is_none());
    }

    #[test]
    fn config_default_timeout_is_30() {
        let config = VncClientConfig::default();
        assert_eq!(config.timeout_secs, 30);
    }

    #[test]
    fn config_default_has_encodings() {
        let config = VncClientConfig::default();
        assert!(!config.encodings.is_empty());
    }

    // ============================================================================
    // Config Serialization Tests
    // ============================================================================

    #[test]
    fn config_json_serialization() {
        let config = VncClientConfig::new("localhost")
            .with_port(5901)
            .with_view_only(true);

        let json = serde_json::to_string(&config).expect("serialize");
        let restored: VncClientConfig = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(config.host, restored.host);
        assert_eq!(config.port, restored.port);
        assert_eq!(config.view_only, restored.view_only);
    }

    #[test]
    fn config_password_not_serialized() {
        let config = VncClientConfig::new("localhost").with_password("secret");

        let json = serde_json::to_string(&config).expect("serialize");

        // Password should not appear in JSON (skip_serializing)
        assert!(!json.contains("secret"));
    }
}
