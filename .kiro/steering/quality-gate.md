---
inclusion: manual
description: "Unified quality gate: quick (fmt+clippy), full (fmt+clippy+tests), or tests-only. Invoke the appropriate section."
---

# Quality Gate

Single entry point for all code quality checks. Use the section matching the request.

## Quick (fmt + clippy)

Use before committing or when the developer asks for a quick check.

Invoke the `rust-quality-check` sub-agent with prompt:
"Run fmt and clippy checks. Do NOT run tests unless explicitly requested."

Report the result. If all pass — remind to commit.

## Full (fmt + clippy + tests)

Use when finishing a feature, before release, or when explicitly asked for "full checks".

Run sequentially in workspace root:

1. `cargo fmt --check` — if formatting errors, run `cargo fmt --all`, report changes.
2. `cargo clippy --all-targets -- -D warnings` — must produce 0 warnings. Fix and re-run if any.
3. Before tests: `pgrep -f 'cargo test'` — if running, report "Tests already in progress, skipping" and stop.
4. `cargo test --workspace` — run directly, NO pipes (no tail/grep). Allow 180s timeout (argon2 ~120s is normal).

Report pass/fail for each step. If tests fail — list failing test names.

## Tests only

Use when the developer explicitly asks to run tests.

1. `pgrep -f 'cargo test'` — if running, report "Tests already in progress, skipping."
2. `cargo test --workspace` — run directly, NO pipes. Allow 180s.
3. Report final summary (e.g. "test result: ok. 42 passed; 0 failed"). If failures — list test names.

## Rules (all modes)

- **Never** pipe cargo output through `tail`, `grep`, or any filter.
- **Never** start cargo if another instance is already running (`pgrep -f 'cargo'`).
- Tests take ~120s (argon2 property tests in debug) — this is normal, do not assume timeout.
- One terminal owner at a time — do not run bash while a sub-agent is active.
