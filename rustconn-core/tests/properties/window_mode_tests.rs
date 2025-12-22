//! Property-based tests for Window Mode and Geometry Persistence
//!
//! **Feature: rustconn-enhancements, Property 33: Window Geometry Persistence**
//! **Validates: Requirements 9.4**

use proptest::prelude::*;
use rustconn_core::models::{Connection, WindowGeometry, WindowMode};

// ========== Generators ==========

/// Strategy for generating valid window geometry
fn arb_window_geometry() -> impl Strategy<Value = WindowGeometry> {
    (
        -10000i32..10000i32, // x position
        -10000i32..10000i32, // y position
        100i32..4096i32,     // width (valid range)
        100i32..2160i32,     // height (valid range)
    )
        .prop_map(|(x, y, width, height)| WindowGeometry::new(x, y, width, height))
}

/// Strategy for generating optional window geometry
fn arb_optional_geometry() -> impl Strategy<Value = Option<WindowGeometry>> {
    prop_oneof![Just(None), arb_window_geometry().prop_map(Some),]
}

/// Strategy for generating window mode
fn arb_window_mode() -> impl Strategy<Value = WindowMode> {
    prop_oneof![
        Just(WindowMode::Embedded),
        Just(WindowMode::External),
        Just(WindowMode::Fullscreen),
    ]
}

/// Strategy for generating valid connection names
fn arb_name() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_-]{0,31}".prop_map(|s| s)
}

/// Strategy for generating valid hostnames
fn arb_host() -> impl Strategy<Value = String> {
    "[a-z0-9]([a-z0-9-]{0,15}[a-z0-9])?(\\.[a-z0-9]([a-z0-9-]{0,15}[a-z0-9])?)*".prop_map(|s| s)
}

