#
# spec file for package rustconn
#
# Copyright (c) 2024 Anton Isaiev
# SPDX-License-Identifier: GPL-3.0-or-later
#

Name:           rustconn
Version:        0.4.2
Release:        0
Summary:        Modern connection manager for Linux (SSH, RDP, VNC, SPICE)
License:        GPL-3.0-or-later
URL:            https://github.com/totoshko88/rust-cm
Source0:        %{name}-%{version}.tar.xz
Source1:        vendor.tar.zst

BuildRequires:  cargo >= 1.76
BuildRequires:  rust >= 1.76
%if 0%{?suse_version}
BuildRequires:  cargo-packaging
%endif
BuildRequires:  pkgconfig(gtk4) >= 4.14
BuildRequires:  pkgconfig(vte-2.91-gtk4)
BuildRequires:  pkgconfig(libadwaita-1)
BuildRequires:  pkgconfig(dbus-1)
BuildRequires:  pkgconfig(openssl)
BuildRequires:  zstd

# Runtime dependencies
Requires:       gtk4 >= 4.14
Requires:       libadwaita
%if 0%{?suse_version}
Requires:       vte >= 0.74
Requires:       openssh-clients
%else
Requires:       vte291-gtk4
Requires:       openssh-clients
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

%prep
%autosetup -a1 -n %{name}-%{version}
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
