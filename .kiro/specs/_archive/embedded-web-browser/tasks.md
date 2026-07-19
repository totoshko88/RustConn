# Implementation Plan: Embedded Web Browser

## Overview

Embed a WebKitGTK 6.0 WebView inside RustConn tabs for Web protocol connections. Implementation follows the established `EmbeddedWidget` trait pattern (RDP/VNC), adds per-connection persistent sessions, navigation toolbar, credential autofill, and full conditional compilation behind the `web-embedded` feature flag. The feature ships as v0.19.0.

## Tasks

- [x] 1. Feature flag setup and model changes in rustconn-core
  - [x] 1.1 Add `WebBrowserMode` enum and extend `WebConfig` in `rustconn-core/src/models/protocol.rs`
    - Define `WebBrowserMode` enum with `Embedded` (cfg-gated), `System`, `Custom` variants
    - Add `browser_mode: WebBrowserMode`, `javascript_enabled: bool`, `user_agent: Option<String>` fields to `WebConfig`
    - Implement `Default` for `WebBrowserMode` with compile-time conditional default
    - Add custom deserialization validation: reject `user_agent` > 512 chars, reject unknown `browser_mode` variants
    - Add `#[serde(default)]` for `browser_mode` to handle missing field on deserialization
    - Serialize all fields unconditionally (remove `skip_serializing_if` for new fields)
    - _Requirements: 7.1, 7.2, 7.3, 7.4, 7.5, 7.6, 7.7, 7.8, 7.9_

  - [x] 1.2 Add `web-embedded` feature flag to `rustconn-core/Cargo.toml`
    - Add `[features] web-embedded = []` section
    - _Requirements: 8.1, 8.3_

  - [x] 1.3 Update `ProtocolCapabilities` in `rustconn-core/src/protocol/web.rs`
    - Add `#[cfg(feature = "web-embedded")]` conditionals on `embedded` and `split_view` fields
    - Add `file://` to accepted URL schemes in `validate_connection`
    - _Requirements: 6.3, 6.4, 2.2_

  - [x] 1.4 Write property tests for `WebConfig` serialization (Properties 4, 5, 6)
    - **Property 4: WebBrowserMode Serialization Round-Trip** — serialize/deserialize any valid `WebConfig` and assert equality
    - **Property 5: User Agent Length Validation** — strings > 512 chars must fail deserialization
    - **Property 6: Browser Mode Compile-Time Default** — JSON without `browser_mode` key deserializes to compile-time default
    - **Validates: Requirements 7.1, 7.4, 7.5, 7.6, 7.7, 7.8**

- [x] 2. Feature flag setup in rustconn GUI crate
  - [x] 2.1 Add `web-embedded` feature and `webkit6` dependency to `rustconn/Cargo.toml`
    - Add `webkit6 = { version = "0.4", optional = true }` dependency
    - Add `web-embedded = ["webkit6", "rustconn-core/web-embedded"]` feature
    - Include `web-embedded` in the `default` feature list
    - _Requirements: 8.1, 8.2, 8.5_

- [x] 3. Implement embedded web widget core
  - [x] 3.1 Create `rustconn/src/embedded_web/mod.rs` with `EmbeddedWebWidget` struct
    - Define `EmbeddedWebWidget` struct with all fields from design (container, toolbar, web_view, network_session, state, home_url, connection_uuid, callbacks, load_timeout, autofill, reconnect_banner)
    - Implement `EmbeddedWebWidget::new()` constructor: validate URL, create network session, configure settings, set up WebView, connect signals
    - Implement `validate_url()` — accept `http://`, `https://`, `file://` schemes; reject empty or unsupported
    - Implement `create_network_session()` — persistent `NetworkSession` at `~/.local/share/rustconn/webkit/<uuid>/` and `~/.cache/rustconn/webkit/<uuid>/`; fall back to ephemeral on failure with warning toast
    - Implement `start_load_timeout()` / `cancel_load_timeout()` — 60-second timer
    - Connect `load-changed` signal: STARTED → Connecting, FINISHED → Connected, FAILED → Error with truncated description (200 chars)
    - Implement `EmbeddedWidget` trait: `widget()`, `state()`, `is_embedded()` (always true), `disconnect()` (stop loading, navigate to about:blank), `reconnect()` (reload home URL), `send_ctrl_alt_del()` (no-op), `protocol_name()` ("Web")
    - Gate entire module with `#[cfg(feature = "web-embedded")]`
    - _Requirements: 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 2.7, 2.8, 2.9, 2.10, 4.1, 4.2, 4.3, 4.5, 4.6_

  - [x] 3.2 Write property test for URL validation (Property 1)
    - **Property 1: URL Validation Round-Trip** — any string accepted by `validate_url()` starts with a supported scheme and is non-empty; any string without a supported scheme or empty is rejected
    - **Validates: Requirements 2.2, 2.8**

  - [x] 3.3 Write property test for session isolation (Property 3)
    - **Property 3: Session Isolation** — for any two distinct UUIDs, `create_network_session` produces non-overlapping directory paths
    - **Validates: Requirements 4.1, 4.6**

