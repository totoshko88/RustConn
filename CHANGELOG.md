# Changelog

All notable changes to RustConn will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.6.4] - 2026-01-17

### Added
- **Snap Package** - New distribution format for easy installation via Snapcraft:
  - Classic confinement for full system access (SSH keys, network, etc.)
  - Automatic updates via Snap Store
  - Available via `sudo snap install rustconn --classic`
- **GitHub Actions Snap Workflow** - Automated Snap package builds:
  - Builds on tag push (`v*`) and manual trigger
  - Uploads artifacts for testing
  - Publishes to Snap Store stable channel on release tags
- **RDP/VNC Performance Modes** - New dropdown in connection dialog to optimize for different network conditions:
  - Quality: Best visual quality (32-bit color for RDP, Tight encoding with high quality for VNC)
  - Balanced: Good balance of quality and performance (24-bit color, medium compression)
  - Speed: Optimized for slow connections (16-bit color for RDP, ZRLE encoding with high compression for VNC)

### Changed
- Updated documentation with Snap installation instructions

### Fixed
- **RDP Initial Resolution** - Embedded RDP sessions now start with correct resolution matching actual widget size
  - Previously used saved window settings which could differ from actual content area
  - Now waits for GTK layout (100ms) to get accurate widget dimensions
- **RDP Dynamic Resolution** - Window resize now triggers automatic reconnect with new resolution
  - Debounced reconnect after 500ms of no resize activity
  - Preserves shared folders and credentials during reconnect
  - Works around Windows RDP servers not supporting Display Control channel
- **Sidebar Fixed Width** - Sidebar no longer resizes when window is resized
  - Content area (RDP/VNC/terminal) now properly expands to fill available space
- **RDP Cursor Colors** - Fixed inverted cursor colors in embedded RDP sessions (BGRA→ARGB conversion)

### Updated Dependencies
- `ironrdp` 0.13 → 0.14 (embedded RDP client)
- `ironrdp-tokio` 0.7 → 0.8
- `ironrdp-tls` 0.1 → 0.2
- `sspi` 0.16 → 0.18.7 (Windows authentication)
- `picky` 7.0.0-rc.17 → 7.0.0-rc.20
- `picky-krb` 0.11 → 0.12 (Kerberos support)
- `hickory-proto` 0.24 → 0.25
- `hickory-resolver` 0.24 → 0.25
- `cc` 1.2.52 → 1.2.53
- `find-msvc-tools` 0.1.7 → 0.1.8
- `js-sys` 0.3.83 → 0.3.85
- `rand_core` 0.9.3 → 0.9.5
- `rustls-pki-types` 1.13.2 → 1.14.0
- `rustls-webpki` 0.103.8 → 0.103.9
- `wasm-bindgen` 0.2.106 → 0.2.108
- `web-sys` 0.3.83 → 0.3.85
- `wit-bindgen` 0.46.0 → 0.51.0

## [0.6.3] - 2026-01-16

### Added
- **Bitwarden CLI Integration** - New secret backend for Bitwarden password manager:
  - Full `SecretBackend` trait implementation with async credential resolution
  - Vault status checking (locked/unlocked/unauthenticated)
  - Session token management with automatic refresh
  - Secure credential lookup by connection name or host
  - Settings UI with vault status indicator and unlock functionality
  - Master password persistence with encrypted storage (machine-specific)
- **Password Manager Detection** - Automatic detection of installed password managers:
  - Detects GNOME Secrets, KeePassXC, KeePass2, Bitwarden CLI, 1Password CLI
  - Shows installed managers with version info in Settings → Secrets tab
  - New "Installed Password Managers" section for quick overview
- **Enhanced Secrets Settings UI** - Improved backend selection experience:
  - Backend dropdown now includes all 4 options: KeePassXC, libsecret, KDBX File, Bitwarden
  - Dynamic configuration groups based on selected backend
  - Bitwarden-specific settings with vault status checking
- **Universal Password Vault Button** - Sidebar button now opens appropriate password manager:
  - Opens KeePassXC/GNOME Secrets for KeePassXC backend
  - Opens Seahorse/GNOME Settings for libsecret backend
  - Opens Bitwarden web vault for Bitwarden backend

### Changed
- `SecretBackendType` enum extended with `Bitwarden` variant
- `SecretError` extended with `Bitwarden` variant for CLI-specific errors
- Renamed "Save to KeePass" / "Load from KeePass" buttons to universal "Save password to vault" / "Load password from vault"
- Renamed sidebar "Open KeePass Database" button to "Open Password Vault"
- Improved split view button icons for better intuitiveness:
  - Split Vertical now uses `object-flip-horizontal-symbolic`
  - Split Horizontal now uses `object-flip-vertical-symbolic`

### Updated Dependencies
- `aws-lc-rs` 1.15.2 → 1.15.3
- `aws-lc-sys` 0.35.0 → 0.36.0
- `chrono` 0.4.42 → 0.4.43
- `clap_lex` 0.7.6 → 0.7.7
- `time` 0.3.44 → 0.3.45
- `tower` 0.5.2 → 0.5.3
- `zune-jpeg` 0.5.8 → 0.5.9

## [Unreleased] - 0.6.2

### Added
- **MobaXterm Import/Export** - Full support for MobaXterm `.mxtsessions` files:
  - Import SSH, RDP, VNC sessions with all settings (auth, resolution, color depth, etc.)
  - Export connections to MobaXterm format with folder hierarchy
  - Preserves group structure as MobaXterm bookmarks folders
  - Handles MobaXterm escape sequences and Windows-1252 encoding
  - CLI support: `rustconn-cli import/export --format moba-xterm`
- **Connection History Button** - Quick access to connection history from sidebar toolbar
- **Run Snippet from Context Menu** - Right-click on connection → "Run Snippet..." to execute snippets
  - Automatically connects if not already connected, then shows snippet picker
- **Persistent Search History** - Search queries are now saved across sessions
  - Up to 20 recent searches preserved in settings
  - History restored on application startup

