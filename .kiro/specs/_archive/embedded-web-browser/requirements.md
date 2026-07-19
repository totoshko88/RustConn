# Requirements Document

## Introduction

This document specifies requirements for embedding a WebKitGTK 6.0 WebView widget inside RustConn tabs for Web protocol connections. Currently, Web connections open URLs in an external system browser. This feature adds an embedded browser mode as the default on Linux, with System and Custom browser as fallback options. The feature ships as v0.19.0 and is gated behind a `web-embedded` Cargo feature flag (disabled on macOS where WebKitGTK is unavailable).

## Glossary

- **Embedded_Web_Widget**: The GTK4 widget wrapping a WebKitGTK 6.0 WebView, implementing the `EmbeddedWidget` trait, displayed inside a RustConn tab.
- **WebView**: The WebKitGTK 6.0 `webkit::WebView` GTK4 widget that renders web content.
- **Navigation_Toolbar**: A compact horizontal toolbar above the WebView providing Back, Forward, Reload, Home, Autofill, Zoom, and Menu controls.
- **Browser_Mode**: The user-selected mode for opening Web connections: Embedded, System, or Custom.
- **Network_Session**: A WebKitGTK `NetworkSession` instance providing per-connection isolated cookie and session storage.
- **Autofill**: Automatic credential injection into web page login forms via JavaScript execution or HTTP Basic Auth signal handling.
- **Web_Config**: The `rustconn-core` model struct holding Web protocol connection settings including browser mode, JavaScript enabled flag, and user agent.
- **Session_Widget_Storage**: The enum in the terminal module that tracks which widget type backs a given tab session.
- **Protocol_Capabilities**: The struct describing what features a protocol supports (embedded viewer, split view, clipboard, etc.).

## Requirements

### Requirement 1: Browser Mode Selection

**User Story:** As a user, I want to choose between an embedded browser, the system browser, or a custom browser command for each Web connection, so that I can use the most convenient viewing method.

#### Acceptance Criteria

1. THE Web_Config SHALL provide a `browser_mode` field with three possible values: Embedded, System, and Custom, with a default value of System for newly created connections.
2. WHEN a user creates or edits a Web connection, THE Connection_Dialog SHALL display a Browser Mode dropdown (`adw::ComboRow`) in the Protocol tab with options: Embedded, System, and Custom, with the current connection's `browser_mode` value pre-selected.
3. WHEN the `web-embedded` feature is disabled at compile time, THE Connection_Dialog SHALL hide the Embedded option and set the selected value to System if the connection's stored `browser_mode` was Embedded.
4. WHEN Browser Mode is set to System, THE Web_Protocol SHALL open the URL using `xdg-open` or GTK4 `UriLauncher`.
5. WHEN Browser Mode is set to Custom, THE Web_Protocol SHALL open the URL by executing the user-specified browser command with the URL appended as an argument.
6. IF Browser Mode is set to Custom and the browser command field is empty or contains only whitespace, THEN THE Connection_Dialog SHALL prevent saving the connection and display an error message indicating that a browser command is required.
7. IF Browser Mode is set to Custom and the specified browser command fails to execute, THEN THE Web_Protocol SHALL display an error notification indicating the command could not be launched and SHALL NOT open any fallback browser.
8. WHEN Browser Mode is set to Embedded, THE Session_Manager SHALL create an Embedded_Web_Widget inside the tab instead of launching an external browser.

### Requirement 2: Embedded WebView Widget

**User Story:** As a user, I want web pages to render inside my RustConn tab, so that I can browse without switching to a separate application.

#### Acceptance Criteria

