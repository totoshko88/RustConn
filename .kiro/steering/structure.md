---
inclusion: always
---

# RustConn Project Structure

Three-crate Cargo workspace with strict GUI/logic separation.

## Crate Boundaries

| Crate | Type | Constraint |
|-------|------|------------|
| `rustconn/` | Binary | GUI only — may import `gtk4`, `vte4`, `adw` |
| `rustconn-core/` | Library | GUI-free — NEVER import `gtk4`, `vte4`, `adw` |
| `rustconn-cli/` | Binary | CLI — depends on `rustconn-core` only |

Placement rule: "Does this need GTK?" → No → `rustconn-core` → Yes → `rustconn`

## Where to Add New Code

| Feature Type | Location | Required Action |
|--------------|----------|-----------------|
| Data model | `rustconn-core/src/models/` | Re-export in `models.rs` and `lib.rs` |
| Protocol impl | `rustconn-core/src/protocol/` | Implement `Protocol` trait |
| Import format | `rustconn-core/src/import/` | Implement `ImportSource` trait |
| Export format | `rustconn-core/src/export/` | Implement `ExportTarget` trait |
| Secret backend | `rustconn-core/src/secret/` | Implement `SecretBackend` trait |
| Dialog | `rustconn/src/dialogs/` | Register in `dialogs/mod.rs` |
| Property test | `rustconn-core/tests/properties/` | Add `mod` entry in `properties/mod.rs` |
| Integration test | `rustconn-core/tests/integration/` | Add `mod` entry in `integration/mod.rs` |
| Performance code | `rustconn-core/src/performance/` | Re-export in `lib.rs` |
| Benchmark | `rustconn-core/benches/` | — |

All new public types in `rustconn-core` must be re-exported through `lib.rs`.

## GUI Crate Layout (`rustconn/src/`)

| Path | Purpose |
|------|---------|
| `app.rs` | GTK Application setup, global actions, keyboard shortcuts |
| `window/mod.rs` | Main window; split by domain into `window/*.rs` (clusters, protocols, sessions, groups, sorting, etc.) |
| `sidebar/` | Connection tree — `mod.rs` (logic), `view.rs` (widget), `filter.rs`, `search.rs`, `drag_drop.rs` |
| `sidebar_types.rs`, `sidebar_ui.rs` | Sidebar type definitions and UI helpers (top-level) |
| `state.rs` | `SharedAppState = Rc<RefCell<AppState>>` — central mutable state |
| `dialogs/` | Modal dialogs; subdirs for `connection/` and `settings/` |
| `embedded_rdp/` | IronRDP embedded viewer (buffer, launcher, thread, UI) |
| `embedded_vnc.rs`, `embedded_vnc_ui.rs`, `embedded_vnc_types.rs` | vnc-rs embedded viewer |
| `embedded_spice.rs` | SPICE embedded viewer |
| `embedded_trait.rs` | Shared trait for embedded protocol viewers |
| `terminal/` | VTE terminal notebook (config, types) |
| `split_view/` | Split view UI (adapter, bridge, manager, types) |
| `session/` | Session UI components |
| `async_utils.rs` | `spawn_async`, `block_on_async`, `with_runtime` — GTK↔tokio bridge |
| `audio.rs` | RDP audio playback via cpal |
| `tray.rs` | System tray icon (ksni) |
| `toast.rs` | Toast notification helpers |
| `error.rs`, `error_display.rs` | GUI error types and display formatting |
| `wayland_surface.rs` | Wayland surface utilities |
| `validation.rs` | Input validation for dialogs |

## Core Library Layout (`rustconn-core/src/`)

| Directory | Purpose |
|-----------|---------|
| `models/` | Connection, Group, Protocol configs, Snippet, Template, History, Credentials, CustomProperty |
| `config/` | `AppSettings`, `ConfigManager`, persistence |
| `connection/` | Connection CRUD, lazy loading, virtual scroll, retry, selection, port checking, string interning |
| `protocol/` | `Protocol` trait, `ProtocolRegistry`, per-protocol impls, client detection |
| `import/` | Format importers (Remmina, Asbru-CM, SSH config, Ansible, MobaXterm, Royal TS, virt-viewer) |
| `export/` | Format exporters, native format, batch export |
| `secret/` | `SecretBackend` trait, `SecretManager`, KeePassXC, libsecret, async credential resolution |
| `session/` | Session state, `SessionManager`, logging (`LogConfig`, `LogContext`) |
| `automation/` | Expect scripts, key sequences, connection tasks |
| `search/` | `SearchEngine`, fuzzy search with `SearchCache` and debouncing |
| `rdp_client/` | IronRDP client, input handling, scancode mapping, clipboard |
| `vnc_client/` | vnc-rs client |
| `spice_client/` | SPICE client, shared folders |
| `split/` | Split view layout model, color pool, panel tree |
| `variables/` | Variable substitution engine with vault integration |
| `tracing/` | Structured logging config and initialization |
| `wol/` | Wake-on-LAN magic packet generation |
| `cluster/` | Cluster command broadcasting and session management |
| `dashboard/` | Dashboard statistics and session stats |
| `document/` | Encrypted document management |
| `ssh_agent/` | SSH agent key management |
| `performance/` | Memory optimization, metrics, string interning, object pooling, batch processing |
| `testing/` | Connection testing framework |
| `snippet/` | `SnippetManager` |
| `ffi/` | FFI display types for embedded clients |

