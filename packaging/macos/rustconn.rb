class Rustconn < Formula
  desc "Remote connection manager - SSH, RDP, VNC, SPICE, Telnet, Serial, and more"
  homepage "https://github.com/totoshko88/RustConn"
  # This is the canonical formula; the release workflow copies it into the tap
  # and rewrites the two lines below with the release tag and the measured
  # checksum of that tarball. Keep them as a single active `url` and a single
  # active `sha256` at this indentation — the sed patterns and the CI
  # verification gate are anchored to `^  url` and `^  sha256` (issue #251).
  # PLACEHOLDER_SHA256 is expected here in-tree; only the tap copy has a hash.
  url "https://github.com/totoshko88/RustConn/archive/refs/tags/v0.21.11.tar.gz"
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
  # The gfx-h264 feature dlopen's libopenh264.dylib at runtime for RDP EGFX/AVC.
  # Unlike the DMG producer (scripts/macos-build.sh), this formula does not bundle
  # dylibs into the .app — it relies on the Homebrew keg — so OpenH264 must be a
  # declared runtime dependency, or H.264 would silently degrade to a non-AVC path.
  depends_on "openh264"
  depends_on "openssl@3"
  depends_on "vte3"

  def install
    # Homebrew's `rust` is not a rustup proxy, so rust-toolchain.toml is ignored
    # here and this compiles with whatever `rust` Homebrew ships. Guard the floor:
    # assert rustc is at least the MSRV (`rust-version` in Cargo.toml — keep this
    # literal in step with it) so a too-old Homebrew rust fails with a clear
    # message instead of a confusing edition/feature error mid-compile.
    msrv = "1.95"
    rustc_version = Utils.safe_popen_read("rustc", "--version").split[1]
    if Gem::Version.new(rustc_version) < Gem::Version.new(msrv)
      odie "RustConn needs Rust >= #{msrv}, but Homebrew's rust is #{rustc_version}. " \
           "Run `brew upgrade rust` and try again."
    end

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

      # The ladder's ceiling is its highest rung. When Homebrew moves past it
      # (e.g. libadwaita 1.10 while the top rung is still 1.8), the newer feature
      # is not selected and its capabilities silently never build — the "shipped
      # the 1.5 baseline" regression, but quiet. Compare the installed minor
      # against the top rung's minor and emit a hint when it is ahead, so the
      # drift shows up in the build log and prompts a new rung here (and in the
      # OBS/RPM twins) rather than being discovered by a user. Best-effort: an
      # unparseable version simply skips the check.
      highest = ladder.keys.first
      installed = Utils.safe_popen_read("pkg-config", "--modversion", pc_name).strip
      top_parts = highest.split(".").map(&:to_i)
      cur_parts = installed.split(".").map(&:to_i)
      if cur_parts.length >= 2 && top_parts.length >= 2 &&
         (cur_parts[0] > top_parts[0] ||
          (cur_parts[0] == top_parts[0] && cur_parts[1] > top_parts[1]))
        opoo "#{pc_name} #{installed} is newer than the highest known rung " \
             "(#{highest}); RustConn may be missing features for it — add a rung " \
             "to this formula and its OBS/RPM twins."
      end
    rescue ErrorDuringExecution
      # pkg-config could not report a version; skip the drift hint.
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

    # Install locales. `--check` matches the canonical producer
    # (scripts/macos-build.sh): it validates format placeholders and headers, so
    # a catalog with a dropped `{}` placeholder fails the build here instead of
    # silently shipping a broken translation in the main macOS artifact.
    Dir["po/*.po"].each do |po|
      lang = File.basename(po, ".po")
      mkdir_p "#{share}/locale/#{lang}/LC_MESSAGES"
      system "msgfmt", "--check", "-o", "#{share}/locale/#{lang}/LC_MESSAGES/rustconn.mo", po
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

    # Icon. Delegated to scripts/make-iconset.sh, the same script the canonical
    # producer uses, so the render-and-package logic cannot drift between the two.
    #
    # The previous inline version rendered each size through `system
    # rsvg-convert` with no check that a well-formed PNG came back, then packaged
    # with `iconutil`. In the build sandbox that surfaced as an opaque
    # "Invalid Iconset" from iconutil whenever a render came back empty or the
    # wrong size, with nothing in the log to say which member was bad. The shared
    # script renders directly into the canonical Apple names, verifies every PNG
    # with `sips` before packaging, and fails naming the offending file — turning
    # an intermittent, undiagnosable packaging failure into a clear one.
    system "bash", "scripts/make-iconset.sh",
           "rustconn/assets/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg",
           "#{app_dir}/Resources/RustConn.icns"

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
      <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
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
    # Compile GSettings schemas and refresh the icon cache. Both operate on the
    # shared HOMEBREW_PREFIX/share tree, not the keg, and `system` raises on a
    # non-zero exit — so a missing/unwritable icons directory or schema dir would
    # turn a cosmetic refresh into a failed install. Best-effort for the same
    # reason as lsregister above; the app resolves schemas and icons at runtime
    # regardless.
    begin
      system "#{Formula["glib"].opt_bin}/glib-compile-schemas",
             "#{HOMEBREW_PREFIX}/share/glib-2.0/schemas"
    rescue StandardError => e
      opoo "Could not compile GSettings schemas: #{e.message}"
    end
    begin
      system "#{Formula["gtk4"].opt_bin}/gtk4-update-icon-cache", "-f", "-t",
             "#{HOMEBREW_PREFIX}/share/icons/hicolor"
    rescue StandardError => e
      opoo "Could not update the icon cache: #{e.message}"
    end
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
