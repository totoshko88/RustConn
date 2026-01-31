---
inclusion: always
---

# RustConn Product Context

Linux connection manager for SSH, RDP, VNC, SPICE protocols. GTK4/libadwaita GUI, Wayland-first.

## Protocol Architecture

| Protocol | Backend | Session Type |
|----------|---------|--------------|
| SSH | VTE terminal | Embedded tab (primary) |
| RDP | IronRDP (embedded) or FreeRDP (`xfreerdp`) | Embedded tab or external window |
| VNC | vnc-rs (embedded) or TigerVNC (`vncviewer`) | Embedded tab or external window |
| SPICE | `remote-viewer` | External window |

## Core Capabilities

- Connection organization via groups and tags
- Import/export: Remmina, Asbru-CM, SSH config, Ansible inventory, Royal TS, MobaXterm
- Credential backends: libsecret (default), KeePassXC, Bitwarden, 1Password (optional)
- Session logging, command snippets, cluster commands, Wake-on-LAN
- Split view for multiple simultaneous sessions
- Variables and templates for connection configuration

## Critical Rules

### Credential Security

All passwords and keys MUST use `SecretString`:

```rust
use secrecy::SecretString;
let password: SecretString = SecretString::new(value.into());
```

Persist credentials via `SecretBackend` trait only. Never store as plain `String`.

### Crate Boundaries

| Code Type | Crate | Constraint |
|-----------|-------|------------|
| Business logic | `rustconn-core` | NO `gtk4`/`vte4`/`adw` imports |
| GUI/widgets | `rustconn` | Depends on `rustconn-core` |
| CLI | `rustconn-cli` | Depends on `rustconn-core` only |

Decision: "Does this need GTK?" → No: `rustconn-core` / Yes: `rustconn`

### Display Server

- Wayland-first — avoid X11-specific APIs
- Test Wayland before X11

### Graceful Degradation

Optional features (KeePassXC, tray icon) must not break core functionality. Check availability at runtime.

## Extensibility

| Feature | Trait | Location |
|---------|-------|----------|
| Protocol | `Protocol` | `rustconn-core/src/protocol/` |
| Import format | `ImportSource` | `rustconn-core/src/import/` |
| Export format | `ExportTarget` | `rustconn-core/src/export/` |
| Secret backend | `SecretBackend` | `rustconn-core/src/secret/` |

## UI Patterns (`rustconn/`)

| Pattern | Implementation |
|---------|----------------|
| Widgets | Prefer `adw::` over `gtk::` equivalents |
| Toasts | `adw::ToastOverlay` |
| Dialogs | `adw::Dialog` or `gtk::Window` with `set_modal(true)` |
| Layout | Sidebar `gtk::ListView` + `gtk::Notebook` tabs |
| Spacing | 12px margins, 6px between related elements (GNOME HIG) |

## Error Handling

### `rustconn-core`

```rust
#[derive(Debug, thiserror::Error)]
pub enum FeatureError {
    #[error("description: {0}")]
    Variant(String),
}
```

- Return `Result<T, E>` from fallible functions
- No panics; `unwrap()`/`expect()` only for provably impossible states

### `rustconn` (GUI)

- Show user-friendly toast/dialog for errors
- Log technical details via `tracing`
- Never expose internal error messages to users

## Pre-Implementation Checklist

1. **Crate placement** — Business logic → `rustconn-core`; UI → `rustconn`
2. **Secrets** — Wrapped in `SecretString`, persisted via `SecretBackend`
3. **Degradation** — Feature works when optional dependencies missing
4. **Errors** — All fallible functions return `Result<T, E>`
5. **UI** — Follows libadwaita patterns and GNOME HIG