1. THE Embedded_Web_Widget SHALL implement the `EmbeddedWidget` trait (providing `widget()`, `state()`, `is_embedded()`, `disconnect()`, `reconnect()`, `send_ctrl_alt_del()`, and `protocol_name()` methods).
2. WHEN a Web connection with Embedded mode is activated, THE Embedded_Web_Widget SHALL validate that the configured URL is non-empty and begins with `http://`, `https://`, or `file://`, and load it in a WebKitGTK 6.0 WebView.
3. WHILE the WebView is loading a page, THE Embedded_Web_Widget SHALL report `EmbeddedConnectionState::Connecting`.
4. WHEN the WebView finishes loading, THE Embedded_Web_Widget SHALL report `EmbeddedConnectionState::Connected`.
5. IF the WebView encounters a load error (network failure, TLS error, DNS resolution failure), THEN THE Embedded_Web_Widget SHALL report `EmbeddedConnectionState::Error` and display the error description (truncated to 200 characters) in a status overlay using `draw_status_overlay`.
6. WHEN `disconnect()` is called, THE Embedded_Web_Widget SHALL stop all page loading, navigate to `about:blank`, and report `EmbeddedConnectionState::Disconnected`.
7. WHEN `reconnect()` is called, THE Embedded_Web_Widget SHALL report `EmbeddedConnectionState::Connecting` and reload the original configured URL.
8. IF the configured URL is empty or does not begin with a supported scheme (`http://`, `https://`, `file://`), THEN THE Embedded_Web_Widget SHALL report `EmbeddedConnectionState::Error` with an `EmbeddedError::ConfigurationError` indicating the URL is invalid, without initiating a network request.
9. WHEN `send_ctrl_alt_del()` is called, THE Embedded_Web_Widget SHALL perform no action (no-op), as the Web protocol does not support remote key injection.
10. IF the WebView has not finished loading within 60 seconds, THEN THE Embedded_Web_Widget SHALL report `EmbeddedConnectionState::Error` and display a timeout indication in the status overlay.

### Requirement 3: Navigation Toolbar

**User Story:** As a user, I want browser navigation controls (back, forward, reload, home) above the web view, so that I can navigate web pages without external browser chrome.

#### Acceptance Criteria

1. THE Navigation_Toolbar SHALL display buttons in the following order from left to right: Back, Forward, Reload, Home, page title label, Autofill, Zoom In, Zoom Out, and a Menu button.
2. WHILE the WebView cannot go back, THE Navigation_Toolbar SHALL disable the Back button.
3. WHILE the WebView cannot go forward, THE Navigation_Toolbar SHALL disable the Forward button.
4. WHEN the user activates the Back button, THE WebView SHALL navigate to the previous page in history.
5. WHEN the user activates the Forward button, THE WebView SHALL navigate to the next page in history.
6. WHEN the user activates the Reload button, THE WebView SHALL reload the current page.
7. WHEN the user activates the Home button, THE WebView SHALL navigate to the original configured URL for the current connection.
8. THE Navigation_Toolbar SHALL display the current page title as a label between the Home button and the Autofill button, truncated with an ellipsis if it exceeds the available horizontal space.
9. WHEN the user presses Ctrl+Plus or Ctrl+Equal or activates the Zoom In button, THE WebView SHALL increase the zoom level by 10 percent, up to a maximum of 300 percent.
10. WHEN the user presses Ctrl+Minus or activates the Zoom Out button, THE WebView SHALL decrease the zoom level by 10 percent, down to a minimum of 30 percent.
11. WHEN the user presses Ctrl+0, THE WebView SHALL reset the zoom level to 100 percent.
12. WHILE the WebView zoom level is at 300 percent, THE Navigation_Toolbar SHALL disable the Zoom In button.
13. WHILE the WebView zoom level is at 30 percent, THE Navigation_Toolbar SHALL disable the Zoom Out button.

### Requirement 4: Persistent Sessions

**User Story:** As a user, I want my login sessions and cookies to persist across RustConn restarts, so that I do not have to re-authenticate each time I open a Web connection.

#### Acceptance Criteria

1. WHEN creating an Embedded_Web_Widget, THE Widget SHALL create a WebKitGTK `NetworkSession` with `base_data_directory` set to `~/.local/share/rustconn/webkit/<connection-uuid>/` and `base_cache_directory` set to `~/.cache/rustconn/webkit/<connection-uuid>/`.
2. THE Network_Session SHALL store cookies in SQLite format within the connection-specific data directory and SHALL set the cookie acceptance policy to no third-party cookies.
3. WHEN the same Web connection is opened again after a RustConn restart, THE Widget SHALL reuse the existing persistent session directory, and THE Network_Session SHALL restore previously stored cookies and local storage data without requiring the user to re-authenticate.
4. WHEN a Web connection is deleted from RustConn, THE Connection_Manager SHALL remove both the data directory (`~/.local/share/rustconn/webkit/<connection-uuid>/`) and the cache directory (`~/.cache/rustconn/webkit/<connection-uuid>/`) for the corresponding connection.
5. IF the persistent storage directory cannot be created or is inaccessible, THEN THE Widget SHALL fall back to an ephemeral (non-persistent) NetworkSession and SHALL display a warning indicating that session data will not persist across restarts.
6. THE Network_Session SHALL enforce session isolation such that each connection's NetworkSession instance operates on its own storage directory, preventing one connection from reading or writing cookies or local storage belonging to another connection.

