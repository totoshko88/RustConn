# Design: AireOS Collectors

## Alternatives
- Port the TextFSM templates — rejected: new runtime dependency, no WLC to
  verify against, and no TextFSM in Rust.
- Generic whitespace-column parser — chosen: one helper serves all six kinds
  and keeps malformed rows out of the document.

## Changes
- `cli.rs`: six `ReportKind` variants.
- `report.rs`: `command`, `headers`, `parse` arms and `parse_columns`.
- Sample-output and width tests; CLI_REFERENCE table.

## Verification performed
First against a local SSH mock, then against a licensed vWLC 8.7.102.0 booted
in isolated QEMU. The real controller's `show auth-list`, `show interface
summary`, `show mobility summary` and `show wlan summary` output was captured
over the serial console and pinned as test fixtures. It exposed four parser
mismatches (dotted key/value lines and column layouts), all fixed. SSH onto
the guest did not yield a banner through QEMU user-mode NAT, so the report
commands themselves were not re-run over SSH against the real controller.

## Constraints
AireOS 8.7 `show ap summary` returns no AP table until APs join; the AP detail
kind still uses `show ap summary` and needs a controller with APs to verify.
