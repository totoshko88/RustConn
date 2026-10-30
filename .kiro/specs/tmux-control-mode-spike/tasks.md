# Tasks: tmux Control Mode Spike

## Task 1: Line framer

- [x] 1.1 Created `rustconn-core/src/tmux_control/mod.rs`, registered in `rustconn-core/src/lib.rs`
- [x] 1.2 `LineFramer { buf: Vec<u8> }` with `push`, buffering the partial trailing line across calls; lines kept as `Vec<u8>`; tolerates CRLF
- [x] 1.3 Covered by the chunk-boundary and CRLF tests (mid-line splits at every offset, no-trailing-LF tail retention)

## Task 2: ControlEvent and parser

- [x] 2.1 ID newtypes `SessionId`/`WindowId`/`PaneId` and the full `ControlEvent` enum
- [x] 2.2 `ControlModeParser::feed(&[u8]) -> Vec<ControlEvent>` over the framer
- [x] 2.3 Block handling: `%begin` opens, accumulate until `%end`/`%error`, carrying time/num/flags
- [x] 2.4 Every listed notification parsed
- [x] 2.5 Octal un-escape for `%output`/`%extended-output`, non-UTF-8 preserved
- [x] 2.6 Unrecognised line → `Unknown`, never panic/drop
- [x] 2.7 A non-`%end`/`%error` line inside an open block is treated as block output (incl. one starting with `%`)

## Task 3: Parser tests (golden streams)

- [x] 3.1 Wiki fixtures reproduced (new-session attach, `neww`, output, `lsp -a` block, quoted `ls -F`) with exact event asserts
- [x] 3.2 `assert_chunk_robust` feeds each fixture split at every byte offset
- [x] 3.3 Un-escape tests: `\015\015\012`, `\134`, raw 0xFF non-UTF-8 round-trip
- [x] 3.4 Open-block-at-EOF holds rather than emitting a partial

## Task 4: TmuxSessionModel

- [x] 4.1 `PaneState`/`WindowState`/`TmuxSessionModel` with `apply(&ControlEvent)`
- [x] 4.2 Folds window add/close/rename, `%window-pane-changed`, `%pane-mode-changed`
- [x] 4.3 Folds session changed/renamed, `%exit` sets `ended`
- [x] 4.4 Stores `%layout-change` verbatim (no geometry decoding)
- [x] 4.5 `%pause`/`%continue` fold into `PaneState.paused` (modelled, not actuated)
- [x] 4.6 Keyed by tmux IDs via `BTreeMap<WindowId, WindowState>`

## Task 5: Model tests

- [x] 5.1 Event sequences assert window/session state, layout capture, pause/continue, ended flag (20 tests total, all passing)
- [x] 5.2 No GTK/VTE/display dependency anywhere; `tmux_control` imports only `std`

## Task 6: Quality gate and docs

- [x] 6.1 `scripts/verify.sh --tests` all green (fmt, machete, clippy `-D warnings`, doc, full workspace tests); no new runtime dependency (`tmux_control` imports only `std`)
- [x] 6.2 Module doc states the spike scope and the explicit 0.25 out-of-scope list
- [x] 6.3 CHANGELOG entry under `[Unreleased]` → `### Groundwork` (core-only, no UI)