### Requirement 5: Autofill

**User Story:** As a user, I want my stored credentials to be automatically filled into web login forms, so that I can log in with one click.

#### Acceptance Criteria

1. WHEN the user activates the Autofill button and the connection has stored credentials, THE Embedded_Web_Widget SHALL inject JavaScript to fill HTML form fields matching selectors `input[type=password]`, `input[type=text][name*=user]`, `input[type=text][name*=login]`, `input[type=email]`, and `input[name=username]` with the stored username and password, and dispatch `input` and `change` events on each filled field.
2. IF the JavaScript injection does not locate at least one username field and one password field within 3 seconds of activation, THEN THE Embedded_Web_Widget SHALL display an inline notification indicating that no compatible login form was detected on the current page.
3. WHEN the WebView emits the WebKitGTK `authenticate` signal (HTTP Basic/Digest Auth), THE Embedded_Web_Widget SHALL respond with the stored credentials from the configured secret backend within the same signal callback, before the authentication challenge times out.
4. IF no credentials are stored for the connection, THEN THE Autofill button SHALL be visually disabled (non-interactive) and display a tooltip stating that no credentials are configured for this connection.
5. IF the secret backend is unavailable or returns an error when credentials are requested during autofill, THEN THE Embedded_Web_Widget SHALL display an inline error notification indicating that credential retrieval failed, and SHALL NOT inject any partial or empty values into form fields.
6. THE Autofill module SHALL use `SecretString` for credential handling and zeroize all temporary credential values within the same function scope immediately after injection completes or fails.
7. WHEN the user activates the Autofill button, THE Embedded_Web_Widget SHALL complete the credential retrieval and form injection within 2 seconds of button activation.

### Requirement 6: Split View Support

**User Story:** As a user, I want to split the embedded web browser alongside terminals or other sessions, so that I can view documentation while working.

#### Acceptance Criteria

1. THE Session_Widget_Storage enum SHALL include an `EmbeddedWeb` variant wrapping an `Rc<Embedded_Web_Widget>`.
2. WHEN the session widget storage is `EmbeddedWeb`, THE Split_Eligibility function SHALL return `SplitEligibility::Embeddable`.
3. IF the `web-embedded` feature is enabled at compile time, THEN THE Protocol_Capabilities for the Web protocol SHALL report `embedded: true` and `split_view: true`.
4. IF the `web-embedded` feature is disabled at compile time, THEN THE Protocol_Capabilities for the Web protocol SHALL report `embedded: false` and `split_view: false`.
5. IF the session widget storage does not contain an `EmbeddedWeb` entry for a Web session AND the session has a VTE terminal, THEN THE Split_Eligibility function SHALL return `SplitEligibility::Embeddable` via the existing terminal fallback.

### Requirement 7: Web Configuration Model Extension

**User Story:** As a developer, I want the Web connection model to include embedded browser settings, so that per-connection preferences are persisted.

#### Acceptance Criteria

1. THE Web_Config struct SHALL include a `browser_mode` field of type `WebBrowserMode` (enum with variants: Embedded, System, Custom).
2. THE Web_Config struct SHALL include a `javascript_enabled` field of type `bool`, defaulting to `true`.
3. THE Web_Config struct SHALL include a `user_agent` field of type `Option<String>` with a maximum length of 512 characters, defaulting to `None` (use WebKitGTK default).
4. IF the `web-embedded` feature is enabled at compile time, THEN THE `WebBrowserMode` enum SHALL default to `Embedded`.
5. IF the `web-embedded` feature is disabled at compile time, THEN THE `WebBrowserMode` enum SHALL default to `System`.
6. WHEN deserializing a Web_Config that lacks the `browser_mode` field, THE Deserializer SHALL apply the compile-time default value determined by the `web-embedded` feature flag.
7. IF the `user_agent` field contains a value exceeding 512 characters, THEN THE Deserializer SHALL reject the input and return an error indicating the user agent string exceeds the maximum allowed length.
8. WHEN serializing a Web_Config, THE Serializer SHALL include all three fields (`browser_mode`, `javascript_enabled`, `user_agent`) in the output regardless of whether their values match defaults.
9. IF a Web_Config is deserialized with a `browser_mode` value that does not match any `WebBrowserMode` variant, THEN THE Deserializer SHALL return an error indicating an unrecognized browser mode value.

### Requirement 8: Feature Flag and Platform Gating

**User Story:** As a developer, I want the embedded browser to be behind a feature flag, so that macOS builds and minimal Linux builds compile without WebKitGTK dependencies.

