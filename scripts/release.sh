#!/usr/bin/env bash
# Validate the current release branch and run merge → tag → push.
#
# Convention:
#   - Each release is developed on a branch named after its semver version
#     (e.g. `0.15.0`, `0.14.10`).
#   - The branch name MUST match `[workspace.package].version` in Cargo.toml.
#   - CHANGELOG.md must contain `## [<version>] - YYYY-MM-DD` for that version.
#   - The same date must appear in metainfo.xml, both Debian changelogs, the OBS
#     .changes header and the spec %changelog header.
#   - Every changelog-style file must LEAD with this version, and no
#     version-only file may still mention the previous release.
#   - Tag `v<version>` MUST NOT exist yet.
#
# Usage:
#   ./scripts/release.sh                # validate + run merge/tag/push (asks for confirmation)
#   ./scripts/release.sh --dry-run      # validate + show what WOULD be done
#   ./scripts/release.sh --no-push      # validate + merge + tag locally; skip push
#   ./scripts/release.sh --skip-tests   # skip cargo test (saves ~120s)
#   ./scripts/release.sh --skip-checks  # skip cargo fmt/clippy/tests AND the
#                                       # cargo-sources.json check (NOT recommended)
#   ./scripts/release.sh --yes          # do not prompt before push
#
# Exit codes:
#   0 — release operations completed, or dry-run validation passed, or the user
#       declined at the confirmation prompt
#   1 — any validation gate failed, a required tool is missing, the tree is
#       dirty, or the release dates were refreshed and need review

set -euo pipefail

# ──────────────────────────────────────────────────────────────────────────────
# Colors (only when stdout is a TTY)
# ──────────────────────────────────────────────────────────────────────────────
if [[ -t 1 ]]; then
    C_RESET=$'\033[0m'
    C_BOLD=$'\033[1m'
    C_GREEN=$'\033[32m'
    C_YELLOW=$'\033[33m'
    C_RED=$'\033[31m'
    C_BLUE=$'\033[34m'
else
    C_RESET="" C_BOLD="" C_GREEN="" C_YELLOW="" C_RED="" C_BLUE=""
fi

ok()    { printf '%s[ ok ]%s %s\n'    "$C_GREEN"  "$C_RESET" "$*"; }
info()  { printf '%s[info]%s %s\n'    "$C_BLUE"   "$C_RESET" "$*"; }
warn()  { printf '%s[warn]%s %s\n'    "$C_YELLOW" "$C_RESET" "$*" >&2; }
fail()  { printf '%s[fail]%s %s\n'    "$C_RED"    "$C_RESET" "$*" >&2; exit 1; }
plan()  { printf '%s[plan]%s %s\n'    "$C_YELLOW" "$C_RESET" "$*"; }
run()   { printf '%s[run]%s  %s\n'    "$C_BOLD"   "$C_RESET" "$*"; "$@"; }

# Every gate this run did not actually execute, so the end of the run can say so
# in one place.
#
# The problem this solves: a dry run that skipped three gates ended with exactly
# the same "validation passed" as one that skipped none. The skips were there, as
# individual [warn] lines on stderr, interleaved with a few hundred lines of
# other output — which is to say they were reported and not read. On macOS, the
# platform releases are prepared on, three gates silently stood down: `typos`
# (whose absence is what let v0.20.1 reach a published release with a red CI over
# one word), the release-date check, and the cargo-sources.json check. Recording
# them is not a substitute for making them runnable — the date check has been
# fixed rather than reported, see 4c — but for the ones that genuinely cannot run
# here, the operator has to leave the run knowing which they are.
SKIPPED_GATES=()
skipped() {
    warn "skipping $1 — $2"
    SKIPPED_GATES+=("$1 — $2")
}

# ──────────────────────────────────────────────────────────────────────────────
# Args
# ──────────────────────────────────────────────────────────────────────────────
DRY_RUN=false
NO_PUSH=false
SKIP_TESTS=false
SKIP_CHECKS=false
ASSUME_YES=false

for arg in "$@"; do
    case "$arg" in
        --dry-run)     DRY_RUN=true ;;
        --no-push)     NO_PUSH=true ;;
        --skip-tests)  SKIP_TESTS=true ;;
        --skip-checks) SKIP_CHECKS=true ;;
        --yes|-y)      ASSUME_YES=true ;;
        --help|-h)
            # Print the leading comment block: every line from 2 up to (but not
            # including) the first non-comment line. Hard-coded line numbers went
            # stale the moment the header grew, and printed `set -euo pipefail`.
            awk 'NR > 1 { if ($0 !~ /^#/) exit; print }' "$0"
            exit 0
            ;;
        *)
            fail "Unknown argument: $arg (use --help)"
            ;;
    esac
done

# ──────────────────────────────────────────────────────────────────────────────
# Sanity: required tools, repo root
# ──────────────────────────────────────────────────────────────────────────────
for tool in git grep sed awk cargo; do
    command -v "$tool" >/dev/null || { fail "Missing tool: $tool"; }
done

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || fail "Not inside a git repo"
cd "$REPO_ROOT"

# Portable in-place sed (BSD sed on macOS needs -i '')
sedi() {
    if [[ "$(uname -s)" == "Darwin" ]]; then
        sed -i '' -E "$@"
    else
        sed -i -E "$@"
    fi
}

# ──────────────────────────────────────────────────────────────────────────────
# Release-date refresh: rewrites the date of THIS version's entry in every
# release file, each in its native format. LC_ALL=C keeps month/day names
# English regardless of the user's locale.
# ──────────────────────────────────────────────────────────────────────────────
METAINFO="rustconn/assets/io.github.totoshko88.RustConn.metainfo.xml"
DATE_FILES=(CHANGELOG.md "$METAINFO" debian/changelog
            packaging/obs/debian.changelog packaging/obs/rustconn.changes
            packaging/obs/rustconn.spec)

