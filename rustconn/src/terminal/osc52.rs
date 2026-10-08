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
/// because the only path here is the one [`deliver_output_to`] runs.
///
/// ponytail: silent, like copy-on-select — a remote yank is not worth a toast,
/// and the paste that follows is the user's own.
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
/// C1 `OSC` — the 8-bit introducer, which some programs emit instead of `ESC ]`.
const INTRODUCER_8BIT: u8 = 0x9d;
/// C1 `ST` — the 8-bit terminator.
const TERMINATOR_8BIT: u8 = 0x9c;
/// `BEL` — xterm accepts it as an OSC terminator and most `printf` recipes use it.
const BEL: u8 = 0x07;

/// Where [`Osc52Filter`] is in the stream.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Between sequences. Bytes go straight to the output.
    #[default]
    Ground,
    /// Saw `ESC`. The next byte decides whether an OSC string starts here;
    /// anything else means this `ESC` opened a different sequence.
    Escape,
    /// Inside an OSC string, collecting its body.
    Osc,
    /// Inside an OSC string, just saw `ESC`: `\` ends the string, and anything
    /// else abandoned it and begins a new escape sequence.
    OscEscape,
}

/// Removes OSC 52 clipboard offers from one PTY output stream.
///
/// A PTY read is bounded by `READ_CHUNK` (8 KiB), which is smaller than a large
/// yank, so a sequence routinely arrives in pieces and the filter has to hold
/// state between chunks. One filter therefore belongs to one stream, and lives
/// as long as the reader does.
///
/// Bytes are never reordered and never dropped unless they belong to a
/// recognised OSC 52 clipboard offer — including the case where the sequence is
/// cut off by the end of the stream, which [`finish`](Self::finish) hands over.
#[derive(Debug, Default)]
pub struct Osc52Filter {
    state: State,
    /// The sequence in progress, introducer included — or just the `ESC` held by
    /// [`State::Escape`].
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
    /// A child that closes its PTY with an OSC string still open leaves control
    /// characters that VTE would have discarded anyway, but they are handed over
    /// rather than kept, so the filter never swallows the tail of a session. An
    /// unterminated string carries no payload to trust, so `sink` is not
    /// involved.
    ///
    /// The result borrows the filter, as [`push`](Self::push)'s does.
    pub fn finish(&mut self) -> &[u8] {
        self.out.clear();
        self.out.append(&mut self.pending);
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
                INTRODUCER_8BIT => {
                    self.pending.push(byte);
                    self.state = State::Osc;
                }
                _ => self.out.push(byte),
            },
            State::Escape => {
                if byte == INTRODUCER_7BIT {
                    self.pending.push(byte);
                    self.state = State::Osc;
                } else {
                    // Not an OSC string — this `ESC` opened a CSI, a charset
                    // selector, a keypad mode or something else. Hand the held
                    // `ESC` over and classify this byte again from the ground,
                    // because `ESC ESC ]` is two sequences and not one.
                    self.out.append(&mut self.pending);
                    self.state = State::Ground;
                    self.step(byte, sink);
                }
            }
            State::Osc => match byte {
                BEL | TERMINATOR_8BIT => {
                    // The terminator joins `pending`, so that `body_of` sees the
                    // same framing whichever of the three ended the string.
                    self.push_body(byte);
                    self.finish_sequence(sink);
                }
                ESC => {
                    self.push_body(byte);
                    self.state = State::OscEscape;
                }
                _ => self.push_body(byte),
            },
            State::OscEscape => {
                if byte == TERMINATOR_7BIT {
                    self.push_body(byte);
                    self.finish_sequence(sink);
                } else {
                    // The string was abandoned mid-flight. Pass on what was
                    // collected — leaving an unterminated OSC in the stream
                    // would make VTE swallow every byte after it as body — and
                    // hold this `ESC` as the possible start of the next
                    // sequence.
                    self.out.append(&mut self.pending);
                    self.pending.clear();
                    self.pending.push(byte);
                    self.state = State::Escape;
                }
            }
        }
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
            return;
        }

        // Disabled: byte-for-byte what the stream looked like before this filter
        // existed, which is what VTE — and RustConn — do with the sequence today.
        if !enabled() {
            self.out.extend_from_slice(&sequence);
            return;
        }

        match parse(&sequence) {
            Parsed::Write(text) => sink(&text),
            Parsed::Query => {
                // Deliberately unanswered. A reply would be the local clipboard's
                // contents travelling to the remote, which is the one direction
                // this feature must never open.
                tracing::debug!("dropped OSC 52 clipboard query; reads are not served");
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
        .or_else(|| sequence.strip_prefix(&[INTRODUCER_8BIT]))?;
    // The 7-bit `ST` is two bytes, and its `ESC` is already in `rest` because
    // [`State::OscEscape`] holds it back while it waits for the `\`.
    rest.strip_suffix(&[ESC, TERMINATOR_7BIT])
        .or_else(|| rest.strip_suffix(&[TERMINATOR_8BIT]))
        .or_else(|| rest.strip_suffix(&[BEL]))
}

#[cfg(test)]
mod tests {
    use super::{MAX_OSC_BODY, Osc52Filter, Parsed, body_of, parse, set_enabled};
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
        assert!(fed.is_empty(), "VTE must not see the sequence: {fed:?}");
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
        assert!(writes.is_empty());
    }

    #[test]
    fn output_around_a_removed_offer_keeps_its_order() {
        let mut stream = b"before ".to_vec();
        stream.extend_from_slice(&offer("the yank"));
        stream.extend_from_slice(b" after");
        let (fed, writes) = run(&stream);
        assert_eq!(fed, b"before  after");
        assert_eq!(writes, ["the yank"]);
    }

    #[test]
    fn several_offers_in_one_chunk_are_all_handled() {
        let mut stream = offer("first");
        stream.extend_from_slice(&offer("second"));
        stream.extend_from_slice(&offer("third"));
        let (fed, writes) = run(&stream);
        assert_eq!(writes, ["first", "second", "third"]);
        assert!(fed.is_empty());
    }

    #[test]
    fn an_esc_that_starts_something_else_is_not_swallowed() {
        // `ESC [` is a CSI, and the `ESC` before the OSC must survive it.
        let mut stream = b"\x1b[2J".to_vec();
        stream.extend_from_slice(&offer("yank"));
        stream.extend_from_slice(b"\x1b[1;1H");
        let (fed, writes) = run(&stream);
        assert_eq!(fed, b"\x1b[2J\x1b[1;1H");
        assert_eq!(writes, ["yank"]);
    }

    #[test]
    fn a_doubled_escape_starts_a_second_sequence() {
        // `ESC ESC ] 52 …` is a lone `ESC` followed by a perfectly good OSC, the
        // way xterm reads it: the leading byte goes to VTE and the offer is still
        // honoured, rather than one greedy sequence swallowing the pair.
        let mut stream = b"\x1b".to_vec();
        stream.extend_from_slice(&offer("yank"));
        let (fed, writes) = run(&stream);
        assert_eq!(writes, ["yank"]);
        assert_eq!(fed, b"\x1b");
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
        assert!(writes.is_empty());
        // Nothing lost and nothing reordered: the fragment plus the CSI that
        // replaced it comes out as it went in.
        assert_eq!(fed, stream);
    }

    #[test]
    fn a_string_the_stream_ends_inside_is_released_by_finish() {
        let payload = glib::base64_encode(b"cut off");
        let stream = format!("\x1b]52;c;{payload}").into_bytes();
        set_enabled(true);
        let mut filter = Osc52Filter::default();
        let mut writes = Vec::new();
        let mut sink = |text: &str| writes.push(text.to_owned());
        let mut fed = filter.push(&stream, &mut sink).to_vec();
        fed.extend_from_slice(filter.finish());
        assert_eq!(fed, stream);
        assert!(
            writes.is_empty(),
            "an unterminated payload must not be trusted"
        );
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
        assert_eq!(fed, b"visible");
    }

    #[test]
    fn a_query_is_never_answered() {
        let (fed, writes) = run(b"\x1b]52;c;?\x07");
        assert!(
            writes.is_empty(),
            "the local clipboard must not be sent out"
        );
        assert!(fed.is_empty());
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
        assert!(fed.is_empty());
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
        assert!(writes.is_empty());

        // And an offer split across chunks stays whole on the way to VTE.
        set_enabled(false);
        let mut filter = Osc52Filter::default();
        let mut fed = Vec::new();
        for byte in &sequence {
            let mut sink = |text: &str| writes.push(text.to_owned());
            fed.extend_from_slice(filter.push(std::slice::from_ref(byte), &mut sink));
        }
        assert_eq!(fed, sequence);
        assert!(writes.is_empty());
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
        assert_eq!(fed, offer("first"));
    }

    /// Builds `OSC 52 ; c ; <base64 of "hello">` with the framing asked for, as
    /// raw bytes.
    ///
    /// Built byte-wise on purpose: a C1 control in a terminal stream is the
    /// single byte 0x9c, not the two-byte UTF-8 encoding of U+009C that `\u{9c}`
    /// would put in a `&str`. Only the byte is a terminator.
    fn framed_offer(introducer: &[u8], terminator: &[u8]) -> Vec<u8> {
        let mut sequence = introducer.to_vec();
        sequence.extend_from_slice(b"52;c;");
        sequence.extend_from_slice(glib::base64_encode(b"hello").as_bytes());
        sequence.extend_from_slice(terminator);
        sequence
    }

    #[test]
    fn both_terminators_and_both_introducers_are_understood() {
        for (introducer, terminator) in [
            (b"\x1b]".as_slice(), b"\x1b\\".as_slice()),
            (b"\x1b]".as_slice(), b"\x07".as_slice()),
            (b"\x1b]".as_slice(), b"\x9c".as_slice()),
            (b"\x9d".as_slice(), b"\x9c".as_slice()),
            (b"\x9d".as_slice(), b"\x07".as_slice()),
            (b"\x9d".as_slice(), b"\x1b\\".as_slice()),
        ] {
            let sequence = framed_offer(introducer, terminator);
            let (fed, writes) = run(&sequence);
            assert_eq!(writes, ["hello"], "{sequence:?} was not understood");
            assert!(fed.is_empty(), "{sequence:?} reached VTE: {fed:?}");
        }
    }

    #[test]
    fn a_utf8_encoded_c1_byte_is_not_a_terminator() {
        // 0xc2 0x9c is how U+009C looks in text. No terminal emits it to end a
        // string, so it must not be accepted as one — otherwise a payload could
        // be cut short by a sequence that never happened.
        let mut sequence = b"\x1b]52;c;".to_vec();
        sequence.extend_from_slice(glib::base64_encode(b"hello").as_bytes());
        sequence.extend_from_slice("\u{9c}".as_bytes());
        let (fed, writes) = run(&sequence);
        assert!(writes.is_empty(), "a two-byte sequence ended the string");
        assert_eq!(fed, sequence);
    }

    #[test]
    fn body_of_strips_only_the_framing() {
        assert_eq!(body_of(b"\x1b]0;title\x07"), Some(&b"0;title"[..]));
        assert_eq!(body_of(b"\x1b]0;title\x1b\\"), Some(&b"0;title"[..]));
        assert_eq!(body_of(b"\x9d0;title\x9c"), Some(&b"0;title"[..]));
        assert_eq!(body_of(b"\x1b[2J"), None);
        assert_eq!(body_of(b"\x1b]"), None);
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
