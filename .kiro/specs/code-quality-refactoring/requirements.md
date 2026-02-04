# Code Quality Refactoring - Requirements

## Overview

This spec addresses technical debt identified during a comprehensive code audit of RustConn v0.7.4.
The refactoring focuses on reducing code duplication, improving reliability, and cleaning up legacy code.

## User Stories

### US-1: Import File I/O Consolidation

**As a** maintainer  
**I want** a single helper function for reading import files  
**So that** error handling is consistent and code is DRY

**Acceptance Criteria:**
- 1.1 Create `read_import_file(path, source_name) -> Result<String, ImportError>` in `import/traits.rs`
- 1.2 Create async variant `read_import_file_async()` for future use
- 1.3 Replace duplicated code in `ssh_config.rs`, `ansible.rs`, `remmina.rs`, `asbru.rs`, `royalts.rs`
- 1.4 All import tests pass after refactoring
- 1.5 Error messages remain identical to preserve user experience

### US-2: Protocol Error Types Unification

**As a** developer  
**I want** a single error type for embedded protocol clients  
**So that** error handling is consistent across RDP, VNC, and SPICE

**Acceptance Criteria:**
- 2.1 Create `EmbeddedClientError` enum in new `rustconn-core/src/embedded_client_error.rs`
- 2.2 Include all common variants: `ConnectionFailed`, `AuthenticationFailed`, `ProtocolError`, `IoError`, `NotConnected`, `AlreadyConnected`, `InvalidConfig`, `ChannelError`, `Timeout`, `ServerDisconnected`, `Unsupported`
- 2.3 Include protocol-specific variants: `TlsError`, `UsbRedirectionError`, `SharedFolderError`, `NativeClientNotAvailable`
- 2.4 Create type aliases: `RdpClientError`, `VncClientError`, `SpiceClientError`
- 2.5 Update `rdp_client/mod.rs`, `vnc_client/mod.rs`, `spice_client/mod.rs` to re-export aliases
- 2.6 All existing code compiles without changes (backward compatible)
- 2.7 Property tests updated for new error type

### US-3: Atomic Config Writes

**As a** user  
**I want** my configuration files to be saved atomically  
**So that** a crash during save doesn't corrupt my data

**Acceptance Criteria:**
- 3.1 Modify `save_toml_file_async()` in `config/manager.rs` to use temp file + rename pattern
- 3.2 Write to `{path}.tmp` first, then rename to `{path}`
- 3.3 Handle rename errors gracefully with descriptive error message
- 3.4 Config save tests pass
- 3.5 No data loss on simulated crash (test with temp file)

### US-4: Legacy Code Cleanup

**As a** maintainer  
**I want** unused legacy types removed  
**So that** the codebase is cleaner and easier to understand

**Acceptance Criteria:**
- 4.1 Verify `TabDisplayMode`, `SessionWidgetStorage`, `TabLabelWidgets` are unused via grep
- 4.2 Remove unused types from `rustconn/src/terminal/types.rs`
- 4.3 All tests pass after removal
- 4.4 No compilation errors

### US-5: GTK Lifecycle Documentation

**As a** new contributor  
**I want** documentation explaining why GTK widget fields are marked `#[allow(dead_code)]`  
**So that** I don't accidentally remove them and cause segfaults

**Acceptance Criteria:**
- 5.1 Add module-level doc comment in `rustconn/src/lib.rs` or dedicated `gtk_patterns.rs`
- 5.2 Document that fields are kept alive for signal handlers and event controllers
- 5.3 Warn that removing these fields causes segfaults when signals fire

## Non-Functional Requirements

### NFR-1: Code Quality Gates
- All changes MUST pass `cargo clippy --all-targets` with zero warnings
- All changes MUST pass `cargo fmt --check`
- All changes MUST pass `cargo test`

### NFR-2: Backward Compatibility
- Type aliases MUST be used to maintain API compatibility
- No breaking changes to public API

### NFR-3: Documentation
- CHANGELOG.md MUST be updated with each change
- Code comments MUST explain non-obvious patterns

## Out of Scope

- GUI changes
- New features
- Performance optimizations beyond atomic writes
- X11/Wayland compatibility (already compliant)

## Dependencies

- None (internal refactoring only)

## Risks

| Risk | Mitigation |
|------|------------|
| Breaking existing imports | Run full test suite after each change |
| Type alias confusion | Clear documentation and re-exports |
| Atomic write race conditions | Use unique temp file names with PID |
