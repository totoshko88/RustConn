# Implementation Plan: Split View Redesign

## Overview

This implementation plan breaks down the split view redesign into incremental tasks. The approach is:
1. Build core data models first (testable without GTK)
2. Add property tests to validate core logic
3. Build GUI adapter layer
4. Integrate with existing window/tab system
5. Wire up drag-and-drop interactions

## Tasks

- [ ] 1. Create core split layout data models in rustconn-core
  - [ ] 1.1 Create split module structure
    - Create `rustconn-core/src/split/mod.rs` with module declarations
    - Create type definitions: `PanelId`, `TabId`, `SessionId`, `ColorId`
    - Create `SplitDirection` enum
    - Re-export types in `rustconn-core/src/lib.rs`
    - _Requirements: 11.1, 11.3_

  - [ ] 1.2 Implement PanelNode tree structure
    - Create `rustconn-core/src/split/tree.rs`
    - Implement `PanelNode` enum (Leaf/Split variants)
    - Implement `LeafPanel` and `SplitNode` structs
    - Add tree traversal methods: `find_panel`, `panel_ids`, `depth`
    - Add tree mutation methods: `insert_split`, `remove_panel`
    - _Requirements: 5.1, 5.2, 5.3_

  - [ ] 1.3 Implement SplitLayoutModel
    - Create `rustconn-core/src/split/model.rs`
    - Implement `SplitLayoutModel` struct with root node and focus tracking
    - Implement `new()`, `is_split()`, `panel_count()`
    - Implement `split()` method for both directions
    - Implement `place_in_panel()` with eviction logic
    - Implement `remove_panel()` with tree collapse
    - Implement `set_focus()` and `get_focused_panel()`
    - _Requirements: 2.1, 2.2, 2.3, 2.4, 9.1, 10.1, 10.2, 13.1, 13.2_

  - [ ] 1.4 Implement ColorPool
    - Create `rustconn-core/src/split/color.rs`
    - Implement `ColorPool` with allocation and release
    - Implement wrap-around when colors exhausted
    - Define standard color palette constants
    - _Requirements: 2.5, 6.1_

  - [ ] 1.5 Implement SplitError and DropResult
    - Add `SplitError` enum with thiserror derive
    - Add `DropResult` enum (Placed, Evicted variants)
    - Ensure all fallible functions return `Result<T, SplitError>`
    - _Requirements: 10.2, 10.3, 10.4_

- [ ] 2. Checkpoint - Core models compile and pass unit tests
  - Ensure `cargo build -p rustconn-core` succeeds
  - Ensure `cargo clippy -p rustconn-core` passes with no warnings
  - Ensure all unit tests pass
  - Ask the user if questions arise.

- [ ] 3. Add property tests for core models
  - [ ] 3.1 Write property test for layout independence
    - **Property 1: Layout Independence**
    - Create `rustconn-core/tests/properties/split_view.rs`
    - Register module in `tests/properties/mod.rs`
    - Test that operations on one layout don't affect others
    - **Validates: Requirements 1.3, 1.4, 3.1, 3.3, 3.4**

  - [ ] 3.2 Write property test for split operation invariants
    - **Property 2: Split Operation Invariants**
    - Test panel count increases by 1 after split
    - Test original session stays in first child
    - Test second child is empty
    - **Validates: Requirements 2.1, 2.2, 2.3, 2.4**

  - [ ] 3.3 Write property test for color allocation uniqueness
    - **Property 3: Color Allocation Uniqueness**
    - Test allocated colors are unique until released
    - Test wrap-around behavior
    - **Validates: Requirements 2.5, 6.1**

  - [ ] 3.4 Write property test for recursive nesting integrity
    - **Property 4: Recursive Nesting Integrity**
    - Test tree maintains valid structure after multiple splits
    - Test all panels are reachable
    - Test panel count equals splits + 1
    - **Validates: Requirements 5.1, 5.2, 5.3**

  - [ ] 3.5 Write property test for empty panel placement
    - **Property 5: Empty Panel Placement**
    - Test placing in empty panel returns Placed
    - Test session is stored correctly
    - Test other panels unaffected
    - **Validates: Requirements 9.1, 9.3, 9.4**

  - [ ] 3.6 Write property test for occupied panel eviction
    - **Property 6: Occupied Panel Eviction**
    - Test placing in occupied panel returns Evicted
    - Test evicted session matches original
    - Test new session is stored
    - **Validates: Requirements 10.1, 10.2, 10.3, 10.4**

  - [ ] 3.7 Write property test for panel removal and collapse
    - **Property 7: Panel Removal and Tree Collapse**
    - Test panel count decreases by 1 after removal
    - Test removed session is returned
    - Test tree collapses correctly
    - Test removing last panel fails or empties layout
    - **Validates: Requirements 4.6, 5.4, 13.1, 13.2, 13.3, 13.5**

