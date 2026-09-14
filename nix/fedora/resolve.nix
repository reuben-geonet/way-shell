{ nixpkgs, request }:
let
  pkgs = import nixpkgs { system = "x86_64-linux"; };
  config = builtins.fromJSON (builtins.readFile request);
in
pkgs.vmTools.rpmClosureGenerator {
  name = "fedora-${config.release}-dependencies";
  inherit (config) packages archs;
  packagesLists = map (
    repo:
    pkgs.fetchurl {
      inherit (repo.primary) url sha256;
    }
  ) config.repositories;
  urlPrefixes = map (repo: repo.baseUrl) config.repositories;
}
