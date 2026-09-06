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

# `git commit` in command position, stepping over leading wrappers, so that
# `echo "run git commit"` and `grep -n 'git commit' docs/` are left alone. Same
# shape as release-manual-only-guard.sh.
commit_invocation='(^|[;&|(]|-c[[:space:]]+["'"'"']?)[[:space:]]*((nohup|exec|time|bash|sh)[[:space:]]+)*git([[:space:]]+-[^[:space:]]+)*[[:space:]]+commit([[:space:]]|$)'
printf '%s' "$cmd" | grep -qE "$commit_invocation" || exit 0

# `--dry-run` inspects; it does not record anything.
printf '%s' "$cmd" | grep -qE '(^|[[:space:]])--dry-run([[:space:]]|$)' && exit 0

repo=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
journal="$repo/target/.kiro-session-edits"
[ -s "$journal" ] || exit 0

needed=""

if grep -qE '^rustconn-(pty|locale|env|dock)-sys/.*\.rs$' -- "$journal" 2>/dev/null; then
    needed="$needed unsafe-reviewer (a rustconn-*-sys crate changed — the only sanctioned unsafe in the workspace);"
fi

if grep -qE '^(rustconn-core/src/secret/.*\.rs|.*credential[^/]*\.rs|.*credentials[^/]*\.rs|.*password[^/]*\.rs)$' -- "$journal" 2>/dev/null; then
    needed="$needed security-reviewer (credential-handling code changed — SecretString, zeroize, stdin pipes, no secrets in logs);"
fi

if grep -qxF 'po/uk.po' -- "$journal" 2>/dev/null; then
    needed="$needed uk-translation-reviewer (po/uk.po changed — DSTU terminology, Kharkiv orthography, imperative mood);"
fi

[ -n "$needed" ] || exit 0

reason="This change touches code that gets a dedicated review before it lands:${needed} Run the reviewer(s) now, or confirm they already ran this session. Scope is the agent's own edits from target/.kiro-session-edits, not the dirty tree."

# PreToolUse honours a decision on stdout with exit 0.
jq -cn --arg r "$reason" \
    '{hookSpecificOutput:{permissionDecision:"ask",permissionDecisionReason:$r}}' 2>/dev/null ||
    exit 0

exit 0
