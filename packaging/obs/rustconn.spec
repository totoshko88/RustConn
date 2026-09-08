#
# spec file for package rustconn
#
# Copyright (c) 2026 Anton Isaiev
# SPDX-License-Identifier: GPL-3.0-or-later
#

Name:           rustconn
Version:        0.21.9
Release:        0
# rpmlint caps Summary at 79 characters (summary-too-long, badness 200); the
# protocol list belongs in %description, which has room for all of it. Kept in
# step with debian.control's short description.
Summary:        Modern connection manager for SSH, RDP, VNC, SPICE and more
License:        GPL-3.0-or-later
URL:            https://github.com/totoshko88/RustConn
Source0:        %{name}-%{version}.tar.xz
Source1:        vendor.tar.zst
Source2:        rust-toolchain.tar.zst

# Which targets build with the bundled toolchain (Source2) rather than the
# distribution's Rust. One flag instead of repeating a distro test at four sites,
# because the sites have to agree and a mismatch between them is silent.
#
# Fedora and RHEL: their system Rust has been below the MSRV.
#
# openSUSE Slowroll: this one is not about the Rust version but about which glibc
# it was linked against. Slowroll and Tumbleweed take Rust from the same place,
# `devel:languages:rust/openSUSE_Tumbleweed`, but their bases differ — Tumbleweed
# sits on openSUSE:Factory (glibc 2.44) while Slowroll is a delayed Factory
# snapshot (glibc 2.43). So the resolver picks `rust1.98`, built against Factory,
# and Slowroll cannot satisfy `libm.so.6(GLIBC_2.44)`: unresolvable, every time
# Factory bumps a glibc symbol version. Slowroll's own repository has rust1.97.1,
# which is above the 1.95 MSRV and needs only GLIBC_2.4 — but `cargo-packaging`,
# which provides %%{cargo_build}, exists only in devel:languages:rust, so the
# repository path cannot simply be dropped. Using the bundled toolchain removes
# the need for all three of `rust`, `cargo` and `cargo-packaging` there, which
# takes Slowroll out of that coupling for good rather than until the next bump.
#
# `%%_repository` is an OBS macro naming the repository being built for, and it is
# exported into the build root, so the spec can test it.
%if 0%{?fedora} || 0%{?rhel}
%global bundled_rust 1
%endif
%if "%{?_repository}" == "openSUSE_Slowroll"
%global bundled_rust 1
%endif

# Rust 1.95+ required (MSRV)
# openSUSE: use devel:languages:rust repo for Rust 1.95+
# Fedora 42+: system Rust 1.93 is sufficient
# Fedora <42/RHEL: use rustup fallback since system Rust < 1.95
%if 0%{?suse_version}
%if !0%{?bundled_rust}
BuildRequires:  cargo >= 1.95
BuildRequires:  rust >= 1.95
BuildRequires:  cargo-packaging
%endif
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
# The floors are the crate feature baselines in Cargo.toml — gtk4 v4_14,
# vte4 v0_76, libadwaita v1_5. system-deps fails the build script rather than
# the dependency solver when they are not met, which is a much less obvious
# error, so state them here.
BuildRequires:  pkgconfig(gtk4) >= 4.14
BuildRequires:  pkgconfig(vte-2.91-gtk4) >= 0.76
BuildRequires:  pkgconfig(libadwaita-1) >= 1.5
BuildRequires:  pkgconfig(dbus-1)
# WebKitGTK 6.0 — only on distros that ship it (Tumbleweed, Fedora 43+)
%if 0%{?suse_version} > 1600 || 0%{?fedora} >= 43
BuildRequires:  pkgconfig(webkitgtk-6.0)
BuildRequires:  pkgconfig(javascriptcoregtk-6.0)
%endif
BuildRequires:  desktop-file-utils
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
#
# libadwaita and the ALSA library are deliberately absent: rpm derives
# `libadwaita-1.so.0()(64bit)` and `libasound.so.2()(64bit)` from the linked ELF
# by itself, so naming them by hand only adds a second, weaker claim — rpmlint
# flags it as explicit-lib-dependency, and on openSUSE `libadwaita` is not even a
# package name (the shared library lives in `libadwaita-1-0`).
%if 0%{?suse_version}
Requires:       gtk4 >= 4.14
Requires:       vte >= 0.74
Requires:       openssh-clients
%endif

%if 0%{?fedora} || 0%{?rhel}
Requires:       gtk4 >= 4.14
Requires:       vte291-gtk4
Requires:       openssh-clients
%endif

# Optional runtime dependencies
Recommends:     freerdp
Recommends:     tigervnc
Recommends:     virt-viewer
Recommends:     picocom
Recommends:     kubectl
# Used by the external FreeRDP fallback client for /gfx:AVC420, which accepts a
# distribution build. The *embedded* client cannot: it loads through
# openh264-sys2, which validates the library's SHA-256 against Cisco's own
# published binaries and refuses everything else, so H.264 in embedded mode needs
# a blob from ciscobinary.openh264.org and RUSTCONN_OPENH264 pointing at it.
# See docs/INSTALL.md.
Recommends:     libopenh264

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
- System keyring (GNOME Keyring / KDE Wallet)
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

# Unpack the standalone Rust toolchain where it is used (OBS has no internet for
# rustup). See the bundled_rust definition near the top for which targets and why.
%if 0%{?bundled_rust}
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

# openSUSE's Rust toolchain defaults the gnu-target linker to clang, but the
# bare `/usr/bin/clang` symlink is not reliably present in the OBS build root
# (only the versioned clang-NN binary is installed), so cargo fails with
# "linker `clang` not found". Pin the linker to gcc, which is always present
# via the `gcc` BuildRequires. The `-Wl,-z,relro,-z,now` hardening link-args in
# RUSTFLAGS are forwarded by gcc exactly as by clang, so no hardening is lost.
%if 0%{?suse_version}
cat >> .cargo/config.toml <<EOF

[target.x86_64-unknown-linux-gnu]
linker = "gcc"

[target.aarch64-unknown-linux-gnu]
linker = "gcc"
EOF
%endif

%build
# The bundled toolchain again: %prep and %build are separate shell invocations, so
# the PATH set there does not survive to here.
%if 0%{?bundled_rust}
export PATH="$PWD/rust-toolchain/bin:$PATH"
%endif

# Pick the version features the build host can actually back by asking
# pkg-config, the way debian.rules already does, instead of mapping a distro
# release number to a library version by hand.
#
# The hand-written table this replaces needed an edit for every new distro
# release, and it was wrong about the ones it already listed: Fedora 43/44,
# Tumbleweed and Ubuntu 26.04 carry GTK 4.20 or 4.22, and the table said 4.18
# everywhere. Nothing fails loudly when that goes stale — the build quietly
# compiles a lower baseline and no one finds out.
#
# Every test is --atleast-version rather than a glob over --modversion, which is
# what debian.rules used to do and which had a trap in it: a case pattern of
# 1.8 or 1.9 does not match libadwaita 1.10, so the first distro to ship 1.10
# would have dropped silently to the 1.5 baseline. --atleast-version also gets
# GTK 4.24 and VTE 0.100 right for free.
#
# Highest match wins, so each ladder is ordered newest-first. Everything lands
# in one comma-separated list and one --features argument; cargo unions repeated
# flags anyway, but a single list is easier to read in a build log.
GTK_VERSION=$(pkg-config --modversion gtk4 2>/dev/null)
ADW_VERSION=$(pkg-config --modversion libadwaita-1 2>/dev/null)
VTE_VERSION=$(pkg-config --modversion vte-2.91-gtk4 2>/dev/null)

# Everything from `default` except web-embedded, which is detected below.
FEATURES="tray,system-keyring,vnc-embedded,rdp-embedded,gfx-h264,rdp-audio,rd-gateway,wayland-native"

if pkg-config --atleast-version=1.8 libadwaita-1 2>/dev/null; then
    FEATURES="$FEATURES,adw-1-8"
elif pkg-config --atleast-version=1.7 libadwaita-1 2>/dev/null; then
    FEATURES="$FEATURES,adw-1-7"
elif pkg-config --atleast-version=1.6 libadwaita-1 2>/dev/null; then
    FEATURES="$FEATURES,adw-1-6"
fi

# GTK: a newer library's runtime behaviour arrives with the link and needs no
# feature at all. What the feature buys is access to API added in that version,
# and deprecation warnings for what it retired.
if pkg-config --atleast-version=4.22 gtk4 2>/dev/null; then
    FEATURES="$FEATURES,gtk-4-22"
elif pkg-config --atleast-version=4.20 gtk4 2>/dev/null; then
    FEATURES="$FEATURES,gtk-4-20"
elif pkg-config --atleast-version=4.18 gtk4 2>/dev/null; then
    FEATURES="$FEATURES,gtk-4-18"
fi

# VTE termprops, and with them the Command monitoring mode. Below 0.78
# system-deps fails inside the build script rather than in the dependency
# solver, which is a much less obvious error than a resolver conflict.
if pkg-config --atleast-version=0.78 vte-2.91-gtk4 2>/dev/null; then
    FEATURES="$FEATURES,vte-0-78"
fi

# WebKitGTK 6.0 is absent on Leap 16.0 and Fedora 42, which carry only the
# GTK3-flavoured webkit2gtk 4.1.
if pkg-config --exists webkitgtk-6.0 2>/dev/null; then
    FEATURES="$FEATURES,web-embedded"
fi

echo "=== gtk4 $GTK_VERSION | libadwaita $ADW_VERSION | vte $VTE_VERSION => --features $FEATURES ==="

# The cargo_build macro comes from cargo-packaging, which is only build-required
# when the distribution's Rust is used. A bundled-toolchain target has neither the
# macro nor a reason to want it, so it takes the plain invocation even on openSUSE.
#
# Do not write that macro's name with a single percent sign anywhere in this
# section, comment or not. RPM expands macros in %%build before the shell ever sees
# the text, and cargo_build expands to several lines: the '#' hides only the first,
# and the rest become live commands. Doing exactly that here broke Tumbleweed and
# Leap for one cycle with `error: unexpected argument 'comes' found`, the tail of
# this very sentence having been appended to a real cargo invocation.
%if 0%{?suse_version} && !0%{?bundled_rust}
%{cargo_build} -p rustconn --no-default-features --features "$FEATURES"
%{cargo_build} -p rustconn-cli --features full
%else
# --offline: belt-and-suspenders against accidental network access —
# all crates come from the vendored sources configured in .cargo/config.toml
cargo build --release --offline -p rustconn --no-default-features --features "$FEATURES"
cargo build --release --offline -p rustconn-cli --features full
%endif

%install
install -Dm755 target/release/rustconn %{buildroot}%{_bindir}/rustconn
install -Dm755 target/release/rustconn-cli %{buildroot}%{_bindir}/rustconn-cli
install -Dm644 rustconn/assets/io.github.totoshko88.RustConn.desktop \
    %{buildroot}%{_datadir}/applications/io.github.totoshko88.RustConn.desktop
desktop-file-validate %{buildroot}%{_datadir}/applications/io.github.totoshko88.RustConn.desktop
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

# Locale files (compile .po to .mo). The .po basename *is* the locale directory
# name, which is why the catalogue is `zh_CN.po` and not `zh-cn.po`: gettext looks
# up `zh_CN`, so the hyphenated directory this used to create was never found and
# the Chinese translation never loaded.
for po_file in po/*.po; do
    [ -f "$po_file" ] || continue
    lang=$(basename "$po_file" .po)
    mkdir -p "%{buildroot}%{_datadir}/locale/$lang/LC_MESSAGES"
    msgfmt -o "%{buildroot}%{_datadir}/locale/$lang/LC_MESSAGES/rustconn.mo" "$po_file"
done

# Generates rustconn.lang with a %%lang() marker per catalogue, so locale
# filtering works and rpm owns the directories. Replaces the hand-maintained
# %%dir entries that were needed for locales the base system does not create.
%find_lang %{name}

# No %%check section, on purpose.
#
# It existed from 0.20.9 (added with the rpmlint cleanup) until 0.20.10, but it
# never actually ran: the spec was not synced to OBS until then, so the first
# build that executed it was also the first to fail on it. GitHub CI runs the
# very same suites on the very same commit before a tag exists — 3900-odd tests
# across the workspace, plus property tests — so OBS would be re-running work
# that is already green, five times over, once per RPM repository.
#
# What it did catch was its own environment. `cargo test -p rustconn-core` fails
# in an OBS worker on
#
#     mc_ssh::tests::wrapper_hands_ssh_the_jump_host_end_to_end
#     wrapper failed: .../ssh: line 4: /usr/bin/ssh: No such file or directory
#
# because that test executes the generated wrapper, which `exec`s the real ssh.
# openssh-clients is a runtime Requires here, not a BuildRequires, so a build VM
# has no ssh at all and `find_real_ssh` falls back to a path that does not exist.
# Adding openssh-clients to BuildRequires would buy a passing test at the cost of
# a build dependency that exists only to satisfy a test, on every repository.
#
# If a package-time check is ever wanted, scope it to suites that touch no
# external binary — and expect to keep that list honest by hand.

%files -f %{name}.lang
%license LICENSE
%doc README.md CHANGELOG.md docs/
%{_bindir}/rustconn
%{_bindir}/rustconn-cli
%{_datadir}/applications/io.github.totoshko88.RustConn.desktop
%{_datadir}/mime/packages/io.github.totoshko88.RustConn-rdp.xml
%{_datadir}/mime/packages/io.github.totoshko88.RustConn-vv.xml
%{_datadir}/metainfo/io.github.totoshko88.RustConn.metainfo.xml
%{_datadir}/icons/hicolor/*/apps/io.github.totoshko88.RustConn.*

%changelog
* Tue Sep 08 2026 Anton Isaiev <totoshko88@gmail.com> - 0.21.9-0
- Version bump to 0.21.9
- Added: "Open Browser via Tunnel" on an SSH connection — SOCKS proxy to the host
  with a browser (embedded or external Chromium) routed through it, configurable
  start page and external-browser command, default-browser aware with embedded fallback
