{ pkgs }:
pkgs.vmTools.override {
  customQemu = pkgs.writeShellScript "way-shell-qemu" ''
    # Guest distributions need CPU features beyond QEMU's default CPU. Require
    # KVM and retain readable serial output in Nix logs and successful outputs.
    set +e
    set -o pipefail
    ${pkgs.qemu_kvm}/bin/qemu-system-x86_64 \
      -enable-kvm -cpu host "$@" -nic none -monitor none \
      -serial stdio 2>&1 \
      | ${pkgs.coreutils}/bin/stdbuf -oL ${pkgs.coreutils}/bin/tr -d '\r' \
      | ${pkgs.coreutils}/bin/tee guest-console.log
    status=$?
    if [ -d "$out" ]; then
      ${pkgs.coreutils}/bin/mkdir -p "$out/logs"
      ${pkgs.coreutils}/bin/cp guest-console.log "$out/logs/guest-console.log"
    fi
    exit "$status"
  '';
}
