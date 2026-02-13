# Changelog

All notable changes to RustConn will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.8.3] - 2026-02-13

### Added
- **Wake On LAN from GUI** — Send WoL magic packets directly from the GUI ([#8](https://github.com/totoshko88/RustConn/issues/8)):
  - Right-click connection → "Wake On LAN" sends packet using configured MAC address
  - Auto-WoL before connecting: if a connection has WoL configured, a magic packet is sent automatically on connect (fire-and-forget, does not block connection)
  - Standalone WoL dialog (Menu → Tools → "Wake On LAN...") with connection picker and manual MAC entry
  - Retry with 3 packets at 500 ms intervals for reliability
  - Non-blocking: all sends run on background threads via `spawn_blocking_with_callback`
  - Toast notifications for success/failure

### Fixed
- **Flatpak libsecret Build** — Fixed Flatpak build failure: disabled `bash_completion` in libsecret module (EROFS in sandbox)
- **Flatpak libsecret Crypto Option** — Fixed libsecret 0.21.7 build: renamed `gcrypt` option to `crypto`
- **Thread Safety** — Removed `std::env::set_var` calls from FreeRDP spawned thread (`embedded_rdp_thread.rs`); env vars (`QT_LOGGING_RULES`, `QT_QPA_PLATFORM`) are already set per-process via `Command::env()` in `launch_freerdp()`, eliminating a data race (unsafe since Rust 1.66+)
- **Flatpak Machine Key** — `get_machine_key()` now generates and persists an app-specific key file in `$XDG_DATA_HOME/rustconn/.machine-key` as first priority; `/etc/machine-id` (inaccessible in Flatpak sandbox) is now a fallback, with hostname+username as last resort
- **Variables Dialog Panic** — Replaced `expect()` on `btn.root().and_downcast::<Window>()` in vault load callback with `if let Some(window)` pattern and `tracing::warn!` fallback
- **Keyring `secret-tool` Check** — `keyring::store()` now checks `is_secret_tool_available()` before attempting to store; returns `SecretError::BackendUnavailable` with user-friendly message if `secret-tool` is not installed
- **Flatpak CLI Paths** — Secret backend CLI detection (`bw`, `op`, `passbolt`) no longer adds hardcoded `/snap/bin/` and `/usr/local/bin/` paths when running inside Flatpak; checks `get_cli_install_dir()` for Flatpak-installed tools instead
- **Settings Dialog Performance** — Moved all secret backend CLI detection (`keepassxc-cli`, `bw`, `op`, `passbolt`) and keyring auto-load operations (Bitwarden auto-unlock, 1Password token, Passbolt passphrase, KeePassXC password) from synchronous main-thread execution to background threads via `glib::spawn_future`; Settings dialog now opens instantly with "Detecting..." placeholders, updating widgets asynchronously when detection and keyring lookups complete (~10 s → instant)
- **Settings Clients Tab Performance** — Added 3-second timeout to all CLI version checks (`get_version` in `detection.rs`, `get_version_with_env` in `clients_tab.rs`) preventing slow CLIs (`gcloud`, `az`, `oci`) from blocking detection; parallelized all 9 zero trust CLI detections and core/zero trust groups via `std::thread::scope`; moved SSH agent `get_status()` (`ssh-add -l`) from GTK main thread to background thread via `glib::spawn_future` — total Clients tab detection time reduced from ~15 s sequential to ~3 s parallel
- **Settings Dialog Instant Display** — Moved `dialog.present()` before `load_settings()` in `SettingsDialog::run()` so the window appears immediately; all widget population and async background operations (CLI detection, keyring lookups, SSH agent status) now run after the dialog is already visible
- **Settings Dialog Visual Render Blocking** — Replaced `glib::spawn_future_local` + `glib::spawn_future` async pattern with `std::thread::spawn` + `std::sync::mpsc::channel` + `glib::idle_add_local` in all three Settings tabs (Clients, Secrets, SSH Agent); the previous pattern kept pending futures in the GTK main loop which prevented frame rendering until background tasks completed (~6 s delay); the new pattern fully decouples background work from the main loop so the dialog renders instantly while CLI detection runs in parallel

## [0.8.2] - 2026-02-11

### Added
- **Shared Keyring Module** — New `rustconn-core::secret::keyring` module with generic `store()`, `lookup()`, `clear()`, and `is_secret_tool_available()` functions for all backends
- **Keyring Support for All Secret Backends** — System keyring (GNOME Keyring / KDE Wallet) integration for all backends:
  - Bitwarden: refactored to use shared keyring module
  - 1Password: `store_token_in_keyring()` / `get_token_from_keyring()` / `delete_token_from_keyring()`
  - Passbolt: `store_passphrase_in_keyring()` / `get_passphrase_from_keyring()` / `delete_passphrase_from_keyring()`
  - KeePassXC: `store_kdbx_password_in_keyring()` / `get_kdbx_password_from_keyring()` / `delete_kdbx_password_from_keyring()`
- **Auto-Load Credentials from Keyring** — On settings load, all backends with "Save to system keyring" enabled automatically restore credentials:
  - 1Password: loads token and sets `OP_SERVICE_ACCOUNT_TOKEN` env var
  - Passbolt: loads GPG passphrase into entry field
  - KeePassXC: loads KDBX password into entry field
  - Bitwarden: auto-unlocks vault (existing behavior)
- **`secret-tool` Availability Check** — Toggling "Save to system keyring" for any backend now checks if `secret-tool` is installed; if missing, unchecks the checkbox and shows "Install libsecret-tools for keyring" warning
- **Flatpak `secret-tool` Support** — Added `libsecret` 0.21.7 as a Flatpak build module in all manifests (flatpak, local, flathub), providing `secret-tool` binary inside the sandbox for system keyring integration
- **Passbolt Server URL Setting** — New `passbolt_server_url` field in `SecretSettings` for configuring Passbolt server address
- **Passbolt UI in Settings** — Secrets tab now includes Server URL entry and "Open Vault" button for Passbolt:
  - Server URL auto-fills from `go-passbolt-cli` config on startup
  - "Open Vault" button opens configured URL in browser
- **Unified Credential Save Options** — All secret backends now offer consistent "Save password" (encrypted local) and "Save to system keyring" (libsecret/KWallet) options with mutual exclusion:
  - KeePassXC: Added "Save to system keyring" checkbox alongside existing "Save password"
  - Bitwarden: Added mutual exclusion between "Save password" and "Save to system keyring"
  - 1Password: Added Service Account Token entry with "Save token" and "Save to system keyring"
  - Passbolt: Added GPG Passphrase entry with "Save passphrase" and "Save to system keyring"
- **New `SecretSettings` fields** — `kdbx_save_to_keyring`, `onepassword_service_account_token`, `onepassword_save_to_keyring`, `passbolt_passphrase`, `passbolt_save_to_keyring` for unified credential persistence

### Fixed
- **Secret Lookup Key Mismatch** — Fixed credential store/retrieve inconsistency across all secret backends:
  - libsecret: `store_unified()` now uses `"{name} ({protocol})"` key format matching `resolve_from_keyring` lookup
  - Bitwarden/1Password/Passbolt: resolve functions now try `rustconn/{name}` first (matching store), then UUID fallback, then `{name} ({protocol})`
  - Previously stored credentials were unretrievable because store and retrieve used different lookup keys
- **Passbolt Server Address Always None** — `get_passbolt_status()` now reads server address from `~/.config/go-passbolt-cli/config.json` via new `read_passbolt_server_address()` function
- **Passbolt "Open Password Vault" URL** — Button now opens the configured Passbolt server URL instead of hardcoded `https://passbolt.local`; reads URL from Settings or falls back to CLI config
- **Variable Secrets Ignoring Preferred Backend** — `save_variable_to_vault()` and `load_variable_from_vault()` now respect `preferred_backend` setting; previously they always used KeePass/libsecret regardless of configured backend (Bitwarden, 1Password, Passbolt)
- **Bitwarden Folder Parsing Crash** — `BitwardenFolder.id` now accepts `null` values from Bitwarden CLI (e.g. "No Folder" system entry); previously caused `Failed to parse folders` error
- **Bitwarden Vault Auto-Unlock** — Variable save/load now automatically unlocks Bitwarden vault using saved master password from keyring or encrypted settings; previously required manual `bw unlock` or `BW_SESSION` env var

### Changed
- **Dependencies** — Updated: `clap` 4.5.57→4.5.58, `clap_builder` 4.5.57→4.5.58, `clap_lex` 0.7.7→1.0.0, `deranged` 0.5.5→0.5.6

### Removed
- **Unused `picky` pin** — Removed `picky = "=7.0.0-rc.20"` version pin from `rustconn-core`; cargo resolves the correct version transitively via ironrdp/sspi without an explicit pin

### Improved
- **Workspace dependency consistency** — Moved `regex` in `rustconn` crate from inline `"1.11"` to `{ workspace = true }` for unified version management
- **Description consistency** — Unified short description ("Manage remote connections easily") and long description across all packaging metadata, README, Welcome screen, About dialog, Cargo.toml, .desktop, metainfo.xml, and Snap manifest; added missing `telnet` and `zerotrust` keywords; fixed About dialog `developer_name` field to show author name instead of product description

## [0.8.1] - 2026-02-10

### Added
- **Passbolt Secret Backend** — Passbolt password manager integration ([#6](https://github.com/totoshko88/RustConn/issues/6)):
  - `PassboltBackend` implementing `SecretBackend` trait via `go-passbolt-cli`
  - Store, retrieve, and delete credentials as Passbolt resources
  - CLI detection and version display in Settings → Secrets
  - Server configuration status check (configured/not configured/auth failed)
  - `PasswordSource::Passbolt` option in connection dialog password source dropdown
  - `SecretBackendType::Passbolt` option in settings backend selector
  - Credential resolution and rename support in `CredentialResolver`
  - Requires `passbolt configure` CLI setup before use

### Changed
- **Unified Secret Backends** — Replaced individual `PasswordSource` variants (KeePass, Keyring, Bitwarden, OnePassword, Passbolt) with single `Vault` variant:
  - Connection dialog password source dropdown: Prompt, Vault, Variable, Inherit, None
  - Serde aliases preserve backward compatibility with existing configs
  - `PasswordSource` is now `Clone` only (no longer `Copy`) due to `Variable(String)`
- **Variable Password Source** — New `PasswordSource::Variable(String)` reads credentials from a named secret global variable:
  - Connection dialog shows variable dropdown when "Variable" is selected
  - Dropdown populated with secret global variables only
- **Variables Dialog Improvements** — Show/Hide and Load from Vault buttons for secret variables:
  - Toggle password visibility with `view-reveal-symbolic`/`view-conceal-symbolic` icon
  - Load secret value from vault with key `rustconn/var/{name}`
  - Secret variable values auto-saved to vault on dialog save, cleared from settings file

### Fixed
- **Secret Variable Vault Backend** — Fixed secret variables always using libsecret instead of configured backend:
  - Save/load secret variable values now respects Settings → Secrets backend (KeePassXC, libsecret)
  - Added `save_variable_to_vault()` and `load_variable_from_vault()` functions using settings snapshot
  - Toast notification on vault save/load failure with message to check Settings
- **Variable Dropdown Empty in Connection Dialog** — Fixed Variable dropdown showing "(Немає)" when editing connections:
  - `set_global_variables()` was never called when creating/editing connections
  - Added call to all three `ConnectionDialog` creation sites (new, edit, template)
  - Edit dialog: `set_global_variables()` called before `set_connection()` so variable selection works
- **Telnet Backspace/Delete Key Handling** — Fixed keyboard settings not working correctly for Telnet connections ([#5](https://github.com/totoshko88/RustConn/issues/5)):
  - Replaced `stty erase` shell wrapper approach with VTE native `EraseBinding` API
  - Backspace/Delete settings now applied directly on the VTE terminal widget before process spawn
  - `Automatic` mode uses VTE defaults (termios for Backspace, VT220 `\e[3~` for Delete)
  - `Backspace (^H)` sends ASCII `0x08`, `Delete (^?)` sends ASCII `0x7F` as expected
  - Fixes Delete key showing `3~` escape artifacts on servers that don't support VT220 sequences
- **Split View Panel Sizing** — Fixed left panel shrinking when splitting vertically then horizontally:
  - Use model's fractional position (0.0–1.0) instead of hardcoded `size / 2` for divider placement
  - Disable `shrink_start_child`/`shrink_end_child` to prevent panels from collapsing below minimum size
  - One-shot position initialization via `connect_map` prevents repeated resets on widget remap
  - Save user-dragged divider positions back to the model via `connect_notify_local("position")`
  - Each split now correctly divides the current panel in half without affecting other panels

## [0.8.0] - 2026-02-10

### Added
- **Telnet Backspace/Delete Configuration** — Configurable keyboard behavior for Telnet connections ([#5](https://github.com/totoshko88/RustConn/issues/5)):
  - `TelnetBackspaceSends` and `TelnetDeleteSends` enums with Automatic/Backspace/Delete options
  - Connection dialog Keyboard group with two dropdowns for Backspace and Delete key behavior
  - `stty erase` shell wrapper in `spawn_telnet()` to apply key settings before connecting
  - Addresses common backspace/delete inversion issue reported by users
- **Flatpak Telnet Support** — GNU inetutils built as Flatpak module:
  - `telnet` binary available at `/app/bin/` in Flatpak sandbox
  - Built from inetutils 2.7 source with `--disable-servers` (client tools only)
  - Added to all three Flatpak manifests (flatpak, flatpak-local, flathub)

### Changed
- **Dependencies** — Updated: `libc` 0.2.180→0.2.181, `tempfile` 3.24.0→3.25.0, `unicode-ident` 1.0.22→1.0.23

### Fixed
- **OBS Screenshot Display** — Updated `_service` revision from `v0.5.3` to current version tag for proper AppStream metadata processing on software.opensuse.org
- **Flatpak AWS CLI** — Replaced `awscliv2` pip package (Docker wrapper) with official AWS CLI v2 binary installer from `awscli.amazonaws.com`; `aws --version` now shows real AWS CLI instead of Docker error
- **Flatpak Component Detection** — Fixed SSM Plugin, Azure CLI, and OCI CLI showing as "Not installed" after installation:
  - Added explicit search paths for SSM Plugin (`usr/local/sessionmanagerplugin/bin`) and AWS CLI (`v2/current/bin`)
  - Increased recursive binary search depth from 3 to 5/6 levels
- **Flatpak Python Version** — Wrapper scripts for pip-installed CLIs (Azure CLI, OCI CLI) now dynamically detect Python version instead of hardcoding `python3.13`

## [0.7.9] - 2026-02-09

### Added
- **Telnet Protocol Support** — Full Telnet protocol implementation across all crates ([#5](https://github.com/totoshko88/RustConn/issues/5)):
  - Core model: `TelnetConfig`, `ProtocolType::Telnet`, `ProtocolConfig::Telnet` with configurable host, port (default 23), and extra arguments
  - Protocol trait implementation with external `telnet` client
  - Import support: Remmina, Asbru, MobaXterm, RDM importers recognize Telnet connections
  - Export support: Remmina, Asbru, MobaXterm exporters write Telnet connections
  - CLI: `rustconn-cli telnet` subcommand with `--host`, `--port`, `--extra-args` options
  - GUI: Connection dialog with Telnet-specific configuration tab
  - Template dialog: Telnet protocol option with default port 23
  - Sidebar: Telnet filter button with `network-wired-symbolic` icon
  - Terminal: `spawn_telnet()` method for launching telnet sessions
  - Quick Connect: Telnet protocol option in quick connect bar
  - Cluster dialog: Telnet connections selectable for cluster membership
  - Property tests: All existing property tests updated with Telnet coverage

### Fixed
- **Sidebar Icon Missing** — Added missing `"telnet"` mapping in sidebar `get_protocol_icon()` function; Telnet connections now display the correct icon in the connection tree
- **Telnet Icon Mismatch** — Changed Telnet protocol icon from `network-wired-symbolic` to `call-start-symbolic` across all views (sidebar, filter buttons, dialogs, templates); the previous icon resembled a shield in breeze-dark theme, which was misleading for an insecure protocol
- **ZeroTrust Sidebar Icon** — Unified ZeroTrust sidebar icon to `folder-remote-symbolic` for all providers; previously showed provider-specific icons that were inconsistent with the filter button icon

## [0.7.8] - 2026-02-08

### Added
- **Remmina Password Import** — Importing from Remmina now automatically transfers saved passwords into the configured secret backend (libsecret, KeePassXC, etc.); connections are marked with `PasswordSource::Keyring` so credentials resolve seamlessly on first connect

### Fixed
- **Import Error Swallowing** — Replaced 14 `.unwrap_or_default()` calls in import dialog with proper error propagation; import failures now display user-friendly messages instead of silently returning empty results
- **MobaXterm Import Double Allocation** — Removed unnecessary `.clone()` on byte buffer during UTF-8 conversion; recovers original bytes from error on fallback path instead of cloning upfront

### Improved
- **Import File Size Guard** — Added 50 MB file size limit check in `read_import_file()` to prevent OOM on accidentally selected large files
- **Native Export Streaming I/O** — `NativeExport::to_file()` now uses `BufWriter` with `serde_json::to_writer_pretty()` instead of serializing entire JSON to `String` first; eliminates intermediate allocation
- **Native Import Streaming I/O** — `NativeExport::from_file()` now uses `BufReader` with `serde_json::from_reader()` instead of reading entire file to `String`; reduces peak memory by ~50%
- **Native Import Version Pre-Check** — Version validation now runs before full deserialization; rejects unsupported format versions without parsing all connections and groups
- **Export File Writing** — Added centralized `write_export_file()` helper with `BufWriter` for consistent buffered writes across all exporters

### Refactored
- **Export Write Consolidation** — Replaced duplicated `fs::write` + error mapping boilerplate in SSH config, Ansible, Remmina, Asbru, Royal TS, and MobaXterm exporters with shared `write_export_file()` helper
- **TOCTOU Elimination** — Removed redundant `path.exists()` checks before file reads in importers; the subsequent `read_import_file()` already returns `ImportError` on failure
- **Unused Imports Cleanup** — Removed unused `ExportError` import from Asbru exporter and moved `std::fs` import to `#[cfg(test)]` in MobaXterm exporter

### Dependencies
- Updated `memchr` 2.7.6 → 2.8.0
- Updated `ryu` 1.0.22 → 1.0.23
- Updated `zerocopy` 0.8.38 → 0.8.39
- Updated `zmij` 1.0.19 → 1.0.20

## [0.7.7] - 2026-02-08

### Fixed
- **Keyboard Shortcuts** — `Delete`, `Ctrl+E`, and `Ctrl+D` no longer intercept input when VTE terminal or embedded viewers have focus; these shortcuts now only activate from the sidebar ([#4](https://github.com/totoshko88/RustConn/issues/4))

### Improved
- **Thread Safety** — Audio mutex locks use graceful fallback instead of `unwrap()`, preventing potential panics in real-time audio callbacks
- **Thread Safety** — Search engine mutex locks use graceful recovery patterns throughout `DebouncedSearchEngine`
- **Security** — VNC client logs a warning when connection is attempted without a password

### Refactored
- **Runtime Consolidation** — Replaced 23 redundant `tokio::runtime::Runtime::new()` calls across GUI code with shared `with_runtime()` pattern, reducing resource overhead
- **Collection Optimization** — Snippet tag collection uses `flat_map` with `iter().cloned()` instead of `clone()`, and `sort_unstable()` for better performance
- **Dead Code Removal** — Removed 3 deprecated blocking credential methods from `AppState` (`store_credentials`, `retrieve_credentials`, `delete_credentials`)
- **Dead Code Removal** — Removed unused `build_pane_context_menu` from `MainWindow`

## [0.7.6] - 2026-02-07

### Added
- **Flatpak Components Manager** — On-demand CLI download for Flatpak environment:
  - Menu → Flatpak Components... (visible only in Flatpak)
  - Download and install CLIs to `~/.var/app/io.github.totoshko88.RustConn/cli/`
  - Supports: AWS CLI, AWS SSM Plugin, Google Cloud CLI, Azure CLI, OCI CLI, Teleport, Tailscale, Cloudflare Tunnel, Boundary, Bitwarden CLI, 1Password CLI, TigerVNC
  - Python-based CLIs installed via pip, .deb packages extracted automatically
  - Install/Remove/Update with progress indicators and cancel support
  - SHA256 checksum verification (except AWS SSM Plugin which uses "latest" URL)
  - Settings → Clients detects CLIs installed via Flatpak Components

- **Snap Strict Confinement** — Migrated from classic to strict confinement:
  - Snap-aware path resolution for data, config, and SSH directories
  - Interface connection detection with user-friendly messages
  - Uses embedded clients (IronRDP, vnc-rs, spice-gtk) — no bundled external CLIs
  - External CLIs accessed from host via `system-files` interface

### Changed
- **Flatpak Permissions** — Simplified security model:
  - Removed `--talk-name=org.freedesktop.Flatpak` (no host command access)
  - SSH available in runtime, embedded clients for RDP/VNC/SPICE
  - Use Flatpak Components dialog to install additional CLIs

- **Snap Package** — Strict confinement with host CLI access:
  - Added plugs for ssh-keys, personal-files, system-files
  - Data stored in `~/snap/rustconn/current/`
  - Smaller package (~50 MB) using host-installed binaries

- **Settings → Clients** — Improved client detection display:
  - All protocols (SSH, RDP, VNC, SPICE) show embedded client status
  - Blue indicator (●) for embedded clients, green (✓) for external
  - Fixed AWS SSM Plugin detection (was looking for wrong binary name)

### Improved
- **UI/UX** — GNOME HIG compliance:
  - Accessible labels for status icons and protocol filter buttons
  - Sidebar minimum width increased to 200px
  - Connection dialog uses adaptive `adw::ViewSwitcherTitle`
  - Toast notifications with proper priority levels

- **Thread Safety** — Mutex poisoning recovery in FreeRDP thread

### Fixed
- **RDP Variable Substitution** — Global variables now resolve in username/domain fields

### Refactored
- **Dialog Widget Builders** — Reusable UI components (`CheckboxRowBuilder`, `EntryRowBuilder`, `SpinRowBuilder`, `DropdownRowBuilder`, `SwitchRowBuilder`)
- **Protocol Dialogs** — Applied widget builders to SSH, RDP, VNC, SPICE panels
- **Legacy Cleanup** — Removed unused `TabDisplayMode`, `TabLabelWidgets` types

### Documentation
- **New**: `docs/SNAP.md` — Snap user guide with interface setup
- **Updated**: `docs/INSTALL.md`, `docs/USER_GUIDE.md`

## [0.7.5] - 2026-02-06

### Refactored
- **Code Quality Audit** - Comprehensive codebase analysis and cleanup:
  - Removed duplicate SSH options code from `dialog.rs` (uses `ssh::create_ssh_options()`)
  - Removed duplicate VNC/SPICE/ZeroTrust options code from `dialog.rs` (~830 lines)
  - Removed duplicate RDP options code from `dialog.rs` (~350 lines, uses `rdp::create_rdp_options()`)
  - Removed legacy dialog functions (`create_automation_tab`, `create_tasks_tab`, `create_wol_tab`) (~250 lines)
  - Extracted shared folders UI into reusable `shared_folders.rs` module
  - Extracted Zero Trust UI into `zerotrust.rs` module (~450 lines)
  - Created `protocol_layout.rs` with `ProtocolLayoutBuilder` for consistent protocol UI
  - Consolidated `with_runtime()` into `async_utils.rs` (removed duplicate from `state.rs`)
  - Changed FreeRDP launcher to Wayland-first (`force_x11: false` by default)
  - Removed legacy no-op methods from terminal module (~40 lines)
  - **Total dead/duplicate code removed: ~1850+ lines**

### Fixed
- **Wayland-First FreeRDP** - External RDP client now uses Wayland backend by default:
  - Changed `SafeFreeRdpLauncher::default()` to set `force_x11: false`
  - X11 fallback still available via `with_x11_fallback()` constructor

### Changed
- **Dependencies** - Updated: proptest 1.9.0→1.10.0, time 0.3.46→0.3.47, time-macros 0.2.26→0.2.27
- **Architecture Documentation** - Updated `docs/ARCHITECTURE.md` with:
  - Current architecture diagram
  - Recommended layered architecture for future refactoring
  - Module responsibility guidelines
  - New modules: `protocol_layout.rs`, `shared_folders.rs`

## [0.7.4] - 2026-02-05

### Fixed
- **Split View Protocol Restriction** - Split view is now disabled for RDP, VNC, and SPICE tabs:
  - Only SSH, Local Shell, and ZeroTrust tabs support split view
  - Attempting to split an embedded protocol tab shows a toast notification
  - Prevents UI issues with embedded widgets that cannot be reparented
- **Split View Tab Close Cleanup** - Closing a tab now properly clears its panel in split view:
  - Panel shows "Empty Panel" placeholder with "Select Tab" button after tab is closed
  - Works for both per-session split bridges and global split view
  - Added `on_split_cleanup` callback to `TerminalNotebook` for proper cleanup coordination
  - Fixes issue where terminal content remained visible after closing tab
- **Document Close Dialog** - Fixed potential panic when closing document without parent window:
  - `CloseDocumentDialog::present()` now gracefully handles missing parent window
  - Logs error and calls callback with `None` instead of panicking
- **Zero Trust Entry Field Alignment** -додай зміни в чендлог і онови architecture.md в doc Fixed inconsistent width of input fields in Zero Trust provider panels:
  - Converted all Zero Trust provider fields from `ActionRow` + `Entry` to `adw::EntryRow`
  - All 10 provider panels (AWS SSM, GCP IAP, Azure Bastion, Azure SSH, OCI Bastion, Cloudflare, Teleport, Tailscale, Boundary, Generic) now have consistent field widths
  - Follows GNOME HIG guidelines for proper libadwaita input field usage

### Refactored
- **Import File I/O** - Extracted common file reading pattern into `read_import_file()` helper:
  - Reduces code duplication across 5 import sources (SSH config, Ansible, Remmina, Asbru, Royal TS)
  - Consistent error handling with `ImportError::ParseError`
  - Added async variant `read_import_file_async()` for future use
- **Protocol Client Errors** - Consolidated duplicate error types into unified `EmbeddedClientError`:
  - Merged `RdpClientError`, `VncClientError`, `SpiceClientError` (~60 lines reduced)
  - Type aliases maintain backward compatibility
  - Common variants: `ConnectionFailed`, `AuthenticationFailed`, `ProtocolError`, `IoError`, `Timeout`
- **Config Atomic Writes** - Improved reliability of configuration file saves:
  - Now uses temp file + atomic rename pattern
  - Prevents config corruption on crash during write
  - Applied to `save_toml_file_async()` in `ConfigManager`
- **Connection Dialog Modularization** - Refactored monolithic `connection.rs` into modular structure:
  - Created `rustconn/src/dialogs/connection/` directory with protocol-specific modules
  - `dialog.rs` - Main `ConnectionDialog` implementation (~6,600 lines)
  - `ssh.rs` - SSH options panel (~460 lines, prepared for future integration)
  - `rdp.rs` - RDP options panel (~414 lines, prepared for future integration)
  - `vnc.rs` - VNC options panel (~249 lines, prepared for future integration)
  - `spice.rs` - SPICE options panel (~240 lines, reuses rdp:: folder functions)
  - Improves code organization and maintainability

### Added
- **Variables Menu Item** - Added "Variables..." menu item to Tools menu for managing global variables:
  - Opens Variables dialog to view/edit global variables
  - Variables are persisted to settings and substituted at connection time
  - Accessible via Tools → Variables...
- **GTK Lifecycle Documentation** - Added module-level documentation explaining `#[allow(dead_code)]` pattern:
  - Documents why GTK widget fields must be kept alive for signal handlers
  - Prevents accidental removal of "unused" fields that would cause segfaults
- **Type Alias Documentation** - Added documentation explaining why `Rc` is used instead of `Arc`:
  - GTK4 is single-threaded, so atomic operations are unnecessary overhead
  - `Rc<RefCell<_>>` pattern matches GTK's single-threaded model
  - Documented in `window_types.rs` module header

### Changed
- **Dialog Size Unification** - Standardized dialog window sizes for visual consistency:
  - Connection History: 750×500 (increased from 550 for better content display)
  - Keyboard Shortcuts: 550×500 (increased from 500 for consistency)
- **Code Quality** - Comprehensive cleanup based on code audit:
  - Removed legacy `TabDisplayMode`, `SessionWidgetStorage`, `TabLabelWidgets` types
  - Standardized error type patterns with `#[from]` attribute
  - Reduced unnecessary `.clone()` calls in callback chains
  - Improved `expect()` messages to clarify provably impossible states
  - Added `# Panics` documentation for functions with justified `expect()` calls
- **Dependencies** - Updated: clap 4.5.56→4.5.57, criterion 0.8.1→0.8.2, hybrid-array 0.4.6→0.4.7, zerocopy 0.8.37→0.8.38

### Tests
- Updated property tests for consolidated error types
- Verified all changes pass `cargo clippy --all-targets` and `cargo fmt --check`

## [0.7.3] - 2026-02-03

### Fixed
- **Azure CLI Version Parsing** - Fixed version detection showing "-" instead of actual version:
  - Added dedicated parser for Azure CLI's unique output format (`azure-cli  2.82.0 *`)
  - Version now correctly extracted and displayed in Settings → Clients
- **Teleport CLI Version Parsing** - Fixed version showing full output instead of clean version:
  - Added dedicated parser for Teleport's output format (`Teleport v18.6.5 git:...`)
  - Now displays clean version like `v18.6.5`
- **Flatpak XDG Config** - Removed unnecessary `--filesystem=xdg-config/rustconn:create` permission:
  - Flatpak sandbox automatically provides access to `$XDG_CONFIG_HOME`
  - Configuration now stored in standard Flatpak location (`~/.var/app/io.github.totoshko88.RustConn/config/`)
- **Teleport CLI Detection** - Fixed detection using wrong binary name (`teleport` → `tsh`)

### Changed
- **RDP Client Detection** - Improved FreeRDP detection with Wayland support:
  - Priority order: FreeRDP 3.x (wlfreerdp3/xfreerdp3) → FreeRDP 2.x (wlfreerdp/xfreerdp) → rdesktop
  - Wayland-native clients (wlfreerdp3/wlfreerdp) now checked before X11 variants
  - Updated install hint to recommend freerdp3-wayland package
- **Client Install Hints** - Unified and improved package installation messages:
  - Format: `Install <deb-package> (<rpm-package>) package`
  - SSH: `openssh-client (openssh-clients)`
  - RDP: `freerdp3-wayland (freerdp)`
  - VNC: `tigervnc-viewer (tigervnc)`
  - Zero Trust CLIs: simplified to package names only
- **Dependencies** - Updated: bytes 1.11.0→1.11.1, flate2 1.1.8→1.1.9, regex 1.12.2→1.12.3

### Refactored
- **Client Detection** - Unified detection logic in `rustconn-core`:
  - Removed duplicate version parsing from `clients_tab.rs` (~200 lines)
  - Added `detect_spice_client()` to core detection module
  - Added `ZeroTrustDetectionResult` struct for all Zero Trust CLI clients
  - GUI now uses `ClientDetectionResult` and `ZeroTrustDetectionResult` from core

## [0.7.2] - 2026-02-03

### Added
- **Flatpak Host Command Support** - New `flatpak` module for running host commands from sandbox:
  - `is_flatpak()` - Detects if running inside Flatpak sandbox
  - `host_command()` - Creates command that runs on host via `flatpak-spawn --host`
  - `host_has_command()`, `host_which()` - Check for host binaries
  - `host_exec()`, `host_spawn()` - Execute/spawn host commands
  - Enables external clients (xfreerdp, vncviewer, aws, gcloud) to work in Flatpak

### Changed
- **Dependencies** - Updated: hyper-util 0.1.19→0.1.20, system-configuration 0.6.1→0.7.0, zmij 1.0.18→1.0.19
- **Flatpak Permissions** - Extended sandbox permissions for full functionality:
  - `xdg-config/rustconn:create` - Config directory access
  - `org.freedesktop.Flatpak` - Host command execution (xfreerdp, vncviewer, aws, etc.)
  - `org.freedesktop.secrets` - GNOME Keyring access
  - `org.kde.kwalletd5/6` - KWallet access
  - `org.keepassxc.KeePassXC.BrowserServer` - KeePassXC proxy
  - `org.kde.StatusNotifierWatcher` - System tray support

### Fixed
- **Flatpak Config Access** - Added `xdg-config/rustconn:create` permission to Flatpak manifests:
  - Connections, groups, snippets, and settings now persist correctly in Flatpak sandbox
  - Previously, Flatpak sandbox blocked access to `~/.config/rustconn`
- **Split View Equal Proportions** - Fixed split panels having unequal sizes:
  - Changed from timeout-based to `connect_map` + `idle_add` for reliable size detection
  - Panels now correctly split 50/50 regardless of timing or rendering delays
  - Added `shrink_start_child` and `shrink_end_child` for balanced resizing

## [0.7.1] - 2026-02-01

### Added
- **Undo/Trash Functionality** - Safely recover deleted items (COMP-FUNC-01):
  - Deleted items are moved to Trash and can be restored via "Undo" notification
  - Implemented persisted Trash storage for recovery across sessions
- **Group Inheritance** - Simplify connection configuration (COMP-FUNC-03):
  - Added ability to inherit Username and Domain from parent Group
  - "Load from Group" buttons auto-fill credential fields from group settings

### Changed
- **Dependencies** - Updated: bytemuck 1.24.0→1.25.0, portable-atomic 1.13.0→1.13.1, slab 0.4.11→0.4.12, zerocopy 0.8.36→0.8.37, zerocopy-derive 0.8.36→0.8.37, zmij 1.0.17→1.0.18
- **Persistence Optimization** - Implemented debounced persistence for connections and groups (TECH-02):
  - Changes are now batched and saved after 2 seconds of inactivity
  - Reduces disk I/O during rapid modifications (e.g., drag-and-drop reordering)
  - Added `flush_persistence` to ensure data safety on application exit
- **Sort Optimization** - Improved rendering performance (COMP-FUNC-02):
  - Sorting is now skipped when data order hasn't changed, reducing CPU usage
  - Optimized `sort_all` calls during UI updates
- **Connection History Sorting** - History entries now sorted by date descending (newest first)

### Fixed
- **Credential Inheritance from Groups** - Fixed password inheritance not working for connections:
  - Connections with `password_source=Inherit` now correctly resolve credentials from parent group's KeePass entry
  - Added direct KeePass lookup for group credentials in `resolve_credentials_blocking`
- **GTK Widget Parenting** - Fixed `gtk_widget_set_parent` assertion failure in split view:
  - `set_panel_content` now checks if widget has parent before calling `unparent()`
- **Connection History Reconnect** - Fixed reconnecting from Connection History not opening tab:
  - History reconnect now uses `start_connection_with_credential_resolution` for proper credential handling
  - Previously showed warning about missing credentials for RDP connections
- **Blocking I/O** - Fixed UI freezing during save operations by moving persistence to background tasks (Async Persistence):
  - Added global Tokio runtime to main application
  - Implemented async save methods in `ConfigManager`
  - `ConnectionManager` now saves connections and groups in non-blocking background tasks
- **Code Quality** - Comprehensive code cleanup and optimization:
  - Fixed `future_not_send` issues in async persistence layer
  - Resolved type complexity warnings in `ConnectionManager`
  - Removed dead code and unused imports across sidebar modules
  - Enforced `clippy` pedantic checks for better robustness

### Refactored
- **Sidebar Module** - Decomposed monolithic `sidebar.rs` into focused submodules (TECH-03):
  - `search.rs`: Encapsulated search logic, predicates, and history management
  - `filter.rs`: centralized protocol filter button creation and state management
  - `view.rs`: Isolated UI list item creation, binding, and signal handling
  - `drag_drop.rs`: Prepared structure for drag-and-drop logic separation
  - Improved compile times and navigation by splitting 2300+ line file
- **Drag and Drop Refactoring** - Replaced string-based payloads with strongly typed `DragPayload` enum (TECH-04):
  - Uses `serde_json` for robust serialization instead of manual string parsing
  - Centralized drag logic in `drag_drop.rs`
  - Improved type safety for drag-and-drop operations

### UI/UX
- **Search Highlighting** - Added visual feedback for search matches (TECH-05):
  - Matched text substrings are now highlighted in bold
  - Implemented case-insensitive fuzzier matching with Pango markup
  - Improved `Regex`-based search logic

## [0.7.0] - 2026-02-01

### Fixed
- **Asbru Import Nested Groups** - Fixed group hierarchy being lost when importing from Asbru-CM:
  - Groups with subgroups (e.g., Group1 containing Group11, Group12, etc.) now correctly preserve parent-child relationships
  - Previously, HashMap iteration order caused child groups to be processed before their parents were added to the UUID map, resulting in orphaned root-level groups
  - Now uses two-pass algorithm: first creates all groups and populates UUID map, then resolves parent references
  - Special Asbru parent keys (`__PAC__EXPORTED__`, `__PAC__ROOT__`) are now properly skipped
- **Asbru Export Description Field** - Fixed description not being exported for connections and groups:
  - Connection description now exports from `connection.description` field directly
  - Falls back to legacy `desc:` tags only if description field is empty
  - Group description now exports when present

### Added
- **Group Description Field** - Groups can now have a description field for storing project info, contacts, notes:
  - Added `description: Option<String>` to `ConnectionGroup` model
  - Asbru importer now imports group descriptions
  - Edit Group dialog now includes Description text area for viewing/editing
  - New Group dialog now includes Description text area (unified with Edit Group)
- **Asbru Global Variable Conversion** - Asbru-CM global variable syntax is now converted during import:
  - `<GV:VAR_NAME>` is automatically converted to RustConn syntax `${VAR_NAME}`
  - Applies to username field (e.g., `<GV:US_Parrallels_User>` → `${US_Parrallels_User}`)
  - Plain usernames remain unchanged
- **Variable Substitution at Connection Time** - Global variables are now resolved when connecting:
  - `${VAR_NAME}` in host and username fields are replaced with variable values
  - Works for SSH, RDP, VNC, and SPICE connections
  - Variables are defined in Settings → Variables

### Changed
- **Export Dialog** - Added informational message about credential storage:
  - New info row explains that passwords are stored in password manager and not exported by default
  - Reminds users to export credential structure separately if needed for team sharing
- **Dialog Size Unification** - Standardized dialog window sizes for visual consistency:
  - New Group dialog: 450×550 (added Description field, unified with Edit Group)
  - Export dialog: 750×650 (increased height for content)
  - Import dialog: 750×800 (increased height for content)
  - Medium forms (550×550): New Snippet, New Cluster, Statistics
  - Info dialogs (500×500): Keyboard Shortcuts, Connection History
  - Simple forms (450): Quick Connect, Edit Group, Rename
  - Password Generator: 750×650 (unified with Connection/Template dialogs)

## [0.6.9] - 2026-01-31

### Added
- **Password Caching TTL** - Cached credentials now expire after configurable time (default 5 minutes):
  - `CachedCredentials` with `cached_at` timestamp and `is_expired()` method
  - `cleanup_expired_credentials()` for automatic cleanup
  - `refresh_cached_credentials()` to extend TTL on use
- **Connection Retry Logic** - Automatic retry with exponential backoff for failed connections:
  - `RetryConfig` with max_attempts, base_delay, max_delay, jitter settings
  - `RetryState` for tracking retry progress
  - Preset configurations: `aggressive()`, `conservative()`, `no_retry()`
- **Loading States** - Visual feedback for long-running operations:
  - `LoadingOverlay` component for inline loading indicators
  - `LoadingDialog` for modal operations with cancel support
  - `with_loading_dialog()` helper for async operations
- **Keyboard Navigation Helpers** - Improved dialog keyboard support:
  - `setup_dialog_shortcuts()` for Escape/Ctrl+S/Ctrl+W
  - `setup_entry_activation()` for Enter key handling
  - `make_default_button()` and `make_destructive_button()` styling helpers
- **Session State Persistence** - Split layouts preserved across restarts:
  - `SessionRestoreData` and `SplitLayoutRestoreData` structs
  - JSON serialization for session state
  - Automatic save/load from config directory
- **Connection Health Check** - Periodic monitoring of active sessions:
  - `HealthStatus` enum (Healthy, Unhealthy, Unknown, Terminated)
  - `HealthCheckConfig` with interval and auto_cleanup settings
  - `perform_health_check()` and `get_session_health()` methods
- **Log Sanitization** - Automatic removal of sensitive data from logs:
  - `SanitizeConfig` with patterns for passwords, API keys, tokens
  - AWS credentials and private key detection
  - `contains_sensitive_prompt()` helper
- **Async Architecture Helpers** - Improved async handling in GUI:
  - `spawn_async()` for non-blocking operations
  - `spawn_async_with_callback()` for result handling
  - `block_on_async_with_timeout()` for bounded blocking
  - `is_main_thread()` and `ensure_main_thread()` utilities
- **RDP Backend Selector** - Centralized RDP backend selection:
  - `RdpBackend` enum (IronRdp, WlFreeRdp, XFreeRdp3, XFreeRdp, FreeRdp)
  - `RdpBackendSelector` with detection caching
  - `select_embedded()`, `select_external()`, `select_best()` methods
- **Import/Export Enhancement** - Detailed import statistics:
  - `SkippedField` and `SkippedFieldReason` for tracking skipped data
  - `ImportStatistics` with detailed reporting
  - `detailed_report()` for human-readable summaries
- **Bulk Credential Operations** - Mass credential management:
  - `store_bulk()`, `delete_bulk()`, `update_bulk()` methods
  - `update_credentials_for_group()` for group-wide updates
  - `copy_credentials()` between connections
- **1Password as PasswordSource** - 1Password can now be selected per-connection:
  - Added `OnePassword` variant to `PasswordSource` enum
  - 1Password option in password source dropdown (index 4)
  - Password save/load support for 1Password backend
  - Default selection based on `preferred_backend` setting
- **Credential Rename on Connection Rename** - Credentials are now automatically renamed in secret backends when connection is renamed:
  - KeePass: Entry path updated to match new connection name
  - Keyring: Entry key updated from old to new name format
  - Bitwarden: Entry name updated to match new connection name
  - 1Password: Uses connection ID, no rename needed

### Changed
- **Safe State Access** - New helpers to reduce RefCell borrow panics:
  - `with_state()` and `try_with_state()` for read access
  - `with_state_mut()` and `try_with_state_mut()` for write access
- **Toast Queue** - Fixed toast message sequencing with `schedule_toast_hide()` helper

### Fixed
- **KeePass Password Retrieval for Subgroups** - Fixed password not being retrieved when connection is in nested groups:
  - Save and read operations now both use hierarchical paths via `KeePassHierarchy::build_entry_path()`
  - Paths like `RustConn/Group1/Group2/ConnectionName (protocol)` are now consistent
- **Keyring Password Retrieval** - Fixed password never found after saving:
  - Save used `"{name} ({protocol})"` format, read used UUID
  - Now both use `"{name} ({protocol})"` with legacy UUID fallback
- **Bitwarden Password Retrieval** - Fixed password never found after saving:
  - Save used `"{name} ({protocol})"` format, read used `"rustconn/{name}"`
  - Now both use `"{name} ({protocol})"` with legacy format fallback
- **Status Icon on Tab Close** - Status icons now clear when closing RDP/SSH tabs:
  - Previously showed red/green status for closed connections
  - Now clears status (empty string) instead of setting "failed"/"disconnected"

### Tests
- Added 370+ new property tests (total: 1241 tests):
  - `vnc_client_tests.rs` - VNC client configuration and events (28 tests)
  - `terminal_theme_tests.rs` - Terminal theme parsing (26 tests)
  - `error_tests.rs` - Error type coverage (45 tests)
  - `retry_tests.rs` - Retry logic (14 tests)
  - `session_restore_tests.rs` - Session persistence (10 tests)
  - `rdp_backend_tests.rs` - RDP backend selection (13 tests)
  - `log_sanitization_tests.rs` - Log sanitization (19 tests)
  - `health_check_tests.rs` - Health monitoring (13 tests)
  - `bulk_credential_tests.rs` - Bulk operations (25 tests)
  - `import_statistics_tests.rs` - Import statistics (28 tests)
  - And more...

### Fixed
- **Local Shell in Split View** - Local Shell tabs can now be added to split view panels:
  - Fixed protocol filter that excluded "local" protocol from available sessions
  - Multiple Local Shell tabs now appear in "Select Tab" dialog for split panels

## [0.6.8] - 2026-01-30

### Added
- **1Password CLI Integration** - New secret backend for 1Password password manager:
  - Full `SecretBackend` trait implementation with async credential resolution
  - Uses `op` CLI v2 with desktop app integration (biometric authentication)
  - Service account support via `OP_SERVICE_ACCOUNT_TOKEN` environment variable
  - Automatic vault creation ("RustConn" vault) for storing credentials
  - Items tagged with "rustconn" for easy filtering
  - Account status checking with `op whoami`
  - Settings UI with version display and sign-in status indicator
  - "Sign In" button opens terminal for interactive `op signin`
- **1Password Detection** - `detect_onepassword()` function in detection module:
  - Checks multiple paths for `op` CLI installation
  - Reports version, sign-in status, and account email
  - Integrated into `detect_password_managers()` for unified discovery
- **Bitwarden API Key Authentication** - New `login_with_api_key()` function:
  - Uses `BW_CLIENTID` and `BW_CLIENTSECRET` environment variables
  - Recommended for automated workflows and CI/CD pipelines
- **Bitwarden Self-Hosted Support** - New `configure_server()` function:
  - Configure CLI to use self-hosted Bitwarden server
- **Bitwarden Logout** - New `logout()` function for session cleanup

### Changed
- `SecretBackendType` enum extended with `OnePassword` variant
- Connection dialog password source dropdown now includes 1Password (index 4)
- Settings → Secrets tab shows 1Password configuration group when selected
- Property test generators updated to include `Bitwarden` and `OnePassword` variants
- **Bitwarden unlock** now uses `--passwordenv` option as recommended by official documentation (more secure than stdin)
- **Bitwarden retrieve** now syncs vault before lookup to ensure latest credentials
- **Dependencies** - Updated: cc 1.2.54→1.2.55, find-msvc-tools 0.1.8→0.1.9

## [Unreleased] - 0.6.7

### Added
- **Group-Level Secret Storage** - Groups can now store passwords in secret backends:
  - Auto-select password backend based on application settings when creating groups
  - "Load from vault" button to retrieve group passwords from KeePass/Keyring/Bitwarden
  - Hierarchical storage in KeePass: `RustConn/Groups/{path}` mirrors group structure
  - New `build_group_entry_path()` and `build_group_lookup_key()` functions in hierarchy module
- **CLI Secret Management** - New `secret` command for managing credentials from command line:
  - `rustconn-cli secret status` - Show available backends and their status
  - `rustconn-cli secret get <connection>` - Retrieve credentials for a connection
  - `rustconn-cli secret set <connection>` - Store credentials (interactive password prompt)
  - `rustconn-cli secret delete <connection>` - Delete credentials from backend
  - `rustconn-cli secret verify-keepass` - Verify KeePass database credentials
  - Supports `--backend` flag to specify keyring, keepass, or bitwarden

### Changed
- **Dependencies** - Updated: clap 4.5.55→4.5.56, clap_builder 4.5.55→4.5.56, zerocopy 0.8.35→0.8.36, zerocopy-derive 0.8.35→0.8.36, zune-jpeg 0.5.11→0.5.12
- **MSRV** - Synchronized `.clippy.toml` MSRV from 1.87 to 1.88 to match `Cargo.toml`

### Fixed

## [0.6.6] - 2026-01-29

### Added
- **KeePass Password Saving for RDP/VNC** - Fixed password saving when creating/editing connections with KeePass password source:
  - Connection dialog now returns password separately from connection object
  - Password is saved to KeePass database when password source is set to KeePass
  - Works for new connections, edited connections, and template-based connections
- **Load Password from Vault** - New button in connection dialog to load password from KeePass or Keyring:
  - Click the folder icon next to the Value field to load password from configured vault
  - Works with KeePass (KDBX) and system Keyring (libsecret) backends
  - Automatically uses connection name and protocol for lookup key
  - Shows loading indicator during retrieval
- **Keyring Password Storage** - Passwords are now saved to system Keyring when password source is set to Keyring:
  - Uses libsecret via `secret-tool` CLI for GNOME Keyring / KDE Wallet integration
  - Passwords stored with connection name and protocol as lookup key
  - Requires `libsecret-tools` package to be installed
- **SSH X11 Forwarding & Compression** - New SSH session options:
  - X11 Forwarding (`-X` flag) for running graphical applications on remote hosts
  - Compression (`-C` flag) for faster transfer over slow connections
  - GUI controls in Connection dialog → SSH → Session group
  - CLI support via `rustconn-cli connect` (reads from connection config)
  - Import support: Asbru-CM (`-X`, `-C`, `-A` flags), SSH config (`ForwardX11`, `Compression`), Remmina (`ssh_tunnel_x11`, `ssh_compression`)
- **Import Normalizer** - New `ImportNormalizer` module for post-import consistency:
  - Group deduplication (merges groups with same name and parent)
  - Port normalization to protocol defaults
  - Auth method normalization based on key_path presence
  - Key path validation and tilde expansion
  - Import source/timestamp tags for tracking
  - Helper functions: `parse_host_port()`, `is_valid_hostname()`, `looks_like_hostname()`
- **IronRDP Enhanced Features** - Major expansion of embedded RDP client capabilities:
  - **Reconnection support** (`reconnect.rs`): `ReconnectPolicy` with exponential backoff and jitter, `ReconnectState` tracking, `DisconnectReason` classification, `ConnectionQuality` monitoring (RTT, FPS, bandwidth)
  - **Multi-monitor preparation** (`multimonitor.rs`): `MonitorDefinition` with position/DPI, `MonitorLayout` configuration, `MonitorArrangement` modes (Extend/Duplicate/PrimaryOnly), `detect_monitors()` helper
  - **RD Gateway support** (`gateway.rs`): `GatewayConfig` with hostname/auth/bypass, `GatewayAuthMethod` (NTLM/Kerberos/SmartCard/Basic/Cookie), automatic local address bypass
  - **Graphics modes** (`graphics.rs`): `GraphicsMode` selection (Auto/Legacy/RemoteFX/GFX/H264), `ServerGraphicsCapabilities` detection, `GraphicsQuality` presets, `FrameStatistics` for performance monitoring
  - **Extended RdpClientConfig**: gateway, monitor_layout, reconnect_policy, graphics_mode, graphics_quality, remote_app (RemoteApp), printer/smartcard/microphone redirection flags, `validate()` method

### Changed
- **RDP Performance Mode** - Performance mode setting now controls bitmap compression and codec selection:
  - **Quality (RemoteFX)**: Lossless compression with RemoteFX codec for best visual quality
  - **Balanced (Adaptive)**: Lossy compression with RemoteFX codec for adaptive quality/bandwidth tradeoff
  - **Speed (Legacy)**: Lossy compression with legacy bitmap codec for slow connections
  - All modes use 32-bit color depth for AWS EC2 Windows server compatibility
- **Remmina Importer** - Major refactor for proper group support:
  - Changed from tags (`remmina:{group}`) to real `ConnectionGroup` objects
  - Added nested group support (e.g., "Production/Web Servers" creates hierarchy)
  - Added SPICE protocol support
- **RDM Importer** - Added SSH key support:
  - Parses `PrivateKeyPath` field from RDM JSON
  - Sets `auth_method` to `PublicKey` when key present
  - Added `view_only` support for VNC connections
- **Royal TS Importer** - Added SSH key support:
  - Parses `PrivateKeyFile`, `KeyFilePath`, `PrivateKeyPath` fields
  - Sets `auth_method` based on key presence
  - Tilde expansion for key paths
- **SSH Config Importer** - Enhanced option parsing:
  - Now preserves `ServerAliveInterval`, `ServerAliveCountMax`, `TCPKeepAlive`
  - Preserves `Compression`, `ConnectTimeout`, `ConnectionAttempts`
  - Preserves `StrictHostKeyChecking`, `UserKnownHostsFile`, `LogLevel`
- **Dependencies** - Updated: aws-lc-rs 1.15.3→1.15.4, aws-lc-sys 0.36.0→0.37.0, cc 1.2.53→1.2.54, cfg-expr 0.20.5→0.20.6, hybrid-array 0.4.5→0.4.6, libm 0.2.15→0.2.16, moka 0.12.12→0.12.13, notify-types 2.0.0→2.1.0, num-conv 0.1.0→0.2.0, proc-macro2 1.0.105→1.0.106, quote 1.0.43→1.0.44, siphasher 1.0.1→1.0.2, socket2 0.6.1→0.6.2, time 0.3.45→0.3.46, time-core 0.1.7→0.1.8, time-macros 0.2.25→0.2.26, uuid 1.19.0→1.20.0, yuv 0.8.9→0.8.10, zerocopy 0.8.33→0.8.34, zmij 1.0.16→1.0.17

### Fixed
- **AWS EC2 RDP Compatibility** - Fixed IronRDP connection failures with AWS EC2 Windows servers by using 32-bit color depth in `BitmapConfig` (24-bit caused connection reset during `BasicSettingsExchange` phase)
- **GCloud Provider Detection** - Fixed GCloud commands being incorrectly detected as AWS when instance names contain patterns resembling EC2 instance IDs (e.g., `ai-0000a00a`). GCloud patterns are now checked before AWS instance ID patterns

### Refactored
- **Display Server Detection** - Consolidated duplicate display server detection code from `embedded.rs` and `wayland_surface.rs` into a unified `display.rs` module with cached detection and comprehensive capability methods
- **Sidebar Filter Buttons** - Reduced code duplication in sidebar filter button creation and event handling with `create_filter_button()` and `connect_filter_button()` helper functions
- **Window UI Components** - Extracted header bar and application menu creation from `window.rs` into dedicated `window_ui.rs` module

## [0.6.5] - 2026-01-21

### Changed
- **Split View Redesign** - Complete rewrite of split view functionality with tab-scoped layouts:
  - Each tab now maintains its own independent split layout (no more global split state)
  - Tree-based panel structure supporting unlimited nested splits
  - Color-coded panel borders (6 colors) to visually identify split containers
  - All panels within the same split container now share the same border color (per design spec)
  - Tab color indicators match their container's color when in split view
  - "Select Tab" button in empty panels as alternative to drag-and-drop
  - Proper cleanup when closing split view (colors released, terminals reparented)
  - When last panel is closed, split view closes and session returns to regular tab
  - New `rustconn-core/src/split/` module with GUI-free split layout logic
  - Comprehensive property tests for split view operations
- **Terminal Tabs Migration** - Migrated terminal notebook from `gtk::Notebook` to `adw::TabView`:
  - Modern GNOME HIG compliant tab bar with `adw::TabBar`
  - Native tab drag-and-drop support
  - Automatic tab overflow handling
  - Better integration with libadwaita theming
  - Improved accessibility with proper ARIA labels
- **Dependencies** - Updated: thiserror 2.0.18, zbus 5.13.2, zvariant 5.9.2, euclid 0.22.13, openssl-probe 0.2.1, zmij 1.0.16, zune-jpeg 0.5.11

### Fixed
- **KeePass Password Saving** - Fixed "Failed to Save Password" error when connection name contains `/` character (e.g., connections in subgroups). Now sanitizes lookup keys by replacing `/` with `-`
- **Connection Dialog Password Field** - Renamed "Password:" label to "Value:" and added show/hide toggle button. Field visibility now depends on password source selection (hidden for Prompt/Inherit/None, shown for Stored/KeePass/Keyring)
- **Group Dialog Password Source** - Added password source dropdown (Prompt, Stored, KeePass, Keyring, Inherit, None) with Value field and show/hide toggle to group dialogs
- **Template Dialog Field Alignment** - Changed Basic tab fields from `Entry` to `adw::EntryRow` for proper width stretching consistent with Connection dialog
- **CSS Parser Errors** - Removed unsupported `:has()` pseudoclass from CSS rules, eliminating 6 "Unknown pseudoclass" errors on startup
- **zbus DEBUG Spam** - Added tracing filter to suppress verbose zbus DEBUG messages (`zbus=warn` directive)
- **Split View "Loading..." Panels** - Fixed panels getting stuck showing "Loading..." after multiple splits and "Select Tab" operations:
  - Terminals moved via "Select Tab" are now stored in bridge's internal map for restoration
  - `restore_panel_contents()` is now called after each split to restore terminal content
  - `show_session()` is only called on first split; subsequent splits preserve existing panel content
- **Split View Context Menu Freeze** - Fixed window freeze when right-clicking in split view panels. Context menu popover is now created dynamically on each click to avoid GTK popup grabbing conflicts
- **Split View Tab Colors** - Fixed tabs in the same split container having different colors. Now all tabs/panels within a split container share a single container color (allocated once on first split)
- Empty panel close button now properly triggers panel removal and split view cleanup
- Focus rectangle properly follows active panel when clicking or switching tabs

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
