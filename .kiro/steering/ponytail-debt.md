---
inclusion: manual
description: "Збирає всі відкладені спрощення (`// ponytail:`) у крейтах в один леджер технічного боргу, щоб «later» не стало «never»."
---

Harvest the ponytail debt ledger for RustConn. Steps:

1. Run `grep -rn 'ponytail:' --include='*.rs' rustconn-core/src rustconn-cli/src rustconn/src rustconn-pty-sys/src` to find every intentional simplification marker.

2. For each hit, present: file:line, the crate, the stated ceiling, and the upgrade path from the comment.

3. Group by crate. Flag any `// ponytail:` that does NOT name a ceiling + upgrade path (per project rules a marker must name both) — these are incomplete and need fixing.

4. Do NOT edit any code. This is a read-only report. End with a one-line summary: total markers, and how many are incomplete.
