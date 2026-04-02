# Changelog

All notable changes to RustConn will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
