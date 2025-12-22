# Implementation Plan

## Phase 1: Foundation - Variables System

- [x] 1. Implement Variables System in rustconn-core
  - [x] 1.1 Create variables module structure
    - Create `rustconn-core/src/variables/mod.rs` with module exports
    - Create `Variable` struct with name, value, is_secret, description fields
    - Create `VariableScope` enum (Global, Document, Connection)
    - Create `VariableError` error type using thiserror
    - _Requirements: 6.1, 6.2, 6.3_

  - [x] 1.2 Implement VariableManager core functionality
    - Implement `resolve()` method for single variable lookup with scope chain
    - Implement `substitute()` method for replacing ${var} patterns in strings
    - Implement `parse_references()` to extract variable names from string
    - _Requirements: 6.2, 6.3_

  - [x] 1.3 Write property test for variable substitution
    - **Property 1: Variable Substitution Consistency**
    - **Validates: Requirements 6.3, 6.6**

  - [x] 1.4 Write property test for variable override
    - **Property 2: Variable Resolution with Override**
    - **Validates: Requirements 6.2**

  - [x] 1.5 Implement nested variable resolution
    - Implement recursive resolution with depth tracking
    - Implement cycle detection using visited set
    - Add MAX_NESTING_DEPTH constant (default: 10)
    - _Requirements: 6.4, 6.7_

  - [x] 1.6 Write property test for nested resolution
    - **Property 3: Nested Variable Resolution Depth**
    - **Validates: Requirements 6.4, 6.7**

  - [x] 1.7 Implement variable serialization
    - Add Serialize/Deserialize derives to Variable
    - Implement secure storage for secret variables
    - _Requirements: 6.5, 6.8_

  - [x] 1.8 Write property test for variable round-trip
    - **Property 4: Variable Serialization Round-Trip**
    - **Validates: Requirements 6.8**

  - [x] 1.9 Write unit tests for edge cases
    - Test undefined variable handling
    - Test empty variable names
    - Test special characters in values
    - _Requirements: 6.6_

- [x] 2. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Phase 2: Custom Properties

- [x] 3. Implement Custom Properties in rustconn-core
  - [x] 3.1 Create custom property types
    - Create `PropertyType` enum (Text, Url, Protected)
    - Create `CustomProperty` struct
    - Add `custom_properties: Vec<CustomProperty>` to Connection model
    - _Requirements: 10.2_

  - [x] 3.2 Implement property management
    - Add getter/setter methods to Connection
    - Implement protected property encryption using existing secret module
    - _Requirements: 10.2, 10.3_

  - [x] 3.3 Write property test for custom property serialization
    - **Property 22: Custom Property Type Preservation**
    - **Validates: Requirements 10.6**

  - [x] 3.4 Write unit tests for custom properties
    - Test all property types
    - Test protected property encryption
    - _Requirements: 10.2, 10.3_

## Phase 3: Session Logging

- [x] 4. Implement Session Logging in rustconn-core
  - [x] 4.1 Create logging module structure
    - Create `rustconn-core/src/session/logger.rs`
    - Create `LogConfig` struct with path_template, timestamp_format, max_size_mb, retention_days
    - Create `SessionLogger` struct with file handle and byte counter
    - Create `LogError` error type
    - _Requirements: 7.1, 7.2_

  - [x] 4.2 Implement log writing
    - Implement `write()` method with timestamp prefixing
    - Implement configurable timestamp formatting
    - _Requirements: 7.1, 7.2_

  - [x] 4.3 Write property test for timestamp formatting
    - **Property 16: Log Timestamp Formatting**
    - **Validates: Requirements 7.2**

  - [x] 4.4 Implement path template expansion
    - Support ${connection_name}, ${date}, ${time}, ${protocol} variables
    - Use VariableManager for substitution
    - _Requirements: 7.5_

  - [x] 4.5 Write property test for path template expansion
    - **Property 17: Log Path Template Expansion**
    - **Validates: Requirements 7.5**

  - [x] 4.6 Implement log rotation
    - Check file size before each write
    - Rotate when exceeding max_size_mb
    - Implement retention policy cleanup
    - _Requirements: 7.3_

  - [x] 4.7 Write property test for log rotation
    - **Property 18: Log Rotation Trigger**
    - **Validates: Requirements 7.3**

  - [x] 4.8 Implement proper file closing
    - Implement `close()` method with flush
    - Add Drop implementation for cleanup
    - _Requirements: 7.4_

