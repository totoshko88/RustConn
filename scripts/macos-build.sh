#!/usr/bin/env bash
# Build RustConn for macOS and produce the canonical self-contained .app bundle.
#
# Usage:
#   ./scripts/macos-build.sh                         # debug, unsigned bundle
#   ./scripts/macos-build.sh --release --adhoc      # local release bundle
#   ./scripts/macos-build.sh --release --sign-identity "Developer ID Application: ..."
#   ./scripts/macos-build.sh --launch --adhoc       # explicitly launch local bundle
#   ./scripts/macos-build.sh --print-features       # print canonical feature set

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"

APP_DIR="$PROJECT_DIR/dist/RustConn.app"
BUNDLE_ID="io.github.totoshko88.RustConn"
MACOS_FEATURES="tray-macos,system-keyring,vnc-embedded,rdp-embedded,gfx-h264,rdp-audio,rd-gateway,adw-1-8"
ENTITLEMENTS="$PROJECT_DIR/packaging/macos/RustConn.entitlements"
ICON_SVG="$PROJECT_DIR/rustconn/assets/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg"
VERSION="$(awk -F'"' '/^\[workspace\.package\]/{p=1} p&&/^version[[:space:]]*=/{print $2; exit}' Cargo.toml)"

RELEASE=false
CLEAN=false
LAUNCH=false
SIGN_MODE="none"
SIGN_IDENTITY="${MACOS_SIGN_IDENTITY:-}"
if [[ -n "$SIGN_IDENTITY" ]]; then
    SIGN_MODE="developer"
fi

info() { printf '\033[34m[info]\033[0m %s\n' "$*"; }
ok() { printf '\033[32m[ ok ]\033[0m %s\n' "$*"; }
fail() { printf '\033[31m[fail]\033[0m %s\n' "$*" >&2; exit 1; }
require_tool() { command -v "$1" >/dev/null 2>&1 || fail "Missing required tool: $1"; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        --release) RELEASE=true ;;
        --clean) CLEAN=true ;;
        --launch) LAUNCH=true ;;
        --no-launch) LAUNCH=false ;;
        --adhoc)
            [[ "$SIGN_MODE" != "developer" ]] || fail "--adhoc conflicts with Developer ID signing"
            SIGN_MODE="adhoc"
            ;;
        --no-sign)
            SIGN_MODE="none"
            SIGN_IDENTITY=""
            ;;
        --sign-identity)
            shift
            [[ $# -gt 0 ]] || fail "--sign-identity requires a value"
            [[ "$SIGN_MODE" != "adhoc" ]] || fail "--sign-identity conflicts with --adhoc"
            SIGN_MODE="developer"
            SIGN_IDENTITY="$1"
            ;;
        --sign-identity=*)
            [[ "$SIGN_MODE" != "adhoc" ]] || fail "--sign-identity conflicts with --adhoc"
            SIGN_MODE="developer"
            SIGN_IDENTITY="${1#*=}"
            ;;
        --print-features)
            printf '%s\n' "$MACOS_FEATURES"
            exit 0
            ;;
        -h|--help)
            sed -n '2,10p' "$0"
            exit 0
            ;;
        *) fail "Unknown option: $1" ;;
    esac
    shift
done

[[ "$(uname -s)" == "Darwin" ]] || fail "macOS bundle creation must run on macOS"
[[ -n "$VERSION" ]] || fail "Cannot read workspace version"
if $LAUNCH && [[ "$SIGN_MODE" == "none" ]]; then
    fail "Launching an unsigned relocated bundle is disabled; pass --adhoc for local use"
fi
if [[ "$SIGN_MODE" == "developer" ]]; then
    [[ -n "$SIGN_IDENTITY" ]] || fail "Developer ID signing requires an identity"
    [[ -f "$ENTITLEMENTS" ]] || fail "Missing entitlements: $ENTITLEMENTS"
fi

for tool in cargo brew otool install_name_tool codesign file iconutil rsvg-convert sips msgfmt awk sed grep cmp; do
    require_tool "$tool"
done

if $RELEASE; then
    PROFILE="release"
    TARGET_DIR="$PROJECT_DIR/target/release"
    CARGO_PROFILE_ARGS=(--release)
