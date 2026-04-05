---
name: "rustconn"
displayName: "RustConn"
description: "Development and release workflow for RustConn — GTK4/Rust connection manager with strict clippy, property tests, crate boundaries, and automated packaging"
keywords: ["rustconn", "rust", "clippy", "fmt", "cargo", "release", "version", "changelog", "packaging", "gtk4", "property test", "proptest"]
author: "Anton Isaiev"
---

# RustConn Development Power

Linux connection manager for SSH, RDP, VNC, SPICE, Telnet, Serial, Kubernetes, Zero Trust.
GTK4/libadwaita GUI, Wayland-first. Rust 2024 edition, MSRV 1.92, three-crate Cargo workspace.

## Available Steering Files

- **release.md** — Full release process: version bumps, dependencies, CLI updates, changelog, packaging, checklist

## Development Flow

1. Create a new branch from main
2. Bump version and create a CHANGELOG.md entry (use steering `release.md`)
3. Implement features incrementally
4. After each feature — automated checks via `rustconn-checks` hook (agentStop)
5. Manual GUI testing
6. Before merge — update dependencies and CLI versions (see steering `release.md`)
7. Merge into main
8. `git tag -a vX.Y.Z -m "Release X.Y.Z" && git push origin main --tags` — triggers CI

## Automated Checks

After completing a feature, delegate checks to the `general-task-execution` sub-agent:

```
Run sequentially and return results (pass/fail + errors if any):
1. cargo fmt --check
2. cargo clippy --all-targets (must be 0 warnings)
3. cargo test --workspace (timeout 180s, argon2 property tests take ~120s in debug mode)
If fmt or clippy have errors — fix automatically and re-run.
```

This saves the main dialog context from thousands of lines of cargo output.

**NOTE:** The `rustconn-checks` hook (agentStop) already runs these checks automatically after every agent response. No need to delegate manually — the hook handles it.

## Quick Reference

| Task | Command |
|------|---------|
| Check compilation | `cargo check --all-targets` |
| Clippy | `cargo clippy --all-targets` |
| Clippy + fix | `cargo clippy --all-targets --fix --allow-dirty` |
| Format | `cargo fmt` |
| Format check | `cargo fmt --check` |
| All tests | `cargo test --workspace` |
| Property tests | `cargo test -p rustconn-core --test property_tests` |
| Build release | `cargo build --release` |
| Run GUI | `cargo run -p rustconn` |
| Run CLI | `cargo run -p rustconn-cli` |
| Check CLI versions | `./scripts/check-cli-versions.sh` |

## Crate Boundaries

**Core rule: "Does it need GTK?"**

| Answer | Crate | Restrictions |
|--------|-------|-------------|
| No | `rustconn-core` | GUI-free — `gtk4`, `vte4`, `adw` FORBIDDEN |
| Yes | `rustconn` | May import GTK |
| CLI | `rustconn-cli` | Only `rustconn-core` |

### Where to add code

| Feature type | Location | Action |
|-------------|----------|--------|
| Data model | `rustconn-core/src/models/` | Re-export in `models.rs` and `lib.rs` |
| Protocol | `rustconn-core/src/protocol/` | Implement `Protocol` trait |
| Import format | `rustconn-core/src/import/` | Implement `ImportSource` trait |
| Export format | `rustconn-core/src/export/` | Implement `ExportTarget` trait |
| Secret backend | `rustconn-core/src/secret/` | Implement `SecretBackend` trait |
| Template mgmt | `rustconn-core/src/template/` | Via `TemplateManager` |
| Snippet mgmt | `rustconn-core/src/snippet/` | Via `SnippetManager` |
| Dialog | `rustconn/src/dialogs/` | Register in `dialogs/mod.rs` |
| Property test | `rustconn-core/tests/properties/` | Register in `properties/mod.rs` |
| Integration test | `rustconn-core/tests/integration/` | Register in `integration/mod.rs` |

## Strict Rules

| ✅ REQUIRED | ❌ FORBIDDEN |
|-------------|--------------|
| `Result<T, Error>` for fallible functions | `unwrap()`/`expect()` (except provably impossible) |
| `thiserror` for all error types | Error types without `#[derive(thiserror::Error)]` |
| `SecretString` for credentials | Plain `String` for passwords/keys |
| `tokio` for async | Mixing async runtimes |
| GUI-free `rustconn-core` | `gtk4`/`vte4`/`adw` in `rustconn-core` |
| `adw::` widgets | Deprecated GTK patterns |
| `tracing` for structured logging | `println!`/`eprintln!` for log output |
| Line width 100 chars, 4 spaces, LF | Tabs, CRLF, long lines |
| `unsafe_code = "forbid"` | Any unsafe code |
| Rust 2024 edition patterns (let-chains) | Legacy `if let` + `collapsible_if` |

