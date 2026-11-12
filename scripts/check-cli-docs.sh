#!/usr/bin/env bash
# Every rustconn-cli subcommand must be documented in docs/CLI_REFERENCE.md.
#
# Reads the command tree from the binary's own `--help`, so a new subcommand
# fails here until it has a doc entry (`codec` shipped undocumented in 0.24):
#
#   * each top-level command needs a `### <name> — …` heading
#     (`### pin / unpin — …` covers both names);
#   * each nested command needs its `<parent> <child>` text somewhere in the file.
#
# Usage: scripts/check-cli-docs.sh [path/to/rustconn-cli]
# Without an argument it uses $RUSTCONN_CLI, else builds the debug binary with
# cargo (honouring CARGO_TARGET_DIR).
#
# ponytail: two levels deep, which is all the CLI has today; recurse if a third
# level appears.

set -euo pipefail

cd "$(dirname "$0")/.." || exit 1

doc="docs/CLI_REFERENCE.md"
bin="${1:-${RUSTCONN_CLI:-}}"

if [ -z "$bin" ]; then
    CARGO="${CARGO:-$HOME/.cargo/bin/cargo}"
    command -v "$CARGO" >/dev/null 2>&1 || CARGO=cargo
    "$CARGO" build --quiet -p rustconn-cli
    bin="${CARGO_TARGET_DIR:-target}/debug/rustconn-cli"
fi

if [ ! -x "$bin" ]; then
    printf 'check-cli-docs: binary not found: %s\n' "$bin" >&2
    exit 2
fi

# Prints the subcommand names from the "Commands:" block of `--help`.
subcommands() {
    "$bin" "$@" --help |
        awk '/^Commands:/ { in_block = 1; next }
             in_block && /^[^ ]/ { in_block = 0 }
             in_block && /^  [a-z]/ { print $1 }' |
        grep -vx 'help' || true
}

missing=0
top=$(subcommands)
if [ -z "$top" ]; then
    printf 'check-cli-docs: no subcommands parsed from %s --help\n' "$bin" >&2
    exit 2
fi

for cmd in $top; do
    if ! grep -qE "^### ([a-z-]+ / )*${cmd}( |$)" "$doc"; then
        printf 'missing heading in %s: ### %s\n' "$doc" "$cmd"
        missing=$((missing + 1))
    fi
    for sub in $(subcommands "$cmd"); do
        if ! grep -qF "$cmd $sub" "$doc"; then
            printf 'missing in %s: %s %s\n' "$doc" "$cmd" "$sub"
            missing=$((missing + 1))
        fi
    done
done

if [ "$missing" -gt 0 ]; then
    printf '%d undocumented subcommand(s).\n' "$missing"
    exit 1
fi
printf 'check-cli-docs: every subcommand is documented in %s.\n' "$doc"
