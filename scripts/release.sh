#!/usr/bin/env bash
# Validate the current release branch and run merge → tag → push.
#
# Convention:
#   - Each release is developed on a branch named after its semver version
#     (e.g. `0.15.0`, `0.14.10`).
#   - The branch name MUST match `[workspace.package].version` in Cargo.toml.
#   - CHANGELOG.md must contain `## [<version>] - YYYY-MM-DD` for that version.
#   - The same date must appear in debian/changelog and metainfo.xml.
#   - Tag `v<version>` MUST NOT exist yet.
#
# Usage:
#   ./scripts/release.sh                # validate + run merge/tag/push (asks for confirmation)
#   ./scripts/release.sh --dry-run      # validate + show what WOULD be done
#   ./scripts/release.sh --no-push      # validate + merge + tag locally; skip push
#   ./scripts/release.sh --skip-tests   # skip cargo test (saves ~120s)
#   ./scripts/release.sh --skip-checks  # skip cargo fmt/clippy/tests (NOT recommended)
#   ./scripts/release.sh --yes          # do not prompt before push
#
# Exit codes:
#   0 — release operations completed (or dry-run validation passed)
#   1 — validation failed
#   2 — error (missing tools, dirty tree, etc.)

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
            sed -n '2,25p' "$0"
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
    sedi "s|^## \[$VERSION\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$|## [$VERSION] - $iso|" CHANGELOG.md

    # metainfo.xml: <release version="X.Y.Z" date="YYYY-MM-DD">
    sedi "s|(<release version=\"$VERSION\" date=\")[0-9-]+(\")|\\1$iso\\2|" "$METAINFO"

    # Debian changelogs: trailer line of this version's entry
    #   -- Name <email>  Tue, 10 Jun 2026 14:00:00 +0300
    local f
    for f in debian/changelog packaging/obs/debian.changelog; do
        sedi "/^rustconn \($VERSION-1\)/,/^ -- /{s|^( -- .*>)  .*\$|\\1  $deb_date|;}" "$f"
    done

    # OBS rustconn.changes: entry header  Tue Jun 10 2026 Name <email> - X.Y.Z
    sedi "s|^[A-Z][a-z]{2} [A-Z][a-z]{2} [0-9]{1,2} [0-9]{4}( .* - $VERSION)$|$obs_date\\1|" \
        packaging/obs/rustconn.changes

    # rustconn.spec %changelog: * Tue Jun 10 2026 Name <email> - X.Y.Z-N
    sedi "s|^\* [A-Z][a-z]{2} [A-Z][a-z]{2} [0-9]{1,2} [0-9]{4}( .* - $VERSION(-[0-9]+)?)$|* $obs_date\\1|" \
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
CHANGELOG_LINE="$(grep -m1 -E "^## \[$VERSION\] - [0-9]{4}-[0-9]{2}-[0-9]{2}$" CHANGELOG.md || true)"
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
META_LINE="$(grep -m1 -E "<release version=\"$VERSION\" date=\"[0-9]{4}-[0-9]{2}-[0-9]{2}\"" "$METAINFO" || true)"
[[ -n "$META_LINE" ]] || fail "$METAINFO missing <release version=\"$VERSION\" date=\"...\">"

META_DATE="$(echo "$META_LINE" | sed -nE 's/.*date="([0-9-]+)".*/\1/p')"
if [[ "$META_DATE" != "$CHANGELOG_DATE" ]]; then
    fail "Date mismatch: CHANGELOG.md=$CHANGELOG_DATE, metainfo.xml=$META_DATE"
fi
ok "metainfo.xml release date matches: $META_DATE"

# ──────────────────────────────────────────────────────────────────────────────
# 5. Packaging files version sync
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
    "docs/USER_GUIDE.md"
    "docs/ARCHITECTURE.md"
)
PKG_PATS=(
    "^rustconn \\($VERSION-1\\)"
    "^rustconn \\($VERSION-1\\)"
    "^Version: $VERSION-1$"
    "^Version: $VERSION-1$"
    "^Version:[[:space:]]+$VERSION$"
    " - $VERSION(-[0-9]+)?$"
    "revision\">v$VERSION<"
    "^[[:space:]]+version: $VERSION$"
    "tag: v$VERSION$"
    "tag: v$VERSION$"
    "^version: '$VERSION'$"
    "\\*\\*Version $VERSION\\*\\*"
    "\\*\\*Version $VERSION\\*\\*"
)

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
# 6. Tag does not exist yet (local + remote)
# ──────────────────────────────────────────────────────────────────────────────
info "Fetching latest tags from origin..."
git fetch --tags --quiet origin 2>/dev/null || warn "git fetch failed — checking local only"

if git rev-parse "$TAG" >/dev/null 2>&1; then
    fail "Tag $TAG already exists (local or remote). Aborting."
fi
# Definitive remote check — catches the case where fetch failed above
if git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1; then
    fail "Tag $TAG already exists on origin. Aborting."
fi
ok "Tag $TAG does not exist yet"

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
else
    warn "msgfmt not installed — skipping po validation"
fi

# ──────────────────────────────────────────────────────────────────────────────
# 10. cargo fmt + clippy + tests
# ──────────────────────────────────────────────────────────────────────────────
if $SKIP_CHECKS; then
    warn "Skipping cargo fmt/clippy/tests (--skip-checks)"
else
    info "Running: cargo fmt --all -- --check"
    cargo fmt --all -- --check || fail "cargo fmt --check failed"
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
            --features "tray-macos,vnc-embedded,rdp-embedded,rdp-audio,spice-embedded,adw-1-8" \
            -- -D warnings || fail "cargo clippy reported warnings (rustconn)"
    else
        cargo clippy --all-targets --quiet -- -D warnings || fail "cargo clippy reported warnings"
    fi
    ok "cargo clippy: 0 warnings"

    if $SKIP_TESTS; then
        warn "Skipping tests (--skip-tests)"
    else
        info "Running: cargo test --workspace (this takes ~120s)"
        # On macOS, gdk4-wayland cannot build (no Wayland). Run tests per-crate
        # with macOS-compatible features for the GUI crate.
        if [[ "$(uname -s)" == "Darwin" ]]; then
            cargo test -p rustconn-core -p rustconn-cli || fail "cargo test failed"
            cargo test -p rustconn --no-default-features \
                --features "tray-macos,vnc-embedded,rdp-embedded,rdp-audio,spice-embedded,adw-1-8" \
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
