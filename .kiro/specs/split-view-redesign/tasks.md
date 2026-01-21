# Implementation Plan: Split View Redesign

## Overview

This implementation plan breaks down the split view redesign into incremental tasks. The approach is:
1. Build core data models first (testable without GTK)
2. Add property tests to validate core logic
3. Build GUI adapter layer
4. Integrate with existing window/tab system
5. Wire up drag-and-drop interactions

## Tasks

- [x] 1. Create core split layout data models in rustconn-core
  - [x] 1.1 Create split module structure
    - Create `rustconn-core/src/split/mod.rs` with module declarations
    - Create type definitions: `PanelId`, `TabId`, `SessionId`, `ColorId`
    - Create `SplitDirection` enum
    - Re-export types in `rustconn-core/src/lib.rs`
    - _Requirements: 11.1, 11.3_

  - [x] 1.2 Implement PanelNode tree structure
    - Create `rustconn-core/src/split/tree.rs`
    - Implement `PanelNode` enum (Leaf/Split variants)
    - Implement `LeafPanel` and `SplitNode` structs
    - Add tree traversal methods: `find_panel`, `panel_ids`, `depth`
    - Add tree mutation methods: `insert_split`, `remove_panel`
    - _Requirements: 5.1, 5.2, 5.3_

  - [x] 1.3 Implement SplitLayoutModel
    - Create `rustconn-core/src/split/model.rs`
    - Implement `SplitLayoutModel` struct with root node and focus tracking
    - Implement `new()`, `is_split()`, `panel_count()`
    - Implement `split()` method for both directions
    - Implement `place_in_panel()` with eviction logic
    - Implement `remove_panel()` with tree collapse
    - Implement `set_focus()` and `get_focused_panel()`
    - _Requirements: 2.1, 2.2, 2.3, 2.4, 9.1, 10.1, 10.2, 13.1, 13.2_

  - [x] 1.4 Implement ColorPool
    - Create `rustconn-core/src/split/color.rs`
    - Implement `ColorPool` with allocation and release
    - Implement wrap-around when colors exhausted
    - Define standard color palette constants
    - _Requirements: 2.5, 6.1_

  - [x] 1.5 Implement SplitError and DropResult
    - Add `SplitError` enum with thiserror derive
    - Add `DropResult` enum (Placed, Evicted variants)
    - Ensure all fallible functions return `Result<T, SplitError>`
    - _Requirements: 10.2, 10.3, 10.4_

- [x] 2. Checkpoint - Core models compile and pass unit tests
  - Ensure `cargo build -p rustconn-core` succeeds
  - Ensure `cargo clippy -p rustconn-core` passes with no warnings
  - Ensure all unit tests pass
  - Ask the user if questions arise.

- [x] 3. Add property tests for core models
  - [x] 3.1 Write property test for layout independence
    - **Property 1: Layout Independence**
    - Create `rustconn-core/tests/properties/split_view.rs`
    - Register module in `tests/properties/mod.rs`
    - Test that operations on one layout don't affect others
    - **Validates: Requirements 1.3, 1.4, 3.1, 3.3, 3.4**

  - [x] 3.2 Write property test for split operation invariants
    - **Property 2: Split Operation Invariants**
    - Test panel count increases by 1 after split
    - Test original session stays in first child
    - Test second child is empty
    - **Validates: Requirements 2.1, 2.2, 2.3, 2.4**

  - [x] 3.3 Write property test for color allocation uniqueness
    - **Property 3: Color Allocation Uniqueness**
    - Test allocated colors are unique until released
    - Test wrap-around behavior
    - **Validates: Requirements 2.5, 6.1**

  - [x] 3.4 Write property test for recursive nesting integrity
    - **Property 4: Recursive Nesting Integrity**
    - Test tree maintains valid structure after multiple splits
    - Test all panels are reachable
    - Test panel count equals splits + 1
    - **Validates: Requirements 5.1, 5.2, 5.3**

  - [x] 3.5 Write property test for empty panel placement
    - **Property 5: Empty Panel Placement**
    - Test placing in empty panel returns Placed
    - Test session is stored correctly
    - Test other panels unaffected
    - **Validates: Requirements 9.1, 9.3, 9.4**

  - [x] 3.6 Write property test for occupied panel eviction
    - **Property 6: Occupied Panel Eviction**
    - Test placing in occupied panel returns Evicted
    - Test evicted session matches original
    - Test new session is stored
    - **Validates: Requirements 10.1, 10.2, 10.3, 10.4**

  - [x] 3.7 Write property test for panel removal and collapse
    - **Property 7: Panel Removal and Tree Collapse**
    - Test panel count decreases by 1 after removal
    - Test removed session is returned
    - Test tree collapses correctly
    - Test removing last panel fails or empties layout
    - **Validates: Requirements 4.6, 5.4, 13.1, 13.2, 13.3, 13.5**

- [x] 4. Checkpoint - Property tests pass
  - Run `cargo test -p rustconn-core --test property_tests`
  - Ensure all property tests pass with 100+ iterations
  - Ask the user if questions arise.

