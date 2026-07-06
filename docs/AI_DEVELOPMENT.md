# AI-Assisted Development Architecture

**Version 0.18.1** | Last updated: July 2026

This document describes the Kiro AI agent infrastructure used to automate
development workflows, enforce architectural constraints, and streamline the
release cycle of RustConn.

> **Single source of truth:** the authoritative, always-current inventory is the
> set of files in `.kiro/hooks/` (one file per hook) and `.kiro/steering/`
> (one file per steering rule). This document explains the *approach* and
> *rationale* — it intentionally does **not** duplicate every prompt or pattern,
> because hand-maintained inventories drift out of sync. When in doubt, read the
> `.kiro/` files.

---

## Table of Contents

- [Overview](#overview)
- [Steering Files](#steering-files)
- [Hooks](#hooks)
- [Design Decisions](#design-decisions)
- [Known Limitations](#known-limitations)
- [Maintenance](#maintenance)

---

## Overview

RustConn uses Kiro **steering files** and **hooks** in two complementary layers:

- **Steering = knowledge.** Persistent context injected into agent sessions so
  the agent always knows project conventions without being re-told. Located in
  `.kiro/steering/`.
- **Hooks = action.** React to IDE events (file save, tool use, manual trigger)
  to run checks or commands automatically. Located in `.kiro/hooks/`.

```
┌─────────────────────────────────────────────────────┐
│                     Developer                       │
└──────────────┬──────────────────────────────────────┘
               │
       ┌───────▼─────────┐    knowledge, always present
       │  Steering Files │ ── (project rules, guides, standards)
       └───────┬─────────┘
               │
       ┌───────▼─────────┐    automatic + on-demand actions
       │      Hooks      │ ── (checks, syncs, quality gates)
       └─────────────────┘
```

---

## Steering Files

`.kiro/steering/` currently holds **14** files. The agent loads them according
to each file's `inclusion:` front-matter (`always`, `fileMatch`, or `manual`).

| Group | Files | Purpose |
|-------|-------|---------|
| Core rules | `project-rules.md` (always), `rust-pragmatic-guidelines.md`, `error-resolution.md` | Architecture, absolute rules, lazy-senior philosophy, Microsoft pragmatic guidelines, compiler-error remedies |
| UI / GNOME | `gnome-hig.md`, `window-guide.md`, `dialogs-guide.md` | HIG compliance, window and dialog patterns |
| Domain guides | `protocol-guide.md`, `secrets-guide.md` | Adding protocols; credential/secret handling |
| Process | `release-reminder.md`, `verification-checklist.md`, `bugfix-workflow.md`, `spec-templates.md`, `test-patterns.md` | Release steps, verification, bugfix flow, spec scaffolding, test conventions |
| Tooling | `kirograph.md` | When/how to use the KiroGraph code-graph tools |

The exact inclusion mode and content of each file is in the file itself — that
is the canonical source.

---

## Hooks

`.kiro/hooks/` currently holds **20** hooks. Grouped by trigger:

### `fileEdited` — react to saves
| Hook | Watches | Enabled | Action |
|------|---------|---------|--------|
| `cargo-security-scan` | `Cargo.lock` | ✅ | `cargo deny`/`cargo audit` advisories |
| `flatpak-manifest-check` | `Cargo.lock` | ✅ | Warn if `cargo-sources.json` is stale |
| `translation-sync` | `rustconn/src/**/*.rs` | ✅ | Add file to `POTFILES.in` when new `i18n()` appears |
| `uk-translation-review` | `po/uk.po` | ✅ | Invoke `uk-translation-reviewer` sub-agent |
| `security-review` | secret/credential/password `*.rs` | ✅ | Invoke `security-reviewer` sub-agent (SecretString, zeroize, stdin pipes, no secrets in logs) |
| `sync-package-versions` | `Cargo.toml` | ❌ disabled | Superseded by `release-version` (see below) |

### KiroGraph index upkeep (`fileCreated` / `fileEdited` / `fileDeleted` / `agentStop`)
| Hook | Trigger | Action |
|------|---------|--------|
| `kirograph-mark-dirty-on-create` | file created | `kirograph mark-dirty` |
| `kirograph-mark-dirty-on-save` | file edited | `kirograph mark-dirty` |
| `kirograph-sync-if-dirty` | (deferred sync) | `kirograph sync-if-dirty` |
| `kirograph-sync-on-delete` | file deleted | `kirograph sync-if-dirty` |

### `preToolUse` — gate before an action
| Hook | Scope | Action |
|------|-------|--------|
| `pre-commit-checks` | shell commands | Before `git commit`/`push`: `fmt` + `clippy` must pass |
| `crate-boundary-guard` | write tools (`.rs`) | Deny writes that add GUI imports to `rustconn-core`/`rustconn-cli`, or `unsafe` outside `rustconn-pty-sys` |

### `userTriggered` — manual buttons
| Hook | Action |
|------|--------|
| `rustconn-checks` | Full quality gate: `fmt` → `clippy` → `cargo test --workspace` |
| `post-task-tests` ("Run Tests") | On-demand `cargo test --workspace` with duplicate-process guard |
| `dependency-audit` | Read-only: crate updates, advisories, CLI version drift |
| `commit-message-helper` | Generate a conventional-commit message from the diff |
| `ponytail-debt` | Read-only ledger of all `// ponytail:` markers; flags any missing a ceiling + upgrade path |
| `release-version` | **Release finalize**: bump version in all packaging files, propagate changelog, regenerate `cargo-sources.json`, verify consistency (no git) |

### Session / task lifecycle
| Hook | Trigger | Action |
|------|---------|--------|
| `post-session-diagnostics` | `agentStop` | Post-session diagnostics on touched files |
| `post-task-diagnostics` | `postTaskExecution` | `getDiagnostics` on `.rs` files a spec task changed (terminal-free, no cargo) |

> **Release note:** version-string propagation is done by the manual
> `release-version` hook at finalize time, **not** automatically on every
> `Cargo.toml` save. The old auto `sync-package-versions` hook is disabled to
> avoid an agent call on every dependency edit. Changelog *content* is always
> written by hand; `release-version` only *propagates* it to packaging formats.

---

## Design Decisions

### Steering vs hooks

Steering provides the *mental model* (what conventions exist and why); hooks
perform the *mechanical work* (run a command, edit a file). For releases this
pairing matters: `release-reminder.md` tells the agent the correct sequence
(write `CHANGELOG.md` before bumping the version, update deps after), while the
`release-version` hook executes the propagation. Without the steering, the agent
might run the hook in the wrong order.

### Pre (blocking) vs post/advisory checks

- **Blocking (`preToolUse`)** is reserved for things that would otherwise fail
  the build or CI anyway — formatting and clippy before a commit. Catching them
  early avoids a push-then-fail cycle. These are binary checks.
- **Advisory** checks (i18n coverage, credential patterns, protocol architecture)
  are nuanced — a missing `i18n()` wrapper does not break compilation and a
  pattern may be intentional. These are enforced via the **Self-Check Rules** in
  `project-rules.md` (applied mentally by the agent) rather than as blocking
  hooks, which keeps per-write LLM cost low.

### Why a 180s budget for tests

Property tests in `rustconn-core` use argon2 key derivation, intentionally slow
(~120s in debug mode). Test-running hooks allow up to 180s and guard against
launching a second `cargo test` while one is already running (shared terminal).

### Why KiroGraph upkeep hooks

The `kirograph-*` hooks keep the code-graph index fresh (mark dirty on
create/edit, sync on delete) so `kirograph` queries stay accurate without a
manual re-index. They fail silently (`|| true`) when KiroGraph is absent.

---

## Known Limitations

1. **Advisory checks are not enforced by tooling.** i18n / credential / protocol
   conventions live in `project-rules.md` Self-Check Rules and rely on the agent
   applying them. A determined mistake can slip through to `cargo clippy` / review.
2. **Shared terminal.** The main agent and sub-agents share one bash session;
   concurrent cargo runs interleave. Hooks and rules centralize cargo through a
   single `rust-quality-check` invocation to avoid collisions.
3. **`translation-sync` does not run `update-pot.sh`.** It only updates
   `POTFILES.in` and reminds the developer — regenerating 16 `.po` files is too
   invasive for an automatic hook.
4. **`flatpak-manifest-check` is advisory only.** Regenerating `cargo-sources.json`
   needs Python and produces large diffs; the hook warns but does not act.
5. **KiroGraph semantic search may be unavailable.** The embedding model can fail
   to load in some Node environments; structural queries (search, callers,
   architecture) still work. See `kirograph.md`.

---

## Maintenance

### Adding or changing a hook
1. Edit/create `.kiro/hooks/<name>.kiro.hook` (JSON schema below).
2. Bump its `"version"` field.
3. If it changes a *group* of behaviour above, update the relevant table in this
   file — but keep per-hook detail in the hook file, not here.

### Adding or changing a steering file
1. Edit/create `.kiro/steering/<name>.md` with the right `inclusion:` front-matter.
2. If it adds a new *group* of knowledge, add a row to the Steering table above.

### Keeping this document honest
When the hook/steering counts in this file no longer match `ls .kiro/hooks/`
and `ls .kiro/steering/`, the document has drifted — fix the counts and the
group tables, and resist the urge to inline every prompt.

### Hook file schema
```json
{
  "enabled": true,
  "name": "Human-readable name",
  "description": "What the hook does",
  "shortName": "kebab-case-id",
  "version": "1",
  "when": {
    "type": "preToolUse | postToolUse | fileEdited | fileCreated | fileDeleted | userTriggered | promptSubmit | agentStop | preTaskExecution | postTaskExecution",
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

Valid tool categories for `toolTypes`: `read`, `write`, `shell`, `web`, `spec`,
`*`. Regex patterns are also supported (e.g., `".*sql.*"`).
