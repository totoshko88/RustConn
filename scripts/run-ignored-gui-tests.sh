#!/usr/bin/env bash
# Runs every `#[ignore]`d GTK test of the `rustconn` binary, one process per test.
#
# These tests initialise GTK, which can happen once per process and needs a
# display, so `cargo test` skips them by default. Nothing else runs them: without
# this script a regression in the VTE contract they pin is invisible
# (fractional_scroll_position_is_kept failed for a whole release that way).
#
# Needs a display. In CI and on a headless box wrap it:
#   xvfb-run -a scripts/run-ignored-gui-tests.sh
#
# Exit codes: 0 all passed, 1 a test failed or none were found.

set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

# A11y bus lookups only add noise and a startup delay without a session bus.
export GTK_A11Y="${GTK_A11Y:-none}"

run=(cargo test -p rustconn --bin rustconn --)
mapfile -t tests < <("${run[@]}" --ignored --list 2>/dev/null | sed -n 's/: test$//p')
if [ "${#tests[@]}" -eq 0 ]; then
    printf 'FAIL: no ignored tests found (did the list format change?)\n' >&2
    exit 1
fi

failed=0
for t in "${tests[@]}"; do
    if out=$("${run[@]}" --ignored --exact "$t" 2>&1); then
        printf 'ok    %s\n' "$t"
    else
        printf 'FAIL  %s\n%s\n' "$t" "$out"
        failed=$((failed + 1))
    fi
done

printf '%d ignored GTK test(s), %d failed\n' "${#tests[@]}" "$failed"
[ "$failed" -eq 0 ]
