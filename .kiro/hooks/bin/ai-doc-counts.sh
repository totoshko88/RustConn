#!/usr/bin/env bash
# Reports a stale .kiro/ inventory count when it goes stale, not one release later.
#
# docs/AI_DEVELOPMENT.md asserts how many steering files and hooks exist, and
# scripts/check-ai-docs.sh gates those numbers in the CI Hygiene job. Nothing
# checked them locally: no hook ran the script, and it is in neither the
# Definition of Done nor the command list in AGENTS.md. So a8bdb01e added the
# 30th steering file, the count stayed at 29, and the first signal was a red
# Hygiene job on main — after v0.21.12 had been merged, tagged and published to
# every channel. It was the third time that number went stale; the header of
# check-ai-docs.sh records 14 against an actual 27.
#
# PostFileCreate, not PostFileSave, because creating a file under .kiro/steering
# or .kiro/hooks is the exact event that breaks a count. Editing a steering file
# cannot change how many there are, so saving one has nothing to check.
#
# Delivery goes through target/.kiro-session-report: a PostFileCreate hook's
# stdout is discarded, since only SessionStart, UserPromptSubmit and PreToolUse
# forward it. The Session Report (deliver) hook prints it on the next turn, which
# was going to happen anyway. See bin/session-report.sh for the shared-channel
# contract — producers append a self-contained paragraph, flush is the sole
# consumer.
#
# Note the self-reference: adding a hook file trips this hook, which then reports
# the hook count as stale. That is the intended behaviour and not a loop. The
# action is a script, so no agent loop is spent, and the fix is one number.
#
# Deliberately does not edit the number itself. The count sits inside a sentence,
# and a hook that rewrites prose in a document about not hand-maintaining
# inventories is a worse bargain than a hook that says which number is wrong.
#
# Fails OPEN and silent: a broken inventory note must not interrupt a session.

set -uo pipefail

trap 'exit 0' ERR

repo=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
cd "$repo" 2>/dev/null || exit 0

[ -x scripts/check-ai-docs.sh ] || exit 0

# Both streams: the script prints `ok:` lines on stdout and FAIL lines on stderr.
findings=$(./scripts/check-ai-docs.sh 2>&1) && exit 0

fails=$(printf '%s\n' "$findings" | grep -E '^FAIL' || true)
[ -n "$fails" ] || fails="$findings"

mkdir -p target 2>/dev/null || exit 0

{
    printf 'STALE INVENTORY: docs/AI_DEVELOPMENT.md no longer matches .kiro/:\n'
    printf '%s\n' "$fails"
    printf 'Correct the number in docs/AI_DEVELOPMENT.md — the CI Hygiene job gates it,\n'
    printf 'and a hook or steering file with no row in hooks-map.md fails the same gate.\n'
} >>"target/.kiro-session-report" 2>/dev/null || true

exit 0
