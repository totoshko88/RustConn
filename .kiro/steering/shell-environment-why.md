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

The detached form in recipe 2 was verified by probe on 2026-09-02: it returns
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

## Sub-agent PATH

`rust-quality-check` reported on 2026-08-20 that `cargo` was not on its PATH
despite the terminal-profile injection. Sub-agents do not reliably inherit it. The
symptom is `cargo: command not found` from a sub-agent while the same command works
in the main terminal — always this, never a broken toolchain.

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
