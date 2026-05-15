# macOS Port — Changelog

## Changes for macOS Support

### Added

- **macOS Keychain backend** (`rustconn-core/src/secret/macos_keychain.rs`) — native credential storage using Security.framework via `security-framework` crate. Implements `SecretBackend` trait with store/retrieve/delete operations through macOS Keychain Services API. Uses `tokio::task::spawn_blocking` for async compatibility. No CLI tools needed, no PATH issues in `.app` bundles. Added `SecretBackendType::MacOsKeychain` variant with full integration across resolver, manager, detection, settings UI, and CLI.
- **Native PTY spawn for macOS** (`rustconn/src/macos_pty.rs`) — VTE's built-in `spawn_async` does not work on macOS (Homebrew build); the PTY is created but never connected to child process output. New module creates PTY via `nix::pty::openpty()`, spawns the child with slave fd as stdin/stdout/stderr, and hands the master fd to VTE via `Pty::foreign_sync()`. Uses `process_group(0)` for basic job control. Conditional compilation `#[cfg(target_os = "macos")]` ensures zero impact on Linux.
- **macOS PATH extension in `get_extended_path()`** — GUI apps launched via `.app` bundle have minimal PATH (`/usr/bin:/bin`). Added `/opt/homebrew/bin`, `/opt/homebrew/sbin`, `/usr/local/bin`, and `/Applications/KeePassXC.app/Contents/MacOS` to the extended PATH on macOS. This fixes detection of all CLI tools (keepassxc-cli, bw, op, pass, gcloud, kubectl, etc.) without using `set_var`.
- **Unified `detection_command()` helper** — All `detect_*` functions in `detection.rs` now use a shared helper that injects the extended PATH into every spawned `Command`. This ensures all secret backends (KeePassXC, Bitwarden, 1Password, Pass, Passbolt) are discoverable on macOS without per-backend fallback paths.
- **Platform-aware URL opener** — `url_open_command()` returns `open` on macOS and `xdg-open` on Linux. Used by all backends that open web vaults or file managers.
- **macOS .app bundle** — `RustConn.app` with proper `Info.plist`, `.icns` icon, wrapper script setting environment variables, and self-contained binary.
- **macOS tray icon** (`tray-macos` feature) — native NSStatusItem via `tray-icon` + `muda` crates. Provides menu bar icon with Show/Hide Window, Recent Connections, Quick Connect, Local Shell, About, and Quit actions. Replaces the Linux-only `ksni` D-Bus tray on macOS.
- **Homebrew formula** (`packaging/macos/rustconn.rb`) — Complete formula for Homebrew Tap distribution with all dependencies, locale compilation, icon generation, and .app bundle creation.
- **DMG build script** (`packaging/macos/build-dmg.sh`) — Automated script to build release `.dmg` with self-contained `.app` bundle including Adwaita icons, locales, and GSettings schemas. Version is read dynamically from `Cargo.toml`.

### Fixed

