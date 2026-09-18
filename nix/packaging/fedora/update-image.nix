{
  nixpkgs,
  lock,
  size ? 16384,
}:
import ./image.nix {
  pkgs = import nixpkgs { system = "x86_64-linux"; };
  manifest = builtins.fromJSON (builtins.readFile lock);
  inherit size;
}
