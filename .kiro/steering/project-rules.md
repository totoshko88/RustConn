---
inclusion: manual
description: "Long-form project rules: code philosophy, Codex target split, workflow, escape hatches, terminal discipline. The always-loaded subset (crate table, Absolute Rules, Definition of Done) is in core-rules.md and is not repeated here."
---

# RustConn — Project Rules

> **Read `core-rules.md` first.** It is `inclusion: always`, so it is already in
> context, and it owns the crate table, the Absolute Rules, the Definition of
> Done and the quick commands. This file is the long form: everything that is too
> situational to carry in every request. Rules are *not* duplicated between the
> two files — if you are looking for an invariant and it is not here, it is there.
>
> This file's front matter used to claim it was "already injected as user-rule at
> session start". It was not: `inclusion: manual` means it loads only via
> `#project-rules` or a slash command, and `~/.kiro/steering/` holds no global
> copy. That is why the invariants were split out into `core-rules.md` on
> 2026-08-12 — a fresh session had none of them.

## Architecture

Crate table and the FFI rule: see `core-rules.md`.

### Codex Target Split

- Core ticket → start in `rustconn-core/src/lib.rs`, `models`, `config`, `connection`, `protocol`, `secret`; keep runtime integrations behind features.
- CLI ticket → start in `rustconn-cli/src/cli.rs`, `commands`, `error.rs`; keep the default path to config/CRUD/list/import/export/simple operations.
- GUI ticket → start in `rustconn/src/dialogs`, `window`, `embedded_*`; do not move GTK/libadwaita concepts into core or CLI.
- Cross-layer work must name the boundary being crossed and keep each layer's change reviewable on its own.

## Code Philosophy (YAGNI / lazy-senior)

The best code is the code never written. Lazy means efficient, not careless.
The ladder runs *after* you understand the problem, not instead of it: read the
task and the code it touches, trace the real flow end to end, then climb. A small
diff in the wrong place isn't lazy, it's a second bug.
Before writing any code, stop at the first rung that holds:

1. Does this need to exist at all? → no: skip it (YAGNI)
2. Does it already exist in this repo? → reuse the helper/util/pattern (usually in `rustconn-core`), don't re-write it
3. Does `std` already do this? → use it
4. Is there a native platform / GTK4 / libadwaita feature? → use it
5. Does an already-present dependency solve it? → use it
6. Can it be one line? → make it one line
7. Only then: the minimum code that works

- Deletion over addition. Boring over clever. Fewest files possible.
- No abstractions, traits, or generics that weren't asked for. No boilerplate nobody requested.
- No new crate if it can be avoided (also respects `cargo deny` / supply-chain).
- Question complex requests: "Do you actually need X, or does Y cover it?"
- Architectural decisions (new crate, protocol backend, storage/secret model, threading) name at least two alternatives and why the pick won, in the reply or commit body. One-liner is enough; the point is the discarded option is on record.
- Bug fix = root cause, not symptom. A report names one symptom; grep every caller of the function you touch and fix the shared function once (one guard in `rustconn-core` beats one per caller in `rustconn`). Patching only the path the ticket names leaves sibling callers broken.
- When two `std` approaches are the same size, pick the edge-case-correct one. Lazy means less code, not the flimsier algorithm.
- Mark intentional simplifications with a `// ponytail:` comment that names the ceiling and the upgrade path, e.g. `// ponytail: O(n²) scan, fine for <100 hosts; index if the list grows`.
- **Never lazy about** (these are never on the chopping block): trust-boundary input validation, error handling that prevents data loss, security/credentials (see Absolute Rules), accessibility (see GNOME HIG).
- Tests are **not** subject to laziness: the existing test policy below stands — keep at least the rustconn-core property-test coverage; never drop a test to "save code".
- **Lazy code without its check is unfinished.** The rule above is a prohibition; this is the matching obligation, and it was missing until 2026-08-20. Non-trivial logic leaves behind ONE runnable check — the smallest thing that fails if the logic breaks. In this repo that is a property test in `rustconn-core/tests/properties/` (registered in `mod.rs`) or an integration test in `tests/integration/`; for a `-sys` crate it is the contract test that asserts the precondition guard, since the FFI call itself is not reachable from a test harness. No frameworks, no fixtures beyond `tempfile`. A trivial one-liner needs no test — the bar is "would a reader be able to tell this broke?".

## Absolute Rules

See `core-rules.md`. They are unconditional and always loaded — that is the point
of keeping them there rather than here.

## Quick Commands

Core set: see `core-rules.md`. Additional targets used less often:

```
cargo test -p rustconn-core --test property_tests  # Property tests only
cargo test -p rustconn-cli --features full         # CLI with all features
cargo machete                                      # Unused dependencies
./scripts/check-cli-versions.sh                    # Bundled CLI tool versions
```

## Quality Checks

