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
cargo build -p rustconn-cli
cargo build --release -p rustconn-cli
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
| `vnc-embedded` | ✓ | Embedded VNC client (vnc-rs) |
| `rdp-embedded` | ✓ | Embedded RDP client (IronRDP) |
| `rdp-audio` | ✓ | RDP session audio (cpal); enables `rdp-embedded` |
| `spice-embedded` | ✓ | Embedded SPICE client |
| `wayland-native` | ✓ | Wayland surface support (gdk4-wayland) |
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

| Flag | Default | Description |
|------|:-------:|-------------|
| `vnc-embedded` | ✓ | vnc-rs |
| `rdp-embedded` | ✓ | IronRDP |
| `spice-embedded` | ✓ | spice-client |

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
