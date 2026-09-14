{ pkgs }:
{
  # The only supported-release list. Outputs, checks and update targets derive
  # from these keys; each entry also requires locks/<release>.json.
  releases = {
    "43" = {
      baseUrl = "https://dl.fedoraproject.org/pub/fedora/linux/releases/43/Everything/x86_64/os";
      powerProvider = "power-profiles-daemon";
      extraPackages = [ "power-profiles-daemon" ];
    };
    "44" = {
      baseUrl = "https://dl.fedoraproject.org/pub/fedora/linux/releases/44/Everything/x86_64/os";
      powerProvider = "tuned-ppd";
      # The pinned closure generator does not resolve RPM's conditional
      # requirements; image validation identified these additional providers.
      extraPackages = [
        "tuned-ppd"
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
        "python3"
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
        "systemd"
        "dbus-broker"
        "cmake-rpm-macros"
        "systemd-rpm-macros"
        "binutils"
      ]
    )
  );
}
