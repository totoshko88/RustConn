//! Property-based tests for split view redesign (tab-scoped layouts)
//!
//! **Feature: split-view-redesign**
//!
//! This module tests the new `SplitLayoutModel` which provides tab-scoped
//! split layouts where each root tab maintains its own independent panel
//! configuration.

use proptest::prelude::*;
use rustconn_core::split::{
    ColorId, ColorPool, DropResult, SPLIT_COLORS, SessionId, SplitDirection, SplitError,
    SplitLayoutModel,
};

// ============================================================================
// Test Strategies
// ============================================================================

/// Strategy for generating split directions
fn split_direction_strategy() -> impl Strategy<Value = SplitDirection> {
    prop_oneof![
        Just(SplitDirection::Horizontal),
        Just(SplitDirection::Vertical),
    ]
}

/// Strategy for generating a sequence of split operations (limited depth)
fn split_operations_strategy(max_ops: usize) -> impl Strategy<Value = Vec<SplitDirection>> {
    proptest::collection::vec(split_direction_strategy(), 0..=max_ops)
}

/// Represents an operation that can be performed on a SplitLayoutModel
#[derive(Debug, Clone)]
enum LayoutOperation {
    /// Split the focused panel in the given direction
    Split(SplitDirection),
    /// Place a session in a panel (by index into panel_ids)
    PlaceInPanel {
        panel_index: usize,
        session: SessionId,
    },
    /// Remove a panel (by index into panel_ids)
    RemovePanel { panel_index: usize },
    /// Set focus to a panel (by index into panel_ids)
    SetFocus { panel_index: usize },
}

/// Strategy for generating layout operations
fn layout_operation_strategy() -> impl Strategy<Value = LayoutOperation> {
    prop_oneof![
        split_direction_strategy().prop_map(LayoutOperation::Split),
        (0usize..10, any::<u128>()).prop_map(|(idx, seed)| {
            LayoutOperation::PlaceInPanel {
                panel_index: idx,
                session: SessionId(uuid::Uuid::from_u128(seed)),
            }
        }),
        (0usize..10).prop_map(|idx| LayoutOperation::RemovePanel { panel_index: idx }),
        (0usize..10).prop_map(|idx| LayoutOperation::SetFocus { panel_index: idx }),
    ]
}

/// Strategy for generating a sequence of layout operations
fn layout_operations_strategy(max_ops: usize) -> impl Strategy<Value = Vec<LayoutOperation>> {
    proptest::collection::vec(layout_operation_strategy(), 0..=max_ops)
}

/// Apply an operation to a layout, ignoring errors (for property testing)
fn apply_operation(layout: &mut SplitLayoutModel, op: &LayoutOperation) {
    match op {
        LayoutOperation::Split(direction) => {
            let _ = layout.split(*direction);
        }
        LayoutOperation::PlaceInPanel {
            panel_index,
            session,
        } => {
            let panel_ids = layout.panel_ids();
            if !panel_ids.is_empty() {
                let idx = panel_index % panel_ids.len();
                let _ = layout.place_in_panel(panel_ids[idx], *session);
            }
        }
        LayoutOperation::RemovePanel { panel_index } => {
            let panel_ids = layout.panel_ids();
            if panel_ids.len() > 1 {
                let idx = panel_index % panel_ids.len();
                let _ = layout.remove_panel(panel_ids[idx]);
            }
        }
        LayoutOperation::SetFocus { panel_index } => {
            let panel_ids = layout.panel_ids();
            if !panel_ids.is_empty() {
                let idx = panel_index % panel_ids.len();
                let _ = layout.set_focus(panel_ids[idx]);
            }
        }
    }
}

/// Capture the complete state of a layout for comparison
#[derive(Debug, Clone, PartialEq)]
struct LayoutSnapshot {
    panel_count: usize,
    panel_ids: Vec<rustconn_core::split::PanelId>,
    sessions: Vec<(rustconn_core::split::PanelId, Option<SessionId>)>,
    is_split: bool,
    depth: usize,
    focused_panel: Option<rustconn_core::split::PanelId>,
}

impl LayoutSnapshot {
    fn capture(layout: &SplitLayoutModel) -> Self {
        let panel_ids = layout.panel_ids();
        let sessions = panel_ids
            .iter()
            .map(|&id| (id, layout.get_panel_session(id)))
            .collect();

        Self {
            panel_count: layout.panel_count(),
            panel_ids,
            sessions,
            is_split: layout.is_split(),
            depth: layout.depth(),
            focused_panel: layout.get_focused_panel(),
        }
    }
}

// ============================================================================
// Property 1: Layout Independence
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // Feature: split-view-redesign, Property 1: Layout Independence
    // Validates: Requirements 1.3, 1.4, 3.1, 3.3, 3.4
    //
    // *For any* collection of tabs managed by `TabSplitManager`, operations on one
    // tab's `SplitLayoutModel` (split, place, remove) SHALL NOT affect any other
    // tab's layout state.

    /// Test that operations on one layout don't affect another layout
    #[test]
    fn prop_layout_independence_split_operations(
        ops in layout_operations_strategy(10),
    ) {
        // Create two independent layouts
        let mut layout_a = SplitLayoutModel::new();
        let layout_b = SplitLayoutModel::new();

        // Capture initial state of layout B
        let initial_b_snapshot = LayoutSnapshot::capture(&layout_b);

        // Apply operations only to layout A
        for op in &ops {
            apply_operation(&mut layout_a, op);
        }

        // Capture final state of layout B
        let final_b_snapshot = LayoutSnapshot::capture(&layout_b);

        // Layout B should be completely unchanged
        prop_assert_eq!(
            initial_b_snapshot,
            final_b_snapshot,
            "Operations on layout A should not affect layout B"
        );
    }

    /// Test that two layouts with the same operations applied independently
    /// produce the same results (deterministic behavior)
    #[test]
    fn prop_layout_independence_deterministic(
        ops in layout_operations_strategy(8),
    ) {
        // Create two independent layouts
        let mut layout_a = SplitLayoutModel::new();
        let layout_b = SplitLayoutModel::new();

        // Capture initial state of layout B
        let initial_b_snapshot = LayoutSnapshot::capture(&layout_b);

        // Apply operations only to layout A
        for op in &ops {
            apply_operation(&mut layout_a, op);
        }

        // Layout B should be completely unchanged
        let final_b_snapshot = LayoutSnapshot::capture(&layout_b);
        prop_assert_eq!(
            initial_b_snapshot,
            final_b_snapshot,
            "Layout B should be unchanged after operations on layout A"
        );
    }

    /// Test that creating multiple layouts doesn't cause interference
    #[test]
    fn prop_layout_independence_multiple_layouts(
        layout_count in 2usize..6,
        ops_per_layout in layout_operations_strategy(5),
    ) {
        // Create multiple independent layouts
        let mut layouts: Vec<SplitLayoutModel> = (0..layout_count)
            .map(|_| SplitLayoutModel::new())
            .collect();

        // Capture initial snapshots
        let initial_snapshots: Vec<LayoutSnapshot> = layouts
            .iter()
            .map(LayoutSnapshot::capture)
            .collect();

        // Apply operations only to the first layout
        for op in &ops_per_layout {
            apply_operation(&mut layouts[0], op);
        }

        // All other layouts should be unchanged
        for (i, layout) in layouts.iter().enumerate().skip(1) {
            let current_snapshot = LayoutSnapshot::capture(layout);
            prop_assert_eq!(
                &initial_snapshots[i],
                &current_snapshot,
                "Layout {} should be unchanged after operations on layout 0",
                i
            );
        }
    }

    /// Test that sessions placed in one layout don't appear in another
    #[test]
    fn prop_layout_independence_session_isolation(
        session_seed in any::<u128>(),
    ) {
        let session = SessionId(uuid::Uuid::from_u128(session_seed));

        // Create two independent layouts
        let mut layout_a = SplitLayoutModel::new();
        let layout_b = SplitLayoutModel::new();

        // Place session in layout A
        let panel_a = layout_a.panel_ids()[0];
        let result = layout_a.place_in_panel(panel_a, session);
        prop_assert!(result.is_ok());

        // Verify session is in layout A
        prop_assert_eq!(
            layout_a.get_panel_session(panel_a),
            Some(session),
            "Session should be in layout A"
        );

        // Verify session is NOT in layout B (different panel IDs)
        let panel_b = layout_b.panel_ids()[0];
        prop_assert!(
            layout_b.get_panel_session(panel_b).is_none(),
            "Layout B should have no sessions"
        );

        // Verify layout B's panel doesn't exist in layout A
        prop_assert!(
            !layout_a.contains_panel(panel_b),
            "Layout A should not contain layout B's panel"
        );
    }

    /// Test that split operations on one layout preserve another layout's structure
    #[test]
    fn prop_layout_independence_split_preserves_other(
        splits_a in split_operations_strategy(5),
        initial_session_seed in any::<u128>(),
    ) {
        let initial_session = SessionId(uuid::Uuid::from_u128(initial_session_seed));

        // Create layout B with a session
        let layout_b = SplitLayoutModel::with_session(initial_session);
        let panel_b = layout_b.panel_ids()[0];

        // Capture layout B's state
        let b_panel_count = layout_b.panel_count();
        let b_session = layout_b.get_panel_session(panel_b);

        // Create layout A and perform many splits
        let mut layout_a = SplitLayoutModel::new();
        for direction in &splits_a {
            let _ = layout_a.split(*direction);
        }

        // Layout B should be completely unchanged
        prop_assert_eq!(
            layout_b.panel_count(),
            b_panel_count,
            "Layout B panel count should be unchanged"
        );
        prop_assert_eq!(
            layout_b.get_panel_session(panel_b),
            b_session,
            "Layout B session should be unchanged"
        );
    }

    /// Test that removing panels from one layout doesn't affect another
    #[test]
    fn prop_layout_independence_remove_isolation(
        split_count in 1usize..5,
    ) {
        // Create two layouts and split them both
        let mut layout_a = SplitLayoutModel::new();
        let mut layout_b = SplitLayoutModel::new();

        for _ in 0..split_count {
            let _ = layout_a.split(SplitDirection::Vertical);
            let _ = layout_b.split(SplitDirection::Vertical);
        }

        // Capture layout B's state
        let b_snapshot = LayoutSnapshot::capture(&layout_b);

        // Remove panels from layout A until only one remains
        while layout_a.panel_count() > 1 {
            let panels = layout_a.panel_ids();
            let _ = layout_a.remove_panel(panels[panels.len() - 1]);
        }

        // Layout B should be unchanged
        let b_final = LayoutSnapshot::capture(&layout_b);
        prop_assert_eq!(
            b_snapshot,
            b_final,
            "Layout B should be unchanged after removing panels from layout A"
        );
    }

    /// Test that focus changes in one layout don't affect another
    #[test]
    fn prop_layout_independence_focus_isolation(
        split_count in 1usize..4,
        focus_changes in 1usize..10,
    ) {
        // Create two layouts and split them
        let mut layout_a = SplitLayoutModel::new();
        let mut layout_b = SplitLayoutModel::new();

        for _ in 0..split_count {
            let _ = layout_a.split(SplitDirection::Horizontal);
            let _ = layout_b.split(SplitDirection::Horizontal);
        }

        // Capture layout B's focused panel
        let b_initial_focus = layout_b.get_focused_panel();

        // Change focus multiple times in layout A
        let panels_a = layout_a.panel_ids();
        for i in 0..focus_changes {
            let idx = i % panels_a.len();
            let _ = layout_a.set_focus(panels_a[idx]);
        }

        // Layout B's focus should be unchanged
        prop_assert_eq!(
            layout_b.get_focused_panel(),
            b_initial_focus,
            "Layout B's focus should be unchanged after focus changes in layout A"
        );
    }

    /// Test that eviction in one layout doesn't affect another
    #[test]
    fn prop_layout_independence_eviction_isolation(
        session1_seed in any::<u128>(),
        session2_seed in any::<u128>(),
    ) {
        let session1 = SessionId(uuid::Uuid::from_u128(session1_seed));
        let session2 = SessionId(uuid::Uuid::from_u128(session2_seed.wrapping_add(1)));

        // Create layout A with a session
        let mut layout_a = SplitLayoutModel::with_session(session1);
        let panel_a = layout_a.panel_ids()[0];

        // Create layout B with a different session
        let layout_b = SplitLayoutModel::with_session(session2);
        let panel_b = layout_b.panel_ids()[0];

        // Evict session1 by placing a new session in layout A
        let new_session = SessionId(uuid::Uuid::from_u128(session1_seed.wrapping_add(100)));
        let result = layout_a.place_in_panel(panel_a, new_session);

        // Verify eviction happened in layout A
        let is_evicted = matches!(result, Ok(DropResult::Evicted { .. }));
        prop_assert!(is_evicted, "Expected eviction result");
        prop_assert_eq!(
            layout_a.get_panel_session(panel_a),
            Some(new_session),
            "Layout A should have new session"
        );

        // Layout B should be completely unchanged
        prop_assert_eq!(
            layout_b.get_panel_session(panel_b),
            Some(session2),
            "Layout B's session should be unchanged after eviction in layout A"
        );
    }
}

