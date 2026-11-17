//! OSC 52 — clipboard offers from the remote side.
//!
//! libvte parses OSC 52 (`ESC ] 52 ; …`) and then does nothing with it: through
//! 0.84 the `XTERM_SET_XSELECTION` handler is a no-op, so a yank in a remote
//! Neovim — or in tmux with `set-clipboard external` — puts the text nowhere.
//! GNOME Terminal and xfce4-terminal inherit the same hole, so this is a VTE
//! limitation and not a RustConn bug. VTE's 4 KiB `VTE_SEQ_STRING_MAX_CAPACITY`
//! is a second ceiling on the way: an OSC string longer than that is dropped
//! before any handler would see it.
//!
//! [`Osc52Filter`] does what VTE will not. It lifts clipboard offers out of the
//! PTY byte stream on the way to `Terminal::feed`, hands the text to the system
//! clipboard, and passes every other byte through untouched.
//!
//! The filter must never disagree with VTE about what a byte means: anything it
//! holds back that VTE would draw is a bug (a prompt that does not show), and
//! anything VTE treats as OSC 52 that the filter misses is a hole (a remote
//! clipboard write). VTE decodes the stream as UTF-8 before parsing it, so the
//! 8-bit C1 controls are accepted here only in that form — `c2 9d` for `OSC`,
//! `c2 9c` for `ST` — and a bare `0x9d`/`0x9c` is just a continuation byte, as
//! in `❯` (`e2 9d af`). `CAN`, `SUB` and the other C1 controls abandon an open
//! string, as they do in VTE, and only what VTE would also wait for (an open
//! string, a lone `ESC` or `c2`) is held at the end of a chunk. Whatever the
//! filter removes is replaced by a single `CAN`, so that nothing it released
//! before the removal can pair with what follows it ([`Osc52Filter`]).
//!
//! Two rules keep that from becoming a remote-controlled write primitive:
//!
//! * **Opt-in.** [`set_enabled`] mirrors `TerminalSettings::allow_osc52_clipboard`
//!   and starts out off, the way [`super::safe_paste`] does for its own flag. A
//!   host that can write the clipboard can aim a pastejacking payload at the
//!   next terminal the user pastes into, so it is the user's call, per install.
//! * **Write-only, one selection.** A `?` payload is a *query* — the terminal is
//!   being asked to send its clipboard back to the application. Answering it
//!   would stream local clipboard contents to a remote, so the sequence is
//!   dropped unanswered and the application falls back to its own timeout. The
//!   `p` (primary) and `s` (select) selections are passed through rather than
//!   half-supported: they are real, and writing them from a remote was not asked
//!   for.
//!
//! ponytail: only the `c` selection and only `ST`/`BEL`-terminated strings are
//! recognised — no DCS tmux passthrough (`ESC P tmux ; …`), which `allow-passthrough
//! on` wraps sequences in. `set-clipboard external`, the documented route, writes
//! OSC 52 straight to the outer tty and needs none of that. Add a DCS state to
//! [`State`] if a real user turns out to need the wrapper.

use std::cell::Cell;
use std::mem;

use gtk4::glib;
use gtk4::prelude::*;
use vte4::Terminal;
use zeroize::Zeroizing;

thread_local! {
    /// Whether an OSC 52 clipboard offer is honoured. Mirrors
    /// `TerminalSettings::allow_osc52_clipboard`; read once per completed
    /// sequence, so a change in Preferences applies to sessions that are already
    /// open. GTK is single-threaded, so a thread-local is the whole-process
    /// value here — the same reasoning as [`super::safe_paste`].
    static ENABLED: Cell<bool> = const { Cell::new(false) };
}

/// Records whether OSC 52 clipboard offers are honoured.
///
/// Called from `configure_terminal_with_settings`, which the settings dialog
/// re-runs on save, so the flag tracks the setting without threading it into
/// every call site.
pub fn set_enabled(enabled: bool) {
    ENABLED.with(|flag| flag.set(enabled));
}

/// Whether an OSC 52 clipboard offer is honoured.
fn enabled() -> bool {
    ENABLED.with(Cell::get)
}

/// Puts a decoded OSC 52 payload on the system clipboard.
///
/// Takes the terminal's own display, the way every other clipboard write in this
/// module does, and must be called on the GTK main thread — which the caller is,
/// because the only path here is the one `deliver_output_to` runs.
///
/// ponytail: silent, like copy-on-select — a remote yank is not worth a toast,
/// and the paste that follows is the user's own; add a toast here if users
/// report not noticing that a remote program overwrote their clipboard.
pub fn offer_to_clipboard(terminal: &Terminal, text: &str) {
    terminal.display().clipboard().set_text(text);
}

/// Longest OSC string body [`Osc52Filter`] will hold, in bytes.
///
/// The remote is the untrusted side of this boundary, so this bounds how much
/// of a stream it can make us buffer: a string that outgrows it is discarded up
/// to its terminator, which is what VTE does with anything past its own 4 KiB
/// limit. 1 MiB sits far enough above that to keep every sequence VTE would
/// have accepted working here — and above tmux's 100 kB `set-clipboard-limit` —
/// while still capping a hostile sender.
const MAX_OSC_BODY: usize = 1024 * 1024;

/// `ESC` — an introducer on its own, and the first byte of the 7-bit `ST`.
const ESC: u8 = 0x1b;
/// `]` — what turns `ESC` into a 7-bit OSC introducer.
const INTRODUCER_7BIT: u8 = b']';
/// `\` — the second byte of the 7-bit `ST` terminator.
const TERMINATOR_7BIT: u8 = b'\\';
/// First byte of the UTF-8 encoding of every C1 control, `U+0080`–`U+009F`
/// (`c2 80`–`c2 9f`).
///
/// VTE decodes the PTY stream as UTF-8 *before* its parser sees it
/// (`Terminal::process_incoming_utf8` → `Parser::feed`), so a C1 control exists
/// only as these two bytes. A bare `0x9d` or `0x9c` is a continuation byte
/// (`❯` is `e2 9d af`, `➜` is `e2 9e 9c`) or, on its own, `U+FFFD` — never a
/// control. `c2` always starts a new character, so one byte of lookahead after
/// it is all the decoding the filter needs.
const C1_LEAD: u8 = 0xc2;
/// Second byte of C1 `OSC` (`c2 9d`), the 8-bit introducer.
const C1_OSC: u8 = 0x9d;
/// Second byte of C1 `ST` (`c2 9c`), the 8-bit terminator.
const C1_ST: u8 = 0x9c;
/// `CAN` — VTE abandons whatever is in progress on it, and draws nothing.
///
/// In `Parser::feed` (VTE 0.76–0.84) `0x18` is handled before the state is
/// looked at: `GROUND`, `action_ignore`, from `ESCAPE`, `OSC_STRING`, `ST_ESC`
/// and every other state alike. The UTF-8 decoder (`UTF8Decoder::decode`) treats
/// it like any ASCII byte: a character it interrupts becomes `U+FFFD` (an
/// unexpected byte in a non-accepting state is `REJECT_REWIND`) and the `CAN` is
/// then read as itself. That makes it the one byte that cleans up after a
/// removal: whatever the filter let through just before — a lone `ESC`, a
/// UTF-8 lead, the head of an abandoned string — cannot pair with the bytes
/// after the gap.
const CAN: u8 = 0x18;
/// `SUB` — likewise.
const SUB: u8 = 0x1a;
/// `DEL` — ignored by VTE wherever it turns up.
const DEL: u8 = 0x7f;
/// `BEL` — xterm accepts it as an OSC terminator and most `printf` recipes use it.
const BEL: u8 = 0x07;

/// Whether VTE leaves its parser state alone on `byte` while an `ESC` is open:
/// every C0 control but `ESC`, `CAN` and `SUB` is executed on the spot, and
/// `DEL` is ignored (`Parser::feed` and the `ESC` state of `feed_to_state`). So
/// `ESC BEL ] 52 …` still opens an OSC string in VTE, and must in the filter.
const fn is_transparent_in_escape(byte: u8) -> bool {
    matches!(byte, 0x00..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f)
}

/// Whether VTE drops `byte` from the body of an OSC string: C0 controls other
/// than `BEL`, `ESC`, `CAN` and `SUB` are `action_nop` in `OSC_STRING`, and `DEL`
/// is ignored in every state. The filter keeps them in `pending`, so a string
/// passes on untouched, but classifies the body without them, as VTE does —
/// `ESC ] 5 NUL 2 ; c ; … BEL` is OSC 52 to VTE.
const fn is_ignored_in_osc_body(byte: u8) -> bool {
    matches!(byte, 0x00..=0x06 | 0x08..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f)
}

/// Where [`Osc52Filter`] is in the stream.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Between sequences. Bytes go straight to the output.
    #[default]
    Ground,
    /// Saw `ESC`. The next byte decides whether an OSC string starts here;
    /// anything else means this `ESC` opened a different sequence.
    Escape,
    /// Saw `c2`, the first half of a possible C1 control. `9d` makes it the
    /// 8-bit OSC introducer; anything else means it was an ordinary character.
    C1Lead,
    /// Inside an OSC string, collecting its body.
    Osc,
    /// Inside an OSC string, just saw `ESC` — which is *not* in `pending` yet:
    /// `\` ends the string, and anything else abandoned it and begins a new
    /// escape sequence.
    OscEscape,
    /// Inside an OSC string, just saw `c2` — not in `pending` yet either. `9c`
    /// ends the string, `9d` restarts it, the other C1 controls abandon it, and
    /// anything else makes `c2` an ordinary character of the body.
    OscC1Lead,
}

