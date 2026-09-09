# AI-Assisted Development Architecture

**Version 0.21.10** | Last updated: August 2026

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

`.kiro/steering/` currently holds **29** files. The agent loads each according to
its `inclusion:` front-matter:

| Mode | What it means | Roughly |
|------|---------------|---------|
| `always` | In every session, unconditionally | `core-rules.md`, `shell-environment.md`, `kirograph.md` — ~4 000 words with root `AGENTS.md`, paid on every request |
| `fileMatch` | Loaded when a matching file enters context | the GUI, domain and error-playbook guides |
| `manual` | Only when named with `#` in chat, or by a hook | the process runbooks |
| `auto` | Matched against the file's own `description` | needs both `name` and `description`, or it matches nothing |

That last row is a trap worth stating: `auto` without `name` + `description` in the
front matter silently never loads. `shell-environment.md` sat that way and asserted
in its own text that it was always present.

### When `auto` is the wrong mode

`manual` has an obvious drawback — half the ruleset never loads unless someone
types `#name` — so `auto` looks like a free upgrade. It is not, and the
distinction is what a file *does* when it lands in context.

A file that is **guidance** changes how the agent approaches work and starts
nothing on its own. `bugfix-workflow.md` is the example: loading it during a bug
fix is exactly what you want, and it was converted to `auto` on 2026-09-02.

### Rules in the `always` set, reasoning in a companion

The `always` set is a fixed token cost on every request in every session, so it
holds invariants and nothing else. Measurements, incidents and worked examples go
into a `manual` companion, where they are more useful anyway — you read them when a
rule looks arbitrary, not on every turn.

`shell-environment.md` was split this way on 2026-09-06: 1 691 words became 890
words of rules plus `shell-environment-why.md` for the queued-sleep failure, the
cargo timings, the `/tmp`-to-`target/` correction and the portal-blocked GUI
diagnosis. No rule lives only in the companion — if you find one there, it belongs
in the always file. Policy: `cost-discipline.md`.

A file that is a **runbook** opens with an instruction and expects to be obeyed.
Four of these must stay `manual`, because `auto` would have them fire on a passing
mention rather than a decision:

| File | First line commits the agent to |
|------|---------------------------------|
| `dependency-audit.md` | running `scripts/dep-audit.sh` and a round of web lookups |
| `release-version.md` | editing every packaging file for a version bump |
| `code-review.md` | spawning several reviewer sub-agents in parallel |
| `ponytail-audit.md` | scanning a crate or the whole tree for over-engineering |

Each is expensive, and each is a decision the developer makes — "audit the
dependencies" is a request, not a topic. An audit on 2026-09-02 proposed
converting all four to `auto` on the strength of the count alone, then withdrew it
after reading what the files actually say. Read the first paragraph of a file
before changing its mode.

Per-directory rules live outside this mechanism, in a nested `AGENTS.md` in each
crate, `po/` and `packaging/` — Kiro loads those by directory tree, and unlike
steering they are also read by agents that do not know about `.kiro/`.

The file list is not reproduced here. `ls .kiro/steering/` is the inventory, each
file's front matter is its mode, and `scripts/check-ai-docs.sh` gates the count
above so this paragraph cannot quietly go stale again — it claimed 14 for long
enough that the number was off by thirteen.

---

## Hooks

`.kiro/hooks/` currently holds **17** hooks, one JSON file each, in the v2 format
the agent executes directly. Triggers are PascalCase.

| Trigger | Hooks | What the group is for |
|---------|-------|-----------------------|
| `PreToolUse` | `crate-boundary-guard`, `agent-model-guard`, `bash-serialization-guard`, `release-manual-only-guard`, `commit-review-gate` | Refuse or question an action before it happens: a GUI import in a headless crate or `unsafe` outside a `-sys` crate; an agent profile with no model tier; a cargo run that will lose its output or wedge the terminal; any push or route to cutting a release; a commit that skips a review its paths require |
| `PostToolUse` | `edit-journal` | Record what the agent wrote, so later hooks can scope to its own work instead of the dirty tree |
| `PostFileSave` | `translation-sync`, `cargo-security-scan`, `flatpak-manifest-check`, `kirograph-mark-dirty-on-save` | React to a save, each with a path matcher so it fires only for the files it is about |
| `PostFileCreate` / `PostFileDelete` | `kirograph-mark-dirty-on-create`, `kirograph-sync-on-delete` | Keep the code graph honest about files appearing and disappearing |
| `SessionStart` | `session-reset` | Clear the previous session's journal and report |
| `Stop` | `session-report`, `kirograph-sync-if-dirty` | Post-session work: scan the journalled files for debug leftovers, deferred graph sync |
| `UserPromptSubmit` | `session-report-flush` | Deliver the `Stop` scan's finding on the next turn — the only place a free hook can speak |
| `PostTaskExec` | `post-task-diagnostics` | `getDiagnostics` on `.rs` files a spec task touched — terminal-free, no cargo |

**One agent action left.** As of 2026-09-06 only `post-task-diagnostics` starts an
agent loop, and it fires on spec task completion. The others are `command` actions,
which run locally and cost no credits. Four used to be agent actions:
`post-session-diagnostics` on every `Stop`, and three reviews on every matching
`PostFileSave`. The conversion is described in steering `cost-discipline.md`,
including the two patterns that made it possible — attributing edits by journal
rather than by content hash, and delivering a free hook's output through
`UserPromptSubmit`.

Each hook's own `description` field carries its rationale, including the hardening
notes that matter (why the KiroGraph sync checks for a stale lock, why the release
guard covers three separate routes). That is the canonical text.

