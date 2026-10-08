---
inclusion: manual
description: "Hunts for over-engineering — what can be removed, simplified, or replaced with std / GTK4 / an existing dependency. A ranked list, one line per finding. Applies nothing."
---

Audit RustConn for over-engineering. Adapted from the upstream `ponytail-review`
and `ponytail-audit` skills (<https://github.com/DietrichGebert/ponytail>, MIT;
tags synced with Ponytail 5, 2026-10-07). Upstream renamed the marker to
`shortcut:` and still reads `ponytail:` as the older form; this repo keeps
`ponytail:`, which `scripts/ponytail-ledger.sh` collects.

Two modes — pick by what the request names:

- **diff mode** — review the current `git diff` (or a named commit range). The
  diff's best outcome is getting shorter.
- **repo mode** — scan a whole crate or the whole tree. Rank biggest cut first.

## Tags

One line per finding: `<file>:L<line>: <tag> <what to cut>. <replacement>.`

- `delete:` dead code, unused flexibility, speculative feature. Replacement: nothing.
- `stdlib:` hand-rolled thing `std` already ships. Name the function.
- `native:` code or dependency doing what the platform already does. Name the
  GTK4 / libadwaita / glib feature.
- `reuse:` the repo already has this helper. Name its path.
- `yagni:` abstraction with one implementation, config nobody sets, layer with one caller.
- `merge:` near-copies that must change together. Name every copy.
- `split:` one function doing several unrelated jobs, so it is hard to read or
  test. Split by job, never by line count, and never into helpers that exist
  only to shorten a function.
- `shrink:` same logic, fewer lines. Show the shorter form.

## Where to hunt in this repo

- **`native:`** is the richest vein here, because libadwaita covers a lot that is
  easy to hand-roll: a manual `gtk::Box` where `adw::ToolbarView` fits, a custom
  list where `adw::PreferencesGroup` + `adw::ActionRow` fits, a hand-built confirm
  dialog where `adw::AlertDialog` fits, `gtk::Paned` where
  `adw::OverlaySplitView` fits. See `gnome-hig.md` for the mapping and its
  anti-pattern list.
- **`yagni:`** traits with a single impl, `ProtocolConfig` fields nothing reads,
  builder structs wrapping three fields, feature flags with one call site.
- **`reuse:`** — check `rustconn-core` before concluding something is missing.
  Rung 2 of the ladder ("does it already exist in this repo") is the one most
  often skipped, and a helper reimplemented in `rustconn/` that already exists in
  `rustconn-core/` is the common shape here.
- **`merge:`** — GUI code that maps one enum in several places: combo indices,
  protocol switches, dialog load/save pairs. `rustconn/src/monitor_mode.rs` exists
  because one such mapping had seven copies across four files.
- **`.kiro/` itself is in scope** in repo mode. Steering is not compiler-checked,
  so it rots quietly: rules duplicated across files that each claim to be the
  single source, tables listing hooks that no longer exist, scaffolds that violate
  the current gate. `core-rules.md` states "one source of truth per rule" — audit
  against that.

## Output

Ranked, one line per finding. End with `net: -<N> lines, -<M> deps possible.`
If there is nothing to cut: `Lean already. Ship.` and stop.

## Boundaries

**Scope is over-engineering and complexity only.** Correctness bugs, security
holes and performance are explicitly out of scope — route them to the normal
review pass, `security-reviewer`, or `semantic_reviewer`. Keeping this boundary is
what stops this pass from duplicating the three reviewers the repo already has.

Never flag for deletion:
- a test (the test policy in `project-rules.md` overrides laziness),
- the single runnable check that non-trivial logic is required to leave behind,
- input validation at a trust boundary, error handling that prevents data loss,
  credential handling, or accessibility — the "never lazy about" list.

**Reports findings, applies nothing.** Read-only. The developer decides.
