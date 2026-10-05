---
inclusion: always
---

# RustConn — Core Rules

The rule list itself is the root `AGENTS.md` (always loaded): Non-negotiable,
Definition of Done, commits, releases. This file holds only what it does not —
the crate table, the `-sys` mechanics and the detail behind a few rules.
Terminal rules: `shell-environment.md`. Philosophy and escape hatches:
`project-rules.md` (manual). One source per rule.

## Architecture (7 crates)

| Crate | Purpose | Restrictions |
|-------|---------|-------------|
| `rustconn-core` | Domain logic: models, config, CRUD managers, import/export, protocol data, credential abstractions | **FORBIDDEN**: gtk4, adw, vte4. Default features stay headless. |
| `rustconn-cli` | Headless management over core data | Only rustconn-core. Default features minimal. |
| `rustconn` | GTK4/libadwaita GUI, dialogs, embedded/external session presentation | May import GUI crates |
| `rustconn-pty-sys` | Isolated FFI: macOS PTY controlling terminal (`setsid`+`TIOCSCTTY`) | Sanctioned `unsafe` (M-UNSAFE); `libc` only |
| `rustconn-locale-sys` | Isolated FFI: startup `setlocale`, refused once *this program* spawns a thread of its own (baseline growth, Linux only), once a call arrives from another thread, or after sealing | Sanctioned `unsafe` (M-UNSAFE); `gettext-rs` only |
| `rustconn-env-sys` | Isolated FFI: the startup `GSK_RENDERER` and `LANGUAGE` writes, guarded the same way | Sanctioned `unsafe` (M-UNSAFE); no dependencies |
| `rustconn-dock-sys` | Isolated FFI: the macOS Dock tile image via `-[NSApplication setApplicationIconImage:]`, for launches with no `.app` behind them. Main-thread proof via `objc2::MainThreadMarker`; a violation is an outcome, not a panic — a wrong Dock tile is cosmetic | Sanctioned `unsafe` (M-UNSAFE); `objc2` + AppKit bindings, macOS-gated |

Every `-sys` crate is an **unconditional** workspace member and dependency, with
`#[cfg]` inside where the platform differs — no CI job builds macOS, so a
macOS-only crate would be `unsafe` nothing ever checks. `rustconn-dock-sys` is
the one crate whose *dependencies* are target-gated, because `objc2-app-kit` does
not compile off Apple; its API and guard are still built and tested everywhere.
Use that shape only when bindings genuinely cannot build elsewhere, and check the
gated path with `cargo clippy -p <crate> --target aarch64-apple-darwin` (no SDK
needed for check/clippy). New FFI gets its own `rustconn-*-sys` crate.

## Detail behind the Non-negotiable rules

- **`unsafe`**: `unsafe_code = "deny"` in `[workspace.lints.rust]`, re-opened by a
  crate-level `#![expect(unsafe_code, reason = "…")]` in each `-sys` helper.
  `deny`, not `forbid`, on purpose: `forbid` cannot be overridden, so each helper
  would need its own `[lints]` table, which *replaces* the inherited one and
  leaves the only `unsafe` crates with no clippy lints. All helpers carry
  `[lints] workspace = true`. `rustconn` keeps a local `forbid`.
- **`unwrap()`/`expect()`** are fine in tests and `#[cfg(test)]` modules
  (`.clippy.toml`: `allow-unwrap-in-tests = true`).
- **i18n**: `display_name()` values used in UI are wrapped in `i18n()` at the call
  site. After new strings: `bash po/update-pot.sh`, then `msgmerge --update` every
  catalogue (`ls po/*.po` is the count).
- **Rust 2024**: let-chains instead of `collapsible_if`.
- **`set_startup_var`** may only be called from `main()` before this program starts
  a thread, and panics otherwise. Use it only when a C library reads the variable
  later and offers no API (GTK: `GSK_RENDERER`, gettext: `LANGUAGE`). The second
  caller seals the window, so a third panics rather than quietly working.

## Committing

- Stage from `target/.kiro-session-edits` only. If the journal overlaps changes
  the agent did not make — the checkout is shared with the IDE and a second
  session — stop and ask.
- Run the quality gate once for the whole change, before the commit; delegate it
  to `rust-quality-check` ("Run scripts/verify.sh --tests") rather than running
  it in the main context. Single-file validation: `getDiagnostics` where present.
- Exception: during release preparation `release-version.md` forbids git
  entirely — that flow leaves a clean tree for `release.sh` and commits nothing.
- The Definition of Done is the finish line for `/goal` loops too. If a loop
  cannot reach it, stop and report; never loosen the gate.

## Releases: why "prepare, never cut" is a rule

v0.20.1 was cut by an agent with `release.sh --yes`: it merged to main, pushed a
tag and published a release with five artifacts, carrying a red CI job and code
deletions the maintainer had never read. Undoing it meant deleting a published
release. The tag push is the one push that cannot be taken back — it triggers the
Release workflow and the Flathub/OBS/Snap updates.

## Agent profiles declare their model

Every profile in `.kiro/agents/` carries an explicit `model:`, picked by whether
an arbiter checks the agent's answer, not by how simple the task looks.
`agent-model-guard` rejects a profile without it; the table and the delegation
rules are in `cost-discipline.md`.
