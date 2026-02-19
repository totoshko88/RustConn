//! Property-based tests for FreeRDP external mode command building
//!
//! These tests validate the correctness properties for FreeRDP command generation
//! as defined in the design document.
//!
//! # Requirements Coverage
//!
//! - Requirement 6.1: `/decorations` flag for window controls
//! - Requirement 6.2: Window geometry persistence (save on close)
//! - Requirement 6.3: Window geometry restoration (load on start)
//! - Requirement 6.4: Respect `remember_window_position` setting

use proptest::prelude::*;
use rustconn_core::models::WindowGeometry;
use rustconn_core::protocol::{
    build_freerdp_args, extract_geometry_from_args, has_decorations_flag, FreeRdpConfig,
};

// ============================================================================
// Generators for FreeRDP configurations
// ============================================================================

/// Strategy for generating valid hostnames
fn arb_hostname() -> impl Strategy<Value = String> {
    "[a-z0-9]([a-z0-9-]{0,30}[a-z0-9])?(\\.[a-z0-9]([a-z0-9-]{0,30}[a-z0-9])?)*"
}

/// Strategy for generating valid ports
fn arb_port() -> impl Strategy<Value = u16> {
    1u16..65535
}

/// Strategy for generating optional usernames
fn arb_username() -> impl Strategy<Value = Option<String>> {
    prop::option::of("[a-z][a-z0-9_-]{0,20}")
}

/// Strategy for generating optional passwords
fn arb_password() -> impl Strategy<Value = Option<String>> {
    prop::option::of("[a-zA-Z0-9!@#$%^&*]{1,30}")
}

/// Strategy for generating optional domains
fn arb_domain() -> impl Strategy<Value = Option<String>> {
    prop::option::of("[A-Z][A-Z0-9_-]{0,15}")
}

/// Strategy for generating valid resolutions
fn arb_resolution() -> impl Strategy<Value = (u32, u32)> {
    (640u32..3840, 480u32..2160)
}

/// Strategy for generating valid window geometry
fn arb_window_geometry() -> impl Strategy<Value = WindowGeometry> {
    (
        -10000i32..10000i32, // x position
        -10000i32..10000i32, // y position
        100i32..3840i32,     // width
        100i32..2160i32,     // height
    )
        .prop_map(|(x, y, width, height)| WindowGeometry::new(x, y, width, height))
}

/// Strategy for generating optional window geometry
fn arb_optional_geometry() -> impl Strategy<Value = Option<WindowGeometry>> {
    prop_oneof![Just(None), arb_window_geometry().prop_map(Some),]
}

/// Strategy for generating FreeRDP configurations
fn arb_freerdp_config() -> impl Strategy<Value = FreeRdpConfig> {
    (
        arb_hostname(),
        arb_port(),
        arb_username(),
        arb_password(),
        arb_domain(),
        arb_resolution(),
        any::<bool>(),           // clipboard_enabled
        arb_optional_geometry(), // window_geometry
        any::<bool>(),           // remember_window_position
    )
        .prop_map(
            |(
                host,
                port,
                username,
                password,
                domain,
                (width, height),
                clipboard_enabled,
                window_geometry,
                remember_window_position,
            )| {
                let mut config = FreeRdpConfig::new(host)
                    .with_port(port)
                    .with_resolution(width, height)
                    .with_clipboard(clipboard_enabled)
                    .with_remember_window_position(remember_window_position);

                if let Some(u) = username {
                    config = config.with_username(u);
                }
                if let Some(p) = password {
                    config = config.with_password(p);
                }
                if let Some(d) = domain {
                    config = config.with_domain(d);
                }
                if let Some(g) = window_geometry {
                    config = config.with_window_geometry(g);
                }

                config
            },
        )
}

