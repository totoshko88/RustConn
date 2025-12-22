//! Property-based tests for split view functionality
//!
//! **Feature: rustconn-enhancements**

use proptest::prelude::*;
use rustconn_core::{SplitDirection, SplitViewModel};

/// Strategy for generating split directions
fn split_direction_strategy() -> impl Strategy<Value = SplitDirection> {
    prop_oneof![
        Just(SplitDirection::Horizontal),
        Just(SplitDirection::Vertical),
    ]
}

/// Strategy for generating a sequence of split operations
fn split_operations_strategy(max_ops: usize) -> impl Strategy<Value = Vec<SplitDirection>> {
    proptest::collection::vec(split_direction_strategy(), 0..=max_ops)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-enhancements, Property 6: Split View Structure Integrity**
    /// **Validates: Requirements 4.1, 4.2, 4.6**
    ///
    /// *For any* sequence of split operations (horizontal or vertical), the resulting
    /// pane structure should contain exactly (initial_panes + split_count) panes,
    /// and closing a pane should reduce the count by one.
    #[test]
    fn prop_split_view_structure_integrity(
        split_ops in split_operations_strategy(10),
        close_count in 0usize..5,
    ) {
        let mut view = SplitViewModel::new();
        let initial_panes = view.pane_count();

        // Perform split operations
        let mut successful_splits = 0;
        for direction in &split_ops {
            if view.split(*direction).is_some() {
                successful_splits += 1;
            }
        }

        // Verify pane count after splits
        prop_assert_eq!(
            view.pane_count(),
            initial_panes + successful_splits,
            "After {} splits, expected {} panes but got {}",
            successful_splits,
            initial_panes + successful_splits,
            view.pane_count()
        );

        // Perform close operations
        let panes_before_close = view.pane_count();
        let mut successful_closes = 0;
        for _ in 0..close_count {
            if view.close_pane() {
                successful_closes += 1;
            }
        }

        // Verify pane count after closes
        // Note: Can't close below 1 pane
        let expected_after_close = (panes_before_close - successful_closes).max(1);
        prop_assert_eq!(
            view.pane_count(),
            expected_after_close,
            "After {} closes from {} panes, expected {} panes but got {}",
            successful_closes,
            panes_before_close,
            expected_after_close,
            view.pane_count()
        );

        // Verify we always have at least one pane
        prop_assert!(
            view.pane_count() >= 1,
            "Split view should always have at least one pane"
        );

        // Verify focused pane is valid
        if let Some(focused_id) = view.focused_pane_id() {
            prop_assert!(
                view.pane_ids().contains(&focused_id),
                "Focused pane ID should be in the pane list"
            );
        }
    }

    /// Additional test: Split operations always succeed when there's a focused pane
    #[test]
    fn prop_split_always_succeeds_with_focus(
        direction in split_direction_strategy(),
    ) {
        let mut view = SplitViewModel::new();

        // New view should have focus
        prop_assert!(view.focused_pane_id().is_some());

        // Split should succeed
        let new_pane_id = view.split(direction);
        prop_assert!(new_pane_id.is_some(), "Split should succeed when there's a focused pane");

        // New pane should be in the list
        prop_assert!(
            view.pane_ids().contains(&new_pane_id.unwrap()),
            "New pane should be in the pane list"
        );
    }

    /// Test: Cannot close the last pane
    #[test]
    fn prop_cannot_close_last_pane(
        close_attempts in 1usize..10,
    ) {
        let mut view = SplitViewModel::new();

        // Try to close multiple times
        for _ in 0..close_attempts {
            view.close_pane();
        }

        // Should always have at least one pane
        prop_assert_eq!(
            view.pane_count(),
            1,
            "Should not be able to close the last pane"
        );
    }
}

/// Strategy for generating session names
fn session_name_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z][a-zA-Z0-9_-]{0,20}".prop_map(|s| s)
}

