# Building and Releasing RustConn on macOS

## Prerequisites

RustConn requires macOS 13 or later, Rust 1.95+, Xcode command-line tools, and Homebrew.

```bash
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup update
brew install gtk4 libadwaita vte3 adwaita-icon-theme \
  openssl@3 dbus gettext pkg-config librsvg openh264
```

Verify the native libraries before building:

```bash
pkg-config --modversion gtk4
pkg-config --modversion libadwaita-1
pkg-config --modversion vte-2.91-gtk4
```

Homebrew currently provides libadwaita 1.8 or newer, which is required by the canonical feature profile.

## Canonical macOS Feature Profile

Every macOS build, package, and local audit uses this feature set:

```text
tray-macos,system-keyring,vnc-embedded,rdp-embedded,gfx-h264,rdp-audio,rd-gateway,adw-1-8
```

Print the authoritative value directly from the producer script:

```bash
./scripts/macos-build.sh --print-features
```

Linux-only `tray`, `wayland-native`, and `web-embedded` are intentionally excluded. SPICE continues to use an external viewer; the removed `spice-embedded` feature must not be added to macOS commands.

## Canonical Application Build

`scripts/macos-build.sh` is the only supported producer of `dist/RustConn.app`. The DMG script and local CI consume this bundle rather than recreating it independently.

```bash
# Debug, unsigned, no launch
./scripts/macos-build.sh

# Release, unsigned
./scripts/macos-build.sh --release --clean --no-launch

# Release, explicitly ad-hoc signed for local testing
./scripts/macos-build.sh --release --clean --no-launch --adhoc

# Build, ad-hoc sign, and launch
./scripts/macos-build.sh --release --clean --launch --adhoc
```

Unsigned output is the default. Launching an unsigned relocated bundle through the script is rejected; use `--adhoc` for local launch testing or a Developer ID identity for distribution.

The script builds both binaries:

```bash
cargo build -p rustconn --no-default-features \
  --features "tray-macos,system-keyring,vnc-embedded,rdp-embedded,gfx-h264,rdp-audio,rd-gateway,adw-1-8"
cargo build -p rustconn-cli --features full
```

For release binaries, add `--release` to both commands. Manual Cargo builds are useful for development, but only `scripts/macos-build.sh` creates a distributable bundle.

## Self-Contained Bundle Layout

The canonical producer writes:

```text
dist/RustConn.app/Contents/
├── Frameworks/                 # relocated non-system dylibs
├── MacOS/
│   ├── rustconn                # CFBundleExecutable
│   └── rustconn-cli
├── Resources/
│   ├── bin/rustconn-wrapper    # optional manual-terminal launcher
│   ├── locale/
│   ├── share/glib-2.0/schemas/
│   ├── share/icons/
│   └── RustConn.icns
└── Info.plist
```

The producer recursively discovers non-system Mach-O dependencies, copies them into `Contents/Frameworks`, rewrites their install names and references to `@rpath`, and adds `@executable_path/../Frameworks` to executables. It relocates the non-system Homebrew dylibs its binaries depend on (the exact count depends on the installed library versions) and fails if an absolute `/opt/homebrew` or `/usr/local` dependency remains.

`libopenh264.dylib` is bundled explicitly because it is loaded with `dlopen` at runtime rather than recorded as a load command. `rustconn-core` probes `Contents/Frameworks/libopenh264.dylib` first and falls back to Homebrew prefixes only for development runs of the bare `target/` binary, so the producer fails when OpenH264 is missing instead of silently shipping a bundle that degrades H.264 to non-AVC codecs.

Runtime data is resolved relative to the bundle. No Homebrew runtime installation is required by the resulting `.app`; Homebrew is only required on the build machine. LaunchServices executes `Contents/MacOS/rustconn`. The wrapper lives under `Contents/Resources/bin` so it does not interfere with nested-code signing and is only for manual terminal launches:

```bash
dist/RustConn.app/Contents/Resources/bin/rustconn-wrapper
```

## Signing

### Ad-Hoc Signing for Local Validation

```bash
./scripts/macos-build.sh --release --clean --no-launch --adhoc
codesign --verify --deep --strict --verbose=2 dist/RustConn.app
```

Ad-hoc signing is intentionally opt-in and does not establish Gatekeeper trust. `spctl` rejection of an ad-hoc artifact is expected.

### Developer ID Signing

List available identities:

```bash
security find-identity -v -p codesigning
```

Build with an explicit identity:

```bash
./scripts/macos-build.sh --release --clean --no-launch \
  --sign-identity "Developer ID Application: Example Org (TEAMID)"
```

Alternatively, set `MACOS_SIGN_IDENTITY`. The producer signs inside-out in this order: bundled frameworks, CLI executable, main executable, then the application bundle. Developer ID signatures use `packaging/macos/RustConn.entitlements`, hardened runtime, secure timestamps, and no silent ad-hoc fallback.

