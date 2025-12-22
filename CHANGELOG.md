# Changelog

All notable changes to RustConn will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

#### Protocol Support
- **SSH** - Embedded VTE terminal with full PTY support, key-based and password authentication
- **RDP** - Windows Remote Desktop via FreeRDP (`xfreerdp`/`xfreerdp3`), optional embedded IronRDP
- **VNC** - Virtual Network Computing via TigerVNC/TightVNC, optional embedded `vnc-rs`
- **SPICE** - VM-optimized protocol via `remote-viewer`, optional embedded `spice-client`

#### Connection Management
- Connection organization with groups and tags
- Nested group hierarchy support
- Drag-and-drop connection reordering
- Connection templates for quick setup
- Copy/paste connections with automatic name deduplication

#### Import/Export
- Import from Asbru-CM, Remmina, SSH config, Ansible inventory
- Export to Asbru-CM, Remmina, SSH config, Ansible inventory
- Native `.rustconn` format for backup and migration
- Batch import/export with progress reporting

#### Credential Management
- KeePassXC integration via browser protocol
- Direct KDBX file access
- libsecret (GNOME Keyring/KDE Wallet) support
- Secure credential caching with `SecretString`

#### Session Features
- Session logging with configurable formats
- Split terminal view (horizontal/vertical)
- Command snippets with variable substitution
- Cluster commands for multi-host execution
- Wake-on-LAN support

#### Zero Trust Integrations
- AWS Systems Manager (SSM)
- Google Cloud IAP
- Azure Bastion
- HashiCorp Boundary
- Teleport
- Cloudflare Access
- Tailscale

#### Performance Optimizations
- Search result caching with configurable TTL
- Lazy loading for connection groups
- Virtual scrolling for large connection lists
- String interning for memory optimization
- Debounced search (100ms)

#### UI/UX
- GTK4/libadwaita native interface
- Wayland-first design
- System tray integration (optional)
- Adaptive tab bar
- Dashboard with session statistics

### Changed
- Reduced Clippy `#[allow(...)]` directives in GUI code
- Improved module documentation

### Security
- All credentials wrapped in `secrecy::SecretString`
- No plaintext password storage
- `unsafe_code = "forbid"` enforced across all crates
- Clippy `pedantic` and `nursery` lints enabled

## [0.1.0] - TBD

- Initial public release

---

## Protocol Support Matrix

| Protocol | External Client | Embedded Client | Default Port |
|----------|-----------------|-----------------|--------------|
| SSH | OpenSSH | VTE4 terminal | 22 |
| RDP | FreeRDP | IronRDP (optional) | 3389 |
| VNC | TigerVNC | vnc-rs (optional) | 5900 |
| SPICE | remote-viewer | spice-client (optional) | 5900 |

## Feature Flags

| Flag | Description |
|------|-------------|
| `vnc-embedded` | Native VNC via `vnc-rs` (default) |
| `rdp-embedded` | Native RDP via IronRDP (default) |
| `spice-embedded` | Native SPICE via `spice-client` |
| `tray` | System tray icon support |

[Unreleased]: https://github.com/totoshko88/rustconn/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/totoshko88/rustconn/releases/tag/v0.1.0
