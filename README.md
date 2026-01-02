# RustConn

Modern connection manager for Linux with GTK4/Wayland-native interface.

[![Demo](https://img.youtube.com/vi/2Z-cX56RSC0/maxresdefault.jpg)](https://youtu.be/2Z-cX56RSC0)

## Features

| Category | Details |
|----------|---------|
| **Protocols** | SSH (embedded), RDP, VNC, SPICE, Zero Trust (AWS SSM, GCP IAP, Azure, OCI, Cloudflare, Teleport, Tailscale, Boundary) |
| **Organization** | Groups, tags, templates, connection history & statistics |
| **Import/Export** | Asbru-CM, Remmina, SSH config, Ansible inventory, Royal TS, native (.rcn) |
| **Security** | KeePassXC (KDBX), libsecret integration |
| **Productivity** | Split terminals, command snippets, cluster commands, Wake-on-LAN |

## Installation

**Flatpak** / **AppImage** / **Debian** / **openSUSE (OBS)** — see [Installation Guide](docs/INSTALL.md)

```bash
# From source
git clone https://github.com/totoshko88/rustconn.git
cd rustconn
cargo build --release
./target/release/rustconn
```

**Dependencies:** GTK4 4.14+, VTE4, libadwaita | **Optional:** FreeRDP, TigerVNC, virt-viewer

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
[![PayPal](https://img.shields.io/badge/PayPal-Donate-00457C?logo=paypal)](https://www.paypal.com/qrcodes/p2pqrc/JJLUXRZSQ5V3A)
[![Monobank](https://img.shields.io/badge/Monobank-UAH-black?logo=monobank)](https://send.monobank.ua/jar/2UgaGcQ3JC)

## License

GPL-3.0 — Made with ❤️ in Ukraine 🇺🇦