Verify the result:

```bash
codesign --verify --deep --strict --verbose=4 dist/RustConn.app
codesign -d --entitlements :- --verbose=4 dist/RustConn.app
spctl --assess --type execute --verbose=4 dist/RustConn.app
```

## DMG and Notarization

The DMG packager consumes the canonical bundle and writes `dist/RustConn-<version>-macOS-$(uname -m).dmg`, where `<version>` is the workspace version from `Cargo.toml`.

```bash
# Build and ad-hoc sign a local artifact
./packaging/macos/build-dmg.sh --adhoc

# Package an already-built ad-hoc app
./packaging/macos/build-dmg.sh --skip-build --adhoc
```

Before staging, the packager verifies the enclosed application. In Developer ID
mode it refuses to continue when the app is ad-hoc signed, signed by a different
identity, or missing the hardened runtime, so `--skip-build` cannot turn a local
bundle into a seemingly notarizable artifact.

```bash

# Developer ID distribution build
./packaging/macos/build-dmg.sh \
  --sign-identity "Developer ID Application: Example Org (TEAMID)"
```

Store notarization credentials once in the login keychain:

```bash
xcrun notarytool store-credentials rustconn-notary \
  --apple-id "developer@example.com" \
  --team-id "TEAMID" \
  --password "APP-SPECIFIC-PASSWORD"
```

Then build, sign, submit, staple, and validate in one command:

```bash
./packaging/macos/build-dmg.sh \
  --sign-identity "Developer ID Application: Example Org (TEAMID)" \
  --notary-profile rustconn-notary
```

`MACOS_SIGN_IDENTITY` and `MACOS_NOTARY_PROFILE` provide equivalent non-command-line configuration. Notarization is rejected unless Developer ID signing is active.

Inspect a finished DMG:

```bash
hdiutil attach dist/RustConn-<version>-macOS-$(uname -m).dmg
codesign --verify --deep --strict --verbose=4 /Volumes/RustConn/RustConn.app
spctl --assess --type execute --verbose=4 /Volumes/RustConn/RustConn.app
hdiutil detach /Volumes/RustConn
```

## Local-Only CI and Audit

No hosted workflow is required or modified for the macOS gate. Run it locally:

```bash
./scripts/macos-ci.sh
```

The gate checks formatting, warning-free Clippy with the canonical macOS features, the `rustconn-core`, `rustconn-cli` and GUI test suites, `cargo deny`, `cargo audit`, `cargo outdated`, a fresh ad-hoc release bundle, CLI smoke tests, plist validity, code-signature integrity, `@rpath`, the bundled OpenH264 library and its architecture, and absence of absolute Homebrew dylib references.

The audit steps require both tools locally:

```bash
cargo install cargo-audit cargo-outdated --locked
```

Accepted advisories are declared in `deny.toml` and `.cargo/audit.toml`. `RUSTSEC-2023-0071` (rsa Marvin Attack, reached only through IronRDP) is accepted there with its rationale and review trigger, so the audit passes without hiding new findings.

For focused iterations:

```bash
./scripts/macos-ci.sh --skip-tests
./scripts/macos-ci.sh --skip-bundle
```

The full release flow still requires a machine with the Developer ID certificate and a configured notarytool keychain profile. Intel and universal runtime behavior must be validated on an Intel host or with an explicit universal build; an Apple Silicon-only run proves only the arm64 artifact.

## Running During Development

A direct debug run can use Homebrew resources:

```bash
BREW_PREFIX="$(brew --prefix)"
XDG_DATA_DIRS="$HOME/.local/share:$BREW_PREFIX/share:/usr/local/share:/usr/share" \
GSETTINGS_SCHEMA_DIR="$(brew --prefix glib)/share/glib-2.0/schemas" \
RUST_LOG=info \
./target/debug/rustconn
```

For realistic runtime and Dock behavior, use the bundle:

```bash
open dist/RustConn.app
```

## Homebrew Formula

The repository formula is `packaging/macos/rustconn.rb`. Its tag-only source is a temporary pre-tag state that avoids a fabricated checksum; a Git tag is mutable, so the formula must not be published in that form. Before publishing to a tap, replace the source with the release archive and its measured SHA-256 (or pin the release commit as the Git revision):

```bash
curl -sL https://github.com/totoshko88/RustConn/archive/refs/tags/v<version>.tar.gz \
  | shasum -a 256
```

Never copy a checksum from another version. Test formula syntax and installation with:

```bash
ruby -c packaging/macos/rustconn.rb
brew install --build-from-source ./packaging/macos/rustconn.rb
```

The formula builds its own `.app` under `#{prefix}/RustConn.app`, and it must obey the same rule as the canonical producer: `CFBundleExecutable` names a real binary inside `Contents/MacOS`, never a wrapper that `exec`s one from the keg's `bin`. Re-exec destroys the bundle identity — generic Dock tile, wrong name, and no LaunchServices scene for `NSStatusItem`. Because that copy is not reached by `scripts/macos-build.sh`, verify the formula's bundle separately after changing it:

