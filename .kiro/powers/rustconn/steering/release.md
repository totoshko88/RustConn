# RustConn Release Workflow

Automates version, dependency, CLI, and changelog updates across all packaging files.

## When to use

1. At the start of a new branch — bump version and create an empty CHANGELOG entry
2. During development — update dependencies and CLI versions
3. After all features are complete — propagate changelog to all packaging files

## Stage 1: Starting a new version

1. Read the current version from `Cargo.toml` → `[workspace.package] version`
2. Bump the version (patch/minor/major as instructed)
3. Update the version in all files (see "Files to update")
4. Create a CHANGELOG.md entry under `## [X.Y.Z] - YYYY-MM-DD`
5. Commit: `chore: bump version to X.Y.Z`

## Stage 2: Updating dependencies (every release)

### Cargo dependencies

```bash
cargo update --dry-run          # Preview what will be updated
cargo update                    # Apply updates
cargo build                     # Verify compilation
cargo clippy --all-targets      # 0 warnings
cargo fmt --check               # Formatting
cargo test -p rustconn-core --test property_tests  # Property tests (timeout 180s)
```

Record updated packages in the CHANGELOG.md `### Dependencies` section:
```markdown
### Dependencies
- **Updated**: crate1 X.Y.Z→X.Y.W, crate2 A.B.C→A.B.D
```

Include only significant updates (not every transitive dep). Group wasm-bindgen/js-sys/web-sys into one line.

### CLI versions (Flatpak downloads)

File: `rustconn-core/src/cli_download.rs`

Run the automated version checker:

```bash
./scripts/check-cli-versions.sh          # human-readable report
./scripts/check-cli-versions.sh --json   # JSON for automation
```

The script checks the CLI tools against upstream latest releases:
- kubectl (dl.k8s.io/release/stable.txt)
- Tailscale (pkgs.tailscale.com/stable)
- Teleport (GitHub API)
- Boundary (HashiCorp checkpoint API)
- Bitwarden CLI (GitHub API, cli-v* tags)
- 1Password CLI (agilebits check endpoint)
- TigerVNC (GitHub API)

Exit code 0 = all current, 1 = updates available.

**Only TigerVNC (`vncviewer`) is actually pinned** (`pinned_version` + `Static`
checksum) and thus the only one that ever needs a manual version bump. The rest
resolve "latest" at runtime (`pinned_version: None`, `SkipLatest`) and are shown
as `(auto)` — no action required for them.

When the script reports an update for a **pinned** component (i.e. TigerVNC):
1. Update `pinned_version` in `DownloadableComponent`
2. Update `download_url` and `aarch64_url` (version in URL)
3. Update `checksum` (`ChecksumPolicy::Static`) — download the `.sha256` file
4. Record in CHANGELOG.md (under `- Updated:` line or as a separate entry):
   ```
   - **CLI downloads** — TigerVNC 1.16.1→1.16.2
   ```
5. `cargo build && cargo clippy --all-targets` — verify compilation

Auto-latest components (`SkipLatest`, no `pinned_version` — kubectl, Tailscale,
Teleport, Boundary, Bitwarden, 1Password, AWS CLI, gcloud, cloudflared, etc.)
never require URL or version updates.

### Flatpak runtime & bundled libraries

File: `packaging/flatpak/io.github.totoshko88.RustConn.yml` (keep
`packaging/flathub/io.github.totoshko88.RustConn.yml` in sync).

Check each release:
1. **GNOME runtime** — `runtime-version` of `org.gnome.Platform` / `org.gnome.Sdk`
   (currently `'50'`). Compare against the latest stable GNOME runtime on Flathub.
   A bump may unlock newer libadwaita (gate behind the `adw-1-8` feature).
2. **Rust SDK extension** — `org.freedesktop.Sdk.Extension.rust-stable` tracks the
   freedesktop base; note if the freedesktop runtime moved.
3. **Bundled source modules** with a pinned version + `x-checker-data`
   (FreeRDP `freerdp-X.Y.Z.tar.xz`, cJSON). For any upstream bump, update **both**
   the version in the URL **and** the `sha256`. FreeRDP latest →
   `pub.freerdp.com/releases` (or its `x-checker-data` project-id).

Record bumps in CHANGELOG.md `### Changed` (e.g. `- Flatpak — FreeRDP 3.27.1→3.28.0`).

### Snap base & extension

File: `snap/snapcraft.yaml`.

