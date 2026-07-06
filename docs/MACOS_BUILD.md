# Building and Running RustConn on macOS

## Prerequisites

### Rust 1.95+

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup update
```

### System Dependencies (Homebrew)

```bash
brew install gtk4 libadwaita vte3 adwaita-icon-theme \
    openssl@3 dbus gettext pkg-config
```

### Verify Installation

```bash
pkg-config --modversion gtk4          # 4.22+
pkg-config --modversion libadwaita-1  # 1.5+ (1.8+ recommended for full widget set)
pkg-config --modversion vte-2.91-gtk4 # 0.76+
```

> **Tip:** If libadwaita ≥ 1.8 is available (Homebrew ships 1.9+), build with `adw-1-8` feature
> for access to `AdwToggleGroup`, `AdwShortcutsDialog`, and other modern widgets.

---

## Building

### One-Command Build + Launch (Recommended)

```bash
./scripts/macos-build.sh              # debug build + .app bundle + launch
./scripts/macos-build.sh --release    # release build + .app bundle + launch
./scripts/macos-build.sh --no-launch  # build only, don't launch
./scripts/macos-build.sh --clean      # remove old bundle before building
```

The script handles everything: cargo build with correct features, `.app` bundle creation,
icon generation, locale compilation, Adwaita icons, ad-hoc code signing, and launch.

### Manual: Debug Build (fast compilation)

```bash
cargo build -p rustconn --no-default-features \
  --features "tray-macos,vnc-embedded,rdp-embedded,rdp-audio,adw-1-8"
```

### Manual: Release Build (optimized)

```bash
cargo build --release -p rustconn --no-default-features \
  --features "tray-macos,vnc-embedded,rdp-embedded,rdp-audio,adw-1-8"
```

### CLI Only

```bash
cargo build -p rustconn-cli
```

### Disabled Features on macOS

| Feature | Reason |
|---------|--------|
| `tray` | Requires D-Bus StatusNotifierItem (Linux only) |
| `wayland-native` | Wayland doesn't exist on macOS |
| `adw-1-8` | Optional; requires libadwaita ≥ 1.8 (Homebrew provides 1.9+) |

---

## Running (Development)

### From Terminal

```bash
XDG_DATA_DIRS="$HOME/.local/share:/opt/homebrew/share:/usr/local/share:/usr/share" \
GSETTINGS_SCHEMA_DIR="/opt/homebrew/share/glib-2.0/schemas" \
LOCALEDIR="$(pwd)/locale" \
RUST_LOG=info \
./target/debug/rustconn
```

> **Note:** When launched directly (not via `.app` bundle), macOS Dock will show a generic icon.
> For proper Dock icon, launch via `open RustConn.app`.

### Via .app Bundle (Recommended)

The `.app` bundle provides proper macOS session setup (Dock icon, fzf-completion, no Documents permission prompt):

```bash
open RustConn.app
```

### With Debug Logging

```bash
XDG_DATA_DIRS="$HOME/.local/share:/opt/homebrew/share:/usr/local/share:/usr/share" \
GSETTINGS_SCHEMA_DIR="/opt/homebrew/share/glib-2.0/schemas" \
RUST_LOG=debug \
./target/debug/rustconn
```

---

## Creating the .app Bundle

### Quick Development Bundle

```bash
# 1. Build
cargo build -p rustconn --no-default-features \
  --features "tray-macos,vnc-embedded,rdp-embedded,rdp-audio,adw-1-8"

# 2. Create bundle structure
mkdir -p RustConn.app/Contents/{MacOS,Resources}

# 3. Copy binary
cp target/debug/rustconn RustConn.app/Contents/MacOS/

# 4. Create icon
for size in 16 32 64 128 256 512 1024; do
  rsvg-convert -w $size -h $size \
    rustconn/assets/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg \
    -o /tmp/icon_${size}.png
done
mkdir -p /tmp/RustConn.iconset
cp /tmp/icon_16.png /tmp/RustConn.iconset/icon_16x16.png
cp /tmp/icon_32.png /tmp/RustConn.iconset/icon_16x16@2x.png
cp /tmp/icon_32.png /tmp/RustConn.iconset/icon_32x32.png
cp /tmp/icon_64.png /tmp/RustConn.iconset/icon_32x32@2x.png
cp /tmp/icon_128.png /tmp/RustConn.iconset/icon_128x128.png
cp /tmp/icon_256.png /tmp/RustConn.iconset/icon_128x128@2x.png
cp /tmp/icon_256.png /tmp/RustConn.iconset/icon_256x256.png
cp /tmp/icon_512.png /tmp/RustConn.iconset/icon_256x256@2x.png
cp /tmp/icon_512.png /tmp/RustConn.iconset/icon_512x512.png
cp /tmp/icon_1024.png /tmp/RustConn.iconset/icon_512x512@2x.png
iconutil -c icns /tmp/RustConn.iconset -o RustConn.app/Contents/Resources/RustConn.icns

