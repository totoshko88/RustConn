# Implementation Plan

## Phase 1: SSH Agent Module

- [x] 1. Implement SSH Agent Module in rustconn-core
  - [x] 1.1 Create ssh_agent module structure
    - Create `rustconn-core/src/ssh_agent/mod.rs` with module exports
    - Create `AgentStatus` struct with running, socket_path, keys fields
    - Create `AgentKey` struct with fingerprint, bits, key_type, comment fields
    - Create `SshKeySource` enum (File, Agent, Default)
    - Create `AgentError` error type using thiserror
    - _Requirements: 1.1, 1.6, 2.3_

  - [x] 1.2 Implement SSH agent output parsing
    - Implement `parse_agent_output()` to extract SSH_AUTH_SOCK from ssh-agent -s output
    - Handle different output formats (bash, csh)
    - _Requirements: 1.2_

  - [x] 1.3 Write property test for agent output parsing
    - **Property 1: SSH Agent Output Parsing**
    - **Validates: Requirements 1.2**

  - [x] 1.4 Implement SSH key list parsing
    - Implement `parse_key_list()` to parse ssh-add -l output
    - Extract fingerprint, bits, key type, and comment for each key
    - _Requirements: 1.6_

  - [x] 1.5 Write property test for key list parsing
    - **Property 2: SSH Key List Parsing**
    - **Validates: Requirements 1.6**

  - [x] 1.6 Implement SshAgentManager
    - Implement `start_agent()` to execute ssh-agent and capture socket
    - Implement `get_status()` to check agent status and list keys
    - Implement `add_key()` to add key with optional passphrase
    - Implement `remove_key()` to remove key from agent
    - Implement `list_key_files()` to find keys in ~/.ssh/
    - _Requirements: 1.2, 1.4, 1.6, 1.7_

  - [x] 1.7 Extend SshConfig with agent key support
    - Add `key_source: SshKeySource` field to SshConfig
    - Add `agent_key_fingerprint: Option<String>` field
    - Update serialization/deserialization
    - _Requirements: 2.3_

  - [x] 1.8 Write property test for agent key fingerprint storage
    - **Property 3: Agent Key Fingerprint Storage**
    - **Validates: Requirements 2.3**

  - [x] 1.9 Write unit tests for SSH agent module
    - Test agent output parsing with various formats
    - Test key list parsing with multiple keys
    - Test error handling for invalid input
    - _Requirements: 1.2, 1.6_

- [x] 2. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Phase 2: Export Module Foundation

- [x] 3. Create Export Module Structure
  - [x] 3.1 Create export module and traits
    - Create `rustconn-core/src/export/mod.rs` with module exports
    - Create `ExportFormat` enum (Ansible, SshConfig, Remmina, Asbru)
    - Create `ExportOptions` struct with format, include_passwords, output_path
    - Create `ExportResult` struct with exported_count, skipped_count, warnings
    - Create `ExportError` error type
    - Create `ExportTarget` trait with export methods
    - _Requirements: 3.1, 4.1, 5.1, 6.1_

  - [x] 3.2 Update lib.rs to export new modules
    - Add `pub mod export;` to lib.rs
    - Add `pub mod ssh_agent;` to lib.rs
    - _Requirements: 3.1_

## Phase 3: Ansible Export

- [x] 4. Implement Ansible Exporter
  - [x] 4.1 Create Ansible exporter
    - Create `rustconn-core/src/export/ansible.rs`
    - Implement `export_ini()` for INI format inventory
    - Implement `export_yaml()` for YAML format inventory
    - Implement `format_host_entry()` for single host
    - _Requirements: 3.1, 3.2_

  - [x] 4.2 Implement Ansible export with groups
    - Group connections by their group_id
    - Create [group_name] sections in INI format
    - Create nested structure in YAML format
    - _Requirements: 3.3_

  - [x] 4.3 Implement Ansible variable handling
    - Include ansible_host, ansible_user, ansible_port
    - Include ansible_ssh_private_key_file when key_path is set
    - Only include ansible_port when not default (22)
    - _Requirements: 3.2, 3.4_

  - [x] 4.4 Write property test for Ansible export round-trip
    - **Property 4: Ansible Export Round-Trip**
    - **Validates: Requirements 3.6, 9.5**

  - [x] 4.5 Write property test for Ansible export completeness
    - **Property 5: Ansible Export Completeness**
    - **Validates: Requirements 3.2, 3.3, 3.4**

