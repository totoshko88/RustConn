# Implementation Plan

## Phase 1: Critical Bug Fixes

- [x] 1. Fix Tray Icon Display
  - [x] 1.1 Update tray icon path resolution
    - Modify `rustconn/src/tray.rs` to prioritize SVG icon
    - Add `find_icon_theme_path()` function with priority: dev path → installed path → XDG
    - Ensure `icon_theme_path()` returns path to hicolor directory containing scalable/apps/
    - _Requirements: 1.1, 1.2, 1.3_

  - [x] 1.2 Write unit test for icon path resolution
    - Test development path detection
    - Test fallback to PNG when SVG unavailable
    - _Requirements: 1.3_

- [x] 2. Fix GTK PopoverMenu Warning
  - [x] 2.1 Update context menu lifecycle management
    - Modify `show_context_menu_for_item()` in `rustconn/src/sidebar.rs`
    - Connect to `closed` signal and call `unparent()` before destruction
    - Use `popup()` instead of `present()` for proper state management
    - _Requirements: 3.1, 3.2, 3.3_

- [x] 3. Fix Session Logging
  - [x] 3.1 Debug and fix SessionLogger integration
    - Verify `SessionLogger::new()` is called when logging enabled
    - Ensure log directory is created with `fs::create_dir_all()`
    - Add error logging when file creation fails
    - _Requirements: 9.1, 9.3_

  - [x] 3.2 Wire SessionLogger to terminal output
    - Connect VTE terminal output signal to logger.write()
    - Ensure flush is called periodically
    - _Requirements: 9.2_

  - [x] 3.3 Write property test for logging
    - **Property 7: Session Logger File Creation**
    - **Validates: Requirements 9.1, 9.2**

- [x] 4. Fix Cluster Save
  - [x] 4.1 Debug ClusterDialog save callback
    - Verify `on_save` callback is properly connected in `ClusterListDialog`
    - Ensure cluster is added to ConfigManager
    - Call `config_manager.save()` after adding cluster
    - _Requirements: 10.1, 10.2_

  - [x] 4.2 Write property test for cluster persistence
    - **Property 8: Cluster Serialization Round-Trip**
    - **Validates: Requirements 10.1, 10.2**

- [x] 5. Fix Template Creation Button
  - [x] 5.1 Debug TemplateManagerDialog new button
    - Verify `new_btn.connect_clicked()` handler is connected
    - Ensure `on_new` callback is set before showing dialog
    - Check that TemplateDialog is created and presented
    - _Requirements: 11.1, 11.2, 11.4_

  - [x] 5.2 Write property test for template persistence
    - **Property 9: Template Serialization Round-Trip**
    - **Validates: Requirements 11.3**

- [x] 6. Fix Copy/Paste Functionality
  - [x] 6.1 Implement ConnectionClipboard
    - Create `ConnectionClipboard` struct in `rustconn/src/state.rs`
    - Add `copy()`, `paste()`, `has_content()`, `source_group()` methods
    - Add clipboard field to AppState
    - _Requirements: 12.1, 12.2, 12.3_

  - [x] 6.2 Wire Copy/Paste actions
    - Update `win.copy` action to call clipboard.copy()
    - Update `win.paste` action to call clipboard.paste() and add connection
    - Update menu item sensitivity based on clipboard state
    - _Requirements: 12.4, 12.5, 12.6_

  - [x] 6.3 Write property tests for clipboard
    - **Property 10: Connection Copy Creates Valid Duplicate**
    - **Property 11: Connection Paste Preserves Group**
    - **Validates: Requirements 12.1, 12.2, 12.3**

- [x] 7. Checkpoint - Ensure all bug fixes work
  - Run application and verify all fixes manually
  - Ensure all tests pass

## Phase 2: UX Improvements

- [x] 8. Implement Drag-and-Drop Visual Feedback
  - [x] 8.1 Create drop indicator widget
    - Add `DropIndicator` struct to `rustconn/src/sidebar.rs`
    - Create horizontal Separator widget with CSS class "drop-indicator"
    - Add CSS styling for indicator (accent color, 2px height)
    - _Requirements: 2.1_

  - [x] 8.2 Implement drop position tracking
    - Add `connect_motion` handler to DropTarget
    - Calculate drop position (before/after/into) based on Y coordinate
    - Update indicator position in real-time
    - _Requirements: 2.2, 2.3_

  - [x] 8.3 Implement indicator cleanup
    - Hide indicator on drag leave
    - Hide indicator on drop completion
    - _Requirements: 2.4_

