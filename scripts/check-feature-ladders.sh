#!/usr/bin/env bash
# Verifies that the GNOME feature ladders agree across every packaging channel.
#
# The mapping "libadwaita >= 1.8 -> adw-1-8, gtk4 >= 4.22 -> gtk-4-22, ..." is
# duplicated in seven places that are built by different toolchains and can only
# be edited by hand. Four of them detect the version and pick a rung:
#
#   * packaging/macos/rustconn.rb   (Ruby, Homebrew)
#   * .github/workflows/release.yml (bash, build-rpm)
#   * packaging/obs/debian.rules    (make, OBS Debian/Ubuntu)
#   * packaging/obs/README.md       (the human-readable table)
#
# The other three do not detect anything — they hardcode the feature list,
# because the Flatpak SDK version is pinned and therefore known in advance:
#
#   * packaging/flatpak/io.github.totoshko88.RustConn.yml
#   * packaging/flatpak/io.github.totoshko88.RustConn.local.yml
#   * packaging/flathub/io.github.totoshko88.RustConn.yml
#
# There is no shared implementation because the build systems share no language,
# so the guard is this text check instead. When a new rung is added (e.g.
# libadwaita 1.10 -> adw-1-10), it must land everywhere, and this catches the copy
# that was forgotten — the "shipped an old baseline" regression that has fired
# more than once, always because one copy lagged. The Flatpak manifests are
# checked because they are where it last happened: they passed `--features
# adw-1-8` and nothing else, so `vte-0-78` was never enabled in the channel most
# users install from.
#
# Two different questions are asked, because the two groups say different things:
#
#   detecting sources -> must agree on the *set of version thresholds*. Each
#     spells the comparison differently (`--atleast-version=1.8`, a table cell
#     `≥ 1.8`), so they are normalised to a sorted set of versions per library.
#   hardcoding sources -> must name the *highest rung's feature*. A manifest that
#     names a lower rung is building an older baseline than the ladder offers.

set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

status=0

formula="packaging/macos/rustconn.rb"
rpm_workflow=".github/workflows/release.yml"
debian_rules="packaging/obs/debian.rules"
obs_readme="packaging/obs/README.md"

# Manifests that hardcode the feature list rather than detecting a version.
flatpak_manifests="
packaging/flatpak/io.github.totoshko88.RustConn.yml
packaging/flatpak/io.github.totoshko88.RustConn.local.yml
packaging/flathub/io.github.totoshko88.RustConn.yml
"

for f in "$formula" "$rpm_workflow" "$debian_rules" "$obs_readme" $flatpak_manifests; do
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
        # Markdown table form: `| `adw-1-8` … | `libadwaita-1` ≥ 1.8 / 1.7 / 1.6 |`.
        # Only the text after `≥` is read, so the 2.91 in the vte package name
        # cannot be mistaken for a threshold.
        grep -E "\`${pc_name}\`[^|]*≥" "$file" \
            | sed 's/.*≥//' \
            | grep -oE '[0-9]+\.[0-9]+'
    } 2>/dev/null | sort -V -u | tr '\n' ' ' | sed 's/ $//'
}

# The feature a channel must name for the ladder's highest rung: `libadwaita-1`
# at 1.8 -> `adw-1-8`. The prefix cannot be derived from the pc-config name
# (`libadwaita-1` -> `adw`, `vte-2.91-gtk4` -> `vte`), so callers pass it.
top_feature_for() {
    local prefix="$1" thresholds="$2"
    local highest
    # `sort -V` above puts the newest last, so 1.10 ranks above 1.6 — which plain
    # lexicographic sorting gets wrong, and is the exact version that broke a
    # glob-based check before.
    highest="$(printf '%s\n' $thresholds | tail -1)"
    printf '%s-%s' "$prefix" "$(printf '%s' "$highest" | tr '.' '-')"
}

compare_library() {
    local label="$1" pc_name="$2" feature_prefix="$3"
    local a b c d
    a="$(thresholds_for "$formula" "$pc_name")"
    b="$(thresholds_for "$rpm_workflow" "$pc_name")"
    c="$(thresholds_for "$debian_rules" "$pc_name")"
    d="$(thresholds_for "$obs_readme" "$pc_name")"

    if [ -z "$a" ]; then
        printf 'FAIL: no %s thresholds found in %s\n' "$label" "$formula" >&2
        status=1
        return
    fi
    if [ "$a" != "$b" ] || [ "$a" != "$c" ] || [ "$a" != "$d" ]; then
        printf 'FAIL: %s ladder differs between channels:\n' "$label" >&2
        printf '  formula (%s): %s\n' "$formula" "$a" >&2
        printf '  build-rpm (%s): %s\n' "$rpm_workflow" "$b" >&2
        printf '  debian.rules (%s): %s\n' "$debian_rules" "$c" >&2
        printf '  obs README (%s): %s\n' "$obs_readme" "$d" >&2
        printf '  Add the new rung to every one of them.\n' >&2
        status=1
        return
    fi

    printf 'OK: %s ladder agrees across detecting channels: %s\n' "$label" "$a"

    # The Flatpak manifests detect nothing — they must spell out the top rung.
    local feature
    feature="$(top_feature_for "$feature_prefix" "$a")"
    local manifest
    for manifest in $flatpak_manifests; do
        if grep -q -- "$feature" "$manifest"; then
            printf 'OK: %s names %s\n' "$manifest" "$feature"
        else
            printf 'FAIL: %s does not enable %s\n' "$manifest" "$feature" >&2
            printf '  The manifest hardcodes its --features list, so a new rung has\n' >&2
            printf '  to be added there by hand. Without it the Flatpak build ships an\n' >&2
            printf '  older baseline than every other channel, silently.\n' >&2
            printf '  Current --features line:\n' >&2
            grep -nE -- '--features' "$manifest" | sed 's/^/    /' >&2
            status=1
        fi
    done
}

compare_library "libadwaita" "libadwaita-1" "adw"
compare_library "gtk4" "gtk4" "gtk"
compare_library "vte" "vte-2.91-gtk4" "vte"

exit "$status"
