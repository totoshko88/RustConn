---
inclusion: always
---

# RustConn Tech Stack

Rust 2021 edition, MSRV 1.88, three-crate Cargo workspace.

## Crates

| Crate | Role | GUI imports allowed? |
|-------|------|:--------------------:|
| `rustconn` | GTK4 binary | Yes |
| `rustconn-core` | Business logic library | No — never import `gtk4`, `vte4`, `adw` |
| `rustconn-cli` | CLI binary | No — depends on `rustconn-core` only |

Placement rule: if it doesn't need GTK, it belongs in `rustconn-core`.

## Key Dependencies

| Domain | Crate(s) | Notes |
|--------|----------|-------|
| GUI | `gtk4` 0.10 (`v4_14`), `libadwaita` 0.8 (`v1_5`), `vte4` 0.9 (`v0_72`) | `rustconn` only |
| Async | `tokio` 1.49 (full features) | All crates; sole async runtime |
| Serialization | `serde` + `serde_json` + `serde_yaml` + `toml` | Derive feature enabled |
| Errors | `thiserror` 2.0 | Every error enum |
| Secrets | `secrecy` 0.10 (serde feature) | Every credential field |
| Crypto | `ring` 0.17, `argon2` 0.5 | `rustconn-core` only |
| IDs | `uuid` 1.11 (v4 + serde) | All crates |
| Time | `chrono` 0.4 (serde feature) | All crates |
| CLI | `clap` 4.5 (derive) | `rustconn-cli` only |
| RDP | `ironrdp` 0.14 | Feature-gated (`rdp-embedded`) |
| VNC | `vnc-rs` 0.5 | Feature-gated (`vnc-embedded`) |
| Testing | `proptest` 1.6, `tempfile` 3.15 | dev-dependencies in `rustconn-core` |

## Lints & Formatting

- `unsafe_code = "forbid"` — no unsafe code, ever.
- Clippy: `all` + `pedantic` + `nursery` at warn level. Zero warnings required.
- Line width: 100 chars. Indentation: 4 spaces. Line endings: LF only.
- `.clippy.toml` thresholds: cognitive-complexity 25, too-many-arguments 7, type-complexity 250.
- Allowed clippy exceptions are declared in workspace `Cargo.toml`. Do not add new `#[allow(...)]` in source without justification.
- Test entry points (`property_tests.rs`, `integration_tests.rs`) carry their own `#![allow(...)]` blocks for common test patterns — that is the only acceptable location for broad allows.

## Required Code Patterns

### Error types — always `thiserror`

```rust
#[derive(Debug, thiserror::Error)]
pub enum MyError {
    #[error("description: {0}")]
    Variant(String),
}
```

Use domain-specific Result aliases (`ConfigResult<T>`, `ProtocolResult<T>`, `SecretResult<T>`, `SessionResult<T>`, `ImportOperationResult<T>`) within their domain. Use `RustConnError` / `Result<T>` at crate API boundaries.

### Credentials — always `SecretString`

```rust
use secrecy::SecretString;
let password: SecretString = SecretString::new(value.into());
```

Never store passwords, keys, or tokens as plain `String`.

### Identifiers

```rust
let id = uuid::Uuid::new_v4();
```

### Timestamps

```rust
let now: chrono::DateTime<chrono::Utc> = chrono::Utc::now();
```

### Async traits

```rust
#[async_trait::async_trait]
impl MyTrait for MyStruct {
    async fn method(&self) -> Result<(), Error> { /* ... */ }
}
```

### Public API docs

`#![warn(missing_docs)]` is enabled on `rustconn-core`. Document every public item with `///`.

### Re-exports

All new public types in `rustconn-core` must be re-exported through `lib.rs`.

## Strict Rules

| REQUIRED | FORBIDDEN |
|----------|-----------|
| `Result<T, Error>` from fallible functions | `unwrap()`/`expect()` except provably impossible states |
| `thiserror` for all error types | Error types without `#[derive(thiserror::Error)]` |
| `SecretString` for all credentials | Plain `String` for passwords/keys/tokens |
| `tokio` as sole async runtime | Mixing async runtimes |
| GUI-free `rustconn-core` | `gtk4`/`vte4`/`adw` imports in `rustconn-core` or `rustconn-cli` |
| `adw::` widgets over `gtk::` equivalents | Deprecated GTK patterns |
| `tracing` for structured logging | `println!`/`eprintln!` for log output |

## Feature Flags

| Flag | Crate | Default | Purpose |
|------|-------|:-------:|---------|
| `vnc-embedded` | `rustconn-core` | Yes | Native VNC via vnc-rs |
| `rdp-embedded` | `rustconn-core` | Yes | Native RDP via IronRDP |
| `spice-embedded` | `rustconn-core` | No | Native SPICE client |
| `tray` | `rustconn` | Yes | System tray (ksni + resvg) |
| `rdp-audio` | `rustconn` | Yes | RDP audio playback (cpal) |
| `wayland-native` | `rustconn` | Yes | Wayland surface support |

Guard feature-gated code with `#[cfg(feature = "...")]`. Check before referencing embedded client types.

## Testing

### Structure

| Type | Location | Entry point | Registration |
|------|----------|-------------|--------------|
| Property tests | `rustconn-core/tests/properties/*.rs` | `tests/property_tests.rs` | Add `mod` in `properties/mod.rs` |
| Integration tests | `rustconn-core/tests/integration/*.rs` | `tests/integration_tests.rs` | Add `mod` in `integration/mod.rs` |
| Fixtures | `rustconn-core/tests/fixtures/` | — | — |
| Benchmarks | `rustconn-core/benches/` | — | — |

### Rules

- Property tests use `proptest` 1.6. Temp files use `tempfile` crate.
- New property test modules must be registered in `tests/properties/mod.rs`.
- Full test suite runs ~1 minute. Wait for completion before running the next command.

## Build & Test Commands

```bash
cargo build                                          # Debug build
cargo build --release                                # Release build
cargo test                                           # All tests
cargo test -p rustconn-core --test property_tests    # Property tests only
cargo clippy --all-targets                           # Lint (zero warnings required)
cargo fmt --check                                    # Format check
```
