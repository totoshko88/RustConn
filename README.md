# RustConn

Manage remote connections easily.

RustConn is a connection orchestrator for Linux with a GTK4/Wayland-native interface.
It brings SSH, RDP, VNC, SPICE, Telnet, Serial, Kubernetes, and Zero Trust connections under one roof — with embedded Rust clients where possible and seamless integration with external tools where needed.

[![Demo](https://img.youtube.com/vi/DpYM0UbR9TM/maxresdefault.jpg)](https://youtu.be/DpYM0UbR9TM)

## Features

| Category | Details |
|----------|---------|
| **Protocols** | SSH, RDP, VNC, SPICE, Telnet, Serial, Kubernetes, Zero Trust |
| **File Transfer** | SFTP file browser via system file manager (sftp:// URI, D-Bus portal) |
| **Organization** | Groups, tags, templates, connection history & statistics |
| **Import/Export** | Asbru-CM, Remmina, SSH config, Ansible inventory, Royal TS, MobaXterm, virt-viewer (.vv), native (.rcn) |
| **Security** | KeePassXC (KDBX), libsecret, Bitwarden CLI, 1Password CLI, Passbolt CLI integration |
| **Productivity** | Split terminals, command snippets, cluster commands, Wake-on-LAN, SSH port forwarding, startup action |
| **CLI** | `rustconn-cli` — headless management: list/add/update/delete connections, import/export, snippets, groups, templates, clusters, secrets, WoL, shell completions |

| Protocol | Client | Type |
|----------|--------|------|
| SSH | VTE terminal (port forwarding: -L/-R/-D) | Embedded |
| RDP | IronRDP / FreeRDP fallback | Embedded + external |
| VNC | vnc-rs / vncviewer fallback | Embedded + external |
| SPICE | remote-viewer | External |
| Telnet | VTE terminal | Embedded |
| Serial | picocom via VTE | External (bundled in Flatpak) |
| Kubernetes | kubectl exec via VTE | External |
| Zero Trust | AWS SSM, GCP IAP, Azure, OCI, Cloudflare, Teleport, Tailscale, Boundary | External |

## Installation

| Method | Notes |
|--------|-------|
| **Flatpak** | Recommended. Extensions mechanism built-in, sandboxed |
| **Package manager** | Uses system dependencies (GTK4, VTE, libadwaita) |
| **Snap** | Strict confinement, requires additional permissions |
| **From source** | Requires Rust 1.88+, GTK4 dev libraries |

<a href="https://flathub.org/apps/io.github.totoshko88.RustConn">
  <img width="200" alt="Download on Flathub" src="https://flathub.org/api/badge?locale=en"/>
</a>

```bash
flatpak install flathub io.github.totoshko88.RustConn
```

**Snap** / **AppImage** / **Debian** / **openSUSE (OBS)** — see [Installation Guide](docs/INSTALL.md)

```bash
# Snap (strict confinement - requires interface connections)
sudo snap install rustconn
sudo snap connect rustconn:ssh-keys
# See docs/SNAP.md for all required permissions
```

```bash
# From source
git clone https://github.com/totoshko88/rustconn.git
cd rustconn
cargo build --release
./target/release/rustconn
```

**Build dependencies:** GTK4 4.14+, VTE4, libadwaita, Rust 1.88+ | **Optional:** FreeRDP, TigerVNC, virt-viewer, picocom, kubectl


## Quick Start

| Shortcut | Action |
|----------|--------|
| `Ctrl+N` | New connection |
| `Ctrl+I` | Import |
| `Ctrl+,` | Settings |
| `Ctrl+Shift+S/H` | Split vertical/horizontal |

Full documentation: [User Guide](docs/USER_GUIDE.md)

## Support

[![Ko-Fi](https://img.shields.io/badge/Ko--Fi-Support-ff5e5b?logo=ko-fi)](https://ko-fi.com/totoshko88)
[![PayPal](https://img.shields.io/badge/PayPal-Donate-00457C?logo=paypal)](https://paypal.me/totoshko88)
[![Monobank](https://img.shields.io/badge/Monobank-UAH-black?logo=monobank)](https://send.monobank.ua/jar/2UgaGcQ3JC)

## License

GPL-3.0 — Made with ❤️ in Ukraine 🇺🇦