else
    PROFILE="debug"
    TARGET_DIR="$PROJECT_DIR/target/debug"
    CARGO_PROFILE_ARGS=()
fi

info "Building rustconn ($PROFILE) with features: $MACOS_FEATURES"
cargo build -p rustconn --no-default-features --features "$MACOS_FEATURES" "${CARGO_PROFILE_ARGS[@]}"
cargo build -p rustconn-cli --features full "${CARGO_PROFILE_ARGS[@]}"

if $CLEAN && [[ -d "$APP_DIR" ]]; then
    info "Removing existing bundle"
    rm -rf "$APP_DIR"
fi
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" \
         "$APP_DIR/Contents/Frameworks" \
         "$APP_DIR/Contents/Resources/bin" \
         "$APP_DIR/Contents/Resources/share/icons" \
         "$APP_DIR/Contents/Resources/share/glib-2.0/schemas"
cp "$TARGET_DIR/rustconn" "$APP_DIR/Contents/MacOS/rustconn"
cp "$TARGET_DIR/rustconn-cli" "$APP_DIR/Contents/MacOS/rustconn-cli"

# Build the native application icon. The render-and-validate logic lives in a
# shared script so the icon step cannot drift between this producer and the
# Homebrew formula, and so a broken render fails with a named file rather than
# an opaque "Invalid Iconset" from iconutil.
"$SCRIPT_DIR/make-iconset.sh" "$ICON_SVG" "$APP_DIR/Contents/Resources/RustConn.icns"

# Compile all translations; a broken catalog must fail packaging.
for catalog in "$PROJECT_DIR"/po/*.po; do
    language="$(basename "$catalog" .po)"
    locale_dir="$APP_DIR/Contents/Resources/locale/$language/LC_MESSAGES"
    mkdir -p "$locale_dir"
    msgfmt --check -o "$locale_dir/rustconn.mo" "$catalog"
done

# Bundle runtime data without assuming Intel or Apple Silicon Homebrew paths.
HOMEBREW_PREFIX="$(brew --prefix)"
GLIB_PREFIX="$(brew --prefix glib)"
ICON_PREFIX="$(brew --prefix adwaita-icon-theme)"
for theme in Adwaita hicolor; do
    source_theme="$ICON_PREFIX/share/icons/$theme"
    [[ -d "$source_theme" ]] || source_theme="$HOMEBREW_PREFIX/share/icons/$theme"
    [[ -d "$source_theme" ]] || fail "Missing $theme icon theme (brew install adwaita-icon-theme)"
    cp -RL "$source_theme" "$APP_DIR/Contents/Resources/share/icons/"
done
SCHEMAS="$GLIB_PREFIX/share/glib-2.0/schemas/gschemas.compiled"
if [[ ! -f "$SCHEMAS" ]]; then
    SCHEMAS="$HOMEBREW_PREFIX/share/glib-2.0/schemas/gschemas.compiled"
fi
[[ -f "$SCHEMAS" ]] || fail "Missing compiled GSettings schemas (brew install glib)"
cp "$SCHEMAS" "$APP_DIR/Contents/Resources/share/glib-2.0/schemas/"
mkdir -p "$APP_DIR/Contents/Resources/share/icons/hicolor/scalable/apps"
cp "$ICON_SVG" "$APP_DIR/Contents/Resources/share/icons/hicolor/scalable/apps/"

# Manual-terminal wrapper only. LaunchServices executes the native rustconn binary.
cat > "$APP_DIR/Contents/Resources/bin/rustconn-wrapper" <<'WRAPPER'
#!/bin/sh
CONTENTS="$(cd "$(dirname "$0")/../.." && pwd)"
export XDG_DATA_DIRS="$CONTENTS/Resources/share${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"
export GSETTINGS_SCHEMA_DIR="$CONTENTS/Resources/share/glib-2.0/schemas"
export LOCALEDIR="$CONTENTS/Resources/locale"
cd "$HOME"
exec "$CONTENTS/MacOS/rustconn" "$@"
WRAPPER
chmod 0755 "$APP_DIR/Contents/Resources/bin/rustconn-wrapper"

cat > "$APP_DIR/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>rustconn</string>
    <key>CFBundleIconFile</key><string>RustConn</string>
    <key>CFBundleIdentifier</key><string>${BUNDLE_ID}</string>
    <key>CFBundleName</key><string>RustConn</string>
    <key>CFBundleDisplayName</key><string>RustConn</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleVersion</key><string>${VERSION}</string>
    <key>CFBundleShortVersionString</key><string>${VERSION}</string>
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSDocumentsFolderUsageDescription</key>
    <string>RustConn needs access to import SSH configurations and connection files.</string>
    <key>NSAppleEventsUsageDescription</key>
    <string>RustConn needs to open URLs in your default browser.</string>
</dict>
</plist>
PLIST
plutil -lint "$APP_DIR/Contents/Info.plist" >/dev/null

is_system_dependency() {
    case "$1" in
        /System/Library/*|/usr/lib/*) return 0 ;;
        *) return 1 ;;
    esac
}

macho_dependencies() {
    otool -L "$1" | tail -n +2 | sed -E 's/^[[:space:]]+([^[:space:]]+).*/\1/'
}