- [x] 5. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Phase 4: Key Sequence Automation

- [x] 6. Implement Key Sequence System in rustconn-core
  - [x] 6.1 Create key sequence types
    - Create `rustconn-core/src/automation/mod.rs`
    - Create `rustconn-core/src/automation/key_sequence.rs`
    - Create `KeyElement` enum (Text, SpecialKey, Wait, Variable)
    - Create `SpecialKey` enum with all supported keys
    - Create `KeySequence` struct
    - Create `KeySequenceError` error type
    - _Requirements: 2.1, 2.3_

  - [x] 6.2 Implement key sequence parser
    - Parse text literals
    - Parse special keys like {ENTER}, {TAB}, {F1}
    - Parse {WAIT:1000} commands
    - Parse ${variable} references
    - _Requirements: 2.3, 2.4, 2.6_

  - [x] 6.3 Write property test for key sequence parsing
    - **Property 6: Key Sequence Validation**
    - **Validates: Requirements 2.6**

  - [x] 6.4 Implement key sequence serialization
    - Implement `to_string()` method
    - Ensure round-trip consistency
    - _Requirements: 2.7_

  - [x] 6.5 Write property test for key sequence round-trip
    - **Property 5: Key Sequence Parse Round-Trip**
    - **Validates: Requirements 2.7**

  - [x] 6.6 Implement variable substitution in key sequences
    - Integrate with VariableManager
    - Substitute before execution
    - _Requirements: 2.4_

  - [x] 6.7 Write property test for key sequence variable substitution
    - **Property 7: Key Sequence Variable Substitution**
    - **Validates: Requirements 2.4**

- [x] 7. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Phase 5: Expect Automation

- [x] 8. Implement Expect System in rustconn-core
  - [x] 8.1 Create expect types
    - Create `rustconn-core/src/automation/expect.rs`
    - Create `ExpectRule` struct with pattern, response, priority, timeout
    - Create `ExpectEngine` struct
    - Create `ExpectError` error type
    - _Requirements: 4.1, 4.2_

  - [x] 8.2 Implement pattern validation
    - Validate regex syntax on rule creation
    - Store compiled patterns for efficiency
    - _Requirements: 4.6_

  - [x] 8.3 Write property test for pattern validation
    - **Property 8: Expect Pattern Validation**
    - **Validates: Requirements 4.6**

  - [x] 8.4 Implement pattern matching
    - Match output against all rules
    - Return highest priority match
    - _Requirements: 4.2, 4.3_

  - [x] 8.5 Write property test for pattern matching priority
    - **Property 10: Expect Pattern Matching Priority**
    - **Validates: Requirements 4.3**

  - [x] 8.6 Implement expect rule serialization
    - Add Serialize/Deserialize derives
    - Ensure round-trip consistency
    - _Requirements: 4.7_

  - [x] 8.7 Write property test for expect rule round-trip
    - **Property 9: Expect Rule Serialization Round-Trip**
    - **Validates: Requirements 4.7**

## Phase 6: Connection Tasks

