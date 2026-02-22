---
inclusion: always
---

# RustConn Product Rules

Linux connection manager for SSH, RDP, VNC, SPICE, Telnet, Serial, Kubernetes.
GTK4/libadwaita GUI, Wayland-first, GNOME HIG compliant.

## Protocol Strategy

Embedded clients are preferred. External clients are fallbacks only.

| Protocol | Embedded Client | External Fallback | Feature Gate |
|----------|----------------|-------------------|--------------|
| SSH | VTE terminal | — | — |
| RDP | IronRDP | `xfreerdp` | `rdp-embedded` |
| VNC | vnc-rs | `vncviewer` | `vnc-embedded` |
| SPICE | — | `remote-viewer` | `spice-embedded` |
| Telnet | VTE terminal | — | — |
| Serial | VTE terminal | — | — |
| Kubernetes | VTE terminal (`kubectl exec`) | — | — |

When implementing or modifying protocol connection code, follow this order:

1. Guard embedded path with `#[cfg(feature = "...")]` where applicable.
2. Attempt embedded client first.
3. On embedded failure, check external client availability on `PATH` before launching.
4. Log every attempt and outcome with `tracing` (structured fields: protocol, host, port, error).
5. Surface a user-friendly toast or dialog on failure — never raw errors.

## Connection Lifecycle

Every connection must pass through these stages. Log transitions via `tracing::info!` / `tracing::error!`.

```
Idle → Connecting → Authenticating → Connected → Disconnecting → Disconnected
                 ↘ Failed (log + toast/dialog)
```

- On `Connecting`: validate host/port, resolve credentials from `SecretManager`.
- On `Authenticating`: use `SecretString` — never log or display credential values.
- On `Failed`: map the internal error to a user-facing message (see Error Handling below).
- On `Disconnecting`: clean up session resources, flush session log if logging is enabled.

## Error Handling

Two-layer pattern: friendly message to user, full details to structured log.

### Choosing the right surface

| Condition | Surface | Example |
|-----------|---------|---------|
| Recoverable / transient | `adw::Toast` | Connection timeout, auth failure, missing client |
| Requires user decision | `adw::Dialog` (modal) | Missing config, destructive action confirmation |
| Background operation failure | `tracing` only | Periodic keep-alive failure, non-critical sync |

### Toast message rules

- Maximum ~60 characters.
- Use imperative, actionable language: "Check credentials" not "Credentials may be wrong".
- Never include: raw error variants, stack traces, internal paths, debug identifiers.

### Pattern

```rust
// Always pair user message with structured log
toast_overlay.add_toast(adw::Toast::new("Connection failed. Check your credentials."));
tracing::error!(?error, protocol = "ssh", host = %host, "Connection failed");
```

## Graceful Degradation

Optional features must never break core functionality. Degrade silently or show a single toast.

| Feature | Runtime Check | Fallback |
|---------|--------------|----------|
| System tray | `tray` feature flag | Hide tray-related menu items |
| KeePassXC | `keepassxc-proxy` on `PATH` | Fall back to libsecret |
| Embedded RDP | IronRDP connection attempt | Launch `xfreerdp` |
| Embedded VNC | vnc-rs connection attempt | Launch `vncviewer` |
| Audio playback | `rdp-audio` feature + audio device | Silent mode |

When adding a new optional feature:

1. Define a compile-time feature gate or runtime availability check.
2. Implement the fallback path.
3. Show at most one toast when degrading — no repeated warnings.
4. Never `panic!` or `unwrap()` on a missing optional dependency.

## UI Rules (GNOME HIG)

### Widget selection

- Prefer `adw::` widgets over `gtk::` equivalents in all cases.
- Dialogs: use `adw::Dialog` (preferred) or `gtk::Window` with `set_modal(true)`.
- Notifications: `adw::ToastOverlay` only — never system notifications.
- Header bars: `adw::HeaderBar`, not `gtk::HeaderBar`.

### Layout

- Margins: 12px around content areas.
- Spacing: 6px between related elements.
- Use `adw::PreferencesGroup` / `adw::PreferencesRow` for settings-style layouts.

### Accessibility and input

- All new UI must be fully keyboard-navigable (tab order, Enter/Escape handling).
- Set `accessible-role`, `tooltip-text`, and mnemonic labels on interactive widgets.
- Wayland-first: never use X11-specific APIs (`gdk_x11_*`, `XSetInputFocus`, etc.).

## Internationalization (i18n)

- All user-visible strings must be wrapped with `gettext` / `ngettext` for translation.
- String keys go in `po/POTFILES.in`. Run `po/update-pot.sh` after adding new translatable strings.
- Never concatenate translated fragments — use format strings with positional placeholders.
- Toast messages and dialog labels are translatable; log messages are English-only.

## Pre-Implementation Checklist

Before writing any user-facing feature, verify all six:

1. Is there a fallback if the feature or dependency is unavailable?
2. Are all error messages user-friendly, actionable, and translatable?
3. Is the feature fully keyboard-accessible?
4. Does it work on Wayland without X11-specific APIs?
5. Are connection attempts and failures logged via `tracing` with structured fields?
6. Are user-visible strings wrapped for translation (`gettext`)?
