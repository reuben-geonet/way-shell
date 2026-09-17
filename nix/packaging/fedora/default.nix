{ inputs, ... }:
{
  imports = [ ./rpm.nix ];

  perSystem =
    {
      config,
      pkgs,
      lib,
      ...
    }:
    let
      fedora = import ./config.nix { inherit pkgs; };
      readManifests =
        kind: packages:
        lib.mapAttrs (
          release: value:
          let
            lock = builtins.fromJSON (builtins.readFile (./locks + "/${release}-${kind}.json"));
          in
          assert lib.assertMsg (
            lock.format == 1
            && lock.release == release
            && lock.arch == fedora.arch
            && (packages == null || lock.requestedPackages == lib.sort builtins.lessThan (lib.unique packages))
            && (map (repo: repo.baseUrl) lock.repositories) == [ value.baseUrl ]
          ) "Fedora ${release} lock is stale; run nix run .#update-fedora-locks";
          lock
        ) fedora.releases;
      manifests = readManifests "build" fedora.packages;
    in
    {
      wayShell = {
        inherit fedora manifests;
        runtimeManifests = readManifests "runtime" fedora.runtimePackages;
        serviceManifests = readManifests "services" null;
        vmTools = import ./vm-tools.nix { inherit pkgs; };
        images = lib.mapAttrs (_: manifest: import ./image.nix { inherit pkgs manifest; }) manifests;
      };
      apps.update-fedora-locks = {
        type = "app";
        meta.description = "Regenerate and validate all configured Fedora dependency locks";
        program = lib.getExe (
          pkgs.writeShellApplication {
            name = "update-fedora-locks";
            runtimeInputs = [
              pkgs.nix
              pkgs.python3
              pkgs.dnf5
              pkgs.rpm
              pkgs.zstd
            ];
            text = ''
              exec python3 ${./update-locks.py} \
                --config ${pkgs.writeText "fedora-config.json" (builtins.toJSON fedora)} \
                --nixpkgs ${inputs.nixpkgs} --revision ${inputs.nixpkgs.rev} \
                --tools ${./.} --spec ${../../../way-shell.spec} "$@"
            '';
          }
        );
      };
    };
}
