# Changelog

All notable changes to RustConn will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.13.14] - 2026-05-13

### Added
- **Welcome page: Import button** — added "Import" action button alongside "New Connection" and "Quick Connect" on the welcome page, making it easier for users migrating from PuTTY, Remmina, mRemoteNG, or SecureCRT to get started
- **Template Manager: empty state** — added `adw::StatusPage` placeholder with icon and description when no templates exist, consistent with Recordings and History dialogs
- **Reconnect banner: auto-reconnect indicator** — when auto-reconnect is active, the disconnected session banner now shows "Auto-reconnecting…" status label so users know background polling is in progress

### Fixed
- **Credential memory safety** — intermediate password strings from `expose_secret().to_string()` are now wrapped in `zeroize::Zeroizing` across VNC, RDP, and document password flows; passwords are zeroed in memory on drop instead of lingering as plain `String`
- **Potential panic in resize debounce** — replaced `unwrap()` on `Instant::checked_sub()` with `unwrap_or_else` fallback in terminal resize handler (`window/mod.rs`)
- **CLI `show` command panic** — replaced `expect("json object")` with proper `let-else` error propagation in `rustconn-cli`
- **Port overflow in SecureCRT/libvirt importers** — replaced truncating `as u16` casts with `u16::try_from().ok()` fallback to default port; prevents silent corruption when imported files contain port values > 65535
- **Sync file path traversal** — added `validate_sync_filename()` that rejects absolute paths, `..` components, and directory separators in `sync_file` field; prevents writing outside the configured sync directory via crafted `.rcn` files

### Dependencies
- **Updated**: filetime 0.2.28→0.2.29, libbz2-rs-sys 0.2.3→0.2.4, open 5.3.4→5.3.5, zerofrom 0.1.7→0.1.8

## [0.13.13] - 2026-05-12

