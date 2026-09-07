#!/usr/bin/env bash
# Writes the release version into every packaging file that carries nothing but a
# version, and reports the ones that need a hand-written changelog entry instead.
#
# Why this exists: scripts/release.sh already owns the canonical list of files
# that must carry the release version (PKG_FILES, with PKG_PATS beside it), but it
# only ever *verifies* them. The writing was done by an agent walking a
# sixteen-bullet list in steering `release-version.md` — a list that duplicated the
# array and asked, in its own text, to be kept in sync with it by hand. So the
# release had a gate whose input was produced by copying the gate's own
# configuration into prose.
#
# This script takes the file list from release.sh at runtime instead of copying it.
# What it owns is the substitution rule per file, and if release.sh grows a file
# this script has no rule for, it says so and exits non-zero rather than skipping
# it quietly.
#
# Usage:
#   scripts/bump-version.sh 0.21.0            dry run: show the diff, change nothing
#   scripts/bump-version.sh 0.21.0 --write    apply
#
# Dry run is the default on purpose. This edits seventeen files immediately before
# a release.
#
# It does NOT do git. No add, no commit, no tag, no push. The merge → tag → push is
# scripts/release.sh, run by the maintainer.
#
# ── Why every rule below is line-anchored ────────────────────────────────────
#
# A global `s/0.20.9/0.21.0/g` would corrupt this repo. Three real examples from
# the 0.20.9 tree:
#
#   * packaging/obs/rustconn.spec:385 records a dependency bump,
#     "cfg-expr 0.20.8→0.20.9". That is a crates.io version that happens to equal
#     the release version.
#   * docs/USER_GUIDE.md:3074 says "Changed in 0.20.9:" in prose about behaviour.
#     It must keep saying 0.20.9 forever.
#   * packaging/obs/rustconn.spec and the debian changelogs keep their whole
#     history. Rewriting old entries would rewrite the past.
#
# So each rule matches a specific line shape, never a bare version string.

set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

# ── Args ─────────────────────────────────────────────────────────────────────
NEW=""
WRITE=0
for arg in "$@"; do
    case "$arg" in
    --write) WRITE=1 ;;
    -h | --help)
        sed -n '2,26p' "$0" | sed 's/^# \{0,1\}//'
        exit 0
        ;;
    -*)
        printf 'unknown option: %s\n' "$arg" >&2
        exit 2
        ;;
    *)
        if [ -n "$NEW" ]; then
            printf 'give exactly one version\n' >&2
            exit 2
        fi
        NEW="$arg"
        ;;
    esac
done

if [ -z "$NEW" ]; then
    printf 'usage: %s <x.y.z> [--write]\n' "$0" >&2
    exit 2
fi
if ! printf '%s' "$NEW" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$'; then
    printf 'FAIL: "%s" is not x.y.z\n' "$NEW" >&2
    exit 2
fi

RELEASE_SH="scripts/release.sh"
[ -f "$RELEASE_SH" ] || {
    printf 'FAIL: %s not found — it owns the file list\n' "$RELEASE_SH" >&2
    exit 1
}

# Any semver, so a rule works regardless of what the file currently holds.
SV='[0-9]+\.[0-9]+\.[0-9]+'

# ── The canonical file list, read from release.sh ─────────────────────────────
extract_array() {
    awk -v name="$1" '
        $0 ~ "^" name "=\\($" { f = 1; next }
        f && /^\)$/ { exit }
        f
    ' "$RELEASE_SH" | sed -E 's/^[[:space:]]*"(.*)"[[:space:]]*$/\1/'
}

mapfile -t PKG_FILES < <(extract_array PKG_FILES)
if [ "${#PKG_FILES[@]}" -eq 0 ]; then
    printf 'FAIL: could not read PKG_FILES out of %s — did its shape change?\n' \
        "$RELEASE_SH" >&2
    exit 1
fi

# The workspace Cargo.toml is deliberately absent from PKG_FILES: release.sh
# checks it separately, against the branch name, because it is the version every
# other file is being synced *to*. It still has to be written, and first.
TARGETS=("Cargo.toml" "${PKG_FILES[@]}")

