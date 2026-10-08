#!/usr/bin/env bash
# PreToolUse on the shell tool: at `git commit`, ask for the high-risk reviews the
# journal says this change needs.
#
# What this replaces. Three agent hooks fired on PostFileSave — security-review on
# any credential file, unsafe-review on any rustconn-*-sys file,
# uk-translation-review on po/uk.po. PostFileSave is per file, so editing six
# files in the secret subsystem cost six agent loops, each reviewing one file in
# isolation, none of them able to see the change as a whole. uk-translation-review
# additionally fired after every `msgmerge`, which rewrites the entire catalogue
# without changing a single existing msgstr.
#
# A review is worth one pass over the finished change, at the moment the change
# stops moving. That moment is the commit.
#
# `ask` rather than exit 2. The reviews are obligations, not invariants: a guard
# cannot tell whether they already ran this session, and a hard block on a
# reviewer it cannot observe would be a guard that lies. Surfacing the question
# once, at the commit, with the exact sub-agent named, puts the decision where it
# belongs. The crate-boundary and release guards still block, because those they
# can actually verify.
#
# Scope comes from target/.kiro-session-edits (bin/edit-journal.sh) — the paths
# this agent wrote. Deriving it from the dirty tree would ask for an unsafe review
# because someone else's session touched a -sys crate.
#
# Fails OPEN: no journal, no jq, no git, nothing recognised -> allow.

set -uo pipefail

trap 'exit 0' ERR

payload=$(cat) || exit 0
command -v jq >/dev/null 2>&1 || exit 0

cmd=$(printf '%s' "$payload" | jq -r '.tool_input.command // ""' 2>/dev/null) || exit 0
[ -n "$cmd" ] || exit 0

# Find a real `git commit` with the same parser release-manual-only-guard.sh
# uses, so `echo "run git commit"` and `grep -n 'git commit' docs/` are left alone
# while `GIT_EDITOR=true git commit` and `git add a && git commit` are not. The
# regex it replaced missed the NAME=value form (scripts/test-hooks.sh has the row).
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd) || exit 0
committing=0
while IFS=$'\t' read -r kind verb args; do
    [ "$kind" = GIT ] && [ "$verb" = commit ] || continue
    # `--dry-run` inspects; it does not record anything.
    case " $args " in *" --dry-run "*) continue ;; esac
    committing=1
done < <(printf '%s' "$cmd" | awk -f "$here/lib/command-segments.awk" 2>/dev/null)
[ "$committing" -eq 1 ] || exit 0

repo=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
# KIRO_EDIT_JOURNAL lets scripts/test-hooks.sh point the gate at a scratch journal.
journal="${KIRO_EDIT_JOURNAL:-$repo/target/.kiro-session-edits}"
[ -s "$journal" ] || exit 0

needed=""

# The crate-name shape, not a list: crate-boundary-guard.sh sanctions any
# rustconn-<x>-sys crate, so a fifth one must get the review too. Cargo.toml is in
# scope because a -sys crate's [lints] table is where its unsafe budget lives.
if grep -qE '^rustconn-[a-z0-9-]+-sys/(.*\.rs|Cargo\.toml)$' -- "$journal" 2>/dev/null; then
    needed="$needed unsafe-reviewer (a rustconn-*-sys crate changed — the only sanctioned unsafe in the workspace);"
fi

if grep -qE '^(rustconn-core/src/secret/.*\.rs|.*credential[^/]*\.rs|.*credentials[^/]*\.rs|.*password[^/]*\.rs)$' -- "$journal" 2>/dev/null; then
    needed="$needed security-reviewer (credential-handling code changed — SecretString, zeroize, stdin pipes, no secrets in logs);"
fi

if grep -qxF 'po/uk.po' -- "$journal" 2>/dev/null; then
    needed="$needed uk-translation-reviewer (po/uk.po changed — DSTU terminology, Kharkiv orthography, imperative mood);"
fi

# scripts/change-inventory.sh repeats these four patterns for the rustconn-review
# workflow, which skips a reviewer the inventory does not list. Change both.
#
# config-mapping-reviewer: persisted<->runtime config drift and export/import
# round-trips. A stored-but-unread field or a fake export `method:` compiles and
# passes clippy, so the quality gate never catches it (SPICE proxy, RDP smartcard,
# Asbru export all shipped broken before 0.22.12). The file set mirrors
# config-mapping-guide.md's fileMatch.
if grep -qE '^(rustconn-core/src/models/protocol\.rs|rustconn-core/src/[a-z_]+_client/config\.rs|rustconn-core/src/protocol/freerdp\.rs|rustconn/src/window/(protocols|rdp_vnc)\.rs|rustconn/src/embedded_rdp/launcher\.rs|rustconn-core/src/(export|import)/.*\.rs)$' -- "$journal" 2>/dev/null; then
    needed="$needed config-mapping-reviewer (a persisted config, runtime config, launch mapper or import/export converter changed — check for stored-but-unread fields and broken round-trips);"
fi

[ -n "$needed" ] || exit 0

reason="This change touches code that gets a dedicated review before it lands:${needed} Run the reviewer(s) now, or confirm they already ran this session. Scope is the agent's own edits from target/.kiro-session-edits, not the dirty tree."

# PreToolUse honours a decision on stdout with exit 0.
jq -cn --arg r "$reason" \
    '{hookSpecificOutput:{permissionDecision:"ask",permissionDecisionReason:$r}}' 2>/dev/null ||
    exit 0

exit 0
