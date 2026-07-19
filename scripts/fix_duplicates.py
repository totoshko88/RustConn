#!/usr/bin/env python3
"""Remove duplicate Reload/Disconnected entries appended at end of .po files."""
import os

po_dir = os.path.dirname(os.path.abspath(__file__))
files = [
    'de.po', 'fr.po', 'es.po', 'it.po', 'pl.po', 'cs.po', 'sk.po',
    'da.po', 'sv.po', 'nl.po', 'pt.po', 'be.po', 'kk.po', 'uz.po', 'zh-cn.po'
]

for f in files:
    path = os.path.join(po_dir, f)
    with open(path, 'r') as fh:
        lines = fh.readlines()
    # Remove last 7 lines (blank + msgid "Reload" + msgstr + blank + msgid "Disconnected" + msgstr + trailing)
    lines = lines[:-7]
    with open(path, 'w') as fh:
        fh.writelines(lines)
    print(f'{f}: trimmed to {len(lines)} lines')

print('Done')
