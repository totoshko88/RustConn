---
inclusion: always
---

# RustConn Product Context

Linux connection manager for SSH, RDP, VNC, and SPICE protocols. GTK4/libadwaita GUI targeting Wayland-first environments.

## Protocol Architecture

| Protocol | Backend | Session Type | Notes |
|----------|---------|--------------|-------|
| SSH | VTE terminal | Embedded tab | Primary use case |
| RDP | FreeRDP (`xfreerdp`) | External window | Requires FreeRDP installed |
| VNC | TigerVNC (`vncviewer`) | External window | Requires TigerVNC installed |
| SPICE | `remote-viewer` | External window | For VM connections |

## Core Features

- Connection organization via groups and tags
- Import/export: Remmina, Asbru-CM, SSH config, Ansible inventory, Royal TS
- Credential backends: libsecret (default), KeePassXC (optional)
- Session logging, command snippets, cluster commands, Wake-on-LAN

## Mandatory Constraints

These rules apply to ALL code changes:

### Credentials
- MUST wrap in `secrecy::SecretString`
- MUST persist via `SecretBackend` trait only
- NEVER store passwords as plain `String`

### Display Server
- Wayland-first design
- AVOID X11-specific APIs
- Test on Wayland before X11

### Crate Boundaries
- `rustconn-core` MUST NOT import `gtk4`, `vte4`, or `adw`
- Business logic belongs in `rustconn-core`
- GUI code belongs in `rustconn`

### Extensibility Patterns
- New protocols → implement `Protocol` trait
- New import formats → implement `ImportSource` trait
- New export formats → implement `ExportTarget` trait
- New secret backends → implement `SecretBackend` trait

### Graceful Degradation
- Optional features (KeePassXC, tray icon) MUST NOT break core functionality when unavailable
- Check feature availability at runtime, not compile time where possible

## UI Implementation Rules

When writing GUI code in `rustconn/`:

| Pattern | Implementation |
|---------|----------------|
| Widget preference | `adw::` over `gtk::` equivalents |
| Transient messages | `adw::ToastOverlay` |
| Modal dialogs | `adw::Dialog` or `gtk::Window` with `set_modal(true)` |
| Main layout | Sidebar `gtk::TreeView` + `gtk::Notebook` session tabs |
| Spacing | 12px margins, 6px between related elements (GNOME HIG) |

## Error Handling

### In `rustconn-core`
- Define errors using `thiserror`
- Return `Result<T, E>` from all fallible functions
- NO panics; `unwrap()`/`expect()` only for provably impossible states

```rust
#[derive(Debug, thiserror::Error)]
pub enum FeatureError {
    #[error("description: {0}")]
    Variant(String),
}
```

### In `rustconn` (GUI)
- Display user-friendly toast or dialog for errors
- Log technical details via `tracing`
- Never expose internal error messages to users

## Pre-Implementation Checklist

Before writing code, verify:

1. **Crate placement** — Business logic → `rustconn-core`; UI → `rustconn`
2. **Secrets** — Wrapped in `SecretString`, persisted via `SecretBackend`
3. **Degradation** — Feature works when optional dependencies missing
4. **Error handling** — All fallible functions return `Result<T, E>`
5. **UI patterns** — Follows libadwaita patterns and GNOME HIG
