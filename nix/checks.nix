{
  perSystem =
    {
      config,
      pkgs,
      lib,
      ...
    }:
    let
      package = config.packages.way-shell;
      schemaProbe = package.testHelpers;
      compositorEnvironment = {
        FONTCONFIG_FILE = "${pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts ]; }}";
        WAY_SHELL_TEST_EGL_VENDOR = "${pkgs.mesa}/share/glvnd/egl_vendor.d/50_mesa.json";
      };
      rustCheck =
        name: attrs:
        pkgs.rustPlatform.buildRustPackage (
          config.wayShell.rustBuildArgs
          // {
            pname = "way-shell-${name}";
            doCheck = false;
            installPhase = ''
              runHook preInstall
              mkdir -p "$out"
              runHook postInstall
            '';
          }
          // attrs
        );
      applicationSmoke =
        backend:
        pkgs.runCommand "way-shell-native-${backend}-application-check"
          (
            compositorEnvironment
            // {
              nativeBuildInputs = [
                pkgs.sway
                pkgs.niri
                pkgs.dbus
              ];
            }
          )
          ''
            set -euo pipefail
            mkdir -p "$out" "$TMPDIR/home"
            HOME="$TMPDIR/home" sh ${../tests/application-smoke.sh} \
              ${package.testHelpers}/bin/application-smoke \
              ${package}/bin/way-shell ${package}/bin/way-sh \
              ${package}/share/gsettings-schemas/${package.name}/glib-2.0/schemas \
              ${backend} "$out"
          '';
      coreChecks = {
        check-format-rs =
          pkgs.runCommand "way-shell-check-format-rs"
            {
              nativeBuildInputs = [
                pkgs.cargo
                pkgs.rustfmt
              ];
            }
            ''
              export HOME="$TMPDIR"
              cd ${config.wayShell.source}
              cargo fmt --all --check
              mkdir -p "$out"
            '';
        clippy-rs = rustCheck "clippy-rs" {
          nativeBuildInputs = config.wayShell.rustBuildArgs.nativeBuildInputs ++ [ pkgs.clippy ];
          buildPhase = ''
            runHook preBuild
            cargo clippy --workspace --all-targets --frozen --jobs 1 -- -D warnings
            runHook postBuild
          '';
        };
        native-package = pkgs.runCommand "way-shell-native-package-check" { } ''
          set -x
          mkdir -p "$out"
          test -x ${package}/bin/way-shell
          test -x ${package}/bin/way-sh
          test ! -e ${package}/bin/schema-probe
          test ! -e ${package}/bin/application-smoke
          test -x ${package.testHelpers}/bin/application-smoke
          test "$(head -c 4 ${package.testHelpers}/bin/application-smoke)" = "$(printf '\177ELF')"
          test -s ${package}/share/licenses/way-shell/LICENSE
          test -s ${package}/share/gsettings-schemas/${package.name}/glib-2.0/schemas/gschemas.compiled
          grep -Fx 'ExecStart=${package}/bin/way-shell' ${package}/lib/systemd/user/way-shell.service
          grep -F '.way-shell-wrapped' ${package}/bin/way-shell
          env -i HOME="$TMPDIR" ${package}/bin/way-shell --help > "$out/help.txt"
        '';
        native-schemas = pkgs.runCommand "way-shell-native-schemas-check" { } ''
          set -x
          mkdir -p "$out" empty home
          # Reuse the installed application's wrapper verbatim up to exec.
          # Only substitute its final executable with the Rust/GIO schema probe.
          test -x ${schemaProbe}/bin/schema-probe
          # The helper must not add its own wrapper environment to this test.
          test "$(head -c 4 ${schemaProbe}/bin/schema-probe)" = "$(printf '\177ELF')"
          grep -q '^exec ' ${package}/bin/way-shell
          grep -F '${lib.getLib pkgs.dconf}/lib/gio/modules' ${package}/bin/way-shell
          sed '/^exec /,$d' ${package}/bin/way-shell > probe-wrapper
          echo 'exec ${schemaProbe}/bin/schema-probe ${lib.escapeShellArgs config.wayShell.schemaIds}' >> probe-wrapper
          env -i HOME="$PWD/home" XDG_DATA_HOME="$PWD/empty" XDG_DATA_DIRS="$PWD/empty" \
            GSETTINGS_BACKEND=memory ${pkgs.bash}/bin/bash ./probe-wrapper > "$out/schemas.txt"
        '';
      };
      integrationChecks = {
        native-components = rustCheck "native-components" (
          compositorEnvironment
          // {
            buildPhase = ''
              runHook preBuild
              cargo build -p way-shell --examples --frozen --jobs 1
              cargo test -p way-shell --lib --no-run --message-format=json --frozen --jobs 1 > "$TMPDIR/runtime-test-artifact.json"
              export WAY_SHELL_RUNTIME_TEST_BINARY=$(sed -n 's/.*"executable":"\([^"]*\)".*/\1/p' "$TMPDIR/runtime-test-artifact.json")
              test -x "$WAY_SHELL_RUNTIME_TEST_BINARY"
              runHook postBuild
            '';
            doCheck = true;
            nativeCheckInputs = [
              pkgs.pipewire
              pkgs.wireplumber
              pkgs.sway
              pkgs.niri
              pkgs.dbus
            ];
            # Keep execution beside compilation: probes embed build-time fixture
            # and schema paths which would not survive in an exported binary.
            checkPhase = ''
              runHook preCheck
              sh tests/audio-smoke.sh target/debug/examples/audio-smoke
              sh tests/component-smoke.sh target/debug/examples
              G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh tests/runtime-smoke.sh
              G_DEBUG=fatal-warnings GSK_RENDERER=cairo sh tests/wayland-component.sh tests/runtime-smoke.sh niri
              runHook postCheck
            '';
          }
        );
        native-application-sway = applicationSmoke "sway";
        native-application-niri = applicationSmoke "niri";
      };
    in
    {
      checks = coreChecks;
      packages =
        coreChecks
        // integrationChecks
        // {
          check-integration = pkgs.linkFarm "way-shell-integration-checks" integrationChecks;
        };
    };
}
