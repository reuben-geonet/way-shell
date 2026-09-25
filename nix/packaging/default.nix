{
  imports = [ ./fedora ./arch ];
  perSystem = { config, pkgs, lib, ... }: {
    packages.packages = pkgs.runCommand "way-shell-packages" { } ''
      mkdir -p "$out"
      cp -r ${config.packages.fedora-packages}/rpms "$out/"
      mkdir -p "$out/arch"
      cp ${config.packages.package-arch}/*.pkg.tar.zst "$out/arch/"
    '';
  };
}
