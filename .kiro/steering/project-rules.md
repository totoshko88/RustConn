---
inclusion: always
---

# RustConn — Project Rules

Communication language: Ukrainian.

## Architecture (4 crates)

| Crate | Purpose | Restrictions |
|-------|---------|-------------|
| `rustconn-core` | Domain logic: models, config, CRUD managers, import/export, protocol data, credential abstractions | **FORBIDDEN**: gtk4, adw, vte4. Default features stay headless/empty. Embedded clients, GFX, RD Gateway, and host keyring are optional features only. |
| `rustconn-cli` | Headless management over core data | Only rustconn-core. Default features stay minimal; client launch and secret-management paths are optional. |
| `rustconn` | GTK4/libadwaita GUI, dialogs, embedded/external session presentation | May import GUI crates and enable core integration features |
| `rustconn-pty-sys` | Isolated FFI helper (macOS PTY controlling terminal, `setsid`+`TIOCSCTTY`) | **Only** sanctioned `unsafe` location (M-UNSAFE); `libc` only, no gtk4/adw/vte4 |

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

## Absolute Rules

- `unsafe_code = "forbid"` in all crates **except** `rustconn-pty-sys` — the single sanctioned FFI location (M-UNSAFE). Never write `unsafe` anywhere else.
- Passwords/keys → `secrecy::SecretString`, never plain String
- Intermediate `expose_secret().to_string()` → wrap in `zeroize::Zeroizing::new()`
- Errors → `thiserror::Error`, never `unwrap()`/`expect()`
- Logging → `tracing`, never `println!`/`eprintln!`
- i18n → `i18n()` / `i18n_f()` with `{}` placeholders for all user-facing strings
- `display_name()` values used in UI → wrap in `i18n()` at call site
- After new i18n strings → `bash po/update-pot.sh` + `msgmerge --update` for 16 languages
- Rust 2024 edition: let-chains instead of collapsible_if
- Never `set_var`/`remove_var` (unsafe in Rust 2024)

## Quick Commands

```
cargo fmt --all                    # Format
cargo clippy --all-targets         # Lint (0 warnings)
cargo test --workspace             # Tests (~120s, argon2 is slow)
cargo test -p rustconn-core --test property_tests  # Property tests only
bash po/update-pot.sh              # Regenerate POT after new i18n strings
```

## Quality Checks

Delegate to `rust-quality-check` sub-agent for fmt+clippy+tests instead of running in main context.
For quick single-file validation → `getDiagnostics`.

### Self-Check Rules (hooks + mental)

The hardest-to-reverse invariants are now enforced automatically by the
`crate-boundary-guard` preToolUse hook (it denies the write if a `.rs` change
adds GUI imports to `rustconn-core`/`rustconn-cli`, or `unsafe` outside
`rustconn-pty-sys`). Still verify them yourself BEFORE writing — the hook is a
safety net, not an excuse to skip thinking:
- **Crate boundary**: `rustconn-core/` and `rustconn-cli/` must NOT contain `use gtk4`, `use adw`, `use vte4`, `gtk4::`, `adw::`, `vte4::`. Move GUI code to `rustconn/`. *(hook-enforced)*
- **No unsafe**: never write `unsafe {`, `unsafe fn`, `unsafe impl`, `unsafe trait` — **except** in `rustconn-pty-sys` (the sole sanctioned FFI crate, M-UNSAFE). New `unsafe` outside it is forbidden. *(hook-enforced)*

After writing `.rs` files in `rustconn/src/`, verify (these stay mental — caught later by clippy + the `post-session-diagnostics` agentStop hook, not pre-write):
- **i18n**: all user-facing strings (`.set_label()`, `.set_title()`, `.set_tooltip_text()`, `Button::with_label()`) wrapped in `i18n()` or `i18n_f()`. Ignore: tracing, CSS, icons, action names.
- **Credentials** (in secret/password/credential files): `SecretString` for passwords, `.zeroize()` intermediates, no secrets in logs/args/errors. *(editing these files also triggers the `security-review` hook → `security-reviewer` sub-agent)*
- **Protocol files**: business logic in rustconn-core, GTK in rustconn.

