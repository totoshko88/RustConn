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
#   ./scripts/release.sh --with-tests   # also run `cargo test --workspace` (slow, ~120s)
#   ./scripts/release.sh --skip-checks  # skip cargo fmt/clippy (NOT recommended)
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
WITH_TESTS=false
SKIP_CHECKS=false
ASSUME_YES=false

for arg in "$@"; do
    case "$arg" in
        --dry-run)     DRY_RUN=true ;;
        --no-push)     NO_PUSH=true ;;
        --with-tests)  WITH_TESTS=true ;;
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

# ──────────────────────────────────────────────────────────────────────────────
# 4. metainfo.xml has matching <release version="..." date="...">
# ──────────────────────────────────────────────────────────────────────────────
METAINFO="rustconn/assets/io.github.totoshko88.RustConn.metainfo.xml"
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
declare -A PKG_PATTERNS=(
    [debian/changelog]="^rustconn \\($VERSION-1\\)"
    [packaging/obs/debian.changelog]="^rustconn \\($VERSION-1\\)"
    [packaging/obs/rustconn.dsc]="^Version: $VERSION-1$"
    [packaging/obs/debian.dsc]="^Version: $VERSION-1$"
    [packaging/obs/rustconn.spec]="^Version:[[:space:]]+$VERSION$"
    [packaging/obs/AppImageBuilder.yml]="^[[:space:]]+version: $VERSION$"
    [packaging/flatpak/io.github.totoshko88.RustConn.yml]="tag: v$VERSION$"
    [packaging/flathub/io.github.totoshko88.RustConn.yml]="tag: v$VERSION$"
    [docs/USER_GUIDE.md]="\\*\\*Version $VERSION\\*\\*"
    [docs/ARCHITECTURE.md]="\\*\\*Version $VERSION\\*\\*"
)

PKG_FAILED=0
for file in "${!PKG_PATTERNS[@]}"; do
    pattern="${PKG_PATTERNS[$file]}"
    if [[ ! -f "$file" ]]; then
        warn "Packaging file missing: $file"
        ((PKG_FAILED+=1))
        continue
    fi
    if ! grep -qE "$pattern" "$file"; then
        warn "Version $VERSION not found in $file (pattern: $pattern)"
        ((PKG_FAILED+=1))
    fi
done

if (( PKG_FAILED > 0 )); then
    fail "$PKG_FAILED packaging file(s) out of sync"
fi
ok "All ${#PKG_PATTERNS[@]} packaging files synced to $VERSION"

# ──────────────────────────────────────────────────────────────────────────────
# 6. Tag does not exist yet
# ──────────────────────────────────────────────────────────────────────────────
if git rev-parse "$TAG" >/dev/null 2>&1; then
    fail "Tag $TAG already exists. Aborting."
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
# 8. main branch exists and is reachable
# ──────────────────────────────────────────────────────────────────────────────
git rev-parse --verify main >/dev/null 2>&1 || fail "Branch 'main' does not exist"
ok "Branch 'main' exists"

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
# 10. cargo fmt + clippy + (optional) tests
# ──────────────────────────────────────────────────────────────────────────────
if $SKIP_CHECKS; then
    warn "Skipping cargo fmt/clippy (--skip-checks)"
else
    info "Running: cargo fmt --check"
    cargo fmt --check || fail "cargo fmt --check failed"
    ok "cargo fmt clean"

    info "Running: cargo clippy --all-targets --quiet -- -D warnings"
    cargo clippy --all-targets --quiet -- -D warnings || fail "cargo clippy reported warnings"
    ok "cargo clippy: 0 warnings"

    if $WITH_TESTS; then
        info "Running: cargo test --workspace (this is slow, ~120s)"
        cargo test --workspace --quiet || fail "cargo test failed"
        ok "cargo test passed"
    fi
fi

# ──────────────────────────────────────────────────────────────────────────────
# 11. Plan or execute release operations
# ──────────────────────────────────────────────────────────────────────────────
echo
printf '%s%s═══ Release plan for %s ═══%s\n' "$C_BOLD" "$C_GREEN" "$VERSION" "$C_RESET"
plan "git checkout main"
plan "git merge --no-ff $BRANCH -m \"Merge branch '$BRANCH' — Release $TAG\""
plan "git tag -a $TAG -m \"Release $VERSION\""
if $NO_PUSH; then
    plan "(push skipped — --no-push)"
else
    plan "git push origin main --tags"
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
    echo "    git push origin main --tags"
else
    run git push origin main --tags
fi

echo
ok "Release $VERSION completed."
