---
inclusion: manual
description: "The measurements and incidents behind the terminal rules in shell-environment.md: the queued-sleep failure, cargo timings, the /tmp-to-target correction, the portal-blocked GUI diagnosis. Load with #shell-environment-why when a rule there looks arbitrary."
---

# Shell Environment — Why

Companion to `shell-environment.md`, which is `inclusion: always` and holds the
rules. This file holds the evidence: what was measured, what went wrong, and what
each rule cost to learn. Split out on 2026-09-06 — a rule is worth carrying in
every request, its origin story is not, and at 1 700 words the combined file was
paying to re-explain itself on every turn.

No rule lives only here. If you find one, move it to the always file.

## The queued-sleep failure

The expensive failure is not a slow build, it is trying to wait for one.

`cargo test --workspace` starts with the default 120 s timeout → the tool returns
while cargo is still running → the wait looks necessary → `sleep 115; echo W10` is
sent to the *same* terminal → bash is not reading stdin while a foreground job
runs, so the line sits in the tty buffer, and so do the next eighteen → cargo
exits, bash drains the buffer and runs every queued sleep back to back.

Nineteen queued `sleep 115` is 36 minutes of nothing, and each one looks like a
command that legitimately timed out. This is the reason for three separate rules:
never wait with `sleep`, always pass explicit timeout headroom, and treat a
terminal with a live foreground job as not yours.

The detached form was verified by probe on 2026-09-02: it returns
immediately, so its timeout is never reached, but `bash-serialization-guard` cannot
distinguish it from a foreground run and blocks it without `timeout=900000`.

## Cargo timings

Measured 2026-08-20: a full `cargo test --workspace` is ~2.5 min wall — 1m49s
compile plus ~45 s of test time.

The test count is a moving number: 3843 on 2026-08-20, 3874 six days later, ~3900
now. Treat any figure written in prose as an order of magnitude and read the run's
own `test result:` lines.

The single slowest test is the argon2 credential round-trip at ~38 s. It was ~193 s
until `[profile.test.package.argon2] opt-level = 3` was added — worth knowing if
that profile entry ever gets removed as "unnecessary".

## Logs: `/tmp` versus `target/`

The rule is `target/`, and the reason is not access — the file-reading tool reaches
both. A log under `target/` is gitignored, is visible to sub-agents and to the
developer looking at the same checkout, and survives the rest of the session.

The examples in the always file used `/tmp` until 2026-08-20, contradicting
`project-rules.md`, which already had the rule and the better reason for it. Two
copies of one rule drifted, which is why neither file restates the other now.

## Background processes

A `control_bash_process` job left running holds its terminal. They accumulate:
roughly 30 stray jobs once wedged the cargo lock and the shell tool together, which
looks like a broken environment rather than a housekeeping problem.

## `cargo: command not found`, main agent and sub-agent alike

`rust-quality-check` reported on 2026-08-20 that `cargo` was not on its PATH, and
this was first written down as "sub-agents do not reliably inherit the PATH". That
put the blame in the wrong place. Measured on 2026-09-28, `$PATH` is byte-identical
in the main bash, in `sh -c` and under `nohup sh -c`, and **none** of them has
`~/.cargo/bin` — the terminal-profile injection the old note assumed does not add
it. So `cargo: command not found` from a sub-agent and from the main agent are the
same single cause, not two. The fix is the same everywhere: write the absolute
`~/.cargo/bin/cargo`, or prepend `PATH="$HOME/.cargo/bin:$PATH"` for a script that
calls `cargo` itself. `shell-environment.md` carries the rule; this is only why the
earlier explanation was wrong.

## `inclusion: auto` and the file that lied about itself

`shell-environment.md` was `inclusion: auto` until 2026-08-12. `auto` matches a
request against the file's `description` and requires both `name` and `description`
in the front matter. Neither was present, so the file matched nothing and was never
loaded — while its own text asserted that "the non-negotiable parts live here where
they are always loaded".

A steering file cannot verify its own mode. Check the front matter against what the
mode requires.

## The portal-blocked GUI diagnosis

`cargo run -p rustconn` started from the Kiro terminal produces a process whose
`/proc/<pid>/root` the desktop portal refuses to open:

```
Gdk-WARNING: Failed to read portal settings: GDBus.Error:org.freedesktop.DBus.Error.AccessDenied:
             Portal operation not allowed: Unable to open /proc/<pid>/root
Gtk-WARNING:  Creating a portal monitor failed: <the same error>
```

Measured 2026-09-04 with the same binary in both terminals:

- **`GtkFileDialog` never completes.** The task is started and its callback is never
  invoked — not with a result, not with an error, not with `DialogError::Dismissed`.
  Nothing is logged and GTK emits no warning or critical at the click. From the
  app's side this is indistinguishable from a button with no handler attached, which
  is how it was first reported.
- **Light/dark and the icon theme resolve wrongly**, because the settings portal is
  where they come from: `dark=false` here against `dark=true` and
  `previous_theme=Yaru-purple-dark` in an external terminal.

Run the same build from an ordinary terminal and the warnings disappear, the chooser
opens, and the theme is correct.

It cost about an hour once, across three eliminated hypotheses: whether the portal
was in use at all, the contents of `~/.ssh`, and
`FileDialog::set_initial_folder`. The diagnosis only became possible after the code
stopped discarding the outcome — `show_add_key_file_chooser` matched with
`if let Ok(file) = result`, so a dismissal, a real error and a callback that never
fires all looked identical.

Hence the corollary in the always file: when a GTK callback can fail three ways, log
all three. The absence of a line is then evidence too.

## `bash-serialization-guard` false positives

The guard triggers on `cargo` followed by
`build|test|clippy|check|run|bench|doc|nextest|machete|audit`, with no notion of
whether the pair is being invoked or merely appears in the command line. A bare
`cargo` with no verb does not trigger it, and `pgrep -f cargo` is allowed.

Three classes do trigger, all verified by probe — the full list, with workarounds,
is in `hooks-map.md` under "Known false positives". The one that bites most often:
a `cargo <verb>` pair inside a search pattern or a test fixture, including a test
table in a shell loop.

## Why long cargo runs are detached by default (2026-10-05)

The always file used to offer four recipes "cheapest first", with the blocking
call first. In the release 0.23 turn (224 min, 802 credits) ten
`general-task-execution` sub-agents started cargo/verify 44 times and made ~94
calls that only checked whether a run had finished: a blocking `cargo test`
returns early, the agent then waits by re-reading or re-asking, and every one of
those reads is a paid model call. A detached run with a `.rc` sentinel costs one
call to start and one to collect, so it is now the default for anything that
does not reliably finish inside one call; blocking is kept for the short runs
(`check`/`clippy -p`, `fmt`, `test-hooks.sh`, `verify.sh --quick`).

## Details trimmed from the always file

- `typos` sits in the tool table because `AGENTS.md` lists the gate as a bare
  `typos`; unlike cargo, its absence fails nothing visibly.
- `release.sh` refusing to run without cargo on PATH is left as the caller's
  problem on purpose: a release should build with the toolchain the operator put
  there, not one a script went looking for.
- The `control_bash_process` route recovered a wedged tty on 2026-09-28 after a
  queued `nohup verify.sh` had refused to start. The queued copy can still fire
  later, which is why `bash-serialization-guard` R5 refuses a second runner.
