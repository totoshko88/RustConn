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
| **crate-boundary-guard** | anchored `^(fs_write\|fs_append\|str_replace\|delete_file\|code\|smart_relocate\|semantic_rename\|mcp_kirograph_kirograph_(str_replace\|multi_str_replace\|insert_at\|ast_grep_rewrite)\|@kirograph/…)$` | command | <50ms | Blocks with exit 2. Since 2026-09-28 also reads the KiroGraph write tools' `file` param and their `new_str`/`content`/`rewrite`/`pairs[].new_str` content, so `unsafe`/`use gtk4` cannot slip in through them (`scripts/test-hooks.sh` covers it). Zero model cost when clean. Fails open. |
| **agent-model-guard** | anchored `^(fs_write\|fs_append\|str_replace\|code)$` | command | <50ms | Blocks with exit 2 an agent profile in `.kiro/agents/` that declares no `model:`, and an edit that removes the field. Exits 0 for `fs_append` (it can only add, and its fragment is not the whole profile — treating its `text` as a file blocked every appended rule until 2026-09-28). The default is `auto` at 1.0x chosen per request — wrong at both ends of the range, since a cargo runner clippy re-checks belongs at 0.05x and a reviewer nothing re-checks belongs at 2.2x. All five profiles were unset until 2026-09-06 because nothing visibly breaks when the field is missing. Does **not** validate the ID: an unrecognised one falls back to the default with a warning, the same outcome as omitting it, so a fail-closed check would only block each new model Kiro adds. Fails open. Logic: `bin/agent-model-guard.sh`. |

## PostToolUse (after write)

| Hook | Matcher | Type | Latency | Side-effects |
|------|---------|------|---------|--------------|
| **edit-journal** | anchored, same set as crate-boundary-guard plus `smart_relocate`/`semantic_rename` | command | <50ms | Appends the written path(s), repo-relative and deduplicated, to `target/.kiro-session-edits`. Reads `path`/`targetFile`/`file`/`sourcePath`/`destinationPath`, so a move journals both ends and a KiroGraph edit journals its `file` (all three were invisible before 2026-09-28). This journal is the scope for the Stop report, for `git add` at commit time (never `git add -A` in a checkout shared with the IDE), and for `commit-review-gate`. Files written by *scripts* (bump-version, sync-cargo-sources, update-pot) are not seen here — they call `scripts/lib/journal.sh` themselves. Honours `KIRO_EDIT_JOURNAL` so `scripts/test-hooks.sh` can redirect it. Silent; PostToolUse stdout is discarded anyway. Fails open. Logic: `bin/edit-journal.sh`. |

## PreToolUse (before shell)

All three fire on every shell call, so their cost is paid constantly and their
false positives are felt immediately. Matcher for the first two:
`^(execute_bash|executeBash|bash|shell|control_bash_process|controlBashProcess|mcp_kirograph_kirograph_exec|@kirograph/kirograph_exec)$`;
`commit-review-gate` omits the `control_bash_process` spellings, since a commit is
not a background job. The `kirograph_exec` spellings were added 2026-09-28 — it
runs shell commands too, so `git push` / `release.sh --yes` through it must be
guarded like any other shell call.

