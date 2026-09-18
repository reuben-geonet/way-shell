{
  perSystem =
    {
      config,
      pkgs,
      ...
    }:
    let
      package = config.packages.way-shell;
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
        native-package = pkgs.runCommand "way-shell-native-package-check" { } ''
          mkdir -p "$out"
          env -i HOME="$TMPDIR" ${package}/bin/way-shell --help > "$out/way-shell-help.txt"
          env -i HOME="$TMPDIR" ${package}/bin/way-sh --help > "$out/way-sh-help.txt"
          test -s ${package}/share/licenses/way-shell/LICENSE
          grep -Fx 'ExecStart=${package}/bin/way-shell' ${package}/lib/systemd/user/way-shell.service
          env -i HOME="$TMPDIR" GSETTINGS_BACKEND=memory ${pkgs.glib.bin}/bin/gsettings \
            --schemadir ${package}/share/gsettings-schemas/${package.name}/glib-2.0/schemas \
            list-recursively org.ldelossa.way-shell.system > "$out/settings.txt"
        '';
      };
    in
    {
      checks = coreChecks;
      packages = coreChecks // {
        clippy-rs = pkgs.rustPlatform.buildRustPackage (
          config.wayShell.rustBuildArgs
          // {
            pname = "way-shell-clippy-rs";
            nativeBuildInputs = config.wayShell.rustBuildArgs.nativeBuildInputs ++ [ pkgs.clippy ];
            doCheck = false;
            buildPhase = ''
              runHook preBuild
              cargo clippy --workspace --all-targets --frozen --jobs 1 -- -D warnings
              runHook postBuild
            '';
            installPhase = ''
              runHook preInstall
              mkdir -p "$out"
              runHook postInstall
            '';
          }
        );
      };
    };
}
