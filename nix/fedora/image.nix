{
  pkgs,
  manifest,
  size ? 16384,
}:
let
  vm = import ./vm-tools.nix { inherit pkgs; };
in
vm.fillDiskWithRPMs {
  name = "fedora-${manifest.release}-${manifest.arch}";
  fullName = "Fedora ${manifest.release} (${manifest.arch})";
  # Build images need room for Rust's GTK bindings and artifacts; runtime
  # images request less space. The virtual disk is sparse, not reserved RAM.
  inherit size;
  memSize = 2048;
  unifiedSystemDir = true;
  rpms = map (rpm: pkgs.fetchurl { inherit (rpm) url sha256; }) manifest.rpms;
}
