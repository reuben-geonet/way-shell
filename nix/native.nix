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
        way-shell = pkgs.rustPlatform.buildRustPackage (
          config.wayShell.rustBuildArgs
          // {
            pname = "way-shell";
            outputs = [
              "out"
              "testHelpers"
            ];
            wrapGAppsInOutputs = [ "out" ];
            nativeBuildInputs = config.wayShell.rustBuildArgs.nativeBuildInputs ++ [
              # A shell wrapper lets the schema check reuse the exact environment
              # of the installed application with a GLib probe as its executable.
              (pkgs.wrapGAppsHook4.override { makeWrapper = pkgs.makeShellWrapper; })
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
            nativeCheckInputs = [
              pkgs.pipewire
              pkgs.wireplumber
              pkgs.dbus
            ];
            checkPhase = ''
              runHook preCheck
              cargo test --workspace --frozen --jobs 1
              runHook postCheck
            '';
            WAY_SHELL_TEST_EGL_VENDOR = "${pkgs.mesa}/share/glvnd/egl_vendor.d/50_mesa.json";
            FONTCONFIG_FILE = "${pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts ]; }}";
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
          }
        );
      };
    };
}