# 5. Compile locales
for f in po/*.po; do
  lang=$(basename "$f" .po)
  mkdir -p "RustConn.app/Contents/Resources/locale/${lang}/LC_MESSAGES"
  msgfmt -o "RustConn.app/Contents/Resources/locale/${lang}/LC_MESSAGES/rustconn.mo" "$f"
done

# 6. Create wrapper script (kept for manual terminal launches)
cat > RustConn.app/Contents/MacOS/rustconn-wrapper << 'EOF'
#!/bin/bash
DIR="$(cd "$(dirname "$0")/.." && pwd)"
export XDG_DATA_DIRS="$DIR/Resources/share:/opt/homebrew/share:/usr/local/share:/usr/share"
export GSETTINGS_SCHEMA_DIR="/opt/homebrew/share/glib-2.0/schemas"
export LOCALEDIR="$DIR/Resources/locale"
export PATH="/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:$PATH"
cd "$HOME"
exec "$DIR/MacOS/rustconn" "$@"
EOF
chmod +x RustConn.app/Contents/MacOS/rustconn-wrapper

# 7. Create Info.plist (CFBundleExecutable = native binary, no wrapper)
#    The rustconn binary detects the bundle and configures i18n/icons/schemas
#    programmatically without re-exec, preserving the LaunchServices scene
#    required for NSStatusItem (tray icon).
cat > RustConn.app/Contents/Info.plist << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>rustconn</string>
    <key>CFBundleIconFile</key>
    <string>RustConn</string>
    <key>CFBundleIdentifier</key>
    <string>io.github.totoshko88.RustConn</string>
    <key>CFBundleName</key>
    <string>RustConn</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleVersion</key>
    <string>0.18.1</string>
    <key>CFBundleShortVersionString</key>
    <string>0.18.1</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>NSDocumentsFolderUsageDescription</key>
    <string>RustConn needs access to import SSH configs and connection files.</string>
    <key>NSAppleEventsUsageDescription</key>
    <string>RustConn needs to open URLs in your default browser.</string>
</dict>
</plist>
EOF

# 8. Launch
open RustConn.app
```

### DMG Distribution Build

```bash
./packaging/macos/build-dmg.sh --release
# Output: dist/RustConn-<VERSION>-macOS-$(uname -m).dmg
```

---

## Homebrew Tap Installation

### For Users (recommended)

The Homebrew formula installs RustConn with all required dependencies automatically:

```bash
# 1. Add the tap
brew tap totoshko88/rustconn

# 2. Install (builds from source with all dependencies)
brew install rustconn

# 3. Launch the .app bundle
open $(brew --prefix)/opt/rustconn/RustConn.app
```

This will automatically install all required runtime libraries (GTK4, libadwaita, VTE, Adwaita icons, etc.) via Homebrew dependencies.

### What Gets Installed

| Component | Location |
|-----------|----------|
| `rustconn` binary | `$(brew --prefix)/bin/rustconn` |
| `rustconn-cli` binary | `$(brew --prefix)/bin/rustconn-cli` |
| `.app` bundle | `$(brew --prefix)/opt/rustconn/RustConn.app` |
| Locales (16 languages) | `$(brew --prefix)/share/locale/*/LC_MESSAGES/rustconn.mo` |
| App icon | `$(brew --prefix)/share/icons/hicolor/scalable/apps/` |

### Optional: Add to Applications

To have RustConn appear in Launchpad / Applications:

```bash
ln -sf $(brew --prefix)/opt/rustconn/RustConn.app /Applications/RustConn.app
```

### Optional: CLI Tools for Secret Backends

RustConn can integrate with external password managers. Install the ones you use:

```bash
# KeePassXC (local database)
brew install --cask keepassxc

# Bitwarden CLI
brew install bitwarden-cli

# 1Password CLI
brew install --cask 1password-cli

