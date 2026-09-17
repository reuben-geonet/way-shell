{ self, ... }:
{
  perSystem =
    {
      config,
      pkgs,
      lib,
      system,
      ...
    }:
    let
      cfg = config.wayShell;
      releases = builtins.attrNames cfg.fedora.releases;
      archive =
        assert lib.assertMsg (lib.hasInfix "\nVersion: ${cfg.version}\n" (
          builtins.readFile ../way-shell.spec
        )) "Cargo and RPM versions disagree";
        pkgs.runCommand "way-shell-${cfg.version}.tar.gz" { } ''
          mkdir source
          cp -r ${cfg.source}/. source/
          cp ${../way-shell.spec} source/way-shell.spec
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
      builds = lib.genAttrs releases (
        release:
        cfg.vmTools.buildRPM {
          name = "way-shell-${cfg.version}-fedora-${release}-build";
          src = archive;
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
            export XDG_DATA_DIRS=/usr/local/share:/usr/share
            srcName="$(rpmspec --srpm -q --qf '%{source}' *.spec)"
            cp "$src" "$srcName"
            rpmout=/var/tmp/way-shell-rpm
            mkdir -p "$rpmout"/{BUILD,BUILDROOT,SPECS,SOURCES,RPMS,SRPMS}
            export CARGO_HOME="$rpmout/cargo-home"
            df -h /tmp "$rpmout"
            rustc --version
            cargo --version
            rpmbuild --define "_topdir $rpmout" --define "_smp_mflags -j1" -ta "$srcName"
            runHook postBuild
          '';
        }
      );
      rpmApp = pkgs.writeShellApplication {
        name = "rpm";
        runtimeInputs = [ pkgs.nix ];
        text = ''
          fail() { echo "rpm: $*" >&2; exit 2; }
          target=rpms
          case "$#" in
            0) ;;
            1)
              case "$1" in
                --list) printf '%s\n' ${lib.escapeShellArgs releases}; exit 0 ;;
                *) fail "invalid or incomplete argument: $1" ;;
              esac ;;
            2)
              [ "$1" = --fedora ] || fail "expected --fedora VERSION"
              case "$2" in
                ${lib.concatStringsSep "|" releases}) target="rpm-fedora-$2" ;;
                *) fail "unsupported Fedora release: $2 (use --list)" ;;
              esac ;;
            *) fail "too many arguments" ;;
          esac
          # Embed this flake's immutable snapshot, including dirty tracked edits.
          # Never resolve '.' at runtime: callers may be in a different checkout.
          exec nix build --no-update-lock-file \
            --option allow-import-from-derivation false --print-build-logs \
            "path:${self}#packages.${system}.$target"
        '';
      };
    in
    {
      wayShell = { inherit rpmApp; };
      packages =
        lib.mapAttrs' (release: output: lib.nameValuePair "rpm-fedora-${release}" output) builds
        // {
          rpms = pkgs.runCommand "way-shell-rpms" { } ''
            mkdir -p "$out/rpms" "$out/logs"
            ${lib.concatMapStringsSep "\n" (release: ''
              cp -r ${builds.${release}}/rpms/* "$out/rpms/"
              cp -r ${builds.${release}}/logs "$out/logs/fedora-${release}-x86_64"
            '') releases}
          '';
        };
      apps.rpm = {
        type = "app";
        program = lib.getExe rpmApp;
        meta.description = "Build RPMs for configured Fedora releases";
      };
    };
}
