//! Property-based tests for Multi-Selection operations
//!
//! **Feature: rustconn-enhancements, Property 1: Multi-Selection Consistency**
//! **Validates: Requirements 2.1, 2.2, 2.3, 2.6**
//!
//! Note: These tests verify the selection logic at the data model level.
//! The actual GTK4 SelectionModel integration is tested through the sidebar module.

use proptest::prelude::*;
use std::collections::HashSet;
use uuid::Uuid;

// ========== Selection State Model ==========

/// A simplified model of selection state for property testing
/// This mirrors the behavior of SelectionModelWrapper without GTK dependencies
#[derive(Debug, Clone)]
pub struct SelectionState {
    /// All available item IDs
    items: Vec<Uuid>,
    /// Currently selected item indices
    selected_indices: HashSet<usize>,
    /// Whether multi-selection mode is active
    multi_mode: bool,
}

impl SelectionState {
    /// Creates a new selection state with the given items
    pub fn new(items: Vec<Uuid>) -> Self {
        Self {
            items,
            selected_indices: HashSet::new(),
            multi_mode: false,
        }
    }

    /// Enables or disables multi-selection mode
    pub fn set_multi_mode(&mut self, enabled: bool) {
        self.multi_mode = enabled;
        if !enabled {
            // When switching to single mode, clear all selections
            self.selected_indices.clear();
        }
    }

    /// Returns whether multi-selection mode is active
    pub fn is_multi_mode(&self) -> bool {
        self.multi_mode
    }

    /// Selects an item at the given index
    pub fn select(&mut self, index: usize) {
        if index < self.items.len() {
            if self.multi_mode {
                self.selected_indices.insert(index);
            } else {
                // In single mode, replace selection
                self.selected_indices.clear();
                self.selected_indices.insert(index);
            }
        }
    }

    /// Deselects an item at the given index
    pub fn deselect(&mut self, index: usize) {
        self.selected_indices.remove(&index);
    }

    /// Toggles selection of an item at the given index
    pub fn toggle(&mut self, index: usize) {
        if index < self.items.len() {
            if self.selected_indices.contains(&index) {
                self.selected_indices.remove(&index);
            } else {
                self.select(index);
            }
        }
    }

    /// Selects all items (only works in multi-mode)
    pub fn select_all(&mut self) {
        if self.multi_mode {
            for i in 0..self.items.len() {
                self.selected_indices.insert(i);
            }
        }
    }

    /// Clears all selections
    pub fn clear_selection(&mut self) {
        self.selected_indices.clear();
    }

    /// Returns the selected item IDs
    pub fn get_selected_ids(&self) -> Vec<Uuid> {
        self.selected_indices
            .iter()
            .filter_map(|&idx| self.items.get(idx).copied())
            .collect()
    }

    /// Returns the selected indices
    pub fn get_selected_indices(&self) -> HashSet<usize> {
        self.selected_indices.clone()
    }

    /// Returns the number of selected items
    pub fn selection_count(&self) -> usize {
        self.selected_indices.len()
    }

    /// Returns the total number of items
    ///
    /// Reserved for future use in property tests that need to verify
    /// selection bounds against total item count.
    #[allow(dead_code)]
    pub fn item_count(&self) -> usize {
        self.items.len()
    }
}

// ========== Selection Operation for Property Testing ==========

/// Represents a selection operation
#[derive(Debug, Clone)]
pub enum SelectionOp {
    Select(usize),
    Deselect(usize),
    Toggle(usize),
    SelectAll,
    ClearSelection,
    SetMultiMode(bool),
}

// ========== Generators ==========

/// Strategy for generating a list of UUIDs
fn arb_uuid_list() -> impl Strategy<Value = Vec<Uuid>> {
    prop::collection::vec(any::<[u8; 16]>().prop_map(Uuid::from_bytes), 1..20)
}

/// Strategy for generating a selection operation
fn arb_selection_op(max_index: usize) -> impl Strategy<Value = SelectionOp> {
    prop_oneof![
        (0..max_index).prop_map(SelectionOp::Select),
        (0..max_index).prop_map(SelectionOp::Deselect),
        (0..max_index).prop_map(SelectionOp::Toggle),
        Just(SelectionOp::SelectAll),
        Just(SelectionOp::ClearSelection),
        any::<bool>().prop_map(SelectionOp::SetMultiMode),
    ]
}

