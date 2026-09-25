{ config, pkgs }:
let
  vm = import ../packaging/vm-tools.nix { inherit pkgs; };
  image = import ../packaging/arch/image.nix {
    inherit pkgs;
    manifest = config.wayShell.arch.runtimeLock;
    size = 4096;
  };
in
vm.runInLinuxImage (pkgs.runCommand "way-shell-arch-installation" {
  diskImage = image;
  diskImageFormat = "qcow2";
  memSize = 2048;
} ''
  set -euo pipefail
  export PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
  unset GSETTINGS_SCHEMA_DIR GIO_EXTRA_MODULES GIO_MODULE_DIR LD_LIBRARY_PATH LD_PRELOAD
  export HOME=/tmp/way-shell-check XDG_DATA_HOME=/tmp/way-shell-check/data
  export XDG_DATA_DIRS=/usr/local/share:/usr/share GSETTINGS_BACKEND=memory
  mkdir -p "$HOME" "$XDG_DATA_HOME" "$out/logs"
  exec > >(tee "$out/logs/check.log") 2>&1
  packages=(${config.packages.package-arch}/*.pkg.tar.zst)
  test "''${#packages[@]}" -eq 1
  pacman -U --noconfirm "''${packages[0]}"
  test -f /usr/share/glib-2.0/schemas/org.ldelossa.way-shell.gschema.xml
  test -f /usr/lib/systemd/user/way-shell.service
  /usr/bin/way-shell --help
  /usr/bin/way-sh --help
  gsettings list-recursively org.ldelossa.way-shell.system > "$out/logs/settings.txt"
  pacman -R --noconfirm way-shell
  ! pacman -Q way-shell
'')
