# RustConn

A modern connection manager for Linux, designed to manage SSH, RDP, and VNC remote connections through a GTK4-based GUI with Wayland-native support.

## Demo

[![RustConn Demo](https://img.youtube.com/vi/2Z-cX56RSC0/maxresdefault.jpg)](https://youtu.be/2Z-cX56RSC0)

*Click the image above to watch the demo video*

![RustConn Logo](rustconn/assets/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg)

## Features

- **Multi-protocol support**: SSH (embedded terminal), RDP, VNC, and SPICE
- **Connection organization**: Groups and tags for easy management
- **Import from existing tools**: Asbru-CM, Remmina, SSH config, Ansible inventory
- **Secure credential storage**: KeePassXC and libsecret integration
- **Session management**: Logging capabilities and session tracking
- **Command snippets**: Variable substitution for common commands
- **Split view**: Multiple terminals in split panes
- **Wayland-first**: Native GTK4 interface
- **Performance optimizations**: Smart caching, lazy loading, and virtual scrolling

## Performance Features

RustConn includes several performance optimizations for handling large connection databases:

- **Search Caching**: Search results are cached with configurable TTL (default 30s) for instant repeated searches
- **Lazy Loading**: Connection groups load on-demand when expanded, reducing startup time
- **Virtual Scrolling**: Efficiently handles 1000+ connections by rendering only visible items
- **Debounced Search**: 100ms debounce prevents excessive searches during rapid typing
- **String Interning**: Memory optimization for repeated strings (protocol names, hostnames)
- **Batch Processing**: Efficient import/export of large connection sets with progress reporting

## Installation

### Flatpak

```bash
# Install from Flatpak bundle (after release)
flatpak install RustConn-0.5.1.flatpak

# Run
flatpak run io.github.totoshko88.RustConn
```

### AppImage

```bash
chmod +x RustConn-0.5.1-x86_64.AppImage
./RustConn-0.5.1-x86_64.AppImage
```

### Debian/Ubuntu

```bash
sudo dpkg -i rustconn_0.5.1-1_amd64.deb
sudo apt-get install -f  # Install dependencies if needed
```

### openSUSE (OBS)

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

### Dependencies

- GTK4 (4.12+)
- VTE4 (for terminal emulation)
- FreeRDP (xfreerdp/xfreerdp3) for RDP connections
- TigerVNC or TightVNC for VNC connections
- remote-viewer (fallback for SPICE connections)

### Rust Dependencies

Key crates used by RustConn:

| Crate                 | Version | Purpose                           |
|-----------------------|---------|-----------------------------------|
| `gtk4`                | 0.10    | GTK4 GUI framework                |
| `vte4`                | 0.9     | Terminal emulation                |
| `tokio`               | 1.48    | Async runtime                     |
| `tracing`             | 0.1     | Structured logging and diagnostics|
| `tracing-subscriber`  | 0.3     | Log formatting and filtering      |
| `spice-client`        | 0.2.0   | Native SPICE protocol             |
| `ironrdp`             | 0.13    | Native RDP protocol               |
| `vnc-rs`              | 0.5     | Native VNC protocol               |
| `serde`               | 1.x     | Serialization/deserialization     |
| `secrecy`             | 0.8     | Secure credential handling        |

### Embedded RDP Features (IronRDP)

The following capabilities are available:
- Bidirectional clipboard (copy/paste between local and remote)
- Shared folders (access local directories from Windows)
- RemoteFX codec for better image quality
- Dynamic resolution changes
- Audio playback (RDPSND) with optional `rdp-audio` feature

## Usage

```bash
# Run from source
cargo run -p rustconn

# Or after installation
rustconn
```

## Keyboard Shortcuts

| Shortcut          | Action             |
|-------------------|--------------------|
| `Ctrl+N`          | New connection     |
| `Ctrl+I`          | Import connections |
| `Ctrl+,`          | Settings           |
| `Ctrl+Shift+S`    | Split vertical     |
| `Ctrl+Shift+H`    | Split horizontal   |
| `Ctrl+W`          | Close tab          |
| `Ctrl+Tab`        | Next tab           |
| `Ctrl+Shift+Tab`  | Previous tab       |

## Building

```bash
# Development build
cargo build

# Release build
cargo build --release

# Run tests
cargo test

# Check lints
cargo clippy --all-targets
```

## Configuration

### Tracing and Debugging

RustConn uses the `tracing` crate for structured logging. Configure log levels via environment variables:

```bash
# Enable debug logging
RUST_LOG=debug cargo run -p rustconn

# Enable trace logging for specific modules
RUST_LOG=rustconn_core::search=trace cargo run -p rustconn

# Enable performance profiling spans
RUST_LOG=info cargo run -p rustconn
```

### Performance Tuning

The following settings can be adjusted in the Settings dialog:

- **Search Cache TTL**: How long search results are cached (default: 30 seconds)
- **Search Cache Size**: Maximum cached queries (default: 100 entries)
- **Virtual Scrolling Threshold**: Connection count to enable virtual scrolling (default: 100)
- **Batch Size**: Items per batch for import/export operations (default: 50)

## License

This project is licensed under the GPL-3.0 License - see the [LICENSE](LICENSE) file for details.

## Support the Project

If you find RustConn useful, consider supporting its development:

| Platform    | Link                                                                                          | Description                 |
|-------------|-----------------------------------------------------------------------------------------------|-----------------------------|
| ☕ Ko-Fi    | [ko-fi.com/totoshko88](https://ko-fi.com/totoshko88)                                          | One-time or monthly support |
| 💳 PayPal   | [PayPal QR](https://www.paypal.com/qrcodes/p2pqrc/JJLUXRZSQ5V3A)                              | International payments      |
| 💸 Payoneer | [Payoneer Link](https://link.payoneer.com/Token?t=135B68D8EB1E4860B4B632ECD755182F&src=pl)    | International transfers     |
| 🇺🇦 UAH     | [Monobank Jar](https://send.monobank.ua/jar/2UgaGcQ3JC)                                       | Ukrainian hryvnia           |

Your support helps maintain and improve RustConn!

## Acknowledgments

- GTK4 and the GNOME project
- The Rust community
- FreeRDP project
- All contributors and supporters

---

Made with ❤️ in Ukraine 🇺🇦
