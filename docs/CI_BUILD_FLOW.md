# CI & Build Flow

This document describes the complete CI/CD pipeline for RustConn, including
GitHub Actions workflows, OBS (Open Build Service) packaging, and Flathub releases.

## Overview

```mermaid
graph LR
    Push[git push main] --> CI[CI Checks]
    Tag[git push v*] --> CI
    Tag --> Release[Release Workflow]
    Tag --> Flathub[Flathub Update]
    Release --> GHRelease[GitHub Release]
    Release --> SnapStore[Snap Store]
    Release --> OBS[OBS Update]
    OBS --> RPM[openSUSE/Fedora RPMs]
    OBS --> DEB[Debian/Ubuntu DEBs]
```

## CI Checks (`ci.yml`)

Runs on every push to `main`/`develop` and on pull requests.

```mermaid
graph TD
    Push[Push / PR] --> fmt[Format Check]
    Push --> clippy[Clippy Lint]
    Push --> check[cargo check]
    Push --> test[Test - workspace]
    Push --> core[Test Core - no GUI]
    Push --> prop[Property Tests]
    Push --> msrv[MSRV 1.95]
    Push --> deny[cargo-deny]
    Push --> audit[Security Audit]
```

| Job | What it does |
|-----|-------------|
| Format | `cargo fmt --all -- --check` |
| Clippy | `cargo clippy --all-targets -- -D warnings` |
| Check | `cargo check --all-targets` |
| Test | `cargo test --workspace` |
| Test Core | `cargo test -p rustconn-core --all-features` (no GUI deps) |
| Property Tests | `cargo test -p rustconn-core --test property_tests` (10 min timeout) |
| MSRV | `cargo check` with Rust 1.95 |
| cargo-deny | License and advisory checks |
| Security Audit | `cargo audit` with `audit.toml` ignore list |

## Release Workflow (`release.yml`)

Triggered by pushing a version tag (`v*`). This is the **single unified release workflow**
that builds all package formats, publishes to Snap Store, creates the GitHub Release,
and updates OBS.

```mermaid
graph TD
    Tag[git push tag v0.15.11] --> BuildDeb[Build .deb]
    Tag --> BuildRPM[Build .rpm]
    Tag --> BuildAppImage[Build AppImage]
    Tag --> BuildSnap[Build .snap]
    Tag --> BuildFlatpak[Build .flatpak]

    BuildDeb --> |ubuntu-24.04| DebArtifact[rustconn_0.15.11_amd64.deb]
    BuildRPM --> |fedora:44 container| RPMArtifact[rustconn-0.15.11-1.fc44.x86_64.rpm]
    BuildAppImage --> |ubuntu-24.04| AppImageArtifact[RustConn-0.15.11-x86_64.AppImage]
    BuildSnap --> |LXD + snapcraft| SnapArtifact[rustconn_0.15.11_amd64.snap]
    BuildFlatpak --> |GNOME 50 container| FlatpakArtifact[RustConn-0.15.11.flatpak]

    BuildSnap --> |continue-on-error| SnapStore[Publish to Snap Store edge]

    DebArtifact --> Release[Create GitHub Release]
    RPMArtifact --> Release
    AppImageArtifact --> Release
    SnapArtifact --> Release
    FlatpakArtifact --> Release

    Release --> UpdateOBS[Update OBS Package]
```

### Build Jobs

| Job | Runner | Output |
|-----|--------|--------|
| `build-deb` | ubuntu-24.04 | `rustconn_X.Y.Z_amd64.deb` |
| `build-rpm` | fedora:44 container | `rustconn-X.Y.Z-1.fc44.x86_64.rpm` |
| `build-appimage` | ubuntu-24.04 + linuxdeploy | `RustConn-X.Y.Z-x86_64.AppImage` |
| `build-snap` | ubuntu-latest + LXD + snapcraft | `rustconn_X.Y.Z_amd64.snap` |
| `build-flatpak` | GNOME 50 Flatpak container | `RustConn-X.Y.Z.flatpak` |

### Snap Store Publishing

The `build-snap` job also publishes to Snap Store (edge channel) after building.
This step uses `continue-on-error: true` — if publishing fails (e.g., awaiting
manual review/approval on Snap Store), the job still succeeds and the `.snap`
artifact is included in the GitHub Release.

### GitHub Release Artifacts

The `release` job collects artifacts from all five build jobs and creates a GitHub
Release with `.deb`, `.rpm`, `.AppImage`, `.snap`, and `.flatpak` files attached.

## OBS Update Flow

The `update-obs` job runs after the GitHub Release is created. OBS VMs have no internet
access, so all dependencies must be bundled as tarballs.

```mermaid
sequenceDiagram
    participant CI as GitHub Actions
    participant OBS as build.opensuse.org

    CI->>CI: cargo vendor → vendor.tar.zst (109 MB)
    CI->>CI: Rust sysroot → rust-toolchain.tar.zst (172 MB)
    CI->>CI: Extract changelog from CHANGELOG.md
    CI->>OBS: osc checkout package
    CI->>OBS: Update _service (tag revision)
    CI->>OBS: Update rustconn.spec (Version)
    CI->>OBS: Update debian.dsc (Version, DEBTRANSFORM-TAR)
    CI->>OBS: Update debian.changelog (prepend entry)
    CI->>OBS: Update rustconn.changes (prepend entry)
    CI->>OBS: Upload vendor.tar.zst
    CI->>OBS: Upload rust-toolchain.tar.zst
    CI->>OBS: osc commit
    CI->>OBS: osc rebuild (all targets)

    OBS->>OBS: Source service: git clone → rustconn-X.Y.Z.tar.xz
    OBS->>OBS: Build 8 targets in parallel
```

### OBS Build Targets

