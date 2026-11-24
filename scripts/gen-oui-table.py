#!/usr/bin/env python3
"""Regenerate rustconn-core/src/oui_data.rs from the IEEE MA-L public listing.

    curl -fsSL -o /tmp/oui.csv https://standards-oui.ieee.org/oui/oui.csv
    python3 scripts/gen-oui-table.py /tmp/oui.csv

The listing is factual registry data (no licence text in the file; Debian
ships the same data as `ieee-data`). Only MA-L 24-bit OUIs are taken: MA-M,
MA-S and IAB blocks are 28/36-bit and need mask matching the lookup does
not have yet.
"""

import csv
import sys
import unicodedata

OUT = "rustconn-core/src/oui_data.rs"
HEADER = """\
//! Generated OUI -> vendor table. DO NOT EDIT BY HAND.
//!
//! Regenerate with `scripts/gen-oui-table.py` from the IEEE MA-L public
//! listing (<https://standards-oui.ieee.org/oui/oui.csv>, fetched {date},
//! {count} assignments). The listing is factual registry data — no licence
//! text in the file, and the same data has shipped in Debian's `ieee-data`
//! for years. Vendor spellings are upstream's own.

/// Every MA-L (24-bit OUI) assignment as `(prefix, vendor)` pairs,
/// sorted by prefix.
#[rustfmt::skip]
pub(crate) const MA_L: &[(&str, &str)] = &[
"""


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit(f"usage: {sys.argv[0]} oui.csv")
    rows: list[tuple[str, str]] = []
    with open(sys.argv[1], encoding="utf-8", newline="") as fh:
        for row in csv.DictReader(fh):
            if row["Registry"] != "MA-L":
                continue
            prefix = row["Assignment"].strip().upper()
            if len(prefix) != 6 or any(c not in "0123456789ABCDEF" for c in prefix):
                continue
            name = row["Organization Name"].strip()
            # clippy::unicode_not_nfc is denied workspace-wide; keep
            # upstream's letters but in their composed (NFC) form. Format
            # and soft characters (zero-width spaces, BOM, direction
            # marks) are dropped — clippy::invisible_characters.
            name = unicodedata.normalize("NFC", name)
            name = "".join(
                c for c in name
                if unicodedata.category(c) != "Cf" and c not in "\u200b\u00ad"
            ).strip()
            if not name:
                continue
            rows.append((prefix, name.replace("\\", "\\\\").replace('"', '\\"')))
    rows.sort()
    with open(OUT, "w", encoding="utf-8") as out:
        out.write(HEADER.format(date="2026-11-24", count=len(rows)))
        for prefix, name in rows:
            out.write(f'    ("{prefix}", "{name}"),\n')
        out.write("];\n")
    print(f"{OUT}: {len(rows)} entries")


if __name__ == "__main__":
    main()
