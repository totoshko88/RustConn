# Changelog

All notable changes to RustConn will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0] - Planned

### Added
- Real-time connection status indicators in the sidebar (green/red dots) to show connected/disconnected state.
- Support for custom cursors in RDP sessions (server-side cursor updates).
- Full integration of "Expect" automation engine:
    - Regex-based pattern matching on terminal output.
    - Automatic response injection.
    - Support for "one-shot" triggers.
- Terminal improvements:
    - Added context menu (Right-click) with Copy, Paste, and Select All options.
    - Added keyboard shortcuts: Ctrl+Shift+C (Copy) and Ctrl+Shift+V (Paste).
- Refactored `Connection` model to support extensible automation configuration (`AutomationConfig`).

### Changed
- Updated `thiserror` from 1.0 to 2.0 (backwards compatible, no API changes required)
- Note: `picky` remains pinned at `=7.0.0-rc.17` due to sspi 0.16.0 incompatibility with newer versions

## [0.4.2] - 2025-12-25

### Fixed
- Asbru-CM import now correctly parses installed Asbru configuration (connections inside `environments` key)
- Application icon now properly resolves in all installation scenarios (system, Flatpak, local, development)

### Changed
- Icon theme search paths extended to support multiple installation methods

## [0.4.1] - 2025-12-25

### Added
- IronRDP audio backend (RDPSND) with PCM format support (48kHz, 44.1kHz, 22.05kHz)
- Optional `rdp-audio` feature for audio playback via cpal (requires libasound2-dev)
- Bidirectional clipboard improvements for embedded RDP sessions

### Changed
- Updated MSRV to 1.87 (required by zune-jpeg 0.5.8)
- Updated dependencies: tempfile 3.24, criterion 0.8, cpal 0.17

## [0.4.0] - 2025-12-24

### Added
- Zero Trust: Improved UI by hiding irrelevant fields (Host, Port, Username, Password, Tags) when Zero Trust protocol is selected.

### Changed
- Upgraded `ironrdp` to version 0.13 (async API support).
- Refactored `rustconn-core` to improve code organization and maintainability.
- Made `spice-embedded` feature mandatory for better integration.

## [0.3.1] - 2025-12-23

### Changed
- Code cleanup: fixed all Clippy warnings (pedantic, nursery)
- Applied rustfmt formatting across all crates
- Added Deactivation-Reactivation sequence handling for RDP sessions

### Fixed
- Removed sensitive clipboard debug logging (security improvement)
- Fixed nested if statements and match patterns in RDPDR module

## [0.3.0] - 2025-12-23

### Added
- IronRDP clipboard integration for embedded RDP sessions (bidirectional copy/paste)
- IronRDP shared folders (RDPDR) support for embedded RDP sessions
- RemoteFX codec support for better RDP image quality
- RDPSND channel (required for RDPDR per MS-RDPEFS spec)

### Changed
- Migrated IronRDP dependencies from GitHub to crates.io (version 0.11)
- Reduced verbose logging in RDPDR module (now uses tracing::debug/trace)

### Fixed
- Pinned sspi to 0.16.0 and picky to 7.0.0-rc.16 to avoid rand_core conflicts

## [0.2.0] - 2025-12-22

### Added
- Tree view state persistence (expanded/collapsed folders saved between sessions)
- Native format (.rcn) import/export with proper group hierarchy preservation

### Fixed
- RDP embedded mode window sizing now uses saved window geometry
- Sidebar reload now preserves expanded/collapsed state
- Group hierarchy correctly maintained during native format import

### Changed
- Dependencies updated:
  - `ksni` 0.2 → 0.3 (with blocking feature)
  - `resvg` 0.44 → 0.45
  - `dirs` 5.0 → 6.0
  - `criterion` 0.5 → 0.6
- Migrated from deprecated `criterion::black_box` to `std::hint::black_box`

### Removed
- Removed obsolete TODO comment and unused variable in window.rs

## [0.1.0] - 2025-12-01

### Added
- Initial release of RustConn connection manager
- Multi-protocol support: SSH, RDP, VNC, SPICE
- Zero Trust provider integrations (AWS SSM, GCP IAP, Azure Bastion, etc.)
- Connection organization with groups and tags
- Import from Asbru-CM, Remmina, SSH config, Ansible inventory
- Export to Asbru-CM, Remmina, SSH config, Ansible inventory
- Native format import/export for backup and migration
- Secure credential storage via KeePassXC and libsecret
- Session logging with configurable formats
- Command snippets with variable substitution
- Cluster commands for multi-host execution
- Wake-on-LAN support
- Split terminal view
- System tray integration (optional)
- Performance optimizations:
  - Search result caching with configurable TTL
  - Lazy loading for connection groups
  - Virtual scrolling for large connection lists
  - String interning for memory optimization
  - Batch processing for import/export operations
- Embedded protocol clients (optional features):
  - VNC via vnc-rs
  - RDP via IronRDP
  - SPICE via spice-client

### Security
- All credentials wrapped in `SecretString`
- No plaintext password storage
- `unsafe_code = "forbid"` enforced

[Unreleased]: https://github.com/totoshko88/RustConn/compare/v0.4.2...HEAD
[0.4.2]: https://github.com/totoshko88/RustConn/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/totoshko88/RustConn/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/totoshko88/RustConn/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/totoshko88/RustConn/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/totoshko88/RustConn/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/totoshko88/RustConn/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/totoshko88/RustConn/releases/tag/v0.1.0