- [x] 9. Implement Connection Tree State Preservation
  - [x] 9.1 Create TreeState struct
    - Add `TreeState` struct with expanded_groups, scroll_position, selected_id
    - Add `save_state()` method to ConnectionSidebar
    - Add `restore_state()` method to ConnectionSidebar
    - _Requirements: 7.1, 7.2, 7.3_

  - [x] 9.2 Integrate state preservation
    - Call `save_state()` before refresh operations
    - Call `restore_state()` after refresh
    - Add `refresh_preserving_state()` convenience method
    - _Requirements: 7.4_

- [x] 10. Checkpoint - Verify UX improvements
  - Test drag-and-drop with visual indicator
  - Test tree state preservation after edit

## Phase 3: SSH Enhancements

- [x] 11. Implement SSH IdentitiesOnly Option
  - [x] 11.1 Add fields to SshConfig
    - Add `identities_only: bool` field to SshConfig
    - Add `ssh_agent_key_fingerprint: Option<String>` field
    - Update Serialize/Deserialize derives
    - _Requirements: 6.1, 8.1_

  - [x] 11.2 Update SSH command builder
    - Modify `build_command_args()` to include `-o IdentitiesOnly=yes` when enabled
    - Ensure identity file is specified with `-i` flag
    - _Requirements: 6.2, 6.3_

  - [x] 11.3 Write property tests for SSH options
    - **Property 4: SSH IdentitiesOnly Command Generation**
    - **Property 5: SSH Config Serialization Round-Trip**
    - **Property 6: SSH Agent Key Fingerprint Persistence**
    - **Validates: Requirements 6.2, 6.3, 6.4, 8.1, 8.2, 8.4**

  - [x] 11.4 Add UI controls
    - Add "Use only specified key (IdentitiesOnly)" checkbox to SSH tab
    - Persist ssh_agent_key_fingerprint when key is selected
    - Restore key selection when loading connection
    - _Requirements: 6.1, 8.2, 8.3_

- [x] 12. Checkpoint - Verify SSH enhancements
  - Test IdentitiesOnly prevents "Too many authentication failures"
  - Test key selection persistence

## Phase 4: CLI Enhancements

- [x] 13. Implement CLI Output Feedback
  - [x] 13.1 Create output formatting functions
    - Add `format_connection_message()` function in `rustconn-core/src/protocol/cli.rs`
    - Add `format_command_message()` function
    - Use emoji prefixes: 🔗 for connection, ⚡ for command
    - _Requirements: 5.1, 5.2, 5.3, 5.4_

  - [x] 13.2 Write property tests for CLI output
    - **Property 2: CLI Output Message Format**
    - **Property 3: CLI Command Echo Format**
    - **Validates: Requirements 5.1, 5.2, 5.3, 5.4**

  - [x] 13.3 Integrate feedback into CLI connections
    - Update CLI protocol handler to echo messages before execution
    - Update ZeroTrust protocol handler similarly
    - _Requirements: 5.1, 5.2_

- [x] 14. Implement ZeroTrust Provider Icons
  - [x] 14.1 Create provider icon cache module
    - Create `rustconn-core/src/protocol/icons.rs`
    - Implement `CloudProvider` enum (Aws, Gcloud, Azure, Generic)
    - Implement `ProviderIconCache` struct
    - _Requirements: 4.1, 4.4_

  - [x] 14.2 Implement provider detection
    - Add `detect_provider()` function to analyze CLI command
    - Return appropriate CloudProvider based on command content
    - _Requirements: 4.2_

  - [x] 14.3 Write property test for provider detection
    - **Property 1: Provider Icon Detection**
    - **Validates: Requirements 4.2**

  - [x] 14.4 Integrate icons into sidebar
    - Update sidebar icon selection for ZeroTrust connections
    - Use cached provider icons when available
    - Fall back to generic cloud icon
    - _Requirements: 4.2, 4.3_

- [x] 15. Checkpoint - Verify CLI enhancements
  - Test CLI output messages appear correctly
  - Test provider icons display for AWS/GCloud/Azure

## Phase 5: Native Export/Import