- [ ] 4. Checkpoint - Property tests pass
  - Run `cargo test -p rustconn-core --test property_tests`
  - Ensure all property tests pass with 100+ iterations
  - Ask the user if questions arise.

- [ ] 5. Create GUI adapter layer in rustconn
  - [ ] 5.1 Create split view module structure
    - Create `rustconn/src/split_view/mod.rs`
    - Create `rustconn/src/split_view/types.rs` for GUI-specific types
    - Define `DropSource` enum
    - Update `rustconn/src/main.rs` to include module
    - _Requirements: 11.2_

  - [ ] 5.2 Implement SplitViewAdapter
    - Create `rustconn/src/split_view/adapter.rs`
    - Implement struct with model, root widget, panel widget map
    - Implement `new()` and `widget()` methods
    - Implement `split()` that updates both model and widgets
    - Implement `rebuild_widgets()` to sync widget tree with model
    - _Requirements: 2.1, 2.2, 5.1_

  - [ ] 5.3 Implement empty panel placeholder
    - Create `create_empty_placeholder()` method
    - Use `adw::StatusPage` with "Drag Tab Here" text
    - Add close button (X) in top-right corner using overlay
    - Connect close button to panel removal
    - Style with appropriate CSS classes
    - _Requirements: 4.1, 4.5, 4.6_

  - [ ] 5.4 Implement panel widget creation
    - Create `create_panel_widget()` method
    - Set up proper expansion (hexpand, vexpand)
    - Apply color border styling based on ColorId
    - Handle both empty and occupied panel states
    - _Requirements: 6.3_

  - [ ] 5.5 Implement color indicator styling
    - Create CSS classes for each color in palette
    - Apply color class to panel borders
    - Create tab indicator CSS classes
    - Ensure colors work in both light and dark themes
    - _Requirements: 6.2, 6.3, 6.4, 12.4_

- [ ] 6. Implement drag-and-drop system
  - [ ] 6.1 Set up drop targets on panels
    - Create `setup_drop_target()` method
    - Use `gtk4::DropTarget` for string type (session ID)
    - Connect enter/leave for highlight feedback
    - Connect drop handler to `handle_drop()`
    - _Requirements: 8.1, 8.2, 8.3_

  - [ ] 6.2 Implement drop handling logic
    - Create `handle_drop()` method in adapter
    - Parse drop source from drag data
    - Call model's `place_in_panel()`
    - Handle `DropResult::Placed` - update panel widget
    - Handle `DropResult::Evicted` - signal to create new tab
    - _Requirements: 9.1, 9.2, 9.3, 9.4, 10.1, 10.2, 10.3, 10.4_

  - [ ] 6.3 Set up drag sources on tabs
    - Modify existing tab drag source in `terminal/tabs.rs`
    - Ensure session ID is passed as drag data
    - Add visual feedback during drag
    - _Requirements: 7.1, 7.4_

  - [ ] 6.4 Set up drag sources on split panes
    - Add drag source to occupied panel widgets
    - Pass session ID and source panel info
    - Handle removal from source after successful drop
    - _Requirements: 7.2_

  - [ ] 6.5 Set up drag sources on sidebar items
    - Modify sidebar to support drag for connection items
    - Pass connection ID as drag data
    - Distinguish from session ID in drop handler
    - _Requirements: 7.3_

