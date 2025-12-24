---
inclusion: always
---

# RustConn Product Context

Linux connection manager for SSH, RDP, VNC, and SPICE protocols. GTK4/libadwaita GUI targeting Wayland-first environments.

## Protocol Implementation

| Protocol | Backend | Session Type |
|----------|---------|--------------|
| SSH | VTE terminal | Embedded tab |
| RDP | FreeRDP (`xfreerdp`) | External window |
| VNC | TigerVNC (`vncviewer`) | External window |
| SPICE | `remote-viewer` | External window |

## Core Features

- Connection organization via groups and tags
- Import/export: Remmina, Asbru-CM, SSH config, Ansible inventory
- Credential backends: libsecret, KeePassXC
- Session logging, command snippets, cluster commands, Wake-on-LAN

## Design Rules

When implementing or modifying code, enforce these constraints:

| Rule | Requirement |
|------|-------------|
| Credentials | Wrap in `secrecy::SecretString`; persist via `SecretBackend` trait only |
| Display server | Wayland-first; avoid X11-specific APIs |
| Crate separation | `rustconn-core` must not import `gtk4`, `vte4`, or `adw` |
| Extensibility | New protocols → `Protocol` trait; formats → `ImportSource`/`ExportTarget`; secrets → `SecretBackend` |
| Resilience | Optional features (KeePassXC, tray) must not break core when unavailable |

## UI Patterns

- Prefer `adw::` widgets over `gtk::` equivalents
- Transient messages: `adw::ToastOverlay`
- Modal dialogs: `adw::Dialog` or `gtk::Window` with `set_modal(true)`
- Layout: sidebar `gtk::TreeView` + `gtk::Notebook` session tabs
- Spacing: 12px margins, 6px between related elements (GNOME HIG)

## Error Handling

```rust
// In rustconn_core::error
#[derive(Debug, thiserror::Error)]
pub enum FeatureError {
    #[error("description: {0}")]
    Variant(String),
}

pub fn fallible_op() -> Result<T, FeatureError> { ... }
```

- GUI layer: display user-friendly toast or dialog
- Logging: `tracing` macros for technical details
- No panics in library code; `unwrap()`/`expect()` only for impossible states

## Implementation Checklist

Before writing code, verify:

1. Crate placement: business logic → `rustconn-core`; UI → `rustconn`
2. Secrets use `SecretString` and `SecretBackend`
3. Feature degrades gracefully when dependencies are missing
4. All fallible functions return `Result<T, E>`
5. UI follows libadwaita patterns and GNOME HIG
