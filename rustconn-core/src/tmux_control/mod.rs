//! **Experimental** — GUI-free foundation for a future `tmux -CC` (control mode)
//! integration. Nothing in the application uses it yet; the API may change.
//!
//! tmux control mode is a line-oriented, text-only protocol: tmux emits
//! `%`-prefixed notifications and wraps each command's output in `%begin`/`%end`
//! (or `%error`) guards instead of drawing a screen, so a terminal can present
//! tmux windows as native tabs and panes while the session survives an SSH drop.
//! See the tmux Control-Mode wiki.
//!
//! This module is the **groundwork only** (the tmux-control-mode-spike spec): a
//! streaming parser ([`ControlModeParser`]) and an in-memory
//! [`TmuxSessionModel`]. It has no GTK/VTE dependency and does nothing with a
//! real tmux yet. The hard part of the full feature — mapping windows/panes onto
//! RustConn's tabs and splits, `capture-pane` reconnect, flow-control actuation,
//! input routing — is the 0.25 epic and is deliberately out of scope here.
//!
//! # What is explicitly out of scope (0.25 epic)
//!
//! VTE wiring, tab/split creation, decoding a `%layout-change` string into pane
//! rectangles, reconnect with `capture-pane`, acting on flow control
//! (`refresh-client -A/-f`), size negotiation (`refresh-client -C`), and input
//! via `send-keys -H`. This module stops at a tested parser and a tested model.

use std::collections::BTreeMap;

/// A tmux session id (`$n`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(pub String);

/// A tmux window id (`@n`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(pub String);

/// A tmux pane id (`%n`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PaneId(pub String);

/// Buffers partial lines across `push` calls so a chunk boundary never splits a
/// control-mode line.
///
/// Lines are returned as raw bytes, not `String`: a `%output` payload may not be
/// valid UTF-8. The trailing partial line (if the chunk did not end on `\n`) is
/// retained until the next `push` completes it.
#[derive(Debug, Default)]
struct LineFramer {
    buf: Vec<u8>,
}

impl LineFramer {
    fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(bytes);
        let mut lines = Vec::new();
        // Drain complete lines; keep the tail after the last `\n`.
        let mut start = 0;
        for i in 0..self.buf.len() {
            if self.buf[i] == b'\n' {
                // Strip a trailing `\r` as well, tolerating CRLF.
                let mut end = i;
                if end > start && self.buf[end - 1] == b'\r' {
                    end -= 1;
                }
                lines.push(self.buf[start..end].to_vec());
                start = i + 1;
            }
        }
        if start > 0 {
            self.buf.drain(..start);
        }
        lines
    }
}

