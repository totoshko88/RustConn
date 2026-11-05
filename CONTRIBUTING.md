# Contributing to RustConn

Thanks for being here. RustConn is a GTK4/libadwaita connection manager written
in Rust, and it grows from bug reports, translations, packaging fixes, and code
alike. This file explains how to get a change merged.

By participating you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).

## Ways to contribute

| | Where to start |
|---|---|
| **Report a bug** | [New issue](https://github.com/totoshko88/RustConn/issues/new/choose) — the bug form asks for the version, install method, and desktop, all of which matter |
| **Request a feature** | [New issue](https://github.com/totoshko88/RustConn/issues/new/choose) — describe the workflow you are trying to complete, not only the widget you want |
| **Translate** | `.po` files in [`po/`](po/) — see [Translations](#translations) below |
| **Fix or build something** | Read [Development setup](#development-setup), then [The quality gate](#the-quality-gate) |
| **Package for a distro** | [`packaging/`](packaging/) holds the Flatpak, OBS, Snap, nixpkgs, and macOS manifests |
| **Improve the docs** | [`docs/`](docs/) — the user guide and CLI reference are the two most read |
| **Report a vulnerability** | **Not** in an issue. Use [Security Advisories](https://github.com/totoshko88/RustConn/security/advisories/new); see [SECURITY.md](SECURITY.md) |

For anything larger than a bug fix, open an issue first. It is cheaper to agree
on the approach in a paragraph than to rewrite a finished branch.

## Development setup

Prerequisites, per-distro system packages, and every feature flag are in the
[Build Guide](docs/BUILD.md). The short version on a Debian-family system:

```bash
sudo apt install build-essential libgtk-4-dev libvte-2.91-gtk4-dev \
    libadwaita-1-dev libdbus-1-dev libssl-dev pkg-config libasound2-dev \
    clang cmake gettext

git clone https://github.com/totoshko88/RustConn.git
cd RustConn
cargo build
cargo run --bin rustconn
```

The toolchain is pinned in `rust-toolchain.toml`, so rustup picks the right
compiler on its own. MSRV is a separate, older number (`rust-version` in
`Cargo.toml`, currently 1.95) and CI checks it in its own job — avoid syntax and
APIs newer than that.

[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) is worth a skim before your first
patch; it explains why the code is split the way it is.

## The quality gate

CI runs twelve jobs and a red one blocks the merge. You can reproduce all of the
important ones locally:

```bash
cargo fmt --all                    # formatting (CI runs --check)
cargo clippy --all-targets         # must be 0 warnings
cargo test --workspace             # ~2.5 min including the compile
typos                              # spell check, configured in typos.toml
cargo machete                      # unused dependencies
```

Two traps worth knowing before they cost you an hour:

- **Never `cargo clippy --all-features`.** It turns on a gtk3-dependent path
  that fails at build time on a missing `gdk-3.0.pc`. Use `--all-targets`.
- **A repeat clippy run with nothing changed reports nothing.** It prints
  `Finished ... in 0.2s` and zero warnings without inspecting anything. Touch the
  files you changed, or `cargo clean -p <crate>`, if you need a real answer.

Beyond that, CI also runs the i18n gates (`scripts/check-potfiles.sh`,
`check-i18n-escapes.sh`, `check-pot-current.sh`, `check-po-complete.sh`),
`cargo-deny` for licences and advisories, and a macOS job for the `-sys` crates.
`./scripts/verify.sh` runs the whole battery in one go if you would rather not
remember the list — `--quick` for docs- or translation-only work, `--tests` to
include `cargo test --workspace`. It writes everything to `target/verify.log`.

## Rules that will get a PR sent back

These are structural, not stylistic, and they are the ones most often missed.

**Crate boundaries.** There are eight crates and only `rustconn` may import
`gtk4`, `adw`, or `vte4`. `rustconn-core` is the domain logic and
`rustconn-cli` sits on top of it; neither may touch a GUI crate. Business logic
belongs in core even when the bug was reported against a dialog.

**`unsafe` lives in exactly five crates.** `rustconn-pty-sys`,
`rustconn-locale-sys`, `rustconn-env-sys`, `rustconn-dock-sys`, and
`rustconn-lock-sys` are the isolated FFI helpers, and the workspace denies
`unsafe_code` everywhere else. New FFI gets a new `-sys` crate rather than an
exception where the caller lives.

**Credentials.** Passwords, keys, and tokens are `secrecy::SecretString`, never
`String`. An intermediate `expose_secret().to_string()` gets wrapped in
`zeroize::Zeroizing::new()`. Secrets reach external CLIs over a stdin pipe,
never as `Command::arg(password)`, and never appear in a log line or an error
message.

**Errors and logging.** Errors are `thiserror::Error` types; no `unwrap()` or
`expect()` in production code (both are fine in tests). Logging goes through
`tracing` — no `println!` or `eprintln!`, except in `rustconn-cli`, where
printing *is* the interface.

**Every user-facing string is translatable.** Wrap it in `i18n()` or `i18n_f()`
with `{}` placeholders, then regenerate the template with
`bash po/update-pot.sh`. A source file containing `i18n()` must also be listed in
`po/POTFILES.in` or its strings are silently dropped.

**No `std::env::set_var` / `remove_var`.** They are `unsafe` in Rust 2024. The
single exception is `rustconn-env-sys::set_startup_var`, whose window is already
closed by its two existing callers.

**Accessibility.** New widgets need accessible labels, and dialogs follow the
[GNOME HIG](https://developer.gnome.org/hig/). `adw::AlertDialog` rather than
`gtk::MessageDialog`.

The full set, with the reasoning behind each rule, lives in
[`.kiro/steering/`](.kiro/steering/) — `core-rules.md` is the entry point. Each
crate also has its own `AGENTS.md` with contracts specific to that tree.

## Style

Minimum viable change. Before adding code, stop at the first of these that
holds: does it need to exist; does this repo already have it; does `std` cover
it; is there a GTK4/libadwaita feature for it; does an existing dependency do
it. Prefer deleting over adding and boring over clever. No new dependency,
abstraction, or generic that the change did not require.

Fix root causes. If a report names one call site, check the other callers of the
function you are touching and fix the shared function once.

A deliberate simplification gets a `// ponytail:` comment naming the ceiling and
the upgrade path, for example:

```rust
// ponytail: O(n²) scan, fine for <100 hosts; index if the list grows
```

Non-trivial logic leaves behind one runnable check — a property test in
`rustconn-core/tests/properties/` or an integration test in
`rustconn-core/tests/integration/`, both registered in the directory's `mod.rs`.
For a `-sys` crate it is the contract test that asserts the precondition guard,
since the FFI call itself is not reachable from a test harness.

## Commits

[Conventional commits](https://www.conventionalcommits.org/), imperative mood,
lowercase, no trailing period:

```
fix(rustconn-core): resolve jump host before launching SFTP
```

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`, `perf`,
`ci`, `build`. Scopes: `rustconn-core`, `rustconn-cli`, `rustconn` (GUI),
`i18n`, `packaging`, `ci`. Use the commit body for *why*, not *what* — the diff
already covers what.

Keep unrelated changes in separate commits.

## Changelog

Any user-facing change gets a `CHANGELOG.md` entry under `## [Unreleased]`,
following [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Sections in
this order, empty ones omitted: **Added**, **Fixed**, **Changed**, **Removed**,
**Improved**, **Documentation**, **Dependencies**. Each entry opens with a bold
summary and an optional issue reference, then explains itself in the same
bullet:

```markdown
### Fixed

- **Jump host ignored by SFTP launches (issue #123)** — the SFTP path read the
  picker's field directly instead of the three-tier resolver, so a bastion
  inherited from a group was dropped.
```

## Translations

RustConn ships every catalogue listed in `po/LINGUAS`. `ls po/*.po` is the
authoritative list — numbers written in prose have been wrong before.

To correct or complete an existing language, edit its `po/<lang>.po` and run the
three gates:

```bash
./scripts/check-potfiles.sh      # POTFILES.in matches reality
./scripts/check-i18n-escapes.sh  # no \u{...} in translatable literals
./scripts/check-po-complete.sh   # no fuzzy or missing entries
```

Fuzzy entries matter as much as missing ones: gettext ignores a fuzzy string and
falls back to English, so it looks translated in the statistics and untranslated
on screen.

For a new language, copy `po/rustconn.pot` to `po/<lang>.po`, add the code to
`po/LINGUAS`, and translate. Placeholders are `{}`, positional, in source order —
a translation may not reorder or drop one. Write characters directly rather than
as `\u{...}` escapes; the escape survives extraction and reaches the user
verbatim.

More detail in [`po/AGENTS.md`](po/AGENTS.md).

## Pull requests

1. Branch off `main`.
2. Make the change, with a test if the logic is non-trivial.
3. Run the quality gate above.
4. Update `CHANGELOG.md` if the change is user-facing.
5. Open the PR. The template asks you to confirm the same list.

Do not silence a lint, drop a test, or skip `i18n()` to make a branch look
finished. If something external is genuinely blocking you — an upstream bug, a
new compiler lint — say so in the PR and it will get sorted out together.

Small, reviewable PRs land faster than large ones. If a change spans crate
boundaries, name the boundary in the description and keep each layer readable on
its own.

## Releases

Releases are cut by the maintainer. `./scripts/release.sh` runs every gate and
then tags, pushes, and triggers the artifact build plus the Flathub, OBS, and
Snap updates, so please do not run it, pass `--yes`, or push a `v*` tag on a
contribution. A `--dry-run` is welcome if you want to check that your change
does not break the release gates.

## Licence

RustConn is GPL-3.0-or-later. By submitting a contribution you agree that it is
licensed under the same terms. There is no CLA.

## Getting help

- [User Guide](docs/USER_GUIDE.md) and [CLI Reference](docs/CLI_REFERENCE.md)
- [Discussions](https://github.com/totoshko88/RustConn/discussions) for questions
- [Issues](https://github.com/totoshko88/RustConn/issues) for bugs and feature requests