update_release_dates() {
    local iso="$1"
    local deb_date obs_date
    deb_date="$(LC_ALL=C date '+%a, %d %b %Y %H:%M:%S %z')"
    obs_date="$(LC_ALL=C date '+%a %b %d %Y')"

    # CHANGELOG.md: ## [X.Y.Z] - YYYY-MM-DD
    sedi "s|^## \[$VERSION_RE\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$|## [$VERSION] - $iso|" CHANGELOG.md

    # metainfo.xml: <release version="X.Y.Z" date="YYYY-MM-DD">
    sedi "s|(<release version=\"$VERSION_RE\" date=\")[0-9-]+(\")|\\1$iso\\2|" "$METAINFO"

    # Debian changelogs: trailer line of this version's entry
    #   -- Name <email>  Tue, 10 Jun 2026 14:00:00 +0300
    local f
    for f in debian/changelog packaging/obs/debian.changelog; do
        sedi "/^rustconn \($VERSION_RE-1\)/,/^ -- /{s|^( -- .*>)  .*\$|\\1  $deb_date|;}" "$f"
    done

    # OBS rustconn.changes: entry header  Tue Jun 10 2026 Name <email> - X.Y.Z
    sedi "s|^[A-Z][a-z]{2} [A-Z][a-z]{2} [0-9]{1,2} [0-9]{4}( .* - $VERSION_RE)$|$obs_date\\1|" \
        packaging/obs/rustconn.changes

    # rustconn.spec %changelog: * Tue Jun 10 2026 Name <email> - X.Y.Z-N
    sedi "s|^\* [A-Z][a-z]{2} [A-Z][a-z]{2} [0-9]{1,2} [0-9]{4}( .* - $VERSION_RE(-[0-9]+)?)$|* $obs_date\\1|" \
        packaging/obs/rustconn.spec
}

# ──────────────────────────────────────────────────────────────────────────────
# 1. Branch name = version (semver)
# ──────────────────────────────────────────────────────────────────────────────
BRANCH="$(git branch --show-current)"
[[ -n "$BRANCH" ]] || fail "Detached HEAD — checkout a release branch first"

if [[ ! "$BRANCH" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    fail "Branch '$BRANCH' is not a semver version (expected X.Y.Z)"
fi
VERSION="$BRANCH"
TAG="v$VERSION"
# Dots are regex wildcards: unescaped, "0.19.1" also matches "0x19y1". Every
# grep pattern below interpolates VERSION_RE so a version is matched literally.
VERSION_RE="${VERSION//./\\.}"
ok "Release branch: $BRANCH"

# ──────────────────────────────────────────────────────────────────────────────
# 2. Cargo.toml version matches branch
# ──────────────────────────────────────────────────────────────────────────────
CARGO_VERSION="$(awk -F'"' '/^\[workspace\.package\]/{p=1} p&&/^version[[:space:]]*=/{print $2; exit}' Cargo.toml)"
[[ -n "$CARGO_VERSION" ]] || fail "Cannot read [workspace.package].version from Cargo.toml"

if [[ "$CARGO_VERSION" != "$VERSION" ]]; then
    fail "Cargo.toml version is '$CARGO_VERSION', branch is '$VERSION'"
fi
ok "Cargo.toml version matches branch: $VERSION"

# ──────────────────────────────────────────────────────────────────────────────
# 3. CHANGELOG.md has `## [<version>] - YYYY-MM-DD`
# ──────────────────────────────────────────────────────────────────────────────
CHANGELOG_LINE="$(grep -m1 -E "^## \[$VERSION_RE\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$" CHANGELOG.md || true)"
[[ -n "$CHANGELOG_LINE" ]] || fail "CHANGELOG.md missing '## [$VERSION] - YYYY-MM-DD' header"

CHANGELOG_DATE="$(echo "$CHANGELOG_LINE" | awk '{print $4}')"
ok "CHANGELOG.md: $CHANGELOG_LINE"

# A stale date is easy to miss when the section was written days earlier.
# Offer to refresh the date of this version's entry in every release file.
TODAY="$(date +%Y-%m-%d)"
if [[ "$CHANGELOG_DATE" != "$TODAY" ]]; then
    warn "CHANGELOG date is $CHANGELOG_DATE but today is $TODAY"
    if $DRY_RUN; then
        info "Re-run without --dry-run to be offered an automatic date refresh"
    else
        DO_DATE_UPDATE=false
        if $ASSUME_YES; then
            DO_DATE_UPDATE=true
        elif [[ -t 0 ]]; then
            read -r -p "Update the $VERSION release date to $TODAY in all release files? [y/N] " ans
            case "$ans" in
                y|Y|yes|YES) DO_DATE_UPDATE=true ;;
            esac
        else
            warn "stdin is not a TTY — keeping the stale date (pass --yes to auto-update)"
        fi
        if $DO_DATE_UPDATE; then
            update_release_dates "$TODAY"
            CHANGELOG_DATE="$TODAY"
            ok "Release dates refreshed to $TODAY:"
            git --no-pager diff --stat -- "${DATE_FILES[@]}" || true
            fail "Dates updated — review the diff, commit, and re-run the script"
        fi
    fi
fi

# ──────────────────────────────────────────────────────────────────────────────
# 4. metainfo.xml has matching <release version="..." date="...">
#    ($METAINFO is defined next to update_release_dates above)
# ──────────────────────────────────────────────────────────────────────────────
META_LINE="$(grep -m1 -E "<release version=\"$VERSION_RE\" date=\"[0-9]{4}-[0-9]{2}-[0-9]{2}\"" "$METAINFO" || true)"
[[ -n "$META_LINE" ]] || fail "$METAINFO missing <release version=\"$VERSION\" date=\"...\">"

META_DATE="$(echo "$META_LINE" | sed -nE 's/.*date="([0-9-]+)".*/\1/p')"
if [[ "$META_DATE" != "$CHANGELOG_DATE" ]]; then
    fail "Date mismatch: CHANGELOG.md=$CHANGELOG_DATE, metainfo.xml=$META_DATE"
fi
ok "metainfo.xml release date matches: $META_DATE"

# ──────────────────────────────────────────────────────────────────────────────
# 4b. metainfo.xml is well-formed and valid AppStream
#     The <release> block is hand-written every release, and a broken one is
#     only discovered by the Flathub build — long after the tag was pushed.
# ──────────────────────────────────────────────────────────────────────────────
if command -v xmllint >/dev/null; then
    xmllint --noout "$METAINFO" 2>&1 || fail "$METAINFO is not well-formed XML"
    ok "metainfo.xml is well-formed XML"
else
    skipped "metainfo XML well-formedness" "xmllint is not installed"
fi