### When to Run fmt/clippy/tests

- **Do NOT** run `cargo fmt`/`cargo clippy` automatically on every change — use `getDiagnostics` for quick validation.
- Run `rust-quality-check` sub-agent only when: (a) about to commit, (b) user explicitly asks, (c) finishing a multi-file feature.
- Run tests only when: (a) user explicitly asks, (b) finishing a spec task, (c) before release.
- After completing work, inform the user: "Done. Run quality check (fmt+clippy)?" — wait for confirmation.

### Learning Loop (after non-trivial tasks)

When a task surfaced something worth keeping for next time — an interpretation of an ambiguous request, an intentional deviation from these rules, or a tradeoff between two approaches — record it once with `kirograph_mem_store` (kind `decision`/`pattern`), or propose a one-line addition to a `.kiro/steering` file if it's a durable project rule. Fire only when there's a real lesson; most tasks add nothing, and that's fine. Don't re-store what memory already holds — `kirograph_mem_search` first.

### Definition of Done (goal-loop acceptance gate)

A task is done ONLY when all hold — this is the finish line for `/goal` loops and any self-verification:

1. `cargo clippy --all-targets` → 0 warnings
2. `cargo test --workspace` green (or the targeted tests for the change)
3. Crate boundaries intact (no gtk4/adw/vte4 in core/cli, no `unsafe` outside `rustconn-pty-sys`)
4. New user-facing strings wrapped in `i18n()`/`i18n_f()` + POT updated (`bash po/update-pot.sh`)
5. No debug leftovers (`dbg!`/`todo!`/`println!`/`eprintln!`)

If a goal-loop can't reach this within its iterations, STOP and report what's blocking — never loosen the gate (drop a test, silence clippy, skip i18n) just to "finish".

### Test Run Rules (CRITICAL)

- **NEVER** pipe `cargo test` through `tail`, `grep`, or any filter — run directly to see progress.
- **NEVER** start `cargo test` if another instance is already running (`pgrep -f 'cargo test'`).
- Tests take ~120s (argon2 property tests). This is normal — wait for completion, do NOT assume timeout.
- If a hook or sub-agent already ran tests in this turn, do NOT re-run them.
- Use timeout 180s for test commands.

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
- **Logs go inside the workspace** (`target/*.log`), never `/tmp` — `readFile` is
  restricted to the workspace and cannot read `/tmp`.
- **Check before launching.** Run `pgrep -f 'cargo'` first; if anything is running,
  do not start another cargo command.
- **One command per `executeBash` call.** Do not chain unrelated commands with
  `;`/`&&` into a single line that the shared shell may split incorrectly.

## 16 Translation Languages

be, cs, da, de, es, fr, it, kk, nl, pl, pt, sk, sv, uk, uz, zh-cn

## External Standards

In addition to the local rules above, RustConn follows:

- **[Microsoft Pragmatic Rust Guidelines](https://microsoft.github.io/rust-guidelines/)** — details and adaptation in `rust-pragmatic-guidelines.md` (auto-included for `*.rs`). Key points: `#[expect]` instead of `#[allow]`, M-PANIC-ON-BUG, `# Errors` / `# Panics` sections in public APIs, `mimalloc` as an option.
- **[GNOME HIG](https://developer.gnome.org/hig/)** — details and adaptation in `gnome-hig.md` (auto-included for `rustconn/src/**/*.rs`). Key points: `adw::AlertDialog` instead of `gtk::MessageDialog`, CSS class `suggested-action` / `destructive-action`, mandatory keyboard shortcuts (Ctrl+W, Ctrl+Q, F10), Toast vs Banner vs Dialog.
- **[Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)** — standard Rust conventions (C-CONV, C-GETTER, C-COMMON-TRAITS).
