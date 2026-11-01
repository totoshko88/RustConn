//! Tests for the control-mode parser and session model.
//!
//! Golden streams are taken from the tmux Control-Mode wiki examples; the
//! chunk-boundary tests feed each stream split at every byte offset and assert
//! the events are identical to a whole-stream feed.

use super::*;

/// Parse a whole stream in one feed.
fn parse_all(input: &[u8]) -> Vec<ControlEvent> {
    let mut parser = ControlModeParser::new();
    parser.feed(input)
}

/// Parse a stream split at `at`, to prove chunk boundaries do not matter.
fn parse_split(input: &[u8], at: usize) -> Vec<ControlEvent> {
    let mut parser = ControlModeParser::new();
    let mut events = parser.feed(&input[..at]);
    events.extend(parser.feed(&input[at..]));
    events
}

/// Feed `input` split at every byte offset; assert every split yields `expected`.
fn assert_chunk_robust(input: &[u8], expected: &[ControlEvent]) {
    assert_eq!(parse_all(input), expected, "whole-stream feed");
    for at in 0..=input.len() {
        assert_eq!(
            parse_split(input, at),
            expected,
            "split at byte {at} must match whole-stream feed"
        );
    }
}

#[test]
fn new_session_attach_sequence() {
    // From the wiki's `tmux -Ltest -C new` example (control chars trimmed).
    let input = b"%begin 1578920019 258 0\n\
                  %end 1578920019 258 0\n\
                  %window-add @1\n\
                  %sessions-changed\n\
                  %session-changed $1 1\n\
                  %window-renamed @1 tmux\n\
                  %output %1 nicholas@yelena:~$\n\
                  %window-renamed @1 ksh\n";
    let events = parse_all(input);
    assert_eq!(
        events,
        vec![
            ControlEvent::BlockEnd {
                time: 1_578_920_019,
                num: 258,
                flags: 0,
                lines: vec![],
            },
            ControlEvent::WindowAdd(WindowId("@1".into())),
            ControlEvent::SessionsChanged,
            ControlEvent::SessionChanged(SessionId("$1".into()), "1".into()),
            ControlEvent::WindowRenamed(WindowId("@1".into()), "tmux".into()),
            ControlEvent::Output {
                pane: PaneId("%1".into()),
                data: b"nicholas@yelena:~$".to_vec(),
            },
            ControlEvent::WindowRenamed(WindowId("@1".into()), "ksh".into()),
        ]
    );
}

#[test]
fn new_window_command_block() {
    let input = b"neww\n%begin 1578923903 256 1\n%end 1578923903 256 1\n";
    // `neww` is user input echoed before control mode framing in the wiki
    // transcript; outside a block a non-% line is Unknown.
    let events = parse_all(input);
    assert_eq!(
        events,
        vec![
            ControlEvent::Unknown(b"neww".to_vec()),
            ControlEvent::BlockEnd {
                time: 1_578_923_903,
                num: 256,
                flags: 1,
                lines: vec![],
            },
        ]
    );
}

#[test]
fn command_block_with_output_lines() {
    let input = b"%begin 1578922740 269 1\n\
                  0:0.0: [80x24] [history 0/2000, 0 bytes] %0 (active)\n\
                  1:0.0: [80x24] [history 0/2000, 0 bytes] %1 (active)\n\
                  %end 1578922740 269 1\n";
    let events = parse_all(input);
    assert_eq!(
        events,
        vec![ControlEvent::BlockEnd {
            time: 1_578_922_740,
            num: 269,
            flags: 1,
            lines: vec![
                b"0:0.0: [80x24] [history 0/2000, 0 bytes] %0 (active)".to_vec(),
                b"1:0.0: [80x24] [history 0/2000, 0 bytes] %1 (active)".to_vec(),
            ],
        }]
    );
}

#[test]
fn error_block() {
    let input = b"abcdef\n%begin 1578923149 270 1\nparse error: unknown command: abcdef\n%error 1578923149 270 1\n";
    let events = parse_all(input);
    assert_eq!(
        events,
        vec![
            ControlEvent::Unknown(b"abcdef".to_vec()),
            ControlEvent::BlockError {
                time: 1_578_923_149,
                num: 270,
                flags: 1,
                lines: vec![b"parse error: unknown command: abcdef".to_vec()],
            },
        ]
    );
}

