# rustconn (GUI)

GTK4/libadwaita presentation: windows, dialogs, embedded and external session
handling. Root `AGENTS.md` still applies. Detailed guides load automatically when
you open the matching files — `gnome-hig.md`, `window-guide.md`,
`dialogs-guide.md`. This file is the part that is easy to get wrong from the root
rules alone.

- **`unsafe_code = "forbid"` locally**, stricter than the workspace `deny`. A
  crate-level `#![expect(unsafe_code)]` cannot re-open it here, by design: if
  something needs FFI, it goes in a new `rustconn-*-sys` crate. The local
  `[lints]` table also spells out the full clippy set, because a crate-local table
  replaces the inherited one — when adding a lint, add it here, not only to the
  workspace.
- Every user-visible string goes through `i18n()` / `i18n_f()` from
  `crate::i18n`, with `{}` placeholders. A `display_name()` coming out of
  `rustconn-core` is untranslated on purpose — wrap it at the call site, here.
- A new file containing `i18n()` must appear in `po/POTFILES.in` or
  `po/update-pot.sh` silently drops its strings. The `translation-sync` hook
  notes a missing entry on save (it does not add it);
  `./scripts/check-potfiles.sh` is the CI gate.
- Icon-only buttons need **both** a tooltip and an accessible label. One is not
  the other, and a screen reader only sees the second.
- `adw::` before `gtk::` where both exist. `adw::AlertDialog`, never the
  deprecated `gtk::MessageDialog`.
- GTK objects are not `Send`, so `tokio::spawn` will not compile against them.
  The house helpers are in `src/async_utils.rs` — `spawn_async`,
  `spawn_async_with_callback`, `with_runtime`, `ensure_main_thread`; raw
  `glib::spawn_future_local` is used where no helper fits. Clone the `Rc` before
  the closure rather than holding a `Ref`/`RefMut` across an `.await`.
- "Cannot start a runtime from within a runtime" means a nested block-on. Go
  through `with_runtime()`, which owns the thread-local runtime.
- `BorrowMutError` at runtime means a nested `borrow_mut()`, not a race. The fix
  is take-invoke-restore: `take()` the field, call out, put it back — see
  `error-resolution.md`.
