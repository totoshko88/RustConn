#!/usr/bin/env bash
# Render the RustConn application icon into a macOS .icns, resiliently.
#
# Usage:
#   ./scripts/make-iconset.sh <icon.svg> <output.icns>
#
# This script prefers the prebuilt .icns shipped in the source tree at
# packaging/macos/RustConn.icns, and only renders one when it has to. macOS 27's
# `iconutil` rejects valid iconsets with "Invalid Iconset", which broke Homebrew
# builds even though every member PNG was present, correctly sized and accepted
# by `sips`. Shipping the .icns keeps `iconutil` off the build path. (Issue #323)
#
# Freshness is tracked by the SVG's content hash in
# packaging/macos/RustConn.icns.svghash, never by modification time. Git records
# no mtimes: on a fresh `git clone` every file is stamped at checkout, in an order
# nobody controls, so an `-nt` test decides "the SVG is newer" at random and sends
# an arbitrary subset of builds down the path this script exists to avoid. A
# content hash gives the same answer in a clone, in a tarball and in a dirty
# working tree.
#
# A stale hash is a *warning*, not a failure. A slightly outdated icon is
# cosmetic; a build that stops because `iconutil` is broken is not, and that
# trade-off is the whole point of the prebuilt. Use `--check` in CI, where a
# mismatch can fail loudly without breaking anyone's install:
#
#   ./scripts/make-iconset.sh --check <icon.svg>
#
# The only route to `iconutil` is FORCE_ICONUTIL=1, which is a development
# action: after editing the SVG, run the canonical producer with it set and the
# script refreshes the prebuilt and its hash in place, ready to commit.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
PREBUILT_ICNS="$PROJECT_DIR/packaging/macos/RustConn.icns"
PREBUILT_HASH="$PREBUILT_ICNS.svghash"

die() { printf '%s: %s\n' "$(basename "$0")" "$*" >&2; exit 1; }
info() { printf '%s: %s\n' "$(basename "$0")" "$*"; }
warn() { printf '%s: warning: %s\n' "$(basename "$0")" "$*" >&2; }

# `shasum` is what macOS ships, `sha256sum` is what Linux ships, and CI runs
# `--check` on both.
sha256_of() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d' ' -f1
    elif command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        return 1
    fi
}

# Prints nothing and returns non-zero when the answer is not knowable, which the
# callers treat as "unknown", never as "stale".
prebuilt_is_current() {
    local recorded actual
    [[ -r "$PREBUILT_HASH" ]] || return 1
    recorded="$(tr -d '[:space:]' < "$PREBUILT_HASH")"
    actual="$(sha256_of "$ICON_SVG")" || return 1
    [[ "$recorded" == "$actual" ]]
}

# CI gate: assert the committed .icns was built from the committed SVG.
if [[ "${1:-}" == "--check" ]]; then
    ICON_SVG="${2:-}"
    [[ -n "$ICON_SVG" ]] || die "usage: make-iconset.sh --check <icon.svg>"
    [[ -f "$ICON_SVG" ]] || die "source SVG not found: $ICON_SVG"
    [[ -s "$PREBUILT_ICNS" ]] || die "prebuilt icon missing or empty: $PREBUILT_ICNS"
    [[ -r "$PREBUILT_HASH" ]] || die "prebuilt icon has no recorded SVG hash: $PREBUILT_HASH"
    if prebuilt_is_current; then
        info "prebuilt icon matches $ICON_SVG"
        exit 0
    fi
    die "prebuilt icon is stale: regenerate with FORCE_ICONUTIL=1 on macOS and commit
       $PREBUILT_ICNS and $PREBUILT_HASH"
fi

ICON_SVG="${1:-}"
OUTPUT_ICNS="${2:-}"

[[ -n "$ICON_SVG" && -n "$OUTPUT_ICNS" ]] || die "usage: make-iconset.sh <icon.svg> <output.icns>"
[[ -f "$ICON_SVG" ]] || die "source SVG not found: $ICON_SVG"

if [[ -z "${FORCE_ICONUTIL:-}" && -s "$PREBUILT_ICNS" ]]; then
    if ! prebuilt_is_current; then
        warn "prebuilt icon may not match $ICON_SVG; using it anyway, because a stale
         icon is cosmetic and iconutil is unreliable on macOS 27 (issue #323).
         Regenerate with FORCE_ICONUTIL=1 and commit the result."
    fi
    info "using prebuilt icon from $PREBUILT_ICNS"
    mkdir -p "$(dirname "$OUTPUT_ICNS")"
    cp "$PREBUILT_ICNS" "$OUTPUT_ICNS"
    exit 0
fi

if [[ -n "${FORCE_ICONUTIL:-}" ]]; then
    info "FORCE_ICONUTIL set — rendering from SVG and refreshing the prebuilt"
else
    info "no prebuilt icon at $PREBUILT_ICNS — rendering from SVG"
fi

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

# Refresh the committed prebuilt so the developer who just re-rendered has
# something to commit. Doing it here is what closes the loop: told to "commit the
# new .icns", the previous version left the copy step to be remembered, and a
# forgotten copy silently keeps shipping the old icon.
if [[ -n "${FORCE_ICONUTIL:-}" ]]; then
    mkdir -p "$(dirname "$PREBUILT_ICNS")"
    cp "$OUTPUT_ICNS" "$PREBUILT_ICNS"
    if svg_hash="$(sha256_of "$ICON_SVG")"; then
        printf '%s\n' "$svg_hash" > "$PREBUILT_HASH"
        info "refreshed $PREBUILT_ICNS and $PREBUILT_HASH — commit both"
    else
        warn "refreshed $PREBUILT_ICNS but could not hash the SVG: no shasum or sha256sum"
    fi
fi
