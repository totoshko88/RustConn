# Building and Running RustConn

## Prerequisites

Rust 1.95+ ([rustup.rs](https://rustup.rs/)):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup update
```

System dependencies:

**Ubuntu/Debian:**
```bash
sudo apt install build-essential libgtk-4-dev libvte-2.91-gtk4-dev \
    libadwaita-1-dev libdbus-1-dev libssl-dev pkg-config libasound2-dev \
    clang cmake gettext
```

**Fedora:**
```bash
sudo dnf install gcc gtk4-devel vte291-gtk4-devel libadwaita-devel \
    dbus-devel openssl-devel alsa-lib-devel clang cmake gettext
```

**openSUSE:**
```bash
sudo zypper install gcc gtk4-devel vte-devel libadwaita-devel \
    dbus-1-devel libopenssl-devel alsa-devel clang cmake gettext-tools
```

**Arch Linux:**
```bash
sudo pacman -S base-devel gtk4 vte4 libadwaita dbus openssl alsa-lib \
    clang cmake gettext
```

---

## Workspace Structure

| Crate | Type | Binary |
|-------|------|--------|
| `rustconn` | GTK4 GUI | `target/*/rustconn` |
| `rustconn-cli` | CLI | `target/*/rustconn-cli` |
| `rustconn-core` | Library | — (used by both binaries) |

---

## Building Individual Crates

### GUI Application (`rustconn`)

```bash
# Debug (fast compilation, slow execution)
cargo build -p rustconn

# Release (slow compilation, optimized execution)
cargo build --release -p rustconn
```

Binary: `target/debug/rustconn` or `target/release/rustconn`.

### CLI (`rustconn-cli`)

```bash
# Minimal/headless build
cargo build -p rustconn-cli
cargo build --release -p rustconn-cli

# Full CLI with desktop/client-launch and secret-management commands
cargo build -p rustconn-cli --features full
```

Binary: `target/debug/rustconn-cli` or `target/release/rustconn-cli`.

### Library (`rustconn-core`)

```bash
cargo build -p rustconn-core
```

### Everything at Once

```bash
cargo build                # debug, all crates
cargo build --release      # release, all crates
```

---

## Feature Flags

### rustconn (GUI)

| Flag | Default | Description |
|------|:-------:|-------------|
| `tray` | ✓ | System tray icon (ksni + resvg) |
| `system-keyring` | ✓ | Host keyring integration via `rustconn-core/system-keyring` |
| `vnc-embedded` | ✓ | Embedded VNC client (vnc-rs) |
| `rdp-embedded` | ✓ | Embedded RDP client (IronRDP) |
| `rdp-audio` | ✓ | RDP session audio (cpal); enables `rdp-embedded` |
| `rd-gateway` | ✓ | Native RD Gateway tunneling for embedded RDP |
| `wayland-native` | ✓ | Wayland surface support (gdk4-wayland) |
| `tray-macos` | — | macOS tray icon support |
| `adw-1-6` | — | libadwaita 1.6+ (AdwSpinner, CSS variables) |
| `adw-1-7` | — | libadwaita 1.7+ (AdwWrapBox); enables `adw-1-6` |
| `adw-1-8` | — | libadwaita 1.8+ (AdwShortcutsDialog); enables `adw-1-7` |

Examples:

```bash
# Minimal build without embedded clients or tray
cargo build -p rustconn --no-default-features

# SSH/Telnet/Serial (VTE) + tray only
cargo build -p rustconn --no-default-features --features tray

# Full build with libadwaita 1.8 (GNOME 49+)
cargo build --release -p rustconn --features adw-1-8

# Full build with libadwaita 1.7 (GNOME 48+)
cargo build --release -p rustconn --features adw-1-7
```

### rustconn-core (library)

`rustconn-core` defaults to a headless domain kernel (`default = []`). Enable
integration features only from crates that need host keyrings or embedded
client runtimes.

| Flag | Default | Description |
|------|:-------:|-------------|
| `system-keyring` | — | Host keyring integration (`oo7` on Linux/BSD, macOS Keychain on macOS) |
| `vnc-embedded` | — | vnc-rs runtime for embedded VNC sessions |
| `rdp-embedded` | — | IronRDP runtime for embedded RDP sessions |
| `gfx-h264` | — | RDP EGFX/H.264 pipeline; enables `rdp-embedded` |
| `rd-gateway` | — | Native RD Gateway tunneling; enables `rdp-embedded` |

Examples:

```bash
# Headless domain kernel
cargo build -p rustconn-core --no-default-features

# Core with embedded clients and system keyring integration
cargo build -p rustconn-core --no-default-features \
  --features "system-keyring vnc-embedded rdp-embedded rd-gateway"
```

### rustconn-cli

`rustconn-cli` defaults to the minimal headless management path. Desktop-adjacent
commands are opt-in.

| Flag | Default | Description |
|------|:-------:|-------------|
| `client-launch` | — | External client/file-manager launch commands such as `connect` |
| `secret-management` | — | `secret` subcommands and `rustconn-core/system-keyring` |
| `desktop-integration` | — | Convenience group: `client-launch` + `secret-management` |
| `full` | — | Full CLI feature set; currently enables `desktop-integration` |

Examples:

```bash
# Minimal/headless CLI
cargo build -p rustconn-cli --no-default-features

# Full CLI including optional desktop/client-launch paths
cargo build -p rustconn-cli --features full
```

---

## Running

### GUI

```bash
# After cargo build
./target/debug/rustconn

# Or directly
cargo run -p rustconn

# Release
cargo run --release -p rustconn
```

### CLI

```bash
cargo run -p rustconn-cli -- --help
cargo run -p rustconn-cli -- list
cargo run -p rustconn-cli -- --verbose list
```

---

## Log Levels (Debugging)

RustConn uses `tracing` + `tracing-subscriber` with filtering via the `RUST_LOG` environment variable.

### GUI (`rustconn`)

```bash
# Errors only (default without RUST_LOG)
./target/debug/rustconn

# Info — connections, sessions, key events
RUST_LOG=info ./target/debug/rustconn

# Debug — detailed protocol, configuration, and secret resolution info
RUST_LOG=debug ./target/debug/rustconn

# Trace — maximum detail (including every RDP/VNC packet)
RUST_LOG=trace ./target/debug/rustconn

# Per-module filter — RDP client at trace, everything else at info
RUST_LOG=info,rustconn_core::rdp_client=trace ./target/debug/rustconn

# Per-module filter — secrets only at debug
RUST_LOG=info,rustconn_core::secret=debug ./target/debug/rustconn

# Per-module filter — import only at debug
RUST_LOG=info,rustconn_core::import=debug ./target/debug/rustconn

# Suppress zbus noise (already suppressed by default, shown for reference)
RUST_LOG=debug,zbus=warn ./target/debug/rustconn
```

### CLI (`rustconn-cli`)

The CLI has built-in `-v` / `-q` flags:

```bash
# Warn (default)
rustconn-cli list

# Info
rustconn-cli -v list

# Debug
rustconn-cli -vv list

# Trace
rustconn-cli -vvv list

# Quiet mode (errors only)
rustconn-cli -q list
```

`RUST_LOG` takes precedence over `-v`/`-q`:

```bash
RUST_LOG=trace rustconn-cli list
```

### Levels and What They Show

| Level | Output |
|-------|--------|
| `error` | Critical errors, connection failures |
| `warn` | Warnings, fallback to external client, missing dependencies |
| `info` | Connection open/close, import/export, key actions |
| `debug` | Connection parameters, configuration, secret resolution, client discovery |
| `trace` | Every network packet, internal FSM state, full data flow |

---

## Tests

```bash
# All tests (rustconn-core, ~2 min due to argon2)
cargo test -p rustconn-core

# Property tests only
cargo test -p rustconn-core --test property_tests

# Integration tests only
cargo test -p rustconn-core --test integration_tests

# CLI tests
cargo test -p rustconn-cli

# Specific test
cargo test -p rustconn-core --test property_tests -- test_name

# Benchmarks
cargo bench -p rustconn-core
```

---

## Linting and Formatting

```bash
# Clippy (zero warnings required)
cargo clippy --all-targets

# Core only
cargo clippy -p rustconn-core --all-targets

# Format check
cargo fmt --check

# Auto-format
cargo fmt
```

---

## Flatpak (Local Build)

### Prerequisites

```bash
# Flatpak SDK and runtime
flatpak install flathub org.gnome.Sdk//50 org.gnome.Platform//50
flatpak install flathub org.freedesktop.Sdk.Extension.rust-stable//24.08
```

### Generate cargo-sources.json

The generator needs `aiohttp`, `PyYAML` and `tomlkit`. The script declares them
as PEP 723 inline metadata, so [`uv`](https://docs.astral.sh/uv/) installs them
into a throwaway environment automatically — nothing to install globally:

```bash
uv run packaging/flatpak/flatpak-cargo-generator.py \
    Cargo.lock \
    -o packaging/flatpak/cargo-sources.json
```

Without `uv`, install the dependencies first, then use plain `python3`:

```bash
pip install aiohttp PyYAML tomlkit
python3 packaging/flatpak/flatpak-cargo-generator.py \
    Cargo.lock \
    -o packaging/flatpak/cargo-sources.json
```

### Build

```bash
flatpak-builder --user --install --force-clean \
    build-dir \
    packaging/flatpak/io.github.totoshko88.RustConn.local.yml
```

### Run

```bash
flatpak run io.github.totoshko88.RustConn
```

### Run with Logging

```bash
flatpak run --env=RUST_LOG=debug io.github.totoshko88.RustConn
flatpak run --env=RUST_LOG=trace io.github.totoshko88.RustConn
```

---

## Smart Cards / PKCS#11 (YubiKey) in Sandboxed Builds

RustConn's `PKCS11Provider` option just passes `-o PKCS11Provider=<path>` to the
external `ssh` binary. OpenSSH then `dlopen()`s that module and talks to the
`pcscd` smart-card daemon. Inside a Flatpak/Snap sandbox neither the host module
(e.g. `/usr/lib64/libykcs11.so.2`) nor the `pcscd` socket is available, and the
GNOME runtime ships no `libpcsclite`, so loading the host module fails. Bundling
a PKCS#11 stack into the package is heavy for a rare use case, so the
recommended approach is to let the **host** `ssh-agent` do the smart-card work:

```bash
# On the HOST (not in the sandbox), load the YubiKey/PIV keys into the agent:
ssh-add -s /usr/lib64/libykcs11.so.2   # or your distro's opensc-pkcs11.so
ssh-add -L                             # verify the token key is listed
```

The Flatpak manifest already grants `--socket=ssh-auth`, so the sandboxed app
authenticates through the host agent — no `PKCS11Provider`, no bundled modules,
no overrides needed. Leave the connection's PKCS#11 provider field empty and
enable agent-based auth.

> Snap (strict confinement) cannot reach the host agent the same way and would
> require a packaged `pcscd` plug plus a bundled module; use the Flatpak or a
> native (`.deb`/AppImage/OBS) build for smart-card auth instead.

---

## Installing the Desktop File (From Source)

```bash
cargo build --release -p rustconn -p rustconn-cli
./install-desktop.sh
```

Installs the binaries (to `~/.local/bin`), icon, `.desktop` file, MIME types, and
locales to `~/.local/`. Ensure `~/.local/bin` is on your `PATH`.

> The script installs `target/release/rustconn` and `rustconn-cli` if present. To
> use a custom prefix: `PREFIX=/usr/local sudo ./install-desktop.sh`.

---

## Compiling Locales Manually

```bash
# Update .pot template
bash po/update-pot.sh

# Compile .mo for a specific language
msgfmt -o po/uk.mo po/uk.po

# Compile all languages
for f in po/*.po; do
    lang=$(basename "$f" .po)
    msgfmt -o "po/${lang}.mo" "$f"
done
```

---

## Notes for Downstream Packagers

RustConn's test suite takes approximately **2 minutes** on modern hardware due to
argon2-based property tests (deliberately slow — they verify password hashing
round-trips at realistic cost). All tests run in CI on every commit and release
tag, so **running tests during package build is unnecessary and not recommended**.

### Recommended build command

```bash
cargo build --release
```

That is sufficient. Do **not** add `cargo test` to your `check()` / build phase
unless you specifically want to re-verify the test suite locally.

### Why tests are slow

| Test group | Time | Reason |
|------------|------|--------|
| `property_tests` (argon2) | ~90 s | Real argon2id KDF rounds (not mocked) |
| `integration_tests` | ~20 s | Full round-trip protocol tests |
| Unit tests | ~5 s | Fast |

### CI guarantees

Every release tag published on GitHub has passed the full matrix:
- `cargo fmt --check`
- `cargo clippy --all-targets` (zero warnings)
- `cargo test --workspace`

on Ubuntu (latest), Fedora, and Arch runners. There is no benefit in repeating
this during downstream packaging.

---

## Quick Reference

```bash
# Build everything in release
cargo build --release

# Build with adw-1-8 and run with debug logs
cargo build --release -p rustconn --features adw-1-8
RUST_LOG=debug ./target/release/rustconn

# CLI with trace logs
cargo run -p rustconn-cli -- -vvv list

# Full pre-commit check
cargo fmt --check
cargo clippy --all-targets
cargo test -p rustconn-core
```
