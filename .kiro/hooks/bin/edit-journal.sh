#!/usr/bin/env bash
# PostToolUse: record every path THIS agent wrote, so later hooks can scope to
# the agent's own work instead of guessing.
#
# Why this replaces hash comparison. The Stop hook used to decide "did this
# session change it?" by comparing a SessionStart content hash against the file
# on disk. That answers a different question — "did anything change it?" — and
# the two answers diverge whenever something else touches the tree: a second Kiro
# session in the same checkout, the maintainer editing in the IDE while an ACP
# session runs, or a hook of our own like translation-sync rewriting every
# po/*.po. On 2026-09-06 that produced five consecutive Stop reports naming 29
# .rs files in a session whose agent wrote nothing at all. Each report cost an
# agent loop to conclude nothing was wrong.
#
# A journal cannot make that mistake: a path is in it because a write tool was
# called on it, which is the actual question every consumer wants answered.
#
# Consumers: session-report.sh (leftover scan), commit-review-gate.sh (which
# reviews a change needs), the commit rule in
# core-rules.md (exact `git add` list — never `git add -A`, which in a shared
# checkout would stage another session's half-finished work), and
# scripts/change-inventory.sh (handoff for a fresh verification session).
#
# Reset at SessionStart by session-reset.sh. Append-only within a session,
# deduplicated, order preserved. Silent always; PostToolUse stdout is not
# forwarded to the agent anyway, and a journal is not news.
#
# Fails OPEN: a broken journal must never block or delay a write. Every failure
# path just skips the record.

set -uo pipefail

trap 'exit 0' ERR

payload=$(cat) || exit 0
command -v jq >/dev/null 2>&1 || exit 0

# fs_write / fs_append / str_replace use `path`; delete_file uses `targetFile`.
path=$(printf '%s' "$payload" | jq -r '.tool_input.path // .tool_input.targetFile // ""' 2>/dev/null) || exit 0
[ -n "$path" ] || exit 0

# Normalise to repo-relative, the same way crate-boundary-guard.sh does and for
# the same reason: the tools accept absolute or relative paths, and every
# consumer wants one spelling. Anchor at the nearest existing ancestor — for
# delete_file the file is already gone, and for a fresh file its directory may
# not exist yet.
abs=$path
case "$abs" in
/*) ;;
*) abs=$PWD/$abs ;;
esac

anchor=$(dirname "$abs")
while [ "$anchor" != "/" ] && [ ! -d "$anchor" ]; do
    anchor=$(dirname "$anchor")
done
[ -d "$anchor" ] || anchor=$PWD

repo_root=$(git -C "$anchor" rev-parse --show-toplevel 2>/dev/null || true)
[ -n "$repo_root" ] || exit 0

rel=${abs#"$repo_root"/}
# Still absolute -> the write landed outside the repo. Not our business.
case "$rel" in
/*) exit 0 ;;
esac

# The journal lives beside the other session state, under target/: gitignored,
# visible to sub-agents and to the developer in the same checkout, and wiped by
# `cargo clean` — which is acceptable, since a wiped journal only costs one
# session's scoping, and the SessionStart reset would have emptied it anyway.
journal="$repo_root/target/.kiro-session-edits"
mkdir -p "$repo_root/target" 2>/dev/null || exit 0

# Never journal our own bookkeeping: the report file and the journal itself are
# written by hooks, not by the agent.
case "$rel" in
target/*) exit 0 ;;
esac

# Deduplicate. -x so `rustconn/src/lib.rs` does not match
# `rustconn/src/lib.rs.bak`, -F so a path with regex metacharacters is compared
# literally.
if [ -f "$journal" ] && grep -qxF -- "$rel" "$journal" 2>/dev/null; then
    exit 0
fi

printf '%s\n' "$rel" >>"$journal" 2>/dev/null || exit 0

exit 0
