---
inclusion: always
---
# Shell Environment

Rules only. The measurements and incidents behind each one are in
`shell-environment-why.md` (`#shell-environment-why`); read it when a rule here
looks arbitrary. Nothing is duplicated between the two. These rules apply to
sub-agents unchanged — they share this terminal.

## Terminal profile

`bash --noprofile --norc`: no `.bashrc`/`.profile`, no `direnv`. `~/.local/bin`
is on PATH (`uv`, `pipx`, `kiro-cli`, `kirograph`). **`~/.cargo/bin` is not** — in
bash, `sh -c` and `nohup` alike, main agent and sub-agent alike — so a bare
`cargo`, `rustfmt` or `typos` is `command not found`. A missing `typos` is worse:
it looks like a gate that ran and found nothing.

| Tool | Path |
|------|------|
| cargo / typos | `~/.cargo/bin/cargo`, `~/.cargo/bin/typos` |
| gh, flatpak-builder | system (gh is authenticated) |
| kirograph | `~/.local/bin/kirograph` |

A script that runs cargo itself needs the PATH prepended; `release.sh` fails its
first gate without it, and that refusal is correct:

```bash
PATH="$HOME/.cargo/bin:$PATH" ./scripts/release.sh --dry-run
```

## Terminal discipline

- **Multiline text never goes inline** (`--body '…'`). `fs_write` a temp file,
  pass `--body-file`, delete the file.
- **Never pipe cargo output** through any filter. Redirect to a file, read the file.
- **Logs go under `target/`**, not `/tmp`. `cargo clean` wipes them.
- **One cargo at a time** (`pgrep -x cargo` first). **One terminal owner**: no
  bash while a sub-agent works.
- **Stop background processes** you started (`list_processes`).
- **Start repo-root commands with** `cd /home/totoshko88/Documents/RustConn || exit 1`
  — the tool can lose its working directory.
- **Never wait with `sleep`**, and never spin on a PID (`wait`, `tail --pid`).
- **Pass `timeout=900000`** to any cargo build/test call, detached or not. 120 000
  (the default) and 180 000 are both below the measured wall time.

`bash-serialization-guard` enforces sleep-waiting, piped cargo, a second cargo, a
second `verify.sh`/`release.sh` and a missing timeout; it fails open, so know the
rules anyway.

## Completion: the tool returning is not the command finishing

- **`Exit Code: -1` means nothing** — it appears on nearly every call, done or not.
- End every command with `; echo "rc=$?"`. An `rc=` line means finished, with the
  real status. No `rc=` line means still running: the terminal is now **not yours**
  — no status check, no `echo`, no `^C`. Read the log with the file tool.
- **Two calls in a row without `rc=`** = busy or wedged. Stop sending it commands.
- Read a file in a *later* call than the one that writes it.

## How to run cargo — pick by duration

| Run | How |
|-----|-----|
| `cargo check`/`clippy -p <crate>`, `cargo fmt`, `scripts/test-hooks.sh`, `verify.sh --quick` | Blocking, in one call: `… > target/<name>.log 2>&1; echo "rc=$?"`, `timeout=900000`, then read the log |
| `cargo test` (any scope wider than one test), `cargo clippy --all-targets`, `verify.sh`, `verify.sh --tests` (~4 min) | **Detached by default** (below). Do other work, then read the `.rc` |
| The main tty has wedged (a redirect that should create a file creates none) | `control_bash_process(action="start", …)` with the same detached command and `PATH="$HOME/.cargo/bin:$PATH"` prepended; ignore its "not long-running" warning; `stop` it when the `.rc` appears |

```bash
cd /home/totoshko88/Documents/RustConn || exit 1
rm -f target/rc-test.log target/rc-test.rc
nohup sh -c 'PATH="$HOME/.cargo/bin:$PATH" cargo test --workspace > target/rc-test.log 2>&1; echo $? > target/rc-test.rc' >/dev/null 2>&1 &
echo "rc=started"
```

Done **exactly** when `target/rc-test.rc` exists — read it with the file tool, never
the shell. Do not poll it in a tight loop: each read is a paid call, so read it
only when you have nothing else to do, and read the log tail at most every few
reads. For the whole gate, delegate to `rust-quality-check`, which does all of this
for one call of yours.

## Cargo traps

Prefer `scripts/verify.sh` over a hand-assembled chain: it forces a real clippy
re-check and never passes `--all-features`, and an inline `sh -c` chain trips on
`${PIPESTATUS}` under `/bin/sh`.

- **A cached clippy hides warnings**: `Finished … in 0.2s` with zero warnings
  checked nothing. Force a re-check (`touch` the files or `cargo clean -p`) and
  confirm compilation happened.
- **Never `--all-features`** — a gtk3 path fails on missing `gdk-3.0.pc`. Use
  `--all-targets`.

## GUI behaviour is not evidence from this terminal

`cargo run -p rustconn` from here gets refused by the desktop portal: a
`GtkFileDialog` never completes (no result, no error, no warning) and light/dark
and the icon theme resolve wrongly. Anything a portal touches — file choosers,
theme, screen casting, notifications, monitors — must be reproduced from an
ordinary terminal. Build, clippy and test here are fine. When a GTK callback can
fail three ways, log all three.