if command -v appstreamcli >/dev/null; then
    # --no-net: no screenshot fetching, so this stays offline and fast.
    # Pedantic hints do not affect the exit status; only errors and warnings do.
    if ! appstreamcli validate --no-net "$METAINFO" >/dev/null 2>&1; then
        appstreamcli validate --no-net "$METAINFO" >&2 || true
        fail "appstreamcli validate failed on $METAINFO"
    fi
    ok "metainfo.xml passes appstreamcli validate"
else
    skipped "AppStream validation" "appstreamcli is not installed"
fi

# ──────────────────────────────────────────────────────────────────────────────
# 4c. The release date agrees across every changelog format
#     The header convention says the same date must appear in debian/changelog
#     too, but only metainfo.xml was ever verified. update_release_dates already
#     rewrites the Debian/OBS/spec dates, so verify them and close the loop —
#     a hand-written entry otherwise ships a date CHANGELOG.md disagrees with.
#     Two formats have to be normalised: the Debian trailer is RFC 2822
#     ("Sat, 01 Aug 2026 12:00:00 +0300") and the OBS/spec header is
#     "Sat Aug 01 2026". GNU `date -d` parses both and BSD `date` parses neither,
#     so this check used to skip on macOS — which is where releases are prepared,
#     making it a gate that ran nowhere it was needed and reported success. It now
#     falls back to python3, which is available on both platforms and parses both
#     formats from its standard library, so there is nothing left to skip.
# ──────────────────────────────────────────────────────────────────────────────
if date -d 2026-01-01 +%Y-%m-%d >/dev/null 2>&1; then
    DATE_TO_ISO=gnu
elif command -v python3 >/dev/null 2>&1; then
    DATE_TO_ISO=python
else
    DATE_TO_ISO=none
fi

if [[ "$DATE_TO_ISO" != none ]]; then
    DATE_FAILED=0

    # Normalise one date string to YYYY-MM-DD, or print nothing.
    to_iso_date() {
        if [[ "$DATE_TO_ISO" == gnu ]]; then
            LC_ALL=C date -d "$1" +%Y-%m-%d 2>/dev/null || true
            return
        fi
        python3 - "$1" 2>/dev/null <<'PY' || true
import sys
from datetime import datetime
from email.utils import parsedate_to_datetime

raw = sys.argv[1].strip()
# The Debian changelog trailer is RFC 2822, which the stdlib parses including
# the offset. Tried first because it is the only one of the three with a comma
# and would otherwise fall through to strptime and fail on the timezone.
try:
    print(parsedate_to_datetime(raw).date().isoformat())
    sys.exit(0)
except (TypeError, ValueError):
    pass
# The OBS .changes and spec %changelog headers, and a bare ISO date.
for fmt in ("%a %b %d %Y", "%Y-%m-%d"):
    try:
        print(datetime.strptime(raw, fmt).date().isoformat())
        sys.exit(0)
    except ValueError:
        continue
sys.exit(1)
PY
    }

    # <label> <raw date string> — empty or unparsable counts as a failure.
    check_release_date() {
        local label="$1" raw="$2" iso
        if [[ -z "$raw" ]]; then
            warn "$label: no dated $VERSION entry found"
            ((DATE_FAILED += 1))
            return
        fi
        iso="$(to_iso_date "$raw")"
        if [[ -z "$iso" ]]; then
            warn "$label: cannot parse date '$raw'"
            ((DATE_FAILED += 1))
        elif [[ "$iso" != "$CHANGELOG_DATE" ]]; then
            warn "$label: date $iso disagrees with CHANGELOG.md ($CHANGELOG_DATE)"
            ((DATE_FAILED += 1))
        fi
    }

    # Debian trailer:  -- Anton Isaiev <a@b>  Sat, 01 Aug 2026 12:00:00 +0300
    for f in debian/changelog packaging/obs/debian.changelog; do
        [[ -f "$f" ]] || continue
        check_release_date "$f" "$(awk -v pat="^rustconn \\\\($VERSION-1\\\\)" '
            $0 ~ pat { found = 1 }
            found && /^ -- / { sub(/^ -- .*>[[:space:]]+/, ""); print; exit }' "$f")"
    done

    # OBS .changes header:  Sat Aug 01 2026 Anton Isaiev <a@b> - 0.19.10
    check_release_date "packaging/obs/rustconn.changes" \
        "$(grep -m1 -E " - $VERSION_RE\$" packaging/obs/rustconn.changes \
            | sed -E 's/^([A-Za-z]{3} [A-Za-z]{3} [0-9]{1,2} [0-9]{4}).*/\1/')"

    # spec %changelog top entry:  * Sat Aug 01 2026 Anton Isaiev <a@b> - 0.19.10-0
    check_release_date "packaging/obs/rustconn.spec" \
        "$(sed -n '/^%changelog/,$p' packaging/obs/rustconn.spec | grep -m1 '^\* ' \
            | sed -E 's/^\* ([A-Za-z]{3} [A-Za-z]{3} [0-9]{1,2} [0-9]{4}).*/\1/')"

    if (( DATE_FAILED > 0 )); then
        fail "$DATE_FAILED release file(s) have a date that disagrees with CHANGELOG.md"
    fi
    ok "Release date $CHANGELOG_DATE consistent across CHANGELOG, Debian, OBS and spec"
else
    skipped "cross-format release-date check" "neither GNU 'date -d' nor python3 is available"
fi

