//! Drag-and-drop model for connection tree operations
//!
//! This module provides a pure data model for drag-and-drop operations,
//! allowing property-based testing without GTK dependencies.

/// Drop position relative to a target item
///
/// Determines where a dragged item will be placed relative to the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropPosition {
    /// Drop before the target item (line indicator above)
    Before,
    /// Drop after the target item (line indicator below)
    After,
    /// Drop into the target item (for groups/documents only)
    Into,
}

/// Type of item in the connection tree
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemType {
    /// A connection item
    Connection,
    /// A group/folder item
    Group,
    /// A document item
    Document,
}

/// Configuration for drop position calculation
#[derive(Debug, Clone, Copy)]
pub struct DropConfig {
    /// Height of each row in pixels
    pub row_height: f64,
    /// Ratio of row height for before/after zones (e.g., 0.25 = top/bottom 25%)
    pub drop_zone_ratio: f64,
}

impl Default for DropConfig {
    fn default() -> Self {
        Self {
            row_height: 32.0,
            drop_zone_ratio: 0.25,
        }
    }
}

/// Calculates the drop position based on Y coordinate within a row
///
/// For groups and documents, the row is divided into three zones:
/// - Top zone (`drop_zone_ratio`): Before
/// - Middle zone: Into
/// - Bottom zone (`drop_zone_ratio`): After
///
/// For connections, the row is divided into two zones:
/// - Top half: Before
/// - Bottom half: After
///
/// # Arguments
/// * `y_in_row` - Y coordinate relative to the top of the row
/// * `item_type` - Type of the target item
/// * `config` - Drop configuration
///
/// # Returns
/// The calculated drop position
#[must_use]
pub fn calculate_drop_position(
    y_in_row: f64,
    item_type: ItemType,
    config: &DropConfig,
) -> DropPosition {
    let drop_zone_size = config.row_height * config.drop_zone_ratio;

    match item_type {
        ItemType::Group | ItemType::Document => {
            // For groups/documents: top zone = before, middle = into, bottom = after
            if y_in_row < drop_zone_size {
                DropPosition::Before
            } else if y_in_row > config.row_height - drop_zone_size {
                DropPosition::After
            } else {
                DropPosition::Into
            }
        }
        ItemType::Connection => {
            // For connections: top half = before, bottom half = after
            if y_in_row < config.row_height / 2.0 {
                DropPosition::Before
            } else {
                DropPosition::After
            }
        }
    }
}

/// Calculates the Y position for the drop indicator line
///
/// The indicator should be positioned at the boundary between rows:
/// - For `Before`: at the top of the target row
/// - For `After`: at the bottom of the target row
/// - For `Into`: no line indicator (returns None)
///
/// # Arguments
/// * `row_index` - Index of the target row
/// * `position` - The drop position
/// * `config` - Drop configuration
///
/// # Returns
/// The Y coordinate for the indicator, or None if no line should be shown
#[must_use]
pub fn calculate_indicator_y(
    row_index: u32,
    position: DropPosition,
    config: &DropConfig,
) -> Option<f64> {
    match position {
        DropPosition::Before => Some(f64::from(row_index) * config.row_height),
        DropPosition::After => Some((f64::from(row_index) + 1.0) * config.row_height),
        DropPosition::Into => None, // No line indicator for drop-into
    }
}

/// Calculates which row index is at a given Y coordinate
///
/// # Arguments
/// * `y` - Y coordinate in the list view
/// * `config` - Drop configuration
///
/// # Returns
/// The row index at the given Y coordinate
#[must_use]
pub fn calculate_row_index(y: f64, config: &DropConfig) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let index = (y / config.row_height) as u32;
    index
}

/// Validates that a drop position is valid for the given item type
///
/// - Connections can only have Before or After positions
/// - Groups and Documents can have Before, After, or Into positions
///
/// # Arguments
/// * `position` - The drop position to validate
/// * `item_type` - Type of the target item
///
/// # Returns
/// True if the position is valid for the item type
#[must_use]
pub const fn is_valid_drop_position(position: DropPosition, item_type: ItemType) -> bool {
    match item_type {
        ItemType::Connection => !matches!(position, DropPosition::Into),
        ItemType::Group | ItemType::Document => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_only_before_or_after() {
        let config = DropConfig::default();

        // Top half should be Before
        let pos = calculate_drop_position(5.0, ItemType::Connection, &config);
        assert_eq!(pos, DropPosition::Before);

        // Bottom half should be After
        let pos = calculate_drop_position(20.0, ItemType::Connection, &config);
        assert_eq!(pos, DropPosition::After);
    }

    #[test]
    fn test_group_has_into_zone() {
        let config = DropConfig::default();

        // Middle should be Into
        let pos = calculate_drop_position(16.0, ItemType::Group, &config);
        assert_eq!(pos, DropPosition::Into);
    }

    #[test]
    fn test_indicator_y_before() {
        let config = DropConfig::default();
        let y = calculate_indicator_y(2, DropPosition::Before, &config);
        assert_eq!(y, Some(64.0)); // 2 * 32.0
    }

    #[test]
    fn test_indicator_y_after() {
        let config = DropConfig::default();
        let y = calculate_indicator_y(2, DropPosition::After, &config);
        assert_eq!(y, Some(96.0)); // 3 * 32.0
    }

    #[test]
    fn test_indicator_y_into_returns_none() {
        let config = DropConfig::default();
        let y = calculate_indicator_y(2, DropPosition::Into, &config);
        assert_eq!(y, None);
    }
}
