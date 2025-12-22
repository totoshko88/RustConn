# Implementation Plan

## Phase 1: Critical Bug Fixes

- [x] 1. Fix VNC Connection (Empty Tab Issue)
  - [x] 1.1 Debug VNC session launch
    - Verify VNC viewer detection in `rustconn/src/session/vnc.rs`
    - Ensure `spawn_viewer()` is called when connection opens
    - Add error handling for missing VNC viewer
    - _Requirements: 8.1, 8.2, 8.3_

  - [x] 1.2 Implement VNC viewer detection
    - Add `detect_vnc_viewer()` function checking for vncviewer, tigervnc, gvncviewer
    - Return appropriate error if no viewer found
    - _Requirements: 8.3_

  - [x] 1.3 Write property test for VNC viewer detection
    - **Property 10: VNC Viewer Detection**
    - **Validates: Requirements 8.1, 8.3**

- [x] 2. Fix Template Creation and Persistence
  - [x] 2.1 Debug template dialog protocol selection
    - Verify protocol combo box is populated with all protocols
    - Ensure protocol-specific fields are shown/hidden correctly
    - _Requirements: 10.1_

  - [x] 2.2 Fix template save functionality
    - Verify `save_template()` is called on Save button click
    - Ensure template is added to ConfigManager
    - Call `config_manager.save()` after adding template
    - _Requirements: 10.2, 10.3_

  - [x] 2.3 Fix template list refresh
    - Refresh template list after save
    - _Requirements: 10.5_

  - [x] 2.4 Write property test for template persistence
    - **Property 12: Template Protocol Persistence**
    - **Validates: Requirements 10.1, 10.2, 10.3**

- [x] 3. Fix Cluster Dialog Auto-Refresh
  - [x] 3.1 Add refresh after cluster operations
    - Add `refresh_list()` method to ClusterListDialog
    - Call refresh after new cluster creation
    - Call refresh after cluster deletion
    - Call refresh after cluster edit
    - _Requirements: 4.1, 4.2, 4.3_

  - [x] 3.2 Write property test for cluster list consistency
    - **Property 3: Cluster List Refresh After Modification**
    - **Validates: Requirements 4.1, 4.2, 4.3**

- [x] 4. Checkpoint - Verify critical fixes
  - Ensure all tests pass, ask the user if questions arise.

## Phase 2: Provider Icons and Detection

- [x] 5. Fix ZeroTrust Provider Icon Detection
  - [x] 5.1 Enhance AWS SSM detection
    - Add detection for "aws ssm", "aws-ssm", instance ID patterns (i-*, mi-*)
    - Add detection for "ssm start-session"
    - _Requirements: 5.1_

  - [x] 5.2 Enhance GCloud detection
    - Add detection for "gcloud", "iap-tunnel", "compute ssh"
    - _Requirements: 5.2_

  - [x] 5.3 Enhance Azure detection
    - Add detection for "az ", "azure", "bastion"
    - _Requirements: 5.3_

  - [x] 5.4 Wire provider detection to sidebar
    - Update `get_icon_for_connection()` to use `detect_provider()`
    - Ensure ZeroTrust connections show provider-specific icons
    - _Requirements: 5.1, 5.2, 5.3, 5.4_

  - [x] 5.5 Write property tests for provider detection
    - **Property 4: AWS SSM Command Detection**
    - **Property 5: GCloud Command Detection**
    - **Validates: Requirements 5.1, 5.2**

- [x] 6. Add Protocol-Specific Icons
  - [x] 6.1 Define distinct icons for each protocol
    - SSH: "utilities-terminal-symbolic"
    - RDP: "computer-symbolic" or custom icon
    - VNC: "video-display-symbolic"
    - SPICE: "preferences-desktop-remote-desktop-symbolic"
    - _Requirements: 7.1, 7.2, 7.3, 7.4_

  - [x] 6.2 Update sidebar icon selection
    - Modify `get_icon_for_protocol()` in sidebar.rs
    - _Requirements: 7.1, 7.2, 7.3, 7.4_

  - [x] 6.3 Write property test for protocol icons
    - **Property 9: Protocol Icons Are Distinct**
    - **Validates: Requirements 7.1, 7.2, 7.3, 7.4**

- [x] 7. Checkpoint - Verify icon fixes
  - Ensure all tests pass, ask the user if questions arise.

## Phase 3: Drag-and-Drop Visual Indicator

- [x] 8. Fix Drag-and-Drop Line Indicator
  - [x] 8.1 Replace frame with horizontal line
    - Create Separator widget with "drop-indicator" CSS class
    - Add CSS styling: accent color, 2px height
    - _Requirements: 9.1_

  - [x] 8.2 Implement line positioning
    - Calculate drop position (before/after) based on Y coordinate
    - Position line above or below target row
    - _Requirements: 9.2_

  - [x] 8.3 Implement group highlighting
    - Different visual style when dragging over group
    - _Requirements: 9.3_

  - [x] 8.4 Cleanup on drag end
    - Hide indicator on drag leave and drop completion
    - _Requirements: 9.4_

  - [x] 8.5 Write property test for drop indicator
    - **Property 11: Drop Indicator Position**
    - **Validates: Requirements 9.1, 9.2**

