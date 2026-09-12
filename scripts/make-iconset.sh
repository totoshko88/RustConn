#!/usr/bin/env bash
# Render the RustConn application icon into a macOS .icns, resiliently.
#
# Usage:
#   ./scripts/make-iconset.sh <icon.svg> <output.icns>
#
# This script prefers a prebuilt .icns shipped with the source tree over
# generating one at build time. The prebuilt icon lives at
# packaging/macos/RustConn.icns and is used when:
#   1. It exists and is non-empty
#   2. The source SVG has not been modified since the prebuilt was created
#      (checked via modification time; if SVG is newer, regenerate)
#
# Fallback to iconutil generation happens when:
#   - The prebuilt is missing or empty
#   - The source SVG is newer than the prebuilt
#   - The FORCE_ICONUTIL environment variable is set
#
# Why prefer prebuilt: macOS 27's iconutil introduced a regression or
# compatibility change that rejects valid iconsets with "Invalid Iconset",
# breaking Homebrew builds. Shipping a prebuilt .icns avoids the dependency
# on iconutil entirely for release builds. See GitHub issue #323.
#
# The fallback path remains for development: if you update the SVG, delete
# the prebuilt (or set FORCE_ICONUTIL=1) to regenerate. Then commit the new
# .icns so downstream builds use it.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
PREBUILT_ICNS="$PROJECT_DIR/packaging/macos/RustConn.icns"

ICON_SVG="${1:-}"
OUTPUT_ICNS="${2:-}"

die() { printf '%s: %s\n' "$(basename "$0")" "$*" >&2; exit 1; }
info() { printf '%s: %s\n' "$(basename "$0")" "$*"; }

[[ -n "$ICON_SVG" && -n "$OUTPUT_ICNS" ]] || die "usage: make-iconset.sh <icon.svg> <output.icns>"
[[ -f "$ICON_SVG" ]] || die "source SVG not found: $ICON_SVG"

# Check if we can use the prebuilt .icns
use_prebuilt() {
    [[ -z "${FORCE_ICONUTIL:-}" ]] || return 1
    [[ -s "$PREBUILT_ICNS" ]] || return 1
    # If SVG is newer than prebuilt, regenerate
    [[ ! "$ICON_SVG" -nt "$PREBUILT_ICNS" ]] || return 1
    return 0
}

if use_prebuilt; then
    info "using prebuilt icon from $PREBUILT_ICNS"
    mkdir -p "$(dirname "$OUTPUT_ICNS")"
    cp "$PREBUILT_ICNS" "$OUTPUT_ICNS"
    exit 0
fi

info "generating icon from SVG (prebuilt not available or SVG is newer)"

for tool in rsvg-convert iconutil sips; do
    command -v "$tool" >/dev/null 2>&1 || die "missing required tool: $tool"
done

# Canonical Apple iconset members: "<name>:<pixels>". A Retina @2x entry is the
# same pixels as the next size up, which is why 32/256/512 each appear twice.
CANONICAL=(
    "icon_16x16:16"
    "icon_16x16@2x:32"
    "icon_32x32:32"
    "icon_32x32@2x:64"
    "icon_128x128:128"
    "icon_128x128@2x:256"
    "icon_256x256:256"
    "icon_256x256@2x:512"
    "icon_512x512:512"
    "icon_512x512@2x:1024"
)

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT
ICONSET="$WORK_DIR/RustConn.iconset"
mkdir -p "$ICONSET"

# Verify a PNG exists, is non-empty, and is exactly the expected square size.
# `sips` is the authority macOS itself uses, so a file that passes here is a
# file `iconutil` will accept.
verify_png() {
    local file="$1" expected="$2"
    [[ -s "$file" ]] || die "render produced an empty file: $(basename "$file")"
    local w h
    w="$(sips -g pixelWidth "$file" 2>/dev/null | awk '/pixelWidth:/ {print $2}')"
    h="$(sips -g pixelHeight "$file" 2>/dev/null | awk '/pixelHeight:/ {print $2}')"
    [[ "$w" == "$expected" && "$h" == "$expected" ]] \
        || die "wrong size for $(basename "$file"): got ${w:-?}x${h:-?}, expected ${expected}x${expected}"
}

for entry in "${CANONICAL[@]}"; do
    name="${entry%%:*}"
    px="${entry##*:}"
    out="$ICONSET/${name}.png"
    rsvg-convert -w "$px" -h "$px" "$ICON_SVG" -o "$out" \
        || die "rsvg-convert failed for ${name} (${px}px)"
    verify_png "$out" "$px"
done

# The .iconset now contains exactly the canonical members and nothing else.
mkdir -p "$(dirname "$OUTPUT_ICNS")"
iconutil -c icns "$ICONSET" -o "$OUTPUT_ICNS" \
    || die "iconutil failed to build $OUTPUT_ICNS from a validated iconset"

[[ -s "$OUTPUT_ICNS" ]] || die "iconutil reported success but produced no output: $OUTPUT_ICNS"