/// One structured event lifted from the control-mode stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlEvent {
    /// `%begin <time> <num> <flags>` — a command's output block opened.
    BlockBegin {
        /// Seconds since the epoch, from the guard line.
        time: u64,
        /// Unique command number, matching the closing `%end`/`%error`.
        num: u64,
        /// Guard flags (currently always 1).
        flags: u64,
    },
    /// `%end` — the open block closed successfully, carrying its output lines.
    BlockEnd {
        /// Seconds since the epoch, from the guard line.
        time: u64,
        /// Unique command number, matching the opening `%begin`.
        num: u64,
        /// Guard flags.
        flags: u64,
        /// The block's output lines, verbatim (may not be valid UTF-8).
        lines: Vec<Vec<u8>>,
    },
    /// `%error` — the open block closed with an error, carrying its output.
    BlockError {
        /// Seconds since the epoch, from the guard line.
        time: u64,
        /// Unique command number, matching the opening `%begin`.
        num: u64,
        /// Guard flags.
        flags: u64,
        /// The block's error output lines, verbatim.
        lines: Vec<Vec<u8>>,
    },
    /// `%output %pane <data>` — pane output, octal-unescaped to raw bytes.
    Output {
        /// The pane the output belongs to.
        pane: PaneId,
        /// The output bytes, octal-unescaped.
        data: Vec<u8>,
    },
    /// `%extended-output %pane <age> : <data>` — pane output under flow control.
    ExtendedOutput {
        /// The pane the output belongs to.
        pane: PaneId,
        /// Milliseconds by which this pane is behind.
        age_ms: u64,
        /// The output bytes, octal-unescaped.
        data: Vec<u8>,
    },
    /// `%window-add @w` — a window was added to the attached session.
    WindowAdd(WindowId),
    /// `%window-close @w` — a window was closed in the attached session.
    WindowClose(WindowId),
    /// `%window-renamed @w name` — a window in the attached session was renamed.
    WindowRenamed(WindowId, String),
    /// `%window-pane-changed @w %p` — a window's active pane changed.
    WindowPaneChanged(WindowId, PaneId),
    /// `%unlinked-window-add @w` — a window was added to another session.
    UnlinkedWindowAdd(WindowId),
    /// `%unlinked-window-close @w` — a window was closed in another session.
    UnlinkedWindowClose(WindowId),
    /// `%unlinked-window-renamed @w name` — a window in another session renamed.
    UnlinkedWindowRenamed(WindowId, String),
    /// `%session-changed $s name` — the attached session changed.
    SessionChanged(SessionId, String),
    /// `%session-renamed $s name` — a session was renamed.
    SessionRenamed(SessionId, String),
    /// `%sessions-changed` — a session was created or destroyed.
    SessionsChanged,
    /// `%session-window-changed $s @w` — a session's current window changed.
    SessionWindowChanged(SessionId, WindowId),
    /// `%client-session-changed client $s name` — another client's session.
    ClientSessionChanged {
        /// The client whose session changed.
        client: String,
        /// The session it switched to.
        session: SessionId,
        /// The session's name.
        name: String,
    },
    /// `%pane-mode-changed %p` — a pane's mode (copy/choose) changed.
    PaneModeChanged(PaneId),
    /// `%layout-change @w <layout>` — a window's layout changed.
    LayoutChange {
        /// The window whose layout changed.
        window: WindowId,
        /// The raw layout string, stored verbatim (geometry decoding is 0.25).
        layout: String,
    },
    /// `%pause %p` — tmux paused output to a pane under flow control.
    Pause(PaneId),
    /// `%continue %p` — tmux resumed output to a paused pane.
    Continue(PaneId),
    /// `%subscription-changed name …` — a format subscription's value changed.
    SubscriptionChanged {
        /// The subscription name.
        name: String,
        /// The remaining fields of the notification, verbatim.
        rest: String,
    },
    /// `%exit [reason]` — the control client is exiting.
    Exit {
        /// The optional reason tmux gave for exiting.
        reason: Option<String>,
    },
    /// A line the parser did not recognise, surfaced rather than dropped so
    /// protocol drift is observable.
    Unknown(Vec<u8>),
}

/// Accumulates the lines of an open `%begin … %end`/`%error` block.
#[derive(Debug)]
struct PartialBlock {
    time: u64,
    num: u64,
    flags: u64,
    lines: Vec<Vec<u8>>,
}

/// A streaming parser for the tmux control-mode protocol.
///
/// Feed it raw bytes with [`ControlModeParser::feed`]; it returns the events
/// completed by those bytes. Incremental feeding is supported — a line split
/// across two `feed` calls yields the same events as the whole stream at once.
#[derive(Debug, Default)]
pub struct ControlModeParser {
    framer: LineFramer,
    block: Option<PartialBlock>,
}

impl ControlModeParser {
    /// Creates an empty parser.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses `bytes` and returns the events they complete.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<ControlEvent> {
        let mut events = Vec::new();
        for line in self.framer.push(bytes) {
            self.handle_line(line, &mut events);
        }
        events
    }

    fn handle_line(&mut self, line: Vec<u8>, events: &mut Vec<ControlEvent>) {
        // Inside an open block, only `%end`/`%error` are control lines; every
        // other line is block output — including one that happens to start with
        // `%`, which the protocol says cannot be a notification here.
        if self.block.is_some() {
            if line.starts_with(b"%end") || line.starts_with(b"%error") {
                self.close_block(&line, events);
            } else {
                // Safe: block is Some.
                if let Some(block) = self.block.as_mut() {
                    block.lines.push(line);
                }
            }
            return;
        }

        if !line.starts_with(b"%") {
            // Outside a block, a non-% line is not part of the protocol stream.
            events.push(ControlEvent::Unknown(line));
            return;
        }

        let text = String::from_utf8_lossy(&line).into_owned();
        if let Some(event) = parse_notification(&text, &line) {
            if let ParsedLine::OpensBlock(block) = event {
                self.block = Some(block);
            } else if let ParsedLine::Event(ev) = event {
                events.push(ev);
            }
        } else {
            events.push(ControlEvent::Unknown(line));
        }
    }

    fn close_block(&mut self, line: &[u8], events: &mut Vec<ControlEvent>) {
        let Some(block) = self.block.take() else {
            return;
        };
        let text = String::from_utf8_lossy(line);
        let is_error = text.starts_with("%error");
        // `%end <time> <num> <flags>` — if the trailer does not parse, keep the
        // block's own begin metadata (the guards always match per the protocol).
        let (time, num, flags) = parse_three_u64(text.split_whitespace().skip(1)).unwrap_or((
            block.time,
            block.num,
            block.flags,
        ));
        let event = if is_error {
            ControlEvent::BlockError {
                time,
                num,
                flags,
                lines: block.lines,
            }
        } else {
            ControlEvent::BlockEnd {
                time,
                num,
                flags,
                lines: block.lines,
            }
        };
        events.push(event);
    }
}