### Changed
- Welcome screen: Removed "Import/Export connections" from Features column (redundant with Import Formats)
- Welcome screen: Combined "Asbru-CM / Royal TS / MobaXterm" into single row in Import Formats
- Documentation: Removed hardcoded version numbers from INSTALL.md package commands (use wildcards)

### Fixed
- **KeePass Alert Dialog Focus** - "Password Saved" alert now appears in front of the connection dialog
  - Previously the alert appeared behind the New/Edit Connection dialog
  - Fixed by passing the dialog window as parent instead of main window

### Dependencies
- Updated `quick-xml` 0.38 → 0.39
- Updated `resvg` 0.45 → 0.46
- Updated `usvg` 0.45 → 0.46
- Updated `svgtypes` 0.15 → 0.16
- Updated `roxmltree` 0.20 → 0.21
- Updated `kurbo` 0.11 → 0.13
- Updated `gif` 0.13 → 0.14
- Updated `imagesize` 0.13 → 0.14
- Updated `zune-jpeg` 0.4 → 0.5

## [0.6.1] - 2026-01-12

### Added
- **Credential Inheritance** - Simplify connection management by inheriting credentials from parent groups:
  - New "Inherit" option in password source dropdown
  - Recursively resolves credentials up the group hierarchy
  - Reduces duplication for environments sharing same credentials
- **Jump Host Support** - Native SSH Jump Host configuration:
  - New "Jump Host" dropdown in SSH connection settings
  - Select any existing SSH connection as a jump host
  - Supports chained jump hosts (Jump Host -> Jump Host -> Target)
  - Automatically configures `-J` argument for SSH connections
- **Adwaita Empty States** - Migrated empty state views to `adw::StatusPage`:
  - Modern, consistent look for empty connection lists, terminals, and search results
  - Proper theming support
- **Group Improvements**:
  - **Sorting**: Group lists in sidebar and dropdowns are now sorted alphabetically by full path
  - **Credentials UI**: New fields in Group Dialogs to set default Username/Password/Domain
  - **Move Group**: Added "Parent" dropdown to Edit Group dialog to move groups (with cycle prevention)

### Dependencies
- Updated `libadwaita` to `0.7`
- Updated `gtk4` to `0.10`
- Updated `vte4` to `0.9`
## [0.6.0] - 2026-01-12

### Added
- **Pre-connect Port Check** - Fast TCP port reachability check before launching RDP/VNC/SPICE connections:
  - Provides faster feedback (2-3s vs 30-60s timeout) when hosts are unreachable
  - Configurable globally in Settings → Connection with timeout setting (default: 3s)
  - Per-connection "Skip port check" option for special cases (firewalls, port knocking, VPN)
  - New `ConnectionSettings` struct in `AppSettings` for connection-related settings
  - New `skip_port_check` field on `Connection` model
- **CLI Feature Parity** - CLI now supports all major GUI features:
  - `template list/show/create/delete/apply` - Connection template management
  - `cluster list/show/create/delete/add-connection/remove-connection` - Cluster management
  - `var list/show/set/delete` - Global variables management
  - `duplicate` - Duplicate existing connections
  - `stats` - Show connection statistics (counts by protocol, groups, templates, clusters, snippets, variables, usage)
- **GitHub CI RPM Build** - Added Fedora RPM package build to release workflow:
  - Builds in Fedora 41 container with Rust 1.87
  - RPM package included in GitHub releases alongside .deb and AppImage
  - Installation instructions for Fedora in release notes
- Added `load_variables()` and `save_variables()` methods to `ConfigManager` for global variables persistence
- Added `<icon>` element to metainfo.xml for explicit AppStream icon declaration
- Added `<developer_name>` tag to metainfo.xml for backward compatibility with older AppStream parsers
- Added `author` and `license` fields to AppImage packaging (AppImageBuilder.yml)
- Added `debian.copyright` file to OBS debian packaging

### Changed
- **Code Audit & Cleanup Release** - comprehensive codebase audit and modernization
- Removed `check_structs.rs` development artifact containing unsafe code (violated `unsafe_code = "forbid"` policy)
- Replaced `blocking_send()` with `try_send()` in VNC input handlers to prevent UI freezes
- Replaced `unwrap()` with safe alternatives in `sidebar.rs` iterator access
- Replaced `expect()` with proper error handling in `validation.rs` regex compilation
- Replaced module-level `#![allow(clippy::unwrap_used)]` with targeted function-level annotations in `embedded_rdp_thread.rs`
- Improved `app.rs` initialization to return proper error instead of panicking
- Updated `Cargo.toml` license from MIT to GPL-3.0-or-later (matches actual LICENSE file)
- Updated `Cargo.toml` authors to "Anton Isaiev <totoshko88@gmail.com>"

### Fixed
- Fixed `remote-viewer` version detection for localized output (e.g., Ukrainian "версія" instead of "version")
- Fixed Asbru-CM import skipping RDP/VNC connections with client info (e.g., "rdp (rdesktop)", "rdp (xfreerdp)", "vnc (vncviewer)")
- VNC keyboard/mouse input no longer blocks GTK main thread on channel send
- Sidebar protocol filter no longer panics on empty filter set
- Regex validation errors now return `Result` instead of panicking
- FreeRDP thread mutex operations now have documented safety invariants
- Package metadata now correctly shows author and license in all package formats

