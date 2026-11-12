#!/usr/bin/env bash
# Headless GUI smoke test: `rustconn --version`, then a 10 s launch that must
# neither panic nor exit early. Run it under a display:
#   xvfb-run -a scripts/gui-smoke.sh [path/to/rustconn]
#
# The launch is killed by `timeout` (exit 124) — that is the pass. Any other exit
# before the deadline means the app died on startup.

set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

bin="${1:-${CARGO_TARGET_DIR:-target}/debug/rustconn}"
[ -x "$bin" ] || { printf 'FAIL: %s not found or not executable\n' "$bin" >&2; exit 1; }

export GDK_BACKEND=x11 GTK_A11Y=none
# Never touch the real profile of whoever runs this.
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export XDG_CONFIG_HOME="$tmp/config" XDG_DATA_HOME="$tmp/data" XDG_CACHE_HOME="$tmp/cache"

"$bin" --version || { printf 'FAIL: --version\n' >&2; exit 1; }

log="$tmp/launch.log"
timeout 10 "$bin" >"$log" 2>&1
code=$?
cat "$log"

if grep -qE "panicked at" "$log"; then
    printf 'FAIL: panic during launch\n' >&2
    exit 1
fi
if [ "$code" -ne 124 ]; then
    printf 'FAIL: exited with %d before the 10 s deadline\n' "$code" >&2
    exit 1
fi
printf 'ok: launched for 10 s, no panic\n'