- [x] 4. Implement navigation toolbar
  - [x] 4.1 Create `rustconn/src/embedded_web/navigation.rs` with `NavigationToolbar`
    - Build toolbar layout: Back, Forward, Reload, Home, title label, Autofill, Zoom In, Zoom Out, Menu button
    - All icon-only buttons must have `set_tooltip_text` and `update_property` for accessibility
    - Implement `bind_to_webview()`: connect `can-go-back`/`can-go-forward` property notifications to button sensitivity, connect `title` property to label, connect zoom button clicks
    - Implement zoom logic: step 0.1, range [0.3, 3.0], disable buttons at boundaries
    - Add keyboard shortcuts: Ctrl+Plus/Ctrl+Equal (zoom in), Ctrl+Minus (zoom out), Ctrl+0 (reset)
    - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5, 3.6, 3.7, 3.8, 3.9, 3.10, 3.11, 3.12, 3.13_

  - [x] 4.2 Write property test for zoom clamping (Property 2)
    - **Property 2: Zoom Level Clamping** — any sequence of zoom in/out/reset operations keeps level within [0.3, 3.0] and each step changes by exactly 0.1 or is unchanged at boundary
    - **Validates: Requirements 3.9, 3.10, 3.11, 3.12, 3.13**

  - [x] 4.3 Write property test for navigation button sensitivity (Property 7)
    - **Property 7: Navigation Button Sensitivity Consistency** — for any `(can_go_back, can_go_forward)` state, corresponding buttons are disabled when false
    - **Validates: Requirements 3.2, 3.3**

- [x] 5. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 6. Implement WebView settings and autofill
  - [x] 6.1 Create `rustconn/src/embedded_web/settings.rs` with `apply_settings()`
    - Apply `javascript_enabled` to WebKitSettings
    - Apply `user_agent` override if present
    - Set hardened defaults: disable developer extras, disallow modal dialogs
    - _Requirements: 9.2, 9.3, 9.4, 7.3_

  - [x] 6.2 Create `rustconn/src/embedded_web/autofill.rs` with `AutofillManager`
    - Implement `AutofillManager::new()` with optional credentials
    - Implement `is_available()` — returns false if no credentials configured
    - Implement `inject_credentials()` — JavaScript injection targeting form selectors, using `SecretString` and `Zeroizing<String>`, dispatching `input`/`change` events
    - Implement `handle_authenticate()` — respond to WebKitGTK `authenticate` signal with stored credentials
    - Add 3-second timeout for field detection with inline notification on failure
    - Ensure no partial injection on secret backend error
    - Zeroize all temporary credential values within function scope
    - _Requirements: 5.1, 5.2, 5.3, 5.4, 5.5, 5.6, 5.7_

  - [x] 6.3 Write property test for autofill all-or-nothing (Property 8)
    - **Property 8: Autofill Credential Handling (No Partial Injection)** — if secret backend returns error, zero values are injected; either all credentials injected or none
    - **Validates: Requirements 5.5, 5.6**

- [x] 7. Integrate with session management and split view
  - [x] 7.1 Extend `SessionWidgetStorage` enum in `rustconn/src/terminal/mod.rs`
    - Add `#[cfg(feature = "web-embedded")] EmbeddedWeb(Rc<EmbeddedWebWidget>)` variant
    - Update `SplitEligibility` match arm to return `Embeddable` for `EmbeddedWeb`
    - _Requirements: 6.1, 6.2, 6.5_

  - [x] 7.2 Wire `EmbeddedWebWidget` creation into session manager
    - When Browser Mode is Embedded, create `EmbeddedWebWidget` inside the tab
    - Store in `SessionWidgetStorage::EmbeddedWeb`
    - Handle connection deletion: remove `~/.local/share/rustconn/webkit/<uuid>/` and `~/.cache/rustconn/webkit/<uuid>/`
    - _Requirements: 1.8, 4.4_