- Added: a Web connection can browse through an SSH host (SOCKS tunnel) in the embedded and a Chromium browser
- Added: interactive "ASK" variables — prompt for a value at connect time
- Added: a dynamic SOCKS forward can pick a free local port automatically
- Added: built-in date, time and environment variables in ${...} substitutions
- Added: built-in Expect templates for the newer OpenSSH host-key prompt and bare "any key to continue" banners
- Fixed: an embedded Web connection to an unreachable host no longer leaves a blank browser tab open
- Fixed: the Jump Host and Tunnel dropdowns' search box now filters the list
- Fixed: a ProxyCommand relay no longer leaves the target session hung at the password prompt (#322)
- Fixed: the "Executing:" echo now quotes a ProxyCommand value so it no longer looks ignored (#322)

* Mon Sep 07 2026 Anton Isaiev <totoshko88@gmail.com> - 0.21.8-0
- Version bump to 0.21.8
- Fixed (security): a password typed at an interactive prompt could reach the
  session log in the clear (#321); the input after a sensitive prompt is redacted
- Fixed: an Expect-rule response could not resolve a connection-local variable
  (#317); local variables are now resolved too
- Fixed: embedded RDP clipboard file transfer now works in both directions —
  downloads ("Save N Files") and dragging a file onto the session, offered via
  IronRDP's file-copy API; the file transfers in full
- Improved: a file dropped on an RDP session confirms with a toast — it lands on
  the remote clipboard, press Ctrl+V in the session to paste it
- Fixed (security): a server clipboard filename could escape the chosen folder
  via an absolute path or ".."; names are reduced to one safe component and an
  existing file is never clobbered
- Fixed: a failed-write report panicked, a refused file never let the batch
  finish, and a hostile server could loop or corrupt a download; a 512 MiB cap,
  a re-entry guard, monotonic stream ids and directory filtering fix it
- Fixed (security): an oversized clipboard file request could allocate up to
  4 GiB; capped at 8 MiB per response
- Fixed (security): SecretSettings and Credentials Debug could print stored
  credentials in the clear; both redact by hand now
- Fixed: secret-backend CLI calls could hang the caller forever; bounded by a
  30-second timeout that also kills the child
- Improved: the auto-login timeout can now be set on a group
- Improved: the macOS window header is closer to a native titlebar height
- Dependencies: rustls 0.23.43->0.23.44

* Sun Sep 06 2026 Anton Isaiev <totoshko88@gmail.com> - 0.21.7-0
- Version bump to 0.21.7
- Fixed: the Session Logs dialog could not be made narrow — it is an
  adw::OverlaySplitView now, collapsing the file list into an overlay below 500sp
  with a header toggle, instead of a gtk::Paned that kept both panes on screen
- Fixed: two boxed lists were drawn inside a second border, and the Credentials
  Storage row carried both an "ℹ" character and an information icon; both removed
- Fixed: client detection on Preferences ▸ Clients reported by colour and glyph
  alone; each indicator now carries the state as words as accessible label and
  tooltip
- Fixed: the Keyboard Shortcuts page showed internal action identifiers as row
  subtitles; the subtitle is gone
- Fixed: three switches in Preferences ▸ Secrets were bare GtkSwitch widgets in an
  AdwActionRow suffix rather than AdwSwitchRow; all three are AdwSwitchRow now
- Fixed: a smart folder whose icon was an icon name displayed the name as text;
  the folder row now tells an emoji from a themed icon name
- Fixed: the Smart Folders context menus were invisible to a screen reader and
  unusable from the keyboard; they go through the shared show_popover builder now
- Fixed: the embedded VNC status screen was English-only and its Copy button did
  nothing; both now go through i18n() and the RDP clipboard helper
- Fixed: the connection wizard threw away the icon when Advanced… was pressed, and
  the connection editor kept calling a URL a Host and left empty rows behind
- Changed: every ellipsis in the interface is one "…" character instead of three
  periods; 111 strings across 31 files normalised, merging duplicated msgids
- Removed: around 390 lines of unreachable UI code, including a fullscreen button
  that flipped a private bool and called no window API
- Dependencies: crossbeam-channel 0.5.16→0.5.17, crossbeam-deque 0.8.7→0.8.8,
  crossbeam-epoch 0.9.20→0.9.21, crossbeam-utils 0.8.22→0.8.23, der 0.8.1→0.8.2,
  ipnet 2.12.1→2.12.2

* Sat Sep 05 2026 Anton Isaiev <totoshko88@gmail.com> - 0.21.6-0
- Version bump to 0.21.6
- Added: an optional second confirmation before a snippet runs (#315) — a Confirm
  before running switch in the snippet editor, off by default so existing snippets
  keep running on a single action. It covers every route a snippet can be started
  from, including the inline entries in the terminal's right-click menu and the
  Scripts menu of an embedded RDP session
- Added: rustconn-cli snippet honours the same flag — --confirm on add, --confirm
  [true|false] on edit, and snippet run --execute prompts on stderr. With no
  terminal on stdin it refuses rather than prompting into a pipe; --force is the
  opt-out for scripts that mean it
- Fixed: Bitwarden auto-unlock did nothing in every interface language but English
  (#312). The vault state was compared as a translated display string against the
  literal "Locked", so outside English the guard read "not locked" and skipped the
  unlock. The state is an enum now and the decision is made on the variant. It
  also fires on an inconclusive probe, a second route to the same silence
- Fixed: the startup banner announced that Bitwarden could not store passwords
  while Bitwarden was storing them (#312). The readiness probe ran bw status
  without BW_SESSION, so the CLI could not see the session and answered locked
- Fixed: a Bitwarden password was written to the vault and reported as refused at
  the same time (#312). The ten-second budget expired mid-write while bw carried
  on and completed it; the budget is chosen per backend now, 45s for the four
  CLI-backed ones, and the message states the budget actually applied
- Fixed: every bw unlock ran without a deadline, so an unlock stalled on a network
  sync blocked its caller indefinitely. All four bw invocations now share one 30s
  ceiling. Alongside: the unlock logged the master password's length, and raw bw
  stderr could carry a session key into a log and into a user-visible error
- Fixed: Add SSH Key did nothing and there was no way to find out why. The file
  chooser's callback discarded every failure. Dismissal is now told apart from
  failure, and a second click supersedes the first request instead of leaving the
  button unable to open a chooser at all
- Fixed: the Add Key passphrase dialog had no visible way out — Cancel in the
  header now, and Enter in the passphrase field submits
- Fixed: global variables that could not be written to disk were reported as saved
- Fixed: a standalone SSH tunnel could fail in complete silence while the
  diagnosis was being built and thrown away. It reports Failed rather than
  Stopped, with ssh's own words in a Last Error row, and the remedy names the
  package that is actually missing — mptcpize ships with the Multipath TCP tools,
  not with the OpenSSH client. The tunnel manager is redrawn while it is open
- Fixed: running a snippet from the terminal's right-click menu could do nothing
  at all when a variable could not be resolved; it opens the variable dialog now
- Fixed: the embedded RDP confirmation showed the command untruncated, so a
  generated one-liner made the dialog unreadable on exactly the snippets a
  confirmation is worth having for. Same 400-character cap as the terminal gate
- Fixed: RUST_LOG=debug wrote the RDP account password into the log in clear.
  sspi logs the encoded CredSSP TSRequest at debug level, and field [2] of
  TSPasswordCreds is the password as plain UTF-16LE, so the log carried a
  decodable byte array while the same line printed "password: Secret" in its span
  fields. An sspi=warn directive is added after EnvFilter::from_default_env(), so
  RUST_LOG cannot re-enable it. Anyone who ran 0.21.5 or earlier with
  RUST_LOG=debug against an RDP host should treat that password as disclosed
- Fixed: a clean shutdown reported that every terminal child had ignored SIGTERM.
  The liveness check answers true for a zombie, which on shutdown is the normal
  state, since GLib's child watch is gone by then and cannot reap. It reads state
  Z from /proc/<pid>/stat now; the SIGKILL to the process group stays, because an
  ssh with a ProxyCommand shares its group with helpers the exit does not reach
- Fixed: an unset console keymap made every RDP session US English. localectl
  status prints VC Keymap above X11 Layout, and on a desktop the console keymap is
  normally unconfigured, so the parser read "(unset)" and fell back with the
  layout it wanted on the next line — every scancode then interpreted against the
  wrong table. X11 Layout now wins wherever it appears, systemd's placeholders for
  an unset value are rejected, and both sources report the whole group list
- Fixed: with LibSecret, a password saved on a connection in no group was never
  found again (#316). Four call sites built the group argument themselves and
  disagreed on what "no group" is: saving passed Some(""), resolving passed None,
  so the password was written under RustConn/{name} ({protocol}) and looked for at
  {name} ({protocol}). A grouped connection escaped it because there the legacy
  second lookup differs and hits. One function decides now, and tests pin the
  GUI's key against the resolver's
- Fixed: the shutting-down flag was set after the session exits it exists to
  explain, from a GTK shutdown handler that runs after every session child has
  already been signalled — so the guard suppressing a reconnect banner for a
  session the teardown just killed was unreachable. It is set at the top of the
  teardown every exit funnels through, which a tray-minimize never reaches. The
  post-disconnect command is skipped at shutdown rather than started and abandoned
- Fixed: a second Quit stacked a second confirmation instead of raising the first.
  The close confirmation is reached from the window's close_request and from the
  quit action behind Ctrl+Q, the primary menu and the tray, and neither asked
  whether the question was already up. One helper now raises the window carrying
  the pending dialog instead of building another; nothing is read as consent
- Improved: the four hand-rolled bw unlock invocations on the Secrets page are
  gone; they all call one core function with the extended PATH and the deadline
- Improved: one confirmation prompt in rustconn-cli instead of three, reporting
  three outcomes so "nobody to ask" is never treated as consent
- Improved: OpenH264 was probed again on every RDP connection. The embedded client
  dlopens a decoder rather than linking one, and each connection rewalked the
  candidate list, re-emitting the same loader warnings — nine identical lines for
  three connections in one log. The outcome is cached and a fresh decoder built
  from it per session; installing a Cisco blob now needs a restart to be seen
- Improved: a KeePass lookup paid for one database open that could never match.
  Every keepassxc-cli run reopens the KDBX and pays its Argon2 cost, about 700 ms,
  and one of the three candidate paths for a grouped connection was a form
  build_entry_path has never written. The order is pinned by tests now, and the
  search for the binary is remembered instead of redone for every read and write
- Documentation: three claims that were not true of the code — the CLI reference
  said snippet run resolves ${VARIABLE} from Global Variables, which it cannot,
  since a global may be vault-backed and the CLI has no session to unlock it; the
  rustconn-cli agent rules required i18n() in a crate that has never contained
  one; and a comment listing the routes to the snippet picker omitted the terminal
  context menu's own entry, which is that action's main caller
- Documentation: the debug-logging instructions did not say what a debug log
  discloses. The user guide told users to run with RUST_LOG=debug and attach the
  output, which names every host, username and connection opened, and until the
  leak fixed above carried the RDP account password itself. The warning now comes
  before the command rather than after it
- Dependencies: cc 1.4.4-1.4.5, find-msvc-tools 0.1.11-0.1.12, indexmap
  2.14.1-2.14.2, js-sys 0.3.104-0.3.105, syn 3.0.4-3.0.5, tinyvec 1.12.0-1.13.2,
  tokio-rustls 0.26.4-0.26.5, wasm-bindgen 0.2.127-0.2.128 together with its
  -futures, -macro, -macro-support and -shared crates, web-sys 0.3.104-0.3.105,
  zstd-safe 7.2.4-7.3.0, zstd-sys 2.0.16-2.1.0. Six of the fifteen are the
  wasm-bindgen family, which nothing this project ships compiles — no path to it
  exists on the host target, only behind a wasm32 one, so they are recorded
  because Cargo.lock moved and not because a binary did. indexmap is the one with
  real reach, arriving through h2/hyper/reqwest, through serde_yaml_ng and
  through toml_edit behind the GTK macro crates. Thirteen further crates remain
  behind their latest release on semver grounds, a count this run leaves
  unchanged, and cargo deny check advisories is clean against the new lockfile
- Dependencies: FreeRDP (Flatpak) 3.31.0-3.31.1. A papercut release with no
  advisory behind it, but it fixes flickering in the SDL3 client on displays
  scaled other than 1.0, maps keys that client used to drop, and makes WinPR read
  JSON files in binary mode — the sdl-freerdp.json path the bundled cJSON module
  exists to enable. Only Flatpak/Flathub bundle FreeRDP; this package uses the
  distribution's copy and is unaffected
- Dependencies: openh264 2.6.0 is now bundled in all three Flatpak manifests, not
  just the release one. The embedded RDP client dlopens a decoder rather than
  linking one, so the Flathub build had none at all and every AVC420 or AVC444
  session negotiated down to RemoteFX; a local flatpak-builder run had none
  either. Does not affect this package
- Dependencies: the nine other bundled Flatpak modules were checked against
  upstream and are already current, so only FreeRDP moved

* Thu Sep 03 2026 Anton Isaiev <totoshko88@gmail.com> - 0.21.5-0
- Version bump to 0.21.5
- Added: terminal colours can follow the desktop's light/dark preference — a new
  Follow System entry in Preferences > Terminal > Theme. The System colour scheme
  previously reached only the GTK chrome, so a light desktop could surround a dark
  terminal. Terminals also repaint on a mid-session switch (#99), without undoing
  a per-session Backspace/Delete choice (#271)
- Added: a monitoring mode that fires when the remote shell reports a command
  finished, read from VTE's vte.shell.postexec termprop (OSC 133), so it carries
  the exit code and distinguishes success from failure (#236). Needs shell
  integration on the remote host and VTE 0.78+
- Added: monitoring notifications also set AdwTabPage:needs-attention, so the tab
  keeps a mark until selected; this repairs a latent gap in Activity and Silence,
  whose only signal was the single indicator-icon slot
- Fixed: the window rendered light on a dark desktop when the theme was System — a
  notify handler cleared the property AdwStyleManager uses to say it resolved dark
- Fixed: every OBS Debian and Ubuntu package was built with no libadwaita feature
  and no in-tab browser; debian.rules detected the versions and then lost them to a
  comment inside a make continuation chain, which gives each chain its own shell
- Fixed: this spec chose features from a hand-written distro table that had gone
  stale and had no VTE branch at all. It now asks pkg-config --atleast-version for
  libadwaita, GTK, VTE and WebKitGTK, which also closes a second trap: a glob over
  --modversion does not match libadwaita 1.10
- Fixed: the Flatpak, the release RPM and the Homebrew formula were in the same
  position and now detect too
- Fixed: four stylesheet declarations had never applied — .monitoring-bar used
  margin-start/margin-end, which GTK's CSS does not have. A new gate,
  scripts/check-css.sh, parses the sheet through the installed GTK
- Fixed: the monitoring mode picker no longer needs editing when a mode is added
- Fixed: openSUSE Slowroll had no package at all, because it was being handed a Rust
  built for a newer base than its own — "unresolvable: nothing provides
  libm.so.6(GLIBC_2.44)(64bit) needed by rust1.98". Slowroll and Tumbleweed take Rust
  from the same devel:languages:rust/openSUSE_Tumbleweed repository, but Tumbleweed
  sits on Factory with glibc 2.44 while Slowroll is a delayed Factory snapshot on
  2.43, so the newest visible Rust is one it cannot install, and this recurs whenever
  Factory bumps glibc. Slowroll's own repository has rust1.97.1, above the 1.95 MSRV
  and needing only GLIBC_2.4, but the repository path could not simply be dropped
  because cargo-packaging — which provides the cargo_build macro this spec uses on
  openSUSE — exists only in devel:languages:rust. Slowroll now builds with the bundled
  toolchain, the same way Fedora, Debian and Ubuntu already did, which removes the
  need for rust, cargo and cargo-packaging there. The four sites that must agree —
  build requirements, the toolchain unpack in %prep, the PATH export in %build and the
  choice of build invocation — are driven by a single bundled_rust flag rather than a
  distro test repeated four times, because a mismatch between them would be silent.
  Verified by parsing the spec in three colours before uploading: Tumbleweed still
  takes the cargo_build macro and still requires cargo-packaging, Slowroll takes plain cargo
  and drops both, Fedora unchanged
- Fixed: no OBS Debian or Ubuntu package had ever contained the in-tab browser,
  because one build-dependency list was checked against another that did not have it.
  OBS assembles the chroot from the Build-Depends in debian.dsc, and
  dpkg-checkbuilddeps inside that chroot then validates debian.control — two separate
  lists that nothing keeps in sync. libwebkitgtk-6.0-dev was in neither, and
  debian.control asked for libwebkitgtk-6.0-dev | libglib2.0-dev, whose second branch
  is already present through libgtk-4-dev, so the check passed while WebKitGTK was
  never installed. debian.rules then probes with pkg-config, that probe failed, and
  web-embedded was compiled out of a build that reported success. Measured across all
  three deb targets in the 0.21.5 logs: libwebkitgtk-6.0-dev appears zero times and
  the detection line printed an empty web field. It is now listed in both files, so
  Debian 13, Ubuntu 24.04 and Ubuntu 26.04 get the embedded browser for the first
  time. Fixing only debian.control proves the mechanism — the build then fails with
  "dpkg-checkbuilddeps: unmet build dependencies", which is what happened when this
  was first attempted in 0.20.10 and was then papered over by weakening
  debian.control instead of correcting the .dsc. Availability was checked on all
  three repositories first (2.52.6 everywhere, above the 2.40 floor the bindings
  need). This spec was never affected — it has always required
  pkgconfig(webkitgtk-6.0) outright
- Fixed: the .deb and .rpm attached to a GitHub release named three fewer libraries
  than the binary loads, so the .deb failed in the dynamic linker before reaching
  main() with "error while loading shared libraries: libwebkitgtk-6.0.so.4", on a
  package that had installed without complaint (#313, reported by Phil Clifford).
  Both are assembled by hand in the release workflow — the .deb with an inline
  control packed by dpkg-deb, the .rpm by fpm with an explicit --depends list — so
  neither ran the dependency machinery a normal build provides, and both lists fell
  behind the binary when web-embedded entered the crate's default features. Against
  the recursive closure of the old Depends (266 packages) three were unreachable:
  libwebkitgtk-6.0-4, libjavascriptcoregtk-6.0-1 and libasound2t64, the last from
  the RDP audio feature. The lists are now derived instead of remembered —
  dpkg-shlibdeps over the staged binaries for the .deb, --rpm-autoreqprov for the
  .rpm, since fpm writes AutoReqProv: no unless told otherwise — and both steps
  fail the build if WebKitGTK is absent from the result. openssh-client and the
  gtk4 4.14 floor stay declared by hand, being a program and a version no
  referenced symbol proves. The OBS packages were never affected: that Debian build
  goes through dh, so dh_shlibdeps derives the list, and the RPM gets rpmbuild's
  own generator
- Fixed: every tray menu item that opens a session did nothing — Local Shell,
  Quick Connect and all of Recent Connections, on KDE's StatusNotifier and on the
  macOS tray alike, since both feed the same dispatch. They went through the
  widget action muxer, which picks a group by splitting the name on the first dot,
  carrying names spelled for the window's own action group; with no prefix there
  was no group to find and the FALSE return was discarded. Recent Connections was
  broken twice over: it named connect, which takes no parameter and acts on the
  sidebar selection, so the connection picked in the tray had nowhere to arrive.
  All three now activate on the window's own action group, and tray messages are
  logged on arrival — this path had no logging at all
- Fixed: local shell tabs never took part in activity monitoring, in any mode.
  Resolving the configuration gave up when a session had no connection record and
  the caller read that as "do not monitor", returning before the command-finished
  subscription was wired, so the new Command mode could not fire on a local shell
  whatever the shell emitted and there was no "Activity monitoring started" line
  to show it. A connection-less session now takes the global defaults, which is
  what they are for, and notifications use the tab's own name
- Fixed: five icon names had been dropped by adwaita-icon-theme 50 and drew as
  missing-image placeholders, one of them the success mark of the new Command
  mode. The app forces the Adwaita theme at startup, so a name the theme no
  longer carries resolves nowhere and GTK reports nothing — it surfaces only as a
  broken glyph. All nine call sites now use names verified present: the success
  mark, the four sync indicators, the Statistics empty state and two RDP quick
  actions
- Fixed: KeePassXC reported "Could not read the password" for a database that was
  open and healthy, in every non-English interface language. keepassxc-cli exits 1
  for "entry not found", "wrong database key" and "database unreadable" alike, so
  they are told apart by matching its English prose — but it is a Qt program that
  translates that prose, and RustConn exports LANGUAGE to honour its own language
  setting. A merely missing entry was classified as an unreadable database, which
  produced a misleading modal and also skipped "Also read from the encrypted
  file", making a password in credentials.enc unreachable. The child now gets
  LC_MESSAGES=C with LANGUAGE cleared, in the one place all six callers build
  their command, so four other stderr matchers in that file are fixed by the same
  two lines. The character encoding is left as the user had it, so a non-ASCII
  group name or a database named Паролі.kdbx still works
- Fixed: two strings added in 0.21.4 — the Login Timeout row and its subtitle —
  shipped untranslated in all 17 languages, because the template was never
  regenerated and the completeness check compared against that same incomplete
  template. Both are now translated everywhere
- Changed: new gtk-4-18/gtk-4-20/gtk-4-22 features; enabling 4.22 surfaced three
  deprecations, all resolved
- Changed: new installations default to the Follow System terminal theme
- Dependencies: mio 1.2.2→1.2.3, open 5.4.2→5.4.3, toml 1.1.4→1.1.5

* Wed Sep 02 2026 Anton Isaiev <totoshko88@gmail.com> - 0.21.4-0
- Version bump to 0.21.4
- Fixed: a SPICE connection with a stored password failed outright in Flatpak
  with "connection type cannot be detected from URI" (#308), a regression from
  the 0.21.2 fix. The .vv connection file carrying the password was written to
  the sandbox $XDG_RUNTIME_DIR, whose path the host remote-viewer cannot open;
  it is now translated to the host's view and verified, with a URI fallback that
  makes the viewer prompt as before 0.21.2. No new Flatpak permissions
- Fixed: rustconn-cli add/update silently dropped --key and --auth-method for
  non-SSH protocols; both now reject them for anything other than SSH and SFTP,
  and update honours them for SFTP (previously ignored)
- Fixed: --window-mode reported SPICE as supported while ignoring it; SPICE is
  always external, so it is excluded and the code, CLI help and reference agree
- Added: the automatic-login timeout (login_timeout_secs) is now editable in the
  connection editor's Automation tab; previously TOML-only
- Documentation: CLI reference and user guide realigned with the code
- Dependencies: aws-lc-rs 1.18.0→1.18.1, aws-lc-sys 0.44.0→0.45.0

* Tue Sep 01 2026 Anton Isaiev <totoshko88@gmail.com> - 0.21.3-0
- Version bump to 0.21.3
- Fixed: a password the selected backend refused was moved into the encrypted
  file without asking, where the connect path never looks. Saving now targets the
  selected backend only; a refusal opens a dialog and offers the encrypted file
  as a deliberate choice. Reads still walk the chain so pre-switch passwords keep
  resolving
- Fixed: secret backends disagreed on how to report a failure, and three of them
  called an unreadable store a missing entry (KeePassXC, pass, and the KeePass
  resolve path, which could serve a password from another store). All three now
  separate "opened, entry absent" from "did not open"
- Fixed: secret status was reported with the wrong words, the wrong store, or not
  at all — a locked backend shown as unconfigured, a keyring-shaped startup
  banner, Passbolt and Pass stuck on "Detecting…", and the pass probe reading the
  wrong store
- Fixed: the Bitwarden master password was demoted out of its Zeroizing wrapper
  before use; both copies stay wrapped now
- Fixed: an embedded RDP session against a Windows 11 host could be killed by the
  server's own keepalive heartbeat (#262)
- Fixed: an idle window with no connections showed network warnings during a
  Wi-Fi flap
- Changed: "Enable fallback" is now "Also read from the encrypted file" and
  governs reads only
- Changed: the Secrets page reports whether the selected backend can actually
  store a password, re-checked after settings are saved
- Documentation: Bitwarden guide, architecture, user guide and build docs updated
  for the read/write asymmetry and Flatpak CLI login state (#312, #129)
- Dependencies: libredox 0.1.21→0.1.23, ppmd-rust 1.4.0→1.4.1, smallvec
  1.15.2→1.16.0, semver-compatible transitive updates

* Mon Aug 31 2026 Anton Isaiev <totoshko88@gmail.com> - 0.21.2-0
- Version bump to 0.21.2
- Fixed: SSH through a jump host could hang at the target password prompt when
  the ProxyCommand step was slow (#301)
- Changed: a stored SSH password is now handed to OpenSSH through SSH_ASKPASS
  instead of typed into the terminal, in a mode-0600 runtime file whose path
  alone reaches the SSH environment; connections askpass does not cover keep the
  terminal watcher, so no authentication setup loses its stored password
- Fixed: a SPICE connection asked for the password every time, whatever the
  password source (#308); the password is now passed via a mode-0600 .vv file
- Fixed: the embedded web browser lost its login on every restart (#309);
  cookies are now persisted to disk
- Fixed: a new Local Shell tab in Flatpak could open at the wrong size when the
  window had changed size since the last tab (#294)

* Fri Aug 28 2026 Anton Isaiev <totoshko88@gmail.com> - 0.21.1-0
- Version bump to 0.21.1
- Fixed: connections that authenticate with a key waited on a password manager
  lookup before every connect, then warned about the password they never wanted
  (#307). A user whose hosts are all key-authenticated, with no stored
  passwords at all, paid a full round trip each time; against the Bitwarden CLI
  that is several seconds, because it decrypts on every read, and the answer was
  the same every time. The empty answer is now remembered for five minutes, and
  the notice is only shown when the connection is actually set up to want a
  password. Which credential a connection uses is untouched: the cache sits
  after that decision, so a stale record can cost a lookup, never hand a
  connection the wrong password
- Fixed: Local Shell failed to open on a host without script (#306). The button
  reported "Failed to start command: script" and nothing opened. The host shell
  is wrapped in script (util-linux) because that is what gives the shell job
  control — Ctrl-Z, fg, bg — and it was called unconditionally. Fedora moved
  the binary into a package of its own, util-linux-script, in F42, so a minimal
  install does not have it. The host is now probed and the shell is run directly
  when script is missing. Job control is lost in that fallback, which is a real
  downgrade and still better than a button that does nothing
- Fixed: quitting from the tray put the confirmation dialog on a window the user
  was not looking at. Quitting with sessions open asks for confirmation, which
  is correct, and the tray route presented the window first only when it was
  hidden — but a window can be visible and still be behind others, on another
  workspace, or simply unfocused, and in each of those cases the dialog was
  drawn on a surface nobody was watching. The window is now presented whenever
  there is something to confirm
- Fixed: the embedded browser was missing from every OBS .deb, so Web
  connections there offered only System and Custom. That did not affect this
  spec, whose WebKit BuildRequires is guarded by the same condition that selects
  the feature, so the RPMs always had the embedded browser

* Fri Aug 28 2026 Anton Isaiev <totoshko88@gmail.com> - 0.21.0-0
- Version bump to 0.21.0
- Fixed: a Jump Host set on a group or in Preferences was ignored when the
  connection actually opened (#301). It was stored, shown in the editor as
  inherited and synced between machines, then dropped at connect time — for SSH
  when picked from the dropdown, and for RDP, VNC and SPICE however it was set.
  The 0.20.9 notes said inheritance read the same for every protocol; it did
  not. All seven places that start a connection now resolve it the same way,
  including the check that stops the target's password being offered to the
  bastion's own prompt (#191). A free-text ProxyJump set on a group or globally
  still does not reach RDP, VNC or SPICE, which need a saved connection to open
  a tunnel through
- Fixed: the Secrets page in Settings could stay empty instead of listing the
  password managers it found. Each manager is probed by running its
  command-line tool, the probes run together, and the page waited for the
  slowest — so one tool that never answered held all of them. The three most
  likely to do that are the three that reach out over the network or wait for a
  fingerprint prompt. Every probe now gives up after five seconds and reports
  that manager as unavailable
- Fixed: a KeePass operation could freeze RustConn with no way out. Every call
  to keepassxc-cli waited indefinitely, so a database on a network share that
  had gone away, or one locked by another program, left the window
  unresponsive. Reads now give up after ten seconds and writes after thirty,
  and the message says what to look at — including a database whose
  key-derivation settings are too heavy for the machine, which is the cause
  nothing used to mention
- Fixed: printing from an RDP session could stall the session itself. RustConn
  asks the local print system for the queue list when the connection opens, and
  sends the document through it when the guest prints; none of those waits had
  a limit. They are now bounded at two seconds, and losing the answer costs the
  printer list rather than the session
- Security: nine advisory warnings are gone from the dependency list. The macOS
  tray brought in the entire GTK3 binding stack, unmaintained since 2024,
  because the two crates behind it were taken with their default features. None
  of that code was ever compiled — it is Linux-only inside those crates and the
  tray is built only on macOS — but the advisories were reported on every
  platform, since the tools that report them read the lock file. Thirty-nine
  packages removed
- Fixed: a translatable string could ship untranslated with every check
  reporting the catalogues complete, because nothing verified that the
  translation template still described the strings in the source. That had
  happened in three releases in a row. It is now checked, and the check runs
  before a release as well as in CI
- Fixed: a Flatpak build could be stopped by any one of seven download hosts
  having a bad day, and one did — the 0.20.11 release build failed twice on a
  GNU mirror outage. The bundled sources now carry verified fallback mirrors
- Changed: the interface for saving a password into a KeePass database now
  takes a protected string type rather than a plain one. No caller was leaking,
  but the old signature did not prevent the next one from doing so. This is a
  breaking change for anything building against rustconn-core
- Improved: CI now builds and tests the macOS-specific code. Nothing outside
  the maintainer's own machine had ever compiled it, and a test guarding one of
  the low-level system calls had been failing there unnoticed
- Dependencies: argon2 0.5 to 0.6 — the key derivation behind the encrypted
  credential stores. Existing files still open; that is now proved by a fixed
  test vector rather than assumed. quick-xml 0.41 to 0.42 in the connection
  importers

* Thu Aug 27 2026 Anton Isaiev <totoshko88@gmail.com> - 0.20.11-0
- Version bump to 0.20.11
- Fixed: a telnet session left its process running after RustConn was closed
  (#304). Closing the window, pressing Ctrl+Q or quitting from the tray with
  session tabs open ended the application and left every telnet, ssh, picocom
  and shell child alive, so a device that allows one connection at a time stayed
  occupied. The per-tab kill added for #172 works; quitting never closes the
  pages, so it never ran
- Fixed: quitting from the tray tore nothing down at all (#304, and #209/#236
  with it) — it called app.quit() directly, reaching neither the window's close
  handler nor the quit action, so it skipped the close confirmation, the
  external-viewer shutdown and the detached-window close as well. That is the
  exit path a tray user actually takes
- Fixed: a pre_exec contract test in rustconn-pty-sys had never passed on macOS
  — it accepted ENOTTY and EPERM, and macOS answers ENODEV for /dev/null. CI has
  no macOS runner, so the one test guarding the unsafe block behind SSH password
  prompts failed only on the maintainer's platform
- Fixed: SPICE refused to launch with "Install virt-viewer" on a machine that
  had virt-viewer, and Settings -> Clients called it not installed. Client
  detection spawned /usr/bin/which, so every probe reported "missing" on a
  system without that binary. Lookup is now resolved in process, for every
  client probe in the application, and SPICE additionally gained the Flatpak and
  snap search paths plus a flatpak-spawn --host fallback
- Improved: binary lookup now searches /app/bin under Flatpak,
  $SNAP/{usr/bin,bin,usr/local/bin} under snap, the writable per-application CLI
  directories of either, and the Homebrew prefixes a macOS bundle launched from
  Finder does not inherit. The host probe is bounded at two seconds
- Improved: one session teardown for every way the application can exit. The
  close handler, the quit action and the tray's Quit item each carried their own
  list and none was complete; three divergent copies is what #304 was
- Improved: the CI clippy gate was passing on a cache hit rather than on a check
  — it caches target/ keyed on Cargo.lock, so an unchanged lock restored an
  already-linted tree and reported zero warnings without looking. Nineteen
  warnings had accumulated behind it; all are fixed, because this release
  changes both Cargo.lock and rustconn-core/src/lib.rs and so forces a real
  re-lint
- FreeRDP (Flatpak) updated 3.30.0 -> 3.31.0. Upstream calls it a security
  release, lists 22 advisories and asks distributors to update immediately. Only
  Flatpak/Flathub bundle FreeRDP; the deb only recommends the distribution's own
  package, which its own security updates cover
- waypipe (Flatpak) updated 0.11.0 -> 0.11.2
- Dependency updates: chacha20 0.10.2, libredox 0.1.21, rtoolbox 0.0.6, uuid
  1.26.0, wide 1.7.0. cargo audit and cargo deny report no vulnerabilities
  across 733 dependencies
* Wed Aug 26 2026 Anton Isaiev <totoshko88@gmail.com> - 0.20.10-0
- Version bump to 0.20.10
- Note: no 0.20.9 package was ever built. Every 0.20.9 build job failed on the
  compile error below, so everything listed under 0.20.9 reaches users with this
  release. The 0.20.9 tag was left in place rather than moved, which would be
  destructive for anyone who already fetched it
- Fixed: rustconn-cli did not compile with --features full, the set every
  package enables — 0.20.9 gave build_sftp_browser_uri a fourth argument for the
  new global jump-host tier and one call site, the file-manager branch of the
  sftp command, still passed three. That branch sits behind
  #[cfg(feature = "client-launch")] and the crate defaults to no features, so no
  local gate and no CI clippy run compiled it at all
- Improved: scripts/verify.sh now runs clippy over rustconn-cli with
  --features full, and passes -D warnings on the existing clippy gate, so the
  local gate no longer approves a tree the packaging jobs reject

* Wed Aug 26 2026 Anton Isaiev <totoshko88@gmail.com> - 0.20.9-0
- Version bump to 0.20.9
- Added: a connection can take its jump host from its group or from the
  application — the connection's own Jump Host, then the group chain, then a
  new global tier in Settings → Connection → Network. A Network Mode row on the
  editor's Basic page chooses between inheriting and Direct — on Basic because
  the choice applies to every protocol, and the protocol pages show only one at
  a time; rustconn-cli gained --network-mode (issue #301)
- Added: the external RDP client can be told how to size its window — a new
  External Window row in the editor's Display group offers Fit to screen (the
  default), Fullscreen, Custom resolution and All monitors. It governs the
  External client mode and the window an embedded session hands over to;
  rustconn-cli gained --rdp-display-mode and --rdp-resolution
- Fixed: an external RDP window opened at about a quarter of a 4K screen — the
  size handed to FreeRDP came from the embedded viewer's own widget geometry in
  logical pixels. Every RDP profile also stored a 1920x1080 resolution nobody
  chose, which that path then applied; a connection that deliberately used a
  fixed size needs Custom resolution selected once
- Fixed: External client mode ignored an RD Gateway completely and never
  filtered custom FreeRDP arguments, while Display Scale and Color Depth were
  collected from every connection and emitted by nobody. The three argument
  builders are now the single one in rustconn-core
- Fixed: a group's Jump Host was never used — the picker stored the choice but
  nothing read it at connect time, a group with only a jump host lost it on the
  next save, and the bastion rows were hidden until an authentication method
  was chosen (issue #301)
- Fixed: whether a connection inherited its proxy depended on where it got its
  SSH key, and RDP, VNC and SPICE inherited unconditionally with no way to
  refuse; inheritance now keys off Network Mode
- Fixed: rustconn-cli group set --ssh-proxy-jump "" produced ssh -J "" — a
  blank now counts as unset at all three tiers, the global one included
- Fixed: a pinned connection's real row never updated — no status icon,
  recording dot, external-viewer emblem or split marker, and a context menu
  that offered Connect for an open session (issue #302)
- Fixed: the recording dot, external-viewer emblem and split marker vanished on
  any sidebar reload
- Fixed: the Simplified Chinese translation has never loaded in any package —
  zh-cn.po was installed to share/locale/zh-cn/ while gettext looks up zh_CN;
  the catalogue is now po/zh_CN.po
- Fixed: Settings populated its connection dropdowns before it had the list
- Changed: the sidebar's "Open new session" is offered whenever there is a
  session to duplicate, not only for external viewers (issue #302)
- Changed: a server that runs RDS licensing is named as such instead of
  "incompatible"; the external FreeRDP fallback is unchanged
- Changed: an OpenH264 library that is installed but refused now says why, and
  RUSTCONN_OPENH264 names a library to try first
- Improved: every Secret Service call has a deadline, not just the GUI's whole
  credential resolution
- Packaging: rpmlint errors cleared — Summary within 79 characters, %find_lang
  for the locale files, no Requires that rpm already derives, and a %check
  section running the rustconn-core suites
- Localisation: Simplified Chinese now loads; 13 new strings translated in all
  17 locales
- Dependencies: combine 4.6.7→4.6.8
* Mon Aug 24 2026 Anton Isaiev <totoshko88@gmail.com> - 0.20.8-0
- Version bump to 0.20.8
- Fixed: the sidebar context menu did not open when there was no room for it
  below the pointer — its minimum height was the full menu, so GTK had nowhere
  to place the popup; the menu now scrolls within a monitor-derived cap
  (issue #298)
- Fixed: Embedded mode on Web connections reverted to System permanently — a
  build without the web-embedded feature read a stored "embedded" as System and
  the next save rewrote the file; all modes now exist in every build
- Fixed: rustconn-cli --browser-mode embedded stored System instead
- Fixed: editing a Web connection reset its page zoom and certificate exception
- Fixed: embedded RDP and VNC toolbars were barely readable on a light theme —
  all embedded toolbars now follow the local theme
- Fixed: config writes contended with each other inside a single process, with
  an unbounded wait from the GTK thread and a shutdown flush that discarded all
  three pending files on one error
- Fixed: one credential lookup opened 25 encrypted Secret Service sessions;
  each operation now opens one
- Dependencies: h2 0.4.18→0.4.19, open 5.4.1→5.4.2, syn 3.0.3→3.0.4

* Sun Aug 23 2026 Anton Isaiev <totoshko88@gmail.com> - 0.20.7-0
- Version bump to 0.20.7
- Fixed: SSH sessions offered no way to reconnect once the connection ended —
  session logging evicted the disconnect handler from the same signal, so no
  reconnect banner or auto-reconnect ever ran; handlers are now keyed by
  purpose as well as by session (issue #297)
- Fixed: cloned connections could not be edited and their right-click menu did
  not open — a recycled list row cleared the sidebar selection that every
  action resolves through (issue #298)
- Fixed: sidebar context menu opened and vanished within a frame on GNOME
  Wayland — the menu is now parented to the enclosing scrolled window and
  takes the input grab on Wayland (issue #299)
- Changed: moving the sidebar context menu between rows takes a second
  right-click on Wayland; X11 is unchanged
- Dependencies: cc 1.4.3→1.4.4, cfg-expr 0.20.8→0.20.9, crc32fast 1.5.0→1.5.1,
  font-types 0.12.3→0.12.4, icu_provider 2.3.0→2.3.1, keccak 0.2.1→0.2.2,
  log 0.4.33→0.4.34, rustls-webpki 0.103.14→0.103.15, uuid 1.24.1→1.25.0,
  zerovec-derive 0.11.5→0.11.6

* Fri Aug 21 2026 Anton Isaiev <totoshko88@gmail.com> - 0.20.6-0
- Version bump to 0.20.6
- Fixed: RDP used the RemoteFX path even with OpenH264 installed — the probe
  looked only for the unversioned libopenh264.so, which ships in the -dev
  package; versioned sonames are now scanned for
- Fixed: portable file passphrase could be discarded without saying why
- Fixed: warning logged about a portable passphrase that was never entered
- Fixed: spurious "Session not found" warnings when quitting with sessions open
- Fixed: macOS Dock icon changed to a generic tile when pinned and closed
- Improved: sidebar connection status is set once per change instead of twice
- Improved: header bar Shell button restyled flat
- Packaging: libopenh264 added as a recommended package

* Fri Aug 21 2026 Anton Isaiev <totoshko88@gmail.com> - 0.20.5-0
- Version bump to 0.20.5
- Fixed: Flatpak terminal still started at 24x80 on some hosts (issue #294)
- Fixed: shortcut recorder did not warn about conflicts with fixed
  shortcuts like Ctrl+V (issue #295)
- Removed: the stty call that claimed to forward window resizes to a
  Flatpak host shell; it acted on the wrong terminal and never worked
- Documentation: WSL guide updated with Flatpak install option

* Wed Aug 19 2026 Anton Isaiev <totoshko88@gmail.com> - 0.20.4-0
- Version bump to 0.20.4
- Added: portable encrypted file backend, passphrase-protected and syncable
  between machines via cloud storage, including across Linux and macOS
  (issue #293)
- Fixed: terminal starts at the real window size instead of 24x80 (issue #294)
- Fixed: keyboard shortcuts window shows remapped bindings instead of defaults,
  and lists the shortcuts that cannot be rebound (issue #295)
- Improved: rustconn-cli --backend accepts encrypted-file and portable
- Security: credential store temp files are created 0600 rather than chmod'ed
  after the write
- Localisation: new strings translated in all 17 locales; Georgian (ka) added
  (PR #296, Ekaterine Papava)

* Mon Aug 17 2026 Anton Isaiev <totoshko88@gmail.com> - 0.20.3-0
- Version bump to 0.20.3
- Added: keyboard passthrough state is now saved across restarts
- Improved: sidebar tooltips show full tree path for nested items

* Sat Aug 15 2026 Anton Isaiev <totoshko88@gmail.com> - 0.20.2-0
- Version bump to 0.20.2
- Added: toolbar hover/click setting (PR #286)
- Fixed: vault credential not copied on duplicate (PR #280)
- Fixed: child-exited handler stacking on reconnect (PR #283)

* Fri Aug 14 2026 Anton Isaiev <totoshko88@gmail.com> - 0.20.1-0
- Version bump to 0.20.1
- Added: Settings > Interface > Window > Show connection name in split panes — a
  compact colored header at the top of each split pane showing the connection
  name and protocol, off by default (#277)
- Fixed: group names are now unique per parent folder rather than globally, so
  hierarchies like "Site A/RDP" and "Site B/RDP" are possible and CSV imports no
  longer merge unrelated branches (#291)
- Fixed: a split pane could not be closed at all when its connection had the
  floating toolbar switched off — the panel's own detach and close buttons were
  suppressed along with the session toolbar (#260 follow-up)
- Fixed: RDP — xrdp hosts no longer stop at their own greeter after a successful
  NLA exchange; INFO_AUTOLOGON is now set when credentials are available (#290)
- Fixed: opening the connection dialog crashed the application when an SSH agent
  key had a non-ASCII comment — the comment was sliced at byte offsets and is now
  measured and cut in characters (#278)
- Fixed: vault credentials were not found for connections inside a group — saving
  used the hierarchical key and resolving searched only the flat one, so every
  connection in a folder prompted for a password (#289)
- Fixed: minimizing to tray silently destroyed port forwards, recordings and
  external viewers; the tray decision now runs before any teardown (#279)
- Fixed: a post-disconnect automation task froze the entire application for up to
  60 seconds — it now runs on a background thread (#281)
- Fixed: remote session recordings were lost when quitting the application; the
  retrieval now runs inline on shutdown, bounded by ConnectTimeout=5 (#282)
- Fixed: Web embedded — links requesting a new view (target="_blank",
  window.open) did nothing; the URI is now loaded in the same view (#288)
- Fixed: the network monitor reported a false outage on every Flatpak launch, and
  its debounce erased short down-up flaps so the recovery sweep never ran (#284)
- Fixed: a session closed with "exit" was silently reconnected on the next Wi-Fi
  roam — the sweep now requires explicit eligibility, not just a visible
  reconnect banner (#285)
- Fixed: Web embedded — the toolbar's reveal handle was unreachable in a split
  pane, buried under the split view's own panel arrow at the same corner; the
  handle is back at the top centre for every viewer
- Fixed: three strings new in this release reached only Ukrainian, one of them
  never extracted into the POT at all; all 16 catalogues are complete again
- Improved: scripts/release.sh now runs "typos", the last CI gate it did not, so
  a red Hygiene job cannot reach a tag
- Dependencies: cc 1.4.3, inotify 0.11.5, pkg-config 0.3.34, safe_arch 1.2.0 and
  the ICU/zerovec family at 2.3.0, all transitive

* Thu Aug 13 2026 Anton Isaiev <totoshko88@gmail.com> - 0.20.0-0
- Version bump to 0.20.0
- Added: opening a cluster now labels every member tab with a tab group named
  after the cluster, so it reads "[cluster] host"; Close All in Group on any
  member closes the whole cluster
- Added: signed build provenance for every .deb, .rpm, AppImage and Flatpak bundle
  attached to a GitHub release — verify with
  "gh attestation verify <file> --repo totoshko88/RustConn"
- Added: Web embedded mode — auto-hide floating toolbar with reveal zone
- Added: Settings > Interface > Rendering — Automatic, Hardware (GPU) or Software (Cairo) (#274)
- Fixed: an RDP, VNC, SPICE or Web member of a cluster was never registered in
  it, so "Disconnect all cluster sessions" could not close it
- Fixed: a tab returning from a split pane lost its "[group]" title label
- Fixed: the floating viewer toolbar was revealed but not clickable for its first
  two seconds in RDP, VNC and Web sessions
- Fixed: choosing a non-system interface language cost macOS users the tray icon —
  applying the language re-executed the process; the re-exec is gone (#158)
- Fixed: Web zoom shortcuts (Ctrl+/-/0) did not work in split view
- Fixed: Web toolbar clipped instead of collapsing in a narrow split panel; the
  collapse point is now measured rather than a fixed pixel breakpoint
- Fixed: the Web reveal handle sat over the page's top centre, and the floating
  toolbar ignored the local theme
- Fixed: a failed Web page load left the toolbar logic unrun, and the load timeout reported nothing
- Fixed: the header bar's busy spinner lost its accessible name
- Fixed: macOS inside a virtual machine had input lag and stuttering scroll — the
  automatic renderer choice now detects a hypervisor and selects Cairo (#274)
- Fixed: seven translatable strings were in the source but in no catalogue; now
  translated in all 16 locales
- Fixed: POTFILES.in did not list the two modules extracted from terminal/mod.rs
- Changed: the 60-second Web load timeout now reports itself in the reconnect banner
- Improved: neither the X11 Cairo fallback nor the language selection re-execs the
  process any more — both environment writes moved into the new rustconn-env-sys
  crate, so startup spawns two processes fewer
- Improved: terminal/mod.rs split into three modules — 4365 lines down to 3052
- Improved: adw::Spinner where the runtime has it (opt-in adw-1-6), gtk4::Spinner where it does not; baseline stays libadwaita 1.5
- Improved: build dependencies state the versions the crate features require (libadwaita >= 1.5, VTE >= 0.76)
- Improved: the three FFI crates now inherit the workspace clippy lint set; they
  had been the only crates in the workspace with no lints at all
- Documentation: how to verify a downloaded release artifact, in docs/INSTALL.md
  and SECURITY.md

* Wed Aug 12 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.22-0
- Version bump to 0.19.22
- Fixed: KeePassXC "Don't save" mode did not unlock the database on demand (#273)
- Improved: Sidebar search uses a cached search engine with result caching
- Improved: Keyboard group dropdowns show what "Automatic" actually sends (#271)

* Tue Aug 11 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.21-0
- Version bump to 0.19.21
- Fixed: RustConn did not start at all on Fedora 44 — the setlocale guard
  required exactly one live thread and aborted the process at startup (#271)
- Changed: the thread-count clause of the setlocale contract is documented as
  a judgement rather than a proof (#267)

* Tue Aug 11 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.20-0
- Added: SSH/MOSH connections can choose what Backspace and Delete send (#271)
- Fixed: Saving Preferences threw away the Backspace/Delete choice on a live session (#271)
- Fixed: RDM JSON import still aborted on an integer field, one bad entry cost the whole file (#234)
- Fixed: Royal TS Telnet sessions were imported as SSH on port 22 (#234)
- Fixed: KeePass database password was never written to the system keyring (#272)
- Fixed: Bitwarden, 1Password and Passbolt had the same keyring hole as KeePass (#272)
- Fixed: Moving a secret from encrypted file to system keyring destroyed it (#272)
- Changed: RUSTSEC-2026-0244 resolved via new rustconn-locale-sys crate (#267)
- Dependencies: gettext-rs 0.7.7→0.8.0

* Sun Aug 09 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.19-0
- Fixed: Deleting a connection now removes its credential from the password vault (#263)
- Fixed: Renaming a connection in the configuration panel now updates the vault entry (#263)
- Fixed: Credentials of connections outside any group were missed by vault cleanup and migration
- Fixed: System Keyring entries now visible in KDE Wallet, item label uses / instead of : (#264)
- Fixed: Moving a connection between groups no longer orphans its keyring entry (#264)
- Fixed: Deleting a group with a vault password no longer leaves a mangled orphan entry in KeePass
- Fixed: macOS Keychain credentials were stored where the resolver never looked
- Improved: Keyring credential retrieval wipes its intermediate plaintext buffers
- Changed: Saved interface language applied only during startup, before any thread exists (RUSTSEC-2026-0244)

* Sun Aug 09 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.18-0
- Fixed: Renaming a connection now updates the credential entry in the vault (#263)
- Fixed: System Keyring password collision for same-named connections in different groups (#264)
- Fixed: KeePass database password keyring entry now namespaced with RustConn prefix (#265)
- Changed: System Keyring credentials transparently migrated to new hierarchical key format

* Sat Aug 08 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.17-1
- Fixed: Settings panel no longer wipes in-memory KeePassXC password (#259)
- Fixed: OCI CLI version detection no longer shows Python tracebacks
- Improved: Settings dirty-tracking, async unlock/check, keyring saves, SSH agent error toast
- Improved: Import dialog async, error messaging, export open non-blocking

* Sat Aug 08 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.16-0
- Version bump to 0.19.16
- Fixed embedded RDP reading local clipboard on every desktop copy instead of only on paste (#261)
- Fixed Clipboard Off setting not gating toolbar Copy/Paste/Type Clipboard buttons (#261)

* Fri Aug 07 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.15-0
- Version bump to 0.19.15
- Fixed embedded RDP sessions in Automatic graphics mode showing a frozen desktop (#262)
- Fixed scrolling, window drags and solid fills silently discarded in GFX sessions (#262)
- Fixed sessions with no OpenH264 opening a GFX channel they could not paint through (#262)
- Fixed session status bar reporting a graphics pipeline the session was not using (#262)
- Fixed pressing Backspace in a session killing the whole window (VTE assertion failure)
- Fixed embedded RDP clipboard watcher leak causing coredumps (#261)
- Improved embedded RDP rendering performance
- Improved split view panel buttons auto-hide behind an arrow indicator

* Thu Aug 06 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.14-0
- Version bump to 0.19.14
- Added a Local Shell button to the empty split panel for starting a scratch shell there
- Fixed closing one pane taking the whole split layout apart
- Fixed session logs losing output and repeating whole screens; the transcript now comes from the session's own PTY (#247)
- Fixed opening Settings panel corrupted KeePassXC config (#259)
- Improved: one PTY path for every session on every platform (#175)

* Thu Aug 06 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.13-0
- Version bump to 0.19.13
- Added US Dvorak RDP keyboard layout (PR #258)
- Fixed RDM JSON import rejecting real-world Devolutions exports with numeric fields (#234)
- Improved session transcript initial capture timing (#247)
- Improved embedded RDP and VNC toolbar: auto-hide floating overlay

* Tue Aug 04 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.12-0
- Version bump to 0.19.12
- Added preservation of terminal history across a reconnect, with a dim rule separating the old output from the new session (#253)
- Added automatic login for Telnet and serial sessions, with configurable Username Prompt and Password Prompt fields inherited down the group chain (#254)
- Added Remove from Split (Ctrl+Shift+R) and Remove Split (Ctrl+Shift+J) so a session can leave a split view without being closed (#252)
- Fixed connect detection and prompt detection reading the oldest scrollback instead of the visible screen (#253)
- Fixed an expect-rule response being written to the application log in clear text
- Fixed "Move to New Tab" and "Close Connection" in a split pane's context menu doing nothing (#252)
- Fixed a collapsed split leaving its layout behind, which the next split then reused
- Fixed broadcast mirroring keystrokes out of a session that had left the split
- Fixed SFTP being unable to reach a host behind a jump host: mc's sh:// VFS now receives the connection's SSH options through a generated ssh_config behind a PATH wrapper (#255)
- Fixed a jump host picked from the connection list being invisible to every SFTP path; chain resolution moved into rustconn-core::connection::jump_chain
- Fixed rustconn-cli sftp refusing connections whose protocol is SFTP
- Fixed a pre-connect port check running against hosts reachable only through a hand-typed ProxyJump
- Fixed ${password} in an expect-rule response resolving to an empty string: the four built-in placeholders are now supplied from the connection at connection time, and substitution validates for a terminal instead of for a shell, so a password containing shell metacharacters survives (#257)
- Fixed a backslash inside a resolved expect-rule value being reinterpreted as an escape sequence
- Fixed a failed expect-rule substitution typing the literal placeholder into the session; the rule is now skipped with a warning
- Improved prompt watching by replacing three duplicated implementations with window::prompt_autofill::install_login_autofill
- Improved testability by moving prompt matching into rustconn-core::connection::login_prompt
- Improved the file-manager SFTP path, which now warns that it cannot route through a jump host instead of failing silently
- Updated dependencies: ipnet 2.12.0 to 2.12.1, libredox 0.1.18 to 0.1.19
- Documented Automatic Login for Telnet and Serial in the user guide

* Sun Aug 02 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.11-0
- Version bump to 0.19.11
- Fixed the Homebrew release automation patching only commented-out examples, which left the tap formula pinned to v0.19.6 since that release (#251)
- Fixed KeePassXC group passwords failing to load when the database is password-protected: both group dialogs now pass the stored master password (#250)
- Fixed the snap keyring error naming the exact snap connect command instead of implying the keyring is broken (#249)
- Improved embedded RDP CPU usage for background tabs: the polling loop now skips 15 of every 16 ticks when the drawing area is not mapped
- Improved connection and group listing with list_connections_owned and list_groups_owned, removing the intermediate Vec<&T> at 39 window call sites and in both persist paths
- Improved the readability of setup_ironrdp_polling by extracting its event handling into polling_handlers.rs behind two borrowed context structs
- Improved allocation behaviour in hot paths with with_capacity hints for trash, the group hierarchy index and sidebar filtering
- Improved responsiveness of emptying the trash by moving webkit session directory removal to a background thread
- Removed PackageManager, detect_package_manager and get_system_install_command from the rustconn-core re-exports
- Documented password-manager-service as a manual snap interface, which it has always been

* Sat Aug 01 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.10-0
- Version bump to 0.19.10
- Added "Session Logs..." to the primary menu and "Session Log..." to a connection's context menu, opening the directory that connection actually writes to (#247)
- Added expansion of a leading ~ in a log path template; a relative template now resolves inside the log directory from Settings (#247)
- Fixed session logging configured on a connection never writing anything: the Logs tab settings had no reader outside the dialog (#247)
- Fixed session logs recording typed passwords in clear text; every written line is now redacted (#247)
- Fixed "Retention (days)" never deleting a file: age-based pruning now also runs when a session log is opened (#247)
- Fixed the "Timestamps" switch doing nothing; it now controls the session transcript layout (#247)
- Fixed a log file that could not be created failing silently; the failure now raises an error toast naming the reason (#247)
- Fixed the KDBX "Use password" and "Use key file" switches only hiding rows without changing which credentials opened the database
- Fixed eight RDP and VNC dropdown labels being untranslated in every language
- Fixed RDP and VNC sessions freezing instead of reconnecting after the computer woke from sleep: both embedded clients now set TCP keepalive and TCP_USER_TIMEOUT (#248)
- Added resume detection so a session frozen by a suspend is dimmed and offered its reconnect banner immediately (#248)
- Fixed picking a key from the SSH agent not restricting which key was offered: the choice is now passed as -i with IdentitiesOnly=yes
- Changed an empty per-connection Log Path to mean the log directory from Settings
- Removed the Encrypted Documents feature, which was never reachable from the user interface
- Removed ProgressDialog, a second unused retry model, three unreachable menu actions and two configuration fields nothing read
- Updated time 0.3.54 to 0.3.55

* Sat Aug 01 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.9-0
- Version bump to 0.19.9
- Added a three-state RDP audio setting: play on this computer, on the remote computer, or not at all (#245)
- Fixed the RDP audio setting being ignored at connect time, which left every session with audio disabled (#245)
- Fixed custom FreeRDP arguments being dropped silently in embedded mode (#245)
- Fixed the disabled audio backend still advertising volume control (#245)
- Pinned the PulseAudio and ALSA backends in the bundled FreeRDP so it cannot ship without sound (#245)
- Fixed the snap package failing to start because WebKitGTK libraries were missing from the runtime (#244)
- Added a CI check that resolves every snap binary's shared libraries before publishing (#244)
- Fixed RDP through an RD Gateway resolving the target host locally and failing (#246)
- Fixed the RD Gateway tunnel sending an empty user name and a portless endpoint (#246)
- Fixed a failed gateway tunnel stranding the session instead of falling back to FreeRDP (#246)
- Fixed gateway connections to a target port other than 3389 going to the wrong port (#246)
- Fixed .rdp import reading the gateway port from a non-standard field (#246)
- Updated clap 4.6.4 to 4.6.5

* Fri Jul 31 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.8-0
- Version bump to 0.19.8
- Added a setting to open a new session on every double-click (#242)
- Fixed Remote Desktop Manager JSON import aborting on the first entry: ConnectionType is a numeric enum in real exports (#234)
- Fixed RDM import losing usernames, passwords and the folder hierarchy: Url/UserName/nested Credentials and Group paths are now read (#234)
- Fixed Royal TS import skipping every RDP connection: the object is RoyalRDSConnection with RDPPort (#234)
- Fixed Royal TS connections importing without a username: credentials assigned by name, typed inline, or inherited from the folder are now resolved (#234)
- Added support for compressed .rtsz containers and uncompressed .rtsx Royal TS documents (#234)
- Fixed XML entities corrupting imported Royal TS names (#234)
- Fixed Royal TS export writing RoyalRDPConnection, an element Royal TS cannot read
- Unsupported import entries are now reported as skipped with their source type (#234)
- Updated dependencies: hybrid-array 0.4.13→0.4.14, wide 1.5.0→1.6.0

* Thu Jul 30 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.7-0
- Version bump to 0.19.7
- Fixed double-click on a connection focusing a disconnected tab instead of connecting (#242)
- Fixed Enter in the connection sidebar doing nothing
- Implemented "Restore sessions on startup", which had no effect at all (#243)
- Fixed an unresolvable hostname aborting the connection instead of skipping the probe (#241)
- Added host-side resolution of mDNS .local names inside Flatpak (#241)
- Added ${password} to Custom Command templates, passed via environment, never via a command line (#151)
- Fixed Host, Port, Username and password not being editable for a Custom Command (#151)
- Fixed a password-only connection never caching its resolved credential

* Wed Jul 29 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.6-0
- Version bump to 0.19.6
- Fixed auxiliary keyring operations using Linux-only secret-tool on macOS
- Added local-only macOS CI, dependency, signing, and bundle audit workflow
- Improved the canonical macOS app and DMG to bundle and relocate 58 non-system dylibs
- Added explicit ad-hoc and Developer ID hardened-runtime signing plus notarization and stapling
- Expanded cargo-deny checks to aarch64 and x86_64 macOS targets
- Synchronized the canonical macOS feature profile across build and packaging paths
- Updated dependencies: displaydoc 0.2.6→0.2.7, toml 1.1.3→1.1.4

* Tue Jul 28 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.5-0
- Version bump to 0.19.5
- Fixed SSH session dying with "path too long for Unix domain socket" on long hostnames (issue #239)
- Fixed ${variable} placeholders never substituted in a Custom Command template (issue #151)
- Fixed Custom Command leaving a dead terminal tab behind after a one-shot launcher exits
- Fixed Tags field hidden for Custom Command and other Zero Trust connections
- Fixed in-place reconnect breaking a Custom Command line
- Fixed folders could not be nested by drag and drop in the sidebar (issue #237)
- Fixed multi-codepoint emoji icons (ZWJ sequences, flags, keycaps) saved but never drawn
- Improved fallback to a visible icon when the active theme lacks the stored icon name
- Improved localization: new string translated in all 16 languages
- Updated dependencies: socket2 0.5→0.6, aes, clap_complete, event-listener, toml_parser, tray-icon

* Mon Jul 27 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.4-0
- Version bump to 0.19.4
- Added detachable session windows (issue #236)
- Fixed RDP connection failing when the server only supports Standard RDP Security (issue #235)
- Fixed FreeRDP fallback broken on FreeRDP 3.24/3.25 due to the args-from file: prefix
- Fixed CredSSP logon failures triggering a pointless FreeRDP fallback
- Removed dead external_window module (ExternalWindowManager)
- Improved RDP failure classification moved into rustconn-core
- Improved single code path for building a session's tab content
- Improved localization: new strings translated in all 16 languages

* Wed Jul 23 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.3-0
- Version bump to 0.19.3
- Added option to hide Welcome tab at startup (issue #232)
- Fixed FreeRDP fallback fails on FreeRDP 3.26+ due to args-from exclusivity (issue #234)
- Fixed RDP clipboard syncing even when disabled in connection settings (issue #233)
- Fixed SSH MPTCP used non-existent TCPMultipath option (issue #231)

* Thu Jul 23 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.2-0
- Version bump to 0.19.2
- Added Multipath TCP (MPTCP) support for SSH, RDP, and VNC connections
- Fixed VPN connect/disconnect no longer kills healthy SSH sessions

* Mon Jul 21 2026 Anton Isaiev <totoshko88@gmail.com> - 0.19.1-0
- Version bump to 0.19.1
- Fixed RDP certificate mismatch causing silent connection failure (exit 255)
- Fixed SSH password auto-fill intermittently stuck on prompt

* Sat Jul 18 2026 Anton Isaiev <totoshko88@gmail.com> - 0.18.12-0
- Version bump to 0.18.12
- Added Graphics Pipeline selector for embedded RDP connections (issue #218)
- Fixed missing mouse cursor on remote Wayland VNC sessions (issue #220)

* Thu Jul 16 2026 Anton Isaiev <totoshko88@gmail.com> - 0.18.11-0
- Fixed: Missing mouse cursor on remote Wayland sessions over VNC (#220)
- Fixed: Embedded RDP retry without GFX before FreeRDP fallback (#218)
- Fixed: Nix flake tests no longer run during nix build
- Fixed: Nix flake duplicate CHANGELOG entry merged

* Thu Jul 16 2026 Anton Isaiev <totoshko88@gmail.com> - 0.18.10-0
- Added Nix flake for NixOS / Nix users
- Fixed: Embedded RDP to WinServer 2019 AD auth false fallback (#218)
- Fixed: FreeRDP fallback auth failure on single-session servers
- Dependencies: bitflags, clap, ksni, regex, sspi, uuid updated

* Wed Jul 15 2026 Anton Isaiev <totoshko88@gmail.com> - 0.18.9-0
- Security: vault password retrieval returns Zeroizing<String>
- Security: clipboard password wrapped in Zeroizing<String>
- Fixed: pre/post-connect tasks enforce 60s timeout ceiling
- Fixed: keyring save operations have 5s timeout
- Dependencies: FreeRDP 3.28.0 → 3.29.0

* Tue Jul 14 2026 Anton Isaiev <totoshko88@gmail.com> - 0.18.8-0
- Version bump to 0.18.8
- Fixed: Network interface change breaks connections (#217)
- Fixed: Terminal shortcuts after remapping (#216)
- Improved: Captive portal detection, rate limiting, embedded reconnect

* Sun Jul 13 2026 Anton Isaiev <totoshko88@gmail.com> - 0.18.7-0
- Version bump to 0.18.7
- Headless core-cli cleanup: rustconn-core defaults to empty features
- CLI is minimal by default; client-launch and secret-management are opt-in
- System keyring (oo7/macOS Keychain) gated behind system-keyring feature
- NO_COLOR env var handling follows the no-color.org spec
- Updated lockfile: cc 1.2.67, http-body 1.1.0, mio 1.2.2, open 5.4.0, rand 0.9.5, rustls 0.23.42, uuid 1.23.5

* Thu Jul 09 2026 Anton Isaiev <totoshko88@gmail.com> - 0.18.4-0
- Version bump to 0.18.4
- Added SFTP file browser now opens in the login home directory instead of the server root, with an optional SFTP Remote Directory field to pin a path (#212)
- Fixed external viewers (TigerVNC, FreeRDP, remote-viewer) left as zombies when shutdown() ran without the process exiting; they are now reaped
- Improved sidebar bottom toolbar to match the header bar's icon size and spacing
- Improved SFTP home-directory resolution: failed probes are cached, a per-connection SSH agent socket is honoured, and keepalives stop a stalled probe pinning the worker
- Dependency updates: libadwaita 0.9.1 -> 0.9.2 (pin lifted), plus der, inotify, regex, regex-automata, zerocopy, zerocopy-derive, zlib-rs patch bumps

* Thu Jul 09 2026 Anton Isaiev <totoshko88@gmail.com> - 0.18.3-0
- Version bump to 0.18.3
- Added external-session tracking for VNC/RDP/SPICE external-viewer sessions: process registry + shared poll timer, sidebar external-viewer emblem, Disconnect / Stop tracking context menu, split-membership marker, smart double-click
- Fixed VNC/RDP/SPICE External Window mode leaving a dead notebook tab; the session is surfaced in the sidebar instead (#209)
- Fixed Telnet connections stuck on the Vault password source (#210)
- Improved orthogonal (shape + icon, color-independent) sidebar state indicators
- Dependency updates: bytes 1.12.1, memchr 2.8.3

* Wed Jul 08 2026 Anton Isaiev <totoshko88@gmail.com> - 0.18.2-0
- Version bump to 0.18.2
- SPICE unix socket connections (spice+unix://) with Browse button
- Compact interface: primary-menu toggle + Ctrl+Shift+D shortcut, automatic mode on small windows
- Compact mode extended to monitoring bar, split panels, and playback toolbar
- Fixed SPICE CA-certificate Browse button (was wired to no handler)
- Fixed CLI/GUI SPICE viewer USB-redirection flag divergence
- Fixed compact mode inflating the header/banner area
- FreeRDP (Flatpak) updated 3.27.1 -> 3.28.0
- Dependency updates: zbus 5.17, zvariant 5.13

* Tue Jul 07 2026 Anton Isaiev <totoshko88@gmail.com> - 0.18.1-0
- Version bump to 0.18.1
- Split view generalized to embedded RDP/VNC/SPICE tabs
- Embedded viewers adapt toolbar/resolution to narrow panels
- Fixed split-owner tab close stranding guest sessions
- Fixed embedded RDP small-window scaling, blank-after-unsplit, and resize loop
- Fixed clicking on embedded panel in split not passing mouse events
- Fixed SSH ProxyCommand parallel connections failing with "Permission denied"
- Fixed workspace restore skipping RDP/VNC/SPICE connections
- Fixed workspace split restore for async connections, Local Shell, multi-panel layouts
- Updated dependencies: cc, crossbeam-*, inotify, jobserver, lzma-rust2, num-bigint, zerocopy

* Sun Jul 05 2026 Anton Isaiev <totoshko88@gmail.com> - 0.18.0-0
- Added a Native (full HiDPI) Display Scale option for embedded RDP/VNC — a "retina" mode that follows the live display scale, alongside Auto and 125–400% (#207)
- Fixed embedded VNC showing noise on Tight/JPEG rectangles — the client now decodes JPEG to BGRA and re-enables the Tight encoding
- Fixed embedded VNC leaving stale regions after a server-side scroll or window move — CopyRect is mirrored into the Cairo-backed buffer
- Fixed the RDP display scale being lost on dynamic resize, and the HiDPI cursor rendering half-missing and mis-sized (#207)
- Fixed several UI strings never being translated because their \u{…} escapes leaked into the message catalog; typographic strings now localise correctly
- Changed operation-result feedback to non-blocking toasts, and a secret-save failure to a blocking dialog to prevent silent credential loss
- Improved performance — fewer allocations in search results, removed redundant per-frame VNC/RDP buffer copies, no per-keystroke connection cloning
- Removed the abandoned native embedded SPICE experiment, an unused KeePassXC browser-integration backend, a parallel tracing-init subsystem, and dead render buffers / Wayland placeholders
- Updated dependencies — cpal 0.18, muda 0.19, tray-icon 0.24

* Sat Jul 04 2026 Anton Isaiev <totoshko88@gmail.com> - 0.17.10-0
- Fixed embedded RDP/VNC requesting a scale-inflated resolution on HiDPI displays — the remote desktop is now requested at the widget's logical size instead of device pixels, cutting HiDPI bandwidth roughly 4× at 2× scale; explicit Display Scale values still request a higher resolution for sharpness
- Fixed embedded SPICE showing a black, unresponsive screen by default — spice-embedded is no longer a default feature; SPICE uses the external viewer (opt in with --features spice-embedded)
- Fixed embedded VNC rendering garbage against TightVNC/TigerVNC — the Tight encoding is removed from the defaults; ZRLE, CopyRect and Raw remain
- Fixed VNC input momentarily stalling the UI — commands are now sent non-blocking on the GTK main thread
- Improved embedded RDP frame handling — removed a redundant per-frame buffer copy on the IronRDP path (~33 MB per frame at 4K)
- Improved sidebar search — result lookup is now O(1) instead of O(n) per hit
- Removed a dead multi-monitor detection placeholder; wrapped remaining user-facing strings for translation; dropped the unused futures dependency
* Fri Jul 03 2026 Anton Isaiev <totoshko88@gmail.com> - 0.17.9-0
- Fixed SSH multi-hop password chains still crashing when a bastion's host key is unknown — every hop authenticating via forced SSH_ASKPASS now sets StrictHostKeyChecking=accept-new, so a first-seen host-key prompt is no longer routed to the password helper; a changed key is still rejected, preserving MITM protection (#203)
- Fixed the embedded RDP first frame being blurry and never sharpening — the desktop is re-requested at the drawing area's real size over MS-RDPEDISP once layout settles, so the first real frame arrives at a 1:1 pixel map (#206)
- Fixed the embedded RDP initial snap causing a visible connect to reconnect flicker (and occasional connection resets) — the snap now runs only when the DisplayControlReady event confirms the channel is negotiated and resizes smoothly over MS-RDPEDISP; it is never forced on a timer, so a slow server no longer triggers a full reconnect. If the server never negotiates Display Control the frame is simply scaled to fit (#206)
- Fixed a stale seam/line left on the embedded RDP screen after a resolution change — the client now sends a full-desktop Refresh Rect PDU on every resize so the server repaints the whole screen instead of leaving an untouched strip with its old fill (#206)
- Fixed over-conservative embedded RDP resolution rounding — dimensions are rounded to the minimum the protocol requires (both forced even) and clamped to 7680x4320, so a resize on a >4K display is no longer silently rejected

* Fri Jul 03 2026 Anton Isaiev <totoshko88@gmail.com> - 0.17.8-0
- Fixed nested groups losing their parent when importing an Asbru-CM config with three or more group levels — the topological group sort now tracks already-sorted IDs in a dedicated set instead of a map populated later, so arbitrarily deep hierarchies are preserved deterministically on every import (#205)
- Fixed a too-large minimum window width in some locales (notably German) that prevented tiling or resizing the window narrow — the runtime width measurement now collapses the sidebar first, as the narrow layout tier does, so localized sidebar labels no longer inflate the minimum (#204)
- Fixed the welcome-screen hint holding the window wider in verbose locales — the hint now wraps instead of reporting its full translated width (#204)
- Dependencies: updated arrayvec 0.7.7→0.7.8, num-bigint 0.4.6→0.4.7

* Thu Jul 02 2026 Anton Isaiev <totoshko88@gmail.com> - 0.17.7-0
- Fixed narrow-window layout — window controls no longer vanish when narrow, the window shrinks smoothly to a measured minimum, non-essential header buttons are shed and the sidebar auto-hides, and the welcome screen reflows its columns instead of wrapping shortcut labels character-by-character (#204)
- Fixed SSH multi-hop password chain — only one bastion received a password; entry bastions in nested ProxyCommand now get their own per-hop SSH_ASKPASS helper with indexed env vars (#203)
- Fixed Snap build failing on Launchpad with Snapcraft 9.0 — switched from plugin: rust to plugin: nil with an explicit rustup install in the rust-deps part
- Security: updated quick-xml 0.39.4→0.41.0 to close RUSTSEC-2026-0194 and RUSTSEC-2026-0195 — crafted XML could trigger unbounded allocation via namespace/attribute flooding on the RoyalTS/libvirt import paths
- Dependencies: updated inotify-sys 0.1.6→0.1.7, rand 0.10.1→0.10.2

* Wed Jul 01 2026 Anton Isaiev <totoshko88@gmail.com> - 0.17.6-0
- Fixed RDP sessions still aborting on resize — RustConn no longer requests bulk (MPPC/NCRUSH/XCRUSH) compression at all, matching the ironrdp-client default, so the server never sends compressed FastPath updates that desynchronised across a Deactivation-Reactivation Sequence (#200)
- Fixed Saving Preferences wiping persisted window/sidebar state — window size/maximized, expanded sidebar groups, and search history are now preserved instead of reset to defaults (#202)
- Fixed credentials being silently lost when the system keyring is unavailable — RustConn probes real backend availability, raises an actionable dialog pointing to Settings → Secrets, and falls back to the new encrypted-file store (#201)
- Added: window now remembers its maximized state across restarts (#202)
- Added an application-managed encrypted-file secret backend (AES-256-GCM with an Argon2id-derived key) for hosts without a working system keyring
- Added proactive keyring availability surfacing at startup and in Settings → Secrets
- Changed: the system keyring path migrated to the in-process oo7 client on Linux/BSD, removing the bundled libsecret/secret-tool from the Flatpak manifests; macOS keeps using the system Keychain
- Dependencies: updated aws-lc-rs, aws-lc-sys, clap_complete, inotify-sys, libredox, rustls-pki-types, time, and zlib-rs to latest compatible releases

* Tue Jun 30 2026 Anton Isaiev <totoshko88@gmail.com> - 0.17.5-0
- Fixed embedded RDP sessions aborting when resized — the reactivation handler now rebuilds a fresh decompressor for the negotiated compression type (RDP4/5/6/6.1), so compressed FastPath updates keep decoding after a Deactivation-Reactivation Sequence (#200)
- Fixed RDP to GNOME Remote Desktop dead-ending — the protocol-incompatibility detector now matches the actual error wrapper and the IronRDP "invalid state" bug signature, so these servers transparently fall back to the external FreeRDP client (#199)
- Fixed the native arm64 snap link step — the build now prepends the SDK arch-triplet lib dir to the rustc link search path so both halves of pango resolve from the SDK (#198)
- Fixed Variable password auto-login on network equipment — detection now reads the line under the cursor and matches via a pure looks_like_password_prompt helper, with an idle re-check for the prompt-render race (#194)
- Fixed jump host authenticating with its own Variable/Vault password, delivered out-of-band via SSH_ASKPASS so the target password never leaks to the bastion prompt (#191)
- Added a "Send terminal control shortcuts to the session" setting so readline chords (Ctrl+F/P/N/W/H/M/I) reach the focused terminal or embedded RDP/VNC/SPICE viewer instead of the app accelerators (#197)
- Added a native arm64 (aarch64) snap build alongside amd64
- Changed: refreshed the gtk4-rs stack and other Cargo dependencies to their latest compatible patch releases; dropped the unused pathdiff

* Fri Jun 27 2026 Anton Isaiev <totoshko88@gmail.com> - 0.17.4-0
- Fixed RDP vault login sending the correct domain — when credentials came from the secret vault, the domain field was passed as an empty string instead of the configured value, causing NLA/CredSSP to reject DOMAIN\user logins with STATUS_LOGON_FAILURE; the vault path now falls back to the connection's saved domain (#188)
- Fixed Variable password auto-login on network equipment — the password auto-fill relied solely on VTE's contents-changed signal, which does not fire reliably for SSH password prompts in no-echo mode with cursor-positioning escapes; detection now also subscribes to cursor-moved (#194)
- Changed: CUPS printer redirection forwards all local queues — the embedded IronRDP printer channel previously announced a single dummy "RustConn" printer; it now enumerates all local CUPS queues (or a configured subset via with_printers) and registers each as its own redirected printer, routing print jobs back to the correct local queue (#192)

* Fri Jun 26 2026 Anton Isaiev <totoshko88@gmail.com> - 0.17.3-0
- Fixed switching GNOME workspaces with Super+digit breaking RDP keyboard input — the compositor grabbed the Super chord before its key-release reached the widget, so the embedded session treated Super as stuck down until a reconnect; held keys are now released when the widget loses focus (#193)
- Fixed RD Gateway connections with FreeRDP 3.x — the launcher emitted the removed 2.x aliases /g: /gu: /gp:, which 3.x rejects; it now builds the unified /gateway:g:HOST:PORT option and reuses the session credentials (#187)
- Fixed multi-hop (double) jump hosts in Flatpak — each hop now gets its own nested ProxyCommand so it inherits the identity key and Flatpak known_hosts (terminal SSH, RDP/VNC/SPICE tunnels, monitoring probe) (#191 follow-up)
- Fixed multi-hop jump host order outside Flatpak — the -J hop list is now reversed to match OpenSSH's client-first order, so chains of three or more bastions connect; single-bastion connections are unaffected
- Added RDP printer redirection: a "Printer Redirection" toggle maps the local printer into the session (issue #192). The embedded IronRDP client announces a virtual PostScript printer over RDPDR and forwards print jobs to the local CUPS spooler (lp) off the session thread; the external xfreerdp3 client passes /printer. Available in the GUI, the CLI (--printer), and imported from .rdp files (redirectprinters)
- Changed: external RDP now prefers the maintained SDL3 client (sdl-freerdp3) for RD Gateway, RemoteApp, and IronRDP fallback launches; embedded mode still uses wlfreerdp where present

* Thu Jun 25 2026 Anton Isaiev <totoshko88@gmail.com> - 0.17.2-0
- Security: the password generator's Copy now auto-clears the clipboard after 30 seconds (only if it still holds that password)
- Security: the auto-login SSH password is wrapped in Zeroizing so plaintext is wiped right after it is handed to VTE
- Security: fixed an SSH tunnel askpass file race by adding a per-tunnel UUID to the helper script filename
- Fixed SSH jump host authenticating with the target's password instead of its own; the bastion now uses its own saved password (#191)
- Fixed RDP dynamic resize requesting sub-640x480 desktops; RD Gateway password now sent (/gp:) for same-account gateways via a single-use 0600 args file; shared-folder names with commas no longer corrupt drive redirection; multilingual SSH password-prompt detection on reconnect
- Added Simple Sync — bidirectional multi-device sync of connections, groups (full create/update/delete), templates, snippets, and non-secret variables via full-sync.rcn with UUID merge and deletion tombstones
- Added SSH config import following Include directives (globs, ~/.ssh relative paths, 16-level recursion cap, each file parsed once)
- Changed: tab groups persist in workspaces; jump-host / SSH-args resolution deduplicated; dynamic-connection IDs now use stable UUID v5
- Updated uuid 1.23.3->1.23.4 and the wasm-bindgen/js-sys/web-sys ecosystem

* Wed Jun 24 2026 Anton Isaiev <totoshko88@gmail.com> - 0.17.1-0
- Added a WinBox connection preset (Remote Desktop category) for MikroTik RouterOS management (#190)
- Added native PKCS#11 / YubiKey SSH authentication via a "PKCS#11 Provider" field, also imported from ~/.ssh/config and injected into ProxyJump hops (#189)
- Fixed Flatpak Generic commands failing with "Failed to start command: script" on hosts without a reachable script binary, and GUI tools (e.g. WinBox) not launching (#190)
- Internal: removed the dead VNC FFI stub, archived-spec traceability comments, and unused performance scaffolding (~1900 LOC)
- Updated chacha20 0.10.0->0.10.1

* Tue Jun 23 2026 Anton Isaiev <totoshko88@gmail.com> - 0.17.0-0
- Packaging (openSUSE): pin the cargo linker to gcc in %prep to fix "linker `clang` not found" — the bare /usr/bin/clang symlink is not reliably present in the OBS build root
- Security: kubectl and Zero Trust Generic sessions now spawn argv directly instead of via sh -c, preventing shell-metacharacter command injection from imported/untrusted configs
- Security: removed the obsolete legacy XOR credential fallback (only AES-256-GCM credentials are read)
- Security: documented the machine-key threat model and the Passbolt passphrase Known Issue in SECURITY.md
- Security: wrap transient Bitwarden/KeePassXC serialized buffers in Zeroizing
- Added workspace split-layout restore on Open
- Improved terminal highlight rendering (pre-parsed colours, allocation-free hot path) and connection sort (cached lowercase keys)
- Converted the embedded-RDP autotype dialog to adw::Dialog; header-bar icon buttons now meet the 44x44 minimum tap target
- Build: narrowed the tokio feature set from "full" to the features used
- Updated rustls 0.23.40->0.23.41

* Mon Jun 22 2026 Anton Isaiev <totoshko88@gmail.com> - 0.16.13-0
- Added RDP round-trip time (latency) display — embedded IronRDP sessions show RTT in the toolbar when the server reports network characteristics via the Auto-Detect PDU; the Echo virtual channel is also registered
- Dynamic RDP resolution change now works in embedded mode — the Display Control channel is now registered, so window resizes no longer force a full reconnect on servers that support it
- Updated ironrdp 0.15->0.16 (session, dvc, displaycontrol, echo) plus minor crates (quote, time, zlib-rs)

* Sun Jun 21 2026 Anton Isaiev <totoshko88@gmail.com> - 0.16.12-0
- Added workspace profiles — save and restore named sets of connections
- Added built-in port knocking — TCP/UDP knock sequences before connecting
- Added fwknop SPA configuration model
- Integrated port knock into the pre-connect chain
- Added port knock sequence field in Advanced connection editor

* Sat Jun 20 2026 Anton Isaiev <totoshko88@gmail.com> - 0.16.11-0
- Fixed the connection wizard's "Zero Trust" card showing only a custom-command field instead of the provider list — it now restores the full provider picker (AWS SSM, GCP IAP, Azure, Cloudflare, Teleport, Tailscale, Boundary, Hoop.dev) and defaults to AWS Session Manager, like the Advanced editor
- Fixed the RDP Mouse Jiggler never actually running in Embedded (IronRDP) mode (#185) — the timer was only ever armed from set_state, which embedded connections bypass, so neither the mouse-move nor the Scroll Lock keep-alive was ever sent; it is now armed directly from the embedded connection events
- Fixed External RDP (sdl-freerdp) ignoring its sdl-freerdp.json in the Flatpak build (#183) — the bundled FreeRDP had no JSON backend, so WinPR silently discarded the config and its SDL hotkeys could not be remapped; a static cJSON module is now built ahead of FreeRDP
- Fixed RDP connections created through the New Connection wizard never storing the typed password (#188) — they were created with no usable credential, causing an immediate NLA authentication failure; the wizard now persists the password to the vault, mirroring the full editor
- Fixed RDP through an RD Gateway rendering a broken/black session in embedded mode (#187) — gateway connections now go directly to the external client, which wires up gateway routing, instead of falling through to embedded wlfreerdp which never emits the gateway arguments
- The Advanced connection editor now has a distinct "New Connection (Advanced)" window title through every entry point, including the wizard's Advanced hand-off
- Updated bundled dependencies — cJSON (Flatpak) 1.7.18->1.7.19, arrayvec 0.7.6->0.7.7

* Fri Jun 19 2026 Anton Isaiev <totoshko88@gmail.com> - 0.16.10-0
- Fixed RDP Mouse Jiggler not preventing the remote desktop from locking (#185) — in Embedded (IronRDP) mode the jiggler only moved the mouse, which keeps the session alive but does not reset the Windows workstation lock timer; each tick now also taps Scroll Lock (a no-op, state-preserving keystroke) so unattended desktops stay unlocked
- Documented that the Mouse Jiggler works in Embedded mode only (the External FreeRDP client has no input channel from RustConn)
- Updated cc 1.2.64->1.2.65
- FreeRDP (Flatpak) 3.27.0->3.27.1

* Fri Jun 19 2026 Anton Isaiev <totoshko88@gmail.com> - 0.16.9-0
- Fixed RDP Quick Actions and shell launchers typing wrong characters on non-QWERTY remote keyboard layouts (#184) — Run-dialog commands and the PowerShell/CMD launchers are now sent via layout-independent Unicode keyboard events instead of hard-coded US-QWERTY scancodes
- Internal cleanup — removed the dead ad-hoc broadcast controller, the unused virtual-scroll tuning API and protocol-layout builder setters, and stale dead_code overrides; corrected stale doc comments
- Updated bitvec 1.0.1->1.1.1

* Thu Jun 18 2026 Anton Isaiev <totoshko88@gmail.com> - 0.16.8-0
- Fixed KeePassXC not being detected in Flatpak ("keepassxc-cli not found") (#182) — detection and all KDBX operations now resolve and run the host binary via flatpak-spawn --host, so the host's KeePassXC is found and piped database/entry passwords reach it
- Fixed KDBX status text overflowing the row (#182) — the status label now ellipsizes at a capped width and shows the full text as a tooltip on hover
- Fixed the Settings → Interface theme segmented control not reflecting the saved colour scheme on libadwaita >= 1.7 builds — the toggle group is now held in its wrapper box and the loader sets the active segment from the saved scheme
- Documented the external FreeRDP SDL client keyboard shortcuts (Right Shift hotkeys) and where to place sdl-freerdp.json for the Flatpak build (#183)
- Updated dependencies: ironrdp-graphics 0.8.0->0.8.1, ironrdp-rdpsnd 0.8.0->0.8.1, bytes 1.11.1->1.12.0, crypto-bigint 0.7.3->0.7.4, getrandom 0.4.2->0.4.3, syn 2.0.117->2.0.118

* Tue Jun 16 2026 Anton Isaiev <totoshko88@gmail.com> - 0.16.7-0
- Fixed sidebar right-click menu rows rendering invisible under KDE / the Breeze GTK theme (#181) — the custom popover now pins its colours to the libadwaita popover palette so the menu rows stay legible under any GTK theme
- Fixed the smart-folder right-click menus (folder actions and connections inside a smart folder) sharing the same invisible-rows defect — they now reuse the same popover styling; the destructive Delete keeps its red accent
- Updated bundled Flatpak components: FreeRDP 3.26.0->3.27.0, fast_float 8.0.2->8.2.10

* Mon Jun 15 2026 Anton Isaiev <totoshko88@gmail.com> - 0.16.6-0
- Fixed Activity Monitor doing nothing on most connections and the per-tab "Monitor" menu stuck on Off (#180) — monitoring is wired from a single session-creation choke point, covering every terminal protocol and connect path (sidebar, command palette, cluster) and both synchronous and port-checked connections; sessions register even when Off so the tab menu can cycle Off -> Activity -> Silence live, and in-place reconnect re-arms monitoring
- Fixed silence notification reporting the wrong connection name with several monitored tabs open — name is now resolved per session
- Updated h2 0.4.14->0.4.15

* Sun Jun 14 2026 Anton Isaiev <totoshko88@gmail.com> - 0.16.5-0
- Fixed external FreeRDP client closing immediately with no explanation (#177 follow-up) — stderr captured and forwarded to the log; startup watchdog (~3s) detects an immediate exit
- Fixed wrong default secret backend on macOS — fresh install defaults to the system Keychain instead of the unavailable libsecret
- Fixed misleading "not installed" message for any terminal spawn failure on macOS — only a genuine missing executable uses that wording
- Skipped the Linux dark-theme workaround on macOS, where it fought the system NSAppearance
- Suppressed the harmless CSS theme-parser warning flood (libadwaita >= 1.9 vs GTK4 parser)
- macOS Keychain: secret bytes zeroized on a UTF-8 decode failure
- Updated yuv 0.8.15->0.8.16

* Sun Jun 14 2026 Anton Isaiev <totoshko88@gmail.com> - 0.16.4-0
- Fixed MobaXterm import/export losing nested folder structure (#178) — SubRep paths rebuild the full folder tree and round-trip correctly
- Fixed SecureCRT export mangling folders nested 3+ levels deep (#178) — paths built by walking the parent chain; empty intermediate folders preserved
- Fixed Asbru-CM export dropping parent links on deep hierarchies (#178) — group UUID map built up front so nesting survives any group order
- Updated time 0.3.47->0.3.49, time-core 0.1.8->0.1.9, time-macros 0.2.27->0.2.29

* Sat Jun 13 2026 Anton Isaiev <totoshko88@gmail.com> - 0.16.3-0
- Added RDP "Fit resolution to window" toolbar button — re-requests session resolution to match the window
- Added error details in Connection History for failed connections (the toast is transient)
- Fixed failed connections missing from history — port-check timeouts now recorded for all protocols (SSH, RDP, VNC, SPICE, Telnet, MOSH, SFTP)
- Fixed RDP connects but the desktop never appears (#177) — first-frame watchdog falls back to external FreeRDP for GFX/H.264-only servers
- Updated openssl 0.10.80->0.10.81, zeroize 1.8.2->1.9.0, wasm-bindgen 0.2.123->0.2.125

* Sat Jun 13 2026 Anton Isaiev <totoshko88@gmail.com> - 0.16.2-0
- GNOME HIG follow-up audit — critical errors as blocking alert dialogs, destructive button styling removed, DnD indicators use accent color
- Dead code cleanup — removed unused ContainerState, is_split/is_welcome, load_variable_from_vault, stale #[allow(dead_code)]
- Fixed F10 opening application menu in keyboard passthrough mode — primary flag dropped during passthrough
- Fixed Ctrl+T (SSH Tunnel Manager) ignoring passthrough and not customizable — now a regular keybinding
- Updated block-buffer 0.12.0->0.12.1, cc 1.2.63->1.2.64, memchr 2.8.1->2.8.2, smallvec 1.15.1->1.15.2, yuv 0.8.14->0.8.15

* Fri Jun 12 2026 Anton Isaiev <totoshko88@gmail.com> - 0.16.1-0
- Settings dialog GNOME HIG pass — secret fields to PasswordEntryRow, highlight-rules editor rebuilt, rows activatable, Reset All confirmation, restore dialog, backup/restore failures shown
- Fixed Settings dialog taking 5+ seconds to appear with Bitwarden backend (flatpak)
- Fixed UI froze for seconds after startup with Bitwarden (flatpak)
- Fixed sidebar context menu dismissed on deeply nested rows (KDE Plasma, #157)

* Thu Jun 11 2026 Anton Isaiev <totoshko88@gmail.com> - 0.16.0-0
- Added batch edit for multi-selected connections — change group, tags, or icon in one pass with undo
- Added notes badge in the sidebar — connections with description show document-edit-symbolic badge
- Added search matches connection notes (weight below name/host/tags)
- Added Windows (WSL2) guide — step-by-step setup in docs/WSL.md (#137)
- Structured validation errors in core — ValidationError enum with thiserror
- RDP catch_unwind wrapper kept for 0.16 (ironrdp 0.15 still may panic on malformed PDUs)
- Fixed sidebar context menu failing to open on KDE Plasma (#157)
- Internal: dialogs/connection/dialog.rs split into 7 submodules
- Security: RDP/SPICE client Debug output leak regression-tested
- New property tests for shell_escape and smart_folder

* Thu Jun 11 2026 Anton Isaiev <totoshko88@gmail.com> - 0.15.14-0
- External RDP/VNC sessions no longer freeze the window for 1.5s on connect — non-blocking spawn with 250ms poll
- Tray messages are now event-driven instead of polled (async-channel)
- Secret backend detection in Settings is parallel and cached (30s)
- Connection history writes are debounced (2s) and off the main thread
- One suggested action per dialog (GNOME HIG)
- Added async-channel 2.5.0
- Updated crypto-primes 0.7.1 -> 0.7.2

* Wed Jun 10 2026 Anton Isaiev <totoshko88@gmail.com> - 0.15.13-0
- Added Menu key / Shift+F10 to open the sidebar context menu for the selected row — keyboard fallback where right-click on nested rows is unreliable (#157)
- Added confirmation before closing with open session tabs (window close button and Ctrl+Q share one dialog; skipped with minimize-to-tray)
- Added a recording indicator (red dot) in the sidebar while any session of a connection is being recorded
- Added import duplicate handling — Cancel / Skip Duplicates / Import All instead of silently creating renamed copies
- Added a persistent cloud-sync failure banner (manual Sync Now and background auto-export); transient toasts kept for success only
- Added touch long-press to open the sidebar context menu
- Improved context-menu keyboard navigation and screen-reader roles, error message wording (GNOME HIG), and 44x44 px sidebar tap targets
- Fixed sidebar context menu not opening for rows nested deeper than the root level on some systems (KDE Plasma) (#157)
- Fixed crash (SIGSEGV) in pango when opening a new SSH tab or on screen unlock — refresh VTE fonts on fontconfig change, defer child-exited work to idle (#171)
- Updated crypto-primes 0.7.0 -> 0.7.1, ksni 0.3.4 -> 0.3.5, regex 1.12.3 -> 1.12.4, zerocopy 0.8.50 -> 0.8.52

* Tue Jun 09 2026 Anton Isaiev <totoshko88@gmail.com> - 0.15.12-0
- Fixed macOS SSH password authentication always failing with "Permission denied" — native PTY child now claims the slave PTY as its controlling terminal (setsid + TIOCSCTTY) (#175)
- Added rustconn-pty-sys crate to isolate the controlling-terminal FFI (pre_exec) so the main crates keep unsafe_code = "forbid"
- Updated uuid 1.23.2 -> 1.23.3 and wasm-bindgen 0.2.122 -> 0.2.123 (with related js-sys, web-sys, wasm-bindgen-futures)

* Sun Jun 07 2026 Anton Isaiev <totoshko88@gmail.com> - 0.15.11-0
- Fixed keybinding recorder still not registering keystrokes on Flatpak — dedicated modal capture dialog (#170, #167)
- Fixed keybinding overrides not displayed in Settings after reopening or restart (#170)
- Fixed Snap package still failing to start on Ubuntu 26.04 — core24 base with gnome extension (#174)
- Changed keybinding recorder to store shortcuts in a layout-independent (Latin) form (#170)
- Changed Snap base core26 -> core24 (GNOME 46 / libadwaita 1.5, without adw-1-8); Snap CI installs Snapcraft from latest/stable

* Thu Jun 05 2026 Anton Isaiev <totoshko88@gmail.com> - 0.15.10-0
- Fixed keybinding overrides not displayed correctly after reopening Settings (#170)
- Fixed keybinding conflict detection ignoring modifier order
- Fixed Snap package failing to start on Ubuntu 26.04 (AppArmor error) (#174)
- Updated bitflags 2.12.1 -> 2.13.0

* Thu Jun 05 2026 Anton Isaiev <totoshko88@gmail.com> - 0.15.9-0
- Lazy init secret backends — only preferred backend initialized at startup
- KeePass keyring failure toast at startup when password not loaded
- Connection wizard: ComboRow model lazy init eliminates auth method flash
- Fixed KeePass vault credentials not resolved on Flatpak after restart (#170)
- Fixed crash (SIGSEGV) when opening new SSH tab or on screen lock/unlock (#171)
- Fixed connection wizard auth method label overflow — ComboRow (#169)
- Fixed Telnet connection not closed when closing the tab (#172)
- Fixed 1Password/Passbolt credentials not passed in vault entry lookups
- Fixed connection wizard redundant Method dropdown for non-SSH protocols

* Wed Jun 04 2026 Anton Isaiev <totoshko88@gmail.com> - 0.15.8-0
- Fixed keybinding reassignment not registering keystrokes — recorder
  moves focus to parent ActionRow and disables PreferencesDialog search
- Fixed sidebar right-click context menu not appearing for hosts in groups
- Fixed secret variable with vault entry name writing duplicate to vault
- Fixed sidebar status icon size inconsistent with custom icons
- Variable dialog: vault entry UX hints — placeholder and tooltip
- Updated: chrono 0.4.44→0.4.45, log 0.4.31→0.4.32, yoke 0.8.2→0.8.3

* Tue Jun 03 2026 Anton Isaiev <totoshko88@gmail.com> - 0.15.7-0
- Variable password source: discoverability — subtitle hint and "+"
  button to open Variables manager directly from connection dialog
- Variable password source: custom vault entry name — reference existing
  vault entries by name instead of default rustconn/var/{name} key
- Fixed Proxmox SPICE inline PEM CA certificate now saved automatically
  on import from .vv file
- Fixed keybinding reassignment not working — accelerators suspended
  during recording, EventControllerKey uses Capture phase

* Mon Jun 02 2026 Anton Isaiev <totoshko88@gmail.com> - 0.15.6-0
- VNC: new "Accept Certificate" toggle for VeNCrypt servers with
  self-signed TLS certificates; CLI --ignore-certificate for VNC
- Fixed Welcome screen "Remote host monitoring" icon missing on macOS
- Fixed sidebar right-click context menu not opening for nested items
- Updated: bitflags 2.11.1→2.12.1, log 0.4.30→0.4.31,
  lzma-rust2 0.16.2→0.16.4; removed sha2 0.10.9
- Flathub: inetutils 2.7→2.8

* Mon Jun 01 2026 Anton Isaiev <totoshko88@gmail.com> - 0.15.5-0
- IronRDP 0.14→0.15: bulk compression, slow-path rendering, RemoteApp
  alternate_shell, multitransport dispatch, pixel format fix
- macOS: fixed passwords not saving to Keychain (wrong backend dispatch)
- macOS: fixed tray icon missing when launched from .app bundle
- macOS: fixed AWS SSM "session-manager-plugin not found"
- Compact mode: denser sidebar, toolbar, search bar, popover menus
- Compact mode enabled by default on macOS for new installations
- Hamburger menu restructured: Tools and Sessions submenus
- Upgraded: ironrdp 0.14→0.15 (+ ironrdp-bulk 0.1)
- Updated: inotify 0.11.2, ironrdp-tls 0.2.1, rustls-native-certs 0.8.4,
  unicode-segmentation 1.13.3

* Sat May 31 2026 Anton Isaiev <totoshko88@gmail.com> - 0.15.4-0
- Fixed macOS UI hang when editing connection with broken/throttled ssh-agent (#163)
- SSH agent probe now runs asynchronously with a 5-second timeout

* Sat May 30 2026 Anton Isaiev <totoshko88@gmail.com> - 0.15.3-0
- Snap base bumped to core26 (Ubuntu 26.04 / GNOME 50 / libadwaita 1.8)
- Removed system-files plug; external CLIs download on demand inside sandbox
- Components dialog now works in both snap and Flatpak
- Added is_sandboxed() predicate unifying snap/Flatpak CLI logic
- Fixed broken Ukrainian translation; updated all 16 language catalogs
- Dependencies: hyper 1.10.1, uuid 1.23.2, zbus 5.16.0, zerocopy 0.8.50, zvariant 5.12.0

* Fri May 29 2026 Anton Isaiev <totoshko88@gmail.com> - 0.15.2-0
- Corrected misleading copy-pasted #[allow]/#[expect] reasons in rustconn-core and property tests; lints that fire now use #[expect]
- Removed dead notify_tx field and its #[allow(dead_code)] from DirectoryWatcher
- rustconn-cli update: replaced .position(...).unwrap() with a CliError instead of panicking
- Shortcuts help dialog now lists Ctrl+T, F10, Ctrl+W and font-zoom shortcuts; added a test guarding against keybinding-registry drift
- Docs: fixed Create Group shortcut (Ctrl+Shift+G), removed non-existent Ctrl+K, added Ctrl+Shift+B

* Fri May 29 2026 Anton Isaiev <totoshko88@gmail.com> - 0.15.1-0
- Flatpak language switch fix: relocate translations to /app/share/rustconn/locale/ to bypass GNOME Locale extension subset split (#158)

* Wed May 27 2026 Anton Isaiev <totoshko88@gmail.com> - 0.15.0-0
- Version bump to 0.15.0 — quality pass release
- RDP RemoteApp: closed /p: cmdline leak via single-use args file in $XDG_RUNTIME_DIR
- Migrated ~350 #[allow] overrides to #[expect(reason = "...")]; dropped ~50 stale ones
- Manual Debug impls for all secret backends with leak-protection tests
- # Errors documentation on every public CLI command function
- Settings-save failures now surface a toast instead of being dropped silently
- EntryRowBuilder requires pre-translated titles so xgettext catches every UI label
- GUI spacing follows GNOME HIG steps (6/12/18/24 px) consistently

* Wed May 27 2026 Anton Isaiev <totoshko88@gmail.com> - 0.14.10-0
- Version bump to 0.14.10
- Hardened secret handling: SecretString in vault save signatures
- Backend deserializers wrap passwords in SecretString immediately
- Removed password_len from Bitwarden unlock log
- F10 opens primary menu (GNOME HIG)
- Graceful exit on Tokio runtime failure
- PasswordGenerator surfaces RNG errors as RngError
- Named timeout constants for downloads and vault operations

* Mon May 26 2026 Anton Isaiev <totoshko88@gmail.com> - 0.14.9-0
- Version bump to 0.14.9
- Added Server Manager quick action in RDP admin tools menu
- Fixed split view and RemoteApp issues

* Mon May 26 2026 Anton Isaiev <totoshko88@gmail.com> - 0.14.8-0
- Split-view broadcast toggle in header bar (#160)
- Compact interface mode for reduced chrome (#157)
- Fixed broadcast mode — rewired around split view (#160)
- Fixed broadcast toggle visibility after Select Tab placement (#160)
- Fixed broadcast doubling typed characters in split panels (#160)
- Fixed language change in Flatpak (#158)
- Improved tray icon visibility on dark panels (#157)

* Sun May 25 2026 Anton Isaiev <totoshko88@gmail.com> - 0.14.7-0
- Version bump to 0.14.7
- Added Visual Tunnel Builder
- Added Keyboard passthrough mode

* Sat May 23 2026 Anton Isaiev <totoshko88@gmail.com> - 0.14.6-0
- Version bump to 0.14.6
- Variables dialog: collapsible rows, duplicate name validation
- RDP toolbar: Registry Editor and Device Manager replace PowerShell/CMD
- RDP scripts: instant clipboard paste
- Snippet delivery mode

* Thu May 22 2026 Anton Isaiev <totoshko88@gmail.com> - 0.14.5-0
- Version bump to 0.14.5
- All dialogs migrated to adw::Dialog
- Edit Group dialog redesigned with 5 tabs
- RDP Scripts via clipboard-paste
- Snippet target platform
- Quick Connect theme fix (#156)

* Tue May 20 2026 Anton Isaiev <totoshko88@gmail.com> - 0.14.4-1
- CLI: history, pin/unpin, tag, move, monitor commands — full connection management
- CLI: import --auto / --dry-run — auto-detect sources and preview imports
- CLI: export --csv-delimiter, --csv-fields — customize CSV export format
- CLI: add/update — full GUI parity for SSH, RDP, VNC, SPICE, MOSH, Serial fields
- Config file locking — exclusive advisory lock (fs2) on write; GUI + CLI no longer conflict
- SSH agent: add_key() accepts &SecretString — intermediate strings wrapped in Zeroizing
- Quick Connect: history persisted across sessions (up to 15 entries, no passwords)
- RDP Quick Actions: 3 new Windows admin tools (diskmgmt.msc, resmon, compmgmt.msc)
- Settings: Azure/gcloud/OCI CLI not detected in Flatpak — env vars now passed through
- Command Palette: fixed shortcut display Ctrl+Shift+N → Ctrl+Shift+G

* Tue May 20 2026 Anton Isaiev <totoshko88@gmail.com> - 0.14.3-1
- Settings: removed duplicate group titles above collapsible sections
- CLI: secret set --password wrapped in Zeroizing immediately, added --password-stdin
- External window: migrated to libadwaita (adw::ApplicationWindow + ToolbarView)
- Settings: collapsible sections (GNOME HIG) for Terminal, Interface, and Edit Connection
- Settings: credential storage as a 3-state ComboRow for all secret backends
- Settings UI: 25 toggles converted from CheckButton to AdwSwitchRow (GNOME HIG)
- Settings dialog field types unified to adw::SwitchRow

* Mon May 19 2026 Anton Isaiev <totoshko88@gmail.com> - 0.14.2-1
- CLI: fix `add --protocol web` port=0 error
- CLI: add SecureCRT export/import format
- CSV import: reject invalid port values instead of silent fallback
- Security: eliminate plain String intermediates for keyring passwords
- Fix tooltip showing wrong shortcut for New Group
- i18n: localize validation messages, Ctrl+Alt+Del labels, OK button

* Sun May 18 2026 Anton Isaiev <totoshko88@gmail.com> - 0.14.1-0
- Update to version 0.14.1
- [Added] Predefined connection templates — 20 built-in templates for common
  CLI tools (RustDesk, Docker, Podman, LXC, Incus, Distrobox, Virsh, Proxmox,
  IPMI, Picocom, WireGuard, Teleport, Ansible, and more) with emoji icons
- [Added] Template grid in Connection Wizard — Custom Command mode shows
  template buttons; user templates first, predefined fill remaining;
  "More…" popover with all templates by category
- [Added] Template icon field — templates support custom icon (emoji or GTK
  icon name); inherited by connections
- [Added] Per-connection "Skip port check" toggle
- [Fixed] "Use Template" freezes UI
- [Fixed] RDP Gateway: "Host unreachable" before connection
- [Fixed] Highlight overlay: colored underlines persisted after clear

* Sun May 18 2026 Anton Isaiev <totoshko88@gmail.com> - 0.14.0-0
- Update to version 0.14.0
- [Added] Connection Wizard (Ctrl+N) — step-by-step dialog for creating
  connections with 4-column protocol grid, adaptive fields, auth/appearance
- [Added] Quick Connect runtime history — last 15 sessions with filtering
- [Added] Duplicate via Wizard — clone and modify existing connections
- [Fixed] Wizard: Zero Trust provider fields, Mosh port, Serial baud rate
- [Changed] Ctrl+N opens Wizard; Ctrl+Shift+N opens full dialog
- [Changed] Highlight Rules collapsed into ExpanderRow

* Fri May 16 2026 Anton Isaiev <totoshko88@gmail.com> - 0.13.17-1
- Update to version 0.13.17
- [Fixed] Cloud Sync in Flatpak — detect XDG Document Portal paths when
  selecting sync directory; show a warning dialog with flatpak override
  command instead of silently saving an unusable portal path (#152)
- [Fixed] Highlight overlay not cleared by clear command — colored underlines
  and background highlights now disappear immediately when the terminal screen
  is erased (#154)

* Thu May 15 2026 Anton Isaiev <totoshko88@gmail.com> - 0.13.16-0
- Update to version 0.13.16
- [Added] macOS port — native PTY, tray icon, Keychain backend, Homebrew formula, DMG packaging
- [Added] macOS tray icon (tray-macos feature) — NSStatusItem with dynamic menu
- [Added] macOS Keychain backend — native credential storage via Security.framework
- [Fixed] macOS tray: main-thread initialization (NSStatusItem requirement)
- [Fixed] macOS tray: dynamic menu rebuild on state change
- [Fixed] X11 renderer fallback skipped on macOS
- [Fixed] Cross-platform statvfs types, secret backend detection, PTY cleanup
- [Dependencies] winnow 1.0.2→1.0.3

* Wed May 14 2026 Anton Isaiev <totoshko88@gmail.com> - 0.13.15-1
- [Added] Local Shell: custom command — new "Command" field in Settings →
  Terminal → Local Shell allows specifying a custom command instead of
  the default login shell
- [Fixed] Split screen snippet execution — snippets now execute in the
  focused pane of a split terminal tab instead of always targeting the
  first pane; uses per-session split bridge to resolve the correct
  focused session before sending text
- [Improved] Dynamic snippet context menu — when ≤5 snippets exist,
  they appear as individual items directly in the terminal right-click
  menu; when more than 5 exist, the previous picker is shown

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

