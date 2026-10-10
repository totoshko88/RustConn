#!/usr/bin/env bash
# Automate the Flathub release update after a RustConn version tag is pushed.
#
# What this replaces — the manual sequence done once per release:
#   1. Wait for the "Update Flathub" GitHub Actions job to finish on RustConn.
#   2. Download its `flathub-update-v<version>` artifact (the regenerated
#      cargo-sources.json and the manifest, which CI pins to tag + a
#      pre-release commit SHA).
#   3. Drop the artifact's README.md, and strip the `commit:` line from the
#      manifest — CI pins a PRE-RELEASE commit that does not match the tag, so
#      the Flathub build fails on it. We ship tag-only and let Flathub resolve
#      the real commit from the tag.
#   4. In the Flathub checkout: checkout master, pull, create branch <version>,
#      copy the two files in, commit, push.
#   5. Open the PR, wait for CI, merge.
#
# This script does all of that via `gh`. It is READ-SAFE until the push: every
# mutating step (branch create, commit, push, PR, merge) is announced, and the
# push+PR+merge block asks for confirmation unless --yes is given.
#
# It does NOT push the RustConn tag — that is scripts/release.sh's job. Run this
# AFTER release.sh has pushed the tag and the Update Flathub job has started.
#
# Requirements: gh (authenticated), git, jq, unzip.
#
# Usage:
#   scripts/flathub-release.sh                 # auto-detect version, wait, PR, prompt before merge
#   scripts/flathub-release.sh v0.23.8         # explicit tag
#   scripts/flathub-release.sh --no-merge      # open the PR but stop before merging
#   scripts/flathub-release.sh --no-wait-ci    # merge immediately after opening the PR (don't wait for Flathub CI)
#   scripts/flathub-release.sh --yes           # don't prompt before push/PR/merge
#   scripts/flathub-release.sh --dry-run       # show every step without changing anything
#
# Environment overrides:
#   FLATHUB_DIR   Path to the local Flathub checkout
#                 (default: ~/Documents/io.github.totoshko88.RustConn)
#   RUSTCONN_REPO GitHub slug of the source repo   (default: totoshko88/RustConn)
#   FLATHUB_REPO  GitHub slug of the Flathub repo
#                 (default: flathub/io.github.totoshko88.RustConn)
#
# Exit codes:
#   0 — PR opened (and merged, unless --no-merge), or dry-run completed, or the
#       user declined at a confirmation prompt
#   1 — a required tool is missing, the artifact/job was not found, a git/gh
#       step failed, or an input was invalid

set -euo pipefail

# ──────────────────────────────────────────────────────────────────────────────
# Config (overridable via environment)
# ──────────────────────────────────────────────────────────────────────────────
FLATHUB_DIR="${FLATHUB_DIR:-$HOME/Documents/io.github.totoshko88.RustConn}"
RUSTCONN_REPO="${RUSTCONN_REPO:-totoshko88/RustConn}"
FLATHUB_REPO="${FLATHUB_REPO:-flathub/io.github.totoshko88.RustConn}"
APP_ID="io.github.totoshko88.RustConn"
WORKFLOW="flathub-update.yml"
DEFAULT_BRANCH="master"
WAIT_JOB_TIMEOUT="${WAIT_JOB_TIMEOUT:-1800}"  # seconds to wait for the Update Flathub job
POLL_INTERVAL="${POLL_INTERVAL:-20}"          # seconds between polls

# ──────────────────────────────────────────────────────────────────────────────
# Colors (only on a TTY)
# ──────────────────────────────────────────────────────────────────────────────
if [[ -t 1 ]]; then
    C_RESET=$'\033[0m'; C_BOLD=$'\033[1m'; C_GREEN=$'\033[32m'
    C_YELLOW=$'\033[33m'; C_RED=$'\033[31m'; C_BLUE=$'\033[34m'
else
    C_RESET=''; C_BOLD=''; C_GREEN=''; C_YELLOW=''; C_RED=''; C_BLUE=''
fi
info()  { printf '%s==>%s %s\n'   "$C_BLUE"   "$C_RESET" "$*"; }
ok()    { printf '%s✓%s %s\n'     "$C_GREEN"  "$C_RESET" "$*"; }
warn()  { printf '%s!%s %s\n'     "$C_YELLOW" "$C_RESET" "$*" >&2; }
die()   { printf '%s✗%s %s\n'     "$C_RED"    "$C_RESET" "$*" >&2; exit 1; }
step()  { printf '\n%s%s%s\n'     "$C_BOLD"   "$*" "$C_RESET"; }