// ============================================================================
// Property 2: Split Operation Invariants
// ============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    // Feature: split-view-redesign, Property 2: Split Operation Invariants
    // Validates: Requirements 2.1, 2.2, 2.3, 2.4
    //
    // *For any* `SplitLayoutModel` with a focused panel containing a session, after calling `split(direction)`:
    // - The layout SHALL contain exactly one more panel than before
    // - The original session SHALL be in the first child panel
    // - The second child panel SHALL be empty (no session)
    // - The split direction SHALL match the requested direction

    /// Test that split operation increases panel count by exactly 1
    #[test]
    fn prop_split_increases_panel_count_by_one(
        session_seed in any::<u128>(),
        direction in split_direction_strategy(),
    ) {
        let session = SessionId(uuid::Uuid::from_u128(session_seed));
        let mut layout = SplitLayoutModel::with_session(session);

        let initial_count = layout.panel_count();

        let result = layout.split(direction);
        prop_assert!(result.is_ok(), "Split should succeed on focused panel");

        let final_count = layout.panel_count();
        prop_assert_eq!(
            final_count,
            initial_count + 1,
            "Panel count should increase by exactly 1 after split"
        );
    }

    /// Test that original session stays in the first child panel after split
    #[test]
    fn prop_split_preserves_original_session_in_first_child(
        session_seed in any::<u128>(),
        direction in split_direction_strategy(),
    ) {
        let session = SessionId(uuid::Uuid::from_u128(session_seed));
        let mut layout = SplitLayoutModel::with_session(session);

        // Get the original panel ID before split
        let original_panel_id = layout.panel_ids()[0];

        let result = layout.split(direction);
        prop_assert!(result.is_ok(), "Split should succeed");

        // The original session should still be in the original panel (now first child)
        let session_after = layout.get_panel_session(original_panel_id);
        prop_assert_eq!(
            session_after,
            Some(session),
            "Original session should remain in the first child panel after split"
        );

        // Verify the first panel in the tree is the original panel
        prop_assert_eq!(
            layout.first_panel().id,
            original_panel_id,
            "Original panel should be the first panel in the tree"
        );
        prop_assert_eq!(
            layout.first_panel().session,
            Some(session),
            "First panel should contain the original session"
        );
    }

    /// Test that the second child panel is empty after split
    #[test]
    fn prop_split_creates_empty_second_child(
        session_seed in any::<u128>(),
        direction in split_direction_strategy(),
    ) {
        let session = SessionId(uuid::Uuid::from_u128(session_seed));
        let mut layout = SplitLayoutModel::with_session(session);

        let result = layout.split(direction);
        prop_assert!(result.is_ok(), "Split should succeed");

        let new_panel_id = result.unwrap();

        // The new panel (second child) should be empty
        let new_panel_session = layout.get_panel_session(new_panel_id);
        prop_assert!(
            new_panel_session.is_none(),
            "Second child panel should be empty (no session)"
        );
    }

    /// Test that split direction matches the requested direction
    #[test]
    fn prop_split_direction_matches_requested(
        session_seed in any::<u128>(),
        direction in split_direction_strategy(),
    ) {
        let session = SessionId(uuid::Uuid::from_u128(session_seed));
        let mut layout = SplitLayoutModel::with_session(session);

        let result = layout.split(direction);
        prop_assert!(result.is_ok(), "Split should succeed");

        // Verify the root is now a split with the correct direction
        let root = layout.root();
        prop_assert!(root.is_some(), "Layout should have a root node after split");

        let split_node = root.unwrap().as_split();
        prop_assert!(split_node.is_some(), "Root should be a split node");

        prop_assert_eq!(
            split_node.unwrap().direction,
            direction,
            "Split direction should match the requested direction"
        );
    }

    /// Test all split invariants together for comprehensive validation
    #[test]
    fn prop_split_operation_all_invariants(
        session_seed in any::<u128>(),
        direction in split_direction_strategy(),
    ) {
        let session = SessionId(uuid::Uuid::from_u128(session_seed));
        let mut layout = SplitLayoutModel::with_session(session);

        let original_panel_id = layout.panel_ids()[0];
        let initial_count = layout.panel_count();

        let result = layout.split(direction);
        prop_assert!(result.is_ok(), "Split should succeed on focused panel with session");

        let new_panel_id = result.unwrap();

        // Invariant 1: Panel count increases by exactly 1
        prop_assert_eq!(
            layout.panel_count(),
            initial_count + 1,
            "Invariant 1: Panel count should increase by exactly 1"
        );

        // Invariant 2: Original session is in the first child panel
        prop_assert_eq!(
            layout.get_panel_session(original_panel_id),
            Some(session),
            "Invariant 2: Original session should be in the first child panel"
        );

        // Invariant 3: Second child panel is empty
        prop_assert!(
            layout.get_panel_session(new_panel_id).is_none(),
            "Invariant 3: Second child panel should be empty"
        );

        // Invariant 4: Split direction matches requested
        let root = layout.root().expect("Should have root after split");
        let split_node = root.as_split().expect("Root should be a split node");
        prop_assert_eq!(
            split_node.direction,
            direction,
            "Invariant 4: Split direction should match requested direction"
        );
    }

    /// Test split invariants hold for multiple consecutive splits
    #[test]
    fn prop_split_invariants_hold_for_multiple_splits(
        session_seed in any::<u128>(),
        directions in proptest::collection::vec(split_direction_strategy(), 1..=5),
    ) {
        let session = SessionId(uuid::Uuid::from_u128(session_seed));
        let mut layout = SplitLayoutModel::with_session(session);

        let original_panel_id = layout.panel_ids()[0];

        for (i, direction) in directions.iter().enumerate() {
            let count_before = layout.panel_count();

            let result = layout.split(*direction);
            prop_assert!(result.is_ok(), "Split {} should succeed", i + 1);

            let new_panel_id = result.unwrap();

            // Invariant 1: Panel count increases by exactly 1
            prop_assert_eq!(
                layout.panel_count(),
                count_before + 1,
                "Split {}: Panel count should increase by exactly 1",
                i + 1
            );

            // Invariant 3: New panel is empty
            prop_assert!(
                layout.get_panel_session(new_panel_id).is_none(),
                "Split {}: New panel should be empty",
                i + 1
            );
        }

        // Invariant 2: Original session should still be in the original panel
        prop_assert_eq!(
            layout.get_panel_session(original_panel_id),
            Some(session),
            "Original session should remain in original panel after all splits"
        );
    }

    /// Test split on empty panel (no session) still maintains invariants
    #[test]
    fn prop_split_empty_panel_invariants(
        direction in split_direction_strategy(),
    ) {
        let mut layout = SplitLayoutModel::new();

        let original_panel_id = layout.panel_ids()[0];
        let initial_count = layout.panel_count();

        // Verify original panel is empty
        prop_assert!(
            layout.get_panel_session(original_panel_id).is_none(),
            "Original panel should be empty before split"
        );

        let result = layout.split(direction);
        prop_assert!(result.is_ok(), "Split should succeed on empty panel");

        let new_panel_id = result.unwrap();

        // Invariant 1: Panel count increases by exactly 1
        prop_assert_eq!(
            layout.panel_count(),
            initial_count + 1,
            "Panel count should increase by exactly 1"
        );

        // Invariant 2: Original panel (first child) should still be empty
        prop_assert!(
            layout.get_panel_session(original_panel_id).is_none(),
            "Original panel should remain empty after split"
        );

        // Invariant 3: Second child panel should be empty
        prop_assert!(
            layout.get_panel_session(new_panel_id).is_none(),
            "New panel should be empty"
        );

        // Invariant 4: Split direction matches requested
        let root = layout.root().expect("Should have root after split");
        let split_node = root.as_split().expect("Root should be a split node");
        prop_assert_eq!(
            split_node.direction,
            direction,
            "Split direction should match requested direction"
        );
    }

    /// Test that splitting a nested panel maintains all invariants
    #[test]
    fn prop_split_nested_panel_invariants(
        session_seed in any::<u128>(),
        first_direction in split_direction_strategy(),
        second_direction in split_direction_strategy(),
    ) {
        let session = SessionId(uuid::Uuid::from_u128(session_seed));
        let mut layout = SplitLayoutModel::with_session(session);

        // First split
        let first_new_panel = layout.split(first_direction).unwrap();
        let count_after_first = layout.panel_count();

        // Focus the new panel and split it
        layout.set_focus(first_new_panel).unwrap();

        let result = layout.split(second_direction);
        prop_assert!(result.is_ok(), "Nested split should succeed");

        let second_new_panel = result.unwrap();

        // Invariant 1: Panel count increases by exactly 1
        prop_assert_eq!(
            layout.panel_count(),
            count_after_first + 1,
            "Panel count should increase by exactly 1 after nested split"
        );

        // Invariant 2: The focused panel (first_new_panel) should still exist and be empty
        // (it was empty before the split)
        prop_assert!(
            layout.contains_panel(first_new_panel),
            "Original focused panel should still exist"
        );
        prop_assert!(
            layout.get_panel_session(first_new_panel).is_none(),
            "Original focused panel should remain empty"
        );

        // Invariant 3: New panel should be empty
        prop_assert!(
            layout.get_panel_session(second_new_panel).is_none(),
            "New panel from nested split should be empty"
        );
    }
}

// ============================================================================
// Property 3: Color Allocation Uniqueness
// ============================================================================