- [x] 5. Create GUI adapter layer in rustconn
  - [x] 5.1 Create split view module structure
    - Create `rustconn/src/split_view/mod.rs`
    - Create `rustconn/src/split_view/types.rs` for GUI-specific types
    - Define `DropSource` enum
    - Update `rustconn/src/main.rs` to include module
    - _Requirements: 11.2_

  - [x] 5.2 Implement SplitViewAdapter
    - Create `rustconn/src/split_view/adapter.rs`
    - Implement struct with model, root widget, panel widget map
    - Implement `new()` and `widget()` methods
    - Implement `split()` that updates both model and widgets
    - Implement `rebuild_widgets()` to sync widget tree with model
    - _Requirements: 2.1, 2.2, 5.1_

  - [x] 5.3 Implement empty panel placeholder
    - Create `create_empty_placeholder()` method
    - Use `adw::StatusPage` with "Drag Tab Here" text
    - Add close button (X) in top-right corner using overlay
    - Connect close button to panel removal
    - Style with appropriate CSS classes
    - _Requirements: 4.1, 4.5, 4.6_

  - [x] 5.4 Implement panel widget creation
    - Create `create_panel_widget()` method
    - Set up proper expansion (hexpand, vexpand)
    - Apply color border styling based on ColorId
    - Handle both empty and occupied panel states
    - _Requirements: 6.3_

  - [x] 5.5 Implement color indicator styling
    - Create CSS classes for each color in palette
    - Apply color class to panel borders
    - Create tab indicator CSS classes
    - Ensure colors work in both light and dark themes
    - _Requirements: 6.2, 6.3, 6.4, 12.4_

- [x] 6. Implement drag-and-drop system
  - [x] 6.1 Set up drop targets on panels
    - Create `setup_drop_target()` method
    - Use `gtk4::DropTarget` for string type (session ID)
    - Connect enter/leave for highlight feedback
    - Connect drop handler to `handle_drop()`
    - _Requirements: 8.1, 8.2, 8.3_

  - [x] 6.2 Implement drop handling logic
    - Create `handle_drop()` method in adapter
    - Parse drop source from drag data
    - Call model's `place_in_panel()`
    - Handle `DropResult::Placed` - update panel widget
    - Handle `DropResult::Evicted` - signal to create new tab
    - _Requirements: 9.1, 9.2, 9.3, 9.4, 10.1, 10.2, 10.3, 10.4_

  - [x] 6.3 Set up drag sources on tabs
    - Modify existing tab drag source in `terminal/tabs.rs`
    - Ensure session ID is passed as drag data
    - Add visual feedback during drag
    - _Requirements: 7.1, 7.4_

  - [x] 6.4 Set up drag sources on split panes
    - Add drag source to occupied panel widgets
    - Pass session ID and source panel info
    - Handle removal from source after successful drop
    - _Requirements: 7.2_

  - [x] 6.5 Set up drag sources on sidebar items
    - Modify sidebar to support drag for connection items
    - Pass connection ID as drag data
    - Distinguish from session ID in drop handler
    - _Requirements: 7.3_

- [x] 7. Checkpoint - Drag and drop works in isolation
  - Test dragging tabs to empty panels
  - Test dragging tabs to occupied panels (eviction)
  - Test dragging sidebar items to panels
  - Ask the user if questions arise.

- [x] 8. Implement TabSplitManager for tab-scoped layouts
  - [x] 8.1 Create TabSplitManager
    - Create `rustconn/src/split_view/manager.rs`
    - Implement struct with HashMap of TabId to SplitViewAdapter
    - Implement shared ColorPool
    - Implement `get_or_create()`, `remove()`, `get_tab_color()`
    - _Requirements: 3.1, 3.3, 3.4_

  - [x] 8.2 Integrate with TerminalNotebook
    - Modify `TerminalNotebook` to use `TabSplitManager`
    - Create split adapter when tab becomes split container
    - Clean up adapter when tab is closed
    - _Requirements: 1.3, 1.4, 3.4_

  - [x] 8.3 Implement tab color indicators
    - Update tab header to show color indicator when split
    - Use CSS class from `get_tab_color()`
    - Update indicator when split state changes
    - _Requirements: 6.2_

- [x] 9. Implement panel lifecycle management
  - [x] 9.1 Implement panel context menu
    - Add right-click context menu to occupied panels
    - Include "Close Connection" option
    - Include "Move to New Tab" option
    - _Requirements: 13.4_

  - [x] 9.2 Implement "Close Connection" action
    - Remove panel from split container
    - Redistribute space to adjacent panels
    - Close parent tab if last panel
    - _Requirements: 13.1, 13.2, 13.3_

  - [x] 9.3 Implement "Move to New Tab" action
    - Extract session from panel
    - Create new root tab with session
    - Remove original panel (triggers 9.2 logic)
    - _Requirements: 13.5_

  - [x] 9.4 Handle server disconnect
    - Listen for session termination events
    - Trigger panel removal on disconnect
    - Show appropriate feedback to user
    - _Requirements: 13.1_