# Pass (GPG-based)
brew install pass
```

### Updating

```bash
brew update
brew upgrade rustconn
```

### Uninstalling

```bash
brew uninstall rustconn
brew untap totoshko88/rustconn
rm -f /Applications/RustConn.app  # if symlinked
```

### Publishing a New Release (Maintainers)

1. Tag the release on GitHub: `git tag vX.Y.Z && git push --tags`
2. Get the archive SHA256:
   ```bash
   curl -sL https://github.com/totoshko88/RustConn/archive/refs/tags/vX.Y.Z.tar.gz | shasum -a 256
   ```
3. Update `url`, `sha256` in `packaging/macos/rustconn.rb`
4. Push to `homebrew-rustconn` tap repository
5. Verify: `brew update && brew upgrade rustconn`

---

## Troubleshooting

### Icons Missing

```bash
brew install adwaita-icon-theme
```

Or install the app icon manually:
```bash
mkdir -p ~/.local/share/icons/hicolor/scalable/apps/
cp rustconn/assets/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg \
   ~/.local/share/icons/hicolor/scalable/apps/
```

### Local Shell Empty (no prompt)

This is a known VTE issue on macOS. The native PTY workaround (`macos_pty.rs`) handles this automatically. If you still see an empty terminal:

1. Ensure you're running the latest build with macOS PTY support
2. Launch via `.app` bundle: `open RustConn.app`

### KeePassXC Not Detected

Ensure `keepassxc-cli` is accessible:
```bash
which keepassxc-cli
# Should show: /opt/homebrew/bin/keepassxc-cli
```

If installed via KeePassXC.app but not Homebrew:
```bash
# The app already checks /Applications/KeePassXC.app/Contents/MacOS/keepassxc-cli
```

### CSS Warnings in Console

```
Gtk-WARNING: Theme parser warning: gtk.css: Expected ';' at end of block
```

These are harmless — libadwaita 1.9 CSS uses features not yet supported by GTK4's CSS parser. No functional impact.

### Tray Icon Warning

```
Tray initialization thread exited without creating tray
```

Expected if built with the Linux `tray` feature instead of `tray-macos`. The Linux tray uses D-Bus StatusNotifierItem which doesn't exist on macOS. Build with `--features tray-macos` (not `tray`) to get native NSStatusItem menu bar icon.

### AWS SSM "session-manager-plugin not found"

The Session Manager plugin is not bundled with the AWS CLI on macOS. Install it separately:

```bash
brew install --cask session-manager-plugin
```

If installed via the official AWS installer instead of Homebrew, ensure `/usr/local/sessionmanagerplugin/bin/` is in PATH. RustConn adds this path automatically since v0.15.5.

### Window Too Large / DPI Issues

GTK4 handles HiDPI scaling natively on macOS. If the window appears too large or too small, override with:

```bash
export GDK_DPI_SCALE=0.75  # Try different values (default: let GTK4 decide)
```

### Permission Dialog (Documents Access)

macOS TCC asks for Documents access on first launch because RustConn scans for SSH configs (`~/.ssh/config`) and import sources. Grant access once — it won't ask again.

---

## Architecture Notes

### macOS-Specific Code

All macOS-specific code is gated with `#[cfg(target_os = "macos")]`:

| File | Purpose |
|------|---------|
| `rustconn/src/macos_pty.rs` | Native PTY spawn via `openpty()` + `Pty::foreign_sync()` |
| `rustconn/src/terminal/mod.rs` | Conditional: native PTY on macOS, VTE `spawn_async` on Linux |
| `rustconn/src/window/mod.rs` | `--login` flag for shell on macOS |
| `rustconn-core/src/cli_download/mod.rs` | Homebrew paths in `get_extended_path()` |
| `rustconn-core/src/secret/status.rs` | macOS paths for `keepassxc-cli` |
| `rustconn-core/src/secret/detection.rs` | Fallback path detection for macOS |
| `rustconn-core/src/rdp_client/rdpdr.rs` | `u64::from()` for cross-platform `statvfs` |

### Why VTE spawn_async Doesn't Work on macOS

VTE's `spawn_async` internally uses GLib's `g_spawn_async_with_pipes` which on macOS (quartz backend) doesn't properly connect the PTY master/slave pair to the child process. The child starts (PID exists) but its stdout never reaches VTE.

The workaround creates the PTY natively via `nix::pty::openpty()`, spawns the child with `std::process::Command` using the slave fd as stdio, then hands the master fd to VTE via `Pty::foreign_sync()`. VTE reads from the master fd and renders output normally.

### Linux Compatibility

All changes are backward-compatible with Linux:
- `#[cfg(target_os = "macos")]` blocks are skipped on Linux
- `#[cfg(not(target_os = "macos"))]` preserves original Linux behavior
- `u64::from()` on `statvfs` fields is a no-op on Linux (already `u64`)
- Added `nix` dependency to GUI crate (already used by `rustconn-core`)
