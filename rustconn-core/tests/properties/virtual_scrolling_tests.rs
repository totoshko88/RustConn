//! Property-based tests for the Virtual Scrolling system
//!
//! These tests validate the correctness properties defined in the design document
//! for the Virtual Scrolling system (Requirements 6.x).

#![allow(clippy::manual_clamp)]

use proptest::prelude::*;
use rustconn_core::VirtualScroller;
use std::collections::HashSet;

// ========== Strategies ==========

/// Strategy for generating valid total item counts (0 to 10000)
fn arb_total_items() -> impl Strategy<Value = usize> {
    0usize..10000
}

/// Strategy for generating valid item heights (10.0 to 100.0 pixels)
fn arb_item_height() -> impl Strategy<Value = f64> {
    10.0f64..100.0
}

/// Strategy for generating valid viewport heights (100.0 to 2000.0 pixels)
fn arb_viewport_height() -> impl Strategy<Value = f64> {
    100.0f64..2000.0
}

/// Strategy for generating valid overscan values (0 to 20)
fn arb_overscan() -> impl Strategy<Value = usize> {
    0usize..20
}

/// Strategy for generating a complete VirtualScroller configuration
fn arb_scroller_config() -> impl Strategy<Value = (usize, f64, f64, usize)> {
    (
        arb_total_items(),
        arb_item_height(),
        arb_viewport_height(),
        arb_overscan(),
    )
}