```bash
plutil -p "$(brew --prefix)/opt/rustconn/RustConn.app/Contents/Info.plist" | grep CFBundleExecutable
file "$(brew --prefix)/opt/rustconn/RustConn.app/Contents/MacOS/rustconn"   # must be Mach-O, not a script
```

## Troubleshooting

### Local Shell Is Empty

The macOS VTE path uses the isolated `rustconn-pty-sys` FFI helper to create a controlling terminal with `setsid` and `TIOCSCTTY`. Launch the canonical bundle and confirm it was built with the current workspace version.

### Native Keychain Is Unavailable

Build with `system-keyring`. macOS uses Security.framework directly for primary and auxiliary secrets; `secret-tool` and `libsecret-tools` are not required.

### Tray Icon Warning

Use `tray-macos`, not Linux `tray`. The macOS path creates a native `NSStatusItem` through `tray-icon` and `muda`.

### Dock Shows a Generic Terminal Tile

Expected for any launch with no bundle behind it, and not something the window icon can fix: macOS reads the Dock icon from the launched bundle's `Info.plist`, and `gtk_window_set_icon_name` is an X11/Wayland concept the GDK macOS backend ignores.

RustConn compensates by setting the tile image at runtime through `rustconn-dock-sys`, so a shell launch of `rustconn` shows the right icon from 0.20.4 onwards. Only the tile: the name under it, the Cmd-Tab entry and the application menu still come from the bundle, so use `open dist/RustConn.app` when those matter. Confirm the runtime path took effect with:

```bash
RUST_LOG=debug rustconn 2>&1 | grep -i 'dock tile'
```

`Set the Dock tile icon programmatically` means it applied; `Dock icon left to the bundle's CFBundleIconFile` means RustConn detected a real bundle and deliberately did nothing, which is correct inside `dist/RustConn.app`. A stale tile for a bundle you just rebuilt is LaunchServices caching rather than a packaging fault — `touch dist/RustConn.app` and relaunch.

### Missing Icons or Schemas During Development

```bash
brew install adwaita-icon-theme glib
```

The canonical `.app` already embeds these resources. Missing-resource errors in that bundle indicate a packaging failure and must not be worked around with a runtime Homebrew dependency.

### AWS SSM Plugin Is Missing

```bash
brew install --cask session-manager-plugin
```

RustConn also checks the official `/usr/local/sessionmanagerplugin/bin` installation path.

### Sluggish Interface Inside a macOS Virtual Machine

Apple's paravirtualised GPU gives a macOS guest Metal but no accelerated OpenGL, and Homebrew builds `gtk4` with `-Dvulkan=disabled`. GSK's GL renderer therefore falls back to software inside a guest, which shows up as input lag, late frames and stuttering scroll while a core sits busy ([#274](https://github.com/totoshko88/RustConn/issues/274)).

RustConn detects this itself: on macOS the automatic renderer choice asks `sysctl -n kern.hv_vmm_present` and switches to the Cairo renderer when the answer is `1`. The log line naming the choice appears at `info` level:

```
Selected GSK renderer renderer="cairo" reason="guest VM: paravirtualised GPU has no accelerated OpenGL (#274)"
```

To check what the guest actually offers, and to confirm the diagnosis on a machine that behaves differently:

```bash
sysctl -n kern.hv_vmm_present          # 1 in a guest, 0 on bare metal
GSK_RENDERER=help rustconn             # which renderers this GTK build has
GDK_DEBUG=opengl rustconn              # which GL renderer the guest hands out
```

`Settings ▸ Interface ▸ Rendering` overrides the automatic choice in either direction, and an explicit `GSK_RENDERER` in the environment overrides both. Note that `GDK_SCALE` is an X11-only variable — the macOS backend takes its scale factor from `NSWindow.backingScaleFactor`, so setting it in a wrapper script does nothing.

## Architecture Notes

All macOS-specific Rust paths are target-gated. Key areas are:

| File | Purpose |
|------|---------|
| `rustconn-pty-sys/` | Isolated PTY and controlling-terminal FFI |
| `rustconn-core/src/secret/macos_keychain.rs` | Security.framework secret backend |
| `rustconn-core/src/secret/keyring.rs` | Native auxiliary Keychain delegation on macOS |
| `rustconn-core/src/cli_download/` | Homebrew and application search paths |
| `scripts/macos-build.sh` | Canonical self-contained `.app` producer and signer |
| `packaging/macos/build-dmg.sh` | DMG packaging and notarization |
| `scripts/macos-ci.sh` | Local-only quality and portability gate |

Linux behavior remains behind the existing non-macOS paths. The macOS producer deliberately excludes Wayland and WebKitGTK features while preserving embedded RDP/VNC, GFX/H.264, audio, RD Gateway, and native Keychain support.
