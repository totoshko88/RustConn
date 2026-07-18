---
inclusion: manual
description: "On-demand audit of ALL dependency types: Cargo crates, security advisories, bundled CLI tools, Flatpak runtime/SDK/bundled libs, and Snap base/extension. Checks for available updates and version drift. Reports only — never auto-applies."
---

Run a comprehensive dependency audit across EVERY dependency type. Report findings only — do NOT apply updates.

1. **Cargo crates** — `cargo update --dry-run 2>&1`; summarize updates grouped as patch (safe) / minor (review) / major (breaking). Use the web_search tool for any major bump to note breaking changes.

2. **Security advisories** — `cargo deny check advisories` (reads deny.toml, the single source of truth for the RustSec ignore list). Fall back to `cargo audit` only if cargo-deny is unavailable; if neither is installed, skip and note it.

3. **Bundled CLI tools** — run `scripts/check-cli-versions.sh` if present; otherwise read `rustconn-core/src/cli_download.rs` and list pinned versions (TigerVNC, Teleport, Tailscale, Boundary, Bitwarden CLI, 1Password CLI, kubectl). Note which resolve 'latest' at runtime (AWS CLI, SSM, gcloud, Azure, OCI, cloudflared) and need no pin update. A weekly GitHub Action (`check-cli-versions.yml`) also monitors these.

4. **Flatpak dependencies** — read `packaging/flatpak/io.github.totoshko88.RustConn.yml` and check:
   - `runtime-version` of `org.gnome.Platform` / `org.gnome.Sdk` (currently '50') against the latest stable GNOME runtime on Flathub (web_search 'latest GNOME Platform flatpak runtime version').
   - `org.freedesktop.Sdk.Extension.rust-stable` — usually tracks the freedesktop runtime; note if the freedesktop base moved.
   - Bundled source modules with a pinned version + `x-checker-data` (FreeRDP `freerdp-X.Y.Z.tar.xz`, cJSON, and any others): compare the pinned version against upstream latest (FreeRDP → pub.freerdp.com/releases or its x-checker-data project-id). Report drift and the matching `sha256` that would need updating.
   - Confirm `packaging/flathub/*.yml` runtime/tag stays in sync with the flatpak manifest.

5. **Snap dependencies** — read `snap/snapcraft.yaml` and check:
   - `base` (currently core24) and the `gnome` extension platform (gnome-46-2404). Note that the gnome extension is only available for core22/core24; flag if a core26 gnome extension has shipped (it would let the snap match the Flatpak's GNOME 50 / libadwaita 1.8 — see issue #174 context).
   - Any `stage-packages` / `build-packages` pinned versions (e.g. VTE, waypipe) that have known newer releases in the core24 (noble) archive.

6. **Summary** — concise report with counts and recommended actions per category:
   - Cargo: N patch / N minor / N major outdated; any advisories
   - CLI: tools with newer upstream versions (pinned only)
   - Flatpak: GNOME runtime drift, rust extension, bundled-lib drift (FreeRDP/cJSON) with new sha256
   - Snap: base/extension status, staged-package drift
   - Recommended actions, ordered by risk. The developer decides what to update.