1. **Base + gnome extension** — `base: core24` with the `gnome` extension
   (gnome-46-2404, ships libadwaita 1.5 → snap builds WITHOUT `adw-1-8`). The gnome
   extension is only available for core22/core24. **Flag if a core26 gnome extension
   has shipped** — it would let the snap match the Flatpak's GNOME 50 / libadwaita 1.8
   instead of trading polish for strict confinement (issue #174).
2. **Staged packages** — pinned `stage-packages` / `build-packages` (e.g. VTE,
   waypipe, python3) against the core24 (noble) archive.

> The `dependency-audit` hook automates all of the above (cargo + advisories + CLI +
> flatpak + snap) on demand, and the `release-version` hook runs the same freshness
> check (step 2) before bumping the version.

## Stage 3: Finalizing the release

1. Ensure CHANGELOG.md contains a complete description of changes, including:
   - `### Added` — new features
   - `### Fixed` — bug fixes
   - `### Improved` — improvements
   - `### Changed` — CLI version changes, etc.
   - `### Dependencies` — Cargo dependency updates
   - `### Security` — if there are security-related changes
2. Propagate changelog to all packaging files (see below)
3. Sync version across all packaging files
4. Run final checks:
   ```bash
   cargo fmt --check
   cargo clippy --all-targets    # 0 warnings
   cargo test --workspace        # ~120s
   cargo build --release         # Release build
   ```
5. Run final checks:
   ```bash
   cargo fmt --check
   cargo clippy --all-targets    # 0 warnings
   cargo test --workspace        # ~120s
   cargo build --release         # Release build
   ```
6. **Hand off to `scripts/release.sh`** — do NOT run `git merge`/`git tag`/`git push` by hand.
   The script validates branch ↔ version ↔ changelog ↔ packaging consistency, then performs
   `merge → tag → push` atomically:
   ```bash
   ./scripts/release.sh --dry-run   # validate first
   ./scripts/release.sh             # merge main, tag vX.Y.Z, push (asks for confirmation)
   ```
   Pushing the `v*` tag is what triggers the GitHub release + OBS/Flathub/Homebrew workflows.

## Files to update

### 1. `Cargo.toml` (workspace root)
```toml
[workspace.package]
version = "X.Y.Z"
```

### 2. `CHANGELOG.md`
Source of truth. Format:
```markdown
## [Unreleased]

## [X.Y.Z] - YYYY-MM-DD

### Added
- **Feature Name** — Description ([#N](url))

### Fixed
- **Bug Name** — Description ([#N](url))

### Improved
- **Area** — Description

### Changed
- **CLI downloads** — Component X.Y.Z→X.Y.W

### Dependencies
- **Updated**: crate1 X.Y.Z→X.Y.W, crate2 A.B.C→A.B.D
```

### 3. `docs/USER_GUIDE.md`
First line after the heading:
```
**Version X.Y.Z** | GTK4/libadwaita Connection Manager for Linux
```

### 4. `docs/ARCHITECTURE.md`
First line after the heading:
```
**Version X.Y.Z** | Last updated: Month YYYY
```

### 5. `debian/changelog`
New section at the TOP of the file:
```
rustconn (X.Y.Z-1) unstable; urgency=medium

  * Version bump to X.Y.Z
  * [changelog entries]

 -- Anton Isaiev <totoshko88@gmail.com>  DAY, DD MON YYYY HH:MM:SS +0200
```
DAY — abbreviated day of week (Mon, Tue, Wed, Thu, Fri, Sat, Sun).

### 6. `packaging/obs/debian.changelog`
Same format as `debian/changelog`.

### 7. `packaging/obs/rustconn.changes`
```
-------------------------------------------------------------------
DAY MON DD YYYY Anton Isaiev <totoshko88@gmail.com> - X.Y.Z

- Version bump to X.Y.Z
  * [changelog entries]

```
Date format: `Sun Feb 15 2026` (no commas).

### 8. `packaging/obs/rustconn.spec`
Update `Version:` in the header + `Summary:` if protocols changed.
Add a section to `%changelog` at the TOP:
```
* DAY MON DD YYYY Anton Isaiev <totoshko88@gmail.com> - X.Y.Z-0
- Version bump to X.Y.Z
- [changelog entries]
```

### 9. `packaging/obs/rustconn.dsc`
```
Version: X.Y.Z-1
Files:
 00000000000000000000000000000000 0 rustconn_X.Y.Z.orig.tar.xz
 00000000000000000000000000000000 0 rustconn_X.Y.Z-1.debian.tar.xz
```

### 10. `packaging/obs/debian.dsc`
```
Version: X.Y.Z-1
DEBTRANSFORM-TAR: rustconn-X.Y.Z.tar.xz
```

### 10b. `packaging/obs/_service`
The OBS source service builds the tarball from this git tag, so its revision
MUST point at the new release tag (the `release.sh` gate checks
`revision">vX.Y.Z<`). Easy to miss — it carries `vX.Y.Z`, not bare `X.Y.Z`.
```xml
    <param name="revision">vX.Y.Z</param>
```

### 11. `packaging/obs/AppImageBuilder.yml`
```yaml
    version: X.Y.Z
```

### 12. `packaging/flatpak/io.github.totoshko88.RustConn.yml`
```yaml
        tag: vX.Y.Z
```

### 13. `packaging/flatpak/io.github.totoshko88.RustConn.local.yml`
DO NOT modify — uses local path.

### 14. `packaging/flathub/io.github.totoshko88.RustConn.yml`
```yaml
        tag: vX.Y.Z
```

### 15. `rustconn/assets/io.github.totoshko88.RustConn.metainfo.xml`
Add `<release>` at the TOP of `<releases>`:
```xml
    <release version="X.Y.Z" date="YYYY-MM-DD">
      <description>
        <p>Version X.Y.Z - [short description]:</p>
        <ul>
          <li>[item]</li>
        </ul>
      </description>
    </release>
```
Also update `<description>` if protocols/features changed.

### 16. `snap/snapcraft.yaml` (if exists)
```yaml
version: 'X.Y.Z'
```

## Version sync checklist

> **Canonical list:** the authoritative set of packaging files + version patterns
> lives in `PKG_FILES`/`PKG_PATS` in `scripts/release.sh` — that script is the gate
> that blocks a release on drift. The table below mirrors it for preparation; if
> they ever disagree, `release.sh` wins. Update the script first when files change.

Before tagging the release, verify that version `X.Y.Z` is present in ALL files:

```bash
# Quick check (should find the version in all files):
grep -r "X.Y.Z" Cargo.toml debian/changelog packaging/ rustconn/assets/*.xml docs/USER_GUIDE.md docs/ARCHITECTURE.md
```

| File | What to verify |
|------|----------------|
| `Cargo.toml` | `version = "X.Y.Z"` |
| `debian/changelog` | `rustconn (X.Y.Z-1)` |
| `packaging/obs/debian.changelog` | `rustconn (X.Y.Z-1)` |
| `packaging/obs/rustconn.dsc` | `Version: X.Y.Z-1` |
| `packaging/obs/debian.dsc` | `Version: X.Y.Z-1` |
| `packaging/obs/rustconn.spec` | `Version: X.Y.Z` |
| `packaging/obs/rustconn.changes` | `- X.Y.Z` |
| `packaging/obs/_service` | `revision">vX.Y.Z<` |
| `packaging/obs/AppImageBuilder.yml` | `version: X.Y.Z` |
| `packaging/flatpak/*.yml` | `tag: vX.Y.Z` |
| `packaging/flathub/*.yml` | `tag: vX.Y.Z` |
| `metainfo.xml` | `<release version="X.Y.Z"` |
| `docs/USER_GUIDE.md` | `Version X.Y.Z` |
| `docs/ARCHITECTURE.md` | `Version X.Y.Z` |

## Changelog conversion

### CHANGELOG.md → Debian
```
### Added
- **Feature** — Description (#N)
```
→
```
  * Added Feature (#N):
    - Description
```

### CHANGELOG.md → RPM spec / .changes
```
### Added
- **Feature** — Description (#N)
```
→
```
- Added Feature (#N)
```

### CHANGELOG.md → Metainfo XML
- Strip markdown bold (`**...**`)
- Strip backticks
- Strip markdown links — keep text only
- Keep each `<li>` short (1 line)
- Escape XML entities: `&` → `&amp;`

## Final checklist

```bash
# 1. Code
cargo fmt --check
cargo clippy --all-targets       # 0 warnings
cargo test --workspace           # ~120s (argon2 property tests are slow in debug mode)
cargo build --release

# 2. CLI versions
./scripts/check-cli-versions.sh  # exit 0 = all current

# 3. Versions synced (all packaging files)
# 4. CHANGELOG.md contains Dependencies and Changed (CLI) sections
# 5. Packaging changelogs updated
# 6. metainfo.xml has a new <release>
# 7. Cargo.lock updated (cargo update)
```