```mermaid
graph TD
    OBS[OBS Package] --> TW[openSUSE Tumbleweed]
    OBS --> SR[openSUSE Slowroll]
    OBS --> Leap[openSUSE Leap 16.0]
    OBS --> Fed44[Fedora 44]
    OBS --> Fed43[Fedora 43]
    OBS --> Deb[Debian 13 Trixie]
    OBS --> U24[Ubuntu 24.04 LTS]
    OBS --> U26[Ubuntu 26.04 LTS]

    TW --> |rustconn.spec| RPM1[.rpm]
    SR --> |rustconn.spec| RPM2[.rpm]
    Leap --> |rustconn.spec| RPM3[.rpm]
    Fed44 --> |rustconn.spec + rust-toolchain| RPM4[.rpm]
    Fed43 --> |rustconn.spec + rust-toolchain| RPM5[.rpm]
    Deb --> |debian.dsc + rust-toolchain| DEB1[.deb]
    U24 --> |debian.dsc + rust-toolchain| DEB2[.deb]
    U26 --> |debian.dsc + rust-toolchain| DEB3[.deb]
```

### How Rust is Provided per Target

| Target | Rust source | Why |
|--------|------------|-----|
| openSUSE Tumbleweed | `devel:languages:rust` repo | System Rust may lag behind MSRV |
| openSUSE Slowroll | `devel:languages:rust` repo | Same as Tumbleweed |
| openSUSE Leap 16.0 | `devel:languages:rust` repo | System Rust too old |
| Fedora 44 | `rust-toolchain.tar.zst` | No BuildRequires for Rust; bundled for consistency |
| Fedora 43 | `rust-toolchain.tar.zst` | No BuildRequires for Rust; bundled for consistency |
| Debian 13 | `rust-toolchain.tar.zst` | No Rust in repos >= 1.95, no internet |
| Ubuntu 24.04 | `rust-toolchain.tar.zst` | System Rust is only 1.75 (MSRV requires 1.95), no internet for rustup |
| Ubuntu 26.04 | `rust-toolchain.tar.zst` | May have Rust 1.95+, but bundled for consistency |

### libadwaita Feature Flags

OBS builds auto-detect the libadwaita version and enable appropriate features:

| libadwaita | Feature flag | Distros |
|-----------|-------------|---------|
| 1.8+ | `adw-1-8` | Tumbleweed, Slowroll, Fedora 43, Ubuntu 26.04 |
| 1.7 | `adw-1-7` | Leap 16.0, Fedora 44, Debian 13 |
| 1.5 | (baseline) | Ubuntu 24.04 |

For RPM builds (`rustconn.spec`), feature flags are set via `%if` macros based on distro version.
For DEB builds (`debian.rules`), `pkg-config --modversion libadwaita-1` detects the version at build time.

## Flathub Release (Semi-automated)

Flathub updates are semi-automated. The `flathub-update.yml` workflow generates the
necessary files, but the PR to Flathub must be created manually.

```mermaid
sequenceDiagram
    participant Dev as Developer
    participant GH as GitHub Actions
    participant FH as flathub/io.github.totoshko88.RustConn

    Dev->>GH: Push tag v0.15.11
    GH->>GH: Generate cargo-sources.json
    GH->>GH: Update manifest tag
    GH->>GH: Upload artifact: flathub-update-v0.15.11

    Dev->>Dev: Download artifact
    Dev->>FH: Create branch
    Dev->>FH: Upload manifest + cargo-sources.json
    Dev->>FH: Create PR "Update to v0.15.11"
    FH->>FH: Flathub CI builds and tests
    FH->>FH: Merge → published to Flathub
```

### Flathub Update Steps

1. Wait for `flathub-update.yml` to complete after tagging
2. Download the `flathub-update-vX.Y.Z` artifact from GitHub Actions
3. Go to https://github.com/flathub/io.github.totoshko88.RustConn
4. Create a new branch, upload `io.github.totoshko88.RustConn.yml` and `cargo-sources.json`
5. Create a PR — Flathub CI will build and test automatically
6. After CI passes, merge the PR — the update is published to Flathub

Note: `flathub.json` has `automerge-flathubbot-prs: true`, so Flathub Bot PRs
(triggered by `x-checker-data` in the manifest) are auto-merged after CI passes.
However, `cargo-sources.json` still needs manual regeneration.

## Standalone Workflows (Manual Testing)

These workflows are triggered only via `workflow_dispatch` for manual testing:

| Workflow | Purpose |
|----------|---------|
| `flatpak.yml` | Test Flatpak build without creating a release |
| `snap.yml` | Test Snap build without publishing |

## CLI Version Check (`check-cli-versions.yml`)

Runs weekly (Monday 06:00 UTC) to check for updates to bundled CLI tools
(kubectl, Tailscale, Cloudflare, Boundary, Teleport, Bitwarden, 1Password, Hoop.dev, TigerVNC).

## Files That Need Manual Updates at Release Time

| File | What to update | Automated? |
|------|---------------|-----------|
| `Cargo.toml` | workspace version | Release Version hook |
| `CHANGELOG.md` | Release notes | Manual |
| `debian/changelog` | Debian changelog entry | Release Version hook |
| `packaging/obs/debian.changelog` | OBS Debian changelog | CI (release.yml) |
| `packaging/obs/rustconn.changes` | OBS RPM changelog | CI (release.yml) |
| `packaging/obs/rustconn.spec` | Version field | CI (release.yml) |
| `packaging/obs/debian.dsc` | Version + tarball name | CI (release.yml) |
| `packaging/obs/_service` | Tag revision | CI (release.yml) |
| `snap/snapcraft.yaml` | version field | Release Version hook |
| `metainfo.xml` | `<release>` entry | Release Version hook |