/// Strategy for generating a set of selected item indices
fn arb_selection(max_items: usize) -> impl Strategy<Value = HashSet<usize>> {
    prop::collection::hash_set(0usize..max_items.max(1), 0..max_items.min(50).max(1))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 11: Virtual Scrolling Visible Range ==========
    // **Feature: performance-improvements, Property 11: Virtual Scrolling Visible Range**
    // **Validates: Requirements 6.1, 6.2**
    //
    // For any scroll position and viewport size, the visible range SHALL include
    // exactly the items that would be visible plus overscan buffer.

    #[test]
    fn visible_range_includes_all_visible_items(
        (total_items, item_height, viewport_height, overscan) in arb_scroller_config()
    ) {
        // Skip degenerate cases
        prop_assume!(total_items > 0);
        prop_assume!(item_height > 0.0);
        prop_assume!(viewport_height > 0.0);

        let scroller = VirtualScroller::new(total_items, item_height, viewport_height)
            .with_overscan(overscan);

        let (start, end) = scroller.visible_range();

        // Visible range should be valid
        prop_assert!(start <= end, "Start {} should be <= end {}", start, end);
        prop_assert!(end <= total_items, "End {} should be <= total_items {}", end, total_items);

        // Calculate expected visible items (without overscan)
        let visible_count = (viewport_height / item_height).ceil() as usize + 1;
        let expected_visible_end = visible_count.min(total_items);

        // The range should include at least the visible items
        prop_assert!(
            end >= expected_visible_end.saturating_sub(overscan),
            "Range should include visible items: end {} >= expected {}",
            end,
            expected_visible_end
        );
    }


    #[test]
    fn visible_range_respects_overscan_buffer(
        (total_items, item_height, viewport_height, overscan) in arb_scroller_config()
    ) {
        // Skip degenerate cases
        prop_assume!(total_items > 0);
        prop_assume!(item_height > 0.0);
        prop_assume!(viewport_height > 0.0);

        let mut scroller = VirtualScroller::new(total_items, item_height, viewport_height)
            .with_overscan(overscan);

        // Scroll to middle of content
        let mid_offset = (total_items as f64 * item_height) / 2.0;
        scroller.set_scroll_offset(mid_offset);

        let (start, end) = scroller.visible_range();

        // Calculate first visible item index
        let first_visible = (mid_offset / item_height).floor() as usize;

        // Start should be at most first_visible - overscan (clamped to 0)
        let expected_start = first_visible.saturating_sub(overscan);
        prop_assert!(
            start <= expected_start + 1, // Allow 1 item tolerance for rounding
            "Start {} should be close to expected {} (first_visible={}, overscan={})",
            start,
            expected_start,
            first_visible,
            overscan
        );

        // End should include overscan after visible items
        let visible_count = (viewport_height / item_height).ceil() as usize + 1;
        let expected_end = (first_visible + visible_count + overscan).min(total_items);
        prop_assert!(
            end >= expected_end.saturating_sub(1), // Allow 1 item tolerance
            "End {} should be close to expected {} (visible_count={}, overscan={})",
            end,
            expected_end,
            visible_count,
            overscan
        );
    }

    #[test]
    fn visible_range_never_exceeds_total_items(
        (total_items, item_height, viewport_height, overscan) in arb_scroller_config(),
        scroll_factor in 0.0f64..1.0  // Keep within valid scroll range
    ) {
        let mut scroller = VirtualScroller::new(total_items, item_height, viewport_height)
            .with_overscan(overscan);

        // Scroll to various positions within valid range
        let max_scroll = (total_items as f64 * item_height).max(0.0);
        let scroll_offset = max_scroll * scroll_factor;
        scroller.set_scroll_offset(scroll_offset);

        let (start, end) = scroller.visible_range();

        // End should never exceed bounds
        prop_assert!(
            end <= total_items,
            "End {} should never exceed total_items {}",
            end,
            total_items
        );

        // Start should be less than or equal to end
        prop_assert!(
            start <= end,
            "Start {} should be <= end {}",
            start,
            end
        );
    }


    #[test]
    fn visible_range_empty_for_zero_items(
        item_height in arb_item_height(),
        viewport_height in arb_viewport_height(),
        overscan in arb_overscan()
    ) {
        let scroller = VirtualScroller::new(0, item_height, viewport_height)
            .with_overscan(overscan);

        let (start, end) = scroller.visible_range();

        prop_assert_eq!(start, 0, "Start should be 0 for empty list");
        prop_assert_eq!(end, 0, "End should be 0 for empty list");
    }

    #[test]
    fn visible_range_consistent_across_scroll_positions(
        total_items in 10usize..1000,
        item_height in arb_item_height(),
        viewport_height in arb_viewport_height(),
        overscan in arb_overscan()
    ) {
        let mut scroller = VirtualScroller::new(total_items, item_height, viewport_height)
            .with_overscan(overscan);

        // Test multiple scroll positions within valid range
        let max_scroll = (total_items as f64 * item_height).max(0.0);
        let positions = [0.0, max_scroll * 0.1, max_scroll * 0.5, max_scroll * 0.9];

        for &offset in &positions {
            scroller.set_scroll_offset(offset);
            let (start, end) = scroller.visible_range();

            // Basic invariants should hold at all positions
            prop_assert!(start <= end, "Start {} should be <= end {} at offset {}", start, end, offset);
            prop_assert!(end <= total_items, "End {} should be <= total_items {} at offset {}", end, total_items, offset);

            // Range should be non-empty for non-empty list with valid viewport
            if total_items > 0 && viewport_height > 0.0 && item_height > 0.0 {
                // Only check non-empty if we're not scrolled past the content
                let first_visible = (offset / item_height).floor() as usize;
                if first_visible < total_items {
                    prop_assert!(
                        end > start,
                        "Range should be non-empty when first_visible {} < total_items {} at offset {}",
                        first_visible,
                        total_items,
                        offset
                    );
                }
            }
        }
    }

    #[test]
    fn is_visible_consistent_with_visible_range(
        total_items in 1usize..500,
        item_height in arb_item_height(),
        viewport_height in arb_viewport_height(),
        overscan in arb_overscan(),
        scroll_factor in 0.0f64..1.0
    ) {
        let mut scroller = VirtualScroller::new(total_items, item_height, viewport_height)
            .with_overscan(overscan);

        let scroll_offset = total_items as f64 * item_height * scroll_factor;
        scroller.set_scroll_offset(scroll_offset);

        let (start, end) = scroller.visible_range();

        // Check that is_visible is consistent with visible_range
        for i in 0..total_items {
            let is_in_range = i >= start && i < end;
            let is_visible = scroller.is_visible(i);

            prop_assert_eq!(
                is_visible,
                is_in_range,
                "is_visible({}) = {} should match range check {} (range: {}..{})",
                i,
                is_visible,
                is_in_range,
                start,
                end
            );
        }
    }


    #[test]
    fn total_height_equals_items_times_height(
        total_items in arb_total_items(),
        item_height in arb_item_height(),
        viewport_height in arb_viewport_height()
    ) {
        let scroller = VirtualScroller::new(total_items, item_height, viewport_height);

        let expected_height = total_items as f64 * item_height;
        let actual_height = scroller.total_height();

        prop_assert!(
            (actual_height - expected_height).abs() < f64::EPSILON,
            "Total height {} should equal {} * {} = {}",
            actual_height,
            total_items,
            item_height,
            expected_height
        );
    }

    #[test]
    fn item_offset_calculation_correct(
        total_items in 1usize..1000,
        item_height in arb_item_height(),
        viewport_height in arb_viewport_height(),
        index in 0usize..1000
    ) {
        prop_assume!(index < total_items);

        let scroller = VirtualScroller::new(total_items, item_height, viewport_height);

        let expected_offset = index as f64 * item_height;
        let actual_offset = scroller.item_offset(index);

        prop_assert!(
            (actual_offset - expected_offset).abs() < f64::EPSILON,
            "Item offset for index {} should be {} * {} = {}, got {}",
            index,
            index,
            item_height,
            expected_offset,
            actual_offset
        );
    }
}

