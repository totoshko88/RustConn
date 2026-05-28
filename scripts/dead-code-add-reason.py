#!/usr/bin/env python3
"""Convert `#[allow(dead_code)] // <comment>` and bare `#[allow(dead_code)]`
to `#[allow(dead_code, reason = "<comment>")]` so the workspace can enable
`clippy::allow_attributes_without_reason`.

For bare `#[allow(dead_code)]` without a comment, uses a generic reason.
For multi-allow forms (e.g. `#[allow(dead_code, unreachable_code)]`) the
comment becomes the reason for the whole list.
"""
import re
import sys
from pathlib import Path


# Match `#[allow(...)] // optional comment` on a single line.
# Capture groups: 1 = lints, 2 = optional inline comment text (without leading //)
ALLOW_RE = re.compile(
    r"^(?P<indent>\s*)#\[allow\((?P<lints>[^)]*?)\)\]"
    r"(?P<rest>.*?)$",
    re.MULTILINE,
)


GENERIC_REASON = (
    "kept alive for GTK widget lifecycle / future API exposure"
)


def needs_reason(lints: str) -> bool:
    return "reason" not in lints


def escape(text: str) -> str:
    return text.replace("\\", "\\\\").replace('"', '\\"')


def replace_one(match: re.Match[str]) -> str:
    indent = match.group("indent")
    lints = match.group("lints").strip()
    rest = match.group("rest")
    if not needs_reason(lints):
        return match.group(0)
    # Pull a trailing line comment if present.
    comment_match = re.match(r"\s*//\s*(.*)$", rest)
    reason = (
        comment_match.group(1).strip()
        if comment_match and comment_match.group(1).strip()
        else GENERIC_REASON
    )
    return f'{indent}#[allow({lints}, reason = "{escape(reason)}")]'


def migrate_file(path: Path) -> int:
    text = path.read_text()
    new_text, count = ALLOW_RE.subn(replace_one, text)
    if count and new_text != text:
        path.write_text(new_text)
    return count


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: dead-code-add-reason.py FILE [FILE ...]", file=sys.stderr)
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
