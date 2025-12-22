---
inclusion: always
---

# RustConn Project Structure

## Workspace Layout

Three-crate Cargo workspace with strict separation of concerns:

| Crate | Type | Purpose |
|-------|------|---------|
| `rustconn/` | Binary | GTK4 GUI application |
| `rustconn-core/` | Library | Business logic, models, protocols (GUI-free) |
| `rustconn-cli/` | Binary | CLI for headless operations |

## Crate Boundaries (Critical)

- `rustconn-core` MUST NOT import `gtk4`, `vte4`, `adw`, or any GUI crate
- `rustconn` and `rustconn-cli` depend on `rustconn-core`
- All business logic, data models, and protocol implementations belong in `rustconn-core`
- GUI-specific code (widgets, dialogs, rendering) belongs in `rustconn`

## Key Directories

### rustconn/ (GUI)

| File/Directory | Responsibility |
|----------------|----------------|
| `src/app.rs` | GTK Application, actions, keyboard shortcuts |
| `src/window.rs` | Main window layout, header bar |
| `src/sidebar.rs` | Connection tree view |
| `src/terminal.rs` | VTE terminal notebook for SSH |
| `src/state.rs` | `SharedAppState` (`Rc<RefCell<AppState>>`) |
| `src/dialogs/` | Modal dialogs (connection, import, export, settings) |
| `src/session/` | Protocol-specific session widgets (rdp.rs, vnc.rs, spice.rs) |

### rustconn-core/src/

| Directory | Responsibility |
|-----------|----------------|
| `lib.rs` | Public API exports |
| `error.rs` | Error types via `thiserror` |
| `models/` | Data structures: Connection, Group, Protocol, Snippet, Template |
| `config/` | Settings persistence (manager.rs, settings.rs) |
| `connection/` | Connection CRUD operations |
| `protocol/` | Protocol trait + implementations (ssh, rdp, vnc, spice) |
| `import/` | Import sources: ssh_config, remmina, asbru, ansible |
| `export/` | Export targets: ssh_config, remmina, asbru, ansible |
| `secret/` | Credential backends: libsecret, keepassxc, kdbx |
| `session/` | Session state, logging |
| `automation/` | Expect scripts, key sequences, tasks |
| `cluster/` | Multi-host command execution |
| `variables/` | Variable substitution engine |
| `search/` | Connection search/filtering |
| `wol/` | Wake-on-LAN |

### rustconn-core/tests/

| Directory | Purpose |
|-----------|---------|
| `properties/` | Property-based tests using `proptest` |
| `integration/` | Integration tests |
| `fixtures/` | Test data files (ssh_config, remmina, asbru, ansible) |

## Trait-Based Extension Points

Implement these traits when adding new functionality:

| Adding | Trait to Implement | Location |
|--------|-------------------|----------|
| New protocol | `Protocol` | `rustconn-core/src/protocol/` |
| New import format | `ImportSource` | `rustconn-core/src/import/` |
| New export format | `ExportTarget` | `rustconn-core/src/export/` |
| New credential backend | `SecretBackend` | `rustconn-core/src/secret/` |

## Error Handling Pattern

```rust
// Define domain error enum in appropriate module
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("description: {0}")]
    Variant(String),
}

// Type alias for convenience
pub type ConfigResult<T> = Result<T, ConfigError>;
```

## State Management

- GUI state: `SharedAppState` = `Rc<RefCell<AppState>>` (interior mutability)
- Persistence: Manager structs (`ConfigManager`, `ConnectionManager`, etc.)
- Managers own data and handle file I/O

## Module Organization

- Each feature area has its own directory with `mod.rs`
- Public types re-exported through `lib.rs`
- Tests mirror source structure in `tests/properties/`

## File Placement Quick Reference

| Adding... | Location | Additional Steps |
|-----------|----------|------------------|
| Data model | `rustconn-core/src/models/` | Add to `models.rs` re-exports |
| Protocol | `rustconn-core/src/protocol/` | Implement `Protocol` trait |
| Import format | `rustconn-core/src/import/` | Implement `ImportSource` trait |
| Export format | `rustconn-core/src/export/` | Implement `ExportTarget` trait |
| Dialog | `rustconn/src/dialogs/` | Register in `mod.rs` |
| Property tests | `rustconn-core/tests/properties/` | Add module to `mod.rs` |