/// The outcome of parsing a single `%`-line outside a block.
enum ParsedLine {
    Event(ControlEvent),
    OpensBlock(PartialBlock),
}

/// Parses one `%`-prefixed line. `raw` is the original bytes, used for `%output`
/// whose payload may not be valid UTF-8.
fn parse_notification(text: &str, raw: &[u8]) -> Option<ParsedLine> {
    let mut parts = text.split(' ');
    let verb = parts.next()?;

    match verb {
        "%begin" => {
            let (time, num, flags) = parse_three_u64(text.split_whitespace().skip(1))?;
            Some(ParsedLine::OpensBlock(PartialBlock {
                time,
                num,
                flags,
                lines: Vec::new(),
            }))
        }
        "%output" => parse_output(&mut parts, raw),
        "%extended-output" => parse_extended_output(&mut parts, text, raw),
        "%window-add" => simple_window(&mut parts, ControlEvent::WindowAdd),
        "%window-close" => simple_window(&mut parts, ControlEvent::WindowClose),
        "%unlinked-window-add" => simple_window(&mut parts, ControlEvent::UnlinkedWindowAdd),
        "%unlinked-window-close" => simple_window(&mut parts, ControlEvent::UnlinkedWindowClose),
        "%window-renamed" => renamed_window(&mut parts, false),
        "%unlinked-window-renamed" => renamed_window(&mut parts, true),
        "%window-pane-changed" => {
            let window = WindowId(parts.next()?.to_string());
            let pane = PaneId(parts.next()?.to_string());
            Some(ParsedLine::Event(ControlEvent::WindowPaneChanged(
                window, pane,
            )))
        }
        "%layout-change" => {
            let window = WindowId(parts.next()?.to_string());
            // The layout is the next field; later fields (visible/flags) ignored.
            let layout = parts.next()?.to_string();
            Some(ParsedLine::Event(ControlEvent::LayoutChange {
                window,
                layout,
            }))
        }
        "%session-changed" => {
            let session = SessionId(parts.next()?.to_string());
            let name = parts.collect::<Vec<_>>().join(" ");
            Some(ParsedLine::Event(ControlEvent::SessionChanged(
                session, name,
            )))
        }
        "%session-renamed" => {
            let session = SessionId(parts.next()?.to_string());
            let name = parts.collect::<Vec<_>>().join(" ");
            Some(ParsedLine::Event(ControlEvent::SessionRenamed(
                session, name,
            )))
        }
        "%sessions-changed" => Some(ParsedLine::Event(ControlEvent::SessionsChanged)),
        "%session-window-changed" => {
            let session = SessionId(parts.next()?.to_string());
            let window = WindowId(parts.next()?.to_string());
            Some(ParsedLine::Event(ControlEvent::SessionWindowChanged(
                session, window,
            )))
        }
        "%client-session-changed" => {
            let client = parts.next()?.to_string();
            let session = SessionId(parts.next()?.to_string());
            let name = parts.collect::<Vec<_>>().join(" ");
            Some(ParsedLine::Event(ControlEvent::ClientSessionChanged {
                client,
                session,
                name,
            }))
        }
        "%pane-mode-changed" => {
            let pane = PaneId(parts.next()?.to_string());
            Some(ParsedLine::Event(ControlEvent::PaneModeChanged(pane)))
        }
        "%pause" => {
            let pane = PaneId(parts.next()?.to_string());
            Some(ParsedLine::Event(ControlEvent::Pause(pane)))
        }
        "%continue" => {
            let pane = PaneId(parts.next()?.to_string());
            Some(ParsedLine::Event(ControlEvent::Continue(pane)))
        }
        "%subscription-changed" => {
            let name = parts.next()?.to_string();
            let rest = parts.collect::<Vec<_>>().join(" ");
            Some(ParsedLine::Event(ControlEvent::SubscriptionChanged {
                name,
                rest,
            }))
        }
        "%exit" => {
            let reason = text.strip_prefix("%exit").map(str::trim).and_then(|r| {
                if r.is_empty() {
                    None
                } else {
                    Some(r.to_string())
                }
            });
            Some(ParsedLine::Event(ControlEvent::Exit { reason }))
        }
        _ => None,
    }
}