- [x] 9. Implement Connection Tasks in rustconn-core
  - [x] 9.1 Create task types
    - Create `rustconn-core/src/automation/tasks.rs`
    - Create `TaskTiming` enum (PreConnect, PostDisconnect)
    - Create `TaskCondition` struct
    - Create `ConnectionTask` struct
    - Create `TaskError` error type
    - _Requirements: 1.1, 1.5_

  - [x] 9.2 Implement task executor
    - Create `TaskExecutor` struct
    - Implement async `execute()` method using tokio::process
    - Integrate variable substitution
    - _Requirements: 1.2, 1.6_

  - [x] 9.3 Write property test for task variable substitution
    - **Property 11: Task Variable Substitution**
    - **Validates: Requirements 1.6**

  - [x] 9.4 Implement failure handling
    - Check exit code
    - Return error for non-zero exit
    - _Requirements: 1.3_

  - [x] 9.5 Write property test for task failure handling
    - **Property 12: Task Failure Handling**
    - **Validates: Requirements 1.3**

  - [x] 9.6 Implement conditional execution
    - Track active connections per folder
    - Execute only for first/last based on condition
    - _Requirements: 1.5_

  - [x] 9.7 Add task fields to Connection model
    - Add `pre_connect_task: Option<ConnectionTask>`
    - Add `post_disconnect_task: Option<ConnectionTask>`
    - _Requirements: 1.1_

- [x] 10. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Phase 7: Cluster Connections

- [x] 11. Implement Cluster System in rustconn-core
  - [x] 11.1 Create cluster types
    - Create `rustconn-core/src/cluster/mod.rs`
    - Create `Cluster` struct with id, name, connection_ids
    - Create `ClusterSession` struct
    - Create `ClusterError` error type
    - _Requirements: 3.1_

  - [x] 11.2 Implement cluster session management
    - Track individual session states
    - Handle partial failures gracefully
    - _Requirements: 3.2, 3.4_

  - [x] 11.3 Write property test for cluster session independence
    - **Property 13: Cluster Session Independence**
    - **Validates: Requirements 3.4**

  - [x] 11.4 Implement broadcast mode
    - Add broadcast_mode flag
    - Implement input distribution to all sessions
    - _Requirements: 3.3_

  - [x] 11.5 Add cluster serialization
    - Add Serialize/Deserialize derives
    - Store clusters in configuration
    - _Requirements: 3.1_

## Phase 8: Wake On LAN

- [x] 12. Implement WOL in rustconn-core
  - [x] 12.1 Create WOL types
    - Create `rustconn-core/src/wol/mod.rs`
    - Create `MacAddress` struct with parsing
    - Create `WolConfig` struct
    - Create `WolError` error type
    - _Requirements: 5.1_

  - [x] 12.2 Implement MAC address parsing
    - Support colon separator (AA:BB:CC:DD:EE:FF)
    - Support dash separator (AA-BB-CC-DD-EE-FF)
    - Validate format
    - _Requirements: 5.1_

  - [x] 12.3 Write property test for MAC address round-trip
    - **Property 14: MAC Address Parse Round-Trip**
    - **Validates: Requirements 5.1**

  - [x] 12.4 Implement magic packet generation
    - Create 102-byte packet (6 x 0xFF + 16 x MAC)
    - Send via UDP broadcast
    - _Requirements: 5.2_

  - [x] 12.5 Write property test for magic packet format
    - **Property 15: WOL Magic Packet Format**
    - **Validates: Requirements 5.2**

  - [x] 12.6 Add WOL config to Connection model
    - Add `wol_config: Option<WolConfig>` field
    - _Requirements: 5.1_

- [x] 13. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Phase 9: Connection Templates

- [x] 14. Implement Templates in rustconn-core
  - [x] 14.1 Create template types
    - Create `rustconn-core/src/models/template.rs`
    - Create `ConnectionTemplate` struct mirroring Connection fields
    - Add template_id field to Connection
    - _Requirements: 8.1_

  - [x] 14.2 Implement template application
    - Create `apply_template()` method
    - Copy all template fields to new connection
    - _Requirements: 8.2_

  - [x] 14.3 Write property test for template application
    - **Property 20: Template Application**
    - **Validates: Requirements 8.2**

  - [x] 14.4 Write property test for template independence
    - **Property 21: Template Independence**
    - **Validates: Requirements 8.3**

  - [x] 14.5 Implement template serialization
    - Add Serialize/Deserialize derives
    - Organize by protocol type
    - _Requirements: 8.4, 8.5_

  - [x] 14.6 Write property test for template serialization
    - **Property 19: Template Field Preservation**
    - **Validates: Requirements 8.5**