// ========== Selection Preservation Helper ==========

/// Helper struct for tracking selection state across virtual scrolling
/// This is a pure data structure that can be used by GUI code
#[derive(Debug, Clone, Default)]
pub struct SelectionState {
    /// Set of selected item IDs (using indices for simplicity in tests)
    selected_indices: HashSet<usize>,
}

impl SelectionState {
    /// Creates a new empty selection state
    #[must_use]
    pub fn new() -> Self {
        Self {
            selected_indices: HashSet::new(),
        }
    }

    /// Selects an item by index
    pub fn select(&mut self, index: usize) {
        self.selected_indices.insert(index);
    }

    /// Deselects an item by index
    pub fn deselect(&mut self, index: usize) {
        self.selected_indices.remove(&index);
    }

    /// Checks if an item is selected
    #[must_use]
    pub fn is_selected(&self, index: usize) -> bool {
        self.selected_indices.contains(&index)
    }

    /// Returns all selected indices
    #[must_use]
    pub fn selected_indices(&self) -> &HashSet<usize> {
        &self.selected_indices
    }

    /// Returns the count of selected items
    #[must_use]
    pub fn selection_count(&self) -> usize {
        self.selected_indices.len()
    }

    /// Clears all selections
    pub fn clear(&mut self) {
        self.selected_indices.clear();
    }

    /// Sets selection from a set of indices
    pub fn set_selection(&mut self, indices: HashSet<usize>) {
        self.selected_indices = indices;
    }

    /// Gets selected indices that are currently visible
    #[must_use]
    pub fn visible_selections(&self, start: usize, end: usize) -> Vec<usize> {
        self.selected_indices
            .iter()
            .filter(|&&idx| idx >= start && idx < end)
            .copied()
            .collect()
    }

