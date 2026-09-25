{
  arch = "x86_64";
  buildPackages = [
    "base-devel" "rust" "clang" "pkgconf" "gtk4" "libadwaita"
    "gtk4-layer-shell" "wireplumber" "libpulse" "libnm" "pipewire"
    "systemd-libs" "glib2" "pacman" "gawk" "coreutils"
  ];
  runtimePackages = [
    "pacman" "glib2" "gtk4" "libadwaita" "gtk4-layer-shell"
    "wireplumber" "libpulse" "libnm" "pipewire" "systemd-libs"
  ];
}