/// Strategy for generating valid ports
fn arb_port() -> impl Strategy<Value = u16> {
    1u16..=65535u16
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-enhancements, Property 33: Window Geometry Persistence**
    /// **Validates: Requirements 9.4**
    ///
    /// For any external window with "remember position" enabled, closing and reopening
    /// should restore the same geometry (width and height).
    #[test]
    fn window_geometry_persistence_round_trip(
        name in arb_name(),
        host in arb_host(),
        port in arb_port(),
        geometry in arb_window_geometry(),
    ) {
        // Create a connection with External window mode and remember_window_position enabled
        let mut conn = Connection::new_ssh(name, host, port);
        conn.window_mode = WindowMode::External;
        conn.remember_window_position = true;

        // Simulate saving geometry when window closes
        conn.update_window_geometry(geometry.x, geometry.y, geometry.width, geometry.height);

        // Verify geometry was saved
        prop_assert!(conn.window_geometry.is_some(), "Geometry should be saved");

        let saved = conn.window_geometry.as_ref().unwrap();
        prop_assert_eq!(saved.x, geometry.x, "X position should be preserved");
        prop_assert_eq!(saved.y, geometry.y, "Y position should be preserved");
        prop_assert_eq!(saved.width, geometry.width, "Width should be preserved");
        prop_assert_eq!(saved.height, geometry.height, "Height should be preserved");
    }

    /// **Feature: rustconn-enhancements, Property 33: Window Geometry Persistence**
    /// **Validates: Requirements 9.4**
    ///
    /// When remember_window_position is disabled, geometry should not be saved.
    #[test]
    fn geometry_not_saved_when_disabled(
        name in arb_name(),
        host in arb_host(),
        port in arb_port(),
        geometry in arb_window_geometry(),
    ) {
        // Create a connection with remember_window_position disabled
        let mut conn = Connection::new_ssh(name, host, port);
        conn.window_mode = WindowMode::External;
        conn.remember_window_position = false;

        // Try to save geometry
        conn.update_window_geometry(geometry.x, geometry.y, geometry.width, geometry.height);

        // Verify geometry was NOT saved
        prop_assert!(conn.window_geometry.is_none(), "Geometry should not be saved when disabled");
    }

    /// **Feature: rustconn-enhancements, Property 33: Window Geometry Persistence**
    /// **Validates: Requirements 9.4**
    ///
    /// Window geometry serialization round-trip should preserve all values.
    #[test]
    fn geometry_serialization_round_trip(geometry in arb_window_geometry()) {
        // Serialize to JSON
        let json = serde_json::to_string(&geometry).expect("Should serialize");

        // Deserialize back
        let deserialized: WindowGeometry = serde_json::from_str(&json).expect("Should deserialize");

        // Verify all fields are preserved
        prop_assert_eq!(deserialized.x, geometry.x, "X should be preserved");
        prop_assert_eq!(deserialized.y, geometry.y, "Y should be preserved");
        prop_assert_eq!(deserialized.width, geometry.width, "Width should be preserved");
        prop_assert_eq!(deserialized.height, geometry.height, "Height should be preserved");
    }

    /// **Feature: rustconn-enhancements, Property 33: Window Geometry Persistence**
    /// **Validates: Requirements 9.4**
    ///
    /// Window mode serialization round-trip should preserve the mode.
    #[test]
    fn window_mode_serialization_round_trip(mode in arb_window_mode()) {
        // Serialize to JSON
        let json = serde_json::to_string(&mode).expect("Should serialize");

        // Deserialize back
        let deserialized: WindowMode = serde_json::from_str(&json).expect("Should deserialize");

        // Verify mode is preserved
        prop_assert_eq!(deserialized, mode, "Window mode should be preserved");
    }

    /// **Feature: rustconn-enhancements, Property 33: Window Geometry Persistence**
    /// **Validates: Requirements 9.4**
    ///
    /// Connection with window mode and geometry should serialize and deserialize correctly.
    #[test]
    fn connection_window_settings_round_trip(
        name in arb_name(),
        host in arb_host(),
        port in arb_port(),
        mode in arb_window_mode(),
        remember in any::<bool>(),
        geometry in arb_optional_geometry(),
    ) {
        // Create connection with window settings
        let mut conn = Connection::new_ssh(name, host, port);
        conn.window_mode = mode;
        conn.remember_window_position = remember;
        conn.window_geometry = geometry.clone();

        // Serialize to JSON
        let json = serde_json::to_string(&conn).expect("Should serialize connection");

        // Deserialize back
        let deserialized: Connection = serde_json::from_str(&json).expect("Should deserialize connection");

        // Verify window settings are preserved
        prop_assert_eq!(deserialized.window_mode, mode, "Window mode should be preserved");
        prop_assert_eq!(deserialized.remember_window_position, remember, "Remember position should be preserved");
        prop_assert_eq!(deserialized.window_geometry, geometry, "Window geometry should be preserved");
    }

    /// **Feature: rustconn-enhancements, Property 33: Window Geometry Persistence**
    /// **Validates: Requirements 9.4**
    ///
    /// WindowGeometry::is_valid should return true only for positive dimensions.
    #[test]
    fn geometry_validity_check(
        x in any::<i32>(),
        y in any::<i32>(),
        width in -100i32..4096i32,
        height in -100i32..2160i32,
    ) {
        let geometry = WindowGeometry::new(x, y, width, height);

        let expected_valid = width > 0 && height > 0;
        prop_assert_eq!(
            geometry.is_valid(),
            expected_valid,
            "is_valid should return true only when width > 0 and height > 0"
        );
    }

    /// **Feature: rustconn-enhancements, Property 33: Window Geometry Persistence**
    /// **Validates: Requirements 9.4**
    ///
    /// WindowMode index conversion should be consistent.
    #[test]
    fn window_mode_index_round_trip(mode in arb_window_mode()) {
        let index = mode.index();
        let from_index = WindowMode::from_index(index);

        prop_assert_eq!(from_index, mode, "Mode should be preserved through index conversion");
    }

    /// **Feature: rustconn-enhancements, Property 33: Window Geometry Persistence**
    /// **Validates: Requirements 9.4**
    ///
    /// Connection helper methods for window mode should work correctly.
    #[test]
    fn connection_window_mode_helpers(
        name in arb_name(),
        host in arb_host(),
        port in arb_port(),
        mode in arb_window_mode(),
    ) {
        let mut conn = Connection::new_ssh(name, host, port);
        conn.set_window_mode(mode);

        prop_assert_eq!(conn.get_window_mode(), mode, "get_window_mode should return set mode");

        match mode {
            WindowMode::External => {
                prop_assert!(conn.is_external_window(), "is_external_window should be true for External");
                prop_assert!(!conn.is_fullscreen(), "is_fullscreen should be false for External");
            }
            WindowMode::Fullscreen => {
                prop_assert!(!conn.is_external_window(), "is_external_window should be false for Fullscreen");
                prop_assert!(conn.is_fullscreen(), "is_fullscreen should be true for Fullscreen");
            }
            WindowMode::Embedded => {
                prop_assert!(!conn.is_external_window(), "is_external_window should be false for Embedded");
                prop_assert!(!conn.is_fullscreen(), "is_fullscreen should be false for Embedded");
            }
        }
    }
}