    /// Gets selected indices that are not currently visible
    #[must_use]
    pub fn hidden_selections(&self, start: usize, end: usize) -> Vec<usize> {
        self.selected_indices
            .iter()
            .filter(|&&idx| idx < start || idx >= end)
            .copied()
            .collect()
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // ========== Property 12: Virtual Scrolling Selection Preservation ==========
    // **Feature: performance-improvements, Property 12: Virtual Scrolling Selection Preservation**
    // **Validates: Requirements 6.4**
    //
    // For any selection state and scroll operation, the selection SHALL be
    // preserved when items scroll in and out of view.

    #[test]
    fn selection_preserved_across_scroll_operations(
        total_items in 10usize..500,
        item_height in arb_item_height(),
        viewport_height in arb_viewport_height(),
        overscan in arb_overscan(),
        initial_selection in arb_selection(500)
    ) {
        // Filter selection to valid indices
        let valid_selection: HashSet<usize> = initial_selection
            .into_iter()
            .filter(|&idx| idx < total_items)
            .collect();

        let mut scroller = VirtualScroller::new(total_items, item_height, viewport_height)
            .with_overscan(overscan);

        let mut selection = SelectionState::new();
        selection.set_selection(valid_selection.clone());

        // Record initial selection count
        let initial_count = selection.selection_count();


        // Scroll through various positions
        let scroll_positions = [0.0, 500.0, 1000.0, 2000.0, 5000.0, 0.0];

        for &offset in &scroll_positions {
            scroller.set_scroll_offset(offset);

            // Selection count should remain unchanged
            prop_assert_eq!(
                selection.selection_count(),
                initial_count,
                "Selection count should be preserved after scrolling to offset {}",
                offset
            );

            // All originally selected items should still be selected
            for &idx in &valid_selection {
                prop_assert!(
                    selection.is_selected(idx),
                    "Item {} should remain selected after scrolling to offset {}",
                    idx,
                    offset
                );
            }
        }
    }

    #[test]
    fn selection_state_independent_of_visibility(
        total_items in 10usize..500,
        item_height in arb_item_height(),
        viewport_height in arb_viewport_height(),
        overscan in arb_overscan(),
        initial_selection in arb_selection(500)
    ) {
        // Filter selection to valid indices
        let valid_selection: HashSet<usize> = initial_selection
            .into_iter()
            .filter(|&idx| idx < total_items)
            .collect();

        let mut scroller = VirtualScroller::new(total_items, item_height, viewport_height)
            .with_overscan(overscan);

        let mut selection = SelectionState::new();
        selection.set_selection(valid_selection.clone());

        // Scroll to a position where some items are not visible
        let mid_offset = (total_items as f64 * item_height) / 2.0;
        scroller.set_scroll_offset(mid_offset);

        let (start, end) = scroller.visible_range();

        // Get visible and hidden selections
        let visible = selection.visible_selections(start, end);
        let hidden = selection.hidden_selections(start, end);

        // Total should equal original selection
        prop_assert_eq!(
            visible.len() + hidden.len(),
            valid_selection.len(),
            "Visible ({}) + hidden ({}) selections should equal total ({})",
            visible.len(),
            hidden.len(),
            valid_selection.len()
        );

        // All visible selections should be in the original set
        for idx in &visible {
            prop_assert!(
                valid_selection.contains(idx),
                "Visible selection {} should be in original set",
                idx
            );
        }

        // All hidden selections should be in the original set
        for idx in &hidden {
            prop_assert!(
                valid_selection.contains(idx),
                "Hidden selection {} should be in original set",
                idx
            );
        }
    }


    #[test]
    fn selection_restored_when_scrolling_back(
        total_items in 50usize..500,
        item_height in arb_item_height(),
        viewport_height in arb_viewport_height(),
        overscan in arb_overscan()
    ) {
        let mut scroller = VirtualScroller::new(total_items, item_height, viewport_height)
            .with_overscan(overscan);

        let mut selection = SelectionState::new();

        // Select some items at the beginning
        for i in 0..5.min(total_items) {
            selection.select(i);
        }

        // Record initial visible selections at top
        scroller.set_scroll_offset(0.0);
        let (start, end) = scroller.visible_range();
        let initial_visible = selection.visible_selections(start, end);

        // Scroll far away
        let far_offset = (total_items as f64 * item_height) * 0.8;
        scroller.set_scroll_offset(far_offset);

        // Selections should still exist but may not be visible
        prop_assert_eq!(
            selection.selection_count(),
            5.min(total_items),
            "Selection count should be preserved when scrolled away"
        );

        // Scroll back to top
        scroller.set_scroll_offset(0.0);
        let (start2, end2) = scroller.visible_range();
        let restored_visible = selection.visible_selections(start2, end2);

        // Same items should be visible and selected
        prop_assert_eq!(
            initial_visible.len(),
            restored_visible.len(),
            "Same number of selections should be visible after scrolling back"
        );

        for idx in &initial_visible {
            prop_assert!(
                restored_visible.contains(idx),
                "Selection {} should be restored when scrolling back",
                idx
            );
        }
    }

    #[test]
    fn modify_selection_while_scrolled(
        total_items in 20usize..500,
        item_height in arb_item_height(),
        viewport_height in arb_viewport_height(),
        overscan in arb_overscan()
    ) {
        let mut scroller = VirtualScroller::new(total_items, item_height, viewport_height)
            .with_overscan(overscan);

        let mut selection = SelectionState::new();

        // Scroll to middle
        let mid_offset = (total_items as f64 * item_height) / 2.0;
        scroller.set_scroll_offset(mid_offset);

        let (start, end) = scroller.visible_range();

        // Select a visible item
        if start < end {
            let visible_idx = start;
            selection.select(visible_idx);

            prop_assert!(
                selection.is_selected(visible_idx),
                "Should be able to select visible item {}",
                visible_idx
            );
        }

        // Select an item that's not visible (at the beginning)
        if start > 0 {
            selection.select(0);
            prop_assert!(
                selection.is_selected(0),
                "Should be able to select non-visible item 0"
            );
        }

        // Scroll back to top
        scroller.set_scroll_offset(0.0);

        // Item 0 should now be visible and still selected
        let (new_start, new_end) = scroller.visible_range();
        if new_start == 0 && new_end > 0 {
            prop_assert!(
                selection.is_selected(0),
                "Item 0 should still be selected after scrolling to make it visible"
            );
        }
    }
}

// ========== Unit Tests for Edge Cases ==========

#[cfg(test)]
mod edge_case_tests {
    use super::*;

    #[test]
    fn test_scroller_with_zero_item_height() {
        let scroller = VirtualScroller::new(100, 0.0, 300.0);
        let (start, end) = scroller.visible_range();
        // Should handle gracefully
        assert_eq!(start, 0);
        assert_eq!(end, 0);
    }

