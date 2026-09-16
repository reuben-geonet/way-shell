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
        way-shell = pkgs.rustPlatform.buildRustPackage {
          pname = "way-shell";
          inherit (config.wayShell) version;
          src = config.wayShell.source;
          strictDeps = true;
          outputs = [ "out" "testHelpers" ];
          wrapGAppsInOutputs = [ "out" ];
          enableParallelBuilding = false;
          cargoDeps = config.wayShell.cargoVendor;
          # Use the same Cargo entry point and artifact layout as the Fedora build.
          auditable = false;
          CARGO_TARGET_DIR = "target";
          nativeBuildInputs = with pkgs; [
            pkg-config
            glib
            rustfmt
            clippy
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
            libpulseaudio
            networkmanager
            pipewire
            wayland
            wireplumber
            dconf
          ];
          # The default Rust hooks pass --target. These explicit native phases
          # keep the shared installer and component probes on target/{release,debug}.
          buildPhase = ''
            runHook preBuild
            cargo build --workspace --bins --release --frozen --jobs 1
            cargo build -p way-shell --example schema-probe --example application-smoke --frozen --jobs 1
            runHook postBuild
          '';
          doCheck = true;
          nativeCheckInputs = [ pkgs.pipewire pkgs.wireplumber pkgs.sway pkgs.niri pkgs.dbus ];
          checkPhase = ''
            runHook preCheck
            runHook postCheck
          '';
          WAY_SHELL_TEST_EGL_VENDOR = "${pkgs.mesa}/share/glvnd/egl_vendor.d/50_mesa.json";
          FONTCONFIG_FILE = "${pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts ]; }}";
          preCheck = ''
            cargo test --workspace --frozen --jobs 1
            cargo build -p way-shell --example audio-smoke --frozen --jobs 1
            sh tests/audio-smoke.sh target/debug/examples/audio-smoke
            cargo build -p way-shell --example theme-smoke --frozen --jobs 1
            FONTCONFIG_FILE=${pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts ]; }} \
              sh tests/wayland-component.sh target/debug/examples/theme-smoke
            cargo build -p way-shell --example sway-smoke --frozen --jobs 1
            sh tests/wayland-component.sh target/debug/examples/sway-smoke
            cargo build -p way-shell --example notification-ui-smoke --example message-tray-media-smoke --example message-tray-window-smoke --example message-tray-smoke --frozen --jobs 1
            for probe in notification-ui-smoke message-tray-media-smoke message-tray-window-smoke message-tray-smoke; do
              G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh "target/debug/examples/$probe"
              G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh "target/debug/examples/$probe" niri
            done
            cargo build -p way-shell --example tray-ui-smoke --example panel-status-smoke --example panel-smoke --frozen --jobs 1
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/tray-ui-smoke
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/tray-ui-smoke niri
            GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/panel-status-smoke
            GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/panel-status-smoke niri
            GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/panel-smoke
            GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/panel-smoke niri
            cargo build -p way-shell --example niri-smoke --frozen --jobs 1
            FONTCONFIG_FILE=${pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts ]; }} \
              sh tests/wayland-component.sh target/debug/examples/niri-smoke niri
            FONTCONFIG_FILE=${pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts ]; }} \
              sh tests/wayland-component.sh target/debug/examples/theme-smoke niri
            cargo build -p way-shell --example wayland-smoke --frozen --jobs 1
            GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/wayland-smoke
            GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/wayland-smoke niri
            cargo build -p way-shell --example window-smoke --frozen --jobs 1
            GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/window-smoke
            GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/window-smoke niri
            cargo build -p way-shell --example popup-click-away-smoke --frozen --jobs 1
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/popup-click-away-smoke sway
            cargo build -p way-shell --example switcher-smoke --frozen --jobs 1
            GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/switcher-smoke
            GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/switcher-smoke niri
            cargo build -p way-shell --example workspace-switchers-smoke --frozen --jobs 1
            GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/workspace-switchers-smoke
            GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/workspace-switchers-smoke niri
            cargo build -p way-shell --example activities-smoke --frozen --jobs 1
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/activities-smoke
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/activities-smoke niri
            cargo build -p way-shell --example quick-settings-window-smoke --example quick-settings-system-smoke --frozen --jobs 1
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/quick-settings-window-smoke
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/quick-settings-window-smoke niri
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/quick-settings-system-smoke
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/quick-settings-system-smoke niri
            cargo build -p way-shell --example quick-settings-bluetooth-smoke --frozen --jobs 1
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/quick-settings-bluetooth-smoke
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/quick-settings-bluetooth-smoke niri
            cargo build -p way-shell --example quick-settings-network-smoke --frozen --jobs 1
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/quick-settings-network-smoke
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/quick-settings-network-smoke niri
            cargo build -p way-shell --example dialog-smoke --frozen --jobs 1
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/dialog-smoke
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/dialog-smoke niri
            cargo build -p way-shell --example osd-smoke --frozen --jobs 1
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/osd-smoke
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/osd-smoke niri
            cargo build -p way-shell --example quick-settings-audio-smoke --example quick-settings-smoke --frozen --jobs 1
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/quick-settings-audio-smoke
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/quick-settings-audio-smoke niri
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/quick-settings-smoke
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh target/debug/examples/quick-settings-smoke niri
            cargo test -p way-shell --lib --no-run --message-format=json --frozen --jobs 1 > "$TMPDIR/runtime-test-artifact.json"
            export WAY_SHELL_RUNTIME_TEST_BINARY=$(sed -n 's/.*"executable":"\([^"]*\)".*/\1/p' "$TMPDIR/runtime-test-artifact.json")
            test -x "$WAY_SHELL_RUNTIME_TEST_BINARY"
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh tests/runtime-smoke.sh
            G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh tests/runtime-smoke.sh niri
            cargo fmt --all --check
            cargo clippy --workspace --all-targets --frozen --jobs 1 -- -D warnings
          '';
          installPhase = ''
            runHook preInstall
            PREFIX="$out" CARGO_ARTIFACT_DIR=target/release \
              DEPENDENCY_LICENSES=${config.wayShell.dependencyLicenses} \
              sh scripts/install.sh
            runHook postInstall
          '';
          postInstall = ''
            install -Dm755 target/debug/examples/schema-probe "$testHelpers/bin/schema-probe"
            install -Dm755 target/debug/examples/application-smoke "$testHelpers/bin/application-smoke"
            glib-compile-schemas "$out/share/glib-2.0/schemas"
            substituteInPlace "$out/lib/systemd/user/way-shell.service" \
              --replace-fail /usr/bin/way-shell "$out/bin/way-shell"
          '';
          meta = {
            description = "GNOME-like desktop shell for Sway and Niri";
            homepage = "https://github.com/ldelossa/way-shell";
            license = lib.licenses.gpl2Only;
            platforms = [ "x86_64-linux" ];
            mainProgram = "way-shell";
          };
        };
      };
    };
}
