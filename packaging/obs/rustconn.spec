#
# spec file for package rustconn
#
# Copyright (c) 2025 Anton Isaiev
# SPDX-License-Identifier: GPL-3.0-or-later
#

Name:           rustconn
Version:        0.5.6
Release:        0
Summary:        Modern connection manager for Linux (SSH, RDP, VNC, SPICE)
License:        GPL-3.0-or-later
URL:            https://github.com/totoshko88/RustConn
Source0:        %{name}-%{version}.tar.xz
Source1:        vendor.tar.zst

# Rust 1.87+ required (MSRV)
# openSUSE: use devel:languages:rust repo for Rust 1.87+
# Fedora/Ubuntu/Debian: use rustup fallback since system Rust < 1.87
%if 0%{?suse_version}
BuildRequires:  cargo >= 1.87
BuildRequires:  rust >= 1.87
BuildRequires:  cargo-packaging
BuildRequires:  alsa-devel
%endif

%if 0%{?fedora}
# All Fedora versions: use rustup (even F42 has only 1.85)
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

%description
RustConn is a modern connection manager for Linux desktops built with GTK4.
It provides a unified interface for managing SSH, RDP, VNC, and SPICE
remote connections with support for both embedded and external clients.

Features:
- SSH connections with embedded terminal and split view
- RDP support via FreeRDP (embedded and external)
- VNC support via TigerVNC (embedded and external)
- SPICE support via virt-viewer (embedded and external)
- Zero Trust providers: AWS SSM, GCP IAP, Azure Bastion, OCI Bastion
- Connection groups and tags
- Import/export (Remmina, Asbru-CM, SSH config, Ansible)
- Secure credential storage (KeePassXC, libsecret)
- Session logging
- Command snippets and cluster commands
- Wake-on-LAN
- RDP audio playback support

%prep
%autosetup -a1 -n %{name}-%{version}

# Install rustup for Fedora/RHEL (system Rust < 1.87)
%if 0%{?fedora}
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.87.0 --profile minimal
export PATH="$HOME/.cargo/bin:$PATH"
%endif

%if 0%{?rhel}
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.87.0 --profile minimal
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
# Ensure rustup path is available for Fedora/RHEL
%if 0%{?fedora} || 0%{?rhel}
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
for size in 48 64 128 256; do
    if [ -f "rustconn/assets/icons/hicolor/${size}x${size}/apps/io.github.totoshko88.RustConn.png" ]; then
        install -Dm644 "rustconn/assets/icons/hicolor/${size}x${size}/apps/io.github.totoshko88.RustConn.png" \
            "%{buildroot}%{_datadir}/icons/hicolor/${size}x${size}/apps/io.github.totoshko88.RustConn.png"
    fi
done

if [ -f "rustconn/assets/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg" ]; then
    install -Dm644 "rustconn/assets/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg" \
        "%{buildroot}%{_datadir}/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg"
fi

%files
%license LICENSE
%doc README.md CHANGELOG.md docs/
%{_bindir}/rustconn
%{_bindir}/rustconn-cli
%{_datadir}/applications/io.github.totoshko88.RustConn.desktop
%{_datadir}/metainfo/io.github.totoshko88.RustConn.metainfo.xml
%{_datadir}/icons/hicolor/*/apps/io.github.totoshko88.RustConn.*

%changelog
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