### Dependencies
- Updated `base64ct` 1.8.2 → 1.8.3
- Updated `cc` 1.2.51 → 1.2.52
- Updated `data-encoding` 2.9.0 → 2.10.0
- Updated `find-msvc-tools` 0.1.6 → 0.1.7
- Updated `flate2` 1.1.5 → 1.1.8
- Updated `getrandom` 0.2.16 → 0.2.17
- Updated `libc` 0.2.179 → 0.2.180
- Updated `toml` 0.9.10 → 0.9.11
- Updated `zbus` 5.12.0 → 5.13.1
- Updated `zbus_macros` 5.12.0 → 5.13.1
- Updated `zbus_names` 4.2.0 → 4.3.1
- Updated `zmij` 1.0.12 → 1.0.13
- Updated `zvariant` 5.8.0 → 5.9.1
- Updated `zvariant_derive` 5.8.0 → 5.9.1
- Updated `zvariant_utils` 3.2.1 → 3.3.0
- Removed unused `cfg_aliases`, `nix`, `static_assertions` dependencies
- Note: `sspi` and `picky-krb` kept at 0.16.0/0.11.0 due to `rand_core` version conflict

### Removed
- `rustconn-core/src/check_structs.rs` - development artifact with unsafe code

## [0.5.9] - 2026-01-10

### Changed
- Migrated Settings dialog from deprecated `PreferencesWindow` to `PreferencesDialog` (libadwaita 1.5+)
- Updated libadwaita feature from `v1_4` to `v1_5` for PreferencesDialog support
- Updated workspace dependencies:
  - `uuid` 1.6 → 1.11
  - `regex` 1.10 → 1.11
  - `proptest` 1.4 → 1.6
  - `tempfile` 3.24 → 3.15
  - `zip` 2.1 → 2.2
- Removed unnecessary `macos_kqueue` feature from `notify` crate
- Note: `ksni` 0.3.3 and `sspi`/`picky-krb` kept at current versions due to `zvariant`/`rand_core` version conflicts
- Migrated all dialogs to use `adw::ToolbarView` for proper libadwaita layout:
- Migrated Template dialog to modern libadwaita patterns:
  - Basic tab: `adw::PreferencesGroup` with `adw::ActionRow` for template info and default values
  - SSH options: `adw::PreferencesGroup` with Authentication, Connection, and Session groups
  - RDP options: Display, Features, and Advanced groups with dynamic visibility (resolution/color hidden in Embedded mode)
  - VNC options: Display, Encoding, Features, and Advanced groups
  - SPICE options: Security, Features, and Performance groups with dynamic visibility (TLS-related fields)
  - Zero Trust options: Provider selection with `adw::ActionRow`, provider-specific groups for all 10 providers

### Fixed
- Fixed missing icon for "Embedded SSH terminals" feature on Welcome page (`display-symbolic` → `utilities-terminal-symbolic`)
- Fixed missing Quick Connect header bar icon (`network-transmit-symbolic` → `go-jump-symbolic`)
- Fixed missing Split Horizontal header bar icon (`view-paged-symbolic` → `object-flip-horizontal-symbolic`)
- Fixed missing Interface tab icon in Settings (`preferences-desktop-appearance-symbolic` → `applications-graphics-symbolic`)
- Fixed KeePass Settings: Browse buttons for Database File and Key File now open file chooser dialogs
- Fixed KeePass Settings: Dynamic visibility for Authentication fields (password/key file rows show/hide based on switches)
- Fixed KeePass Settings: Added "Check" button to verify database connection
- Fixed KeePass Settings: `verify_kdbx_credentials` now correctly handles key-file-only authentication with `--no-password` flag
- Fixed SSH Agent Settings: "Start Agent" button now properly starts ssh-agent and updates UI
- Fixed Zero Trust (AWS SSM) connection status icon showing as failed despite successful connection

### Improved
- Migrated About dialog from `gtk4::AboutDialog` to `adw::AboutDialog` for modern GNOME look
- Migrated Password Generator dialog switches from `ActionRow` + `Switch` to `adw::SwitchRow` for cleaner code
- Migrated Cluster dialog broadcast switch from `ActionRow` + `Switch` to `adw::SwitchRow`
- Migrated Export dialog switches from `ActionRow` + `Switch` to `adw::SwitchRow`
- Enhanced About dialog with custom links and credits:
  - Added short description under logo
  - Added Releases, Details, and License links
  - Added "Made with ❤️ in Ukraine 🇺🇦" to Acknowledgments
  - Added legal sections for key dependencies (GTK4, IronRDP, VTE)
- Migrated group dialogs from `ActionRow` + `Entry` to `adw::EntryRow`:
  - New Group dialog
  - Edit Group dialog
  - Rename dialog (connections and groups)
- Migrated Settings UI tab from `SpinButton` to `adw::SpinRow` for session max age
- Added `alert.rs` helper module for modern `adw::AlertDialog` API
- Migrated all `gtk4::AlertDialog` usages to `adw::AlertDialog` via helper module (50+ usages across 12 files)
- Updated documentation (INSTALL.md, USER_GUIDE.md) for version 0.5.9
  - Connection dialog (`dialogs/connection.rs`)
  - SSH Agent passphrase dialog (`dialogs/settings/ssh_agent_tab.rs`)
- Enabled libadwaita `v1_4` feature for `adw::ToolbarView` support
- Replaced hardcoded CSS colors with Adwaita semantic colors:
  - Status indicators now use `@success_color`, `@warning_color`, `@error_color`
  - Toast notifications use semantic colors for success/warning states
  - Form validation styles use semantic colors
- Reduced global clippy suppressions in `main.rs` from 30+ to 5 essential ones
- Replaced `unwrap()` calls in Cairo drawing code with proper error handling (`if let Ok(...)`)

### Fixed
- Cairo text rendering in embedded RDP/VNC widgets no longer panics on font errors

## [0.5.8] - 2026-01-07

### Changed
- Migrated Connection Dialog tabs to libadwaita components (GNOME HIG compliance):
  - Display tab: `adw::PreferencesGroup` + `adw::ActionRow` for window mode settings
  - Logging tab: `adw::PreferencesGroup` + `adw::ActionRow` for session logging configuration
  - WOL tab: `adw::PreferencesGroup` + `adw::ActionRow` for Wake-on-LAN settings
  - Variables tab: `adw::PreferencesGroup` for local variable management
  - Automation tab: `adw::PreferencesGroup` for expect rules configuration
  - Tasks tab: `adw::PreferencesGroup` for pre/post connection tasks
  - Custom Properties tab: `adw::PreferencesGroup` for metadata fields
