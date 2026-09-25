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
        release-versions = pkgs.runCommand "way-shell-release-versions" { } ''
          grep -Fx 'Version: ${config.wayShell.version}' ${../way-shell.spec}
          grep -Fx 'pkgver=${config.wayShell.version}' ${./packaging/arch/PKGBUILD.in}
          for name in way-sh way-shell way-shell-core way-shell-shortcuts; do
            ${pkgs.python3}/bin/python3 -c 'import sys,tomllib; d=tomllib.load(open(sys.argv[1],"rb")); assert next(p["version"] for p in d["package"] if p["name"] == sys.argv[2]) == sys.argv[3]' \
              ${../Cargo.lock} "$name" ${config.wayShell.version}
          done
          touch "$out"
        '';
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
          grep -Fq 'GNU AFFERO GENERAL PUBLIC LICENSE' ${package}/share/licenses/way-shell/LICENSE
          grep -Fq '13. Remote Network Interaction; Use with the GNU General Public License.' ${package}/share/licenses/way-shell/LICENSE
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