- [x] 5. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Phase 4: SSH Config Export

- [x] 6. Implement SSH Config Exporter
  - [x] 6.1 Create SSH config exporter
    - Create `rustconn-core/src/export/ssh_config.rs`
    - Implement `export()` to generate SSH config content
    - Implement `format_host_entry()` for single Host block
    - _Requirements: 4.1, 4.2_

  - [x] 6.2 Implement SSH config directives
    - Use connection name as Host alias
    - Include HostName, User, Port, IdentityFile directives
    - Include ProxyJump if configured
    - Include custom options from SshConfig
    - _Requirements: 4.2, 4.3_

  - [x] 6.3 Implement special character handling
    - Escape spaces and special characters in values
    - Handle paths with spaces correctly
    - _Requirements: 4.4_

  - [x] 6.4 Write property test for SSH config export round-trip
    - **Property 6: SSH Config Export Round-Trip**
    - **Validates: Requirements 4.6, 9.5**

  - [x] 6.5 Write property test for SSH config export completeness
    - **Property 7: SSH Config Export Completeness**
    - **Validates: Requirements 4.2, 4.3, 4.4**

## Phase 5: Remmina Export

- [x] 7. Implement Remmina Exporter
  - [x] 7.1 Create Remmina exporter
    - Create `rustconn-core/src/export/remmina.rs`
    - Implement `export_connection()` to generate .remmina file content
    - Implement `generate_filename()` for connection filename
    - Implement `export_to_directory()` for batch export
    - _Requirements: 5.1_

  - [x] 7.2 Implement protocol-specific export
    - SSH: Include protocol=SSH, server, username, ssh_privatekey
    - RDP: Include protocol=RDP, server, username, domain, resolution
    - VNC: Include protocol=VNC, server (with port)
    - _Requirements: 5.2, 5.3, 5.4_

  - [x] 7.3 Write property test for Remmina export round-trip
    - **Property 8: Remmina Export Round-Trip**
    - **Validates: Requirements 5.6, 9.5**

  - [x] 7.4 Write property test for Remmina protocol handling
    - **Property 9: Remmina Export Protocol Handling**
    - **Validates: Requirements 5.2, 5.3, 5.4**

- [x] 8. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Phase 6: Asbru Export

- [x] 9. Implement Asbru Exporter
  - [x] 9.1 Create Asbru exporter
    - Create `rustconn-core/src/export/asbru.rs`
    - Implement `export()` to generate Asbru YAML content
    - Implement `connection_to_entry()` for connection conversion
    - Implement `group_to_entry()` for group conversion
    - _Requirements: 6.1, 6.2_

  - [x] 9.2 Implement Asbru group hierarchy
    - Generate UUID keys for entries
    - Set _is_group field (0 for connections, 1 for groups)
    - Set parent field for group membership
    - _Requirements: 6.3_

  - [x] 9.3 Write property test for Asbru export round-trip
    - **Property 10: Asbru Export Round-Trip**
    - **Validates: Requirements 6.5, 9.5**

  - [x] 9.4 Write property test for Asbru group hierarchy
    - **Property 11: Asbru Export Group Hierarchy**
    - **Validates: Requirements 6.2, 6.3**

## Phase 7: Connection Testing Module

