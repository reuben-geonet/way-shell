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
            cargo build -p way-shell --examples --frozen --jobs 1
            sh tests/audio-smoke.sh target/debug/examples/audio-smoke
            sh tests/component-smoke.sh target/debug/examples
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
