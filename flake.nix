{
  description = "RustConn — GTK4/libadwaita connection manager for SSH, RDP, VNC, SPICE, and more";

  # No flake.lock is committed on purpose (docs/INSTALL.md, "flake.lock policy"):
  # nixpkgs follows nixpkgs-unstable so the GTK4/libadwaita stack stays current.
  # scripts/release.sh and the CI `nix` job evaluate this flake against the
  # current unstable instead; pin a rev here only if that proves too noisy.
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "rustconn";
          version = "0.24.11";

          src = self;

          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = with pkgs; [
            pkg-config
            cmake
            clang
            gettext
            wrapGAppsHook4
          ];

          buildInputs = with pkgs; [
            gtk4
            libadwaita
            vte-gtk4
            webkitgtk_6_0
            openssl
            dbus
            alsa-lib
            glib
            pango
            gdk-pixbuf
            graphene
            cairo
          ];

          # Build GUI and CLI as separate cargo invocations because:
          # - rustconn (GUI) uses its default features (all embedded clients)
          # - rustconn-cli needs --features full (connect, secret-management, SFTP)
          # Without --features full, CLI ships without connect/secret/SFTP commands.
          # (Same fix as release.yml / Flatpak in 0.18.7)
          buildPhase = ''
            runHook preBuild
            # nixpkgs' Rust hooks export AWS_LC_SYS_USE_SYSTEM=1, but no system
            # AWS-LC is provided (only OpenSSL); build aws-lc-sys from source
            # (cmake and clang are in nativeBuildInputs).
            unset AWS_LC_SYS_USE_SYSTEM
            cargo build --release --frozen --offline -p rustconn
            cargo build --release --frozen --offline -p rustconn-cli --features full
            runHook postBuild
          '';

          installPhase = ''
            runHook preInstall
            install -Dm755 target/release/rustconn $out/bin/rustconn
            install -Dm755 target/release/rustconn-cli $out/bin/rustconn-cli
            runHook postInstall
          '';

          # Skip tests during nix build — they require ~120s (argon2 property tests)
          # and are not meaningful for end-user installation
          doCheck = false;

          # GUI crate needs gettext (msgfmt) at build time for locale compilation
          RUSTCONN_SKIP_LOCALE_BUILD = "0";

          postInstall = ''
            # Install desktop file and icon
            install -d $out/share/applications
            msgfmt --desktop --template rustconn/assets/io.github.totoshko88.RustConn.desktop -d po \
              -o $out/share/applications/io.github.totoshko88.RustConn.desktop
            install -Dm644 rustconn/assets/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg \
              $out/share/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg
            # Install metainfo
            install -Dm644 rustconn/assets/io.github.totoshko88.RustConn.metainfo.xml \
              $out/share/metainfo/io.github.totoshko88.RustConn.metainfo.xml
          '';

          meta = with pkgs.lib; {
            description = "GTK4/libadwaita connection manager for SSH, RDP, VNC, SPICE, Telnet, Serial, Kubernetes";
            homepage = "https://github.com/totoshko88/RustConn";
            license = licenses.gpl3Plus;
            maintainers = [ ];
            platforms = platforms.linux;
            mainProgram = "rustconn";
          };
        };

        # Development shell with all build dependencies
        devShells.default = pkgs.mkShell {
          inputsFrom = [ self.packages.${system}.default ];
          packages = with pkgs; [
            rust-analyzer
            clippy
            rustfmt
          ];
        };
      }
    );
}
