# RustConn

A modern connection manager for Linux, designed to manage SSH, RDP, VNC, and SPICE remote connections through a GTK4-based GUI with Wayland-native support.

![RustConn Screenshot](rustconn/assets/icons/hicolor/scalable/apps/org.rustconn.RustConn.svg)

## Supported Protocols

| Protocol | Implementation | Session Type | Default Port |
|----------|----------------|--------------|--------------|
| **SSH** | VTE terminal embedding | Embedded tab | 22 |
| **RDP** | FreeRDP (`xfreerdp`/`xfreerdp3`) or IronRDP | External window / Embedded | 3389 |
| **VNC** | TigerVNC/TightVNC or `vnc-rs` | External window / Embedded | 5900 |
| **SPICE** | `remote-viewer` or `spice-client` | External window / Embedded | 5900 |

### Protocol Details

- **SSH**: Full terminal emulation via VTE4 with PTY support, key-based and password authentication, SSH agent forwarding
- **RDP**: Windows Remote Desktop Protocol via FreeRDP, supports NLA, TLS, clipboard sharing, drive redirection
- **VNC**: Virtual Network Computing for cross-platform remote desktop, multiple security types supported
- **SPICE**: Simple Protocol for Independent Computing Environments, optimized for virtual machines

## Features

- **Multi-protocol support**: SSH (embedded terminal), RDP, VNC, and SPICE
- **Connection organization**: Groups and tags for easy management
- **Import from existing tools**: Asbru-CM, Remmina, SSH config, Ansible inventory
- **Export to multiple formats**: Asbru-CM, Remmina, SSH config, Ansible inventory
- **Secure credential storage**: KeePassXC and libsecret integration
- **Session management**: Logging capabilities and session tracking
- **Command snippets**: Variable substitution for common commands
- **Cluster commands**: Execute commands on multiple hosts simultaneously
- **Split view**: Multiple terminals in split panes
- **Wake-on-LAN**: Wake sleeping machines before connecting
- **Wayland-first**: Native GTK4/libadwaita interface
- **Zero Trust integrations**: AWS SSM, GCP IAP, Azure Bastion, Teleport, Boundary

## Performance Features

RustConn includes several performance optimizations for handling large connection databases:

- **Search Caching**: Search results are cached with configurable TTL (default 30s)
- **Lazy Loading**: Connection groups load on-demand when expanded
- **Virtual Scrolling**: Efficiently handles 1000+ connections
- **Debounced Search**: 100ms debounce prevents excessive searches
- **String Interning**: Memory optimization for repeated strings
- **Batch Processing**: Efficient import/export with progress reporting

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/totoshko88/rustconn.git
cd rustconn

# Build release version
cargo build --release

# Install (optional)
./install-desktop.sh
```

### System Dependencies

#### Ubuntu/Debian
```bash
sudo apt install libgtk-4-dev libvte-2.91-gtk4-dev libdbus-1-dev pkg-config
```

#### Fedora
```bash
sudo dnf install gtk4-devel vte291-gtk4-devel dbus-devel
```

#### Arch Linux
```bash
sudo pacman -S gtk4 vte4 dbus
```

### Protocol Client Dependencies

| Protocol | Required Package | Optional |
|----------|-----------------|----------|
| SSH | OpenSSH client | Built-in |
| RDP | `freerdp` or `freerdp3` | Yes |
| VNC | `tigervnc` or `tightvnc` | Yes |
| SPICE | `virt-viewer` (provides `remote-viewer`) | Yes |

## Optional Features

Build with optional embedded protocol clients:

```bash
# All embedded clients (default)
cargo build --release

# Specific features
cargo build --release --features vnc-embedded
cargo build --release --features rdp-embedded
cargo build --release --features spice-embedded

# System tray support
cargo build --release --features tray
```

| Feature | Crate | Description |
|---------|-------|-------------|
| `vnc-embedded` | `vnc-rs` 0.5 | Native VNC protocol embedding |
| `rdp-embedded` | `ironrdp-*` | Native RDP protocol (IronRDP) |
| `spice-embedded` | `spice-client` 0.2.0 | Native SPICE protocol embedding |
| `tray` | `ksni` + `resvg` | System tray icon support |

## Usage

```bash
# Run from source
cargo run -p rustconn

# Or after installation
rustconn

# CLI interface
cargo run -p rustconn-cli -- --help
```

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Ctrl+N` | New connection |
| `Ctrl+G` | New group |
| `Ctrl+I` | Import connections |
| `Ctrl+Shift+E` | Export connections |
| `Ctrl+,` | Settings |
| `Ctrl+Shift+S` | Split vertical |
| `Ctrl+Shift+H` | Split horizontal |
| `Ctrl+W` | Close tab |
| `Ctrl+Tab` | Next tab |
| `Ctrl+Shift+Tab` | Previous tab |
| `Ctrl+F` | Focus search |

## Configuration

Configuration files are stored in `~/.config/rustconn/`:

- `settings.toml` - Application settings
- `connections.json` - Saved connections
- `snippets.json` - Command snippets
- `templates.json` - Connection templates
- `clusters.json` - Cluster definitions

### Tracing and Debugging

```bash
# Enable debug logging
RUST_LOG=debug cargo run -p rustconn

# Module-specific logging
RUST_LOG=rustconn_core::search=trace cargo run -p rustconn
```

## Project Structure

```
rustconn/
├── rustconn/          # GTK4 GUI application
├── rustconn-core/     # Business logic library (GUI-free)
├── rustconn-cli/      # Command-line interface
└── docs/              # Documentation
```

## Building

```bash
cargo build              # Development build
cargo build --release    # Release build
cargo test               # Run all tests
cargo clippy --all-targets  # Check lints
cargo fmt --check        # Verify formatting
```

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Support the Project

If you find RustConn useful, consider supporting its development:

| Platform | Link |
|----------|------|
| ☕ Ko-Fi | [ko-fi.com/totoshko88](https://ko-fi.com/totoshko88) |
| 💳 PayPal | [PayPal QR](https://www.paypal.com/qrcodes/p2pqrc/JJLUXRZSQ5V3A) |
| 💸 Payoneer | [Payoneer Link](https://link.payoneer.com/Token?t=135B68D8EB1E4860B4B632ECD755182F&src=pl) |
| 🇺🇦 UAH | [Monobank Jar](https://send.monobank.ua/jar/2UgaGcQ3JC) |

## Acknowledgments

- GTK4 and the GNOME project
- The Rust community
- FreeRDP, TigerVNC, and SPICE projects
- All contributors and supporters

---

Made with ❤️ in Ukraine 🇺🇦