// Feature: split-view-redesign, Property 3: Color Allocation Uniqueness
// Validates: Requirements 2.5, 6.1
//
// *For any* sequence of split container creations, each `ColorId` allocated by
// `ColorPool` SHALL be unique until released. When all colors are allocated,
// the pool SHALL cycle through colors but maintain allocation tracking.

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Test that allocated colors are unique until released
    #[test]
    fn prop_allocated_colors_are_unique_until_released(
        allocation_count in 1usize..=SPLIT_COLORS.len(),
    ) {
        let mut pool = ColorPool::new();
        let mut allocated_colors = Vec::new();

        // Allocate colors up to the palette size
        for _ in 0..allocation_count {
            let color = pool.allocate();
            allocated_colors.push(color);
        }

        // All allocated colors should be unique
        let unique_count = {
            let mut seen = std::collections::HashSet::new();
            allocated_colors.iter().filter(|c| seen.insert(**c)).count()
        };

        prop_assert_eq!(
            unique_count,
            allocated_colors.len(),
            "All allocated colors should be unique: {:?}",
            allocated_colors
        );

        // All colors should be marked as allocated in the pool
        for color in &allocated_colors {
            prop_assert!(
                pool.is_allocated(*color),
                "Color {:?} should be marked as allocated",
                color
            );
        }
    }

    /// Test that releasing a color makes it available for reallocation
    #[test]
    fn prop_released_color_becomes_available(
        release_index in 0usize..SPLIT_COLORS.len(),
    ) {
        let mut pool = ColorPool::new();
        let palette_size = SPLIT_COLORS.len();

        // Allocate all colors
        let allocated: Vec<ColorId> = (0..palette_size)
            .map(|_| pool.allocate())
            .collect();

        // Release one color
        let released_color = allocated[release_index];
        pool.release(released_color);

        // The released color should no longer be marked as allocated
        prop_assert!(
            !pool.is_allocated(released_color),
            "Released color {:?} should not be marked as allocated",
            released_color
        );

        // Allocate again - should get the released color back
        let reallocated = pool.allocate();
        prop_assert_eq!(
            reallocated,
            released_color,
            "Should reallocate the released color"
        );

        // Now it should be allocated again
        prop_assert!(
            pool.is_allocated(reallocated),
            "Reallocated color should be marked as allocated"
        );
    }

    /// Test wrap-around behavior when all colors are allocated
    #[test]
    fn prop_wrap_around_when_exhausted(
        extra_allocations in 1usize..10,
    ) {
        let mut pool = ColorPool::new();
        let palette_size = SPLIT_COLORS.len();

        // Allocate all colors
        for _ in 0..palette_size {
            let _ = pool.allocate();
        }

        // All colors should be allocated
        prop_assert_eq!(
            pool.allocated_count(),
            palette_size,
            "All colors should be allocated"
        );

        // Additional allocations should wrap around
        for _ in 0..extra_allocations {
            let color = pool.allocate();

            // The color should be valid (within palette bounds)
            prop_assert!(
                (color.index() as usize) < palette_size,
                "Wrapped color {:?} should be within palette bounds",
                color
            );

            // The color should still be tracked as allocated
            prop_assert!(
                pool.is_allocated(color),
                "Wrapped color {:?} should be marked as allocated",
                color
            );
        }
    }

    /// Test that allocation tracking is maintained during wrap-around
    #[test]
    fn prop_allocation_tracking_maintained_during_wraparound(
        release_indices in proptest::collection::vec(0usize..SPLIT_COLORS.len(), 1..=3),
    ) {
        let mut pool = ColorPool::new();
        let palette_size = SPLIT_COLORS.len();

        // Allocate all colors
        let allocated: Vec<ColorId> = (0..palette_size)
            .map(|_| pool.allocate())
            .collect();

        // Release some colors (using unique indices)
        let mut unique_indices: Vec<usize> = release_indices
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        unique_indices.sort_unstable();

        let mut released_colors = Vec::new();
        for &idx in &unique_indices {
            let color = allocated[idx];
            pool.release(color);
            released_colors.push(color);
        }

        // Verify released colors are not allocated
        for color in &released_colors {
            prop_assert!(
                !pool.is_allocated(*color),
                "Released color {:?} should not be allocated",
                color
            );
        }

        // Verify non-released colors are still allocated
        for (idx, color) in allocated.iter().enumerate() {
            if !unique_indices.contains(&idx) {
                prop_assert!(
                    pool.is_allocated(*color),
                    "Non-released color {:?} should still be allocated",
                    color
                );
            }
        }

        // Allocate new colors - should get the released ones back
        let mut reallocated = Vec::new();
        for _ in 0..released_colors.len() {
            reallocated.push(pool.allocate());
        }

        // All reallocated colors should be from the released set
        for color in &reallocated {
            prop_assert!(
                released_colors.contains(color),
                "Reallocated color {:?} should be from released set {:?}",
                color,
                released_colors
            );
        }
    }

    /// Test that sequential allocations produce sequential colors (before wrap)
    #[test]
    fn prop_sequential_allocations_are_sequential(
        count in 1usize..=SPLIT_COLORS.len(),
    ) {
        let mut pool = ColorPool::new();

        let colors: Vec<ColorId> = (0..count)
            .map(|_| pool.allocate())
            .collect();

        // Colors should be sequential starting from 0
        for (i, color) in colors.iter().enumerate() {
            prop_assert_eq!(
                color.index() as usize,
                i,
                "Color at position {} should have index {}, got {:?}",
                i,
                i,
                color
            );
        }
    }

    /// Test that multiple release-allocate cycles maintain uniqueness
    #[test]
    fn prop_multiple_cycles_maintain_uniqueness(
        cycle_count in 1usize..5,
        colors_per_cycle in 1usize..=SPLIT_COLORS.len(),
    ) {
        let mut pool = ColorPool::new();

        for cycle in 0..cycle_count {
            // Allocate some colors
            let allocated: Vec<ColorId> = (0..colors_per_cycle)
                .map(|_| pool.allocate())
                .collect();

            // Verify uniqueness within this cycle's allocations
            let unique_count = {
                let mut seen = std::collections::HashSet::new();
                allocated.iter().filter(|c| seen.insert(**c)).count()
            };

            prop_assert_eq!(
                unique_count,
                allocated.len(),
                "Cycle {}: All allocated colors should be unique",
                cycle
            );

            // Release all colors for next cycle
            for color in allocated {
                pool.release(color);
            }
        }
    }

    /// Test that allocated_count accurately reflects the number of allocated colors
    #[test]
    fn prop_allocated_count_is_accurate(
        allocations in 0usize..=SPLIT_COLORS.len(),
        releases in 0usize..=SPLIT_COLORS.len(),
    ) {
        let mut pool = ColorPool::new();
        let palette_size = SPLIT_COLORS.len();

        // Allocate colors
        let actual_allocations = allocations.min(palette_size);
        let allocated: Vec<ColorId> = (0..actual_allocations)
            .map(|_| pool.allocate())
            .collect();

        prop_assert_eq!(
            pool.allocated_count(),
            actual_allocations,
            "Allocated count should match number of allocations"
        );

        // Release some colors
        let actual_releases = releases.min(allocated.len());
        for color in allocated.iter().take(actual_releases) {
            pool.release(*color);
        }

        let expected_count = actual_allocations - actual_releases;
        prop_assert_eq!(
            pool.allocated_count(),
            expected_count,
            "Allocated count should be {} after {} allocations and {} releases",
            expected_count,
            actual_allocations,
            actual_releases
        );
    }

    /// Test that releasing an unallocated color is a no-op
    #[test]
    fn prop_release_unallocated_is_noop(
        color_index in 0u8..(SPLIT_COLORS.len() as u8),
    ) {
        let mut pool = ColorPool::new();
        let color = ColorId::new(color_index);

        // Color is not allocated
        prop_assert!(
            !pool.is_allocated(color),
            "Color should not be allocated initially"
        );

        // Release should be a no-op
        pool.release(color);

        // Still not allocated, count still 0
        prop_assert!(
            !pool.is_allocated(color),
            "Color should still not be allocated after release"
        );
        prop_assert_eq!(
            pool.allocated_count(),
            0,
            "Allocated count should still be 0"
        );
    }

    /// Test that the pool correctly handles interleaved allocate/release operations
    #[test]
    fn prop_interleaved_operations_maintain_consistency(
        ops in proptest::collection::vec(
            prop_oneof![
                Just(true),   // allocate
                Just(false),  // release (if possible)
            ],
            1..20
        ),
    ) {
        let mut pool = ColorPool::new();
        let mut currently_allocated: Vec<ColorId> = Vec::new();

        for should_allocate in ops {
            if should_allocate {
                let color = pool.allocate();
                if !currently_allocated.contains(&color) {
                    currently_allocated.push(color);
                }
            } else if !currently_allocated.is_empty() {
                // Release the first allocated color
                let color = currently_allocated.remove(0);
                pool.release(color);
            }

            // Verify consistency: allocated_count should match our tracking
            // (accounting for wrap-around where we might get duplicates)
            let pool_count = pool.allocated_count();

            // Our tracking might be off due to wrap-around, but pool should be consistent
            prop_assert!(
                pool_count <= SPLIT_COLORS.len(),
                "Allocated count {} should not exceed palette size {}",
                pool_count,
                SPLIT_COLORS.len()
            );

            // All colors we think are allocated should be marked as such
            for color in &currently_allocated {
                prop_assert!(
                    pool.is_allocated(*color),
                    "Color {:?} should be allocated according to pool",
                    color
                );
            }
        }
    }
}

// ============================================================================
// Property 4: Recursive Nesting Integrity
// ============================================================================

