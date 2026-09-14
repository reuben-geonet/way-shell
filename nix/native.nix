{
  perSystem =
    {
      config,
      pkgs,
      lib,
      ...
    }:
    {
      packages = rec {
        default = way-shell;
        way-shell = pkgs.stdenv.mkDerivation {
          pname = "way-shell";
          inherit (config.wayShell) version;
          src = config.wayShell.source;
          strictDeps = true;
          enableParallelBuilding = false;
          cargoDeps = config.wayShell.cargoVendor;
          CARGO_BUILD_FLAGS = "--frozen";
          nativeBuildInputs = with pkgs; [
            pkg-config
            glib
            wayland-scanner
            python3
            cargo
            rustc
            rustfmt
            clippy
            rustPlatform.cargoSetupHook
            rustPlatform.bindgenHook
            # A shell wrapper lets the schema check reuse the exact environment
            # of the installed application with a GLib probe as its executable.
            (wrapGAppsHook4.override { makeWrapper = pkgs.makeShellWrapper; })
          ];
          buildInputs = with pkgs; [
            glib
            gtk4
            gtk4-layer-shell
            libadwaita
            json-glib
            libpulseaudio
            networkmanager
            pipewire
            upower
            wayland
            wayland-protocols
            wireplumber
            dconf
          ];
          installFlags = [ "PREFIX=$(out)" ];
          preBuild = ''
            cargo build --workspace --frozen --jobs 1
          '';
          doCheck = true;
          nativeCheckInputs = [ pkgs.pipewire pkgs.sway pkgs.niri pkgs.dbus ];
          checkTarget = "check";
          WAY_SHELL_TEST_EGL_VENDOR = "${pkgs.mesa}/share/glvnd/egl_vendor.d/50_mesa.json";
          preCheck = ''
            cargo test --workspace --frozen --jobs 1
            cargo build -p way-shell --example audio-compat --frozen --jobs 1
            sh tests/audio-compat.sh target/debug/examples/audio-compat
            cargo build -p way-shell --example theme-compat --frozen --jobs 1
            FONTCONFIG_FILE=${pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts ]; }} \
              sh tests/wayland-component.sh target/debug/examples/theme-compat
            cargo build -p way-shell --example sway-compat --frozen --jobs 1
            sh tests/wayland-component.sh target/debug/examples/sway-compat
            cargo build -p way-shell --example niri-compat --frozen --jobs 1
            FONTCONFIG_FILE=${pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts ]; }} \
              sh tests/wayland-component.sh target/debug/examples/niri-compat niri
            FONTCONFIG_FILE=${pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts ]; }} \
              sh tests/wayland-component.sh target/debug/examples/theme-compat niri
            cargo fmt --all --check
            cargo clippy --workspace --all-targets --frozen --jobs 1 -- -D warnings
          '';
          postInstall = ''
            glib-compile-schemas "$out/share/glib-2.0/schemas"
            install -Dm644 LICENSE "$out/share/licenses/way-shell/LICENSE"
            cp -r ${config.wayShell.dependencyLicenses} "$out/share/licenses/way-shell/dependencies"
            substituteInPlace "$out/lib/systemd/user/way-shell.service" \
              --replace-fail /usr/bin/way-shell "$out/bin/way-shell"
          '';
          meta = {
            description = "GNOME-like desktop shell for Sway";
            homepage = "https://github.com/ldelossa/way-shell";
            license = lib.licenses.gpl2Only;
            platforms = [ "x86_64-linux" ];
            mainProgram = "way-shell";
          };
        };
      };
    };
}
