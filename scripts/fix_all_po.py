#!/usr/bin/env python3
"""
Fix all .po files by removing duplicate msgid entries.
Keeps the FIRST occurrence of each msgid and removes later duplicates.
"""
import os
import re
import subprocess

po_dir = os.path.dirname(os.path.abspath(__file__))

def parse_and_deduplicate(filepath):
    """Parse a .po file, remove duplicate msgid entries, return cleaned content."""
    with open(filepath, 'r', encoding='utf-8') as f:
        content = f.read()

    # Split into entries (separated by blank lines)
    blocks = re.split(r'\n{2,}', content)

    seen_msgids = set()
    result_blocks = []
    duplicates_removed = 0

    for block in blocks:
        block = block.strip()
        if not block:
            continue

        # Extract msgid from this block
        msgid_match = re.search(r'^msgid\s+"(.+)"$', block, re.MULTILINE)
        if msgid_match:
            msgid_value = msgid_match.group(1)
            if msgid_value in seen_msgids:
                duplicates_removed += 1
                continue
            seen_msgids.add(msgid_value)

        result_blocks.append(block)

    cleaned = '\n\n'.join(result_blocks) + '\n'
    return cleaned, duplicates_removed


def check_msgfmt(filepath):
    """Run msgfmt --check and return (success, error_output)."""
    try:
        result = subprocess.run(
            ['msgfmt', '--check', '--output-file=/dev/null', filepath],
            capture_output=True, text=True, timeout=10
        )
        return result.returncode == 0, result.stderr
    except (subprocess.TimeoutExpired, FileNotFoundError):
        return True, ""


# Process all .po files
po_files = sorted([f for f in os.listdir(po_dir) if f.endswith('.po')])

print(f"Processing {len(po_files)} .po files...")

for filename in po_files:
    filepath = os.path.join(po_dir, filename)
    cleaned, dupes = parse_and_deduplicate(filepath)

    if dupes > 0:
        with open(filepath, 'w', encoding='utf-8') as f:
            f.write(cleaned)
        print(f"  {filename}: removed {dupes} duplicate(s)")
    else:
        pass

    success, errors = check_msgfmt(filepath)
    if not success:
        error_lines = [l for l in errors.strip().split('\n') if l.strip()]
        if error_lines:
            print(f"  {filename}: msgfmt still reports issues:")
            for line in error_lines[:5]:
                print(f"    {line}")
        else:
            print(f"  {filename}: msgfmt failed (no details)")
    elif dupes == 0:
        print(f"  {filename}: OK")

print("\nDone.")