- [x] 10. Implement Connection Tester
  - [x] 10.1 Create testing module
    - Create `rustconn-core/src/testing/mod.rs`
    - Create `TestResult` struct with connection_id, success, latency_ms, error
    - Create `TestSummary` struct with total, passed, failed, results
    - Create `TestError` error type
    - _Requirements: 8.1, 8.5_

  - [x] 10.2 Implement port connectivity test
    - Implement `test_port()` for TCP connection test
    - Measure connection latency
    - Handle timeout and connection refused errors
    - _Requirements: 8.3, 8.4_

  - [x] 10.3 Implement SSH handshake test
    - Implement `test_ssh()` for SSH protocol verification
    - Verify SSH banner exchange
    - _Requirements: 8.2_

  - [x] 10.4 Implement batch testing
    - Implement `test_batch()` for multiple connections
    - Run tests concurrently with configurable parallelism
    - Generate summary with pass/fail counts
    - _Requirements: 8.6_

  - [x] 10.5 Write property test for test result error details
    - **Property 16: Test Result Error Details**
    - **Validates: Requirements 8.5**

  - [x] 10.6 Write property test for batch test summary accuracy
    - **Property 17: Batch Test Summary Accuracy**
    - **Validates: Requirements 8.6**

- [x] 11. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Phase 8: CLI Binary Setup

- [x] 12. Create CLI Binary Crate
  - [x] 12.1 Create rustconn-cli crate
    - Create `rustconn-cli/Cargo.toml` with clap dependency
    - Create `rustconn-cli/src/main.rs` with CLI structure
    - Add rustconn-cli to workspace members in root Cargo.toml
    - _Requirements: 7.1_

  - [x] 12.2 Implement CLI argument parsing
    - Define `Cli` struct with clap derive macros
    - Define `Commands` enum with all subcommands
    - Define `OutputFormat` enum for list output
    - _Requirements: 7.8_

  - [x] 12.3 Implement help command
    - Configure clap for automatic help generation
    - Add descriptions for all commands and arguments
    - _Requirements: 7.8_

## Phase 9: CLI Commands Implementation

- [x] 13. Implement CLI List Command
  - [x] 13.1 Implement list command
    - Load connections from config
    - Format output as table (default), JSON, or CSV
    - Support protocol filter
    - _Requirements: 7.1_

  - [x] 13.2 Write property test for list output completeness
    - **Property 12: CLI List Output Completeness**
    - **Validates: Requirements 7.1**

- [x] 14. Implement CLI Add Command
  - [x] 14.1 Implement add command
    - Parse connection parameters from arguments
    - Create connection with specified protocol
    - Save to config file
    - _Requirements: 7.3_

  - [x] 14.2 Write property test for add connection
    - **Property 13: CLI Add Connection**
    - **Validates: Requirements 7.3**

- [x] 15. Implement CLI Export Command
  - [x] 15.1 Implement export command
    - Load connections from config
    - Call appropriate exporter based on format
    - Write output to specified path
    - _Requirements: 7.4_

  - [x] 15.2 Write property test for export format selection
    - **Property 14: CLI Export Format Selection**
    - **Validates: Requirements 7.4**
    - Covered by existing export tests in export_tests.rs

- [x] 16. Implement CLI Import Command
  - [x] 16.1 Implement import command
    - Detect or use specified format
    - Call appropriate importer
    - Merge imported connections with existing
    - Display import summary
    - _Requirements: 7.5, 9.4_

  - [x] 16.2 Write property test for import field preservation
    - **Property 18: Import Field Preservation**
    - **Validates: Requirements 9.1, 9.2, 9.3**

- [x] 17. Implement CLI Test Command
  - [x] 17.1 Implement test command
    - Find connection by name or ID
    - Support "all" for batch testing
    - Call ConnectionTester
    - Display results with colors
    - _Requirements: 7.6, 8.1_

- [x] 18. Implement CLI Connect Command
  - [x] 18.1 Implement connect command
    - Find connection by name or ID
    - Launch appropriate connection handler
    - _Requirements: 7.2_

