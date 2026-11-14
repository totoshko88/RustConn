#!/usr/bin/env bash
# Set the RustConn git tag in the Flathub manifest — the one line release.sh's
# packaging gate reports as out of sync every release:
#
#   [warn] Version X.Y.Z not found in packaging/flathub/io.github.totoshko88.RustConn.yml
#
# MAINTAINER-RUN. Flathub's generative AI policy keeps agents out of
# packaging/flathub/ (packaging/AGENTS.md, "Flathub is maintainer-only"), so an
# agent hands this command over and does not run it against the real tree. The
# test, scripts/test-flathub-tag.sh, runs it on a temporary copy.
#
# What it changes, and nothing else:
#   * the single `tag: vX.Y.Z` line of the app's git source;
#   * a `commit:` line in that same source entry, only if the manifest has one
#     AND the tag v<version> exists locally (resolved with
#     `git rev-parse v<version>^{commit}`). Otherwise the commit line is left
#     alone and the script says so. It never invents a hash. Today the manifest
#     is tag-only on purpose, so this branch normally does nothing.
#
# The substitution is the same line-anchored rule scripts/bump-version.sh uses
# for this file, and the result is checked against the pattern release.sh greps
# for (read out of PKG_FILES/PKG_PATS at runtime), so the gate and the writer
# cannot disagree silently.
#
# Usage:
#   scripts/flathub-tag.sh X.Y.Z             write the tag line, print the diff
#   scripts/flathub-tag.sh X.Y.Z --dry-run   print the diff, write nothing
#   scripts/flathub-tag.sh X.Y.Z --force     allow X.Y.Z != Cargo.toml version
#   scripts/flathub-tag.sh X.Y.Z --manifest PATH   edit another copy (tests)
#
# Idempotent: a manifest already at vX.Y.Z is left untouched.
# No network. No git command that changes state: no add, commit, tag or push.
#
# Exit codes:
#   0 — written, already current, or dry run shown
#   1 — the manifest, Cargo.toml or release.sh is not in the expected shape
#   2 — bad arguments

set -euo pipefail

die() {
    printf 'flathub-tag: %s\n' "$1" >&2
    exit "${2:-1}"
}

usage() {
    awk 'NR > 1 { if ($0 !~ /^#/) exit; sub(/^# ?/, ""); print }' "$0"
}

# ── Args ─────────────────────────────────────────────────────────────────────
NEW=""
DRY_RUN=0
FORCE=0
MANIFEST=""
while [ "$#" -gt 0 ]; do
    case "$1" in
    --dry-run | -n) DRY_RUN=1 ;;
    --force) FORCE=1 ;;
    --manifest)
        [ "$#" -ge 2 ] || die '--manifest needs a path' 2
        MANIFEST="$2"
        shift
        ;;
    -h | --help)
        usage
        exit 0
        ;;
    -*) die "unknown option: $1 (see --help)" 2 ;;
    *)
        [ -z "$NEW" ] || die 'give exactly one version' 2
        NEW="$1"
        ;;
    esac
    shift
done

[ -n "$NEW" ] || die "usage: $0 <x.y.z> [--dry-run] [--force] [--manifest PATH]" 2
[[ "$NEW" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "\"$NEW\" is not x.y.z (no leading v)" 2

# Resolve a caller-relative manifest path before moving to the repo root.
if [ -n "$MANIFEST" ]; then
    [ -f "$MANIFEST" ] || die "manifest not found: $MANIFEST"
    MANIFEST="$(cd "$(dirname "$MANIFEST")" && pwd)/$(basename "$MANIFEST")"
fi

cd "$(dirname "$0")/.." || die 'cannot cd to the repo root'

REAL_MANIFEST="packaging/flathub/io.github.totoshko88.RustConn.yml"
MANIFEST="${MANIFEST:-$REAL_MANIFEST}"
[ -f "$MANIFEST" ] || die "manifest not found: $MANIFEST"

# ── Cargo.toml must agree, unless --force ────────────────────────────────────
# Same read as release.sh section 2.
CARGO_VERSION="$(awk -F'"' '/^\[workspace\.package\]/{p=1} p&&/^version[[:space:]]*=/{print $2; exit}' Cargo.toml)"
[ -n "$CARGO_VERSION" ] || die 'cannot read [workspace.package].version from Cargo.toml'
if [ "$CARGO_VERSION" != "$NEW" ]; then
    [ "$FORCE" -eq 1 ] || die "Cargo.toml version is $CARGO_VERSION, not $NEW — bump Cargo.toml first, or pass --force"
    printf 'flathub-tag: --force: Cargo.toml says %s, writing %s anyway\n' "$CARGO_VERSION" "$NEW" >&2
fi

# ── release.sh's own pattern for this file ───────────────────────────────────
RELEASE_SH="scripts/release.sh"
[ -f "$RELEASE_SH" ] || die "$RELEASE_SH not found — it owns the gate pattern"
extract_array() { # same reader as scripts/bump-version.sh
    awk -v name="$1" '
        $0 ~ "^" name "=\\($" { f = 1; next }
        f && /^\)$/ { exit }
        f
    ' "$RELEASE_SH" | sed -E 's/^[[:space:]]*"(.*)"[[:space:]]*$/\1/'
}
mapfile -t PKG_FILES < <(extract_array PKG_FILES)
mapfile -t PKG_PATS < <(extract_array PKG_PATS)
[ "${#PKG_FILES[@]}" -gt 0 ] && [ "${#PKG_FILES[@]}" -eq "${#PKG_PATS[@]}" ] ||
    die "cannot read PKG_FILES/PKG_PATS from $RELEASE_SH — did its shape change?"
GATE_PAT=""
for i in "${!PKG_FILES[@]}"; do
    if [ "${PKG_FILES[$i]}" = "$REAL_MANIFEST" ]; then
        GATE_PAT="${PKG_PATS[$i]}"
        break
    fi
done
[ -n "$GATE_PAT" ] || die "$RELEASE_SH no longer lists $REAL_MANIFEST in PKG_FILES"
VERSION_RE="${NEW//./\\.}"
GATE_PAT="${GATE_PAT//\$VERSION_RE/$VERSION_RE}"
GATE_PAT="${GATE_PAT//\\\\/\\}"

# ── The manifest must have exactly one tag line, in the shape the gate expects ─
TAG_ANY='^[[:space:]]*tag:[[:space:]]'
TAG_LINE='^[[:space:]]+tag: v[0-9]+\.[0-9]+\.[0-9]+$'
tag_count="$(grep -cE "$TAG_ANY" "$MANIFEST" || true)"
case "$tag_count" in
0) die "$MANIFEST has no tag: line — nothing to set" ;;
1) ;;
*) die "$MANIFEST has $tag_count tag: lines — refusing to guess which one is the app's" ;;
esac
grep -qE "$TAG_LINE" "$MANIFEST" ||
    die "the tag: line in $MANIFEST is not 'tag: vX.Y.Z' alone on its line: $(grep -nE "$TAG_ANY" "$MANIFEST")"

