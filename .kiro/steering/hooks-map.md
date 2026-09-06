---
inclusion: manual
description: "Reference map of all Kiro hooks — triggers, matchers, concurrency, and side-effects."
---

# Hooks Map

Quick reference for all `.kiro/hooks/*.json` — what fires when, what it does, and what it touches.

`scripts/check-ai-docs.sh` asserts that every hook file has a row somewhere in
this document. That gate exists because this table silently lost one:
`session-baseline` (since replaced by `session-reset`) landed on 2026-08-26 and was
still undocumented on 2026-09-02, in a file whose first line promises to cover them
all. It is the same failure the same script already guards for the counts in
`docs/AI_DEVELOPMENT.md` — a hand-maintained inventory with no check against
reality.

**Reading this for cost:** the type column is the credit column. A `command` hook
runs locally and costs nothing; an `agent` hook starts a new agent loop and is
billed. On 2026-09-06 four of the six agent hooks became command hooks or moved to
a once-per-commit trigger. Only `post-task-diagnostics` remains an agent action,
and it fires on spec task completion rather than per turn or per file. The
reasoning, and the two patterns that made the conversion possible, are in
`cost-discipline.md`.

## SessionStart

| Hook | Matcher | Type | Latency | Side-effects |
|------|---------|------|---------|--------------|
| **session-reset** | (none) | command | <50ms | Truncates the agent edit journal (`target/.kiro-session-edits`) and removes any leftover session report. Replaced **session-baseline** on 2026-09-06. That hook recorded a content hash per dirty file so the Stop hook could infer what the session changed; the inference answers "did *anything* change this file?" and therefore credited the IDE's and other sessions' edits to the agent. The tree was clean at one session's start, so the baseline was legitimately empty, then 47 files went dirty from outside — and all 29 `.rs` among them were reported, five turns in a row. `edit-journal` records writes as they happen, so the hashes went away with the failure mode. Silent always; fails open. Logic: `bin/session-reset.sh`. |

## PreToolUse (before write)

| Hook | Matcher | Type | Latency | Side-effects |
|------|---------|------|---------|--------------|
| **crate-boundary-guard** | `fs_write\|fs_append\|str_replace\|delete_file\|code` | command | <50ms | Blocks with exit 2. Zero model cost when clean. Fails open. |
| **agent-model-guard** | `fs_write\|fs_append\|str_replace\|code` | command | <50ms | Blocks with exit 2 an agent profile in `.kiro/agents/` that declares no `model:`, and an edit that removes the field. The default is `auto` at 1.0x chosen per request — wrong at both ends of the range, since a cargo runner clippy re-checks belongs at 0.05x and a reviewer nothing re-checks belongs at 2.2x. All five profiles were unset until 2026-09-06 because nothing visibly breaks when the field is missing. Does **not** validate the ID: an unrecognised one falls back to the default with a warning, the same outcome as omitting it, so a fail-closed check would only block each new model Kiro adds. Fails open. Logic: `bin/agent-model-guard.sh`. |

## PostToolUse (after write)

| Hook | Matcher | Type | Latency | Side-effects |
|------|---------|------|---------|--------------|
| **edit-journal** | `fs_write\|fs_append\|str_replace\|delete_file\|code` | command | <50ms | Appends the written path, repo-relative and deduplicated, to `target/.kiro-session-edits`. Skips its own bookkeeping under `target/`. This journal is the scope for the Stop report, for `git add` at commit time (never `git add -A` in a checkout shared with the IDE), and for `commit-review-gate`. Silent; PostToolUse stdout is discarded anyway. Fails open. Logic: `bin/edit-journal.sh`. |

## PreToolUse (before shell)

All three fire on every shell call, so their cost is paid constantly and their
false positives are felt immediately. Matcher for the first two:
`^(execute_bash|executeBash|bash|shell|control_bash_process|controlBashProcess)$`;
`commit-review-gate` omits the `control_bash_process` spellings, since a commit is
not a background job.

