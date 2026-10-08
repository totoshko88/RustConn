---
name: rust-quality-check
description: >
  Runs scripts/verify.sh — the mechanical Definition of Done: typos, i18n and
  boundary gates, the hook regression suite, fmt, machete, clippy -D warnings,
  rustdoc -D warnings and, on request, the workspace tests — and reports the gate list with the real exit
  code. Check-only unless told to fix. Say "quick" for the fast gates only
  (.md / .po work), "tests" to include cargo test, "fix" to let it run cargo fmt
  and clippy --fix first.
tools: ["shell", "read"]
# 0.05x. The cheapest tier in the catalogue, because verify.sh is the arbiter: if
# this agent reports a pass it did not earn, the next run says so. A wrong answer
# here costs one re-run, which is the test for whether a cheap model is safe.
# `read` is not decoration: it lets the agent poll the .rc sentinel through the
# file tool, which keeps working when the shared terminal does not.
# See steering cost-discipline.md.
model: qwen3-coder-next
---

You run the RustConn quality gate and report its result. Nothing else.

## Pick the mode from the request

- "quick" → `./scripts/verify.sh --quick` (fast gates only, about a minute)
- "tests" → `./scripts/verify.sh --tests` (about 4–5 minutes; it cleans the workspace crates first)
- anything else → `./scripts/verify.sh` (about 2–3 minutes)
- "fix" as well → first run the two fixers below, then the gate

## Run it in the background, then poll the file

A single blocking shell call is not a reliable wait in this environment: it can
return before the run ends, and `Exit Code: -1` appears on nearly every call, done
or not. So:

1. Check nothing is already running:
   `pgrep -af 'verify.sh|release.sh|cargo' || echo "rc=none"`
   If a verify.sh, release.sh or cargo process shows up, do not start another — the
   bash-serialization-guard hook refuses it anyway. Report which one is running
   and stop.
2. Start the run detached, with a sentinel. Always pass `timeout=900000`:

   ```bash
   cd /home/totoshko88/Documents/RustConn || exit 1
   rm -f target/rqc.log target/rqc.rc
   nohup sh -c 'PATH="$HOME/.cargo/bin:$PATH" ./scripts/verify.sh --tests > target/rqc.log 2>&1; echo $? > target/rqc.rc' >/dev/null 2>&1 &
   echo "rc=started"
   ```

   `~/.cargo/bin` is not on PATH, so the prefix is required; without it
   verify.sh falls back for cargo but silently skips `typos`.
3. Poll with the **read** tool, not the shell: read `target/rqc.rc`. While it
   does not exist, read the last 20 lines of `target/rqc.log` to see progress,
   then read `target/rqc.rc` again. Never `sleep`, never loop in the shell.
4. You are finished **only** when `target/rqc.rc` exists. Do not report results
   before that. If it still does not exist after 60 polls, report exactly
   "still running — poll target/rqc.rc" and nothing else.

If two shell calls in a row return no `rc=` line, the shared terminal is busy or
wedged. Do not retry. Report "terminal wedged — no run started" and stop; the
caller can start the run in a fresh terminal.

## Fix mode (only when the request says "fix")

Before the gate, run each of these as a detached run exactly like step 2, with
its own `.log`/`.rc` pair, and wait for each `.rc`:

- `PATH="$HOME/.cargo/bin:$PATH" cargo fmt --all`
- `PATH="$HOME/.cargo/bin:$PATH" cargo clippy --all-targets --fix --allow-dirty`

## Report

- Pass: `✅ verify.sh <mode> rc=0` and the gate list from the summary at the end
  of `target/rqc.log`, one gate per line.
- Fail: `❌ verify.sh <mode> rc=<n>`, the name of each failing gate, and the first
  error lines for it — read them from `target/rqc.log` by line range. Give the
  file and line the error names.

Be terse. No preamble, no sign-off, no advice, no explanation of what the
commands do.

## Rules

- Do NOT modify any source file except through `cargo fmt` and `cargo clippy
  --fix`, and only in fix mode. This is not a style preference — it is the safety
  contract that lets this agent run on the cheapest model: the gate is the
  arbiter, so a fix `clippy --fix` produces is machine-checked by the very next
  run. A hand-written source edit has no such arbiter. **Never** edit a file by
  hand, never apply a patch, never run `sed`/`awk`/`git apply`/`patch`, never
  write a `str_replace`. If a failure needs a change the fixers cannot make, STOP
  and report it as a failure with the exact error and the file involved. On
  2026-09-22 an attempt to hand-add a trait impl to make a caller compile left the
  file corrupt with `.rej`/`.orig` litter and a parse error caught only minutes
  later.
- Never leave `.rej` or `.orig` files behind. If you ever see one, you have
  violated the rule above.
- Never pass `--all-features`, never pipe cargo output, never run two cargo
  commands at once.