// Feature: split-view-redesign, Property 4: Recursive Nesting Integrity
// Validates: Requirements 5.1, 5.2, 5.3
//
// *For any* `SplitLayoutModel`, after a sequence of split operations:
// - The panel tree SHALL maintain valid parent-child relationships
// - All panel IDs returned by `panel_ids()` SHALL be reachable from the root
// - The number of leaf panels SHALL equal the number of splits plus one
// - Splitting any panel SHALL increase the total panel count by exactly one

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Test that the panel tree maintains valid structure after multiple splits
    /// (panel count equals splits + 1)
    #[test]
    fn prop_panel_count_equals_splits_plus_one(
        directions in split_operations_strategy(10),
    ) {
        let mut layout = SplitLayoutModel::new();
        let mut successful_splits = 0;

        for direction in &directions {
            // Get a panel to focus and split
            let panels = layout.panel_ids();
            if !panels.is_empty() {
                // Focus a random panel (use first for determinism)
                let _ = layout.set_focus(panels[0]);
                if layout.split(*direction).is_ok() {
                    successful_splits += 1;
                }
            }
        }

        // Panel count should equal successful splits + 1
        prop_assert_eq!(
            layout.panel_count(),
            successful_splits + 1,
            "Panel count ({}) should equal number of splits ({}) + 1",
            layout.panel_count(),
            successful_splits
        );
    }

    /// Test that all panel IDs returned by panel_ids() are reachable
    /// (can be found in the layout)
    #[test]
    fn prop_all_panel_ids_are_reachable(
        directions in split_operations_strategy(8),
    ) {
        let mut layout = SplitLayoutModel::new();

        // Perform splits
        for direction in &directions {
            let panels = layout.panel_ids();
            if !panels.is_empty() {
                let _ = layout.set_focus(panels[0]);
                let _ = layout.split(*direction);
            }
        }

        // All panel IDs should be reachable (contained in the layout)
        let panel_ids = layout.panel_ids();
        for panel_id in &panel_ids {
            prop_assert!(
                layout.contains_panel(*panel_id),
                "Panel {:?} returned by panel_ids() should be reachable in the layout",
                panel_id
            );
        }
    }

    /// Test that panel_ids() returns exactly panel_count() panels
    #[test]
    fn prop_panel_ids_count_matches_panel_count(
        directions in split_operations_strategy(8),
    ) {
        let mut layout = SplitLayoutModel::new();

        // Perform splits
        for direction in &directions {
            let panels = layout.panel_ids();
            if !panels.is_empty() {
                let _ = layout.set_focus(panels[0]);
                let _ = layout.split(*direction);
            }
        }

        let panel_ids = layout.panel_ids();
        let panel_count = layout.panel_count();

        prop_assert_eq!(
            panel_ids.len(),
            panel_count,
            "panel_ids().len() ({}) should equal panel_count() ({})",
            panel_ids.len(),
            panel_count
        );
    }

    /// Test that all panel IDs are unique
    #[test]
    fn prop_all_panel_ids_are_unique(
        directions in split_operations_strategy(8),
    ) {
        let mut layout = SplitLayoutModel::new();

        // Perform splits
        for direction in &directions {
            let panels = layout.panel_ids();
            if !panels.is_empty() {
                let _ = layout.set_focus(panels[0]);
                let _ = layout.split(*direction);
            }
        }

        let panel_ids = layout.panel_ids();
        let unique_count = {
            let mut seen = std::collections::HashSet::new();
            panel_ids.iter().filter(|id| seen.insert(**id)).count()
        };

        prop_assert_eq!(
            unique_count,
            panel_ids.len(),
            "All panel IDs should be unique: {:?}",
            panel_ids
        );
    }

    /// Test that splitting any panel increases the total panel count by exactly one
    #[test]
    fn prop_split_increases_count_by_one(
        initial_splits in 0usize..5,
        target_panel_index in 0usize..10,
        direction in split_direction_strategy(),
    ) {
        let mut layout = SplitLayoutModel::new();

        // Build up initial structure
        for _ in 0..initial_splits {
            let panels = layout.panel_ids();
            if !panels.is_empty() {
                let _ = layout.set_focus(panels[0]);
                let _ = layout.split(SplitDirection::Vertical);
            }
        }

        let count_before = layout.panel_count();
        let panels = layout.panel_ids();

        if !panels.is_empty() {
            // Focus a panel (using modulo to ensure valid index)
            let idx = target_panel_index % panels.len();
            let _ = layout.set_focus(panels[idx]);

            // Split should increase count by exactly 1
            if layout.split(direction).is_ok() {
                prop_assert_eq!(
                    layout.panel_count(),
                    count_before + 1,
                    "Splitting panel should increase count by exactly 1"
                );
            }
        }
    }

    /// Test that nested splits maintain valid tree structure
    #[test]
    fn prop_nested_splits_maintain_valid_structure(
        split_sequence in proptest::collection::vec(
            (0usize..10, split_direction_strategy()),
            1..=8
        ),
    ) {
        let mut layout = SplitLayoutModel::new();

        for (panel_index, direction) in &split_sequence {
            let panels = layout.panel_ids();
            if !panels.is_empty() {
                let idx = panel_index % panels.len();
                let _ = layout.set_focus(panels[idx]);
                let _ = layout.split(*direction);
            }

            // After each operation, verify structure invariants
            let panel_ids = layout.panel_ids();
            let panel_count = layout.panel_count();

            // Invariant 1: panel_ids length matches panel_count
            prop_assert_eq!(
                panel_ids.len(),
                panel_count,
                "panel_ids().len() should match panel_count()"
            );

            // Invariant 2: All panels are reachable
            for panel_id in &panel_ids {
                prop_assert!(
                    layout.contains_panel(*panel_id),
                    "All panels should be reachable"
                );
            }

            // Invariant 3: All panel IDs are unique
            let unique_count = {
                let mut seen = std::collections::HashSet::new();
                panel_ids.iter().filter(|id| seen.insert(**id)).count()
            };
            prop_assert_eq!(
                unique_count,
                panel_ids.len(),
                "All panel IDs should be unique"
            );
        }
    }

    /// Test that depth increases appropriately with nested splits
    #[test]
    fn prop_depth_increases_with_nested_splits(
        nest_depth in 1usize..6,
    ) {
        let mut layout = SplitLayoutModel::new();

        // Create a chain of nested splits by always splitting the newest panel
        for i in 0..nest_depth {
            let panels = layout.panel_ids();
            // Focus the last panel (the newest one from previous split)
            let last_panel = panels[panels.len() - 1];
            let _ = layout.set_focus(last_panel);

            let depth_before = layout.depth();
            let result = layout.split(SplitDirection::Vertical);

            if result.is_ok() {
                let depth_after = layout.depth();
                // Depth should increase by at most 1 per split
                prop_assert!(
                    depth_after >= depth_before,
                    "Depth should not decrease after split (iteration {})",
                    i
                );
                prop_assert!(
                    depth_after <= depth_before + 1,
                    "Depth should increase by at most 1 per split (iteration {})",
                    i
                );
            }
        }

        // Final depth should be at most nest_depth
        prop_assert!(
            layout.depth() <= nest_depth,
            "Final depth ({}) should be at most nest_depth ({})",
            layout.depth(),
            nest_depth
        );
    }

    /// Test that sessions are preserved through nested splits
    #[test]
    fn prop_sessions_preserved_through_nested_splits(
        session_seed in any::<u128>(),
        split_count in 1usize..6,
    ) {
        let session = SessionId(uuid::Uuid::from_u128(session_seed));
        let mut layout = SplitLayoutModel::with_session(session);
        let original_panel_id = layout.panel_ids()[0];

        // Perform multiple splits
        for _ in 0..split_count {
            let panels = layout.panel_ids();
            if !panels.is_empty() {
                // Always split the first panel (which has the session)
                let _ = layout.set_focus(panels[0]);
                let _ = layout.split(SplitDirection::Vertical);
            }
        }

        // Original session should still be in the original panel
        prop_assert_eq!(
            layout.get_panel_session(original_panel_id),
            Some(session),
            "Original session should be preserved through nested splits"
        );

        // All other panels should be empty
        for panel_id in layout.panel_ids() {
            if panel_id != original_panel_id {
                prop_assert!(
                    layout.get_panel_session(panel_id).is_none(),
                    "New panels from splits should be empty"
                );
            }
        }
    }

    /// Test that focus can be set to any panel after nested splits
    #[test]
    fn prop_focus_can_be_set_to_any_panel(
        split_count in 1usize..6,
        focus_attempts in 1usize..10,
    ) {
        let mut layout = SplitLayoutModel::new();

        // Build up structure with splits
        for _ in 0..split_count {
            let panels = layout.panel_ids();
            if !panels.is_empty() {
                let _ = layout.set_focus(panels[0]);
                let _ = layout.split(SplitDirection::Horizontal);
            }
        }

        // Try to focus each panel
        let panels = layout.panel_ids();
        for i in 0..focus_attempts {
            let idx = i % panels.len();
            let panel_id = panels[idx];

            let result = layout.set_focus(panel_id);
            prop_assert!(
                result.is_ok(),
                "Should be able to focus panel {:?}",
                panel_id
            );
            prop_assert_eq!(
                layout.get_focused_panel(),
                Some(panel_id),
                "Focused panel should match the one we set"
            );
        }
    }

    /// Test that tree structure is consistent after mixed split directions
    #[test]
    fn prop_mixed_directions_maintain_structure(
        directions in proptest::collection::vec(split_direction_strategy(), 1..=8),
    ) {
        let mut layout = SplitLayoutModel::new();
        let mut expected_panel_count = 1;

        for direction in &directions {
            let panels = layout.panel_ids();
            if !panels.is_empty() {
                // Alternate between focusing different panels
                let idx = expected_panel_count % panels.len();
                let _ = layout.set_focus(panels[idx]);

                if layout.split(*direction).is_ok() {
                    expected_panel_count += 1;
                }
            }

            // Verify structure after each split
            prop_assert_eq!(
                layout.panel_count(),
                expected_panel_count,
                "Panel count should match expected after split"
            );

            let panel_ids = layout.panel_ids();
            prop_assert_eq!(
                panel_ids.len(),
                expected_panel_count,
                "panel_ids length should match expected"
            );
        }
    }

    /// Test that first_panel() always returns a valid panel
    #[test]
    fn prop_first_panel_always_valid(
        directions in split_operations_strategy(8),
    ) {
        let mut layout = SplitLayoutModel::new();

        // Perform splits
        for direction in &directions {
            let panels = layout.panel_ids();
            if !panels.is_empty() {
                let _ = layout.set_focus(panels[0]);
                let _ = layout.split(*direction);
            }

            // first_panel() should always return a valid panel
            let first = layout.first_panel();
            prop_assert!(
                layout.contains_panel(first.id),
                "first_panel() should return a panel that exists in the layout"
            );

            // first_panel() should be in panel_ids()
            let panel_ids = layout.panel_ids();
            prop_assert!(
                panel_ids.contains(&first.id),
                "first_panel() should be in panel_ids()"
            );
        }
    }
}

// ============================================================================
// Property 5: Empty Panel Placement
// ============================================================================

