# AI-Assisted Development Architecture

**Version 0.10.11** | Last updated: April 2026

This document describes the Kiro AI agent infrastructure used to automate
development workflows, enforce architectural constraints, and streamline the
release cycle of RustConn.

---

## Table of Contents

- [Overview](#overview)
- [Steering Files](#steering-files)
- [Hook Inventory](#hook-inventory)
- [Hook Details](#hook-details)
- [Hook Interaction Map](#hook-interaction-map)
- [Workflow: Full Release](#workflow-full-release)
- [Workflow: Adding a New Protocol](#workflow-adding-a-new-protocol)
- [Workflow: Daily Development](#workflow-daily-development)
- [Design Decisions](#design-decisions)
- [Known Limitations](#known-limitations)
- [Maintenance](#maintenance)

---

## Overview

RustConn uses Kiro hooks and steering files to create a layered automation
system.  The layers are designed so that each concern is handled exactly once
and feedback loops are short.

```
┌─────────────────────────────────────────────────────┐
│                   Developer                         │
│          (writes code, triggers releases)           │
└──────────────┬──────────────────────────────────────┘
               │
       ┌───────▼────────┐
       │  Steering Files │  ← persistent context injected into sessions
       │  (3 files)      │     release-reminder, commit-checklist,
       │                 │     test-patterns
       └───────┬─────────┘
               │
       ┌───────▼────────┐
       │  Event Hooks    │  ← react to IDE events automatically
       │  (8 hooks)      │     file edits, tool use, task lifecycle
       └───────┬─────────┘
               │
       ┌───────▼────────┐
       │  Manual Hooks   │  ← developer-triggered on demand
       │  (2 hooks)      │     quality checks, dependency audit
       └─────────────────┘
```

Total: 3 steering files + 10 hooks.

---

## Steering Files

Located in `.kiro/steering/`.  These inject context into agent sessions
so the agent always knows project conventions without being told each time.

| File | Inclusion | Activates When | Purpose |
|------|-----------|----------------|---------|
| `release-reminder.md` | `fileMatch` | Agent reads `Cargo.toml` | Reminds of mandatory release steps when version is bumped |
| `commit-checklist.md` | `manual` | Developer adds `#commit-checklist` to chat | Pre-commit fmt + clippy checklist |
| `test-patterns.md` | `fileMatch` | Agent reads `rustconn-core/tests/**/*.rs` | Test conventions: proptest, integration, fixtures, rules |

### Steering vs Hooks

Steering files provide **knowledge** — they tell the agent what conventions
exist.  Hooks provide **action** — they automatically run checks or commands.
The `release-reminder` steering tells the agent *what* to do during a release;
the `changelog-propagator` and `sync-package-versions` hooks *do* parts of it
automatically.

---

## Hook Inventory

### Summary Table (10 hooks)

| # | Hook | Trigger | Type | Action |
|---|------|---------|------|--------|
| 1 | `pre-write-guard` | `preToolUse: write` | Gate | Blocks GTK imports in core/cli + blocks unsafe code |
| 2 | `post-write-review` | `postToolUse: write` | Review | i18n + protocol architecture + credential security |
| 3 | `pre-commit-checks` | `preToolUse: shell` | Gate | fmt + clippy before git commit/push |
| 4 | `sync-package-versions` | `fileEdited: Cargo.toml` | Auto | Syncs version to 11 packaging/docs files |
| 5 | `changelog-propagator` | `fileEdited: CHANGELOG.md` | Auto | Propagates release notes to 5 changelog files |
| 6 | `translation-sync` | `fileEdited: rustconn/src/**/*.rs` | Auto | Updates POTFILES.in when new i18n() calls appear |
| 7 | `flatpak-manifest-check` | `fileEdited: Cargo.lock` | Warn | Warns about stale cargo-sources.json |
| 8 | `post-task-tests` | `postTaskExecution` | Auto | Runs `cargo test --workspace` after spec tasks |
| 9 | `rustconn-checks` | `userTriggered` | Manual | Full quality gate: fmt → clippy → tests |
| 10 | `dependency-audit` | `userTriggered` | Manual | Crate updates, CLI versions, security audit |

### By Trigger Type

**preToolUse** (block before action):
- `pre-write-guard` — every file write (`.rs` only)
- `pre-commit-checks` — every shell command (git commit/push only)

**postToolUse** (review after action):
- `post-write-review` — every file write (`.rs` only)

**fileEdited** (react to file saves):
- `sync-package-versions` — `Cargo.toml`
- `changelog-propagator` — `CHANGELOG.md`
- `translation-sync` — `rustconn/src/**/*.rs`
- `flatpak-manifest-check` — `Cargo.lock`

**postTaskExecution** (after spec task completes):
- `post-task-tests`

**userTriggered** (manual button click):
- `rustconn-checks`
- `dependency-audit`

---

## Hook Details

### 1. pre-write-guard

**Trigger:** `preToolUse: write` (every file write)
**Action:** `askAgent` — blocks the write if violations found
**Skips:** Non-`.rs` files (immediate pass-through)

Runs two checks on every `.rs` file write:

| Check | Scope | What it blocks |
|-------|-------|----------------|
| Crate boundary | `rustconn-core/` and `rustconn-cli/` | `use gtk4`, `use adw`, `use vte4`, `use libadwaita`, `gtk4::`, `adw::`, `vte4::` |
| Unsafe code | All `.rs` files | `unsafe {`, `unsafe fn`, `unsafe impl`, `unsafe trait`, `#[allow(unsafe_code)]` |

**Why blocking (pre)?** These violations would fail compilation anyway
(`unsafe_code = "forbid"` in workspace lints, missing GTK deps in core/cli).
Catching them before write saves a compile-fail-fix cycle.

### 2. post-write-review

**Trigger:** `postToolUse: write` (every file write)
**Action:** `askAgent` — reviews and reports findings
**Skips:** Non-`.rs` files (silent)

Runs up to three checks depending on file path:

| Check | Activates for | What it reviews |
|-------|---------------|-----------------|
| A: i18n | `rustconn/src/**` | `.set_label("...")`, `.set_title("...")`, `Button::with_label("...")` etc. without `i18n()` wrapper |
| B: Protocol | `rustconn-core/src/protocol/`, `rustconn-core/src/connection/`, `rustconn/src/dialogs/connection/`, `rustconn/src/embedded_*.rs`, `rustconn/src/session/` | Crate boundary, ProtocolType enum, capabilities, default_port, CLI parity |
| C: Credentials | `rustconn-core/src/secret/`, `rustconn-core/src/credentials/`, `rustconn/src/dialogs/password*.rs`, or content with `SecretString`/`password`/`credential`/`keyring`/`kdbx`/`bitwarden`/`onepassword`/`passbolt` | SecretString usage, zeroize, no logging of secrets, no CLI arg exposure, no secrets in error messages |

**Why reviewing (post)?** These are advisory — the code may be correct but
worth a second look.  Blocking would be too aggressive for style/pattern checks.

### 3. pre-commit-checks

**Trigger:** `preToolUse: shell` (every shell command)
**Action:** `askAgent` — intercepts git commit/push
**Skips:** Non-git commands (immediate pass-through)

Before `git commit` or `git push`, runs:
1. `cargo fmt --all` — auto-fixes formatting
2. `cargo clippy --all-targets -- -D warnings` — must pass clean

If either fails, the agent fixes issues before allowing the git command.
Mirrors the CI `fmt` and `clippy` jobs to prevent push-then-fail cycles.

### 4. sync-package-versions

**Trigger:** `fileEdited: Cargo.toml`
**Action:** `askAgent` — updates version numbers
**Skips:** If `[workspace.package] version` didn't change

Updates the version number (NOT changelog content) in 11 files:

| # | File | What changes |
|---|------|-------------|
| 1 | `packaging/obs/AppImageBuilder.yml` | `version:` field |
| 2 | `packaging/flatpak/io.github.totoshko88.RustConn.yml` | `tag: vX.Y.Z` |
| 3 | `packaging/flathub/io.github.totoshko88.RustConn.yml` | `tag: vX.Y.Z` |
| 4 | `packaging/obs/rustconn.dsc` | `Version:` + filenames |
| 5 | `packaging/obs/debian.dsc` | `Version:` + `DEBTRANSFORM-TAR` |
| 6 | `packaging/obs/rustconn.spec` | `Version:` header only |
| 7 | `packaging/obs/_service` | `<param name="revision">` |
| 8 | `docs/USER_GUIDE.md` | Version in first line |
| 9 | `docs/ARCHITECTURE.md` | Version in first line |
| 10 | `docs/INSTALL.md` | Flatpak bundle version |
| 11 | `docs/AI_DEVELOPMENT.md` | Version in first line |

**Explicitly excluded** (need actual changelog content):
CHANGELOG.md, debian/changelog, packaging/obs/debian.changelog,
packaging/obs/rustconn.changes, packaging/obs/rustconn.spec %changelog,
metainfo.xml, flatpak local manifest.

### 5. changelog-propagator

**Trigger:** `fileEdited: CHANGELOG.md`
**Action:** `askAgent` — propagates release notes
**Skips:** If no new `## [X.Y.Z] - YYYY-MM-DD` section was added

Propagates the new release section to 5 files, each in its own format:

| # | File | Format |
|---|------|--------|
| 1 | `debian/changelog` | Debian changelog (RFC 2822 date) |
| 2 | `packaging/obs/debian.changelog` | Same Debian format |
| 3 | `packaging/obs/rustconn.changes` | OBS changes format |
| 4 | `packaging/obs/rustconn.spec` | RPM `%changelog` section |
| 5 | `rustconn/assets/...metainfo.xml` | AppStream `<release>` element |

**Relationship with sync-package-versions:** These two hooks are complementary.
`sync-package-versions` handles version *numbers* in packaging configs.
`changelog-propagator` handles release *notes* in changelog files.
They never touch the same files.

### 6. translation-sync

**Trigger:** `fileEdited: rustconn/src/**/*.rs`
**Action:** `askAgent` — updates POTFILES.in
**Skips:** If the file has no `i18n()` calls

When a GUI source file is saved with `i18n()` calls:
1. Checks if the file is listed in `po/POTFILES.in`
2. If missing, adds it in alphabetical order
3. Reminds developer to run `po/update-pot.sh` and `po/fill_translations.py`

Does NOT auto-run the scripts — they modify 15+ `.po` files and should be
run intentionally.

### 7. flatpak-manifest-check

**Trigger:** `fileEdited: Cargo.lock`
**Action:** `askAgent` — warns developer
**Skips:** Never (always warns when Cargo.lock changes)

Warns that `packaging/flatpak/cargo-sources.json` and
`packaging/flathub/cargo-sources.json` may be stale.  Provides the
regeneration command but does NOT run it automatically — these are large
generated files.

### 8. post-task-tests

**Trigger:** `postTaskExecution` (after any spec task completes)
**Action:** `runCommand`
**Timeout:** 300 seconds

Runs `cargo test --workspace --no-fail-fast 2>&1 | tail -20`.

The 300s timeout accounts for property tests with argon2 key derivation
that take ~120s in debug mode.  This is normal and not a failure.

### 9. rustconn-checks

**Trigger:** `userTriggered` (manual button click)
**Action:** `askAgent` — runs checks and auto-fixes

Full quality gate, run on demand:
1. `cargo fmt --check` → auto-fix with `cargo fmt --all` if needed
2. `cargo clippy --all-targets -- -D warnings` → fix warnings and re-run
3. `cargo test --workspace` (300s) → report summary

The agent fixes clippy warnings automatically and re-runs to confirm.
Test failures are reported to the developer without auto-fix.

### 10. dependency-audit

**Trigger:** `userTriggered` (manual button click)
**Action:** `askAgent` — reports findings

Read-only audit, never auto-applies changes:
1. `cargo update --dry-run` — groups updates by patch/minor/major
2. `cargo audit` — security advisories (if installed)
3. CLI version check — pinned versions in `rustconn-core/src/cli_download.rs`
   vs latest available (kubectl, tailscale, cloudflared, boundary, teleport,
   bitwarden-cli, 1password-cli, passbolt-cli)
4. Summary with recommended actions

Note: A weekly GitHub Action (`check-cli-versions.yml`) also monitors CLI
versions independently.

---

## Hook Interaction Map

### Per-Write Cost

Every file write triggers exactly 2 hook evaluations:

```
Any file write
  ├─► pre-write-guard   (preToolUse: write)
  └─► post-write-review (postToolUse: write)
```

Both hooks skip non-`.rs` files immediately (first line of prompt).
For `.rs` files, `pre-write-guard` runs 2 checks, `post-write-review`
runs 0–3 checks depending on file path.

**Previous cost:** 5 separate hooks = 5 LLM evaluations per write.
**Current cost:** 2 merged hooks = 2 LLM evaluations per write.

### Per-Shell-Command Cost

```
Any shell command
  └─► pre-commit-checks (preToolUse: shell)
```

Skips immediately if the command is not `git commit` or `git push`.

---

## Workflow: Full Release

Step-by-step release process showing which hooks and steering files activate.

```
Step 1: Bump version in Cargo.toml
  ├─► [steering] release-reminder.md activates (fileMatch: Cargo.toml)
  │     → Agent sees full release checklist
  ├─► [hook] sync-package-versions fires (fileEdited: Cargo.toml)
  │     → Updates version in 11 packaging/docs files automatically
  │
Step 2: Write CHANGELOG.md with new ## [X.Y.Z] section
  ├─► [hook] changelog-propagator fires (fileEdited: CHANGELOG.md)
  │     → Propagates release notes to 5 changelog files automatically
  │
Step 3: Update dependencies
  ├─► [hook] dependency-audit (userTriggered, optional)
  │     → Reports available updates before applying
  ├─► Run: cargo update
  ├─► Run: cargo check --all-targets
  ├─► [hook] flatpak-manifest-check fires (fileEdited: Cargo.lock)
  │     → Warns about stale cargo-sources.json
  │
Step 4: CLI version check (if scripts/check-cli-versions.sh exists)
  ├─► Run: ./scripts/check-cli-versions.sh
  │     → Update rustconn-core/src/cli_download.rs if needed
  │
Step 5: Quality gate
  ├─► [hook] rustconn-checks (userTriggered)
  │     → fmt + clippy + tests
  │
Step 6: Commit and tag
  ├─► [hook] pre-commit-checks fires (preToolUse: shell)
  │     → Final fmt + clippy before git commit
  └─► git tag vX.Y.Z → triggers GitHub release workflow
```

**What's automated:** Steps 1 (version sync), 2 (changelog propagation),
3 (Cargo.lock warning), 5 (quality checks), 6 (pre-commit checks).

**What's manual:** Writing CHANGELOG.md content, deciding which deps to
update, CLI version updates, the actual git commit/tag.

---

## Workflow: Adding a New Protocol

```
Step 1: Add protocol type in rustconn-core/src/connection/types.rs
  ├─► pre-write-guard: crate boundary (no GTK) + no unsafe
  └─► post-write-review: Check B (protocol architecture)
        → Verifies ProtocolType enum, capabilities, default_port

Step 2: Add protocol logic in rustconn-core/src/protocol/new_proto.rs
  ├─► pre-write-guard: crate boundary + no unsafe
  └─► post-write-review: Check B (protocol architecture)

Step 3: Add connection dialog in rustconn/src/dialogs/connection/new_proto.rs
  ├─► pre-write-guard: no unsafe
  └─► post-write-review: Check A (i18n) + Check B (protocol)
        → Verifies all labels/tooltips use i18n()

Step 4: Add CLI handler in rustconn-cli/src/new_proto.rs
  ├─► pre-write-guard: crate boundary (no GTK) + no unsafe

Step 5: Add tests in rustconn-core/tests/
  ├─► [steering] test-patterns.md activates
  │     → Agent knows proptest conventions, module registration
  ├─► post-task-tests fires after spec task
  │     → Runs cargo test --workspace

Step 6: Update translations
  ├─► translation-sync fires (fileEdited: rustconn/src/**/*.rs)
  │     → Adds new file to POTFILES.in if i18n() calls present
```

---

## Workflow: Daily Development

```
Write code in any .rs file
  ├─► pre-write-guard: boundary + unsafe checks
  └─► post-write-review: i18n / protocol / credential checks

Save GUI source file (rustconn/src/)
  └─► translation-sync: checks POTFILES.in

Run spec task
  └─► post-task-tests: cargo test --workspace

Ready to commit
  ├─► rustconn-checks (optional, manual): full quality gate
  └─► pre-commit-checks: fmt + clippy before git commit
```

---

## Design Decisions

### Why merged hooks instead of separate ones?

`preToolUse` and `postToolUse` with `toolTypes: ["write"]` fire on EVERY
write — including `.md`, `.toml`, `.json`, `.yml` files.  With separate hooks:

- 3 postToolUse hooks × 1 write = 3 LLM evaluations, all saying "not .rs, skip"
- 2 preToolUse hooks × 1 write = 2 LLM evaluations, all saying "not .rs, skip"

Merged hooks reduce this to 1+1 = 2 evaluations total.  The routing logic
("is this file in path X?") moves inside the prompt instead of being
duplicated across hook configs.

### Why preToolUse for boundary/unsafe but postToolUse for i18n/protocol/credentials?

- **Pre (blocking):** Crate boundary violations and unsafe code will always
  fail compilation.  Blocking the write prevents a write-compile-fail-rewrite
  cycle.  These are binary checks — either the code has GTK imports or it
  doesn't.

- **Post (advisory):** i18n coverage, protocol architecture, and credential
  patterns are nuanced.  A missing `i18n()` wrapper doesn't break compilation.
  A credential pattern might be intentional.  These checks inform rather than
  block.

### Why steering + hooks for releases instead of just hooks?

The `release-reminder` steering provides the full mental model of the release
process.  The `sync-package-versions` and `changelog-propagator` hooks
automate the mechanical parts.  The steering ensures the agent knows the
*sequence* and *why*, while hooks handle the *what*.

Without steering, the agent would execute hooks but might not know to write
CHANGELOG.md *before* bumping the version, or to run dependency updates
*after* writing the changelog.

### Why 300s timeout for tests?

Property tests in `rustconn-core/tests/property_tests.rs` use argon2 key
derivation which is intentionally slow (~120s in debug mode).  The previous
180s timeout was too close to the actual runtime, causing false timeout
failures under load.  300s provides comfortable headroom.

---

## Known Limitations

1. **postToolUse hooks cannot filter by file path** — `toolTypes` is the only
   filter available.  The `post-write-review` hook must evaluate its prompt
   for every write, even non-`.rs` files.  The prompt's first line handles
   this ("if not .rs, do nothing") but the LLM call still happens.

2. **preToolUse hooks fire on deleteFile too** — The `pre-write-guard` hook
   triggers on file deletions because `deleteFile` is categorized as a write
   tool.  The prompt handles this (non-`.rs` files pass through) but it's
   unnecessary overhead.

3. **changelog-propagator requires well-formed CHANGELOG.md** — The hook
   parses `## [X.Y.Z] - YYYY-MM-DD` headers.  Non-standard formatting
   (e.g., missing date, different header level) will cause it to skip
   propagation silently.

4. **translation-sync doesn't run update-pot.sh** — It only updates
   POTFILES.in and reminds the developer to run the scripts.  Full automation
   would modify 15+ `.po` files which is too invasive for an automatic hook.

5. **flatpak-manifest-check is advisory only** — Regenerating cargo-sources.json
   requires Python and produces large diffs.  The hook warns but doesn't act.

6. **No hook for rustconn-cli/ i18n** — The CLI crate doesn't use gettext
   (it uses plain English strings).  If CLI i18n is added in the future,
   `post-write-review` Check A scope should be expanded.

---

## Maintenance

### Adding a new hook

1. Create `.kiro/hooks/<name>.kiro.hook` with the JSON schema
2. Add it to the Hook Inventory table in this document
3. Add it to the relevant Workflow sections
4. If it's a `preToolUse`/`postToolUse` write hook, consider merging it into
   `pre-write-guard` or `post-write-review` instead of creating a new one

### Modifying an existing hook

1. Update the `.kiro.hook` file
2. Bump the `"version"` field
3. Update the corresponding Hook Details section in this document

### Hook file schema

```json
{
  "enabled": true,
  "name": "Human-readable name",
  "description": "What the hook does",
  "shortName": "kebab-case-id",
  "version": "1",
  "when": {
    "type": "preToolUse | postToolUse | fileEdited | ...",
    "toolTypes": ["write"],
    "patterns": ["*.rs"]
  },
  "then": {
    "type": "askAgent | runCommand",
    "prompt": "Instructions for askAgent",
    "command": "shell command for runCommand",
    "timeout": 300
  }
}
```

Valid event types: `fileEdited`, `fileCreated`, `fileDeleted`,
`userTriggered`, `promptSubmit`, `agentStop`, `preToolUse`, `postToolUse`,
`preTaskExecution`, `postTaskExecution`.

Valid tool categories for `toolTypes`: `read`, `write`, `shell`, `web`,
`spec`, `*`.  Regex patterns also supported (e.g., `".*sql.*"`).