# rustconn-lock-sys

Sanctioned FFI crate (M-UNSAFE). It observes the two macOS notifications that
stand in for the D-Bus signals `rustconn-core::lock_events` listens to on Linux:
`NSWorkspaceWillSleepNotification` on `NSWorkspace.sharedWorkspace`'s own
notification centre, and the `com.apple.screenIsLocked` distributed notification.
The GUI maps each `LockSignal` onto a `LockEvent` and hands it to
`lock_events::notify`, so the own-key eviction and any other subscriber run
unchanged. This crate knows nothing about SSH keys or about `rustconn-core`, and
`rustconn-core` does not depend on it.

One function, `observe`, one guard, `LockObservers`, one outcome type,
`ObserveOutcome`.

- **Same shape as `rustconn-dock-sys`: dependencies target-gated, the crate not.**
  `objc2`, `objc2-app-kit` and `objc2-foundation` sit under
  `[target.'cfg(target_os = "macos")'.dependencies]` because `objc2-app-kit` does
  not compile off Apple platforms. The crate, its API, its panic barrier and their
  tests build and run everywhere; the `expect(unsafe_code)` is wrapped in
  `cfg_attr(target_os = "macos", …)` because off macOS there is no `unsafe` here.
- **`DeliverImmediately` on the distributed centre is load-bearing.** AppKit
  suspends distributed notifications while the app is inactive; with the default
  suspension behaviour a lock that happens with RustConn in the background is held
  back until the user returns to it, after unlocking. Do not switch to the
  block-based `addObserverForName:…` API, which has no suspension parameter.
- **The observers live as long as `LockObservers`.** Its `Drop` removes them from
  both centres. The GUI keeps it until `shutdown`.
- **The handler is `Send + Sync` and must return quickly.** It runs on the main
  run loop, which is the GTK main loop. The GUI's handler only queues the event;
  `lock_events::notify` runs the subscribers on its own dispatch thread.
- **Every observer method goes through `deliver`**, which stops a panic before it
  can unwind into Foundation's notification posting.
- Main thread is proved by `objc2::MainThreadMarker::new()`. A wrong thread is an
  outcome (`OffMainThread`), not a panic: a missed lock leaves the keys to their
  exit and 12-hour bounds.
- `NSWorkspaceScreensDidSleepNotification` is not observed: display sleep is not a
  lock, and when it is followed by one, macOS posts `com.apple.screenIsLocked` too.
- Verify the gated path on Linux with
  `cargo clippy -p rustconn-lock-sys --target aarch64-apple-darwin -- -D warnings`.
  The objc2 bindings are pure Rust, so `check`/`clippy` need no SDK; linking and
  running the tests do, which is what the `macos-sys` CI job and a Mac are for.