#[test]
fn percent_line_inside_a_block_is_output_not_a_notification() {
    // A block output line that itself begins with `%` must stay block output.
    let input = b"%begin 1 2 1\n%some-tool progress 50%\n%end 1 2 1\n";
    let events = parse_all(input);
    assert_eq!(
        events,
        vec![ControlEvent::BlockEnd {
            time: 1,
            num: 2,
            flags: 1,
            lines: vec![b"%some-tool progress 50%".to_vec()],
        }]
    );
}

#[test]
fn output_octal_unescaping() {
    // `ls /\015\015\012` → the CR CR LF the wiki shows.
    let input = b"%output %1 ls /\\015\\015\\012\n";
    let events = parse_all(input);
    assert_eq!(
        events,
        vec![ControlEvent::Output {
            pane: PaneId("%1".into()),
            data: b"ls /\r\r\n".to_vec(),
        }]
    );
}

#[test]
fn output_unescapes_backslash_and_preserves_non_utf8() {
    // `\134` is a literal backslash; a raw 0xFF byte (non-UTF-8) survives.
    let mut input = b"%output %2 a\\134b".to_vec();
    input.push(0xFF);
    input.push(b'\n');
    let events = parse_all(&input);
    assert_eq!(
        events,
        vec![ControlEvent::Output {
            pane: PaneId("%2".into()),
            data: vec![b'a', b'\\', b'b', 0xFF],
        }]
    );
}

#[test]
fn output_octal_escape_above_a_byte_is_kept_verbatim() {
    // `\400`..=`\777` are octal digits but exceed 255. tmux never emits them,
    // but a malformed/adversarial stream could; the decoder must neither panic
    // (debug) nor wrap (release) — it keeps the sequence verbatim.
    let input = b"%output %3 \\377\\400\\777\n";
    let events = parse_all(input);
    assert_eq!(
        events,
        vec![ControlEvent::Output {
            pane: PaneId("%3".into()),
            // `\377` = 255 (valid, decoded); `\400` and `\777` overflow a byte
            // and survive as their literal characters.
            data: {
                let mut v = vec![0xFFu8];
                v.extend_from_slice(b"\\400\\777");
                v
            },
        }]
    );
}

#[test]
fn extended_output_under_flow_control() {
    let input = b"%extended-output %0 1234 : abcdef\n";
    let events = parse_all(input);
    assert_eq!(
        events,
        vec![ControlEvent::ExtendedOutput {
            pane: PaneId("%0".into()),
            age_ms: 1234,
            data: b"abcdef".to_vec(),
        }]
    );
}

#[test]
fn quoted_session_names_survive() {
    // `ls -F` output inside a block is not parsed as notifications.
    let input = b"%begin 1578925957 337 1\n$4 \"\\\"quoted\\\"\"\n%end 1578925957 337 1\n";
    let events = parse_all(input);
    assert_eq!(
        events,
        vec![ControlEvent::BlockEnd {
            time: 1_578_925_957,
            num: 337,
            flags: 1,
            lines: vec![b"$4 \"\\\"quoted\\\"\"".to_vec()],
        }]
    );
}

#[test]
fn unknown_notification_is_surfaced() {
    let input = b"%totally-new-notification @9 stuff\n";
    let events = parse_all(input);
    assert_eq!(
        events,
        vec![ControlEvent::Unknown(input[..input.len() - 1].to_vec())]
    );
}

#[test]
fn exit_with_and_without_reason() {
    assert_eq!(
        parse_all(b"%exit\n"),
        vec![ControlEvent::Exit { reason: None }]
    );
    assert_eq!(
        parse_all(b"%exit server exited\n"),
        vec![ControlEvent::Exit {
            reason: Some("server exited".into()),
        }]
    );
}

#[test]
fn open_block_at_eof_holds_rather_than_emitting_partial() {
    let mut parser = ControlModeParser::new();
    let events = parser.feed(b"%begin 1 2 1\nsome output\n");
    assert!(events.is_empty(), "no event until the block closes");
    let events = parser.feed(b"%end 1 2 1\n");
    assert_eq!(
        events,
        vec![ControlEvent::BlockEnd {
            time: 1,
            num: 2,
            flags: 1,
            lines: vec![b"some output".to_vec()],
        }]
    );
}

