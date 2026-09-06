# AGENTS.md — RustConn

Instructions for AI coding agents. RustConn is a GTK4/libadwaita connection
manager for SSH, RDP, VNC, SPICE, Telnet, Serial, Kubernetes and Zero Trust
brokers. Rust 2024 edition, MSRV 1.95, Wayland-first, Linux and macOS.

Communication language with the maintainer: **Ukrainian**.

## How to use this file

This is the rule *list*. The reasoning behind each rule, and the detail needed to
apply it correctly, lives in `.kiro/steering/*.md` — plain markdown in this repo,
which Kiro loads automatically and every other tool has to be pointed at. Open the
file that matches your task before you start: a rule taken from here without its
steering file is a rule you will apply too literally.

Nothing below repeats a rationale, a measurement or a worked example that a
steering file already owns. That is deliberate. Two copies of the same fact drift,
and this file has already proved it twice — it told agents to write cargo logs to
`/tmp` for weeks after the rule became `target/`, and the locale count was out of
step with `ls po/*.po`.

| Task | Read |
|------|------|
| Anything — invariants, crate table, Definition of Done | `.kiro/steering/core-rules.md` |
| Running cargo, terminals, background jobs | `.kiro/steering/shell-environment.md`, and `shell-environment-why.md` when a rule there looks arbitrary |
| Code philosophy, workflow, escape hatches | `.kiro/steering/project-rules.md` |
| Adding an agent, a hook, or an `always` steering file | `.kiro/steering/cost-discipline.md` |
| GUI work — HIG, windows, dialogs | `.kiro/steering/gnome-hig.md`, `window-guide.md`, `dialogs-guide.md` |
| Credential handling | `.kiro/steering/secrets-guide.md` |
| Rust idiom | `.kiro/steering/rust-pragmatic-guidelines.md` |
| Compiler errors | `.kiro/steering/error-resolution.md` |
| CHANGELOG entries | `.kiro/steering/changelog-format.md` |
| Architecture overview | `docs/ARCHITECTURE.md` |

Rules that apply to one tree live in that tree's own `AGENTS.md`, which Kiro loads
when you work there. There are nine: one per crate, plus `po/` and `packaging/`.
They hold the things that are wrong to state globally — `rustconn-cli` inverts the
"never `println!`" rule because printing is its interface, and is untranslated
besides, and the four `-sys` crates are the only places `unsafe` is legal. Read the
local file before editing a tree; it is shorter than this one and it is more
specific.

## Commands

```bash
cargo fmt --all                                   # format
cargo clippy --all-targets                        # lint — must be 0 warnings
cargo test --workspace                            # ~45s test time, ~2.5 min with compile
cargo test -p rustconn-core --test property_tests  # property tests only
typos                                             # spell check (typos.toml)
cargo machete                                     # unused dependencies
bash po/update-pot.sh                             # after adding i18n strings
./scripts/check-potfiles.sh                       # POTFILES.in consistency (CI gate)
./scripts/check-i18n-escapes.sh                   # no \u{...} in translatable literals
./scripts/check-po-complete.sh                    # no fuzzy/missing translations
```

## Cargo and the terminal

Every rule here has a measured reason behind it in `shell-environment.md`. Read
that file before your first cargo run; these are only the conclusions.

- **Never** `cargo clippy --all-features` — it enables a gtk3 path that fails at
  build time on missing `gdk-3.0.pc`. Use `--all-targets`.
- **Never** pipe cargo output through `tail`/`grep`/`head`. Redirect to a file
  under `target/` and read the file.
- **Never** run two cargo commands at once — check `pgrep -f cargo` first.
- **Never wait with `sleep`.** A terminal that already has a live foreground job
  is not yours: bash is not reading stdin, so anything else you send queues in the
  tty buffer and then all of it runs, one command after another.
- A workspace test run is ~2.5 min wall, well past the 120 s default tool timeout,
  so a plain `cargo test` tends to return while cargo is still alive. Give the call
  explicit headroom (~900 000 ms), or background it with a sentinel file and poll
  the file.
- A repeat `cargo clippy` with nothing changed prints `Finished ... in 0.2s` and
  reports zero warnings **without checking anything**. Force a real re-check before
  claiming it passed.

The toolchain is pinned in `rust-toolchain.toml`. MSRV is a separate, older number
in `Cargo.toml` (`rust-version`).

## Crate boundaries — the rule most often broken