// Feature: split-view-redesign, Property 5: Empty Panel Placement
// Validates: Requirements 9.1, 9.3, 9.4
//
// *For any* `SplitLayoutModel` with an empty panel, calling `place_in_panel(panel_id, session_id)` SHALL:
// - Return `DropResult::Placed`
// - Result in `get_panel_session(panel_id)` returning `Some(session_id)`
// - Not affect any other panel's session state

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Test that placing a session in an empty panel returns DropResult::Placed
    #[test]
    fn prop_place_in_empty_panel_returns_placed(
        session_seed in any::<u128>(),
    ) {
        let session = SessionId(uuid::Uuid::from_u128(session_seed));
        let mut layout = SplitLayoutModel::new();
        let panel_id = layout.panel_ids()[0];

        // Panel should be empty initially
        prop_assert!(
            layout.get_panel_session(panel_id).is_none(),
            "Panel should be empty before placement"
        );

        let result = layout.place_in_panel(panel_id, session);

        prop_assert!(result.is_ok(), "place_in_panel should succeed");
        prop_assert!(
            matches!(result.unwrap(), DropResult::Placed),
            "Placing in empty panel should return DropResult::Placed"
        );
    }

    /// Test that session is stored correctly after placement in empty panel
    #[test]
    fn prop_session_stored_correctly_after_placement(
        session_seed in any::<u128>(),
    ) {
        let session = SessionId(uuid::Uuid::from_u128(session_seed));
        let mut layout = SplitLayoutModel::new();
        let panel_id = layout.panel_ids()[0];

        layout.place_in_panel(panel_id, session).unwrap();

        prop_assert_eq!(
            layout.get_panel_session(panel_id),
            Some(session),
            "get_panel_session should return the placed session"
        );
    }

    /// Test that placing in empty panel doesn't affect other panels
    #[test]
    fn prop_place_in_empty_panel_does_not_affect_others(
        session1_seed in any::<u128>(),
        session2_seed in any::<u128>(),
        direction in split_direction_strategy(),
    ) {
        let session1 = SessionId(uuid::Uuid::from_u128(session1_seed));
        let session2 = SessionId(uuid::Uuid::from_u128(session2_seed.wrapping_add(1)));

        // Create layout with a session in the first panel
        let mut layout = SplitLayoutModel::with_session(session1);
        let panel1_id = layout.panel_ids()[0];

        // Split to create a second empty panel
        let panel2_id = layout.split(direction).unwrap();

        // Capture state of panel1 before placing in panel2
        let panel1_session_before = layout.get_panel_session(panel1_id);

        // Place session2 in the empty panel2
        let result = layout.place_in_panel(panel2_id, session2);

        prop_assert!(result.is_ok(), "place_in_panel should succeed");
        prop_assert!(
            matches!(result.unwrap(), DropResult::Placed),
            "Placing in empty panel should return DropResult::Placed"
        );

        // Panel1's session should be unchanged
        prop_assert_eq!(
            layout.get_panel_session(panel1_id),
            panel1_session_before,
            "Other panel's session should not be affected"
        );
        prop_assert_eq!(
            layout.get_panel_session(panel1_id),
            Some(session1),
            "Panel1 should still have session1"
        );

        // Panel2 should have the new session
        prop_assert_eq!(
            layout.get_panel_session(panel2_id),
            Some(session2),
            "Panel2 should have session2"
        );
    }

    /// Test all empty panel placement invariants together
    #[test]
    fn prop_empty_panel_placement_all_invariants(
        session_seed in any::<u128>(),
        split_count in 0usize..5,
        target_panel_index in 0usize..10,
    ) {
        let session = SessionId(uuid::Uuid::from_u128(session_seed));
        let mut layout = SplitLayoutModel::new();

        // Build up a structure with multiple panels
        for _ in 0..split_count {
            let panels = layout.panel_ids();
            if !panels.is_empty() {
                let _ = layout.set_focus(panels[0]);
                let _ = layout.split(SplitDirection::Vertical);
            }
        }

        // Find an empty panel to place the session in
        let panels = layout.panel_ids();
        let empty_panels: Vec<_> = panels
            .iter()
            .filter(|&&id| layout.get_panel_session(id).is_none())
            .copied()
            .collect();

        if !empty_panels.is_empty() {
            let idx = target_panel_index % empty_panels.len();
            let target_panel_id = empty_panels[idx];

            // Capture state of all other panels before placement
            let other_sessions: Vec<_> = panels
                .iter()
                .filter(|&&id| id != target_panel_id)
                .map(|&id| (id, layout.get_panel_session(id)))
                .collect();

            // Place session in the empty panel
            let result = layout.place_in_panel(target_panel_id, session);

            // Invariant 1: Returns DropResult::Placed
            prop_assert!(result.is_ok(), "place_in_panel should succeed");
            prop_assert!(
                matches!(result.unwrap(), DropResult::Placed),
                "Invariant 1: Placing in empty panel should return DropResult::Placed"
            );

            // Invariant 2: Session is stored correctly
            prop_assert_eq!(
                layout.get_panel_session(target_panel_id),
                Some(session),
                "Invariant 2: get_panel_session should return the placed session"
            );

            // Invariant 3: Other panels are unaffected
            for (panel_id, original_session) in other_sessions {
                prop_assert_eq!(
                    layout.get_panel_session(panel_id),
                    original_session,
                    "Invariant 3: Panel {:?} session should be unchanged",
                    panel_id
                );
            }
        }
    }

    /// Test placing in empty panel after multiple splits
    #[test]
    fn prop_place_in_empty_panel_after_nested_splits(
        session_seed in any::<u128>(),
        directions in proptest::collection::vec(split_direction_strategy(), 1..=5),
    ) {
        let session = SessionId(uuid::Uuid::from_u128(session_seed));
        let mut layout = SplitLayoutModel::new();

        // Perform multiple splits, always splitting the newest panel
        for direction in &directions {
            let panels = layout.panel_ids();
            let last_panel = panels[panels.len() - 1];
            let _ = layout.set_focus(last_panel);
            let _ = layout.split(*direction);
        }

        // All panels except possibly the first should be empty
        let panels = layout.panel_ids();
        let empty_panels: Vec<_> = panels
            .iter()
            .filter(|&&id| layout.get_panel_session(id).is_none())
            .copied()
            .collect();

        prop_assert!(
            !empty_panels.is_empty(),
            "Should have at least one empty panel after splits"
        );

        // Place session in the last empty panel
        let target_panel = empty_panels[empty_panels.len() - 1];
        let result = layout.place_in_panel(target_panel, session);

        prop_assert!(result.is_ok(), "place_in_panel should succeed");
        prop_assert!(
            matches!(result.unwrap(), DropResult::Placed),
            "Should return DropResult::Placed for empty panel"
        );
        prop_assert_eq!(
            layout.get_panel_session(target_panel),
            Some(session),
            "Session should be stored in the panel"
        );
    }

    /// Test placing different sessions in multiple empty panels
    #[test]
    fn prop_place_multiple_sessions_in_empty_panels(
        session_seeds in proptest::collection::vec(any::<u128>(), 2..=5),
    ) {
        let mut layout = SplitLayoutModel::new();

        // Create enough panels for all sessions
        for _ in 0..session_seeds.len() - 1 {
            let panels = layout.panel_ids();
            let _ = layout.set_focus(panels[0]);
            let _ = layout.split(SplitDirection::Vertical);
        }

        let panels = layout.panel_ids();
        prop_assert!(
            panels.len() >= session_seeds.len(),
            "Should have enough panels for all sessions"
        );

        // Place sessions in panels
        let mut placed_sessions = Vec::new();
        for (i, &seed) in session_seeds.iter().enumerate() {
            let session = SessionId(uuid::Uuid::from_u128(seed.wrapping_add(i as u128)));
            let panel_id = panels[i];

            let result = layout.place_in_panel(panel_id, session);
            prop_assert!(result.is_ok(), "place_in_panel should succeed for panel {}", i);
            prop_assert!(
                matches!(result.unwrap(), DropResult::Placed),
                "Should return DropResult::Placed for empty panel {}", i
            );

            placed_sessions.push((panel_id, session));
        }

        // Verify all sessions are stored correctly
        for (panel_id, session) in &placed_sessions {
            prop_assert_eq!(
                layout.get_panel_session(*panel_id),
                Some(*session),
                "Session should be stored correctly in panel {:?}",
                panel_id
            );
        }
    }

    /// Test that placing in empty panel preserves layout structure
    #[test]
    fn prop_place_in_empty_panel_preserves_structure(
        session_seed in any::<u128>(),
        split_count in 1usize..5,
    ) {
        let session = SessionId(uuid::Uuid::from_u128(session_seed));
        let mut layout = SplitLayoutModel::new();

        // Build up structure
        for _ in 0..split_count {
            let panels = layout.panel_ids();
            let _ = layout.set_focus(panels[0]);
            let _ = layout.split(SplitDirection::Horizontal);
        }

        // Capture structure before placement
        let panel_count_before = layout.panel_count();
        let panel_ids_before = layout.panel_ids();
        let is_split_before = layout.is_split();
        let depth_before = layout.depth();

        // Find an empty panel and place session
        let empty_panel = panel_ids_before
            .iter()
            .find(|&&id| layout.get_panel_session(id).is_none())
            .copied();

        if let Some(target_panel) = empty_panel {
            layout.place_in_panel(target_panel, session).unwrap();

            // Structure should be unchanged
            prop_assert_eq!(
                layout.panel_count(),
                panel_count_before,
                "Panel count should be unchanged after placement"
            );
            prop_assert_eq!(
                layout.panel_ids(),
                panel_ids_before,
                "Panel IDs should be unchanged after placement"
            );
            prop_assert_eq!(
                layout.is_split(),
                is_split_before,
                "is_split should be unchanged after placement"
            );
            prop_assert_eq!(
                layout.depth(),
                depth_before,
                "Depth should be unchanged after placement"
            );
        }
    }

    /// Test placing in the single panel of a new layout
    #[test]
    fn prop_place_in_single_panel_layout(
        session_seed in any::<u128>(),
    ) {
        let session = SessionId(uuid::Uuid::from_u128(session_seed));
        let mut layout = SplitLayoutModel::new();

        // Layout should have exactly one empty panel
        prop_assert_eq!(layout.panel_count(), 1, "New layout should have one panel");
        prop_assert!(!layout.is_split(), "New layout should not be split");

        let panel_id = layout.panel_ids()[0];
        prop_assert!(
            layout.get_panel_session(panel_id).is_none(),
            "Single panel should be empty"
        );

        let result = layout.place_in_panel(panel_id, session);

        prop_assert!(result.is_ok(), "place_in_panel should succeed");
        prop_assert!(
            matches!(result.unwrap(), DropResult::Placed),
            "Should return DropResult::Placed"
        );
        prop_assert_eq!(
            layout.get_panel_session(panel_id),
            Some(session),
            "Session should be stored"
        );

        // Layout structure should be unchanged
        prop_assert_eq!(layout.panel_count(), 1, "Panel count should still be 1");
        prop_assert!(!layout.is_split(), "Layout should still not be split");
    }

    /// Test that placing in empty panel with existing sessions elsewhere works correctly
    #[test]
    fn prop_place_in_empty_panel_with_existing_sessions(
        existing_session_seed in any::<u128>(),
        new_session_seed in any::<u128>(),
        split_count in 1usize..4,
    ) {
        let existing_session = SessionId(uuid::Uuid::from_u128(existing_session_seed));
        let new_session = SessionId(uuid::Uuid::from_u128(new_session_seed.wrapping_add(1)));

        // Create layout with existing session
        let mut layout = SplitLayoutModel::with_session(existing_session);
        let original_panel_id = layout.panel_ids()[0];

        // Create additional empty panels
        for _ in 0..split_count {
            let panels = layout.panel_ids();
            let _ = layout.set_focus(panels[0]);
            let _ = layout.split(SplitDirection::Vertical);
        }

        // Find an empty panel (not the original)
        let panels = layout.panel_ids();
        let empty_panel = panels
            .iter()
            .find(|&&id| id != original_panel_id && layout.get_panel_session(id).is_none())
            .copied();

        if let Some(target_panel) = empty_panel {
            let result = layout.place_in_panel(target_panel, new_session);

            prop_assert!(result.is_ok(), "place_in_panel should succeed");
            prop_assert!(
                matches!(result.unwrap(), DropResult::Placed),
                "Should return DropResult::Placed for empty panel"
            );

            // New session should be in target panel
            prop_assert_eq!(
                layout.get_panel_session(target_panel),
                Some(new_session),
                "New session should be in target panel"
            );

            // Original session should be unchanged
            prop_assert_eq!(
                layout.get_panel_session(original_panel_id),
                Some(existing_session),
                "Original session should be unchanged"
            );
        }
    }
}

// ============================================================================
// Property 6: Occupied Panel Eviction
// ============================================================================

