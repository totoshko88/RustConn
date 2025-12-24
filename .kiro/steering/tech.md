---
inclusion: always
---

# RustConn Tech Stack

Rust 2021 edition, MSRV 1.85. Cargo workspace with three crates.

## Crate Overview

| Crate | Type | Key Dependencies |
|-------|------|------------------|
| `rustconn` | GUI binary | `gtk4` 0.10 (`v4_14`), `vte4` 0.9, optional `ksni`+`resvg` (tray) |
| `rustconn-core` | Library | `tokio` 1.48 (full), `serde`/`serde_json`/`serde_yaml`/`toml`, `uuid` (v4), `chrono`, `thiserror`, `secrecy`, `ring`+`argon2`, `regex` |
| `rustconn-cli` | CLI binary | `clap` 4.5 (derive) |

## Code Style

- `unsafe_code = "forbid"` — never use unsafe
- Clippy lints: `all`, `pedantic`, `nursery` enabled
- Line width: 100 chars max
- Indentation: 4 spaces
- Line endings: Unix (LF)

## Required Patterns

### Error Handling

Always use `thiserror` for error types:

```rust
#[derive(Debug, thiserror::Error)]
pub enum MyError {
    #[error("description: {0}")]
    Variant(String),
}
```

### Sensitive Data

Wrap credentials in `SecretString`:

```rust
use secrecy::SecretString;
let password: SecretString = SecretString::new(value.into());
```

### Identifiers

Use UUID v4 for unique IDs:

```rust
use uuid::Uuid;
let id = Uuid::new_v4();
```

### Timestamps

Use chrono with UTC:

```rust
use chrono::{DateTime, Utc};
let now: DateTime<Utc> = Utc::now();
```

### Async Traits

Use `async-trait` macro:

```rust
#[async_trait::async_trait]
impl MyTrait for MyStruct {
    async fn method(&self) -> Result<(), Error> { ... }
}
```

## Do / Don't

| Do | Don't |
|----|-------|
| Return `Result<T, Error>` | Use `unwrap()`/`expect()` except impossible states |
| Use `thiserror` for errors | Define errors without `#[derive(thiserror::Error)]` |
| Wrap secrets in `SecretString` | Store credentials as plain `String` |
| Use `tokio` for async | Mix async runtimes |
| Keep `rustconn-core` GUI-free | Import `gtk4`/`vte4` in `rustconn-core` |

## Testing

- Property-based tests: `rustconn-core/tests/properties/` using `proptest`
- Temp directories: use `tempfile` crate
- New property tests: add module to `tests/properties/mod.rs`

## Commands

```bash
cargo build                    # Build all crates
cargo build --release          # Release build
cargo run -p rustconn          # Run GUI app
cargo run -p rustconn-cli      # Run CLI
cargo test                     # Run all tests
cargo test -p rustconn-core --test property_tests  # Property tests only
cargo clippy --all-targets     # Check lints
cargo fmt --check              # Verify formatting
```
