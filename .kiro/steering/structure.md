---
inclusion: always
---

# RustConn Project Structure

Three-crate Cargo workspace. Strict GUI/logic separation is enforced.

## Crate Boundaries

| Crate | Type | Constraint |
|-------|------|------------|
| `rustconn/` | Binary | GUI only — may import `gtk4`, `vte4`, `adw` |
| `rustconn-core/` | Library | GUI-free — NEVER import `gtk4`, `vte4`, `adw` |
| `rustconn-cli/` | Binary | CLI — depends on `rustconn-core` only |

**Decision rule:** "Does this need GTK?" → No → `rustconn-core` / Yes → `rustconn`

## Where to Add New Code

| Feature Type | Location | Required Action |
|--------------|----------|-----------------|
| Data model | `rustconn-core/src/models/` | Re-export in `models.rs` |
| Protocol impl | `rustconn-core/src/protocol/` | Implement `Protocol` trait |
| Import format | `rustconn-core/src/import/` | Implement `ImportSource` trait |
| Export format | `rustconn-core/src/export/` | Implement `ExportTarget` trait |
| Secret backend | `rustconn-core/src/secret/` | Implement `SecretBackend` trait |
| Dialog | `rustconn/src/dialogs/` | Register in `dialogs/mod.rs` |
| Property test | `rustconn-core/tests/properties/` | Register in `properties/mod.rs` |

## GUI Crate (`rustconn/src/`)

| File Pattern | Purpose |
|--------------|---------|
| `app.rs` | GTK Application, global actions, keyboard shortcuts |
| `window.rs` | Main window layout, header bar |
| `window_*.rs` | Window functionality split by domain |
| `sidebar.rs`, `sidebar_ui.rs`, `sidebar_types.rs` | Connection tree (logic/widgets/types) |
| `state.rs` | `SharedAppState` = `Rc<RefCell<AppState>>` |
| `dialogs/` | Modal dialogs |
| `embedded_*.rs` | Embedded protocol viewers (RDP, VNC, SPICE) |
| `terminal/` | VTE terminal notebook for SSH |

## Core Library (`rustconn-core/src/`)

| Directory | Purpose |
|-----------|---------|
| `models/` | Connection, Group, Protocol, Snippet, Template |
| `config/` | Settings persistence |
| `connection/` | Connection CRUD, lazy loading, virtual scroll |
| `protocol/` | Protocol trait + implementations |
| `import/` | Format importers |
| `export/` | Format exporters |
| `secret/` | Credential backends |
| `session/` | Session state, logging |
| `automation/` | Expect scripts, key sequences |
| `search/` | Connection filtering with caching |

## State Management Pattern

```rust
pub type SharedAppState = Rc<RefCell<AppState>>;
```

- Pass `&SharedAppState` to functions needing mutable access
- Manager structs own data and handle I/O
- Async: use thread-local tokio runtime via `with_runtime()`

## Module Conventions

- Feature directories use `mod.rs`
- Public types re-exported through `lib.rs`
- Split large files: `*_types.rs`, `*_ui.rs`

## Test Locations

| Type | Location | Registration |
|------|----------|--------------|
| Property tests | `rustconn-core/tests/properties/` | `properties/mod.rs` |
| Integration tests | `rustconn-core/tests/integration/` | `integration/mod.rs` |
| Fixtures | `rustconn-core/tests/fixtures/` | — |