`scripts/check-ai-docs.sh` gates the count above and, since 2026-09-02, also
asserts that every hook file has a row in steering `hooks-map.md`. That second
check was added because the map lost `session-baseline` (since replaced by
`session-reset`) for a week while claiming in its first line to cover every hook —
the table above had it, the map did not, and nothing compared either against
`ls .kiro/hooks/`.

The manual runbooks that used to be listed here as hooks — the quality gate, the
dependency audit, the ponytail ledger, the release preparation, the commit-message
helper — are steering files under `.kiro/steering/`, invoked with `#`. They were
never hooks, and counting them as such is where the "20" came from.

> **Release note:** version strings are written by `scripts/bump-version.sh`, run
> deliberately at finalize time, **not** automatically on a `Cargo.toml` save.
> Changelog *content* is always written by hand; only its propagation into the
> packaging formats is mechanical.

---

## Design Decisions

### A prompt that only runs commands should be a script

The costly mistake in this setup was not a bad prompt, it was using a prompt at all
for work with no judgement in it. Two examples, both now converted:

`translation-sync` used to ask the model to run three greps after every `.rs` save.
`post-session-diagnostics` used to ask it to run `git diff --name-only HEAD`, then
`getDiagnostics` on up to ten files, then `git diff HEAD` and scan for debug
macros — on every single `Stop`. Neither needed a model. The second was also
*wrong*: `git diff HEAD` reports the whole dirty working tree, not the session, so
on 2026-08-26 three consecutive Stop hooks spent a shell call plus up to ten tool
calls to conclude that nothing had changed, in a session whose only edit was
markdown.

The shape that works: a `command` hook does the mechanical part and stays silent
when there is nothing to say; the agent is invoked only for the step a script cannot
take.

`post-session-diagnostics` was first converted to that shape in August — a script
did the file selection and the leftover scan, and the prompt was reduced to "run
this script and act only on its output", keeping one `agent` action for the single
step a script cannot take: calling `getDiagnostics`. That was still one agent loop
per turn, and on 2026-09-06 the remaining step turned out to be worth nothing in an
ACP session, where `getDiagnostics` is not in the tool set at all — five turns in a
row produced a report about files the agent had never touched and ended in a tool
that did not exist. The hook is now `session-report`, fully `command`, with two
lessons that generalise:

- **A hook must not depend on a tool only one client provides.** The prompt was
  correct in the IDE and a dead end everywhere else, and nothing in its own text
  said which.
- **When the last step a script cannot take is worth less than an agent loop per
  turn, drop the step.** Compile diagnostics moved to the commit gate, where clippy
  runs once per feature. `session-report` computes for free at `Stop` and hands its
  finding over through `session-report-flush` on the next prompt.

The same reasoning moved four runbooks out of prose and into `scripts/`:

| Script | Replaced |
|--------|----------|
| `verify.sh` | The mechanical half of `verification-checklist.md` and `quality-gate.md`, including the check for whether clippy actually re-checked anything |
| `bump-version.sh` | A sixteen-bullet list in `release-version.md` that mirrored `PKG_FILES` in `release.sh` and asked to be kept in sync with it by hand |
| `ponytail-ledger.sh` | The grep-and-group half of `ponytail-debt.md` |
| `dep-audit.sh` | Steps 1-3 of `dependency-audit.md`, the ones that are commands |

What stayed in steering is what needs a decision: whether a ponytail ceiling is
still honest, whether a major dependency bump is worth taking, whether a string
needs `i18n()`, whether the new code is in the right crate.

### Codex target boundaries

Keep future AI tasks sliced by crate boundary:

| Target | Scope | First files to inspect |
|--------|-------|------------------------|
| Core | Domain logic, models, config persistence, import/export, protocol data, credential abstractions | `rustconn-core/src/lib.rs`, `rustconn-core/src/models`, `rustconn-core/src/config`, `rustconn-core/src/connection`, `rustconn-core/src/protocol` |
| CLI | Headless management over core data: list/show/add/update/delete, import/export, tests, tags/groups/templates | `rustconn-cli/src/cli.rs`, `rustconn-cli/src/commands`, `rustconn-cli/src/error.rs` |
| GUI | GTK/libadwaita presentation, dialogs, embedded/external sessions, toasts, window state | `rustconn/src/dialogs`, `rustconn/src/window`, `rustconn/src/embedded_*` |

Do not mix these in one Codex ticket unless the change is an end-to-end feature
that explicitly requires all layers. `rustconn-core` has an empty default
feature set; embedded clients, RD Gateway/GFX, host keyring support, and CLI
client-launch behavior are optional integration features.

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
   `POTFILES.in` and reminds the developer — regenerating every `.po` file is too
   invasive for an automatic hook. (`ls po/*.po` is the count; this line said 16
   while there were 17.)
4. **`flatpak-manifest-check` is advisory only.** Regenerating `cargo-sources.json`
   needs Python and produces large diffs; the hook notes the staleness but does not
   act. Since 2026-09-06 it is a timestamp comparison in a script rather than an
   agent prompt, so the advice is now free.
5. **KiroGraph semantic search may be unavailable.** The embedding model can fail
   to load in some Node environments; structural queries (search, callers,
   architecture) still work. See `kirograph.md`.

---

## Maintenance

### Adding or changing a hook
1. Edit/create `.kiro/hooks/<name>.json` (JSON schema below). The `.kiro.hook`
   extension this line named is not what the loader reads — `ls .kiro/hooks/`
   is all `.json`.
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