#### Acceptance Criteria

1. THE `rustconn` crate SHALL define a `web-embedded` feature in `Cargo.toml` that depends on WebKitGTK 6.0 bindings and propagates to `rustconn-core` as `rustconn-core/web-embedded`.
2. THE `web-embedded` feature SHALL be included in the `default` feature list in `rustconn/Cargo.toml`; macOS builds SHALL exclude it by building with `--no-default-features` and explicitly listing all other desired features.
3. WHEN `web-embedded` is disabled, THE Embedded_Web_Widget module and all types, imports, and match arms referencing WebKitGTK SHALL not be compiled (all gated with `#[cfg(feature = "web-embedded")]`), ensuring the crate compiles without WebKitGTK headers or libraries installed.
4. WHEN `web-embedded` is disabled, THE Web protocol handler SHALL offer only System and Custom browser modes, and THE `WebBrowserMode::Embedded` enum variant SHALL be excluded from compilation via conditional compilation.
5. WHEN `web-embedded` is enabled, THE crate SHALL compile and link against WebKitGTK 6.0 without requiring additional feature flags or manual dependency configuration beyond the Cargo feature.

### Requirement 9: JavaScript Control

**User Story:** As a user, I want to enable or disable JavaScript per connection, so that I can control page behavior for security-sensitive internal tools.

#### Acceptance Criteria

1. WHEN a user edits a Web connection, THE Connection_Dialog SHALL display a JavaScript toggle switch (adw::SwitchRow) in the Protocol tab, with the toggle reflecting the current `javascript_enabled` value of the connection's WebConfig.
2. WHEN `javascript_enabled` is set to `false` in WebConfig, THE Embedded_Web_Widget SHALL set the WebKitGTK `enable-javascript` property to `false` before loading any content in the WebView, preventing all JavaScript execution for that session.
3. WHEN `javascript_enabled` is set to `true` in WebConfig, THE Embedded_Web_Widget SHALL set the WebKitGTK `enable-javascript` property to `true` before loading content, allowing JavaScript execution in the WebView.
4. WHEN a new Web connection is created without an explicit `javascript_enabled` value, THE System SHALL default `javascript_enabled` to `true`.
5. WHEN the user toggles the JavaScript switch in the Connection_Dialog and saves the connection, THE System SHALL persist the updated `javascript_enabled` value in WebConfig so that subsequent sessions for that connection use the saved setting.

### Requirement 10: CI and Packaging

**User Story:** As a maintainer, I want CI and packaging configurations updated, so that the embedded web browser compiles and packages correctly across all targets.

#### Acceptance Criteria

1. THE GitHub Actions install-deps action SHALL include `libwebkitgtk-6.0-dev` in the list of installed packages.
2. THE macOS Homebrew formula SHALL build with `--no-default-features` and explicitly list features excluding `web-embedded`.
3. THE Flatpak manifest SHALL require no changes (the GNOME Platform 50 runtime already includes WebKitGTK 6.0).

### Requirement 11: Version and Release

**User Story:** As a maintainer, I want the release properly versioned as v0.19.0 with a changelog entry, so that the feature is traceable.

#### Acceptance Criteria

1. THE `[workspace.package].version` field in the root `Cargo.toml` SHALL be set to `"0.19.0"`, and all workspace member crates (`rustconn`, `rustconn-core`, `rustconn-cli`, `rustconn-pty-sys`) SHALL inherit or declare the same `0.19.0` version, so that `cargo metadata` reports a single consistent version across the workspace.
2. THE `CHANGELOG.md` SHALL contain a `## [0.19.0] - YYYY-MM-DD` section (with the actual release date) that includes at minimum an `### Added` subsection describing the embedded WebKitGTK 6.0 web browser feature, following the Keep a Changelog 1.1.0 format used by existing entries.
3. WHEN the version is bumped, THE changelog entry SHALL be propagated to `debian/changelog`, `packaging/obs/debian.changelog`, `packaging/obs/rustconn.changes`, `packaging/obs/rustconn.spec`, and `rustconn/assets/io.github.totoshko88.RustConn.metainfo.xml` before the release tag is created.
4. THE release SHALL be tagged as `v0.19.0` in Git and follow the existing release workflow: tag creation, CI build for all targets, and artifact publishing.
5. IF any workspace member crate has a version that differs from `0.19.0` after the bump, THEN THE build verification (`cargo check --all-targets`) SHALL fail, indicating a version synchronization error.