/// `%output %pane <data…>` — data from the raw bytes so non-UTF-8 survives.
fn parse_output(parts: &mut std::str::Split<'_, char>, raw: &[u8]) -> Option<ParsedLine> {
    let pane = parts.next()?.to_string();
    let data = raw_after_two_spaces(raw);
    Some(ParsedLine::Event(ControlEvent::Output {
        pane: PaneId(pane),
        data: unescape_octal(&data),
    }))
}

/// `%extended-output %pane <age_ms> : <data…>`.
fn parse_extended_output(
    parts: &mut std::str::Split<'_, char>,
    text: &str,
    raw: &[u8],
) -> Option<ParsedLine> {
    let pane = parts.next()?.to_string();
    let age_ms = parts.next()?.parse().ok()?;
    // Skip the lone `:` argument, then the rest is data.
    if parts.next()? != ":" {
        return None;
    }
    let prefix_len = text.find(" : ").map(|i| i + 3)?;
    let data = raw.get(prefix_len..).unwrap_or(&[]).to_vec();
    Some(ParsedLine::Event(ControlEvent::ExtendedOutput {
        pane: PaneId(pane),
        age_ms,
        data: unescape_octal(&data),
    }))
}

fn simple_window(
    parts: &mut std::str::Split<'_, char>,
    make: fn(WindowId) -> ControlEvent,
) -> Option<ParsedLine> {
    let window = WindowId(parts.next()?.to_string());
    Some(ParsedLine::Event(make(window)))
}

fn renamed_window(parts: &mut std::str::Split<'_, char>, unlinked: bool) -> Option<ParsedLine> {
    let window = WindowId(parts.next()?.to_string());
    let name = parts.collect::<Vec<_>>().join(" ");
    let event = if unlinked {
        ControlEvent::UnlinkedWindowRenamed(window, name)
    } else {
        ControlEvent::WindowRenamed(window, name)
    };
    Some(ParsedLine::Event(event))
}

/// Parses the first three whitespace-separated `u64`s of an iterator.
fn parse_three_u64<'a, I: Iterator<Item = &'a str>>(mut it: I) -> Option<(u64, u64, u64)> {
    let a = it.next()?.parse().ok()?;
    let b = it.next()?.parse().ok()?;
    let c = it.next()?.parse().ok()?;
    Some((a, b, c))
}

/// Returns the bytes after the second space in `line` (the `%output` payload).
fn raw_after_two_spaces(line: &[u8]) -> Vec<u8> {
    let mut spaces = 0;
    for (i, &b) in line.iter().enumerate() {
        if b == b' ' {
            spaces += 1;
            if spaces == 2 {
                return line.get(i + 1..).unwrap_or(&[]).to_vec();
            }
        }
    }
    Vec::new()
}

/// Decodes tmux's octal escaping of control bytes and `\`.
///
/// tmux replaces every byte below ASCII 32 and `\` with its three-digit octal
/// form (`\134` for `\`). A `\` not followed by three octal digits that form a
/// byte value (`\000`..=`\377`) is kept verbatim: that cannot arise from tmux
/// but keeps the decoder total, so a malformed or adversarial stream (e.g.
/// `\400`..=`\777`, which would not fit in a byte) never panics or wraps.
fn unescape_octal(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if data[i] == b'\\' && i + 4 <= data.len() {
            let digits = &data[i + 1..i + 4];
            if digits.iter().all(|d| (b'0'..=b'7').contains(d)) {
                // Compute in a wider type so first-digit 4..=7 cannot overflow a
                // `u8`; reject values above 255 (keep verbatim) rather than wrap.
                let value = u16::from(digits[0] - b'0') * 64
                    + u16::from(digits[1] - b'0') * 8
                    + u16::from(digits[2] - b'0');
                if let Ok(byte) = u8::try_from(value) {
                    out.push(byte);
                    i += 4;
                    continue;
                }
            }
        }
        out.push(data[i]);
        i += 1;
    }
    out
}

