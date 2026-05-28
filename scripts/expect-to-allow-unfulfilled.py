#!/usr/bin/env python3
"""For each unfulfilled #[expect(LINT, reason=...)] reported by clippy, downgrade
that block to #[allow(LINT, reason=...)] so the lint stays suppressed without
emitting `unfulfilled_lint_expectations`.

Usage: cargo clippy --workspace --all-targets 2>&1 | scripts/expect-to-allow-unfulfilled.py
"""
import re
import sys
from pathlib import Path
from collections import defaultdict


# Match warning blocks of the form
#   warning: this lint expectation is unfulfilled
#      --> path:line:col
LOC_RE = re.compile(r"--> (\S+\.rs):(\d+):(\d+)")


def collect_unfulfilled(text: str) -> dict[Path, set[int]]:
    """Return a mapping of file -> set of (1-based) line numbers where unfulfilled
    expectations were reported."""
    by_file: dict[Path, set[int]] = defaultdict(set)
    lines = text.splitlines()
    for i, line in enumerate(lines):
        if "unfulfilled" not in line:
            continue
        # Walk forward until we hit the --> location line.
        for j in range(i + 1, min(i + 6, len(lines))):
            m = LOC_RE.search(lines[j])
            if m:
                by_file[Path(m.group(1))].add(int(m.group(2)))
                break
    return by_file


def downgrade_block(file_lines: list[str], line_no_1based: int) -> bool:
    """Find the #[expect(...)] block enclosing `line_no_1based` and rewrite to
    #[allow(...)]. Returns True if a change was made."""
    idx = line_no_1based - 1
    # Walk upward until we find the opening `#[expect(`. The expect block is
    # multiline; the lint name is on its own line, the opening attr is above.
    open_idx = None
    for k in range(idx, max(idx - 10, -1), -1):
        if "#[expect(" in file_lines[k]:
            open_idx = k
            break
    if open_idx is None:
        return False
    # Replace `#[expect(` with `#[allow(` on that single line.
    new_line = file_lines[open_idx].replace("#[expect(", "#[allow(", 1)
    if new_line == file_lines[open_idx]:
        return False
    file_lines[open_idx] = new_line
    return True


def main() -> int:
    text = sys.stdin.read()
    by_file = collect_unfulfilled(text)
    total = 0
    for path, lines in by_file.items():
        if not path.is_file():
            continue
        file_lines = path.read_text().splitlines(keepends=True)
        # Process from highest line numbers downward so earlier edits don't
        # shift later positions (although in our case we never insert/remove
        # lines, the order doesn't strictly matter).
        for ln in sorted(lines, reverse=True):
            if downgrade_block(file_lines, ln):
                total += 1
        path.write_text("".join(file_lines))
    print(f"Downgraded {total} expect block(s) to allow.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
