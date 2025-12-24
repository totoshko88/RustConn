---
inclusion: always
---

# RustConn Project Structure

## Workspace Architecture

Three-crate Cargo workspace with strict separation:

| Crate | Type | Purpose |
|-------|------|---------|
| `rustconn/` | Binary | GTK4 GUI application |
| `rustconn-core/` | Library | Business logic, models, protocols (GUI-free) |
| `rustconn-cli/` | Binary | CLI for headless operations |

## Critical Crate Boundaries

**NEVER violate these rules:**

- `rustconn-core` MUST NOT import `gtk4`, `vte4`, `adw`, or any GUI crate
- All business logic, data models, and protocol implementations → `rustconn-core`
- GUI-specific code (widgets, dialogs, rendering) → `rustconn`
- Both `rustconn` and `rustconn-cli` depend on `rustconn-core`

## Directory Map

### rustconn/ (GUI Binary)

| Path | Responsibility |
|------|----------------|
| `src/app.rs` | GTK Application, actions, keyboard shortcuts |
| `src/window.rs` | Main window layout, header bar |
| `src/sidebar.rs` | Connection tree view |
| `src/terminal.rs` | VTE terminal notebook for SSH |
| `src/state.rs` | `SharedAppState` = `Rc<RefCell<AppState>>` |
| `src/dialogs/` | Modal dialogs (connection, import, export, settings) |
| `src/session/` | Protocol session widgets (rdp.rs, vnc.rs, spice.rs) |

### rustconn-core/src/ (Library)

| Path | Responsibility |
|------|----------------|
| `lib.rs` | Public API exports |
| `error.rs` | Error types via `thiserror` |
| `models/` | Connection, Group, Protocol, Snippet, Template |
| `config/` | Settings persistence |
| `connection/` | Connection CRUD operations |
| `protocol/` | Protocol trait + implementations |
| `import/` | Import: ssh_config, remmina, asbru, ansible |
| `export/` | Export: ssh_config, remmina, asbru, ansible |
| `secret/` | Credential backends: libsecret, keepassxc, kdbx |
| `session/` | Session state, logging |
| `automation/` | Expect scripts, key sequences, tasks |
| `cluster/` | Multi-host command execution |
| `variables/` | Variable substitution engine |
| `search/` | Connection search/filtering |
| `wol/` | Wake-on-LAN |

### rustconn-core/tests/

| Path | Purpose |
|------|---------|
| `properties/` | Property-based tests (`proptest`) |
| `integration/` | Integration tests |
| `fixtures/` | Test data files |

## Extension Points

When adding new functionality, implement these traits:

| Feature | Trait | Location |
|---------|-------|----------|
| Protocol | `Protocol` | `rustconn-core/src/protocol/` |
| Import format | `ImportSource` | `rustconn-core/src/import/` |
| Export format | `ExportTarget` | `rustconn-core/src/export/` |
| Credential backend | `SecretBackend` | `rustconn-core/src/secret/` |

## State Management

- GUI state: `SharedAppState` = `Rc<RefCell<AppState>>` (interior mutability pattern)
- Persistence: Manager structs (`ConfigManager`, `ConnectionManager`) own data and handle I/O

## Module Conventions

- Feature directories contain `mod.rs` for module organization
- Public types re-exported through `lib.rs`
- Test modules mirror source structure in `tests/properties/`

## File Placement Decision Tree

| Adding... | Location | Required Steps |
|-----------|----------|----------------|
| Data model | `rustconn-core/src/models/` | Re-export in `models.rs` |
| Protocol | `rustconn-core/src/protocol/` | Implement `Protocol` trait |
| Import format | `rustconn-core/src/import/` | Implement `ImportSource` trait |
| Export format | `rustconn-core/src/export/` | Implement `ExportTarget` trait |
| Dialog | `rustconn/src/dialogs/` | Register in `mod.rs` |
| Property tests | `rustconn-core/tests/properties/` | Add module to `mod.rs` |
