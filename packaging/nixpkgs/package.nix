{
  lib,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
  cmake,
  clang,
  gettext,
  wrapGAppsHook4,
  gtk4,
  libadwaita,
  vte-gtk4,
  webkitgtk_6_0,
  openssl,
  alsa-lib,
  dbus,
  glib,
  pango,
  gdk-pixbuf,
  graphene,
  cairo,
  nix-update-script,
}:

rustPlatform.buildRustPackage (finalAttrs: {
  pname = "rustconn";
  version = "0.19.4";

  src = fetchFromGitHub {
    owner = "totoshko88";
    repo = "RustConn";
    tag = "v${finalAttrs.version}";
    hash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
  };

  # Replace with the real hash obtained by running:
  #   nix-prefetch --option extra-experimental-features flakes \
  #     '(import <nixpkgs> {}).rustconn.cargoDeps'
  # or by setting lib.fakeHash here and reading the actual hash from the build error.
  cargoHash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

  nativeBuildInputs = [
    pkg-config
    cmake
    clang
    gettext
    wrapGAppsHook4
  ];

  buildInputs = [
    gtk4
    libadwaita
    vte-gtk4
    webkitgtk_6_0
    openssl
    alsa-lib
    dbus
    glib
    pango
    gdk-pixbuf
    graphene
    cairo
  ];

  # The workspace ships two separate binaries with different feature sets:
  #   rustconn     — GUI, default features (embedded SSH/RDP/VNC clients)
  #   rustconn-cli — CLI, requires --features full (connect, secret, SFTP)
  # buildRustPackage only supports a single cargo invocation via cargoBuildFlags,
  # so we override buildPhase to run both explicitly.
  buildPhase = ''
    runHook preBuild
    cargo build --release --frozen -p rustconn
    cargo build --release --frozen -p rustconn-cli --features full
    runHook postBuild
  '';

  doCheck = false;

  installPhase = ''
    runHook preInstall

    install -Dm755 target/release/rustconn     "$out/bin/rustconn"
    install -Dm755 target/release/rustconn-cli "$out/bin/rustconn-cli"

    install -Dm644 rustconn/assets/io.github.totoshko88.RustConn.desktop \
      "$out/share/applications/io.github.totoshko88.RustConn.desktop"

    install -Dm644 rustconn/assets/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg \
      "$out/share/icons/hicolor/scalable/apps/io.github.totoshko88.RustConn.svg"

    install -Dm644 rustconn/assets/io.github.totoshko88.RustConn.metainfo.xml \
      "$out/share/metainfo/io.github.totoshko88.RustConn.metainfo.xml"

    # Compile and install gettext message catalogues
    for po_file in po/*.po; do
      [ -f "$po_file" ] || continue
      lang=$(basename "$po_file" .po)
      mkdir -p "$out/share/locale/$lang/LC_MESSAGES"
      msgfmt -o "$out/share/locale/$lang/LC_MESSAGES/rustconn.mo" "$po_file"
    done

    runHook postInstall
  '';

  passthru = {
    updateScript = nix-update-script { };
  };

  meta = {
    description = "GTK4/libadwaita connection manager for SSH, RDP, VNC, SPICE, Telnet, Serial, and Kubernetes";
    longDescription = ''
      RustConn is a modern connection manager for Linux with a GTK4/Wayland-native
      interface. Manage SSH, RDP, VNC, SPICE, MOSH, Telnet, Serial, Kubernetes, and
      Zero Trust connections from a single application. Terminals and RDP and VNC
      sessions open inside the window; SPICE sessions open in an external viewer
      (remote-viewer). Some features use other command-line tools, such as a cloud
      provider's CLI for Zero Trust connections or a password manager's client.
    '';
    homepage = "https://github.com/totoshko88/RustConn";
    changelog = "https://github.com/totoshko88/RustConn/blob/v${finalAttrs.version}/CHANGELOG.md";
    license = lib.licenses.gpl3Plus;
    maintainers = with lib.maintainers; [ totoshko88 ];
    platforms = lib.platforms.linux;
    mainProgram = "rustconn";
  };
})
