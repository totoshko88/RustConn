# Design: Caption and OUI Verbs

## Alternatives
- `caption` as a GUI-only action — rejected: not scriptable.
- `oui` via `run grep` against a shipped `manuf.txt` — rejected: data licence
  and sandbox path issues.
- Two DSL verbs backed by core code — chosen.

## Changes
- `ScriptStep::{Caption,Oui}`, `ScriptHost::set_caption`, parser and engine
  dispatch in `rustconn-core/src/automation/script.rs`.
- `rustconn-core/src/oui.rs`: ~40 common prefixes, longest-prefix match.
- `TerminalNotebook::set_session_caption` in `tab_labels.rs`; `GuiHost`
  implementation in `script_runner.rs`; recorder fakes updated.
- Reference/docs/tests.

## Constraints
The OUI table is intentionally small: `shortcut:` comment marks the upgrade
path to a licensed generated dataset.