// ── Session model ────────────────────────────────────────────────────────────

/// A pane's tracked state within a window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneState {
    /// The pane's tmux id.
    pub id: PaneId,
    /// Whether the pane last reported a mode change (copy/choose mode, etc.).
    pub mode_changed: bool,
    /// Whether tmux has paused output to this pane under flow control.
    pub paused: bool,
}

/// A window's tracked state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowState {
    /// The window's tmux id.
    pub id: WindowId,
    /// The window's name, from the last `%window-renamed`.
    pub name: String,
    /// The raw `%layout-change` string, stored verbatim (geometry decoding is
    /// the 0.25 epic's job).
    pub layout: String,
    /// The panes seen in this window.
    pub panes: Vec<PaneState>,
}

/// The in-memory model of the attached control-mode session.
///
/// Fold each [`ControlEvent`] in with [`TmuxSessionModel::apply`]. Everything is
/// keyed by tmux IDs, never index or name, per the protocol's recommendation.
#[derive(Debug, Default)]
pub struct TmuxSessionModel {
    /// The attached session's id and name, once reported.
    pub session: Option<(SessionId, String)>,
    /// The windows of the attached session, keyed by tmux window id.
    pub windows: BTreeMap<WindowId, WindowState>,
    /// Set once `%exit` has been seen.
    pub ended: bool,
}

impl TmuxSessionModel {
    /// Creates an empty session model.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Folds one event into the model.
    pub fn apply(&mut self, event: &ControlEvent) {
        match event {
            ControlEvent::WindowAdd(w) => {
                self.windows
                    .entry(w.clone())
                    .or_insert_with(|| WindowState {
                        id: w.clone(),
                        name: String::new(),
                        layout: String::new(),
                        panes: Vec::new(),
                    });
            }
            ControlEvent::WindowClose(w) => {
                self.windows.remove(w);
            }
            ControlEvent::WindowRenamed(w, name) => {
                if let Some(win) = self.windows.get_mut(w) {
                    win.name = name.clone();
                } else {
                    self.windows.insert(
                        w.clone(),
                        WindowState {
                            id: w.clone(),
                            name: name.clone(),
                            layout: String::new(),
                            panes: Vec::new(),
                        },
                    );
                }
            }
            ControlEvent::LayoutChange { window, layout } => {
                if let Some(win) = self.windows.get_mut(window) {
                    win.layout = layout.clone();
                } else {
                    self.windows.insert(
                        window.clone(),
                        WindowState {
                            id: window.clone(),
                            name: String::new(),
                            layout: layout.clone(),
                            panes: Vec::new(),
                        },
                    );
                }
            }
            ControlEvent::WindowPaneChanged(window, pane) => {
                let win = self
                    .windows
                    .entry(window.clone())
                    .or_insert_with(|| WindowState {
                        id: window.clone(),
                        name: String::new(),
                        layout: String::new(),
                        panes: Vec::new(),
                    });
                if !win.panes.iter().any(|p| &p.id == pane) {
                    win.panes.push(PaneState {
                        id: pane.clone(),
                        mode_changed: false,
                        paused: false,
                    });
                }
            }
            ControlEvent::PaneModeChanged(pane) => {
                self.set_pane_flag(pane, |p| p.mode_changed = true);
            }
            ControlEvent::Pause(pane) => self.set_pane_flag(pane, |p| p.paused = true),
            ControlEvent::Continue(pane) => self.set_pane_flag(pane, |p| p.paused = false),
            ControlEvent::SessionChanged(id, name) | ControlEvent::SessionRenamed(id, name) => {
                self.session = Some((id.clone(), name.clone()));
            }
            ControlEvent::Exit { .. } => self.ended = true,
            // The rest carry no state this spike tracks (unlinked windows belong
            // to other sessions; subscriptions/outputs are transient).
            _ => {}
        }
    }

    fn set_pane_flag(&mut self, pane: &PaneId, mutate: impl Fn(&mut PaneState)) {
        for win in self.windows.values_mut() {
            if let Some(p) = win.panes.iter_mut().find(|p| &p.id == pane) {
                mutate(p);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests;
