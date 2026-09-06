#!/usr/bin/env bash
# SessionStart: clear the per-session state the other hooks accumulate.
#
# Replaces the content-hash baseline this file used to build. That baseline
# existed to answer "did this session change the file?", and it answered it by
# comparing hashes — which really answers "did *anything* change the file?". The
# two diverge as soon as something else touches the checkout, and on 2026-09-06
# they did: the tree was clean at SessionStart, so the baseline was legitimately
# empty, then 47 files went dirty from outside the session, and every one of them
# was attributed to it. Five Stop reports, 29 files each, none of them the
# agent's.
#
# bin/edit-journal.sh answers the question directly by recording the writes as
# they happen, so the hashes are not needed and the whole failure mode is gone
# with them. Less code, one fewer thing to be subtly wrong.
#
# Also drops the stale target/.kiro-session-baseline left by the old scheme, so a
# checkout that ran the previous version does not keep a dead file around.
#
# Silent: SessionStart forwards stdout to the agent, and housekeeping is not news.
# Fails open — a failure here must not stop a session from starting.

set -uo pipefail

trap 'exit 0' ERR

repo=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
cd "$repo" 2>/dev/null || exit 0

mkdir -p target 2>/dev/null || exit 0

# Truncate rather than delete: the journal's consumers all treat "missing" and
# "empty" the same, and truncating keeps the file's permissions stable across a
# session.
: >target/.kiro-session-edits 2>/dev/null || true

# A report written at the end of the previous session was already flushed on its
# last prompt, or is about work that is no longer in scope. Either way it must not
# surface in this one.
rm -f target/.kiro-session-report target/.kiro-session-report.tmp 2>/dev/null || true

# Retired with the hash baseline.
rm -f target/.kiro-session-baseline target/.kiro-session-baseline.tmp 2>/dev/null || true

exit 0
