{
  config,
  pkgs,
  lib,
}:
let
  cfg = config.wayShell;
in
lib.mapAttrs' (
  release: _:
  let
    runtimeImage = import ../fedora/image.nix {
      inherit pkgs;
      manifest = cfg.runtimeManifests.${release};
      size = 4096;
    };
    # Reuse the build lock's library packages as candidates; DNF installs only
    # what the RPM actually needs into the separate runtime image.
    rpms = lib.unique (
      cfg.manifests.${release}.rpms
      ++ cfg.runtimeManifests.${release}.rpms
      ++ cfg.serviceManifests.${release}.rpms
    );
    repository =
      pkgs.runCommand "fedora-${release}-installation-repository"
        {
          nativeBuildInputs = [
            pkgs.createrepo_c
            pkgs.rpm
          ];
        }
        ''
          # DNF can silently skip unavailable recommendations; keep their lock current.
          rpmspec --define 'fedora ${release}' --define 'dist .fc${release}' \
            --query --recommends ${../../way-shell.spec} | sort > recommendations
          diff -u ${
            pkgs.writeText "fedora-${release}-recommendations" (
              lib.concatMapStrings (package: package + "\n") cfg.serviceManifests.${release}.requestedPackages
            )
          } recommendations
          mkdir -p "$out"
          ${lib.concatMapStringsSep "\n" (rpm: ''
            ln -s ${pkgs.fetchurl { inherit (rpm) url sha256; }} "$out/${baseNameOf rpm.url}"
          '') rpms}
          createrepo_c "$out"
        '';
    test = cfg.vmTools.runInLinuxImage (
      pkgs.runCommand "way-shell-fedora-${release}-installation"
        {
          diskImage = runtimeImage;
          diskImageFormat = "qcow2";
          memSize = 2048;
        }
        ''
          export RPM_DIRECTORY=${
            config.packages."rpm-fedora-${release}"
          }/rpms/fedora-${release}-${cfg.fedora.arch}
          export INSTALL_REPOSITORY=${repository}
          source ${./fedora.sh}
        ''
    );
  in
  lib.nameValuePair "test-install-fedora-${release}" test
) cfg.fedora.releases