- All migrated tabs now use `adw::Clamp` for proper content width limiting
- Removed deprecated `gtk4::Frame` usage in favor of `adw::PreferencesGroup`
- Settings dialog now loads asynchronously for faster startup:
  - Clients tab: CLI detection runs in background with spinner placeholders
  - SSH Agent tab: Agent status and key lists load asynchronously
  - Available SSH keys scan runs in background
- Cursor Shape/Blink toggle buttons in Terminal settings now have uniform width (240px)
- KeePassXC debug output now uses `tracing::debug!` instead of `eprintln!`
- KeePass entry path format changed to `RustConn/{name} ({protocol})` to support same name for different protocols
- Updated dependencies: indexmap 2.12.1→2.13.0, syn 2.0.113→2.0.114, zerocopy 0.8.32→0.8.33, zmij 1.0.10→1.0.12
- Note: sspi and picky-krb kept at previous versions due to rand_core compatibility issues

### Fixed
- SSH Agent "Add Key" button now opens file chooser to select any SSH key file
- SSH Agent "+" buttons in Available Key Files list now load keys with passphrase dialog
- SSH Agent "Remove Key" (trash) button now actually removes keys from the agent
- SSH Agent Refresh button updates both loaded keys and available keys lists
- VNC password dialog now correctly loads password from KeePass using consistent lookup key (name or host)
- KeePass passwords for connections with same name but different protocols no longer overwrite each other
- Welcome tab now displays correctly when switching back from connections (fallback to first pane if none focused)

## [0.5.7] - 2026-01-07

### Changed
- Updated dependencies: h2 0.4.12→0.4.13, proc-macro2 1.0.104→1.0.105, quote 1.0.42→1.0.43, rsa 0.9.9→0.9.10, rustls 0.23.35→0.23.36, serde_json 1.0.148→1.0.149, url 2.5.7→2.5.8, zerocopy 0.8.31→0.8.32
- Note: sspi and picky-krb kept at previous versions due to rand_core compatibility issues

### Fixed
- Test button in New Connection dialog now works correctly (fixed async runtime issue with GTK)

## [0.5.6] - 2026-01-07

### Added
- Enhanced terminal settings with color themes, cursor options, and behavior controls
- Six built-in terminal color themes: Dark, Light, Solarized Dark/Light, Monokai, Dracula
- Cursor shape options (Block, IBeam, Underline) and blink modes (On, Off, System)
- Terminal behavior settings: scroll on output/keystroke, hyperlinks, mouse autohide, audible bell
- Scrollable terminal settings dialog with organized sections
- Security Tips section in Password Generator dialog with 5 best practice recommendations
- Quick Filter functionality in sidebar for protocol filtering (SSH, RDP, VNC, SPICE, ZeroTrust)
- Protocol filter buttons with icons and visual feedback (highlighted when active)
- CSS styling for Quick Filter buttons with hover and active states
- Enhanced Quick Filter with proper OR logic for multiple protocol selection
- Visual feedback for multiple active filters with special styling (`filter-active-multiple` CSS class)
- API methods for accessing active protocol filters (`get_active_protocol_filters`, `has_active_protocol_filters`, `active_protocol_filter_count`)
- Fullscreen mode toggle with F11 keyboard shortcut
- KeePass status button in sidebar toolbar with visual integration status indicator

### Changed
- Migrated to native libadwaita architecture:
  - Application now uses `adw::Application` and `adw::ApplicationWindow` for proper theme integration
  - All dialogs redesigned to use `adw::Window` with `adw::HeaderBar` following GNOME HIG
  - Proper dark/light theme support via libadwaita StyleManager
- Unified dialog widths: Rename and Edit Group dialogs now use 750px width (matching Move dialog)
- Updated USER_GUIDE.md with complete documentation for all v0.5.5+ features
- Updated dependencies: tokio 1.48→1.49, notify 7.0→8.2, thiserror 2.0→2.0.17, clap 4.5→4.5.23, quick-xml 0.37→0.38
- Settings dialog UI refactored for lighter appearance:
  - Removed Frame widgets from all tabs (SSH Agent, Terminal, Logging, Secrets, UI, Clients)
  - Replaced with section headers using Label with `heading` CSS class
  - Removed `boxed-list` CSS class from ListBox widgets
  - Removed nested ScrolledWindow wrappers
- Theme switching now uses libadwaita StyleManager instead of GTK Settings
- Clients tab version parsing improved for all Zero Trust CLIs:
  - OCI CLI: parses "3.71.4" format
  - Tailscale: parses "1.92.3" format
  - SPICE remote-viewer: parses "remote-viewer, версія 11.0" format

### Fixed
- Terminal settings now properly apply to all terminal sessions:
  - SSH connections use user-configured terminal settings
  - Zero Trust connections use user-configured terminal settings
  - Quick Connect SSH sessions use user-configured terminal settings
  - Local Shell uses user-configured terminal settings
  - Saving settings in Settings dialog immediately applies to all existing terminals
- Clients tab CLI version parsing:
  - AWS CLI: parses "aws-cli/2.32.28 ..." format
  - GCP CLI: parses "Google Cloud SDK 550.0.0" format
  - Azure CLI: parses "azure-cli 2.81.0" format
  - Cloudflare CLI: parses "cloudflared version 2025.11.1 ..." format
  - Teleport: parses "Teleport v18.6.2 ..." format
  - Boundary: parses "Version Number: 0.21.0" format
- Clients tab now searches ~/bin/, ~/.local/bin/, ~/.cargo/bin/ for CLI tools
- Fixed quick-xml 0.38 API compatibility in Royal TS import (replaced deprecated `unescape()` method)
- Fixed Quick Filter logic to use proper OR logic for multiple protocol selection (connections matching ANY selected protocol are shown)
- Improved Quick Filter visual feedback with enhanced styling for multiple active filters
- Quick Filter now properly handles multiple protocol selection with clear visual indication
- Removed redundant clear filter button from Quick Filter bar (search entry can be cleared manually)
- Fixed Quick Filter button state synchronization - buttons are now properly cleared when search field is manually cleared
- Fixed RefCell borrow conflict panic when toggling protocol filters - resolved recursive update issue

