{
  imports = [ ./fedora ./arch ];
  perSystem = { config, pkgs, lib, ... }: {
    apps.bump-version = {
      type = "app";
      meta.description = "Update Cargo, RPM and Arch release versions";
      program = lib.getExe (pkgs.writeShellApplication {
        name = "bump-version";
        runtimeInputs = [ pkgs.python3 ];
        text = ''exec python3 ${../../scripts/bump-version.py} "$@"'';
      });
    };
    packages.packages = pkgs.runCommand "way-shell-packages" { } ''
      mkdir -p "$out"
      cp -r ${config.packages.fedora-packages}/rpms "$out/"
      mkdir -p "$out/arch"
      cp ${config.packages.package-arch}/*.pkg.tar.zst "$out/arch/"
    '';
  };
}
