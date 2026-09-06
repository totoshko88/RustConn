#!/usr/bin/env bash
# PreToolUse guard on the shell tool: an agent may validate a release, never cut
# one.
#
# `scripts/release.sh` ends with an interactive confirmation and *refuses* to run
# without a TTY unless it is handed `--yes`:
#
#     if [[ ! -t 0 ]]; then
#         fail "stdin is not a TTY — pass --yes to confirm non-interactively"
#     fi
#     read -r -p "Proceed? [y/N] " ans
#
# That is not an obstacle to route around. It is the one point in the process
# where a human decides, and an agent shell has no TTY, so `--yes` is the only
# way an agent ever reaches the merge/tag/push — which is exactly why passing it
# is the thing being forbidden here.
#
# What went wrong once, and is the reason this file exists: v0.20.1 was cut by an
# agent with `./scripts/release.sh --yes`. It merged to main, pushed a tag and
# published a GitHub release with five artifacts — carrying a red CI (the Hygiene
# job, which release.sh did not run at the time) and carrying code deletions the
# maintainer had never seen. Undoing it meant deleting a published release.
#
# Rules:
#   R1  `release.sh` without `--dry-run` is refused.
#   R2  `--yes` / `-y` is refused, with or without `--dry-run`.
#   R3  creating a `v<semver>` tag by hand is refused too, otherwise R1 just
#       moves the problem to `git tag`.
#   R4  `git push` is refused outright — any remote, any ref, tag or branch.
#
# R4 replaced a narrower rule on 2026-09-06 at the maintainer's instruction. It
# used to refuse only a push that carried a version tag, on the reasoning that the
# tag push is the irreversible act. That is true and it was the wrong boundary:
# publishing is the maintainer's call regardless of what is being published, and
# an agent that may push a branch is an agent deciding when work becomes visible
# to CI, to reviewers and to anything watching the remote. Committing stays
# allowed — a local commit is reversible and is where the work is recorded.
#
# `--dry-run` is not merely allowed, it is the expected agent action: it runs
# every gate and stops before the plan is executed.
#
# Fails OPEN on anything unexpected. A guard that blocks every shell call because
# jq changed its output shape would be worse than the problem it prevents.

set -uo pipefail

trap 'exit 0' ERR

payload=$(cat) || exit 0
command -v jq >/dev/null 2>&1 || exit 0

cmd=$(printf '%s' "$payload" | jq -r '.tool_input.command // ""' 2>/dev/null) || exit 0
[ -n "$cmd" ] || exit 0

block() {
    printf 'release-manual-only-guard: %s\n' "$1" >&2
    shift
    printf '%s\n' "$@" >&2
    exit 2
}

hand_back=(
    ''
    '  Cutting the release is the maintainer'"'"'s action, in the maintainer'"'"'s'
    '  terminal, with the live "Proceed? [y/N]" prompt. Prepare it, validate it,'
    '  report it — then stop and hand over.'
)

# --- R1/R2: release.sh may only be validated ------------------------------
# Only a real invocation counts, so the path has to sit in *command position*:
# at the start, after a `;`/`&&`/`||`/`|`/`(`, or inside a `sh -c "…"`. That last
# alternative is not optional — `nohup sh -c "./scripts/release.sh --yes"` is
# exactly how this was done the time it went wrong.
#
# Anything else is a mention, not a call: `echo "see scripts/release.sh"` and
# `grep -n scripts/release.sh docs/` are preceded by a text command and are left
# alone. Leading `bash`/`sh`/`nohup`/`exec`/`time` wrappers are stepped over.
release_invocation='(^|[;&|(]|-c[[:space:]]+["'"'"']?)[[:space:]]*((nohup|exec|time|bash|sh)[[:space:]]+)*["'"'"']?([./]|[A-Za-z0-9_./-]*/)?scripts/release\.sh([[:space:]]|$)'

