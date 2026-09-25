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
    packages = {
      release-source-files = pkgs.runCommand "way-shell-release-source-files" { } ''
        mkdir -p "$out/release"
        cp ${config.packages.release-archive} "$out/release/way-shell-${config.wayShell.version}.tar.gz"
        cp ${config.packages.release-pkgbuild} "$out/release/PKGBUILD"
      '';
      packages = pkgs.runCommand "way-shell-packages" { } ''
        mkdir -p "$out/release" "$out/logs"
        cp ${config.packages.release-source-files}/release/* "$out/release/"
        ${lib.concatMapStringsSep "\n" (release: ''
          cp ${config.packages."package-fedora-${release}"}/release/*.rpm "$out/release/"
          cp -r ${config.packages."package-fedora-${release}"}/logs "$out/logs/fedora-${release}"
        '') (builtins.attrNames config.wayShell.fedora.releases)}
        cp ${config.packages.package-arch}/release/*.pkg.tar.zst "$out/release/"
        cp -r ${config.packages.package-arch}/logs "$out/logs/arch"
        cd "$out/release"
        sha256sum -- * > SHA256SUMS
      '';
    };
  };
}