# ──────────────────────────────────────────────────────────────────────────────
# Args
# ──────────────────────────────────────────────────────────────────────────────
TAG=""
ASSUME_YES=0
DRY_RUN=0
DO_MERGE=1
WAIT_CI=1

usage() { sed -n '2,45p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }

for arg in "$@"; do
    case "$arg" in
        --yes|-y)     ASSUME_YES=1 ;;
        --dry-run|-n) DRY_RUN=1 ;;
        --no-merge)   DO_MERGE=0 ;;
        --no-wait-ci) WAIT_CI=0 ;;
        -h|--help)    usage 0 ;;
        v[0-9]*)      TAG="$arg" ;;
        [0-9]*)       TAG="v$arg" ;;
        *)            die "unknown argument: $arg (see --help)" ;;
    esac
done

run() {
    # Echo and execute, or just echo under --dry-run.
    if [[ "$DRY_RUN" -eq 1 ]]; then
        printf '  %s[dry-run]%s %s\n' "$C_YELLOW" "$C_RESET" "$*"
        return 0
    fi
    "$@"
}

confirm() {
    # $1 = prompt. Returns 0 to proceed, 1 to skip.
    [[ "$ASSUME_YES" -eq 1 ]] && return 0
    [[ "$DRY_RUN" -eq 1 ]] && return 0
    local reply
    printf '%s%s%s [y/N] ' "$C_BOLD" "$1" "$C_RESET"
    read -r reply
    [[ "$reply" =~ ^[Yy]$ ]]
}

# ──────────────────────────────────────────────────────────────────────────────
# Preflight
# ──────────────────────────────────────────────────────────────────────────────
step "Preflight"
for tool in gh git jq unzip; do
    command -v "$tool" >/dev/null 2>&1 || die "$tool is not installed"
done
gh auth status >/dev/null 2>&1 || die "gh is not authenticated — run: gh auth login"
[[ -d "$FLATHUB_DIR/.git" ]] || die "Flathub checkout not found at: $FLATHUB_DIR
Set FLATHUB_DIR, or clone it:
  git clone https://github.com/$FLATHUB_REPO.git \"$FLATHUB_DIR\""
[[ -f "$FLATHUB_DIR/$APP_ID.yml" ]] || die "$FLATHUB_DIR does not look like the Flathub repo (no $APP_ID.yml)"
ok "gh authenticated; Flathub checkout at $FLATHUB_DIR"

# ──────────────────────────────────────────────────────────────────────────────
# Resolve the version/tag
# ──────────────────────────────────────────────────────────────────────────────
if [[ -z "$TAG" ]]; then
    # Latest v* tag on the source repo (what release.sh just pushed).
    TAG=$(gh api "repos/$RUSTCONN_REPO/tags" --jq '.[0].name' 2>/dev/null || true)
    [[ -n "$TAG" ]] || die "could not auto-detect the latest tag from $RUSTCONN_REPO — pass it explicitly (e.g. v0.23.8)"
    info "Auto-detected latest tag: $C_BOLD$TAG$C_RESET"
