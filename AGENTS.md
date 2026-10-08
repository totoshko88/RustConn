# AGENTS.md — RustConn

Instructions for AI coding agents. RustConn is a GTK4/libadwaita connection
manager for SSH, RDP, VNC, SPICE, Telnet, Serial, Kubernetes and Zero Trust
brokers. Rust 2024 edition, MSRV 1.95, Wayland-first, Linux and macOS.

Communication language with the maintainer: **Ukrainian**.

## How to use this file

This is the rule *list*. Reasoning and detail live in `.kiro/steering/*.md`
(Kiro loads them automatically; other tools must be pointed at them). Open the
one matching your task: a rule taken from here without its steering file gets
applied too literally. Nothing here repeats what a steering file owns — two copies
of one fact drift.

| Task | Read |
|------|------|
| Crate table, `-sys` mechanics, detail behind the rules below | `.kiro/steering/core-rules.md` |
| Running cargo, terminals, background jobs | `.kiro/steering/shell-environment.md` (+ `-why.md`) |
| Code philosophy, workflow, escape hatches | `.kiro/steering/project-rules.md` |
| Adding an agent, a hook, a steering file; delegating | `.kiro/steering/cost-discipline.md` |
| GUI work — HIG, windows, dialogs | `.kiro/steering/gnome-hig.md`, `window-guide.md`, `dialogs-guide.md` |
| Credential handling | `.kiro/steering/secrets-guide.md` |
| Rust idiom | `.kiro/steering/rust-pragmatic-guidelines.md` |
| Compiler errors | `.kiro/steering/error-resolution.md` |
| CHANGELOG entries | `.kiro/steering/changelog-format.md` |
| Architecture overview | `docs/ARCHITECTURE.md` |

Each crate, `po/` and `packaging/` has its own `AGENTS.md` with the rules that are
wrong to state globally (`rustconn-cli` prints on purpose and is untranslated; the
`-sys` crates are the only home of `unsafe`). Read it before editing that tree.

## Commands

The mechanical Definition of Done is one script; prefer it over reassembling the
commands below, and never use an inline `sh -c` cargo chain:

```bash
scripts/verify.sh --tests   # fmt + machete + clippy -D warnings + rustdoc -D warnings + tests + i18n/boundary gates
                            # (log at target/verify.log; ~4 min — run it detached, see shell-environment.md)
scripts/verify.sh --quick   # fast gates only — .md / .po-only work
```

```bash
cargo fmt --all                                   # format
cargo clippy --all-targets                        # lint — 0 warnings; never --all-features (gtk3 path breaks)
cargo test --workspace                            # ~2.5 min wall
cargo test -p rustconn-core --test property_tests  # property tests only
typos                                             # spell check (typos.toml)
cargo machete                                     # unused dependencies
bash po/update-pot.sh                             # after adding i18n strings
./scripts/check-potfiles.sh                       # POTFILES.in consistency (CI gate)
./scripts/check-i18n-escapes.sh                   # no \u{...} in translatable literals
./scripts/check-po-complete.sh                    # no fuzzy/missing translations
```

A repeat `cargo clippy` with nothing changed prints `Finished … in 0.2s` and
checks nothing — force a real re-check before claiming a pass. Never pipe cargo
output, never run two cargo at once, never wait with `sleep`. The toolchain is
pinned in `rust-toolchain.toml`; MSRV is `rust-version` in `Cargo.toml`.

## Crate boundaries — the rule most often broken

Seven crates. `rustconn` alone may import `gtk4`/`adw`/`vte4`; `rustconn-core`
and `rustconn-cli` may not. The four `rustconn-*-sys` crates are isolated FFI and
the only legal home for `unsafe` (`unsafe_code = "deny"`, re-opened per helper).
New FFI gets a new `-sys` crate — never an exception where the caller lives, and
never a macOS-only crate. A pre-write hook blocks both violations; do not rely on it.

## Non-negotiable

- Passwords, keys, tokens → `secrecy::SecretString`, never `String`
- Intermediate `expose_secret().to_string()` → wrap in `zeroize::Zeroizing::new()`
- Secrets to external CLIs → stdin pipe, **never** `Command::arg(password)`
- Never log or format a secret into an error message
- Errors → `thiserror::Error`. No `unwrap()`/`expect()` outside tests
- Logging → `tracing`, never `println!`/`eprintln!`
- Every user-facing string in `rustconn` → `i18n()` / `i18n_f()` with `{}`
  placeholders, then `bash po/update-pot.sh`. `ls po/*.po` is the locale count —
  never a number in prose. `rustconn-cli` is English throughout (see its `AGENTS.md`)
- Never `std::env::set_var`/`remove_var` (unsafe in Rust 2024). The sole exception
  is `rustconn-env-sys::set_startup_var`, whose window is sealed by its two
  callers — see `rustconn-env-sys/AGENTS.md`

## Definition of done

1. `cargo clippy --all-targets` → 0 warnings, from a run that actually re-checked
2. Relevant tests green
3. Crate boundaries intact
4. New strings wrapped in `i18n()` and POT regenerated
5. No `dbg!`/`todo!`/`println!`/`eprintln!` left behind
6. `CHANGELOG.md` updated for any user-facing change

If you cannot reach this, stop and say what is blocking. Do not drop a test,
silence a lint, or skip i18n to make it look finished. Sanctioned workarounds are
for *external* blockers only (`project-rules.md`).

## Style

Minimum viable change. Stop at the first rung that holds: does it need to exist;
does this repo already have it; does `std` cover it; is there a GTK4/libadwaita
feature for it; does an existing dependency do it. Prefer deleting over adding,
boring over clever. No new dependency, abstraction or generic that was not asked for.

Fix root causes: if a bug report names one call site, check every caller of the
function you touch and fix the shared function once.

Mark a deliberate simplification with `// ponytail:` naming the ceiling and the
upgrade path, e.g. `// ponytail: O(n²) scan, fine for <100 hosts; index if the
list grows`.

Do not be lazy about input validation at trust boundaries, error handling that
prevents data loss, credential handling, accessibility, or tests.

## Commits and releases

Conventional commits: `type(scope): description`, imperative, lowercase, no
trailing period. Types: feat, fix, docs, style, refactor, test, chore, perf, ci,
build. Scopes: rustconn-core, rustconn-cli, rustconn (gui), i18n, packaging, ci.

- **A finished feature gets a changelog entry, then a commit**, once the Definition
  of Done holds. Stage only the files this session edited —
  `target/.kiro-session-edits` — never `git add -A`/`git add .`.
- **Never `git push`** — no branch, no tag, no remote. Commit locally and hand
  over. `git push --dry-run` is fine.
- **An agent prepares a release; it never cuts one.** `./scripts/release.sh
  --dry-run` is the agent action. Running it for real, passing `--yes`, or tagging
  `v<x.y.z>` by hand is the maintainer's call. Report the dry-run gate list and the
  diff, then stop. `release-manual-only-guard` enforces every route; do not rely on
  it. Channel mechanics: `packaging/AGENTS.md`.
