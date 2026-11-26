# Requirements: CLI Device Reports

## Introduction
Finish the untracked, never-committed `rustconn-cli/src/commands/report.rs`
(unfinished work from `feat/cli-batch-exec`) and extend it with the structured
report family from securecrt-tools, including the six AireOS WLC collectors.

## Requirements

### Requirement 1 — Command is wired
WHEN `rustconn-cli report <kind>` is invoked THEN the CLI SHALL dispatch to
`cmd_report` with selection, format and quiet flags, and SHALL use the same
SSH argv as `connect`.

### Requirement 2 — Structured rows
WHEN a host returns recognised `show` output THEN each report kind SHALL emit
rows whose field count equals its header count, in table/JSON/CSV output.

### Requirement 3 — A host failure is contained
WHEN one host's command fails THEN the CLI SHALL report it (unless `--quiet`),
skip that host and still print the remaining rows.

### Requirement 4 — Unchanged behaviour
WHEN `--format` is omitted and stdout is not a terminal THEN output SHALL
CONTINUE to default to JSON.

## Acceptance Criteria
- [ ] 22 report kinds available (including BGP for Cisco, Huawei and Juniper);
      parser tests plus a rectangular-row test.
- [ ] `--quiet` suppresses warnings; CLI_REFERENCE and CHANGELOG updated.
