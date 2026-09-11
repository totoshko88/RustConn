#!/usr/bin/env bash
# Render the RustConn application icon into a macOS .icns, resiliently.
#
# Usage:
#   ./scripts/make-iconset.sh <icon.svg> <output.icns>
#
# Why this exists as its own script rather than inline in the build:
# `iconutil -c icns` reports only "Invalid Iconset" and nothing else when any
# member PNG is missing, zero-length, or the wrong pixel size. In the Homebrew
# build sandbox that failure was intermittent and impossible to diagnose from
# the log, because `rsvg-convert` was invoked through `system` with no check
# that it actually produced a well-formed file. This script renders each icon
# directly under its canonical Apple name, verifies every PNG is non-empty and
# exactly the pixel size its name promises, and only then runs `iconutil` — so a
# broken render fails here, loudly, naming the offending file, instead of
# surfacing as an opaque `iconutil` error one step later. Both the canonical
# producer (scripts/macos-build.sh) and the Homebrew formula call it, so the
# icon step cannot drift between the two.

set -euo pipefail

ICON_SVG="${1:-}"
OUTPUT_ICNS="${2:-}"

die() { printf '%s: %s\n' "$(basename "$0")" "$*" >&2; exit 1; }

[[ -n "$ICON_SVG" && -n "$OUTPUT_ICNS" ]] || die "usage: make-iconset.sh <icon.svg> <output.icns>"
[[ -f "$ICON_SVG" ]] || die "source SVG not found: $ICON_SVG"

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
