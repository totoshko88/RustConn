## [0.4.1] - 2025-12-25

### Added
- IronRDP audio backend (RDPSND) with PCM format support (48kHz, 44.1kHz, 22.05kHz)
- Optional `rdp-audio` feature for audio playback via cpal (requires libasound2-dev)
- Bidirectional clipboard improvements for embedded RDP sessions

### Changed
- Updated MSRV to 1.86
- Updated dependencies: tempfile 3.24, criterion 0.8, cpal 0.17
