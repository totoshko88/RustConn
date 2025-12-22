# Implementation Plan: Performance Improvements

## Overview

This implementation plan covers performance improvements for RustConn including native SPICE embedding, search caching, lazy loading, tracing integration, and optimization of existing performance utilities.

## Tasks

- [x] 1. Add dependencies and feature flags
  - Add `tracing` and `tracing-subscriber` to rustconn-core/Cargo.toml
  - Add `spice-client = "0.2.0"` as optional dependency with `spice-embedded` feature
  - Update rustconn/Cargo.toml to forward the feature flag
  - _Requirements: 1.1, 4.1_

- [x] 2. Implement Search Result Caching
  - [x] 2.1 Create SearchCache struct in rustconn-core/src/search/cache.rs
    - Implement cache with HashMap, TTL, and max entries
    - Add get(), insert(), invalidate_all() methods
    - Add evict_stale() and evict_oldest() for LRU behavior
    - _Requirements: 2.1, 2.2, 2.3, 2.4, 2.5_

  - [x] 2.2 Write property test for cache round-trip
    - **Property 1: Search Cache Round-Trip**
    - **Validates: Requirements 2.1, 2.2**

  - [x] 2.3 Write property test for cache invalidation
    - **Property 2: Search Cache Invalidation**
    - **Validates: Requirements 2.3**

  - [x] 2.4 Write property test for cache TTL expiration
    - **Property 3: Search Cache TTL Expiration**
    - **Validates: Requirements 2.4**

  - [x] 2.5 Write property test for cache size limit
    - **Property 4: Search Cache Size Limit**
    - **Validates: Requirements 2.5**

  - [x] 2.6 Integrate SearchCache into SearchEngine
    - Add cache field to DebouncedSearchEngine
    - Use cache in search_debounced() method
    - _Requirements: 2.1, 2.2_

- [x] 3. Implement Lazy Loading for Connection Groups
  - [x] 3.1 Create LazyGroupLoader in rustconn-core/src/connection/lazy_loader.rs
    - Track loaded groups with HashSet<Uuid>
    - Implement is_group_loaded(), mark_group_loaded()
    - Implement get_children_to_load()
    - _Requirements: 3.1, 3.2, 3.3_

  - [x] 3.2 Write property test for lazy loading initial state
    - **Property 5: Lazy Loading Initial State**
    - **Validates: Requirements 3.1**

  - [x] 3.3 Write property test for lazy loading expansion
    - **Property 6: Lazy Loading Expansion**
    - **Validates: Requirements 3.2, 3.3**

  - [x] 3.4 Integrate LazyGroupLoader into Sidebar
    - Modify sidebar initialization to load only root items
    - Add expand handler to load children on demand
    - Ensure search searches all connections
    - _Requirements: 3.1, 3.2, 3.5_

  - [x] 3.5 Write property test for search ignoring lazy loading
    - **Property 7: Search Ignores Lazy Loading**
    - **Validates: Requirements 3.5**

- [x] 4. Implement Tracing Integration
  - [x] 4.1 Create tracing module in rustconn-core/src/tracing/mod.rs
    - Define TracingConfig struct
    - Implement init_tracing() function
    - Create trace_operation! macro
    - _Requirements: 4.1, 4.4_

  - [x] 4.2 Add tracing spans to key operations
    - Add spans to connection establishment
    - Add spans to search execution
    - Add spans to import/export operations
    - Add spans to credential resolution
    - _Requirements: 4.2, 4.3, 4.5_

  - [x] 4.3 Write property test for tracing span creation
    - **Property 8: Tracing Span Creation**
    - **Validates: Requirements 4.2, 4.5**

- [x] 5. Integrate String Interning
  - [x] 5.1 Add string interning to connection loading
    - Modify ConnectionManager to intern protocol names
    - Intern common hostnames and usernames
    - _Requirements: 5.1, 5.2_

  - [x] 5.2 Add interning statistics logging
    - Log hit rate and bytes saved periodically
    - Add warning when hit rate falls below 30%
    - _Requirements: 5.3, 5.4_

  - [x] 5.3 Write property test for string interning deduplication
    - **Property 9: String Interning Deduplication**
    - **Validates: Requirements 5.1, 5.2**

  - [x] 5.4 Write property test for interning statistics
    - **Property 10: String Interning Statistics**
    - **Validates: Requirements 5.3, 5.4**

- [x] 6. Integrate Virtual Scrolling
  - [x] 6.1 Add VirtualScroller to Sidebar
    - Initialize scroller when connection count > 100
    - Set overscan buffer to 5 items
    - Connect scroll handler to update visible range
    - _Requirements: 6.1, 6.2_

  - [x] 6.2 Implement selection preservation
    - Store selection state separately from rendered items
    - Restore selection when items scroll into view
    - _Requirements: 6.4_

  - [x] 6.3 Write property test for visible range calculation
    - **Property 11: Virtual Scrolling Visible Range**
    - **Validates: Requirements 6.1, 6.2**

  - [x] 6.4 Write property test for selection preservation
    - **Property 12: Virtual Scrolling Selection Preservation**
    - **Validates: Requirements 6.4**

