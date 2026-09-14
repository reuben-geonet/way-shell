{ pkgs, manifest }:
let
  vm = import ./vm-tools.nix { inherit pkgs; };
in
vm.fillDiskWithRPMs {
  name = "fedora-${manifest.release}-${manifest.arch}";
  fullName = "Fedora ${manifest.release} (${manifest.arch})";
  size = 8192;
  memSize = 2048;
  unifiedSystemDir = true;
  rpms = map (rpm: pkgs.fetchurl { inherit (rpm) url sha256; }) manifest.rpms;
}
