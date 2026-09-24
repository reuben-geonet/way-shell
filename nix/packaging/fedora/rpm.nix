{
  perSystem =
    {
      config,
      pkgs,
      lib,
      ...
    }:
    let
      cfg = config.wayShell;
      releases = builtins.attrNames cfg.fedora.releases;
      mkArchive =
        source:
        assert lib.assertMsg (lib.hasInfix "\nVersion: ${cfg.version}\n" (
          builtins.readFile ../../../way-shell.spec
        )) "Cargo and RPM versions disagree";
        pkgs.runCommand "way-shell-${cfg.version}.tar.gz" { } ''
          mkdir source
          cp -r ${source}/. source/
          cp ${../../../way-shell.spec} source/way-shell.spec
          chmod -R u+w source
          cp -rL ${cfg.cargoVendor} source/vendor
          cp -r ${cfg.dependencyLicenses} source/dependency-licenses
          mkdir -p source/.cargo
          sed 's/directory = "cargo-vendor-dir"/directory = "vendor"/' \
            ${cfg.cargoVendor}/.cargo/config.toml > source/.cargo/config.toml
          ! grep -F /nix/store source/.cargo/config.toml
          tar --sort=name --mtime=@1 --owner=0 --group=0 --numeric-owner \
            --mode='u+rwX,go+rX,go-w' --format=gnu \
            --transform='s,^\.,way-shell-${cfg.version},' \
            -C source -cf - . | gzip -n > "$out"
        '';
      archive = mkArchive cfg.source;
      dependencyArchive = mkArchive cfg.fedoraDummySource;
      workspacePackages = map (
        member: (builtins.fromTOML (builtins.readFile (../../../. + "/${member}/Cargo.toml"))).package.name
      ) (builtins.fromTOML (builtins.readFile ../../../Cargo.toml)).workspace.members;
      adapter = pkgs.writeText "way-shell-cargo-artifacts.sh" ''
        set -eo pipefail
        case "$1" in
          import)
            source ${cfg.craneLib.inheritCargoArtifactsHook}/nix-support/setup-hook
            # Copy the full target tree, retaining Fedora's release -> rpm link.
            inheritCargoArtifacts "$2" target
            ;;
          export)
            # Cargo's public clean command removes workspace binaries, libraries
            # and build scripts without rewriting third-party fingerprints.
            cargo clean --offline --profile rpm ${
              lib.concatMapStringsSep " " (name: "--package ${name}") workspacePackages
            }
            source ${cfg.craneLib.installCargoArtifactsHook}/nix-support/setup-hook
            # Crane eb35abda's compressed export resets mtimes to epoch 1,
            # older than Fedora headers tracked by bindgen. Use its directory
            # export before Nix normalizes the files, then preserve timestamps
            # in the archive. Remove this wrapper when Crane's compressed
            # exporter supports retaining timestamps. Never edit fingerprints.
            # Stay on the guest disk: /tmp is RAM-backed in vmTools.
            staging=$(mktemp -d "$PWD/.cargo-artifacts.XXXXXX")
            prepareAndInstallCargoArtifactsDir "$staging" target use-symlink
            tar --sort=name --owner=0 --group=0 --numeric-owner \
              -C "$staging/target" -cf - . | zstd -T1 -o "$2/target.tar.zst"
            ;;
          *) exit 2 ;;
        esac
      '';
      dependencyBuilds = lib.genAttrs releases (release: mkBuild release true);
      mkBuild =
        release: dependenciesOnly:
        cfg.vmTools.buildRPM (
          {
            name = "way-shell-${cfg.version}-fedora-${release}-${if dependenciesOnly then "deps" else "build"}";
            src = if dependenciesOnly then dependencyArchive else archive;
            diskImage = cfg.images.${release};
            diskImageFormat = "qcow2";
            memSize = 3072;
            enableParallelBuilding = false;
            preBuild = ''
              mkdir -p "$out/logs"
              exec > >(tee "$out/logs/build.log") 2>&1
            '';
            # vmTools' default /tmp/rpmout is on a RAM-backed filesystem.
            # Keep Cargo artifacts on the sparse guest disk instead.
            buildPhase = ''
              runHook preBuild
              # stdenv contributes Nix data paths; guest libraries such as Glycin
              # must discover the configuration installed by Fedora packages.
              export CARGO_TERM_VERBOSE=true
              export NIX_BUILD_CORES=1
              export XDG_DATA_DIRS=/usr/local/share:/usr/share
              srcName="$(rpmspec --srpm -q --qf '%{source}' *.spec)"
              cp "$src" "$srcName"
              rpmout=/var/tmp/way-shell-rpm
              mkdir -p "$rpmout"/{BUILD,BUILDROOT,SPECS,SOURCES,RPMS,SRPMS}
              export CARGO_HOME="$rpmout/cargo-home"
              df -h /tmp "$rpmout"
              rustc --version
              cargo --version
              rpmbuild --define "_topdir $rpmout" --define "_smp_mflags -j1" \
                ${lib.optionalString dependenciesOnly ''--define "way_shell_artifacts_export bash ${adapter} export $out"''} \
                ${
                  lib.optionalString (
                    !dependenciesOnly
                  ) ''--define "way_shell_artifacts_import bash ${adapter} import ${dependencyBuilds.${release}}"''
                } \
                ${if dependenciesOnly then "-tc" else "-ta"} "$srcName"
              runHook postBuild
            '';
          }
          // lib.optionalAttrs dependenciesOnly {
            # Export happens in %build, before RPM can clean the target tree.
            installPhase = ''test -s "$out/target.tar.zst"'';
            dontFixup = true;
          }
        );
      builds = lib.genAttrs releases (release: mkBuild release false);
    in
    {
      packages =
        lib.mapAttrs' (release: output: lib.nameValuePair "rpm-fedora-${release}" output) builds
        // lib.mapAttrs' (
          release: output: lib.nameValuePair "cargo-deps-fedora-${release}" output
        ) dependencyBuilds
        // {
          rpms = pkgs.runCommand "way-shell-rpms" { } ''
            mkdir -p "$out/rpms" "$out/logs"
            ${lib.concatMapStringsSep "\n" (release: ''
              cp -r ${builds.${release}}/rpms/* "$out/rpms/"
              cp -r ${builds.${release}}/logs "$out/logs/fedora-${release}-x86_64"
            '') releases}
          '';
        };
    };
}
