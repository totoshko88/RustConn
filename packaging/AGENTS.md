# packaging/

Six delivery channels: Flatpak (local + Flathub), OBS (RPM, Debian, AppImage),
Snap, Nix, Homebrew/macOS, and the Debian tree at the repo root. Root `AGENTS.md`
still applies.

## The one rule that matters most

**An agent prepares a release; it never cuts one.** `./scripts/release.sh
--dry-run` runs every gate and stops before executing the plan — that is the agent
action and it is expected. Running the script for real, passing `--yes`, or
reaching for `git tag`/`git push` by hand is the maintainer's call. The tag push is
what triggers the Release workflow and the Flathub/OBS/Snap updates, so there is no
quiet way to undo it: v0.20.1 was cut by an agent and reached a published release
with five artifacts while carrying a red CI job. The
`release-manual-only-guard` hook blocks all three routes; do not rely on it.

## Flathub is maintainer-only

Flathub's [generative AI policy](https://docs.flathub.org/docs/for-app-authors/requirements)
forbids AI-generated or AI-assisted content in Flathub manifests, and forbids AI
tools from opening or automating Flathub pull requests or writing their commit
messages, descriptions or replies. So an agent:

- does not write anything under `packaging/flathub/`: not by hand, not through
  `scripts/bump-version.sh --write` (it writes the Flathub manifest's version
  line), and not through `scripts/sync-cargo-sources.sh` (it copies
  `cargo-sources.json` there);
- does not prepare, open or describe a Flathub pull request.

`packaging/flatpak/*.yml` is the manifest an agent may edit. A change that needs
the Flathub manifest stops there and is reported as a maintainer task, with what
Flathub needs. During release preparation the Flathub version line and the Flathub
copy of `cargo-sources.json` are listed as maintainer steps; read-only checks
(`scripts/bump-version.sh X.Y.Z` without `--write`, `scripts/sync-cargo-sources.sh
--check`, `release.sh --dry-run`) are fine.

## `PKG_FILES` in `scripts/release.sh` is canonical

That array — with its parallel `PKG_PATS` — is the single source of truth for which
files must carry the release version, and it is what actually blocks a release on
drift. When adding or removing a packaging file, edit **there first**, then update
whatever mirrors it.

Note what that gate does and does not do: it greps for one matching line per file.
A file with two version strings passes on the first one, which is why
`rustconn/Cargo.toml` needs *every* sibling path dependency bumped, not just the
first.

## Pairs that drift

- `packaging/flatpak/*.yml` and `packaging/flathub/*.yml` are separate manifests
  with the same content requirements. An agent edits the local one, including the
  bundled FreeRDP version *and* its `sha256`, and reports what the Flathub one
  needs to match; the maintainer edits Flathub (see above).
- `cargo-sources.json` exists twice for the same reason. The maintainer
  regenerates both — from `Cargo.lock`, then copied to Flathub — with one command
  so they cannot drift:
  ```bash
  scripts/sync-cargo-sources.sh          # regenerate + copy; --check to verify only
  ```
  It is due straight after any `cargo update`; otherwise the first
  `release.sh --dry-run` fails on stale sources (release.sh section 11b). An
  agent runs only `--check` and reports a drift as a maintainer task.
- `.kiro/powers/rustconn/` and `~/.kiro/powers/installed/rustconn/` are copies of
  each other. The installed one drifts silently.

## Channel facts worth knowing before editing

- `rust-toolchain.toml` is a **rustup** feature. Flatpak/Flathub use the SDK
  extension's cargo and OBS/Debian use the distro cargo, so neither reads it — they
  compile with whatever their base provides. The file itself lists every channel
  and whether it honours the pin; keep that list correct when adding a channel.
- Snap is on `core26` with the `gnome` extension (platform `gnome-core26`, GPU
  `mesa-2604`) and builds with `adw-1-8`. It needs Snapcraft >= 9.2, the first
  release whose `gnome` extension accepts core26 (issue #174). It still trails
  the Flatpak's GNOME runtime by a release or two. It builds the default
  features plus `adw-1-8`, so `web-embedded` is on: WebKitGTK 6.0 comes from the
  platform, and WebKit skips its bubblewrap sandbox inside snaps (issue #244,
  `docs/SNAP.md`).
- OBS needs the version in four places with four different syntaxes:
  `rustconn.spec` (`Version:` plus a `%changelog` entry), `rustconn.dsc` and
  `debian.dsc` (`Version: X.Y.Z-1`, plus tar filenames), and `_service`
  (`<param name="revision">vX.Y.Z</param>`).
- Changelog *content* is written by hand and then propagated into Debian format,
  OBS `.changes` format and a metainfo `<release>` entry. Propagating is mechanical;
  writing it is not, and it is not something to generate from commit subjects.

## Writing the version, not just checking it

```bash
scripts/bump-version.sh X.Y.Z            # dry run, shows the diff
scripts/bump-version.sh X.Y.Z --write    # apply, then self-check (maintainer: it writes Flathub too)
```

`--write` includes the Flathub manifest, so it is the maintainer's command. An
agent runs the dry run and writes the other files from its diff.

It reads `PKG_FILES` out of `release.sh` at runtime rather than keeping a second
copy, writes the workspace `Cargo.toml` and all 16 version-only files, and then
re-checks its own work with release.sh's patterns. Three files are expected to
still fail afterwards — `debian/changelog`, `packaging/obs/debian.changelog` and
`packaging/obs/rustconn.changes` need a written entry, as do the `%changelog`
section of `rustconn.spec` and the metainfo `<release>`.

Every rule in it is line-anchored, and must stay that way. A global replace of the
version string corrupts real content: `rustconn.spec` records dependency bumps like
`cfg-expr 0.20.8→0.20.9`, and `docs/USER_GUIDE.md` says "Changed in 0.20.9" about
behaviour that changed in that release and always will have.
