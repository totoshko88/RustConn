#!/bin/bash
# Build RustConn.dmg for macOS distribution
# Usage: ./packaging/macos/build-dmg.sh [--release]
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
BUILD_TYPE="${1:---release}"
APP_NAME="RustConn"
APP_DIR="$PROJECT_DIR/dist/${APP_NAME}.app"
DMG_DIR="$PROJECT_DIR/dist"
VERSION=$(grep '^version' "$PROJECT_DIR/Cargo.toml" | head -1 | sed 's/.*"\(.*\)"/\1/')

echo "=== Building RustConn for macOS ==="

# 1. Build the binary
echo "Building binary ($BUILD_TYPE)..."
if [ "$BUILD_TYPE" = "--release" ]; then
    cargo build -p rustconn --release --no-default-features \
        --features "tray-macos,vnc-embedded,rdp-embedded,rdp-audio,spice-embedded"
    BINARY="$PROJECT_DIR/target/release/rustconn"
else
    cargo build -p rustconn --no-default-features \
        --features "tray-macos,vnc-embedded,rdp-embedded,rdp-audio,spice-embedded"
    BINARY="$PROJECT_DIR/target/debug/rustconn"
fi

# 2. Create .app bundle structure
echo "Creating app bundle..."
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

# 3. Copy binary
cp "$BINARY" "$APP_DIR/Contents/MacOS/rustconn"

# 4. Create icon
echo "Creating icon..."
ICONSET_DIR=$(mktemp -d)/RustConn.iconset
mkdir -p "$ICONSET_DIR"
SVG="$PROJECT_DIR/rustconn/assets/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg"

for size in 16 32 64 128 256 512 1024; do
    rsvg-convert -w $size -h $size "$SVG" -o "$ICONSET_DIR/icon_${size}.png"
done

cp "$ICONSET_DIR/icon_16.png" "$ICONSET_DIR/icon_16x16.png"
cp "$ICONSET_DIR/icon_32.png" "$ICONSET_DIR/icon_16x16@2x.png"
cp "$ICONSET_DIR/icon_32.png" "$ICONSET_DIR/icon_32x32.png"
cp "$ICONSET_DIR/icon_64.png" "$ICONSET_DIR/icon_32x32@2x.png"
cp "$ICONSET_DIR/icon_128.png" "$ICONSET_DIR/icon_128x128.png"
cp "$ICONSET_DIR/icon_256.png" "$ICONSET_DIR/icon_128x128@2x.png"
cp "$ICONSET_DIR/icon_256.png" "$ICONSET_DIR/icon_256x256.png"
cp "$ICONSET_DIR/icon_512.png" "$ICONSET_DIR/icon_256x256@2x.png"
cp "$ICONSET_DIR/icon_512.png" "$ICONSET_DIR/icon_512x512.png"
cp "$ICONSET_DIR/icon_1024.png" "$ICONSET_DIR/icon_512x512@2x.png"
rm -f "$ICONSET_DIR"/icon_*.png  # remove intermediate files

iconutil -c icns "$ICONSET_DIR" -o "$APP_DIR/Contents/Resources/RustConn.icns"

# 5. Compile locales
echo "Compiling locales..."
for f in "$PROJECT_DIR"/po/*.po; do
    lang=$(basename "$f" .po)
    mkdir -p "$APP_DIR/Contents/Resources/locale/${lang}/LC_MESSAGES"
    msgfmt -o "$APP_DIR/Contents/Resources/locale/${lang}/LC_MESSAGES/rustconn.mo" "$f" 2>/dev/null || true
done

# 6. Copy Adwaita icons (subset needed by the app)
echo "Bundling Adwaita icons..."
mkdir -p "$APP_DIR/Contents/Resources/share/icons"
cp -R /opt/homebrew/share/icons/Adwaita "$APP_DIR/Contents/Resources/share/icons/"
cp -R /opt/homebrew/share/icons/hicolor "$APP_DIR/Contents/Resources/share/icons/"

# 7. Copy GSettings schemas
mkdir -p "$APP_DIR/Contents/Resources/share/glib-2.0/schemas"
cp /opt/homebrew/share/glib-2.0/schemas/gschemas.compiled \
   "$APP_DIR/Contents/Resources/share/glib-2.0/schemas/"

# 8. Create wrapper script
cat > "$APP_DIR/Contents/MacOS/rustconn-wrapper" << 'EOF'
#!/bin/bash
DIR="$(cd "$(dirname "$0")/.." && pwd)"
export XDG_DATA_DIRS="$DIR/Resources/share:/opt/homebrew/share:/usr/local/share:/usr/share"
export GSETTINGS_SCHEMA_DIR="$DIR/Resources/share/glib-2.0/schemas"
export LOCALEDIR="$DIR/Resources/locale"
# Let GTK4 handle HiDPI scaling natively; override with GDK_DPI_SCALE env if needed.
exec "$DIR/MacOS/rustconn" "$@"
EOF
chmod +x "$APP_DIR/Contents/MacOS/rustconn-wrapper"

# 9. Create Info.plist
cat > "$APP_DIR/Contents/Info.plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>rustconn-wrapper</string>
    <key>CFBundleIconFile</key>
    <string>RustConn</string>
    <key>CFBundleIdentifier</key>
    <string>io.github.totoshko88.RustConn</string>
    <key>CFBundleName</key>
    <string>RustConn</string>
    <key>CFBundleDisplayName</key>
    <string>RustConn</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>NSAppleEventsUsageDescription</key>
    <string>RustConn needs to open URLs in your default browser.</string>
</dict>
</plist>
EOF

# 10. Ad-hoc code sign (prevents Gatekeeper quarantine issues during development)
echo "Code signing (ad-hoc)..."
codesign --force --deep --sign - "$APP_DIR" 2>/dev/null || true

# 11. Create DMG
echo "Creating DMG..."
mkdir -p "$DMG_DIR"
DMG_PATH="$DMG_DIR/RustConn-${VERSION}-macOS-arm64.dmg"
rm -f "$DMG_PATH"

# Create a temporary folder with .app and Applications symlink for drag-install UX
DMG_STAGING="$DMG_DIR/dmg-staging"
rm -rf "$DMG_STAGING"
mkdir -p "$DMG_STAGING"
cp -R "$APP_DIR" "$DMG_STAGING/"
ln -s /Applications "$DMG_STAGING/Applications"

hdiutil create -volname "RustConn" -srcfolder "$DMG_STAGING" \
    -ov -format UDZO "$DMG_PATH"
rm -rf "$DMG_STAGING"

echo ""
echo "=== Done ==="
echo "App bundle: $APP_DIR"
echo "DMG: $DMG_PATH"
echo ""
echo "Note: This DMG requires Homebrew GTK4/libadwaita/VTE libraries installed."
echo "Install dependencies: brew install gtk4 libadwaita vte3 adwaita-icon-theme"
