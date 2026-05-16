<p align="center">
  <img src="rustconn/assets/icons/hicolor/128x128/apps/io.github.totoshko88.RustConn.png" width="96" alt="RustConn">
</p>
<h1 align="center">RustConn</h1>
<p align="center">
  Modern connection manager — SSH, RDP, VNC, SPICE, and more
</p>
<p align="center">
  <a href="https://flathub.org/apps/io.github.totoshko88.RustConn"><img src="https://img.shields.io/flathub/v/io.github.totoshko88.RustConn" alt="Flathub"></a>
  <a href="https://snapcraft.io/rustconn"><img src="https://img.shields.io/snapcraft/v/rustconn/latest/edge" alt="Snap"></a>
  <a href="https://aur.archlinux.org/packages/rustconn"><img src="https://img.shields.io/aur/version/rustconn" alt="AUR"></a>
  <a href="https://build.opensuse.org/package/show/home:totoshko88:rustconn/rustconn"><img src="https://img.shields.io/badge/OBS-rustconn-green" alt="OBS"></a>
  <a href="https://www.freshports.org/net/rustconn/"><img src="https://img.shields.io/badge/FreeBSD-ports-red" alt="FreeBSD"></a>
  <a href="https://github.com/totoshko88/RustConn/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/totoshko88/RustConn/ci.yml?label=CI" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0-blue" alt="License"></a>
</p>

---

RustConn is a cross-platform connection orchestrator with a GTK4/libadwaita interface.
It brings SSH, RDP, VNC, SPICE, MOSH, Telnet, Serial, Kubernetes, and Zero Trust connections under one roof — with embedded Rust clients where possible and seamless integration with external tools where needed.

