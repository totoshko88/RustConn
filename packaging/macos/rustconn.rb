class Rustconn < Formula
  desc "Manage remote connections easily - SSH, RDP, VNC, SPICE, Telnet, Serial"
  homepage "https://github.com/totoshko88/RustConn"
  # This is the canonical formula; the release workflow copies it into the tap
  # and rewrites the two lines below with the release tag and the measured
  # checksum of that tarball. Keep them as a single active `url` and a single
  # active `sha256` at this indentation — the sed patterns and the CI
  # verification gate are anchored to `^  url` and `^  sha256` (issue #251).
  # PLACEHOLDER_SHA256 is expected here in-tree; only the tap copy has a hash.
  url "https://github.com/totoshko88/RustConn/archive/refs/tags/v0.21.8.tar.gz"
  sha256 "PLACEHOLDER_SHA256"
  license "GPL-3.0-or-later"
  head "https://github.com/totoshko88/RustConn.git", branch: "main"

  depends_on "gettext" => :build
  depends_on "librsvg" => :build
  depends_on "pkg-config" => :build
  depends_on "rust" => :build

  depends_on "adwaita-icon-theme"
  depends_on "dbus"
  depends_on "glib"
  depends_on "gtk4"
  depends_on "libadwaita"
  depends_on :macos
  depends_on "openssl@3"
  depends_on "vte3"

  def install
    # Detected, not written out by hand. Homebrew's gtk4, libadwaita and vte3 move
    # independently of this formula: `adw-1-8` was hardcoded, and no GTK or VTE
    # feature was selected at all, so the Command monitoring mode could not appear
    # on macOS whatever VTE was installed.
    #
    # pkg-config is asked rather than Homebrew's formula metadata, so the answer
    # comes from the same files the compiler will read, and `--atleast-version` is
    # the comparator the OBS spec and debian.rules use too — a glob over
    # `--modversion` misses libadwaita 1.10. Ruby hashes keep insertion order, so
    # each ladder is walked newest-first and nothing is added when even the lowest
    # rung is unmet. Names are package-qualified because two -p packages are
    # selected below and a bare name would be ambiguous.
    features = %w[
      rustconn/tray-macos
      rustconn/system-keyring
      rustconn/vnc-embedded
      rustconn/rdp-embedded
      rustconn/gfx-h264
      rustconn/rdp-audio
      rustconn/rd-gateway
    ]

    {
      "libadwaita-1" => { "1.8" => "adw-1-8", "1.7" => "adw-1-7", "1.6" => "adw-1-6" },
      "gtk4" => { "4.22" => "gtk-4-22", "4.20" => "gtk-4-20", "4.18" => "gtk-4-18" },
      "vte-2.91-gtk4" => { "0.78" => "vte-0-78" },
    }.each do |pc_name, ladder|
      rung = ladder.find { |minimum, _| quiet_system("pkg-config", "--atleast-version=#{minimum}", pc_name) }
      features << "rustconn/#{rung.last}" if rung
    end

    ohai "RustConn feature set: #{features.join(",")}"

    # Build both binaries in a single cargo invocation to avoid
    # duplicate dependency resolution and share compilation artifacts.
    system "cargo", "build", "--release",
           "-p", "rustconn", "-p", "rustconn-cli",
           "--no-default-features",
           "--features", features.join(",")

    bin.install "target/release/rustconn"
    bin.install "target/release/rustconn-cli"

    # Install locales
    Dir["po/*.po"].each do |po|
      lang = File.basename(po, ".po")
      mkdir_p "#{share}/locale/#{lang}/LC_MESSAGES"
      system "msgfmt", "-o", "#{share}/locale/#{lang}/LC_MESSAGES/rustconn.mo", po
    end

    # Install icon
    mkdir_p "#{share}/icons/hicolor/scalable/apps"
    cp "rustconn/assets/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg",
       "#{share}/icons/hicolor/scalable/apps/"

    # Create .app bundle for macOS
    app_dir = prefix/"RustConn.app/Contents"
    mkdir_p "#{app_dir}/MacOS"
    mkdir_p "#{app_dir}/Resources/bin"

    # LaunchServices must execute a real binary *inside* the bundle. When
    # CFBundleExecutable named a wrapper that exec'd the keg's bin/rustconn, the
    # process that ended up owning the window had no enclosing bundle, so it got
    # the generic Unix-executable Dock tile and no Info.plist identity — and
    # replacing the process image also destroys the LaunchServices scene
    # registration NSStatusItem needs, which is the same root cause fixed for the
    # canonical producer in 0.19.x (see CHANGELOG, "tray icon missing when
    # launched from .app bundle").
    #
    # A second copy rather than a link, deliberately. A symlink would make
    # `_NSGetExecutablePath` — and therefore the bundle detection in
    # rustconn/src/main.rs — depend on whether macOS resolves it, which Apple
    # documents only as "may contain symlinks"; a hard link would be undone by any
    # Homebrew relocation pass that rewrites the file rather than editing it in
    # place, silently leaving the bundle copy unrelocated. Two independent Mach-O
    # files are seen by every such pass and behave identically on both launch
    # paths, at the cost of the binary's size in the keg.
    #
    # The source is the keg's bin, not target/release: `bin.install` above *moves*
    # the artefact, so the build path no longer holds it by this point.
    cp "#{bin}/rustconn", "#{app_dir}/MacOS/rustconn"
    chmod 0555, "#{app_dir}/MacOS/rustconn"

    # Translations, bundle-relative: with no wrapper to export LOCALEDIR, a
    # LaunchServices start resolves them through the .app detection in
    # `i18n::locale_dir()`, which looks here and nowhere else inside a bundle.
    cp_r "#{share}/locale", "#{app_dir}/Resources/locale"

    # Icon
    mkdir_p buildpath/"iconset/RustConn.iconset"
    [16, 32, 64, 128, 256, 512, 1024].each do |size|
      system "rsvg-convert", "-w", size.to_s, "-h", size.to_s,
             "rustconn/assets/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg",
             "-o", buildpath/"iconset/icon_#{size}.png"
    end
    cp buildpath/"iconset/icon_16.png", buildpath/"iconset/RustConn.iconset/icon_16x16.png"
    cp buildpath/"iconset/icon_32.png", buildpath/"iconset/RustConn.iconset/icon_16x16@2x.png"
    cp buildpath/"iconset/icon_32.png", buildpath/"iconset/RustConn.iconset/icon_32x32.png"
    cp buildpath/"iconset/icon_64.png", buildpath/"iconset/RustConn.iconset/icon_32x32@2x.png"
    cp buildpath/"iconset/icon_128.png", buildpath/"iconset/RustConn.iconset/icon_128x128.png"
    cp buildpath/"iconset/icon_256.png", buildpath/"iconset/RustConn.iconset/icon_128x128@2x.png"
    cp buildpath/"iconset/icon_256.png", buildpath/"iconset/RustConn.iconset/icon_256x256.png"
    cp buildpath/"iconset/icon_512.png", buildpath/"iconset/RustConn.iconset/icon_256x256@2x.png"
    cp buildpath/"iconset/icon_512.png", buildpath/"iconset/RustConn.iconset/icon_512x512.png"
    cp buildpath/"iconset/icon_1024.png", buildpath/"iconset/RustConn.iconset/icon_512x512@2x.png"
    system "iconutil", "-c", "icns", buildpath/"iconset/RustConn.iconset",
           "-o", "#{app_dir}/Resources/RustConn.icns"

    # Optional manual-terminal launcher. Under Resources/bin, not MacOS/, so it is
    # not mistaken for the bundle executable and does not interfere with
    # nested-code signing — the same placement the canonical producer uses. Nothing
    # launches through it automatically; the app resolves schemas and icons from
    # Homebrew on its own, and this only exists for a terminal start that wants the
    # bundle's own translations.
    (app_dir/"Resources/bin/rustconn-wrapper").write <<~EOS
      #!/bin/bash
      CONTENTS="$(cd "$(dirname "$0")/../.." && pwd)"
      export XDG_DATA_DIRS="$HOME/.local/share:#{HOMEBREW_PREFIX}/share:/usr/local/share:/usr/share"
      export GSETTINGS_SCHEMA_DIR="#{HOMEBREW_PREFIX}/share/glib-2.0/schemas"
      export LOCALEDIR="$CONTENTS/Resources/locale"
      export PATH="#{HOMEBREW_PREFIX}/bin:#{HOMEBREW_PREFIX}/sbin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"
      cd "$HOME"
      exec "$CONTENTS/MacOS/rustconn" "$@"
    EOS
    chmod 0755, "#{app_dir}/Resources/bin/rustconn-wrapper"

    # Info.plist
    (app_dir/"Info.plist").write <<~EOS
      <?xml version="1.0" encoding="UTF-8"?>
      <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
      <plist version="1.0">
      <dict>
          <key>CFBundleExecutable</key>
          <string>rustconn</string>
          <key>CFBundleIconFile</key>
          <string>RustConn</string>
          <key>CFBundleIdentifier</key>
          <string>io.github.totoshko88.RustConn</string>
          <key>CFBundleName</key>
          <string>RustConn</string>
          <key>CFBundleDisplayName</key>
          <string>RustConn</string>
          <key>CFBundlePackageType</key>
          <string>APPL</string>
          <key>CFBundleVersion</key>
          <string>#{version}</string>
          <key>CFBundleShortVersionString</key>
          <string>#{version}</string>
          <key>NSHighResolutionCapable</key>
          <true/>
          <key>LSMinimumSystemVersion</key>
          <string>13.0</string>
          <key>NSDocumentsFolderUsageDescription</key>
          <string>RustConn needs access to import SSH configs and connection files.</string>
          <key>NSAppleEventsUsageDescription</key>
          <string>RustConn needs to open URLs in your default browser.</string>
      </dict>
      </plist>
    EOS

    # Create a launch script in bin for convenience (no env vars needed)
    (bin/"rustconn-app").write <<~EOS
      #!/bin/bash
      # Launch RustConn via its .app bundle for proper LaunchServices identity.
      # For Dock pinning: symlink to /Applications first, then pin from there.
      open "#{prefix}/RustConn.app" "$@"
    EOS
    chmod 0755, bin/"rustconn-app"
  end

  def post_install
    # Register .app bundle with LaunchServices so macOS recognises the bundle
    # identity for Dock pinning, file associations and the Cmd-Tab switcher.
    #
    # Best effort on purpose: lsregister is a private tool at an undocumented
    # path, and `system` raises on a non-zero exit, so treating a failure as
    # fatal would turn a cosmetic Dock icon into a failed install.
    lsregister = "/System/Library/Frameworks/CoreServices.framework/Frameworks/" \
                 "LaunchServices.framework/Support/lsregister"
    if File.executable?(lsregister)
      begin
        system lsregister, "-f", "#{prefix}/RustConn.app"
      rescue StandardError => e
        opoo "Could not register RustConn.app with LaunchServices: #{e.message}"
      end
    end
    # Compile GSettings schemas (required for GTK4 apps)
    system "#{Formula["glib"].opt_bin}/glib-compile-schemas",
           "#{HOMEBREW_PREFIX}/share/glib-2.0/schemas"
    # Update icon cache
    system "#{Formula["gtk4"].opt_bin}/gtk4-update-icon-cache", "-f", "-t",
           "#{HOMEBREW_PREFIX}/share/icons/hicolor"
  end

  def caveats
    <<~EOS
      RustConn has been installed with all dependencies.

      To launch the GUI:
        open #{prefix}/RustConn.app
        # or from terminal (tray icon works on all macOS versions this way):
        rustconn

      To pin to Dock with the correct icon:
        ln -sf #{prefix}/RustConn.app /Applications/RustConn.app
        # Then open from /Applications and right-click the Dock icon →
        # Options → Keep in Dock.

      Convenience launcher (calls `open RustConn.app`):
        rustconn-app

      CLI tool:
        rustconn-cli --help

      Optional password manager integrations:
        brew install --cask keepassxc     # KeePassXC
        brew install bitwarden-cli        # Bitwarden
        brew install --cask 1password-cli # 1Password
        brew install pass                 # Pass (GPG)
    EOS
  end

  test do
    assert_match "rustconn", shell_output("#{bin}/rustconn --help 2>&1")
    assert_match "rustconn-cli", shell_output("#{bin}/rustconn-cli --help 2>&1")
  end
end
