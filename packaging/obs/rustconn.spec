#
# spec file for package rustconn
#
# Copyright (c) 2026 Anton Isaiev
# SPDX-License-Identifier: GPL-3.0-or-later
#

Name:           rustconn
Version:        0.13.14
Release:        0
Summary:        Modern connection manager for Linux (SSH, RDP, VNC, SPICE, MOSH, Telnet, Serial, Kubernetes, Zero Trust)
License:        GPL-3.0-or-later
URL:            https://github.com/totoshko88/RustConn
Source0:        %{name}-%{version}.tar.xz
Source1:        vendor.tar.zst
Source2:        rust-toolchain.tar.zst

# Rust 1.95+ required (MSRV)
# openSUSE: use devel:languages:rust repo for Rust 1.95+
# Fedora 42+: system Rust 1.93 is sufficient
# Fedora <42/RHEL: use rustup fallback since system Rust < 1.95
%if 0%{?suse_version}
BuildRequires:  cargo >= 1.95
BuildRequires:  rust >= 1.95
BuildRequires:  cargo-packaging
BuildRequires:  alsa-devel
%endif

%if 0%{?fedora}
# Rust provided via bundled toolchain (rust-toolchain.tar.zst)
BuildRequires:  alsa-lib-devel
%endif

%if 0%{?rhel}
# Rust provided via bundled toolchain (rust-toolchain.tar.zst)
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
%if 0%{?suse_version}
BuildRequires:  gettext-tools
%endif
%if 0%{?fedora} || 0%{?rhel}
BuildRequires:  gettext-devel
%endif

# Runtime dependencies
%if 0%{?suse_version}
Requires:       gtk4 >= 4.14
Requires:       libadwaita
Requires:       vte >= 0.74
Requires:       openssh-clients
Requires:       libasound2
%endif

%if 0%{?fedora} || 0%{?rhel}
Requires:       gtk4 >= 4.14
Requires:       libadwaita
Requires:       vte291-gtk4
Requires:       openssh-clients
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
interface. Manage SSH, RDP, VNC, SPICE, MOSH, Telnet, Serial, Kubernetes, and
Zero Trust connections from a single application. Core protocols use embedded
Rust implementations — no external dependencies required.

Protocols (embedded Rust implementations):
- SSH with embedded VTE terminal, split view, and scrollbar
- RDP via IronRDP (embedded, with FreeRDP fallback)
- VNC via vnc-rs (embedded, with TigerVNC fallback)
- SPICE via spice-client (embedded, with remote-viewer fallback)
- MOSH with predict mode and UDP port range
- Telnet via external telnet client
- Serial via picocom (RS-232/USB serial consoles)
- Kubernetes via kubectl exec (shell access to pods)
- Zero Trust: AWS SSM, GCP IAP, Azure Bastion, OCI Bastion,
  Cloudflare, Teleport, Tailscale, Boundary, Hoop.dev

Cloud Sync:
- Synchronize connections via shared directories (Google Drive,
  Syncthing, Nextcloud, Dropbox, USB)
- Group Sync with Master/Import access model
- Simple Sync with UUID-based merge and tombstone deletion

File Transfer:
- SFTP file browser via Midnight Commander with split-panel navigation

Organization:
- Groups, tags, templates, and smart folders
- Connection history and statistics
- Session logging and recording
- Tab Overview, Tab Pinning, and Tab Switcher

Import/Export:
- Asbru-CM, Remmina, SSH config, Ansible inventory, CSV
- Royal TS, MobaXterm, RDP files, libvirt, native format (.rcn)

Security:
- KeePassXC (KDBX files and proxy)
- libsecret (GNOME Keyring)
- Bitwarden CLI
- 1Password CLI
- Passbolt CLI
- Pass (passwordstore.org)

Productivity:
- Split terminals with multi-panel layouts
- Command snippets and cluster broadcast
- Custom terminal themes and per-connection color overrides
- Remote host monitoring (CPU, memory, disk, network)
- Wake-on-LAN with auto-connect

%prep
%autosetup -a1 -n %{name}-%{version}

# Unpack standalone Rust toolchain for Fedora/RHEL (OBS has no internet for rustup)
%if 0%{?fedora} || 0%{?rhel}
tar --zstd -xf %{SOURCE2}
export PATH="$PWD/rust-toolchain/bin:$PATH"
rustc --version
cargo --version
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
# Use bundled Rust toolchain for Fedora/RHEL
%if 0%{?fedora} || 0%{?rhel}
export PATH="$PWD/rust-toolchain/bin:$PATH"
%endif

# Determine libadwaita feature flags based on distro version:
#   adw-1-8: libadwaita >= 1.8 (Tumbleweed/Slowroll, Fedora 43+)
#   adw-1-7: libadwaita >= 1.7 (Leap 16.0, Fedora 42)
#   (none):  libadwaita 1.5 baseline (older distros)
%if 0%{?suse_version} > 1600
# Tumbleweed / Slowroll — libadwaita 1.8+ (1.9 with GNOME 50)
%define adw_features --features adw-1-8
%else
%if 0%{?suse_version} == 1600
# Leap 16.0 — GNOME 48, libadwaita 1.7
%define adw_features --features adw-1-7
%endif
%endif

%if 0%{?fedora} >= 43
# Fedora 43+ — GNOME 49+, libadwaita 1.8+
%define adw_features --features adw-1-8
%else
%if 0%{?fedora} == 42
# Fedora 42 — GNOME 48, libadwaita 1.7
%define adw_features --features adw-1-7
%endif
%endif

%if 0%{?suse_version}
%{cargo_build} -p rustconn %{?adw_features} -p rustconn-cli
%else
cargo build --release -p rustconn %{?adw_features} -p rustconn-cli
%endif

%install
install -Dm755 target/release/rustconn %{buildroot}%{_bindir}/rustconn
install -Dm755 target/release/rustconn-cli %{buildroot}%{_bindir}/rustconn-cli
install -Dm644 rustconn/assets/io.github.totoshko88.RustConn.desktop \
    %{buildroot}%{_datadir}/applications/io.github.totoshko88.RustConn.desktop
install -Dm644 rustconn/assets/io.github.totoshko88.RustConn-rdp.xml \
    %{buildroot}%{_datadir}/mime/packages/io.github.totoshko88.RustConn-rdp.xml