[![Demo](https://img.youtube.com/vi/4cMxo-lxfQ8/maxresdefault.jpg)](https://youtu.be/4cMxo-lxfQ8)

## Features

| Category | Details |
|----------|---------|
| **Protocols** | SSH, RDP, VNC, SPICE, MOSH, Telnet, Serial, Kubernetes, Zero Trust, Web Bookmarks |
| **File Transfer** | SFTP file browser via system file manager (sftp:// URI, D-Bus portal) |
| **Organization** | Groups, tags, templates, custom icons (emoji/GTK), connection history & statistics |
| **Monitoring** | Remote host metrics bar (CPU, RAM, disk, network, load, system info) — agentless, per-connection toggle |
| **Import/Export** | Asbru-CM, Remmina, SSH config, Ansible inventory, Royal TS, MobaXterm, SecureCRT, Remote Desktop Manager, RDP files (.rdp), virt-viewer (.vv), libvirt XML, CSV, native (.rcn) |
| **Security** | KeePassXC (KDBX), libsecret, Bitwarden CLI, 1Password CLI, Passbolt CLI, Pass (passwordstore.org), script credentials |
| **Productivity** | Split terminals, command snippets, cluster broadcast, ad-hoc broadcast, smart folders, session recording, text highlighting rules, Wake-on-LAN, SSH port forwarding, automation (expect rules, key sequences, pre/post-connect tasks), session reconnect, settings backup/restore, .rdp file association, tab overview, tab pinning, custom terminal themes |
| **Cloud Sync** | Synchronize connections via shared cloud directory (Google Drive, Syncthing, Nextcloud, Dropbox); group sync with Master/Import access model; simple sync with UUID-based merge |
| **CLI** | `rustconn-cli` — headless management: list/add/update/delete connections, import/export, snippets, groups, templates, clusters, secrets, WoL, shell completions |

| Protocol | Client | Type |
|----------|--------|------|
| SSH | VTE terminal (port forwarding: -L/-R/-D) | Embedded |
| RDP | IronRDP / FreeRDP fallback (bundled in Flatpak) | Embedded + external |
| VNC | vnc-rs / vncviewer fallback | Embedded + external |
| SPICE | spice-client / remote-viewer fallback | Embedded + external |
| Telnet | VTE terminal | Embedded |
| Serial | picocom via VTE | External (bundled in Flatpak) |
| Kubernetes | kubectl exec via VTE | External |
| MOSH | mosh via VTE | External |
| Zero Trust | AWS SSM, GCP IAP, Azure, OCI, Cloudflare, Teleport, Tailscale, Boundary, Hoop.dev | External |
| Web | System browser (UriLauncher / xdg-open) | External |

## Installation

<a href="https://flathub.org/apps/io.github.totoshko88.RustConn">
  <img width="200" alt="Download on Flathub" src="https://flathub.org/api/badge?locale=en"/>
</a>

```bash
flatpak install flathub io.github.totoshko88.RustConn
```

| Method | Command / Link |
|--------|---------------|
| **Flatpak** (recommended) | `flatpak install flathub io.github.totoshko88.RustConn` |
| **Snap** | `sudo snap install rustconn` ([permissions](docs/SNAP.md)) |
| **macOS (Homebrew)** | `brew tap totoshko88/rustconn && brew install rustconn` |
| **Debian 13 / Ubuntu 24.04 / Ubuntu 26.04** | OBS apt repository ([setup](docs/INSTALL.md#debian--ubuntu-obs-repository)) |
| **openSUSE** (Tumbleweed, Slowroll, Leap 16.0) | OBS zypper repository ([setup](docs/INSTALL.md#opensuse-obs)) |
| **Fedora 43 / 44** | OBS dnf repository ([setup](docs/INSTALL.md#fedora-obs)) |
| **Arch Linux** | `yay -S rustconn` ([AUR](https://aur.archlinux.org/packages/rustconn), community) |
| **FreeBSD** | `pkg install rustconn` ([ports](https://www.freshports.org/net/rustconn/), community) |
| **AppImage** | [GitHub Releases](https://github.com/totoshko88/RustConn/releases) |
| **From source** | Rust 1.95+, GTK4 4.14+ ([build guide](docs/BUILD.md)) |

## Quick Start

| Shortcut | Action |
|----------|--------|
| `Ctrl+N` | New connection |
| `Ctrl+I` | Import |
| `Ctrl+,` | Settings |
| `Ctrl+Shift+S/H` | Split vertical/horizontal |

## Documentation

| Document | Description |
|----------|-------------|
| [User Guide](docs/USER_GUIDE.md) | Complete usage documentation |
| [Installation](docs/INSTALL.md) | All installation methods and repository setup |
| [Build Guide](docs/BUILD.md) | Building from source, feature flags, per-distro prerequisites |
| [macOS Build](docs/MACOS_BUILD.md) | Building and running on macOS, Homebrew, DMG packaging |
| [CLI Reference](docs/CLI_REFERENCE.md) | `rustconn-cli` commands and examples |
| [Architecture](docs/ARCHITECTURE.md) | Crate structure and design decisions |
| [CI & Build Flow](docs/CI_BUILD_FLOW.md) | CI pipelines, OBS packaging, Flathub release process |
| [Zero Trust](docs/ZERO_TRUST.md) | AWS SSM, GCP IAP, Azure, OCI, Cloudflare, Teleport, Tailscale, Boundary |

## Support

<p>
  <a href="https://donatello.to/totoshko88"><img src="https://img.shields.io/badge/Donatello-Support-ff5e5b" alt="Donatello"></a>
  <a href="https://ko-fi.com/totoshko88"><img src="https://img.shields.io/badge/Ko--fi-Support-ff5e5b?logo=ko-fi" alt="Ko-fi"></a>
  <a href="https://send.monobank.ua/jar/2UgaGcQ3JC"><img src="https://img.shields.io/badge/Monobank-UAH-black" alt="Monobank"></a>
</p>

## License

GPL-3.0 — Made with ❤️ in Ukraine 🇺🇦