if printf '%s' "$cmd" | grep -qE "$release_invocation"; then

    # `--help` prints the header comment and exits 0 before any gate or git
    # operation. Reading the script's own usage is not cutting a release.
    if printf '%s' "$cmd" | grep -qE '(^|[[:space:]])(--help|-h)([[:space:]]|$)'; then
        exit 0
    fi

    if printf '%s' "$cmd" | grep -qE '(^|[[:space:]])(--yes|-y)([[:space:]]|$)'; then
        block '--yes on release.sh is never the agent'"'"'s to pass.' \
            '  It exists so a human can confirm non-interactively. An agent shell has no' \
            '  TTY, so passing it is the agent standing in for the person who should be' \
            '  deciding — which is how a release once went out with a red CI and unreviewed' \
            '  code deletions in it.' \
            "${hand_back[@]}"
    fi

    if ! printf '%s' "$cmd" | grep -qE '(^|[[:space:]])--dry-run([[:space:]]|$)'; then
        block 'release.sh without --dry-run performs merge → tag → push.' \
            '  Run the validation instead — it executes every gate and stops before the' \
            '  plan is carried out:' \
            '    ./scripts/release.sh --dry-run' \
            "${hand_back[@]}"
    fi

    exit 0
fi

# --- R3/R4: no cutting a release by hand either ----------------------------
# AGENTS.md already states this ("Never run `git tag`/`git push` by hand for a
# release"); without it here, R1 would only redirect the same action through git.
semver='v[0-9]+\.[0-9]+\.[0-9]+'

# Command position, same reasoning as the release.sh matcher above: at the start,
# after a `;`/`&&`/`||`/`|`/`(`, or inside `sh -c "…"`, stepping over leading
# wrappers. Without this, prose containing the words is treated as a call — and
# under R4, which blocks every push rather than only a tagged one, that turned
# `echo "then git push it"` into a refusal. The narrower pre-R4 rule hid the flaw
# because it also required a version tag on the line.
git_verb() {
    printf '%s' "$cmd" | grep -qE "(^|[;&|(]|-c[[:space:]]+[\"']?)[[:space:]]*((nohup|exec|time|bash|sh)[[:space:]]+)*git([[:space:]]+-[^[:space:]]+)*[[:space:]]+$1([[:space:]]|\$)"
}

if git_verb tag; then
    # Listing and deleting are not cutting a release. `-l`/`--list`/`-n`/`-d`/
    # `--delete`/`--verify` all leave refs alone or remove one, which is what
    # undoing a bad tag needs.
    if ! printf '%s' "$cmd" | grep -qE '(^|[[:space:]])(-l|--list|-n[0-9]*|-d|--delete|--verify|--contains|--points-at|--merged|--no-merged|--sort)([[:space:]=]|$)' \
        && printf '%s' "$cmd" | grep -qE "$semver"; then
        block 'creating a release tag by hand is the same action release.sh performs.' \
            '  Releases go through ./scripts/release.sh so the gates cannot be skipped and' \
            '  the tag cannot disagree with the version in Cargo.toml and the changelogs.' \
            "${hand_back[@]}"
    fi
fi

# --- R4: never push, full stop ---------------------------------------------
# `--dry-run` shows what would be pushed and updates nothing, so it stays
# allowed; it is the push equivalent of `release.sh --dry-run`.
if git_verb push; then
    if printf '%s' "$cmd" | grep -qE '(^|[[:space:]])(--dry-run|-n)([[:space:]]|$)'; then
        exit 0
    fi

    # A tag push gets the specific explanation, because the consequence is
    # specific: it is what publishes a release.
    if printf '%s' "$cmd" | grep -qE "(^|[[:space:]])(--tags|--follow-tags)([[:space:]]|$)" \
        || printf '%s' "$cmd" | grep -qE "refs/tags/$semver|(^|[[:space:]:])$semver([[:space:]]|$)"; then
        block 'pushing a release tag is what publishes the release.' \
            '  The tag push triggers the Release workflow, the artifact build and the' \
            '  Flathub/OBS/Snap updates — none of which can be taken back cleanly.' \
            "${hand_back[@]}"
    fi

    block 'pushing is never the agent'"'"'s action — not a branch, not a tag, not any remote.' \
        '  Commit locally, then hand over. Say what is ready and let the maintainer push,' \
        '  or point at the release path:' \
        '    git push -u origin <branch>      the maintainer'"'"'s call' \
        '    ./scripts/release.sh --dry-run   validate a release, then hand over' \
        '' \
        '  A commit is local and reversible. A push is not: it is the point where work' \
        '  becomes visible to CI, to reviewers and to anything watching the remote, and' \
        '  deciding when that happens belongs to the maintainer.' \
        '  `git push --dry-run` is allowed if you need to show what would go.'
fi

exit 0