/// Strategy for generating a sequence of selection operations
fn arb_selection_ops(max_index: usize) -> impl Strategy<Value = Vec<SelectionOp>> {
    prop::collection::vec(arb_selection_op(max_index), 1..50)
}

// ========== Property Tests ==========

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-enhancements, Property 1: Multi-Selection Consistency**
    /// **Validates: Requirements 2.1, 2.2, 2.3, 2.6**
    ///
    /// For any set of selection operations (select, deselect, select-all, clear),
    /// the set of selected IDs returned by get_selected_ids() must exactly match
    /// the items that should be selected based on the operations performed.
    #[test]
    fn selection_operations_are_consistent(
        items in arb_uuid_list(),
    ) {
        let max_idx = items.len().max(1);
        let ops = arb_selection_ops(max_idx);

        proptest!(|(operations in ops)| {
            let mut state = SelectionState::new(items.clone());

            // Apply all operations
            for op in &operations {
                match op {
                    SelectionOp::Select(idx) => state.select(*idx),
                    SelectionOp::Deselect(idx) => state.deselect(*idx),
                    SelectionOp::Toggle(idx) => state.toggle(*idx),
                    SelectionOp::SelectAll => state.select_all(),
                    SelectionOp::ClearSelection => state.clear_selection(),
                    SelectionOp::SetMultiMode(enabled) => state.set_multi_mode(*enabled),
                }
            }

            // Verify consistency: selected indices should all be valid
            let selected_indices = state.get_selected_indices();
            for idx in &selected_indices {
                prop_assert!(
                    *idx < items.len(),
                    "Selected index {} should be within bounds (0..{})",
                    idx, items.len()
                );
            }

            // Verify: selected IDs should match selected indices
            let selected_ids = state.get_selected_ids();
            prop_assert_eq!(
                selected_ids.len(),
                selected_indices.len(),
                "Number of selected IDs should match number of selected indices"
            );

            // Verify: each selected ID should correspond to a selected index
            for id in &selected_ids {
                let found = items.iter().position(|item| item == id);
                prop_assert!(
                    found.is_some() && selected_indices.contains(&found.unwrap()),
                    "Selected ID {:?} should correspond to a selected index",
                    id
                );
            }
        });
    }

    /// **Feature: rustconn-enhancements, Property 1: Multi-Selection Consistency**
    /// **Validates: Requirements 2.1, 2.6**
    ///
    /// When switching from multi-selection mode to single-selection mode,
    /// all selections should be cleared.
    #[test]
    fn switching_to_single_mode_clears_selection(
        items in arb_uuid_list(),
        selections in prop::collection::vec(0usize..20usize, 1..10),
    ) {
        let mut state = SelectionState::new(items.clone());

        // Enable multi-mode and make some selections
        state.set_multi_mode(true);
        for idx in &selections {
            if *idx < items.len() {
                state.select(*idx);
            }
        }

        // Switch to single mode
        state.set_multi_mode(false);

        // Verify all selections are cleared
        prop_assert_eq!(
            state.selection_count(),
            0,
            "Switching to single mode should clear all selections"
        );
        prop_assert!(
            state.get_selected_ids().is_empty(),
            "No IDs should be selected after switching to single mode"
        );
    }

    /// **Feature: rustconn-enhancements, Property 1: Multi-Selection Consistency**
    /// **Validates: Requirements 2.3**
    ///
    /// In multi-selection mode, select_all should select all items.
    #[test]
    fn select_all_selects_all_items_in_multi_mode(items in arb_uuid_list()) {
        let mut state = SelectionState::new(items.clone());

        // Enable multi-mode
        state.set_multi_mode(true);

        // Select all
        state.select_all();

        // Verify all items are selected
        prop_assert_eq!(
            state.selection_count(),
            items.len(),
            "All items should be selected after select_all"
        );

        let selected_ids = state.get_selected_ids();
        for item in &items {
            prop_assert!(
                selected_ids.contains(item),
                "Item {:?} should be selected after select_all",
                item
            );
        }
    }

    /// **Feature: rustconn-enhancements, Property 1: Multi-Selection Consistency**
    /// **Validates: Requirements 2.3**
    ///
    /// In single-selection mode, select_all should have no effect.
    #[test]
    fn select_all_has_no_effect_in_single_mode(items in arb_uuid_list()) {
        let mut state = SelectionState::new(items.clone());

        // Ensure single mode (default)
        prop_assert!(!state.is_multi_mode(), "Should start in single mode");

        // Try to select all
        state.select_all();

        // Verify no items are selected
        prop_assert_eq!(
            state.selection_count(),
            0,
            "select_all should have no effect in single mode"
        );
    }

    /// **Feature: rustconn-enhancements, Property 1: Multi-Selection Consistency**
    /// **Validates: Requirements 2.2**
    ///
    /// In single-selection mode, selecting a new item should deselect the previous one.
    #[test]
    fn single_mode_allows_only_one_selection(
        items in arb_uuid_list(),
        idx1 in 0usize..20usize,
        idx2 in 0usize..20usize,
    ) {
        prop_assume!(items.len() >= 2);
        let idx1 = idx1 % items.len();
        let idx2 = idx2 % items.len();
        prop_assume!(idx1 != idx2);

        let mut state = SelectionState::new(items.clone());

        // Ensure single mode
        state.set_multi_mode(false);

        // Select first item
        state.select(idx1);
        prop_assert_eq!(state.selection_count(), 1, "Should have one selection");

        // Select second item
        state.select(idx2);
        prop_assert_eq!(
            state.selection_count(),
            1,
            "Should still have only one selection in single mode"
        );

        // Verify only the second item is selected
        let selected = state.get_selected_indices();
        prop_assert!(
            selected.contains(&idx2) && !selected.contains(&idx1),
            "Only the most recently selected item should be selected"
        );
    }

    /// **Feature: rustconn-enhancements, Property 1: Multi-Selection Consistency**
    /// **Validates: Requirements 2.2**
    ///
    /// In multi-selection mode, selecting multiple items should keep all of them selected.
    #[test]
    fn multi_mode_allows_multiple_selections(
        items in arb_uuid_list(),
        selections in prop::collection::hash_set(0usize..20usize, 1..10),
    ) {
        let mut state = SelectionState::new(items.clone());

        // Enable multi-mode
        state.set_multi_mode(true);

        // Select multiple items
        let valid_selections: HashSet<usize> = selections
            .iter()
            .filter(|&&idx| idx < items.len())
            .copied()
            .collect();

        for idx in &valid_selections {
            state.select(*idx);
        }

        // Verify all selected items are still selected
        let selected = state.get_selected_indices();
        prop_assert_eq!(
            selected,
            valid_selections,
            "All selected items should remain selected in multi mode"
        );
    }

    /// **Feature: rustconn-enhancements, Property 1: Multi-Selection Consistency**
    /// **Validates: Requirements 2.5**
    ///
    /// Clear selection should remove all selections regardless of mode.
    #[test]
    fn clear_selection_removes_all_selections(
        items in arb_uuid_list(),
        multi_mode in any::<bool>(),
        selections in prop::collection::vec(0usize..20usize, 1..10),
    ) {
        let mut state = SelectionState::new(items.clone());

        // Set mode
        state.set_multi_mode(multi_mode);

        // Make some selections
        for idx in &selections {
            if *idx < items.len() {
                state.select(*idx);
            }
        }

        // Clear selection
        state.clear_selection();

        // Verify no items are selected
        prop_assert_eq!(
            state.selection_count(),
            0,
            "clear_selection should remove all selections"
        );
        prop_assert!(
            state.get_selected_ids().is_empty(),
            "No IDs should be selected after clear_selection"
        );
    }

    /// **Feature: rustconn-enhancements, Property 1: Multi-Selection Consistency**
    /// **Validates: Requirements 2.2**
    ///
    /// Toggle operation should correctly add or remove items from selection.
    #[test]
    fn toggle_correctly_adds_and_removes(
        items in arb_uuid_list(),
        idx in 0usize..20usize,
    ) {
        prop_assume!(!items.is_empty());
        let idx = idx % items.len();

        let mut state = SelectionState::new(items.clone());
        state.set_multi_mode(true);

        // Initially not selected
        prop_assert!(
            !state.get_selected_indices().contains(&idx),
            "Item should not be selected initially"
        );

        // Toggle to select
        state.toggle(idx);
        prop_assert!(
            state.get_selected_indices().contains(&idx),
            "Item should be selected after first toggle"
        );

        // Toggle to deselect
        state.toggle(idx);
        prop_assert!(
            !state.get_selected_indices().contains(&idx),
            "Item should not be selected after second toggle"
        );
    }
}
