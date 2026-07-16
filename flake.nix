{
  description = "RustConn — GTK4/libadwaita connection manager for SSH, RDP, VNC, SPICE, and more";

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
          version = "0.18.10";

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
            openssl
            dbus
            alsa-lib
            glib
            pango
            gdk-pixbuf
            graphene
            cairo
          ];

          # Build both GUI and CLI
          cargoBuildFlags = [ "--workspace" "--exclude" "rustconn-pty-sys" ];

          # GUI crate needs gettext (msgfmt) at build time for locale compilation
          RUSTCONN_SKIP_LOCALE_BUILD = "0";

          postInstall = ''
            # Install desktop file and icon
            install -Dm644 rustconn/assets/io.github.totoshko88.RustConn.desktop \
              $out/share/applications/io.github.totoshko88.RustConn.desktop
            install -Dm644 rustconn/assets/io.github.totoshko88.RustConn.svg \
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
