---
inclusion: fileMatch
fileMatchPattern: "rustconn-core/src/protocol/**/*.rs"
---

# Protocols — Development Rules

You are editing a file in `rustconn-core/src/protocol/`.

## Crate Boundary

This crate is GUI-free. FORBIDDEN: `gtk4`, `adw`, `vte4`, `libadwaita`.
Keep protocol model/validation/argument-building in core. Put embedded runtime
clients behind explicit core features, CLI launch behavior behind CLI features,
and GTK dialogs/session presentation in `rustconn/`.

## New Protocol — Checklist

1. Add variant to `ProtocolType` enum
2. Define `capabilities` (has_terminal, has_password, has_port, etc.)
3. Set `default_port`
4. Implement `Protocol` trait
5. Add headless CLI CRUD flags in `rustconn-cli`; gate launch-only behavior if needed
6. Add dialog/session UI in `rustconn/src/dialogs/connection/` and `rustconn/src/window/`
7. Update sidebar filter icons if needed

## Errors

Use `thiserror::Error` for all protocol error types.
`Result<T, ProtocolError>` for fallible operations.