- [x] 16. Implement Native Export Format
  - [x] 16.1 Create native export module
    - Create `rustconn-core/src/export/native.rs`
    - Define `NATIVE_FORMAT_VERSION` constant
    - Implement `NativeExport` struct with all data types
    - _Requirements: 13.1, 13.4_

  - [x] 16.2 Implement export functionality
    - Add `to_json()` method for serialization
    - Include connections, groups, templates, clusters, variables
    - Add metadata and version info
    - _Requirements: 13.2_

  - [x] 16.3 Implement import functionality
    - Add `from_json()` method with version validation
    - Implement `migrate()` function for future version upgrades
    - Return appropriate errors for unsupported versions
    - _Requirements: 13.3, 13.5_

  - [x] 16.4 Write property tests for native format
    - **Property 12: Native Export Contains All Data Types**
    - **Property 13: Native Import Restores All Data**
    - **Property 14: Native Format Round-Trip**
    - **Property 15: Native Format Schema Version**
    - **Property 16: Native Import Version Validation**
    - **Validates: Requirements 13.2, 13.3, 13.4, 13.5, 13.6**

  - [x] 16.5 Add UI integration
    - Add "RustConn Native (.rcn)" option to export dialog
    - Add import support for .rcn files
    - _Requirements: 13.1_

- [x] 17. Checkpoint - Verify export/import
  - Test full export/import round-trip
  - Verify all data types are preserved

## Phase 6: GTK4 Upgrade

- [x] 18. Upgrade to GTK 4.20
  - [x] 18.1 Update dependencies
    - Update `gtk4` version in Cargo.toml to support 4.20
    - Update `vte4` if needed for compatibility
    - Update feature flags (v4_20 if available)
    - _Requirements: 14.1_

  - [x] 18.2 Update deprecated APIs
    - Review and update any deprecated GTK4 patterns
    - Use modern APIs where available
    - _Requirements: 14.2_

  - [x] 18.3 Verify compatibility
    - Run full test suite
    - Test all UI components manually
    - _Requirements: 14.3_

  - [x] 18.4 Document improvements
    - Note any new GTK 4.20 features that could improve UI
    - Create list of potential future enhancements
    - _Requirements: 14.4_

- [x] 19. Checkpoint - Verify GTK upgrade
  - Ensure all tests pass
  - Verify no regressions in UI

## Phase 7: Embedded RDP/VNC (Advanced)

- [x] 20. Implement Embedded RDP Widget
  - [x] 20.1 Create embedded RDP module
    - Create `rustconn/src/embedded_rdp.rs`
    - Implement `EmbeddedRdpWidget` struct with DrawingArea
    - Add Wayland surface handling infrastructure
    - _Requirements: 16.1, 16.3_

  - [x] 20.2 Implement FreeRDP integration
    - Add wlfreerdp detection and initialization
    - Set up BeginPaint/EndPaint callbacks
    - Implement pixel buffer capture and blit to wl_buffer
    - _Requirements: 16.4_

  - [x] 20.3 Implement input forwarding
    - Forward keyboard events to FreeRDP
    - Forward mouse events to FreeRDP
    - Handle focus management
    - _Requirements: 16.5_

  - [x] 20.4 Implement resize handling
    - Detect widget resize events
    - Notify remote session of resolution change
    - Reallocate buffers as needed
    - _Requirements: 16.7_

  - [x] 20.5 Implement cleanup and fallback
    - Proper cleanup on disconnect
    - Detect wlfreerdp availability
    - Fall back to external xfreerdp if unavailable
    - _Requirements: 16.6, 16.8_

- [x] 21. Implement Embedded VNC Widget
  - [x] 21.1 Create embedded VNC module
    - Create `rustconn/src/embedded_vnc.rs`
    - Similar structure to RDP widget
    - Use appropriate VNC library (libvncclient or similar)
    - _Requirements: 16.2_

  - [x] 21.2 Implement VNC rendering
    - Set up frame buffer handling
    - Blit to Wayland surface
    - _Requirements: 16.3, 16.4_

  - [x] 21.3 Implement VNC input
    - Forward keyboard and mouse
    - _Requirements: 16.5_

- [x] 22. Checkpoint - Verify embedded RDP/VNC
  - Test embedded RDP connection
  - Test embedded VNC connection
  - Test fallback to external mode

## Phase 8: Performance Optimization

- [x] 23. Optimize Application Performance
  - [x] 23.1 Profile startup time
    - Measure current startup time
    - Identify bottlenecks
    - Optimize lazy loading where possible
    - _Requirements: 15.1_

  - [x] 23.2 Optimize sidebar rendering
    - Profile large connection list rendering
    - Implement virtual scrolling if needed
    - Optimize tree model updates
    - _Requirements: 15.2_

  - [x] 23.3 Optimize search performance
    - Profile search with large datasets
    - Implement debouncing for search input
    - Optimize fuzzy matching algorithm
    - _Requirements: 15.3_

  - [x] 23.4 Optimize memory usage
    - Profile memory usage
    - Identify memory leaks
    - Optimize data structures
    - _Requirements: 15.4_

- [x] 24. Final Checkpoint
  - Run full test suite
  - Verify all requirements are met
  - Performance benchmarks pass