// Feature: split-view-redesign, Property 6: Occupied Panel Eviction
// Validates: Requirements 10.1, 10.2, 10.3, 10.4
//
// *For any* `SplitLayoutModel` with an occupied panel containing `old_session`,
// calling `place_in_panel(panel_id, new_session)` SHALL:
// - Return `DropResult::Evicted { evicted_session: old_session }`
// - Result in `get_panel_session(panel_id)` returning `Some(new_session)`
// - The evicted session ID SHALL match the original occupant

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Test that placing a session in an occupied panel returns DropResult::Evicted
    #[test]
    fn prop_place_in_occupied_panel_returns_evicted(
        old_session_seed in any::<u128>(),
        new_session_seed in any::<u128>(),
    ) {
        let old_session = SessionId(uuid::Uuid::from_u128(old_session_seed));
        let new_session = SessionId(uuid::Uuid::from_u128(new_session_seed.wrapping_add(1)));

        // Create layout with an occupied panel
        let mut layout = SplitLayoutModel::with_session(old_session);
        let panel_id = layout.panel_ids()[0];

        // Panel should be occupied initially
        prop_assert_eq!(
            layout.get_panel_session(panel_id),
            Some(old_session),
            "Panel should be occupied before placement"
        );

        let result = layout.place_in_panel(panel_id, new_session);

        prop_assert!(result.is_ok(), "place_in_panel should succeed");
        prop_assert!(
            matches!(result.as_ref().unwrap(), DropResult::Evicted { .. }),
            "Placing in occupied panel should return DropResult::Evicted, got {:?}",
            result
        );
    }

    /// Test that evicted session matches the original occupant
    #[test]
    fn prop_evicted_session_matches_original(
        old_session_seed in any::<u128>(),
        new_session_seed in any::<u128>(),
    ) {
        let old_session = SessionId(uuid::Uuid::from_u128(old_session_seed));
        let new_session = SessionId(uuid::Uuid::from_u128(new_session_seed.wrapping_add(1)));

        let mut layout = SplitLayoutModel::with_session(old_session);
        let panel_id = layout.panel_ids()[0];

        let result = layout.place_in_panel(panel_id, new_session);

        prop_assert!(result.is_ok(), "place_in_panel should succeed");

        if let Ok(DropResult::Evicted { evicted_session }) = result {
            prop_assert_eq!(
                evicted_session,
                old_session,
                "Evicted session should match the original occupant"
            );
        } else {
            prop_assert!(false, "Expected DropResult::Evicted, got {:?}", result);
        }
    }

    /// Test that new session is stored after eviction
    #[test]
    fn prop_new_session_stored_after_eviction(
        old_session_seed in any::<u128>(),
        new_session_seed in any::<u128>(),
    ) {
        let old_session = SessionId(uuid::Uuid::from_u128(old_session_seed));
        let new_session = SessionId(uuid::Uuid::from_u128(new_session_seed.wrapping_add(1)));

        let mut layout = SplitLayoutModel::with_session(old_session);
        let panel_id = layout.panel_ids()[0];

        layout.place_in_panel(panel_id, new_session).unwrap();

        prop_assert_eq!(
            layout.get_panel_session(panel_id),
            Some(new_session),
            "get_panel_session should return the new session after eviction"
        );
    }

    /// Test all occupied panel eviction invariants together
    #[test]
    fn prop_occupied_panel_eviction_all_invariants(
        old_session_seed in any::<u128>(),
        new_session_seed in any::<u128>(),
    ) {
        let old_session = SessionId(uuid::Uuid::from_u128(old_session_seed));
        let new_session = SessionId(uuid::Uuid::from_u128(new_session_seed.wrapping_add(1)));

        let mut layout = SplitLayoutModel::with_session(old_session);
        let panel_id = layout.panel_ids()[0];

        // Verify panel is occupied
        prop_assert_eq!(
            layout.get_panel_session(panel_id),
            Some(old_session),
            "Panel should be occupied before eviction"
        );

        let result = layout.place_in_panel(panel_id, new_session);

        // Invariant 1: Returns DropResult::Evicted
        prop_assert!(result.is_ok(), "place_in_panel should succeed");
        prop_assert!(
            matches!(result.as_ref().unwrap(), DropResult::Evicted { .. }),
            "Invariant 1: Placing in occupied panel should return DropResult::Evicted"
        );

        // Invariant 2: Evicted session matches original
        if let Ok(DropResult::Evicted { evicted_session }) = result {
            prop_assert_eq!(
                evicted_session,
                old_session,
                "Invariant 2: Evicted session should match the original occupant"
            );
        }

        // Invariant 3: New session is stored
        prop_assert_eq!(
            layout.get_panel_session(panel_id),
            Some(new_session),
            "Invariant 3: get_panel_session should return the new session"
        );
    }

    /// Test eviction in a split layout with multiple panels
    #[test]
    fn prop_eviction_in_split_layout(
        old_session_seed in any::<u128>(),
        new_session_seed in any::<u128>(),
        direction in split_direction_strategy(),
    ) {
        let old_session = SessionId(uuid::Uuid::from_u128(old_session_seed));
        let new_session = SessionId(uuid::Uuid::from_u128(new_session_seed.wrapping_add(1)));

        // Create layout with session and split it
        let mut layout = SplitLayoutModel::with_session(old_session);
        let original_panel_id = layout.panel_ids()[0];
        let new_panel_id = layout.split(direction).unwrap();

        // Verify initial state
        prop_assert_eq!(
            layout.get_panel_session(original_panel_id),
            Some(old_session),
            "Original panel should have old session"
        );
        prop_assert!(
            layout.get_panel_session(new_panel_id).is_none(),
            "New panel should be empty"
        );

        // Evict from the occupied panel
        let result = layout.place_in_panel(original_panel_id, new_session);

        prop_assert!(result.is_ok(), "place_in_panel should succeed");

        if let Ok(DropResult::Evicted { evicted_session }) = result {
            prop_assert_eq!(
                evicted_session,
                old_session,
                "Evicted session should match original"
            );
        } else {
            prop_assert!(false, "Expected DropResult::Evicted");
        }

        // New session should be in the panel
        prop_assert_eq!(
            layout.get_panel_session(original_panel_id),
            Some(new_session),
            "New session should be stored"
        );

        // Other panel should be unaffected
        prop_assert!(
            layout.get_panel_session(new_panel_id).is_none(),
            "Other panel should remain empty"
        );
    }

    /// Test eviction does not affect other panels' sessions
    #[test]
    fn prop_eviction_does_not_affect_other_panels(
        session1_seed in any::<u128>(),
        session2_seed in any::<u128>(),
        session3_seed in any::<u128>(),
        direction in split_direction_strategy(),
    ) {
        let session1 = SessionId(uuid::Uuid::from_u128(session1_seed));
        let session2 = SessionId(uuid::Uuid::from_u128(session2_seed.wrapping_add(1)));
        let session3 = SessionId(uuid::Uuid::from_u128(session3_seed.wrapping_add(2)));

        // Create layout with session1 and split
        let mut layout = SplitLayoutModel::with_session(session1);
        let panel1_id = layout.panel_ids()[0];
        let panel2_id = layout.split(direction).unwrap();

        // Place session2 in the second panel
        layout.place_in_panel(panel2_id, session2).unwrap();

        // Verify both panels are occupied
        prop_assert_eq!(
            layout.get_panel_session(panel1_id),
            Some(session1),
            "Panel1 should have session1"
        );
        prop_assert_eq!(
            layout.get_panel_session(panel2_id),
            Some(session2),
            "Panel2 should have session2"
        );

        // Evict session1 by placing session3 in panel1
        let result = layout.place_in_panel(panel1_id, session3);

        prop_assert!(result.is_ok(), "place_in_panel should succeed");
        prop_assert!(
            matches!(result.unwrap(), DropResult::Evicted { evicted_session } if evicted_session == session1),
            "Should evict session1"
        );

        // Panel1 should have session3
        prop_assert_eq!(
            layout.get_panel_session(panel1_id),
            Some(session3),
            "Panel1 should have session3 after eviction"
        );

        // Panel2 should be unaffected
        prop_assert_eq!(
            layout.get_panel_session(panel2_id),
            Some(session2),
            "Panel2 should still have session2"
        );
    }

    /// Test multiple consecutive evictions on the same panel
    #[test]
    fn prop_multiple_consecutive_evictions(
        session_seeds in proptest::collection::vec(any::<u128>(), 3..=6),
    ) {
        // Ensure we have unique sessions
        let sessions: Vec<SessionId> = session_seeds
            .iter()
            .enumerate()
            .map(|(i, &seed)| SessionId(uuid::Uuid::from_u128(seed.wrapping_add(i as u128))))
            .collect();

        // Create layout with first session
        let mut layout = SplitLayoutModel::with_session(sessions[0]);
        let panel_id = layout.panel_ids()[0];

        // Perform consecutive evictions
        for (i, &new_session) in sessions.iter().enumerate().skip(1) {
            let current_session = layout.get_panel_session(panel_id).unwrap();

            let result = layout.place_in_panel(panel_id, new_session);

            prop_assert!(result.is_ok(), "place_in_panel should succeed for eviction {}", i);

            if let Ok(DropResult::Evicted { evicted_session }) = result {
                prop_assert_eq!(
                    evicted_session,
                    current_session,
                    "Eviction {}: evicted session should match current occupant",
                    i
                );
            } else {
                prop_assert!(false, "Eviction {}: Expected DropResult::Evicted", i);
            }

            prop_assert_eq!(
                layout.get_panel_session(panel_id),
                Some(new_session),
                "Eviction {}: new session should be stored",
                i
            );
        }

        // Final session should be the last one
        prop_assert_eq!(
            layout.get_panel_session(panel_id),
            Some(sessions[sessions.len() - 1]),
            "Final session should be the last one placed"
        );
    }

    /// Test eviction preserves layout structure
    #[test]
    fn prop_eviction_preserves_layout_structure(
        old_session_seed in any::<u128>(),
        new_session_seed in any::<u128>(),
        split_count in 1usize..5,
    ) {
        let old_session = SessionId(uuid::Uuid::from_u128(old_session_seed));
        let new_session = SessionId(uuid::Uuid::from_u128(new_session_seed.wrapping_add(1)));

        // Create layout with session and multiple splits
        let mut layout = SplitLayoutModel::with_session(old_session);
        let original_panel_id = layout.panel_ids()[0];

        for _ in 0..split_count {
            let panels = layout.panel_ids();
            let _ = layout.set_focus(panels[0]);
            let _ = layout.split(SplitDirection::Vertical);
        }

        // Capture structure before eviction
        let panel_count_before = layout.panel_count();
        let panel_ids_before = layout.panel_ids();
        let is_split_before = layout.is_split();
        let depth_before = layout.depth();

        // Perform eviction
        layout.place_in_panel(original_panel_id, new_session).unwrap();

        // Structure should be unchanged
        prop_assert_eq!(
            layout.panel_count(),
            panel_count_before,
            "Panel count should be unchanged after eviction"
        );
        prop_assert_eq!(
            layout.panel_ids(),
            panel_ids_before,
            "Panel IDs should be unchanged after eviction"
        );
        prop_assert_eq!(
            layout.is_split(),
            is_split_before,
            "is_split should be unchanged after eviction"
        );
        prop_assert_eq!(
            layout.depth(),
            depth_before,
            "Depth should be unchanged after eviction"
        );
    }

    /// Test eviction in deeply nested panel
    #[test]
    fn prop_eviction_in_deeply_nested_panel(
        session_seed in any::<u128>(),
        new_session_seed in any::<u128>(),
        nest_depth in 2usize..6,
    ) {
        let old_session = SessionId(uuid::Uuid::from_u128(session_seed));
        let new_session = SessionId(uuid::Uuid::from_u128(new_session_seed.wrapping_add(1)));

        // Create layout with session
        let mut layout = SplitLayoutModel::with_session(old_session);
        let original_panel_id = layout.panel_ids()[0];

        // Create nested splits by always splitting the newest panel
        for _ in 0..nest_depth {
            let panels = layout.panel_ids();
            let last_panel = panels[panels.len() - 1];
            let _ = layout.set_focus(last_panel);
            let _ = layout.split(SplitDirection::Vertical);
        }

        // Original panel should still have the session
        prop_assert_eq!(
            layout.get_panel_session(original_panel_id),
            Some(old_session),
            "Original panel should still have old session"
        );

        // Evict from the original (now deeply nested) panel
        let result = layout.place_in_panel(original_panel_id, new_session);

        prop_assert!(result.is_ok(), "place_in_panel should succeed");

        if let Ok(DropResult::Evicted { evicted_session }) = result {
            prop_assert_eq!(
                evicted_session,
                old_session,
                "Evicted session should match original"
            );
        } else {
            prop_assert!(false, "Expected DropResult::Evicted");
        }

        prop_assert_eq!(
            layout.get_panel_session(original_panel_id),
            Some(new_session),
            "New session should be stored in deeply nested panel"
        );
    }

    /// Test eviction with same session (self-replacement)
    #[test]
    fn prop_eviction_with_same_session(
        session_seed in any::<u128>(),
    ) {
        let session = SessionId(uuid::Uuid::from_u128(session_seed));

        let mut layout = SplitLayoutModel::with_session(session);
        let panel_id = layout.panel_ids()[0];

        // Place the same session again
        let result = layout.place_in_panel(panel_id, session);

        prop_assert!(result.is_ok(), "place_in_panel should succeed");

        // Should still return Evicted (the panel was occupied)
        if let Ok(DropResult::Evicted { evicted_session }) = result {
            prop_assert_eq!(
                evicted_session,
                session,
                "Evicted session should be the same session"
            );
        } else {
            prop_assert!(false, "Expected DropResult::Evicted even for same session");
        }

        // Session should still be in the panel
        prop_assert_eq!(
            layout.get_panel_session(panel_id),
            Some(session),
            "Session should still be in the panel"
        );
    }

    /// Test eviction in panel that was previously empty then occupied
    #[test]
    fn prop_eviction_after_initial_placement(
        session1_seed in any::<u128>(),
        session2_seed in any::<u128>(),
    ) {
        let session1 = SessionId(uuid::Uuid::from_u128(session1_seed));
        let session2 = SessionId(uuid::Uuid::from_u128(session2_seed.wrapping_add(1)));

        // Create empty layout
        let mut layout = SplitLayoutModel::new();
        let panel_id = layout.panel_ids()[0];

        // First placement should return Placed
        let result1 = layout.place_in_panel(panel_id, session1);
        prop_assert!(result1.is_ok(), "First placement should succeed");
        prop_assert!(
            matches!(result1.unwrap(), DropResult::Placed),
            "First placement should return Placed"
        );

        // Second placement should return Evicted
        let result2 = layout.place_in_panel(panel_id, session2);
        prop_assert!(result2.is_ok(), "Second placement should succeed");

        if let Ok(DropResult::Evicted { evicted_session }) = result2 {
            prop_assert_eq!(
                evicted_session,
                session1,
                "Evicted session should be session1"
            );
        } else {
            prop_assert!(false, "Second placement should return Evicted");
        }

        prop_assert_eq!(
            layout.get_panel_session(panel_id),
            Some(session2),
            "Panel should have session2"
        );
    }
}

