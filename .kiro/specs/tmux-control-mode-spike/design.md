# Design Document: tmux Control Mode Spike

## Overview

This spike delivers the GUI-free foundation for a future `tmux -CC` integration:
a streaming parser for the control-mode line protocol and an in-memory
session/window/pane model, both in `rustconn-core`, with no VTE, no GTK, and no
changes to any hot path. It exists to de-risk the 0.25 flagship epic by proving
the protocol layer in isolation, where it can be exhaustively unit-tested without
a display.

Scope discipline is the point: the hard part of the full feature is the UI (tmux
owns layout, reconnect needs `capture-pane` replay, flow control must cooperate
with the bounded PTY queue). None of that is here. What is here is small,
self-contained, and fully testable.

### Why it fits RustConn's architecture

RustConn's `pty_relay` already owns the master FD and feeds VTE directly, and the
`Osc52Filter` already demonstrates sitting between the PTY stream and VTE. A
future tmux consumer would plug into exactly that seam: parse the `%` protocol,
feed only `%output` for a given pane into that pane's VTE. This spike builds the
parser that seam will call, staying in `rustconn-core` (no VTE dependency) so it
is testable and reusable.

## Architecture

```mermaid
graph TD
    subgraph "rustconn-core (GUI-free, this spike)"
        FEED[feed: &[u8] chunks]
        LEX[Line framer: split on LF, buffer partial]
        PARSE[ControlModeParser: line -> ControlEvent]
        EV[ControlEvent enum]
        MODEL[TmuxSessionModel: apply event]
    end

    subgraph "0.25 epic (NOT in this spike)"
        UI[VTE panes / tabs / splits]
        RECON[Reconnect + capture-pane]
    end

    FEED --> LEX --> PARSE --> EV --> MODEL
    EV -.future.-> UI
    MODEL -.future.-> UI
    MODEL -.future.-> RECON
```

## Components and Interfaces

### 1. Line framer

Control mode is line-oriented (LF-terminated). The framer buffers a partial
trailing line across `feed` calls so that chunk boundaries never split an event
(Requirement 1.7). This mirrors how `pty_relay` hands over arbitrary-sized
chunks.

```rust
struct LineFramer { buf: Vec<u8> }
impl LineFramer {
    /// Push bytes; return complete lines, retaining any partial tail.
    fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>>;
}
```

Lines are kept as `Vec<u8>`, not `String`: `%output` payloads may not be valid
UTF-8 (Requirement 1.3).

### 2. `ControlEvent` and the parser

```rust
enum ControlEvent {
    BlockBegin { time: u64, num: u64, flags: u64 },
    BlockEnd   { time: u64, num: u64, flags: u64, lines: Vec<Vec<u8>> },
    BlockError { time: u64, num: u64, flags: u64, lines: Vec<Vec<u8>> },
    Output { pane: PaneId, data: Vec<u8> },           // %output (un-escaped)
    ExtendedOutput { pane: PaneId, age_ms: u64, data: Vec<u8> }, // %extended-output
    WindowAdd(WindowId), WindowClose(WindowId),
    WindowRenamed(WindowId, String), WindowPaneChanged(WindowId, PaneId),
    UnlinkedWindowAdd(WindowId), UnlinkedWindowClose(WindowId),
    UnlinkedWindowRenamed(WindowId, String),
    SessionChanged(SessionId, String), SessionRenamed(SessionId, String),
    SessionsChanged, SessionWindowChanged(SessionId, WindowId),
    ClientSessionChanged { client: String, session: SessionId, name: String },
    PaneModeChanged(PaneId), LayoutChange { window: WindowId, layout: String },
    Pause(PaneId), Continue(PaneId),
    SubscriptionChanged { name: String, rest: String },
    Exit { reason: Option<String> },
    Unknown(Vec<u8>),                                  // Requirement 1.6
}

struct ControlModeParser { framer: LineFramer, block: Option<PartialBlock> }
impl ControlModeParser {
    fn feed(&mut self, bytes: &[u8]) -> Vec<ControlEvent>;
}
```

ID newtypes (`SessionId($n)`, `WindowId(@n)`, `PaneId(%n)`) keep the model honest
(Requirement 2.6) and make the API self-documenting.

Parsing rules, straight from the protocol:
- A line starting `%begin` opens a block; subsequent non-`%` lines accumulate
  until `%end`/`%error` closes it. Notifications never appear inside a block, so
  the parser treats a `%`-line while a block is open as the block's output only
  for `%end`/`%error`; any other `%`-line is a protocol violation surfaced as
  `Unknown` (defensive, Requirement 1.5/1.6).
- `%output`/`%extended-output` payloads are un-escaped: octal `\nnn` → byte,
  `\134` → `\` (Requirement 1.3).

### 3. `TmuxSessionModel`

```rust
struct PaneState { id: PaneId, mode_flag: bool, paused: bool }
struct WindowState { id: WindowId, name: String, layout: String, panes: Vec<PaneState> }
struct TmuxSessionModel {
    session: Option<(SessionId, String)>,
    windows: BTreeMap<WindowId, WindowState>,
    ended: bool,
}
impl TmuxSessionModel {
    fn apply(&mut self, ev: &ControlEvent);
}
```

`apply` folds each event into the model (Requirement 2). `%layout-change` stores
the raw layout string; decoding it into rectangles is explicitly deferred to the
0.25 epic (Requirement 2.3). Flow-control events flip `paused` so the model is
ready for the epic without acting on it here (Requirement 3.4).

## Data Models

Everything is in-memory and transient. Nothing is serialized to `settings.toml`;
the spike adds no config and no new runtime dependency (Requirement 3.5).

## Error Handling

- Unknown or malformed lines become `ControlEvent::Unknown`, never a panic and
  never a silent drop, so protocol drift across tmux versions is observable in
  tests and logs.
- A `%begin` without a matching close at end-of-stream leaves the block open;
  the parser holds it rather than emitting a half block, matching the OSC-filter
  discipline of not handing partial state downstream.

## Testing Strategy

- **Golden-stream unit tests**: feed the exact examples from the tmux wiki
  (new-session attach, `neww`, `ls /`, `lsp -a`, the quoted-name `ls -F` sample)
  and assert the precise `ControlEvent` sequence.
- **Chunk-boundary tests**: feed the same streams split at every byte offset and
  assert identical event output (Requirement 1.7).
- **Un-escape tests**: `%output` with `\015\012`, `\134`, and non-UTF-8 bytes
  round-trip to the right raw bytes.
- **Model-folding tests**: apply event sequences and assert window add/close/
  rename, session change, layout capture, pause/continue, and `%exit`.
- No display, no GTK, no VTE anywhere in the test tree (Requirement 3.1/3.2).

## Explicitly out of scope (0.25 epic)

VTE wiring, tab/split creation, layout-string → geometry decoding, reconnect with
`capture-pane`, flow-control actuation (`refresh-client -A/-f`), size negotiation
(`refresh-client -C`), input routing via `send-keys -H`, and any UI. This spec
stops at a tested parser and a tested model.
