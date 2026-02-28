#
# spec file for package rustconn
#
# Copyright (c) 2025 Anton Isaiev
# SPDX-License-Identifier: GPL-3.0-or-later
#

Name:           rustconn
Version:        0.9.4
Release:        0
Summary:        Modern connection manager for Linux (SSH, RDP, VNC, SPICE, Telnet, Serial, Kubernetes, Zero Trust)
License:        GPL-3.0-or-later
URL:            https://github.com/totoshko88/RustConn
Source0:        %{name}-%{version}.tar.xz
Source1:        vendor.tar.zst

# Rust 1.88+ required (MSRV)
# openSUSE: use devel:languages:rust repo for Rust 1.88+
# Fedora 42+: system Rust 1.93 is sufficient
# Fedora <42/RHEL: use rustup fallback since system Rust < 1.88
%if 0%{?suse_version}
BuildRequires:  cargo >= 1.88
BuildRequires:  rust >= 1.88
BuildRequires:  cargo-packaging
BuildRequires:  alsa-devel
%endif

%if 0%{?fedora} >= 42
BuildRequires:  cargo >= 1.88
BuildRequires:  rust >= 1.88
BuildRequires:  alsa-lib-devel
%endif

%if 0%{?fedora} && 0%{?fedora} < 42
# Older Fedora: use rustup
BuildRequires:  curl
BuildRequires:  alsa-lib-devel
%endif

%if 0%{?rhel}
# RHEL: use rustup
BuildRequires:  curl
BuildRequires:  alsa-lib-devel
%endif

# Common build dependencies
BuildRequires:  pkgconfig(gtk4) >= 4.14
BuildRequires:  pkgconfig(vte-2.91-gtk4)
BuildRequires:  pkgconfig(libadwaita-1)
BuildRequires:  pkgconfig(dbus-1)
BuildRequires:  pkgconfig(openssl)
BuildRequires:  zstd
BuildRequires:  gcc
BuildRequires:  make
BuildRequires:  gettext-tools

# Runtime dependencies
%if 0%{?suse_version}
Requires:       gtk4 >= 4.14
Requires:       libadwaita
Requires:       vte >= 0.74
Requires:       openssh-clients
Requires:       sshpass
Requires:       libasound2
%endif

%if 0%{?fedora} || 0%{?rhel}
Requires:       gtk4 >= 4.14
Requires:       libadwaita
Requires:       vte291-gtk4
Requires:       openssh-clients
Requires:       sshpass
Requires:       alsa-lib
%endif

# Optional runtime dependencies
Recommends:     freerdp
Recommends:     tigervnc
Recommends:     virt-viewer
Recommends:     picocom
Recommends:     kubectl

%description
RustConn is a modern connection manager for Linux with a GTK4/Wayland-native
interface. Manage SSH, RDP, VNC, SPICE, Telnet, Serial, Kubernetes, and Zero Trust
connections from a single application. All core protocols use embedded Rust
implementations — no external dependencies required.

Protocols (embedded Rust implementations):
- SSH with embedded VTE terminal and split view
- RDP via IronRDP (embedded, with FreeRDP fallback)
- VNC via vnc-rs (embedded, with TigerVNC fallback)
- SPICE via remote-viewer (external)
- Telnet via external telnet client (port 23)
- Serial via picocom (RS-232/USB serial consoles)
- Kubernetes via kubectl exec (shell access to pods)
- Zero Trust: AWS SSM, GCP IAP, Azure Bastion, OCI Bastion,
  Cloudflare, Teleport, Tailscale, Boundary