Delegate to `rust-quality-check` sub-agent for fmt+clippy+tests instead of running
in main context. It runs on the cheapest model in the catalogue on purpose — clippy
re-checks its answer, so a wrong pass costs one re-run (`cost-discipline.md`).

For quick single-file validation → `getDiagnostics`, **when you have it**. It is an
IDE-side tool: it reads what the language server already published, which is why it
is cheap and why cargo is the wrong substitute. In a session driven by an ACP
client (`kiro-cli … acp`) it is absent from the tool set entirely, and there is no
free equivalent — nothing has pre-computed the answer. In that case do not
improvise with cargo mid-task; let the commit gate's quality run cover it, or
delegate to `rust-quality-check` when the change is actually finished.

### Self-Check Rules (hooks + mental)

The hardest-to-reverse invariants are now enforced automatically by the
`crate-boundary-guard` preToolUse hook (it denies the write if a `.rs` change
adds GUI imports to `rustconn-core`/`rustconn-cli`, or `unsafe` outside a
`rustconn-*-sys` crate). Still verify them yourself BEFORE writing — the hook is
a safety net, not an excuse to skip thinking:
- **Crate boundary**: `rustconn-core/` and `rustconn-cli/` must NOT contain `use gtk4`, `use adw`, `use vte4`, `gtk4::`, `adw::`, `vte4::`. Move GUI code to `rustconn/`. *(hook-enforced)*
- **No unsafe**: never write `unsafe {`, `unsafe fn`, `unsafe impl`, `unsafe trait` — **except** in a `rustconn-*-sys` crate (`rustconn-pty-sys`, `rustconn-locale-sys`, `rustconn-env-sys`, `rustconn-dock-sys`; M-UNSAFE). New `unsafe` outside them is forbidden — it gets its own `-sys` crate instead. *(hook-enforced)*

After writing `.rs` files in `rustconn/src/`, verify (these stay mental — caught later by clippy, and for debug leftovers by the `session-report` Stop hook, not pre-write):
- **i18n**: all user-facing strings (`.set_label()`, `.set_title()`, `.set_tooltip_text()`, `Button::with_label()`) wrapped in `i18n()` or `i18n_f()`. Ignore: tracing, CSS, icons, action names.
- **Credentials** (in secret/password/credential files): `SecretString` for passwords, `.zeroize()` intermediates, no secrets in logs/args/errors. *(the `commit-review-gate` hook asks for the `security-reviewer` sub-agent at commit time — once for the whole change, where a per-file `PostFileSave` hook used to fire once per saved file)*
- **Protocol files**: business logic in rustconn-core, GTK in rustconn.

### When to Run fmt/clippy/tests

- **Do NOT** run `cargo fmt`/`cargo clippy` automatically on every change — use `getDiagnostics` for quick validation, or nothing at all if the session has no such tool.
- Run `rust-quality-check` sub-agent only when: (a) about to commit, (b) user explicitly asks, (c) finishing a multi-file feature.
- Run tests only when: (a) user explicitly asks, (b) finishing a spec task, (c) before release.
- After completing a feature, the sequence is the quality gate → CHANGELOG entry →
  commit of the journalled files, per "Finishing a feature" in `core-rules.md`.
  Report what was committed and what is left; never push. For a change that is not
  a finished feature, still ask before running the gate: "Done. Run quality check
  (fmt+clippy)?"

### Learning Loop (after non-trivial tasks)

When a task surfaced something worth keeping for next time — an interpretation of an ambiguous request, an intentional deviation from these rules, or a tradeoff between two approaches — record it once with `kirograph_mem_store` (kind `decision`/`pattern`), or propose a one-line addition to a `.kiro/steering` file if it's a durable project rule. Fire only when there's a real lesson; most tasks add nothing, and that's fine. Don't re-store what memory already holds — `kirograph_mem_search` first.

### Definition of Done (goal-loop acceptance gate)

See `core-rules.md`. Never loosen the gate (drop a test, silence clippy, skip
i18n) just to "finish". The one sanctioned exception is below, and it applies
only when the blocker is external.

#### Escape Hatches (when the gate is blocked by external factors)

When the blocker is NOT your code but an upstream/environmental issue, these are the
sanctioned workarounds. Each requires a tracking comment and must be reported to the developer.

| Blocker | Sanctioned workaround | Tracking |
|---------|----------------------|----------|
| Clippy warning from new upstream lint or compiler upgrade | `#[expect(clippy::lint_name, reason = "upstream issue URL")]` | `// ponytail: remove after next clippy update` |
| Flaky test (passes locally, fails non-deterministically) | `#[ignore = "flaky: issue #NNN"]` | File an issue, link in the ignore reason |
| i18n extraction broken (xgettext crash, encoding issue) | Skip POT update, add `// TODO(i18n): re-run po/update-pot.sh after fix` | Report the xgettext bug |
| Dependency compile error blocking `cargo test` | Pin previous version in `Cargo.toml`, add `// ponytail: unpin after crate X releases fix` | File upstream issue |
| Test timeout in CI but passes locally | Increase timeout in test with `// ponytail: CI is slower, revert if infra improves` | Note in PR description |

