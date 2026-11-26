# Design: SecureCRT Behaviour as `.rsc` Scripts

## Alternatives
1. Vendor the original Python/VBS — rejected: `crt.*` API does not exist and
   two repos have no licence.
2. Add a Python/`crt` shim — rejected: new dependency, cross-platform risk.
3. Rewrite in the existing DSL — chosen: no engine changes for these three.

## Changes
- `rustconn/src/dialogs/script_examples.rs`: three new `(id, source)` entries
  reusing the Cisco profile skeleton (mode check, prompt capture, `${CWD}`
  timestamped log, explicit confirmation).
- `rustconn/src/dialogs/script.rs`: engine tests using `engine_for`/`feed`/
  `Recorder` against the new ids.
- Docs: `docs/USER_GUIDE.md`, `CHANGELOG.md`.

## Safety
TDR and capture are read-only. Erase is destructive by design: exact hostname
+ literal `ERASE`, audit log, no auto-run attachment, lab-only live testing.
