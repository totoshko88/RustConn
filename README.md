<p align="center">
  <img src="rustconn/assets/icons/hicolor/128x128/apps/io.github.totoshko88.RustConn.png" width="96" alt="RustConn">
</p>
<h1 align="center">RustConn</h1>
<p align="center">
  Modern connection manager — SSH, RDP, VNC, SPICE, and more
</p>
<p align="center">
  <a href="https://flathub.org/apps/io.github.totoshko88.RustConn"><img src="https://img.shields.io/flathub/v/io.github.totoshko88.RustConn" alt="Flathub"></a>
  <a href="https://snapcraft.io/rustconn"><img src="https://img.shields.io/snapcraft/v/rustconn/latest/stable" alt="Snap"></a>
  <a href="https://aur.archlinux.org/packages/rustconn"><img src="https://img.shields.io/aur/version/rustconn" alt="AUR"></a>
  <a href="https://build.opensuse.org/package/show/home:totoshko88:rustconn/rustconn"><img src="https://img.shields.io/badge/dynamic/xml?url=https%3a%2f%2fapi.opensuse.org%2fpublic%2fbuild%2fhome%3atotoshko88%3arustconn%2fopenSUSE_Tumbleweed%2fx86_64%2frustconn%3fview%3dbinaryversions&query=substring-before%28substring-after%28%2f%2fbinary%5bcontains%28%40name%2c%27x86_64.rpm%27%29%5d%2f%40name%2c%27rustconn-%27%29%2c%27-%27%29&label=OBS&color=green" alt="OBS"></a>
  <a href="https://www.freshports.org/net/rustconn/"><img src="https://repology.org/badge/version-for-repo/freebsd/rustconn.svg?header=FreeBSD" alt="FreeBSD"></a>
  <a href="https://github.com/totoshko88/RustConn/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/totoshko88/RustConn/ci.yml?label=CI" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0-blue" alt="License"></a>
</p>

---

RustConn is a cross-platform connection orchestrator with a GTK4/libadwaita interface.
It brings SSH, RDP, VNC, SPICE, MOSH, Telnet, Serial, Kubernetes, and Zero Trust connections under one roof — with embedded Rust clients where possible and seamless integration with external tools where needed.
Runs on Linux (GTK4/libadwaita), macOS, FreeBSD, and Windows via WSLg.