Seven crates. `rustconn` alone may import `gtk4`/`adw`/`vte4`. `rustconn-core` is
the domain logic, `rustconn-cli` sits on it, and neither may touch a GUI crate. The
four `rustconn-*-sys` crates are isolated FFI and the only legal home for `unsafe`,
which `unsafe_code = "deny"` enforces and each helper re-opens for itself. New FFI
gets a new `-sys` crate — never an exception where the caller lives, and never a
macOS-only crate: the `macos-sys` job covers the four helpers that exist, and
every other job is Linux, so a gated crate's guard and contract tests would be
checked by one job instead of ten. A pre-write hook blocks both violations; do not rely on it.

Per-crate contracts are in each crate's `AGENTS.md`; the full table is in
`core-rules.md`.

## Non-negotiable

- Passwords, keys, tokens → `secrecy::SecretString`, never `String`
- Intermediate `expose_secret().to_string()` → wrap in `zeroize::Zeroizing::new()`
- Secrets to external CLIs → stdin pipe, **never** `Command::arg(password)`
- Never log or format a secret into an error message
- Errors → `thiserror::Error`. No `unwrap()`/`expect()` outside tests
- Logging → `tracing`, never `println!`/`eprintln!`
- Every user-facing string in `rustconn` → `i18n()` / `i18n_f()` with `{}`
  placeholders, then `bash po/update-pot.sh`. `ls po/*.po` is the authoritative
  locale count — do not trust a number written in prose, including one written
  here. `rustconn-cli` is the exception and is English throughout; see
  `rustconn-cli/AGENTS.md` before adding an `i18n()` call there, because one
  wrapped string among hundreds of bare ones is worse than none
- Never `std::env::set_var`/`remove_var` (unsafe in Rust 2024). The sole exception
  is `rustconn-env-sys::set_startup_var`, and its window is already sealed by two
  existing callers — see `rustconn-env-sys/AGENTS.md` before considering a third

Widget choice, accessible labels and dialog patterns are in `rustconn/AGENTS.md`;
catalogue mechanics are in `po/AGENTS.md`. Both load when you work there.

## Definition of done

1. `cargo clippy --all-targets` → 0 warnings, from a run that actually re-checked
2. Relevant tests green
3. Crate boundaries intact
4. New strings wrapped in `i18n()` and POT regenerated
5. No `dbg!`/`todo!`/`println!`/`eprintln!` left behind
6. `CHANGELOG.md` updated for any user-facing change

If you cannot reach this, stop and say what is blocking. Do not drop a test,
silence a lint, or skip i18n to make it look finished. The sanctioned workarounds
are for *external* blockers only and are listed in `project-rules.md`.

## Style

Minimum viable change. Before writing code, stop at the first rung that holds:
does it need to exist; does this repo already have it; does `std` cover it; is
there a GTK4/libadwaita feature for it; does an existing dependency do it. Prefer
deleting over adding, and boring over clever. No new dependency, abstraction or
generic that was not asked for.

Fix root causes, not symptoms: if a bug report names one call site, check every
caller of the function you touch and fix the shared function once.

Mark a deliberate simplification with `// ponytail:` naming the ceiling and the
upgrade path, e.g. `// ponytail: O(n²) scan, fine for <100 hosts; index if the
list grows`.

Do not be lazy about input validation at trust boundaries, error handling that
prevents data loss, credential handling, accessibility, or tests.

## Commits and releases

Conventional commits: `type(scope): description`, imperative, lowercase, no
trailing period. Types: feat, fix, docs, style, refactor, test, chore, perf, ci,
build. Scopes: rustconn-core, rustconn-cli, rustconn (gui), i18n, packaging, ci.

**A finished feature gets a changelog entry and a commit.** In that order, once the
Definition of Done holds. Stage only the files this session edited —
`target/.kiro-session-edits`, kept by the `edit-journal` hook — never `git add -A`:
this checkout is regularly shared with the IDE and a second session.

**Never `git push`.** Not a branch, not a tag, not any remote. Commit locally, say
what is ready, and hand over. `git push --dry-run` is fine to show what would go.

**An agent prepares a release; it never cuts one.** `./scripts/release.sh
--dry-run` is the agent action and is expected: it runs every gate and stops before
the plan executes. Running the script for real, passing `--yes`, or tagging by hand
is the maintainer's call. Report the dry-run gate list and the diff, then hand over.
What goes wrong otherwise, and the mechanics of each channel: `packaging/AGENTS.md`
and `core-rules.md`. The `release-manual-only-guard` hook enforces every route,
including the blanket push refusal, but do not rely on it.
