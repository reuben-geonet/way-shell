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
            nativeBuildInputs = config.wayShell.rustBuildArgs.nativeBuildInputs ++ [ pkgs.wrapGAppsHook4 ];
            # Keep the artifact path used by the shared installer.
            buildPhase = ''
              runHook preBuild
              cargo build --workspace --bins --release --frozen --jobs 1
              runHook postBuild
            '';
            # Keep Cargo tests separate from package builds.
            doCheck = false;
            installPhase = ''
              runHook preInstall
              PREFIX="$out" CARGO_ARTIFACT_DIR=target/release \
                DEPENDENCY_LICENSES=${config.wayShell.dependencyLicenses} \
                sh scripts/install.sh
              runHook postInstall
            '';
            postInstall = ''
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
