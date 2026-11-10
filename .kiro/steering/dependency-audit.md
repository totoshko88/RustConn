---
inclusion: manual
description: "On-demand audit of ALL dependency types: Cargo crates, security advisories, bundled CLI tools, Flatpak runtime/SDK/bundled libs, and Snap base/extension. Checks for available updates and version drift. Reports only — never auto-applies."
---

Run the mechanical half, then do the half that needs a web lookup and a decision.
Report findings only — do **not** apply updates.

```bash
scripts/dep-audit.sh --quiet     # writes target/dep-audit.txt
```

That covers, and you should not re-derive any of it by hand:

- **Cargo crates** — `cargo update --dry-run --verbose`, classified. In-range
  updates are counted; everything held back is grouped as MAJOR (breaking), minor
  (requirement needs widening), patch-held (something pins it explicitly — the
  interesting case, since a patch bump needs no requirement change) and
  pre-release pin (almost always a transitive dependency, not our choice).
- **Security advisories** — `cargo deny check advisories`, which reads `deny.toml`,
  the single source of truth for the RustSec ignore list. Falls back to
  `cargo audit` and says so, because the fallback does not honour those ignores.
- **Bundled CLI tools** — `scripts/check-cli-versions.sh`. Most tools auto-resolve
  their latest version at install time; TigerVNC is the one static pin. A weekly
  GitHub Action (`check-cli-versions.yml`) watches these too.

## Then do these, which the script cannot

1. **Flatpak** — read `packaging/flatpak/io.github.totoshko88.RustConn.yml`:
   - `runtime-version` of `org.gnome.Platform` / `org.gnome.Sdk` (currently '50')
     against the latest stable GNOME runtime on Flathub.
   - `org.freedesktop.Sdk.Extension.rust-stable` — usually tracks the freedesktop
     runtime; note if the base moved.
   - Bundled pinned sources with `x-checker-data` (FreeRDP `freerdp-X.Y.Z.tar.xz`,
     cJSON): compare against upstream latest and report the drift **with the new
     `sha256`**, because a version bump without it fails the build.
   - Compare `packaging/flathub/*.yml` with the local manifest. They drift
     independently; report a drift as a maintainer task and do not edit the
     Flathub copy (`packaging/AGENTS.md`, "Flathub is maintainer-only").
2. **Snap** — read `snap/snapcraft.yaml`: `base` (core26) and the `gnome` extension
   (gnome-core26 platform, mesa-2604 GPU). The snap moved to core26 once Snapcraft
   9.2 added core26 support to the extension (issue #174); it still trails the
   Flatpak's GNOME runtime by a release or two. Also check pinned
   `stage-packages` / `build-packages`.
3. **Judgement on the cargo findings.** A MAJOR bump needs its upstream changelog
   read before it is recommended. A patch-level hold needs the pin found and
   explained. Neither is something the classification alone answers.

## Summary to produce

Counts per category, then recommended actions ordered by risk, then stop. The
developer decides what to update. Any update that does land is recorded in
`CHANGELOG.md` under `### Dependencies`.