install -Dm644 rustconn/assets/io.github.totoshko88.RustConn-vv.xml \
    %{buildroot}%{_datadir}/mime/packages/io.github.totoshko88.RustConn-vv.xml
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
%{_datadir}/mime/packages/io.github.totoshko88.RustConn-rdp.xml
%{_datadir}/mime/packages/io.github.totoshko88.RustConn-vv.xml
%{_datadir}/metainfo/io.github.totoshko88.RustConn.metainfo.xml
%{_datadir}/icons/hicolor/*/apps/io.github.totoshko88.RustConn.*
%dir %{_datadir}/locale/uz
%dir %{_datadir}/locale/uz/LC_MESSAGES
%dir %{_datadir}/locale/zh-cn
%dir %{_datadir}/locale/zh-cn/LC_MESSAGES
%{_datadir}/locale/*/LC_MESSAGES/rustconn.mo

%changelog
* Tue May 13 2026 Anton Isaiev <totoshko88@gmail.com> - 0.13.14-1
- [Added] Welcome page: Import button — added "Import" action button
  alongside "New Connection" and "Quick Connect" on the welcome page
- [Added] Template Manager: empty state — added adw::StatusPage placeholder
  with icon and description when no templates exist
- [Added] Reconnect banner: auto-reconnect indicator — disconnected session
  banner now shows "Auto-reconnecting…" status label when active
- [Fixed] Credential memory safety — intermediate password strings from
  expose_secret().to_string() now wrapped in zeroize::Zeroizing across
  VNC, RDP, and document password flows; zeroed on drop
- [Fixed] Potential panic in resize debounce — replaced unwrap() on
  Instant::checked_sub() with unwrap_or_else fallback in terminal resize handler
- [Fixed] CLI show command panic — replaced expect("json object") with
  proper let-else error propagation in rustconn-cli
- [Fixed] Port overflow in SecureCRT/libvirt importers — replaced truncating
  as u16 casts with u16::try_from().ok() fallback to default port
- [Fixed] Sync file path traversal — added validate_sync_filename() that
  rejects absolute paths, .. components, and directory separators

* Mon May 12 2026 Anton Isaiev <totoshko88@gmail.com> - 0.13.13-0
- [Added] SSH ProxyCommand support — custom proxy for .onion hosts and SOCKS proxies
- [Fixed] SSH Startup Command not executing in GUI terminal
- [Fixed] SSH ProxyCommand port format — jump hosts with non-standard ports
- [Fixed] RDP/VNC/SPICE tunnel through nested jump hosts
- [Improved] StringInterner: HashSet instead of HashMap — 50% less memory overhead per entry
- [Improved] ConfigManager: ensure_config_dir() caching — skip filesystem check after first success
- [Improved] ConnectionManager: collect_descendant_groups() O(n) instead of O(n²)
- [Improved] ConnectionManager: sort_all() refactoring — extracted sort_ids_by_name() helper
- [Improved] WolDialog: migrated to adw::Dialog — better focus management, auto Escape

* Mon May 11 2026 Anton Isaiev <totoshko88@gmail.com> - 0.13.12-0
- Auto-reconnect: per-connection RetryConfig with exponential backoff
- Import: multi-file batch import
- SSH: fix identity key -i duplicated in command
- Terminal: fix per-connection white color displayed as grey