## Code Patterns

### Error Types
```rust
#[derive(Debug, thiserror::Error)]
pub enum MyError {
    #[error("description: {0}")]
    Variant(String),
}
```

### Credentials (MUST use SecretString)
```rust
use secrecy::SecretString;
let password: SecretString = SecretString::new(value.into());
```

### Identifiers
```rust
let id = uuid::Uuid::new_v4();
```

### Timestamps
```rust
let now: chrono::DateTime<chrono::Utc> = chrono::Utc::now();
```

### Async Traits
```rust
#[async_trait::async_trait]
impl MyTrait for MyStruct {
    async fn method(&self) -> Result<(), Error> { /* ... */ }
}
```

### Rust 2024 Edition Patterns
```rust
// Let-chains instead of collapsible_if
if let Some(x) = opt && x > 0 {
    // ...
}

// Never use set_var/remove_var (unsafe in Rust 2024)
// Use OnceLock, RwLock, or process re-exec instead
```

## Testing

### Property Tests

Location: `rustconn-core/tests/properties/`

⏱️ Full test suite takes ~120 seconds (argon2 property tests are slow in debug mode). Always wait for completion (timeout 180s).

Adding a new property test module:
1. Create a file in `rustconn-core/tests/properties/`
2. Register it in `rustconn-core/tests/properties/mod.rs`

Temp files — always use the `tempfile` crate.

## UI Patterns (rustconn/)

| Pattern | Implementation |
|---------|----------------|
| Widgets | `adw::` over `gtk::` equivalents |
| Toasts | `adw::ToastOverlay` with severity icons |
| Dialogs | `adw::Dialog` or `gtk::Window` + `set_modal(true)` |
| Spacing | 12px margins, 6px between related elements (GNOME HIG) |
| Wayland | Avoid X11-specific APIs |
| i18n | `gettext`/`ngettext`, `i18n_f()` with `{}` placeholders |

## State Management

```rust
pub type SharedAppState = Rc<RefCell<AppState>>;
```

- Pass `&SharedAppState` for mutable access
- Manager structs: `ConnectionManager`, `SessionManager`, `SecretManager`, `DocumentManager`, `ClusterManager`, `SnippetManager`, `TemplateManager`
- Async: `with_runtime()` for thread-local tokio runtime
- Never hold a borrow across async boundaries or GTK callbacks

## i18n Notes

- User-visible strings: `gettext("...")` or `i18n("...")`
- With parameters: `i18n_f("{} connections", &[&count.to_string()])` — positional `{}`
- In `window/mod.rs`: use `crate::i18n::i18n(...)` (full path)
- After adding new strings: run `po/update-pot.sh`, then merge into all `.po` files
- 15 languages: uk, de, fr, es, it, pl, cs, sk, da, sv, nl, pt, be, kk, uz

## CLI Downloads (`rustconn-core/src/cli_download.rs`)

Pinned CLI versions for Flatpak sandbox. Run `./scripts/check-cli-versions.sh` to check for updates.

| Component | ID | Current Version |
|-----------|----|-----------------|
| TigerVNC | `vncviewer` | 1.16.1 |
| Teleport | `tsh` | 18.7.2 |
| Tailscale | `tailscale` | 1.96.4 |
| Boundary | `boundary` | 0.21.1 |
| Bitwarden CLI | `bw` | 2026.2.0 |
| 1Password CLI | `op` | 2.33.1 |
| kubectl | `kubectl` | 1.35.3 |

"Latest" URL (no pinned version): AWS CLI, SSM Plugin, gcloud, Azure CLI, OCI CLI, cloudflared.

When updating a pinned version — update `pinned_version`, `download_url`, `aarch64_url`, and `checksum` (if `Static`).

## Clippy Troubleshooting

| Lint | Solution |
|------|----------|
| `cognitive_complexity` | Split into smaller functions |
| `too_many_arguments` | Create a parameter struct |
| `missing_errors_doc` | Add `# Errors` section |
| Clippy doesn't see changes | `cargo clean && cargo clippy --all-targets` |