resolve_dependency() {
    local source="$1" dependency="$2" candidate base suffix
    base="$(basename "$dependency")"
    case "$dependency" in
        /*)
            [[ -f "$dependency" ]] && printf '%s\n' "$dependency" && return 0
            ;;
        @loader_path/*)
            suffix="${dependency#@loader_path/}"
            candidate="$(dirname "$source")/$suffix"
            [[ -f "$candidate" ]] && printf '%s\n' "$candidate" && return 0
            ;;
        @executable_path/*)
            suffix="${dependency#@executable_path/}"
            candidate="$TARGET_DIR/$suffix"
            [[ -f "$candidate" ]] && printf '%s\n' "$candidate" && return 0
            ;;
        @rpath/*)
            for candidate in "$(dirname "$source")/$base" \
                             "$HOMEBREW_PREFIX/lib/$base" \
                             "$APP_DIR/Contents/Frameworks/$base"; do
                [[ -f "$candidate" ]] && printf '%s\n' "$candidate" && return 0
            done
            candidate="$(find "$HOMEBREW_PREFIX/opt" -path "*/lib/$base" -type f -print -quit 2>/dev/null || true)"
            [[ -n "$candidate" ]] && printf '%s\n' "$candidate" && return 0
            ;;
    esac
    return 1
}

QUEUE_SOURCE=("$TARGET_DIR/rustconn" "$TARGET_DIR/rustconn-cli")
QUEUE_DEST=("$APP_DIR/Contents/MacOS/rustconn" "$APP_DIR/Contents/MacOS/rustconn-cli")

