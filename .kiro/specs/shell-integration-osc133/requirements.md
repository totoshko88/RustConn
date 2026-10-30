# Requirements Document

## Introduction

Shell Integration (OSC 133) adds semantic prompt awareness to RustConn terminal
tabs. Modern terminals (Ghostty, iTerm2, WezTerm, VS Code, Windows Terminal) let
a user jump between shell prompts, see at a glance which commands failed, and
mark command boundaries — all driven by the OSC 133 "FinalTerm" semantic prompt
sequences the remote shell emits (`A` prompt start, `B` command start, `C`
command output start, `D` command end with exit code). RustConn already reads one
of these (`vte.shell.postexec`, the `D` exit code) in
`rustconn/src/terminal/termprops.rs`, but does nothing user-visible with it and
tracks no prompt positions.

This feature builds the missing pieces on top of that foundation: it tracks the
screen row of each prompt, lets the user jump to the previous/next prompt with a
keybinding, and visually marks a command whose exit code was non-zero. It stays
within the existing architecture — VTE terminal, `pty_relay` feeding VTE, the
`default_keybindings()` registry, the `vte-0-78` feature flag with a no-op
fallback on the VTE 0.76 baseline (Ukraine 24.04 deb and noble AppImage).

This is the primary feature of the 0.24.3 release.

GitHub issue: TBD (to be filed before implementation starts)

## Glossary

- **OSC_133**: The FinalTerm semantic prompt escape sequences
  (`ESC ] 133 ; A/B/C/D … ST`) a shell emits to mark prompt, command, output and
  command-end boundaries. Also called "shell integration" or "semantic prompts".
- **Prompt_Mark**: A recorded screen/scrollback position where a shell prompt
  began (OSC 133 `A`). Used as a navigation target.
- **Command_End_Mark**: The OSC 133 `D` event, optionally carrying the finished
  command's exit code. Already surfaced by VTE as the `vte.shell.postexec`
  termprop (VTE ≥ 0.78).
- **Mark_Registry**: The per-session store of `Prompt_Mark` positions and their
  associated exit codes, owned by the GTK side.
- **Termprop**: A structured value VTE exposes about remote activity, introduced
  in VTE 0.78. The only one read today is `vte.shell.postexec`.
- **Shell_Integration_Script**: The `vte.sh` (or equivalent) sourced on the
  remote host that makes the shell emit OSC 133. Without it, nothing fires — the
  same precondition the existing Command monitoring mode documents.
- **Prompt_Navigation**: Jumping the terminal viewport to the previous or next
  `Prompt_Mark` via a keybinding or action.
- **Failed_Command_Marker**: A visual indicator (scrollbar pip and/or row
  styling) shown for a command whose `Command_End_Mark` exit code was non-zero.
- **Terminal_Session**: The `TerminalSession` struct in
  `rustconn/src/terminal/types.rs` holding per-session metadata.
- **Feature_Gate**: The `vte-0-78` Cargo feature in `rustconn/Cargo.toml`.
  Builds below it (VTE 0.76) get a compiled no-op, exactly as `termprops.rs`
  already does for `connect_command_finished`.
- **Keybinding_Registry**: `rustconn_core::config::keybindings::default_keybindings()`,
  the source of truth for remappable actions shown in Settings ▸ Keybindings.

## Requirements

### Requirement 1 — Track prompt positions

**User story:** As an operator, I want RustConn to know where each shell prompt
is in my terminal, so that it can offer navigation and failure marking.

#### Acceptance Criteria

1. WHEN the remote shell emits an OSC 133 `A` (prompt start) THEN the
   Mark_Registry SHALL record a Prompt_Mark at the current terminal row for that
   session.
2. WHEN the remote shell emits an OSC 133 `D` (command end) with an exit code
   THEN the Mark_Registry SHALL associate that exit code with the most recent
   Prompt_Mark.
3. WHEN the terminal scrollback trims old rows THEN the Mark_Registry SHALL
   discard Prompt_Marks that have scrolled out of the buffer, so stored positions
   never point past the live buffer.
4. WHEN a session is closed THEN its Mark_Registry SHALL be dropped with the
   session, leaking no timers or references.
5. IF the build is below the Feature_Gate (VTE 0.76) THEN mark tracking SHALL
   compile to a no-op and no navigation or marker UI SHALL be presented.

### Requirement 2 — Navigate between prompts

**User story:** As an operator reviewing a long session, I want to jump to the
previous or next prompt with a keybinding, so that I can move between commands
without hand-scrolling.

#### Acceptance Criteria

1. WHEN the user triggers the "jump to previous prompt" action THEN the viewport
   SHALL scroll to the nearest Prompt_Mark above the current position.
2. WHEN the user triggers the "jump to next prompt" action THEN the viewport
   SHALL scroll to the nearest Prompt_Mark below the current position.
3. WHEN there is no Prompt_Mark in the requested direction THEN the action SHALL
   be a no-op (no scroll, no error).
4. The "jump to previous prompt" and "jump to next prompt" actions SHALL be
   registered in the Keybinding_Registry with sensible defaults and SHALL be
   remappable from Settings ▸ Keybindings and listed in the shortcuts help
   dialog, consistent with every other action.
5. WHEN the active session has no recorded Prompt_Marks (no shell integration on
   the remote side) THEN the actions SHALL be no-ops rather than errors.

### Requirement 3 — Mark failed commands

**User story:** As an operator, I want a command that exited non-zero to be
visually marked, so that I can spot failures while scrolling.

#### Acceptance Criteria

1. WHEN a Command_End_Mark carries a non-zero exit code THEN its Prompt_Mark
   SHALL be rendered as a Failed_Command_Marker.
2. WHEN a Command_End_Mark carries exit code zero or no code THEN no
   Failed_Command_Marker SHALL be shown for that prompt.
3. The Failed_Command_Marker SHALL reuse existing RustConn/libadwaita styling
   conventions and SHALL NOT introduce a blocking or modal element.
4. IF the build is below the Feature_Gate THEN no Failed_Command_Marker SHALL be
   shown.

### Requirement 4 — Discoverability and non-regression

**User story:** As a user without remote shell integration, I want the feature to
stay invisible and change nothing, so that it never gets in my way.

#### Acceptance Criteria

1. WHEN the remote side does not emit OSC 133 THEN RustConn behaviour SHALL be
   identical to today (no marks, no markers, navigation actions are no-ops).
2. The feature SHALL NOT add latency to the `pty_relay` hot path beyond observing
   VTE signals/termprops already available on the main thread.
3. WHEN the feature is documented THEN the user guide SHALL explain the remote
   `vte.sh` precondition, reusing the wording the existing Command monitoring
   mode already uses.
4. All user-facing strings SHALL pass through `i18n()` and be added to the
   translation catalogues for the release.