- [x] 7. Integrate Debounced Search
  - [x] 7.1 Add Debouncer to search entry in Sidebar
    - Use Debouncer::for_search() with 100ms delay
    - Connect to search entry changed signal
    - _Requirements: 7.1, 7.2_

  - [x] 7.2 Add search pending indicator
    - Show spinner or visual feedback during debounce
    - Hide indicator when search executes
    - _Requirements: 7.4_

  - [x] 7.3 Write property test for debounce behavior
    - **Property 13: Debounce Behavior**
    - **Validates: Requirements 7.1, 7.2, 7.3**

- [x] 8. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 9. Implement Native SPICE Protocol Embedding
  - [x] 9.1 Update SpiceClient to use spice-client crate
    - Add conditional compilation for spice-embedded feature
    - Implement connect_native() method
    - Handle framebuffer updates
    - _Requirements: 1.1, 1.2, 1.3_

  - [x] 9.2 Implement input forwarding
    - Forward keyboard events to SPICE server
    - Forward mouse events to SPICE server
    - _Requirements: 1.4_

  - [x] 9.3 Implement fallback mechanism
    - Detect native connection failure
    - Fall back to launch_spice_viewer()
    - _Requirements: 1.5_

  - [x] 9.4 Implement session cleanup
    - Clean up resources on disconnect
    - Update session state
    - _Requirements: 1.6_

  - [x] 9.5 Write property test for SPICE fallback
    - **Property 14: SPICE Fallback on Failure**
    - **Validates: Requirements 1.5**

- [x] 10. Implement Async Credential Resolution
  - [x] 10.1 Add async credential methods to AppState
    - Add resolve_credentials_async() method
    - Add resolve_credentials_with_callback() method
    - Remove block_on() calls where possible
    - _Requirements: 9.1, 9.2_

  - [x] 10.2 Add loading indicator for credential resolution
    - Show spinner during async resolution
    - Handle errors without freezing UI
    - _Requirements: 9.3, 9.4_

  - [x] 10.3 Add cancellation support
    - Use CancellationToken for pending requests
    - Cancel on dialog close or timeout
    - _Requirements: 9.5_

  - [x] 10.4 Write property test for async credential resolution
    - **Property 15: Async Credential Resolution**
    - **Validates: Requirements 9.1, 9.4**

- [x] 11. Integrate Batch Processing for Import/Export
  - [x] 11.1 Create BatchImporter in rustconn-core/src/import/
    - Use BatchProcessor with configurable batch size
    - Add progress callback support
    - _Requirements: 10.1, 10.3, 10.4_

  - [x] 11.2 Create BatchExporter in rustconn-core/src/export/
    - Use BatchProcessor with configurable batch size
    - Add progress callback support
    - _Requirements: 10.2, 10.3, 10.4_

  - [x] 11.3 Add cancellation support to batch operations
    - Check cancellation flag between batches
    - Return partial results on cancellation
    - _Requirements: 10.5_

  - [x] 11.4 Write property test for batch processing size
    - **Property 16: Batch Processing Size**
    - **Validates: Requirements 10.1, 10.2, 10.3**

  - [x] 11.5 Write property test for batch cancellation
    - **Property 17: Batch Processing Cancellation**
    - **Validates: Requirements 10.5**

- [x] 12. Dead Code Cleanup
  - [x] 12.1 Review and fix dead code in GUI (rustconn/)
    - sidebar.rs: DragDropData, drag_drop_callback
    - dialogs/connection.rs: save_button, add_variable_button, etc.
    - dialogs/cluster.rs: connection_name, name_label, count_label
    - wayland_surface.rs: mode field
    - Add justification comments or implement functionality
    - _Requirements: 8.1, 8.2_

  - [x] 12.2 Review and fix dead code in rustconn-core
    - import/asbru.rs: children field
    - secret/keepassxc.rs: name, uuid fields
    - Either use the fields or remove them
    - _Requirements: 8.1, 8.3_

  - [x] 12.3 Review and fix dead code in test files
    - rdp_client_tests.rs: arb_framebuffer_data, arb_pixel_data_for_format, arb_widget_coords
    - selection_tests.rs: item_count()
    - cli_tests.rs: TestCliError
    - key_sequence_tests.rs: arb_key_sequence
    - fixtures/mod.rs: all_sample_groups()
    - Either use the functions or remove them
    - _Requirements: 8.1, 8.4_

- [x] 13. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 14. Update Documentation
  - [x] 14.1 Update Dashboard/Welcome screen
    - Add information about new performance features
    - Mention SPICE embedding capability
    - _Requirements: 11.1_

  - [x] 14.2 Update README.md
    - Document spice-embedded feature flag
    - Document performance optimizations
    - Document new configuration options
    - _Requirements: 11.2_

  - [x] 14.3 Update docs/USER_GUIDE.md
    - Add section on embedded SPICE sessions
    - Document search caching behavior
    - Document lazy loading for large databases
    - Add tracing configuration guide
    - _Requirements: 11.3_

  - [x] 14.4 Update dependency documentation
    - Document spice-client dependency
    - Document tracing dependencies
    - _Requirements: 11.5_

- [x] 15. Final checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Notes

- All tasks are required for comprehensive test coverage
- Each task references specific requirements for traceability
- Checkpoints ensure incremental validation
- Property tests validate universal correctness properties
- Unit tests validate specific examples and edge cases