- [ ] 7. Checkpoint - Drag and drop works in isolation
  - Test dragging tabs to empty panels
  - Test dragging tabs to occupied panels (eviction)
  - Test dragging sidebar items to panels
  - Ask the user if questions arise.

- [ ] 8. Implement TabSplitManager for tab-scoped layouts
  - [ ] 8.1 Create TabSplitManager
    - Create `rustconn/src/split_view/manager.rs`
    - Implement struct with HashMap of TabId to SplitViewAdapter
    - Implement shared ColorPool
    - Implement `get_or_create()`, `remove()`, `get_tab_color()`
    - _Requirements: 3.1, 3.3, 3.4_

  - [ ] 8.2 Integrate with TerminalNotebook
    - Modify `TerminalNotebook` to use `TabSplitManager`
    - Create split adapter when tab becomes split container
    - Clean up adapter when tab is closed
    - _Requirements: 1.3, 1.4, 3.4_

  - [ ] 8.3 Implement tab color indicators
    - Update tab header to show color indicator when split
    - Use CSS class from `get_tab_color()`
    - Update indicator when split state changes
    - _Requirements: 6.2_

- [ ] 9. Implement panel lifecycle management
  - [ ] 9.1 Implement panel context menu
    - Add right-click context menu to occupied panels
    - Include "Close Connection" option
    - Include "Move to New Tab" option
    - _Requirements: 13.4_

  - [ ] 9.2 Implement "Close Connection" action
    - Remove panel from split container
    - Redistribute space to adjacent panels
    - Close parent tab if last panel
    - _Requirements: 13.1, 13.2, 13.3_

  - [ ] 9.3 Implement "Move to New Tab" action
    - Extract session from panel
    - Create new root tab with session
    - Remove original panel (triggers 9.2 logic)
    - _Requirements: 13.5_

  - [ ] 9.4 Handle server disconnect
    - Listen for session termination events
    - Trigger panel removal on disconnect
    - Show appropriate feedback to user
    - _Requirements: 13.1_

- [ ] 10. Integrate with main window
  - [ ] 10.1 Update window split actions
    - Modify `win.split-vertical` action to use new system
    - Modify `win.split-horizontal` action to use new system
    - Ensure actions work on focused panel within tab
    - _Requirements: 2.1, 2.2_

  - [ ] 10.2 Update connection opening flow
    - Ensure new connections create root tabs
    - Integrate with existing `connect_selected()` flow
    - Handle both menu and double-click initiation
    - _Requirements: 1.1, 1.2, 1.3_

  - [ ] 10.3 Update tab switching behavior
    - Preserve split layout when switching tabs
    - Restore correct panel focus when returning to tab
    - Update visible content based on active tab's layout
    - _Requirements: 3.2_

  - [ ] 10.4 Clean up old split view code
    - Remove or deprecate old `split_view.rs` implementation
    - Update imports throughout codebase
    - Ensure no regressions in existing functionality
    - _Requirements: 11.1, 11.2_

- [ ] 11. Final checkpoint - Full integration testing
  - Test complete workflow: open connection → split → drag → evict → close
  - Test multiple tabs with independent split layouts
  - Test color indicators on tabs and panels
  - Verify GNOME HIG compliance (spacing, accessibility)
  - Run `cargo clippy --all-targets` with no warnings
  - Run `cargo test` - all tests pass
  - Ask the user if questions arise.

## Notes

- All tasks including property tests are required for comprehensive coverage
- Each task references specific requirements for traceability
- Checkpoints ensure incremental validation
- Property tests validate universal correctness properties
- Unit tests validate specific examples and edge cases
- The implementation follows strict crate boundaries: data models in rustconn-core, UI in rustconn