**Rules for escape hatches:**
- Never use silently — always inform the developer what was bypassed and why.
- Every workaround has a `// ponytail:` or `#[ignore]` with an issue/URL.
- Re-check at next release: if the blocker is resolved, remove the workaround.
- These do NOT apply to your own bugs — only to external/environmental factors.

### Test Run Rules (CRITICAL)

- **NEVER** pipe `cargo test` through `tail`, `grep`, or any filter. Either run it
  unpiped, or redirect the whole output to a file under `target/` and read the
  file afterwards — both preserve the full run; a pipe is what makes the shell
  tool return nothing at all. (Same rule, stated once more in
  `shell-environment.md`, which unlike this file is always loaded.)
- **NEVER** start `cargo test` if another instance is already running (`pgrep -f 'cargo test'`).
- A full `cargo test --workspace` is ~2.5 min wall (~45s of test time + ~1m49s compile, ~3900 tests, measured 2026-08-20). This is normal — wait for completion, do NOT assume timeout.
- If a hook or sub-agent already ran tests in this turn, do NOT re-run them.
- Use `timeout=900000` for test commands. This said 180s until 2026-08-20, which is *below* the measured wall time and therefore fails the same way the tool default does.

### Shared Terminal & Sub-agents (CRITICAL)

The main agent and all sub-agents (e.g. `rust-quality-check`) share ONE persistent
bash session. Concurrent or queued commands interleave, producing `Exit Code -1`,
glued-together command lines, stale output, and `bash-5.2$` prompt artifacts.
The terminal architecture cannot be fixed from rules — only the collisions can.
Apply this discipline to avoid them:

- **One terminal owner at a time.** While a sub-agent that may touch the terminal
  is running (`rust-quality-check` and any cargo-running agent), the main agent
  MUST NOT run any bash command — wait for the sub-agent's result.
- **Never delegate cargo runs to more than one sub-agent in parallel.** Centralize
  all `cargo build/clippy/test` through a single `rust-quality-check` invocation.
- **No polling loops.** Never use `sleep N; tail …` to watch progress. Run the
  command once, redirect to a log file, then read it with `readFile`.
- **Logs go inside the workspace** (`target/*.log`), never `/tmp`. Not because
  the file-reading tool cannot reach `/tmp` — it can; this claim used to say
  otherwise and was wrong — but because a log under `target/` is gitignored,
  survives for the rest of the session, and is visible to sub-agents and to the
  developer looking at the same checkout.
- **Check before launching.** Run `pgrep -f 'cargo'` first; if anything is running,
  do not start another cargo command.
- **One command per `executeBash` call.** Do not chain unrelated commands with
  `;`/`&&` into a single line that the shared shell may split incorrectly.

## 17 Translation Languages

be, cs, da, de, es, fr, it, ka, kk, nl, pl, pt, sk, sv, uk, uz, zh_CN

This list said 16 and omitted `ka` (Georgian) until 2026-08-20, while
`core-rules.md` and `AGENTS.md` both said 17 and included it. `ls po/*.po` is the
authoritative count — check it rather than any of the three prose copies.

## External Standards

In addition to the local rules above, RustConn follows:

- **[Microsoft Pragmatic Rust Guidelines](https://microsoft.github.io/rust-guidelines/)** — details and adaptation in `rust-pragmatic-guidelines.md` (auto-included for `*.rs`). Key points: `#[expect]` instead of `#[allow]`, M-PANIC-ON-BUG, `# Errors` / `# Panics` sections in public APIs, `mimalloc` as an option.
- **[GNOME HIG](https://developer.gnome.org/hig/)** — details and adaptation in `gnome-hig.md` (auto-included for `rustconn/src/**/*.rs`). Key points: `adw::AlertDialog` instead of `gtk::MessageDialog`, CSS class `suggested-action` / `destructive-action`, mandatory keyboard shortcuts (Ctrl+W, Ctrl+Q, F10), Toast vs Banner vs Dialog.
- **[Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)** — standard Rust conventions (C-CONV, C-GETTER, C-COMMON-TRAITS). Quick review scan: the [checklist](https://rust-lang.github.io/api-guidelines/checklist.html).
- **[rust-analyzer style guide](https://rust-analyzer.github.io/book/contributing/style.html)** — details and adaptation in `rust-analyzer-style.md` (auto-included for `*.rs`). Key points: prefer general borrowed types (`&str`/`&[T]`/`&Path`), preconditions in types, config struct over many bool/Option params, private field + borrowing getter (no setters), top-down file layout, control flow over clever combinators, type ascription over turbofish. Its "Do NOT adopt" section marks the rust-analyzer-internal rules that conflict with RustConn's own (`FxHashMap`, blanket `anyhow::Result`, mangled names, `#[ignore]` ban).