#[test]
fn chunk_boundaries_do_not_change_events() {
    let input = b"%window-add @1\n\
                  %begin 1 2 1\nline one\nline two\n%end 1 2 1\n\
                  %output %3 hi\\012\n\
                  %exit\n";
    let expected = vec![
        ControlEvent::WindowAdd(WindowId("@1".into())),
        ControlEvent::BlockEnd {
            time: 1,
            num: 2,
            flags: 1,
            lines: vec![b"line one".to_vec(), b"line two".to_vec()],
        },
        ControlEvent::Output {
            pane: PaneId("%3".into()),
            data: b"hi\n".to_vec(),
        },
        ControlEvent::Exit { reason: None },
    ];
    assert_chunk_robust(input, &expected);
}

#[test]
fn crlf_line_endings_are_tolerated() {
    let input = b"%window-add @1\r\n%exit\r\n";
    assert_eq!(
        parse_all(input),
        vec![
            ControlEvent::WindowAdd(WindowId("@1".into())),
            ControlEvent::Exit { reason: None },
        ]
    );
}

// ── Model folding ────────────────────────────────────────────────────────────

fn apply_all(events: &[ControlEvent]) -> TmuxSessionModel {
    let mut model = TmuxSessionModel::new();
    for ev in events {
        model.apply(ev);
    }
    model
}

#[test]
fn model_tracks_window_add_rename_close() {
    let model = apply_all(&[
        ControlEvent::WindowAdd(WindowId("@1".into())),
        ControlEvent::WindowRenamed(WindowId("@1".into()), "editor".into()),
        ControlEvent::WindowAdd(WindowId("@2".into())),
        ControlEvent::WindowClose(WindowId("@2".into())),
    ]);
    assert_eq!(model.windows.len(), 1);
    assert_eq!(model.windows[&WindowId("@1".into())].name, "editor");
    assert!(!model.windows.contains_key(&WindowId("@2".into())));
}

#[test]
fn model_tracks_session_and_exit() {
    let model = apply_all(&[
        ControlEvent::SessionChanged(SessionId("$0".into()), "main".into()),
        ControlEvent::SessionRenamed(SessionId("$0".into()), "work".into()),
        ControlEvent::Exit { reason: None },
    ]);
    assert_eq!(model.session, Some((SessionId("$0".into()), "work".into())));
    assert!(model.ended);
}

#[test]
fn model_stores_layout_verbatim() {
    let layout = "bb62,80x24,0,0,0".to_string();
    let model = apply_all(&[ControlEvent::LayoutChange {
        window: WindowId("@1".into()),
        layout: layout.clone(),
    }]);
    assert_eq!(model.windows[&WindowId("@1".into())].layout, layout);
}

#[test]
fn model_tracks_pane_pause_and_continue() {
    let model = apply_all(&[
        ControlEvent::WindowPaneChanged(WindowId("@1".into()), PaneId("%5".into())),
        ControlEvent::Pause(PaneId("%5".into())),
    ]);
    let pane = &model.windows[&WindowId("@1".into())].panes[0];
    assert!(pane.paused);

    let model = apply_all(&[
        ControlEvent::WindowPaneChanged(WindowId("@1".into()), PaneId("%5".into())),
        ControlEvent::Pause(PaneId("%5".into())),
        ControlEvent::Continue(PaneId("%5".into())),
    ]);
    assert!(!model.windows[&WindowId("@1".into())].panes[0].paused);
}

#[test]
fn model_tracks_pane_mode_change() {
    let model = apply_all(&[
        ControlEvent::WindowPaneChanged(WindowId("@1".into()), PaneId("%5".into())),
        ControlEvent::PaneModeChanged(PaneId("%5".into())),
    ]);
    assert!(model.windows[&WindowId("@1".into())].panes[0].mode_changed);
}

#[test]
fn unlinked_window_events_do_not_touch_the_model() {
    // Unlinked windows belong to other sessions; the attached model ignores them.
    let model = apply_all(&[
        ControlEvent::UnlinkedWindowAdd(WindowId("@9".into())),
        ControlEvent::UnlinkedWindowClose(WindowId("@9".into())),
    ]);
    assert!(model.windows.is_empty());
}