enqueue_library() {
    local source="$1" base destination existing i
    base="$(basename "$source")"
    destination="$APP_DIR/Contents/Frameworks/$base"
    for ((i = 0; i < ${#QUEUE_DEST[@]}; i++)); do
        existing="${QUEUE_DEST[$i]}"
        if [[ "$existing" == "$destination" ]]; then
            cmp -s "$source" "${QUEUE_SOURCE[$i]}" || fail "Dylib basename collision: $source and ${QUEUE_SOURCE[$i]}"
            return 0
        fi
    done
    cp -L "$source" "$destination"
    chmod u+w "$destination"
    QUEUE_SOURCE+=("$source")
    QUEUE_DEST+=("$destination")
}

# OpenH264 is dlopen'd at runtime rather than represented in LC_LOAD_DYLIB, so
# it must be bundled explicitly. The canonical feature set enables gfx-h264, and
# rustconn-core looks for Contents/Frameworks/libopenh264.dylib first, so a
# missing library silently downgrades H.264 to non-AVC codecs. Fail instead.
OPENH264_PREFIX="$(brew --prefix openh264 2>/dev/null || true)"
OPENH264_DYLIB="$OPENH264_PREFIX/lib/libopenh264.dylib"
[[ -n "$OPENH264_PREFIX" && -f "$OPENH264_DYLIB" ]] || \
    fail "Missing OpenH264 for the gfx-h264 feature (brew install openh264)"
enqueue_library "$OPENH264_DYLIB"

index=0
while (( index < ${#QUEUE_SOURCE[@]} )); do
    source_macho="${QUEUE_SOURCE[$index]}"
    while IFS= read -r dependency; do
        [[ -n "$dependency" ]] || continue
        is_system_dependency "$dependency" && continue
        case "$dependency" in
            @rpath/*)
                [[ -f "$APP_DIR/Contents/Frameworks/$(basename "$dependency")" ]] && continue
                ;;
        esac
        resolved="$(resolve_dependency "$source_macho" "$dependency" || true)"
        [[ -n "$resolved" ]] || fail "Cannot resolve dependency $dependency required by $source_macho"
        enqueue_library "$resolved"
    done < <(macho_dependencies "$source_macho")
    ((index += 1))
done

# Remove existing signatures before changing Mach-O load commands.
for destination in "${QUEUE_DEST[@]}"; do
    if codesign -dv "$destination" >/dev/null 2>&1; then
        codesign --remove-signature "$destination"
    fi
done

for destination in "${QUEUE_DEST[@]}"; do
    chmod u+w "$destination"
    if [[ "$destination" == "$APP_DIR/Contents/Frameworks/"* ]]; then
        install_name_tool -id "@rpath/$(basename "$destination")" "$destination"
    fi
    while IFS= read -r dependency; do
        [[ -n "$dependency" ]] || continue
        is_system_dependency "$dependency" && continue
        base="$(basename "$dependency")"
        [[ -f "$APP_DIR/Contents/Frameworks/$base" ]] || fail "Unbundled dependency $dependency in $destination"
        [[ "$dependency" == "@rpath/$base" ]] || \
            install_name_tool -change "$dependency" "@rpath/$base" "$destination"
    done < <(macho_dependencies "$destination")

    if ! otool -l "$destination" | grep -A2 LC_RPATH | grep -q 'path @executable_path/../Frameworks'; then
        install_name_tool -add_rpath "@executable_path/../Frameworks" "$destination"
    fi
done

# Reject any remaining absolute non-system dependency before signing.
for destination in "${QUEUE_DEST[@]}"; do
    while IFS= read -r dependency; do
        [[ -n "$dependency" ]] || continue
        is_system_dependency "$dependency" && continue
        case "$dependency" in
            @rpath/*|@loader_path/*|@executable_path/*) ;;
            *) fail "Non-relocatable dependency remains in $destination: $dependency" ;;
        esac
    done < <(macho_dependencies "$destination")
done
ok "Bundled and relocated $((${#QUEUE_DEST[@]} - 2)) non-system dylibs"

sign_artifact() {
    local artifact="$1"
    if [[ "$SIGN_MODE" == "developer" ]]; then
        codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$artifact"
    else
        codesign --force --sign - "$artifact"
    fi
}

if [[ "$SIGN_MODE" != "none" ]]; then
    # Nested code must be signed inside-out. Signing the main executable first
    # makes codesign validate its still-unsigned sibling CLI and frameworks.
    for destination in "${QUEUE_DEST[@]}"; do
        if [[ "$destination" == "$APP_DIR/Contents/Frameworks/"* ]]; then
            sign_artifact "$destination"
        fi
    done
    sign_artifact "$APP_DIR/Contents/MacOS/rustconn-cli"
    sign_artifact "$APP_DIR/Contents/MacOS/rustconn"
    if [[ "$SIGN_MODE" == "developer" ]]; then
        codesign --force --options runtime --timestamp \
            --entitlements "$ENTITLEMENTS" --sign "$SIGN_IDENTITY" "$APP_DIR"
        ok "Signed with Developer ID and hardened runtime"
    else
        codesign --force --sign - "$APP_DIR"
        ok "Signed ad-hoc for explicit local use"
    fi
    codesign --verify --deep --strict --verbose=2 "$APP_DIR"
else
    info "Bundle left unsigned; use --adhoc locally or --sign-identity for distribution"
fi

ARCHITECTURES="$(lipo -archs "$APP_DIR/Contents/MacOS/rustconn")"
ok "Bundle ready: $APP_DIR ($PROFILE, v$VERSION, $ARCHITECTURES)"
printf 'Features: %s\n' "$MACOS_FEATURES"
printf 'CLI: %s/Contents/MacOS/rustconn-cli --help\n' "$APP_DIR"

if $LAUNCH; then
    info "Launching signed bundle"
    open "$APP_DIR"
fi
