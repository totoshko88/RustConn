//! Tab group management for organizing terminal tabs
//!
//! This module provides the [`TabGroupManager`] which assigns colors to named
//! tab groups (e.g. "Production", "Staging", "Development"). Colors are drawn
//! from the existing [`ColorPool`] palette so that groups are visually distinct.

use std::collections::HashMap;

use super::color::SPLIT_COLORS;

/// Manages tab group assignments and their associated colors.
///
/// Each unique group name is assigned a color index from the split color
/// palette. The assignment is stable for the lifetime of the manager:
/// requesting the same group name always returns the same color index.
///
/// # Example
///
/// ```
/// use rustconn_core::split::TabGroupManager;
///
/// let mut mgr = TabGroupManager::new();
///
/// let idx1 = mgr.get_or_assign_color("Production");
/// let idx2 = mgr.get_or_assign_color("Staging");
/// assert_ne!(idx1, idx2);
///
/// // Same group always returns the same color
/// assert_eq!(idx1, mgr.get_or_assign_color("Production"));
/// ```
#[derive(Debug)]
pub struct TabGroupManager {
    /// Group name → color index mapping
    groups: HashMap<String, usize>,
    /// Next color index to assign (wraps around palette)
    next_index: usize,
}

impl TabGroupManager {
    /// Creates a new empty tab group manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            groups: HashMap::new(),
            next_index: 0,
        }
    }

    /// Returns the color index for a group, assigning one if this is the
    /// first time the group name is seen.
    pub fn get_or_assign_color(&mut self, group_name: &str) -> usize {
        if let Some(&idx) = self.groups.get(group_name) {
            return idx;
        }
        let idx = self.next_index % SPLIT_COLORS.len();
        self.next_index += 1;
        self.groups.insert(group_name.to_owned(), idx);
        idx
    }

    /// Returns the color index for a group if it already has one.
    #[must_use]
    pub fn get_color(&self, group_name: &str) -> Option<usize> {
        self.groups.get(group_name).copied()
    }

    /// Removes a group assignment, freeing its name for future reuse.
    ///
    /// Note: the color index is *not* recycled — a new assignment for the
    /// same name will receive the next sequential color.
    pub fn remove_group(&mut self, group_name: &str) {
        self.groups.remove(group_name);
    }

    /// Returns all currently registered group names.
    #[must_use]
    pub fn group_names(&self) -> Vec<String> {
        self.groups.keys().cloned().collect()
    }

    /// Returns the number of registered groups.
    #[must_use]
    pub fn group_count(&self) -> usize {
        self.groups.len()
    }

    /// Returns the RGB color tuple for a given color index.
    ///
    /// Returns `None` if the index is out of palette bounds.
    #[must_use]
    pub fn color_rgb(index: usize) -> Option<(u8, u8, u8)> {
        SPLIT_COLORS.get(index).copied()
    }
}

impl Default for TabGroupManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_manager_has_no_groups() {
        let mgr = TabGroupManager::new();
        assert_eq!(mgr.group_count(), 0);
        assert!(mgr.group_names().is_empty());
    }

    #[test]
    fn assign_returns_sequential_colors() {
        let mut mgr = TabGroupManager::new();
        assert_eq!(mgr.get_or_assign_color("A"), 0);
        assert_eq!(mgr.get_or_assign_color("B"), 1);
        assert_eq!(mgr.get_or_assign_color("C"), 2);
    }

    #[test]
    fn same_group_returns_same_color() {
        let mut mgr = TabGroupManager::new();
        let idx = mgr.get_or_assign_color("Production");
        assert_eq!(mgr.get_or_assign_color("Production"), idx);
        assert_eq!(mgr.group_count(), 1);
    }

    #[test]
    fn wraps_around_palette() {
        let mut mgr = TabGroupManager::new();
        let palette_len = SPLIT_COLORS.len();
        for i in 0..palette_len {
            assert_eq!(mgr.get_or_assign_color(&format!("G{i}")), i);
        }
        // Next group wraps to 0
        assert_eq!(mgr.get_or_assign_color("Overflow"), 0);
    }

    #[test]
    fn get_color_returns_none_for_unknown() {
        let mgr = TabGroupManager::new();
        assert_eq!(mgr.get_color("Unknown"), None);
    }

    #[test]
    fn remove_group_works() {
        let mut mgr = TabGroupManager::new();
        mgr.get_or_assign_color("Temp");
        assert_eq!(mgr.group_count(), 1);
        mgr.remove_group("Temp");
        assert_eq!(mgr.group_count(), 0);
        assert_eq!(mgr.get_color("Temp"), None);
    }

    #[test]
    fn color_rgb_returns_valid_colors() {
        assert!(TabGroupManager::color_rgb(0).is_some());
        assert!(TabGroupManager::color_rgb(SPLIT_COLORS.len()).is_none());
    }

    #[test]
    fn default_creates_empty_manager() {
        let mgr = TabGroupManager::default();
        assert_eq!(mgr.group_count(), 0);
    }
}
