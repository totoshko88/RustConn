---
inclusion: manual
description: "Prepares a release: checks for dependency updates across ALL types (cargo, CLI downloads, flatpak, snap), bumps version in all packaging files, propagates changelog, regenerates cargo-sources.json, and verifies consistency. Does NOT run git — the merge/tag/push is done manually via scripts/release.sh. Trigger manually and provide the version number (e.g. \"0.12.6\") in your message."
---

The user wants to PREPARE a release (file edits only). Read the `rustconn` power steering file `release.md` for the full checklist.

**IMPORTANT — NO GIT:** This hook ONLY edits files. Do NOT run any git command (no `git add`, `git commit`, `git merge`, `git checkout`, `git tag`, `git push`). The actual merge → tag → push is performed manually by `scripts/release.sh` after development is complete. Your job ends at leaving a clean, consistent working tree for that script to validate.

Then perform ALL of the following steps:

1. **Read version** from user message (e.g. "0.12.6"). If not provided, ask.

2. **DEPENDENCY FRESHNESS CHECK (report, then ask before applying)** — before bumping anything, audit EVERY dependency type for available updates so the release ships current deps. Report findings grouped; do NOT silently apply. Ask the user which (if any) to update before continuing.
   - **Cargo**: `cargo update --dry-run 2>&1` (patch/minor/major) + `cargo deny check advisories` (fallback `cargo audit`). Record applied updates in CHANGELOG.md `### Dependencies`.
   - **CLI downloads**: run `scripts/check-cli-versions.sh` (or read `rustconn-core/src/cli_download.rs`). For any outdated PINNED tool, update `pinned_version`, `download_url`, `aarch64_url`, and `checksum` (Static policy → fetch the new .sha256). Record under CHANGELOG.md `### Changed` as `- CLI downloads — Tool X.Y.Z→X.Y.W`.
   - **Flatpak** (`packaging/flatpak/*.yml` + keep `packaging/flathub/*.yml` in sync): check `org.gnome.Platform`/`org.gnome.Sdk` `runtime-version` (currently '50'), the `rust-stable` SDK extension, and bundled pinned source modules with `x-checker-data` (FreeRDP `freerdp-X.Y.Z.tar.xz`, cJSON). For any bump, update the version AND its `sha256`.
   - **Snap** (`snap/snapcraft.yaml`): check `base` (core24) and the `gnome` extension (gnome-46-2404) — flag if a core26 gnome extension now exists (issue #174 context) — plus pinned `stage-packages`/`build-packages` drift.
   - If everything is current, note 'dependencies up to date' and proceed.

3. **Cargo.toml** — set `[workspace.package] version` to the new version.

4. **CHANGELOG.md** — verify a `## [X.Y.Z] - YYYY-MM-DD` section exists. If not, ask user to write it first. Ensure any dependency updates from step 2 are recorded in `### Dependencies` / `### Changed`.

5. **Propagate changelog** to ALL of these files (convert format as needed):
   - `debian/changelog` (Debian format, prepend)
   - `packaging/obs/debian.changelog` (same Debian format, prepend)
   - `packaging/obs/rustconn.changes` (OBS format, prepend)
   - `packaging/obs/rustconn.spec` (update `Version:` field + add `%changelog` entry)
   - `rustconn/assets/io.github.totoshko88.RustConn.metainfo.xml` (add `<release>` entry)

6. **Update version strings** in (CANONICAL list = `PKG_FILES` in `scripts/release.sh`; keep in sync with that array):
   - `snap/snapcraft.yaml` → `version: 'X.Y.Z'`
   - `packaging/obs/AppImageBuilder.yml` → `version: X.Y.Z`
   - `packaging/flatpak/io.github.totoshko88.RustConn.yml` → `tag: vX.Y.Z`
   - `packaging/flathub/io.github.totoshko88.RustConn.yml` → `tag: vX.Y.Z`
   - `packaging/obs/rustconn.dsc` → `Version: X.Y.Z-1` + tar filenames
   - `packaging/obs/debian.dsc` → `Version: X.Y.Z-1` + `DEBTRANSFORM-TAR`
   - `packaging/obs/_service` → `<param name="revision">vX.Y.Z</param>`
   - `docs/USER_GUIDE.md` → `**Version X.Y.Z**`
   - `docs/ARCHITECTURE.md` → `**Version X.Y.Z**`
   - `docs/AI_DEVELOPMENT.md` → version in the first line after heading
   - `docs/CI_BUILD_FLOW.md` → update example version strings

7. **Regenerate Cargo.lock** — run `cargo generate-lockfile`

8. **Regenerate cargo-sources.json** — run:
   ```
   python3 packaging/flatpak/flatpak-cargo-generator.py Cargo.lock -o packaging/flatpak/cargo-sources.json
   cp packaging/flatpak/cargo-sources.json packaging/flathub/cargo-sources.json
   ```

9. **Verify consistency** — grep for the OLD version across the repo (excluding Cargo.lock, target/, .git/) and report any remaining references that are NOT historical changelog entries.

10. **Run quality checks** — delegate to `rust-quality-check` sub-agent for fmt + clippy.

11. **Report summary** — list all files modified, the dependency-update decisions from step 2, and any issues found. Remind the user that NO git operations were performed and the next step is to run `./scripts/release.sh` manually (it does the merge → tag → push).
