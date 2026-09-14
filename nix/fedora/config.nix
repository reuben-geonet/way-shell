{ pkgs }:
{
  # The only supported-release list. Outputs, checks and update targets derive
  # from these keys; each entry also requires locks/<release>.json.
  releases = {
    "43" = {
      baseUrl = "https://dl.fedoraproject.org/pub/fedora/linux/releases/43/Everything/x86_64/os";
    };
    "44" = {
      baseUrl = "https://dl.fedoraproject.org/pub/fedora/linux/releases/44/Everything/x86_64/os";
      # The pinned closure generator does not resolve RPM's conditional
      # requirements; image validation identified these additional providers.
      extraPackages = [
        "python3-setuptools"
        "rpm-plugin-selinux"
        "pam"
        "authselect-libs"
      ];
    };
  };
  arch = "x86_64";
  archs = [
    "noarch"
    "x86_64"
  ];
  packages = pkgs.lib.sort builtins.lessThan (
    pkgs.lib.unique (
      pkgs.vmTools.commonFedoraPackages
      ++ [
        "gpgverify"
        "libadwaita-devel"
        "gtk4-layer-shell-devel"
        "upower-devel"
        "wireplumber-devel"
        "json-glib-devel"
        "NetworkManager-libnm-devel"
        "pipewire-devel"
        "pulseaudio-libs-devel"
        "wayland-devel"
        "wayland-protocols-devel"
        "glib2-devel"
        "meson"
        "cmake"
        "gtk-doc"
        "redhat-rpm-config"
        "NetworkManager"
        "wireplumber"
        "upower"
        "power-profiles-daemon"
        "systemd"
        "dbus-broker"
        "cmake-rpm-macros"
        "systemd-rpm-macros"
        "binutils"
      ]
    )
  );
}