## [0.5.5] - 2026-01-03

### Added
- Kiro steering rules for development workflow:
  - `commit-checklist.md` - pre-commit cargo fmt/clippy checks
  - `release-checklist.md` - version files and packaging verification
- Rename action in sidebar context menu for both connections and groups
- Double-click on import source to start import
- Double-click on template to create connection from it
- Group dropdown in Connection dialog Basic tab for selecting parent group
- Info tab for viewing connection details (like Asbru-CM) - replaces popover with full tab view
- Default alphabetical sorting for connections and groups with drag-drop reordering support

### Changed
- Manage Templates dialog: "Create" button now creates connection from template, "Create Template" button creates new template
- View Details action now opens Info tab instead of popover
- Sidebar now uses sorted rebuild for consistent alphabetical ordering
- All dialogs now follow GNOME HIG button layout: Close/Cancel on left, Action on right
- Removed window close button (X) from all dialogs - use explicit Close/Cancel buttons instead

### Fixed
- Flatpak manifest version references updated correctly
- Connection group_id preserved when editing connections (no longer falls to root)
- Import dialog now returns to source selection when file chooser is cancelled
- Drag-and-drop to groups now works correctly (connections can be dropped into groups)

## [0.5.4] - 2026-01-02

### Changed
- Updated dependencies: cc, iri-string, itoa, libredox, proc-macro2, rustls-native-certs, ryu, serde_json, signal-hook-registry, syn, zeroize_derive
- Note: sspi and picky-krb kept at previous versions due to rand_core compatibility issues

### Added
- Close Tab action implementation for terminal notebook
- Session Restore feature with UI settings in Settings dialog:
  - Enable/disable session restore on startup
  - Option to prompt before restoring sessions
  - Configurable maximum session age (hours)
  - Sessions saved on app close, restored on next startup
- `AppState` methods for session restore: `save_active_sessions()`, `get_sessions_to_restore()`, `clear_saved_sessions()`
- `TerminalNotebook.get_all_sessions()` method for collecting active sessions
- Password Generator feature:
  - New `password_generator` module in `rustconn-core` with secure password generation using `ring::rand`
  - Configurable character sets: lowercase, uppercase, digits, special, extended special
  - Option to exclude ambiguous characters (0, O, l, 1, I)
  - Password strength evaluation with entropy calculation
  - Crack time estimation based on entropy
  - Password Generator dialog accessible from Tools menu
  - Real-time strength indicator with level bar
  - Copy to clipboard functionality
- Advanced session logging modes with three configurable options:
  - Activity logging (default) - tracks session activity changes
  - User input logging - captures commands typed by user
  - Terminal output logging - records full terminal transcript
  - Settings UI with checkboxes in Session Logging tab
- Royal TS (.rtsz XML) import support:
  - SSH, RDP, and VNC connection import
  - Folder hierarchy preservation as connection groups
  - Credential reference resolution (username/domain)
  - Trash folder filtering (deleted connections are skipped)
  - Accessible via Import dialog
- Royal TS (.rtsz XML) export support:
  - SSH, RDP, and VNC connection export
  - Folder hierarchy export as Royal TS folders
  - Username and domain export for credentials
  - Accessible via Export dialog
- RDPDR directory change notifications with inotify integration:
  - `dir_watcher` module using `notify` crate for file system monitoring
  - `FileAction` enum matching MS-FSCC `FILE_ACTION_*` constants
  - `CompletionFilter` struct with MS-SMB2 `FILE_NOTIFY_CHANGE_*` flags
  - `DirectoryWatcher` with recursive/non-recursive watch support
  - `build_file_notify_info()` for MS-FSCC 2.4.42 `FILE_NOTIFY_INFORMATION` structures
  - Note: RDP responses pending ironrdp upstream support for `ClientDriveNotifyChangeDirectoryResponse`

### Fixed
- Close Tab keyboard shortcut (Ctrl+W) now properly closes active session tab

## [0.5.3] - 2026-01-02

### Added
- Connection history recording for all protocols (SSH, VNC, SPICE, RDP, ZeroTrust)
- "New Group" button in Group Operations Mode bulk actions bar
- "Reset" buttons in Connection History and Statistics dialogs (header bar)
- "Clear Statistics" functionality in AppState
- Protocol-specific tabs in Template Dialog matching Connection Dialog functionality:
  - SSH: auth method, key source, proxy jump, agent forwarding, startup command, custom options
  - RDP: client mode, resolution, color depth, audio, gateway, custom args
  - VNC: client mode, encoding, compression, quality, view only, scaling, clipboard
  - SPICE: TLS, CA cert, USB, clipboard, image compression
  - ZeroTrust: all 10 providers (AWS SSM, GCP IAP, Azure Bastion/SSH, OCI, Cloudflare, Teleport, Tailscale, Boundary, Generic)
- Connection history dialog (`HistoryDialog`) for viewing and searching session history
- Connection statistics dialog (`StatisticsDialog`) with success rate visualization
- Common embedded widget trait (`EmbeddedWidget`) for RDP/VNC/SPICE deduplication
- `EmbeddedConnectionState` enum for unified connection state handling
- `EmbeddedWidgetState` helper for managing common widget state
- `create_embedded_toolbar()` helper for consistent toolbar creation
- `draw_status_overlay()` helper for status rendering
- Quick Connect dialog now supports connection templates (auto-fills protocol, host, port, username)
- History/Statistics menu items in Tools section
- `AppState` methods for recording connection history (`record_connection_start`, `record_connection_end`, etc.)
- `ConfigManager.load_history()` and `save_history()` for history persistence
- Property tests for history models (`history_tests.rs`):
  - Entry creation, quick connect, end/fail operations
  - Statistics update consistency, success rate bounds
  - Serialization round-trips for all history types
