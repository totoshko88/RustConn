# rustconn-dock-sys

Sanctioned FFI crate (M-UNSAFE). It wraps exactly one call:
`-[NSApplication setApplicationIconImage:]`, the only interface that changes the
macOS Dock tile of a *running* process. A `rustconn` started from a shell has no
bundle behind it, so LaunchServices has no `CFBundleIconFile` to read and the Dock
falls back to the generic Unix-executable tile. Nothing in GTK can fix that:
`gtk_window_set_icon_name` is an X11/Wayland concept and a no-op on the GDK macOS
backend.

One function, `set_dock_icon_png`, and one result type, `DockIconOutcome`.

This crate is the workspace's one deliberate exception to a rule, so read the next
two points before changing anything.

- **Its dependencies are target-gated, the crate is not.** `objc2`,
  `objc2-app-kit` and `objc2-foundation` live under
  `[target.'cfg(target_os = "macos")'.dependencies]` because `objc2-app-kit` does
  not compile off Apple platforms — there is no cross-platform stand-in for AppKit
  the way `std::env::set_var` is one for `setenv`. The crate itself, its API and its
  precondition guard are built and tested everywhere. Do not generalise this shape
  to a new `-sys` crate unless its bindings cannot build off Apple either
  (`rustconn-lock-sys` is the one other case). The `macos-sys` CI job covers the helpers,
  but every other job is Linux, so a macOS-gated *crate* would still be `unsafe`
  that only one job in the matrix ever compiles — and the crate-level guard, the
  API and the contract tests are what the other jobs are there to check.
- **The `expect` is wrapped too**: `#![cfg_attr(target_os = "macos", expect(unsafe_code, …))]`.
  Off macOS the crate contains no `unsafe`, so a bare `#![expect(unsafe_code)]`
  would fire `unfulfilled_lint_expectations` and break the zero-warning gate on
  Linux.

The rest:

- Main-thread access is proved by `objc2::MainThreadMarker::new()`, which asks the
  runtime instead of trusting the caller. A wrong thread is an **outcome**, not a
  panic — a wrong Dock tile is cosmetic and there is nothing for a caller to
  recover.
- `DockIconOutcome` instead of `Result` for the same reason. Do not "improve" it
  into an error type; a refusal is something to log.
- Scope is the tile image only. The name under the tile, the Cmd-Tab entry and the
  application menu come from the bundle and are not reachable this way. A correct
  `.app` is still the only way to get all of them, which is why the caller skips
  this crate entirely when it is already running inside one.
- Verify the gated path on Linux with
  `cargo clippy -p rustconn-dock-sys --target aarch64-apple-darwin` — the target is
  installed, and the objc2 bindings are pure Rust, so `check`/`clippy` work without
  an SDK. Only linking needs one.
