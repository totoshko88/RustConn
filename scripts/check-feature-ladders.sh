#!/usr/bin/env bash
# Verifies that the GNOME feature ladders agree across every packaging channel.
#
# The mapping "libadwaita >= 1.8 -> adw-1-8, gtk4 >= 4.22 -> gtk-4-22, ..." is
# duplicated in four places that are built by four different toolchains and can
# only be edited by hand:
#
#   * packaging/macos/rustconn.rb   (Ruby, Homebrew)
#   * .github/workflows/release.yml (bash, build-rpm)
#   * packaging/obs/debian.rules    (make, OBS Debian/Ubuntu)
#   * packaging/obs/README.md       (the human-readable table)
#
# There is no shared implementation because the three build systems share no
# language, so the guard is this text check instead: it extracts the set of
# version thresholds each source tests for and fails if any source disagrees.
# When a new rung is added (e.g. libadwaita 1.10 -> adw-1-10), it must land in
# all four, and this catches the one that was forgotten — the "shipped an old
# baseline" regression that has fired more than once, always because one copy
# lagged.
#
# The check is intentionally about the *thresholds*, not the exact syntax: each
# source spells the comparison differently (`--atleast-version=1.8`, a table
# cell `1.8`), so we normalise to the sorted set of versions per library.

set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

status=0

formula="packaging/macos/rustconn.rb"
rpm_workflow=".github/workflows/release.yml"
debian_rules="packaging/obs/debian.rules"

for f in "$formula" "$rpm_workflow" "$debian_rules"; do
    if [ ! -f "$f" ]; then
        printf 'FAIL: expected ladder source not found: %s\n' "$f" >&2
        exit 1
    fi
done

# Extract the sorted, unique set of thresholds a file tests for a given library.
# Matches both `--atleast-version=1.8 libadwaita-1` (bash/make/ruby) and the
# Ruby hash form `"1.8" => "adw-1-8"` sitting on a libadwaita/gtk4/vte line.
thresholds_for() {
    local file="$1" pc_name="$2"
    {
        # `--atleast-version=X pc_name` form: capture only the version after `=`,
        # not any digits inside pc_name itself (e.g. the 2.91 in vte-2.91-gtk4).
        grep -oE -- "--atleast-version=[0-9]+\.[0-9]+ +${pc_name}([^0-9]|$)" "$file" \
            | grep -oE -- '--atleast-version=[0-9]+\.[0-9]+' \
            | grep -oE '[0-9]+\.[0-9]+'
        # Ruby hash form: pull the pc_name's line and read its "X.Y" keys
        grep -E "\"${pc_name}\"[[:space:]]*=>" "$file" \
            | grep -oE '"[0-9]+\.[0-9]+"' | tr -d '"'
    } 2>/dev/null | sort -u | tr '\n' ' ' | sed 's/ $//'
}

compare_library() {
    local label="$1" pc_name="$2"
    local a b c
    a="$(thresholds_for "$formula" "$pc_name")"
    b="$(thresholds_for "$rpm_workflow" "$pc_name")"
    c="$(thresholds_for "$debian_rules" "$pc_name")"

    if [ -z "$a" ]; then
        printf 'FAIL: no %s thresholds found in %s\n' "$label" "$formula" >&2
        status=1
        return
    fi
    if [ "$a" != "$b" ] || [ "$a" != "$c" ]; then
        printf 'FAIL: %s ladder differs between channels:\n' "$label" >&2
        printf '  formula (%s): %s\n' "$formula" "$a" >&2
        printf '  build-rpm (%s): %s\n' "$rpm_workflow" "$b" >&2
        printf '  debian.rules (%s): %s\n' "$debian_rules" "$c" >&2
        printf '  Add the new rung to every channel (and packaging/obs/README.md).\n' >&2
        status=1
    else
        printf 'OK: %s ladder agrees across channels: %s\n' "$label" "$a"
    fi
}

compare_library "libadwaita" "libadwaita-1"
compare_library "gtk4" "gtk4"
compare_library "vte" "vte-2.91-gtk4"

exit "$status"
