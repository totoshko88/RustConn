# Design: CLI Device Reports

## Alternatives
1. Port TextFSM to the `.rsc` DSL — rejected: no table parser or CSV writer;
   new dependency and large engine change.
2. Finish the existing Rust parsers — chosen: `report.rs` already has 8 tests
   and hand-written parsers for five kinds; extend the same shape.

## Changes
- Recover the minimal `select_ssh_jobs` helper from the 0.24.5-era
  `feat/cli-batch-exec` branch into `commands/exec.rs`; no `exec` subcommand.
- `cli.rs`: `ReportKind` (19 variants) and `Commands::Report`.
- `commands/mod.rs`: module declarations and dispatch, passing `quiet`.
- `report.rs`: 14 further kinds, `normalize_row`, quiet-gated warnings.
- Docs: `docs/CLI_REFERENCE.md`, `CHANGELOG.md`.

## Verification performed
GNS3 is not installed and no licensed IOS/AireOS QEMU image is available, so
verification used a local paramiko SSH device that returns recorded-style
`show` output. All 19 report kinds were run end-to-end through the real CLI
with key-based auth and returned exit 0 with correctly shaped rows.

## Real-firmware verification
The Cisco BGP parser was checked against a Cisco 3745 running IOS 12.4(25d)
under Dynamips (`-j -X`, console TCP), and the exact `show ip bgp summary`
output was pinned as a test fixture. It exposed the `never` uptime token, now
handled. AireOS parsers were checked against a licensed vWLC 8.7.102 in QEMU.

## Constraints
Key-based SSH only (`-n`, stdin null), no timeout, sequential hosts. Huawei and
Juniper BGP parsers are verified against representative samples only.
Production switch is read-only.