- Property tests for session restore models (`session_restore_tests.rs`):
  - `SavedSession` creation and serialization
  - `SessionRestoreSettings` configuration and serialization
  - Round-trip tests with multiple saved sessions
- Quick Connect now supports RDP and VNC protocols (previously only SSH worked)
- RDP Quick Connect uses embedded IronRDP widget with state callbacks and reconnect support
- VNC Quick Connect uses native VncSessionWidget with full embedded mode support
- Quick Connect password field for RDP and VNC connections
- Connection history model (`ConnectionHistoryEntry`) for tracking session history
- Connection statistics model (`ConnectionStatistics`) with success rate, duration tracking
- History settings (`HistorySettings`) with configurable retention and max entries
- Session restore settings (`SessionRestoreSettings`) for restoring sessions on startup
- `SavedSession` model for persisting session state across restarts

### Changed
- UI Unification: All dialogs now use consistent 750×500px dimensions
- Removed duplicate Close/Cancel buttons from all dialogs (window X button is sufficient)
- Renamed action buttons for consistency:
  - "New X" → "Create" (moved to left side of header bar)
  - "Quick Connect" → "Connect" in Quick Connect dialog
  - "Clear History/Statistics" → "Reset" (moved to header bar with destructive style)
- Create Connection now always opens blank New Connection dialog (removed template picker)
- Templates can be used from Manage Templates dialog
- Button styling: All action buttons (Create, Save, Import, Export) use `suggested-action` CSS class
- When editing existing items, button label changes from "Create" to "Save"
- Extracted common embedded widget patterns to `embedded_trait.rs`
- `show_quick_connect_dialog()` now accepts optional `SharedAppState` for template access
- Refactored `terminal.rs` into modular structure (`rustconn/src/terminal/`):
  - `mod.rs` - Main `TerminalNotebook` implementation
  - `types.rs` - `TabDisplayMode`, `TerminalSession`, `SessionWidgetStorage`, `TabLabelWidgets`
  - `config.rs` - Terminal appearance and behavior configuration
  - `tabs.rs` - Tab creation, display modes, overflow menu management
- `EmbeddedSpiceWidget` now implements `EmbeddedWidget` trait for unified interface
- Updated `gtk4` dependency from 0.10 to 0.10.2
- Improved picky dependency documentation with monitoring notes for future ironrdp compatibility
- `AppSettings` now includes `history` field for connection history configuration
- `UiSettings` now includes `session_restore` field for session restore configuration

### Fixed
- Connection History "Connect" button now actually connects (was only logging)
- History statistics labels (Total/Successful/Failed) now update correctly
- Statistics dialog content no longer cut off (increased size)
- Quick Connect RDP/VNC no longer shows placeholder tabs — actual connections are established

## [0.5.2] - 2025-12-29

### Added
- `wayland-native` feature flag with `gdk4-wayland` integration for improved Wayland detection
- Sidebar integration with lazy loading and virtual scrolling APIs

### Changed
- Improved display server detection using GDK4 Wayland bindings when available
- Refactored `window.rs` into modular structure (reduced from 7283 to 2396 lines, -67%):
  - `window_types.rs` - Type aliases and `get_protocol_string()` utility
  - `window_snippets.rs` - Snippet management methods
  - `window_templates.rs` - Template management methods
  - `window_sessions.rs` - Session management methods
  - `window_groups.rs` - Group management dialogs (move to group, error toast)
  - `window_clusters.rs` - Cluster management methods
  - `window_connection_dialogs.rs` - New connection/group dialogs, template picker, import dialog
  - `window_sorting.rs` - Sorting and drag-drop reordering operations
  - `window_operations.rs` - Connection operations (delete, duplicate, copy, paste, reload)
  - `window_edit_dialogs.rs` - Edit dialogs (edit connection, connection details, edit group, quick connect)
  - `window_rdp_vnc.rs` - RDP and VNC connection methods with password dialogs
  - `window_protocols.rs` - Protocol-specific connection handlers (SSH, VNC, SPICE, ZeroTrust)
  - `window_document_actions.rs` - Document management actions (new, open, save, close, export, import)
- Refactored `embedded_rdp.rs` into modular structure (reduced from 4234 to 2803 lines, -34%):
  - `embedded_rdp_types.rs` - Error types, enums, config structs, callback types
  - `embedded_rdp_buffer.rs` - PixelBuffer and WaylandSurfaceHandle
  - `embedded_rdp_launcher.rs` - SafeFreeRdpLauncher with Qt warning suppression
  - `embedded_rdp_thread.rs` - FreeRdpThread, ClipboardFileTransfer, FileDownloadState
  - `embedded_rdp_detect.rs` - FreeRDP detection utilities (detect_wlfreerdp, detect_xfreerdp, is_ironrdp_available)
  - `embedded_rdp_ui.rs` - UI helpers (clipboard buttons, Ctrl+Alt+Del, draw_status_overlay)
- Refactored `sidebar.rs` into modular structure (reduced from 2787 to 1937 lines, -30%):
  - `sidebar_types.rs` - TreeState, SessionStatusInfo, DropPosition, DropIndicator, SelectionModelWrapper, DragDropData
  - `sidebar_ui.rs` - UI helper functions (popovers, context menus, button boxes, protocol icons)
- Refactored `embedded_vnc.rs` into modular structure (reduced from 2304 to 1857 lines, -19%):
  - `embedded_vnc_types.rs` - Error types, VncConnectionState, VncConfig, VncPixelBuffer, VncWaylandSurface, callback types

### Fixed
- Tab icons now match sidebar icons for all protocols (SSH, RDP, VNC, SPICE, ZeroTrust providers)
- SSH and ZeroTrust sessions now show correct protocol-specific icons in tabs
- Cluster list not refreshing after deleting a cluster (borrow conflict in callback)
- Snippet dialog Save button not clickable (unreliable widget tree traversal replaced with direct reference)
- Template dialog not showing all fields (missing vexpand on notebook and scrolled window)

