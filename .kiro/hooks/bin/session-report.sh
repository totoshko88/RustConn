#!/usr/bin/env bash
# Debug-leftover report for the files THIS agent wrote, delivered without
# spending an agent loop.
#
#   session-report.sh write   (Stop)              scan, write target/.kiro-session-report
#   session-report.sh flush   (UserPromptSubmit)  print it, then delete it
#
# Why two triggers instead of one. A Stop hook could report directly, but only as
# an `agent` action, and an agent action *is* a new agent loop — on every turn,
# including turns that changed nothing. That is what this replaces: on 2026-09-06
# the old Stop hook ran five times in one session and produced five reports about
# files the agent had never touched. A `command` action costs nothing, but its
# stdout goes nowhere on Stop (only SessionStart, UserPromptSubmit and PreToolUse
# forward it). So the scan happens for free at Stop and the result is handed over
# on the next prompt, which is a turn that was going to happen anyway. One turn
# late, zero credits.
#
# Scope comes from target/.kiro-session-edits (bin/edit-journal.sh), never from
# the dirty tree: in a checkout shared with the IDE or a second session, "dirty"
# and "ours" are different sets.
#
# The report is a shared channel, not this script's private file. Contract:
# producers APPEND a self-contained paragraph, `flush` is the single consumer and
# removes the file after printing. bin/flatpak-manifest-check.sh is the other
# producer; both are command hooks whose own stdout goes nowhere, so this is how a
# free hook says something to the agent. Adding a third producer needs no change
# here — just append.
#
# Silent when there is nothing to report, so a clean session adds no tokens at
# all. What this deliberately no longer does is call getDiagnostics — that is an
# IDE-side tool, absent in an ACP session, where the old hook produced a dead end
# every turn. Compile diagnostics moved to the commit gate, where clippy runs once
# per feature instead of once per turn (core-rules.md, "Finishing a feature").
#
# Fails OPEN and silent: a broken report must not interrupt a session.

set -uo pipefail

trap 'exit 0' ERR

mode=${1:-}

repo=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
cd "$repo" 2>/dev/null || exit 0

journal="target/.kiro-session-edits"
report=${KIRO_SESSION_REPORT:-target/.kiro-session-report}

# Flush ceiling. Whatever is printed here is pasted into the user's prompt and
# then re-read on every later turn of the session, so it has to be bounded.
# Measured 2026-10-05: one release turn flushed ~1.5k doc-claims lines (~330 KB)
# in a single block. Lines beyond the cap go to <report>.full, which is named in
# the last printed line, so nothing is lost — it is one read away, not in-prompt.
max_lines=40
max_bytes=4096

case "$mode" in
flush)
    [ -s "$report" ] || exit 0
    full="${report}.full"
    # Exact duplicates first: two producers, or one producer re-appending an
    # unfixed finding on every Stop, say the same thing twice.
    # No `--` before the file: mawk rejects it and the fallback would skip dedupe.
    deduped=$(awk '!seen[$0]++' "$report" 2>/dev/null) || deduped=$(cat -- "$report" 2>/dev/null)
    total=$(printf '%s\n' "$deduped" | wc -l | tr -d ' ')
    shown=$(printf '%s\n' "$deduped" | head -n "$max_lines" | head -c "$max_bytes")
    printf '%s\n' "$shown"
    if [ "$(printf '%s\n' "$shown" | wc -c)" -lt "$(printf '%s\n' "$deduped" | wc -c)" ]; then
        printf '%s\n' "$deduped" >"$full" 2>/dev/null || true
        printf '[session-report: %s lines total, truncated here; the rest is in %s]\n' "$total" "$full"
    fi
    rm -f -- "$report" 2>/dev/null || true
    exit 0
    ;;
write) ;;
*) exit 0 ;;
esac

[ -s "$journal" ] || exit 0

# .rs only. A `println!` in a markdown fence is documentation, and rustconn-cli
# prints on purpose — see rustconn-cli/AGENTS.md.
rs_files=$(grep '\.rs$' -- "$journal" 2>/dev/null || true)
[ -n "$rs_files" ] || exit 0

pattern='dbg!|todo!|unimplemented!|println!|eprintln!|allow\(dead_code\)'
leftovers=""

while IFS= read -r f; do
    [ -n "$f" ] || continue
    # delete_file entries have no file left to scan.
    [ -f "$f" ] || continue
    if git ls-files --error-unmatch -- "$f" >/dev/null 2>&1; then
        # Tracked: only lines this change adds. A pre-existing `println!` in
        # rustconn-cli is not a finding.
        hits=$(git diff HEAD -- "$f" 2>/dev/null |
            grep -nE '^\+' |
            grep -E "$pattern" || true)
    else
        # Untracked: the whole file is new, so every line counts as added.
        hits=$(grep -nE "$pattern" -- "$f" 2>/dev/null || true)
    fi
    [ -n "$hits" ] || continue
    while IFS= read -r line; do
        [ -n "$line" ] || continue
        leftovers="$leftovers  $f: ${line}"$'\n'
    done <<<"$hits"
done <<<"$rs_files"

[ -n "$leftovers" ] || exit 0

mkdir -p target 2>/dev/null || exit 0

# Append, per the shared-channel contract above. Re-appending an unfixed leftover
# on the next Stop is intended: flush consumed the previous copy, and the macro is
# still there.
{
    printf 'LEFTOVER: debug macros on lines this session added:\n'
    printf '%s' "$leftovers"
    printf 'Fix or justify each before committing. Do not auto-fix silently.\n'
} >>"$report" 2>/dev/null || true

exit 0
