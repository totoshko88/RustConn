//! Virtual scrolling state management for connection lists
//!
//! This module provides helper structures for managing selection state
//! across virtual scrolling operations, where items scroll in and out of view.

use std::collections::HashSet;
use uuid::Uuid;

/// Selection state manager for virtual scrolling
///
/// Tracks which items are selected independently of their visibility,
/// ensuring selections are preserved when items scroll in and out of view.
#[derive(Debug, Clone, Default)]
pub struct SelectionState {
    /// Set of selected item IDs
    selected_ids: HashSet<Uuid>,
}

impl SelectionState {
    /// Creates a new empty selection state
    #[must_use]
    pub fn new() -> Self {
        Self {
            selected_ids: HashSet::new(),
        }
    }

    /// Selects an item by ID
    pub fn select(&mut self, id: Uuid) {
        self.selected_ids.insert(id);
    }

    /// Deselects an item by ID
    pub fn deselect(&mut self, id: Uuid) {
        self.selected_ids.remove(&id);
    }

    /// Toggles selection state for an item
    pub fn toggle(&mut self, id: Uuid) {
        if self.selected_ids.contains(&id) {
            self.selected_ids.remove(&id);
        } else {
            self.selected_ids.insert(id);
        }
    }

    /// Checks if an item is selected
    #[must_use]
    pub fn is_selected(&self, id: Uuid) -> bool {
        self.selected_ids.contains(&id)
    }

    /// Returns all selected IDs
    #[must_use]
    pub const fn selected_ids(&self) -> &HashSet<Uuid> {
        &self.selected_ids
    }

    /// Returns the count of selected items
    #[must_use]
    pub fn selection_count(&self) -> usize {
        self.selected_ids.len()
    }

    /// Returns true if no items are selected
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.selected_ids.is_empty()
    }

    /// Clears all selections
    pub fn clear(&mut self) {
        self.selected_ids.clear();
    }

    /// Sets selection from a set of IDs
    pub fn set_selection(&mut self, ids: HashSet<Uuid>) {
        self.selected_ids = ids;
    }

    /// Selects multiple items at once
    pub fn select_many(&mut self, ids: impl IntoIterator<Item = Uuid>) {
        self.selected_ids.extend(ids);
    }

    /// Gets selected IDs that are in the given set of visible IDs
    #[must_use]
    pub fn visible_selections(&self, visible_ids: &HashSet<Uuid>) -> Vec<Uuid> {
        self.selected_ids
            .intersection(visible_ids)
            .copied()
            .collect()
    }

    /// Gets selected IDs that are NOT in the given set of visible IDs
    #[must_use]
    pub fn hidden_selections(&self, visible_ids: &HashSet<Uuid>) -> Vec<Uuid> {
        self.selected_ids.difference(visible_ids).copied().collect()
    }

    /// Returns an iterator over selected IDs
    pub fn iter(&self) -> impl Iterator<Item = &Uuid> {
        self.selected_ids.iter()
    }
}

/// Configuration for virtual scrolling behavior
#[derive(Debug, Clone)]
pub struct VirtualScrollConfig {
    /// Minimum number of items before enabling virtual scrolling
    pub threshold: usize,
    /// Number of items to render above/below visible area
    pub overscan: usize,
    /// Estimated height of each item in pixels
    pub item_height: f64,
}

impl Default for VirtualScrollConfig {
    fn default() -> Self {
        Self {
            threshold: 100,
            overscan: 5,
            item_height: 30.0,
        }
    }
}

impl VirtualScrollConfig {
    /// Creates a new configuration with custom threshold
    #[must_use]
    pub const fn with_threshold(mut self, threshold: usize) -> Self {
        self.threshold = threshold;
        self
    }

    /// Creates a new configuration with custom overscan
    #[must_use]
    pub const fn with_overscan(mut self, overscan: usize) -> Self {
        self.overscan = overscan;
        self
    }

    /// Creates a new configuration with custom item height
    #[must_use]
    pub const fn with_item_height(mut self, item_height: f64) -> Self {
        self.item_height = item_height;
        self
    }

    /// Returns whether virtual scrolling should be enabled for the given item count
    #[must_use]
    pub const fn should_enable(&self, item_count: usize) -> bool {
        item_count > self.threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selection_state_basic() {
        let mut state = SelectionState::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        assert!(state.is_empty());
        assert_eq!(state.selection_count(), 0);

        state.select(id1);
        assert!(state.is_selected(id1));
        assert!(!state.is_selected(id2));
        assert_eq!(state.selection_count(), 1);

        state.select(id2);
        assert_eq!(state.selection_count(), 2);

        state.deselect(id1);
        assert!(!state.is_selected(id1));
        assert!(state.is_selected(id2));
        assert_eq!(state.selection_count(), 1);
    }

    #[test]
    fn test_selection_state_toggle() {
        let mut state = SelectionState::new();
        let id = Uuid::new_v4();

        assert!(!state.is_selected(id));
        state.toggle(id);
        assert!(state.is_selected(id));
        state.toggle(id);
        assert!(!state.is_selected(id));
    }

    #[test]
    fn test_selection_state_clear() {
        let mut state = SelectionState::new();
        state.select(Uuid::new_v4());
        state.select(Uuid::new_v4());
        state.select(Uuid::new_v4());

        assert_eq!(state.selection_count(), 3);
        state.clear();
        assert!(state.is_empty());
    }

    #[test]
    fn test_selection_state_visible_hidden() {
        let mut state = SelectionState::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();
        let id4 = Uuid::new_v4();

        state.select(id1);
        state.select(id2);
        state.select(id3);

        let visible: HashSet<Uuid> = [id1, id4].into_iter().collect();

        let visible_selections = state.visible_selections(&visible);
        let hidden_selections = state.hidden_selections(&visible);

        assert_eq!(visible_selections.len(), 1);
        assert!(visible_selections.contains(&id1));

        assert_eq!(hidden_selections.len(), 2);
        assert!(hidden_selections.contains(&id2));
        assert!(hidden_selections.contains(&id3));
    }

    #[test]
    fn test_virtual_scroll_config() {
        let config = VirtualScrollConfig::default();
        assert_eq!(config.threshold, 100);
        assert_eq!(config.overscan, 5);

        assert!(!config.should_enable(50));
        assert!(!config.should_enable(100));
        assert!(config.should_enable(101));
        assert!(config.should_enable(500));

        let custom = VirtualScrollConfig::default()
            .with_threshold(50)
            .with_overscan(10)
            .with_item_height(40.0);

        assert_eq!(custom.threshold, 50);
        assert_eq!(custom.overscan, 10);
        assert!((custom.item_height - 40.0).abs() < f64::EPSILON);
    }
}
