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
      archive = pkgs.runCommand "way-shell-${cfg.version}.tar.gz" { } ''
        mkdir source
        cp -r ${cfg.source}/. source/
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
            # Export the Fedora-built test helper separately from the RPM payload.
            export WAY_SHELL_TEST_ARTIFACTS="$out/test-helpers"
            df -h /tmp "$rpmout"
            rustc --version
            cargo --version
            rpmbuild -vv --define "_topdir $rpmout" -ta "$srcName"
            runHook postBuild
          '';
        }
      );
      installations = lib.genAttrs releases (
        release:
        cfg.vmTools.runInLinuxImage (
          pkgs.runCommand "way-shell-fedora-${release}-installation"
            {
              diskImage = cfg.images.${release};
              diskImageFormat = "qcow2";
              memSize = 2048;
            }
            ''
              export RPM_DIRECTORY=${builds.${release}}/rpms/fedora-${release}-x86_64
              export POWER_PROVIDER=${cfg.fedora.releases.${release}.powerProvider}
              export SCHEMA_PROBE=${builds.${release}}/test-helpers/schema-probe
              export APPLICATION_SMOKE_HELPER=${builds.${release}}/test-helpers/application-smoke
              export APPLICATION_SMOKE_SCRIPT=${../tests/application-smoke.sh}
              export EXPECTED_SCHEMAS=${lib.escapeShellArg (lib.concatStringsSep " " cfg.schemaIds)}
              source ${./tests/fedora-install.sh}
            ''
        )
      );
      verified = lib.genAttrs releases (
        release:
        pkgs.runCommand "way-shell-fedora-${release}-verified" { } ''
          mkdir -p "$out/rpms" "$out/logs/fedora-${release}-x86_64"
          cp -r ${builds.${release}}/rpms/* "$out/rpms/"
          cp -r ${builds.${release}}/logs "$out/logs/fedora-${release}-x86_64/build"
          cp -r ${installations.${release}}/logs "$out/logs/fedora-${release}-x86_64/install"
        ''
      );
      rpmApp = pkgs.writeShellApplication {
        name = "rpm";
        runtimeInputs = [ pkgs.nix ];
        text = ''
          usage() {
            cat <<'USAGE'
          Usage: nix run .#rpm -- [--fedora VERSION | --list | --help]
          With no arguments, build verified RPMs for every supported Fedora release.
          Use --list to see the supported releases. Artifacts are under result/rpms/.
          USAGE
          }
          fail() { echo "rpm: $*" >&2; usage >&2; exit 2; }
          target=rpms
          case "$#" in
            0) ;;
            1)
              case "$1" in
                --list) printf '%s\n' ${lib.escapeShellArgs releases}; exit 0 ;;
                --help) usage; exit 0 ;;
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
      wayShell = { inherit verified rpmApp; };
      packages =
        lib.mapAttrs' (release: output: lib.nameValuePair "rpm-fedora-${release}" output) verified
        // {
          rpms = pkgs.runCommand "way-shell-rpms" { } ''
            mkdir -p "$out/rpms" "$out/logs"
            ${lib.concatMapStringsSep "\n" (release: ''
              cp -r ${verified.${release}}/rpms/* "$out/rpms/"
              cp -r ${verified.${release}}/logs/* "$out/logs/"
            '') releases}
          '';
        };
      apps.rpm = {
        type = "app";
        program = lib.getExe rpmApp;
        meta.description = "Build verified RPMs for configured Fedora releases";
      };
    };
}
