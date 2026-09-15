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
        "rpm-plugin-selinux"
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
        "rust"
        "cargo"
        "clang-devel"
        "libadwaita-devel"
        "gtk4-layer-shell-devel"
        "wireplumber-devel"
        "NetworkManager-libnm-devel"
        "pipewire-devel"
        "pipewire"
        "pipewire-pulseaudio"
        "pulseaudio-libs-devel"
        "wayland-devel"
        "glib2-devel"
        "redhat-rpm-config"
        "NetworkManager"
        "wireplumber"
        "upower"
        "systemd"
        "dbus-broker"
        "dbus-daemon"
        "binutils"
        # These conditional requirements belong to the image's native tooling,
        # even though the application no longer uses systemd macros or Python.
        "systemd-rpm-macros"
        "python3-setuptools"
        # Installed-package smoke checks run both compositors as a private user.
        "sway"
        # The pinned resolver selects this provider of the sway-config virtual
        # package. Selecting another provider would install conflicting configs.
        "sway-config-upstream"
        "niri"
        "util-linux"
        "shadow-utils"
        "dejavu-sans-fonts"
        "fontconfig"
        "mesa-libEGL"
        "mesa-dri-drivers"
        # util-linux has conditional PAM requirements with systemd. The pinned
        # resolver needs both providers explicitly for either Fedora release.
        "pam"
        "authselect-libs"
      ]
    )
  );
}
