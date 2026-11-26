# Design: Guarded Network Changes

## Alternatives
- Port bulk CSV-driven multi-device scripts — rejected: no CSV/loop support
  in the DSL and no lab testbed; would be unverifiable.
- Single-device scripts with check mode — chosen: minimum verifiable slice.

## Changes
- Three new bundled examples in `script_examples.rs`, each with prompt/mode
  checks, strict input regexes, proposal print, `APPLY` gate, audit log and
  optional `YES` save.
- One engine test asserting check mode sends nothing.

## Verification performed
No GNS3/IOS image was available. The change scripts were driven through the
real `ScriptEngine` with a simulated Cisco prompt stream: check mode sends no
`configure terminal`, and the `APPLY`/`YES` path sends the configuration
commands, `end` and `write memory`. No live device was changed.

## Out of scope
`create sessions from CDP` / `import sessions from CSV`: RustConn already has
import paths; adding a DSL-driven connection creator would duplicate them.
Live verification requires a lab switch, not the production one.
