{ pkgs, manifest }:
let
  vm = import ./vm-tools.nix { inherit pkgs; };
in
vm.fillDiskWithRPMs {
  name = "fedora-${manifest.release}-${manifest.arch}";
  fullName = "Fedora ${manifest.release} (${manifest.arch})";
  # Rust's GTK bindings and build/test artifacts need room beyond the RPM
  # toolchain closure. This is a sparse virtual disk, not reserved host RAM.
  size = 16384;
  memSize = 2048;
  unifiedSystemDir = true;
  rpms = map (rpm: pkgs.fetchurl { inherit (rpm) url sha256; }) manifest.rpms;
}
