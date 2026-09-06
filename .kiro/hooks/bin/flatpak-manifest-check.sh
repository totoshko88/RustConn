#!/usr/bin/env bash
# PostFileSave on Cargo.lock: note that the Flatpak vendored-source manifests are
# now older than the lockfile.
#
# The Flatpak build does not resolve dependencies itself — it consumes
# packaging/*/cargo-sources.json, generated from Cargo.lock. When the lockfile
# moves and those files do not, the Flatpak build either fails or, worse, builds
# the previous dependency set.
#
# This used to be an `agent` action: a prompt telling the model to check whether
# two files exist and print a fixed warning with two fixed commands in it. That is
# a full agent loop per Cargo.lock save to run `test -f` twice. The check is a
# timestamp comparison, so it belongs in a script, and the same PostFileSave that
# fired the prompt fires this for nothing.
#
# Delivery goes through target/.kiro-session-report, because a command hook's
# stdout is discarded on PostFileSave; the UserPromptSubmit flush hook prints it
# on the next turn. See bin/session-report.sh for the shared-channel contract.
#
# Deliberately does NOT regenerate anything. cargo-sources.json files are large
# generated artefacts and regenerating them is an intentional act before a
# Flatpak release, not a side effect of touching a dependency.
#
# Fails open and silent.

set -uo pipefail

trap 'exit 0' ERR

repo=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
cd "$repo" 2>/dev/null || exit 0

lock="Cargo.lock"
[ -f "$lock" ] || exit 0

stale=""
for sources in packaging/flatpak/cargo-sources.json packaging/flathub/cargo-sources.json; do
    # Not generated yet -> nothing is out of date. A first generation is a
    # packaging decision, not a staleness warning.
    [ -f "$sources" ] || continue
    if [ "$lock" -nt "$sources" ]; then
        stale="$stale  $sources"$'\n'
    fi
done

[ -n "$stale" ] || exit 0

report="target/.kiro-session-report"
mkdir -p target 2>/dev/null || exit 0

{
    printf 'STALE FLATPAK SOURCES: Cargo.lock is newer than:\n'
    printf '%s' "$stale"
    printf 'Regenerate before the next Flatpak release (not now, unless asked):\n'
    printf '  python3 packaging/flatpak/flatpak-cargo-generator.py Cargo.lock -o packaging/flatpak/cargo-sources.json\n'
    printf '  python3 packaging/flatpak/flatpak-cargo-generator.py Cargo.lock -o packaging/flathub/cargo-sources.json\n'
} >>"$report" 2>/dev/null || true

exit 0
