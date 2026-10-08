---
inclusion: manual
description: "The manual half of the Definition of Done — what needs judgement, not a command: whether new code is in the right crate, whether a ponytail marker names an honest ceiling, whether a string needs i18n. The mechanical half is run by scripts/verify.sh."
---

# Verification Checklist

Use after completing a feature or before merge. Adapted from AI-DLC methodology.

## 1. Compilation & Quality — one command

```bash
scripts/verify.sh --tests     # or without --tests for fmt + machete + clippy + rustdoc only
scripts/verify.sh --quick     # .md / .po-only work: skips every cargo gate
scripts/verify.sh --cached    # skip the pre-clippy clean; a cache hit then only warns
```

That covers `typos`, the three po gates, a GUI-imports-in-core/cli grep,
`cargo fmt --check`, `cargo machete`, `cargo clippy --all-targets` and optionally
`cargo test --workspace`. One log at `target/verify.log`, one exit code. It also
refuses to start while another cargo holds the target-dir lock, and it **warns when
clippy reported zero warnings without compiling anything** — the cache hit that
prints `Finished ... in 0.2s` and verifies nothing. Treat that warning as a failed
verification, not a pass.

A new `cargo machete` hit is either a real dead dependency or an
import-path/package-name mismatch that belongs in
`[package.metadata.cargo-machete] ignored`.

What the script cannot do, and you still must:

- [ ] `getDiagnostics` on modified files — no errors. Note that `git diff` does not
      list **untracked** new files; check those separately.
- [ ] `git status` before claiming anything is verified. A cargo run only proves
      something about the tree as it was when the run started; if files changed
      after it (another editor, a parallel session, a hook), the result is stale
      and must be re-run rather than reported.

## 1b. When a Subsystem Was Deleted

- [ ] `grep -rn "<Name>" .kiro/` — steering, POWER.md and agent descriptions all
      name concrete types, and none of it is compiler-checked. `DocumentManager`
      survived in POWER.md after the feature it described was removed.
- [ ] `./scripts/check-potfiles.sh` — a deleted GUI file leaves a stale POTFILES
      entry, which breaks `po/update-pot.sh`
- [ ] `.kiro/powers/rustconn/` and `~/.kiro/powers/installed/rustconn/` are copies
      of each other — edit both, or the installed one silently drifts

## 2. Architecture

- [ ] New code in the correct crate (core vs gui vs cli)
- [ ] No GUI imports in `rustconn-core` or `rustconn-cli`
- [ ] Public API not changed unintentionally (if changed — documented)
- [ ] New modules registered in `mod.rs`

## 3. Security

- [ ] Passwords/keys → `SecretString` (not plain String)
- [ ] No secrets in logs/errors
- [ ] CLI passwords via stdin pipe (not `.arg()`)
- [ ] Timeout on all vault/credential operations

## 4. i18n

- [ ] All user-facing strings in `i18n()` / `i18n_f()`
- [ ] File added to `po/POTFILES.in` (if new i18n strings)
- [ ] `display_name()` values wrapped in `i18n()` at call site

## 5. Testing

- [ ] New code covered by property test or integration test
- [ ] Temp files via `tempfile` crate
- [ ] Tests do not use `unwrap()`/`expect()` without reason

## 6. Documentation

- [ ] CHANGELOG.md updated (if user-facing change)
- [ ] `/// # Errors` section for new `Result` functions
- [ ] Comments for non-obvious logic
- [ ] If a module was removed/renamed → crate-level `//!` doc in `lib.rs` still matches
- [ ] If a public trait/signature changed → its example in `docs/ARCHITECTURE.md` re-verified

## 7. Cleanup

- [ ] No `dbg!`, `todo!`, `println!`, `eprintln!`
- [ ] No `#[allow(dead_code)]` on new code
- [ ] No commented-out code
- [ ] No `.clone()` where `&T` can be passed

## Quick check (delegate)

```
Delegate to rust-quality-check: "Run scripts/verify.sh --tests"
```

Delegating keeps the ~2.5 min run out of the main context and gives the sub-agent
its own terminal. Either route is fine; do not run both at once, since they would
fight over the same target-dir lock.

## When the full checklist is NOT needed

- Typo fix / comment → fmt + clippy is sufficient
- Only .md / .po files → no cargo checks needed
- Only hook/steering changes → nothing needed
