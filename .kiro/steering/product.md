---
inclusion: always
---

# RustConn Product Guidelines

Linux connection manager for SSH, RDP, VNC, SPICE. GTK4/libadwaita GUI, Wayland-first.

## Protocol Behavior

| Protocol | Embedded | External Fallback | Notes |
|----------|----------|-------------------|-------|
| SSH | VTE terminal | — | Always embedded |
| RDP | IronRDP | `xfreerdp` | Fall back if IronRDP fails |
| VNC | vnc-rs | `vncviewer` | Fall back if vnc-rs fails |
| SPICE | — | `remote-viewer` | External only |

When implementing protocol features:
- Embedded clients are preferred; external fallback is for compatibility
- Check client availability at runtime before attempting connection
- Log connection attempts and failures via `tracing`

## User-Facing Error Handling

GUI errors must be user-friendly:
- Show `adw::Toast` for transient errors (connection timeout, auth failure)
- Show modal `adw::Dialog` for blocking errors requiring user action
- Never display raw error messages, stack traces, or internal paths
- Log full technical details via `tracing` for debugging

Example toast pattern:
```rust
toast_overlay.add_toast(adw::Toast::new("Connection failed. Check your credentials."));
tracing::error!(?error, "SSH connection failed to {}", host);
```

## UI Conventions (GNOME HIG)

- Prefer `adw::` widgets over `gtk::` equivalents
- Dialogs: `adw::Dialog` or `gtk::Window` with `set_modal(true)`
- Notifications: `adw::ToastOverlay` (not system notifications)
- Spacing: 12px margins, 6px between related elements
- Wayland-first: avoid X11-specific APIs (`gdk_x11_*`)

## Graceful Degradation

Optional features must not break core functionality:

| Feature | Check | Fallback |
|---------|-------|----------|
| System tray | `ksni` feature enabled | Hide tray menu items |
| KeePassXC | `keepassxc-proxy` available | Use libsecret |
| Embedded RDP | IronRDP connection succeeds | Launch `xfreerdp` |
| Audio playback | `cpal` feature + device available | Silent mode |

Pattern:
```rust
if feature_available() {
    use_feature();
} else {
    show_toast("Feature unavailable. Using fallback.");
    use_fallback();
}
```

## Extensibility Traits

New features should implement existing traits when applicable:

| Adding | Implement | Example |
|--------|-----------|---------|
| New protocol | `Protocol` | Telnet support |
| Import format | `ImportSource` | PuTTY sessions |
| Export format | `ExportTarget` | CSV export |
| Secret backend | `SecretBackend` | HashiCorp Vault |

## Pre-Implementation Checklist

Before writing code:

1. **User impact** — How does this affect the user experience?
2. **Fallback behavior** — What happens if the feature is unavailable?
3. **Error messages** — Are they user-friendly and actionable?
4. **Accessibility** — Can keyboard-only users access this feature?
5. **Wayland compatibility** — Does this work without X11?
