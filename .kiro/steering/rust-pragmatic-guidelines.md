---
inclusion: fileMatch
fileMatchPattern: "**/*.rs"
---

# Pragmatic Rust Guidelines (Microsoft) — RustConn Adaptation

Adaptation of [Microsoft Pragmatic Rust Guidelines](https://microsoft.github.io/rust-guidelines/) for RustConn.
Supplements `project-rules.md`, does not replace it. Only lists points missing from other steering files.

## Universal

### M-LINT-OVERRIDE-EXPECT — `#[expect]` instead of `#[allow]`

When locally overriding a clippy/compiler lint — use `#[expect(..., reason = "...")]`.
`#[expect]` emits a warning if the lint did not fire, preventing accumulation of stale overrides.

```rust
#[expect(clippy::unused_async, reason = "API stable, I/O will be added later")]
pub async fn ping_server() { }
```

`#[allow]` remains appropriate only in macros and generated code.

### M-PANIC-IS-STOP / M-PANIC-ON-BUG — panic = "the program must stop"

Panic is not an exception. `panic!()` means "stop the program now". Do not use panic for:
- communicating errors upward (that is what `Result` does),
- handling controlled conditions (timeout, unreachable host, wrong password),
- assuming the panic will be caught (if `panic = "abort"` — the program crashes).

Valid cases: `expect("must never happen")` for programming bugs, `unwrap()` on `OnceLock::get_or_init`, panic on poisoned lock.

Programming bug → `panic!` / `unreachable!` / `debug_assert!`. Recoverable state → `Result<T, ThisError>`. Do not mix.

### M-DOCUMENTED-MAGIC — document magic values

Any magic constant or default behavior must have a comment.
Especially relevant for timeouts, retry backoffs, buffer limits.

```rust
// Vault operations wait 10 seconds — Bitwarden CLI may trigger a master-pw prompt.
const VAULT_OP_TIMEOUT: Duration = Duration::from_secs(10);
```

### M-LOG-STRUCTURED — structured logging

We already use `tracing`. Additionally:
- pass data as fields, not as a formatted string: `tracing::info!(host = %h, port = p, "connecting")` instead of `tracing::info!("connecting to {}:{}", h, p)`,
- never log `SecretString` (`expose_secret()` in `tracing::*` — forbidden).

## Applications (rustconn / rustconn-cli)

### M-MIMALLOC-APP — global allocator

[Not critical, optional]. Apps can gain ~10–25% speedup on hot paths by replacing the allocator with `mimalloc`. If profiling shows allocation is a bottleneck, add:

```toml
[dependencies]
mimalloc = "0.1"
```

```rust
// rustconn/src/main.rs
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
```

### M-APP-ERROR — `anyhow` is allowed in `rustconn` / `rustconn-cli`

Binary crates may use `anyhow` / `eyre` to reduce boilerplate.
Library functions in `rustconn-core` MUST still use `thiserror::Error`
(M-ERRORS-CANONICAL-STRUCTS — so that callers from GUI/CLI can pattern-match variants).

## Safety

### M-UNSAFE — `unsafe` is confined to the `-sys` crates

Workspace `[lints.rust] unsafe_code = "deny"`, re-opened only by a crate-level
`#![expect(unsafe_code, reason = "…")]` in the four sanctioned FFI crates:
`rustconn-pty-sys` (macOS PTY controlling terminal), `rustconn-locale-sys` (the
startup `setlocale` call), `rustconn-env-sys` (the startup `GSK_RENDERER` and
`LANGUAGE` writes) and `rustconn-dock-sys` (the macOS Dock tile image via
`-[NSApplication setApplicationIconImage:]`; its `expect` is wrapped in
`cfg_attr(target_os = "macos", …)`, since off macOS the crate contains no
`unsafe` and a bare `expect` would fire `unfulfilled_lint_expectations`).
`deny` rather than `forbid` because `forbid` cannot be
overridden at any level. Under `forbid` each helper had to declare its own
`[lints]` table, and a crate-local `[lints]` table *replaces* the inherited one
rather than adding to it — so the only crates allowed to write `unsafe` became
the only crates in the workspace running without `clippy::pedantic`,
`clippy::nursery` or `clippy::unwrap_used`.

**That is history, not the current state.** All four helpers carry
`[lints] workspace = true` and do inherit the full table; verify with
`grep -A1 '^\[lints\]' rustconn-*-sys/Cargo.toml` rather than trusting this
paragraph. The reason it is written down at all is that the trap is easy to walk
back into: do not "tighten" `deny` to `forbid` without also solving the
replaced-table problem, and do not give a helper its own `[lints]` table.
If further FFI is ever needed — create another small `rustconn-*-sys` crate with
a documented `// SAFETY:` contract on every `unsafe` block, rather than relaxing
the lint where the caller lives. Miri cannot execute the syscalls/FFI used here
(`pre_exec`, `ioctl`, `setlocale`, `setenv`), so prefer a contract unit test
(asserting preconditions/behaviour where observable) over a Miri job — see
`rustconn-locale-sys` and `rustconn-env-sys`, where the precondition guard is a
testable type precisely because the FFI call itself is not reachable from a test
harness. Keep the new crate an unconditional dependency even when only one
platform reaches the call. The `macos-sys` job covers the four existing helpers;
every other job is Linux, so a platform-gated `-sys` crate would still be
`unsafe` that only one job in the matrix compiles, and its guard, API and
contract tests would go unchecked everywhere else.
Do not allow unsafe to "spread" across the main crates.

## Documentation

### M-CANONICAL-DOCS — sections in doc comments

Public functions in `rustconn-core` must have:

```rust
/// Summary in one sentence, up to 15 words. (M-FIRST-DOC-SENTENCE)
///
/// Extended description.
///
/// # Errors
/// Returns `MyError::X` if ...
///
/// # Panics
/// Panics if ... (only for programming bugs, see M-PANIC-ON-BUG)
pub fn foo() -> Result<(), MyError> { ... }
```

Do not create a parameter table — describe them in the introductory sentence: `Copies a file from src to dst`.

### M-PUBLIC-DEBUG for types with secrets

If a type contains `SecretString` or credentials — `Debug` must be manual and covered by a test.
`secrecy::SecretString` already redacts itself in `Debug`, but wrappers around it need verification.

```rust
#[test]
fn debug_does_not_leak_secret() {
    let creds = Credentials::new("user", SecretString::new("hunter2".into()));
    let rendered = format!("{creds:?}");
    assert!(!rendered.contains("hunter2"));
}
```

## Naming — compromise M-CONCISE-NAMES

MS guideline recommends avoiding `Manager` / `Service` / `Factory`. We historically have
`ConnectionManager`, `SessionManager`, `SecretManager` — these names stay for compatibility.
For **new** code — choose more specific names: `ConnectionStore`, `SessionRouter`,
`CredentialResolver`, `SnippetCatalog`.

## Universal lints — state, not a wishlist

This section used to be a list of suggestions. Most of it had already been
applied, so it read as a TODO of finished work, and two entries were dismissed
with `# not relevant, we have forbid` — which stopped being true when the
workspace moved from `forbid` to `deny` + a crate-level `#![expect(unsafe_code)]`
in the four helpers, as the M-UNSAFE section above describes. `Cargo.toml` is the
source of truth; this is a reader's map of it.

**Already in `[workspace.lints.rust]`:** `unsafe_code = "deny"`,
`unused_lifetimes`, `redundant_lifetimes`, `unreachable_pub`, `redundant_imports`.

**Already in `[workspace.lints.clippy]`:** `all` / `pedantic` / `nursery`,
`allow_attributes_without_reason`, `clone_on_ref_ptr`, `redundant_clone`,
`empty_drop`, `unwrap_used`, `dbg_macro`, `todo`, `print_stdout`, `print_stderr`,
`wildcard_imports`.

**Deliberately not enabled:** `missing_debug_implementations`. On a GTK4 codebase
it fires on hundreds of widget-wrapping structs, breaking the 0-warning gate and
forcing manual `Debug` impls nobody asked for. The reason is recorded in
`Cargo.toml` too — do not "fix" this by turning it on.

**The unsafe-hardening set** (`unsafe_op_in_unsafe_fn`,
`undocumented_unsafe_blocks`, and the rest of the IronRDP block) is documented in
the next section rather than here, because it is the part that actually still
needed doing.

Whenever this table changes, verify with a run that genuinely re-checked:
`cargo clippy --all-targets`. A cache hit prints `Finished ... in 0.2s` and
reports zero warnings without looking at anything.

## References

- Checklist: <https://microsoft.github.io/rust-guidelines/guidelines/checklist/>
- Universal: <https://microsoft.github.io/rust-guidelines/guidelines/universal/>
- Apps: <https://microsoft.github.io/rust-guidelines/guidelines/apps/>
- Safety: <https://microsoft.github.io/rust-guidelines/guidelines/safety/>
- Docs: <https://microsoft.github.io/rust-guidelines/guidelines/docs/>
- Rust API Guidelines (upstream): <https://rust-lang.github.io/api-guidelines/>
- Rust API Guidelines checklist (quick review scan): <https://rust-lang.github.io/api-guidelines/checklist.html>
- rust-analyzer style (adapted in `rust-analyzer-style.md`): <https://rust-analyzer.github.io/book/contributing/style.html>