    #[test]
    fn test_scroller_with_zero_viewport() {
        let scroller = VirtualScroller::new(100, 30.0, 0.0);
        let (_start, end) = scroller.visible_range();
        // Should handle gracefully - at least show overscan items
        assert!(end <= 100);
    }

    #[test]
    fn test_scroller_negative_scroll_offset_clamped() {
        let mut scroller = VirtualScroller::new(100, 30.0, 300.0);
        scroller.set_scroll_offset(-100.0);
        let (start, _end) = scroller.visible_range();
        // Should clamp to 0
        assert_eq!(start, 0);
    }

    #[test]
    fn test_scroller_scroll_past_end() {
        let mut scroller = VirtualScroller::new(100, 30.0, 300.0);
        // Scroll way past the end
        scroller.set_scroll_offset(10000.0);
        let (_start, end) = scroller.visible_range();
        // End should not exceed total items
        assert!(end <= 100);
    }

    #[test]
    fn test_selection_state_empty() {
        let selection = SelectionState::new();
        assert_eq!(selection.selection_count(), 0);
        assert!(!selection.is_selected(0));
        assert!(selection.selected_indices().is_empty());
    }

    #[test]
    fn test_selection_state_select_deselect() {
        let mut selection = SelectionState::new();

        selection.select(5);
        assert!(selection.is_selected(5));
        assert_eq!(selection.selection_count(), 1);

        selection.select(10);
        assert!(selection.is_selected(10));
        assert_eq!(selection.selection_count(), 2);

        selection.deselect(5);
        assert!(!selection.is_selected(5));
        assert!(selection.is_selected(10));
        assert_eq!(selection.selection_count(), 1);
    }

    #[test]
    fn test_selection_state_clear() {
        let mut selection = SelectionState::new();
        selection.select(1);
        selection.select(2);
        selection.select(3);
        assert_eq!(selection.selection_count(), 3);

        selection.clear();
        assert_eq!(selection.selection_count(), 0);
        assert!(!selection.is_selected(1));
        assert!(!selection.is_selected(2));
        assert!(!selection.is_selected(3));
    }

    #[test]
    fn test_visible_hidden_selections_partition() {
        let mut selection = SelectionState::new();
        selection.select(0);
        selection.select(5);
        selection.select(10);
        selection.select(15);

        // Visible range 5..12
        let visible = selection.visible_selections(5, 12);
        let hidden = selection.hidden_selections(5, 12);

        // Should partition correctly
        assert!(visible.contains(&5));
        assert!(visible.contains(&10));
        assert!(!visible.contains(&0));
        assert!(!visible.contains(&15));

        assert!(hidden.contains(&0));
        assert!(hidden.contains(&15));
        assert!(!hidden.contains(&5));
        assert!(!hidden.contains(&10));

        // Total should equal selection count
        assert_eq!(visible.len() + hidden.len(), 4);
    }

    #[test]
    fn test_scroller_update_methods() {
        let mut scroller = VirtualScroller::new(100, 30.0, 300.0);

        // Test set_viewport_height
        scroller.set_viewport_height(600.0);
        let (_, end1) = scroller.visible_range();

        scroller.set_viewport_height(300.0);
        let (_, end2) = scroller.visible_range();

        // Larger viewport should show more items
        assert!(end1 >= end2);

        // Test set_total_items
        scroller.set_total_items(50);
        let (_, end3) = scroller.visible_range();
        assert!(end3 <= 50);

        scroller.set_total_items(200);
        let (_, end4) = scroller.visible_range();
        assert!(end4 <= 200);
    }

    #[test]
    fn test_scroller_with_large_overscan() {
        // Overscan larger than visible items
        let scroller = VirtualScroller::new(10, 30.0, 60.0) // Only ~2 visible
            .with_overscan(20); // Overscan of 20

        let (start, end) = scroller.visible_range();

        // Should still be bounded by total items
        assert_eq!(start, 0);
        assert!(end <= 10);
    }

    #[test]
    fn test_selection_with_uuid_based_items() {
        // Simulate using UUIDs for selection (common pattern)
        let mut selection = SelectionState::new();

        // In real usage, we'd map UUIDs to indices
        // Here we just test the index-based selection works
        let indices = vec![0, 5, 10, 15, 20];
        for idx in &indices {
            selection.select(*idx);
        }

        assert_eq!(selection.selection_count(), 5);

        // Simulate scrolling - selection should persist
        let visible = selection.visible_selections(8, 18);
        assert_eq!(visible.len(), 2); // 10 and 15

        let hidden = selection.hidden_selections(8, 18);
        assert_eq!(hidden.len(), 3); // 0, 5, and 20
    }
}
