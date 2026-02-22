---
inclusion: always
---

# RustConn Project Structure

Three-crate Cargo workspace. Strict GUI/logic separation is the primary architectural constraint.

## Crate Boundaries

| Crate | Type | Constraint |
|-------|------|------------|
| `rustconn/` | Binary | GUI — may import `gtk4`, `vte4`, `adw` |
| `rustconn-core/` | Library | GUI-free — NEVER import `gtk4`, `vte4`, `adw` |
| `rustconn-cli/` | Binary | CLI — depends on `rustconn-core` only |

Decision rule: "Does this need GTK?" No → `rustconn-core`. Yes → `rustconn`.

## Where to Add New Code

| Feature Type | Location | Required Action |
|--------------|----------|-----------------|
| Data model | `rustconn-core/src/models/` | Re-export in `models.rs` and `lib.rs` |
| Protocol impl | `rustconn-core/src/protocol/` | Implement `Protocol` trait |
| Import format | `rustconn-core/src/import/` | Implement `ImportSource` trait |
| Export format | `rustconn-core/src/export/` | Implement `ExportTarget` trait |
| Secret backend | `rustconn-core/src/secret/` | Implement `SecretBackend` trait |
| Dialog | `rustconn/src/dialogs/` | Register in `dialogs/mod.rs` |
| Property test | `rustconn-core/tests/properties/` | Add `mod` in `properties/mod.rs` |
| Integration test | `rustconn-core/tests/integration/` | Add `mod` in `integration/mod.rs` |
| Performance code | `rustconn-core/src/performance/` | Re-export in `lib.rs` |
| Benchmark | `rustconn-core/benches/` | — |

All new public types in `rustconn-core` must be re-exported through `lib.rs`.

## Core Library Layout (`rustconn-core/src/`)

Directories (each uses `mod.rs` as entry point):

| Directory | Purpose |
|-----------|---------|
| `models/` | Connection, Group, Protocol configs, Snippet, Template, History, Credentials, CustomProperty |
| `config/` | `AppSettings`, `ConfigManager`, persistence |
| `connection/` | Connection CRUD, lazy loading, virtual scroll, retry, selection, port checking, string interning |
| `protocol/` | `Protocol` trait, `ProtocolRegistry`, per-protocol impls, client detection |
| `import/` | Format importers (Remmina, Asbru-CM, SSH config, Ansible, MobaXterm, Royal TS, virt-viewer) |
| `export/` | Format exporters, native format, batch export |
| `secret/` | `SecretBackend` trait, `SecretManager`, KeePassXC, libsecret, Bitwarden, async credential resolution |
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

## GUI Crate Layout (`rustconn/src/`)

| Path | Purpose |
|------|---------|
| `main.rs` | Application entry point |
| `app.rs` | GTK Application setup, global actions, keyboard shortcuts |
| `state.rs` | `SharedAppState = Rc<RefCell<AppState>>` — central mutable state |
| `i18n.rs` | Internationalization / gettext setup |
| `window/mod.rs` | Main window; split by domain into `window/*.rs` (clusters, protocols, sessions, groups, sorting) |
| `sidebar/` | Connection tree — `mod.rs` (logic), `view.rs` (widget), `filter.rs`, `search.rs`, `drag_drop.rs` |
| `sidebar_types.rs`, `sidebar_ui.rs` | Sidebar type definitions and UI helpers (top-level) |
| `dialogs/` | Modal dialogs; subdirs for `connection/` and `settings/` |
| `embedded_rdp/` | IronRDP embedded viewer (buffer, launcher, thread, UI) |
| `embedded_vnc.rs`, `embedded_vnc_ui.rs`, `embedded_vnc_types.rs` | vnc-rs embedded viewer |
| `embedded_spice.rs` | SPICE embedded viewer |
| `embedded_trait.rs` | Shared trait for embedded protocol viewers |
| `embedded.rs` | Embedded client coordination |
| `terminal/` | VTE terminal notebook (config, types) |
| `split_view/` | Split view UI (adapter, bridge, manager, types) |
| `session/` | Session UI components |
| `async_utils.rs` | `spawn_async`, `block_on_async`, `with_runtime` — GTK↔tokio bridge |
| `audio.rs` | RDP audio playback via cpal |
| `tray.rs` | System tray icon (ksni) |
| `toast.rs` | Toast notification helpers |
| `alert.rs` | Alert dialog helpers |
| `automation.rs` | GUI automation integration |
| `display.rs` | Display/monitor utilities |
| `external_window.rs` | External window management |
| `error.rs` | GUI error types |
| `utils.rs` | General GUI utilities |
| `validation.rs` | Input validation for dialogs |
| `wayland_surface.rs` | Wayland surface utilities |

