{ inputs, ... }:
{
  perSystem =
    {
      config,
      pkgs,
      lib,
      ...
    }:
    let
      fedora = import ./fedora/config.nix { inherit pkgs; };
      readManifests =
        suffix: packages:
        lib.mapAttrs (
          release: value:
          let
            lock = builtins.fromJSON (builtins.readFile (./fedora/locks + "/${release}${suffix}.json"));
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
      manifests = readManifests "" fedora.packages;
    in
    {
      wayShell = {
        inherit fedora manifests;
        runtimeManifests = readManifests "-runtime" fedora.runtimePackages;
        serviceManifests = readManifests "-services" null;
        vmTools = import ./fedora/vm-tools.nix { inherit pkgs; };
        images = lib.mapAttrs (_: manifest: import ./fedora/image.nix { inherit pkgs manifest; }) manifests;
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
              exec python3 ${./fedora/update-locks.py} \
                --config ${pkgs.writeText "fedora-config.json" (builtins.toJSON fedora)} \
                --nixpkgs ${inputs.nixpkgs} --revision ${inputs.nixpkgs.rev} \
                --tools ${./fedora} --spec ${../way-shell.spec} "$@"
            '';
          }
        );
      };
    };
}