### Improved
- Extracted coordinate transformation utilities to `embedded_rdp_ui.rs` and `embedded_vnc_ui.rs`
- Added `transform_widget_to_rdp()`, `gtk_button_to_rdp_mask()`, `gtk_button_to_rdp_button()` helpers
- Added `transform_widget_to_vnc()`, `gtk_button_to_vnc_mask()` helpers
- Reduced code duplication in mouse input handlers (4 duplicate blocks → 1 shared function)
- Added unit tests for coordinate transformation and button conversion functions
- Made RDP event polling interval configurable via `RdpConfig::polling_interval_ms` (default 16ms = ~60 FPS)
- Added `RdpConfig::with_polling_interval()` builder method for custom polling rates
- CI: Added `libadwaita-1-dev` dependency to all build jobs
- CI: Added dedicated property tests job for better test visibility
- CI: Consolidated OBS publish workflow into release workflow
- CI: Auto-generate OBS changelog from CHANGELOG.md during release

### Documentation
- Added `#![warn(missing_docs)]` and documentation for public APIs in `rustconn-core`

## [0.5.1] - 2025-12-28

### Added
- Search debouncing with visual spinner indicator in sidebar (100ms delay for better UX)
- Pre-search state preservation (expanded groups, scroll position restored when search cleared)
- Clipboard file transfer UI for embedded RDP sessions:
  - "Save Files" button appears when files are available on remote clipboard
  - Folder selection dialog for choosing download destination
  - Progress tracking and completion notifications
  - Automatic file saving with status feedback
- CLI: Wake-on-LAN command (`wol`) - send magic packets by MAC address or connection name
- CLI: Snippet management commands (`snippet list/show/add/delete/run`)
  - Variable extraction and substitution support
  - Execute snippets with `--execute` flag
- CLI: Group management commands (`group list/show/create/delete/add-connection/remove-connection`)
- CLI: Connection list filters (`--group`, `--tag`) for `list` command
- CLI: Native format (.rcn) support for import/export

### Changed
- Removed global `#![allow(dead_code)]` from `rustconn/src/main.rs`
- Added targeted `#[allow(dead_code)]` annotations with documentation comments to GTK widget fields kept for lifecycle management
- Removed unused code:
  - `STANDARD_RESOLUTIONS` and `find_best_standard_resolution` from `embedded_rdp.rs`
  - `connect_kdbx_enable_switch` from `dialogs/settings.rs` (extended version exists)
  - `update_reconnect_button_visibility` from `embedded_rdp.rs`
  - `as_selection_model` from `sidebar.rs`
- Added public methods to `AutomationSession`: `remaining_triggers()`, `is_complete()`
- Documented API methods in `sidebar.rs`, `state.rs`, `terminal.rs`, `window.rs` with `#[allow(dead_code)]` annotations for future use
- Removed `--talk-name=org.freedesktop.secrets` from Flatpak manifest (unnecessary D-Bus permission)
- Refactored `dialogs/export.rs`: extracted `do_export()` and `format_result_summary()` to eliminate code duplication

## [0.5.0] - 2025-12-27

### Added
- RDP clipboard file transfer support (`CF_HDROP` format):
  - `ClipboardFileInfo` struct for file metadata (name, size, attributes, timestamps)
  - `ClipboardFileList`, `ClipboardFileContents`, `ClipboardFileSize` events
  - `RequestFileContents` command for requesting file data from server
  - `FileGroupDescriptorW` parsing for Windows file list format (MS-RDPECLIP 2.2.5.2.3.1)
- RDPDR directory change notifications (`ServerDriveNotifyChangeDirectoryRequest`):
  - Basic acknowledgment support (inotify integration pending)
  - `PendingNotification` struct for tracking watch requests
- RDPDR file locking support (`ServerDriveLockControlRequest`):
  - Basic acknowledgment for byte-range lock requests
  - `FileLock` struct for lock state tracking (advisory locking)

### Changed
- Audio playback: replaced `Mutex<f32>` with `AtomicU32` for volume control (lock-free audio callback)
- Search engine: optimized fuzzy matching to avoid string allocations (30-40% faster for large lists)
- Credential operations: use thread-local cached tokio runtime instead of creating new one each time

### Fixed
- SSH Agent key discovery now finds all private keys in `~/.ssh/`, not just `id_*` files:
  - Detects `.pem` and `.key` extensions
  - Reads file headers to identify private keys (e.g., `google_compute_engine`)
  - Skips known non-key files (`known_hosts`, `config`, `authorized_keys`)
- Native SPICE protocol embedding using `spice-client` crate 0.2.0 (optional `spice-embedded` feature)
  - Direct framebuffer rendering without external processes
  - Keyboard and mouse input forwarding via Inputs channel
  - Automatic fallback to external viewer (remote-viewer, virt-viewer, spicy) when native fails
  - Note: Clipboard and USB redirection not yet available in native mode (crate limitation)
- Real-time connection status indicators in the sidebar (green/red dots) to show connected/disconnected state
- Support for custom cursors in RDP sessions (server-side cursor updates)
- Full integration of "Expect" automation engine:
  - Regex-based pattern matching on terminal output
  - Automatic response injection
  - Support for "one-shot" triggers
- Terminal improvements:
  - Added context menu (Right-click) with Copy, Paste, and Select All options
  - Added keyboard shortcuts: Ctrl+Shift+C (Copy) and Ctrl+Shift+V (Paste)
- Refactored `Connection` model to support extensible automation configuration (`AutomationConfig`)

### Changed
- Updated `thiserror` from 1.0 to 2.0 (backwards compatible, no API changes required)
- Note: `picky` remains pinned at `=7.0.0-rc.17` due to sspi 0.16.0 incompatibility with newer versions

