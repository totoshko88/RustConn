# Installation Guide

## System Requirements

- **OS:** Linux (Wayland-first, X11 supported), macOS 13+ (Ventura),
  Windows via WSL2/WSLg (see [WSL.md](WSL.md))
- **GTK:** 4.14+
- **libadwaita:** 1.5+
- **Rust:** 1.95+ (for building from source)

## Flatpak (Recommended)

RustConn is available on [Flathub](https://flathub.org/apps/io.github.totoshko88.RustConn):

<a href="https://flathub.org/apps/io.github.totoshko88.RustConn">
  <img width="200" alt="Download on Flathub" src="https://flathub.org/api/badge?locale=en"/>
</a>

```bash
# Install from Flathub
flatpak install flathub io.github.totoshko88.RustConn

# Run
flatpak run io.github.totoshko88.RustConn
```

### Flatpak Permissions

RustConn requests the following permissions for full functionality:

| Permission | Purpose |
|------------|---------|
| `--share=network` | SSH/RDP/VNC/SPICE/Telnet connections |
| `--socket=wayland` / `--socket=fallback-x11` | Display access |
| `--socket=pulseaudio` | RDP session audio playback |
| `--socket=ssh-auth` | SSH agent access |
| `--device=all` | Serial port access (picocom requires `--device=all`; no granular option exists) |
| `--filesystem=home/.ssh:ro` | Read SSH keys and config |
| `--filesystem=home/.aws` | AWS CLI credentials (read-write for SSO token cache) |
| `--filesystem=home/.config/gcloud:ro` | GCP CLI credentials |
| `--filesystem=home/.azure:ro` | Azure CLI credentials |
| `--filesystem=home/.kube:ro` | Kubernetes config |
| `--filesystem=xdg-download:create` | SFTP file transfers via Midnight Commander |
| `--talk-name=org.freedesktop.secrets` | GNOME Keyring access |
| `--talk-name=org.kde.kwalletd5/6` | KWallet access |
| `--talk-name=org.keepassxc.KeePassXC.BrowserServer` | KeePassXC proxy |
| `--talk-name=org.kde.StatusNotifierWatcher` | System tray support |

### Bundled Components

The Flatpak includes all core protocol clients — no separate installation needed:

| Component | Purpose |
|-----------|---------|
| VTE 0.80 | Terminal emulation (SSH, Telnet, Serial, Kubernetes) |
| IronRDP | Embedded RDP client |
| vnc-rs | Embedded VNC client |
| inetutils | Telnet client |
| picocom | Serial console (RS-232/USB) |
| Midnight Commander | SFTP file browser |
| waypipe | Wayland application forwarding over SSH |
| libsecret | GNOME Keyring / KWallet integration |
| openssh | SSH client |

External CLIs (Zero Trust providers, password managers, kubectl) are executed on the host
via `flatpak-spawn --host`. Install them on the host system if needed.

### Install from CI Bundle

CI builds a `.flatpak` bundle on every tagged release and on manual `workflow_dispatch` runs.
The bundle is available in two places:

- **GitHub Release** — file `RustConn-<version>.flatpak` attached to the release
- **CI Artifacts** — file `RustConn.flatpak` in the Actions → Flatpak workflow run artifacts

#### Prerequisites

The bundle requires GNOME Platform runtime 50. Install it once:

```bash
flatpak remote-add --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak install flathub org.gnome.Platform//50
```

#### Install

```bash
flatpak install --user RustConn-<version>.flatpak
```

Confirm runtime dependency installation if prompted.

#### Update

Install a newer bundle with the same command — Flatpak will offer to replace the existing version.

#### Extra filesystem access

The sandbox grants read-only access to `~/.ssh` and read-write to `~/Downloads` by default.
To expose additional directories:

```bash
flatpak override --user --filesystem=/path/to/dir io.github.totoshko88.RustConn
```

## Snap (Strict Confinement)

RustConn snap uses strict confinement with embedded protocol clients. Both `rustconn` (GUI)
and `rustconn-cli` are included.

```bash
# Install
sudo snap install rustconn
```

### Required Interface Connections

```bash
# SSH keys (required for SSH connections)
sudo snap connect rustconn:ssh-keys
```

### Optional Interface Connections

```bash
# Serial port access (for serial console connections)
sudo snap connect rustconn:serial-port

# Cloud provider credentials
sudo snap connect rustconn:aws-credentials
sudo snap connect rustconn:gcloud-credentials
sudo snap connect rustconn:azure-credentials
sudo snap connect rustconn:oci-credentials

# Kubernetes config
sudo snap connect rustconn:kube-credentials
```

### Bundled in Snap

| Component | Purpose |
|-----------|---------|
| openssh-client | SSH client |
| inetutils-telnet | Telnet client |
| picocom | Serial console |
| Midnight Commander | SFTP file browser |
| waypipe | Wayland forwarding |

External CLIs (Zero Trust providers, password managers, kubectl, FreeRDP, VNC viewer)
are downloaded on demand via the Components dialog (Menu → Components) inside the sandbox.

### CLI in Snap

```bash
# The CLI is available as a separate snap app
rustconn.rustconn-cli --help
rustconn.rustconn-cli list
```

See [docs/SNAP.md](SNAP.md) for detailed snap documentation.

## AppImage

```bash
chmod +x RustConn-*-x86_64.AppImage
./RustConn-*-x86_64.AppImage
```

## Verifying a File Downloaded from a GitHub Release

Every `.deb`, `.rpm`, AppImage and Flatpak bundle attached to a GitHub release from
0.20.0 onwards carries a signed [SLSA build provenance
attestation](https://slsa.dev/spec/v1.0/provenance). Check one with the
[GitHub CLI](https://cli.github.com/):

```bash
gh attestation verify RustConn-0.20.0-x86_64.AppImage --repo totoshko88/RustConn
```

A pass means GitHub holds a Sigstore-signed statement that this exact file — bound
by both its SHA-256 digest and its filename — was produced by this repository's
release workflow, from a named commit, on GitHub-hosted runners. The signature uses
a short-lived certificate issued to that workflow run, so it cannot be reproduced
by anyone who merely has a copy of the repository. Verification fails if the file
was rebuilt somewhere else, or altered after the run — including a rename.

The command fetches the attestation from GitHub, so it needs network access. For an
air-gapped check, download the bundle alongside the artifact
(`gh attestation download <file> --repo totoshko88/RustConn`) and pass it with
`--bundle`.

Not covered:

- **The snap.** It is built by a separate post-release job and the Snap Store signs
  it with its own key, which `snapd` verifies on install.
- **Packages installed from a repository** — Flathub, OBS, AUR, nixpkgs, Homebrew.
  Those are signed by the repository or tap that serves them and verified by the
  package manager; the attestation covers the files on the release page only.

## macOS (Homebrew)

RustConn is available via a Homebrew Tap. All dependencies (GTK4, libadwaita, VTE,
Adwaita icons, OpenSSL, D-Bus) are installed automatically by Homebrew.

Requires macOS 13 (Ventura) or later.

```bash
# Install
brew tap totoshko88/rustconn
brew install rustconn

# Launch GUI
rustconn-app
# or: open $(brew --prefix)/opt/rustconn/RustConn.app

# Optional: add to Applications / Launchpad
ln -sf $(brew --prefix)/opt/rustconn/RustConn.app /Applications/RustConn.app
```

### What Gets Installed

| Component | Description |
|-----------|-------------|
| `rustconn` | GUI application binary |
| `rustconn-cli` | CLI tool for scripting and automation |
| `rustconn-app` | Convenience launcher (opens .app bundle) |
| `RustConn.app` | macOS application bundle with icon and environment setup |
| Locales | 17 languages (be, cs, da, de, es, fr, it, ka, kk, nl, pl, pt, sk, sv, uk, uz, zh_CN) |

### Optional: Password Manager CLIs

```bash
brew install --cask keepassxc      # KeePassXC (local database)
brew install bitwarden-cli         # Bitwarden
brew install --cask 1password-cli  # 1Password
brew install pass                  # Pass (GPG-based)
```

### Updating

```bash
brew update && brew upgrade rustconn
```

### Uninstalling

```bash
brew uninstall rustconn
brew untap totoshko88/rustconn
rm -f /Applications/RustConn.app
```

### Building from Source on macOS

See [docs/MACOS_BUILD.md](MACOS_BUILD.md) for manual build instructions, DMG packaging,
and troubleshooting.

## Debian / Ubuntu (OBS Repository)

Pre-built packages are available from the openSUSE Build Service:

```bash
# Debian 13 (Trixie)
echo 'deb http://download.opensuse.org/repositories/home:/totoshko88:/rustconn/Debian_13/ /' \
  | sudo tee /etc/apt/sources.list.d/rustconn.list
curl -fsSL https://download.opensuse.org/repositories/home:/totoshko88:/rustconn/Debian_13/Release.key \
  | gpg --dearmor | sudo tee /etc/apt/trusted.gpg.d/rustconn.gpg > /dev/null
sudo apt update
sudo apt install rustconn

# Ubuntu 24.04 LTS (Noble)
echo 'deb http://download.opensuse.org/repositories/home:/totoshko88:/rustconn/xUbuntu_24.04/ /' \
  | sudo tee /etc/apt/sources.list.d/rustconn.list
curl -fsSL https://download.opensuse.org/repositories/home:/totoshko88:/rustconn/xUbuntu_24.04/Release.key \
  | gpg --dearmor | sudo tee /etc/apt/trusted.gpg.d/rustconn.gpg > /dev/null
sudo apt update
sudo apt install rustconn

# Ubuntu 26.04 LTS (Resolute)
echo 'deb http://download.opensuse.org/repositories/home:/totoshko88:/rustconn/xUbuntu_26.04/ /' \
  | sudo tee /etc/apt/sources.list.d/rustconn.list
curl -fsSL https://download.opensuse.org/repositories/home:/totoshko88:/rustconn/xUbuntu_26.04/Release.key \
  | gpg --dearmor | sudo tee /etc/apt/trusted.gpg.d/rustconn.gpg > /dev/null
sudo apt update
sudo apt install rustconn
```

Debian 12 (bookworm) is not supported — GTK4 4.8 is too old (4.14+ required).

Alternatively, download `.deb` from [GitHub Releases](https://github.com/totoshko88/RustConn/releases):

```bash
sudo dpkg -i rustconn_*_amd64.deb
sudo apt-get install -f  # Install dependencies if needed
```

## Fedora (OBS)

```bash
# Fedora 44
sudo dnf config-manager addrepo --from-repofile=https://download.opensuse.org/repositories/home:/totoshko88:/rustconn/Fedora_44/home:totoshko88:rustconn.repo
sudo dnf install rustconn

# Fedora 43
sudo dnf config-manager addrepo --from-repofile=https://download.opensuse.org/repositories/home:/totoshko88:/rustconn/Fedora_43/home:totoshko88:rustconn.repo
sudo dnf install rustconn
```

## openSUSE (OBS)

```bash
# Tumbleweed
sudo zypper ar https://download.opensuse.org/repositories/home:/totoshko88:/rustconn/openSUSE_Tumbleweed/ rustconn
sudo zypper ref
sudo zypper in rustconn

# Slowroll
sudo zypper ar https://download.opensuse.org/repositories/home:/totoshko88:/rustconn/openSUSE_Slowroll/ rustconn
sudo zypper ref
sudo zypper in rustconn

# Leap 16.0
sudo zypper ar https://download.opensuse.org/repositories/home:/totoshko88:/rustconn/openSUSE_Leap_16.0/ rustconn
sudo zypper ref
sudo zypper in rustconn
```

OBS packages use tiered feature flags based on the distro's libadwaita version:
`adw-1-8` for Tumbleweed/Slowroll/Fedora 43+/Ubuntu 26.04, `adw-1-7` for Leap 16.0/Debian 13,
baseline for Ubuntu 24.04 LTS.

All OBS packages: https://build.opensuse.org/package/show/home:totoshko88:rustconn/rustconn

## NixOS / Nix

RustConn provides a Nix flake for NixOS and any system with Nix installed:

```bash
# Run without installing
nix run github:totoshko88/RustConn

# Install into user profile
nix profile install github:totoshko88/RustConn

# Or add to your NixOS/home-manager configuration as a flake input:
# flake.nix
{
  inputs.rustconn.url = "github:totoshko88/RustConn";

  # In your system/home config:
  # environment.systemPackages = [ inputs.rustconn.packages.x86_64-linux.default ];
}
```

### Build from flake locally

```bash
git clone https://github.com/totoshko88/RustConn.git
cd RustConn
nix build
./result/bin/rustconn
```

### Notes

- The flake builds against nixpkgs `unstable` for GTK4 4.14+ and libadwaita 1.5+.
- Both `rustconn` (GUI) and `rustconn-cli` are included in the default package.
- NixOS stable (24.11) should work — GTK4 4.16 and libadwaita 1.6 are available there.
- Tests are skipped during `nix build` (`doCheck = false`) — the full test suite takes ~120s due to argon2 property tests and is validated in CI instead.
- After cloning, run `nix flake lock` to generate `flake.lock` for reproducible builds (the lock file is not committed to avoid pinning nixpkgs for all users).

## Arch Linux (AUR, community-maintained)

```bash
# Using yay
yay -S rustconn

# Using paru
paru -S rustconn
```

AUR package: https://aur.archlinux.org/packages/rustconn

## FreeBSD (Ports, community-maintained)

```bash
pkg install rustconn
```

FreeBSD port: https://www.freshports.org/net/rustconn/

## From Source

Requires Rust 1.95+, GTK4 4.14+, libadwaita 1.5+, VTE4, and system libraries
(OpenSSL, ALSA, D-Bus, clang, cmake, gettext).

See [docs/BUILD.md](BUILD.md) for per-distro prerequisite packages, feature flags,
testing, debugging, and local Flatpak builds.

```bash
git clone https://github.com/totoshko88/RustConn.git
cd RustConn
cargo build --release
```

The binaries will be at `target/release/rustconn` and `target/release/rustconn-cli`.

To enable newer libadwaita widgets for your distro, add `--features adw-1-8` (GNOME 49+)
or `--features adw-1-7` (GNOME 48). See [BUILD.md — Feature Flags](BUILD.md#feature-flags)
for the full list.

```bash
./install-desktop.sh   # optional: installs .desktop file, icon, and locales
```

## Dependencies

### Required Runtime
- GTK4 (4.14+)
- VTE4 (terminal emulation)
- libadwaita (1.5+)
- D-Bus
- OpenSSL

### Optional Protocol Clients

RustConn uses embedded Rust implementations for RDP and VNC by default.
External clients serve as fallbacks when the embedded client fails (e.g., RD Gateway).
SPICE always uses an external viewer (remote-viewer / virt-viewer).

FreeRDP detection priority: `wlfreerdp3` > `wlfreerdp` > `sdl-freerdp3` > `sdl-freerdp` > `xfreerdp3` > `xfreerdp`

| Protocol | Client | Package |
|----------|--------|---------|
| RDP (fallback) | FreeRDP 3 (Wayland) | `freerdp3` or `freerdp2` |
| VNC (fallback) | TigerVNC | `tigervnc-viewer` |
| SPICE (fallback) | remote-viewer | `virt-viewer` |
| Telnet | telnet | `telnet` or `inetutils-telnet` |
| Serial | picocom | `picocom` |
| SFTP | Midnight Commander | `mc` |
| Wayland forwarding | waypipe | `waypipe` |
| Kubernetes | kubectl | `kubectl` |
| MOSH | mosh | `mosh` |

### H.264 in the Embedded RDP Client

The embedded RDP client can use the GFX pipeline with H.264/AVC420, which costs
noticeably less bandwidth and CPU against a modern Windows host than the RemoteFX
path it otherwise falls back to. It needs an OpenH264 library — and **a
distribution package will not do**, however recent it is:

```
OpenH264 at this path is not one of Cisco's published binaries, so the loader
refuses it — this is expected for a distribution package.
```

The loader (`openh264-sys2`, via `ironrdp-egfx`) compares the library's SHA-256
against a list of the binaries **Cisco itself publishes** and refuses anything
else. That is not a bug to work around: Cisco pays the H.264 patent royalties for
the binaries it distributes, which is why the upstream crate documents downloading
the library from Cisco rather than building or packaging it. `libopenh264-8`,
Fedora's `libopenh264` and a local build from the Cisco *source* tarball are all
refused for the same reason.

To enable it, fetch a matching binary from
[ciscobinary.openh264.org](http://ciscobinary.openh264.org/) (the versions the
loader accepts are listed in `openh264-sys2`'s `blobs/hashes.txt` — 2.6.0 at the
time of writing) and point RustConn at it:

```bash
mkdir -p ~/.local/lib
# extract the downloaded .so, e.g. libopenh264-2.6.0-linux64.8.so
export RUSTCONN_OPENH264=~/.local/lib/libopenh264-2.6.0-linux64.8.so
```

`RUSTCONN_OPENH264` is read when a session first sets up its GFX pipeline, and
the path is tried before every other candidate, so no root and no system
directory is needed. Set it in your session's environment (or in the `.desktop`
file's `Exec=` line) to make it stick.

Without it the session simply uses RemoteFX and works. Note this affects the
**embedded** client only — the external FreeRDP fallback has no such check and
uses the distribution package for `/gfx:AVC420`, which is why `libopenh264`
remains a recommended package.

### Optional Password Managers

| Manager | CLI | Installation |
|---------|-----|--------------|
| Bitwarden | `bw` | `npm install -g @bitwarden/cli` or [bitwarden.com](https://bitwarden.com/help/cli/) |
| 1Password | `op` | [1password.com/downloads/command-line](https://1password.com/downloads/command-line/) |
| KeePassXC | `keepassxc-cli` | `keepassxc` package |
| Passbolt | `go-passbolt-cli` | [passbolt.com](https://www.passbolt.com/) |
| Pass | `pass` | `pass` package ([passwordstore.org](https://www.passwordstore.org/)) |

### Zero Trust CLI Tools

| Provider | CLI | Installation |
|----------|-----|--------------|
| AWS SSM | `aws` + SSM plugin | [AWS CLI](https://aws.amazon.com/cli/) |
| GCP IAP | `gcloud` | [Google Cloud SDK](https://cloud.google.com/sdk) |
| Azure | `az` | [Azure CLI](https://docs.microsoft.com/cli/azure/) |
| OCI | `oci` | [OCI CLI](https://docs.oracle.com/iaas/tools/oci-cli/) |
| Cloudflare | `cloudflared` | [Cloudflare Tunnel](https://developers.cloudflare.com/cloudflare-one/connections/connect-apps/) |
| Teleport | `tsh` | [Teleport](https://goteleport.com/) |
| Tailscale | `tailscale` | [Tailscale](https://tailscale.com/) |
| Boundary | `boundary` | [HashiCorp Boundary](https://www.boundaryproject.io/) |

## Rust Installation

RustConn requires Rust 1.95+. See [docs/BUILD.md — Prerequisites](BUILD.md#prerequisites)
for installation instructions.

## Verification

After installation, verify RustConn works:

```bash
rustconn-cli --version

rustconn-cli --help
# Shows CLI commands
```

## Uninstallation

**Flatpak:**
```bash
flatpak uninstall io.github.totoshko88.RustConn
```

**Snap:**
```bash
sudo snap remove rustconn
```

**macOS (Homebrew):**
```bash
brew uninstall rustconn
brew untap totoshko88/rustconn
rm -f /Applications/RustConn.app
```

**Debian/Ubuntu:**
```bash
sudo apt remove rustconn
```

**From source:**
```bash
rm -rf ~/.local/share/applications/rustconn.desktop
rm -rf ~/.local/share/icons/hicolor/*/apps/rustconn.*
rm -f ~/.local/bin/rustconn
rm -f ~/.local/bin/rustconn-cli
```

Configuration is stored in `~/.config/rustconn/` — remove manually if needed.
