class Rustconn < Formula
  desc "Manage remote connections easily - SSH, RDP, VNC, SPICE, Telnet, Serial"
  homepage "https://github.com/totoshko88/RustConn"
  url "https://github.com/totoshko88/RustConn/archive/refs/tags/v0.18.0.tar.gz"
  sha256 "54f345e618ef9066e1a81e821c4c1ba252abb1a0fd631a9e7ca265558ffa4a7f"
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
    # Build both binaries in a single cargo invocation to avoid
    # duplicate dependency resolution and share compilation artifacts.
    system "cargo", "build", "--release",
           "-p", "rustconn", "-p", "rustconn-cli",
           "--no-default-features",
           "--features", "rustconn/tray-macos,rustconn/vnc-embedded,rustconn/rdp-embedded,rustconn/rdp-audio"

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
    mkdir_p "#{app_dir}/Resources"

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

    # Wrapper script — sets up environment for GTK4/libadwaita runtime
    (app_dir/"MacOS/rustconn-wrapper").write <<~EOS
      #!/bin/bash
      export XDG_DATA_DIRS="$HOME/.local/share:#{HOMEBREW_PREFIX}/share:/usr/local/share:/usr/share"
      export GSETTINGS_SCHEMA_DIR="#{HOMEBREW_PREFIX}/share/glib-2.0/schemas"
      export LOCALEDIR="#{share}/locale"
      export PATH="#{HOMEBREW_PREFIX}/bin:#{HOMEBREW_PREFIX}/sbin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"
      cd "$HOME"
      exec "#{bin}/rustconn" "$@"
    EOS
    chmod 0755, "#{app_dir}/MacOS/rustconn-wrapper"

    # Info.plist
    (app_dir/"Info.plist").write <<~EOS
      <?xml version="1.0" encoding="UTF-8"?>
      <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
      <plist version="1.0">
      <dict>
          <key>CFBundleExecutable</key>
          <string>rustconn-wrapper</string>
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
      open "#{prefix}/RustConn.app" "$@"
    EOS
    chmod 0755, bin/"rustconn-app"
  end

  def post_install
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
        rustconn-app
        # or: open #{prefix}/RustConn.app

      To add to Applications (Launchpad):
        ln -sf #{prefix}/RustConn.app /Applications/RustConn.app

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