/// Removes OSC 52 clipboard offers from one PTY output stream.
///
/// A PTY read is bounded by `READ_CHUNK` (8 KiB), which is smaller than a large
/// yank, so a sequence routinely arrives in pieces and the filter has to hold
/// state between chunks. One filter therefore belongs to one stream, and lives
/// as long as the reader does.
///
/// What VTE is fed is *VTE-equivalent* to the raw stream, not byte-identical:
/// on the ground and inside OSC strings it draws the same text, executes the
/// same controls, and dispatches the same OSC strings, minus the recognised
/// OSC 52 clipboard offers. The filter keeps no DCS, SOS/PM/APC or SCI state
/// (see the module `ponytail`), so the claim does not extend to the body of
/// those strings. A
/// sequence the filter removes — an offer, a query, a string past
/// [`MAX_OSC_BODY`], and an OSC string the stream ended inside while offers are
/// honoured ([`finish`](Self::finish)) — is replaced by exactly one `CAN`.
/// Without it, a byte released just before the gap would join what follows it:
/// a lone `ESC` would eat the next character or turn `]52;…` into an OSC, a
/// UTF-8 lead would complete a character from the continuation bytes after the
/// gap, and an abandoned string would swallow the text. VTE ignores `CAN` on the
/// ground and drops all of those with it (see [`CAN`]).
///
/// Two more byte-level differences exist. The first is invisible to VTE, the
/// second is not inside a DCS, SOS/PM/APC or SCI string:
///
/// * an OSC string abandoned for a new `ESC` sequence is released closed with
///   `CAN` as well, because the held `ESC` may be overtaken by a C0 control
///   that would otherwise land inside the string;
/// * a C0 control that arrives while an `ESC` is held goes out ahead of it.
///   That changes nothing on the ground, in a CSI or in an intermediate
///   sequence, and an abandoned OSC string has been closed by then. It does
///   inside a DCS string, an SOS/PM/APC string (VTE's `ST_IGNORE`) and after
///   an SCI (`ESC Z`). In a DCS, `ESC P … ESC NUL \` reaches VTE as
///   `ESC P … NUL ESC \`, which VTE takes as DCS data followed by the
///   terminator instead of an aborted DCS. In an APC, `ESC _ x ESC LF \`
///   becomes `ESC _ x LF ESC \`: VTE ignores the `LF` inside the string where
///   the raw stream would have executed it. After `ESC Z`, `ESC LF` becomes
///   `LF ESC` and the `LF` is taken as the SCI final instead of being executed.
///   The filter is on the ground while VTE is inside any of these, so it cannot
///   tell the cases apart. This opens no capability the remote lacks (it can
///   send the well-formed sequence directly) and does not bypass OSC 52
///   interception, since `]` after the held `ESC` still opens a string. Real
///   applications do not end such a string with `ESC <C0> \`. Keeping the
///   order would need a state for each of them.
#[derive(Debug, Default)]
pub struct Osc52Filter {
    state: State,
    /// What the filter holds back from the output: the string in progress,
    /// introducer included, or the lone `ESC` / `c2` held by [`State::Escape`] /
    /// [`State::C1Lead`]. The lookahead byte of [`State::OscEscape`] and
    /// [`State::OscC1Lead`] is not in it until it is classified.
    pending: Vec<u8>,
    /// Set once `pending` outgrew [`MAX_OSC_BODY`]: the rest of the string is
    /// discarded rather than buffered.
    oversized: bool,
    /// Where the filtered bytes land. Owned by the filter and reused, so the
    /// common case — nothing removed — costs no allocation per chunk.
    out: Vec<u8>,
}

impl Osc52Filter {
    /// Feeds one chunk of PTY output through the filter.
    ///
    /// Returns the bytes to hand to VTE, which is `chunk` itself in borrowed
    /// form whenever nothing was removed. A clipboard offer is passed to `sink`
    /// instead of the output.
    ///
    /// The result borrows the filter, so it stays valid until the next call.
    pub fn push<'a>(&'a mut self, chunk: &[u8], sink: &mut dyn FnMut(&str)) -> &'a [u8] {
        self.out.clear();
        for &byte in chunk {
            self.step(byte, sink);
        }
        &self.out
    }

    /// Releases whatever the stream ended in the middle of.
    ///
    /// A lone `ESC` or `c2` is handed over, so the filter never swallows the
    /// tail of a session. An OSC string still open is *dropped* while offers are
    /// honoured, and replaced by a `CAN` like any other removal: VTE never
    /// dispatches an unterminated string, but fed the fragment it would sit in
    /// its string state, and the next `BEL` or `ST` fed to the same widget (a
    /// local message, a reconnect) would complete it behind the filter's back.
    /// With the setting off the fragment is released as it came, like every
    /// other byte. An unterminated string carries no payload to trust, so `sink`
    /// is not involved.
    ///
    /// The result borrows the filter, as [`push`](Self::push)'s does.
    pub fn finish(&mut self) -> &[u8] {
        self.out.clear();
        let open_string = matches!(self.state, State::Osc | State::OscEscape | State::OscC1Lead);
        if open_string && enabled() {
            self.pending.clear();
            self.out.push(CAN);
        } else {
            self.out.append(&mut self.pending);
            // The lookahead byte of an open string is not in `pending`, so it
            // follows it.
            match self.state {
                State::OscEscape => self.out.push(ESC),
                State::OscC1Lead => self.out.push(C1_LEAD),
                State::Ground | State::Escape | State::C1Lead | State::Osc => {}
            }
        }
        self.state = State::Ground;
        self.oversized = false;
        &self.out
    }

    /// Classifies one byte and either passes it on or collects it.
    fn step(&mut self, byte: u8, sink: &mut dyn FnMut(&str)) {
        match self.state {
            State::Ground => match byte {
                ESC => {
                    self.pending.push(byte);
                    self.state = State::Escape;
                }
                C1_LEAD => {
                    self.pending.push(byte);
                    self.state = State::C1Lead;
                }
                _ => self.out.push(byte),
            },
            State::C1Lead => {
                if byte == C1_OSC {
                    self.pending.push(byte);
                    self.state = State::Osc;
                } else {
                    // `c2` + a continuation byte is an ordinary character (or
                    // another C1 control, which this filter has no use for), and
                    // `c2` + anything else is a malformed prefix that VTE draws
                    // as `U+FFFD`. Either way it goes on as it is: a later `9d`
                    // cannot pair with it across a removed sequence, because
                    // that leaves a `CAN` in between.
                    self.out.append(&mut self.pending);
                    self.state = State::Ground;
                    // Classify this byte again, because `c2 c2 9d` and
                    // `c2 ESC ]` each hold a sequence of their own.
                    self.step(byte, sink);
                }
            }
            State::Escape => {
                if byte == INTRODUCER_7BIT {
                    self.pending.push(byte);
                    self.state = State::Osc;
                } else if is_transparent_in_escape(byte) {
                    // VTE runs this control (or ignores `DEL`) and stays in its
                    // `ESC` state, so the filter stays too, with the `ESC` still
                    // held. The control goes out ahead of that `ESC`: executing
                    // it does not depend on the `ESC`, and holding it back would
                    // let a run of controls grow `pending` without a bound.
                    self.out.push(byte);
                } else {
                    // Not an OSC string — this `ESC` opened a CSI, a charset
                    // selector, a keypad mode or something else. Hand the held
                    // `ESC` over and classify this byte again from the ground,
                    // because `ESC ESC ]` is two sequences and not one (and so is
                    // `ESC c2 9d`). If the second is a removed offer, its `CAN`
                    // ends the first, as VTE's own `ESC ESC` / `ESC OSC` does.
                    self.out.append(&mut self.pending);
                    self.state = State::Ground;
                    self.step(byte, sink);
                }
            }
            State::Osc => match byte {
                BEL => {
                    // The terminator joins `pending`, so that `body_of` sees the
                    // same framing whichever of the three ended the string.
                    self.push_body(byte);
                    self.finish_sequence(sink);
                }
                ESC => self.state = State::OscEscape,
                C1_LEAD => self.state = State::OscC1Lead,
                CAN | SUB => {
                    // VTE drops the string on either and carries on from the
                    // ground, so the fragment and this byte both go to VTE.
                    self.abandon();
                    self.out.push(byte);
                }
                _ => self.push_body(byte),
            },
            State::OscEscape => {
                if byte == DEL {
                    // VTE ignores `DEL` in every state, `ST_ESC` included, so
                    // `ESC DEL \` still ends the string. Dropping it is
                    // invisible to VTE and keeps the lookahead `ESC` unclassified.
                } else if byte == TERMINATOR_7BIT {
                    self.push_body(ESC);
                    self.push_body(byte);
                    self.finish_sequence(sink);
                } else {
                    // The string was abandoned mid-flight. Pass on what was
                    // collected, closed with `CAN`, and treat the `ESC` as the
                    // start of the next sequence, with this byte as its second
                    // half (VTE's `ST_ESC` does the same).
                    self.abandon_closed();
                    self.pending.push(ESC);
                    self.state = State::Escape;
                    self.step(byte, sink);
                }
            }
            State::OscC1Lead => match byte {
                C1_ST => {
                    self.push_body(C1_LEAD);
                    self.push_body(byte);
                    self.finish_sequence(sink);
                }
                C1_OSC => {
                    // A new OSC replaces the one in progress. The fragment goes
                    // on as it is: the new string either passes too, and VTE
                    // restarts exactly as it does on the raw stream, or is
                    // removed and leaves a `CAN` behind (or `finish` does).
                    self.abandon();
                    self.pending.extend_from_slice(&[C1_LEAD, byte]);
                    self.state = State::Osc;
                }
                0x80..=0x9f => {
                    // Any other C1 control ends the string; both bytes go on.
                    self.abandon();
                    self.out.extend_from_slice(&[C1_LEAD, byte]);
                }
                _ => {
                    // `c2 a0`–`c2 bf` is a printable character, and anything
                    // else leaves `c2` as a malformed prefix that VTE shows as
                    // `U+FFFD` while it reads this byte afresh.
                    self.push_body(C1_LEAD);
                    self.state = State::Osc;
                    self.step(byte, sink);
                }
            },
        }
    }

    /// Gives up on the string in `pending`: what was collected goes on to VTE
    /// as it is, and the filter is back on the ground.
    fn abandon(&mut self) {
        self.out.append(&mut self.pending);
        self.oversized = false;
        self.state = State::Ground;
    }

    /// [`abandon`](Self::abandon), and ends the released fragment with `CAN`.
    ///
    /// For the abandon by `ESC` + a byte that is not `\`, where the `ESC` stays
    /// held and a C0 control may be released ahead of it. Left open, the
    /// fragment would take that control as part of the string, and the `ESC \`
    /// that follows would dispatch it — an OSC 52 the filter never saw. `CAN`
    /// ends the string in VTE without a dispatch, which is what the original
    /// bytes did.
    fn abandon_closed(&mut self) {
        self.abandon();
        self.out.push(CAN);
    }

    /// Collects one body byte, and gives up on a string that outgrew the cap.
    fn push_body(&mut self, byte: u8) {
        if self.pending.len() >= MAX_OSC_BODY {
            self.oversized = true;
            return;
        }
        self.pending.push(byte);
    }

    /// Ends the sequence in `pending`: offers it as a clipboard write, or passes
    /// it through for VTE to deal with.
    fn finish_sequence(&mut self, sink: &mut dyn FnMut(&str)) {
        let oversized = self.oversized;
        let sequence = mem::take(&mut self.pending);
        self.state = State::Ground;
        self.oversized = false;

        // Over-long: discard it rather than emit the fragment, exactly as VTE
        // does past its own limit, and let the next sequence start clean.
        if oversized {
            self.out.push(CAN);
            return;
        }

        // Disabled: the sequence goes to VTE as it came, which is what VTE — and
        // RustConn — do with it today.
        if !enabled() {
            self.out.extend_from_slice(&sequence);
            return;
        }

        // A sequence that is removed leaves one `CAN` where it was (see [`CAN`]).
        match parse(&sequence) {
            Parsed::Write(text) => {
                sink(&text);
                self.out.push(CAN);
            }
            Parsed::Query => {
                // Deliberately unanswered. A reply would be the local clipboard's
                // contents travelling to the remote, which is the one direction
                // this feature must never open.
                tracing::debug!("dropped OSC 52 clipboard query; reads are not served");
                self.out.push(CAN);
            }
            Parsed::Other => self.out.extend_from_slice(&sequence),
        }
    }
}

