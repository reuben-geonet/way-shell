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
      mkdir -p "$out/rpms"
      ${lib.concatMapStringsSep "\n" (release: ''
        cp -r ${config.packages."package-fedora-${release}"}/rpms/* "$out/rpms/"
      '') (builtins.attrNames config.wayShell.fedora.releases)}
      mkdir -p "$out/arch"
      cp ${config.packages.package-arch}/*.pkg.tar.zst "$out/arch/"
    '';
  };
}