fi
[[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "tag '$TAG' is not a vX.Y.Z semver tag"
VERSION="${TAG#v}"
ARTIFACT="flathub-update-$TAG"
ok "Release: $TAG  (branch '$VERSION', artifact '$ARTIFACT')"

if gh api "repos/$FLATHUB_REPO/branches/$VERSION" >/dev/null 2>&1; then
    warn "Branch '$VERSION' already exists on $FLATHUB_REPO."
    confirm "Continue anyway (it will be reused/overwritten locally)?" || die "aborted"
fi

# ──────────────────────────────────────────────────────────────────────────────
# 1. Wait for the Update Flathub job
# ──────────────────────────────────────────────────────────────────────────────
step "1. Waiting for the 'Update Flathub' run on $RUSTCONN_REPO ($TAG)"
RUN_ID=""
deadline=$(( $(date +%s) + WAIT_JOB_TIMEOUT ))
while :; do
    # Match the workflow run for this tag by its head_branch (the tag ref).
    RUN_JSON=$(gh run list --repo "$RUSTCONN_REPO" --workflow "$WORKFLOW" \
                   --limit 20 --json databaseId,headBranch,status,conclusion,event 2>/dev/null || echo '[]')
    RUN_ID=$(echo "$RUN_JSON" | jq -r --arg t "$TAG" \
        'map(select(.headBranch==$t)) | sort_by(.databaseId) | last | .databaseId // empty')
    if [[ -n "$RUN_ID" ]]; then
        STATUS=$(echo "$RUN_JSON" | jq -r --argjson id "$RUN_ID" 'map(select(.databaseId==$id))[0].status')
        CONCL=$(echo "$RUN_JSON"  | jq -r --argjson id "$RUN_ID" 'map(select(.databaseId==$id))[0].conclusion')
        if [[ "$STATUS" == "completed" ]]; then
            [[ "$CONCL" == "success" ]] || die "Update Flathub run $RUN_ID finished with conclusion: $CONCL
   See: https://github.com/$RUSTCONN_REPO/actions/runs/$RUN_ID"
            ok "Update Flathub run $RUN_ID completed successfully."
            break
        fi
        info "Run $RUN_ID status=$STATUS … (waiting)"
    else
        info "No Update Flathub run for $TAG yet … (waiting — did you push the tag?)"
    fi
    [[ $(date +%s) -lt $deadline ]] || die "timed out after ${WAIT_JOB_TIMEOUT}s waiting for the Update Flathub job"
    [[ "$DRY_RUN" -eq 1 ]] && { warn "dry-run: not polling further"; RUN_ID="${RUN_ID:-DRYRUN}"; break; }
    sleep "$POLL_INTERVAL"
done

# ──────────────────────────────────────────────────────────────────────────────
# 2. Download the artifact  +  3. drop the README
# ──────────────────────────────────────────────────────────────────────────────
step "2. Downloading artifact '$ARTIFACT'"
WORKDIR=$(mktemp -d "${TMPDIR:-/tmp}/flathub-$VERSION.XXXXXX")
trap 'rm -rf "$WORKDIR"' EXIT
if [[ "$DRY_RUN" -eq 1 ]]; then
    printf '  %s[dry-run]%s gh run download %s --repo %s --name %s --dir %s\n' \
        "$C_YELLOW" "$C_RESET" "$RUN_ID" "$RUSTCONN_REPO" "$ARTIFACT" "$WORKDIR"
else
    gh run download "$RUN_ID" --repo "$RUSTCONN_REPO" --name "$ARTIFACT" --dir "$WORKDIR" \
        || die "failed to download artifact '$ARTIFACT' from run $RUN_ID"
    [[ -f "$WORKDIR/$APP_ID.yml" ]]      || die "artifact is missing $APP_ID.yml"
    [[ -f "$WORKDIR/cargo-sources.json" ]] || die "artifact is missing cargo-sources.json"
    # Step 3a: the README.md is an instruction file for humans, not a repo file.
    rm -f "$WORKDIR/README.md"
    # Step 3b: strip the `commit:` line CI inserted. It pins the PRE-RELEASE
    # commit (the tag is created on the release commit, but CI resolves the SHA
    # before that commit is the tag's target), which makes the Flathub build
    # abort with "commit does not match tag". Ship tag-only; Flathub resolves
    # the real commit from the tag.
    sed -i -E '/^[[:space:]]*commit: [0-9a-f]{7,40}[[:space:]]*$/d' "$WORKDIR/$APP_ID.yml"
    ok "Downloaded manifest + cargo-sources.json (README.md dropped, commit: line stripped)."
    # Sanity: the manifest must keep the tag pin and must NOT carry a commit pin.
    grep -qE "^[[:space:]]*tag: $TAG[[:space:]]*$" "$WORKDIR/$APP_ID.yml" \
        || die "manifest in artifact is not pinned to $TAG — refusing to proceed"
    if grep -qE "^[[:space:]]*commit: [0-9a-f]{7,40}[[:space:]]*$" "$WORKDIR/$APP_ID.yml"; then
        die "a commit: line is still present after stripping — refusing to proceed (would fail the Flathub build)"
    fi
    ok "Manifest is tag-only (tag: $TAG, no commit pin)."
fi

# ──────────────────────────────────────────────────────────────────────────────
# 4. Flathub checkout: branch, copy, commit, push
# ──────────────────────────────────────────────────────────────────────────────
step "4. Preparing the Flathub branch '$VERSION'"
pushd "$FLATHUB_DIR" >/dev/null

# Refuse to clobber unrelated local work.
if [[ -n "$(git status --porcelain)" ]]; then
    die "Flathub checkout has uncommitted changes — commit/stash them first:
  (cd \"$FLATHUB_DIR\" && git status)"
fi

run git checkout "$DEFAULT_BRANCH"
run git pull --ff-only origin "$DEFAULT_BRANCH"

# Create or reset the version branch off the freshly-pulled master.
if git show-ref --verify --quiet "refs/heads/$VERSION"; then
    warn "Local branch '$VERSION' exists — resetting it onto $DEFAULT_BRANCH."
    run git branch -f "$VERSION" "$DEFAULT_BRANCH"
fi
run git switch -c "$VERSION" 2>/dev/null || run git switch "$VERSION"

if [[ "$DRY_RUN" -eq 1 ]]; then
    printf '  %s[dry-run]%s cp %s/{%s.yml,cargo-sources.json} %s/\n' \
        "$C_YELLOW" "$C_RESET" "$WORKDIR" "$APP_ID" "$FLATHUB_DIR"
else
    cp "$WORKDIR/$APP_ID.yml" "$FLATHUB_DIR/$APP_ID.yml"
    cp "$WORKDIR/cargo-sources.json" "$FLATHUB_DIR/cargo-sources.json"
fi

run git add "$APP_ID.yml" cargo-sources.json

if [[ "$DRY_RUN" -eq 0 ]] && git diff --cached --quiet; then
    warn "No changes to commit — the Flathub files already match $TAG. Nothing to do."
    popd >/dev/null
    exit 0
fi

step "Review the staged diff"
run git --no-pager diff --cached --stat

if ! confirm "Commit, push branch '$VERSION', and open the PR?"; then
    warn "Stopped before pushing. The branch '$VERSION' is prepared locally."
    popd >/dev/null
    exit 0
fi

run git commit -m "$VERSION"
run git push -u origin "$VERSION"
ok "Pushed branch '$VERSION' to $FLATHUB_REPO."

# ──────────────────────────────────────────────────────────────────────────────
# 5. PR → wait for CI → merge
# ──────────────────────────────────────────────────────────────────────────────
step "5. Opening the Pull Request"
PR_TITLE="Update to $TAG"
PR_BODY="Automated Flathub update to $TAG.

- Manifest pinned to \`tag: $TAG\` (tag-only; Flathub resolves the commit, the
  pre-release commit SHA is intentionally stripped).
- \`cargo-sources.json\` regenerated from \`Cargo.lock\`.

Generated from the \`$ARTIFACT\` CI artifact."

if [[ "$DRY_RUN" -eq 1 ]]; then
    printf '  %s[dry-run]%s gh pr create --repo %s --base %s --head %s --title %q\n' \
        "$C_YELLOW" "$C_RESET" "$FLATHUB_REPO" "$DEFAULT_BRANCH" "$VERSION" "$PR_TITLE"
    PR_URL="(dry-run: no PR)"
else
    PR_URL=$(gh pr create --repo "$FLATHUB_REPO" --base "$DEFAULT_BRANCH" --head "$VERSION" \
                 --title "$PR_TITLE" --body "$PR_BODY" 2>/dev/null) \
        || PR_URL=$(gh pr view "$VERSION" --repo "$FLATHUB_REPO" --json url --jq .url)
fi
ok "PR: $PR_URL"

if [[ "$DO_MERGE" -eq 0 ]]; then
    info "--no-merge: leaving the PR open for manual review/merge."
    popd >/dev/null
    exit 0
fi

if [[ "$WAIT_CI" -eq 1 && "$DRY_RUN" -eq 0 ]]; then
    step "Waiting for Flathub CI checks on the PR"
    info "This is the flatpak build on Flathub's infra — it can take several minutes."
    if ! gh pr checks "$VERSION" --repo "$FLATHUB_REPO" --watch --interval "$POLL_INTERVAL"; then
        die "Flathub CI did not pass. Inspect the PR before merging:
  $PR_URL"
    fi
    ok "Flathub CI passed."
fi

if ! confirm "Merge the PR now?"; then
    warn "PR left open (not merged): $PR_URL"
    popd >/dev/null
    exit 0
fi

run gh pr merge "$VERSION" --repo "$FLATHUB_REPO" --merge --delete-branch
ok "Merged $TAG into $FLATHUB_REPO. Flathub will build and publish shortly."

popd >/dev/null
step "Done — $TAG is on its way to Flathub 🎉"