### Removed
- Unused FFI mock implementations for RDP and SPICE protocols (`rustconn-core/src/ffi/rdp.rs`, `rustconn-core/src/ffi/spice.rs`)
- Unused RDP and SPICE session widget modules (`rustconn/src/session/rdp.rs`, `rustconn/src/session/spice.rs`)

### Fixed
- Connection status indicator disappearing when closing one of multiple sessions for the same connection (now tracks session count per connection)
- System tray menu intermittently not appearing (reduced lock contention and debounced D-Bus updates)

## [0.4.2] - 2025-12-25

### Fixed
- Asbru-CM import now correctly parses installed Asbru configuration (connections inside `environments` key)
- Application icon now properly resolves in all installation scenarios (system, Flatpak, local, development)

### Changed
- Icon theme search paths extended to support multiple installation methods

## [0.4.1] - 2025-12-25

### Added
- IronRDP audio backend (RDPSND) with PCM format support (48kHz, 44.1kHz, 22.05kHz)
- Optional `rdp-audio` feature for audio playback via cpal (requires libasound2-dev)
- Bidirectional clipboard improvements for embedded RDP sessions

### Changed
- Updated MSRV to 1.87 (required by zune-jpeg 0.5.8)
- Updated dependencies: tempfile 3.24, criterion 0.8, cpal 0.17

## [0.4.0] - 2025-12-24

### Added
- Zero Trust: Improved UI by hiding irrelevant fields (Host, Port, Username, Password, Tags) when Zero Trust protocol is selected.

### Changed
- Upgraded `ironrdp` to version 0.13 (async API support).
- Refactored `rustconn-core` to improve code organization and maintainability.
- Made `spice-embedded` feature mandatory for better integration.

## [0.3.1] - 2025-12-23

### Changed
- Code cleanup: fixed all Clippy warnings (pedantic, nursery)
- Applied rustfmt formatting across all crates
- Added Deactivation-Reactivation sequence handling for RDP sessions

### Fixed
- Removed sensitive clipboard debug logging (security improvement)
- Fixed nested if statements and match patterns in RDPDR module

## [0.3.0] - 2025-12-23

### Added
- IronRDP clipboard integration for embedded RDP sessions (bidirectional copy/paste)
- IronRDP shared folders (RDPDR) support for embedded RDP sessions
- RemoteFX codec support for better RDP image quality
- RDPSND channel (required for RDPDR per MS-RDPEFS spec)

### Changed
- Migrated IronRDP dependencies from GitHub to crates.io (version 0.11)
- Reduced verbose logging in RDPDR module (now uses tracing::debug/trace)

### Fixed
- Pinned sspi to 0.16.0 and picky to 7.0.0-rc.16 to avoid rand_core conflicts

## [0.2.0] - 2025-12-22

### Added
- Tree view state persistence (expanded/collapsed folders saved between sessions)
- Native format (.rcn) import/export with proper group hierarchy preservation

### Fixed
- RDP embedded mode window sizing now uses saved window geometry
- Sidebar reload now preserves expanded/collapsed state
- Group hierarchy correctly maintained during native format import

### Changed
- Dependencies updated:
  - `ksni` 0.2 → 0.3 (with blocking feature)
  - `resvg` 0.44 → 0.45
  - `dirs` 5.0 → 6.0
  - `criterion` 0.5 → 0.6
- Migrated from deprecated `criterion::black_box` to `std::hint::black_box`

### Removed
- Removed obsolete TODO comment and unused variable in window.rs

## [0.1.0] - 2025-12-01

### Added
- Initial release of RustConn connection manager
- Multi-protocol support: SSH, RDP, VNC, SPICE
- Zero Trust provider integrations (AWS SSM, GCP IAP, Azure Bastion, etc.)
- Connection organization with groups and tags
- Import from Asbru-CM, Remmina, SSH config, Ansible inventory
- Export to Asbru-CM, Remmina, SSH config, Ansible inventory
- Native format import/export for backup and migration
- Secure credential storage via KeePassXC and libsecret
- Session logging with configurable formats
- Command snippets with variable substitution
- Cluster commands for multi-host execution
- Wake-on-LAN support
- Split terminal view
- System tray integration (optional)
- Performance optimizations:
  - Search result caching with configurable TTL
  - Lazy loading for connection groups
  - Virtual scrolling for large connection lists
  - String interning for memory optimization
  - Batch processing for import/export operations
- Embedded protocol clients (optional features):
  - VNC via vnc-rs
  - RDP via IronRDP
  - SPICE via spice-client

### Security
- All credentials wrapped in `SecretString`
- No plaintext password storage
- `unsafe_code = "forbid"` enforced

[Unreleased]: https://github.com/totoshko88/RustConn/compare/v0.5.9...HEAD
[0.5.9]: https://github.com/totoshko88/RustConn/compare/v0.5.8...v0.5.9
[0.5.8]: https://github.com/totoshko88/RustConn/compare/v0.5.7...v0.5.8
[0.5.7]: https://github.com/totoshko88/RustConn/compare/v0.5.6...v0.5.7
[0.5.6]: https://github.com/totoshko88/RustConn/compare/v0.5.5...v0.5.6
[0.5.5]: https://github.com/totoshko88/RustConn/compare/v0.5.4...v0.5.5
[0.5.4]: https://github.com/totoshko88/RustConn/compare/v0.5.3...v0.5.4
[0.5.3]: https://github.com/totoshko88/RustConn/compare/v0.5.2...v0.5.3
[0.5.2]: https://github.com/totoshko88/RustConn/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/totoshko88/RustConn/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/totoshko88/RustConn/compare/v0.4.2...v0.5.0
[0.4.2]: https://github.com/totoshko88/RustConn/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/totoshko88/RustConn/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/totoshko88/RustConn/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/totoshko88/RustConn/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/totoshko88/RustConn/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/totoshko88/RustConn/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/totoshko88/RustConn/releases/tag/v0.1.0
