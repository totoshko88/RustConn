#!/usr/bin/env bash
# Stop-hook body: sync the KiroGraph index if a dirty marker is present.
#
# Why a script and not the inline command it replaced (2026-09-28). Three faults,
# all measured on this repo:
#
#   1. It ran in the hook's foreground. A sync is ~3-4 min even with nothing
#      changed (full scan + resolve of ~47k symbols) and ~20 min after a large
#      branch. The hook JSON carried its `timeout` inside `action`, where the
#      engine ignores it, so the effective cap was the 60 s default — well under
#      a sync. The .kirograph/hook.log for a whole week held nothing but four
#      "Database is locked" lines and no successful sync: every run was cut off
#      mid-flight, and the killed process left the lock behind, which is fault 2.
#   2. `pgrep -f 'kirograph [s]ync'` was meant to skip when a sync is already
#      running, but `-f` matches the whole argument line, so it also matched this
#      very hook's own shell (whose argv contains `kirograph sync-if-dirty`) and
#      any editor or grep with that string on its command line. A false match
#      makes the hook skip the sync it was supposed to run. Matching the process
#      *name* — the kirograph binary — instead cannot match a shell or a grep.
#   3. Every log line was undated and unlabelled, so "did the Stop sync ever
#      succeed?" could not be answered from the log. Each run now brackets itself
#      with an ISO timestamp and the hook name.
#
# 2026-10-05, two more, found by measuring rather than reading:
#
#   4. This file was committed 100644. The engine execs it, gets EACCES (exit
#      126) and moves on in milliseconds, so from 2026-09-28 on not one Stop sync
#      ran — hook.log has no `kirograph-sync: start` line at all, and every
#      "Database is locked" in it predates this script. scripts/test-hooks.sh now
#      asserts every hook-referenced script is executable.
#   5. It ran `kirograph unlock` unconditionally before syncing. In kirograph
#      1.3.0 that deletes `.kirograph/kirograph.lock`, the PROCESS lock — which a
#      live writer (the long-lived `serve --mcp`, mid edit-time sync) may hold —
#      and never touches `kirograph.db.lock`, the file behind "Database is locked".
#      So it could only do harm. A stale process lock needs no help:
#      LockManager.acquire() ignores one whose PID is dead or older than 5 min.
#      A present `kirograph.db.lock` makes GraphDatabase refuse to open, so the
#      sync is skipped with a dated log line instead of an "Uncaught error".
#
# The `done rc=` line used to print `$?` of the preceding printf, i.e. always 0;
# it now records the sync's own status.
#
# Detached with setsid so it outlives the turn, and the JSON now sets
# `timeout: 0` (disables the cap) as defence in depth. Low priority, since it is
# a background convenience, not on any critical path. Fails open and silent — a
# graph that is one turn stale is a documented, tolerable state (kirograph.md);
# a Stop hook that blocks the turn is not.

set -uo pipefail
trap 'exit 0' ERR

cd "$(git rev-parse --show-toplevel 2>/dev/null || pwd)" || exit 0

log=.kirograph/hook.log
mkdir -p .kirograph 2>/dev/null || exit 0

# Nothing to do unless something marked the graph dirty.
[ -f .kirograph/dirty ] || exit 0

# The DB lock makes every open fail; say so once instead of a stack trace.
if [ -e .kirograph/kirograph.db.lock ]; then
    printf '[%s] kirograph-sync: skipped, .kirograph/kirograph.db.lock exists (see kirograph.md)\n' \
        "$(date -Is 2>/dev/null || date)" >>"$log" 2>&1
    exit 0
fi

# Already syncing? Match the process by name, not by a substring of its argv, so
# this hook's own shell and any grep/editor mentioning "kirograph sync" do not
# count as a running sync. pgrep -x needs the exact program name; kirograph is a
# node CLI, so the process is usually `node`, and `pgrep -f` on the resolved
# binary path is the closest reliable check. Exclude our own PID and children.
if pgrep -f "kirograph.*sync-if-dirty" 2>/dev/null | grep -qv "^$$\$"; then
    printf '[%s] kirograph-sync: skipped, a sync is already running\n' \
        "$(date -Is 2>/dev/null || date)" >>"$log" 2>&1
    exit 0
fi

# Do the work in the background so the turn is not held. setsid detaches it from
# the hook's process group, so the engine reaping the hook does not kill the sync
# (fault 1). Everything it prints is bracketed and dated in the log.
setsid nohup sh -c '
    log=.kirograph/hook.log
    stamp() { date -Is 2>/dev/null || date; }
    printf "[%s] kirograph-sync: start\n" "$(stamp)" >>"$log" 2>&1
    nice -n 15 kirograph sync-if-dirty --quiet >>"$log" 2>&1
    rc=$?
    printf "[%s] kirograph-sync: done rc=%d\n" "$(stamp)" "$rc" >>"$log" 2>&1
' >/dev/null 2>&1 &

exit 0