- [x] 19. Implement CLI Error Handling
  - [x] 19.1 Implement error exit codes
    - Return exit code 0 on success
    - Return exit code 1 on general error
    - Return exit code 2 on connection failure
    - Display error messages to stderr
    - _Requirements: 7.7_

  - [x] 19.2 Write property test for error exit codes
    - **Property 15: CLI Error Exit Codes**
    - **Validates: Requirements 7.7**

- [x] 20. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Phase 10: GUI Integration

- [x] 21. Implement SSH Agent UI in Settings
  - [x] 21.1 Add SSH Agent section to settings dialog
    - Add "SSH Agent" section header
    - Display agent status (running/stopped)
    - Display socket path when running
    - Add "Start Agent" button
    - _Requirements: 1.1, 1.2_

  - [x] 21.2 Implement key management UI
    - Display list of loaded keys with fingerprint and comment
    - Add "Add Key" button with file chooser
    - Add "Remove" button for each key
    - Add passphrase dialog for protected keys
    - _Requirements: 1.3, 1.4, 1.5, 1.7_

  - [x] 21.3 Implement key list refresh
    - Refresh key list after add/remove operations
    - Show loading indicator during operations
    - Display error messages on failure
    - _Requirements: 1.6_

- [x] 22. Implement SSH Key Selection in Connection Dialog
  - [x] 22.1 Add key source dropdown to SSH connection dialog
    - Add "Key Source" dropdown (File, Agent, Default)
    - Show file chooser when "File" selected
    - Show agent key dropdown when "Agent" selected
    - _Requirements: 2.1, 2.2_

  - [x] 22.2 Implement agent key dropdown
    - Populate with keys from ssh-agent
    - Display fingerprint and comment for each key
    - Store selected fingerprint in connection
    - _Requirements: 2.2, 2.3_

  - [x] 22.3 Implement key availability check
    - Check if selected agent key is available on connect
    - Display error if key not found
    - Offer to select alternative key
    - _Requirements: 2.4, 2.5_
    - Implemented via SshAgentManager::get_status() validation

- [x] 23. Implement Export Dialog
  - [x] 23.1 Create export dialog
    - Create `rustconn/src/dialogs/export.rs`
    - Add format selection dropdown
    - Add output path selection
    - Add options checkboxes (include passwords, include groups)
    - _Requirements: 3.1, 4.1, 5.1, 6.1_

  - [x] 23.2 Implement export action
    - Call appropriate exporter based on format
    - Show progress indicator
    - Display success/error message
    - Open output location on success
    - _Requirements: 3.1, 4.1, 5.1, 6.1_

  - [x] 23.3 Add export menu item
    - Add "Export..." to File menu
    - Add keyboard shortcut (Ctrl+Shift+E)
    - _Requirements: 3.1_

- [x] 24. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Phase 11: Test Data and Integration Tests

- [x] 25. Create Test Fixtures
  - [x] 25.1 Create test data files
    - Create `rustconn-core/tests/fixtures/ansible_inventory.ini`
    - Create `rustconn-core/tests/fixtures/ssh_config`
    - Create `rustconn-core/tests/fixtures/remmina/test.remmina`
    - Create `rustconn-core/tests/fixtures/asbru.yml`
    - _Requirements: 9.1, 9.2_

  - [x] 25.2 Create sample connections for testing
    - SSH connection with key file
    - RDP connection with domain
    - VNC connection with custom port
    - _Requirements: 9.1, 9.2_

- [x] 26. Final Integration Tests
  - [x] 26.1 Write CLI integration tests
    - Test list command with various formats
    - Test add command with all parameters
    - Test export/import round-trip for all formats
    - Test error handling and exit codes
    - _Requirements: 7.1, 7.3, 7.4, 7.5, 7.7_

  - [x] 26.2 Write export/import integration tests
    - Test Ansible export/import round-trip
    - Test SSH config export/import round-trip
    - Test Remmina export/import round-trip
    - Test Asbru export/import round-trip
    - _Requirements: 3.6, 4.6, 5.6, 6.5, 9.5_

- [x] 27. Final Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.
