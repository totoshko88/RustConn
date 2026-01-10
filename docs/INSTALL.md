# Installation Guide

## System Requirements

- **OS:** Linux (Wayland-first, X11 supported)
- **GTK:** 4.14+
- **libadwaita:** 1.6+
- **Rust:** 1.87+ (for building from source)

## Flatpak (Recommended)

```bash
# Install from Flathub (when available)
flatpak install flathub io.github.totoshko88.RustConn

# Or install from bundle
flatpak install RustConn-0.5.9.flatpak

# Run
flatpak run io.github.totoshko88.RustConn
```

## AppImage

```bash
chmod +x RustConn-0.5.9-x86_64.AppImage
./RustConn-0.5.9-x86_64.AppImage
```

## Debian/Ubuntu

```bash
sudo dpkg -i rustconn_0.5.9-1_amd64.deb
sudo apt-get install -f  # Install dependencies if needed
```

## openSUSE (OBS)

```bash
# Tumbleweed
sudo zypper ar https://download.opensuse.org/repositories/home:/totoshko88:/rustconn/openSUSE_Tumbleweed/ rustconn
sudo zypper ref
sudo zypper in rustconn

# Leap 16.0
sudo zypper ar https://download.opensuse.org/repositories/home:/totoshko88:/rustconn/openSUSE_Leap_16.0/ rustconn
sudo zypper ref
sudo zypper in rustconn
```

## From Source

### Prerequisites

**Ubuntu/Debian:**
```bash
sudo apt install build-essential libgtk-4-dev libvte-2.91-gtk4-dev \
    libadwaita-1-dev libdbus-1-dev pkg-config libasound2-dev
```

**Fedora:**
```bash
sudo dnf install gcc gtk4-devel vte291-gtk4-devel libadwaita-devel \
    dbus-devel alsa-lib-devel
```

**openSUSE:**
```bash
sudo zypper install gcc gtk4-devel vte-devel libadwaita-devel \
    dbus-1-devel alsa-devel
```

**Arch Linux:**
```bash
sudo pacman -S base-devel gtk4 vte4 libadwaita dbus alsa-lib
```

### Build

```bash
git clone https://github.com/totoshko88/RustConn.git
cd RustConn
cargo build --release
```

The binary will be at `target/release/rustconn`.

### Install (optional)

```bash
./install-desktop.sh
```

This installs the desktop file and icon for application menu integration.

## Dependencies

### Required Runtime
- GTK4 (4.14+)
- VTE4 (terminal emulation)
- libadwaita (1.6+)
- D-Bus

### Optional Protocol Clients

| Protocol | Client | Package |
|----------|--------|---------|
| RDP | FreeRDP | `freerdp2-x11` or `freerdp3` |
| VNC | TigerVNC | `tigervnc-viewer` |
| SPICE | remote-viewer | `virt-viewer` |

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

RustConn requires Rust 1.87+ (MSRV). Install via [rustup](https://rustup.rs/):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
rustup update
```

## Optional Features

Build with specific features:

```bash
# With RDP audio support (requires libasound2-dev)
cargo build --release --features rdp-audio

# With native SPICE embedding
cargo build --release --features spice-embedded

# With system tray icon
cargo build --release --features tray-icon
```

## Verification

After installation, verify RustConn works:

```bash
rustconn --version
# RustConn 0.5.9

rustconn-cli --help
# Shows CLI commands
```

## Uninstallation

**Flatpak:**
```bash
flatpak uninstall io.github.totoshko88.RustConn
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
```

Configuration is stored in `~/.config/rustconn/` — remove manually if needed.