## Phase 10: Documents

- [x] 15. Implement Document System in rustconn-core
  - [x] 15.1 Create document types
    - Create `rustconn-core/src/document/mod.rs`
    - Create `Document` struct with connections, groups, variables, templates
    - Create `DocumentManager` struct
    - Create `DocumentError` error type
    - _Requirements: 11.1_

  - [x] 15.2 Implement document CRUD
    - Implement `create()`, `load()`, `save()` methods
    - Track dirty state
    - _Requirements: 11.1, 11.5_

  - [x] 15.3 Write property test for document dirty tracking
    - **Property 26: Document Dirty Tracking**
    - **Validates: Requirements 11.5**

  - [x] 15.4 Implement document encryption
    - Use existing secret module for encryption
    - Support password-based encryption
    - _Requirements: 11.3_

  - [x] 15.5 Write property test for document encryption round-trip
    - **Property 25: Document Encryption Round-Trip**
    - **Validates: Requirements 11.3**

  - [x] 15.6 Implement document serialization
    - Define JSON/YAML format
    - Implement export/import
    - _Requirements: 11.4, 11.6_

  - [x] 15.7 Write property test for document serialization round-trip
    - **Property 24: Document Serialization Round-Trip**
    - **Validates: Requirements 11.6**

- [x] 16. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Phase 11: Search System

- [x] 17. Implement Search in rustconn-core
  - [x] 17.1 Create search types
    - Create `rustconn-core/src/search/mod.rs`
    - Create `SearchQuery` struct with text and filters
    - Create `SearchFilter` enum
    - Create `SearchResult` struct with score and highlights
    - Create `SearchError` error type
    - _Requirements: 14.1, 14.4_

  - [x] 17.2 Implement query parsing
    - Parse plain text queries
    - Parse operators (protocol:, tag:, group:)
    - _Requirements: 14.4_

  - [x] 17.3 Write property test for search operator parsing
    - **Property 29: Search Operator Parsing**
    - **Validates: Requirements 14.4**

  - [x] 17.4 Implement fuzzy matching
    - Implement fuzzy score algorithm
    - Match against all searchable fields
    - _Requirements: 14.1, 14.2_

  - [x] 17.5 Write property test for fuzzy matching
    - **Property 27: Search Fuzzy Matching**
    - **Validates: Requirements 14.1**

  - [x] 17.6 Write property test for search scope coverage
    - **Property 28: Search Scope Coverage**
    - **Validates: Requirements 14.2**

  - [x] 17.7 Implement result ranking
    - Sort by relevance score
    - Include match highlights
    - _Requirements: 14.3_

  - [x] 17.8 Write property test for result ranking
    - **Property 30: Search Result Ranking**
    - **Validates: Requirements 14.3**

  - [x] 17.9 Implement custom property search
    - Include custom properties in search scope
    - _Requirements: 10.5_

  - [x] 17.10 Write property test for custom property search
    - **Property 23: Custom Property Search Inclusion**
    - **Validates: Requirements 10.5**

- [x] 18. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Phase 12: GUI - Variables UI

- [-] 19. Implement Variables UI in rustconn
  - [x] 19.1 Create global variables dialog
    - Create `rustconn/src/dialogs/variables.rs`
    - Implement list view with add/edit/delete
    - Support secret variable masking
    - _Requirements: 6.1, 6.5_

  - [x] 19.2 Add local variables to connection dialog
    - Add variables tab to connection dialog
    - Show inherited global variables
    - Allow local overrides
    - _Requirements: 6.2_

## Phase 13: GUI - Session Logging UI

