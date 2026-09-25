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
      archive = import ../archive.nix { inherit config pkgs lib; };
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
    in
    {
      packages = lib.mapAttrs' (release: output: lib.nameValuePair "package-fedora-${release}" output) builds;
    };
}
