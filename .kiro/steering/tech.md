---
inclusion: always
---

# RustConn Tech Stack

Rust 2021 edition, MSRV 1.88, three-crate Cargo workspace.

## Crate Overview

| Crate | Purpose | Key Dependencies |
|-------|---------|------------------|
| `rustconn` | GTK4 GUI | `gtk4` 0.10 (`v4_14`), `vte4` 0.9, `libadwaita` 0.8 (`v1_5`), optional `ksni`+`resvg`+`cpal` |
| `rustconn-core` | Business logic (GUI-free) | `tokio` 1.49, `serde`/`serde_json`/`serde_yaml`/`toml`, `uuid`, `chrono`, `thiserror`, `secrecy`, `ring`+`argon2`, `regex`, `ironrdp` 0.14, `vnc-rs` 0.5 |
| `rustconn-cli` | CLI interface | `clap` 4.5 (derive), depends on `rustconn-core` only |

## Enforced Code Style

- `unsafe_code = "forbid"` — no unsafe code allowed
- Clippy lints: `all`, `pedantic`, `nursery` — all warnings must be resolved
- Line width: 100 chars max
- Indentation: 4 spaces
- Line endings: LF only

## Required Code Patterns

Always use these exact patterns:

### Error Types

```rust
#[derive(Debug, thiserror::Error)]
pub enum MyError {
    #[error("description: {0}")]
    Variant(String),
}
```

### Credential Handling

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

## Strict Rules

| REQUIRED | FORBIDDEN |
|----------|-----------|
| Return `Result<T, Error>` from fallible functions | `unwrap()`/`expect()` except for provably impossible states |
| `thiserror` for all error types | Error types without `#[derive(thiserror::Error)]` |
| `SecretString` for all credentials | Plain `String` for passwords/keys |
| `tokio` for all async code | Mixing async runtimes |
| GUI-free `rustconn-core` | `gtk4`/`vte4`/`adw` imports in `rustconn-core` |
| `adw::` widgets over `gtk::` equivalents | Deprecated GTK patterns |

## Testing Requirements

- Property tests: `rustconn-core/tests/properties/` using `proptest`
- Temp files: always use `tempfile` crate
- New property test modules must be registered in `tests/properties/mod.rs`
- Full test suite runs ~1 minute; wait for completion after code changes before next command

## Build & Test Commands

```bash
cargo build                    # Build all crates
cargo build --release          # Release build
cargo run -p rustconn          # Run GUI
cargo run -p rustconn-cli      # Run CLI
cargo test                     # Run all tests
cargo test -p rustconn-core --test property_tests  # Property tests only
cargo clippy --all-targets     # Lint check (must pass with no warnings)
cargo fmt --check              # Format check
```
