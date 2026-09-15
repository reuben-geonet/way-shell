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
          nativeCheckInputs = [ pkgs.pipewire pkgs.wireplumber pkgs.sway pkgs.niri pkgs.dbus ];
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
            make -C tests power-profiles-widgets-test
            sh tests/wayland-component.sh tests/power-profiles-widgets-test
            make -C tests battery-widgets-test
            sh tests/wayland-component.sh tests/battery-widgets-test
            sh tests/wayland-component.sh tests/battery-widgets-test niri
            make -C tests power-widgets-test
            sh tests/wayland-component.sh tests/power-widgets-test
            sh tests/wayland-component.sh tests/power-widgets-test niri
            make -C tests audio-widgets-test
            sh tests/wayland-component.sh tests/audio-widgets-test
            sh tests/wayland-component.sh tests/audio-widgets-test niri
            make -C tests media-widgets-test
            sh tests/wayland-component.sh tests/media-widgets-test
            sh tests/wayland-component.sh tests/media-widgets-test niri
            make -C tests media-presentation-test
            sh tests/wayland-component.sh tests/media-presentation-test
            sh tests/wayland-component.sh tests/media-presentation-test niri
            make -C tests notification-presentation-test
            sh tests/wayland-component.sh tests/notification-presentation-test
            sh tests/wayland-component.sh tests/notification-presentation-test niri
            make -C tests notification-replacement-test
            sh tests/wayland-component.sh tests/notification-replacement-test
            sh tests/wayland-component.sh tests/notification-replacement-test niri
            cargo build -p way-shell --example tray-ui-compat --example panel-status-compat --example panel-compat --frozen --jobs 1
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/tray-ui-compat
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/tray-ui-compat niri
            GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/panel-status-compat
            GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/panel-status-compat niri
            GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/panel-compat
            GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/panel-compat niri
            make -C tests brightness-widgets-test
            sh tests/wayland-component.sh tests/brightness-widgets-test
            sh tests/wayland-component.sh tests/brightness-widgets-test niri
            cargo build -p way-shell --example niri-compat --frozen --jobs 1
            FONTCONFIG_FILE=${pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts ]; }} \
              sh tests/wayland-component.sh target/debug/examples/niri-compat niri
            FONTCONFIG_FILE=${pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts ]; }} \
              sh tests/wayland-component.sh target/debug/examples/theme-compat niri
            cargo build -p way-shell --example wayland-compat --frozen --jobs 1
            GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/wayland-compat
            GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/wayland-compat niri
            cargo build -p way-shell --example window-compat --frozen --jobs 1
            GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/window-compat
            GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/window-compat niri
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