- [x] 10. Integrate with main window
  - [x] 10.1 Update window split actions
    - Modify `win.split-vertical` action to use new system
    - Modify `win.split-horizontal` action to use new system
    - Ensure actions work on focused panel within tab
    - _Requirements: 2.1, 2.2_

  - [x] 10.2 Update connection opening flow
    - Ensure new connections create root tabs
    - Integrate with existing `connect_selected()` flow
    - Handle both menu and double-click initiation
    - _Requirements: 1.1, 1.2, 1.3_

  - [x] 10.3 Update tab switching behavior
    - Preserve split layout when switching tabs
    - Restore correct panel focus when returning to tab
    - Update visible content based on active tab's layout
    - _Requirements: 3.2_

  - [x] 10.4 Clean up old split view code
    - Remove or deprecate old `split_view.rs` implementation
    - Update imports throughout codebase
    - Ensure no regressions in existing functionality
    - _Requirements: 11.1, 11.2_

- [x] 11. Final checkpoint - Full integration testing
  - Test complete workflow: open connection → split → drag → evict → close
  - Test multiple tabs with independent split layouts
  - Test color indicators on tabs and panels
  - Verify GNOME HIG compliance (spacing, accessibility)
  - Run `cargo clippy --all-targets` with no warnings
  - Run `cargo test` - all tests pass
  - Ask the user if questions arise.

- [x] 12. Replace legacy SplitTerminalView with new implementation
  - [x] 12.1 Update MainWindow to use TabSplitManager
    - Replace `SharedSplitView` (SplitTerminalView) with `TabSplitManager`
    - Update `window_types.rs` to use new types
    - Update MainWindow struct fields
    - Update terminal_container layout to use new split view widget
    - _Requirements: 1.3, 3.1_

  - [x] 12.2 Migrate split actions to new system
    - Update `setup_split_view_actions()` to use `TabSplitManager`
    - Update `win.split-vertical` action implementation
    - Update `win.split-horizontal` action implementation
    - Update `win.close-pane` action implementation
    - Update `win.unsplit-session` action implementation
    - _Requirements: 2.1, 2.2, 13.1_

  - [x] 12.3 Migrate session display logic
    - Update `show_session()` to use new adapter
    - Update `clear_session_from_panes()` to use new system
    - Update `close_session_from_panes()` to use new system
    - Update focus management for new panel structure
    - _Requirements: 9.1, 13.1, 13.2_

  - [x] 12.4 Migrate drag-and-drop handling
    - Update drop handlers to use `DropOutcome`
    - Handle eviction by creating new tabs
    - Update sidebar drag source integration
    - Update tab drag source integration
    - _Requirements: 9.2, 10.1, 10.2, 10.3_

  - [x] 12.5 Update copy/paste actions for new focus system
    - Update `win.copy` action to use new focused panel
    - Update `win.paste` action to use new focused panel
    - Update terminal search to work with new system
    - _Requirements: 3.2_

- [x] 13. Migrate TerminalNotebook integration
  - [x] 13.1 Update tab-to-split-view coordination
    - Use `TabSplitManager.get_or_create()` when splitting
    - Use `TabSplitManager.remove()` when closing tabs
    - Sync tab colors with `ColorPool` allocation
    - _Requirements: 3.3, 3.4, 6.1_

  - [x] 13.2 Update session lifecycle in notebook
    - Update `add_session()` to work with new split system
    - Update `close_tab()` to clean up split layouts
    - Update `get_active_session_id()` for split awareness
    - _Requirements: 1.3, 13.1, 13.3_

  - [x] 13.3 Update tab switching for split layouts
    - Show correct split layout widget when tab becomes active
    - Hide split layout when switching to non-split tab
    - Preserve focus state per tab
    - _Requirements: 3.2_

- [-] 14. Remove legacy code and final cleanup
  - [x] 14.1 Remove legacy.rs exports from mod.rs
    - Remove `SplitTerminalView` export
    - Remove `TerminalPane` export
    - Remove legacy helper functions
    - Keep only new implementation exports
    - _Requirements: 11.1, 11.2_

  - [x] 14.2 Update all imports throughout codebase
    - Update `window.rs` imports
    - Update `window_sessions.rs` imports
    - Update `window_operations.rs` imports
    - Update any other files using legacy types
    - _Requirements: 11.2_

  - [x] 14.3 Delete legacy.rs file
    - Ensure all functionality is migrated
    - Remove `rustconn/src/split_view/legacy.rs`
    - Update mod.rs to not include legacy module
    - _Requirements: 11.1_

- [x] 15. Final integration checkpoint
  - Run `cargo build` - ensure compilation succeeds
  - Run `cargo clippy --all-targets` - no warnings
  - Run `cargo test` - all tests pass
  - Manual testing: open connection → split → drag → evict → close
  - Manual testing: multiple tabs with independent layouts
  - Manual testing: color indicators work correctly
  - Manual testing: context menus work (Close, Move to Tab)
  - Ask the user if questions arise.

## Notes

- All tasks including property tests are required for comprehensive coverage
- Each task references specific requirements for traceability
- Checkpoints ensure incremental validation
- Property tests validate universal correctness properties
- Unit tests validate specific examples and edge cases
- The implementation follows strict crate boundaries: data models in rustconn-core, UI in rustconn
