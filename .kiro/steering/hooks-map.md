---
inclusion: manual
description: "Reference map of all Kiro hooks — triggers, matchers, concurrency, and side-effects."
---

# Hooks Map

Quick reference for all `.kiro/hooks/*.json` — what fires when, what it does, and what it touches.

## PreToolUse (before write)

| Hook | Matcher | Type | Latency | Side-effects |
|------|---------|------|---------|--------------|
| **crate-boundary-guard** | `fs_write\|fs_append\|str_replace\|delete_file\|code` | agent | <1s (prompt reflection) | Can DENY the write. Silent if non-.rs or clean. |

## PostFileSave (after user or agent saves)

| Hook | Matcher | Type | Latency | Side-effects |
|------|---------|------|---------|--------------|
| **translation-sync** | `rustconn/src/.*\.rs$` | agent | ~2s | May edit `po/POTFILES.in` |
| **security-review** | `secret/.*\.rs$\|credential.*\.rs$\|password.*\.rs$` | agent | ~10s | Invokes `security-reviewer` sub-agent (read-only audit) |
| **uk-translation-review** | `po/uk\.po$` | agent | ~15s | Invokes `uk-translation-reviewer` sub-agent, may edit `po/uk.po` |
| **cargo-security-scan** | `Cargo\.lock$` | command | ~5s | Runs `cargo deny`/`cargo audit` (read-only) |
| **flatpak-manifest-check** | `Cargo\.lock$` | agent | ~2s | Warns about stale cargo-sources.json (no auto-fix) |
| **sync-package-versions** | `Cargo\.toml$` | agent | ~5s | **LOGICALLY DISABLED** (description says superseded, but `enabled: true` — see note below) |
| **kirograph-mark-dirty-on-save** | `\.(ts\|tsx\|js\|...\|rs\|...)$` | command | <100ms | Touches dirty marker file |

## PostFileCreate

| Hook | Matcher | Type | Latency | Side-effects |
|------|---------|------|---------|--------------|
| **kirograph-mark-dirty-on-create** | `\.(ts\|tsx\|js\|...\|rs\|...)$` | command | <100ms | Touches dirty marker file |

## PostFileDelete

| Hook | Matcher | Type | Latency | Side-effects |
|------|---------|------|---------|--------------|
| **kirograph-sync-on-delete** | `\.(ts\|tsx\|js\|...\|rs\|...)$` | command | ~1s | Runs full sync (immediate, not deferred) |

## PostTaskExec (after spec task completes)

| Hook | Matcher | Type | Latency | Side-effects |
|------|---------|------|---------|--------------|
| **post-task-diagnostics** | (none) | agent | ~5s | Runs getDiagnostics on changed .rs files. No cargo commands. |

## Stop (end of agent session)

| Hook | Matcher | Type | Latency | Side-effects |
|------|---------|------|---------|--------------|
| **post-session-diagnostics** | (none) | agent | ~10s | getDiagnostics + scans diff for debug leftovers |
| **kirograph-sync-if-dirty** | (none) | command | ~3s | Syncs KiroGraph index if dirty marker present |

---

## Concurrency notes

When editing a `.rs` file in `rustconn/src/secret/`:

1. `crate-boundary-guard` fires **before** write (PreToolUse)
2. After save, **three** PostFileSave hooks fire simultaneously:
   - `kirograph-mark-dirty-on-save` (~instant, command)
   - `translation-sync` (~2s, agent — checks for i18n calls)
   - `security-review` (~10s, agent — invokes sub-agent)

When editing `Cargo.lock`:
- `cargo-security-scan` + `flatpak-manifest-check` fire together

When editing `Cargo.toml`:
- `sync-package-versions` fires (but should be disabled — see cleanup note)

## Known issues

- **sync-package-versions**: description says "DISABLED — superseded" but `"enabled": true`. Should be set to `"enabled": false` or deleted.
- **KiroGraph matchers** include TypeScript/Python/Java/etc. for a Rust-only project. Can be narrowed to `\\.rs$` and `\\.toml$`.
