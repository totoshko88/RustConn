---
inclusion: auto
name: release-reminder
description: "Mandatory steps when bumping the workspace version in Cargo.toml — changelog propagation to debian/OBS/metainfo, dependency refresh, CLI version check. Use when preparing a release, bumping a version, or writing release notes."
---

# Release Process Reminder

> Was `inclusion: fileMatch` on `Cargo.toml` until 2026-08-12, which fired on
> *any* read of `Cargo.toml` — including plain dependency lookups and audits that
> have nothing to do with a release. `auto` matches the request instead of the
> file, so it now loads when the task actually is a release. Force it with
> `/release-reminder` if the matcher misses.

When the workspace version in `Cargo.toml` is being bumped:

## Mandatory Steps (in order)

1. **CHANGELOG.md** — add `## [X.Y.Z] - YYYY-MM-DD` section with `### Fixed` / `### Added` / `### Improved` etc.
2. **Changelog propagation** — after writing CHANGELOG.md, propagate to:
   - `debian/changelog` — new entry at top (Debian format)
   - `packaging/obs/debian.changelog` — same format
   - `packaging/obs/rustconn.changes` — OBS changes format
   - `packaging/obs/rustconn.spec` — add `%changelog` entry
   - `rustconn/assets/io.github.totoshko88.RustConn.metainfo.xml` — add `<release>` entry
3. **Dependency updates** — the full flow, including that you *report and ask
   before applying* rather than running `cargo update` outright, is step 2 of
   `release-version.md`. Do not preview-and-apply from here; follow that file, and
   record any applied updates in CHANGELOG.md `### Dependencies`.
4. **CLI version check** (if `scripts/check-cli-versions.sh` exists):
   ```bash
   ./scripts/check-cli-versions.sh
   ```
   If updates available — update `rustconn-core/src/cli_download.rs` and record in CHANGELOG.md.

## Important

- Version-number propagation to packaging files (flatpak/flathub tags, dsc files, AppImage, docs, spec `Version:` field) is `scripts/bump-version.sh X.Y.Z --write`, which writes the Flathub manifest too and is therefore the maintainer's command; an agent runs the dry run (`packaging/AGENTS.md`, "Flathub is maintainer-only") — see the **`release-version.md`** steering file (manual), which is the full checklist this file only summarises.
- YOU must handle all changelog/release-notes files manually — no hook or script creates changelog entries.
- For full release process details read `release.md` in the **`rustconn`** power (`kiro_powers` → readSteering), and `release-version.md` here. There is no `rustconn-dev` power; the installed one is `rustconn`.
