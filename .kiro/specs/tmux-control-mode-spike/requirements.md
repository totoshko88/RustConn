# Requirements Document

## Introduction

tmux control mode (`tmux -CC`) lets a terminal emulator drive a tmux session over
a text-only protocol: each tmux window becomes a native tab, each pane a native
split, scrollback is local, and the session survives an SSH drop or app restart.
It is the single most valuable capability RustConn lacks against WezTerm and
iTerm2 for its sysadmin/DevOps audience (persistent sessions over flaky links).

Full integration is a flagship epic too large for the 0.24.3 release, mainly
because the UI side is hard: tmux owns the window/pane layout, reconnect needs
`capture-pane` replay, and flow control must cooperate with RustConn's bounded
PTY queue. This spec is deliberately scoped to the **groundwork only**: a
GUI-free control-mode parser and session/window/pane state model in
`rustconn-core`, with unit tests against recorded protocol streams and **no UI**.
It de-risks and unblocks the 0.25 epic without touching any hot path or widget.

Protocol reference: tmux Control-Mode wiki
(https://github.com/tmux/tmux/wiki/Control-Mode).

## Glossary

- **Control_Mode**: tmux's `-CC` text protocol. tmux emits `%`-prefixed lines
  (notifications and command-output guards) instead of drawing a screen.
- **Output_Block**: a command's output wrapped in `%begin … %end` (success) or
  `%begin … %error` (failure), each carrying a timestamp, unique command number,
  and flags.
- **Notification**: an asynchronous `%`-prefixed line sent on a change, never
  inside an Output_Block (e.g. `%output`, `%window-add`, `%layout-change`,
  `%session-changed`, `%exit`, `%pause`, `%continue`, `%extended-output`).
- **Pane_Output**: a `%output %<pane-id> <data>` line; `<data>` has bytes below
  ASCII 32 and `\` octal-escaped (e.g. `\134`) and may not be valid UTF-8.
- **Session_Model**: the in-memory representation of the attached tmux session's
  windows and panes, keyed by tmux IDs (`$n` session, `@n` window, `%n` pane).
- **Flow_Control**: tmux's `pause-after` mechanism; when enabled, `%output`
  is replaced by `%extended-output` carrying a lag value and panes can be paused
  (`%pause`) and resumed (`refresh-client -A`).
- **Spike_Scope**: this spec delivers parser + Session_Model + tests only. No
  VTE wiring, no tab/split creation, no reconnect, no UI.

## Requirements

### Requirement 1 — Parse the control-mode line protocol

**User story:** As a developer building the 0.25 tmux epic, I want a correct,
tested parser for the control-mode stream, so that the UI layer can consume
structured events instead of raw bytes.

#### Acceptance Criteria

1. WHEN the parser receives a `%begin`/`%end` pair THEN it SHALL yield a
   successful Output_Block carrying its timestamp, command number, flags, and the
   lines between the guards.
2. WHEN the parser receives a `%begin`/`%error` pair THEN it SHALL yield a failed
   Output_Block with the same metadata and the error lines.
3. WHEN the parser receives a `%output %<pane-id> <data>` line THEN it SHALL
   yield a Pane_Output event with the pane ID and the un-escaped bytes (octal
   `\nnn` decoded, `\134` → `\`), preserving non-UTF-8 content as raw bytes.
4. WHEN the parser receives any supported Notification THEN it SHALL yield a
   typed event: `%window-add`, `%window-close`, `%window-renamed`,
   `%unlinked-window-*`, `%session-changed`, `%sessions-changed`,
   `%session-window-changed`, `%layout-change`, `%window-pane-changed`,
   `%pane-mode-changed`, `%client-session-changed`, `%session-renamed`,
   `%exit`, `%pause`, `%continue`, `%extended-output`, `%subscription-changed`.
5. WHEN a Notification appears THEN the parser SHALL never emit it as part of an
   Output_Block (notifications never occur inside a block, per the protocol).
6. WHEN the parser receives a line it does not recognise THEN it SHALL surface it
   as an explicit "unknown" event rather than silently dropping it, so protocol
   drift is observable.
7. WHEN the input is fed in arbitrary chunk boundaries (mid-line splits) THEN the
   parser SHALL produce identical events to a whole-stream feed (incremental
   feeding is supported, matching how `pty_relay` delivers chunks).

### Requirement 2 — Maintain a session/window/pane state model

**User story:** As a developer building the 0.25 epic, I want the parser's events
folded into a consistent model of windows and panes, so that the UI can mirror
tmux's structure.

#### Acceptance Criteria

1. WHEN `%window-add @w` / `%window-close @w` events are applied THEN the
   Session_Model SHALL add/remove window `@w`.
2. WHEN `%window-renamed @w name` is applied THEN the model SHALL update that
   window's name.
3. WHEN `%layout-change` is applied THEN the model SHALL record the new layout
   string for the window so a later UI can reconstruct pane geometry (parsing the
   layout into concrete rectangles is explicitly out of Spike_Scope; storing it
   verbatim is in).
4. WHEN `%session-changed $s name` / `%session-renamed` are applied THEN the
   model SHALL track the attached session ID and name.
5. WHEN `%exit` is applied THEN the model SHALL mark the control session ended.
6. The Session_Model SHALL key everything by tmux IDs (`$`, `@`, `%`), never by
   index or name, per the protocol's own recommendation.

### Requirement 3 — GUI-free and test-first

**User story:** As a maintainer, I want this groundwork to carry zero UI risk and
full test coverage, so that it can land in 0.24.3 safely.

#### Acceptance Criteria

1. The parser and Session_Model SHALL live in `rustconn-core` and SHALL NOT
   depend on GTK, VTE, or any display.
2. The spike SHALL NOT create tabs, splits, VTE widgets, or reconnect logic, and
   SHALL NOT alter the `pty_relay` hot path.
3. Unit tests SHALL drive the parser with recorded control-mode streams
   (including the examples in the tmux wiki) and assert the exact event sequence,
   including chunk-boundary robustness and octal un-escaping.
4. Flow_Control events (`%pause`, `%continue`, `%extended-output`) SHALL be
   parsed and modelled as state transitions, even though acting on them is left
   to the 0.25 epic, so the model is ready for it.
5. The code SHALL pass the project quality gate (clippy pedantic/nursery, zero
   warnings) with no new runtime dependency.
