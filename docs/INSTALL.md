# Installation Guide

## System Requirements

- **OS:** Linux (Wayland-first, X11 supported), macOS 13+ (Ventura)
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
| spice-client | Embedded SPICE client |
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
flatpak install --user RustConn-0.15.0.flatpak
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

# Host CLI access (Zero Trust, password managers, kubectl, FreeRDP, VNC viewer)
sudo snap connect rustconn:host-usr-bin
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
must be installed on the host and accessed via the `host-usr-bin` interface.

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
| Locales | 16 languages (be, cs, da, de, es, fr, it, kk, nl, pl, pt, sk, sv, uk, uz, zh-cn) |

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

RustConn uses embedded Rust implementations for RDP, VNC, and SPICE by default.
External clients serve as fallbacks when the embedded client fails (e.g., RD Gateway).

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
