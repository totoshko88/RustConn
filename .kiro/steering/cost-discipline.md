---
inclusion: manual
description: "Cost policy: which model tier an agent gets and why, when a hook may spend an agent loop, and how to keep the always-loaded context small. Load with #cost-discipline before adding an agent, a hook, or an always-included steering file."
---

# Cost Discipline

Deliberately `inclusion: manual`. A policy about not paying for tokens you do not
need, carried in every request, would be its own counterexample. Load it when you
are adding an agent, adding a hook, or considering making a steering file
`always`.

Kiro bills per request in credits, fractionally, metered to 0.01, and a complex
request can exceed one credit. Two things therefore cost money: which model runs,
and how many tokens the run has to chew. Everything below follows from that.

## The rule that decides a model tier

**Cheap where a machine checks the answer. Expensive where nothing does.**

Not "cheap where the task looks simple". `rust-quality-check` runs three fixed
cargo commands and reports pass/fail — if it claims a pass it did not earn, the
next clippy run contradicts it, so a wrong answer costs one re-run.
`unsafe-reviewer` decides whether a `// SAFETY:` comment is verifiable or merely
decorative; nobody repeats that judgement, so a wrong "✅" is undefined behaviour
in a shipped binary. The second task is not more complicated than the first. It is
unattended.

| Tier | `model_id` | Rate | For |
|------|-----------|------|-----|
| floor | `qwen3-coder-next` | 0.05x | fixed command sequences, machine-verified output |
| | `minimax-m2.1` | 0.15x | |
| | `deepseek-3.2` / `minimax-m2.5` | 0.25x | |
| tool-driven | `claude-haiku-4.5` | 0.4x | MCP/graph navigation, deterministic backends |
| | `glm-5` | 0.5x | |
| default | `auto` | 1.0x | router; picks per request |
| judgement | `claude-sonnet-4/4.5/4.6` | 1.3x | findings a human reads and acts on |
| unattended judgement | `claude-opus-4.5` | 2.2x | a wrong "no issues" ships |

Refresh the list — it changes, and it differs per client:

```bash
kiro-cli chat --list-models --format json
```

The IDE offers models the CLI does not. A profile pinned to an IDE-only ID falls
back to the default in a CLI session, **silently**, with the config still looking
deliberate. Prefer IDs present in both. The same silence applies to a typo: an
unrecognised ID is not an error, it is a fallback.

### Current assignment

| Agent | Model | Why this tier |
|-------|-------|---------------|
| `rust-quality-check` | `qwen3-coder-next` | clippy is the arbiter |
| `kirograph` | `claude-haiku-4.5` | graph answers are deterministic, but tool selection must not be |
| `security-reviewer` | `claude-sonnet-4.6` | six mechanical checks, zero arbiters — a missed `SecretString` ships |
| `uk-translation-reviewer` | `claude-sonnet-4.6` | Ukrainian morphology; only a reader can catch a wrong genitive |
| `unsafe-reviewer` | `claude-opus-4.5` | UB is not a re-run; fires only when a `-sys` crate changes |

Note what this is: a **reallocation**, not a saving. Two frequently-invoked agents
got cheaper, three audit agents got more expensive than the 1.0x default. That is
the intended shape — routine work should be cheap so that unattended judgement can
afford to be expensive.

The main agent is not on this table on purpose. Choosing a cheap model for the
session that plans and writes code is a false economy: the interlocking rules here
(crate boundaries, `i18n()`, `SecretString`, no `unwrap()`, `ponytail` markers) are
violated quietly, and only two of them are hook-enforced. The cost of a missed
i18n wrap is a review cycle, not credits.

## Hooks: an agent action is a purchase

Documented, and the sharpest lever in this repo:

- **Shell Command actions do not consume credits** — they run locally, no LLM.
- **Agent Prompt actions do** — each one starts a new agent loop.

So: **an agent action is only for a decision. A check is a script.**

The failure mode this repo actually hit: `post-session-diagnostics` was an agent
action on `Stop`. `Stop` fires at the end of *every* turn, including turns that
only answered a question. On 2026-09-06 it ran five times in one session, each
time reporting 29 `.rs` files the agent had never touched, each time ending in a
`getDiagnostics` call that does not exist outside the IDE. Five agent loops for
five wrong answers.

Two patterns came out of fixing it, and both are reusable:

**Attribute by journal, not by inference.** `edit-journal.sh` records paths as
write tools are called. Comparing content hashes against a session-start baseline
answers "did anything change this file?", which is a different question and is
wrong whenever the IDE, a second session, or one of our own hooks touches the
tree. Any hook that needs "what did the agent change" reads
`target/.kiro-session-edits`.

The same journal is why `core-rules.md` forbids `git add -A`. On 2026-09-06 this
checkout held 47 dirty files, 29 of them `.rs`, none of them the running agent's —
the IDE was open on the same tree and something else committed mid-session (`HEAD`
moved under us). A blanket stage in that state turns "commit my feature" into
"commit someone else's half-finished work". The journal is the only list that is
actually the agent's.

**Free delivery via the report channel.** A command hook's stdout is forwarded
only on `SessionStart`, `UserPromptSubmit` and `PreToolUse` — on `Stop` and
`PostFileSave` it is discarded. So a `Stop` or `PostFileSave` hook computes for
free and appends a paragraph to `target/.kiro-session-report`; the
`UserPromptSubmit` flush hook prints it on the next turn, which was going to
happen anyway. One turn late, zero credits. Producers append, flush is the sole
consumer and deletes. Adding a producer needs no coordination.

**Batch a review; do not repeat it per file.** Three reviews used to fire on
`PostFileSave` — one agent loop per saved file, each seeing one file in isolation,
and in `uk.po`'s case also after every `msgmerge` that rewrote the catalogue
without changing a single translation. They now fire once, at `git commit`, via
`commit-review-gate`, driven by the journal.

**Ask when you cannot verify.** A `PreToolUse` hook can return
`{"hookSpecificOutput":{"permissionDecision":"ask", …}}`. That is the honest
mechanism when the guard cannot observe whether an obligation was met — the
review gate cannot know a reviewer already ran. Reserve `exit 2` for invariants a
script can actually confirm, as `crate-boundary-guard` and
`release-manual-only-guard` do.

## Context: the always-loaded tax

Every `inclusion: always` steering file is paid on every request, in every
session. Under a workflow that opens fresh verification sessions on an expensive
model, it is paid again per session and again per request inside it.

Before making a file `always`, or before adding to one, ask whether the text is an
**invariant** or an **explanation**. Invariants belong in the always set;
measurements, war stories and worked examples belong in a `manual` companion, and
are more useful there because you read them when you need them.

Same principle for a fresh session on an expensive model: it should not spend its
first thousand tokens rediscovering what changed. `scripts/change-inventory.sh`
writes a deterministic inventory — diff stat, touched crates, affected tests — so
the expensive model opens one file and starts judging. Preparing that inventory
costs no LLM at all.

And prefer a local answer over a model answer wherever one exists: KiroGraph is a
SQLite query, not an inference. `kirograph_context` before dispatching an
exploration sub-agent, always.

## What this does not license

Cost is never a reason to weaken the Definition of Done. Do not drop a test,
silence a lint, skip an `i18n()` wrap or leave a changelog entry unwritten to make
a turn cheaper. Those all move the cost to a later session that has lost the
context, which is the most expensive place to put it.