Standalone files in `rustconn-core/src/`:
- `error.rs` — hierarchical error types with typed `Result` aliases
- `models.rs` — re-exports from `models/` directory
- `password_generator.rs`, `sftp.rs`, `flatpak.rs`, `snap.rs`, `cli_download.rs`, `terminal_themes.rs`, `dialog_utils.rs`, `drag_drop.rs`, `progress.rs`, `embedded_client_error.rs`

## Error Type Hierarchy (`rustconn-core/src/error.rs`)

```
RustConnError (top-level)
├── ConfigError    — config parse/validation/IO  → ConfigResult<T>
├── ProtocolError  — connection/auth/client       → ProtocolResult<T>
├── SecretError    — credential storage backends  → SecretResult<T>
├── ImportError    — format parsing/validation    → ImportOperationResult<T>
├── SessionError   — session lifecycle            → SessionResult<T>
└── Io             — std::io::Error passthrough
```

Use the domain-specific `Result` alias (e.g., `ConfigResult<T>`) within that domain. Use `RustConnError` / `Result<T>` at API boundaries.

## State Management

```rust
pub type SharedAppState = Rc<RefCell<AppState>>;
```

- Pass `&SharedAppState` to functions needing mutable access
- Access via helper functions: `with_state()`, `with_state_mut()`, `try_with_state()`, `try_with_state_mut()`
- Manager structs (`ConnectionManager`, `SessionManager`, `SecretManager`, `DocumentManager`, `ClusterManager`, `SnippetManager`) own data and handle I/O
- `AppState` delegates to managers and provides convenience methods

## Async Pattern (GUI ↔ Tokio)

GTK4 is single-threaded. Use these patterns from `rustconn/src/async_utils.rs`:

| Scenario | Function | Blocks UI? |
|----------|----------|------------|
| Short async from GTK callback | `spawn_async()` | No |
| Async with result callback | `spawn_async_with_callback()` | No |
| Need result immediately | `block_on_async()` / `with_runtime()` | Yes |
| With timeout | `block_on_async_with_timeout()` | Yes |

Prefer `spawn_async` over `block_on_async` whenever possible.

## Module Conventions

- Feature directories use `mod.rs` as entry point
- Public types re-exported through `rustconn-core/src/lib.rs`
- Split large files by concern: `*_types.rs`, `*_ui.rs`
- `#![warn(missing_docs)]` is enabled on `rustconn-core` — document all public items

## Test Structure

| Type | Location | Entry Point | Registration |
|------|----------|-------------|--------------|
| Property tests | `rustconn-core/tests/properties/*.rs` | `tests/property_tests.rs` | `properties/mod.rs` |
| Integration tests | `rustconn-core/tests/integration/*.rs` | `tests/integration_tests.rs` | `integration/mod.rs` |
| Fixtures | `rustconn-core/tests/fixtures/` | `tests/fixtures/mod.rs` | — |
| CLI tests | `rustconn-cli/tests/` | — | — |
| Benchmarks | `rustconn-core/benches/` | — | — |

Test entry points (`property_tests.rs`, `integration_tests.rs`) include clippy `#![allow(...)]` directives for common test patterns. New test modules must be added as `mod` entries in the corresponding `mod.rs`.

## Feature Flags

| Flag | Crate | Purpose | Default |
|------|-------|---------|---------|
| `vnc-embedded` | `rustconn-core` | Native VNC client via vnc-rs | Yes |
| `rdp-embedded` | `rustconn-core` | Native RDP client via IronRDP | Yes |
| `spice-embedded` | `rustconn-core` | Native SPICE client | No |
| `ksni` | `rustconn` | System tray icon | Optional |
| `cpal` | `rustconn` | RDP audio playback | Optional |

Conditional re-exports in `lib.rs` use `#[cfg(feature = "...")]` — check before referencing embedded client types.