- [x] 9. Checkpoint - Verify drag-and-drop
  - Ensure all tests pass, ask the user if questions arise.

## Phase 4: KeePass Hierarchy Integration

- [x] 10. Implement KeePass Password Storage with Hierarchy
  - [x] 10.1 Create KeePassManager with hierarchy support
    - Add `build_entry_path()` method
    - Add `resolve_group_path()` method
    - _Requirements: 3.2_

  - [x] 10.2 Implement group creation
    - Add `ensure_groups_exist()` method
    - Create missing groups automatically
    - _Requirements: 3.4_

  - [x] 10.3 Implement password save with hierarchy
    - Save password to correct path based on connection groups
    - _Requirements: 3.1, 3.2_

  - [x] 10.4 Implement password retrieval
    - Load password from hierarchical path
    - _Requirements: 3.5_

  - [x] 10.5 Handle group changes
    - Move entry when connection group changes
    - _Requirements: 3.3_

  - [x] 10.6 Write property tests for KeePass hierarchy
    - **Property 1: KeePass Entry Path Matches Connection Hierarchy**
    - **Property 2: KeePass Entry Creation Creates All Parent Groups**
    - **Validates: Requirements 3.2, 3.3, 3.4**

- [x] 11. Checkpoint - Verify KeePass integration
  - Ensure all tests pass, ask the user if questions arise.

## Phase 5: Embedded RDP/VNC with Error Handling

- [x] 12. Fix Embedded RDP with Qt Error Handling
  - [x] 12.1 Isolate FreeRDP in separate thread
    - Create `FreeRdpThread` wrapper
    - Run FreeRDP operations in dedicated thread
    - _Requirements: 6.3_

  - [x] 12.2 Suppress Qt/Wayland warnings
    - Set QT_LOGGING_RULES environment variable
    - Set QT_QPA_PLATFORM to xcb for FreeRDP
    - _Requirements: 6.1, 6.2_

  - [x] 12.3 Implement fallback to external mode
    - Detect when embedded mode fails
    - Launch xfreerdp in external window
    - Show notification to user
    - _Requirements: 1.2, 6.4_

  - [x] 12.4 Implement proper cleanup
    - Cleanup resources on disconnect
    - Handle process termination
    - _Requirements: 1.6_

  - [x] 12.5 Write property tests for RDP error handling
    - **Property 7: FreeRDP Error Isolation**
    - **Property 8: Embedded RDP Fallback**
    - **Validates: Requirements 6.1, 6.2, 6.4**

- [x] 13. Fix Embedded VNC
  - [x] 13.1 Implement VNC viewer launch
    - Detect available VNC viewer
    - Launch with correct parameters
    - _Requirements: 2.1, 2.2_

  - [x] 13.2 Implement fallback to external mode
    - Fall back to external viewer if native fails
    - _Requirements: 2.2_

  - [x] 13.3 Implement cleanup
    - Cleanup on disconnect
    - _Requirements: 2.4_

  - [x] 13.4 Implement true embedded VNC using pure Rust client (vnc-rs)
    - Added `vnc-rs` dependency to rustconn-core with `vnc-embedded` feature
    - Created `rustconn-core/src/vnc_client/` module with:
      - `VncClient` - async VNC client using tokio
      - `VncClientConfig` - connection configuration
      - `VncClientEvent` - framebuffer updates, state changes
      - `VncClientCommand` - keyboard/mouse input forwarding
    - Updated `EmbeddedVncWidget` to use native VNC client
    - Renders VNC framebuffer to GTK4 DrawingArea using Cairo
    - Supports Tight, Zrle, CopyRect, and Raw encodings
    - _Requirements: 2.1, 2.2, 2.3, 2.4_

- [x] 14. Checkpoint - Verify embedded RDP/VNC
  - Ensure all tests pass, ask the user if questions arise.

## Phase 6: Provider Detection Persistence

- [x] 15. Persist Detected Provider
  - [x] 15.1 Add detected_provider field to ZeroTrustConfig
    - Add `detected_provider: Option<String>` field
    - Update serialization
    - _Requirements: 5.5_

  - [x] 15.2 Save detected provider on connection save
    - Detect provider when saving ZeroTrust connection
    - Persist to config
    - _Requirements: 5.5_

  - [x] 15.3 Use persisted provider for icon display
    - Load persisted provider on startup
    - Use for consistent icon display
    - _Requirements: 5.5_

  - [x] 15.4 Write property test for provider persistence
    - **Property 6: Provider Detection Persistence**
    - **Validates: Requirements 5.5**

- [x] 16. Final Checkpoint
  - Run full test suite
  - Verify all requirements are met
  - Manual testing of all fixes

