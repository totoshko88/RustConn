//! Property-based tests for drag-and-drop indicator positioning
//!
//! These tests verify that the drop indicator is always positioned correctly
//! as a horizontal line (before or after a row), never as a frame around it.

use proptest::prelude::*;
use rustconn_core::{
    calculate_drop_position, calculate_indicator_y, calculate_row_index, is_valid_drop_position,
    DropConfig, DropPosition, ItemType,
};

/// Strategy for generating item types
fn item_type_strategy() -> impl Strategy<Value = ItemType> {
    prop_oneof![
        Just(ItemType::Connection),
        Just(ItemType::Group),
        Just(ItemType::Document),
    ]
}

/// Strategy for generating drop configurations
fn drop_config_strategy() -> impl Strategy<Value = DropConfig> {
    (16.0..64.0f64, 0.1..0.4f64).prop_map(|(row_height, drop_zone_ratio)| DropConfig {
        row_height,
        drop_zone_ratio,
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-fixes-v2, Property 11: Drop Indicator Position**
    /// **Validates: Requirements 9.1, 9.2**
    ///
    /// For any drag operation over a connection row, the drop indicator
    /// should be positioned either above (Before) or below (After) the row,
    /// never as a frame around it (Into is not valid for connections).
    #[test]
    fn prop_connection_drop_position_is_before_or_after(
        y_ratio in 0.0..1.0f64,
        config in drop_config_strategy(),
    ) {
        let y_in_row = y_ratio * config.row_height;
        let position = calculate_drop_position(y_in_row, ItemType::Connection, &config);

        // Connections should only have Before or After positions
        prop_assert!(
            position == DropPosition::Before || position == DropPosition::After,
            "Connection drop position should be Before or After, got {:?}",
            position
        );

        // Verify the position is valid
        prop_assert!(
            is_valid_drop_position(position, ItemType::Connection),
            "Position {:?} should be valid for Connection",
            position
        );
    }

    /// **Feature: rustconn-fixes-v2, Property 11: Drop Indicator Position**
    /// **Validates: Requirements 9.1, 9.2**
    ///
    /// For any Before or After drop position, the indicator Y coordinate
    /// should be at a row boundary (not in the middle of a row).
    #[test]
    fn prop_indicator_y_is_at_row_boundary(
        row_index in 0u32..100,
        config in drop_config_strategy(),
    ) {
        // Test Before position
        let y_before = calculate_indicator_y(row_index, DropPosition::Before, &config);
        prop_assert!(y_before.is_some(), "Before position should have indicator Y");
        let y_before = y_before.unwrap();

        // Y should be at the top of the row (row_index * row_height)
        let expected_before = f64::from(row_index) * config.row_height;
        prop_assert!(
            (y_before - expected_before).abs() < 0.001,
            "Before indicator Y {} should equal row top {}",
            y_before,
            expected_before
        );

        // Test After position
        let y_after = calculate_indicator_y(row_index, DropPosition::After, &config);
        prop_assert!(y_after.is_some(), "After position should have indicator Y");
        let y_after = y_after.unwrap();

        // Y should be at the bottom of the row ((row_index + 1) * row_height)
        let expected_after = (f64::from(row_index) + 1.0) * config.row_height;
        prop_assert!(
            (y_after - expected_after).abs() < 0.001,
            "After indicator Y {} should equal row bottom {}",
            y_after,
            expected_after
        );
    }

    /// **Feature: rustconn-fixes-v2, Property 11: Drop Indicator Position**
    /// **Validates: Requirements 9.1, 9.2**
    ///
    /// For any Into drop position (groups/documents only), no line indicator
    /// should be shown (returns None).
    #[test]
    fn prop_into_position_has_no_line_indicator(
        row_index in 0u32..100,
        config in drop_config_strategy(),
    ) {
        let y = calculate_indicator_y(row_index, DropPosition::Into, &config);
        prop_assert!(
            y.is_none(),
            "Into position should not have a line indicator"
        );
    }

    /// **Feature: rustconn-fixes-v2, Property 11: Drop Indicator Position**
    /// **Validates: Requirements 9.1, 9.2**
    ///
    /// For any Y coordinate, the calculated row index should be consistent
    /// with the drop position calculation.
    #[test]
    fn prop_row_index_calculation_is_consistent(
        y in 0.0..3200.0f64, // Up to 100 rows with default height
        config in drop_config_strategy(),
    ) {
        let row_index = calculate_row_index(y, &config);

        // The row index should be the floor of y / row_height
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let expected_index = (y / config.row_height) as u32;

        prop_assert_eq!(
            row_index,
            expected_index,
            "Row index calculation should be consistent"
        );
    }

    /// **Feature: rustconn-fixes-v2, Property 11: Drop Indicator Position**
    /// **Validates: Requirements 9.1, 9.2**
    ///
    /// For any item type and position, the validity check should be consistent
    /// with the actual position calculation.
    #[test]
    fn prop_drop_position_validity_is_consistent(
        y_ratio in 0.0..1.0f64,
        item_type in item_type_strategy(),
        config in drop_config_strategy(),
    ) {
        let y_in_row = y_ratio * config.row_height;
        let position = calculate_drop_position(y_in_row, item_type, &config);

        // The calculated position should always be valid for the item type
        prop_assert!(
            is_valid_drop_position(position, item_type),
            "Calculated position {:?} should be valid for {:?}",
            position,
            item_type
        );
    }

    /// **Feature: rustconn-fixes-v2, Property 11: Drop Indicator Position**
    /// **Validates: Requirements 9.1, 9.2**
    ///
    /// For groups and documents, the middle zone should result in Into position,
    /// while the edges should result in Before or After.
    #[test]
    fn prop_group_has_three_zones(
        config in drop_config_strategy(),
    ) {
        let drop_zone_size = config.row_height * config.drop_zone_ratio;

        // Top zone (near 0) should be Before
        let top_y = drop_zone_size * 0.5;
        let top_pos = calculate_drop_position(top_y, ItemType::Group, &config);
        prop_assert_eq!(top_pos, DropPosition::Before, "Top zone should be Before");

        // Middle zone should be Into
        let middle_y = config.row_height / 2.0;
        let middle_pos = calculate_drop_position(middle_y, ItemType::Group, &config);
        prop_assert_eq!(middle_pos, DropPosition::Into, "Middle zone should be Into");

        // Bottom zone should be After
        let bottom_y = config.row_height - (drop_zone_size * 0.5);
        let bottom_pos = calculate_drop_position(bottom_y, ItemType::Group, &config);
        prop_assert_eq!(bottom_pos, DropPosition::After, "Bottom zone should be After");
    }
}