/// What a completed OSC string turned out to be.
enum Parsed {
    /// A clipboard write for the `CLIPBOARD` selection, decoded.
    ///
    /// `Zeroizing` because the payload is session output that may be a secret
    /// the user yanked on the remote — the same reasoning as the
    /// `Zeroizing<Vec<u8>>` chunks it arrived in.
    Write(Zeroizing<String>),
    /// A `?` payload: a request for the clipboard's contents.
    Query,
    /// Another OSC, another selection, or something malformed.
    Other,
}

/// Classifies a completed OSC string.
///
/// Every failure mode is `Other`, which means "pass it on" — an unrecognised
/// string is far more likely to be something VTE handles (a title, a colour, a
/// `OSC 133` shell-integration mark) than something worth guessing at.
fn parse(sequence: &[u8]) -> Parsed {
    let Some(body) = body_of(sequence) else {
        return Parsed::Other;
    };
    // Classify the body VTE sees: it ignores some controls inside a string, so
    // `5<NUL>2;c;…` is OSC 52 to it. The copy is wiped on drop because the
    // payload may be a secret the user yanked.
    let cleaned;
    let body = if body.iter().any(|&byte| is_ignored_in_osc_body(byte)) {
        cleaned = Zeroizing::new(
            body.iter()
                .copied()
                .filter(|&byte| !is_ignored_in_osc_body(byte))
                .collect::<Vec<u8>>(),
        );
        cleaned.as_slice()
    } else {
        body
    };

    let mut parts = body.splitn(3, |&byte| byte == b';');
    if parts.next() != Some(b"52".as_slice()) {
        return Parsed::Other;
    }
    // `c` is CLIPBOARD. `p` (primary) and `s` (select) are real xterm
    // selections, but writing them from a remote is not this feature.
    if parts.next() != Some(b"c".as_slice()) {
        return Parsed::Other;
    }

    let Some(payload) = parts.next() else {
        return Parsed::Other;
    };
    if payload == b"?" {
        return Parsed::Query;
    }

    // An empty payload is xterm's way of saying "the selection is now empty",
    // which a remote may as well be allowed to do.
    let Ok(encoded) = std::str::from_utf8(payload) else {
        return Parsed::Other;
    };
    // GLib's decoder stops at the first byte outside the alphabet rather than
    // reporting it, which is acceptable here: the payload is not a trust
    // boundary (the opt-in is), and a remote can send any well-formed base64 it
    // likes regardless. What does matter is that what comes out is text — a
    // clipboard is asked for text, and raw bytes have no business in one.
    //
    // The decoded bytes may be a secret the user yanked, so they are wrapped in
    // `Zeroizing` the instant they land in Rust — before the `String` is built —
    // so the intermediate buffer is wiped on drop rather than left in the heap.
    // (`glib::base64_decode` copies out of its own GLib allocation, which this
    // cannot wipe; swapping in a pure-Rust decoder would close that last gap but
    // is not worth a new dependency for a buffer freed microseconds later.)
    let decoded = Zeroizing::new(glib::base64_decode(encoded));
    match std::str::from_utf8(&decoded) {
        Ok(text) => Parsed::Write(Zeroizing::new(text.to_owned())),
        Err(_) => Parsed::Other,
    }
}

/// Strips the introducer and the terminator off a completed OSC string.
fn body_of(sequence: &[u8]) -> Option<&[u8]> {
    let rest = sequence
        .strip_prefix(&[ESC, INTRODUCER_7BIT])
        .or_else(|| sequence.strip_prefix(&[C1_LEAD, C1_OSC]))?;
    // Both `ST` forms are two bytes, and the filter appends the held first byte
    // together with the second one that completes them.
    rest.strip_suffix(&[ESC, TERMINATOR_7BIT])
        .or_else(|| rest.strip_suffix(&[C1_LEAD, C1_ST]))
        .or_else(|| rest.strip_suffix(&[BEL]))
}

#[cfg(test)]
mod tests {
    use super::{CAN, MAX_OSC_BODY, Osc52Filter, Parsed, State, body_of, parse, set_enabled};
    use gtk4::glib;

    /// Encodes `text` the way an application would, so a test can state its
    /// expectation in plain text.
    fn offer(text: &str) -> Vec<u8> {
        let payload = glib::base64_encode(text.as_bytes());
        format!("\x1b]52;c;{payload}\x07").into_bytes()
    }

    /// Runs `bytes` through the filter and returns what VTE would have been fed
    /// plus every clipboard write the filter made, in order.
    fn run(bytes: &[u8]) -> (Vec<u8>, Vec<String>) {
        run_in_chunks(std::iter::once(bytes))
    }