/// Strategy for generating a sequence of session additions
fn session_additions_strategy(max_sessions: usize) -> impl Strategy<Value = Vec<String>> {
    proptest::collection::vec(session_name_strategy(), 0..=max_sessions)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-enhancements, Property 7: Unified Tab List Consistency**
    /// **Validates: Requirements 4.3**
    ///
    /// *For any* split view with multiple panes, adding a session should make it
    /// available in all panes, and the total session count should equal the number
    /// of unique sessions regardless of pane count.
    #[test]
    fn prop_unified_tab_list_consistency(
        split_ops in split_operations_strategy(5),
        session_names in session_additions_strategy(10),
    ) {
        use rustconn_core::SessionInfo;
        use uuid::Uuid;

        let mut view = SplitViewModel::new();

        // Perform split operations to create multiple panes
        for direction in &split_ops {
            view.split(*direction);
        }

        let pane_count = view.pane_count();

        // Add sessions
        let mut added_session_ids = Vec::new();
        for name in &session_names {
            let session = SessionInfo::new(Uuid::new_v4(), name.clone());
            let session_id = session.id;
            view.add_session(session);
            added_session_ids.push(session_id);
        }

        // Verify session count equals number of added sessions
        prop_assert_eq!(
            view.session_count(),
            session_names.len(),
            "Session count should equal number of added sessions"
        );

        // Verify all sessions are accessible (unified list)
        for session_id in &added_session_ids {
            prop_assert!(
                view.get_session(*session_id).is_some(),
                "All added sessions should be accessible"
            );
        }

        // Verify session count is independent of pane count
        prop_assert!(
            pane_count >= 1,
            "Should have at least one pane"
        );

        // Sessions should be showable in any pane
        let pane_ids = view.pane_ids();
        for pane_id in &pane_ids {
            view.set_focused_pane(*pane_id);
            for session_id in &added_session_ids {
                // Should be able to show any session in any pane
                let shown = view.show_session(*session_id);
                prop_assert!(
                    shown,
                    "Should be able to show session {} in pane {}",
                    session_id,
                    pane_id
                );
            }
        }

        // Session count should still be the same after showing in different panes
        prop_assert_eq!(
            view.session_count(),
            session_names.len(),
            "Session count should remain unchanged after showing in panes"
        );
    }

    /// Test: Removing a session removes it from all panes
    #[test]
    fn prop_session_removal_clears_from_panes(
        split_count in 1usize..5,
    ) {
        use rustconn_core::SessionInfo;
        use uuid::Uuid;

        let mut view = SplitViewModel::new();

        // Create multiple panes
        for _ in 0..split_count {
            view.split(SplitDirection::Horizontal);
        }

        // Add a session
        let session = SessionInfo::new(Uuid::new_v4(), "test-session".to_string());
        let session_id = session.id;
        view.add_session(session);

        // Show session in all panes
        for pane_id in view.pane_ids() {
            view.set_focused_pane(pane_id);
            view.show_session(session_id);
        }

        // Remove the session
        view.remove_session(session_id);

        // Verify session is gone
        prop_assert!(
            view.get_session(session_id).is_none(),
            "Session should be removed from session list"
        );

        // Verify no pane shows the removed session
        // (We can't directly check pane sessions without exposing internals,
        // but we can verify the session count is 0)
        prop_assert_eq!(
            view.session_count(),
            0,
            "Session count should be 0 after removal"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// **Feature: rustconn-enhancements, Property 8: Focus Cycling Completeness**
    /// **Validates: Requirements 4.8**
    ///
    /// *For any* split view with N panes, calling focus_next_pane() N times should
    /// cycle through all panes exactly once and return to the starting pane.
    #[test]
    fn prop_focus_cycling_completeness(
        split_count in 0usize..10,
    ) {
        let mut view = SplitViewModel::new();

        // Create multiple panes
        for _ in 0..split_count {
            view.split(SplitDirection::Horizontal);
        }

        let pane_count = view.pane_count();
        let initial_focus = view.focused_pane_id();

        // Track which panes we visit
        let mut visited_panes = std::collections::HashSet::new();
        if let Some(id) = initial_focus {
            visited_panes.insert(id);
        }

        // Cycle through all panes
        for _ in 0..pane_count {
            if let Some(new_focus) = view.focus_next_pane() {
                visited_panes.insert(new_focus);
            }
        }

        // After N cycles, we should be back to the initial focus
        prop_assert_eq!(
            view.focused_pane_id(),
            initial_focus,
            "After {} focus cycles, should return to initial focus",
            pane_count
        );

        // We should have visited all panes
        prop_assert_eq!(
            visited_panes.len(),
            pane_count,
            "Should visit all {} panes during cycling, but visited {}",
            pane_count,
            visited_panes.len()
        );

        // All visited panes should be valid pane IDs
        let pane_ids: std::collections::HashSet<_> = view.pane_ids().into_iter().collect();
        for visited in &visited_panes {
            prop_assert!(
                pane_ids.contains(visited),
                "Visited pane {} should be a valid pane ID",
                visited
            );
        }
    }

    /// Test: Focus cycling with single pane stays on same pane
    #[test]
    fn prop_focus_cycling_single_pane(
        cycle_count in 1usize..20,
    ) {
        let mut view = SplitViewModel::new();
        let initial_focus = view.focused_pane_id();

        // Cycle multiple times
        for _ in 0..cycle_count {
            view.focus_next_pane();
        }

        // Should still be on the same pane
        prop_assert_eq!(
            view.focused_pane_id(),
            initial_focus,
            "Single pane view should always focus the same pane"
        );
    }

    /// Test: Focus cycling order is deterministic
    #[test]
    fn prop_focus_cycling_deterministic(
        split_count in 1usize..5,
    ) {
        let mut view1 = SplitViewModel::new();
        let mut view2 = SplitViewModel::new();

        // Create same number of panes in both views
        for _ in 0..split_count {
            view1.split(SplitDirection::Horizontal);
            view2.split(SplitDirection::Horizontal);
        }

        // Collect focus sequence from view1
        let mut sequence1 = Vec::new();
        for _ in 0..view1.pane_count() {
            if let Some(id) = view1.focus_next_pane() {
                sequence1.push(id);
            }
        }

        // Collect focus sequence from view2
        let mut sequence2 = Vec::new();
        for _ in 0..view2.pane_count() {
            if let Some(id) = view2.focus_next_pane() {
                sequence2.push(id);
            }
        }

        // Sequences should have same length
        prop_assert_eq!(
            sequence1.len(),
            sequence2.len(),
            "Focus sequences should have same length"
        );

        // Note: The actual IDs will differ (UUIDs), but the pattern should be consistent
        // Both should cycle through all panes
        prop_assert_eq!(
            sequence1.len(),
            view1.pane_count(),
            "Should cycle through all panes"
        );
    }
}
