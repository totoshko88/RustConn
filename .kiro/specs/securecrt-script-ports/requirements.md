# Requirements: SecureCRT Behaviour as Bundled `.rsc` Scripts

## Introduction
Port three behaviours from the SecureCRT script collections into RustConn's
`.rsc` DSL: Cisco TDR cable test, command-output capture, and destructive
device erase. Sources are behavioural inspiration only; no VBScript or Python
is copied (two source repos have no LICENSE).

## Requirements

### Requirement 1 — Scripts exist and parse
WHEN the Scripts dialog opens THEN the three `example-*` ids SHALL be seeded
and SHALL pass `parse_program` with no errors.

### Requirement 2 — Inputs are validated
WHEN a user supplies an interface, command or confirmation THEN the script
SHALL accept only the documented character set and SHALL send nothing to the
device on a rejected value.

### Requirement 3 — Destructive operations need two confirmations
WHEN `example-cisco-erase-device` runs THEN it SHALL require the exact
hostname and a literal `ERASE` before any `write erase` / `delete` / `reload`
command is sent, and SHALL write an audit transcript first.

### Requirement 4 — Unchanged behaviour
WHEN an existing `example-*-save-config` script runs THEN it SHALL CONTINUE to
parse and behave as before.

## Acceptance Criteria
- [ ] Tests: invalid interface/command sends nothing; erase without both
      confirmations sends nothing; all examples parse.
- [ ] USER_GUIDE and CHANGELOG updated; no i18n/POT change (script data).