    /// The same, but with the stream cut into the given chunks — which is how a
    /// real PTY read splits a long yank.
    fn run_in_chunks<'a>(chunks: impl IntoIterator<Item = &'a [u8]>) -> (Vec<u8>, Vec<String>) {
        set_enabled(true);
        let mut filter = Osc52Filter::default();
        let mut fed = Vec::new();
        let mut writes = Vec::new();
        for chunk in chunks {
            let mut sink = |text: &str| writes.push(text.to_owned());
            fed.extend_from_slice(filter.push(chunk, &mut sink));
        }
        fed.extend_from_slice(filter.finish());
        (fed, writes)
    }

    #[test]
    fn clipboard_offer_is_lifted_out_of_the_stream() {
        let (fed, writes) = run(&offer("copied on the remote"));
        assert_eq!(writes, ["copied on the remote"]);
        assert_eq!(fed, [CAN], "VTE must see a CAN and nothing else");
    }

    #[test]
    fn offer_survives_every_possible_chunk_split() {
        let sequence = offer("a payload long enough to cross a chunk boundary for sure");
        // The reference result, fed as one chunk.
        let (fed_whole, writes_whole) = run(&sequence);

        // Every two-way split, which covers an introducer landing on a boundary,
        // the base64 landing on one, and the terminator landing on one.
        for split in 1..sequence.len() {
            let (fed, writes) = run_in_chunks([&sequence[..split], &sequence[split..]]);
            assert_eq!(writes, writes_whole, "writes differ when split at {split}");
            assert_eq!(fed, fed_whole, "output differs when split at {split}");
        }

        // Byte at a time is the worst case a reader can produce.
        let (fed, writes) = run_in_chunks(sequence.chunks(1));
        assert_eq!(writes, writes_whole);
        assert_eq!(fed, fed_whole);
    }

    #[test]
    fn unrelated_output_is_passed_through_byte_for_byte() {
        // A title, an OSC 133 shell-integration mark, a colour, a CSI, a bare
        // ESC and a chunk of plain text — all of which VTE handles today.
        let stream = b"\x1b]0;a title\x07\x1b]133;A\x1b\\hello\x1b[2J\x1b[1;31m\x1b[m world\x1b(B";
        let (fed, writes) = run(stream);
        assert_eq!(fed, stream);
        assert_eq!(writes, Vec::<String>::new());
    }

    #[test]
    fn output_around_a_removed_offer_keeps_its_order() {
        let mut stream = b"before ".to_vec();
        stream.extend_from_slice(&offer("the yank"));
        stream.extend_from_slice(b" after");
        let (fed, writes) = run(&stream);
        assert_eq!(fed, b"before \x18 after");
        assert_eq!(writes, ["the yank"]);
    }

    #[test]
    fn several_offers_in_one_chunk_are_all_handled() {
        let mut stream = offer("first");
        stream.extend_from_slice(&offer("second"));
        stream.extend_from_slice(&offer("third"));
        let (fed, writes) = run(&stream);
        assert_eq!(writes, ["first", "second", "third"]);
        assert_eq!(fed, [CAN, CAN, CAN]);
    }

    #[test]
    fn an_esc_that_starts_something_else_is_not_swallowed() {
        // `ESC [` is a CSI, and the `ESC` before the OSC must survive it.
        let mut stream = b"\x1b[2J".to_vec();
        stream.extend_from_slice(&offer("yank"));
        stream.extend_from_slice(b"\x1b[1;1H");
        let (fed, writes) = run(&stream);
        assert_eq!(fed, b"\x1b[2J\x18\x1b[1;1H");
        assert_eq!(writes, ["yank"]);
    }

    #[test]
    fn a_doubled_escape_starts_a_second_sequence() {
        // `ESC ESC ] 52 …` is one `ESC` to VTE, whose second `ESC` restarts the
        // sequence, and a perfectly good OSC: the offer is honoured. The first
        // `ESC` goes on and the offer's `CAN` ends it, so it cannot eat the `A`.
        let mut stream = b"\x1b".to_vec();
        stream.extend_from_slice(&offer("yank"));
        stream.push(b'A');
        let (fed, writes) = run(&stream);
        assert_eq!(writes, ["yank"]);
        assert_eq!(fed, b"\x1b\x18A");
        // Anything else, and the one `ESC` goes on with it.
        assert_eq!(run(b"\x1b\x1b[2J").0, b"\x1b\x1b[2J");
    }

    #[test]
    fn an_abandoned_string_is_handed_over_rather_than_truncated() {
        // A new escape sequence starts before the string is terminated. The
        // fragment goes on to VTE: leaving it there unterminated would make VTE
        // treat the rest of the session as body.
        let payload = glib::base64_encode(b"half");
        let mut stream = format!("\x1b]52;c;{payload}").into_bytes();
        stream.extend_from_slice(b"\x1b[2J");
        let (fed, writes) = run(&stream);
        assert_eq!(writes, Vec::<String>::new());
        // Nothing lost: the fragment, closed with `CAN`, then the CSI that
        // replaced it.
        let mut expected = format!("\x1b]52;c;{payload}").into_bytes();
        expected.push(0x18);
        expected.extend_from_slice(b"\x1b[2J");
        assert_eq!(fed, expected);
    }

    #[test]
    fn a_string_the_stream_ends_inside_is_dropped_by_finish_while_offers_are_honoured() {
        // Fed to VTE, the fragment would leave it in its OSC state, and the next
        // `BEL` it is given (a local message, a reconnect) would complete the
        // offer behind the filter's back. So nothing of it is released — not
        // the lookahead `ESC` / `c2` either — and a `CAN` takes its place.
        let payload = glib::base64_encode(b"cut off");
        let open = format!("\x1b]52;c;{payload}").into_bytes();
        let mut streams = vec![open.clone()];
        for tail in [&b"\x1b"[..], b"\xc2"] {
            streams.push([open.as_slice(), tail].concat());
        }
        streams.push(b"\xc2\x9d52;c;aGVsbG8=".to_vec());
        for stream in streams {
            let (fed, writes) = run(&stream);
            assert_eq!(fed, [CAN], "{stream:?} leaked an open string");
            assert!(
                writes.is_empty(),
                "an unterminated payload must not be trusted"
            );
            // The bytes VTE would otherwise see after it are not swallowed.
            let mut after = stream.clone();
            after.extend_from_slice(b"\x18text");
            let fed = run(&after).0;
            assert_vte_equivalent(&after, &fed, true);
            assert!(fed.ends_with(b"text"), "{after:?} lost its tail: {fed:?}");
        }
    }

    #[test]
    fn a_string_the_stream_ends_inside_is_released_by_finish_while_the_setting_is_off() {
        let payload = glib::base64_encode(b"cut off");
        let stream = format!("\x1b]52;c;{payload}\x1b").into_bytes();
        set_enabled(false);
        let mut filter = Osc52Filter::default();
        let mut sink = |_: &str| {};
        let mut fed = filter.push(&stream, &mut sink).to_vec();
        fed.extend_from_slice(filter.finish());
        assert_eq!(fed, stream);
    }

    #[test]
    fn controls_ignored_by_vte_do_not_hide_an_offer_in_the_body() {
        // VTE drops NUL, SOH, LF, DEL … inside an OSC string, so each of these
        // is OSC 52 to it and has to be one to the filter.
        let payload = glib::base64_encode(b"hello");
        for noise in ["\x00", "\x01", "\n", "\x17", "\x19", "\x1c", "\x7f"] {
            let streams = [
                format!("\x1b]5{noise}2;c;{payload}\x07"),
                format!("\x1b]52;{noise}c;{payload}\x07"),
                format!("\x1b]52;c;{noise}{payload}{noise}\x07"),
                format!("\x1b]52;c;{payload}{noise}\x1b\\"),
                format!("\u{9d}5{noise}2;c;{payload}\u{9c}"),
            ];
            for stream in streams {
                let (fed, writes) = run(stream.as_bytes());
                assert_eq!(writes, ["hello"], "{stream:?} was not understood");
                assert_eq!(fed, [CAN], "{stream:?} reached VTE");
            }
        }
        // A query dressed the same way is still a query, and unanswered.
        let (fed, writes) = run(b"\x1b]5\x002;c;\x01?\x07");
        assert!(writes.is_empty() && fed == [CAN]);
        // And with the setting off nothing is altered.
        assert_eq!(
            pushed_once(false, b"\x1b]5\x002;c;aGk=\x07"),
            b"\x1b]5\x002;c;aGk=\x07"
        );
    }

    #[test]
    fn a_control_between_esc_and_the_introducer_does_not_hide_an_offer() {
        // In VTE's ESC state a C0 control is executed and `DEL` ignored without
        // leaving it, so `ESC <control> ]` opens an OSC string. The control goes
        // out; the `ESC` and the offer do not.
        let payload = glib::base64_encode(b"hello");
        for control in ["\x00", "\x07", "\x0a", "\x19", "\x1f", "\x7f"] {
            let stream = format!("\x1b{control}]52;c;{payload}\x07");
            let (fed, writes) = run(stream.as_bytes());
            assert_eq!(writes, ["hello"], "{stream:?} was not understood");
            assert_eq!(fed, [control.as_bytes(), &[CAN]].concat(), "{stream:?}");
            // Several in a row, and in front of an `ST`-terminated offer.
            let stream = format!("\x1b{control}{control}]52;c;{payload}\x1b\\");
            let (fed, writes) = run(stream.as_bytes());
            assert_eq!(writes, ["hello"], "{stream:?} was not understood");
            assert_eq!(
                fed,
                [[control, control].concat().as_bytes(), &[CAN]].concat()
            );
        }
        // The same after an abandoned string: `ST_ESC` falls through to `ESC`.
        // The abandoned fragment is closed with `CAN` before the control, and
        // the removed offer leaves one of its own.
        let mut stream = b"\x1b]0;x\x1b\x01]52;c;".to_vec();
        stream.extend_from_slice(payload.as_bytes());
        stream.extend_from_slice(b"\x07visible");
        let (fed, writes) = run(&stream);
        assert_eq!(writes, ["hello"]);
        assert_eq!(fed, b"\x1b]0;x\x18\x01\x18visible");
    }

    #[test]
    fn a_control_after_esc_that_starts_nothing_keeps_the_sequence_for_vte() {
        // Nothing is removed: the `ESC` is released with whatever follows.
        let (fed, writes) = run(b"\x1b\x07[2Jtext");
        assert_eq!(fed, b"\x07\x1b[2Jtext");
        assert!(writes.is_empty());
        // CAN and SUB end the escape in VTE, so they are not transparent.
        let (fed, _) = run(b"\x1b\x18]52;c;aGk=\x07");
        assert_eq!(fed, b"\x1b\x18]52;c;aGk=\x07");
        let (fed, _) = run(b"\x1b\x1a]52;c;aGk=\x07");
        assert_eq!(fed, b"\x1b\x1a]52;c;aGk=\x07");
    }

    #[test]
    fn an_oversized_string_is_discarded_and_the_next_one_still_works() {
        let mut stream = vec![b'\x1b', b']', b'0', b';'];
        stream.extend(std::iter::repeat_n(b'x', MAX_OSC_BODY + 16));
        stream.push(0x07);
        stream.extend_from_slice(b"visible");
        stream.extend_from_slice(&offer("after the flood"));

        let (fed, writes) = run(&stream);
        assert_eq!(writes, ["after the flood"], "the filter did not recover");
        assert_eq!(fed, b"\x18visible\x18");
    }

    #[test]
    fn a_query_is_never_answered() {
        let (fed, writes) = run(b"\x1b]52;c;?\x07");
        assert!(
            writes.is_empty(),
            "the local clipboard must not be sent out"
        );
        assert_eq!(fed, [CAN]);
    }

    #[test]
    fn the_primary_and_select_selections_are_left_alone() {
        let payload = glib::base64_encode(b"yank");
        for selection in ["p", "s", "q0", "0"] {
            let stream = format!("\x1b]52;{selection};{payload}\x07").into_bytes();
            let (fed, writes) = run(&stream);
            assert!(
                writes.is_empty(),
                "selection {selection} must not be written"
            );
            assert_eq!(fed, stream, "selection {selection} must be passed on");
        }
    }

    #[test]
    fn a_payload_that_is_not_text_never_reaches_the_clipboard() {
        // Valid base64 of bytes that are not valid UTF-8.
        let payload = glib::base64_encode(&[0xff, 0xfe, 0xfd]);
        let stream = format!("\x1b]52;c;{payload}\x07").into_bytes();
        let (fed, writes) = run(&stream);
        assert!(writes.is_empty());
        assert_eq!(fed, stream);
    }

    #[test]
    fn an_empty_payload_clears_the_clipboard() {
        let (fed, writes) = run(b"\x1b]52;c;\x07");
        assert_eq!(writes, [""]);
        assert_eq!(fed, [CAN]);
    }

    #[test]
    fn nothing_is_removed_while_the_setting_is_off() {
        let sequence = offer("yank");
        set_enabled(false);
        let mut filter = Osc52Filter::default();
        let mut writes = Vec::new();
        let mut sink = |text: &str| writes.push(text.to_owned());
        let fed = filter.push(&sequence, &mut sink).to_vec();
        assert_eq!(fed, sequence, "the sequence must reach VTE untouched");
        assert_eq!(writes, Vec::<String>::new());

        // And an offer split across chunks stays whole on the way to VTE.
        set_enabled(false);
        let mut filter = Osc52Filter::default();
        let mut fed = Vec::new();
        for byte in &sequence {
            let mut sink = |text: &str| writes.push(text.to_owned());
            fed.extend_from_slice(filter.push(std::slice::from_ref(byte), &mut sink));
        }
        assert_eq!(fed, sequence);
        assert_eq!(writes, Vec::<String>::new());
    }

    #[test]
    fn the_setting_is_read_per_sequence_so_it_applies_to_open_sessions() {
        set_enabled(false);
        let mut filter = Osc52Filter::default();
        let mut writes = Vec::new();
        let mut sink = |text: &str| writes.push(text.to_owned());

        // Off: through to VTE.
        let mut fed = filter.push(&offer("first"), &mut sink).to_vec();
        // The user turns it on in Preferences mid-session.
        set_enabled(true);
        fed.extend_from_slice(filter.push(&offer("second"), &mut sink));

        assert_eq!(writes, ["second"]);
        // The first offer as it came, then the `CAN` that stands for the second.
        assert_eq!(fed, [offer("first"), vec![CAN]].concat());
    }

    /// Builds `OSC 52 ; c ; <base64 of "hello">` with the framing asked for, as
    /// raw bytes.
    ///
    /// A C1 control in a terminal stream is the UTF-8 encoding of its code point
    /// (`c2 9c` for `ST`, as `"\u{9c}".as_bytes()`), because VTE decodes UTF-8
    /// before it parses. A bare `0x9c` is a continuation byte, not a control.
    fn framed_offer(introducer: &[u8], terminator: &[u8]) -> Vec<u8> {
        let mut sequence = introducer.to_vec();
        sequence.extend_from_slice(b"52;c;");
        sequence.extend_from_slice(glib::base64_encode(b"hello").as_bytes());
        sequence.extend_from_slice(terminator);
        sequence
    }

    /// Every introducer VTE accepts, and every terminator.
    const INTRODUCERS: [&[u8]; 2] = [b"\x1b]", b"\xc2\x9d"];
    const TERMINATORS: [&[u8]; 3] = [b"\x1b\\", b"\x07", b"\xc2\x9c"];

    #[test]
    fn both_terminators_and_both_introducers_are_understood() {
        for introducer in INTRODUCERS {
            for terminator in TERMINATORS {
                let sequence = framed_offer(introducer, terminator);
                let (fed, writes) = run(&sequence);
                assert_eq!(writes, ["hello"], "{sequence:?} was not understood");
                assert_eq!(fed, [CAN], "{sequence:?} reached VTE");
            }
        }
    }

    #[test]
    fn a_raw_c1_byte_is_a_utf8_continuation_not_a_control() {
        // A bare 0x9c does not end a string: VTE reads it as part of a character
        // or as U+FFFD. The payload is therefore still open, so nothing is
        // written, and the stream ended inside it. With offers honoured the open
        // string is dropped; with the setting off it all reaches VTE unchanged.
        let mut sequence = b"\x1b]52;c;".to_vec();
        sequence.extend_from_slice(glib::base64_encode(b"hello").as_bytes());
        sequence.push(0x9c);
        sequence.extend_from_slice(b"rest");
        let (fed, writes) = run(&sequence);
        assert!(writes.is_empty(), "a bare 0x9c ended the string");
        assert_eq!(fed, [CAN], "an open string was released");
        set_enabled(false);
        let mut filter = Osc52Filter::default();
        let mut sink = |_: &str| {};
        let mut fed = filter.push(&sequence, &mut sink).to_vec();
        fed.extend_from_slice(filter.finish());
        assert_eq!(fed, sequence);

        // Nor does a bare 0x9d introduce one.
        let mut sequence = vec![0x9d];
        sequence.extend_from_slice(b"52;c;aGVsbG8=\x07");
        for enabled in [false, true] {
            assert_eq!(pushed_once(enabled, &sequence), sequence);
        }
        let (fed, writes) = run(&sequence);
        assert!(writes.is_empty(), "a bare 0x9d introduced a string");
        assert_eq!(fed, sequence);
    }

    #[test]
    fn the_utf8_encoded_st_terminates_like_vte() {
        let mut sequence = b"\x1b]52;c;".to_vec();
        sequence.extend_from_slice(glib::base64_encode(b"hello").as_bytes());
        sequence.extend_from_slice("\u{9c}".as_bytes());
        let (fed, writes) = run(&sequence);
        assert_eq!(writes, ["hello"]);
        assert_eq!(fed, [CAN], "the sequence reached VTE");
    }

    #[test]
    fn body_of_strips_only_the_framing() {
        assert_eq!(body_of(b"\x1b]0;title\x07"), Some(&b"0;title"[..]));
        assert_eq!(body_of(b"\x1b]0;title\x1b\\"), Some(&b"0;title"[..]));
        assert_eq!(body_of(b"\xc2\x9d0;title\xc2\x9c"), Some(&b"0;title"[..]));
        assert_eq!(body_of(b"\xc2\x9d0;title\x07"), Some(&b"0;title"[..]));
        assert_eq!(body_of(b"\x9d0;title\x9c"), None);
        assert_eq!(body_of(b"\x1b[2J"), None);
        assert_eq!(body_of(b"\x1b]"), None);
    }

    #[test]
    fn an_arrow_glyph_inside_a_title_does_not_end_it_early() {
        // `➜` is e2 9e 9c: its last byte is a continuation byte, not `ST`.
        let mut stream = "\x1b]0;\u{279c} dir\x07".as_bytes().to_vec();
        stream.extend_from_slice(&offer("yank"));
        let (fed, writes) = run(&stream);
        assert_eq!(fed, "\x1b]0;\u{279c} dir\x07\x18".as_bytes());
        assert_eq!(writes, ["yank"]);
    }

    #[test]
    fn a_cyrillic_title_with_0x9c_and_0x9d_bytes_stays_whole() {
        // `ѝ` is d1 9d and `ќ` is d1 9c.
        for introducer in INTRODUCERS {
            for terminator in TERMINATORS {
                let mut stream = introducer.to_vec();
                stream.extend_from_slice("0;\u{45d}\u{45c} сервер".as_bytes());
                stream.extend_from_slice(terminator);
                let (fed, writes) = run(&stream);
                assert_eq!(fed, stream, "the title was altered");
                assert!(writes.is_empty());
            }
        }
    }

    #[test]
    fn can_and_sub_abort_an_osc_string_like_vte() {
        let payload = glib::base64_encode(b"hello");
        for abort in [&b"\x18"[..], b"\x1a", b"\xc2\x9b", b"\xc2\x85", b"\xc2\x9e"] {
            let mut stream = format!("\x1b]52;c;{payload}").into_bytes();
            stream.extend_from_slice(abort);
            stream.extend_from_slice("текст".as_bytes());
            // Everything is released by the one `push`, no `finish` needed.
            assert_eq!(pushed_once(true, &stream), stream, "{abort:?} held bytes");
            let (fed, writes) = run(&stream);
            assert_eq!(fed, stream);
            assert!(writes.is_empty(), "{abort:?} did not abort the string");
        }
    }

    #[test]
    fn a_c1_osc_inside_an_osc_starts_a_new_one() {
        let mut stream = b"\x1b]0;x".to_vec();
        stream.extend_from_slice(b"\xc2\x9d52;c;");
        stream.extend_from_slice(glib::base64_encode(b"hello").as_bytes());
        stream.extend_from_slice("\x07\u{276f} visible".as_bytes());
        let (fed, writes) = run(&stream);
        assert_eq!(fed, "\x1b]0;x\x18\u{276f} visible".as_bytes());
        assert_eq!(writes, ["hello"]);
    }

    #[test]
    fn an_offer_right_after_an_abandoned_string_is_still_honoured() {
        let mut stream = b"\x1b]0;x".to_vec();
        stream.extend_from_slice(&offer("hello"));
        stream.extend_from_slice("\u{276f} visible".as_bytes());
        let (fed, writes) = run(&stream);
        // The fragment is closed once by the abandon and once by the removal.
        assert_eq!(fed, "\x1b]0;x\x18\x18\u{276f} visible".as_bytes());
        assert_eq!(writes, ["hello"]);
    }

    #[test]
    fn only_an_incomplete_prefix_is_held_at_a_chunk_end() {
        // Text, a split glyph and a split `❯`: VTE draws what it can, so the
        // filter holds nothing.
        for chunk in [&b"text \xe2"[..], b"text \xe2\x9d", b"text \xd1"] {
            assert_eq!(pushed_once(true, chunk), chunk, "{chunk:?} was held");
        }
        // A lone `c2` or `ESC` is a prefix VTE also waits on.
        for chunk in [&b"text \xc2"[..], b"text \x1b"] {
            assert_eq!(pushed_once(true, chunk), b"text ", "{chunk:?}");
        }
        // And `finish` releases it.
        for held in [&b"\xc2"[..], b"\x1b"] {
            let (fed, writes) = run(held);
            assert_eq!(fed, held);
            assert!(writes.is_empty());
        }
        // A control after the held `ESC` goes out at once; the `ESC` stays.
        assert_eq!(pushed_once(true, b"text \x1b\x07"), b"text \x07");
    }

    #[test]
    fn the_oversized_flag_is_cleared_when_a_string_is_abandoned() {
        let mut stream = b"\x1b]0;".to_vec();
        stream.extend(std::iter::repeat_n(b'x', MAX_OSC_BODY + 16));
        stream.push(0x18);
        stream.extend_from_slice(&offer("after"));
        let (_, writes) = run(&stream);
        assert_eq!(writes, ["after"]);
    }

    /// Streams where a sequence the filter removes follows an abandoned string
    /// or a dangling `c2`: the bytes that ended the string in VTE travel with
    /// the removed sequence, so the filter has to end it itself. `hello` is
    /// `aGVsbG8=` in base64.
    fn review_streams() -> Vec<Vec<u8>> {
        let hello = glib::base64_encode(b"hello");
        vec![
            b"\x1b]52;c;QUFB\x1b]52;c;Qg==\x07\x07".to_vec(),
            format!("\x1b]52;c;{hello}\x1b\0\\").into_bytes(),
            format!("\x1b]52;c;{hello}\x1b\x01\\").into_bytes(),
            b"\x1b]52;c;?\x1b\x07".to_vec(),
            b"\x1b]52;c;?\x1b\0\\".to_vec(),
            format!("\u{9d}52;c;{hello}\x1b]52;c;{hello}\x07\x07").into_bytes(),
            format!("\x1b]0;x\u{9d}52;c;{hello}\x07").into_bytes(),
            format!("\x1b]0;x\x1b\x01]52;c;{hello}\x07").into_bytes(),
            format!("\x1b]0;x\x1b\x01\x1b]52;c;{hello}\x07").into_bytes(),
            format!("\u{9d}52;c;{hello}\u{9d}52;c;{hello}\u{9c}\u{9c}").into_bytes(),
            // An `ESC` held in front of a C1 `OSC` that is then removed.
            format!("\x1b\u{9d}52;c;{hello}\u{9c}]52;c;{hello}\x07").into_bytes(),
            format!("\x1b]0;x\x1b\u{9d}52;c;{hello}\u{9c}]52;c;{hello}\x07").into_bytes(),
            format!("\x1b\u{9d}52;c;{hello}\x07c").into_bytes(),
            format!("\x1b\u{9d}52;c;{hello}\x07[").into_bytes(),
            // A broken UTF-8 prefix in front of a removed sequence.
            [&b"\xe2\x1b]52;c;"[..], hello.as_bytes(), b"\x07\x9d\xaf"].concat(),
            // A bare `c2` before a removed sequence, then a bare `9d`.
            [
                &b"\xc2\x1b]52;c;QQ==\x07\x9d52;c;"[..],
                hello.as_bytes(),
                b"\x07",
            ]
            .concat(),
        ]
    }

    /// Streams that exercise every transition of the filter.
    fn corpus() -> Vec<Vec<u8>> {
        let hello = glib::base64_encode(b"hello");
        let mut corpus: Vec<Vec<u8>> = vec![
            "\x1b]0;anton@pop-os: ~\x07\u{256d}\u{2500} anton\r\n\u{2570}\u{276f} "
                .as_bytes()
                .to_vec(),
            "\x1b]0;\u{279c} dir\x07\x1b]0;\u{279c} dir\x1b\\"
                .as_bytes()
                .to_vec(),
            "\x1b]0;\u{45d}\u{45c} сервер\x07\u{255d}\u{f09d}\u{45d}"
                .as_bytes()
                .to_vec(),
            b"\xc2".to_vec(),
            b"\xc2\xc2\x9d0;x\x07".to_vec(),
            b"\xc2\x1b]0;x\x07".to_vec(),
            b"\x1b\x1b]0;x\x07".to_vec(),
            b"\x1b]0;x\x1b[2Jtext".to_vec(),
            b"\x1b]0;x\x18text\x1b]0;y\x1atext".to_vec(),
            b"\x1b]0;x\xc2\x9btext\xc2\xa0nbsp\xc2\x07".to_vec(),
            b"\x1b]0;x\xc2\x9d0;y\xc2\x9c".to_vec(),
            b"\x1b]0;\x9c\x9d\xc2\xc2\x9c".to_vec(),
        ];
        for introducer in INTRODUCERS {
            for terminator in TERMINATORS {
                corpus.push(framed_offer(introducer, terminator));
                let mut around = b"before ".to_vec();
                around.extend_from_slice(&framed_offer(introducer, terminator));
                around.extend_from_slice("\u{276f} after".as_bytes());
                corpus.push(around);
            }
        }
        // The streams from the review of 2a135c36, each followed by a prompt.
        for stream in review_streams() {
            corpus.push([stream.as_slice(), "\u{276f} ".as_bytes()].concat());
        }
        let mut aborted = b"\x1b]52;c;".to_vec();
        aborted.extend_from_slice(hello.as_bytes());
        aborted.push(0x18);
        aborted.extend_from_slice(&offer("again"));
        corpus.push(aborted);
        corpus
    }

    /// Streams where a C0 control meets a held `ESC` or ignored controls sit in
    /// a string.
    fn escape_corpus() -> Vec<Vec<u8>> {
        let hello = glib::base64_encode(b"hello");
        let mut corpus = vec![
            b"\x1b]0;x\x1b\x07tail".to_vec(),
            b"\x1b\x07[2Jtext".to_vec(),
            b"\x1b\x00\x7f\x19\x1f]0;x\x07".to_vec(),
            b"\x1b\x18]0;x\x07\x1b\x1a]0;y\x07".to_vec(),
            b"\x1b\xc2\x9d0;x\x07".to_vec(),
        ];
        for noise in ["\x00", "\x01", "\n", "\x7f"] {
            corpus.push(format!("\x1b]5{noise}2;c;{hello}{noise}\x07").into_bytes());
            corpus.push(format!("\x1b{noise}]52;c;{hello}\x1b\\ after").into_bytes());
            corpus.push(format!("\x1b]0;x\x1b{noise}]52;c;{hello}\x07").into_bytes());
            corpus.push(format!("\x1b]52;c;{hello}\x1b{noise}\\ after").into_bytes());
            corpus.push(format!("\u{9d}52;{noise}c;{hello}\u{9c}").into_bytes());
        }
        corpus
    }

    /// Runs `chunks` through one filter, `finish` included.
    fn feed_chunks(enabled: bool, chunks: Vec<&[u8]>) -> (Vec<u8>, Vec<String>) {
        set_enabled(enabled);
        let mut filter = Osc52Filter::default();
        let mut fed = Vec::new();
        let mut writes = Vec::new();
        for chunk in chunks {
            let mut sink = |text: &str| writes.push(text.to_owned());
            fed.extend_from_slice(filter.push(chunk, &mut sink));
        }
        fed.extend_from_slice(filter.finish());
        (fed, writes)
    }

    /// A model of the part of VTE's input path the filter has to agree with, for
    /// the tests to compare a stream before and after filtering.
    ///
    /// It is a model and not a port: UTF-8 decoding as `process_incoming_utf8`
    /// does it (a malformed prefix is `U+FFFD`, and the byte that broke it is
    /// read again), then the `GROUND` / `ESCAPE` / `OSC_STRING` / `ST_ESC`
    /// states of `Parser::feed` and `feed_to_state` (VTE 0.76–0.84, see
    /// `.agents/tasks/osc52-c1/vte-analysis.md`). CSI, DCS and the rest are not
    /// modelled: `ESC` + anything but `]` simply ends in `GROUND` with that
    /// byte consumed. Both sides of every comparison go through the same
    /// model, so what it leaves out cannot hide a difference the filter makes.
    mod vte_model {
        /// What the parser did on the ground.
        #[derive(Debug, PartialEq, Eq)]
        pub enum Event {
            /// A character was drawn.
            Print(u32),
            /// A C0 or C1 control was executed.
            Exec(u32),
        }

        /// What a stream did to the parser.
        #[derive(Debug, Default, PartialEq, Eq)]
        pub struct Outcome {
            /// Drawn characters and executed controls, in order.
            pub events: Vec<Event>,
            /// The payload of every OSC string that was dispatched.
            pub osc: Vec<Vec<u32>>,
            /// The stream ended inside an OSC string.
            pub in_string: bool,
        }

        #[derive(Clone, Copy)]
        enum State {
            Ground,
            Escape,
            OscString,
            StEscape,
        }

        const REPLACEMENT: u32 = 0xfffd;

        /// Decodes `bytes` as VTE does: UTF-8 in, code points out.
        fn decode(bytes: &[u8]) -> Vec<u32> {
            let mut cps = Vec::new();
            let (mut need, mut acc, mut min) = (0u8, 0u32, 0u32);
            for &byte in bytes {
                let mut again = Some(byte);
                while let Some(byte) = again.take() {
                    if need > 0 {
                        if byte & 0xc0 == 0x80 {
                            acc = acc << 6 | u32::from(byte & 0x3f);
                            need -= 1;
                            if need == 0 {
                                let valid = acc >= min
                                    && acc <= 0x10_ffff
                                    && !(0xd800..0xe000).contains(&acc);
                                cps.push(if valid { acc } else { REPLACEMENT });
                            }
                        } else {
                            // The byte broke the character: `U+FFFD`, and the
                            // byte is read again from the start.
                            need = 0;
                            cps.push(REPLACEMENT);
                            again = Some(byte);
                        }
                    } else {
                        match byte {
                            0x00..=0x7f => cps.push(u32::from(byte)),
                            0xc2..=0xdf => (need, acc, min) = (1, u32::from(byte & 0x1f), 0x80),
                            0xe0..=0xef => (need, acc, min) = (2, u32::from(byte & 0x0f), 0x800),
                            0xf0..=0xf4 => (need, acc, min) = (3, u32::from(byte & 0x07), 0x1_0000),
                            _ => cps.push(REPLACEMENT),
                        }
                    }
                }
            }
            if need > 0 {
                cps.push(REPLACEMENT);
            }
            cps
        }

        /// Runs `bytes` through the model.
        pub fn run(bytes: &[u8]) -> Outcome {
            let mut outcome = Outcome::default();
            let mut state = State::Ground;
            let mut body = Vec::new();
            for cp in decode(bytes) {
                let mut again = Some(cp);
                while let Some(cp) = again.take() {
                    match cp {
                        // `DEL` is ignored in every state.
                        0x7f => {}
                        // `CAN` abandons whatever is open and is ignored; `SUB`
                        // abandons it and is executed.
                        0x18 => state = State::Ground,
                        0x1a => {
                            state = State::Ground;
                            outcome.events.push(Event::Exec(cp));
                        }
                        // C1 `OSC` starts a string, whatever the state.
                        0x9d => {
                            state = State::OscString;
                            body.clear();
                        }
                        // Every C1 but `ST` and `OSC` ends what is open.
                        0x80..=0x9b | 0x9e | 0x9f => {
                            state = State::Ground;
                            outcome.events.push(Event::Exec(cp));
                        }
                        _ => match state {
                            State::Ground => match cp {
                                0x1b => state = State::Escape,
                                0x9c => {}
                                0x00..=0x1f => outcome.events.push(Event::Exec(cp)),
                                _ => outcome.events.push(Event::Print(cp)),
                            },
                            State::Escape => match cp {
                                0x5d => {
                                    state = State::OscString;
                                    body.clear();
                                }
                                0x1b => {}
                                0x00..=0x1f => outcome.events.push(Event::Exec(cp)),
                                _ => state = State::Ground,
                            },
                            State::OscString => match cp {
                                0x07 | 0x9c => {
                                    outcome.osc.push(std::mem::take(&mut body));
                                    state = State::Ground;
                                }
                                0x1b => state = State::StEscape,
                                0x00..=0x1f => {}
                                _ => body.push(cp),
                            },
                            State::StEscape => {
                                if cp == 0x5c {
                                    outcome.osc.push(std::mem::take(&mut body));
                                    state = State::Ground;
                                } else {
                                    // The string is dropped and the byte is
                                    // read as the one after an `ESC`.
                                    state = State::Escape;
                                    again = Some(cp);
                                }
                            }
                        },
                    }
                }
            }
            outcome.in_string = matches!(state, State::OscString | State::StEscape);
            outcome
        }
    }

    /// Checks that `filtered` is what VTE should be fed for `raw`: the same text
    /// and controls, the same OSC strings but for the clipboard offers the
    /// filter took (all of them, or none while the setting is off), and no
    /// string left open that the raw stream did not leave open.
    fn check_vte_equivalence(raw: &[u8], filtered: &[u8], enabled: bool) -> Result<(), String> {
        let (raw_out, filtered_out) = (vte_model::run(raw), vte_model::run(filtered));
        if raw_out.events != filtered_out.events {
            return Err(format!(
                "ground events differ: {:?} vs {:?}",
                raw_out.events, filtered_out.events
            ));
        }
        if filtered_out.in_string && !raw_out.in_string {
            return Err("the filtered stream ends inside a string".to_owned());
        }
        if !enabled {
            if raw_out.osc != filtered_out.osc {
                return Err(format!(
                    "dispatched strings differ: {:?} vs {:?}",
                    raw_out.osc, filtered_out.osc
                ));
            }
            return Ok(());
        }
        // The filtered stream dispatches a subsequence of the raw stream's
        // strings, and every one it dropped is a `52;c;` clipboard offer.
        let offer: Vec<u32> = "52;c;".chars().map(u32::from).collect();
        let mut kept = filtered_out.osc.iter().peekable();
        for payload in &raw_out.osc {
            if kept.peek() == Some(&payload) {
                kept.next();
            } else if !payload.starts_with(&offer) {
                return Err(format!("the filter dropped {payload:?}"));
            }
        }
        if let Some(extra) = kept.next() {
            return Err(format!(
                "the filter let through {extra:?}, not in the raw stream"
            ));
        }
        Ok(())
    }

    /// [`check_vte_equivalence`] as an assertion.
    fn assert_vte_equivalent(raw: &[u8], filtered: &[u8], enabled: bool) {
        if let Err(why) = check_vte_equivalence(raw, filtered, enabled) {
            panic!("{why} (enabled = {enabled})\n raw: {raw:?}\n fed: {filtered:?}");
        }
    }

    #[test]
    fn every_split_of_the_corpus_matches_the_whole_feed() {
        for stream in corpus().into_iter().chain(escape_corpus()) {
            for enabled in [true, false] {
                let whole = feed_chunks(enabled, vec![&stream]);
                assert_eq!(
                    feed_chunks(enabled, stream.chunks(1).collect()),
                    whole,
                    "byte-at-a-time differs for {stream:?} (enabled = {enabled})"
                );
                for split in 1..stream.len() {
                    assert_eq!(
                        feed_chunks(enabled, vec![&stream[..split], &stream[split..]]),
                        whole,
                        "split at {split} differs for {stream:?} (enabled = {enabled})"
                    );
                }
                assert_vte_equivalent(&stream, &whole.0, enabled);
                if !enabled {
                    assert!(whole.1.is_empty());
                }
            }
        }
    }

    #[test]
    fn the_vte_model_agrees_with_what_the_analysis_says_of_vte() {
        use vte_model::{Event::Print, run};
        // `❯ ` is three bytes of one character, and nothing is a string.
        let prompt = run("\u{276f} ".as_bytes());
        assert_eq!(prompt.events, [Print(0x276f), Print(0x20)]);
        assert!(prompt.osc.is_empty() && !prompt.in_string);
        // A bare `9d` is `U+FFFD`; `c2 9d` opens a string that `c2 9c` ends.
        assert_eq!(run(b"\x9d").events, [Print(0xfffd)]);
        let title = run(b"\xc2\x9d0;t\xc2\x9c");
        assert_eq!(title.osc, [[0x30, 0x3b, 0x74]]);
        // `ESC` + anything but `\` drops the string; `CAN` does too.
        assert!(run(b"\x1b]0;x\x1b[").osc.is_empty());
        assert!(!run(b"\x1b]0;x\x18").in_string);
        assert!(run(b"\x1b]0;x").in_string);
        // A `c2` with no continuation is `U+FFFD`, and the next byte is read again.
        assert_eq!(run(b"\xc2A").events, [Print(0xfffd), Print(0x41)]);
    }

    /// Pieces that make the filter's sequences turn up often in generated input:
    /// introducers, terminators, the aborting controls, offers and queries, the
    /// glyphs that carry a `9c`/`9d` continuation byte, and lone UTF-8 leads.
    const TOKENS: [&[u8]; 23] = [
        b"\x1b]",
        b"\xc2\x9d",
        b"52;c;",
        b"52;p;",
        b"aGk=",
        b"?",
        b"0;title",
        b"\x07",
        b"\x1b\\",
        b"\xc2\x9c",
        b"\x18",
        b"\x1a",
        b"\x1b",
        b"\x1b[",
        b"\xc2",
        b"\xe2",
        b"\x9d",
        b"\x9c",
        "\u{276f}".as_bytes(),
        "\u{45d}".as_bytes(),
        b"c",
        b"[",
        b"A",
    ];

    /// Whole sequences, for [`framed_stream`]: what may precede one, how it may
    /// open, how it may end, and what is drawn after it.
    const LEADS: [&[u8]; 6] = [b"", b"", b"\x1b", b"\xc2", b"\xe2", b"\x1b]0;x"];
    const OPENERS: [&[u8]; 6] = [
        b"\x1b]52;c;aGk=",
        b"\xc2\x9d52;c;aGk=",
        b"\x1b]52;c;?",
        b"\xc2\x9d52;c;?",
        b"\x1b]0;title",
        b"\xc2\x9d52;p;aGk=",
    ];
    const CLOSERS: [&[u8]; 7] = [
        b"\x07",
        b"\x1b\\",
        b"\xc2\x9c",
        b"\x18",
        b"\x1b",
        b"\x1b\x01\\",
        b"",
    ];
    const TAILS: [&[u8]; 8] = [
        b"",
        "\u{276f} ".as_bytes(),
        b"c",
        b"[",
        b"]52;c;aGk=\x07",
        b"\x9d\xaf",
        b"\x9d52;c;aGk=\x07",
        b"A",
    ];

    /// A stream built from [`TOKENS`] and the odd random byte, so that the
    /// pieces of a sequence turn up in every order.
    fn token_stream() -> impl proptest::strategy::Strategy<Value = Vec<u8>> {
        use proptest::prelude::*;
        let token = prop_oneof![
            8 => proptest::sample::select(TOKENS.to_vec()).prop_map(<[u8]>::to_vec),
            1 => any::<u8>().prop_map(|byte| vec![byte]),
        ];
        proptest::collection::vec(token, 0..24).prop_map(|tokens| tokens.concat())
    }

    /// A stream of whole sequences — offers, queries, titles — each with
    /// something in front of it, a way of ending (or not ending) and a tail, so
    /// that removals and what is released around them are the rule and not a
    /// one-in-a-million coincidence.
    fn framed_stream() -> impl proptest::strategy::Strategy<Value = Vec<u8>> {
        use proptest::prelude::*;
        let segment = (
            proptest::sample::select(LEADS.to_vec()),
            proptest::sample::select(OPENERS.to_vec()),
            proptest::sample::select(CLOSERS.to_vec()),
            proptest::sample::select(TAILS.to_vec()),
        )
            .prop_map(|(lead, opener, closer, tail)| {
                let mut segment = lead.to_vec();
                for part in [opener, closer, tail] {
                    segment.extend_from_slice(part);
                }
                segment
            });
        proptest::collection::vec(segment, 1..4).prop_map(|segments| segments.concat())
    }

    /// Either kind of generated stream.
    fn any_stream() -> impl proptest::strategy::Strategy<Value = Vec<u8>> {
        use proptest::prelude::*;
        prop_oneof![token_stream(), framed_stream()]
    }

    /// Cuts `stream` at the given points (each reduced to a valid offset).
    fn cut_at<'a>(stream: &'a [u8], cuts: &[usize]) -> Vec<&'a [u8]> {
        let mut points: Vec<usize> = cuts.iter().map(|cut| cut % (stream.len() + 1)).collect();
        points.sort_unstable();
        let mut chunks = Vec::new();
        let mut start = 0;
        for point in points {
            chunks.push(&stream[start..point]);
            start = point;
        }
        chunks.push(&stream[start..]);
        chunks
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(10_000))]

        #[test]
        fn chunking_never_changes_the_result(
            stream in any_stream(),
            cuts in proptest::collection::vec(0usize..256, 0..6),
            enabled in proptest::bool::ANY,
        ) {
            let whole = feed_chunks(enabled, vec![&stream]);
            proptest::prop_assert_eq!(feed_chunks(enabled, cut_at(&stream, &cuts)), whole.clone());
            proptest::prop_assert_eq!(feed_chunks(enabled, stream.chunks(1).collect()), whole.clone());
            proptest::prop_assert!(whole.1.is_empty() || enabled);
        }

        #[test]
        fn the_filtered_stream_is_what_vte_would_have_made_of_the_raw_one(
            stream in any_stream(),
            enabled in proptest::bool::ANY,
        ) {
            let (fed, _) = feed_chunks(enabled, vec![&stream]);
            let verdict = check_vte_equivalence(&stream, &fed, enabled);
            proptest::prop_assert!(verdict.is_ok(), "{:?} -> {:?}: {:?}", stream, fed, verdict);
        }

        #[test]
        fn nothing_is_held_back_while_the_filter_is_on_the_ground(
            stream in any_stream(),
            cuts in proptest::collection::vec(0usize..256, 0..6),
            enabled in proptest::bool::ANY,
        ) {
            set_enabled(enabled);
            let mut filter = Osc52Filter::default();
            let mut sink = |_: &str| {};
            let (mut released, mut start) = (Vec::new(), 0);
            for chunk in cut_at(&stream, &cuts) {
                released.extend_from_slice(filter.push(chunk, &mut sink));
                start += chunk.len();
                // Only an open string, a lone `ESC` or a lone `c2` may be held.
                // Anything else has reached VTE, which has made the same of it
                // as of the raw bytes so far.
                if filter.state == State::Ground {
                    let (raw, fed) = (vte_model::run(&stream[..start]), vte_model::run(&released));
                    proptest::prop_assert_eq!(&fed.events, &raw.events);
                    if !enabled {
                        proptest::prop_assert_eq!(fed.osc, raw.osc);
                    }
                }
            }
        }
    }

    /// What one `push` hands back for `bytes`, with no `finish`, so a held byte
    /// shows up as a short result.
    fn pushed_once(enabled: bool, bytes: &[u8]) -> Vec<u8> {
        set_enabled(enabled);
        let mut filter = Osc52Filter::default();
        let mut sink = |_: &str| {};
        filter.push(bytes, &mut sink).to_vec()
    }

    #[test]
    fn a_prompt_glyph_with_a_0x9d_continuation_passes_through_at_once() {
        // The oh-my-bash `powerline-multiline` prompt: `❯` is e2 9d af, and its
        // 0x9d is a UTF-8 continuation byte, not a C1 OSC introducer.
        let stream =
            "\x1b]0;anton@pop-os: ~\x07\u{256d}\u{2500} anton\r\n\u{2570}\u{276f} ".as_bytes();
        for enabled in [false, true] {
            assert_eq!(
                pushed_once(enabled, stream),
                stream,
                "the prompt line was held back (enabled = {enabled})"
            );
        }
    }

    #[test]
    fn box_drawing_and_nerd_font_glyphs_are_not_held() {
        // ╝ e2 95 9d, ┝ e2 94 9d, U+F09D ef 82 9d, ѝ d1 9d.
        let stream = "a \u{255d} b \u{251d} c \u{f09d} d \u{45d} e".as_bytes();
        for enabled in [false, true] {
            assert_eq!(pushed_once(enabled, stream), stream, "enabled = {enabled}");
        }
    }

    /// The result of filtering `stream` and putting the output through the VTE
    /// model: what it dispatched and drew, and the offers the filter took.
    fn filtered_outcome(enabled: bool, stream: &[u8]) -> (vte_model::Outcome, Vec<String>) {
        let (fed, writes) = feed_chunks(enabled, vec![stream]);
        assert_vte_equivalent(stream, &fed, enabled);
        (vte_model::run(&fed), writes)
    }

    /// The code points of an ASCII payload, as the VTE model reports them.
    fn payload(text: &str) -> Vec<u32> {
        text.chars().map(u32::from).collect()
    }

    #[test]
    fn a_removed_sequence_cannot_leave_an_abandoned_string_open() {
        // Both strings are cut short by the second `ESC ]`, which the filter then
        // removes as an offer. VTE must still see the first one end, or it keeps
        // collecting the prompt and dispatches fragment and prompt at a `BEL`.
        let prompt = "\u{276f} ";
        let stream = format!("\x1b]52;c;QUFB\x1b]52;c;Qg==\x07\x07{prompt}");
        let drawn: Vec<_> = prompt.chars().map(u32::from).collect();

        let (on, writes) = filtered_outcome(true, stream.as_bytes());
        assert_eq!(writes, ["B"]);
        assert!(on.osc.is_empty(), "VTE was handed an OSC 52: {:?}", on.osc);
        assert!(!on.in_string);
        let printed: Vec<_> = on
            .events
            .iter()
            .filter_map(|event| match event {
                vte_model::Event::Print(cp) => Some(*cp),
                vte_model::Event::Exec(_) => None,
            })
            .collect();
        assert_eq!(printed, drawn, "the prompt was swallowed");

        // Off: the offer is VTE's to dispatch, and nothing else is.
        let (off, writes) = filtered_outcome(false, stream.as_bytes());
        assert!(writes.is_empty() && !off.in_string);
        assert_eq!(off.osc, [payload("52;c;Qg==")]);
        assert_eq!(off.events, on.events);
    }

    #[test]
    fn a_control_after_an_abandoned_string_cannot_turn_it_into_a_dispatch() {
        // `ESC NUL \` and `ESC BEL` after the fragment are discarded by VTE. The
        // filter holds the `ESC` and passes the control, so unless the fragment
        // is closed first the control would land inside the string and the `ST`
        // or `BEL` would dispatch it.
        let hello = glib::base64_encode(b"hello");
        let cases = [
            format!("\x1b]52;c;{hello}\x1b\0\\"),
            format!("\x1b]52;c;{hello}\x1b\x01\\"),
            "\x1b]52;c;?\x1b\x07".to_owned(),
            "\x1b]52;c;?\x1b\0\\".to_owned(),
        ];
        for case in cases {
            let stream = format!("{case}\u{276f} ");
            for enabled in [true, false] {
                let (outcome, writes) = filtered_outcome(enabled, stream.as_bytes());
                assert!(writes.is_empty(), "{case:?} was taken as an offer");
                assert!(
                    outcome.osc.is_empty(),
                    "{case:?} (enabled = {enabled}) dispatched {:?}",
                    outcome.osc
                );
                assert!(!outcome.in_string, "{case:?} left a string open");
            }
        }
    }

    #[test]
    fn a_dangling_c2_cannot_pair_with_a_later_9d() {
        // `c2` is followed by an `ESC` that starts a sequence the filter removes,
        // and a bare `9d` follows it. In VTE `c2` is `U+FFFD` and `9d` another;
        // handed over bare, `c2` would meet `9d` in VTE's decoder and open an OSC.
        let hello = glib::base64_encode(b"hello");
        let stream = [
            &b"\xc2\x1b]52;c;QQ==\x07\x9d52;c;"[..],
            hello.as_bytes(),
            "\x07\u{276f} ".as_bytes(),
        ]
        .concat();
        let (fed, writes) = feed_chunks(true, vec![&stream]);
        assert_eq!(writes, ["A"]);
        assert!(
            fed.starts_with(&[0xc2, CAN]),
            "the removed sequence left no CAN behind the c2: {fed:?}"
        );
        for enabled in [true, false] {
            let (outcome, writes) = filtered_outcome(enabled, &stream);
            assert_eq!(writes.len(), usize::from(enabled));
            assert!(!outcome.in_string);
            // The second sequence is text, as it is in the raw stream.
            assert_eq!(
                outcome.osc.len(),
                usize::from(!enabled),
                "enabled = {enabled}"
            );
        }
        // A `c2` that does start a character is left alone, and a lone one at
        // the end of the stream is released by `finish`.
        assert_eq!(pushed_once(true, b"a\xc2\xa0b"), b"a\xc2\xa0b");
        assert_eq!(pushed_once(true, b"a\xc2Ab"), b"a\xc2Ab");
        assert_eq!(run(b"a\xc2").0, b"a\xc2");
    }

    /// What a stream draws, as characters (controls left out).
    fn drawn(outcome: &vte_model::Outcome) -> String {
        outcome
            .events
            .iter()
            .filter_map(|event| match event {
                vte_model::Event::Print(cp) => char::from_u32(*cp),
                vte_model::Event::Exec(_) => None,
            })
            .collect()
    }

    #[test]
    fn an_esc_before_a_removed_c1_osc_cannot_pair_with_what_follows() {
        // In VTE the C1 `OSC` replaces the pending `ESC`. Handed over bare, that
        // `ESC` would pair with the next character once the offer is removed:
        // `ESC ]` makes a complete OSC 52 of the text after the first offer, and
        // `ESC c` / `ESC [` swallow or misread what the user should see.
        let hello = glib::base64_encode(b"hello");
        let offer_c1 = format!("\x1b\u{9d}52;c;{hello}");
        let streams = [
            format!("{offer_c1}\u{9c}]52;c;{hello}\x07"),
            format!("\x1b]0;x{offer_c1}\u{9c}]52;c;{hello}\x07"),
            format!("\x1b]0;x{offer_c1}\x07\u{276f} "),
            format!("{offer_c1}\x07\u{276f} "),
            format!("{offer_c1}\x07c"),
            format!("{offer_c1}\x07["),
            format!("{offer_c1}\u{9c}["),
        ];
        for stream in streams {
            let raw = vte_model::run(stream.as_bytes());
            for enabled in [true, false] {
                let (outcome, writes) = filtered_outcome(enabled, stream.as_bytes());
                assert_eq!(
                    drawn(&outcome),
                    drawn(&raw),
                    "{stream:?} (enabled = {enabled})"
                );
                assert!(!outcome.in_string, "{stream:?} left a string open");
                if enabled {
                    assert_eq!(writes, ["hello"], "{stream:?}");
                    assert!(
                        outcome.osc.is_empty(),
                        "{stream:?} let VTE dispatch {:?}",
                        outcome.osc
                    );
                } else {
                    assert_eq!(outcome.osc, raw.osc, "{stream:?}");
                }
            }
        }
        // The model agrees that the text after the offer is drawn, not eaten.
        let after = vte_model::run(format!("{offer_c1}\x07c").as_bytes());
        assert_eq!(drawn(&after), "c");
    }

    #[test]
    fn a_broken_utf8_prefix_cannot_join_the_bytes_after_a_removed_sequence() {
        // `e2 <offer> 9d af`: on the raw stream the `ESC` breaks the prefix and
        // VTE draws three `U+FFFD`. Without a break the filter would let the
        // continuation bytes complete `❯`.
        let mut stream = b"\xe2".to_vec();
        stream.extend_from_slice(&offer("yank"));
        stream.extend_from_slice(b"\x9d\xaf");
        for enabled in [true, false] {
            let (outcome, writes) = filtered_outcome(enabled, &stream);
            assert_eq!(
                drawn(&outcome),
                "\u{fffd}\u{fffd}\u{fffd}",
                "enabled = {enabled}"
            );
            assert_eq!(writes.len(), usize::from(enabled));
        }
    }

    #[test]
    fn every_removed_sequence_is_replaced_by_one_can() {
        // VTE ignores `CAN` on the ground and, anywhere else, drops the pending
        // `ESC`, string or UTF-8 prefix with it.
        let mut stream = offer("yank");
        stream.extend_from_slice(b"\x1b]52;c;?\x07");
        stream.extend_from_slice(b"\xc2\x9d52;c;?\xc2\x9c");
        assert_eq!(pushed_once(true, &stream), [CAN, CAN, CAN]);
        // The sequences are not removed while the setting is off.
        assert_eq!(pushed_once(false, &stream), stream);
        // An over-long string is discarded, whichever way the setting is.
        let mut flood = b"\x1b]0;".to_vec();
        flood.extend(std::iter::repeat_n(b'x', MAX_OSC_BODY + 16));
        flood.push(0x07);
        for enabled in [true, false] {
            assert_eq!(pushed_once(enabled, &flood), [CAN], "enabled = {enabled}");
        }
        // A string the stream ends inside is dropped by `finish`, with a `CAN`
        // for whatever fragment was released before it.
        assert_eq!(run(b"\x1b]52;c;aGk=").0, [CAN]);
    }

    #[test]
    fn parse_classifies_the_three_outcomes() {
        let write = format!("\x1b]52;c;{}\x07", glib::base64_encode(b"text"));
        assert!(matches!(parse(write.as_bytes()), Parsed::Write(_)));
        assert!(matches!(parse(b"\x1b]52;c;?\x07"), Parsed::Query));
        assert!(matches!(parse(b"\x1b]0;title\x07"), Parsed::Other));
        assert!(matches!(parse(b"\x1b]52;c\x07"), Parsed::Other));
        assert!(matches!(parse(b"\x1b]52;c"), Parsed::Other));
    }
}