[![Demo](https://img.youtube.com/vi/huYZkuxtsQg/maxresdefault.jpg)](https://youtu.be/huYZkuxtsQg)

## Screenshots

| | |
|---|---|
| ![Welcome screen](rustconn/assets/screenshots/welcome_light.png) | ![Embedded RDP session](rustconn/assets/screenshots/rdp_session_dark.png) |
| ![Connection wizard](rustconn/assets/screenshots/connection_wizard_light.png) | ![SFTP file browser](rustconn/assets/screenshots/sftp_mc_light.png) |

More screenshots on the [Flathub listing](https://flathub.org/apps/io.github.totoshko88.RustConn).

## Features

### Capabilities

| Category | Details |
|----------|---------|
| **File Transfer** | SFTP file browser via system file manager (sftp:// URI, D-Bus portal) |
| **Organization** | Groups, tags, templates, custom icons (emoji/GTK), connection history & statistics |
| **Monitoring** | Remote host metrics bar (CPU, RAM, disk, network, load, system info) — agentless, per-connection toggle |
| **Import/Export** | Asbru-CM, Remmina, SSH config, Ansible inventory, Royal TS, MobaXterm, SecureCRT, Remote Desktop Manager, RDP files (.rdp), virt-viewer (.vv), libvirt (XML + daemon), CSV, native (.rcn) |
| **Security** | KeePassXC (KDBX), libsecret, macOS Keychain, Bitwarden CLI, 1Password CLI, Passbolt CLI, Pass (passwordstore.org), script credentials, encrypted-file fallback (no keyring required); hardware-token SSH auth (FIDO2 security keys, PKCS#11 / YubiKey / smart cards) |
| **Terminal** | Split terminals, command snippets, text highlighting rules, session recording, custom terminal themes, tab overview, tab pinning |
| **Automation** | Expect rules, key sequences, pre/post-connect tasks, cluster & ad-hoc broadcast, session reconnect, Wake-on-LAN |
| **Workflow** | Smart folders, SSH port forwarding, visual SSH tunnel builder, workspace profiles (save/restore open sessions), settings backup/restore, .rdp file association |
| **Cloud Sync** | Synchronize connections via shared cloud directory (Google Drive, Syncthing, Nextcloud, Dropbox); group sync with Master/Import access model; simple sync with UUID-based merge |
| **CLI** | `rustconn-cli` — headless management: list/add/update/delete connections, import/export, snippets, groups, templates, clusters, WoL, shell completions; optional secret and client-launch commands behind features |
| **Languages** | English + 16 translations (Belarusian, Czech, Danish, German, Spanish, French, Italian, Kazakh, Dutch, Polish, Portuguese, Slovak, Swedish, Ukrainian, Uzbek, Chinese) |

### Protocol support

| Protocol | Client | Type |
|----------|--------|------|
| SSH | VTE terminal (port forwarding: -L/-R/-D) | Embedded |
| RDP | IronRDP / FreeRDP fallback (bundled in Flatpak) | Embedded + external |
| VNC | vnc-rs / vncviewer fallback | Embedded + external |
| SPICE | remote-viewer / virt-viewer | External |
| Telnet | VTE terminal | Embedded |
| Serial | picocom via VTE | External (bundled in Flatpak) |
| Kubernetes | kubectl exec via VTE | External |
| MOSH | mosh via VTE | External |
| Zero Trust | AWS SSM, GCP IAP, Azure, OCI, Cloudflare, Teleport, Tailscale, Boundary, Hoop.dev | External |
| Web | Embedded WebKitGTK 6.0 browser (Linux) / System browser / Custom browser command | Embedded + external |

## Installation

<p>
  <a href="https://flathub.org/apps/io.github.totoshko88.RustConn">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://flathub.org/assets/badges/flathub-badge-i-en.svg">
      <img height="56" alt="Download on Flathub" src="https://flathub.org/assets/badges/flathub-badge-en.svg">
    </picture>
  </a>
  &nbsp;
  <a href="https://snapcraft.io/rustconn">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://snapcraft.io/en/light/install.svg">
      <img height="56" alt="Get it from the Snap Store" src="https://snapcraft.io/en/dark/install.svg">
    </picture>
  </a>
</p>

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
| **Windows (WSL2)** | Runs under WSLg ([guide](docs/WSL.md)) |
| **From source** | Rust 1.95+, GTK4 4.14+ ([build guide](docs/BUILD.md)) |
| **Nix (flake)** | `nix profile install github:totoshko88/RustConn` or add to your flake inputs |

Keyboard shortcuts are customizable in Settings → Keybindings; press `Ctrl+?`
in the app for the complete, always-current list.

## Documentation

| Document | Description |
|----------|-------------|
| [User Guide](docs/USER_GUIDE.md) | Complete usage documentation |
| [Installation](docs/INSTALL.md) | All installation methods and repository setup |
| [Build Guide](docs/BUILD.md) | Building from source, feature flags, per-distro prerequisites |
| [macOS Build](docs/MACOS_BUILD.md) | Building and running on macOS, Homebrew, DMG packaging |
| [Windows / WSL2](docs/WSL.md) | Running RustConn on Windows via WSLg |
| [CLI Reference](docs/CLI_REFERENCE.md) | `rustconn-cli` commands and examples |
| [Architecture](docs/ARCHITECTURE.md) | Crate structure and design decisions |
| [CI & Build Flow](docs/CI_BUILD_FLOW.md) | CI pipelines, OBS packaging, Flathub release process |
| [Zero Trust](docs/ZERO_TRUST.md) | Setup guides for each supported Zero Trust provider |
| [Changelog](CHANGELOG.md) | Release history and notable changes |

## Contributing

Bug reports and feature requests are welcome on the
[issue tracker](https://github.com/totoshko88/RustConn/issues).

- **Code** — see the [Build Guide](docs/BUILD.md) and
  [Architecture](docs/ARCHITECTURE.md) to get started.
- **Translations** — RustConn ships 16 languages; `.po` files live in
  [`po/`](po/). New languages and corrections are appreciated.

## Support

<p>
  <a href="https://donatello.to/totoshko88"><img src="https://img.shields.io/badge/Donatello-Support-ff5e5b" alt="Donatello"></a>
  <a href="https://ko-fi.com/totoshko88"><img src="https://img.shields.io/badge/Ko--fi-Support-ff5e5b?logo=ko-fi" alt="Ko-fi"></a>
  <a href="https://send.monobank.ua/jar/2UgaGcQ3JC"><img src="https://img.shields.io/badge/Monobank-UAH-black" alt="Monobank"></a>
</p>

## License

GPL-3.0 — Made with ❤️ in Ukraine 🇺🇦
