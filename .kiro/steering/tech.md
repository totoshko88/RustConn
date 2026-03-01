---
inclusion: always
---

# RustConn Tech Stack

Rust 2024 edition, MSRV 1.88, three-crate Cargo workspace (`resolver = "2"`).

## Crate Boundaries

| Crate | Role | May import GTK/adw/vte? |
|-------|------|:-----------------------:|
| `rustconn` | GTK4 binary | Yes |
| `rustconn-core` | Business logic library | **No** — never import `gtk4`, `vte4`, `adw` |
| `rustconn-cli` | CLI binary | **No** — depends on `rustconn-core` only |

**Decision rule:** if it doesn't need GTK, put it in `rustconn-core`.

## Key Dependencies

| Domain | Crate(s) | Notes |
|--------|----------|-------|
| GUI | `gtk4` 0.10 (`v4_14`), `libadwaita` 0.8 (`v1_5`), `vte4` 0.9 (`v0_72`) | `rustconn` only |
| Async | `tokio` 1.49 (full) | All crates — sole async runtime, never mix runtimes |
| Serialization | `serde` (derive), `serde_json`, `serde_yaml` (actually `serde_yaml_ng`), `toml` | `serde_yaml` re-exported as `serde_yaml_ng` in workspace deps |
| Errors | `thiserror` 2.0 | Every error enum must derive `thiserror::Error` |
| Secrets | `secrecy` 0.10 (serde) | Every credential field — never plain `String` |
| Crypto | `ring` 0.17, `argon2` 0.5 | `rustconn-core` only |
| IDs | `uuid` 1.21 (v4 + serde) | All crates |
| Time | `chrono` 0.4 (serde) | All crates |
| CLI | `clap` 4.5.60 (derive) | `rustconn-cli` only |
| RDP | `ironrdp` 0.14 | Feature-gated: `rdp-embedded` |
| VNC | `vnc-rs` 0.5 | Feature-gated: `vnc-embedded` |
| Testing | `proptest` 1.9, `tempfile` 3.23 | dev-dependencies in `rustconn-core` |

## Lints & Formatting

All lint config lives in workspace `Cargo.toml` under `[workspace.lints.clippy]`.

- `unsafe_code = "forbid"` — no unsafe code, ever.
- Clippy groups `all` + `pedantic` + `nursery` at warn level. **Zero warnings required.**
- `.clippy.toml` thresholds: `cognitive-complexity-threshold = 25`, `too-many-arguments-threshold = 7`, `type-complexity-threshold = 250`.
- Line width: 100 chars. Indentation: 4 spaces. Line endings: LF only.
- Allowed clippy exceptions (e.g. `cast_precision_loss`, `wildcard_imports`, `needless_pass_by_value`) are declared centrally in workspace `Cargo.toml` with justification comments. **Do not add new `#[allow(...)]` in source** without justification.
- The only acceptable location for broad `#![allow(...)]` blocks is test entry points (`property_tests.rs`, `integration_tests.rs`).

## Required Code Patterns

### Errors — always `thiserror`

```rust
#[derive(Debug, thiserror::Error)]
pub enum MyError {
    #[error("description: {0}")]
    Variant(String),
}
```

Domain-specific `Result` aliases: `ConfigResult<T>`, `ProtocolResult<T>`, `SecretResult<T>`, `SessionResult<T>`, `ImportOperationResult<T>`. Use `RustConnError` / `Result<T>` at crate API boundaries.

### Credentials — always `SecretString`

```rust
use secrecy::SecretString;
let password: SecretString = SecretString::new(value.into());
```

### IDs and timestamps

```rust
let id = uuid::Uuid::new_v4();
let now: chrono::DateTime<chrono::Utc> = chrono::Utc::now();
```

### Async traits

```rust
#[async_trait::async_trait]
impl MyTrait for MyStruct {
    async fn method(&self) -> Result<(), Error> { /* ... */ }
}
```

### Public API documentation

`#![warn(missing_docs)]` is enabled on `rustconn-core`. Document every public item with `///`.

### Re-exports

All new public types in `rustconn-core` must be re-exported through `lib.rs`. Feature-gated types use matching `#[cfg(feature = "...")]` guards on the re-export.

### Logging

Use `tracing` with structured fields. Never use `println!` / `eprintln!` for log output.
**Exception:** `rustconn-cli` uses `println!` for user-facing CLI output (lists, status messages, confirmations). Only diagnostic/log output must use `tracing`.

```rust
tracing::info!(protocol = "ssh", host = %host, port = %port, "Connection established");
tracing::error!(?error, protocol = "rdp", host = %host, "Connection failed");
```

## Strict Rules (Quick Reference)

| REQUIRED | FORBIDDEN |
|----------|-----------|
| `Result<T, Error>` from fallible functions | `unwrap()` / `expect()` except provably impossible states |
| `thiserror` for all error types | Error types without `#[derive(thiserror::Error)]` |
| `SecretString` for all credentials | Plain `String` for passwords/keys/tokens |
| `tokio` as sole async runtime | Mixing async runtimes |
| GUI-free `rustconn-core` | `gtk4`/`vte4`/`adw` imports in `rustconn-core` or `rustconn-cli` |
| `adw::` widgets over `gtk::` equivalents | Deprecated GTK patterns |
| `tracing` for structured logging | `println!` / `eprintln!` for log output (CLI user-facing `println!` is OK) |
| `#[cfg(feature = "...")]` for gated code | Unconditional use of feature-gated types |

## Feature Flags

| Flag | Crate | Default | Purpose |
|------|-------|:-------:|---------|
| `vnc-embedded` | `rustconn-core` | Yes | Native VNC via vnc-rs |
| `rdp-embedded` | `rustconn-core` | Yes | Native RDP via IronRDP |
| `spice-embedded` | `rustconn-core` | No | Native SPICE client |
| `tray` | `rustconn` | Yes | System tray (ksni + resvg) |
| `rdp-audio` | `rustconn` | Yes | RDP audio playback (cpal) |
| `wayland-native` | `rustconn` | Yes | Wayland surface support |

Guard feature-gated code with `#[cfg(feature = "...")]`. Check feature availability before referencing embedded client types.

## Testing

| Type | Location | Entry point | Registration |
|------|----------|-------------|--------------|
| Property tests | `rustconn-core/tests/properties/*.rs` | `tests/property_tests.rs` | Add `mod` in `properties/mod.rs` |
| Integration tests | `rustconn-core/tests/integration/*.rs` | `tests/integration_tests.rs` | Add `mod` in `integration/mod.rs` |
| Fixtures | `rustconn-core/tests/fixtures/` | — | — |
| Benchmarks | `rustconn-core/benches/` | — | — |

- Property tests use `proptest` 1.9. Temp files use `tempfile`.
- New test modules must be registered in the corresponding `mod.rs`.
- Full test suite runs ~2 minutes (argon2 property tests are slow in debug mode). Wait for completion before running the next command.

## Build & Test Commands

```bash
cargo build                                          # Debug build
cargo build --release                                # Release build
cargo test                                           # All tests
cargo test -p rustconn-core --test property_tests    # Property tests only
cargo clippy --all-targets                           # Lint (zero warnings required)
cargo fmt --check                                    # Format check
```