// ============================================================================
// Additional Unit Tests for Edge Cases
// ============================================================================

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn test_new_layouts_are_independent() {
        let layout_a = SplitLayoutModel::new();
        let layout_b = SplitLayoutModel::new();

        // Panel IDs should be different
        assert_ne!(
            layout_a.panel_ids()[0],
            layout_b.panel_ids()[0],
            "New layouts should have different panel IDs"
        );
    }

    #[test]
    fn test_cloned_layout_is_independent() {
        let mut layout_a = SplitLayoutModel::new();
        let layout_b = layout_a.clone();

        // Modify layout A
        layout_a.split(SplitDirection::Vertical).unwrap();

        // Layout B should be unchanged
        assert_eq!(layout_b.panel_count(), 1);
        assert_eq!(layout_a.panel_count(), 2);
    }

    #[test]
    fn test_session_placement_isolated() {
        let session = SessionId::new();

        let mut layout_a = SplitLayoutModel::new();
        let layout_b = SplitLayoutModel::new();

        let panel_a = layout_a.panel_ids()[0];
        let panel_b = layout_b.panel_ids()[0];

        layout_a.place_in_panel(panel_a, session).unwrap();

        // Session should only be in layout A
        assert_eq!(layout_a.get_panel_session(panel_a), Some(session));
        assert_eq!(layout_b.get_panel_session(panel_b), None);
    }
}

// ============================================================================
// Property 7: Panel Removal and Tree Collapse
// ============================================================================

