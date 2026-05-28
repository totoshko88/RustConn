#!/usr/bin/env python3
"""Round set_margin_* / set_spacing literal values to the nearest GNOME HIG
step (6 / 12 / 18 / 24).

Mapping (round up to next HIG step that is >= input):
  0, 2, 3, 4, 5, 6 -> 6
  7, 8, 9, 10, 11, 12 -> 12
  13..18 -> 18
  19..24 -> 24

Anything already in {6, 12, 18, 24} or 0 stays unchanged.
"""
import re
import sys
from pathlib import Path


PATTERN = re.compile(
    r"\.set_(margin_top|margin_bottom|margin_start|margin_end|margin|spacing)"
    r"\((\d+)\)"
)


def round_to_hig(value: int) -> int:
    if value == 0 or value in {6, 12, 18, 24}:
        return value
    if value <= 6:
        return 6
    if value <= 12:
        return 12
    if value <= 18:
        return 18
    return 24


def replace_one(match: re.Match[str]) -> str:
    method = match.group(1)
    raw = int(match.group(2))
    target = round_to_hig(raw)
    if target == raw:
        return match.group(0)
    return f".set_{method}({target})"


def migrate_file(path: Path) -> int:
    text = path.read_text()
    new_text, count = PATTERN.subn(replace_one, text)
    if count and new_text != text:
        path.write_text(new_text)
    return count


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: hig-spacing.py FILE [FILE ...]", file=sys.stderr)
        return 1
    total = 0
    for arg in sys.argv[1:]:
        path = Path(arg)
        if not path.is_file():
            continue
        n = migrate_file(path)
        if n:
            print(f"  {path}: rewrote {n}")
        total += n
    print(f"Total rewritten: {total}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