// ============================================================================
// Property Tests
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // **Feature: native-protocol-embedding, Property 17: FreeRDP Decorations Flag**
    // **Validates: Requirements 6.1**
    //
    // For any FreeRDP external mode launch, the command should include the `/decorations` flag.

    #[test]
    fn prop_freerdp_decorations_flag_always_present(config in arb_freerdp_config()) {
        let args = build_freerdp_args(&config);

        prop_assert!(
            has_decorations_flag(&args),
            "FreeRDP command must always include /decorations flag. Got: {:?}",
            args
        );
    }

    // **Feature: native-protocol-embedding, Property 18: Window Geometry Restoration**
    // **Validates: Requirements 6.3**
    //
    // For any connection with saved window geometry and remember_window_position enabled,
    // the restored geometry should match the saved values.

    #[test]
    fn prop_window_geometry_restoration(
        config in arb_freerdp_config().prop_filter(
            "Config must have geometry and remember_window_position enabled",
            |c| c.window_geometry.is_some() && c.remember_window_position
        )
    ) {
        let args = build_freerdp_args(&config);
        let geometry = config.window_geometry.unwrap();

        // Extract geometry from args
        let extracted = extract_geometry_from_args(&args);

        prop_assert!(
            extracted.is_some(),
            "Geometry should be present in args when remember_window_position is true. Got: {:?}",
            args
        );

        let (x, y) = extracted.unwrap();
        prop_assert_eq!(
            x, geometry.x,
            "X position should match saved geometry"
        );
        prop_assert_eq!(
            y, geometry.y,
            "Y position should match saved geometry"
        );
    }

    // **Feature: native-protocol-embedding, Property 19: Disabled Position Memory**
    // **Validates: Requirements 6.4**
    //
    // For any connection with remember_window_position disabled, no window geometry
    // should be applied to the launch command.

    #[test]
    fn prop_disabled_position_memory(
        config in arb_freerdp_config().prop_filter(
            "Config must have remember_window_position disabled",
            |c| !c.remember_window_position
        )
    ) {
        let args = build_freerdp_args(&config);

        // Extract geometry from args - should be None
        let extracted = extract_geometry_from_args(&args);

        prop_assert!(
            extracted.is_none(),
            "Geometry should NOT be present in args when remember_window_position is false. Got: {:?}",
            args
        );

        // Also verify no /x: or /y: args are present
        let has_x = args.iter().any(|a| a.starts_with("/x:"));
        let has_y = args.iter().any(|a| a.starts_with("/y:"));

        prop_assert!(
            !has_x && !has_y,
            "No /x: or /y: args should be present when remember_window_position is false. Got: {:?}",
            args
        );
    }

    // Additional property: Server address is always last
    #[test]
    fn prop_server_address_is_last(config in arb_freerdp_config()) {
        let args = build_freerdp_args(&config);

        prop_assert!(
            !args.is_empty(),
            "Args should not be empty"
        );

        let last_arg = args.last().unwrap();
        prop_assert!(
            last_arg.starts_with("/v:"),
            "Last argument should be server address (/v:). Got: {}",
            last_arg
        );
    }

    // Additional property: Resolution is always present
    #[test]
    fn prop_resolution_always_present(config in arb_freerdp_config()) {
        let args = build_freerdp_args(&config);

        let has_width = args.iter().any(|a| a.starts_with("/w:"));
        let has_height = args.iter().any(|a| a.starts_with("/h:"));

        prop_assert!(
            has_width && has_height,
            "Resolution (/w: and /h:) should always be present. Got: {:?}",
            args
        );
    }

    // Additional property: Clipboard flag only when enabled
    #[test]
    fn prop_clipboard_flag_matches_config(config in arb_freerdp_config()) {
        let args = build_freerdp_args(&config);

        let has_clipboard = args.iter().any(|a| a == "+clipboard");

        prop_assert_eq!(
            has_clipboard,
            config.clipboard_enabled,
            "Clipboard flag should match config. Config: {}, Args: {:?}",
            config.clipboard_enabled,
            args
        );
    }

    // Additional property: Username only when set
    #[test]
    fn prop_username_only_when_set(config in arb_freerdp_config()) {
        let args = build_freerdp_args(&config);

        let has_username = args.iter().any(|a| a.starts_with("/u:"));

        prop_assert_eq!(
            has_username,
            config.username.is_some(),
            "Username flag should only be present when username is set. Config: {:?}, Args: {:?}",
            config.username,
            args
        );
    }

    // Additional property: Domain only when set and non-empty
    #[test]
    fn prop_domain_only_when_set(config in arb_freerdp_config()) {
        let args = build_freerdp_args(&config);

        let has_domain = args.iter().any(|a| a.starts_with("/d:"));
        let domain_is_set = config.domain.as_ref().is_some_and(|d| !d.is_empty());

        prop_assert_eq!(
            has_domain,
            domain_is_set,
            "Domain flag should only be present when domain is set and non-empty. Config: {:?}, Args: {:?}",
            config.domain,
            args
        );
    }

    // Additional property: Password uses /from-stdin, never /p:
    #[test]
    fn prop_password_only_when_set(config in arb_freerdp_config()) {
        let args = build_freerdp_args(&config);

        let has_from_stdin = args.iter().any(|a| a == "/from-stdin");
        let has_plain_password = args.iter().any(|a| a.starts_with("/p:"));
        let password_is_set =
            config.password.as_ref().is_some_and(|p| !p.is_empty());

        prop_assert!(
            !has_plain_password,
            "Password must never appear as /p: argument. Args: {:?}",
            args
        );
        prop_assert_eq!(
            has_from_stdin,
            password_is_set,
            "/from-stdin should be present iff password is set and non-empty. \
             Config: {:?}, Args: {:?}",
            config.password,
            args
        );
    }

    // Additional property: Custom port in server address
    #[test]
    fn prop_custom_port_in_server_address(config in arb_freerdp_config()) {
        let args = build_freerdp_args(&config);

        let server_arg = args.iter().find(|a| a.starts_with("/v:")).unwrap();

        if config.port == 3389 {
            // Default port - should not include port in address
            prop_assert!(
                !server_arg.contains(':') || server_arg.matches(':').count() == 1,
                "Default port should not be explicitly included. Got: {}",
                server_arg
            );
        } else {
            // Custom port - should include port in address
            let expected_suffix = format!(":{}", config.port);
            prop_assert!(
                server_arg.ends_with(&expected_suffix),
                "Custom port should be included in server address. Expected suffix: {}, Got: {}",
                expected_suffix,
                server_arg
            );
        }
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[test]
fn test_decorations_flag_basic() {
    let config = FreeRdpConfig::new("server.example.com");
    let args = build_freerdp_args(&config);

    assert!(
        has_decorations_flag(&args),
        "Decorations flag should be present"
    );
}

#[test]
fn test_geometry_with_remember_enabled() {
    let geometry = WindowGeometry::new(100, 200, 1920, 1080);
    let config = FreeRdpConfig::new("server.example.com")
        .with_window_geometry(geometry)
        .with_remember_window_position(true);
    let args = build_freerdp_args(&config);

    let extracted = extract_geometry_from_args(&args);
    assert_eq!(extracted, Some((100, 200)));
}

#[test]
fn test_geometry_with_remember_disabled() {
    let geometry = WindowGeometry::new(100, 200, 1920, 1080);
    let config = FreeRdpConfig::new("server.example.com")
        .with_window_geometry(geometry)
        .with_remember_window_position(false);
    let args = build_freerdp_args(&config);

    let extracted = extract_geometry_from_args(&args);
    assert_eq!(extracted, None);
}

#[test]
fn test_no_geometry_set() {
    let config = FreeRdpConfig::new("server.example.com").with_remember_window_position(true);
    let args = build_freerdp_args(&config);

    let extracted = extract_geometry_from_args(&args);
    assert_eq!(extracted, None);
}