# ── Substitution rules ───────────────────────────────────────────────────────
#
# Echoes one `sed -E` expression per line for the given file, or nothing if the
# file is changelog-style and needs prose rather than a substitution.
rules_for() {
    case "$1" in
    "Cargo.toml")
        # [workspace.package] version. The workspace file has exactly one
        # `version = "x.y.z"` at column 0; dependency versions are all indented
        # or inline in a table.
        printf 's/^version = "%s"$/version = "%s"/\n' "$SV" "$NEW"
        ;;
    "rustconn/Cargo.toml" | "rustconn-cli/Cargo.toml")
        # Every sibling path dependency, not just the first. rustconn/Cargo.toml
        # declares five, and release.sh section 5a exists because the gate greps
        # for one matching line per file — which let rustconn-pty-sys sit at
        # 0.19.0 through several releases while resolving anyway via its caret.
        printf 's|(path = "\\.\\./rustconn[a-z-]*", version = ")%s(")|\\1%s\\2|g\n' \
            "$SV" "$NEW"
        ;;
    "packaging/obs/rustconn.dsc")
        printf 's/^Version: %s-1$/Version: %s-1/\n' "$SV" "$NEW"
        # The tar names beside Version:. debian.dsc once carried
        # `Version: 0.19.9-1` with `rustconn-0.19.8.tar.xz` for a whole release.
        printf 's/rustconn_%s\\.orig\\.tar\\.xz/rustconn_%s.orig.tar.xz/g\n' "$SV" "$NEW"
        printf 's/rustconn_%s-1\\.debian\\.tar\\.xz/rustconn_%s-1.debian.tar.xz/g\n' "$SV" "$NEW"
        ;;
    "packaging/obs/debian.dsc")
        printf 's/^Version: %s-1$/Version: %s-1/\n' "$SV" "$NEW"
        printf 's/rustconn-%s\\.tar\\.xz/rustconn-%s.tar.xz/g\n' "$SV" "$NEW"
        ;;
    "packaging/obs/_service")
        printf 's|<param name="revision">v%s</param>|<param name="revision">v%s</param>|\n' \
            "$SV" "$NEW"
        ;;
    "packaging/obs/AppImageBuilder.yml")
        printf 's/^([[:space:]]+)version: %s$/\\1version: %s/\n' "$SV" "$NEW"
        ;;
    "packaging/flatpak/io.github.totoshko88.RustConn.yml" | \
        "packaging/flathub/io.github.totoshko88.RustConn.yml")
        # Only the git tag of the app module. Bundled sources (FreeRDP, cJSON)
        # have their own versions and must not move with the release.
        printf 's/^([[:space:]]+)tag: v%s$/\\1tag: v%s/\n' "$SV" "$NEW"
        ;;
    "snap/snapcraft.yaml")
        printf "s/^version: '%s'\$/version: '%s'/\n" "$SV" "$NEW"
        ;;
    "flake.nix")
        printf 's/^([[:space:]]*)version = "%s";$/\\1version = "%s";/\n' "$SV" "$NEW"
        ;;
    "po/rustconn.pot")
        printf 's/^"Project-Id-Version: rustconn %s/"Project-Id-Version: rustconn %s/\n' \
            "$SV" "$NEW"
        ;;
    "packaging/macos/rustconn.rb")
        printf 's|archive/refs/tags/v%s\\.tar\\.gz|archive/refs/tags/v%s.tar.gz|\n' \
            "$SV" "$NEW"
        ;;
    "packaging/obs/rustconn.spec")
        # The Version: field only. The %changelog below it is history and needs a
        # new entry written by hand.
        printf 's/^(Version:[[:space:]]+)%s$/\\1%s/\n' "$SV" "$NEW"
        ;;
    "docs/USER_GUIDE.md" | "docs/ARCHITECTURE.md" | "docs/AI_DEVELOPMENT.md" | \
        "docs/CLI_REFERENCE.md" | "docs/ZERO_TRUST.md")
        # The header line at the top, anchored. "Changed in x.y.z" in the body is
        # a statement about a past release and must not move.
        printf 's/^\\*\\*Version %s\\*\\*/**Version %s**/\n' "$SV" "$NEW"
        ;;
    *)
        return 1
        ;;
    esac
    return 0
}

# Files that legitimately have no substitution rule: they carry history, so a
# release adds an entry rather than rewriting a version.
needs_prose() {
    case "$1" in
    "debian/changelog" | "packaging/obs/debian.changelog" | "packaging/obs/rustconn.changes")
        return 0
        ;;
    *) return 1 ;;
    esac
}

# ── Apply ────────────────────────────────────────────────────────────────────
tmpdir=$(mktemp -d) || exit 1
trap 'rm -rf "$tmpdir"' EXIT