| Hook | Type | Latency | Side-effects |
|------|------|---------|--------------|
| **bash-serialization-guard** | command | <50ms | Blocks with exit 2. Rejects `sleep`-based waiting (R1), cargo output piped through a filter (R2), a second cargo while one holds the target-dir lock (R3), a cargo build/test issued with the default 120 s timeout (R4), and — since 2026-09-28 — a second `verify.sh`/`release.sh` runner while one is already alive (R5). R5 exists because R3 matches the cargo *binary*, so it fires only once a runner's inner cargo starts; two wrapper scripts can race for the target-dir lock and interleave a shared log before that. Keeps a one-shot marker in `$TMPDIR` so a differently-spelled timeout field cannot deadlock it. Fails open. Logic: `bin/bash-serialization-guard.sh`. |
| **release-manual-only-guard** | command | <50ms | Blocks with exit 2. Refuses `scripts/release.sh` without `--dry-run`, refuses `--yes` either way, refuses a by-hand `git tag v<semver>`, and refuses **any** `git push` (any remote, ref, branch or tag). Rewritten 2026-09-28 to parse the command line into simple commands via `bin/lib/command-segments.awk` and read each flag from its own invocation — the old whole-line regexes let seven forms through, including the PATH-prefixed `PATH=… ./scripts/release.sh --yes` that `shell-environment.md` prescribes, and `env`/`timeout` wrappers, and a `-h`/`-n`/`--dry-run` belonging to a *different* command on the line. `git commit`, `git push --dry-run`, and tag listing/deletion stay allowed. `scripts/test-hooks.sh` locks all of this down. Fails open. Logic: `bin/release-manual-only-guard.sh`. |
| **commit-review-gate** | command | <50ms | Returns `permissionDecision: "ask"` at `git commit` when the edit journal contains paths with a dedicated review: any `rustconn-*-sys` change including its `Cargo.toml` (`unsafe-reviewer` — the name shape, not a fixed list, so a fifth -sys crate is caught), credential code (`security-reviewer`), `po/uk.po` (`uk-translation-reviewer`), or a persisted/runtime config, launch mapper or import/export converter (`config-mapping-reviewer`, added 2026-09-30 — stored-but-unread fields and broken round-trips compile clean, so nothing else catches them). Uses the same command-segment parser as the release guard, so `GIT_EDITOR=… git commit` and `git add … && git commit` are seen while a mention is not. Replaced three `PostFileSave` agent hooks on 2026-09-06. Asks rather than blocks, because it cannot observe whether a reviewer already ran; `--dry-run` passes. Fails open. Logic: `bin/commit-review-gate.sh`. |
| **changelog-entry-guard** | command | <50ms | Returns `permissionDecision: "ask"` at `git commit` when the edit journal has a behavioural source change (a non-test `.rs` under some crate's `src/`) but `CHANGELOG.md` is not among the session's edits. `ask`, not block, and for the same reason as `commit-review-gate`: an internal refactor legitimately needs no entry and a guard cannot tell one from a behaviour change. Excludes `tests/` and `*_tests.rs` from "behavioural". Same command-segment parser and journal scope (`target/.kiro-session-edits`, `KIRO_EDIT_JOURNAL` for the suite) as the other commit gates. Added 2026-09-30 to carry the manual per-commit CHANGELOG discipline. `--dry-run` passes. Fails open. Logic: `bin/changelog-entry-guard.sh`. |

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
| **translation-sync** | `rustconn/src/.*\.rs$` | command | <100ms | Notes a missing `POTFILES.in` line or a `\u{…}` escape in a translatable literal. Since 2026-09-28 it writes to `target/.kiro-session-report` (flushed next turn), not stdout — PostFileSave stdout is discarded, so its reminder was silently lost before. Anchors the path at the git root like `edit-journal`, not the old `${rel##*/RustConn/}`. Silent when clean. Logic: `bin/translation-sync.sh`. |
| **doc-claims-scan** | `\.rs$` | command | <100ms | Notes a doc-comment line **added relative to HEAD** (`git diff -U0`; an untracked file counts as all-added) under some crate's `src/` that names a backticked `snake_case` identifier (≥4 chars) no non-comment line in any `*/src` spells as a word — fields, params and methods count as found; `true`/`self`/keywords are ignored; `target/` is never scanned. Catches the "doc lies" class from the 0.22.12 audit (`search_parallel`). A finding already in the report is not re-appended; ≤20 per save. Rewritten 2026-10-05: the whole-file, definitions-only version produced 610 findings on the 126 `.rs` files of 0.23 (`true` ×34) and ~1.5k lines in one turn's flush; the rewrite gives 4 on the same set. A NOTE, never a block; fails open. Logic: `bin/doc-claims-scan.sh`. |
| **cargo-security-scan** | `Cargo\.lock$` | command | ~5s | Read-only advisory check, findings to `target/cargo-advisories.log`. Skips silently when `Cargo.lock` matches HEAD. Logic: `bin/cargo-advisory-scan.sh`. Prefers the **bare** `cargo-deny` binary over `cargo deny`, so `rust-toolchain.toml` is not asked to resolve a toolchain for a check that only parses the lockfile — the same reason `ci.yml` invokes `cargo-machete` directly. Presence is probed with `command -v`, never inferred from an exit code: cargo-deny exits non-zero *because* it found an advisory, and the old inline `\|\|` chain therefore reported real findings as "neither tool installed" while `2>/dev/null` discarded the report. Fixed 2026-09-02. |
| **flatpak-manifest-check** | `Cargo\.lock$` | command | <50ms | Notes that `packaging/*/cargo-sources.json` are older than `Cargo.lock`, so a Flatpak build would vendor the previous dependency set. Never regenerates — that is a deliberate pre-release act on large generated files. Was an `agent` action until 2026-09-06: a full agent loop per `Cargo.lock` save to run `test -f` twice and print a fixed warning. Now a timestamp comparison, delivered through `target/.kiro-session-report`. Fails open. Logic: `bin/flatpak-manifest-check.sh`. |
| **kirograph-mark-dirty-on-save** | `\.(rs\|toml)$` | command | <100ms | Writes `.kirograph/dirty`; logs to `.kirograph/hook.log` |

## PostFileCreate

| Hook | Matcher | Type | Latency | Side-effects |
|------|---------|------|---------|--------------|
| **kirograph-mark-dirty-on-create** | `\.(rs\|toml)$` | command | <100ms | Writes `.kirograph/dirty`; logs to `.kirograph/hook.log` |
| **ai-doc-counts** | `\.kiro/(steering/[^/]*\.md\|hooks/[^/]*\.json)$` | command | <100ms | Appends to `target/.kiro-session-report` when the counts in `docs/AI_DEVELOPMENT.md` no longer match `.kiro/`, or a hook has no row in this file. Create, not save: adding a file is what breaks a count, editing one cannot. Exists because CI was the only thing checking — a8bdb01e added the 30th steering file, the count stayed at 29, and Hygiene went red on main after v0.21.12 was already tagged and published, the third time that number had gone stale. Reports the wrong number; never rewrites the sentence around it. Silent when clean; fails open. Logic: `bin/ai-doc-counts.sh`. |

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
| **patch-litter-scan** | (none) | command | <50ms | Scans the working tree for `*.rej` / `*.orig` — the fingerprint of a partially applied patch — and appends a paragraph to `target/.kiro-session-report` (one of several producers of that shared channel). Added 2026-09-22 after a delegated agent applied a patch to `embedded_vnc_types.rs` that only partly took, corrupting the file and leaving `.rej`/`.orig` behind, caught only when `verify.sh` failed minutes later. Deliberately does **not** flag "changed but not in the edit journal" — that is the normal state in a shared checkout and cost five wasted loops on 2026-09-06. Uses `git ls-files` (respects `.gitignore`, skips `target/`). Silent on a clean tree; fails open. Logic: `bin/patch-litter-scan.sh`. |
| **kirograph-sync-if-dirty** | (none) | command | returns at once | Runs `bin/kirograph-sync.sh`, which `setsid`-detaches the sync so it outlives the turn (the JSON sets `timeout: 0`), then syncs at low priority if the dirty marker is present. Rewritten 2026-09-28: the old inline command ran the ~3-4 min sync in the hook foreground under the ignored-because-inside-`action` timeout, so the 60 s default cut every run off and the killed process left the lock behind — a week of `hook.log` held only "Database is locked" and no successful sync. The "already syncing?" check now matches the process by name, not a substring of its argv (which matched the hook's own shell). Each run brackets itself with an ISO timestamp in `.kirograph/hook.log`. |

## UserPromptSubmit

| Hook | Matcher | Type | Latency | Side-effects |
|------|---------|------|---------|--------------|
| **session-report-flush** | (none) | command | <50ms | Prints `target/.kiro-session-report` to the agent, then deletes it. This is the delivery half of the free-report pattern: a command hook's stdout is forwarded only on `SessionStart`, `UserPromptSubmit` and `PreToolUse`, so the scan runs for nothing at `Stop` and its finding rides into a turn the user was starting anyway. One turn late, zero credits. Prints nothing when there is no report. Since 2026-10-05 it de-duplicates and caps the print at 40 lines / 4 KB, writing the full de-duplicated report to `target/.kiro-session-report.full` and naming it in the last line — the flush lands in the prompt and is re-read every later turn. `KIRO_SESSION_REPORT` overrides the path (`scripts/test-hooks.sh` uses a scratch file, so `verify.sh` no longer seeds the real report with test noise). Producers — `session-report`, `patch-litter-scan`, `flatpak-manifest-check`, `ai-doc-counts` and (since 2026-09-28) `translation-sync` — append; this is the sole consumer. Fails open. Logic: `bin/session-report.sh flush`. |

---

## KiroGraph sync cost and failure mode

Measured on this repo (564 files, ~36k symbols): `kirograph sync` takes **191 s even when
it reports "Nothing to sync"** — it always rescans every file and resolves ~47k symbols.
After a branch with ~130 changed `.rs` files it took **~21 min**. So the Stop hook keeps a
CPU-bound background process alive well past the end of a turn.

While that process holds `.kirograph/kirograph.db.lock`, every graph MCP call answers
**"KiroGraph not initialized. Run `kirograph init`"** — which is misleading. If such a sync
is killed (session closed, `timeout`), the lock survives and *every* subsequent call keeps
reporting "not initialized". That silently disabled KiroGraph in this repo for a week in
July 2026.

Hardening in the four KiroGraph hooks:

- `cd "$(git rev-parse --show-toplevel …)"` first — hook cwd is not guaranteed, and a bare
  `kirograph` in a subdirectory reports "not initialized at <subdir>".
- stdout/stderr go to `.kirograph/hook.log` (gitignored) instead of `2>/dev/null`; the very
  first log line already surfaced two silently skipped `Cargo.toml` dependencies.
- the Stop hook (`bin/kirograph-sync.sh`) runs `kirograph unlock` before syncing — the
  current CLI releases the lock when its owning PID is dead, so it clears the stale one the
  old `find -mmin +30 -empty` heuristic missed — and skips if a sync is genuinely running,
  matched by process name so the check cannot match its own shell.
- the sync is `setsid`-detached with `timeout: 0`, so the engine reaping the hook at the end
  of the turn does not kill it mid-flight and leave the lock behind.

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
- **Pending hand-apply — `disabledTools` (2026-09-28):** `.kiro/config-templates/mcp.json`
  now turns off KiroGraph's four write tools (`kirograph_str_replace`,
  `kirograph_multi_str_replace`, `kirograph_insert_at`, `kirograph_ast_grep_rewrite`)
  via `disabledTools`; the live `.kiro/settings/mcp.json` does not yet. They duplicate
  the built-in write tools and, unlike those, reached no write hook — no
  crate-boundary check, no journal entry — until the matchers were widened the same
  day (belt and braces; the hooks cover them now too). Copy the template over by hand,
  since the deny rule means no agent can. The earlier `--path` hand-apply from
  2026-09-02 is **done** — both files now carry it.