# ── Build the new manifest in a temp file ────────────────────────────────────
tmp="$(mktemp)"
trap 'rm -f "$tmp" "$tmp.c"' EXIT
sed -E "s/^([[:space:]]+)tag: v[0-9]+\.[0-9]+\.[0-9]+\$/\\1tag: v$NEW/" "$MANIFEST" >"$tmp"

# A `commit:` line in the same source entry as the tag line: same indentation,
# between the list item that holds the tag and the next item or dedent.
commit_line="$(awk '
    { line[NR] = $0 }
    /^[[:space:]]+tag: v/ { t = NR; match($0, /^[[:space:]]+/); ind = RLENGTH }
    END {
        if (!t) exit
        for (i = t - 1; i > 0; i--) {
            s = line[i]; match(s, /^[[:space:]]*/)
            if (s ~ /^[[:space:]]*$/ || s ~ /^[[:space:]]*#/) continue
            if (RLENGTH < ind) { start = i; break }
        }
        if (!start) start = 1
        for (i = start; i <= NR; i++) {
            s = line[i]; match(s, /^[[:space:]]*/)
            if (i > t && s !~ /^[[:space:]]*$/ && s !~ /^[[:space:]]*#/ && RLENGTH < ind) break
            if (RLENGTH == ind && s ~ /^[[:space:]]+commit:[[:space:]]/) print i
        }
    }' "$MANIFEST")"

if [ -n "$commit_line" ]; then
    [ "$(printf '%s\n' "$commit_line" | wc -l)" -eq 1 ] ||
        die "more than one commit: line beside the tag in $MANIFEST — refusing"
    if sha="$(git rev-parse -q --verify "refs/tags/v$NEW^{commit}" 2>/dev/null)"; then
        sed -E "${commit_line}s/^([[:space:]]+commit:[[:space:]]+).*\$/\\1$sha/" "$tmp" >"$tmp.c"
        cat "$tmp.c" >"$tmp"
        printf 'flathub-tag: commit: set to %s (local tag v%s)\n' "$sha" "$NEW"
    else
        printf 'flathub-tag: NOTE: line %s has a commit: pin and tag v%s does not exist locally —\n' \
            "$commit_line" "$NEW"
        printf '             left unchanged. It will not match the new tag; fix it once the tag exists.\n'
    fi
fi

# The result must satisfy release.sh, or the writer and the gate have drifted.
grep -qE -- "$GATE_PAT" "$tmp" ||
    die "after the edit the manifest still fails release.sh's pattern '$GATE_PAT' — update this script and bump-version.sh together"

# ── Report / write ───────────────────────────────────────────────────────────
if cmp -s "$MANIFEST" "$tmp"; then
    printf 'flathub-tag: %s already has tag: v%s — nothing to change.\n' "$MANIFEST" "$NEW"
else
    diff -u --label "a/$MANIFEST" --label "b/$MANIFEST" "$MANIFEST" "$tmp" || true
    if [ "$DRY_RUN" -eq 1 ]; then
        printf '\nflathub-tag: DRY RUN — nothing written. Apply with: %s %s\n' "$0" "$NEW"
        exit 0
    fi
    cat "$tmp" >"$MANIFEST"
    printf '\nflathub-tag: wrote tag: v%s to %s\n' "$NEW" "$MANIFEST"
fi

printf 'Next: ./scripts/release.sh --dry-run\n'