- [x] 20. Implement Logging UI in rustconn
  - [x] 20.1 Add logging config to connection dialog
    - Add logging tab with enable checkbox
    - Add path template field with variable hints
    - Add timestamp format selector
    - Add size limit and retention fields
    - _Requirements: 7.1, 7.2, 7.3, 7.5_

  - [x] 20.2 Add log viewer
    - Create log file browser
    - Implement log content viewer
    - _Requirements: 7.1_

## Phase 14: GUI - Automation UI

- [-] 21. Implement Automation UI in rustconn
  - [x] 21.1 Add key sequence editor to connection dialog
    - Create key sequence input with special key buttons
    - Add syntax highlighting
    - Add test/preview functionality
    - _Requirements: 2.1, 2.5_

  - [x] 21.2 Add expect rules editor to connection dialog
    - Create rules list with add/edit/delete
    - Add pattern tester
    - Add priority ordering
    - _Requirements: 4.1_

  - [x] 21.3 Add tasks editor to connection dialog
    - Add pre-connect task section
    - Add post-disconnect task section
    - Add condition checkboxes
    - _Requirements: 1.1, 1.5_

## Phase 15: GUI - Cluster UI

- [-] 22. Implement Cluster UI in rustconn
  - [x] 22.1 Create cluster management dialog
    - Create cluster list view
    - Implement connection selection for cluster
    - _Requirements: 3.1_

  - [x] 22.2 Implement cluster view
    - Create tiled terminal layout
    - Add broadcast mode toggle with indicator
    - _Requirements: 3.5, 3.6_

## Phase 16: GUI - External Window Mode

- [x] 23. Implement External Window Mode in rustconn
  - [x] 23.1 Add window mode to connection dialog
    - Add window mode dropdown (Embedded, External, Fullscreen)
    - Add "remember position" checkbox
    - _Requirements: 9.1, 9.4_

  - [x] 23.2 Implement external window creation
    - Create new ApplicationWindow for external mode
    - Handle window close to terminate session
    - _Requirements: 9.2, 9.3_

  - [x] 23.3 Implement geometry persistence
    - Save window position/size on close
    - Restore on reconnection
    - _Requirements: 9.4_

  - [x] 23.4 Write property test for geometry persistence
    - **Property 33: Window Geometry Persistence**
    - **Validates: Requirements 9.4**

  - [x] 23.5 Implement fullscreen mode
    - Use GTK fullscreen API
    - Handle Wayland constraints
    - _Requirements: 9.5_

## Phase 17: GUI - Dashboard

- [x] 24. Implement Connection Dashboard in rustconn
  - [x] 24.1 Create dashboard view
    - Create `rustconn/src/dashboard.rs`
    - Display active sessions in grid/list
    - Show status indicators
    - _Requirements: 13.1_

  - [x] 24.2 Implement session statistics display
    - Show connection duration
    - Show bytes sent/received
    - Show host details
    - _Requirements: 13.2_

  - [x] 24.3 Write property test for session statistics
    - **Property 31: Dashboard Session Statistics**
    - **Validates: Requirements 13.2**

  - [x] 24.4 Implement session actions
    - Click to focus session
    - Disconnect button
    - _Requirements: 13.3_

  - [x] 24.5 Implement dashboard filtering
    - Add filter by protocol
    - Add filter by group
    - Add filter by status
    - _Requirements: 13.5_

  - [x] 24.6 Write property test for dashboard filtering
    - **Property 32: Dashboard Filter Application**
    - **Validates: Requirements 13.5**

- [x] 25. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Phase 18: GUI - Tray Icon

- [x] 26. Implement Tray Icon in rustconn
  - [x] 26.1 Add tray icon support
    - Use libappindicator or StatusNotifierItem for Wayland
    - Create tray icon with RustConn logo
    - _Requirements: 12.1_

  - [x] 26.2 Implement tray menu
    - Add recent connections submenu
    - Add quick connect option
    - Add show/hide main window
    - Add quit option
    - _Requirements: 12.2, 12.3_

  - [x] 26.3 Implement activity indication
    - Change icon when sessions active
    - Show session count in tooltip
    - _Requirements: 12.4_

  - [x] 26.4 Implement minimize to tray
    - Add setting for close behavior
    - Hide window instead of quit when enabled
    - _Requirements: 12.5_