| Hook | Type | Latency | Side-effects |
|------|------|---------|--------------|
| **bash-serialization-guard** | command | <50ms | Blocks with exit 2. Rejects `sleep`-based waiting, cargo output piped through a filter, a second cargo while one holds the target-dir lock, and a cargo build/test issued with the default 120 s timeout. Keeps a one-shot marker in `$TMPDIR` so a differently-spelled timeout field cannot deadlock it. Fails open. Logic: `bin/bash-serialization-guard.sh`. |
| **release-manual-only-guard** | command | <50ms | Blocks with exit 2. Refuses `scripts/release.sh` without `--dry-run`, refuses `--yes` either way, refuses a by-hand `git tag v<semver>`, and since 2026-09-06 refuses **any** `git push` — any remote, any ref, branch or tag. The push rule used to fire only on a version tag; widening it also forced both verbs to be anchored to command position, without which a blanket ban turned `echo "then git push it"` into a refusal. `git commit`, `git push --dry-run`, and tag *listing* and *deletion* stay allowed. Fails open. Logic: `bin/release-manual-only-guard.sh`. |
| **commit-review-gate** | command | <50ms | Returns `permissionDecision: "ask"` at `git commit` when the edit journal contains paths with a dedicated review: a `rustconn-*-sys` change (`unsafe-reviewer`), credential code (`security-reviewer`), or `po/uk.po` (`uk-translation-reviewer`). Replaced three `PostFileSave` agent hooks on 2026-09-06 — one agent loop per saved file, each reviewing a file in isolation, plus one on every `msgmerge` that rewrote `uk.po` without changing a translation. Asks rather than blocks, because it cannot observe whether a reviewer already ran; `--dry-run` and a mere mention of the words pass. Fails open. Logic: `bin/commit-review-gate.sh`. |

### Known false positives in `bash-serialization-guard`

The guard triggers on `cargo` followed by one of
`build|test|clippy|check|run|bench|doc|nextest|machete|audit`, with no notion of
whether that pair is being *invoked* or merely appears in the command line. It
does **not** trigger on a bare `cargo` with no verb after it.

This paragraph said "matches the literal string `cargo` anywhere in the command"
until 2026-09-02, and listed `pgrep -f cargo` as the first false positive. Both
were wrong, and had been since the script grew its verb list: the claim was never
re-tested against the script, so it outlived the behaviour it described. Probed
directly by piping payloads to the guard on 2026-09-02 — `pgrep -f cargo` is
**allowed**, and `pgrep -x cargo` (which R3 itself uses) is allowed too. There is
no reason to write `pgrep -f '[c]argo'` for the guard's sake. The bracket idiom
still has its older, unrelated merit of stopping `pgrep` from matching its own
command line.

Three classes do still trigger, all verified by probe:

1. **A `cargo <verb>` pair inside a search pattern.** `grep -rn 'cargo build' …`
   is blocked even though nothing is built. Prefer the `grepSearch` tool over
   shell `grep` here — it is the right tool anyway and sidesteps the guard
   entirely.
2. **A `nohup`-detached run.** It returns immediately and therefore cannot lose
   its output, but the guard cannot tell. Passing `timeout=900000` satisfies it
   and costs nothing, since the call returns either way. The guard's own R1
   message shows this form *with* the timeout, so following the message works.
3. **A payload under test.** Feeding the guard a JSON payload to probe it puts
   the offending string on the outer command line too, so testing `sleep 115`
   trips R1 on the test call itself. Write the probe to a file and run the file.

None of these is worth "fixing" in the script by parsing the command line — a
guard that fails open and occasionally over-triggers is the right trade against
one that tries to be clever and misses a real case. Know the three workarounds
instead.

The four block messages name log paths under `target/`, matching
`shell-environment.md`. They said `/tmp` until 2026-09-02, contradicting the
always-loaded rule at the exact moment an agent was most likely to follow them.

