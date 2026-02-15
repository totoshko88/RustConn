---
inclusion: always
---

# RustConn Product Rules

Linux connection manager for SSH, RDP, VNC, SPICE, Telnet, Serial, Kubernetes. GTK4/libadwaita GUI, Wayland-first.

## Protocol Strategy

Embedded clients are always preferred. External clients are fallbacks only.

| Protocol | Embedded Client | External Fallback |
|----------|----------------|-------------------|
| SSH | VTE terminal (always) | — |
| RDP | IronRDP | `xfreerdp` |
| VNC | vnc-rs | `vncviewer` |
| SPICE | — (external only) | `remote-viewer` |
| Telnet | VTE terminal | — |
| Serial | VTE terminal | — |
| Kubernetes | VTE terminal (kubectl exec) | — |

Rules for protocol code:
- Check external client availability at runtime before launching
- Always attempt embedded first, fall back to external on failure
- Log every connection attempt and failure via `tracing`

## Error Handling

Two-layer pattern: friendly message to user, full details to log.

`adw::Toast` for recoverable/transient errors (connection timeout, auth failure, missing client). Keep under ~60 characters, make actionable.

`adw::Dialog` (modal) for errors requiring user action (missing config, incompatible settings, destructive confirmations).

Never expose to the user: raw error variants, stack traces, internal file paths, debug output.

```rust
// Always pair user message with structured log
toast_overlay.add_toast(adw::Toast::new("Connection failed. Check your credentials."));
tracing::error!(?error, "SSH connection failed to {}", host);
```

## Graceful Degradation

Optional features must never break core functionality. Degrade silently or show a single toast.

| Feature | Runtime Check | Fallback |
|---------|--------------|----------|
| System tray | `ksni` feature flag | Hide tray menu items |
| KeePassXC | `keepassxc-proxy` on PATH | Use libsecret |
| Embedded RDP | IronRDP connection succeeds | Launch `xfreerdp` |
| Audio playback | `cpal` feature + audio device | Silent mode |

```rust
if feature_available() {
    use_feature();
} else {
    show_toast("Feature unavailable. Using fallback.");
    use_fallback();
}
```

## UI Rules (GNOME HIG)

- Dialogs: `adw::Dialog` or `gtk::Window` with `set_modal(true)`
- Notifications: `adw::ToastOverlay` only (never system notifications)
- Spacing: 12px margins, 6px between related elements
- All new UI must be fully keyboard-navigable
- Wayland-first: never use X11-specific APIs (`gdk_x11_*`)

## Pre-Implementation Checklist

Before writing any user-facing feature, verify all five:

1. What is the fallback if the feature or dependency is unavailable?
2. Are all error messages user-friendly and actionable (no raw errors)?
3. Is the feature fully keyboard-accessible?
4. Does it work on Wayland without X11?
5. Are connection attempts and failures logged via `tracing`?