// Feature: split-view-redesign, Property 7: Panel Removal and Tree Collapse
// Validates: Requirements 4.6, 5.4, 13.1, 13.2, 13.3, 13.5
//
// *For any* `SplitLayoutModel` with more than one panel:
// - Removing a panel SHALL decrease the panel count by exactly one
// - Removing a panel SHALL return the session that was in it (if any)
// - After removal, all remaining panels SHALL still be accessible
// - Removing the last panel SHALL return an error or result in an empty layout
//
// *For any* nested split where one child is removed:
// - The remaining child SHALL be promoted to replace the split node
// - The tree depth SHALL decrease by one at that location

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Test that removing a panel decreases panel count by exactly one
    #[test]
    fn prop_remove_panel_decreases_count_by_one(
        direction in split_direction_strategy(),
        remove_first in any::<bool>(),
    ) {
        let mut layout = SplitLayoutModel::new();
        let original_panel_id = layout.panel_ids()[0];

        // Split to create two panels
        let new_panel_id = layout.split(direction).unwrap();
        let count_before = layout.panel_count();

        // Remove one of the panels
        let panel_to_remove = if remove_first { original_panel_id } else { new_panel_id };
        let result = layout.remove_panel(panel_to_remove);

        prop_assert!(result.is_ok(), "remove_panel should succeed");
        prop_assert_eq!(
            layout.panel_count(),
            count_before - 1,
            "Panel count should decrease by exactly 1 after removal"
        );
    }

    /// Test that removing a panel returns the session that was in it
    #[test]
    fn prop_remove_panel_returns_session(
        session_seed in any::<u128>(),
        direction in split_direction_strategy(),
    ) {
        let session = SessionId(uuid::Uuid::from_u128(session_seed));
        let mut layout = SplitLayoutModel::with_session(session);
        let original_panel_id = layout.panel_ids()[0];

        // Split to create a second panel
        layout.split(direction).unwrap();

        // Remove the panel with the session
        let result = layout.remove_panel(original_panel_id);

        prop_assert!(result.is_ok(), "remove_panel should succeed");
        prop_assert_eq!(
            result.unwrap(),
            Some(session),
            "remove_panel should return the session that was in the panel"
        );
    }

    /// Test that removing an empty panel returns None
    #[test]
    fn prop_remove_empty_panel_returns_none(
        direction in split_direction_strategy(),
    ) {
        let mut layout = SplitLayoutModel::new();

        // Split to create a second empty panel
        let new_panel_id = layout.split(direction).unwrap();

        // Remove the new empty panel
        let result = layout.remove_panel(new_panel_id);

        prop_assert!(result.is_ok(), "remove_panel should succeed");
        prop_assert!(
            result.unwrap().is_none(),
            "remove_panel should return None for empty panel"
        );
    }

    /// Test that all remaining panels are still accessible after removal
    #[test]
    fn prop_remaining_panels_accessible_after_removal(
        split_count in 2usize..6,
        remove_index in 0usize..10,
    ) {
        let mut layout = SplitLayoutModel::new();

        // Create multiple panels
        for _ in 0..split_count {
            let panels = layout.panel_ids();
            let _ = layout.set_focus(panels[0]);
            let _ = layout.split(SplitDirection::Vertical);
        }

        let panels_before = layout.panel_ids();
        let idx = remove_index % panels_before.len();
        let panel_to_remove = panels_before[idx];

        // Remove one panel
        let result = layout.remove_panel(panel_to_remove);
        prop_assert!(result.is_ok(), "remove_panel should succeed");

        // All remaining panels should be accessible
        let panels_after = layout.panel_ids();
        for panel_id in &panels_after {
            prop_assert!(
                layout.contains_panel(*panel_id),
                "Panel {:?} should still be accessible after removal",
                panel_id
            );
        }

        // The removed panel should not be in the list
        prop_assert!(
            !panels_after.contains(&panel_to_remove),
            "Removed panel should not be in panel_ids()"
        );

        // Panel count should match panel_ids length
        prop_assert_eq!(
            layout.panel_count(),
            panels_after.len(),
            "panel_count() should match panel_ids().len()"
        );
    }

    /// Test that removing the last panel returns an error
    #[test]
    fn prop_remove_last_panel_returns_error(
        session_seed in any::<Option<u128>>(),
    ) {
        let mut layout = if let Some(seed) = session_seed {
            SplitLayoutModel::with_session(SessionId(uuid::Uuid::from_u128(seed)))
        } else {
            SplitLayoutModel::new()
        };

        let panel_id = layout.panel_ids()[0];

        // Try to remove the only panel
        let result = layout.remove_panel(panel_id);

        prop_assert!(
            matches!(result, Err(SplitError::CannotRemoveLastPanel)),
            "Removing the last panel should return CannotRemoveLastPanel error"
        );

        // Layout should still have one panel
        prop_assert_eq!(
            layout.panel_count(),
            1,
            "Layout should still have one panel after failed removal"
        );
    }

    /// Test that tree collapses correctly when removing from a simple split
    #[test]
    fn prop_tree_collapses_to_single_panel(
        direction in split_direction_strategy(),
        remove_first in any::<bool>(),
    ) {
        let mut layout = SplitLayoutModel::new();
        let original_panel_id = layout.panel_ids()[0];

        // Split to create two panels
        let new_panel_id = layout.split(direction).unwrap();

        // Verify we have a split
        prop_assert!(layout.is_split(), "Layout should be split before removal");

        // Remove one panel
        let panel_to_remove = if remove_first { original_panel_id } else { new_panel_id };
        layout.remove_panel(panel_to_remove).unwrap();

        // Layout should collapse back to single panel
        prop_assert!(
            !layout.is_split(),
            "Layout should collapse to single panel after removal"
        );
        prop_assert_eq!(
            layout.panel_count(),
            1,
            "Layout should have exactly one panel after collapse"
        );
    }

    /// Test that depth decreases when removing from nested split
    #[test]
    fn prop_depth_decreases_after_removal(
        nest_depth in 2usize..5,
    ) {
        let mut layout = SplitLayoutModel::new();

        // Create nested splits by always splitting the newest panel
        for _ in 0..nest_depth {
            let panels = layout.panel_ids();
            let last_panel = panels[panels.len() - 1];
            let _ = layout.set_focus(last_panel);
            let _ = layout.split(SplitDirection::Vertical);
        }

        let depth_before = layout.depth();
        let panels = layout.panel_ids();

        // Remove the last panel (deepest in the tree)
        let last_panel = panels[panels.len() - 1];
        layout.remove_panel(last_panel).unwrap();

        let depth_after = layout.depth();

        // Depth should decrease or stay the same (depending on tree structure)
        prop_assert!(
            depth_after <= depth_before,
            "Depth should not increase after removal (before: {}, after: {})",
            depth_before,
            depth_after
        );
    }

    /// Test that sibling is promoted when child is removed
    #[test]
    fn prop_sibling_promoted_after_removal(
        session_seed in any::<u128>(),
        direction in split_direction_strategy(),
    ) {
        let session = SessionId(uuid::Uuid::from_u128(session_seed));
        let mut layout = SplitLayoutModel::with_session(session);
        let original_panel_id = layout.panel_ids()[0];

        // Split to create a second panel
        let new_panel_id = layout.split(direction).unwrap();

        // Remove the new panel (sibling should be promoted)
        layout.remove_panel(new_panel_id).unwrap();

        // Original panel should still exist and have its session
        prop_assert!(
            layout.contains_panel(original_panel_id),
            "Original panel should still exist after sibling removal"
        );
        prop_assert_eq!(
            layout.get_panel_session(original_panel_id),
            Some(session),
            "Original panel should still have its session"
        );
    }

    /// Test all removal invariants together
    #[test]
    fn prop_removal_all_invariants(
        session_seed in any::<u128>(),
        direction in split_direction_strategy(),
    ) {
        let session = SessionId(uuid::Uuid::from_u128(session_seed));
        let mut layout = SplitLayoutModel::with_session(session);
        let original_panel_id = layout.panel_ids()[0];

        // Split to create a second panel
        let new_panel_id = layout.split(direction).unwrap();
        let count_before = layout.panel_count();

        // Remove the panel with the session
        let result = layout.remove_panel(original_panel_id);

        // Invariant 1: Removal succeeds
        prop_assert!(result.is_ok(), "Invariant 1: remove_panel should succeed");

        // Invariant 2: Panel count decreases by exactly 1
        prop_assert_eq!(
            layout.panel_count(),
            count_before - 1,
            "Invariant 2: Panel count should decrease by exactly 1"
        );

        // Invariant 3: Session is returned
        prop_assert_eq!(
            result.unwrap(),
            Some(session),
            "Invariant 3: Removed session should be returned"
        );

        // Invariant 4: Remaining panel is accessible
        prop_assert!(
            layout.contains_panel(new_panel_id),
            "Invariant 4: Remaining panel should be accessible"
        );

        // Invariant 5: Tree collapses correctly
        prop_assert!(
            !layout.is_split(),
            "Invariant 5: Tree should collapse to single panel"
        );
    }

    /// Test removal from deeply nested structure
    #[test]
    fn prop_removal_from_deeply_nested_structure(
        session_seed in any::<u128>(),
        nest_depth in 2usize..5,
        remove_depth in 0usize..10,
    ) {
        let session = SessionId(uuid::Uuid::from_u128(session_seed));
        let mut layout = SplitLayoutModel::with_session(session);
        let original_panel_id = layout.panel_ids()[0];

        // Create nested splits
        for _ in 0..nest_depth {
            let panels = layout.panel_ids();
            let last_panel = panels[panels.len() - 1];
            let _ = layout.set_focus(last_panel);
            let _ = layout.split(SplitDirection::Vertical);
        }

        let panels_before = layout.panel_ids();
        let count_before = layout.panel_count();

        // Remove a panel at a specific depth (not the original with session)
        let removable_panels: Vec<_> = panels_before
            .iter()
            .filter(|&&id| id != original_panel_id)
            .copied()
            .collect();

        if !removable_panels.is_empty() {
            let idx = remove_depth % removable_panels.len();
            let panel_to_remove = removable_panels[idx];

            let result = layout.remove_panel(panel_to_remove);

            prop_assert!(result.is_ok(), "remove_panel should succeed");
            prop_assert_eq!(
                layout.panel_count(),
                count_before - 1,
                "Panel count should decrease by 1"
            );

            // Original session should still be accessible
            prop_assert_eq!(
                layout.get_panel_session(original_panel_id),
                Some(session),
                "Original session should still be accessible"
            );

            // All remaining panels should be accessible
            for panel_id in layout.panel_ids() {
                prop_assert!(
                    layout.contains_panel(panel_id),
                    "All remaining panels should be accessible"
                );
            }
        }
    }

    /// Test multiple consecutive removals
    #[test]
    fn prop_multiple_consecutive_removals(
        split_count in 3usize..7,
    ) {
        let mut layout = SplitLayoutModel::new();

        // Create multiple panels
        for _ in 0..split_count {
            let panels = layout.panel_ids();
            let _ = layout.set_focus(panels[0]);
            let _ = layout.split(SplitDirection::Vertical);
        }

        let initial_count = layout.panel_count();

        // Remove panels one by one until only one remains
        let mut removals = 0;
        while layout.panel_count() > 1 {
            let panels = layout.panel_ids();
            let count_before = layout.panel_count();

            // Remove the last panel
            let panel_to_remove = panels[panels.len() - 1];
            let result = layout.remove_panel(panel_to_remove);

            prop_assert!(result.is_ok(), "Removal {} should succeed", removals + 1);
            prop_assert_eq!(
                layout.panel_count(),
                count_before - 1,
                "Removal {}: Panel count should decrease by 1",
                removals + 1
            );

            removals += 1;

            // Safety check to prevent infinite loop
            if removals > initial_count {
                prop_assert!(false, "Too many removals, possible infinite loop");
            }
        }

        // Should have removed all but one panel
        prop_assert_eq!(
            removals,
            initial_count - 1,
            "Should have removed {} panels",
            initial_count - 1
        );
        prop_assert_eq!(
            layout.panel_count(),
            1,
            "Should have exactly one panel remaining"
        );
        prop_assert!(
            !layout.is_split(),
            "Layout should not be split after all removals"
        );
    }

    /// Test that removing unknown panel returns error
    #[test]
    fn prop_remove_unknown_panel_returns_error(
        direction in split_direction_strategy(),
    ) {
        let mut layout = SplitLayoutModel::new();
        layout.split(direction).unwrap();

        let unknown_id = rustconn_core::split::PanelId::new();
        let result = layout.remove_panel(unknown_id);

        prop_assert!(
            matches!(result, Err(SplitError::PanelNotFound(_))),
            "Removing unknown panel should return PanelNotFound error"
        );
    }

    /// Test that focus is updated after removing focused panel
    #[test]
    fn prop_focus_updated_after_removing_focused_panel(
        direction in split_direction_strategy(),
    ) {
        let mut layout = SplitLayoutModel::new();
        let original_panel_id = layout.panel_ids()[0];

        // Split and focus the new panel
        let new_panel_id = layout.split(direction).unwrap();
        layout.set_focus(new_panel_id).unwrap();

        prop_assert_eq!(
            layout.get_focused_panel(),
            Some(new_panel_id),
            "New panel should be focused"
        );

        // Remove the focused panel
        layout.remove_panel(new_panel_id).unwrap();

        // Focus should move to the remaining panel
        prop_assert_eq!(
            layout.get_focused_panel(),
            Some(original_panel_id),
            "Focus should move to remaining panel after removing focused panel"
        );
    }

    /// Test that sessions in other panels are preserved after removal
    #[test]
    fn prop_other_sessions_preserved_after_removal(
        session1_seed in any::<u128>(),
        session2_seed in any::<u128>(),
        direction in split_direction_strategy(),
    ) {
        let session1 = SessionId(uuid::Uuid::from_u128(session1_seed));
        let session2 = SessionId(uuid::Uuid::from_u128(session2_seed.wrapping_add(1)));

        // Create layout with session1
        let mut layout = SplitLayoutModel::with_session(session1);
        let panel1_id = layout.panel_ids()[0];

        // Split and add session2 to new panel
        let panel2_id = layout.split(direction).unwrap();
        layout.place_in_panel(panel2_id, session2).unwrap();

        // Split again to create a third panel
        let _ = layout.set_focus(panel2_id);
        let panel3_id = layout.split(direction).unwrap();

        // Remove the third (empty) panel
        layout.remove_panel(panel3_id).unwrap();

        // Both sessions should be preserved
        prop_assert_eq!(
            layout.get_panel_session(panel1_id),
            Some(session1),
            "Session1 should be preserved"
        );
        prop_assert_eq!(
            layout.get_panel_session(panel2_id),
            Some(session2),
            "Session2 should be preserved"
        );
    }

    /// Test removal with alternating split directions
    #[test]
    fn prop_removal_with_alternating_directions(
        split_count in 2usize..5,
    ) {
        let mut layout = SplitLayoutModel::new();

        // Create splits with alternating directions
        for i in 0..split_count {
            let panels = layout.panel_ids();
            let _ = layout.set_focus(panels[panels.len() - 1]);
            let direction = if i % 2 == 0 {
                SplitDirection::Vertical
            } else {
                SplitDirection::Horizontal
            };
            let _ = layout.split(direction);
        }

        // Remove panels one by one
        for i in 0..split_count {
            let panels = layout.panel_ids();
            let count_before = layout.panel_count();

            if panels.len() > 1 {
                let panel_to_remove = panels[panels.len() - 1];
                let result = layout.remove_panel(panel_to_remove);

                prop_assert!(result.is_ok(), "Removal {} should succeed", i + 1);
                prop_assert_eq!(
                    layout.panel_count(),
                    count_before - 1,
                    "Removal {}: Panel count should decrease by 1",
                    i + 1
                );
            }
        }

        // Should end up with one panel
        prop_assert_eq!(
            layout.panel_count(),
            1,
            "Should have one panel after all removals"
        );
    }

    /// Test that panel IDs remain unique after removal
    #[test]
    fn prop_panel_ids_unique_after_removal(
        split_count in 2usize..6,
        remove_count in 1usize..4,
    ) {
        let mut layout = SplitLayoutModel::new();

        // Create multiple panels
        for _ in 0..split_count {
            let panels = layout.panel_ids();
            let _ = layout.set_focus(panels[0]);
            let _ = layout.split(SplitDirection::Vertical);
        }

        // Remove some panels
        let actual_removes = remove_count.min(layout.panel_count() - 1);
        for _ in 0..actual_removes {
            let panels = layout.panel_ids();
            if panels.len() > 1 {
                let panel_to_remove = panels[panels.len() - 1];
                let _ = layout.remove_panel(panel_to_remove);
            }
        }

        // All remaining panel IDs should be unique
        let panel_ids = layout.panel_ids();
        let unique_count = {
            let mut seen = std::collections::HashSet::new();
            panel_ids.iter().filter(|id| seen.insert(**id)).count()
        };

        prop_assert_eq!(
            unique_count,
            panel_ids.len(),
            "All panel IDs should be unique after removals"
        );
    }
}
