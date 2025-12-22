---
inclusion: always
---

# RustConn Product Context

Linux connection manager for SSH, RDP, VNC, and SPICE remote connections. GTK4/libadwaita GUI targeting Wayland.

## Feature Scope

| Protocol | Implementation | Session Type |
|----------|----------------|--------------|
| SSH | VTE terminal embedding | Embedded tab |
| RDP | FreeRDP via `xfreerdp` | External window |
| VNC | TigerVNC via `vncviewer` | External window |
| SPICE | `remote-viewer` | External window |

Supporting features: connection groups/tags, import/export (Remmina, Asbru-CM, SSH config, Ansible), credential backends (libsecret, KeePassXC), session logging, command snippets, cluster commands, Wake-on-LAN.

## Design Constraints

Apply these rules when implementing or modifying features:

1. **No plaintext credentials** — Wrap secrets in `secrecy::SecretString`; store via `SecretBackend` trait implementations only
2. **Wayland-first** — Use GTK4/libadwaita patterns; avoid X11-specific APIs
3. **Crate boundaries** — `rustconn-core` must not import GTK; all business logic lives there
4. **Trait extension** — New protocols implement `Protocol`; new formats implement `ImportSource`/`ExportTarget`; new credential stores implement `SecretBackend`
5. **Graceful fallback** — Optional integrations (KeePassXC, system tray) must not break core functionality when unavailable

## UI Implementation Rules

- Prefer `adw::` widgets over plain `gtk::` equivalents
- Use `adw::ToastOverlay` for transient status messages
- Modal dialogs via `adw::Dialog` or `gtk::Window` with `set_modal(true)`
- Connection tree in sidebar (`gtk::TreeView`); sessions in `gtk::Notebook` tabs
- Follow GNOME HIG spacing: 12px margins, 6px between related elements

## Error Handling Pattern

```rust
// Define in rustconn_core::error
#[derive(Debug, thiserror::Error)]
pub enum FeatureError {
    #[error("description: {0}")]
    Variant(String),
}

// Return Result from all fallible functions
pub fn do_thing() -> Result<Output, FeatureError> { ... }
```

- GUI: Show user-friendly message via toast or dialog
- Logging: Use `tracing` macros for technical details
- Never panic in library code; reserve `unwrap()`/`expect()` for truly impossible states

## Decision Checklist

Before implementing, verify:
- [ ] Does this belong in `rustconn-core` (logic) or `rustconn` (GUI)?
- [ ] Are credentials handled via `SecretString` and `SecretBackend`?
- [ ] Does the feature degrade gracefully if dependencies are missing?
- [ ] Are errors returned as `Result`, not panics?
- [ ] Does UI follow libadwaita patterns and GNOME HIG?