* Sat May 10 2026 Anton Isaiev <totoshko88@gmail.com> - 0.13.11-1
- [Improved] RDP: better diagnostics for IronRDP fallback to FreeRDP —
  error detection now includes detailed comments explaining the upstream
  limitation (IronRDP connector 0.8.0 does not handle ServerDeactivateAll
  during CapabilitiesExchange); submitted fix upstream (IronRDP#1253)

* Sat May 09 2026 Anton Isaiev <totoshko88@gmail.com> - 0.13.10-1
- [Added] Import/Export: SecureCRT session support — import connections
  from SecureCRT Config/Sessions/ directory; export connections back to
  SecureCRT INI format as a directory tree; supports SSH2, Telnet, RDP,
  VNC protocols with hostname, port, username, SSH key path, X11/agent
  forwarding, compression; folder hierarchy preserved (#140)
- [Fixed] Backup/Restore: global variables lost after restore — restoring
  settings from ZIP overwrote restored config.toml with stale in-memory
  state; dialog now reloads AppSettings from disk after restore (#142)
- [Fixed] SSH: ControlMaster sockets now actually closed on application
  exit — shutdown handler scanned active_sessions() but all sessions were
  already Terminated by the time GTK fires connect_shutdown; replaced with
  filesystem scan of runtime directory for rc-* socket files; stale sockets
  that don't respond to ssh -O exit are force-removed (#125)
- [Fixed] KeePass: custom entry path for variables ignored RustConn/
  prefix — added get_password_from_kdbx_exact() that queries the entry at
  the exact user-specified path without any prefix or fallback logic (#143)
- [Dependencies] hashbrown 0.17.0 → 0.17.1

* Sat May 09 2026 Anton Isaiev <totoshko88@gmail.com> - 0.13.9-1
- [Fixed] Flatpak: Zero Trust Generic commands now execute on host —
  custom commands were failing with "No command specified" due to double
  sh -c wrapping; now automatically wrapped with flatpak-spawn --host
  with PTY allocation; single quotes in templates properly escaped (#132)
- [Fixed] Split View: focus border not updating on click — clicking a
  terminal pane sent input correctly but the focus border remained on the
  previous pane; fixed by removing duplicate gesture controllers
- [Improved] RDP: real disk space reported to Windows via shared folders —
  RDPDR backend now queries actual filesystem statistics using
  nix::sys::statvfs instead of returning hardcoded values; Windows Explorer
  and applications see correct total/available disk space; values normalized
  to 4096-byte allocation units; graceful fallback if statvfs call fails
- [Dependencies] cc 1.2.61 → 1.2.62, filetime 0.2.27 → 0.2.28,
  quick-xml 0.39.3 → 0.39.4, tokio 1.52.2 → 1.52.3
- [Dependencies] Added nix 0.31.2 (feature: fs) to rustconn-core for safe
  statvfs access

* Thu May 08 2026 Anton Isaiev <totoshko88@gmail.com> - 0.13.8-1
- [Fixed] Per-connection monitoring toggle not saving state — the
  "Enable Monitoring" switch always appeared enabled and did not persist
  the user's choice; now both ON and OFF states are stored as explicit
  overrides (enabled: Some(true) / Some(false)) (#125)
- [Fixed] Flatpak: CLI tools not found by protocol detection —
  which_binary() now searches all get_cli_path_dirs() directories and
  passes the full resolved path to version detection
- [Fixed] Hoop.dev CLI version not displayed — added parse_json_version()
  to extract the version field from JSON output; fixed get_version() to
  set extended PATH in Flatpak

* Thu May 08 2026 Anton Isaiev <totoshko88@gmail.com> - 0.13.7-1
- [Fixed] SSH: monitoring no longer triggers a second agent confirmation —
  monitoring waits up to 5s for ControlMaster socket, connects as slave
  only; falls back to own master if socket never appears (#125)
- [Fixed] SSH: ControlMaster sockets cleaned up on application exit —
  all sockets gracefully closed via ssh -O exit on shutdown (#125)
- [Fixed] SSH: control socket path shortened for macOS compatibility —
  ControlPath changed to rc-{host}-{port}-%r; uses /tmp on macOS to
  stay within 104-byte Unix socket path limit
- [Fixed] Auto-reconnect: no longer loops infinitely on rapid crashes —
  if session crashes within 5s of starting, auto-reconnect is skipped
- [Fixed] Flatpak: local shell PTY resize now propagates to host —
  VTE resize forwarded via flatpak-spawn stty to host-side PTY (#122)

* Wed May 07 2026 Anton Isaiev <totoshko88@gmail.com> - 0.13.6-1
- [Improved] Preferences: Monitoring moved to its own page with dedicated
  tab and utilities-system-monitor-symbolic icon (#125)
- [Improved] CLI: machine-readable output (--format json|csv|table) for
  show, test, stats, group show commands; defaults to JSON when stdout is
  piped (non-TTY) (#132)
- [Improved] i18n: MonitorMode and ExportFormat display_name() values now
  translatable via i18n() at call sites
- [Improved] Security: intermediate password strings (RDP/VNC) now wrapped
  in zeroize::Zeroizing<String> so plaintext is overwritten on drop
- [Improved] Reliability: host check no longer panics on runtime creation
  failure — proper Result propagation via HostCheckError::Io variant
- [Fixed] SSH: single authentication prompt for connection + monitoring —
  VTE terminal uses ControlMaster=auto with shared ControlPath; monitoring
  reuses the same socket instead of opening a separate session (#125)
- [Fixed] Repository hygiene: removed committed vim swap file; added *.swp,
  *.swo, *.po~ patterns to .gitignore

* Tue May 06 2026 Anton Isaiev <totoshko88@gmail.com> - 0.13.5-0
- Version bump to 0.13.5
- Added Drag & Drop file paths into VTE terminals (#74)
- Added Drag & Drop files to RDP clipboard via CLIPRDR (#74)
- Added RDP "Reconnect on Resize" option for legacy servers
- Fixed RDP dynamic resize without reconnect via Display Control Channel (#131)
- Fixed SSH agent key selection not remembered in connection dialog (#125)

* Mon May 05 2026 Anton Isaiev <totoshko88@gmail.com> - 0.13.4-1
- [Added] RDP Autotype: send text as keystrokes bypassing clipboard
  restrictions — Type Clipboard and Type Text toolbar buttons in embedded
  RDP sessions; TS_UNICODE_KEYBOARD_EVENT PDU, keyboard-layout independent;
  configurable timing (inter-character delay 5–200ms, initial delay
  0–5000ms); iterates by Unicode grapheme clusters (#127)
- [Added] KeePass custom entry path for secret variables — reference
  existing KeePass entries instead of default path (#114)
- [Fixed] RDP toolbar Copy/Paste buttons do nothing on Wayland (COSMIC,
  GNOME) — replaced drawing_area.display().clipboard() with
  root().native().display().clipboard() which uses the top-level window
  surface; Paste button now shows status feedback instead of silently
  swallowing errors; CLIPRDR client_capabilities now advertises
  USE_LONG_FORMAT_NAMES flag required by Windows Server 2016+; added
  tracing for all clipboard button operations (#126)
- [Dependencies] unicode-segmentation 1.13.2 (new),
  tower-http 0.6.8 → 0.6.9

* Mon May 04 2026 Anton Isaiev <totoshko88@gmail.com> - 0.13.3-1
- Version bump to 0.13.3
- Added RDP Security Layer / TLS Compatibility options (#124)
- Improved GNOME HIG: application menu restructured, manager dialogs unified
- Improved Tray menu i18n
- Fixed SSH agent multiple authentication prompts for saved connections (#125)
- Fixed false KeePassXc backend unavailable toast (#123)
- Fixed Flatpak Local Shell no job control warnings (#122)

* Sun May 04 2026 Anton Isaiev <totoshko88@gmail.com> - 0.13.3-0
- [Fixed] False "KeePassXc backend unavailable" toast when KeePassXc is
  running — availability now checked via kdbx_enabled && kdbx_path.exists()
  instead of probing the unrelated LibSecretBackend (#123)

* Sun May 04 2026 Anton Isaiev <totoshko88@gmail.com> - 0.13.2-0
- [Fixed] Mouse scroll not working in terminal sessions (#121)
- [Fixed] Flatpak local shell sandboxed shell (#122)
- [Removed] Mouse passthrough setting
- [Added] Per-connection monitoring toggle (#106)

* Sat May 03 2026 Anton Isaiev <totoshko88@gmail.com> - 0.13.1-1
- [Fixed] Crash when typing in sidebar search field —
  SearchEngine byte-position iteration panics on multi-byte UTF-8
  characters; fixed by iterating over valid char_indices() boundaries
  and using str::get() for safe slicing (#116)
- [Fixed] Export/re-import loses folder hierarchy, icons, SSH settings,
  and smart folders — topological sort for groups, copy all fields via
  update_group, smart folders in NativeExport format v3, group ID
  remapping during import; CLI updated (#118)
- [Added] Automation section in group edit dialog — Expect Rules and
  Post-login Scripts in Edit Group dialog; CLI support (#117)
- [Improved] Connection dialog Automation tab unified with group dialog (#117)
- [Documentation] Remmina Flatpak import troubleshooting (#120)

* Sat May 02 2026 Anton Isaiev <totoshko88@gmail.com> - 0.13.0-1
- Update to version 0.13.0
- [Fixed] External RDP tab shows only toolbar, content area empty
- [Fixed] Group edit: SSH settings toggle reopens confirmation dialog in a loop
- [Fixed] Incomplete translations for Dynamic Folder strings
- [Fixed] Bulk actions: Move to Group icon missing
- [Fixed] External RDP (FreeRDP) fails on changed server certificate (#112)
- [Added] Smart Folders in sidebar (#111)
- [Added] Group-level Expect rules and post-login scripts inheritance (#110)
- [Improved] Vault entry missing toast notification (#114)
- [Improved] Test credential resolution button in connection dialog (#114)
- [Improved] Multi-language SSH password prompt detection (#114)
- [Improved] GNOME HIG: sidebar toolbar decluttered

* Thu May 01 2026 Anton Isaiev <totoshko88@gmail.com> - 0.12.9-1
- [Fixed] Export group exports entire tree instead of selected subtree —
  when exporting a specific group via the Export dialog's group filter,
  all groups were included in the output file even though connections
  were correctly filtered; now both connections and groups are filtered
  to the selected group and its descendants
- [Added] Snippet variable substitution from Global Variables — snippets
  containing ${VARIABLE} placeholders now automatically resolve values
  from Global Variables before execution; if all variables are resolved,
  the snippet executes immediately without showing the input dialog
- [Added] Dynamic Folders — new DynamicFolderConfig on ConnectionGroup
  allows generating connections from an external script; supports SSH,
  RDP, VNC, SPICE, Telnet, and MOSH protocols; connections are read-only
  with stable deterministic UUIDs across refreshes

* Thu May 01 2026 Anton Isaiev <totoshko88@gmail.com> - 0.12.8-1
- [Added] Generic async cache Cached<T> — new rustconn-core/src/cache.rs
  module providing a thread-safe, TTL-based cache with automatic refresh
  via the LoadCacheObject trait; uses double-checked locking with
  tokio::sync::RwLock for concurrent read access; supports incremental
  updates, explicit invalidation, and configurable TTL (default 60s)
- [Added] Busy-state indicator BusyStack — new rustconn-core/src/busy.rs
  module providing a thread-safe RAII counter for tracking in-flight
  operations; callback fires on 0→1 (busy) and 1→0 (idle) transitions;
  nested operations handled correctly without extra callbacks;
  integrated into GUI — header bar spinner appears during connections
- [Added] Extended ProtocolCapabilities — added 9 new capability flags:
  multi_monitor, usb_redirection, port_forwarding, wayland_forwarding,
  x11_forwarding, session_recording, remote_monitoring, command_snippets,
  wake_on_lan; enables UI to adapt controls per-protocol
- [Added] Connection fallback chain ConnectionFallback<T> — generic
  mechanism for trying multiple connection strategies in priority order
- [Added] Virt-viewer .vv file open support — open SPICE/VNC connections
  from libvirt, Proxmox VE, oVirt directly from the file manager
- [Added] Connection failure toast with connection name
- [Dependencies] Teleport CLI 18.7.5 → 18.7.6 (security patch)

* Thu Apr 30 2026 Anton Isaiev <totoshko88@gmail.com> - 0.12.7-1
- [Fixed] Group credentials: Variable source shows password field
  instead of variable selector — now shows dropdown populated with
  secret global variables (#109)
- [Fixed] Group credentials: saving Variable source stored empty
  string instead of actual variable name
- [Improved] GNOME HIG: accessible labels, menu reorganization,
  Keyboard Shortcuts entry added to app menu

* Wed Apr 29 2026 Anton Isaiev <totoshko88@gmail.com> - 0.12.6-1
- [Fixed] Expect script variables not substituted — ${VAR} references
  in Expect rule responses were sent as literal text instead of being
  resolved; now uses VariableManager to substitute global variables
  before creating automation triggers (#105)
- [Added] SSH verbose mode for connection debugging — new Verbose
  toggle in SSH connection settings adds -v flag to the SSH command,
  showing detailed debug output to help diagnose connection issues (#106)

* Tue Apr 29 2026 Anton Isaiev <totoshko88@gmail.com> - 0.12.5-1
- [Fixed] Settings dialog overflows after Hoop.dev install — hoop
  version outputs JSON; added dedicated parser that extracts only
  the version field
- [Fixed] kubectl version not shown in settings — kubectl version
  --client --short fails on kubectl >= 1.28; switched to kubectl
  version --client and parse Client Version: vX.Y.Z
- [Fixed] Tray icon SIGSEGV on restart — connect_shutdown did not
  drop the TrayManager; now explicitly drops tray manager in shutdown
  handler before flushing persistence
- [Fixed] Teleport CLI download URL 404 — pinned version 18.7.6 did
  not exist on the CDN; corrected to 18.7.5
- [Dependencies] Hoop.dev CLI 1.56.1 → 1.59.3
- [Dependencies] Teleport CLI 18.7.6 → 18.7.5 (URL fix)

* Tue Apr 28 2026 Anton Isaiev <totoshko88@gmail.com> - 0.12.4-0
- [Cleaned] Removed dead mosh.rs dialog module — MOSH settings already
  integrated into SSH tab via ssh::create_ssh_options()
- [Cleaned] Removed legacy connect_password_load_button wrapper — all
  callers use connect_password_load_button_with_groups directly
- [Cleaned] Removed unused add_available_file_row from Cloud Sync settings
- [Dependencies] rpassword 7.4.0 → 7.5.0, rustls 0.23.39 → 0.23.40

* Tue Apr 28 2026 Anton Isaiev <totoshko88@gmail.com> - 0.12.3-0
- [Fixed] Sync toast shows raw placeholders instead of values —
  i18n_f() only supports {} placeholders; changed both sync message
  strings and all 16 translations to use {} format
- [Accessibility] Added accessible labels to 24 icon-only buttons
  across 15 files for screen reader support
- [Dependencies] Teleport CLI 18.7.4 → 18.7.6
- [Dependencies] clap_complete 4.6.2 → 4.6.3, gio 0.22.5 → 0.22.6,
  glib 0.22.5 → 0.22.6, gtk4 0.11.2 → 0.11.3, pango 0.22.4 → 0.22.6,
  zbus 5.14.0 → 5.15.0, zvariant 5.10.0 → 5.10.1

* Sun Apr 26 2026 Anton Isaiev <totoshko88@gmail.com> - 0.12.2-0
- [Fixed] Flatpak SFTP ssh-add fails with missing askpass — strips
  SSH_ASKPASS from the environment for bare ssh-add calls (#102)
- [Fixed] Blocking operations on GTK main thread — added 5s timeouts
  to has_secret_backend() and refresh_secret_backend_cache()
- [Fixed] Missing timeouts on blocking async operations — added
  timeouts to flush_persistence (5s), resolve_with_hierarchy (30s),
  auto_unlock (30s), and vault store/retrieve/delete (10s)
- [Translations] All 16 languages aligned to 1697 strings; fixed
  Italian PO syntax error

* Sat Apr 25 2026 Anton Isaiev <totoshko88@gmail.com> - 0.12.1-1
- [Fixed] Split view content disappearing on panel focus switch —
  removed switch_to_tab() call; focus handled via set_focused_pane()
  and grab_focus() (#101)
- [Fixed] cargo-deny CI failure — removed deprecated unlicensed and
  copyleft keys from deny.toml [licenses] section
- [Fixed] cargo-audit CI failure — added RUSTSEC-2023-0071 (rsa Marvin
  Attack) to [advisories].ignore in deny.toml; transitive dependency
  via ironrdp/sspi and spice-client with no upstream fix available
- [Dependencies] Bitwarden CLI 2026.3.0 → 2026.4.1 (fixes supply chain
  attack in 2026.4.0), kubectl 1.35.4 → 1.36.0

* Thu Apr 24 2026 Anton Isaiev <totoshko88@gmail.com> - 0.12.0-0
- [Added] Cloud Sync — synchronize connections via shared cloud directory
- [Added] Group Sync — per-group .rcn files with Master/Import access model
- [Added] Simple Sync — single-file bidirectional sync with UUID-based merge
- [Added] SSH Key Inheritance — group-level SSH settings inherited by children
- [Added] Credential Resolution UX — interactive dialogs for missing variables
- [Added] CLI sync commands: sync status, list, export, import, now
- [Added] Tab Overview — grid view of all open tabs via Ctrl+Shift+O
- [Added] Tab Switcher in Command Palette — % prefix for fuzzy tab search
- [Added] Tab Pinning — right-click tab, Pin Tab
- [Added] Custom terminal themes — create/edit/delete color themes
- [Added] Group Jump Host dropdown with SSH connection selection
- [Added] Accessible labels for icon-only buttons
- [Added] cargo-deny + cargo-audit in CI
- [Added] Document dirty badge — CSS dot indicator
- [Fixed] System tray SIGSEGV and empty menu — moved D-Bus updates to
  dedicated background thread, deferred TrayManager creation off GTK
  main thread, added Flatpak disable_dbus_name workaround
- [Fixed] Tab Overview SIGSEGV with split-view tabs
- [Fixed] Terminal theme reset when Settings dialog is closed
- [Fixed] Pango assertion failure on zero font size
- [Fixed] Highlight rules show color instead of hover-only underline

* Thu Apr 23 2026 Anton Isaiev <totoshko88@gmail.com> - 0.11.7-1
- [Fixed] Monitoring bar broken after scrollbar addition — wrapped the
  horizontal terminal+scrollbar row in a vertical outer container so the
  monitoring bar is correctly appended underneath

* Thu Apr 23 2026 Anton Isaiev <totoshko88@gmail.com> - 0.11.6-0
- [Added] Terminal scrollbar (#95)
- [Added] "Execute Snippet…" in terminal context menu (#95)
- [Fixed] Sidebar status stays gray after reconnect (#96)
- [Fixed] Context menu intermittently fails to open on right-click (#87)

* Wed Apr 22 2026 Anton Isaiev <totoshko88@gmail.com> - 0.11.5-0
- [Added] Simplified Chinese (zh-cn) translation (PR #94)
- [Added] User Guide: libvirt NSS hostname resolution (#91)
- [Dependencies] picky-asn1-der 0.5.6, rustls-webpki 0.103.13,
  winnow 1.0.2, kubectl 1.35.4

* Tue Apr 21 2026 Anton Isaiev <totoshko88@gmail.com> - 0.11.4-1
- [Fixed] Sidebar flashes red during SSH connection — introduced
  ConnectionStartResult enum to distinguish async port check in
  progress (Pending) from real failures (Failed); sidebar stays
  yellow ("connecting") until the port check completes
- [Fixed] Context menu stays open when dialog opens — switched
  popover to autohide so GTK4 dismisses it on focus change (#93)
- [Fixed] Sidebar stays "connecting" after cancelling password
  dialog — VNC/RDP password prompt cancel now clears status
- [Fixed] VNC/RDP with "None" password source prompts immediately —
  first attempt uses empty password; dialog shown on retry only
- [Updated] Teleport CLI 18.7.3 → 18.7.4
- [Updated] 1Password CLI 2.33.1 → 2.34.0

* Mon Apr 20 2026 Anton Isaiev <totoshko88@gmail.com> - 0.11.3-1
- [Added] CLI: --jump-host flag for add and update — set a jump host
  (SSH bastion) for SSH, SFTP, and RDP connections via CLI; validates
  existence and prevents self-referencing
- [Fixed] Flatpak: kubectl and Hoop.dev missing from Settings Clients
  tab and PATH — added Container Orchestration section, Hoop.dev to
  Zero Trust Clients, registered both in get_cli_path_dirs()

* Mon Apr 20 2026 Anton Isaiev <totoshko88@gmail.com> - 0.11.2-0
- [Fixed] Reconnect reuses existing tab for all VTE protocols (#89)
- [Fixed] RDP port check skipped with jump host
- [Fixed] Hoop.dev CLI download — versioned URL (HTTP 403)
- [Fixed] Azure/gcloud/OCI CLI wrapper test in Flatpak
- [Fixed] Flatpak SFTP always uses mc
- [Improved] Reconnect banner consistent across all protocols
- [Improved] Sidebar width tuned for HiDPI — 360px→320px
- [Added] SSH Jump Host for RDP via ssh -L tunnel (#90)
- [Added] Tab context menu: Close Others/Left/Right/All/Ungrouped
- [Added] CLI: all 10 protocols and 11 Zero Trust providers
- [Documentation] Complete CLI reference in User Guide
- [Dependencies] open 5.3.4, openssl 0.10.78, typenum 1.20.0

* Sat Apr 19 2026 Anton Isaiev <totoshko88@gmail.com> - 0.11.1-0
- [Fixed] Reconnect preserves tab position (#89)
- [Fixed] Context menu handoff between items (#87)
- [Fixed] Stale highlight on right-click — residual highlights removed
- [Fixed] Context menu requires single right-click instead of two
- [Improved] Context menu layout follows GNOME HIG
- [Added] SSH Keep-Alive settings (#88)

* Sat Apr 18 2026 Anton Isaiev <totoshko88@gmail.com> - 0.11.0-0
- [Added] General tab migrated to adw:: widgets (TASK-004)
- [Added] Legacy XOR encryption migration warning (TASK-006)
- [Added] State access helpers — with_state/try_with_state (TASK-008)
- [Improved] RDP connection state structured — RdpConnectionContext (TASK-007)
- [Security] Automation task validation hardened (TASK-005)
- [Fixed] Split view tab colors preserved across Settings (TASK-022)
- [Fixed] Group Operations mode — compact pill icon buttons with Revealer
- [Fixed] Split view context menu Copy/Paste/Select All now works
- [Fixed] Eliminated gdk_clipboard_write_async assertion
- [Security] Lazy Bitwarden credential decryption
- [Dependencies] libbz2-rs-sys 0.2.3, rand 0.8.6, rtoolbox 0.0.5

* Thu Apr 17 2026 Anton Isaiev <totoshko88@gmail.com> - 0.10.22-0
- [Fixed] Terminal context menu Copy/Paste now works (#84)
- [Fixed] No more gdk_clipboard_write_async assertion on Copy
- [Fixed] Blank menus on X11 — Cairo renderer fallback (#85)
- [Improved] Context menu labels localized
- [Dependencies] pxfm 0.1.29, tokio 1.52.1, uuid 1.23.1

* Tue Apr 15 2026 Anton Isaiev <totoshko88@gmail.com> - 0.10.21-0
- [Security] Machine key encryption hardened — HKDF-SHA256 for machine-id
- [Fixed] Groups expand/collapse on double-click entire row (#83)
- [Fixed] Ctrl+K no longer hijacks terminal (#83)
- [Fixed] Right-click context menu on all SSH profiles (#83)
- [Fixed] Filter bar opens below search box (#83)
- [Improved] Sidebar accessible labels localized

* Tue Apr 14 2026 Anton Isaiev <totoshko88@gmail.com> - 0.10.20-0
- Tab group chooser dialog with existing group selection
- Close All in Group context menu action
- Group name as tab title prefix
- Updated FreeRDP 3.24.2, VTE 0.80.3

* Sun Apr 13 2026 Anton Isaiev <totoshko88@gmail.com> - 0.10.18-1
- [Added] Terminal font zoom — Ctrl+Scroll wheel, Ctrl+Plus/Minus, and
  Ctrl+0 to reset; uses VTE native font_scale for per-session zoom (#77)
- [Added] Copy on select — optional X11-style auto-copy; selected text is
  automatically copied to clipboard; enable in Settings → Terminal (#78)

* Sun Apr 12 2026 Anton Isaiev <totoshko88@gmail.com> - 0.10.17-1
- [Fixed] clear command not working in Flatpak SSH sessions — forces
  TERM=xterm-256color for all remote commands in Flatpak (#25)
- [Fixed] Sidebar scroll position lost after editing/moving connections —
  restore_state() idle callbacks raced; chained callback fix
- [Fixed] Sorting collapsed all expanded groups — preserves expanded state

* Fri Apr 10 2026 Anton Isaiev <totoshko88@gmail.com> - 0.10.16-1
- [Fixed] Sidebar context menu actions still not working — replaced
  PopoverMenu with plain Popover + Button widgets (#75)

* Fri Apr 10 2026 Anton Isaiev <totoshko88@gmail.com> - 0.10.15-1
- [Fixed] clear command not working in Flatpak — added ANSI escape
  sequence wrapper to all three Flatpak manifests (#25)
- [Fixed] Keyboard shortcuts dialog showed wrong bindings — corrected
  19 discrepancies between shortcuts help dialog and actual GTK accelerators
- [Fixed] Shortcuts dialog missing entries — added 13 missing shortcuts
- [Improved] FreeRDP updated to 3.24.2 (CVE fixes for client-side
  vulnerabilities)
- [Documentation] Keyboard shortcuts fully synchronized in User Guide
- [Documentation] Terminal clear troubleshooting section added to User Guide

* Tue Apr 08 2026 Anton Isaiev <totoshko88@gmail.com> - 0.10.13-0
- Fixed SSH auto-reconnect infinite loop on authentication failures
- Fixed duplicate child-exited signal handlers for SSH and Telnet sessions
- FreeRDP 3.24.0 → 3.24.1 (security fix), Boundary CLI 0.21.1 → 0.21.2

* Mon Apr 07 2026 Anton Isaiev <totoshko88@gmail.com> - 0.10.12-0
- Technical audit: VNC SecretString, pixel buffer guard, RDP zero-copy,
  sidebar regex cache, sanitization optimization, dead code cleanup
- Dependencies: gtk4 0.11.2, glib 0.22.4, pango 0.22.4, zip 8.5.1
- CLI downloads: TigerVNC 1.16.2, Teleport 18.7.3, Bitwarden CLI 2026.3.0

* Sat Apr 04 2026 Anton Isaiev <totoshko88@gmail.com> - 0.10.11-0
- RDP Mouse Jiggler — prevents idle disconnect with periodic mouse movements
- Connect All in Folder — open all connections in a group at once
- Copy Username / Copy Password from sidebar context menu
- Host Online Check — TCP probe with auto-connect when host comes online
- WoL + Auto-Connect — WoL now polls and auto-connects when host is ready

* Sat Apr 04 2026 Anton Isaiev <totoshko88@gmail.com> - 0.10.10-0
- Flatpak: removed extra sandbox permissions rejected by Flathub lint
  (home/.hoop, xdg-run/gnupg, bitwarden data, xdg-run/ssh-agent);
  users can add them manually via flatpak override
- User Guide: added Flatpak Sandbox Overrides section

* Wed Apr 02 2026 Anton Isaiev <totoshko88@gmail.com> - 0.10.9-0
- See CHANGELOG.md for full release notes
- Corrected: v0.9.11 incorrectly stated Flatpak uses --device=serial;
  the actual permission is --device=all (required for serial port access)

* Thu Mar 27 2026 Anton Isaiev <totoshko88@gmail.com> - 0.10.8-0
- See CHANGELOG.md for full release notes

* Wed Mar 26 2026 Anton Isaiev <totoshko88@gmail.com> - 0.10.7-0
- See CHANGELOG.md for full release notes

* Tue Mar 24 2026 Anton Isaiev <totoshko88@gmail.com> - 0.10.6-0
- Fixed Passbolt CLI integration broken with CLI 0.4.2 — serde
  deserialization failed on lowercase JSON fields; added rename
  attributes for underscore-prefixed fields; made id/name optional
  in resource detail (#69)
- Fixed blurry/artifact RDP image on HiDPI displays — set device scale
  on Cairo surface for 1:1 pixel rendering; adaptive filter selection
- Fixed 1Password JSON parse errors silently ignored — now logs warning
- CLI downloads: 1Password CLI 2.33.0→2.33.1
- Dependencies: ipconfig 0.3.2→0.3.4, libredox 0.1.14→0.1.15,
  proptest 1.10.0→1.11.0

* Tue Mar 24 2026 Anton Isaiev <totoshko88@gmail.com> - 0.10.5-0
- Fixed KeePassXC CLI integration — database password not passed (#68)
- Fixed KeePassXC CLI silent error swallowing — added tracing warnings
- Added -q flag to all keepassxc-cli show commands
- Fixed GTK warnings on startup — suppressed deprecated
  gtk-application-prefer-dark-theme on KDE/XFCE; removed unsupported
  @media (prefers-reduced-motion) CSS media query
- CI: replaced deprecated flatpak-builder@master (Node.js 20) with v6
- Dependencies: deflate64 0.1.12, toml 1.1.0, zip 8.4.0

* Sun Mar 22 2026 Anton Isaiev <totoshko88@gmail.com> - 0.10.4-0
- Flatpak manifests: added missing FreeRDP and Waypipe modules
- i18n: wrapped 5 untranslated UI strings, translated for 15 languages
- Snap license corrected: GPL-3.0+ to GPL-3.0-or-later
- ARM64 release builds added (deb, rpm, appimage)
- Removed duplicate changelog entries in packaging files

* Sat Mar 21 2026 Anton Isaiev <totoshko88@gmail.com> - 0.10.3-0
- Security: RDP password no longer exposed in /proc — uses /from-stdin pipe
- Security: SSH agent askpass script zeroized before deletion
- Security: CLI --password flag shows security warning
- Security: Legacy XOR credential decryption now logged
- Fixed highlight rules not applied without per-connection rules (#66)
- Fixed CLI add --protocol zerotrust silently creating SSH connection
- Fixed config file corruption on crash — atomic temp-file + rename
- Fixed blocking DNS in async check_port_async
- Improved sidebar tooltip with full connection name on hover
- Improved log sanitization performance — LazyLock regex compilation
- Improved CLI parse_protocol consolidated into shared utility
- Dead code cleanup — removed unused AppStateError, VncLauncher, FieldValidator
- Dependencies: rustls-webpki 0.103.10, zune-jpeg 0.5.14

* Thu Mar 20 2026 Anton Isaiev <totoshko88@gmail.com> - 0.10.2-0
- Fixed MOSH connections not dispatched — added start_mosh_connection()
- Fixed auto-recording not triggered from session_recording_enabled toggle
- Fixed highlight rules not applied to terminal after connection
- Fixed script command visible on recording start — delayed erase
- Fixed double exit and UI freeze on recording stop — Ctrl+D + async SCP
- Fixed lost commands in recording playback — strip_script_command_echo()
- Fixed .rdp file association — MIME type XML for all packaging formats
- Fixed sidebar stretching with long connection names — ellipsize
- Fixed picocom not detected in Flatpak — which_binary() fallback
- Fixed RDP error message when FreeRDP not installed
- Fixed IronRDP debug log spam — filtered to warn level
- Improved CSV import delimiter auto-detection
- Improved script credentials test button with 30s timeout and feedback
- Added config sync documentation to User Guide
- Dependencies: shell-words 1.x (rustconn crate), aws-lc-rs 1.16.2, tar 0.4.45

* Wed Mar 18 2026 Anton Isaiev <totoshko88@gmail.com> - 0.10.1-0
- MOSH protocol — predict mode, SSH port, UDP port range, server binary
- CSV import/export — RFC 4180, auto column mapping, delimiter options
- Session recording — scriptreplay-compatible format, sanitization, REC indicator
- Text highlighting rules — regex patterns, per-connection + global, VTE integration
- Ad-hoc broadcast — send keystrokes to multiple terminals simultaneously
- Smart Folders — dynamic connection grouping by protocol/tags/host/group
- Script credentials — PasswordSource::Script with shell-words and 30s timeout
- Per-connection terminal theming — background/foreground/cursor color overrides
- 15 language translations for all new v0.10.1 UI strings
- New dependencies: csv 1.x, glob 0.3, shell-words 1.x
- Fixed Flatpak SSH key paths stale after rebuild — stable copy with fallback
- Fixed SFTP ssh-add stale portal key path — resolve before use
- Fixed SFTP mc opens when ssh-add fails — abort with toast error
- Fixed script command format for modern util-linux (--log-out/--log-timing)
- Fixed remote SSH recording using local paths
- Fixed recording playback showing script header
- Fixed script invocation visible in terminal
- Fixed SCP host key verification prompts in stop_recording
- Fixed RDP sidebar status not clearing after disconnect
- Fixed PlaybackToolbar GtkSearchEntry finalization warning
- Fixed cargo/config deprecation warning in Flatpak build
- Flatpak local manifest runtime: GNOME 50beta → 50
- Dependencies: euclid 0.22.14, toml 1.0.7, zerocopy 0.8.47, zip 8.3

* Mon Mar 16 2026 Anton Isaiev <totoshko88@gmail.com> - 0.10.0-0
- Note: Flatpak release will follow after March 18, 2026, when
  GNOME 50 runtime is published on Flathub
- RDP file import in GUI — .rdp files can now be imported via Import dialog
- CLI import: 4 new formats — rdp, rdm, virt-viewer, libvirt
- Split view for Telnet, Serial, Kubernetes — all VTE-based protocols
- Statistics: Most Used connections and Protocol Distribution with progress bars
- 5 new customizable keybindings (31 total): Toggle Sidebar, Connection
  History, Statistics, Password Generator, Wake On LAN
- Secret backend default changed from KeePassXc to LibSecret
- RDP file association — double-click .rdp files to open and connect
- FreeRDP 3.24.0 bundled in Flatpak — external RDP works out of the box
- sdl-freerdp3 and unversioned FreeRDP binary detection
- GTK4/libadwaita/VTE crate upgrade: gtk4 0.11, libadwaita 0.9,
  vte4 0.10, gdk4-wayland 0.11 — unlocks GNOME 48–50 widget APIs
- MSRV bumped to 1.92 across all crates, CI, and packaging
- Flatpak runtime bumped to GNOME 50 with VTE 0.80
- AdwSpinner, AdwShortcutsDialog, AdwSwitchRow, AdwWrapBox migrations (cfg-gated)
- CSS prefers-reduced-motion support for accessibility
- Tiered distro feature flags in OBS packaging: adw-1-8 for
  Tumbleweed/Slowroll/Fedora 43+, adw-1-6 for Leap 16.0/Fedora 42
- Fixed default window size too small on first start
- Fixed RDP gateway ignored in embedded mode — auto-fallback to FreeRDP
- Fixed external RDP sidebar icon stays green after tab close
- Fixed SSH jump host broken in Flatpak
- Fixed mc wrapper not found in Flatpak on openSUSE
- Fixed ZeroTrust and Kubernetes connections broken in Flatpak —
  CLI tools detected and executed via flatpak-spawn --host;
  cloud CLI config dirs mounted into sandbox
- Fixed split view text selection broken by GestureClick handler
- Fixed untranslated protocol display names across all 15 languages
- Codebase cleanup: removed unused CSS classes, consolidated futures-util,
  fixed metainfo.xml, removed dead code

* Wed Mar 11 2026 Anton Isaiev <totoshko88@gmail.com> - 0.9.15-0
- Added "Show Local Cursor" option for embedded RDP, VNC, and SPICE
  viewers — hides local OS cursor to eliminate double cursor (#51)
- Fixed VNC session ignores Display Mode setting — Fullscreen and
  External modes now work correctly (#50)
- Fixed SSH port forwarding via UI broken — protocols.rs skipped
  port_forwards, X11, compression, ControlPersist; now delegates to
  SshConfig::build_command_args() (#49)
- Fixed SSH custom options -o prefix not stripped (#49)
- Fixed SSH custom options placeholder misleading (#49)

* Wed Mar 11 2026 Anton Isaiev <totoshko88@gmail.com> - 0.9.14-0
- Fixed SSH connection fails in Flatpak on KDE — host SSH_ASKPASS
  (e.g. ksshaskpass) stripped from VTE child environment (#48)
- Fixed header bar buttons clipped when sidebar + monitoring enabled —
  ellipsize on variable-length labels, overflow hidden on monitoring bar (#47)
- Dependencies: tokio 1.49→1.50, uuid 1.21→1.22, regex 1.11→1.12,
  proptest 1.9→1.10, tempfile 3.23→3.26, zip 8.1→8.2,
  criterion 0.8.1→0.8.2, rpassword 7.3→7.4

* Mon Mar 09 2026 Anton Isaiev <totoshko88@gmail.com> - 0.9.13-0
- Fixed RDP handshake timeout on heavily loaded servers — Phase 3
  (TLS upgrade + NLA + connect_finalize) wrapped in tokio timeout
- Fixed ARM64 binary download mismatch — no x86_64 fallback on aarch64
- Added RDP Quick Actions menu — 6 Windows admin shortcuts on embedded
  RDP toolbar (Task Manager, Settings, PowerShell, CMD, Event Viewer, Services)

* Sun Mar 08 2026 Anton Isaiev <totoshko88@gmail.com> - 0.9.12-0
- Security: Removed sshpass dependency; uses native VTE injection and SSH_ASKPASS
- Security: Bitwarden master password zeroized on drop (Zeroizing<String>)
- Security: SSH monitoring askpass script cleaned up automatically via RAII
- Changed: SPICE embedded client enabled by default with remote-viewer fallback
- Improved: Extracted vault operations from state.rs (~979 lines)
- Improved: Extracted edit/terminal/split-view actions from window/mod.rs (~1671 lines)
- Removed: sshpass from all packaging manifests

* Sat Mar 07 2026 Anton Isaiev <totoshko88@gmail.com> - 0.9.11-0
- Security: Bitwarden session key uses SecretString with zeroization
- Security: Config files written with 0600 permissions, config dir 0700
- Security: SSH monitoring uses StrictHostKeyChecking=accept-new
- Security: Session log sanitization active by default
- Security: Flatpak device permissions scoped to --device=serial
- Security: Monitoring password uses SecretString with zeroization
- Security: RDP TLS certificate policy documented with tracing::warn
- Fixed encrypted document format ambiguity with V2 magic header RCDB_EN2
- Added monitoring: remote host private IP with IPv4/IPv6 tooltip
- Added monitoring: live uptime counter updates on every polling tick
- Added monitoring: stopped indication with warning icon and dimmed bar
- Added monitoring: all mount points in disk tooltip (snap/tmpfs filtered)
- Removed dead read_import_file_async from import traits

* Fri Mar 06 2026 Anton Isaiev <totoshko88@gmail.com> - 0.9.9-0
- Fixed sshpass not installed in Flatpak (#42)
- Fixed jump host connections fail port check (#41)
- Fixed jump host dropdown — added host address to labels, enabled search
- Fixed jump host monitoring — SSH commands include -J chain (#41)
- Fixed jump host false positive connection status (#41)
- Dependencies: Bitwarden CLI 2026.1.0→2026.2.0, uuid 1.21.0→1.22.0

* Thu Mar 05 2026 Anton Isaiev <totoshko88@gmail.com> - 0.9.8-0
- Security: RDP password no longer exposed on command line (uses /from-stdin)
- Fixed SSH connection status, automation cursor, RDP keyboard duplication
- Protocol dialog improvements for SSH, RDP, VNC, SPICE, Serial, K8s, Telnet, Zero Trust
- SFTP mc split view, context menu "New Connection", granular logging options
- Connection dialog and embedded RDP decomposed into focused submodules

* Wed Mar 04 2026 Anton Isaiev <totoshko88@gmail.com> - 0.9.7-0
- Connection group not saved — dialog used separate Rc for groups_data
  in save closure, so selected subgroup was always lost on save
- Secret variable values lost after settings reopen — values cleared
  before disk persist but never restored from vault on dialog open
  or ${VAR} substitution in connections
- Crash on session reconnect — close_tab held immutable borrow on
  sessions while close_page synchronously fired signal handler needing
  mutable borrow; separated borrow from close call (#39)
- Bitwarden credential lookup speed — removed per-retrieve bw sync and
  added 120s verification cache for bw status; vault syncs once on
  unlock, making reconnect and batch operations significantly faster

* Mon Mar 02 2026 Anton Isaiev <totoshko88@gmail.com> - 0.9.6-0
- Bitwarden Flatpak: build_command falls back to global session store (#28)
- Bitwarden Settings auto-unlock uses resolved bw CLI path (#28)
- Connection dialog credential download uses generate_store_key (UUID-based)
- Vault credential resolve for non-KeePass backends via dispatch_vault_op
- Inherit condition no longer blocked by kdbx_enabled for Bitwarden/1Password
- Group password load dispatches to configured default secret backend
- SSH known_hosts persists in Flatpak via writable UserKnownHostsFile path
- Duplicate reconnect banner prevented via per-session tracking
- SSH dialog hides key fields for Keyboard Interactive auth method

* Sun Mar 01 2026 Anton Isaiev <totoshko88@gmail.com> - 0.9.5-0
- SSH/Telnet pre-connect port check — fail fast with retry toast
- Vault credential lifecycle — orphaned cleanup, paste duplication,
  group rename/move migrates KeePass entries
- Consistent credential keys across all secret backends
- SecretManager cache TTL — entries expire after 5 minutes
- Inherit cycle protection via HashSet visited guard
- Group change in connection dialog now correctly persists on save
- Monitoring waits for SSH handshake before opening channel
- SecretString migration for RDP/SPICE events, GUI structs, CLI input
- VaultOp dispatch consolidation, mutex lock safety, error logging
- CSS extraction, i18n consistency, CI --all-features coverage
- Dead code removal: StateAccessError, unused sidebar methods

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

