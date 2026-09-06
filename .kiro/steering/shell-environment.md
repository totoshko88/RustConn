---
inclusion: always
---
# Shell Environment

`inclusion: always` on purpose — terminal discipline is cheap to carry and
expensive to omit. This file holds the **rules only**. Every measurement, war
story and worked example behind them is in `shell-environment-why.md`
(`inclusion: manual`, load with `#shell-environment-why`); read it once, or when a
rule here looks arbitrary. Nothing is duplicated between the two.

If you ever set a steering file to `inclusion: auto`, give it both `name` and
`description` — without them it matches nothing and silently never loads.

## Terminal profile

`bash --noprofile --norc` with PATH injected by the terminal profile:

- No `.bashrc`, `.profile` or `/etc/profile` is sourced
- `cargo`, `rustfmt`, `clippy` at `~/.cargo/bin/`
- `~/.local/bin/` in PATH (`uv`, `pipx`, `kiro-cli`, user scripts)
- `direnv` is **not** active

**Sub-agents do not reliably inherit that PATH.** Use the absolute
`~/.cargo/bin/cargo` in anything a sub-agent runs. A sub-agent reporting
`cargo: command not found` is this, not a broken toolchain.

| Tool | Path |
|------|------|
| cargo | `~/.cargo/bin/cargo` |
| gh | system, authenticated |
| flatpak-builder | system |
| kirograph | `~/.nvm/.../bin/kirograph` (when `.kirograph/` exists) |

## Multiline text in shell commands

**Never pass multiline text inline** (e.g. `--body '…'` with newlines). Write it to
a temp file with `fs_write`, pass the file (`gh issue comment --body-file …`), then
delete the file.

## Terminal discipline

- **Never pipe cargo output** through `tail`, `grep`, `head` or any filter.
  Redirect to a file and read the file.
- **Logs go under `target/`, not `/tmp`** — gitignored, visible to sub-agents and
  to the developer in the same checkout, and they survive the session. `cargo
  clean` wipes them, so copy a log you still need first.
- **One cargo at a time.** `pgrep -f cargo` before any build or test.
- **One terminal owner.** Do not run bash while a sub-agent is working.
- **Stop background processes when done.** `list_processes`, then stop what you
  started.
- **The shell tool can lose its working directory** between calls. Start anything
  that depends on the repo root with
  `cd /home/totoshko88/Documents/RustConn || exit 1`.
- **Empty output twice in a row** — stop retrying. Delegate to
  `rust-quality-check`, or write to a log file and read it with the file-reading
  tool.
- **A full `cargo test --workspace` is ~2.5 min wall** (~1m49s compile + ~45s of
  tests, ~3900 tests). That is normal, not a hang. Read the run's own
  `test result:` lines for the real count.
- **Never wait with `sleep`.** A sleep cannot observe another terminal, and if the
  terminal is busy the line queues behind the running job instead of executing.
- **Pass an explicit `timeout`** to any cargo build or test — the tool default is
  120 000 ms, below the measured wall time. Use `timeout=900000`. Not 180 000;
  that is also below it.

The `bash-serialization-guard` hook enforces the four of these that are
mechanically checkable: sleep waiting, piped cargo output, a second concurrent
cargo, and a cargo run without timeout headroom. It fails open — a faster failure,
never a substitute for knowing the rules.

## Waiting without blocking the terminal

**Once a terminal has a live foreground job, it is not yours.** Do not send it
another command — not a status check, not an `echo`, not a `^C`. Read the log file.

Three ways out, cheapest first.

**1. Wait inside the one tool call.** Almost always right.

```bash
cd /home/totoshko88/Documents/RustConn || exit 1
cargo test --workspace > target/rc-test.log 2>&1
```

with `timeout=900000`, then read `target/rc-test.log` (the file-reading tool takes
line ranges, so a 20 k-line log costs nothing).

**2. Take a handle when you want to keep working.** Poll the filesystem, never the
clock — the run is done exactly when the `.rc` file appears.

```bash
cd /home/totoshko88/Documents/RustConn || exit 1
rm -f target/rc-test.log target/rc-test.rc
nohup sh -c 'cargo test --workspace > target/rc-test.log 2>&1; echo $? > target/rc-test.rc' >/dev/null 2>&1 &
```

Pass `timeout=900000` here too: the call returns immediately so it is never
reached, but the guard cannot tell a detached run from a foreground one and blocks
the form without it.

**3. Delegate.** `rust-quality-check` owns its own terminal.

## Cargo traps in this workspace

- **A cached clippy run hides warnings.** With nothing changed it prints
  `Finished … in 0.2s` and reports zero warnings *even when warnings exist*. Force
  a real re-check (`touch` the `.rs` files, or `cargo clean -p <crate>`) and
  confirm from the output that compilation happened.
- **Never `--all-features`.** It enables a gtk3 path that fails on missing
  `gdk-3.0.pc`. Use `--all-targets`.

## Never judge GUI behaviour from an app launched in this terminal

A `cargo run -p rustconn` started here produces a process the desktop portal
refuses (`Unable to open /proc/<pid>/root`). Two measured consequences: a
`GtkFileDialog` **never completes** — no result, no error, no `Dismissed`, no
warning, indistinguishable from an unwired button — and light/dark plus the icon
theme resolve wrongly, because the portal is where they come from.

This terminal is fine for `cargo build`, `clippy` and `test`. It is **not evidence**
about anything a portal touches: file choosers, the light/dark preference, the icon
theme, screen casting, notifications, the monitor list. Reproduce that class of bug
in an ordinary terminal before chasing it.

Corollary worth keeping: **when a GTK callback can fail three ways, log all three.**
The absence of a line is then evidence too.