- [x] 8. Update connection dialog
  - [x] 8.1 Extend `rustconn/src/dialogs/connection/web.rs` with Browser Mode and JavaScript controls
    - Add `adw::ComboRow` for Browser Mode (Embedded/System/Custom); hide Embedded when `web-embedded` disabled
    - Add `adw::SwitchRow` for JavaScript toggle, reflecting `javascript_enabled` value
    - Add `adw::EntryRow` for user agent (optional)
    - Make browser command entry required when Custom mode selected; prevent save if empty/whitespace
    - Fallback: if stored `browser_mode` is Embedded but feature disabled, select System
    - Update return type to struct `WebOptionsWidgets` with all widget references
    - _Requirements: 1.1, 1.2, 1.3, 1.6, 9.1, 9.5_

- [x] 9. Update web protocol handler for new browser modes
  - [x] 9.1 Update `WebProtocol::build_command()` in `rustconn-core/src/protocol/web.rs`
    - When mode is System: use `xdg-open` / `UriLauncher`
    - When mode is Custom: execute user command with URL appended; error notification if command fails, no fallback
    - When mode is Embedded: return None (handled by session manager)
    - _Requirements: 1.4, 1.5, 1.7_

- [x] 10. Checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

- [x] 11. Register the embedded_web module
  - [x] 11.1 Add `pub mod embedded_web;` to `rustconn/src/lib.rs` or appropriate parent, gated with `#[cfg(feature = "web-embedded")]`
    - Ensure the module tree compiles with and without the feature flag
    - _Requirements: 8.3, 8.4_

- [x] 12. CI and packaging updates
  - [x] 12.1 Update `.github/actions/install-deps/action.yml`
    - Add `libwebkitgtk-6.0-dev` to the list of installed packages
    - _Requirements: 10.1_

  - [x] 12.2 Update macOS Homebrew formula `packaging/macos/rustconn.rb`
    - Build with `--no-default-features` and explicitly list features excluding `web-embedded`
    - _Requirements: 10.2, 8.2_

- [x] 13. Version bump and changelog
  - [x] 13.1 Bump workspace version to `0.19.0` and update changelog
    - Set `[workspace.package].version = "0.19.0"` in root `Cargo.toml`
    - Add `## [0.19.0] - YYYY-MM-DD` section to `CHANGELOG.md` with `### Added` subsection describing the embedded web browser feature
    - Propagate version to `debian/changelog`, `packaging/obs/debian.changelog`, `packaging/obs/rustconn.changes`, `packaging/obs/rustconn.spec`, and `rustconn/assets/io.github.totoshko88.RustConn.metainfo.xml`
    - _Requirements: 11.1, 11.2, 11.3_

- [x] 14. Final checkpoint - Ensure all tests pass
  - Ensure all tests pass, ask the user if questions arise.

## Notes

- Tasks marked with `*` are optional and can be skipped for faster MVP
- Each task references specific requirements for traceability
- Checkpoints ensure incremental validation
- Property tests validate universal correctness properties from the design document
- Unit tests validate specific examples and edge cases
- All `embedded_web` module code is gated with `#[cfg(feature = "web-embedded")]`
- The `webkit6` crate provides Rust bindings for WebKitGTK 6.0
- Credential handling must use `SecretString` from `secrecy` crate throughout
- All icon-only buttons require both tooltip and accessible label per GNOME HIG

## Task Dependency Graph

```json
{
  "waves": [
    { "id": 0, "tasks": ["1.1", "1.2"] },
    { "id": 1, "tasks": ["1.3", "2.1", "1.4"] },
    { "id": 2, "tasks": ["3.1"] },
    { "id": 3, "tasks": ["3.2", "3.3", "4.1", "6.1", "6.2"] },
    { "id": 4, "tasks": ["4.2", "4.3", "6.3"] },
    { "id": 5, "tasks": ["7.1", "8.1", "9.1"] },
    { "id": 6, "tasks": ["7.2", "11.1"] },
    { "id": 7, "tasks": ["12.1", "12.2"] },
    { "id": 8, "tasks": ["13.1"] }
  ]
}
```
