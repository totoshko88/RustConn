# RustConn OBS Packaging

Build files for the [Open Build Service](https://build.opensuse.org/package/show/home:totoshko88:rustconn/rustconn).

## Supported Distributions

| Distribution | Version | Rust Source |
|-------------|---------|-------------|
| openSUSE Tumbleweed | Rolling | System (devel:languages:rust) |
| openSUSE Slowroll | Rolling | System (devel:languages:rust) |
| openSUSE Leap | 16.0 | devel:languages:rust |
| Fedora | 44 | System |
| Fedora | 43 | System |
| Debian | 13 (Trixie) | Bundled toolchain |
| Ubuntu | 26.04 LTS | Bundled toolchain |
| Ubuntu | 24.04 LTS | Bundled toolchain |

This table used to carry GTK4 and libadwaita version columns per distro. They are
gone on purpose. Nothing read them, they needed an edit every time any of these
eight distros moved, and they were wrong: they claimed GTK 4.18 for Fedora 43/44,
Tumbleweed and Ubuntu 26.04, all of which carry 4.20 or 4.22 — Ubuntu 26.04 was
measured at GTK 4.22.4 with libadwaita 1.9.1 on 2026-09-03. A version number
written down here is a number nobody re-checks, so the build no longer depends on
one: `rustconn.spec` and `debian.rules` ask `pkg-config` at build time.

**MSRV:** 1.95 (Minimum Supported Rust Version)

### Rust Toolchain Strategy

- **openSUSE:** System Rust from `devel:languages:rust` repository (1.95+)
- **Fedora:** Bundled standalone toolchain (`rust-toolchain.tar.zst`) — system Rust may lag behind MSRV
- **Debian/Ubuntu:** Bundled standalone toolchain — system Rust is too old

### Feature Flags

`rustconn.spec` and `debian.rules` both pick the version features at build time
with `pkg-config --atleast-version`, highest match first. There is no distro table
to keep in step, and a distro that moves up gets the newer features on its next
rebuild without a commit here.

| Flag | Enabled when |
|------|--------------|
| `adw-1-10` / `adw-1-8` / `adw-1-7` / `adw-1-6` | `libadwaita-1` ≥ 1.10 / 1.8 / 1.7 / 1.6 |
| `gtk-4-24` / `gtk-4-22` / `gtk-4-20` / `gtk-4-18` | `gtk4` ≥ 4.24 / 4.22 / 4.20 / 4.18 |
| `vte-0-84` / `vte-0-78` | `vte-2.91-gtk4` ≥ 0.84 / 0.78 |
| `web-embedded` | `webkitgtk-6.0` present |
| (none of the above) | the workspace floor: GTK 4.14, libadwaita 1.5, VTE 0.76 |

Two traps this arrangement exists to avoid, both of which had already fired:

- **`--atleast-version`, never a glob over `--modversion`.** A `case` pattern of
  `1.8*|1.9*` does not match libadwaita `1.10`, so the first distro to ship 1.10
  would have dropped silently to the 1.5 baseline.
- **In `debian.rules`, nothing but backslash-continued lines may follow the
  comment block in `override_dh_auto_build`.** A comment inside the continuation
  chain ends it, and make gives each chain its own shell, so variables assigned
  before the comment are empty by the time `cargo` runs. That is what happened
  before 0.21.5: every OBS Debian and Ubuntu package was built with no `adw-1-*`
  feature and no `web-embedded`, because one comment sat in the middle of the
  recipe. Only VTE survived, having been detected after the last comment.

To see what a build actually chose, look for the `=== gtk4 … | libadwaita … ===`
line in the OBS build log. It prints every detected version next to the flag it
selected, which is the check that would have caught both traps.

## File Structure

| File | Purpose |
|------|---------|
| `_meta` | OBS project metadata (repositories, architectures) |
| `_service` | Source download service (git tag checkout) |
| `_multibuild` | Multi-build flavors: `standard` + `appimage` |
| `rustconn.spec` | RPM spec for openSUSE / Fedora |
| `rustconn.changes` | RPM changelog (OBS format) |
| `rustconn.dsc` | Debian source control, `3.0 (quilt)` form — **not the file OBS builds from**, see below |
| `debian.dsc` | Debian source control, `1.0` form — **this is the live one** |
| `debian.changelog` | Debian changelog |
| `debian.control` | Debian build/runtime dependencies |
| `debian.copyright` | Debian copyright file |
| `debian.rules` | Debian build rules |
| `AppImageBuilder.yml` | AppImage configuration |

### Which `.dsc` is live

`scripts/obs-publish.sh` rewrites `Version:` and `DEBTRANSFORM-TAR:` in
**`debian.dsc`** only; nothing in the pipeline reads `rustconn.dsc`. It is kept
because the OBS project has historically carried both forms and removing a source
control file from a live project is not something to do blind, but treat
`debian.dsc` as the authority. Their `Build-Depends` lines are held identical
deliberately: when they drifted, `rustconn.dsc` was missing `libadwaita-1-dev`
altogether, which is exactly the kind of difference nobody notices in a file that
is never built. `.dsc` is a strict deb822 file with no comment syntax, hence this
note lives here.

## Build Dependencies

### RPM (openSUSE)

```
cargo >= 1.95, rust >= 1.95, cargo-packaging
pkgconfig(gtk4) >= 4.14, pkgconfig(vte-2.91-gtk4), pkgconfig(libadwaita-1)
pkgconfig(dbus-1), pkgconfig(openssl), alsa-devel
zstd, gcc, make, gettext-tools
```

### RPM (Fedora)

```
pkgconfig(gtk4) >= 4.14, pkgconfig(vte-2.91-gtk4), pkgconfig(libadwaita-1)
pkgconfig(dbus-1), pkgconfig(openssl), alsa-lib-devel
zstd, gcc, make, gettext-devel
# Rust provided via bundled toolchain (rust-toolchain.tar.zst)
```

### DEB (Debian / Ubuntu)

```
libgtk-4-dev (>= 4.14), libvte-2.91-gtk4-dev, libadwaita-1-dev
libssl-dev, libasound2-dev, pkg-config, clang, cmake, gettext, zstd
# Rust provided via bundled toolchain (rust-toolchain.tar.zst)
```

## CI Automation

When a new release tag is pushed to GitHub, the OBS workflow automatically:

1. Updates `_service` with the new tag
2. Copies `rustconn.changes` and `rustconn.spec`
3. Commits changes to OBS via `osc`
4. Triggers rebuild across all repositories

### Required GitHub Secrets

| Secret | Description |
|--------|-------------|
| `OBS_USERNAME` | Login for build.opensuse.org |
| `OBS_PASSWORD` | Password for build.opensuse.org |

## Manual Operations

### Project Setup

```bash
# Install osc
# openSUSE: sudo zypper install osc
# Fedora:   sudo dnf install osc

# Checkout project
osc checkout home:totoshko88:rustconn/rustconn
cd home:totoshko88:rustconn/rustconn
```

### Update Project Metadata

```bash
# Apply _meta (add/remove repositories)
osc meta prj home:totoshko88:rustconn -F packaging/obs/_meta
```

### Useful Commands

```bash
# Build status for all repos
osc results home:totoshko88:rustconn rustconn

# Build log for a specific repo
osc buildlog home:totoshko88:rustconn rustconn Fedora_43 x86_64

# Local test build
osc build openSUSE_Tumbleweed x86_64

# Trigger rebuild (all repos)
osc rebuild home:totoshko88:rustconn rustconn

# Trigger rebuild (single repo)
osc rebuild home:totoshko88:rustconn rustconn Fedora_43 x86_64
```

## Installation

See [docs/INSTALL.md](../../docs/INSTALL.md) for per-distro installation commands.

All packages: https://build.opensuse.org/package/show/home:totoshko88:rustconn/rustconn

## Troubleshooting

### Rust version too old

Fedora and Debian/Ubuntu builds use a bundled Rust toolchain (`rust-toolchain.tar.zst`)
unpacked during `%prep`. If the toolchain archive is missing or corrupt, the build fails
with "rustc: command not found". Re-upload the archive to OBS.

### ALSA not found

Add `alsa-devel` (openSUSE) or `alsa-lib-devel` (Fedora) to BuildRequires.

### GTK4 version mismatch

Requires GTK4 ≥ 4.14. Available in:
- openSUSE Tumbleweed / Slowroll / Leap 16.0
- Fedora 43+
- Ubuntu 24.04+
- Debian 13+

### VTE package name differs

- openSUSE: `vte` (provides `pkgconfig(vte-2.91-gtk4)`)
- Fedora: `vte291-gtk4` / `vte291-gtk4-devel`
- Debian/Ubuntu: `libvte-2.91-gtk4-0` / `libvte-2.91-gtk4-dev`