## Error Type Hierarchy (`rustconn-core/src/error.rs`)

```
RustConnError (top-level)
├── ConfigError    → ConfigResult<T>
├── ProtocolError  → ProtocolResult<T>
├── SecretError    → SecretResult<T>
├── ImportError    → ImportOperationResult<T>
├── SessionError   → SessionResult<T>
└── Io             (std::io::Error passthrough)
```

Use the domain-specific `Result` alias within its domain. Use `RustConnError` / `Result<T>` at crate API boundaries.

## State Management (`rustconn/src/state.rs`)

```rust
pub type SharedAppState = Rc<RefCell<AppState>>;
```

- Pass `&SharedAppState` to functions needing mutable access.
- Access via helpers: `with_state()`, `with_state_mut()`, `try_with_state()`, `try_with_state_mut()`.
- Manager structs (`ConnectionManager`, `SessionManager`, `SecretManager`, `DocumentManager`, `ClusterManager`, `SnippetManager`) own data and handle I/O.
- `AppState` delegates to managers and provides convenience methods.
- Credential cache uses `CachedCredentials` with TTL-based expiry (default 5 min).

## Async Pattern (GUI ↔ Tokio)

GTK4 is single-threaded. Bridge functions live in `rustconn/src/async_utils.rs`:

| Scenario | Function | Blocks UI? |
|----------|----------|:----------:|
| Short async from GTK callback | `spawn_async()` | No |
| Async with result callback | `spawn_async_with_callback()` | No |
| Need result immediately | `block_on_async()` / `with_runtime()` | Yes |
| With timeout | `block_on_async_with_timeout()` | Yes |

Prefer `spawn_async` over `block_on_async` whenever possible.

## Module Conventions

- Feature directories use `mod.rs` as entry point.
- Public types re-exported through `rustconn-core/src/lib.rs`.
- Split large files by concern: `*_types.rs`, `*_ui.rs`.
- `#![warn(missing_docs)]` is enabled on `rustconn-core` — document all public items with `///`.

## Test Structure

| Type | Location | Entry Point | Registration |
|------|----------|-------------|--------------|
| Property tests | `rustconn-core/tests/properties/*.rs` | `tests/property_tests.rs` | `properties/mod.rs` |
| Integration tests | `rustconn-core/tests/integration/*.rs` | `tests/integration_tests.rs` | `integration/mod.rs` |
| Fixtures | `rustconn-core/tests/fixtures/` | `tests/fixtures/mod.rs` | — |
| CLI tests | `rustconn-cli/tests/` | — | — |
| Benchmarks | `rustconn-core/benches/` | — | — |

Test entry points (`property_tests.rs`, `integration_tests.rs`) carry `#![allow(...)]` blocks for common test patterns — the only acceptable location for broad allows. New test modules must be registered as `mod` entries in the corresponding `mod.rs`.

## Feature Flags

| Flag | Crate | Default | Purpose |
|------|-------|:-------:|---------|
| `vnc-embedded` | `rustconn-core` | Yes | Native VNC via vnc-rs |
| `rdp-embedded` | `rustconn-core` | Yes | Native RDP via IronRDP |
| `spice-embedded` | `rustconn-core` | No | Native SPICE client |
| `tray` | `rustconn` | Yes | System tray (ksni + resvg) |
| `rdp-audio` | `rustconn` | Yes | RDP audio playback (cpal); implies `rdp-embedded` |
| `wayland-native` | `rustconn` | Yes | Wayland surface support (gdk4-wayland) |

Guard feature-gated code with `#[cfg(feature = "...")]`. Conditional re-exports in `lib.rs` use the same guards — check before referencing embedded client types.