# ──────────────────────────────────────────────────────────────────────────────
# 5. Packaging files version sync
#
# CANONICAL LIST: this PKG_FILES array is the single source of truth for which
# packaging files must carry the release version. The release-version hook and
# the `release.md` steering mirror this list for preparation, but THIS gate is
# what actually blocks a release on drift. When adding/removing a packaging
# file, update HERE first, then update the hook + steering to match.
# ──────────────────────────────────────────────────────────────────────────────
PKG_FILES=(
    "debian/changelog"
    "packaging/obs/debian.changelog"
    "packaging/obs/rustconn.dsc"
    "packaging/obs/debian.dsc"
    "packaging/obs/rustconn.spec"
    "packaging/obs/rustconn.changes"
    "packaging/obs/_service"
    "packaging/obs/AppImageBuilder.yml"
    "packaging/flatpak/io.github.totoshko88.RustConn.yml"
    "packaging/flathub/io.github.totoshko88.RustConn.yml"
    "snap/snapcraft.yaml"
    "flake.nix"
    "docs/USER_GUIDE.md"
    "docs/ARCHITECTURE.md"
    "docs/AI_DEVELOPMENT.md"
    "docs/CLI_REFERENCE.md"
    "docs/ZERO_TRUST.md"
    "rustconn/Cargo.toml"
    "rustconn-cli/Cargo.toml"
    "po/rustconn.pot"
    "packaging/macos/rustconn.rb"
)
PKG_PATS=(
    "^rustconn \\($VERSION_RE-1\\)"
    "^rustconn \\($VERSION_RE-1\\)"
    "^Version: $VERSION_RE-1$"
    "^Version: $VERSION_RE-1$"
    "^Version:[[:space:]]+$VERSION_RE$"
    " - $VERSION_RE(-[0-9]+)?$"
    "revision\">v$VERSION_RE<"
    "^[[:space:]]+version: $VERSION_RE$"
    "tag: v$VERSION_RE$"
    "tag: v$VERSION_RE$"
    "^version: '$VERSION_RE'$"
    "version = \"$VERSION_RE\""
    "\\*\\*Version $VERSION_RE\\*\\*"
    "\\*\\*Version $VERSION_RE\\*\\*"
    "\\*\\*Version $VERSION_RE\\*\\*"
    "\\*\\*Version $VERSION_RE\\*\\*"
    "\\*\\*Version $VERSION_RE\\*\\*"
    "version = \"$VERSION_RE\""
    "version = \"$VERSION_RE\""
    "^\"Project-Id-Version: rustconn $VERSION_RE"
    "archive/refs/tags/v$VERSION_RE\\.tar\\.gz"
)

