//! Property tests for `TabGroupManager`

use proptest::prelude::*;
use rustconn_core::split::SPLIT_COLORS;
use rustconn_core::split::tab_groups::TabGroupManager;

proptest! {
    /// Assigning the same group name always returns the same color index.
    #[test]
    fn same_group_returns_stable_color(name in "[a-zA-Z][a-zA-Z0-9 ]{0,30}") {
        let mut mgr = TabGroupManager::new();
        let first = mgr.get_or_assign_color(&name);
        let second = mgr.get_or_assign_color(&name);
        prop_assert_eq!(first, second);
        prop_assert_eq!(mgr.group_count(), 1);
    }

    /// Different group names receive different color indices (until palette wraps).
    #[test]
    fn different_groups_get_different_colors(
        a in "[a-zA-Z][a-zA-Z0-9]{0,10}",
        b in "[a-zA-Z][a-zA-Z0-9]{0,10}",
    ) {
        prop_assume!(a != b);
        let mut mgr = TabGroupManager::new();
        let idx_a = mgr.get_or_assign_color(&a);
        let idx_b = mgr.get_or_assign_color(&b);
        // Only guaranteed distinct when palette hasn't wrapped
        if mgr.group_count() <= SPLIT_COLORS.len() {
            prop_assert_ne!(idx_a, idx_b);
        }
    }

    /// Color indices are always within palette bounds.
    #[test]
    fn color_index_within_palette(names in prop::collection::vec("[a-z]{1,8}", 1..50)) {
        let mut mgr = TabGroupManager::new();
        for name in &names {
            let idx = mgr.get_or_assign_color(name);
            prop_assert!(idx < SPLIT_COLORS.len(), "index {} >= palette len {}", idx, SPLIT_COLORS.len());
        }
    }

    /// `color_rgb` returns `Some` for valid indices and `None` for out-of-bounds.
    #[test]
    fn color_rgb_bounds(idx in 0_usize..=SPLIT_COLORS.len() + 5) {
        let result = TabGroupManager::color_rgb(idx);
        if idx < SPLIT_COLORS.len() {
            prop_assert!(result.is_some());
        } else {
            prop_assert!(result.is_none());
        }
    }

    /// Removing a group decreases the count and makes `get_color` return `None`.
    #[test]
    fn remove_group_clears_assignment(name in "[a-zA-Z]{1,15}") {
        let mut mgr = TabGroupManager::new();
        mgr.get_or_assign_color(&name);
        prop_assert_eq!(mgr.group_count(), 1);
        mgr.remove_group(&name);
        prop_assert_eq!(mgr.group_count(), 0);
        prop_assert!(mgr.get_color(&name).is_none());
    }

    /// `group_names` returns exactly the registered groups.
    #[test]
    fn group_names_reflects_state(
        names in prop::collection::hash_set("[a-z]{1,6}", 1..10)
    ) {
        let mut mgr = TabGroupManager::new();
        for name in &names {
            mgr.get_or_assign_color(name);
        }
        let mut returned: Vec<String> = mgr.group_names();
        returned.sort();
        let mut expected: Vec<String> = names.into_iter().collect();
        expected.sort();
        prop_assert_eq!(returned, expected);
    }
}

#[test]
fn default_creates_empty_manager() {
    let mgr = TabGroupManager::default();
    assert_eq!(mgr.group_count(), 0);
    assert!(mgr.group_names().is_empty());
}

#[test]
fn sequential_assignments_are_sequential() {
    let mut mgr = TabGroupManager::new();
    for i in 0..SPLIT_COLORS.len() {
        assert_eq!(mgr.get_or_assign_color(&format!("G{i}")), i);
    }
    // Wraps around
    assert_eq!(mgr.get_or_assign_color("Overflow"), 0);
}
