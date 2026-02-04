# Code Quality Refactoring - Tasks

## Task List

- [x] 1. Import File I/O Helper
  - [x] 1.1 Add `read_import_file()` function to `rustconn-core/src/import/traits.rs`
  - [x] 1.2 Add `read_import_file_async()` function for future use
  - [x] 1.3 Update `ssh_config.rs` to use `read_import_file()`
  - [x] 1.4 Update `ansible.rs` to use `read_import_file()`
  - [x] 1.5 Update `remmina.rs` to use `read_import_file()`
  - [x] 1.6 Update `asbru.rs` to use `read_import_file()`
  - [x] 1.7 Update `royalts.rs` to use `read_import_file()`
  - [x] 1.8 Run import tests to verify no regressions

- [x] 2. Unified Protocol Error Type
  - [x] 2.1 Create `rustconn-core/src/embedded_client_error.rs` with `EmbeddedClientError` enum
  - [x] 2.2 Update `rdp_client/error.rs` to re-export from `embedded_client_error`
  - [x] 2.3 Update `vnc_client/error.rs` to re-export from `embedded_client_error`
  - [x] 2.4 Update `spice_client/error.rs` to re-export from `embedded_client_error`
  - [x] 2.5 Update `rustconn-core/src/lib.rs` to export unified error type
  - [x] 2.6 Create property tests for `EmbeddedClientError` in `tests/properties/`

- [x] 3. Atomic Config Writes
  - [x] 3.1 Modify `save_toml_file_async()` in `config/manager.rs` to use temp file + rename
  - [x] 3.2 Add error handling for rename failures
  - [x] 3.3 Verify config save tests pass

- [x] 4. Legacy Code Cleanup
  - [x] 4.1 Verify `TabDisplayMode`, `SessionWidgetStorage`, `TabLabelWidgets` are unused
  - [x] 4.2 Remove unused types from `rustconn/src/terminal/types.rs`
  - [x] 4.3 Verify compilation succeeds

- [x] 5. GTK Lifecycle Documentation
  - [x] 5.1 Add module-level documentation to `rustconn/src/lib.rs` explaining dead_code pattern

- [x] 6. Final Verification
  - [x] 6.1 Run `cargo clippy --all-targets` and fix any warnings
  - [x] 6.2 Run `cargo fmt --check` and fix any formatting issues
  - [x] 6.3 Run `cargo test` and verify all tests pass
  - [x] 6.4 Update CHANGELOG.md with final changes