- **Cross-platform `statvfs` types** (`rustconn-core/src/rdp_client/rdpdr.rs`) — `fragment_size()`, `blocks()`, `blocks_available()` return different integer types on macOS vs Linux. Added `u64::from()` with `#[cfg_attr(not(target_os = "macos"), allow(clippy::useless_conversion))]` for cross-platform compatibility without clippy warnings on either platform.
- **Local Shell on macOS** — launches with `--login` flag so `.zprofile`/`.zshrc` are sourced (macOS-only via `#[cfg]`).
- **Secret backend detection on macOS** — all backends now use extended PATH, removing the need for per-tool fallback path lists.
- **Removed invalid Cellar path** — `/usr/local/Cellar/keepassxc/keepassxc-cli` never existed (Cellar paths include version). Removed from `status.rs`.
- **Unified extended PATH for all secret backends** — `bitwarden.rs`, `onepassword.rs`, `passbolt.rs`, `pass.rs`, `libsecret.rs`, `keyring.rs`, and `status.rs` now inject `get_extended_path()` into every spawned `Command`. Previously only `detection.rs` (UI panel) used extended PATH; the actual backend operations used bare `Command::new("tool")` which would fail on macOS `.app` bundles.
- **PTY child process cleanup on error** — if `Pty::foreign_sync()` fails after the child is spawned, the child is now killed via `SIGKILL` and reaped via `waitpid()` to prevent zombie processes.
- **PTY child handle race condition** — `std::mem::forget(child)` prevents `Child::drop()` from calling `waitpid()` which would race with GLib's `child_watch_add_local` reaper.
- **PTY fd leak prevention** — `nix::unistd::dup()` returns `OwnedFd` which auto-closes on drop; if a later dup fails, earlier fds are cleaned up automatically via RAII.
- **Essential env vars in macOS PTY** — when `envv` is non-empty, `HOME`, `USER`, `LOGNAME`, `SHELL`, `LANG`, `PATH` are inherited from parent if not already provided. Prevents shell malfunction in `.app` bundles.
- **Reconnect banner on macOS spawn failure** — previously only a toast was shown; now the full reconnect overlay (indicator icon + banner + button) is displayed, matching Linux behavior.
- **`nix` dependency is macOS-only** — moved to `[target.'cfg(target_os = "macos")'.dependencies]` to avoid unnecessary compilation on Linux.
- **Removed hardcoded `GDK_DPI_SCALE=0.5`** — GTK4 handles HiDPI natively on macOS; the forced scale factor caused incorrect sizing on non-Retina displays.
- **Homebrew formula: added `librsvg` build dependency** — required for `rsvg-convert` used during icon generation.
- **Info.plist: added `LSMinimumSystemVersion` (13.0)** — declares macOS Ventura as minimum for GTK4 compatibility.
- **Info.plist: added `NSAppleEventsUsageDescription`** — prevents unexpected permission dialogs when opening URLs.
- **DMG: ad-hoc code signing** — `codesign --force --deep --sign -` applied before DMG creation to reduce Gatekeeper friction.
- **macOS tray: main-thread initialization** — `NSStatusItem` (via `tray-icon` crate) must be created on the main thread. Moved tray creation out of background thread into synchronous main-thread initialization (Linux `ksni` D-Bus tray still uses background thread).
- **macOS tray: dynamic menu rebuild** — `set_active_sessions()`, `set_recent_connections()`, `set_window_visible()` now call `tray_icon.set_menu()` to rebuild the menu on state change. Previously the menu was static after creation.
- **macOS tray: active sessions in menu** — menu now shows "{N} Active Session(s)" as a disabled informational item when sessions are open.
- **X11 renderer fallback skipped on macOS** — `ensure_x11_renderer_fallback()` is now gated with `#[cfg(not(target_os = "macos"))]` to avoid unnecessary code execution on macOS.

### Known Limitations

- **VTE `spawn_async` broken on macOS** — Homebrew VTE build does not connect PTY to child process. Workaround: native PTY via `openpty()` + `Pty::foreign_sync()`.
- **Full session leader not possible without unsafe** — `process_group(0)` provides basic job control (Ctrl-C), but full `setsid` + `TIOCSCTTY` requires unsafe `pre_exec`. This means `Ctrl-Z` (suspend) may not work in all cases.
- **Wayland not available** — Build without `wayland-native` feature.
- **CSS parser warnings** — libadwaita 1.9 CSS uses features not yet supported by GTK4 4.22 CSS parser. Cosmetic only, no functional impact.
- **libsecret not available** — GNOME Keyring doesn't exist on macOS. Use macOS Keychain (native), KeePassXC, Bitwarden, 1Password, or Pass backends instead.

### Build Configuration for macOS

```bash
cargo build -p rustconn --no-default-features \
  --features "tray-macos,vnc-embedded,rdp-embedded,rdp-audio,spice-embedded"
```

Disabled features: `tray` (Linux D-Bus only), `wayland-native`

Enabled macOS-specific features: `tray-macos` (NSStatusItem via `tray-icon` crate)

### Dependencies (Homebrew)

```bash
brew install gtk4 libadwaita vte3 adwaita-icon-theme openssl@3 dbus gettext
```
