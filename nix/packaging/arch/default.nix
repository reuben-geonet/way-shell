{ inputs, ... }:
{
  perSystem = { config, pkgs, lib, ... }:
    let
      arch = import ./config.nix;
      readLock = kind: roots:
        let lock = builtins.fromJSON (builtins.readFile (./locks + "/${kind}.json"));
        in assert lib.assertMsg (
          lock.format == 1 && lock.kind == kind && lock.arch == arch.arch
          && lock.requestedPackages == lib.sort builtins.lessThan roots
        ) "Arch ${kind} lock is stale; run nix run .#update-arch-locks"; lock;
      buildLock = readLock "build" arch.buildPackages;
      runtimeLock = readLock "runtime" arch.runtimePackages;
      buildImage = import ./image.nix { inherit pkgs; manifest = buildLock; };
      archive = import ../archive.nix { inherit config pkgs lib; };
      vm = import ../vm-tools.nix { inherit pkgs; };
      pkgbuild = pkgs.runCommand "way-shell-PKGBUILD" { nativeBuildInputs = [ pkgs.coreutils ]; } ''
        substitute ${./PKGBUILD.in} "$out" --replace-fail @SOURCE_SHA256@ "$(sha256sum ${archive} | cut -d' ' -f1)"
      '';
      package = vm.runInLinuxImage (pkgs.stdenv.mkDerivation {
        name = "way-shell-${config.wayShell.version}-arch-package";
        diskImage = buildImage;
        diskImageFormat = "qcow2";
        memSize = 3072;
        buildCommand = ''
          mkdir -p "$out/logs" /var/tmp/way-shell-build
          exec > >(tee "$out/logs/build.log") 2>&1
          cp ${archive} /var/tmp/way-shell-build/way-shell-${config.wayShell.version}.tar.gz
          cp ${pkgbuild} /var/tmp/way-shell-build/PKGBUILD
          chmod -R a+rwX /var/tmp/way-shell-build
          cd /var/tmp/way-shell-build
          HOME=/var/tmp/way-shell-build USER=nobody LOGNAME=nobody \
            setpriv --reuid=65534 --regid=65534 --clear-groups \
              makepkg --noconfirm --nodeps --clean --cleanbuild
          cp ./*.pkg.tar.zst "$out/"
        '';
      });
    in {
      wayShell.arch = { inherit buildLock runtimeLock; };
      packages = {
        package-arch = package;
        release-archive = archive;
        release-pkgbuild = pkgbuild;
      };
      apps.update-arch-locks = {
        type = "app";
        meta.description = "Resolve and validate an Arch Archive snapshot";
        program = lib.getExe (pkgs.writeShellApplication {
          name = "update-arch-locks";
          runtimeInputs = [ pkgs.python3 pkgs.nix pkgs.pacman pkgs.zstd ];
          text = ''
            exec python3 ${./update-locks.py} --config ${pkgs.writeText "arch-config.json" (builtins.toJSON arch)} --nixpkgs ${inputs.nixpkgs} "$@"
          '';
        });
      };
    };
}