## Phase 19: GUI - Search UI

- [x] 27. Implement Enhanced Search UI in rustconn
  - [x] 27.1 Enhance search entry
    - Add operator hints/autocomplete
    - Show search syntax help
    - _Requirements: 14.4_

  - [x] 27.2 Implement search results display
    - Highlight matched portions
    - Show relevance indicators
    - _Requirements: 14.3_

  - [x] 27.3 Implement search history
    - Store recent searches
    - Show history dropdown
    - _Requirements: 14.5_

## Phase 20: GUI - Custom Properties UI

- [x] 28. Implement Custom Properties UI in rustconn
  - [x] 28.1 Add custom properties editor to connection dialog
    - Create properties list with add/edit/delete
    - Support all property types
    - Mask protected properties
    - _Requirements: 10.1, 10.2, 10.3_

  - [x] 28.2 Display custom properties in connection details
    - Add properties section to details panel
    - Handle URL properties as clickable links
    - _Requirements: 10.4_

## Phase 21: GUI - Documents UI

- [x] 29. Implement Documents UI in rustconn
  - [x] 29.1 Update sidebar for multiple documents
    - Show document headers in tree
    - Support document switching
    - _Requirements: 11.2_

  - [x] 29.2 Implement document management
    - Add new document action
    - Add open document action
    - Add close document with save prompt
    - _Requirements: 11.1, 11.5_

  - [x] 29.3 Implement document protection dialog
    - Add password protection option
    - Prompt for password on open
    - _Requirements: 11.3_

  - [x] 29.4 Implement export/import
    - Add export document action
    - Add import document action
    - _Requirements: 11.4_

## Phase 22: GUI - WOL UI

- [x] 30. Implement WOL UI in rustconn
  - [x] 30.1 Add WOL config to connection dialog2
    - Add MAC address field with validation
    - Add broadcast address field
    - Add wait time field
    - _Requirements: 5.1, 5.3_

  - [x] 30.2 Implement WOL status feedback
    - Show WOL progress indicator
    - Display success/failure messages
    - _Requirements: 5.4, 5.5_

## Phase 23: GUI - Templates UI

- [x] 31. Implement Templates UI in rustconn
  - [x] 31.1 Create template management dialog
    - List templates by protocol
    - Add create/edit/delete actions
    - _Requirements: 8.1, 8.4_

  - [x] 31.2 Add "create from template" option
    - Add template selector to new connection dialog
    - Pre-populate fields from selected template
    - _Requirements: 8.2_

- [x] 32. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Phase 24: Clippy Compliance and Final Testing

- [x] 33. Clippy Compliance
  - [x] 33.1 Run Clippy on rustconn-core
    - Execute `cargo clippy -p rustconn-core --all-targets -- -D warnings`
    - Fix all warnings and errors
    - Document any necessary `#[allow(...)]` with justification
    - _Requirements: 15.1, 15.3, 15.4_

  - [x] 33.2 Run Clippy on rustconn
    - Execute `cargo clippy -p rustconn --all-targets -- -D warnings`
    - Fix all warnings and errors
    - Document any necessary `#[allow(...)]` with justification
    - _Requirements: 15.2, 15.3, 15.4_

  - [x] 33.3 Run full test suite
    - Execute `cargo test --workspace`
    - Ensure all tests pass
    - _Requirements: 15.1, 15.2_

  - [x] 33.4 Run property tests with extended iterations
    - Execute `cargo test -p rustconn-core --test property_tests -- --test-threads=1`
    - Verify 100+ iterations per property
    - _Requirements: All property requirements_

- [x] 34. Final Checkpoint - Ensure all tests pass
  - All 534 property tests and 491 unit tests pass
  - Clippy passes without warnings
  - All integration tests pass

