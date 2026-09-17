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
      # Fedora 44 workaround: the pinned resolver misses RPM's conditional
      # SELinux dependency and util-linux's PAM dependencies with systemd.
      # Remove these entries when it resolves them, then validate the image lock.
      extraPackages = [ "rpm-plugin-selinux" "pam" "authselect-libs" ];
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
        "cargo-rpm-macros"
        "cargo2rpm"
        "clang-devel"
        "libadwaita-devel"
        "gtk4-layer-shell-devel"
        "wireplumber-devel"
        "NetworkManager-libnm-devel"
        "pipewire-devel"
        "pulseaudio-libs-devel"
        "wayland-devel"
        "glib2-devel"
        "redhat-rpm-config"
        "systemd"
        # Fedora 43/44 resolver workaround: dbus and systemd use rich
        # dependencies the pinned resolver skips. Remove these explicit
        # providers when it resolves those dependencies, then validate the locks.
        "dbus-broker"
        "util-linux-core"
        # systemd-rpm-macros supplies the application's lifecycle macros.
        "systemd-rpm-macros"
        # Fedora 43/44 resolver workaround: python3-setuptools satisfies a
        # conditional tooling dependency. Remove its explicit entry once the
        # resolver includes it automatically, then validate both image locks.
        "python3-setuptools"
      ]
    )
  );
}
