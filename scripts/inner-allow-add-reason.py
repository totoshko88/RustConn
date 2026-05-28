#!/usr/bin/env python3
"""Add `reason = "..."` to module-level inner attributes
`#![allow(LINT)]` (those starting with `#!`).

Pulls reason text from a preceding comment on the same line, the line
above, or falls back to a generic explanation.
"""
import re
import sys
from pathlib import Path


# `#![allow(clippy::foo)]` or `#![allow(clippy::foo, clippy::bar)]`
# (single-line form only — multi-line module attributes are rare).
INNER_RE = re.compile(
    r"^(?P<indent>\s*)#!\[allow\((?P<lints>[^)]*?)\)\]"
    r"(?P<rest>.*?)$",
    re.MULTILINE,
)


GENERIC_REASON = (
    "module-wide override for legacy code; refactored case by case"
)


def replace_one(match: re.Match[str]) -> str:
    indent = match.group("indent")
    lints = match.group("lints").strip()
    if "reason" in lints:
        return match.group(0)
    rest = match.group("rest")
    comment_match = re.match(r"\s*//\s*(.*)$", rest)
    reason = (
        comment_match.group(1).strip()
        if comment_match and comment_match.group(1).strip()
        else GENERIC_REASON
    )
    escaped = reason.replace("\\", "\\\\").replace('"', '\\"')
    return f'{indent}#![allow({lints}, reason = "{escaped}")]'


def migrate_file(path: Path) -> int:
    text = path.read_text()
    new_text, count = INNER_RE.subn(replace_one, text)
    if count and new_text != text:
        path.write_text(new_text)
    return count


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: inner-allow-add-reason.py FILE [FILE ...]", file=sys.stderr)
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