### Added
- **SSH ProxyCommand support** — new "ProxyCommand" field in the SSH connection dialog allows routing connections through a custom proxy (e.g., `ncat --proxy 127.0.0.1:9050 --proxy-type socks5 %h %p` for Tor `.onion` hidden services); skips pre-connect port check when set, since the destination is only reachable through the proxy (fixes [#146](https://github.com/totoshko88/RustConn/issues/146))

### Fixed
- **SSH Startup Command not executing** — the startup command configured in the SSH connection dialog was never appended to the SSH invocation in the GUI terminal; now correctly passed after `user@host` so the remote shell executes it immediately after login (fixes [#147](https://github.com/totoshko88/RustConn/issues/147))
- **SSH ProxyCommand port format** — jump hosts with non-standard ports now correctly use `-p port user@host` instead of invalid `user@host:port` inside `ProxyCommand` (fixes [#144](https://github.com/totoshko88/RustConn/issues/144))
- **RDP/VNC/SPICE tunnel through nested jump hosts** — SSH tunnel now resolves the full recursive jump host chain; previously only the immediate jump host was used, causing tunnel failure when the jump host itself required another jump host to be reachable

### Improved
- **StringInterner: `HashSet` instead of `HashMap`** — reduced memory overhead by 50% per entry (key and value were identical `Arc<str>`)
- **ConfigManager: cache `ensure_config_dir()` result** — skips filesystem check after first successful call, eliminates 1 syscall per debounced save
- **ConnectionManager: `collect_descendant_groups()` O(n) instead of O(n²)** — builds parent→children index instead of scanning all groups on each iteration
- **ConnectionManager: `sort_all()` refactor** — extracted `sort_ids_by_name()` helper, removed 4× duplicated sort-by-lowercase pattern (~60 lines)
- **WolDialog: migrated to `adw::Dialog`** — better focus management, auto close-on-Escape, bottom-sheet behavior on narrow windows (GNOME HIG)

## [0.13.12] - 2026-05-11

### Added
- **Auto-reconnect: per-connection RetryConfig with exponential backoff** — new "Automatic Reconnection" section in the connection dialog Advanced tab; configurable retry behavior: enable/disable, max attempts (1–10), initial delay (100–30000ms), max delay (1000–120000ms); exponential backoff with 2× multiplier; `RetryConfig` serialized with `#[serde(default)]` for backward compatibility
- **Import: multi-file batch import** — new "Multiple Files (batch)" source in the Import dialog; select multiple files at once (CSV, RDP, VV, RCN, JSON, RTSZ, MobaXterm, XML, YAML); sequential import with per-file progress; `BatchImporter` for large sets (>10) with configurable batch sizes and cancellation
- **ExpectEngine: new methods for GUI integration** — `match_line()` with auto-trimming and priority; `remove_by_id(Uuid)`; `remove_expired()` and `remove_expired_individual()` for per-rule timeouts

### Improved
- **Import: success messages now use i18n_f()** for proper localization
- **AutomationSession uses ExpectEngine from core** — delegates to `ExpectEngine` for priority-sorted matching, duplicate ID detection, pattern validation, and timeout handling; `Trigger` struct removed
- **SplitView legacy UUID layer partially removed** — external consumers use `get_pane_session()` instead of `panes_ref_clone()`; `TerminalPane` reduced to `pub(crate)`; `panes_ref()`, `panes_ref_clone()` removed from public API
- **`performance/mod.rs` decomposed** — 2210-line monolith split into 10 submodules; public API unchanged
- **`cli_download.rs` decomposed** — 3391-line monolith split into 10 submodules; public API unchanged
- **MainWindow credential resolution extracted** — 9 methods (~920 lines) moved to `window/credentials.rs`
- **MainWindow session lifecycle extracted** — 5 methods (~760 lines) moved to `window/session_lifecycle.rs`; `window/mod.rs` reduced by 31%

### Fixed
- **SSH: identity key `-i` duplicated in command** — `build_command_args()` and `spawn_ssh()` both added `-i`; now deduplicated in both connect and reconnect paths
- **CSV import: panic on empty group path** — `resolve_group_path()` now returns `ImportError::InvalidEntry` instead of `expect()`
- **VNC/RDP/SPICE embedded: hangs when remote port unreachable through SSH tunnel** — `probe_tunnel_remote()` now verifies end-to-end connectivity after tunnel readiness; fails immediately with clear error instead of hanging
- **Terminal: per-connection white color displayed as grey** — `apply_theme_override_with_base()` now rebuilds full 16-color palette via `set_colors()`, replacing palette entries 7+15 and 0+8 ([#145](https://github.com/totoshko88/RustConn/issues/145))

### Dependencies
- `clap_complete` 4.6.4 → 4.6.5
- `kqueue-sys` 1.1.0 → 1.1.1
- `nix` 0.31.2 → 0.31.3

## [0.13.11] - 2026-05-10

### Improved
- **RDP: better diagnostics for IronRDP fallback to FreeRDP** — when the embedded IronRDP client encounters a protocol incompatibility (e.g. GNOME Remote Desktop sending unexpected PDU during capabilities exchange), the error detection now includes detailed comments explaining the upstream limitation (IronRDP connector 0.8.0 does not handle `ServerDeactivateAll` during `CapabilitiesExchange`); submitted fix upstream ([Devolutions/IronRDP#1253](https://github.com/Devolutions/IronRDP/issues/1253)); narrowed fallback detection patterns to avoid false positives on generic network errors (e.g. "unexpected end of stream" no longer triggers fallback)
- **Security: pre/post-connect tasks now use TaskExecutor from core** — pre-connect and post-disconnect automation tasks are now executed through `TaskExecutor` instead of raw `sh -c`; this adds: timeout enforcement (`timeout_ms` field is now respected — previously commands could hang indefinitely; timed-out processes are now killed instead of orphaned), environment sanitization (removes `BW_SESSION`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`, `AWS_ACCESS_KEY_ID`, `OP_SESSION`, `GITHUB_TOKEN`, `GH_TOKEN` from child process to prevent credential leakage), variable substitution (`${var}` references in task commands are now resolved from both global and connection-scoped variables — `${host}`, `${port}`, `${username}`, `${name}` are available automatically), and conditional execution (`only_first_in_folder` / `only_last_in_folder` conditions are now evaluated correctly via a shared `FolderConnectionTracker` in `AppState`)
- **UX: pre-connect task failure toast now shows error details** — instead of a generic "Pre-connect task failed. Connection aborted." message, the toast now includes the specific error (e.g. "Task timed out after 5000ms", "Task failed with exit code 1") via `i18n_f()` for proper localization
- **Code quality: ConnectionDialog decomposed** — extracted `create_rdp_options` (640 lines), `create_vnc_options` (310 lines), `create_spice_options` (290 lines), and `create_zerotrust_options` (480 lines) from the monolithic `dialog.rs` (8746 → 6968 lines, −20%) into their respective protocol modules (`rdp.rs`, `vnc.rs`, `spice.rs`, `zerotrust.rs`); replaced dead-code placeholder implementations with the actual production code; all protocol tab modules now follow the same pattern as the existing `ssh.rs`, `telnet.rs`, `serial.rs`, `kubernetes.rs`

### Removed
- **Dead code: `ConnectionFallback` module removed from rustconn-core** — the generic fallback chain (`ConnectionFallback`, `ConnectionStrategy`, `FallbackError`, `StrategyAttempt`) was never integrated into the GUI; the RDP fallback (IronRDP → wlfreerdp → xfreerdp) uses a purpose-built ad-hoc chain that is tightly coupled to GTK widget lifecycle; the generic module added complexity without benefit and can be restored from git history if needed
- **Dead code: `VirtualScrollConfig` removed from public API** — `VirtualScrollConfig` was re-exported from `rustconn-core` but never imported by any consumer (GUI or CLI); removed from `lib.rs` and `connection/mod.rs` re-exports, reduced visibility to `pub(crate)` for internal module tests only
- **UI: removed "Window Mode" section from Advanced tab** — the Display Mode dropdown (Embedded/External/Fullscreen) and Remember Position checkbox were shown for all protocols but only worked for RDP and VNC; for SSH, Telnet, SPICE, Serial, Kubernetes, and Mosh it was completely ignored; for RDP/VNC it duplicated the existing "Client Mode" setting in the Protocol tab; the `window_mode` field remains in the data model for backward compatibility but is no longer editable from the dialog ([#130](https://github.com/totoshko88/RustConn/issues/130))

### Dependencies
- `clap_complete` 4.6.3 → 4.6.4
- `hybrid-array` 0.4.11 → 0.4.12

## [0.13.10] - 2026-05-09

### Added
- **Import/Export: SecureCRT session support** — import connections from SecureCRT's `Config/Sessions/` directory (individual `.ini` files); export connections back to SecureCRT INI format as a directory tree; supports SSH2, Telnet, RDP, VNC protocols with hostname, port, username, SSH key path, X11/agent forwarding, compression; folder hierarchy preserved as connection groups; available in GUI export dialog, CLI, and programmatic API ([#140](https://github.com/totoshko88/RustConn/issues/140))

### Fixed
- **Backup/Restore: global variables lost after restore** — restoring settings from a ZIP archive would show "Restored N files" but closing the settings dialog afterwards overwrote the restored `config.toml` with stale in-memory state (which had empty `global_variables`); the dialog now reloads `AppSettings` from disk immediately after a successful restore so that the in-memory state matches the restored file ([#142](https://github.com/totoshko88/RustConn/issues/142))
- **SSH: ControlMaster sockets now actually closed on application exit** — the shutdown handler scanned `active_sessions()` to find SSH connections needing socket cleanup, but by the time GTK's `connect_shutdown` fires, all sessions are already in `Terminated` state (GTK destroys widgets first), so the list was always empty and no sockets were ever closed; replaced with a filesystem scan of the runtime directory for `rc-*` socket files, which works regardless of session state; stale sockets that don't respond to `ssh -O exit` are force-removed ([#125](https://github.com/totoshko88/RustConn/issues/125))
- **KeePass: custom entry path for variables ignored RustConn/ prefix** — when a user specified a custom "KeePass entry" path for a secret variable, the lookup still prepended `RustConn/` and tried fallback paths, making it impossible to reference entries at arbitrary locations in the database; added `get_password_from_kdbx_exact()` that queries the entry at the exact user-specified path without any prefix or fallback logic; default (no custom path) behaviour unchanged ([#143](https://github.com/totoshko88/RustConn/issues/143))
- **KeePass: "Variable Not Configured" dialog loop when database password not set** — the dialog appeared on every connection because: (1) `save_variable_to_vault` silently failed when KeePass couldn't unlock the database (`kdbx_password = None`), (2) the retry always re-read from KeePass which also failed, (3) the user's backend choice in the dialog dropdown was ignored; fixed by: respecting the user-selected backend from the dialog, showing an error toast when save fails instead of silently retrying, adding LibSecret fallback for both read and write when KeePass is unavailable and `enable_fallback` is enabled; the dialog now shows only the configured preferred backend + LibSecret as options ([#143](https://github.com/totoshko88/RustConn/issues/143))

### Dependencies
- `hashbrown` 0.17.0 → 0.17.1

## [0.13.9] - 2026-05-09

### Fixed
- **Flatpak: Zero Trust Generic commands now execute on host** — custom commands in the Generic Zero Trust provider were failing with "No command specified" because the double `sh -c` wrapping broke argument parsing for `flatpak-spawn`; now Generic commands in Flatpak are automatically wrapped with `flatpak-spawn --host -- script -qfc '...' /dev/null` (same PTY-allocating approach as Local Shell), so host-side binaries resolve correctly without manual `flatpak-spawn` prefixes; single quotes in command templates are properly escaped ([#132](https://github.com/totoshko88/RustConn/issues/132))
- **Split View: focus border not updating on click** — clicking a terminal pane in split view would send input to the correct terminal but the focus border (colored outline) remained on the previously focused pane; root cause: each call to `setup_all_panel_click_handlers` added a new `GestureClick` controller without removing the previous one, causing duplicate gesture controllers to compete for the click event; the first (stale) handler would claim the event before the current handler could update focus styling; fixed by removing existing primary-button gesture controllers before adding a new one
- **Zero Trust Generic: skip vault credential lookup** — connecting via Generic Command no longer triggers a ~14-second Bitwarden/KeePass vault lookup; the custom command handles its own authentication interactively in the terminal, so vault resolution is unnecessary and was only adding startup delay ([#132](https://github.com/totoshko88/RustConn/issues/132))

### Improved
- **RDP: real disk space reported to Windows via shared folders** — the RDPDR backend now queries actual filesystem statistics using `nix::sys::statvfs` instead of returning hardcoded values; Windows Explorer and applications connected via shared folders now see correct total/available disk space; values are normalized to 4096-byte allocation units matching the reported sector geometry; graceful fallback to defaults if the statvfs call fails

### Dependencies
- `cc` 1.2.61 → 1.2.62
- `filetime` 0.2.27 → 0.2.28
- `quick-xml` 0.39.3 → 0.39.4
- `tokio` 1.52.2 → 1.52.3
- Added `nix` 0.31.2 (feature: `fs`) to rustconn-core for safe statvfs access

## [0.13.8] - 2026-05-08

### Fixed
- **Per-connection monitoring toggle not saving state** — the "Enable Monitoring" switch in the connection dialog Advanced tab always appeared enabled and did not persist the user's choice; root cause: `update_connection()` in `ConfigManager` unconditionally overwrote `monitoring_config` with the old value from the existing connection (`updated.monitoring_config = existing.monitoring_config.clone()`), discarding the user's change from the dialog; additionally, the save logic stored `None` when the toggle was ON (meaning "use global default") instead of an explicit override, so per-connection enable didn't work when global monitoring was disabled; fixed both: removed the overwrite in `update_connection`, and save now stores explicit `Some(true)` / `Some(false)` so the per-connection toggle correctly overrides the global setting in both directions ([#125](https://github.com/totoshko88/RustConn/issues/125))
- **Flatpak: CLI tools not found by protocol detection** — `which_binary()` in protocol detection only checked `/app/bin` and system PATH; CLI tools installed via Flatpak Components (e.g. hoop, boundary, tailscale) in `~/.var/app/io.github.totoshko88.RustConn/cli/` were not discovered; now searches all `get_cli_path_dirs()` directories and passes the full resolved path to version detection
- **Hoop.dev CLI version not displayed** — `hoop version` outputs JSON (`{"version":"1.59.3",...}`) which the generic version parser did not recognize; added `parse_json_version()` to extract the version field from JSON output; also fixed `get_version()` to set extended PATH in Flatpak so the binary can be executed from its install directory

## [0.13.7] - 2026-05-08

### Improved
- **Flatpak CLI: automatic version resolution for 7 components** — Tailscale, kubectl, Teleport, Boundary, Hoop.dev, Bitwarden CLI, and 1Password CLI no longer use hardcoded version URLs; instead, the latest version is detected at install/update time from upstream APIs (Kubernetes stable.txt, GitHub releases, HashiCorp checkpoint, Tailscale packages page, Hoop latest.txt, AgileBits update API); this eliminates the need to manually bump versions in source code and ensures users always get the latest release; `scripts/check-cli-versions.sh` updated to verify endpoint reachability instead of comparing pinned versions

### Fixed
- **SSH: monitoring no longer triggers a second agent confirmation** — monitoring now waits up to 5 seconds for the main session's ControlMaster socket to appear, then connects as `ControlMaster=no` (slave only); previously monitoring used `ControlMaster=auto` which could race with the main session and create a separate master connection, causing a second Bitwarden/SSH agent prompt; falls back to creating its own master only if the socket never appears ([#125](https://github.com/totoshko88/RustConn/issues/125))
- **SSH: ControlMaster sockets cleaned up on application exit** — all SSH ControlMaster sockets are now gracefully closed (`ssh -O exit`) when the application shuts down; previously sockets lingered in the filesystem until `ControlPersist` timeout expired ([#125](https://github.com/totoshko88/RustConn/issues/125))
- **SSH: control socket path shortened for macOS compatibility** — the ControlPath format changed from `rustconn-ssh-{host}-{port}-%r` to `rc-{host}-{port}-%r`; on macOS, `/tmp` is used instead of `$TMPDIR` (which is ~52 chars under `/var/folders/...`) to stay within the 104-byte Unix socket path limit; long hostnames are truncated to 40 characters with `floor_char_boundary` for UTF-8 safety
- **Auto-reconnect: no longer loops infinitely on rapid crashes** — if a session crashes within 5 seconds of starting (e.g., SIGSEGV in VTE), auto-reconnect is skipped to prevent an infinite reconnect loop; previously a terminal crash would trigger immediate reconnect → crash → reconnect indefinitely at ~17ms intervals
- **Flatpak: local shell PTY resize now propagates to host** — when the VTE widget resizes, the new dimensions are forwarded to the host-side PTY via `flatpak-spawn --host -- stty rows R cols C` with 200ms debounce; previously the host shell retained its initial 24×80 size regardless of window resizing, causing incorrect line wrapping in bash, vim, htop, etc. ([#122](https://github.com/totoshko88/RustConn/issues/122))
- **Shutdown: socket cleanup limited to active sessions only** — previously attempted to close ControlMaster sockets for all saved SSH connections; now only closes sockets for sessions that were actually connected, using `futures::join_all` for parallel execution

## [0.13.6] - 2026-05-07

### Improved
- **Preferences: Monitoring moved to its own page** — monitoring settings (global enable, polling interval, visible metrics, activity monitor) are now on a dedicated "Monitoring" tab with the `utilities-system-monitor-symbolic` icon instead of being buried at the bottom of the Connection page; the subtitle on the global enable switch now clarifies that per-connection overrides are configured in the Advanced tab of the connection dialog ([#125](https://github.com/totoshko88/RustConn/issues/125))
- **CLI: machine-readable output for AI agents and scripting** — `show`, `test`, and `stats` commands now accept `--format json|csv|table`; `group show` also supports `--format`; all default to JSON when stdout is piped (non-TTY); `list --format json` output now includes `description`, `group_id`, `tags`, `last_connected` fields; `test --format json` returns structured results with `latency_ms`, `pass_rate`, and per-connection details; `stats --format json` returns all metrics as a single JSON object; `group show --format json` includes child groups and connections list ([#132](https://github.com/totoshko88/RustConn/issues/132))
- **i18n: activity monitor mode and export format names now translatable** — `MonitorMode::display_name()` values ("Off", "Activity", "Silence") and `ExportFormat::display_name()` values ("Ansible Inventory", "SSH Config", etc.) are now wrapped in `i18n()` at call sites so they appear translated in the UI; previously these were hardcoded English strings passed as arguments to `i18n_f()` without translation
- **Security: intermediate password strings now zeroized on drop** — cached RDP/VNC credentials extracted via `expose_secret().to_string()` are now wrapped in `zeroize::Zeroizing<String>` so the plaintext is overwritten in memory when the variable goes out of scope; previously these intermediate `String` copies remained in heap memory until the allocator reused the page
- **Reliability: host check no longer panics on runtime creation failure** — `tokio::runtime::Runtime::new().expect()` in WoL/reconnect polling replaced with proper `Result` propagation via a new `HostCheckError::Io` variant; if the tokio runtime cannot be created (e.g. file descriptor exhaustion), the error is reported gracefully instead of crashing

### Fixed
- **SSH: single authentication prompt for connection + monitoring** — the main VTE terminal SSH connection now always uses `ControlMaster=auto` with a shared `ControlPath`; the monitoring subsystem reuses the same socket instead of opening a separate SSH session, eliminating the second key/passphrase/agent prompt that previously appeared when monitoring started; if the user already enabled ControlMaster manually in connection settings, the shared ControlPath is still injected so monitoring can find the socket ([#125](https://github.com/totoshko88/RustConn/issues/125))
- **Repository hygiene: removed committed vim swap file and msgmerge backups** — `rustconn-core/src/.ssh_tunnel.rs.swp` removed from git tracking; `*.swp`, `*.swo`, `*.po~` patterns added to `.gitignore` to prevent future accidental commits

## [0.13.5] - 2026-05-07

### Added
- **Drag & Drop file paths into VTE terminals (SSH/Telnet)** — dragging files from a file manager onto a terminal session inserts their shell-escaped paths (single-quoted with `'\''` for embedded quotes), separated by spaces; matches GNOME Terminal behavior; visual highlight on the terminal when dragging over it; works with multiple files simultaneously ([#74](https://github.com/totoshko88/RustConn/issues/74))
- **Drag & Drop files to RDP clipboard (embedded IronRDP)** — dragging files onto an embedded RDP session announces them to the remote server via the CLIPRDR `FileGroupDescriptorW` channel (`CF_HDROP`); the server can then paste the files as if they were copied locally; includes a circuit breaker that auto-disables after 3 consecutive failures with an `adw::Toast` notification and "Try Again" button to re-enable; servers that don't support `STREAM_FILECLIP_ENABLED` are detected at capability negotiation and the feature is disabled gracefully ([#74](https://github.com/totoshko88/RustConn/issues/74))
- **RDP: "Reconnect on Resize" option** — new checkbox in the RDP connection dialog (Advanced → Features) that forces a full reconnect on window resize instead of using the Display Control Channel; useful for legacy RDP servers (Windows Server 2008/2012) that don't support MS-RDPEDISP, or when the server ignores dynamic resolution changes; disabled by default (dynamic resize is preferred)
- **Smart Folders: custom emoji icons** — smart folders now support a custom emoji icon displayed in the sidebar instead of the default 📁; set via the "Icon" field in the New/Edit Smart Folder dialog or `--icon` CLI flag (`rustconn-cli smart-folder create --icon "🚀"`); use "none" to clear in edit mode; the emoji is persisted in settings and shown in the sidebar folder row ([#133](https://github.com/totoshko88/RustConn/issues/133))

### Fixed
- **RDP: automatic reconnect on resize for servers without Display Control Channel** — when the server does not support MS-RDPEDISP (e.g. Windows Server 2008/2012/2016 without the Display Control extension), RustConn now automatically falls back to a full reconnect with the new resolution instead of leaving the session at the old resolution with distorted scaling; previously, if "Reconnect on Resize" was unchecked, the resize was silently ignored for such servers; now the dynamic resize path is always attempted first, and if the server reports Display Control unavailable, a reconnect is triggered regardless of the checkbox state; the "Reconnect on Resize" option now means "always reconnect immediately without trying dynamic resize first" — useful when you know the server doesn't support it and want to skip the initial attempt ([#131](https://github.com/totoshko88/RustConn/issues/131))
- **RDP: Copy/Paste toolbar buttons don't actually copy or paste** — the Copy button set the local GTK clipboard but didn't suppress the `clipboard-changed` handler, causing a feedback loop that re-announced the same text back to the server; the Paste button only updated the server's clipboard buffer via CLIPRDR `FormatList` but never simulated Ctrl+V, so text was never actually pasted into the active remote window; fixed: Copy now suppresses the sync handler for 100ms to prevent the feedback loop; Paste now sends `SendKeySequence` (Ctrl+V) after a 150ms delay to let the server process the clipboard data before pasting ([#126](https://github.com/totoshko88/RustConn/issues/126))
- **RDP: dynamic resize without reconnect via Display Control Channel** — window resize no longer triggers a full session disconnect/reconnect cycle; instead, the new resolution is sent in-place via the MS-RDPEDISP Display Control Virtual Channel (`SetDesktopSize` → `encode_resize`), which is the same mechanism used by mstsc, xfreerdp, and Remmina; the session continues seamlessly with the server-side desktop resized to match the new widget dimensions; debounce (500ms) and threshold (50px) remain to avoid flooding the server with resize requests; FreeRDP external mode still falls back to reconnect since it has no DVC access from the widget side ([#131](https://github.com/totoshko88/RustConn/issues/131))
- **SSH agent key selection not remembered in connection dialog** — when editing a saved connection that uses SSH agent authentication, the previously selected key was always reset to the first entry in the dropdown; root cause: `set_connection()` tried to restore the key selection before `refresh_agent_keys()` loaded the key list, then `refresh_agent_keys()` unconditionally reset the dropdown to index 0; now the selected fingerprint/comment is stored as a pending selection and restored after the agent keys are loaded ([#125](https://github.com/totoshko88/RustConn/issues/125))
- **SSH agent: multiple authentication prompts from monitoring** — the monitoring subsystem spawned a new SSH process every 3 seconds to collect metrics, each requiring a separate SSH agent confirmation (e.g., Bitwarden desktop app); now monitoring uses `ControlMaster=auto` with `ControlPersist=30` so all polling commands multiplex over a single authenticated SSH connection; the agent is only prompted once when monitoring starts ([#125](https://github.com/totoshko88/RustConn/issues/125), [#135](https://github.com/totoshko88/RustConn/issues/135))
- **Smart Folders: double-click and context menu not working for connections** — connections inside smart folders did not respond to double-click (connect) or right-click (context menu); root cause: the inner `ListBox` used `SelectionMode::None` which blocked `row_activated` signal, and the `connect-to` window action was never registered; fixed by switching to `SelectionMode::Single` with `activate_on_single_click(false)`, registering a new `win.connect-to` action that accepts a connection ID parameter, and adding a right-click context menu on connection rows with Connect, Edit, Copy Username/Password, Wake On LAN, Check if Online, and Delete actions
- **Toast error messages showing only "Error" without details** — the retry toast for port check failures (SSH, VNC, SPICE, Telnet, MOSH) displayed only a generic "Error" icon+label with no message text; root cause: `set_custom_title()` replaces the entire toast title widget, so the message passed to `Toast::new()` was never rendered; fixed by including the message in the custom title widget (format: "Error: <message>"); also replaced the generic "Connection failed. Host unreachable." text with the actual `PortCheckError` details (e.g. "Port 22 on '12.23.34.45' is not reachable: connection timed out")

## [0.13.4] - 2026-05-05

### Added
- **RDP Autotype: send text as keystrokes bypassing clipboard restrictions** — new "Type Clipboard" and "Type Text…" toolbar buttons in embedded RDP sessions; sends text character-by-character using `TS_UNICODE_KEYBOARD_EVENT` PDU which is keyboard-layout independent (works regardless of DE/US/other layout mismatches); solves scenarios where server-side paste is blocked by GPO, Citrix policy, UAC dialogs, or password fields that reject Ctrl+V; "Type Clipboard" reads the local clipboard and types it into the remote session; "Type Text…" opens a dialog where the user enters text (with optional password mode) that never touches the system clipboard — ideal for sensitive strings; per-connection configurable timing: inter-character delay (5–200ms, default 20ms) and initial delay before typing starts (0–5000ms, default 0ms); higher delays needed for Citrix gateways that drop characters when events arrive too fast; iterates by Unicode grapheme clusters so composed characters (é, ñ) are sent as single units; only available for embedded IronRDP mode (external FreeRDP runs in a separate process where keystroke injection is not possible) ([#127](https://github.com/totoshko88/RustConn/issues/127))
- **KeePass custom entry path for secret variables** — secret variables can now reference an existing entry in the KeePass database instead of the default `RustConn/rustconn/var/{name}` path; in the Variables dialog (Menu → Tools → Variables), when a variable is marked as Secret and the KeePass backend is active, a new "KeePass entry" field appears where you can specify the full path to an existing entry (e.g., `Internet/MyRouter` or `Network/Switches/RADIUS`); the password is read directly from that entry's Password attribute — no need to duplicate secrets under the RustConn hierarchy; when a custom path is set, RustConn does not attempt to create or overwrite the entry on save ([#114](https://github.com/totoshko88/RustConn/issues/114))

### Fixed
- **RDP toolbar Copy/Paste buttons do nothing on Wayland (COSMIC, GNOME)** — the clipboard buttons used `drawing_area.display().clipboard()` which on Wayland may not have clipboard ownership because the clipboard is tied to the focused surface; replaced with `root().native().display().clipboard()` which uses the top-level window surface — the reliable clipboard owner on all Wayland compositors; also fixed: Paste button silently swallowed errors when local clipboard was empty or unreadable (now shows status feedback: "Local clipboard is empty" / "Cannot read clipboard" / "Clipboard channel not ready"); CLIPRDR `client_capabilities` now advertises `USE_LONG_FORMAT_NAMES` flag required by Windows Server 2016+ for proper format list exchange — without it some servers never announce clipboard formats, making the Copy button permanently disabled; added tracing for all clipboard button operations to aid future debugging ([#126](https://github.com/totoshko88/RustConn/issues/126))

### Dependencies
- unicode-segmentation 1.13.2 (new)
- tower-http 0.6.8 → 0.6.9

## [0.13.3] - 2026-05-05

### Improved
- **GNOME HIG: application menu restructured** — "Fullscreen" (F11) added to the app menu; monolithic Tools section (11 items) split into three logical subsections: Managers (Snippets, Clusters, Templates, Variables), Monitoring & History (Sessions, History, Statistics, Recordings), Security & Network (Password Generator, Wake On LAN, SSH Tunnels); Settings separated from Shortcuts/About/Quit into its own section per HIG convention; "About" renamed to "About RustConn" per GNOME HIG naming pattern
- **GNOME HIG: Fullscreen menu item is now stateful** — the "Fullscreen" entry in the app menu uses a stateful `GAction` so GNOME Shell displays a checkmark (✓) when fullscreen is active; previously the menu item gave no visual indication of the current state
- **GNOME HIG: manager dialogs unified** — all dialog windows now use standard window close buttons (×) instead of text "Close"/"Cancel" buttons; action buttons moved to consistent positions: primary actions as icon buttons on the left of header bar (Add `+`, Refresh, Copy, Reset, Test, Connect, Import), save/submit actions on the right (Save, Create, Export, Import, Send, Rename); Snippets manager replaced bottom text buttons with inline icon buttons per row (▶ ✏ 🗑); Templates moved "Use Template" to bottom action bar; Variables added `+` and Save to bottom; History moved "Connect" to bottom right; Connection dialog uses Test icon (`network-transmit-receive-symbolic`) on the left; Quick Connect uses Connect icon on the left; Statistics uses Reset icon (`edit-clear-all-symbolic`); Password Generator uses Copy icon; Log Viewer and Terminal Search now use standard × close; applies to: Snippets, Clusters, Templates, Variables, Sessions, History, Statistics, Recordings, Quick Connect, Password Generator, Wake On LAN, Import, Export, Group Edit, New Group, Smart Folder, Connection Dialog, Rename, Log Viewer, Terminal Search
- **GNOME HIG: RDP Security Layer dropdown accessibility** — added `accessible::Property::Label` to the Security Layer dropdown in the RDP connection dialog for screen reader support
- **Tray menu i18n** — all system tray menu strings ("Show Window", "Hide Window", "Quick Connect...", "Recent Connections", "Local Shell", "Active Session(s)", "About RustConn", "Quit", tooltip) now use `gettext()` for translation; translations provided for all 16 languages; previously hardcoded in English

### Added
- **RDP Security Layer / TLS Compatibility options** — new "Security Layer" dropdown (Negotiate/RDP/TLS/NLA) and "TLS Security Level" spin (0–5) in the RDP connection dialog; enables connections to legacy servers (Windows Server 2012 / Windows 7) that require lower TLS levels or RDP Security Layer instead of NLA; when RDP or TLS security layer is selected (or TLS level < 2), IronRDP embedded mode automatically falls back to external FreeRDP which supports legacy protocols; TLS level row is shown only when RDP or TLS mode is selected; CLI `show` command displays non-default security settings ([#124](https://github.com/totoshko88/RustConn/issues/124))

### Fixed
- **SSH agent: multiple authentication prompts for saved connections** — when a saved connection uses `SshKeySource::Agent` and the key comment contains a file path, `build_command_args()` previously added `-i <path> -o IdentitiesOnly=yes`; this caused SSH to first attempt file-based auth (triggering an agent confirmation in Bitwarden), then fall back to agent auth (triggering a second confirmation); fixed by removing `-i` and `IdentitiesOnly` for Agent auth entirely — the agent now offers keys naturally with a single prompt, matching Quick Connect behavior ([#125](https://github.com/totoshko88/RustConn/issues/125))
- **AdwTabOverview "exceeds AdwApplicationWindow size" warning in Flatpak** — embedded RDP and VNC sessions caused `AdwTabOverview` to request more space than the window provides (e.g. 1540×865 vs 1200×800 available); the RDP `DrawingArea` had no size constraint and the VNC `DrawingArea` set `content_width(1280)`/`content_height(720)` which GTK reported as natural size; fixed by setting `content_width`/`content_height` to 0 on both drawing areas (they expand via `hexpand`/`vexpand` instead) and setting `overflow: hidden` on the `TabOverview` widget
- **False "KeePassXc backend unavailable" toast when KeePassXc is running** — `check_secret_backend_available` checked `SecretManager.is_available()` for all non-LibSecret backends, but `build_from_settings` registers `LibSecretBackend` (not `KeePassXcBackend`) for KeePassXc/KdbxFile because KDBX credentials are resolved via direct file access in `resolve_credentials_blocking`; the availability probe therefore tested whether `secret-tool` could be spawned within a 5-second `block_on` timeout — which can fail in Flatpak sandboxes or when D-Bus is slow at startup — and incorrectly reported KeePassXc as unavailable; now KeePassXc/KdbxFile availability is determined by checking `kdbx_enabled && kdbx_path.exists()` instead of probing the unrelated LibSecretBackend ([#123](https://github.com/totoshko88/RustConn/issues/123))
- **Flatpak Local Shell: "no job control" warnings and broken PTY** — `flatpak-spawn --host` only forwards stdio without creating a host-side PTY, so the shell cannot become a session leader — causing `tcgetpgrp failed`, `setpgid: Inappropriate ioctl for device` warnings and broken job control (Ctrl-Z, fg, bg); now wraps the host shell in `script -qfc` (util-linux) which allocates a real PTY on the host, giving bash/zsh/fish a proper controlling terminal ([#122](https://github.com/totoshko88/RustConn/issues/122))

### Dependencies
- h2 0.4.13 → 0.4.14
- kqueue-sys 1.0.4 → 1.1.0
- openssl 0.10.78 → 0.10.79, openssl-sys 0.9.114 → 0.9.115
- quick-xml 0.39.2 → 0.39.3
- redox_syscall 0.7.4 → 0.7.5
- serdect 0.4.2 → 0.4.3
- tokio 1.52.1 → 1.52.2

## [0.13.2] - 2026-05-04

### Fixed
- **Mouse scroll not working in terminal sessions** — `set_enable_fallback_scrolling(false)` was incorrectly tied to the "Mouse passthrough" setting, which disabled VTE's scrollback scrolling for normal shell sessions (bash, zsh); VTE natively forwards scroll events to programs that request mouse tracking regardless of this flag — disabling it only broke scrollback for programs without mouse tracking; now always enabled ([#121](https://github.com/totoshko88/RustConn/issues/121))
- **Flatpak local shell provides only a sandboxed minimal shell** — "Local Shell" in Flatpak now spawns the user's host shell via `flatpak-spawn --host` with `--login`, giving full access to system tools, dotfiles, and home directory; added `--talk-name=org.freedesktop.Flatpak` permission to both Flatpak manifests ([#122](https://github.com/totoshko88/RustConn/issues/122))

### Removed
- **"Mouse passthrough" setting removed from Terminal preferences** — the toggle was based on a misunderstanding of VTE's `enable_fallback_scrolling` API; VTE handles mouse event forwarding to terminal apps (mc, vim, htop) automatically via escape sequences — no user-facing setting is needed; removed field from `TerminalSettings`, checkbox from UI, and obsoleted translations in all 16 languages

### Added
- **Per-connection monitoring toggle in connection dialog** — Advanced tab now has a "Remote Monitoring" section with an "Enable Monitoring" switch; when disabled, the monitoring collector does not open a separate SSH session to the remote host, preventing IP bans on devices with concurrent session limits (e.g. network routers); uses the existing `MonitoringConfig` backend — the toggle was already supported in `rustconn-core` but had no GUI ([#106](https://github.com/totoshko88/RustConn/issues/106))

## [0.13.1] - 2026-05-04

### Fixed
- **Crash when typing in sidebar search field** — `SearchEngine::find_case_insensitive` used raw byte-position iteration (`0..=len`) and direct byte slicing (`haystack[i..i+n]`) which panics when connection names or hosts contain multi-byte UTF-8 characters (Cyrillic, CJK, emoji, etc.); the same byte-boundary bug existed in `fuzzy_score_case_insensitive` and `fuzzy_score_optimized` prefix checks; fixed by iterating only over valid `char_indices()` boundaries and using `str::get()` for safe slicing with `is_char_boundary()` guards ([#116](https://github.com/totoshko88/RustConn/issues/116))
- **Export/re-import loses folder hierarchy, icons, SSH settings, and smart folders** — native `.rcn` export/import had four data-loss bugs: (1) group hierarchy flattened because binary root-vs-child sort didn't guarantee parents were created before children — replaced with topological sort; (2) folder icons (emoji), description, SSH auth settings (`ssh_auth_method`, `ssh_key_path`, `ssh_proxy_jump`, `ssh_agent_socket`), login defaults (`username`, `domain`, `password_source`), automation (`expect_rules`, `post_login_scripts`), and `dynamic_folder` config were silently dropped because `create_group_with_parent` only sets name+parent — now copies all fields via `update_group` after creation; (3) smart folders were not included in the export format at all — added `smart_folders` field to `NativeExport` (format version 3, backward-compatible via `#[serde(default)]`); (4) smart folder `filter_group_id` references were not remapped to new group UUIDs — now remapped during import; CLI export/import updated accordingly ([#118](https://github.com/totoshko88/RustConn/issues/118))
- **Settings dialog loses Passbolt, 1Password, Pass, and KeePassXC keyring settings on save** — the close handler used dummy widgets for 9 secret backend fields (passbolt passphrase/URL/save-to-keyring, 1password token/save-to-keyring, pass store dir, kdbx save-to-keyring); replaced with clones of real widgets so all secret backend settings are now correctly persisted
- **Statistics Reset button had no confirmation dialog** — destructive action now requires confirmation via `adw::AlertDialog`
- **Cluster and Template delete had no confirmation dialog** — both now show `adw::AlertDialog` before deletion
- **Template protocol filter dropdown was not connected** — filter now correctly filters templates by SSH/RDP/VNC/SPICE

### Improved
- **GNOME HIG compliance audit across all menu-accessible dialogs** — comprehensive review and fixes: empty states with `adw::StatusPage` (History, Statistics, Sessions, Recordings); accessible labels on status icons (History); search/filter in Recordings dialog; auto-refresh after recording import; confirmation dialogs for all destructive actions; "Add Variable" button style corrected (`suggested-action` → `flat`); Template header button renamed to "Use Template" for clarity; duplicate close buttons removed (Snippets manager); xgettext marker functions for indirect `i18n()` strings (dialog buttons, keyboard shortcuts); WoL dialog auto-sized to content; Export dialog height increased to fit all fields; Sessions window enlarged for StatusPage; copy-to-clipboard toast in Password Generator

### Documentation
- **Remmina Flatpak import troubleshooting** — added instructions to User Guide for importing Remmina connections when both apps are Flatpaks; covers `flatpak override`, Flatseal, and symlink workarounds ([#120](https://github.com/totoshko88/RustConn/issues/120))

### Added
- **Automation section in group edit dialog** — Expect Rules and Post-login Scripts can now be configured directly in the group edit dialog (Edit Group → Automation); full rule editor with pattern/response fields, priority/timeout controls, enabled/one-shot checkboxes, ↑↓🗑️ action buttons per rule, "Insert Variable" (➕) popover for `${password}`, `${username}`, `${host}`, `${port}`, `\n` insertion, "From Template" menu with 5 built-in presets marked with protocol hints (e.g., "Sudo Password (SSH)"), "Clear All" button, collapsible Pattern Tester for real-time rule matching, post-login scripts as individual entries with per-row delete, confirmation dialog on automation disable, stable window width via `set_size_request` + `tightening_threshold` matching Clamp, overlay scrolling to prevent layout shifts; CLI: `group edit --add-expect-rule` (JSON), `--clear-expect-rules`, `--add-post-login-script`, `--clear-post-login-scripts`; `group show` now displays automation config ([#117](https://github.com/totoshko88/RustConn/issues/117))

### Improved
- **Connection dialog Automation tab unified with group dialog** — expect rule editor now uses the same vertical layout as the group dialog: ↑↓🗑️ action buttons at top-right of each rule, "Insert Variable" (➕) popover on Response field, template picker with "(SSH)" protocol hints, variable substitution info banner, removed inner ScrolledWindow (scroll-in-scroll anti-pattern per GNOME HIG), overlay scrolling and `tightening_threshold` matching Clamp to prevent layout width shifts ([#117](https://github.com/totoshko88/RustConn/issues/117))

## [0.13.0] - 2026-05-03

### Fixed
- **Mouse not working in Midnight Commander and ncurses apps** — split view panel click handlers used `EventSequenceState::None` in capture phase, which prevented VTE from receiving raw button-press events for mouse tracking; changed to `EventSequenceState::Denied` so VTE processes mouse clicks for ncurses apps (mc, vim, htop); also unified TERM to `xterm-256color` everywhere (Flatpak previously used `rustconn-256color` which broke mouse tracking)
- **SFTP connection slow with Bitwarden backend** — double-clicking an SFTP connection triggered full Bitwarden credential resolution (~12s of sequential `bw status` + `bw list items` calls) even though mc uses SSH key in agent, not vault passwords; connections with `password_source` set to None or Prompt now skip async vault resolution entirely, benefiting all protocols — not just SFTP
- **External RDP tab shows only toolbar, content area empty** — `EmbeddedSessionTab::new()` determined embed mode from `DisplayServer::supports_embedding()` which returns `true` on both Wayland and X11; when `start_external_rdp_session` created the tab, it got an empty `DrawingArea` (meant for XEmbed) instead of the `StatusPage` with FreeRDP hotkeys; added `force_external` parameter so external sessions always render the informational StatusPage; also added `show_tab_view_content()` call for layout consistency and `markup_escape_text` to protect against special characters in connection names
- **Group edit: SSH settings toggle reopens confirmation dialog in a loop** — clicking "Clear" in the "Clear SSH Settings?" confirmation dialog triggered the `enable_expansion_notify` signal recursively, causing the dialog to reopen immediately; added a guard flag to prevent re-entry
- **Incomplete translations for Dynamic Folder strings** — "Dynamic Folder", "Generate connections from a script", "Working Directory", "Timeout (seconds)", "Shell command executed via sh -c", "Refreshed" toast, and error messages were marked fuzzy in all 16 `.po` files and displayed in English; provided correct translations and removed fuzzy flags for all languages
- **Bulk actions: "Move to Group" icon missing** — `folder-move-symbolic` is not available in all icon themes; replaced with standard `folder-drag-accept-symbolic` from Adwaita
- **External RDP (FreeRDP) fails on changed server certificate** — external xfreerdp always used `/cert:tofu` with no way to override; when a server certificate changed, the connection was silently rejected; added `ignore_certificate` field to `RdpConfig` and "Accept Certificate" checkbox in the RDP connection dialog; when enabled, removes stored FreeRDP certificate (`~/.config/freerdp/server/` and `known_hosts2`) and passes `/cert:ignore`; also fixed: password not passed to `sdl-freerdp3` (now uses `/p:`), `disable_nla` not forwarded to external client (now passes `/sec:nla:off`), connection errors not shown to user (now displays toast with parsed FreeRDP error), tab remained open after FreeRDP window closed (now auto-closes via process monitor), tab placeholder showed keyboard shortcuts and auto-close hint ([#112](https://github.com/totoshko88/RustConn/issues/112))

### Added
- **Smart Folders in sidebar** — the Smart Folders section is now visible in the sidebar below the connection list; dynamically groups connections based on tag, protocol, host pattern, and group filters using AND logic; shows match count badge per folder; "Add" button and right-click context menu (Edit / Delete) fully wired; "New Smart Folder" option added to empty-space context menu; smart folders auto-refresh on every sidebar reload; hidden by default, toggled via toolbar button with persisted visibility state ([#111](https://github.com/totoshko88/RustConn/issues/111))
- **Group-level Expect rules & post-login scripts inheritance** — `ConnectionGroup` now supports `expect_rules` and `post_login_scripts` fields; connections with empty automation config automatically inherit from their parent group chain (same walk-up-hierarchy pattern as SSH/credential inheritance with cycle detection); applies to all terminal protocols: SSH, Telnet, Serial, Kubernetes, MOSH, Zero Trust; set expect rules once on a group and all 600+ connections inherit them automatically ([#110](https://github.com/totoshko88/RustConn/issues/110))
- **Mouse passthrough option for VTE terminal** — new "Mouse passthrough" toggle in Settings → Terminal → Behavior; when enabled (default), mouse clicks and scroll wheel events are forwarded to terminal applications that request mouse tracking (Midnight Commander buttons, vim visual selection, htop scrolling); disables VTE's fallback scrolling so scroll wheel is sent as mouse events on alternate screen; hold Shift to select text when mouse passthrough is active
- **CLI: `--keep-alive-interval`, `--keep-alive-count`, `--ssh-verbose`, `--ignore-certificate` flags** — `add` and `update` commands now support SSH keep-alive settings (`ServerAliveInterval`, `ServerAliveCountMax`), SSH verbose/debug output (`-v` flag), and RDP certificate acceptance; previously these settings were only configurable via the GUI connection dialog
- **CLI: `snippet edit`, `template edit`, `cluster edit`, `smart-folder edit` subcommands** — all four resource types now have full CRUD via CLI; `snippet edit` supports `--new-name`, `--command`, `--description`, `--category`, `--tags`; `template edit` supports `--new-name`, `--host`, `--port`, `--user`, `--description`; `cluster edit` supports `--new-name`, `--broadcast`; `smart-folder edit` supports `--new-name`, `--protocol`, `--host-pattern`, `--tags` (use `"none"` to clear a filter); previously editing required delete+create

### Documentation
- **Auto-login with stored passwords** — added comprehensive troubleshooting section to User Guide explaining how to configure auto-login: Password Source must be "Vault", lookup key formats for each backend (KeePass hierarchical, libsecret flat, Bitwarden/1Password prefixed), common issues table with fixes ([#114](https://github.com/totoshko88/RustConn/issues/114))

### Improved
- **Vault entry missing toast notification** — when credential resolution finds no vault entry for a connection, a warning toast now shows "Vault entry not found for '{name}'" instead of silently falling back to a password prompt; helps users understand why auto-login didn't work ([#114](https://github.com/totoshko88/RustConn/issues/114))
- **"Test credential resolution" button in connection dialog** — new ✓ button next to the password field performs a vault lookup using the current connection name, host, protocol, and group; shows the exact lookup key used and whether the vault returned a password; helps users verify their vault configuration before connecting ([#114](https://github.com/totoshko88/RustConn/issues/114))
- **Multi-language SSH password prompt detection** — VTE password injection now recognizes password prompts from SSH servers configured in non-English locales; supported: German (Passwort/Kennwort), French (mot de passe), Spanish (contraseña), Portuguese (senha), Ukrainian/Belarusian (пароль), Polish (hasło), Czech/Slovak (heslo), Dutch (wachtwoord), Swedish (lösenord), Danish (adgangskode), Chinese (密码/密碼), Japanese (パスワード), Korean (비밀번호); previously only English prompts triggered auto-fill ([#114](https://github.com/totoshko88/RustConn/issues/114))
- **GNOME HIG: sidebar toolbar decluttered** — removed Import/Export buttons from the bottom toolbar (accessible via hamburger menu Ctrl+I and context menu); reduces button count from 8 to 6 for cleaner appearance per GNOME HIG recommendations

### Dependencies
- Boundary CLI 0.21.2 → 0.21.3 (security: CVE fixes in pgx/v5 and go-ntlmssp)
- Hoop.dev CLI 1.59.3 → 1.62.0
- Tailscale CLI 1.96.5 → 1.96.4 (1.96.5 was platform-specific, not available for Linux)
- rpassword 7.5.1 → 7.5.2
- zvariant 5.10.1 → 5.11.0

## [0.12.9] - 2026-05-02

### Fixed
- **Export group exports entire tree instead of selected subtree** — when exporting a specific group via the Export dialog's group filter, all groups were included in the output file even though connections were correctly filtered; now both connections and groups are filtered to the selected group and its descendants; previously importing such an export recreated the entire group hierarchy instead of just the selected branch

### Added
- **Snippet variable substitution from Global Variables** — snippets containing `${VARIABLE}` placeholders now automatically resolve values from Global Variables (Menu → Tools → Variables) before execution; if all variables are resolved, the snippet executes immediately without showing the input dialog; partially resolved variables pre-fill the dialog with known values; resolution order: Global Variables → snippet-defined defaults → manual input; uses the same `VariableManager` and vault-backed secret resolution as Expect rules and SSH connections
- **Dynamic Folders** — new `DynamicFolderConfig` on `ConnectionGroup` allows generating connections from an external script; the script runs via `sh -c` with configurable timeout (default 30s) and optional working directory; output is a JSON array of `[{name, host, port?, protocol?, username?, group?, tags?, description?}]`; connections are read-only (`is_dynamic` flag) with stable deterministic UUIDs across refreshes; supports SSH, RDP, VNC, SPICE, Telnet, and MOSH protocols; async executor in `rustconn-core/src/dynamic_folder.rs` with entry validation, warnings for empty name/host, and `thiserror`-based error types; model in `rustconn-core/src/models/dynamic_folder.rs`; **GUI**: group edit dialog with ExpanderRow for script/timeout/working directory/refresh interval configuration; context menu "Refresh Dynamic Folder" action with async execution and toast notifications; **CLI**: `dynamic-folder list`, `dynamic-folder show`, `dynamic-folder refresh` subcommands with table/JSON/CSV output

### Improved
- **CLI `group edit` extended** — `group edit` now supports `--new-name`, `--parent` (use "none" for root), `--description`, and `--icon` in addition to the existing SSH inheritance fields; enables full group management from the CLI without GUI
- **CLI `dynamic-folder set/remove`** — full CRUD for dynamic folders via CLI: `set` creates or updates the script configuration on any group, `remove` clears the configuration and deletes generated connections

## [0.12.8] - 2026-05-01

### Added
- **Generic async cache `Cached<T>`** — new `rustconn-core/src/cache.rs` module providing a thread-safe, TTL-based cache with automatic refresh via the `LoadCacheObject` trait; uses double-checked locking with `tokio::sync::RwLock` for concurrent read access; supports incremental updates through `previous_value` parameter, explicit invalidation, and configurable TTL (default 60s); replaces ad-hoc caching patterns across the codebase 
- **Busy-state indicator `BusyStack`** — new `rustconn-core/src/busy.rs` module providing a thread-safe RAII counter for tracking in-flight operations; callback fires on 0→1 (busy) and 1→0 (idle) transitions; nested operations handled correctly without extra callbacks; `Clone` for sharing across components; **integrated into GUI** — header bar spinner appears during connection attempts via `glib::MainContext` channel bridge 
- **Extended `ProtocolCapabilities`** — added 9 new capability flags: `multi_monitor`, `usb_redirection`, `port_forwarding`, `wayland_forwarding`, `x11_forwarding`, `session_recording`, `remote_monitoring`, `command_snippets`, `wake_on_lan`; enables UI to adapt controls per-protocol 
  - SSH: `port_forwarding`, `wayland_forwarding`, `x11_forwarding`, `session_recording`, `remote_monitoring`, `command_snippets`
  - RDP: `multi_monitor`
  - SPICE: `multi_monitor`, `usb_redirection`, `audio`
  - All terminal protocols: `session_recording`, `remote_monitoring`, `command_snippets`, `wake_on_lan`
  - All graphical protocols: `wake_on_lan`
- **Connection fallback chain `ConnectionFallback<T>`** — new `rustconn-core/src/connection/fallback.rs` module providing a generic mechanism for trying multiple connection strategies in priority order; `ConnectionStrategy` trait with `name()`, `is_available()`, and async `connect()`; unavailable strategies are skipped automatically; `FallbackError` collects all attempt details for diagnostics; integrated with `tracing` for structured logging 
- **Virt-viewer `.vv` file open support** — RustConn can now open `.vv` files (SPICE/VNC from libvirt, Proxmox VE, oVirt) directly from the file manager or command line (`rustconn file.vv`); adds `StartupAction::VvFile`, `VirtViewerImporter::parse_vv_file()` convenience method, `application/x-virt-viewer` MIME type registration in desktop file and metainfo, and MIME XML definition; completes parity with `.rdp` file support 
- **Connection failure toast** — when a connection fails to start, an error toast now shows the connection name (`"Connection to 'name' failed"`); previously the sidebar turned red with no further feedback

### Dependencies
- Teleport CLI 18.7.5 → 18.7.6 (security: authorization bypass in encrypted session recordings, cross-node recording access, SSRF via AWS database endpoint)

## [0.12.7] - 2026-05-01

### Fixed
- **Group credentials: Variable source shows password field instead of variable selector** — when editing a group and choosing "Variables" as the credential type, the dialog incorrectly displayed a password entry field; now shows a dropdown populated with secret global variables, matching the behavior of individual connection editing ([#109](https://github.com/totoshko88/RustConn/issues/109))
- **Group credentials: saving Variable source stored empty string** — selecting "Variable" and saving the group produced `PasswordSource::Variable("")` instead of the actual variable name; now correctly reads the selected variable from the dropdown
- **Group credentials: no validation for empty variable selection** — saving with "Variable" source when no secret global variables exist silently stored an empty variable name; now shows a validation error prompting the user to select a variable

### Improved
- **GNOME HIG: accessible labels for group credential widgets** — added `LabelledBy` relations for the password source dropdown, password entry, and variable dropdown in the group edit dialog so screen readers can announce their purpose
- **GNOME HIG: menu button tooltip shows F10 shortcut** — the hamburger menu button tooltip now reads "Menu (F10)", consistent with other header bar buttons that show their keyboard shortcut
- **GNOME HIG: "SSH Tunnels" moved to Tools section** — SSH Tunnels was in the App section alongside Settings/About/Quit; moved to the Tools section where it logically belongs with other management dialogs
- **GNOME HIG: "Settings" menu item ellipsis** — "Settings" now shows as "Settings..." to indicate it opens a window, per GNOME HIG ellipsis convention
- **GNOME HIG: "Keyboard Shortcuts" added to app menu** — the existing `app.shortcuts` action (F1) was not discoverable from the hamburger menu; added "Keyboard Shortcuts..." entry in the App section before About, matching standard GNOME app layout

### Documentation
- **User Guide: group credentials rewritten** — replaced outdated KeePass/Keyring/Bitwarden password source list with the current unified model (Prompt, Vault, Variable, Inherit, None); documented that Variable source shows a dropdown of secret global variables
- **User Guide: F10 shortcut** — added F10 (Open Menu) to the Application keyboard shortcuts table

## [0.12.6] - 2026-04-30

### Fixed
- **Expect script variables not substituted** — `${MY_PASSWORD}` and other `${VAR}` references in Expect rule responses were sent as literal text instead of being resolved to their actual values; now uses `VariableManager` to substitute global variables before creating automation triggers ([#105](https://github.com/totoshko88/RustConn/issues/105))
- **GNOME HIG: missing ellipsis on menu items** — "Active Sessions" and "SSH Tunnels" in the hamburger menu now use ellipsis ("Active Sessions...", "SSH Tunnels...") to indicate they open a dialog/window, per GNOME HIG conventions
- **False `c-format` flag on command palette search string** — `xgettext` incorrectly marked `"Search connections, > commands, @ tags, # groups, % tabs"` as a C format string; `% T` was interpreted as a format specifier, causing `msgfmt --check` to fail in 14 languages; removed the flag from POT and all 16 PO files
- **Corrupted `Plural-Forms` in uk.po** — `%` characters in the Ukrainian plural formula were replaced with `{}` placeholders during a previous i18n update; restored the correct `nplurals=3` formula

### Added
- **SSH verbose mode for connection debugging** — new "Verbose" toggle in SSH connection settings adds `-v` flag to the SSH command, showing detailed debug output in the terminal to help diagnose connection issues such as resets by remote devices ([#106](https://github.com/totoshko88/RustConn/issues/106))
- **Sidebar width setting** — new "Sidebar width" control in Settings → Appearance allows adjusting the sidebar width from 260 to 500 pixels; applied immediately and persisted across sessions ([#96](https://github.com/totoshko88/RustConn/issues/96))
- **SSH Tunnel Manager** — standalone window for managing headless SSH port-forwarding tunnels without terminal sessions; supports Local/Remote/Dynamic forwards, auto-start on launch, auto-reconnect, and references existing SSH connections for host/key configuration; accessible via menu or Ctrl+T ([#96](https://github.com/totoshko88/RustConn/issues/96))

### Improved
- **GNOME HIG: tunnel delete confirmation** — deleting a tunnel from the SSH Tunnel Manager now shows an `AdwAlertDialog` confirmation ("Delete Tunnel? — Tunnel "…" will be permanently removed.") with a destructive "Delete" button; previously the tunnel was removed immediately without confirmation
- **GNOME HIG: tunnel dialog inline validation** — the Save button in the Add/Edit Tunnel dialog is disabled while the tunnel name is empty, preventing accidental saves of unnamed tunnels; previously the dialog silently refused to save without any visual feedback

### Documentation
- **User Guide: SSH Tunnel Manager** — complete section with create/manage workflow, tunnel options table, use case examples, and comparison with per-connection port forwarding
- **User Guide: SSH verbose mode** — new section under SSH → Session Options with configuration steps and when-to-use guidance
- **User Guide: Expect variable substitution** — documented `${VARIABLE_NAME}` placeholder support in Expect rule responses with multi-step login example
- **User Guide: Sidebar width** — added to Settings → Appearance documentation
- **User Guide: context menu corrections** — removed non-existent "View Details" entry, fixed "Pin to Favorites"→"Pin / Unpin" to match code, added undocumented "Run Snippet..." and "Start/Stop Recording" entries, clarified Copy/Paste Connection scope (sidebar focus + hamburger menu only)
- **User Guide: Ctrl+T shortcut** — added SSH Tunnel Manager shortcut to Keyboard Shortcuts table

### Translations
- All 16 languages (be, cs, da, de, es, fr, it, kk, nl, pl, pt, sk, sv, uk, uz, zh-cn) aligned to 1748 translatable strings with 0 untranslated entries
- **Chinese Simplified (zh-cn)** — merged community translation update from GaaChun ([PR #108](https://github.com/totoshko88/RustConn/pull/108)); filled 36 new strings from 0.12.6; 20 fuzzy entries remaining (upstream review pending)

## [0.12.5] - 2026-04-29

### Fixed
- **Settings dialog overflows after Hoop.dev install** — `hoop version` outputs JSON (`{"version":"1.59.3","git_commit":"...","build_date":"..."}`) which was displayed as-is in the version suffix label, stretching the `AdwToolbarView` to 1331 px and breaking the 700 px settings dialog layout; added a dedicated `hoop` parser that extracts only the `"version"` field
- **kubectl version not shown in settings** — `kubectl version --client --short` fails on kubectl ≥ 1.28 (`error: unknown flag: --short`); switched to `kubectl version --client` and parse `Client Version: vX.Y.Z`
- **Tray icon SIGSEGV on restart** — `connect_shutdown` did not drop the `TrayManager`, so D-Bus callbacks could reference already-finalized GObjects during GTK teardown; now explicitly drops the tray manager in the shutdown handler before flushing persistence
- **Teleport CLI download URL 404** — pinned version 18.7.6 did not exist on the CDN; corrected to 18.7.5

### Dependencies
- Hoop.dev CLI 1.56.1 → 1.59.3
- Teleport CLI 18.7.6 → 18.7.5 (URL fix)

## [0.12.4] - 2026-04-29

### Fixed
- **"Copy Password" from context menu resolves from vault** — previously only worked with cached credentials (required connecting first); now falls back to `resolve_credentials_gtk` to fetch the password directly from the configured secret backend (KeePass, Bitwarden, 1Password, etc.) when no cached credentials are available

### Cleaned
- **Removed dead `mosh.rs` dialog module** — standalone MOSH options panel was never wired into the connection dialog; MOSH settings are already integrated into the SSH tab via `ssh::create_ssh_options()`
- **Removed legacy `connect_password_load_button` wrapper** — unused passthrough method in `ConnectionDialog` that delegated to `connect_password_load_button_with_groups` with empty groups; all callers already use the `_with_groups` variant directly

### Added
- **Import button in Cloud Sync settings** — "Available in Cloud" section now shows an "Import" button on each unimported `.rcn` file; clicking it creates an Import group and triggers an immediate sync, importing all connections from the file

### Dependencies
- rpassword 7.4.0 → 7.5.0
- rustls 0.23.39 → 0.23.40

## [0.12.3] - 2026-04-28

### Fixed
- **Sync toast shows raw placeholders instead of values** — the Cloud Sync notification displayed `%1`, `%2`, `%3`, `%4` instead of actual connection counts because `i18n_f()` only supports `{}` placeholders; changed both sync message strings and all 16 translations to use `{}` format

### Accessibility
- **Icon-only buttons missing accessible labels** — added `update_property(accessible::Property::Label)` with `i18n()` wrappers to 24 icon-only buttons across 15 files (password generator, terminal search, history, cluster management, split view, SSH agent, settings tabs, flatpak components); screen readers now correctly announce button purpose instead of just the icon name

### Dependencies
- Teleport CLI 18.7.4 → 18.7.6
- clap_complete 4.6.2 → 4.6.3
- gio 0.22.5 → 0.22.6, glib 0.22.5 → 0.22.6, glib-macros 0.22.2 → 0.22.6
- gtk4 0.11.2 → 0.11.3
- pango 0.22.4 → 0.22.6
- zbus 5.14.0 → 5.15.0, zvariant 5.10.0 → 5.10.1

## [0.12.2] - 2026-04-26

### Fixed
- **Flatpak SFTP ssh-add fails with missing askpass** — `ssh-add` inherited the host's `SSH_ASKPASS` (e.g. `ksshaskpass` on KDE) which doesn't exist inside the Flatpak sandbox, causing "No such file or directory" and blocking mc/file-manager SFTP; now strips `SSH_ASKPASS` from the environment for all bare `ssh-add` calls ([#102](https://github.com/totoshko88/RustConn/issues/102))
- **Blocking operations on GTK main thread** — `has_secret_backend()` and `refresh_secret_backend_cache()` called `block_on(is_available())` without timeout on the main thread, freezing the UI if the secret backend was unresponsive; added 5-second timeouts to both methods
- **Missing timeouts on blocking async operations** — `flush_persistence()` (app shutdown), `resolve_with_hierarchy()` (credential fallback), `auto_unlock()` (Bitwarden), and all vault store/retrieve/delete operations in `dispatch_vault_op()` could hang indefinitely; added timeouts (5s for persistence, 30s for credential resolution and Bitwarden unlock, 10s for vault operations) to prevent infinite blocking

### Translations
- All 16 languages (be, cs, da, de, es, fr, it, kk, nl, pl, pt, sk, sv, uk, uz, zh-cn) aligned to 1697 translatable strings
- **Italian (it)** — fixed syntax error in PO file (curly quotes breaking msgfmt)
- **Chinese Simplified (zh-cn)** — 20 fuzzy entries remaining (community-contributed, pending upstream review)

### Dependencies
- FreeRDP 3.24.1 → 3.25.0 (switched to pub.freerdp.com release tarballs)

## [0.12.1] - 2026-04-25

### Fixed
- **Split view content disappearing on panel focus switch** — clicking between split panels caused the content to vanish because the click handler called `switch_to_tab()` which navigated the TabView away from the split-owner's tab (where the split widget lives) to the clicked session's placeholder tab; removed the `switch_to_tab()` call — focus is now handled entirely via `set_focused_pane()` and `grab_focus()` ([#101](https://github.com/totoshko88/RustConn/issues/101))
- **Flatpak SFTP mc host key prompt on every connect** — mc FISH uses SSH internally but could not find the Flatpak-writable `known_hosts` file because `~/.ssh` is read-only in the sandbox; now creates a thin SSH wrapper script that injects `StrictHostKeyChecking=accept-new` and the correct `UserKnownHostsFile`, prepended to `$PATH` for the mc process ([#102](https://github.com/totoshko88/RustConn/issues/102))
- **cargo-deny CI failure** — removed deprecated `unlicensed` and `copyleft` keys from `deny.toml` `[licenses]` section (removed in cargo-deny v2, see [PR #611](https://github.com/EmbarkStudios/cargo-deny/pull/611))
- **cargo-audit CI failure** — added `RUSTSEC-2023-0071` (rsa Marvin Attack) to `[advisories].ignore` in `deny.toml`; transitive dependency via ironrdp/sspi and spice-client with no upstream fix available

### Dependencies
- Bitwarden CLI 2026.3.0 → 2026.4.1 (fixes supply chain attack in 2026.4.0)
- kubectl 1.35.4 → 1.36.0

## [0.12.0] - 2026-04-24

### Added
- **Cloud Sync** — synchronize connection configurations between devices and team members through any shared cloud directory (Google Drive, Syncthing, Nextcloud, Dropbox, USB)
  - **Group Sync** — per-group `.rcn` files with Master/Import access model and name-based merge
  - **Simple Sync** — single-file bidirectional sync with UUID-based merge and tombstone deletion tracking
  - **SSH Key Inheritance** — group-level SSH settings (key path, auth method, proxy jump, agent socket) inherited by child connections; `ssh_key_path` remains local-only per device
  - **Credential Resolution UX** — interactive `AdwAlertDialog` prompts when variables or secret backends are missing at connect time
  - **File Watcher** — automatic import on `.rcn` file changes via `notify` crate with 3s debounce
  - **Cloud Sync Settings page** — `AdwPreferencesPage` with sync directory, device name, synced groups, available files, and Simple Sync toggle
  - **Sidebar sync indicators** — `emblem-synchronizing-symbolic` for synced groups, `dialog-warning-symbolic` for errors
  - **Import group UI restrictions** — synced fields read-only, local fields editable, context menu restrictions
  - **CLI sync commands** — `sync status`, `sync list`, `sync export`, `sync import`, `sync now`
- **Accessible labels** — added `update_property` accessible labels to icon-only buttons (password visibility toggle, password load, RDP quick actions)
- **cargo-deny + cargo-audit in CI** — security advisory checks, license allow-list, ban wildcards, source registry restrictions
- **Document dirty badge** — CSS dot indicator replaces text `"• "` prefix for unsaved documents in sidebar
- **Tab Overview** — grid view of all open tabs (GNOME Web-style) via button on the tab bar or **Ctrl+Shift+O**; makes navigating 10+ tabs significantly easier ([#100](https://github.com/totoshko88/RustConn/issues/100))
- **Tab Switcher in Command Palette** — `%` prefix in Command Palette (or **Ctrl+%**) opens fuzzy search across all open tabs; shows protocol and tab group in results ([#100](https://github.com/totoshko88/RustConn/issues/100))
- **Tab Pinning** — right-click a tab → Pin Tab to keep it always visible at the left edge of the tab bar; pinned tabs don't scroll away ([#100](https://github.com/totoshko88/RustConn/issues/100))
- **Custom terminal themes** — create, edit, and delete custom color themes (background, foreground, cursor, full 16-color ANSI palette) from Settings → Terminal → Colors; custom themes are persisted to `~/.config/rustconn/custom_themes.json` and appear alongside built-in themes in the dropdown ([#98](https://github.com/totoshko88/RustConn/issues/98))
- **Group Jump Host dropdown** — group SSH settings now include a Jump Host dropdown (select from existing SSH connections) in addition to the manual ProxyJump text field; stored as `ssh_jump_host_id` with inheritance support via `resolve_ssh_jump_host_id()`

### Improved
- **Tab Overview + Split View architecture** — complete refactoring of the TabView/SplitView architecture so that split layouts live inside TabPages instead of a global container; Tab Overview now renders correct thumbnails for all tabs including split-view tabs without SIGSEGV crashes or blank previews
- **Split view "Select Tab" popover** — the session picker popover in empty split panels now shows color indicators for sessions already displayed in other split views
- **Split view placeholder** — when a session is moved to another tab's split layout, its own tab shows a "Displayed in Split View" status page with a "Go to Split View" button for quick navigation
- **Split color indicators preserved** — switching between tabs no longer clears the colored dot indicators on split-view tabs
- **Group settings: GNOME HIG enable switches** — Default Credentials and SSH Settings sections now use `AdwExpanderRow` with `show_enable_switch(true)`; when disabled, all fields are cleared to `None`, giving clear semantics of "not configured" vs "configured but empty"
- **SSH tunnel password authentication** — SSH tunnels (used by RDP, VNC, SPICE jump host connections) now support password-authenticated jump hosts via `SSH_ASKPASS` mechanism; previously `BatchMode=yes` was unconditional, silently blocking password auth
- **VTE passphrase prompt guard** — VTE password auto-fill now explicitly rejects SSH key passphrase prompts (`"Enter passphrase for key"`) to prevent sending the wrong secret when SSH auth method is PublicKey
- **Connection dialog: protocol-aware Password Source** — Password Source dropdown is now hidden for protocols that don't use stored passwords (Telnet, Serial, MOSH, Kubernetes, Zero Trust); previously visible but non-functional for these protocols
- **Credential Resolution UX fully wired** — `CredentialResolutionResult` enum now drives the connection flow: `VariableMissing` shows the variable setup `AdwAlertDialog` (enter value + select backend → Save & Connect), `BackendNotConfigured` shows the backend missing dialog (Enter Manually / Open Settings), `VaultEntryMissing` falls through to the protocol's password prompt; previously the resolver silently returned `None` on all failure paths
- **Sidebar sync error indicators** — synced groups now show `dialog-warning-symbolic` with error tooltip when the last sync operation failed (e.g. parse error, missing file); previously always showed the generic synced icon regardless of error state
- **Custom themes atomic write** — `custom_themes.json` now uses temp file + rename (atomic write) with `0600` permissions and `tracing::warn` on errors; consistent with sync file write pattern

### Dependencies
- notify 7 (new — file watching for Cloud Sync)
- hostname 0.4 (new — default device name)
- slug 0.1 (new — sync filename generation)
- Tailscale CLI 1.96.4 → 1.96.5
- cc 1.2.60 → 1.2.61, data-encoding 2.10.0 → 2.11.0, hybrid-array 0.4.10 → 0.4.11, libc 0.2.185 → 0.2.186, rustls-pki-types 1.14.0 → 1.14.1

### Fixed
- **System tray SIGSEGV and empty menu** — tray icon menu could randomly appear empty or crash the application with `object_ref: assertion '!object_already_finalized' failed` (SIGSEGV) on startup; root cause was `ksni::Handle::update()` calling `block_on()` on the GTK main thread which deadlocked with the D-Bus service loop competing for the `TrayState` mutex, and conflicted with the application's tokio runtime guard; moved all D-Bus updates to a dedicated `tray-updater` background thread with coalescing `sync_channel(1)`, moved `TrayManager` creation to a `tray-init` background thread, added re-activation guard in `build_ui`, and ensured polling timers stop when the window is finalized
- **Tab Overview SIGSEGV with split-view tabs** — opening Tab Overview when split-view tabs were active caused Pango `size >= 0` assertion failures and crashes because `AdwTabOverview` attempted to snapshot `TabPage` children with 0×0 allocation; refactored to keep `TabView` always visible with per-tab `TabPageContainer` wrappers that guarantee non-zero allocation
- **Tab Overview blank previews** — split-view tabs showed empty thumbnails in Tab Overview because terminals were reparented to a global split container outside `TabView`; terminals now stay inside `TabPage` children at all times
- **Terminal theme reset when Settings dialog is closed** — closing the Settings dialog applied the global terminal color theme to all terminals, overwriting per-connection theme overrides (custom background/foreground/cursor colors); now re-applies connection-specific theme overrides after global settings are applied ([#99](https://github.com/totoshko88/RustConn/issues/99))
- **Pango assertion failure on zero font size** — guarded against `font_size == 0` in terminal configuration and settings collection to prevent `pango_font_description_set_size: assertion 'size >= 0' failed` crashes when the settings dialog returns an invalid value
- **Highlight rules show color instead of hover-only underline** — VTE's `match_add_regex()` only underlines text on mouse hover without color; added a Cairo `DrawingArea` overlay that reads visible terminal text, runs `CompiledHighlightRules::find_matches()` per line, and draws colored background rectangles and foreground underlines in real time; `SourcePattern` now carries `foreground_color`/`background_color` from the rule ([#97](https://github.com/totoshko88/RustConn/issues/97))

## [0.11.7] - 2026-04-23

### Fixed
- **Monitoring bar broken after scrollbar addition** — the terminal scrollbar (added in 0.11.6) changed the session container from vertical to horizontal layout, causing the monitoring bar to appear side-by-side with the terminal instead of below it; wrapped the horizontal terminal+scrollbar row in a vertical outer container so the monitoring bar is correctly appended underneath
- **Monitoring collector keeps running in split view** — when a session entered split view the monitoring bar was removed but the SSH exec collector continued polling the remote host every 3 seconds; added `suspend_monitoring`/`resume_monitoring` to `MonitoringCoordinator` that stops the collector on split entry and restarts it (with stored connection params) when the session returns to tab view

### Documentation
- **User Guide restructured** — reorganized USER_GUIDE.md from 41 flat sections (~4000 lines) into 13 logically grouped sections (~2000 lines); protocols, sessions, organization, and productivity tools are now grouped by topic instead of scattered across the document
- **CLI Reference extracted** — moved the full CLI command reference (~700 lines) to a dedicated [CLI_REFERENCE.md](docs/CLI_REFERENCE.md) for easier navigation
- **Zero Trust Providers extracted** — moved all Zero Trust provider documentation (~220 lines) to a dedicated [ZERO_TRUST.md](docs/ZERO_TRUST.md)
- **FAQ and Troubleshooting merged** — combined the previously separate FAQ, Troubleshooting, and Migration Guide sections to reduce duplication

### Dependencies
- clap_mangen 0.2.33 → 0.3.0

## [0.11.6] - 2026-04-23

### Added
- **Terminal scrollbar** — VTE terminals now display a vertical scrollbar (using a standalone `GtkScrollbar` connected to VTE's `vadjustment`, the same approach as GNOME Terminal); scrollbar is shown by default and can be toggled in Settings → Terminal → Scrolling ([#95](https://github.com/totoshko88/RustConn/issues/95))
- **"Execute Snippet…" in terminal context menu** — right-clicking inside a terminal now shows an "Execute Snippet…" option that opens the snippet picker; follows GNOME HIG (no nested submenus, verb label with ellipsis) ([#95](https://github.com/totoshko88/RustConn/issues/95))

### Fixed
- **Sidebar status stays gray after reconnect** — clicking "Reconnect" on a disconnected SSH/VTE session now immediately sets the sidebar status to "connecting" (yellow) instead of leaving it gray; the status then transitions to "connected" (green) once the session is established ([#96](https://github.com/totoshko88/RustConn/issues/96))
- **Context menu intermittently fails to open on right-click** — reverted sidebar popover from `autohide(true)` back to `autohide(false)` because GTK4's pointer grab consumed right-click events before the gesture handler could fire; added manual Escape key handler and window `focus-widget` tracking to auto-dismiss the menu when a dialog opens ([#87](https://github.com/totoshko88/RustConn/issues/87))

### Dependencies
- pastey 0.2.1 → 0.2.2
- rustls 0.23.38 → 0.23.39

## [0.11.5] - 2026-04-22

### Added
- **Simplified Chinese (zh-cn) translation** — complete translation of all 1573 UI strings; contributed by GaaChun ([PR #94](https://github.com/totoshko88/RustConn/pull/94))
- **User Guide: libvirt NSS hostname resolution** — added troubleshooting section explaining how to resolve VM hostnames via the libvirt NSS module when connecting with RDP/VNC from Flatpak or native installs ([#91](https://github.com/totoshko88/RustConn/issues/91))

### Dependencies
- picky-asn1-der 0.5.5 → 0.5.6
- rustls-webpki 0.103.12 → 0.103.13
- winnow 1.0.1 → 1.0.2
- kubectl 1.35.3 → 1.35.4

## [0.11.4] - 2026-04-21

### Fixed
- **Sidebar flashes red during SSH connection** — connecting via SSH (and other protocols with port check) briefly showed "failed" (red) status before switching to "connected" (green); introduced `ConnectionStartResult` enum to distinguish async port check in progress (`Pending`) from real failures (`Failed`); the sidebar now stays yellow ("connecting") until the port check completes
- **Context menu stays open when dialog opens** — the sidebar context menu remained visible when opening a dialog via keyboard shortcut or toolbar button (e.g. "New Connection"); switched the popover from `autohide(false)` to `autohide(true)` so GTK4 automatically dismisses it when focus moves elsewhere ([#93](https://github.com/totoshko88/RustConn/issues/93))
- **Sidebar stays "connecting" after cancelling password dialog** — closing the VNC or RDP password prompt without entering credentials left the sidebar status stuck on yellow ("connecting"); the status is now cleared on cancel
- **VNC/RDP with "None" password source prompts immediately** — when Password Source is set to "None", the first connection attempt now uses an empty password; the password dialog is only shown on retry (second attempt) if authentication fails
- **Cannot save SSH connection with default key** — validation incorrectly required an explicit SSH key path even when Key Source was set to "Default"; the check now only applies when Key Source is "File"

### Dependencies
- Teleport CLI 18.7.3 → 18.7.4
- 1Password CLI 2.33.1 → 2.34.0

## [0.11.3] - 2026-04-21

### Added
- **CLI: `--jump-host` flag for `add` and `update`** — set a jump host (SSH bastion) when creating or updating SSH, SFTP, RDP, VNC, and SPICE connections via CLI; accepts connection name or UUID; validates that the referenced connection exists and prevents self-referencing
- **SSH Jump Host for VNC and SPICE** — VNC and SPICE connections now support SSH jump host tunnelling via `ssh -L` local port forwarding; the tunnel process is managed automatically and killed on tab close; port check is skipped when jump host is configured
- **SSH tunnel stderr capture** — SSH tunnel process stderr is now read in a background thread and logged via `tracing::warn`; diagnostic messages (auth failures, port unreachable) are available via `SshTunnel::stderr()` and logged on process exit
- **SSH tunnel health monitoring** — `SshTunnel::is_alive()` checks whether the SSH process is still running; `wait_for_tunnel_ready()` now detects early process exit and fails fast with a descriptive error instead of polling until timeout
- **CLI: `show` displays Jump Host** — `rustconn-cli show` now prints the resolved jump host name for SSH, SFTP, RDP, VNC, and SPICE connections

### Fixed
- **RDP via jump host stuck at "connecting"** — embedded IronRDP connections through an SSH tunnel could hang indefinitely when the remote host was unreachable (firewall DROP); the handshake timeout for tunnel connections is now capped at 15 seconds (down from 60s) and produces a clear error message ([#92](https://github.com/totoshko88/RustConn/issues/92))
- **Flatpak: kubectl and Hoop.dev missing from settings and PATH** — kubectl and Hoop.dev CLI were not shown in the Settings → Clients detection tab and their install directories were missing from the Flatpak PATH extension; added "Container Orchestration" section to settings, added Hoop.dev to "Zero Trust Clients", and registered both directories in `get_cli_path_dirs()` and `find_in_flatpak_cli_dir()`
- **Sidebar status not set on connection start** — "connecting" (yellow) status is now shown immediately on double-click, before credential resolution or tunnel creation begins; previously the status only appeared after the tunnel was established
- **Sidebar status not cleared on RDP error** — non-protocol errors (timeout, unreachable host) now fire the `on_state_changed(Error)` callback, which closes the tab and sets "failed" (red) status; previously the sidebar stayed yellow after a timeout
- **Sidebar "failed" status overridden by Disconnected** — the `Disconnected` handler no longer calls `decrement_session_count` for sessions that were never connected; this prevents the "failed" status set by the Error handler from being cleared back to empty
- **RefCell panic on RDP error** — `handle_ironrdp_error` now uses take-invoke-restore pattern for `on_state_changed` and `on_error` callbacks; the previous `borrow()` approach caused a re-entrancy panic when the callback triggered `close_tab` → `adw_tab_view_close_page` → `Disconnected` state change
- **RDP error toast** — a toast notification ("RDP connection failed. Check that the remote host is reachable.") is now shown when an embedded RDP connection fails before ever connecting

### Improved
- **RDP handshake phase logging** — debug log messages now mark each handshake phase (X.224 negotiation, TLS upgrade, NLA/capabilities) so the exact hang point is visible in logs
- **TCP_NODELAY for tunnel connections** — Nagle's algorithm is disabled on the TCP stream to the tunnel, reducing latency for the RDP handshake
- **Tunnel timeout error message** — tunnel connections show "Connection failed: RDP handshake timed out after 15s — the remote host may be unreachable through the SSH tunnel or the RDP service is not running" instead of generic "Operation timed out"

## [0.11.2] - 2026-04-20

### Fixed
- **Reconnect reuses existing tab for all VTE protocols** — clicking "Reconnect" on a disconnected session now respawns the process in the same terminal tab instead of closing and creating a new one; works for SSH, Telnet, Serial, Kubernetes, ZeroTrust (all providers), and MOSH; tab position, tab group, and split view state are fully preserved ([#89](https://github.com/totoshko88/RustConn/issues/89))
- **RDP port check skipped with jump host** — pre-connect TCP port check is now skipped for RDP connections that have a jump host configured; the destination is only reachable through the SSH tunnel, so direct probing always timed out
- **Hoop.dev CLI download** — `releases.hoop.dev` removed the `latest` URL alias (HTTP 403); switched to versioned URL format; pinned to 1.56.1
- **Azure/gcloud/OCI CLI wrapper test in Flatpak** — `az --version` verification after pip install crashed with `Read-only file system`; now sets Flatpak-writable config dirs during wrapper script test
- **Flatpak SFTP always uses mc** — SFTP in Flatpak now always opens via Midnight Commander; `xdg-open sftp://` is unreachable from the sandbox

### Improved
- **Reconnect banner consistent across all protocols** — RDP, VNC, and SPICE sessions now show the "Session disconnected / Reconnect" banner at the bottom of the tab (same position as SSH/Telnet) instead of a button in the top-right toolbar
- **Sidebar width tuned for HiDPI** — default sidebar width lowered from 360px to 320px and fraction from 30% to 27%; saved widths from older versions are reset on upgrade; fixes overly wide sidebar on 4K displays with 200% scaling while keeping all protocol filter icons visible

### Added
- **SSH Jump Host for RDP** — SSH jump host selector is now available for RDP connections; the session is tunnelled through the selected SSH bastion host via `ssh -L` local port forwarding; tunnel process is managed automatically and killed on tab close ([#90](https://github.com/totoshko88/RustConn/issues/90))
- **Tab context menu: Close Others / Left / Right / All / Ungrouped** — right-click a tab for browser-style close actions: close all other tabs, close tabs to the left or right, close all ungrouped tabs, or close all tabs
- **CLI: all protocols and Zero Trust providers** — `rustconn-cli add` now supports all 10 protocols (`ssh`, `rdp`, `vnc`, `spice`, `sftp`, `telnet`, `serial`, `mosh`, `k8s`, `zt`) and all 11 Zero Trust providers with provider-specific flags (`--aws-region`, `--gcp-zone`, `--resource-group`, `--boundary-target`, etc.)

### Documentation
- **Complete CLI reference in User Guide** — comprehensive documentation for all 23 CLI commands with syntax, options tables, examples for every protocol and Zero Trust provider, shell completions, Flatpak usage with alias, and scripting examples

### Dependencies
- open 5.3.3 → 5.3.4
- openssl 0.10.77 → 0.10.78
- openssl-sys 0.9.113 → 0.9.114
- typenum 1.19.0 → 1.20.0
- Hoop.dev CLI pinned to 1.56.1

## [0.11.1] - 2026-04-18

### Fixed
- **Reconnect preserves tab position** — clicking "Reconnect" on a disconnected session now opens the new tab at the same position in the tab bar instead of appending it to the end; fixes workflow disruption when managing 10+ SSH sessions ([#89](https://github.com/totoshko88/RustConn/issues/89))
- **Context menu handoff between items** — right-clicking a second sidebar item while a context menu is already open now correctly closes the first menu and opens the new one; previously the second menu failed to appear due to GTK4 popover lifecycle conflicts ([#87](https://github.com/totoshko88/RustConn/issues/87))
- **Stale highlight on right-click** — right-clicking multiple sidebar items in succession no longer leaves residual selection highlights on previously clicked rows; the context menu gesture now claims the event sequence to prevent GTK4 from applying sticky `:active` / `:focus-within` pseudo-classes to row widgets
- **Context menu requires single right-click** — switching the context menu between sidebar items now works with a single right-click instead of requiring two clicks (first to dismiss, second to open); achieved by disabling `autohide` on the popover and managing dismissal explicitly via gesture handlers

### Improved
- **Context menu layout follows GNOME HIG** — sidebar context menu items reordered to match GNOME Files conventions: primary action (Connect) at top, organisation (Rename / Duplicate / Move) next, utilities (Copy credentials, SFTP, WOL) in the middle, creation and properties (New Connection, Edit) before the destructive action (Delete) at the bottom
- **MSRV bumped to 1.95** — required by `constant_time_eq` 0.4.3 (transitive dependency via `zip`)

### Improved
- **`SshOptionsWidgets` tuple replaced with named struct** — the 24-element tuple type alias in `ssh.rs` is now a proper struct with named fields; adding new SSH options is a single-point change instead of updating ~6 destructuring sites across `dialog.rs`
- **Split view context menu shares popover lifecycle with sidebar** — split view panel right-click menu now uses the same `ACTIVE_POPOVER` tracking as the sidebar; right-clicking panel B while panel A's menu is open correctly closes the first menu; also fixes cross-component conflicts where a sidebar menu and split view menu could fight for the GTK4 popover grab; menu labels now wrapped in `i18n()` for localization
- **Auto-reconnect guard for closed tabs** — polling callback now checks if the session still exists in `sessions_map` before triggering reconnect; prevents creating an orphan tab if the user manually closes the tab while background polling is active
- **SSH config importer applies `Host *` defaults** — `Host *` entries in `~/.ssh/config` are now parsed as global defaults and merged into each host entry (host-specific values take priority); previously `Host *` was skipped entirely, losing settings like `ServerAliveInterval 60` that apply to all hosts

### Added
- **SSH Keep-Alive settings** — dedicated `Keep-Alive Interval` and `Keep-Alive Count` spin rows in the SSH Connection options group; generates `-o ServerAliveInterval=N` and `-o ServerAliveCountMax=M` flags to prevent idle disconnects caused by firewalls or server timeouts; new connections default to 60s interval / 3 retries; custom_options take precedence if the same key is set manually ([#88](https://github.com/totoshko88/RustConn/issues/88))
- **SSH Config import/export for Keep-Alive** — `ServerAliveInterval` and `ServerAliveCountMax` from `~/.ssh/config` are now mapped to dedicated fields instead of only `custom_options`; exporter outputs them as separate directives with deduplication

## [0.11.0] - 2026-04-18

### Added
- **General tab migrated to adw:: widgets** — connection dialog General tab rebuilt with `adw::PreferencesGroup`, `adw::EntryRow`, `adw::SpinRow`, `adw::ComboRow`, and `adw::PasswordEntryRow`; replaces manual Grid+Label+Entry layout with native GNOME HIG sections (Identity, Connection, Authentication, Organization); 30-element tuple replaced with `BasicTabWidgets` struct; content wrapped in `adw::Clamp` (max 600px) for consistent width; Entry suffix widgets constrained with `width_chars`/`max_width_chars`
- **Legacy XOR encryption migration warning** — credentials still using XOR obfuscation are transparently migrated to AES-256-GCM on load; a toast notification shows the count of migrated credentials; XOR support will be removed in v0.12
- **State access helpers** — `with_state()`, `try_with_state()`, `with_state_mut()`, `try_with_state_mut()` helper functions reduce RefCell borrow panics; documented in ARCHITECTURE.md
- **Runtime warning for `block_on_async`** — logs `tracing::warn` when GTK main thread is blocked for >100ms, suggesting `spawn_async` instead
- **Accessible label for Command Palette list** — screen readers now announce the results list as "Search results"
- **Desktop entry translations** — added `Comment[lang]` translations for uk, de, fr, es, cs

### Improved
- **RDP connection state structured** — `handle_ironrdp_error()` 13-parameter signature replaced with `RdpConnectionContext` struct
- **Automation task validation hardened** — import warnings for connections with automation/expect rules; sensitive env vars (`BW_SESSION`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`) cleared before task execution
- **Localized constants and port descriptions** — `(Root)`, `(None)`, `(No keys loaded)`, and port range labels (`Well-Known`, `Registered`, `Dynamic`) now wrapped in `i18n()` for translation
- **Sidebar GNOME HIG color consistency** — sidebar pane and tab bar backgrounds unified with `@headerbar_bg_color` for GNOME Files-like appearance; bottom toolbar buttons use `.flat` style; separator between search and list hidden for seamless look; works correctly in both light and dark themes
- **KeePass button visibility** — active vault button now uses normal icon color instead of `.suggested-action` (which rendered white-on-white in light theme); inactive state uses `.dim-label`
- **Focus border only in split view** — `.focused-panel` accent border is now hidden when only one panel exists; previously showed a distracting border around the welcome screen and single-tab sessions

### Fixed
- **Split view tab colors preserved across Settings** — opening the Settings dialog no longer resets colored indicators on split view tabs; the root cause was that `apply_protocol_color()` / `clear_protocol_color()` guards relied on an unpopulated `session_tab_ids` map, so they always overwrote split indicators when `set_color_tabs_by_protocol()` was called on dialog close
- **Group Operations mode no longer breaks sidebar layout** — replaced text buttons with compact icon-only pill buttons matching the protocol filter bar style; toolbar wrapped in animated `Revealer` (SlideDown 200ms) instead of abrupt `set_visible()`; delete button uses `@error_color` for visual distinction
- **Split view context menu Copy/Paste/Select All now works** — action group `terminal.*` was installed on the TabView container which is lost when the terminal is reparented into a split panel; moved to the VTE terminal widget itself so actions follow the widget through reparenting

### Security
- **Automation env var sanitization** — task executor removes sensitive environment variables before spawning shell commands
- **Lazy Bitwarden credential decryption** — Bitwarden master password and API credentials are now decrypted at startup only when Bitwarden is the preferred backend; previously they were unconditionally decrypted into memory even when KeePass or other backends were active

### Dependencies
- libbz2-rs-sys 0.2.2 → 0.2.3
- rand 0.8.5 → 0.8.6
- rtoolbox 0.0.4 → 0.0.5

## [0.10.22] - 2026-04-17

### Fixed
- **Terminal context menu Copy/Paste now works** — replaced custom `GestureClick` popover with VTE's native `set_context_menu_model()` API; the old approach broke clipboard actions because the popover stole focus from VTE before callbacks could run ([#84](https://github.com/totoshko88/RustConn/issues/84))
- **No more `gdk_clipboard_write_async` assertion** — Copy action now caches selected text via `text_selected()` before VTE clears the selection on right-click, preventing the `mime_type != NULL` GDK critical warning
- **Blank menus on X11 (MATE, XFCE)** — GTK4's NGL renderer causes popovers to render blank until hovered on some X11 compositors; RustConn now auto-detects X11 sessions and falls back to the Cairo renderer via process re-exec ([#85](https://github.com/totoshko88/RustConn/issues/85))

### Improved
- **Context menu labels localized** — Copy, Paste, Select All strings now wrapped in `i18n()` for translation

### Dependencies
- pxfm 0.1.28 → 0.1.29
- tokio 1.52.0 → 1.52.1
- uuid 1.23.0 → 1.23.1

## [0.10.21] - 2026-04-16

### Security
- **Machine key encryption hardened** — removed predictable `hostname+username` fallback from `get_machine_key()`; `/etc/machine-id` fallback now uses HKDF-SHA256 with app-specific salt; `.machine-key` file created with `0600` permissions

### Fixed
- **Groups expand/collapse on double-click** — double-clicking anywhere on a group row now toggles expand/collapse, not just the expander icon ([#83](https://github.com/totoshko88/RustConn/issues/83))
- **Ctrl+K no longer hijacks terminal** — removed `Ctrl+K` from the global search shortcut; only `Ctrl+F` focuses the search box now, so `Ctrl+K` passes through to terminal applications like nano ([#83](https://github.com/totoshko88/RustConn/issues/83))
- **Right-click context menu on all SSH profiles** — set gesture propagation phase to `Capture` so the right-click handler fires before `TreeExpander` internal handlers that could swallow the event ([#83](https://github.com/totoshko88/RustConn/issues/83))
- **Filter bar opens below search box** — swapped layout order so protocol filters appear below the search entry instead of above it, preventing UI jump ([#83](https://github.com/totoshko88/RustConn/issues/83))

### Improved
- **Sidebar accessible labels localized** — wrapped `"Search connections"`, `"Search syntax help"`, `"Connection list"`, and `"Filter by {protocol} protocol"` in `i18n()` / `i18n_f()` for screen reader localization

### Dependencies
- aws-lc-rs 1.16.2 → 1.16.3
- aws-lc-sys 0.39.1 → 0.40.0
- clap 4.6.0 → 4.6.1

## [0.10.20] - 2026-04-15

### Fixed
- **RDP shared folders only used first folder path** — RDPDR backend now maps each drive to its own base path via `device_id`, so multiple shared folders work correctly in embedded IronRDP mode ([#82](https://github.com/totoshko88/RustConn/issues/82))
- **Tailscale CLI download broken by macOS-only release** — pinned version 1.96.5 only existed for macOS; downgraded to 1.96.4 (latest Linux build) and switched from static checksum to `SkipLatest` policy to prevent future platform-specific release breakage ([#81](https://github.com/totoshko88/RustConn/issues/81))
- **SSH Port Forwarding section missing from connection dialog** — the Port Forwarding group was silently not added because fragile widget tree navigation (`first_child → downcast → child → ...`) failed; now uses the content box directly from `create_ssh_options()` return value ([#80](https://github.com/totoshko88/RustConn/issues/80))

### Docs
- **Flatpak shared folders troubleshooting** — added "RDP Shared Folders in Flatpak" section to User Guide with `flatpak override` commands for granting filesystem access ([#82](https://github.com/totoshko88/RustConn/issues/82))

## [0.10.19] - 2026-04-15

### Added
- **Shell button in header bar** — moved the Local Shell button from the sidebar filter bar to the main header bar as a prominent accent-colored pill button with icon and label; always visible even when sidebar is hidden ([#76](https://github.com/totoshko88/RustConn/issues/76))
- **Optional protocol filter bar** — protocol filters can now be toggled on/off via a button in the search bar or in Settings → Interface → "Show protocol filters"; state is persisted across sessions; hidden by default for a cleaner interface ([#76](https://github.com/totoshko88/RustConn/issues/76))
- **Toggle protocol filters action** — `win.toggle-protocol-filters` window action with sidebar toggle button that persists visibility state to config
- **Tab group chooser dialog** — "Set Group..." dialog now shows existing groups as clickable pill buttons for quick selection, with a text field for creating new groups; no  manual retyping of group names
- **Close All in Group** — new context menu action on grouped tabs; shows a confirmation dialog with tab count and group name, then closes all tabs belonging to that group
- **Group name in tab tooltip** — hovering over a grouped tab now shows `[GroupName]` in the tooltip, visible even when split view colors are active
- **Group name as tab title prefix** — tab groups now display as a `[GroupName]` prefix in the tab title instead of a colored indicator icon; this separates group identity from split view / protocol color indicators, so both are visible simultaneously

### Fixed
- **Terminal not auto-focused after connection** — newly opened SSH session tabs now automatically grab keyboard focus so the user can type immediately; uses idle callback with selected-page guard to prevent focus-stealing when multiple tabs open concurrently ([#79](https://github.com/totoshko88/RustConn/issues/79))
- **SIGSEGV on rapid right-click on tab** — triple right-clicking a terminal tab caused a segfault because each click created a new popover without unparenting the previous one; now tracks the active popover and tears it down before creating a new one
- **Tray menu labels empty when "Minimize to tray" enabled** — the ksni tray `menu()` callback runs on a D-Bus worker thread where `gettext` is not initialised, causing `i18n()` to return empty strings; tray menu now uses plain English labels to avoid the thread-safety issue; window visibility is synced via periodic polling so the Show/Hide toggle stays correct
- **Tab group color conflict with split view** — tab groups and split view previously competed for the same `indicator_icon` slot; groups now use a title prefix while split view keeps the colored indicator, eliminating the conflict

### Improved
- **Wider sidebar** — increased minimum sidebar width from 160px to 360px for better readability of nested items and long hostnames; increased OverlaySplitView max from 280px to 360px default with up to 600px maximum
- **Filter bar cleanup on hide** — active protocol filters are automatically cleared when the filter bar is hidden to prevent invisible filtering confusion

### Dependencies
- bitflags 2.11.0 → 2.11.1
- clap_complete 4.6.1 → 4.6.2
- FreeRDP 3.24.0 → 3.24.1 (security fixes)
- hyper-rustls 0.27.8 → 0.27.9
- rand 0.9.3 → 0.9.4
- rayon 1.11.0 → 1.12.0
- rustls-webpki 0.103.11 → 0.103.12
- tokio 1.51.1 → 1.52.0
- VTE 0.80.0 → 0.80.3

## [0.10.18] - 2026-04-13

### Added
- **Terminal font zoom** — dynamically scale terminal font size using Ctrl+Scroll wheel, Ctrl+Plus/Minus keyboard shortcuts, and Ctrl+0 to reset; uses VTE's native `font_scale` for per-session zoom (0.5×–4.0×) ([#77](https://github.com/totoshko88/RustConn/issues/77))
- **Copy on select** — optional X11-style auto-copy: selected text is automatically copied to the clipboard; enable in Settings → Terminal → Behavior ([#78](https://github.com/totoshko88/RustConn/issues/78))

### Improved
- **Export group filter** — export dialog now includes a group selector to export only connections from a specific group and its subgroups; defaults to "All connections"
- **Import/Export format ordering** — RustConn Native (.rcn) is now the default format in both import and export dialogs; remaining formats sorted alphabetically

### Dependencies
- gio 0.22.4 → 0.22.5
- glib 0.22.4 → 0.22.5
- hyper-rustls 0.27.7 → 0.27.8
- libc 0.2.184 → 0.2.185
- openssl 0.10.76 → 0.10.77
- openssl-sys 0.9.112 → 0.9.113
- pkg-config 0.3.32 → 0.3.33
- rtoolbox 0.0.3 → 0.0.4
- rustls 0.23.37 → 0.23.38

## [0.10.17] - 2026-04-12

### Fixed
- **`clear` command not working in Flatpak SSH sessions** — the Flatpak sandbox inherits `TERM=dumb` from the host, and the previous fix only set `rustconn-256color` for local shells; remote commands (SSH, Telnet, etc.) kept the inherited `dumb` value, breaking `clear`, `htop`, `mc`, `tmux` on remote hosts; now force `TERM=xterm-256color` for all remote commands in Flatpak ([#25](https://github.com/totoshko88/RustConn/issues/25))
- **Sidebar scroll position lost after editing/moving connections** — `restore_state()` scheduled group expansion, scroll restoration, and selection as three independent idle callbacks that raced against each other; scroll was applied before groups finished expanding (which changes content height), causing the sidebar to jump to the top; now runs expansion and selection synchronously in one callback, then restores scroll in a chained second callback
- **Sorting collapsed all expanded groups** — `sort_connections()` and `sort_recent()` rebuilt the sidebar store without saving/restoring expanded group state; now preserves which groups were open before sorting

### Dependencies
- clap_complete 4.6.0 → 4.6.1
- rand 0.9.2 → 0.9.3
- Tailscale CLI 1.96.4 → 1.96.5

## [0.10.16] - 2026-04-10

### Fixed
- **Sidebar context menu actions still not working** — the v0.10.15 fix using `insert_action_group()` proxy was insufficient: `PopoverMenu` inside a `ListView`/`TreeExpander` hierarchy cannot reliably resolve `win.*` actions regardless of where the action group is injected; replaced `PopoverMenu` + `gio::Menu` with a plain `Popover` containing `Button` widgets that directly call `window.activate_action()`, completely bypassing GTK4 action-group resolution ([#75](https://github.com/totoshko88/RustConn/issues/75))

### Dependencies
- cc 1.2.59 → 1.2.60
- gif 0.14.1 → 0.14.2
- hashbrown 0.16.1 → 0.17.0
- indexmap 2.13.1 → 2.14.0
- js-sys 0.3.94 → 0.3.95
- ksni 0.3.3 → 0.3.4
- libredox 0.1.15 → 0.1.16
- redox_syscall 0.7.3 → 0.7.4
- rustls-webpki 0.103.10 → 0.103.11
- wasm-bindgen 0.2.117 → 0.2.118
- web-sys 0.3.94 → 0.3.95

## [0.10.15] - 2026-04-10

### Fixed
- **`clear` command not working in Flatpak** — the `clear` binary from ncurses-utils was missing inside the Flatpak sandbox; added a minimal ANSI escape sequence wrapper (`\033[H\033[2J\033[3J`) to all three Flatpak manifests so `clear` works out of the box ([#25](https://github.com/totoshko88/RustConn/issues/25))
- **Sidebar context menu items not working** — after migration to `PopoverMenu` in v0.10.14, clicking menu items did nothing because the popover lacked access to the window's action group; fixed by explicitly proxying `win.*` actions into the popover via `insert_action_group()` ([#75](https://github.com/totoshko88/RustConn/issues/75))
- **Keyboard shortcuts dialog showed wrong bindings** — 19 discrepancies between the shortcuts help dialog (`shortcuts.rs`) and the actual GTK accelerators (`keybindings.rs`): Ctrl+G was labeled "New group" (actually Password Generator), Ctrl+T was labeled "Open local shell" (actually Ctrl+Shift+T), Ctrl+\` was labeled "Focus terminal" (actually Focus Next Pane), F1 was labeled "Show about dialog" (actually Keyboard Shortcuts); all corrected to match the real bindings
- **Shortcuts dialog missing entries** — added 13 missing shortcuts: Quick Connect, Export, Command Palette, Focus Terminal, Close Pane, Connection History, Statistics, Password Generator, Wake On LAN, Toggle Fullscreen, Toggle Sidebar, and alternative accelerators

### Improved
- **FreeRDP stays at 3.24.1** — 3.24.2 release assets not yet published upstream; keeping 3.24.1 which includes all prior security fixes

### Documentation
- **Keyboard shortcuts fully synchronized** — User Guide shortcuts tables now match the actual keybindings registry; added missing entries for Ctrl+K (Search), Ctrl+PageDown/PageUp (tab switching), Ctrl+Shift+T (Local Shell), Ctrl+H (History), Ctrl+G (Password Generator), Ctrl+Shift+I (Statistics), Ctrl+Shift+L (Wake On LAN)
- **Terminal clear troubleshooting** — added User Guide section explaining VTE's Ctrl+L behavior (scrolls instead of erasing) and workarounds for `clear` command in Flatpak

## [0.10.14] - 2026-04-09

### Dead code cleanup
- **Removed unused CSS classes** — removed `.tab-icon`, `.tab-label`, `.tab-label-disconnected`, `.tab-close-button` (replaced by AdwTabView), `.focused-pane`/`.unfocused-pane` (replaced by `.focused-panel`), `notebook > header > tabs > tab` selector (no longer using GtkNotebook), and stale comment placeholders; updated section headers for clarity

### Improved
- **Success notifications use Toast instead of modal dialogs** — snippet creation, cluster creation now show non-blocking `adw::Toast` instead of `adw::AlertDialog` (GNOME HIG compliance); remaining `show_success` calls with detailed counts (import/export/delete) kept as alerts
- **Fixed missing i18n for export/connection test dialogs** — `"Export Complete"`, `"Connection Test Successful"`, and `"Connection successful! Latency: Xms"` were hardcoded English; now wrapped in `i18n()`/`i18n_f()` for proper localization
- **Accessible labels for status icons and split panels** — sidebar connection status icons (`Connected`, `Connecting`, `Connection failed`) now use `i18n()` for localized screen reader announcements; split panel containers have accessible `"Terminal panel"` label
- **Sidebar context menus migrated to PopoverMenu** — replaced manual `Button`-based `Popover` with `PopoverMenu` + `gio::Menu` for both connection/group and empty-space context menus; provides native GNOME HIG look, keyboard arrow navigation, and screen reader accessibility out of the box

### Fixed
- **Sidebar context menu missing Delete action** — context menu for both connections and groups was cut off at the bottom, hiding the Delete item; fixed by attaching popover to the clicked widget instead of the window, allowing GTK to properly calculate available space and scroll long menus

### Documentation
- **RDP File Transfer** — added User Guide section documenting shared folders (drive redirection) and clipboard file transfer (IronRDP embedded mode "Save Files" button)
- **Complete translations for all 15 languages** — filled all empty/fuzzy translations for be, cs, da, de, es, fr, it, kk, nl, pl, pt, sk, sv, uk, uz; fixed broken PO headers in 10 files; updated version to 0.10.14

## [0.10.13] - 2026-04-08

### Fixed
- **SSH auto-reconnect infinite loop** — when an SSH session failed with "Permission denied" (exit code 255), the auto-reconnect polling detected the host as online (TCP port open) and immediately triggered a reconnect, which failed again with the same auth error, creating an exponential loop of sessions. Fixed by skipping auto-reconnect for SSH authentication failures (exit code 255); the user can still reconnect manually via the overlay button
- **Duplicate `child-exited` handlers for SSH/Telnet** — `setup_child_exited_handler` was called twice per session (before and after spawn), registering two GLib signal handlers. Each exit event fired both handlers, spawning two parallel auto-reconnect polls per failure cycle and doubling the session count on every iteration

### Dependencies
- FreeRDP 3.24.0 → 3.24.1 (security fix: CVE patches for credential zeroing, codec fixes)
- Boundary CLI 0.21.1 → 0.21.2 (search sorting flags)
- tokio 1.51.0 → 1.51.1, toml_edit 0.25.10 → 0.25.11

## [0.10.12] - 2026-04-07

### Security
- **VNC password stored as `SecretString`** — `VncConfig.password` changed from plain `String` to `secrecy::SecretString`, matching RDP/SSH/SPICE credential handling; password is now zeroized on drop and protected from accidental logging via `Debug` trait
- **VNC pixel buffer max resolution guard** — `VncPixelBuffer::new()` and `resize()` now clamp dimensions to 16384×16384 (1 GB max), preventing OOM from a malicious VNC server claiming absurd resolution

### Improved
- **RDP 4K frame conversion zero-copy** — `convert_to_bgra()` now returns `Cow<[u8]>` instead of `Vec<u8>`; when pixel data is already in BGRA format (the common IronRDP case), the function returns a borrowed slice instead of cloning the entire frame buffer (33 MB at 4K per frame)
- **Sidebar search highlight regex cached** — `highlight_match()` now accepts a pre-compiled `Regex` via new `compile_highlight_regex()` helper; the regex is compiled once per query change instead of once per visible list item per keystroke
- **Log sanitization custom patterns pre-compiled** — `SanitizeConfig` now pre-compiles custom regex patterns at construction time instead of recompiling on every call to `sanitize_output()`; affects every line of terminal output when session logging is enabled
- **Log sanitization redundant `to_lowercase()` removed** — `SENSITIVE_PATTERNS` are already lowercase constants; removed unnecessary `pattern.to_lowercase()` allocation on every pattern comparison

### Dead code cleanup
- **Removed `wayland_surface.rs`** — ~1050-line stub module with no callers; all types (`WaylandSubsurface`, `EmbeddedRenderer`, `ShmBuffer`, `DamageRect`, `RenderingMode`) were unused; native Wayland subsurface support can be restored from git history when needed
- **Removed `TracingOutput::OpenTelemetry` variant** — deprecated placeholder that was never constructed; match arm fell back to stderr
- **Removed RDPDR `FileLock` struct and `notify_directory_change()` stub** — dead code placeholders for unimplemented fcntl integration
- **Removed commented-out code** — `set_allow_bold` (VTE4 incompatible), `--full-screen` SPICE arg

### Dependencies
- **Updated**: fastrand 2.4.0→2.4.1, gdk4 0.11.1→0.11.2, gdk4-sys 0.11.1→0.11.2, gio 0.22.2→0.22.4, glib 0.22.3→0.22.4, gtk4 0.11.1→0.11.2, gtk4-sys 0.11.1→0.11.2, libz-sys 1.1.26→1.1.28, pango 0.22.0→0.22.4, zip 8.5.0→8.5.1
- **CLI downloads** — TigerVNC 1.16.1→1.16.2 (security fix for x0vncserver), Teleport 18.7.2→18.7.3, Bitwarden CLI 2026.2.0→2026.3.0

## [0.10.11] - 2026-04-05

### Added
- **RDP Mouse Jiggler** — prevents idle disconnect by sending periodic mouse movements; configurable interval (10–600 seconds, default 60); auto-starts when RDP session connects, auto-stops on disconnect; works with both IronRDP embedded and FreeRDP external modes; settings in Connection Dialog → RDP → Features
- **Connect All in Folder** — right-click a group in the sidebar → "Connect All" opens all connections in that group simultaneously
- **Copy Username / Copy Password from context menu** — right-click a connection → "Copy Username" or "Copy Password" copies credentials to clipboard; password auto-clears from clipboard after 30 seconds for security; uses cached credentials resolved during previous connection
- **Host Online Check** — right-click a connection → "Check if Online" starts async TCP port probing (polls every 5s for up to 2 minutes); auto-connects when host becomes reachable; shows toast notifications for status updates
- **WoL + Auto-Connect** — Wake On LAN now automatically polls the host after sending the magic packet (up to 5 minutes) and auto-connects when the host comes online; replaces the previous fire-and-forget WoL behavior
- **Auto-reconnect on session failure** — when an SSH session disconnects unexpectedly (server reboot, network failure), RustConn automatically starts polling the host (every 5s for up to 5 minutes) and reconnects when the server comes back online; the reconnect banner is still shown for manual reconnect if auto-reconnect times out
- **Host check module** (`rustconn-core::host_check`) — async TCP connect probe with configurable timeout, polling interval, and max duration; cancellation support via `AtomicBool`; `check_host_online()` for single probe, `poll_until_online()` for continuous monitoring
- **Terminal Activity Monitor** — per-session activity and silence detection for terminal tabs, inspired by KDE Konsole ([#72](https://github.com/totoshko88/RustConn/issues/72)); three monitoring modes: Off (default), Activity (notify when new output appears after a configurable quiet period), and Silence (notify when no output occurs for a configurable timeout); notifications delivered through tab indicator icons, in-app toasts, and desktop notifications (when window is unfocused); per-connection config overrides global defaults; settings in Connection Dialog → Advanced → Activity Monitor and Settings → Monitoring → Activity Monitor; tab context menu "Monitor: Off/Activity/Silence" for quick mode cycling; property-based tests for mode cycling, serde round-trip, config resolution, and timeout clamping

### Fixed
- **RDP tabs auto-close on initial connection failure** — RDP tabs that fail during initial connection (CredSSP auth error, connection refused, timeout) now close automatically instead of showing a useless "failed" tab; disconnected tabs are still shown for sessions that were previously connected (for reconnect)
- **Group context menu detection** — fixed `is_group` detection in sidebar context menu to use `ConnectionItem.is_group()` instead of icon name check; groups with custom emoji icons now correctly show group-specific menu items (Connect All, New Connection in Group)

### Dependencies
- **Updated**: fastrand 2.3.0→2.4.0

## [0.10.10] - 2026-04-04

### Changed
- **Flatpak: removed extra sandbox permissions rejected by Flathub lint** — reverted `--filesystem=home/.hoop:ro`, `--filesystem=xdg-run/gnupg:ro`, `--filesystem=home/.var/app/com.bitwarden.desktop/data:ro`, and `--filesystem=xdg-run/ssh-agent:ro` from Flatpak and Flathub manifests; these permissions are now added manually by users via `flatpak override` after installation (see [Flatpak Sandbox Overrides](docs/USER_GUIDE.md#flatpak-sandbox-overrides)); prompted by [flathub-infra/flatpak-builder-lint#972](https://github.com/flathub-infra/flatpak-builder-lint/pull/972#pullrequestreview-4051168156)

### Added
- **User Guide: Flatpak Sandbox Overrides section** — documents how to add filesystem permissions for alternative SSH agent sockets (KeePassXC, Bitwarden, GPG agent, 1Password) and Hoop.dev CLI config after Flatpak installation ([User Guide → Flatpak Sandbox Overrides](docs/USER_GUIDE.md#flatpak-sandbox-overrides))

### Improved
- **Bulk delete dialog migrated to AdwAlertDialog** — replaced custom `adw::Window` with `adw::AlertDialog` using `set_close_response("cancel")` and `ResponseAppearance::Destructive`, following GNOME HIG for destructive confirmation dialogs
- **Background thread result delivery** — `spawn_blocking_with_callback` now uses event-driven `glib::MainContext::channel()` instead of 16ms polling timer, reducing unnecessary main loop wake-ups
- **vault_ops unit tests** — added 14 tests for `select_backend_for_load` (8 backend selection scenarios including KeePass fallback logic) and `generate_store_key` (6 key format scenarios across LibSecret, Bitwarden, 1Password, Pass backends)

### Dependencies
- **Updated**: cc 1.2.58→1.2.59, coreaudio-rs 0.14.0→0.14.1, indexmap 2.13.0→2.13.1, libz-sys 1.1.25→1.1.26, semver 1.0.27→1.0.28, tokio 1.50.0→1.51.0, tokio-macros 2.6.1→2.7.0, writeable 0.6.2→0.6.3, yuv 0.8.12→0.8.13
- **CLI downloads** — TigerVNC 1.16.0→1.16.1

## [0.10.9] - 2026-04-02

### Added
- **Hoop.dev Zero Trust provider** — added Hoop.dev as the 11th Zero Trust provider; supports `hoop connect <connection-name>` with optional `--api-url` and `--grpc-url` flags; includes data model (`HoopDevConfig`), CLI detection (`detect_hoop()`), Flatpak CLI download component, GUI fields in connection dialog, CLI support (`--provider hoop_dev --hoop-connection-name`), Flatpak `~/.hoop:ro` permission, serialization round-trip, i18n, and property-based tests
- **Custom SSH agent socket override** — users can now specify a custom `SSH_AUTH_SOCK` path at two levels: a global setting in Settings → SSH Agent tab (applies to all connections) and a per-connection override in Connection Dialog → SSH tab (overrides global and auto-detected socket); resolves the Flatpak limitation where `--socket=ssh-auth` hard-overwrites `SSH_AUTH_SOCK`, preventing use of alternative agents like KeePassXC or Bitwarden SSH agent ([#71](https://github.com/totoshko88/RustConn/issues/71))
- **CLI `--ssh-agent-socket`** — `rustconn-cli add` and `update` commands accept `--ssh-agent-socket <PATH>` to set per-connection SSH agent socket; `show` command displays the value when set
- **Socket path validation** — real-time feedback in both Settings and Connection dialogs: green for valid socket, yellow for path not found (non-blocking), red for non-absolute path
- **Flatpak: alternative SSH agent socket access** — added `--filesystem` permissions for GPG agent (`xdg-run/gnupg`), Bitwarden SSH agent (`home/.var/app/com.bitwarden.desktop/data`), and custom sockets (`xdg-run/ssh-agent`) in Flatpak and Flathub manifests

### Fixed
- **Orphaned subgroups on group delete** — deleting a group containing only empty subgroups (0 connections) via the GUI now cascade-deletes all descendant subgroups instead of reparenting them to root; CLI `group delete` now delegates to `ConnectionManager` instead of manual `groups.retain()`, fixing dangling `parent_id` references on child groups
- **Startup error dialog orphaned window** — `show_error_dialog` no longer creates a temporary `ApplicationWindow` that lingers after dismissal; now presents via `app.active_window()` parent

### Security
- **Tar archive path traversal (defense-in-depth)** — CLI component downloads now validate each tar entry path against `..` traversal and absolute paths before extraction, matching the existing `enclosed_name()` protection for zip archives; pinned `tar >= 0.4.45` (CVE-2026-33056)
- **RDP certificate validation** — changed default `ignore_certificate` from `true` to `false`; FreeRDP now uses `/cert:tofu` (trust-on-first-use) by default instead of unconditional `/cert:ignore`; applies to all RDP paths (external FreeRDP, embedded launcher, embedded thread)
- **Bitwarden session key no longer exposed in process list** — session key is now passed via `BW_SESSION` environment variable instead of `--session` CLI argument, preventing exposure in `/proc/PID/cmdline`
- **1Password credentials no longer exposed in process list** — password field values are now piped via stdin instead of passed as CLI arguments to `op item create/edit`
- **Export file permissions hardened** — KDBX XML exports and all connection export files now set `0600` (owner-only) permissions on Unix, preventing world-readable credential/topology data
- **Bitwarden session key cleared on vault lock** — `lock_vault()` now calls `clear_session_key()` alongside `clear_verified()`, ensuring the session key does not persist in memory after lock
- **VNC custom args blocklist** — dangerous VNC viewer arguments (`-via`, `-passwd`, `-passwordfile`, `-securitytypes`, `-proxyserver`, `-listen`) are now blocked, matching the existing RDP custom args blocklist
- **FreeRDP extra args blocklist** — `extra_args` in FreeRDP external mode now filtered through the same dangerous-prefix blocklist (`/p:`, `/password:`, `/shell:`, `/proxy:`) as RDP `custom_args`
- **Pass backend path traversal prevention** — `build_pass_path()` now sanitizes `connection_id` and `field` by replacing `/`, `\`, `.` with `_`, preventing directory traversal in the password store
- **Log sanitization expanded** — added `passphrase:`, `client_secret:`, `authorization:` to sensitive prompt patterns; added GitHub (`ghp_*`), GitLab (`glpat-*`), and JWT (`eyJ*`) token detection to value patterns

### Corrected
- **Flatpak `--device=all` clarification** — v0.9.11 release notes incorrectly stated Flatpak permissions were "scoped to `--device=serial`"; Flatpak has no granular `--device=serial` option — the actual permission is `--device=all`, which is required for serial port access via picocom

### Improved
- **Asbru import regex cached** — `convert_asbru_variables()` now uses `LazyLock<Regex>` instead of compiling the regex on every call, matching the pattern used throughout the rest of the codebase
- **Snippet validation strings translated** — "Snippet name is required" and "Command is required" wrapped in `i18n()` for localization
- **Framebuffer fallback warning** — RDP, VNC, and SPICE embedded viewers now log `tracing::warn!` (once per session) when the legacy `to_vec()` pixel buffer copy path is activated instead of `CairoBackedBuffer`
- **Clippy suppressions scoped to GUI crate** — 8 GTK-specific clippy suppressions (`redundant_clone`, `needless_borrow`, `needless_pass_by_value`, `unused_self`, `wildcard_imports`, `needless_borrows_for_generic_args`, `redundant_closure_for_method_calls`, `redundant_closure`) moved from workspace `Cargo.toml` to `rustconn/Cargo.toml`; `rustconn-core` now linted under stricter rules

### Dependencies
- **Updated**: aws-lc-sys 0.39.0→0.39.1, cc 1.2.57→1.2.58, cmake 0.1.57→0.1.58, hybrid-array 0.4.8→0.4.10, hyper 1.8.1→1.9.0, libc 0.2.183→0.2.184, mio 1.1.1→1.2.0, simd-adler32 0.3.8→0.3.9, system-deps 7.0.7→7.0.8, toml_edit 0.25.8→0.25.10, uuid 1.22.0→1.23.0, winnow 1.0.0→1.0.1, zerocopy 0.8.47→0.8.48, zip 8.4.0→8.5.0, zune-jpeg 0.5.14→0.5.15
- **CLI downloads** — Tailscale 1.96.2→1.96.4

## [0.10.8] - 2026-03-27

### Fixed
- **Flatpak: gcloud install fails with read-only filesystem** — `install.sh` now runs with `CLOUDSDK_CONFIG` pointing to the writable sandbox directory, preventing `OSError: [Errno 30]` on `~/.config/gcloud/`

### Improved
- **SPICE/VNC embedded rendering performance** — replaced per-frame `to_vec()` pixel buffer copy with persistent `CairoBackedBuffer` (in-place surface updates + `mark_dirty_rectangle`); eliminates 8–33 MB allocation per frame depending on resolution; same zero-copy pattern already used by embedded RDP since 0.10.7
- **`CairoBackedBuffer` extracted to shared module** — `cairo_buffer.rs` is now used by RDP, VNC, and SPICE embedded widgets instead of three separate implementations
- **`parse_version` regex cached** — `secrets_tab.rs` now reuses `VERSION_REGEX` from `rustconn-core` instead of compiling a new regex on every call
- **`VARIABLE_REGEX` deduplicated** — identical regex was compiled in three modules (`variables/manager.rs`, `snippet/manager.rs`, `utils.rs`); now defined once and re-exported

## [0.10.7] - 2026-03-26

### Changed
- **RDP default quality mode** — new RDP connections now default to Quality (RemoteFX) instead of Balanced; existing connections with explicitly saved Balanced or Speed settings are not affected

### Fixed
- **SPICE fallback viewer reported as failed** — `connect_with_fallback()` returned an error even when the external SPICE viewer launched successfully; now returns `Ok(())` so the GUI correctly shows the connected state
- **SPICE embedded mouse clicks at wrong position** — click and release events sent coordinates (0,0) instead of the actual cursor position; now applies the same widget-to-framebuffer coordinate transformation as mouse motion
- **RDP file import ignores gateway port** — `.rdp` parser read gateway port from `gatewayaccesstoken` instead of the standard `gatewayport` field; gateway connections now use the correct port
- **Session type misclassified for terminal protocols** — only SSH was classified as embedded; Telnet, Serial, Kubernetes, and MOSH sessions are now correctly classified as terminal-embedded
- **MOSH `--ssh` argument not parsed correctly** — `--ssh=ssh -p PORT` was passed as a single argument; now split into `--ssh` and `ssh -p PORT` as two separate arguments for correct parsing
- **MOSH connections accepted port 0** — `validate_connection()` now rejects port 0, consistent with SSH and other protocols
- **Config file corruption on power failure** — synchronous `save_toml_file` now calls `sync_all()` before atomic rename, matching the async version's durability guarantee
- **CLI `delete` auto-confirms in non-interactive mode** — piped input no longer auto-confirms destructive operations; use `--force` to bypass confirmation in scripts
- **CLI `add` allows duplicate connection names** — now returns an error if a connection with the same name already exists
- **CLI `group delete` leaves orphaned connections** — connections belonging to a deleted group now have their `group_id` cleared
- **CLI `update` uses case-sensitive exact match** — now uses `find_connection` for case-insensitive and fuzzy matching, consistent with other commands
- **FreeRDP 2.x flagged as version-incompatible** — detection entries for `wlfreerdp`/`xfreerdp` (2.x) had `min_version("3.0.0")`; corrected to `"2.0.0"`
- **External window saves default size instead of current** — `setup_close_handler` now uses `window.width()`/`height()` to capture actual dimensions after user resize
- **Cluster dialog buttons break on layout change** — Select All / Deselect All buttons are now stored as struct fields instead of being found via fragile `parent()` traversal
- **Whitespace-only group and snippet names accepted** — `validate_group` and `validate_snippet` now trim names before checking emptiness
- **Tray dirty-check hash collision** — replaced simple timestamp sum with `DefaultHasher` combining connection IDs and timestamps
- **`Connection::default_port` duplicated `ProtocolType::default_port`** — now delegates to `self.protocol.default_port()`

### Security
- **Script credential resolver password not zeroed** — intermediate `String` holding the password from script output is now zeroed via `zeroize::Zeroize` after wrapping in `SecretString`
- **Encrypted credential changes not detected** — `SecretSettings::PartialEq` now includes all `*_encrypted` fields so save-if-changed logic detects credential updates

### Improved
- **Highlight rules performance** — `CompiledHighlightRules` now uses `RegexSet` for fast initial filtering before running individual regexes; avoids executing every pattern on every terminal line
- **Command palette sort performance** — `SearchEngine` is now created once before sorting instead of inside every comparator call
- **GTK main loop polling** — `poll_for_result` uses `timeout_add_local` at 16ms intervals instead of `idle_add_local_once` to avoid busy-spinning
- **Terminal themes cached** — `all_themes()` and `theme_names()` use `OnceLock` to avoid repeated allocation
- **Fuzzy search allocation** — `fuzzy_score_optimized` replaced `to_lowercase()` with allocation-free case-insensitive search
- **Export runs on background thread** — large exports no longer freeze the UI
- **CLI download default allocation** — reduced from 10MB to 1MB for small downloads
- **Group descendant collection** — `collect_descendant_groups` uses `HashSet` for O(1) lookups instead of O(n) `Vec::contains`
- **`parse_args` supports quoted strings** — uses `shell_words::split()` so RDP arguments with spaces and quotes are parsed correctly
- **Tray menu translated** — all tray menu strings wrapped in `i18n()`
- **Password generator tips translated** — security tip strings wrapped in `i18n()`
- **Session restore version validation** — `from_json` now warns on version mismatch for forward compatibility
- **ZeroTrust protocol registry documented** — `get_by_type()` explains that ZeroTrust delegates to provider-specific protocols
- **Wayland subsurface code documented** — dead Wayland native paths annotated as future extension points
- **Duplicate CSS rules removed** — `.status-connected` and `.status-connecting` were defined twice in sidebar CSS
- **Dead Flatpak config helpers removed** — unused `get_flatpak_boundary_config_dir` and `get_flatpak_cloudflared_config_dir`
- **`CredentialResolutionContext` struct** — replaces 8-argument function with a bundled context struct
- **Embedded RDP 4K performance** — replaced per-frame 33MB pixel buffer clone (`data.to_vec()`) with a persistent Cairo `ImageSurface` that is updated in-place via `surface.data()` + `mark_dirty_rectangle()`; eliminates the main bottleneck that caused near-slideshow rendering at 4K resolution; old `PixelBuffer` path kept as fallback for FreeRDP external mode
- **RDP frame extraction optimized** — `extract_region_data` replaced per-pixel copy+swap loop with row-based `memcpy` + bulk R↔B channel swap; full-frame fast path avoids row-by-row copy when region covers entire image; LLVM auto-vectorizes the swap loop into SIMD on x86_64
- **RDP cursor artifacts (random pixels below cursor)** — cursor bitmaps from IronRDP are padded to 32×32 or 64×64 with transparent rows; on HiDPI the downscale + compositor upscale caused color bleeding at transparency edges; now crops transparent padding before downscale and uses premultiplied alpha (`B8g8r8a8Premultiplied`) to prevent bleed; R↔B channel swap moved from session layer to cursor handler to avoid double-swap

## [0.10.6] - 2026-03-24

### Fixed
- **Passbolt CLI integration broken with CLI 0.4.2** — `PassboltResourceDetail` deserialization failed because serde looked for `"_id"`, `"_name"`, `"_uri"`, `"_description"` instead of lowercase `"id"`, `"name"`, `"uri"`, `"description"` returned by Passbolt CLI 0.4.2; added `serde(rename)` for all underscore-prefixed fields; made `_id` and `_name` optional since `get resource` no longer returns `id`; added `folder_parent_id` field; same fix applied to `PassboltResource` for `_username` and `_uri` ([#69](https://github.com/totoshko88/RustConn/issues/69))
- **Blurry/artifact RDP image on HiDPI displays** — embedded IronRDP framebuffer was double-scaled on HiDPI (device→CSS→device) because Cairo surface lacked `set_device_scale`; now sets device scale on the pixel buffer surface so Cairo renders 1:1 at native resolution; also uses adaptive filter (Nearest for 1:1, Bilinear for actual scaling)
- **1Password JSON parse errors silently ignored** — `op item list` parse failures were swallowed by `unwrap_or_default()`, masking real issues; now logs warning via `tracing::warn!`

### Changed
- **CLI downloads** — 1Password CLI 2.33.0→2.33.1

### Dependencies
- **Updated**: ipconfig 0.3.2→0.3.4, libredox 0.1.14→0.1.15, proptest 1.10.0→1.11.0

## [0.10.5] - 2026-03-24

### Fixed
- **KeePassXC CLI integration not working** — all vault write/rename/delete/copy operations passed `None` as database password to `keepassxc-cli`, causing "Invalid credentials" errors when the KDBX file is password-protected; now correctly passes `kdbx_password` from settings in all 10 call sites across GUI (`vault_ops.rs`) and CLI (`secret.rs`) ([#68](https://github.com/totoshko88/RustConn/issues/68))
- **KeePassXC CLI silent error swallowing** — `get_password_from_kdbx` silently returned `Ok(None)` for unrecognized errors; `get_password_from_kdbx_with_key` silently skipped failed path attempts; now logs warnings via `tracing::warn!`/`tracing::debug!` for all failure paths
- **KeePassXC CLI missing `-q` flag** — added `-q` (quiet) flag to all `keepassxc-cli show` commands and `verify_kdbx_credentials` to suppress interactive password prompts in scripted usage
- **GTK warnings on application startup** — suppressed `Adwaita-WARNING: gtk-application-prefer-dark-theme` on KDE/XFCE by clearing the deprecated property before `adw::init()`; removed unsupported `@media (prefers-reduced-motion)` CSS media query that caused GTK theme parser warning

### CI
- **GitHub Actions Node.js 20 deprecation** — replaced `flathub-infra/flatpak-github-actions/flatpak-builder@master` (Node.js 20) with `flatpak/flatpak-github-actions/flatpak-builder@v6` (Node.js 24)

### Dependencies
- **Updated**: deflate64 0.1.11→0.1.12, toml 1.0.7→1.1.0, zip 8.3.1→8.4.0

## [0.10.4] - 2026-03-22

### Fixed
- **Flatpak: Zero Trust CLIs crash on read-only filesystem** — gcloud, Azure CLI, Teleport, and OCI CLI need writable config directories; Flatpak mounts host dirs as read-only or doesn't mount them at all; now redirects CLI config paths to writable sandbox directories via environment variables (`CLOUDSDK_CONFIG`, `AZURE_CONFIG_DIR`, `TELEPORT_HOME`, `OCI_CLI_CONFIG_FILE`); bootstraps credentials from host mounts where available; Boundary uses system keyring via D-Bus (works natively in Flatpak); Cloudflare Access SSH uses browser-based auth (no persistent config needed); GCP IAP also gets `--ssh-key-file` and `--strict-host-key-checking=no` to handle read-only `~/.ssh/`
- **Flatpak: Zero Trust CLI tools not found** — `is_host_command_available()` used default PATH which doesn't include Flatpak CLI directories (`~/.var/app/.../cli/`); now uses extended PATH from `get_cli_path_dirs()` so AWS SSM, gcloud, and other installed CLIs are detected correctly
- **Failed connections stuck in "connecting" (yellow) state** — when `start_connection()` returned `None` (e.g. missing CLI, validation error), sidebar status was never reset; now transitions to "failed" (red) on connection launch failure
- **VTE runtime warning on regex match registration** — `match_add_regex()` requires `PCRE2_MULTILINE` compile flag; highlight rules and search highlight regexes were compiled with flags=0, causing `_vte_regex_has_multiline_compile_flag` assertion warning

### Improved
- **Flatpak manifests: FreeRDP and Waypipe modules** — added missing `freerdp` module to `packaging/flatpak/io.github.totoshko88.RustConn.yml` and `packaging/flathub/io.github.totoshko88.RustConn.yml`; added missing `waypipe` module to `packaging/flatpak/io.github.totoshko88.RustConn.yml` — matches documentation claim "FreeRDP 3.24.0 bundled in Flatpak"
- **i18n: 3 untranslated UI strings wrapped** — `"Failed to start"` in settings, `"Enter text above to test patterns"` and `"No patterns matched"` in connection dialog highlight rules, `"Import Failed"` in import dialog, `"Pasted {} chars"` in VNC clipboard — all translated across 15 languages
- **Snap license corrected** — `GPL-3.0+` → `GPL-3.0-or-later` (SPDX)
- **ARM64 release builds** — added `build-deb-arm64`, `build-rpm-arm64`, and `build-appimage-arm64` jobs to release workflow using QEMU emulation

- Updated: `moka` 0.12.14→0.12.15, `yuv` 0.8.11→0.8.12
- **CLI downloads** — Tailscale 1.94.2→1.96.2
- **Libvirt daemon import** — new import source "Libvirt Daemon (virsh)" queries running libvirtd for VMs via `virsh dumpxml`, reusing the existing XML parser; supports `qemu:///session`, `qemu:///system`, and remote URIs ([#63](https://github.com/totoshko88/RustConn/issues/63))

## [0.10.3] - 2026-03-21

### Security
- **RDP password no longer exposed in `/proc`** — legacy `RdpLauncher` passed password as `/p:{pass}` CLI argument visible to all system users; now uses `/from-stdin` pipe matching `SafeFreeRdpLauncher` behavior
- **SSH agent askpass script zeroized before deletion** — passphrase temp file in `/tmp/rustconn-askpass-*/` is now overwritten with zeros and fsynced before `remove_dir_all`, preventing recovery after abnormal termination
- **CLI `--password` flag shows security warning** — `rustconn-cli secret set --password` now prints a warning that the value is visible in process listings and recommends the interactive prompt
- **Legacy XOR credential decryption now logged** — transparent XOR→AES-256-GCM migration now emits `tracing::warn!` so administrators can track remaining legacy credentials

### Fixed
- **Highlight rules not applied without per-connection rules** — built-in defaults (ERROR, WARNING, CRITICAL, FATAL) and global highlight rules were skipped when a connection had no per-connection rules; removed the `is_empty()` guard so highlights always apply ([#66](https://github.com/totoshko88/RustConn/issues/66))
- **CLI `add --protocol zerotrust` silently created SSH connection** — now returns an error instead of logging and falling back to SSH
- **Config file corruption on crash** — sync `save_toml_file` now uses atomic temp-file + rename pattern matching the async version
- **Blocking DNS in async `check_port_async`** — replaced `to_socket_addrs()` with `tokio::net::lookup_host()` to avoid blocking the tokio worker thread

### Improved
- **Sidebar shows full connection name on hover** — tooltip displays full name and host for truncated entries; removed `max_width_chars` limit so labels use all available sidebar space
- **Log sanitization performance** — `sanitize_output()` regex patterns compiled once via `LazyLock` instead of on every call; `SENSITIVE_PATTERNS` deduplicated from 29 to 16 lowercase-only entries
- **CLI `parse_protocol` consolidated** — three duplicate implementations in `add.rs`, `template.rs`, `smart_folder.rs` replaced with shared `parse_protocol_type()` + `default_port_for_protocol()` in `util.rs`
- **`ProtocolResult<T>` deduplication** — removed duplicate type alias from `protocol/mod.rs`, now re-exported from `error.rs`
- **OpenTelemetry tracing variant marked deprecated** — `TracingOutput::OpenTelemetry` now has `#[deprecated]` attribute until implementation is complete
- **Dead code cleanup** — removed unused `AppStateError`, `VncLauncher`, `FieldValidator`/`FormValidator` framework, `initialize_secret_backends()`, `create_async_resolver()`

- Updated: `rustls-webpki` 0.103.9→0.103.10, `zune-jpeg` 0.5.13→0.5.14
## [0.10.2] - 2026-03-20

### Fixed
- **MOSH connections not working** — `start_connection()` dispatch was missing the `"mosh"` arm; MOSH connections silently failed. Added `start_mosh_connection()` with port check, binary detection, and CLI feedback
- **Auto-recording not triggered** — `session_recording_enabled` toggle in connection dialog had no effect; wired auto-recording into SSH, Telnet, Serial, Kubernetes, and MOSH connection handlers using `connect_contents_changed` callback
- **Highlight rules not applied** — per-connection `highlight_rules` were saved but never passed to `TerminalNotebook`; wired `set_highlight_rules()` call into all protocol handlers after terminal tab creation
- **`script` command visible on recording start** — replaced synchronous `feed()` erase with 100ms delayed erase via `glib::timeout_add_local_once` so PTY echo arrives before the clear sequence; added leading space for `HISTCONTROL=ignorespace`
- **Double exit and UI freeze on recording stop** — replaced `exit\n` with `\x04` (Ctrl+D/EOF) to terminate `script` sub-shell without visible echo; moved SCP file retrieval and remote cleanup to background thread via `spawn_blocking_with_callback`
- **Lost commands in recording playback** — added `strip_script_command_echo()` that removes the echoed `script -q -f --log-out …` line from recording data with timing entry adjustment, analogous to existing `strip_script_header()`
- **.rdp files not opening on double-click** — created `application/x-rdp` MIME type XML definition (`io.github.totoshko88.RustConn-rdp.xml`); installed in all packaging formats: Flatpak, Flathub, OBS RPM/DEB, native install script ([#64](https://github.com/totoshko88/RustConn/issues/64))
- **Sidebar stretching with long connection names** — added `ellipsize(End)` and `max_width_chars(35)` to sidebar connection label ([#64](https://github.com/totoshko88/RustConn/issues/64))
- **picocom not detected in Flatpak** — `picocom --help` returns exit code 1 on v3.x causing detection failure; added `which_binary()` fallback that confirms binary existence without running it ([#62](https://github.com/totoshko88/RustConn/issues/62))
- **RDP "indefinite connection" with no feedback** — improved error message when FreeRDP is not installed: now shows "Install FreeRDP 3.x (xfreerdp3 or wlfreerdp3)" instead of raw error ([#61](https://github.com/totoshko88/RustConn/issues/61))
- **IronRDP debug log spam** — filtered `ironrdp`, `ironrdp_session`, `ironrdp_tokio` crates to `warn` level in tracing subscriber; suppresses noisy `Non-32 bpp compressed RLE_BITMAP_STREAM` messages

### Improved
- **CSV import auto-detects delimiter** — `.tsv` files use tab; for `.csv` files, heuristic compares comma/semicolon/tab counts in the first line and picks the most frequent separator
- **Script credentials test feedback** — "Test Script" button now runs the configured command with 30s timeout, shows success with masked output preview or failure with stderr and exit code
- **Config sync documentation** — added "Configuration Sync Between Machines" section to User Guide with Git, Syncthing/rsync, CLI export/import, and built-in Backup/Restore instructions

- New: `shell-words` 1.x added to `rustconn` crate (script credential test button)
- Updated: `aws-lc-rs` 1.16.1→1.16.2, `aws-lc-sys` 0.38.0→0.39.0, `itoa` 1.0.17→1.0.18, `tar` 0.4.44→0.4.45

## [0.10.1] - 2026-03-19

### Note
Thank you to **Todor Todorov** for the support and for pointing out that the donation link was broken. The donation service has been changed and is now working. Today marks 8 months of active development on RustConn. If you'd like to support the project financially, I'd be very grateful: [https://donatello.to/totoshko88](https://donatello.to/totoshko88)

### Added
- **MOSH protocol** — new protocol type with predict mode (Adaptive/Always/Never), SSH port, UDP port range, server binary path, and custom arguments; `MoshProtocol` handler with `build_command()`, `detect_mosh()` in detection module; GUI tab in connection dialog; CLI support
- **CSV import/export** — RFC 4180 compliant CSV parsing and generation; auto column mapping from headers (`name`, `host`, `port`, `protocol`, `username`, `group`, `tags`, `description`); configurable delimiter (comma, semicolon, tab); GUI import dialog with column mapping preview; CLI `import --format csv` and `export --format csv` with `--delimiter` and `--fields` options
- **Session recording** — scriptreplay-compatible format (data + timing files); per-connection toggle in Advanced tab; `●REC` indicator in tab title; sanitization of sensitive output; recordings saved to `$XDG_DATA_HOME/rustconn/recordings/`
- **Text highlighting rules** — regex-based pattern matching with foreground/background colors; per-connection and global rules; built-in defaults for ERROR (red), WARNING (yellow), CRITICAL/FATAL (red background); rules editor in Settings and Connection Dialog; VTE integration
- **Ad-hoc broadcast** — send keystrokes to multiple terminals simultaneously; toolbar toggle button with keyboard shortcut; per-terminal checkboxes for selection; separate from existing cluster broadcast
- **Smart Folders** — dynamic connection grouping with filter criteria: protocol type, tags (AND logic), host glob pattern (`*.prod.example.com`), parent group; sidebar section with read-only connection list; create/edit/delete dialogs; CLI `smart-folders list/show/create/delete` subcommands
- **Script credentials** — `PasswordSource::Script` variant for dynamic credential resolution; shell command parsed via `shell-words`; 30-second timeout via `tokio::time::timeout`; stdout trimmed to `SecretString`; GUI entry with Test button in Auth tab
- **Per-connection terminal theming** — color overrides (background, foreground, cursor) per connection in `#RRGGBB` or `#RRGGBBAA` format; 3 `ColorDialogButton` widgets in Advanced tab; Reset button; VTE `set_color_background/foreground/cursor` integration
- **15 new language translations** — all new UI strings for 8 features translated across uk, de, fr, es, it, pl, cs, sk, da, sv, nl, pt, be, kk, uz

- New: `csv` 1.x (RFC 4180 parsing), `glob` 0.3 (Smart Folder host matching), `shell-words` 1.x (script credential argument splitting)
### Fixed
- Flatpak SSH key paths become stale after rebuild — keys copied to stable `~/.var/app/<app-id>/.ssh/` with fallback resolution ([#62](https://github.com/totoshko88/RustConn/issues/62))
- SFTP `ssh-add` uses stale portal key path — resolved via `resolve_key_path()` before use
- SFTP mc opens even when `ssh-add` fails — now aborts with toast error and "failed" status
- `script` command format updated to `--log-out`/`--log-timing` for modern util-linux
- Remote SSH recording used local paths — now extracts SSH config for remote `script` execution
- Recording playback showed `Script started on …` header — stripped with timing adjustment
- `script` invocation visible in terminal — erased via ANSI escape after `feed_child`
- SCP host key verification prompts in `stop_recording()` — added `-o StrictHostKeyChecking=no`
- RDP sidebar status not clearing after disconnect — `decrement_session_count` called with correct flag
- `PlaybackToolbar` GtkSearchEntry finalization warning — `Drop` unparents popover
- `cargo/config` deprecation warning in Flatpak build — renamed to `config.toml`
- Flatpak local manifest runtime updated from GNOME 50beta to GNOME 50
- Dependencies: euclid 0.22.14, toml 1.0.7, zerocopy 0.8.47, zip 8.3

## [0.10.0] - 2026-03-16

> **Note:** Flatpak release will follow after March 18, 2026, when GNOME 50 runtime is published on Flathub.

### Added
- **RDP file import in GUI** — `.rdp` files can now be imported via the Import dialog (Ctrl+I); previously only available through file association and CLI
- **CLI import: 4 new formats** — `rustconn-cli import` now supports `--format rdp`, `rdm`, `virt-viewer`, and `libvirt` in addition to the existing 7 formats
- **Split view for Telnet, Serial, Kubernetes** — split view now works with all VTE terminal-based protocols, not just SSH/SFTP/Local Shell
- **Statistics: Most Used & Protocol Distribution** — statistics dialog now shows top-3 most used connections and protocol usage breakdown with progress bars
- **5 new customizable keybindings** — Toggle Sidebar (F9), Connection History (Ctrl+H), Statistics (Ctrl+Shift+I), Password Generator (Ctrl+G), Wake On LAN (Ctrl+Shift+L); total now 31 actions
- **Sidebar keyboard shortcuts** — F2 renames selected connection/group, Ctrl+C/Ctrl+V copies/pastes connections, Ctrl+M moves to group; all scoped to sidebar focus so they don't intercept VTE terminal or embedded viewer input
- **Dynamic inventory sync** — new `rustconn-cli sync` command synchronizes connections from external JSON/YAML inventory files; matches by source tag + name + host; supports `--remove-stale` to clean absent connections and `--dry-run` for preview ([#56](https://github.com/totoshko88/RustConn/issues/56))
- **RDP file association** — double-clicking an `.rdp` file opens RustConn and connects automatically; supports address, credentials, gateway, resolution, audio, and clipboard fields ([#54](https://github.com/totoshko88/RustConn/issues/54))
- **FreeRDP bundled in Flatpak** — FreeRDP 3.24.0 SDL3 client built into the Flatpak; external RDP works out of the box on Wayland without `DISPLAY`
- **`sdl-freerdp3` detection** — FreeRDP detection now includes SDL3 variants (`sdl-freerdp3`, `sdl-freerdp`); Wayland priority: `wlfreerdp3` > `wlfreerdp` > `sdl-freerdp3` > `sdl-freerdp` > `xfreerdp3`

### Improved
- **i18n: hardcoded English strings wrapped** — ~40 user-visible strings across sidebar, embedded viewers (RDP, VNC, SPICE), session status overlays, and toolbar buttons now use `i18n()` for translation
- **i18n: accessible labels translatable** — ~25 `update_property` accessible labels in sidebar, window UI, embedded toolbar, and viewer controls wrapped with `i18n()`
- **i18n: protocol display names** — wrapped `display_name()` call sites with `i18n()` and added translations for 15 strings across all 15 languages
- **User-friendly VNC error messages** — raw error variants in VNC session toasts replaced with actionable messages ("Authentication failed. Check your credentials.", "Connection error")
- **VTE context menu moved off terminal widget** — `GestureClick` controller for the right-click context menu moved from the VTE terminal to its container widget; prevents interference with VTE's internal mouse event processing in ncurses/slang applications
- **VTE terminal no longer wrapped in ScrolledWindow** — redundant `ScrolledWindow` wrappers removed since VTE implements `GtkScrollable` natively
- **Monitoring module property tests** — 12 new tests covering `MonitoringSettings`, `MonitoringConfig`, `MetricsParser`, and `MetricsComputer`
- **Stale X11 comment removed** — `embedded.rs` comment referencing `GtkSocket` / X11 embedding updated to reflect native protocol clients

### Fixed
- **Secret backend default mismatch** — `SecretBackendType` default changed from `KeePassXc` to `LibSecret` to match User Guide and provide a universal out-of-the-box experience on all Linux desktops

#### Flatpak sandbox
- **waypipe not detected** — C-only build installs as `waypipe-c`, not `waypipe`; added `post-install` symlink in Flatpak manifest; `detect_waypipe()` now also tries `waypipe-c` as fallback; `which_binary()` checks `/app/bin/` directly in sandbox
- **SFTP file manager ignores SSH key** — external file managers (Dolphin, Nautilus) launched via `xdg-open` run outside the sandbox and cannot access the sandbox's SSH agent; `sftp_use_mc` now defaults to `true` in Flatpak so Midnight Commander (bundled) is used instead
- **ssh-agent socket in read-only `~/.ssh`** — `ensure_ssh_agent()` now uses `-a $XDG_RUNTIME_DIR/rustconn-ssh-agent.sock` inside Flatpak so the agent socket is created in a writable directory
- **KeePassXC not detected** — `keepassxc-cli` on the host system is now detected and executed via `flatpak-spawn --host`; all KDBX operations work transparently inside the sandbox; "Open Password Manager" button launches KeePassXC on the host
- **SSH jump host broken** — replaced `-J` with `-o ProxyCommand=ssh -W %h:%p ...` that passes `StrictHostKeyChecking`, `UserKnownHostsFile`, and identity file to the jump host process
- **mc wrapper not found** — stripped host-exported `mc()` bash function via `--unset-env=BASH_FUNC_mc%%`; installed sandbox wrapper for correct directory-change-on-exit
- **ZeroTrust and Kubernetes connections broken** — CLI tools (`aws`, `gcloud`, `az`, `kubectl`) now detected and executed via `flatpak-spawn --host`; cloud CLI config dirs mounted into sandbox so credentials are shared between sandbox and host
- **mc mouse clicks produce artifacts** — the `xterm-256color` terminfo entry's `XM` extended capability tells ncurses/slang to negotiate SGR mouse mode (1006) with VTE 0.80; mc cannot parse SGR-encoded mouse events, causing raw escape fragments like `7;6M7;6m` on every click; fix: compiled a custom `rustconn-256color` terminfo entry (identical to `xterm-256color` but without `XM`); VTE child processes in Flatpak use `TERM=rustconn-256color` to prevent the negotiation; additionally switched mc build from ncurses to slang and mc SFTP uses `-g` (`--oldmouse`) flag as defense-in-depth

#### Terminal / mc
- **mc SFTP: initial window not fullscreen** — mc read terminal dimensions before VTE widget received its GTK size allocation; added 150ms delay before spawning mc
- **Split view: text selection broken** — `GestureClick` handler no longer claims clicks on `VteTerminal` widgets

#### RDP
- **Crash on RDP connect (RefCell already borrowed)** — the IronRDP event polling loop held an immutable `client_ref.borrow()` while `handle_ironrdp_error` attempted `client_ref.borrow_mut().take()`, causing a double-borrow panic; error handling is now deferred until after the borrow is dropped ([#57](https://github.com/totoshko88/RustConn/issues/57))
- **Crash on RDP connect (ironrdp-tokio panic)** — upstream bug in `ironrdp-tokio 0.8.0` causes `copy_from_slice` panic on 64-bit systems during KDC TCP response parsing; `connect_finalize` is now wrapped in `catch_unwind` so the panic is converted to an error and the GUI falls back to FreeRDP instead of crashing
- **RDP gateway ignored in embedded mode** — IronRDP doesn't support RD Gateway; now falls back to external xfreerdp with a toast ([#53](https://github.com/totoshko88/RustConn/issues/53))
- **External RDP sidebar icon stays green after tab close** — fixed session ID / connection ID mismatch in `add_embedded_session_tab`; external xfreerdp process is now killed on tab close

#### SSH
- **Custom options format unclear** — subtitle now reads "Key=Value, comma-separated" with an example placeholder so users don't have to guess the format ([#58](https://github.com/totoshko88/RustConn/issues/58))
- **`UserKnownHostsFile` defaults to Flatpak path on native build** — `is_flatpak()` now requires `FLATPAK_ID` to match our app ID, preventing false positives when the env var leaks from another Flatpak process ([#59](https://github.com/totoshko88/RustConn/issues/59))

#### Terminal
- **Ctrl+W closes tab instead of deleting word** — changed close-tab shortcut from Ctrl+W to Ctrl+Shift+W (GNOME standard); Ctrl+W now passes through to the shell for backward-kill-word; close-pane moved to Ctrl+Shift+X ([#60](https://github.com/totoshko88/RustConn/issues/60))

#### UI / Clippy
- **Default window size too small on first start** — minimum size increased to 800×500; welcome screen adapts to narrow windows ([#55](https://github.com/totoshko88/RustConn/issues/55))
- **CSS parser warning: `@media (prefers-reduced-motion)`** — GTK4 CSS parser requires explicit value; changed to `@media (prefers-reduced-motion: reduce)`
- **Clippy: `RdpCommand::Connect` large enum variant** — boxed `RdpConfig` payload to reduce enum size from 240 to 16 bytes
- **Clippy: case-sensitive `.rdp` extension check** — now uses `Path::extension()` with `eq_ignore_ascii_case`
- **Clippy: collapsible `if` and `if-not-else`** — cleaned up nested conditionals in protocols, window, and main modules

### Changed
- **GTK4/libadwaita/VTE crate upgrade** — gtk4 0.10→0.11, libadwaita 0.8→0.9, vte4 0.9→0.10; unlocks GNOME 48–50 APIs
- **MSRV bumped to 1.92** — required by updated GTK-rs bindings
- **Flatpak runtime bumped to GNOME 50** — all three manifests now use `org.gnome.Platform` 50 with VTE 0.80
- **AdwSpinner migration** — replaces `gtk::Spinner` in export dialog; cfg-gated `adw-1-6`
- **AdwShortcutsDialog migration** — replaces deprecated `gtk::ShortcutsWindow`; cfg-gated `adw-1-8`
- **AdwSwitchRow migration** — replaces manual `ActionRow` + `Switch` in monitoring, logging, and secrets settings tabs
- **AdwWrapBox for protocol filters** — sidebar filters wrap on narrow sidebars; cfg-gated `adw-1-7` with `GtkBox` fallback
- **Welcome screen refreshed** — updated feature highlights, replaced performance internals with Quick Access tips, added Command Palette / Import / Settings shortcuts
- **CSS `prefers-reduced-motion`** — transitions disabled when reduced motion is requested
- **Tiered distro feature flags** — `adw-1-8` for Tumbleweed/Fedora 43+, `adw-1-6` for Leap 16.0/Fedora 42, baseline for older distros
- **Codebase cleanup** — removed 25+ unused CSS classes, consolidated `futures-util` into `futures`, fixed metainfo.xml duplicates, added k8s keywords, removed dead code

- clap 4.5.60→4.6.0, gtk4 0.11.0→0.11.1, gdk4 0.11.0→0.11.1, gsk4 0.11.0→0.11.1, glib 0.22.2→0.22.3, openssl 0.10.75→0.10.76, tracing-subscriber 0.3.22→0.3.23
- Transitive: anstream 0.6.21→1.0.0, anstyle 1.0.13→1.0.14, anstyle-parse 0.2.7→1.0.0, cc 1.2.56→1.2.57, clap_complete 4.5.66→4.6.0, clap_mangen 0.2.31→0.2.33, colorchoice 1.0.4→1.0.5, glib-sys 0.22.0→0.22.3, once_cell 1.21.3→1.21.4, roff 0.2.2→1.1.0, tinyvec 1.10.0→1.11.0, uds_windows 1.2.0→1.2.1

## [0.9.15] - 2026-03-11

### Added
- **Hide local cursor option for embedded viewers** — new "Show Local Cursor" checkbox in RDP, VNC, and SPICE connection dialogs (Features section) allows hiding the local OS cursor over embedded viewers to eliminate the "double cursor" effect; enabled by default for backward compatibility ([#51](https://github.com/totoshko88/RustConn/issues/51))

### Fixed
- **VNC session ignores Display Mode setting** — the "Display Mode" dropdown (Embedded/External/Fullscreen) in the Advanced tab was saved correctly but had no effect on VNC sessions; now Fullscreen maximizes the main window (same as RDP), and External forces the external VNC viewer (TigerVNC/vncviewer) instead of the embedded vnc-rs client ([#50](https://github.com/totoshko88/RustConn/issues/50))
- **SSH port forwarding via UI broken** — `window/protocols.rs` built SSH args manually, skipping `port_forwards`, X11 forwarding (`-X`), compression (`-C`), and `ControlPersist=10m` from `SshConfig`; refactored to delegate to `SshConfig::build_command_args()` which has the complete logic ([#49](https://github.com/totoshko88/RustConn/issues/49))
- **SSH custom options `-o` prefix not stripped** — `parse_custom_options()` expected `Key=Value` format but users pasted `-o Key=Value` from CLI; now silently strips the `-o` prefix ([#49](https://github.com/totoshko88/RustConn/issues/49))
- **SSH custom options placeholder misleading** — dialog showed `-o StrictHostKeyChecking=no` format but parser expected comma-separated `Key=Value`; updated placeholder and subtitle to clarify correct format ([#49](https://github.com/totoshko88/RustConn/issues/49))

## [0.9.14] - 2026-03-10

### Fixed
- **SSH connection fails in Flatpak on KDE** — host `SSH_ASKPASS` environment variable (e.g. `ksshaskpass`) was inherited by the VTE child process but not available inside the sandbox, causing `Permission denied` before the password prompt appeared; now stripped from the terminal environment since RustConn uses native VTE password injection ([#48](https://github.com/totoshko88/RustConn/issues/48))
- **Header bar buttons clipped when sidebar + monitoring enabled** — monitoring bar's system info label could request more width than available in the content area, causing overflow that pushed header bar buttons out of bounds; fixed by adding `ellipsize` to variable-length labels and `overflow: hidden` on the monitoring bar container ([#47](https://github.com/totoshko88/RustConn/issues/47))

- tokio 1.49→1.50, uuid 1.21→1.22, regex 1.11→1.12, proptest 1.9→1.10, tempfile 3.23→3.26, zip 8.1→8.2, criterion 0.8.1→0.8.2, rpassword 7.3→7.4
- Transitive: hybrid-array 0.4.7→0.4.8, image 0.25.9→0.25.10, libc 0.2.182→0.2.183, libz-sys 1.1.24→1.1.25, moxcms 0.7.11→0.8.1, quinn-proto 0.11.13→0.11.14, schannel 0.1.28→0.1.29, zerocopy 0.8.40→0.8.42

## [0.9.13] - 2026-03-09

### Fixed
- **RDP handshake timeout on loaded servers** — Phase 3 (TLS upgrade + NLA + connect_finalize) now wrapped in `tokio::time::timeout` with `timeout_secs × 2` (minimum 60s); previously only TCP connect had a timeout, causing indefinite hangs when the remote server was under heavy load
- **ARM64 binary download mismatch** — `download_url_for_arch()` on aarch64 no longer falls back to x86_64 URL when no ARM64 binary exists; `get_available_components()` now filters out components unavailable for the current architecture (affects TigerVNC Viewer and Bitwarden CLI)

### Added
- **RDP Quick Actions menu** — new dropdown button on the embedded RDP toolbar with 6 Windows admin shortcuts: Task Manager (Ctrl+Shift+Esc), Settings (Win+I), PowerShell, CMD, Event Viewer, Services; actions send scancode sequences via `SendKeySequence` command with 30ms inter-key delay

## [0.9.12] - 2026-03-08

### Security
- **Removed sshpass dependency** — interactive SSH sessions now use native VTE password injection via `feed_child()`; monitoring SSH uses `SSH_ASKPASS` mechanism with temporary script instead of `SSHPASS` environment variable (no longer visible in `/proc/PID/environ`)
- **Bitwarden master password zeroized on drop** — `unlock_vault()` now wraps the temporary plain-text password copy in `Zeroizing<String>` so heap memory is scrubbed when the blocking task completes
- **SSH monitoring askpass script cleaned up on drop** — temporary `SSH_ASKPASS` helper script is now deleted automatically when the monitoring session ends (RAII wrapper with `Drop` impl)

### Improved
- **Reduced state.rs complexity** — extracted vault operations (~979 lines) into `vault_ops.rs`, trimming `state.rs` from 3143 to 2167 lines
- **Reduced window/mod.rs complexity** — extracted `setup_edit_actions` (637 lines), `setup_terminal_actions` (298 lines), and `setup_split_view_actions` (746 lines) into separate modules, trimming `window/mod.rs` from 5316 to 3648 lines

### Changed
- **SPICE embedded client enabled by default** — `spice-embedded` feature flag now included in default features for both `rustconn-core` and `rustconn` crates; native SPICE client (via `spice-client` crate) is now the primary connection method with `remote-viewer` as fallback

### Removed
- **sshpass** — removed from all packaging manifests (Flatpak, Flathub, Debian, OBS RPM, Snap); no longer a runtime dependency

## [0.9.11] - 2026-03-07

### Security
- **Bitwarden session key now uses SecretString** — session key was stored as plain `String` in memory without zeroization; migrated to `SecretString` with `expose_secret()` only at CLI invocation point
- **Config files written with 0600 permissions** — connection data (hostnames, usernames, port forwards) was world-readable on multi-user systems; config directory now created with 0700
- **SSH monitoring host key verification** — removed unconditional `StrictHostKeyChecking=no`; now uses `accept-new` by default (accepts first-seen keys, rejects changed keys)
- **Session log sanitization active by default** — built-in sensitive patterns (password prompts, API keys, tokens) were defined but never wired into the sanitizer; now active in `SanitizeConfig::default()`
- **Flatpak device permissions documented** — `--device=all` retained in Flatpak manifests with justification comment (serial ports for picocom require it; Flatpak has no granular `--device=serial` option)
- **Monitoring password uses SecretString** — `ssh_exec_factory` password parameter migrated from plain `String` to `SecretString` with zeroization; `expose_secret()` used only at `SSHPASS` env var injection point
- **RDP TLS certificate policy documented** — `establish_connection` now documents that IronRDP does not validate server certificates (standard for RDP self-signed certs); added `tracing::warn!` on each connection

### Fixed
- **Encrypted document format ambiguity** — legacy salt byte could be misinterpreted as encryption strength byte (~1.2% chance); introduced V2 magic header `RCDB_EN2` for unambiguous format detection

### Added
- **Monitoring: remote host private IP** — monitoring bar now shows the primary private IP address in the system info section; hovering shows hostname, all IPv4 and IPv6 addresses grouped separately
- **Monitoring: live uptime counter** — uptime in the system info tooltip now updates on every metrics polling tick instead of remaining static until the next full system info refresh
- **Monitoring: stopped indication** — when the metrics collector stops (3 consecutive failures), the monitoring bar dims to 50% opacity, shows a warning icon, and the tooltip displays "⚠ Monitoring stopped"
- **Monitoring: all mount points** — disk section now shows root filesystem in the level bar and all mounted real filesystems in the tooltip (mount point, used/total, percentage); virtual filesystems (tmpfs, devtmpfs, squashfs, overlay) and snap loop mounts are filtered out

### Removed
- **Dead `read_import_file_async`** — unused async import helper removed from `rustconn-core/src/import/traits.rs`

## [0.9.10] - 2026-03-07

### Fixed
- **Connection dialog Basic tab clipped** — removed redundant outer `ScrolledWindow` wrapping the `ViewStack`; each tab already provides its own scroller, so the nested scroll stole height allocation and clipped the Basic tab content
- **Dialog minimum sizes missing** — added `set_size_request` to Import, Export, and Shortcuts dialogs to prevent UI breakage on small screens
- **Remmina import fails in Flatpak** — importer now also checks the host path `~/.local/share/remmina/` when running inside Flatpak sandbox ([#44](https://github.com/totoshko88/RustConn/issues/44))

### Improved
- **Connection dialog default height** — increased from 500→670px so the Basic tab fields (including Description) are fully visible without scrolling on typical displays

- serde_yaml_ng 0.9→0.10, cfg-expr 0.20.6→0.20.7, inotify 0.11.0→0.11.1, socket2 0.6.2→0.6.3, toml 1.0.4→1.0.6
- CLI downloads: Teleport 18.7.1→18.7.2

## [0.9.9] - 2026-03-06

### Fixed
- **sshpass not installed in Flatpak** — SSH password-authenticated connections broken in Flatpak 0.9.8 ([#42](https://github.com/totoshko88/RustConn/issues/42))
- **Jump host connections fail port check** — pre-connect TCP probe always timed out for destinations reachable only through a jump host; now skipped when `jump_host_id` or `proxy_jump` is configured ([#41](https://github.com/totoshko88/RustConn/issues/41))
- **Jump host dropdown hard to use** — added host address to dropdown labels (`Name (host)`) and enabled search filtering for quick lookup
- **Jump host monitoring fails** — monitoring SSH commands now include `-J` jump host chain so metrics collection works through bastion hosts ([#41](https://github.com/totoshko88/RustConn/issues/41))
- **Jump host false positive connection status** — SSH status detection now checks terminal text for failure patterns (`Connection timed out`, `Connection refused`, etc.) before marking jump host connections as established ([#41](https://github.com/totoshko88/RustConn/issues/41))

- Bitwarden CLI 2026.1.0→2026.2.0, uuid 1.21.0→1.22.0, winnow 0.7.14→0.7.15

## [0.9.8] - 2026-03-05

### Security
- **RDP password no longer exposed on command line** — FreeRDP fallback now uses `/from-stdin` instead of `/p:{password}` argument

### Fixed
- **SSH connection status not turning green** — VTE cursor position axes were swapped; status detection callbacks were skipped when async port check is enabled
- **Automation cursor tracking** — expect-script automation read wrong cursor axis from VTE
- **RDP keyboard input duplication** — deduplicated key press/release handlers via shared `send_ironrdp_key()`
- **Username placeholder on empty `$USER`** — falls back to `$LOGNAME`, then generic placeholder

### Added

**Connection dialog — protocol improvements:**
- **SSH** — password source validation on save, key source "Default" explanation, custom options placeholder, port forwarding duplicate detection
- **RDP** — gateway port/username fields, disable NLA checkbox, clipboard sharing toggle, dynamic resolution info
- **VNC** — encoding dropdown (Auto/Tight/ZRLE/Hextile/Raw/CopyRect), performance mode auto-sync, auth info
- **SPICE** — proxy field for Proxmox VE, CA certificate validation, TLS/skip-verification sensitivity logic
- **Serial** — device auto-detection (`/dev/ttyUSB*`, `/dev/ttyACM*`, `/dev/ttyS*`), dialout group warning
- **Kubernetes** — pod name validation, busybox mode sensitivity
- **Telnet** — plaintext transmission security warning
- **Zero Trust** — CLI availability check, OCI Bastion SSH key/TTL fields, generic command placeholder docs

**Connection dialog — general:**
- Domain field hidden for non-RDP protocols
- MAC address format validation for Wake-on-LAN
- Granular per-connection logging options (activity, input, output, timestamps)
- Password source ↔ SSH auth method auto-sync

**Other:**
- **SFTP mc in split view** — mc-based SFTP sessions now support horizontal/vertical split like SSH
- **Context menu "New Connection"** — opens dialog with the connection's group pre-selected

### Improved
- **Connection dialog decomposition** — extracted 4 tab modules from monolithic `dialog.rs` (~7500→~1500 lines)
- **Embedded RDP decomposition** — extracted 5 modules from monolithic `mod.rs` (~2900→~500 lines)
- **Code quality** — structured tracing fields, i18n coverage, deduplication of clipboard/callback/resize patterns, module-level lint allows removed

- binrw 0.15.0→0.15.1, proc-macro-crate 3.4.0→3.5.0, toml 1.0.3→1.0.4, toml_edit 0.23.10→0.25.4, uds_windows 1.1.0→1.2.0

## [0.9.7] - 2026-03-04

### Fixed
- **Connection group not saved** — connection dialog used a separate `Rc` for `groups_data` in the save closure vs the struct field, so `set_groups()` updated the struct but the save handler always read the initial `[(None, "(Root)")]`; connections now correctly land in the selected subgroup on both create and edit ([#40](https://github.com/totoshko88/RustConn/issues/40))
- **Secret variable values lost after settings reopen** — secret variables had their values cleared before persisting to disk (stored in vault), but were never restored from vault when reopening the Variables dialog or substituting `${VAR}` in connections; added `resolve_global_variables()` that loads secret values from the configured vault backend
- **Crash on session reconnect** — `close_tab` held an immutable borrow on `sessions` while `tab_view.close_page()` synchronously fired the `close-page` signal handler which needed a mutable borrow, causing a `BorrowMutError` panic; separated the borrow from the close call ([#39](https://github.com/totoshko88/RustConn/issues/39))

### Changed
- **Bitwarden credential lookup speed** — removed per-retrieve `bw sync` (network round-trip) and added a 120-second verification cache for `bw status` checks; vault syncs once on unlock instead of on every credential lookup, making reconnect and batch operations significantly faster

- tokio 1.49→1.50, aws-lc-rs 1.16.0→1.16.1, aws-lc-sys 0.37.1→0.38.0, getrandom 0.4.1→0.4.2, ipnet 2.11→2.12, quote 1.0.44→1.0.45, tokio-macros 2.6.0→2.6.1, zip 8.1→8.2

## [0.9.6] - 2026-03-02

### Fixed
- **Bitwarden Flatpak session key** — `build_command` now falls back to the global in-process session store when the instance-level key is absent, so `SecretManager.is_available()` correctly sees an unlocked vault after `auto_unlock` ([#28](https://github.com/totoshko88/RustConn/issues/28))
- **Bitwarden Settings auto-unlock path** — secrets tab auto-unlock now uses `get_bw_cmd()` (globally resolved path) instead of the local `Rc<RefCell>` which may still hold the bare `"bw"` before detection completes
- **Connection dialog credential download** — lookup key now uses `generate_store_key()` (UUID-based) instead of `"{name} ({protocol})"` format, matching the key used by Bitwarden/1Password/Passbolt store operations
- **Vault credential resolve for non-KeePass backends** — `resolve_credentials_blocking` now has a direct `PasswordSource::Vault` block that calls `dispatch_vault_op` with `auto_unlock` for Bitwarden and other backends, instead of falling through to `CredentialResolver` which created a fresh `BitwardenBackend` without session
- **Inherit condition for non-KeePass backends** — group password inheritance no longer blocked when `kdbx_enabled=true` but preferred backend is Bitwarden/1Password/Passbolt/Pass; condition changed from `!kdbx_enabled` to `!matches!(preferred_backend, KeePassXc | KdbxFile)`
- **Group password load from any backend** — group edit dialog password load button now dispatches to the configured default secret backend via `select_backend_for_load` + `dispatch_vault_op`, instead of hardcoded KeePass/Keyring-only branches
- **SSH known_hosts not persisting in Flatpak** — SSH connections now use `-o UserKnownHostsFile=~/.var/app/<app-id>/.ssh/known_hosts` in Flatpak sandbox where `~/.ssh` is mounted read-only; directory is auto-created; applies to interactive SSH, sshpass, Quick Connect, and monitoring; respects user-set `UserKnownHostsFile` in custom options
- **Duplicate reconnect banner** — `TerminalNotebook` now tracks shown reconnect banners per session to prevent duplicates on repeated child-exit signals
- **SSH dialog key fields for Keyboard Interactive** — auth method visibility now correctly hides key path/passphrase fields for Keyboard Interactive (index 2), same as Password (index 0)

### Changed
- **Dependency updates** — moka 0.12.13→0.12.14, pxfm 0.1.27→0.1.28, zlib-rs 0.6.2→0.6.3; kubectl pinned 1.35.1→1.35.2

## [0.9.5] - 2026-03-02

### Fixed
- **SSH/Telnet pre-connect port check** — fail fast with retry toast instead of hanging in "Connecting" state
- **Vault credential lifecycle** — orphaned credentials cleaned on trash empty; paste duplicates credentials; group rename/move migrates KeePass entries
- **Consistent credential keys** — unified `generate_store_key()` across all backends; fixed silent lookup failures from key format mismatch
- **SecretManager cache TTL** — entries expire after 5 minutes, preventing stale credentials
- **Inherit cycle protection** — `HashSet<Uuid>` visited guard prevents infinite loops in group hierarchy
- **Group change in connection dialog** — selecting a different group now correctly persists on save
- **Monitoring race condition** — waits for SSH handshake before opening monitoring channel

### Security
- **SecretString migration** — RDP/SPICE event credentials, GUI password structs, CLI input, and `Variable` (zeroize on Drop) all use `SecretString`

### Changed
- **Backend dispatch consolidation** — `VaultOp` enum + `dispatch_vault_op()` replaces ~200 lines of duplicated match blocks
- **Mutex lock safety** — ~50 `unwrap()` on `Mutex::lock()` replaced with `lock_or_log()` helper
- **Error logging** — `let _ =` on persistence ops replaced with `tracing::warn!`; remaining `eprintln!` migrated
- **CSS extraction** — 595-line inline CSS moved to `rustconn/assets/style.css`
- **i18n consistency** — hardcoded English strings wrapped with `i18n()` / `i18n_f()`
- **CI** — `--all-features` added to test jobs for feature-gated code coverage

### Removed
- Dead code: `StateAccessError`, unused state accessors, legacy dialog tabs, ~30 unused sidebar methods

- js-sys 0.3.90→0.3.91, pin-project-lite 0.2.16→0.2.17, wasm-bindgen 0.2.113→0.2.114, web-sys 0.3.90→0.3.91
## [0.9.4] - 2026-03-01

### Added
- **Session Reconnect** — disconnected VTE tabs show a "Reconnect" banner to re-launch in one click
- **Recursive Group Delete** — three-option dialog: keep children, cascade delete, or cancel
- **Connection History** — search/filter by name/host/protocol; per-entry delete
- **Cluster from sidebar** — "Create Cluster" pre-selects checked connections
- **Shortcut conflict detection** — warning when a keybinding is already assigned
- **Settings Backup/Restore** — export/import all config as ZIP via Settings → Interface
- **Libvirt / GNOME Boxes import** — VNC, SPICE, RDP from domain XML; auto-scans qemu dirs ([#38](https://github.com/totoshko88/RustConn/issues/38))
- **Automation templates** — 5 built-in expect rule presets (Sudo, SSH Host Key, Login, etc.)
- **TemplateManager** — centralized template CRUD with search, protocol filtering, import/export
- **Snippet shell safety** — warns about dangerous metacharacters in variable values before `--execute`

### Fixed
- **Password inheritance** — `PasswordSource::Variable` now resolved in group hierarchy ([#37](https://github.com/totoshko88/RustConn/issues/37))
- **New connection in wrong group** — context menu now pre-selects the target group ([#37](https://github.com/totoshko88/RustConn/issues/37))
- **Toast system** — severity icons, "Retry" on port-check failures, `AlertDialog` fallback, i18n
- **VTE spawn failure** — missing command shows "Command not found" banner + error toast instead of silent empty terminal
- **Cluster broadcast** — keyboard input now actually broadcasts to all cluster terminals; session lifecycle wired; disconnect-all button; full i18n
- **Pango markup** — escaped ampersand in "Backup & Restore" settings title
- **Adwaita dark theme warning** — suppressed on KDE/XFCE desktops

### Improved
- **User Guide** — major rewrite: Zero Trust, Security, FAQ, Migration Guide, expanded all sections
- **Automation engine** — one-shot rules, per-rule timeout, regex validation, template picker, pre-connect/post-disconnect tasks, key sequences on connect
- **Template management** — CLI and GUI migrated to `TemplateManager`; GUI keeps document integration

- **Updated**: js-sys 0.3.90→0.3.91, pin-project-lite 0.2.16→0.2.17, wasm-bindgen 0.2.113→0.2.114, web-sys 0.3.90→0.3.91
## [0.9.3] - 2026-02-27

### Added
- **Waypipe Support** — Wayland application forwarding for SSH connections via `waypipe`; auto-detected on Wayland sessions when `waypipe` binary is available on PATH; per-connection toggle in SSH Session options; graceful fallback to direct SSH when unavailable ([#36](https://github.com/totoshko88/RustConn/issues/36))
- **IronRDP Clipboard Integration** — Bidirectional clipboard sync between local desktop and remote RDP session via cliprdr channel; server→client text is auto-synced to local GTK clipboard; local clipboard changes are automatically announced to the server; Copy/Paste buttons remain as manual fallback; feedback loop prevention via suppression flag

### Fixed
- **Missing icons on KDE and non-GNOME desktops** — Replaced all non-standard icon names (`emblem-ok-symbolic`, `emblem-system-symbolic`, `call-start-symbolic`, `modem-symbolic`, `application-x-executable-symbolic`, etc.) with freedesktop-standard equivalents; replaced icons missing from Adwaita (`emblem-default-symbolic`, `emblem-synchronizing-symbolic`, `utilities-system-monitor-symbolic`, `view-sidebar-start-symbolic`, `tag-symbolic`) with available alternatives; forced Adwaita icon theme via `GtkSettings` for consistent icon availability on all desktops; unified protocol icons via single source of truth in `icons.rs`, eliminating hardcoded duplicates across sidebar, tabs, dialogs, templates, and cluster views ([#35](https://github.com/totoshko88/RustConn/issues/35))
- **Serial connection creation failed** — Serial and Kubernetes connections no longer require host/port validation (they use device path / pod name instead); previously "Host cannot be empty" error blocked saving these connections
- **Serial/Kubernetes missing client toast** — Shows user-friendly toast when picocom (Serial) or kubectl (Kubernetes) is not installed, and when Kubernetes pod/container configuration is incomplete; fixed toast overlay discovery that failed on `adw::ApplicationWindow` internal widget hierarchy
- **libsecret password storage panic** — Fixed `debug_assert` crash in libsecret backend that rejected non-UUID lookup keys (e.g. `"test (vnc)"`); libsecret uses `name (protocol)` format, not UUIDs
- **libsecret password retrieval** — Fixed `is_available()` check that always returned `false` because `secret-tool --version` is not a valid subcommand (exits with code 2); the store path bypassed this check but the retrieve path went through `SecretManager` which skipped the backend, causing saved passwords to never be found on connection
- **VNC/RDP identical icons** — VNC now uses `video-joined-displays-symbolic` (two monitors) instead of `video-display-symbolic` which was identical to RDP's `computer-symbolic` in Adwaita
- **SFTP via mc opens root instead of home** — mc FISH VFS URI now includes `/~` suffix to open the remote user's home directory; mc is launched via `sh -c` wrapper for correct terminal sizing
- **SSH agent not inherited by VTE terminals** — `spawn_command` now injects `SSH_AUTH_SOCK`/`SSH_AGENT_PID` from the global `OnceLock<SshAgentInfo>` into VTE-spawned processes; previously mc, ssh, and other terminal commands could not reach the SSH agent when RustConn started its own agent (Rust 2024 edition forbids `set_var`)

### Improved
- **Client Detection** — Added waypipe to Settings → Clients detection tab
- **Documentation** — Added Waypipe section to User Guide and Architecture docs
- **Translations** — Added waypipe-related strings to all 18 languages

- **Updated**: deflate64 0.1.10→0.1.11, dispatch2 0.3.0→0.3.1, objc2 0.6.3→0.6.4, zerocopy 0.8.39→0.8.40

## [0.9.2] - 2026-02-26

### Added
- **Custom Icons** — Set emoji/unicode or GTK icon names on connections and groups ([#23](https://github.com/totoshko88/RustConn/issues/23))
- **Remote Monitoring** — MobaXterm-style monitoring bar below SSH/Telnet/K8s terminals showing CPU, memory, disk, and network usage from remote Linux hosts; agentless via `/proc/*` parsing; per-connection and global toggle in Settings ([#26](https://github.com/totoshko88/RustConn/issues/26))

### Fixed
- New connections and groups now append to end of list instead of jumping to position 0
- **IronRDP fallback to FreeRDP** — When IronRDP fails during RDP protocol negotiation (e.g. xrdp `ServerDemandActive` incompatibility), the session now auto-falls back to external FreeRDP instead of showing a raw error; shows a user-friendly toast on fallback ([#33](https://github.com/totoshko88/RustConn/issues/33))
- **Monitoring SSH password auth** — Remote monitoring now works with password-authenticated SSH connections via `sshpass`; previously `BatchMode=yes` blocked password auth causing "Permission denied" errors
- **Monitoring error spam** — Monitoring collector now stops after 3 consecutive failures instead of retrying indefinitely and flooding logs
- **Bitwarden CLI not found in Flatpak** — All `bw` command invocations now use a dynamically resolved path instead of hardcoded `"bw"`; `resolve_bw_cmd()` probes Flatpak CLI dir, Snap, `/usr/local/bin`, and `PATH` at startup ([#28](https://github.com/totoshko88/RustConn/issues/28))

### Improved
- **Documentation** — Added User Guide sections for Remote Monitoring and Custom Icons; added monitoring architecture to ARCHITECTURE.md; updated README features table; rewrote Settings section to match the current 4-page `PreferencesDialog` layout (Terminal, Interface, Secrets, Connection); fixed all cross-references to old tab names throughout User Guide; added `docs/BITWARDEN_SETUP.md` step-by-step guide covering Flatpak sandbox, self-hosted servers, API key auth, and troubleshooting
- **Translations** — Completed all 14 language translations to 100% coverage (de, fr, es, it, pl, cs, sk, da, sv, nl, pt, be, kk, uz); added Uzbek (uz) as a new language; fixed corrupted .po file formatting from previous patching

## [0.9.1] - 2026-02-24

### Added
- **Command Palette** — VS Code-style quick launcher (`Ctrl+P` / `Ctrl+Shift+P`) with fuzzy search for connections and `>` / `@` / `#` prefixes for commands, tags, and groups
- **Favorites / Pinning** — Pin connections to a dedicated "Favorites" section at the top of the sidebar via context menu
- **Pass (passwordstore.org) secret backend** — Store and retrieve credentials via `pass` with GPG-encrypted files, custom `PASSWORD_STORE_DIR`, Settings UI, and CLI support ([#32](https://github.com/totoshko88/RustConn/pull/32), contributed by [@h3nnes](https://github.com/h3nnes))
- **Tab coloring by protocol** — Optional colored circle indicator on terminal tabs (SSH=green, RDP=blue, VNC=purple, SPICE=orange, Serial=yellow, K8s=cyan); toggle in Settings → Appearance
- **Snippet timestamps** — `created_at` and `updated_at` fields on `Snippet` model with backward-compatible deserialization
- **Tab grouping** — Right-click context menu on tabs to assign named groups ("Production", "Staging") with color-coded indicators
- **Custom Keybindings** — Fully customizable keyboard shortcuts via Settings → Keybindings with 30+ actions, Record button, per-shortcut Reset, and Reset All

### Fixed
- Command Palette not dismissible via Escape or click-outside
- Favorites group not updating immediately on pin/unpin
- KDBX group visibility regression when loading saved backend preference in Settings
- Doc-comment misplacement in `state.rs` for Pass helper functions

### Improved
- **i18n coverage** — Connection dialog tabs (Basic, Protocol, Data, Logging, Automation, Advanced) and all their content strings now translatable; translations added to all 14 languages
- **User Guide** — Added "Terminal Keybinding Modes" section (vim/emacs in Bash, Zsh, Fish)

- **Updated**: uuid 1.11→1.21, proptest 1.6→1.9, tempfile 3.15→3.23, plus 18 transitive dependency bumps via `cargo update`
### Internal
- Deduplicated `PassBackend` construction in CLI and GUI
- Cached `has_secret_backend()` result in `AppState` to avoid repeated `block_on` calls

## [0.9.0] - 2026-02-21

### Added
- **Startup action** — configure which session opens automatically when RustConn starts: local shell, or any saved connection. Set in Settings → Appearance → Startup, or override via CLI flags `--shell` / `--connect <name|uuid>` ([#30](https://github.com/totoshko88/RustConn/issues/30))

### Security
- All password fields (`FreeRdpConfig`, `RdpConfig`, `SpiceClientConfig`, `KdbxEntry`, `PasswordDialogResult`, `ConnectionDialogResult`) migrated to `SecretString` — credentials are now exposed only at point of use
- FreeRDP embedded thread no longer passes password via CLI arg — uses `/from-stdin` + stdin pipe
- Bitwarden `BW_SESSION` replaced with thread-safe in-process `RwLock` storage instead of `set_var`
- KDBX functions migrated to `SecretString` + `SecretResult` throughout
- SSH `custom_options` now filtered against dangerous directives (`ProxyCommand`, `LocalCommand`, etc.) before passing to `ssh -o`
- Hand-rolled base64 in Bitwarden backend replaced with `data-encoding` crate

### Improved
- **Ukrainian translation** — 674 translations professionally reviewed by Mykola Zubkov for accuracy and modern orthography
- SVG icon optimized and simplified per GNOME HIG; 48×48 and 64×64 PNG removed — GTK renders SVG at any size; 128×128 and 256×256 PNG regenerated from SVG
- Welcome page logo now uses GTK themed icon lookup (same as About dialog) — renders SVG at native HiDPI resolution instead of fixed-size raster
- Flathub metainfo.xml overhauled: description condensed, brand colors improved, screenshots replaced with HiDPI windowed captures with shadows, localized screenshots for uk/be/cs, added translate and contribute URLs
- 8 dialogs migrated to `adw::Dialog` (libadwaita 1.5+) with adaptive sizing and proper modal behavior
- Password field uses `PasswordEntry` with built-in peek icon
- Screen reader support: accessible label relations added to password and connection dialogs
- `adw::Clamp` added to dialogs to prevent content stretching on wide screens
- Dialog header bar pattern deduplicated via shared `dialog_header()` helper
- Clear History now requires confirmation via `adw::AlertDialog`
- Search history popover items are now clickable
- All `eprintln!` calls replaced with structured `tracing`

### Fixed
- **VNC RSA-AES auto-fallback** — servers using RSA-AES security type (type 129, e.g. wayvnc) now automatically fall back to external VNC viewer (TigerVNC) instead of showing a raw error. User sees a friendly toast message ([#31](https://github.com/totoshko88/RustConn/issues/31))
- Embedded RDP cursor size corrected on HiDPI displays — server-sent device-pixel bitmaps now downscaled to logical pixels before GTK cursor creation
- Pango markup warning on welcome page — ampersand in "Embedded & external clients" escaped for GTK label rendering
- Variable password source (`PasswordSource::Variable`) now resolves correctly at connection time — `SecretManager` is initialized with backends from settings, and variable lookup uses the same backend as save
- Locale `.mo` files now included in Debian, RPM, and local Flatpak packages
- Debian build no longer enables `spice-embedded` feature without build dependencies
- AppStream metainfo.xml: categories added explicitly (`Network`, `RemoteAccess`), generic `GTK` category removed
- Debian `Recommends` updated for FreeRDP 3 / Wayland support
- Build dependencies corrected for `gettext` across Debian and RPM

### Removed
- Dead code cleanup: unused credential caching, split view adapter methods, toast helpers, deprecated flatpak host command functions

- **Updated**: deranged 0.5.6→0.5.8, js-sys 0.3.86→0.3.88, wasm-bindgen 0.2.109→0.2.111, wasm-bindgen-futures 0.4.59→0.4.61, web-sys 0.3.86→0.3.88
### Internal
- `Project-Id-Version` updated to `0.9.0` in all `.po` files
- Duplicate `SessionResult` type alias removed from `session/manager.rs` — canonical definition in `error.rs`
- Tray stub no longer allocates orphaned `mpsc` channel when `tray` feature is disabled
- Migrated to Rust 2024 edition (167 files changed across all three crates):
  - Eliminated all `unsafe` `set_var`/`remove_var` calls — SSH agent info stored in `OnceLock<SshAgentInfo>` with `apply_agent_env()` helper, language switching via process re-exec with sentinel guard, Bitwarden session token in `RwLock`
  - Renamed `gen` keyword usages to `generator`/`pw_gen`/`counter` in password generator, dialog, and RDP modules
  - Fixed `ref` binding patterns in match arms across source and test files (Rust 2024 match ergonomics)
  - Hundreds of `collapsible_if` patterns rewritten as let-chains (`if let ... && let ...`)
  - Import ordering updated to Rust 2024 `style_edition` rules via `cargo fmt`

## [0.8.9] - 2026-02-20

### Security
- Input validation hardening across all protocols — `custom_args`, device paths, shell paths, hostnames, proxy URLs, and shared folder names are now validated against injection attacks (null bytes, newlines, shell metacharacters, path traversal)
- SSH config export blocks dangerous directives (`ProxyCommand`, `LocalCommand`, etc.) with inline comments
- KeePassXC socket responses capped at 10 MB; reduced password exposure lifetime
- Async import enforces the same 50 MB file size limit as sync path
- VNC and RDP client passwords migrated to `SecretString` — exposed only at point of use
- FreeRDP external launcher uses `/from-stdin` instead of `/p:{password}` on command line

### Added
- **SSH port forwarding** — Local (`-L`), remote (`-R`), and dynamic SOCKS (`-D`) port forwarding rules can be configured per connection; rules are persisted in `SshConfig.port_forwards` and passed as CLI flags to `ssh` ([#22](https://github.com/totoshko88/RustConn/issues/22))
- **Deferred secret backend initialization** — Bitwarden vault unlock and KDBX password decryption now run asynchronously after the main window is presented, eliminating the 1–3 second startup delay when a secret backend is configured

### Fixed
- `localhost` no longer rejected as placeholder during import
- Bitwarden: fixed duplicate vault writes, false "unlocked" status at startup, auto-unlock after restart, and compatibility with CLI v2026.1.0 including automatic `logout → login → unlock` recovery on "key type mismatch" ([#28](https://github.com/totoshko88/RustConn/issues/28))
- Bitwarden GUI unlock no longer clears password field, preventing stale encrypted password on next save ([#28](https://github.com/totoshko88/RustConn/issues/28))
- Generic ZeroTrust `custom_args` now embedded into shell command instead of passed as positional parameters
- RefCell borrow panic in EmbeddedRdpWidget; VNC polling mutex contention; RDP polling timer leak
- FreeRDP now uses native Wayland backend (removed `QT_QPA_PLATFORM=xcb` override)
- Several `unwrap()` panics replaced with safe fallbacks (VNC, TaskExecutor, tray, build.rs)
- EmbeddedRdpWidget resize signal handler properly cleaned up on disconnect
- Quick connect RDP fails with "Got empty identity" CredSSP error — NLA is now auto-disabled when username or password is not provided, letting the server prompt for credentials ([#29](https://github.com/totoshko88/RustConn/issues/29))
- Bitwarden vault unlock moved to a background thread — eliminates "application not responding" dialog on startup when Bitwarden is the configured secret backend

### Changed
- **CLI downloads** — Tailscale 1.94.1→1.94.2, Teleport 18.6.8→18.7.0, kubectl 1.35.0→1.35.1
- **Documentation** — Updated README, ARCHITECTURE, and USER_GUIDE with SSH port forwarding and deferred secret backend initialization

### Improved
- ~40 `eprintln!` calls migrated to structured `tracing` across GUI crate
- VNC client warns about unencrypted connections

### Internal
- `tracing` moved to workspace dependencies; deprecated flatpak re-exports removed
- API surface migrated from flat re-exports to modular paths (`rustconn_core::models::*`, etc.)
- Architecture audit: 51 findings, 49 resolved

- **serde_yaml** replaced with **serde_yaml_ng** 0.9 (maintained fork; transparent rename)
- **cpal** `0.17.1` → `0.17.3`
- **clap** `4.5.59` → `4.5.60`

## [0.8.8] - 2026-02-18

### Security
- **AES-256-GCM for stored credentials** — Replaced XOR obfuscation with AES-256-GCM + Argon2id key derivation for KeePassXC, Bitwarden, 1Password, and Passbolt passwords in settings; transparent migration from legacy format on first save
- **FreeRDP password via stdin** — Passwords are now passed using `/from-stdin` instead of `/p:{password}` command-line argument, preventing exposure via `/proc/PID/cmdline`

### Changed
- **FreeRDP detection unified** — Single `detect_best_freerdp()` function with Wayland-first candidate ordering (`wlfreerdp3` → `wlfreerdp` → `xfreerdp3` → `xfreerdp`); all detection paths delegate to it
- **RDP `build_args()` decoupled** — New `build_args()` and `build_command_with_binary()` methods on `RdpProtocol` separate argument construction from binary name; callers determine the binary via runtime detection
- **ZeroTrust validation** — Provider-specific `validate()` on `ZeroTrustConfig` checks required fields (AWS SSM target, GCP IAP instance/zone/project, Teleport cluster, Tailscale hostname, Generic command template) before save
- **ZeroTrust CLI detection** — CLI tool availability (`aws`, `gcloud`, `tsh`, `tailscale`) is verified before connection launch; missing tools show a toast and log a warning
- **ZeroTrust tracing** — Connection launch attempts and failures are now logged via `tracing` in both GUI and CLI paths
- **Native export format v2** — `NativeExport` now includes `snippets` field; backward-compatible with v1 imports via `#[serde(default)]`

- **native-tls** `0.2.14` → `0.2.18` — Removed version pin; 0.2.18 fixes the `Tlsv13` compile error from 0.2.17 ([#367](https://github.com/rust-native-tls/rust-native-tls/issues/367))
- **toml** `0.8` → `1.0` — Major version bump; no API changes required (re-export crate, fully compatible)
- **zip** `2.2` → `8.1` — Major version bump; replaced deprecated `mangled_name()` with `enclosed_name()` which adds path traversal validation
### Fixed
- **RDP HiDPI scaling on 4K displays** — IronRDP now sends `desktop_scale_factor` to the Windows server (e.g. 200% on a 2× display), so remote UI elements render at the correct logical size instead of appearing tiny; previously hardcoded to 0
- **RDP mouse coordinate mismatch on HiDPI** — Widget dimensions used for mouse→RDP coordinate transform now store CSS pixels (matching GTK event coordinates) instead of device pixels, fixing misaligned clicks on scaled displays
### Removed
- **Dashboard module** — Removed unused `ConnectionDashboard` GUI widget, core types (`SessionStats`, `DashboardFilter`), and property tests; session monitoring is handled by Active Sessions manager and sidebar indicators
- **5 dead GUI modules** — Removed `adaptive_tabs.rs`, `empty_state.rs`, `error_display.rs`, `floating_controls.rs`, `loading.rs` (all replaced by native adw/GTK4 equivalents)
- **`tab_split_manager` remnants** — Removed unused field from `MainWindow` and `SharedTabSplitManager` type alias; split view fully handled by `SplitViewBridge`

## [0.8.7] - 2026-02-17

### Security
- **Variable injection prevention** — All variable substitution in command-building paths now validates resolved values, rejecting null bytes, newlines, and control characters to prevent command injection
- **Checksum policy for CLI downloads** — Replaced placeholder SHA256 strings with `ChecksumPolicy` enum (`Static`, `SkipLatest`, `None`) for explicit integrity verification
- **Sensitive CLI arguments masked** — Password-like arguments (`/p:`, `--password`, `token=`, etc.) are masked in log output
- **Configurable document encryption** — `EncryptionStrength` enum (Standard/High/Maximum) with per-level Argon2 parameters; backward-compatible with legacy format
- **SSH Agent passphrase handling** — `add_key()` now uses `SSH_ASKPASS` helper script with `SSH_ASKPASS_REQUIRE=force` to securely pass passphrases to `ssh-add` without PTY; temporary script is cleaned up immediately after use

### Added
- **Internationalization (i18n)** — gettext support via `gettext-rs` with system libintl; `i18n` module with `i18n()`, `i18n_f()`, `ni18n()` helpers; translations for 14 languages: uk, de, fr, es, it, pl, cs, sk, da, sv, nl, pt, be, kk; closes [#17](https://github.com/totoshko88/RustConn/issues/17)
- **SPICE proxy support** — `SpiceConfig.proxy` field stores proxy URL from virt-viewer `.vv` imports; `remote-viewer` receives `--spice-proxy` flag for Proxmox VE tunnelled connections; fixes [#18](https://github.com/totoshko88/RustConn/issues/18)
- **RDP HiDPI fix** — IronRDP embedded client now multiplies widget dimensions by `scale_factor()` to negotiate device-pixel resolution on HiDPI displays, eliminating blurry upscaling; fixes [#16](https://github.com/totoshko88/RustConn/issues/16)
- **Property tests for variable injection** — 8 proptest properties validating command injection prevention
- **CLI delete confirmation** — Interactive prompt with `--force` flag to skip
- **CLI `--verbose` / `--quiet`** — Global flags for controlling output verbosity
- **CLI `--no-color` / `NO_COLOR`** — Per [no-color.org](https://no-color.org/) convention
- **CLI shell completions** — `completions <shell>` for bash, zsh, fish, elvish, PowerShell
- **CLI `--dry-run` for connect** — Prints command without executing
- **CLI pager for long output** — Pipes through `less` when output exceeds 40 lines
- **CLI auto-JSON when piped** — List commands switch to JSON when stdout is not a terminal
- **CLI fuzzy suggestions** — "Did you mean: x, y, z?" on connection name mismatch
- **CLI man page generation** — `man-page` subcommand via `clap_mangen`
- **Ctrl+M "Move to Group"** — Keyboard shortcut for moving sidebar items between groups
- **Search history navigation** — Up/Down arrows cycle through sidebar search history
- **CI version check workflow** — Weekly GitHub Action checks upstream CLI versions
- **Client detection caching** — 5-minute cache for CLI version checks
- **Flathub x-checker-data** — Automated dependency tracking for vte, libsecret, inetutils, picocom, mc
- **Flathub device metadata** — `<requires>`, `<recommends>`, `<supports>` in metainfo.xml

### Fixed
- **CLI `--config` flag** — Was declared but never used; now threads through all 43 `ConfigManager` call sites
- **Flatpak components dialog** — Hides unusable protocol clients in sandbox; shows only network-compatible tools
- **SPDX license** — `GPL-3.0+` → `GPL-3.0-or-later` in metainfo.xml

### Changed
- **VTE** — Flatpak manifests use VTE 0.78.7 (LTS branch for GNOME 46/47); `vte4` Rust crate 0.9 with `v0_72` feature
- **CLI modularized** — Split 5000+ line `main.rs` into 18 handler modules
- **CLI structured logging** — `tracing` replaces `eprintln!` with `--verbose`/`--quiet` control
- **VNC viewer list deduplicated** — Single `VNC_VIEWERS` constant shared across detection
- **Protocol icon mapping unified** — `get_protocol_icon_by_name()` in core replaces duplicate match blocks
- **Protocol command building unified** — `Protocol::build_command()` trait; CLI delegates to `ProtocolRegistry`
- **Send Text dialog** — Migrated to `adw::Dialog` per GNOME HIG
- **Sidebar minimum width** — Reduced from 200px to 160px
- **Tray polling optimized** — Split into 50ms message handling + 2s state sync with dirty-flag tracking

### Deprecated
- **Flatpak host command functions** — `host_command()`, `host_has_command()`, etc. in `flatpak.rs`; `flatpak-spawn --host` disabled since 0.7.7

### Improved
- **Accessible labels** — Added to 20+ icon-only buttons for screen reader compatibility
- **Czech translation (cs)** — Native speaker review by [p-bo](https://github.com/p-bo); 45 translations improved ([PR #19](https://github.com/totoshko88/RustConn/pull/19))
- **Remmina RDP import** — Now imports `gateway_server`, `gateway_username`, and `domain` fields from Remmina RDP profiles ([#20](https://github.com/totoshko88/RustConn/issues/20))

## [0.8.6] - 2026-02-16

### Fixed
- **Embedded RDP keyboard layout** — Fixed incorrect key mapping for non-US keyboard layouts (e.g. German QWERTZ) in IronRDP embedded client ([#15](https://github.com/totoshko88/RustConn/issues/15))
- **Secrets management** — Comprehensive fixes to vault credential storage, backend dispatch, and Bitwarden integration ([#14](https://github.com/totoshko88/RustConn/issues/14)):
  - All vault operations now respect `Settings → Secrets → preferred_backend` instead of being hardcoded to libsecret
  - Bitwarden encrypted password is decrypted and vault auto-unlocked at startup when preferred backend is Bitwarden
  - `PasswordSource::Inherit` resolves group passwords through non-KeePass backends with correct hierarchy traversal
  - RDP and VNC password prompts auto-save entered passwords to vault when `password_source == Vault`
  - Toast notifications shown on all vault save error paths
- **Flatpak component checksums** — Fixed kubectl installation failing with `ChecksumMismatch`; updated boundary v0.21.0 checksum
- **Flatpak component uninstall/reinstall** — Fixed `AlreadyInstalled` error when reinstalling AWS CLI and Google Cloud CLI
- **Terminal search Highlight All** — Fixed checkbox toggling to next match instead of highlighting

### Changed
- **Dependencies** — Updated: `futures` 0.3.31→0.3.32, `libc` 0.2.181→0.2.182, `uuid` 1.20.0→1.21.0, `bitflags` 2.10.0→2.11.0, `syn` 2.0.114→2.0.116, `native-tls` 0.2.14→0.2.16, `png` 0.18.0→0.18.1, `cc` 1.2.55→1.2.56

## [0.8.5] - 2026-02-15

### Added
- **Kubernetes Protocol** — Shell access to Kubernetes pods via `kubectl exec -it` ([#14](https://github.com/totoshko88/RustConn/issues/14)):
  - `KubernetesConfig` model with kubeconfig, context, namespace, pod, container, shell, busybox toggle
  - Two modes: exec into existing pod, or launch temporary busybox pod
  - GUI: Connection dialog Kubernetes tab, sidebar K8s quick filter
  - CLI: `kubernetes` subcommand with full flag support
  - Sandbox: kubectl as Flatpak downloadable component
- **Virt-Viewer (.vv) Import** — Import SPICE/VNC connections from virt-viewer files ([#13](https://github.com/totoshko88/RustConn/issues/13)):
  - Parses `[virt-viewer]` INI sections: host, port, tls-port, password, proxy, CA cert, title
  - Supports `type=spice` (with TLS detection) and `type=vnc`
  - Compatible with libvirt, Proxmox VE, and oVirt generated `.vv` files
- **Serial Console Protocol** — Full serial console support via `picocom` ([#11](https://github.com/totoshko88/RustConn/issues/11)):
  - `SerialConfig` model with device path, baud rate (9600–921600), data bits, stop bits, parity, flow control
  - GUI, CLI, and Flatpak sandbox support with bundled `picocom`
- **SFTP File Browser** — SFTP integration for SSH and standalone SFTP connections ([#10](https://github.com/totoshko88/RustConn/issues/10)):
  - "Open SFTP" action via `gtk::UriLauncher` (portal-aware)
  - "SFTP via mc" option with Midnight Commander FISH VFS
  - Standalone `ProtocolType::Sftp` connection type
- **Responsive / Adaptive UI** — Improved dialog sizing and window breakpoints ([#9](https://github.com/totoshko88/RustConn/issues/9))
- **Terminal Rich Search** — Regex, highlights, case-sensitive, wrap-around ([#7](https://github.com/totoshko88/RustConn/issues/7))

### Changed
- **Session Logging moved to Logging tab** — Better discoverability
- **CLI component versions updated** — Bitwarden CLI 2024.12.0→2026.1.0, Teleport 17.1.2→18.6.8, Boundary 0.18.1→0.21.0, 1Password CLI 2.30.0→2.32.1, kubectl 1.32.0→1.35.0

### Fixed
- **Flathub linter `finish-args-home-filesystem-access`** — Replaced `--filesystem=home` with `--filesystem=xdg-download:create`
- **Flathub linter `module-rustconn-source-git-no-commit-with-tag`** — Added explicit `commit` hash
- **ZeroTrust icon inconsistency** — Changed to `security-high-symbolic` across all UI
- **SFTP tab icon** — Correct `folder-remote-symbolic` icon
- **SFTP sidebar status** — Shows connecting/connected status and increments session count

## [0.8.4] - 2026-02-14

### Added
- **FIDO2/SecurityKey SSH authentication** — `SshAuthMethod::SecurityKey` variant for hardware key auth
- **CLI auth-method support** — `--auth-method` flag for `add` and `update` commands

### Fixed
- **CLI version check timeout** — Increased from 3 to 6 seconds for Azure CLI
- **Settings dialog startup delay** — Replaced blocking `is_secret_tool_available_sync()` with cached async detection
- **WoL MAC Entry Disabled on Edit** — Fixed sensitivity conflict between widget and group-level control
- **secret-tool detection** — Replaced invalid `secret-tool --version` with `which secret-tool`
- **Settings version label race condition** — Added `detection_complete` flag
- **Unequal split panel sizes** — Set `size_request(0, 0)` on panel containers

### Refactored
- **ConnectionManager watch channels** — Replaced `Arc<Mutex<Option<Vec<T>>>>` with `tokio::sync::watch`
- **Embedded RDP module directory** — Reorganized into `embedded_rdp/` with 6 submodules
- **Window module directory** — Reorganized 14 flat files into `window/` directory
- **OverlaySplitView sidebar** — Replaced `gtk::Paned` with `adw::OverlaySplitView`
- **Protocol trait capabilities** — Extended with `capabilities()` and `build_command()`

### Changed
- **Dependencies** — Updated `resvg` 0.46→0.47

## [0.8.3] - 2026-02-13

### Added
- **Wake On LAN from GUI** — Send WoL magic packets directly from the GUI ([#8](https://github.com/totoshko88/RustConn/issues/8))

### Fixed
- **Flatpak libsecret Build** — Disabled `bash_completion` (EROFS in sandbox)
- **Flatpak libsecret Crypto Option** — Renamed `gcrypt` to `crypto`
- **Thread Safety** — Removed `std::env::set_var` from FreeRDP spawned thread
- **Flatpak Machine Key** — App-specific key file in `$XDG_DATA_HOME`
- **Variables Dialog Panic** — Replaced `expect()` with `if let Some(window)` pattern
- **Keyring `secret-tool` Check** — Returns `SecretError::BackendUnavailable` if not installed
- **Flatpak CLI Paths** — No longer adds hardcoded paths when running inside Flatpak
- **Settings Dialog Performance** — Moved all detection to background threads; dialog opens instantly
- **Settings Clients Tab Performance** — Parallelized CLI detection; ~15s → ~3s
- **Settings Dialog Visual Render Blocking** — Replaced `glib::spawn_future` with `std::thread::spawn` + `glib::idle_add_local`

## [0.8.2] - 2026-02-11

### Added
- **Shared Keyring Module** — Generic `store()`, `lookup()`, `clear()` for all secret backends
- **Keyring Support for All Backends** — Bitwarden, 1Password, Passbolt, KeePassXC
- **Auto-Load Credentials from Keyring** — Automatic restore on settings load
- **Flatpak `secret-tool` Support** — `libsecret` 0.21.7 as Flatpak build module
- **Passbolt Server URL Setting** — New field in `SecretSettings`
- **Unified Credential Save Options** — Consistent "Save password" / "Save to keyring" across all backends

## [0.8.1] - 2026-02-11

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

## [0.6.7] - 2026-01-29

### Added
- **Group-Level Secret Storage** — groups can now store passwords in secret backends (KeePassXC, libsecret, Bitwarden, 1Password, Passbolt)
- **CLI Secret Management** — new `secret` command for managing credentials from the command line
- **Hierarchical KeePass Storage** — KeePass storage mirrors group structure for organized credential management

## [0.6.6] - 2026-01-27

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

- Updated `quick-xml` 0.38 → 0.39
- Updated `resvg` 0.45 → 0.46
- Updated `usvg` 0.45 → 0.46
- Updated `svgtypes` 0.15 → 0.16
- Updated `roxmltree` 0.20 → 0.21
- Updated `kurbo` 0.11 → 0.13
- Updated `gif` 0.13 → 0.14
- Updated `imagesize` 0.13 → 0.14
- Updated `zune-jpeg` 0.4 → 0.5
## [0.6.2] - 2026-01-15

### Added
- **MobaXterm Import/Export** — full support for `.mxtsessions` files
- **Connection History Button** — quick access from sidebar toolbar
- **Run Snippet from Context Menu** — right-click on connection → "Run Snippet..."
- **Persistent Search History** — up to 20 recent searches saved across sessions

- Updated `quick-xml` 0.38 → 0.39, `resvg` 0.45 → 0.46
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
