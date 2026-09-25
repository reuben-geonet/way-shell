{ pkgs, manifest, size ? 16384 }:
let
  vm = import ../vm-tools.nix { inherit pkgs; };
  bootstrap = pkgs.fetchurl { inherit (manifest.bootstrap) url sha256; };
  packages = map (package: pkgs.fetchurl { inherit (package) url sha256; }) manifest.packages;
in
vm.runInLinuxVM (pkgs.stdenv.mkDerivation {
  name = "arch-${manifest.snapshot}-${manifest.kind}-image";
  memSize = 2048;
  preVM = vm.createEmptyImage {
    inherit size;
    fullName = "Arch Linux ${manifest.snapshot} ${manifest.kind}";
  };
  buildCommand = ''
    ${vm.defaultCreateRootFS}
    tar -I ${pkgs.zstd}/bin/zstd -xf ${bootstrap} -C /mnt --strip-components=1
    mkdir -p /mnt/nix/store /mnt/proc /mnt/dev
    ${pkgs.util-linux}/bin/mount --bind /nix/store /mnt/nix/store
    ${pkgs.util-linux}/bin/mount --bind /dev /mnt/dev
    ${pkgs.util-linux}/bin/mount -t proc proc /mnt/proc
    ${pkgs.coreutils}/bin/env -i PATH=/usr/bin:/bin HOME=/root \
      ${pkgs.coreutils}/bin/chroot /mnt /usr/bin/pacman -U --noconfirm --needed --nodeps ${pkgs.lib.escapeShellArgs packages}
    rm /mnt/.debug
    ${pkgs.util-linux}/bin/umount /mnt/proc /mnt/dev /mnt/nix/store /mnt
  '';
})