Unrelated shell trap in the same territory: `echo '#![allow]'` in double quotes
trips bash history expansion (`bash: ![allow]: event not found`). Single-quote
anything containing `!`.

## PostFileSave (after user or agent saves)

| Hook | Matcher | Type | Latency | Side-effects |
|------|---------|------|---------|--------------|
| **translation-sync** | `rustconn/src/.*\.rs$` | command | <100ms | Silent unless a `POTFILES.in` line must be added |
| **cargo-security-scan** | `Cargo\.lock$` | command | ~5s | Read-only advisory check, findings to `target/cargo-advisories.log`. Skips silently when `Cargo.lock` matches HEAD. Logic: `bin/cargo-advisory-scan.sh`. Prefers the **bare** `cargo-deny` binary over `cargo deny`, so `rust-toolchain.toml` is not asked to resolve a toolchain for a check that only parses the lockfile — the same reason `ci.yml` invokes `cargo-machete` directly. Presence is probed with `command -v`, never inferred from an exit code: cargo-deny exits non-zero *because* it found an advisory, and the old inline `\|\|` chain therefore reported real findings as "neither tool installed" while `2>/dev/null` discarded the report. Fixed 2026-09-02. |
| **flatpak-manifest-check** | `Cargo\.lock$` | command | <50ms | Notes that `packaging/*/cargo-sources.json` are older than `Cargo.lock`, so a Flatpak build would vendor the previous dependency set. Never regenerates — that is a deliberate pre-release act on large generated files. Was an `agent` action until 2026-09-06: a full agent loop per `Cargo.lock` save to run `test -f` twice and print a fixed warning. Now a timestamp comparison, delivered through `target/.kiro-session-report`. Fails open. Logic: `bin/flatpak-manifest-check.sh`. |
| **kirograph-mark-dirty-on-save** | `\.(rs\|toml)$` | command | <100ms | Writes `.kirograph/dirty`; logs to `.kirograph/hook.log` |

## PostFileCreate

| Hook | Matcher | Type | Latency | Side-effects |
|------|---------|------|---------|--------------|
| **kirograph-mark-dirty-on-create** | `\.(rs\|toml)$` | command | <100ms | Writes `.kirograph/dirty`; logs to `.kirograph/hook.log` |

## PostFileDelete

| Hook | Matcher | Type | Latency | Side-effects |
|------|---------|------|---------|--------------|
| **kirograph-sync-on-delete** | `\.(rs\|toml)$` | command | <100ms | Marks dirty only — sync is deferred to the Stop hook |

## PostTaskExec (after spec task completes)

| Hook | Matcher | Type | Latency | Side-effects |
|------|---------|------|---------|--------------|
| **post-task-diagnostics** | (none) | agent | ~5s | Runs getDiagnostics on changed .rs files. No cargo commands. |

## Stop (end of agent session)

| Hook | Matcher | Type | Latency | Side-effects |
|------|---------|------|---------|--------------|
| **session-report** | (none) | command | <100ms | Scans the `.rs` files in the edit journal for debug leftovers on lines this session added, and appends a paragraph to `target/.kiro-session-report`. Silent when clean. Replaced **post-session-diagnostics** on 2026-09-06, an `agent` action that spent a full agent loop on every turn — five in one session, each reporting files the agent had never touched, each ending in a `getDiagnostics` call that does not exist outside the IDE. Compile diagnostics moved to the commit gate, where clippy runs once per feature. Fails open. Logic: `bin/session-report.sh write`. |
| **kirograph-sync-if-dirty** | (none) | command | **~3-4 min**, up to ~20 min | Syncs KiroGraph index if dirty marker present. Runs `nice`d in the background; skipped if a sync is already running |

## UserPromptSubmit

