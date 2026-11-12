# rustconn-pty-sys

Sanctioned FFI crate (M-UNSAFE). Isolated `libc` helpers for the PTY layer:
`setsid` + `TIOCSCTTY` in a `pre_exec` hook, `openpty(2)` (via `nix`), `TIOCSWINSZ`,
`poll(2)` with a timeout, and a close-on-exec `dup`.

**`unsafe` is allowed here and nowhere else in the workspace except the four
sibling `-sys` crates.** It is re-opened by the crate-level
`#![expect(unsafe_code, reason = "…")]` in `lib.rs` — `expect`, not `allow`, so
that if the last `unsafe` block ever goes away the compiler says so, and the crate
gets folded back into its caller instead of lingering as an empty exemption.

Rules for editing this crate:

- Every `unsafe` block carries a `// SAFETY:` comment naming the invariant it
  establishes, not asserting that one exists. `undocumented_unsafe_blocks` is on.
- One unsafe operation per block. `multiple_unsafe_ops_per_block` is on, and it
  has a trap: an `unsafe` block extends lexically **into a closure body**, so an
  inline closure passed to an unsafe registrar puts its own unsafe calls in the
  outer block. Bind the closure with `let` first, then call the registrar in a
  minimal block — `set_controlling_terminal` is the worked example.
- `libc` and `nix` (`term` feature, for `nix::pty::openpty` only) are the only
  dependencies. Keep it that way.
- No panics. This code runs in `pre_exec`, between `fork` and `exec`, where the
  child holds a copy of every lock in the parent — a panic there is not a
  recoverable error, it is a wedged child. Return `io::Error`.
- Reading and writing the PTY is deliberately **not** here. The caller turns the
  master into a `std::fs::File`, so no session data passes through `unsafe`. Do
  not add a read/write helper "for symmetry".
- Miri cannot run these syscalls. Verification is a contract test asserting the
  observable precondition or behaviour, not a Miri job.