File Transfer:
- SFTP file browser via system file manager (sftp:// URI, D-Bus portal)

Organization:
- Groups, tags, and templates
- Connection history and statistics
- Session logging

Import/Export:
- Asbru-CM, Remmina, SSH config, Ansible inventory
- Royal TS, MobaXterm, native format (.rcn)

Security:
- KeePassXC (KDBX files and proxy)
- libsecret (GNOME Keyring)
- Bitwarden CLI
- 1Password CLI
- Passbolt CLI

Productivity:
- Split terminals
- Command snippets
- Cluster commands
- Wake-on-LAN

%prep
%autosetup -a1 -n %{name}-%{version}

# Install rustup for older Fedora/RHEL (system Rust < 1.88)
%if 0%{?fedora} && 0%{?fedora} < 42
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.88.0 --profile minimal
export PATH="$HOME/.cargo/bin:$PATH"
%endif

%if 0%{?rhel}
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.88.0 --profile minimal
export PATH="$HOME/.cargo/bin:$PATH"
%endif

mkdir -p .cargo
cat > .cargo/config.toml <<EOF
[source.crates-io]
replace-with = "vendored-sources"

[source."git+https://github.com/Devolutions/IronRDP"]
git = "https://github.com/Devolutions/IronRDP"
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "vendor"
EOF

%build
# Ensure rustup path is available for older Fedora/RHEL
%if (0%{?fedora} && 0%{?fedora} < 42) || 0%{?rhel}
export PATH="$HOME/.cargo/bin:$PATH"
%endif

%if 0%{?suse_version}
%{cargo_build} -p rustconn -p rustconn-cli
%else
cargo build --release -p rustconn -p rustconn-cli
%endif

%install
install -Dm755 target/release/rustconn %{buildroot}%{_bindir}/rustconn
install -Dm755 target/release/rustconn-cli %{buildroot}%{_bindir}/rustconn-cli
install -Dm644 rustconn/assets/io.github.totoshko88.RustConn.desktop \
    %{buildroot}%{_datadir}/applications/io.github.totoshko88.RustConn.desktop
install -Dm644 rustconn/assets/io.github.totoshko88.RustConn.metainfo.xml \
    %{buildroot}%{_datadir}/metainfo/io.github.totoshko88.RustConn.metainfo.xml

# Install icons
for size in 128 256; do
    if [ -f "rustconn/assets/icons/hicolor/${size}x${size}/apps/io.github.totoshko88.RustConn.png" ]; then
        install -Dm644 "rustconn/assets/icons/hicolor/${size}x${size}/apps/io.github.totoshko88.RustConn.png" \
            "%{buildroot}%{_datadir}/icons/hicolor/${size}x${size}/apps/io.github.totoshko88.RustConn.png"
    fi
done

if [ -f "rustconn/assets/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg" ]; then
    install -Dm644 "rustconn/assets/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg" \
        "%{buildroot}%{_datadir}/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg"
fi

# Locale files (compile .po to .mo)
for po_file in po/*.po; do
    [ -f "$po_file" ] || continue
    lang=$(basename "$po_file" .po)
    mkdir -p "%{buildroot}%{_datadir}/locale/$lang/LC_MESSAGES"
    msgfmt -o "%{buildroot}%{_datadir}/locale/$lang/LC_MESSAGES/rustconn.mo" "$po_file"
done

%files
%license LICENSE
%doc README.md CHANGELOG.md docs/
%{_bindir}/rustconn
%{_bindir}/rustconn-cli
%{_datadir}/applications/io.github.totoshko88.RustConn.desktop
%{_datadir}/metainfo/io.github.totoshko88.RustConn.metainfo.xml
%{_datadir}/icons/hicolor/*/apps/io.github.totoshko88.RustConn.*
%{_datadir}/locale/*/LC_MESSAGES/rustconn.mo

%changelog
* Sun Mar 01 2026 Anton Isaiev <totoshko88@gmail.com> - 0.9.4-0
- Session Reconnect — disconnected VTE tabs show a Reconnect banner
- Recursive Group Delete — keep children, cascade, or cancel
- Cluster broadcast mode wired — keyboard input broadcasts to all terminals
- Libvirt / GNOME Boxes import — VNC, SPICE, RDP from domain XML (#38)
- TemplateManager — centralized template CRUD with search, import/export
- Snippet shell safety check before --execute
- Settings Backup/Restore as ZIP archive
- Automation templates — 5 built-in expect rule presets
- Fixed password inheritance for PasswordSource::Variable (#37)
- Fixed VTE spawn failure — banner + toast instead of silent empty terminal
- Fixed cluster session lifecycle and disconnect-all
- Automation engine: one-shot rules, template picker, pre/post-connect tasks
- User Guide major rewrite

* Fri Feb 27 2026 Anton Isaiev <totoshko88@gmail.com> - 0.9.3-0
- Added Waypipe Support — Wayland application forwarding for SSH (#36)
- Added IronRDP Clipboard Integration — Bidirectional clipboard sync
- Fixed missing icons on KDE and non-GNOME desktops (#35)
- Fixed Serial/Kubernetes connection creation validation
- Fixed Serial/Kubernetes missing client toast
- Fixed libsecret password storage panic on non-UUID keys (#34)
- Fixed libsecret password retrieval — is_available() always false
- Fixed VNC/RDP identical icons
- Fixed SFTP via mc opens root instead of home directory
- Fixed SSH agent not inherited by VTE terminals
- Dependencies: deflate64 0.1.10→0.1.11, zerocopy 0.8.39→0.8.40

* Thu Feb 26 2026 Anton Isaiev <totoshko88@gmail.com> - 0.9.2-0
- Custom Icons — set emoji/unicode or GTK icon names on connections and groups (#23)
- Remote Monitoring — monitoring bar below SSH/Telnet/K8s terminals (#26)
- Fixed new connections and groups appending to end of list
- Fixed IronRDP fallback to FreeRDP on protocol negotiation failure (#33)
- Fixed monitoring SSH password auth via sshpass
- Fixed monitoring error spam — collector stops after 3 consecutive failures
- Fixed Bitwarden CLI not found in Flatpak — dynamic bw path resolution (#28)
- CLI downloads: Teleport 18.7.0→18.7.1
- Dependencies: vnc-rs 0.5.2→0.5.3, rustls 0.23.36→0.23.37

* Mon Feb 24 2026 Anton Isaiev <totoshko88@gmail.com> - 0.9.1-0
- Version bump to 0.9.1

* Sat Feb 21 2026 Anton Isaiev <totoshko88@gmail.com> - 0.9.0-0
- Ukrainian translation reviewed by Mykola Zubkov — 674 translations
  revised for accuracy and modern Ukrainian orthography

* Fri Feb 20 2026 Anton Isaiev <totoshko88@gmail.com> - 0.8.9-0
- SSH port forwarding — Local (-L), remote (-R), and dynamic SOCKS (-D)
  port forwarding rules per connection (#22)
- Deferred secret backend initialization — async startup, eliminates
  1–3 second delay when secret backend is configured
- Security: input validation hardening across all protocols
- Security: SSH config export blocks dangerous directives
- Security: KeePassXC socket responses capped at 10 MB
- Security: VNC and RDP client passwords migrated to SecretString
- Security: FreeRDP external launcher uses /from-stdin
- Fixed Quick Connect RDP "Got empty identity" CredSSP error (#29)
- Fixed Bitwarden duplicate vault writes, false "unlocked" status,
  auto-unlock after restart, CLI v2026.1.0 compatibility (#28)
- Fixed RefCell borrow panic in EmbeddedRdpWidget, VNC polling mutex
  contention, RDP polling timer leak
- Fixed several unwrap() panics (VNC, TaskExecutor, tray, build.rs)
- ~40 eprintln! calls migrated to structured tracing
- Dependencies: serde_yaml replaced with serde_yaml_ng 0.9 (maintained fork)
- Dependencies: cpal 0.17.1→0.17.3, clap 4.5.59→4.5.60
- Internal: architecture audit completed (51 findings, 49 resolved)

* Wed Feb 18 2026 Anton Isaiev <totoshko88@gmail.com> - 0.8.8-0
- Version bump to 0.8.8
- Security: AES-256-GCM replaces XOR obfuscation for stored credentials
  (transparent migration from legacy format)
- Security: FreeRDP password passed via stdin instead of command line
- FreeRDP detection unified with Wayland-first priority
- RDP build_args() decoupled from hardcoded binary name
- ZeroTrust: provider-specific validation and CLI tool detection
- Native export/import now includes snippets (format v2)
- Removed dead code: Dashboard module, 5 unused GUI modules,
  tab_split_manager remnants
- Dependencies: native-tls 0.2.14→0.2.18, toml 0.8→1.0, zip 2.2→8.1
- Fixed RDP HiDPI scaling on 4K displays (desktop_scale_factor)
- Fixed RDP mouse coordinate mismatch on HiDPI displays

* Mon Feb 17 2026 Anton Isaiev <totoshko88@gmail.com> - 0.8.7-0
- Version bump to 0.8.7
- Internationalization (i18n) — 14 languages: uk, de, fr, es, it, pl, cs,
  sk, da, sv, nl, pt, be, kk; gettext support via gettext-rs (#17)
- SPICE proxy support for Proxmox VE tunnelled connections (#18)
- RDP HiDPI fix — IronRDP uses device-pixel resolution on HiDPI displays (#16)
- Security: variable injection prevention in command-building paths
- Security: ChecksumPolicy enum replaces placeholder SHA256 strings
- Security: sensitive CLI arguments masked in log output
- Security: configurable document encryption strength (Standard/High/Maximum)
- Security: SSH Agent passphrase handling via SSH_ASKPASS helper
- CLI overhaul: modularized into 18 handler modules with structured logging
- CLI: shell completions, man page, fuzzy suggestions, dry-run, pager, auto-JSON
- CLI: --config flag now threads through all ConfigManager call sites
- Czech translation improved by native speaker p-bo (PR #19)
- Remmina RDP import: gateway_server, gateway_username, domain fields (#20)
- Accessible labels added to 20+ icon-only buttons
- VTE updated to 0.83.90 in Flatpak manifests
- Flatpak components dialog hides unusable protocol clients in sandbox
- SPDX license corrected: GPL-3.0+ → GPL-3.0-or-later in metainfo.xml

* Mon Feb 16 2026 Anton Isaiev <totoshko88@gmail.com> - 0.8.6-0
- Version bump to 0.8.6
- Fixed Embedded RDP keyboard layout: incorrect key mapping for non-US
  keyboard layouts (e.g. German QWERTZ) in IronRDP embedded client (#15)

* Sun Feb 15 2026 Anton Isaiev <totoshko88@gmail.com> - 0.8.5-0
- Version bump to 0.8.5
- Added Kubernetes Protocol (#14): kubectl exec shell access to pods
  with exec and busybox modes, GUI Kubernetes tab, K8s sidebar filter,
  CLI kubernetes subcommand, Flatpak kubectl component
- Added Serial Console Protocol (#11): picocom-based serial console
  in GUI, CLI, Flatpak, and Snap with 13 property tests
- Added SFTP File Browser (#10): portal-aware file manager launch,
  Midnight Commander FISH VFS, standalone SFTP connection type,
  CLI sftp subcommand
- Added Responsive / Adaptive UI (#9): reduced dialog sizes,
  adw::Clamp on list dialogs, adw::Window for Dashboard/Sessions,
  600sp breakpoint for split view
- Added Terminal Rich Search (#7): regex, highlight all,
  case-sensitive toggles, Ctrl+Shift+F, session log timestamps
- Changed: Session Logging moved to Logging settings tab

* Sat Feb 14 2026 Anton Isaiev <totoshko88@gmail.com> - 0.8.4-0
- Version bump to 0.8.4
- Added FIDO2/SecurityKey SSH authentication with hardware key support
- Added CLI --auth-method flag for add/update commands, --key for SSH key path
- Fixed CLI version check timeout: 3s to 6s for Azure CLI compatibility
- Fixed WoL MAC Entry Disabled on Edit: removed per-widget sensitivity calls
- Refactored ConnectionManager: watch channels replace Arc<Mutex> debounce
- Refactored EmbeddingError, StateAccessError to thiserror derive
- Refactored FreeRDP mutex consolidation into single shared state struct
- Refactored Embedded RDP module directory (7 flat files into module)
- Refactored ConnectionDialog LoggingTab extraction (~310 lines removed)
- Refactored OverlaySplitView sidebar with F9 toggle and gestures
- Refactored responsive sidebar breakpoint (400sp for narrow windows)
- Refactored Window module directory (14 flat files into module)
- Removed ~80 redundant clippy suppression annotations
- Extended Protocol trait with capabilities() and build_command() methods
- Updated dependencies: resvg 0.46->0.47, tiny-skia 0.11->0.12

* Fri Feb 13 2026 Anton Isaiev <totoshko88@gmail.com> - 0.8.3-0
- Version bump to 0.8.3
- Added Wake On LAN from GUI (#8): context menu, auto-WoL, standalone dialog
- Fixed Flatpak libsecret build: disabled bash_completion (EROFS in sandbox)
- Fixed Flatpak libsecret 0.21.7 build: renamed gcrypt option to crypto
- Fixed Thread Safety: removed std::env::set_var from FreeRDP spawned thread
- Fixed Flatpak Machine Key: app-specific key in $XDG_DATA_HOME/rustconn/.machine-key
- Fixed Variables Dialog Panic: replaced expect() with if-let pattern
- Fixed Keyring secret-tool Check: store() validates secret-tool availability
- Fixed Flatpak CLI Paths: no hardcoded /snap/bin/ paths inside Flatpak
- Fixed Settings Dialog Performance: CLI detection moved to background threads
- Fixed Settings Clients Tab: 3s timeout, parallel detection (~15s to ~3s)
- Fixed Settings Dialog Instant Display: present() before load_settings()
- Fixed Settings Dialog Render Blocking: std::thread::spawn + mpsc + idle_add_local

* Wed Feb 11 2026 Anton Isaiev <totoshko88@gmail.com> - 0.8.2-0
- Version bump to 0.8.2
- Added Shared Keyring Module with generic store(), lookup(), clear(),
  and is_secret_tool_available() functions for all backends
- Added Keyring Support for All Secret Backends:
  * Bitwarden: refactored to use shared keyring module
  * 1Password: store/get/delete token in keyring
  * Passbolt: store/get/delete passphrase in keyring
  * KeePassXC: store/get/delete KDBX password in keyring
- Added Auto-Load Credentials from Keyring on settings load
- Added secret-tool availability check when toggling keyring option
- Added Passbolt Server URL Setting and UI in Secrets tab
- Added Unified Credential Save Options with mutual exclusion
- Fixed Secret Lookup Key Mismatch across all secret backends
- Fixed Passbolt Server Address Always None
- Fixed Passbolt "Open Password Vault" URL using configured server
- Fixed Variable Secrets Ignoring Preferred Backend
- Fixed Bitwarden Folder Parsing Crash on null folder IDs
- Fixed Bitwarden Vault Auto-Unlock for variable save/load
- Improved workspace dependency consistency (regex to workspace)
- Removed unused picky pin from rustconn-core
- Updated dependencies: clap, clap_builder, clap_lex, deranged

* Wed Feb 11 2026 Anton Isaiev <totoshko88@gmail.com> - 0.8.1-0
- Version bump to 0.8.1
- Added Passbolt secret backend via go-passbolt-cli (#6):
  * PassboltBackend implementing SecretBackend trait
  * Store, retrieve, and delete credentials as Passbolt resources
  * CLI detection and version display in Settings → Secrets
  * Server configuration status check
- Unified Secret Backends:
  * Replaced individual PasswordSource variants with single Vault variant
  * Connection dialog password source: Prompt, Vault, Variable, Inherit, None
  * Serde aliases preserve backward compatibility with existing configs
- Added Variable password source:
  * PasswordSource::Variable(String) reads credentials from named secret variable
  * Connection dialog shows variable dropdown when Variable is selected
- Variables Dialog improvements:
  * Show/Hide toggle for secret variable values
  * Load from Vault button for secret variables
  * Secret variable values auto-saved to vault on dialog save
- Fixed secret variables always using libsecret instead of configured backend
- Fixed Variable dropdown showing empty when editing connections
- Fixed Telnet backspace/delete: uses VTE native EraseBinding API (#5)
- Fixed split view left panel shrinking on nested splits

* Tue Feb 10 2026 Anton Isaiev <totoshko88@gmail.com> - 0.8.0-0
- Version bump to 0.8.0
- Added Telnet backspace/delete key configuration (#5):
  * TelnetBackspaceSends and TelnetDeleteSends enums (Automatic/Backspace/Delete)
  * Connection dialog Keyboard group with two dropdowns
  * stty erase shell wrapper in spawn_telnet() to apply key settings
  * Addresses common backspace/delete inversion issue
- Added Flatpak Telnet support:
  * GNU inetutils 2.7 built as Flatpak module
  * telnet binary available at /app/bin/ in Flatpak sandbox
  * Added to all three Flatpak manifests
- Fixed Flatpak AWS CLI: replaced awscliv2 Docker wrapper with official binary
- Fixed Flatpak Component Detection: SSM Plugin, Azure CLI, OCI CLI detection
- Fixed Flatpak Python Version: dynamic Python version in wrapper scripts
- Updated OBS _service revision from v0.5.3 to current version tag
- Updated dependencies: libc 0.2.180->0.2.181, tempfile 3.24.0->3.25.0,
  unicode-ident 1.0.22->1.0.23

* Mon Feb 09 2026 Anton Isaiev <totoshko88@gmail.com> - 0.7.9-0
- Version bump to 0.7.9
- Added Telnet protocol support (#5):
  * Full implementation across all three crates (core, CLI, GUI)
  * TelnetConfig model with host, port (default 23), extra arguments
  * Protocol trait implementation using external telnet client
  * Import/export support: Remmina, Asbru, MobaXterm, RDM
  * CLI: rustconn-cli telnet subcommand
  * GUI: connection dialog, template dialog, sidebar filter, quick connect
  * Terminal: spawn_telnet() for launching sessions
  * All property tests updated with Telnet coverage
- Fixed missing Telnet icon mapping in sidebar get_protocol_icon()
- Fixed Telnet icon: changed from network-wired-symbolic to call-start-symbolic
- Fixed ZeroTrust sidebar icon: unified to folder-remote-symbolic for all providers

* Sun Feb 08 2026 Anton Isaiev <totoshko88@gmail.com> - 0.7.8-0
- Version bump to 0.7.8
- Added Remmina password import to configured secret backend
- Fixed import error swallowing: replaced 14 unwrap_or_default() with proper error propagation
- Fixed MobaXterm import double allocation on UTF-8 conversion
- Added 50 MB file size limit in read_import_file() to prevent OOM
- Native export/import uses streaming I/O with BufWriter/BufReader
- Native import version pre-check before full deserialization
- Added centralized write_export_file() helper with BufWriter
- Consolidated export write boilerplate across all exporters
- Removed redundant TOCTOU path.exists() checks in importers
- Removed unused imports in Asbru and MobaXterm exporters
- Updated dependencies: memchr, ryu, zerocopy, zmij

* Fri Feb 07 2026 Anton Isaiev <totoshko88@gmail.com> - 0.7.7-0
- Version bump to 0.7.7
- Fixed keyboard shortcuts intercepting VTE terminal input:
  - Delete, Ctrl+E, Ctrl+D no longer fire when terminal has focus (#4)
  - Shortcuts now scoped to sidebar only
- Improved thread safety:
  - Audio mutex locks use graceful fallback instead of unwrap()
  - Search engine mutex locks use graceful recovery patterns
- Security: VNC client logs warning when connecting without password
- Refactored runtime consolidation:
  - Replaced 23 redundant tokio runtime calls with shared with_runtime()
- Collection optimization: snippet tags use flat_map and sort_unstable
- Dead code removal: removed deprecated credential methods and unused menu builder

* Fri Feb 06 2026 Anton Isaiev <totoshko88@gmail.com> - 0.7.6-0
- Version bump to 0.7.6
- Flatpak Components Manager - On-demand CLI download for Flatpak environment:
  - Menu → Flatpak Components... (visible only in Flatpak)
  - Download and install CLIs to ~/.var/app/io.github.totoshko88.RustConn/cli/
  - Supports: AWS CLI, AWS SSM Plugin, Google Cloud CLI, Azure CLI, OCI CLI,
    Teleport, Tailscale, Cloudflare Tunnel, Boundary, Bitwarden CLI, 1Password CLI, TigerVNC
  - SHA256 checksum verification, progress indicators, cancel support
- Snap Strict Confinement - Migrated from classic to strict confinement:
  - Snap-aware path resolution for data, config, and SSH directories
  - Uses embedded clients (IronRDP, vnc-rs, spice-gtk)
  - External CLIs accessed from host via system-files interface
- UI/UX Enhancements - GNOME HIG compliance improvements:
  - Accessible labels for status icons and protocol filter buttons
  - Sidebar minimum width increased to 200px
  - Connection dialog uses adaptive adw::ViewSwitcherTitle
  - Toast notifications with proper priority levels
- Settings → Clients - Improved client detection display:
  - All protocols show embedded client status with blue indicator
  - Fixed AWS SSM Plugin detection
- Dialog Widget Builders - Reusable UI components (CheckboxRowBuilder, EntryRowBuilder, etc.)
- Protocol Dialogs Refactoring - Applied widget builders to SSH, RDP, VNC, SPICE panels
- Legacy Code Cleanup - Removed unused TabDisplayMode, TabLabelWidgets types

* Thu Feb 06 2026 Anton Isaiev <totoshko88@gmail.com> - 0.7.5-0
- Version bump to 0.7.5
- Code Quality Audit - Comprehensive codebase analysis and cleanup
- Removed duplicate SSH/VNC/SPICE/ZeroTrust/RDP options code (~1850 lines)
- Extracted shared folders UI into reusable shared_folders.rs module
- Created protocol_layout.rs with ProtocolLayoutBuilder for consistent protocol UI
- Consolidated with_runtime() into async_utils.rs
- Changed FreeRDP launcher to Wayland-first (force_x11: false by default)
- Removed legacy no-op methods from terminal module
- Updated dependencies: proptest 1.9.0→1.10.0, time 0.3.46→0.3.47

* Thu Feb 05 2026 Anton Isaiev <totoshko88@gmail.com> - 0.7.4-0
- Version bump to 0.7.4
- Fixed Zero Trust Entry Field Alignment - converted all Zero Trust provider fields to adw::EntryRow
- Refactored Connection Dialog Modularization - split into dialog.rs, ssh.rs, rdp.rs, vnc.rs, spice.rs
- Refactored Import File I/O - extracted common file reading pattern into read_import_file() helper
- Refactored Protocol Client Errors - consolidated duplicate error types into unified EmbeddedClientError
- Refactored Config Atomic Writes - improved reliability with temp file + atomic rename pattern
- Added GTK Lifecycle Documentation - module-level docs explaining #[allow(dead_code)] pattern
- Code Quality - removed legacy types, standardized error patterns, reduced unnecessary clones

* Tue Feb 03 2026 Anton Isaiev <totoshko88@gmail.com> - 0.7.3-0
- Version bump to 0.7.3
- Fixed Azure CLI Version Parsing - version now correctly extracted from unique output format
- Fixed Flatpak XDG Config - removed unnecessary xdg-config/rustconn:create permission
- Fixed Teleport CLI Detection - changed binary from teleport to tsh
- Improved RDP Client Detection - FreeRDP 3.x with Wayland support (wlfreerdp3/xfreerdp3)
- Unified Client Install Hints - format: deb-package (rpm-package)
- Updated dependencies: bytes, flate2, regex

* Tue Feb 03 2026 Anton Isaiev <totoshko88@gmail.com> - 0.7.2-0
- Version bump to 0.7.2
- Flatpak Host Command Support - New flatpak module for running host commands
- Fixed Flatpak Config Access - connections and settings now persist correctly
- Fixed Split View Equal Proportions - panels now split 50/50 reliably

* Sun Feb 01 2026 Anton Isaiev <totoshko88@gmail.com> - 0.7.1-0
- Version bump to 0.7.1
- Refactored Sidebar - Split monolithic sidebar into modular components (TECH-03)
- Refactored Drag & Drop - Strongly typed DragPayload (TECH-04)
- Added Search Highlighting - Visual feedback for search matches (TECH-05)
- Code Quality - Async persistence fixes and cleanup

* Sun Feb 01 2026 Anton Isaiev <totoshko88@gmail.com> - 0.7.0-0
- Version bump to 0.7.0
- Fixed Asbru Import Nested Groups - two-pass algorithm preserves hierarchy
- Fixed Asbru Export Description Field - exports connection and group descriptions
- Added Group Description Field - New Group and Edit Group dialogs
- Added Asbru Global Variable Conversion - <GV:VAR> to ${VAR} syntax
- Added Variable Substitution at Connection Time
- Dialog Size Unification - Export 750×650, Import 750×800, New Group 450×550

* Sat Jan 31 2026 Anton Isaiev <totoshko88@gmail.com> - 0.6.9-0
- Version bump to 0.6.9
- Fixed Local Shell tabs not appearing in Split View "Select Tab" dialog

* Thu Jan 30 2026 Anton Isaiev <totoshko88@gmail.com> - 0.6.8-0
- Version bump to 0.6.8
- 1Password CLI Integration - New secret backend for 1Password password manager
- Bitwarden API Key Authentication - Support for automated workflows and 2FA
- Bitwarden Keyring Storage - Store master password in system keyring

* Thu Jan 29 2026 Anton Isaiev <totoshko88@gmail.com> - 0.6.7-0
- Version bump to 0.6.7

* Tue Jan 27 2026 Anton Isaiev <totoshko88@gmail.com> - 0.6.6-0
- Version bump to 0.6.6

* Sat Jan 17 2026 Anton Isaiev <totoshko88@gmail.com> - 0.6.5-0
- Version bump to 0.6.5

* Fri Jan 17 2026 Anton Isaiev <totoshko88@gmail.com> - 0.6.4-0
- Update to version 0.6.4
- Snap Package - New distribution format for easy installation via Snapcraft
- Classic confinement for full system access (SSH keys, network, etc.)
- Automatic updates via Snap Store
- GitHub Actions Snap Workflow - Automated builds and publishing
- RDP/VNC Performance Modes - Quality/Balanced/Speed presets for different networks
- Fixed RDP initial resolution matching actual widget size
- Fixed RDP dynamic resolution with debounced reconnect (500ms)
- Fixed sidebar fixed width (no longer resizes with window)
- Fixed RDP cursor colors (BGRA→ARGB conversion)
- Updated ironrdp 0.13 → 0.14, ironrdp-tokio 0.7 → 0.8

* Wed Jan 15 2026 Anton Isaiev <totoshko88@gmail.com> - 0.6.3-0
- Update to version 0.6.3
- Bitwarden CLI Integration - New secret backend for Bitwarden password manager
- Password Manager Detection - Automatic detection of installed managers
- Enhanced Secrets Settings UI - Improved backend selection with dynamic config
- Detects GNOME Secrets, KeePassXC, KeePass2, Bitwarden CLI, 1Password CLI

* Wed Jan 15 2026 Anton Isaiev <totoshko88@gmail.com> - 0.6.2-0
- Update to version 0.6.2
- MobaXterm Import/Export - Full support for .mxtsessions files
- Connection History Button - Quick access from sidebar toolbar
- Run Snippet from Context Menu - Right-click on connection → "Run Snippet..."
- Persistent Search History - Up to 20 recent searches saved across sessions
- Updated quick-xml 0.38 → 0.39, resvg 0.45 → 0.46

* Sat Jan 11 2026 Anton Isaiev <totoshko88@gmail.com> - 0.5.9-0
- Update to version 0.5.9
- Migrated Settings dialog from deprecated PreferencesWindow to PreferencesDialog
- Updated libadwaita feature from v1_4 to v1_5
- Migrated Template dialog to modern libadwaita patterns
- Fixed Zero Trust (AWS SSM) connection status icon showing as failed
- Fixed remote-viewer version parsing in Settings Clients tab
- Fixed SSH Agent key selection when connecting
- Improved agent key dropdown display in Connection Dialog

* Tue Jan 07 2026 Anton Isaiev <totoshko88@gmail.com> - 0.5.8-0
- Update to version 0.5.8
- Fixed SSH Agent "Add Key" button - now opens file chooser to select any SSH key file
- Fixed SSH Agent "+" buttons in Available Key Files list - now load keys with passphrase dialog
- Fixed SSH Agent "Remove Key" (trash) button - now actually removes keys from the agent
- Fixed SSH Agent Refresh button - updates both loaded keys and available keys lists

* Tue Jan 07 2026 Anton Isaiev <totoshko88@gmail.com> - 0.5.7-0
- Update to version 0.5.7
- Fixed Test button in New Connection dialog (async runtime issue with GTK)
- Updated dependencies: h2, proc-macro2, quote, rsa, rustls, serde_json, url, zerocopy
- Note: sspi and picky-krb kept at previous versions due to rand_core compatibility

* Sat Jan 03 2026 Anton Isaiev <totoshko88@gmail.com> - 0.5.5-0
- Update to version 0.5.5
- Added Kiro steering rules for development workflow
- Rename action in sidebar context menu for connections and groups
- Double-click on import source to start import
- Double-click on template to create connection from it
- Group dropdown in Connection dialog for selecting parent group
- Info tab for viewing connection details (replaces popover)
- Default alphabetical sorting with drag-drop reordering support
- Toast notification system for non-blocking user feedback
- User-friendly error display utilities
- GUI utility module with safe display access
- Form validation module with visual feedback
- Accessibility improvements on sidebar and terminal tabs
- Keyboard shortcuts help dialog (Ctrl+? or F1)
- Empty state widgets for no connections/search results/sessions
- Color scheme toggle in Settings dialog (System/Light/Dark)
- CSS animations for connection status
- Enhanced drag-drop visual feedback

* Thu Jan 02 2026 Anton Isaiev <totoshko88@gmail.com> - 0.5.3-0
- Update to version 0.5.3
- UI Unification: All dialogs now use consistent 750×500px dimensions
- Connection history recording for all protocols
- Protocol-specific tabs in Template Dialog
- Connection history and statistics dialogs
- Common embedded widget trait for RDP/VNC/SPICE
- Quick Connect supports RDP and VNC with templates
- Refactored terminal.rs into modular structure
- Updated gtk4 dependency to 0.10.2

* Sun Dec 29 2025 Anton Isaiev <totoshko88@gmail.com> - 0.5.2-0
- Update to version 0.5.2
- Refactored window.rs, embedded_rdp.rs, sidebar.rs, embedded_vnc.rs into modular structure
- Fixed tab icons, Snippet dialog Save button, Template dialog layout
- Added wayland-native feature flag with gdk4-wayland integration
- CI improvements: libadwaita-1-dev, property tests job, OBS changelog generation

* Sat Dec 28 2025 Anton Isaiev <totoshko88@gmail.com> - 0.5.1-0
- Update to version 0.5.1
- CLI: Wake-on-LAN, snippet, group management commands
- CLI: Connection list filters (--group, --tag)
- CLI: Native format (.rcn) support for import/export
- Search debouncing with visual spinner indicator
- Clipboard file transfer UI for embedded RDP sessions
- Dead code cleanup and documentation improvements

* Sat Dec 27 2025 Anton Isaiev <totoshko88@gmail.com> - 0.5.0-0
- Update to version 0.5.0
- RDP clipboard file transfer support (CF_HDROP format)
- RDPDR directory change notifications and file locking
- Native SPICE protocol embedding
- Performance optimizations (lock-free audio, optimized search)
- Fixed SSH Agent key discovery

