#!/usr/bin/env bash
# bw-serve-probe.sh — G1 spike go/no-go probe for the 0.24 Bitwarden `bw serve` work.
#
# Runs ON THE MAINTAINER'S MACHINE against a real `bw` CLI and a real vault,
# because the agent sandbox has no vault credentials and snap-confine refuses
# `bw` there. It measures the four numbers the 0.24 plan gates on and prints a
# go/no-go verdict. It never stores or echoes the master password beyond the one
# `POST /unlock` body, and it locks and kills the serve process on exit.
#
# Usage:
#   scripts/bw-serve-probe.sh                 # prompts for the master password
#   BW_TEST_SEARCH="github" scripts/bw-serve-probe.sh   # pick the search term
#
# Go thresholds (plan §Цілі): unlock <= 4 s, first retrieve with root search <= 1 s.
#
# This is a SPIKE artefact on the 0.24.0-bw-serve-spike branch. It is not wired
# into verify.sh and is deleted (or promoted) once the go/no-go decision lands.

set -u

BW="${BW:-bw}"
PORT="${BW_SERVE_PORT:-8087}"
HOST="127.0.0.1"
SEARCH="${BW_TEST_SEARCH:-}"

say()  { printf '%s\n' "$*"; }
fail() { printf 'FAIL: %s\n' "$*" >&2; }

# ── Preconditions ─────────────────────────────────────────────────────────
if ! command -v "$BW" >/dev/null 2>&1; then
  fail "bw not found on PATH (set BW=/path/to/bw)"; exit 2
fi
if ! command -v curl >/dev/null 2>&1; then
  fail "curl is required"; exit 2
fi

BW_VERSION="$("$BW" --version 2>/dev/null | tr -d '\r')"
say "bw version:            ${BW_VERSION:-unknown}"

# `fd+connected://` support is the risky assumption — probe the help text.
if "$BW" serve --help 2>&1 | grep -qi 'fd+connected\|--hostname'; then
  say "serve --hostname:      present in --help"
else
  say "serve --hostname:      NOT advertised in --help (note the risk)"
fi

# ── Timing helper (ms, portable) ────────────────────────────────────────────
now_ms() { python3 - <<'PY'
import time; print(int(time.time()*1000))
PY
}

# ── Start bw serve ──────────────────────────────────────────────────────────
# A plain TCP serve on loopback is enough to MEASURE unlock + retrieve latency
# and RSS; the production transport uses socketpair (`fd+connected://`), which
# the Rust prototype exercises. Here we only need numbers.
say ""
say "Starting: $BW serve --hostname $HOST --port $PORT"
"$BW" serve --hostname "$HOST" --port "$PORT" >/tmp/bw-serve-probe.out 2>&1 &
SERVE_PID=$!

cleanup() {
  # Best-effort lock then kill — never leave an unlocked vault behind.
  curl -s -X POST "http://$HOST:$PORT/lock" >/dev/null 2>&1 || true
  kill "$SERVE_PID" >/dev/null 2>&1 || true
  wait "$SERVE_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# Wait for the HTTP endpoint to answer (max ~5 s).
for _ in $(seq 1 50); do
  if curl -s "http://$HOST:$PORT/status" >/dev/null 2>&1; then break; fi
  sleep 0.1
done

# ── RSS of the serve process ────────────────────────────────────────────────
if [ -r "/proc/$SERVE_PID/status" ]; then
  RSS_KB="$(awk '/VmRSS/{print $2}' "/proc/$SERVE_PID/status" 2>/dev/null)"
  say "serve RSS:             ${RSS_KB:-?} KB"
else
  say "serve RSS:             (not readable on this OS)"
fi

# ── Unlock timing ─────────────────────────────────────────────────────────
printf 'Master password (not echoed, used once for POST /unlock): '
stty -echo 2>/dev/null; read -r MASTER; stty echo 2>/dev/null; printf '\n'

T0="$(now_ms)"
UNLOCK_JSON="$(curl -s -X POST "http://$HOST:$PORT/unlock" \
  -H 'Content-Type: application/json' \
  -d "{\"password\":\"$MASTER\"}" 2>/dev/null)"
T1="$(now_ms)"
MASTER=""  # drop it
UNLOCK_MS=$((T1 - T0))

if printf '%s' "$UNLOCK_JSON" | grep -q '"success":true'; then
  say "unlock:                ${UNLOCK_MS} ms  (success)"
else
  MSG="$(printf '%s' "$UNLOCK_JSON" | sed 's/.*"message":"\([^"]*\)".*/\1/')"
  say "unlock:                ${UNLOCK_MS} ms  (FAILED: ${MSG:-see /tmp/bw-serve-probe.out})"
fi

# ── Retrieve timing (list with search = root search) ────────────────────────
if [ -z "$SEARCH" ]; then
  printf 'Search term for the retrieve probe (e.g. a host or item name): '
  read -r SEARCH
fi
T0="$(now_ms)"
LIST_JSON="$(curl -s "http://$HOST:$PORT/list/object/items?search=$SEARCH" 2>/dev/null)"
T1="$(now_ms)"
RETRIEVE_MS=$((T1 - T0))
COUNT="$(printf '%s' "$LIST_JSON" | grep -o '"id"' | wc -l | tr -d ' ')"
say "retrieve (root search):${RETRIEVE_MS} ms  (${COUNT} items for \"$SEARCH\")"

# ── Verdict ─────────────────────────────────────────────────────────────────
say ""
say "── go/no-go (plan thresholds: unlock <= 4000 ms, retrieve <= 1000 ms) ──"
GO=1
[ "${UNLOCK_MS:-999999}" -le 4000 ]   && say "  unlock   <= 4 s   : PASS (${UNLOCK_MS} ms)"   || { say "  unlock   <= 4 s   : FAIL (${UNLOCK_MS} ms)"; GO=0; }
[ "${RETRIEVE_MS:-999999}" -le 1000 ] && say "  retrieve <= 1 s   : PASS (${RETRIEVE_MS} ms)" || { say "  retrieve <= 1 s   : FAIL (${RETRIEVE_MS} ms)"; GO=0; }
say ""
if [ "$GO" -eq 1 ]; then
  say "VERDICT: GO — bw serve meets the latency thresholds on this environment."
else
  say "VERDICT: NO-GO (on these numbers) — review /tmp/bw-serve-probe.out before deciding."
fi
say ""
say "Re-run on each target environment the plan names: snap bw, Flatpak bw,"
say "macOS Homebrew, and a Vaultwarden server (watch for KeyIdBackfillError)."