changed=0
unchanged=0
prose=()
norule=()
missing=()

for f in "${TARGETS[@]}"; do
    if [ ! -f "$f" ]; then
        missing+=("$f")
        continue
    fi
    if needs_prose "$f"; then
        prose+=("$f")
        continue
    fi

    mapfile -t exprs < <(rules_for "$f")
    if [ "${#exprs[@]}" -eq 0 ]; then
        norule+=("$f")
        continue
    fi

    sed_args=()
    for e in "${exprs[@]}"; do sed_args+=(-e "$e"); done

    out="$tmpdir/$(printf '%s' "$f" | tr '/' '_')"
    if ! sed -E "${sed_args[@]}" "$f" >"$out" 2>/dev/null; then
        printf 'FAIL: sed failed on %s\n' "$f" >&2
        exit 1
    fi

    if cmp -s "$f" "$out"; then
        unchanged=$((unchanged + 1))
        continue
    fi

    changed=$((changed + 1))
    printf '── %s\n' "$f"
    diff -u "$f" "$out" | sed -n '3,$p' | grep -E '^[+-]' | sed 's/^/   /'

    if [ "$WRITE" -eq 1 ]; then
        cat "$out" >"$f"
    fi
done

# ── Report ───────────────────────────────────────────────────────────────────
printf '\n'
if [ "$WRITE" -eq 1 ]; then
    printf 'Wrote %s to %d file(s); %d already current.\n' "$NEW" "$changed" "$unchanged"
else
    printf 'DRY RUN — nothing written. %d file(s) would change, %d already current.\n' \
        "$changed" "$unchanged"
    printf 'Apply with: %s %s --write\n' "$0" "$NEW"
fi

if [ "${#prose[@]}" -gt 0 ]; then
    printf '\nNeeds a hand-written entry, not a substitution:\n'
    for f in "${prose[@]}"; do printf '  %s\n' "$f"; done
    printf '  packaging/obs/rustconn.spec  (the %%changelog section)\n'
    printf '  rustconn/assets/io.github.totoshko88.RustConn.metainfo.xml  (<release>)\n'
    printf 'Write CHANGELOG.md first, then propagate its content into each format.\n'
fi

if [ "${#missing[@]}" -gt 0 ]; then
    printf '\nFAIL: listed in release.sh PKG_FILES but not on disk:\n'
    for f in "${missing[@]}"; do printf '  %s\n' "$f"; done
fi

if [ "${#norule[@]}" -gt 0 ]; then
    printf '\nFAIL: release.sh expects a version in these, and this script has no rule:\n'
    for f in "${norule[@]}"; do printf '  %s\n' "$f"; done
    printf 'Add a case to rules_for() — do not bump them by hand and move on, that\n'
    printf 'is how the two lists drifted apart in the first place.\n'
fi

if [ "${#missing[@]}" -gt 0 ] || [ "${#norule[@]}" -gt 0 ]; then
    exit 1
fi

# ── Verify with release.sh's own patterns ────────────────────────────────────
if [ "$WRITE" -eq 1 ]; then
    VERSION_RE="${NEW//./\\.}"
    mapfile -t PATS < <(extract_array PKG_PATS)
    if [ "${#PATS[@]}" -ne "${#PKG_FILES[@]}" ]; then
        printf '\nWARN: PKG_FILES has %d entries, PKG_PATS has %d — cannot self-verify.\n' \
            "${#PKG_FILES[@]}" "${#PATS[@]}"
        exit 0
    fi
    bad=0
    for i in "${!PKG_FILES[@]}"; do
        f="${PKG_FILES[$i]}"
        # The pattern holds a literal $VERSION_RE; expand it for the new version.
        p="${PATS[$i]//\$VERSION_RE/$VERSION_RE}"
        p="${p//\\\\/\\}"
        [ -f "$f" ] || continue
        if ! grep -qE -- "$p" "$f" 2>/dev/null; then
            printf 'still out of sync: %s (release.sh pattern: %s)\n' "$f" "$p"
            bad=$((bad + 1))
        fi
    done
    printf '\n'
    if [ "$bad" -gt 0 ]; then
        printf '%d file(s) will still fail release.sh. The changelog-style ones above\n' "$bad"
        printf 'are expected until you write their entries.\n'
        exit 1
    fi
    printf 'All %d files satisfy release.sh version patterns.\n' "${#PKG_FILES[@]}"
fi

exit 0
