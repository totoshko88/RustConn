# Tasks: Shell Integration (OSC 133)

## Task 1: MarkRegistry core (unit-testable, no GTK)

- [x] 1.1 Create `rustconn/src/terminal/mark_registry.rs` with `PromptMark { row: i64, exit_code: Option<u64> }` and `MarkRegistry { marks: Vec<PromptMark> }`
- [x] 1.2 Implement `record_prompt(row)`, `set_last_exit(code)`, `trim_below(oldest_row)` (prefix drop), `prev_before(row)` / `next_after(row)` (binary search), `failed_rows()` iterator
- [x] 1.3 Keep the struct free of GTK types so it compiles and tests without a display; register the module in `rustconn/src/terminal/mod.rs`
- [x] 1.4 Unit tests: record ordering, trim invariants (no mark below floor), prev/next nearest-in-direction, no-mark-in-direction returns None, failed_rows reflects only non-zero exit codes (12 tests, all passing; clippy pedantic/nursery clean)

## Task 2: VTE wiring in termprops.rs (feature = vte-0-78)

- [x] 2.1 Add `connect_shell_integration(terminal, registry)` under `#[cfg(feature = "vte-0-78")]`: capture the cursor row at `postexec` (vte4 0.10 binds only `postexec`), convert to a buffer row via `vadjustment().value()`, call `record_prompt`
- [x] 2.2 Attach the exit code in the same handler via `registry.set_last_exit(termprop_uint)` (the separate `connect_command_finished` stays for the activity-monitor caller)
- [x] 2.3 Add the `#[cfg(not(feature = "vte-0-78"))]` twin returning `false` and connecting nothing
- [x] 2.4 Updated doc comments on both twins describing the `postexec`-anchored approach and the remote `vte.sh` precondition
- [ ] 2.5 Call `trim_below` from the terminal's scroll / contents-changed path so stored rows never outlive the buffer *(deferred to the Task 5 wiring, where the registry is owned by the session and the signal is connected)*
- [x] 2.6 Verified clippy clean (pedantic/nursery, all-targets) under both the default feature set (no-op twin) and `--features vte-0-78` (real path)

## Task 3: Keybindings

- [x] 3.1 Added `win.prompt-prev` (`<Control><Shift>bracketleft`) and `win.prompt-next` (`<Control><Shift>bracketright`) to `default_keybindings()`, category Terminal
- [x] 3.2 Confirmed free against the full registry and fixed widget shortcuts; two-modifier so they survive `suspend_terminal_accels` (issue #216); core keybinding unit tests pass (unique actions, valid accels)
- [x] 3.3 Registry-driven: startup `restore_accels` and the Settings recorder both iterate `default_keybindings()`, so no extra wiring. Added the labels to `shortcuts.rs` `i18n_markers` and the test `EXPECTED_ACCELS`; shortcuts drift tests pass

## Task 4: Navigation actions

- [x] 4.1 Registered `win.prompt-prev` / `win.prompt-next` `SimpleAction`s in `rustconn/src/window/terminal_actions.rs`
- [x] 4.2 Added `TerminalNotebook::scroll_to_adjacent_prompt(previous)` (keeps VTE coordinate logic in the terminal module): resolves active session + terminal + registry, uses `vadjustment().value()` as the viewport-top buffer row, queries `prev_before`/`next_after`, scrolls via `set_value`
- [x] 4.3 No target / empty registry / no adjustment → silent no-op
- [x] 4.4 Registered unconditionally; no-op on sub-0.78 since the registry is never populated there

## Task 5: TerminalSession ownership

- [x] 5.1 Added `mark_registries: HashMap<Uuid, Rc<RefCell<MarkRegistry>>>` on `TerminalNotebook` (parallel to `terminals`, the pattern the notebook already uses for per-session state — `TerminalSession` is a `Clone` data struct, not the widget owner, so the registry lives on the notebook)
- [x] 5.2 Create the registry at the session-creation site in `tab_lifecycle.rs` and pass it to `connect_shell_integration` right there
- [x] 5.3 Removed on tab close alongside `cursor_row_base` in the close handler (no leaked refs)
- [x] 5.4 (was 2.5) `trim_below` wired: `scroll_to_adjacent_prompt` trims marks below the adjustment's `lower()` (oldest live row) before querying, so a trimmed row is never a target — no extra signal subscription needed

## Task 6: Failed-command marking

- [x] 6.1 Added `rustconn/src/terminal/failed_marker.rs` — a `FailedMarkerOverlay` that draws a short red left-margin bar for each `failed_rows()` entry currently on screen, on a transparent `DrawingArea` over the terminal, reusing the highlight layer's terminal-overlay plumbing (`grid_origin`, `attach_to_host_overlay`, `next_frame_redraw`, `viewport_rows`). Made those three helpers `pub(super)`
- [x] 6.2 Chose the margin bar (the stated primary); the row-background fallback was unnecessary since the margin bar works. Non-blocking, no modal (Requirement 3.3)
- [x] 6.3 Created and dropped per session via `failed_markers` map on `TerminalNotebook` (alongside `mark_registries`); the marker is purely a function of the registry, so it is invisible without shell integration and on sub-0.78 builds (registry never populated). Repaints on contents-changed/cursor-moved/scroll/scale, like the highlight layer. Handlers disconnected and overlay removed on `Drop`
- [x] 6.4 Verified clippy clean (both feature sets) and full bin test suite green (745 passed)

## Task 7: Documentation and i18n

- [x] 7.1 `docs/USER_GUIDE.md`: added a "Shell Integration (Prompt Navigation)" subsection reusing the Command-finished precondition wording, plus the two shortcuts in the Terminal table
- [x] 7.2 README Terminal feature row updated (OSC 133 shell integration: prompt navigation + failed-command marker)
- [x] 7.3 i18n: `po/update-pot.sh` regenerated the POT (strings surface via `i18n(&def.label)`, already in `i18n_markers`/`POTFILES.in`); `msgmerge` all 17 catalogues; replaced the fuzzy wrong-match ("Previous/Next Match") with correct "Previous/Next Prompt" translations in every locale and cleared the fuzzy flag. All three i18n gates pass (`check-potfiles`, `check-i18n-escapes`, `check-po-complete`) plus `check-pot-current`
- [x] 7.4 CHANGELOG entry added under a new `[Unreleased]` heading (Cargo version left at 0.24.2 — bump is a release step, not feature work)
- [ ] 7.5 **uk.po** translation ("Попереднє/Наступне запрошення") is pending the `uk-translation-reviewer` hook, which fires at commit time per `po/AGENTS.md` — do not hand-tune before then

## Task 8: Quality gate

- [x] 8.1 `scripts/verify.sh --tests` all green: fast gates, fmt, machete, clippy `--all-targets -D warnings` (default + cli `--features full`), `cargo doc -D warnings`, `cargo test --workspace` + cli. Fixed two issues it surfaced that the earlier bin-only checks missed: module-order/line fmt, and a broken intra-doc link to `MarkRegistry` in the sub-0.78 `connect_shell_integration` twin (the type is only imported under the feature) — now a full-path doc link. (One unrelated pre-existing flaky test, `ssh_key_setup::...passphrase_out_of_argv_and_env`, failed once under parallelism and passed in isolation and on re-run; untouched by this work.)
- [ ] 8.2 Manual verification on a `vte.sh`-sourced host: navigation jumps, failure-only marking, and a 0.76 build showing none of it *(requires a live display + remote host — for the maintainer to run)*