# The two arrays are indexed in parallel; a length mismatch would silently drop
# the checks for the trailing files instead of reporting anything.
if (( ${#PKG_FILES[@]} != ${#PKG_PATS[@]} )); then
    fail "release.sh bug: PKG_FILES has ${#PKG_FILES[@]} entries, PKG_PATS has ${#PKG_PATS[@]}"
fi

PKG_FAILED=0
for i in "${!PKG_FILES[@]}"; do
    file="${PKG_FILES[$i]}"
    pattern="${PKG_PATS[$i]}"
    if [[ ! -f "$file" ]]; then
        warn "Packaging file missing: $file"
        ((PKG_FAILED+=1))
        continue
    fi
    if ! grep -qE -- "$pattern" "$file"; then
        warn "Version $VERSION not found in $file (pattern: $pattern)"
        ((PKG_FAILED+=1))
    fi
done

if (( PKG_FAILED > 0 )); then
    fail "$PKG_FAILED packaging file(s) out of sync"
fi
ok "All ${#PKG_FILES[@]} packaging files synced to $VERSION"

# ──────────────────────────────────────────────────────────────────────────────
# 5a. Every sibling path dependency carries the release version
#
# The gate above greps each file for one `version = "X.Y.Z"` line, so a
# Cargo.toml declaring two sibling crates passes as soon as either one is
# current. That is how rustconn-pty-sys stayed pinned at 0.19.0 while
# rustconn-core was bumped release after release: the requirement is a caret
# range, so it resolved anyway and nothing ever complained.
# ──────────────────────────────────────────────────────────────────────────────
SIBLING_FAILED=0
for file in rustconn/Cargo.toml rustconn-cli/Cargo.toml; do
    while IFS= read -r line; do
        # A path dependency without a `version` key is valid — nothing to check.
        [[ "$line" == *'version = "'* ]] || continue
        if ! grep -qE -- "version = \"$VERSION_RE\"" <<<"$line"; then
            warn "$file: sibling dependency '${line%% *}' is not at $VERSION"
            ((SIBLING_FAILED+=1))
        fi
    done < <(grep -E '^[a-z0-9_-]+ = \{[^}]*path = "\.\./rustconn' "$file")
done
if (( SIBLING_FAILED > 0 )); then
    fail "$SIBLING_FAILED sibling path dependency version(s) out of sync"
fi
ok "Sibling path dependencies synced to $VERSION"

# ──────────────────────────────────────────────────────────────────────────────
# 5b. The new version is the TOP entry of every changelog-style file
#
# The gate above uses plain grep, which is satisfied by an entry anywhere in the
# file — including one left behind by an aborted release. Package tooling reads
# only the newest entry, so verify the newest entry is actually this version.
# ──────────────────────────────────────────────────────────────────────────────
TOP_FAILED=0

# <label> <expected> <actual>
check_top_entry() {
    local label="$1" want="$2" got="$3"
    if [[ "$got" != "$want" ]]; then
        warn "$label: newest entry is '${got:-<none>}', expected '$want'"
        ((TOP_FAILED += 1))
    fi
}

for f in debian/changelog packaging/obs/debian.changelog; do
    [[ -f "$f" ]] || continue
    check_top_entry "$f" "$VERSION" \
        "$(sed -nE '/^rustconn \(/{s/^rustconn \(([^-)]+).*/\1/p;q;}' "$f")"
done

if [[ -f packaging/obs/rustconn.changes ]]; then
    check_top_entry "packaging/obs/rustconn.changes" "$VERSION" \
        "$(sed -nE '/ - [0-9]+\.[0-9]+\.[0-9]+$/{s/.* - //p;q;}' packaging/obs/rustconn.changes)"
fi

if [[ -f packaging/obs/rustconn.spec ]]; then
    check_top_entry "packaging/obs/rustconn.spec %changelog" "$VERSION" \
        "$(sed -n '/^%changelog/,$p' packaging/obs/rustconn.spec \
            | sed -nE '/^\* /{s/.* - ([0-9]+\.[0-9]+\.[0-9]+).*/\1/p;q;}')"
fi

check_top_entry "$METAINFO <releases>" "$VERSION" \
    "$(sed -nE '/<release version="/{s/.*<release version="([^"]+)".*/\1/p;q;}' "$METAINFO")"

if (( TOP_FAILED > 0 )); then
    fail "$TOP_FAILED changelog file(s) do not lead with $VERSION"
fi
ok "debian, OBS, spec and metainfo changelogs all lead with $VERSION"

# ──────────────────────────────────────────────────────────────────────────────
# 5b-bis. The propagated changelogs are not merely present — they are current
#
# The gate above proves each file *leads with* the release version. It cannot
# notice that CHANGELOG.md has since grown four more entries than the five
# formats derived from it, because the header is identical either way.
#
# That is not hypothetical. In 0.21.5 two fixes landed after the release-prep
# commit — a tray dispatch that opened no session, and KeePassXC misread in every
# non-English locale — and both updated CHANGELOG.md alone. Every gate stayed
# green while five user-facing fixes were missing from every distro changelog and
# from the AppStream description shown in GNOME Software.
#
# Compare commits rather than content: matching prose across five formats means
# guessing at wording, and a fuzzy match that can be argued with is a gate nobody
# trusts. If the last commit touching a derived file is an ancestor of the last
# commit touching CHANGELOG.md, that file was written from an older CHANGELOG.
# Equal commits mean they moved together, which is the intended shape. A derived
# file touched *later* is not flagged — fixing a typo in debian/changelog alone is
# legitimate.
#
# The escape, when a CHANGELOG.md edit genuinely needs no propagation: put it in
# the same commit as the derived files, or touch them alongside it. There is no
# flag to skip this, deliberately — the whole failure was a silent omission.
# ──────────────────────────────────────────────────────────────────────────────
DERIVED_CHANGELOGS=(
    "debian/changelog"
    "packaging/obs/debian.changelog"
    "packaging/obs/rustconn.changes"
    "packaging/obs/rustconn.spec"
    "$METAINFO"
)
CHANGELOG_COMMIT="$(git log -1 --format=%H -- CHANGELOG.md 2>/dev/null || true)"
STALE_DERIVED=()
if [[ -n "$CHANGELOG_COMMIT" ]]; then
    for f in "${DERIVED_CHANGELOGS[@]}"; do
        [[ -f "$f" ]] || continue
        derived_commit="$(git log -1 --format=%H -- "$f" 2>/dev/null || true)"
        [[ -n "$derived_commit" ]] || continue
        [[ "$derived_commit" == "$CHANGELOG_COMMIT" ]] && continue
        if git merge-base --is-ancestor "$derived_commit" "$CHANGELOG_COMMIT" 2>/dev/null; then
            STALE_DERIVED+=("$f")
        fi
    done
fi
if (( ${#STALE_DERIVED[@]} > 0 )); then
    warn "CHANGELOG.md was last written in $(git log -1 --format=%h -- CHANGELOG.md);"
    for f in "${STALE_DERIVED[@]}"; do
        warn "  behind it: $f (last written in $(git log -1 --format=%h -- "$f"))"
    done
    fail "${#STALE_DERIVED[@]} derived changelog(s) predate the last CHANGELOG.md edit — propagate, then commit them together"
fi
ok "Derived changelogs are as new as CHANGELOG.md"

# ──────────────────────────────────────────────────────────────────────────────
# 5c. No packaging file still carries the PREVIOUS release version
#
# The sync gate proves the new version is present; it cannot notice a second,
# stale version left beside it. That is a real failure mode: debian.dsc carried
# `Version: 0.19.9-1` with `Files: rustconn-0.19.8.tar.xz` for a whole release
# because only the Version: line was ever checked. Restricted to files that
# hold nothing but the current version — changelogs legitimately keep history.
#
# Comment lines are skipped, for exactly the reason changelogs are excluded
# altogether: a comment can legitimately name an old version because it is
# describing something that happened then. 0.21.0 added one — the note beside the
# Flatpak mirror list saying that `ftp.gnu.org` went unreachable during the
# 0.20.11 release — and the gate failed the release over a sentence that has to
# keep saying 0.20.11 to remain true. Commented-out YAML or TOML is inert, so
# nothing that matters can hide behind this.
# ──────────────────────────────────────────────────────────────────────────────
VERSION_ONLY_FILES=(
    "Cargo.toml"
    "rustconn/Cargo.toml"
    "rustconn-cli/Cargo.toml"
    "packaging/obs/rustconn.dsc"
    "packaging/obs/debian.dsc"
    "packaging/obs/_service"
    "packaging/obs/AppImageBuilder.yml"
    "packaging/flatpak/io.github.totoshko88.RustConn.yml"
    "packaging/flathub/io.github.totoshko88.RustConn.yml"
    "snap/snapcraft.yaml"
    "flake.nix"
    "po/rustconn.pot"
    "packaging/macos/rustconn.rb"
)

PREV_VERSION=""
if PREV_TAG="$(git describe --tags --abbrev=0 --match 'v[0-9]*' 2>/dev/null)"; then
    PREV_VERSION="${PREV_TAG#v}"
fi

if [[ -z "$PREV_VERSION" || "$PREV_VERSION" == "$VERSION" ]]; then
    info "No distinct previous tag to scan for — skipping stale-version check"
else
    PREV_RE="${PREV_VERSION//./\\.}"
    STALE_FAILED=0
    for file in "${VERSION_ONLY_FILES[@]}"; do
        [[ -f "$file" ]] || continue
        if hits="$(grep -nE -- "$PREV_RE" "$file" | grep -vE '^[0-9]+:[[:space:]]*#')"; then
            warn "$file still references the previous version $PREV_VERSION:"
            printf '%s\n' "$hits" >&2
            ((STALE_FAILED += 1))
        fi
    done
    if (( STALE_FAILED > 0 )); then
        fail "$STALE_FAILED file(s) still carry $PREV_VERSION — bump them to $VERSION"
    fi
    ok "No file carries the previous version $PREV_VERSION"
fi

# ──────────────────────────────────────────────────────────────────────────────
# 5d. Homebrew formula template is CI-ready
#
# The release workflow copies packaging/macos/rustconn.rb into the Homebrew tap
# and patches the archive URL version + sha256 via sed. If the template drifts
# (e.g. back to a git+tag source), the sed silently fails and the tap stays
# pinned to an old version (see issue #251). Verify the expected format here.
# ──────────────────────────────────────────────────────────────────────────────
BREW_FORMULA="packaging/macos/rustconn.rb"
if [[ -f "$BREW_FORMULA" ]]; then
    BREW_FAILED=0

    # Must have exactly one active archive URL line
    if ! grep -qE '^  url "https://github.com/totoshko88/RustConn/archive/refs/tags/v[^"]+\.tar\.gz"' "$BREW_FORMULA"; then
        warn "$BREW_FORMULA: missing active archive URL (expected '  url \"https://...archive/refs/tags/vX.Y.Z.tar.gz\"')"
        ((BREW_FAILED += 1))
    fi

    # Must have an active sha256 line: either the placeholder that CI replaces,
    # or a real 64-char lowercase hash as emitted by sha256sum. Anything else
    # (a truncated hash, a stray comment) would let CI's sed produce a formula
    # Homebrew rejects at install time.
    if ! grep -qE '^  sha256 "([a-f0-9]{64}|PLACEHOLDER_SHA256)"' "$BREW_FORMULA"; then
        warn "$BREW_FORMULA: missing active sha256 line (expected PLACEHOLDER_SHA256 or a 64-char hash)"
        ((BREW_FAILED += 1))
    fi

    # The archive URL must reference the current version
    if ! grep -qE "^  url.*v$VERSION_RE\\.tar\\.gz" "$BREW_FORMULA"; then
        warn "$BREW_FORMULA: archive URL does not reference v$VERSION"
        ((BREW_FAILED += 1))
    fi

    # Must NOT have a git+tag source (the old broken format)
    if grep -qE '^  url "https://github.com/totoshko88/RustConn\.git"' "$BREW_FORMULA"; then
        warn "$BREW_FORMULA: still uses git+tag source — CI sed patterns will not work"
        ((BREW_FAILED += 1))
    fi

    if (( BREW_FAILED > 0 )); then
        fail "$BREW_FORMULA is not in the expected format for CI automation (see issue #251)"
    fi
    ok "$BREW_FORMULA template is CI-ready (archive URL v$VERSION + sha256)"
else
    skipped "Homebrew formula check" "$BREW_FORMULA not found"
fi

# ──────────────────────────────────────────────────────────────────────────────
# 6. Tag does not exist yet (local + remote)
# ──────────────────────────────────────────────────────────────────────────────
info "Fetching latest tags from origin..."
# A non-zero exit here is usually NOT a network problem — it is normally a local
# tag that disagrees with origin ("would clobber existing tag"), which git
# refuses to overwrite. Either way the authoritative verdict comes from
# ls-remote below, so this is informational only.
git fetch --tags --quiet origin 2>/dev/null \
    || warn "git fetch --tags returned non-zero (diverged local tag, or network) — relying on ls-remote"

# refs/tags/ explicitly: a bare `git rev-parse v0.19.10` also resolves a BRANCH
# of that name, which would abort the release for the wrong reason.
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null 2>&1; then
    fail "Tag $TAG already exists locally. Aborting."
fi

# Definitive remote check. Exit codes are distinct and must be told apart:
#   0   — the tag is on origin, this release was already published
#   2   — no such ref, which is what we want
#   128 — transport/auth error; treating that as "absent" would let the script
#         validate against a remote it never actually reached
set +e
git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1
LS_REMOTE_RC=$?
set -e
case "$LS_REMOTE_RC" in
    0)  fail "Tag $TAG already exists on origin. Aborting." ;;
    2)  ok "Tag $TAG does not exist yet (verified against origin)" ;;
    *)  warn "Cannot reach origin (git ls-remote exit $LS_REMOTE_RC) — tag checked locally only"
        ok "Tag $TAG does not exist locally" ;;
esac

# ──────────────────────────────────────────────────────────────────────────────
# 7. Working tree status
# ──────────────────────────────────────────────────────────────────────────────
if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
    git status --short --untracked-files=no >&2
    fail "Working tree has uncommitted changes — commit or stash first"
fi
ok "Working tree clean (untracked files ignored)"

# ──────────────────────────────────────────────────────────────────────────────
# 8. main branch exists, is reachable, and release branch contains it
# ──────────────────────────────────────────────────────────────────────────────
git rev-parse --verify main >/dev/null 2>&1 || fail "Branch 'main' does not exist"

# Local main must not lag behind origin/main — merging into a stale main
# would either push outdated history or be rejected as non-fast-forward
# (e.g. after a hotfix merged via the GitHub UI).
git fetch --quiet origin main 2>/dev/null || warn "git fetch origin main failed — checking local only"
if git rev-parse --verify origin/main >/dev/null 2>&1; then
    MAIN_BEHIND="$(git rev-list --count "main..origin/main" 2>/dev/null || echo "0")"
    if (( MAIN_BEHIND > 0 )); then
        fail "Local 'main' is $MAIN_BEHIND commit(s) behind 'origin/main'. Update main first."
    fi
fi

# Ensure release branch includes all commits from main (fast-forward safe merge)
BEHIND="$(git rev-list --count "HEAD..main" 2>/dev/null || echo "0")"
if (( BEHIND > 0 )); then
    fail "Branch '$BRANCH' is $BEHIND commit(s) behind 'main'. Rebase or merge main first."
fi
ok "Branch 'main' exists; release branch is up-to-date"

# ──────────────────────────────────────────────────────────────────────────────
# 9. po/*.po files validate (msgfmt --check)
# ──────────────────────────────────────────────────────────────────────────────
if command -v msgfmt >/dev/null; then
    PO_FAILED=0
    for po in po/*.po; do
        [[ -f "$po" ]] || continue
        if ! msgfmt --check -o /dev/null "$po" 2>/dev/null; then
            warn "msgfmt --check failed on $po"
            ((PO_FAILED+=1))
        fi
    done
    if (( PO_FAILED > 0 )); then
        fail "$PO_FAILED po file(s) failed msgfmt validation"
    fi
    ok "All po/*.po files pass msgfmt --check"

    # Fuzzy and untranslated entries pass msgfmt --check but render as English
    # at runtime, so the loop above cannot see them. Same gate as CI.
    if [[ -x scripts/check-po-complete.sh ]]; then
        if PO_COMPLETE_OUTPUT="$(./scripts/check-po-complete.sh 2>&1)"; then
            ok "check-po-complete.sh passed"
        else
            printf '%s\n' "$PO_COMPLETE_OUTPUT" >&2
            fail "check-po-complete.sh failed — CI enforces this"
        fi
    fi

    # And the one no gate covered until 0.21.0: whether the template describes
    # the strings the sources actually contain. Everything above reads the
    # committed catalogues, so a string missing from the template is invisible to
    # all of it — the catalogues are complete with respect to a template that is
    # itself incomplete. Three releases in a row shipped English strings that
    # way. Needs xgettext, from the same gettext package as msgfmt, which is why
    # it sits inside this branch rather than with the tool-free gates below.
    if [[ -x scripts/check-pot-current.sh ]]; then
        if POT_OUTPUT="$(./scripts/check-pot-current.sh 2>&1)"; then
            ok "check-pot-current.sh passed"
        else
            printf '%s\n' "$POT_OUTPUT" >&2
            fail "check-pot-current.sh failed — the template is out of step with the sources"
        fi
    fi
else
    skipped "po validation, check-po-complete and check-pot-current" "msgfmt is not installed"
fi

# ──────────────────────────────────────────────────────────────────────────────
# 9b. The gates that need no tooling at all
# ──────────────────────────────────────────────────────────────────────────────
# The checks above validate the catalogues; they say nothing about whether the
# manifest still matches the sources. The CI `i18n` job runs these and fails the
# build on any of them, so a release prepared without them gets a red CI on the
# release commit — which is how the 0.20.0 terminal split reached a tag with two
# new modules missing from POTFILES.in.
#
# `check-jump-host-wiring.sh` is not i18n. It is here because it is the same kind
# of check — a grep over a connection between a resolver and its callers, which no
# unit test can cover without a GTK window and a real bastion — and because the
# defect it guards reached a release *with a changelog entry saying it was fixed*
# (#301, 0.20.9).
# `check-css.sh` is here for the same reason as the jump-host check: not i18n, but a
# defect class the test suite structurally cannot see. The app's own log filter hid
# four broken declarations in assets/style.css, so nothing failed and nothing
# reported. It skips itself when python3-gi is absent rather than failing, so a
# release host without the bindings loses the check but not the release.
for i18n_check in check-potfiles.sh check-i18n-escapes.sh check-jump-host-wiring.sh check-css.sh; do
    if [[ -x "scripts/$i18n_check" ]]; then
        if I18N_OUTPUT="$(./scripts/"$i18n_check" 2>&1)"; then
            ok "$i18n_check passed"
        else
            printf '%s\n' "$I18N_OUTPUT" >&2
            fail "$i18n_check failed — CI enforces this and will fail the release commit"
        fi
    else
        skipped "$i18n_check" "not found or not executable"
    fi
done

# ──────────────────────────────────────────────────────────────────────────────
# 9c. Spell check — the CI `Hygiene` job runs `typos` and fails the build on it
#
# The i18n gates above were added because a red CI on the release commit is
# discovered after the tag, not before. `typos` is the same story and was left
# out: v0.20.1 was tagged, pushed and published with a red Hygiene job, over a
# Spanish word in a test fixture. Config lives in typos.toml, which documents how
# to allow a legitimate word rather than "correct" it.
# ──────────────────────────────────────────────────────────────────────────────
# `cargo install typos-cli` puts it in ~/.cargo/bin, which is not on PATH in
# every shell this script gets run from. Resolved explicitly rather than left to
# `command -v` alone: a gate that silently skips is worse than no gate, because
# it reports nothing and is believed.
TYPOS_BIN=""
if command -v typos >/dev/null; then
    TYPOS_BIN="typos"
elif [[ -x "$HOME/.cargo/bin/typos" ]]; then
    TYPOS_BIN="$HOME/.cargo/bin/typos"
fi

if [[ -n "$TYPOS_BIN" ]]; then
    if TYPOS_OUTPUT="$("$TYPOS_BIN" 2>&1)"; then
        ok "typos found no spelling errors"
    else
        printf '%s\n' "$TYPOS_OUTPUT" >&2
        fail "typos failed — the CI Hygiene job enforces this and will fail the release commit"
    fi
else
    skipped "spell check (typos)" "not installed — the CI Hygiene job still enforces it, and a red CI is found after the tag rather than before it"
fi

# ──────────────────────────────────────────────────────────────────────────────
# 10. cargo fmt + clippy + tests
# ──────────────────────────────────────────────────────────────────────────────
if $SKIP_CHECKS; then
    skipped "cargo fmt, clippy and tests" "--skip-checks was passed"
else
    info "Running: cargo fmt --all -- --check"
    # imports_granularity and group_imports are nightly-only options defined in
    # rustfmt.toml. Stable rustfmt prints warnings about them but still formats
    # correctly. Suppress those expected warnings to keep the output clean.
    FMT_OUTPUT="$(cargo fmt --all -- --check 2>&1)" || {
        # Filter out known nightly-only warnings before reporting
        REAL_ISSUES="$(echo "$FMT_OUTPUT" | grep -v "^Warning: can't set \`imports_granularity\|^Warning: can't set \`group_imports" || true)"
        if [[ -n "$REAL_ISSUES" ]]; then
            echo "$REAL_ISSUES" >&2
            fail "cargo fmt --check failed"
        fi
    }
    ok "cargo fmt clean"

    info "Running: cargo clippy --all-targets --quiet -- -D warnings"
    # On macOS, gdk4-wayland cannot build (no Wayland). Exclude it via --no-default-features
    # for the GUI crate and re-enable all other defaults.
    if [[ "$(uname -s)" == "Darwin" ]]; then
        cargo clippy --all-targets --quiet \
            -p rustconn-core -p rustconn-cli \
            -- -D warnings || fail "cargo clippy reported warnings"
        cargo clippy --all-targets --quiet \
            -p rustconn --no-default-features \
            --features "tray-macos,system-keyring,vnc-embedded,rdp-embedded,gfx-h264,rdp-audio,rd-gateway,adw-1-8" \
            -- -D warnings || fail "cargo clippy reported warnings (rustconn)"
    else
        cargo clippy --all-targets --quiet -- -D warnings || fail "cargo clippy reported warnings"
    fi
    ok "cargo clippy: 0 warnings"

    if $SKIP_TESTS; then
        skipped "cargo test" "--skip-tests was passed"
    else
        info "Running: cargo test --workspace (this takes ~120s)"
        # On macOS, gdk4-wayland cannot build (no Wayland). Run tests per-crate
        # with macOS-compatible features for the GUI crate.
        if [[ "$(uname -s)" == "Darwin" ]]; then
            cargo test -p rustconn-core -p rustconn-cli || fail "cargo test failed"
            cargo test -p rustconn --no-default-features \
                --features "tray-macos,system-keyring,vnc-embedded,rdp-embedded,gfx-h264,rdp-audio,rd-gateway,adw-1-8" \
                || fail "cargo test failed (rustconn)"
        else
            cargo test --workspace || fail "cargo test failed"
        fi
        ok "cargo test passed"
    fi
fi

# ──────────────────────────────────────────────────────────────────────────────
# 11. Verify Cargo.lock is up-to-date
# ──────────────────────────────────────────────────────────────────────────────
if [[ -n "$(git status --porcelain -- Cargo.lock)" ]]; then
    fail "Cargo.lock is out of sync with Cargo.toml — run 'cargo check' and commit"
fi
ok "Cargo.lock is up-to-date"

# ──────────────────────────────────────────────────────────────────────────────
# 11b. Flatpak cargo-sources.json matches Cargo.lock (read-only)
#      Regenerates into a temp file and diffs against the committed sources.
#      A stale cargo-sources.json otherwise ships silently in the Flatpak build.
#      Generator/network failure → warn (don't block on flaky crates.io);
#      a definitive content mismatch → fail.
# ──────────────────────────────────────────────────────────────────────────────
FCG="packaging/flatpak/flatpak-cargo-generator.py"
SOURCES="packaging/flatpak/cargo-sources.json"
if $SKIP_CHECKS; then
    skipped "cargo-sources.json check" "--skip-checks was passed"
elif [[ ! -f "$FCG" || ! -f "$SOURCES" ]]; then
    skipped "cargo-sources.json check" "flatpak-cargo-generator or the sources file is missing"
elif ! command -v python3 >/dev/null; then
    skipped "cargo-sources.json check" "python3 is not installed"
else
    info "Verifying $SOURCES matches Cargo.lock..."
    TMP_SOURCES="$(mktemp)"
    if python3 "$FCG" Cargo.lock -o "$TMP_SOURCES" >/dev/null 2>&1; then
        if diff -q "$TMP_SOURCES" "$SOURCES" >/dev/null 2>&1; then
            ok "$SOURCES is in sync with Cargo.lock"
            FLATHUB_SOURCES="packaging/flathub/cargo-sources.json"
            if [[ -f "$FLATHUB_SOURCES" ]] && ! diff -q "$SOURCES" "$FLATHUB_SOURCES" >/dev/null 2>&1; then
                rm -f "$TMP_SOURCES"
                fail "$FLATHUB_SOURCES differs from $SOURCES — copy it and commit:
    cp $SOURCES $FLATHUB_SOURCES"
            fi
        else
            rm -f "$TMP_SOURCES"
            fail "$SOURCES is stale — regenerate it, commit, and re-run:
    python3 $FCG Cargo.lock -o $SOURCES
    cp $SOURCES packaging/flathub/cargo-sources.json"
        fi
    else
        skipped "cargo-sources.json check" "flatpak-cargo-generator failed, most likely no network"
    fi
    rm -f "$TMP_SOURCES"
fi

# ──────────────────────────────────────────────────────────────────────────────
# 11c. What this run did not check
#
# Printed here, once, immediately before the plan, because that is where the
# operator is reading. Individual [warn] lines go to stderr among several hundred
# other lines and are reported without being read — which is how a dry run that
# stood three gates down came to end with the same "validation passed" as one
# that ran everything.
# ──────────────────────────────────────────────────────────────────────────────
if (( ${#SKIPPED_GATES[@]} > 0 )); then
    echo
    printf '%s%s═══ %d gate(s) did NOT run ═══%s\n' \
        "$C_BOLD" "$C_YELLOW" "${#SKIPPED_GATES[@]}" "$C_RESET"
    for gate in "${SKIPPED_GATES[@]}"; do
        warn "$gate"
    done
    warn "A gate that did not run has told you nothing. Decide per line above"
    warn "whether something else covers it before you tag."
else
    echo
    ok "Every gate ran — none was skipped for a missing tool or a flag."
fi

# ──────────────────────────────────────────────────────────────────────────────
# 12. Plan or execute release operations
# ──────────────────────────────────────────────────────────────────────────────
echo
printf '%s%s═══ Release plan for %s ═══%s\n' "$C_BOLD" "$C_GREEN" "$VERSION" "$C_RESET"
plan "git checkout main"
plan "git merge --no-ff $BRANCH -m \"Merge branch '$BRANCH' — Release $TAG\""
plan "git tag -a $TAG -m \"Release $VERSION\""
if $NO_PUSH; then
    plan "(push skipped — --no-push)"
else
    plan "git push --atomic origin main refs/tags/$TAG"
fi
echo

if $DRY_RUN; then
    info "Dry-run complete. Re-run without --dry-run to apply."
    exit 0
fi

# ──────────────────────────────────────────────────────────────────────────────
# Confirm before destructive ops
# ──────────────────────────────────────────────────────────────────────────────
if ! $ASSUME_YES; then
    if [[ ! -t 0 ]]; then
        fail "stdin is not a TTY — pass --yes to confirm non-interactively"
    fi
    read -r -p "Proceed? [y/N] " ans
    case "$ans" in
        y|Y|yes|YES) ;;
        *) info "Aborted."; exit 0 ;;
    esac
fi

# ──────────────────────────────────────────────────────────────────────────────
# Execute
# ──────────────────────────────────────────────────────────────────────────────
run git checkout main
run git merge --no-ff "$BRANCH" -m "Merge branch '$BRANCH' — Release $TAG"
run git tag -a "$TAG" -m "Release $VERSION"

if $NO_PUSH; then
    info "Skipping push (--no-push). Run manually:"
    echo "    git push --atomic origin main refs/tags/$TAG"
else
    # --atomic: main and the tag land together or not at all — prevents a
    # half-release (tag without main, or main without the tag that triggers
    # the release workflow). Push ONLY this tag, never --tags: stray local
    # tags must not reach origin.
    run git push --atomic origin main "refs/tags/$TAG"
fi

echo
ok "Release $VERSION completed."
echo
printf '%s%s═══ Next steps ═══%s\n' "$C_BOLD" "$C_BLUE" "$C_RESET"
info "1. Watch the release workflow:  gh run watch --repo totoshko88/RustConn"
info "2. Flathub: download the 'flathub-update-$TAG' artifact and open the PR"
info "   (see docs/CI_BUILD_FLOW.md → Flathub Release)"
info "3. Snap: test the candidate revision, then promote to stable:"
info "     snap install rustconn --candidate   # on a test machine"
info "     snapcraft release rustconn <revision> latest/stable"
info "4. Verify OBS rebuilds: https://build.opensuse.org/package/show/home:totoshko88:rustconn/rustconn"