| Hook | Matcher | Type | Latency | Side-effects |
|------|---------|------|---------|--------------|
| **session-report-flush** | (none) | command | <50ms | Prints `target/.kiro-session-report` to the agent, then deletes it. This is the delivery half of the free-report pattern: a command hook's stdout is forwarded only on `SessionStart`, `UserPromptSubmit` and `PreToolUse`, so the scan runs for nothing at `Stop` and its finding rides into a turn the user was starting anyway. One turn late, zero credits. Prints nothing when there is no report. Producers (`session-report`, `flatpak-manifest-check`) append; this is the sole consumer. Fails open. Logic: `bin/session-report.sh flush`. |

---

## KiroGraph sync cost and failure mode

Measured on this repo (564 files, ~36k symbols): `kirograph sync` takes **191 s even when
it reports "Nothing to sync"** — it always rescans every file and resolves ~47k symbols.
After a branch with ~130 changed `.rs` files it took **~21 min**. So the Stop hook keeps a
CPU-bound background process alive well past the end of a turn.

While that process holds `.kirograph/kirograph.db.lock`, every graph MCP call answers
**"KiroGraph not initialized. Run `kirograph init`"** — which is misleading. If such a sync
is killed (session closed, `timeout`), the empty lock directory survives and *every*
subsequent call keeps reporting "not initialized". That silently disabled KiroGraph in this
repo for a week in July 2026. Note that `kirograph unlock` does not help: it looks for a
lock *file*, while what is left behind is a *directory*.

Hardening in the four KiroGraph hooks:

- `cd "$(git rev-parse --show-toplevel …)"` first — hook cwd is not guaranteed, and a bare
  `kirograph` in a subdirectory reports "not initialized at <subdir>".
- stdout/stderr go to `.kirograph/hook.log` (gitignored) instead of `2>/dev/null`; the very
  first log line already surfaced two silently skipped `Cargo.toml` dependencies.
- the Stop hook exits early if `pgrep -f 'kirograph [s]ync'` matches, so two syncs never
  race for the lock.
- an *empty* `kirograph.db.lock` directory older than 30 min is deleted before syncing.

## Concurrency notes

When editing `rustconn/src/dialogs/password.rs`:

1. Before the write: `crate-boundary-guard`, then `agent-model-guard` (which exits
   immediately — not an agent profile). Both command, both <50ms.
2. After the write: `edit-journal` records the path.
3. After save, **two** PostFileSave hooks fire simultaneously, both command:
   - `kirograph-mark-dirty-on-save` (~instant)
   - `translation-sync` (<100ms — checks for i18n calls)
4. At `git commit`: `commit-review-gate` asks for `security-reviewer`, because the
   journal contains a `password` filename.

Every step is now free except step 4, and step 4 happens once per commit. Until
2026-09-06, step 3 also fired `security-review` — an agent action, ~10s and a full
loop, on **every save of every matching file**.

This example named `rustconn/src/secret/` until 2026-09-02. No such directory
exists, and the old `security-review` hook's `secret/` branch was anchored to
`rustconn-core/src/` anyway. The path-matching lives in `commit-review-gate` now
and uses the same anchoring.

When editing `Cargo.lock`:
- `cargo-security-scan` + `flatpak-manifest-check` fire together, both command,
  both silent unless they find something

## Notes

- KiroGraph matchers are scoped to `\.(rs|toml)$` — matching the Rust-only project.
- Only one hook now runs `kirograph sync`: the Stop hook. The save/create/delete hooks just
  set the dirty marker.
- Permissions, MCP config and other files under `.kiro/settings/` cannot be edited by the
  agent (`kiro-scope` deny). Reviewed copies live in `.kiro/config-templates/` and are
  applied by hand.
- **Pending hand-apply (found 2026-09-02):** `.kiro/config-templates/mcp.json` passes
  `--path /home/totoshko88/Documents/RustConn` to `kirograph serve`; the live
  `.kiro/settings/mcp.json` does not. Without it the server resolves the project root
  from its working directory, which is one of the documented causes of the bogus
  "KiroGraph not initialized" in `kirograph.md`. The template is the corrected copy —
  copy it over by hand, since the deny rule means no agent can.
