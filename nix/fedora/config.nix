{ pkgs }:
{
  # The only supported-release list. Outputs, checks and update targets derive
  # from these keys. Run update-fedora-locks after changing this list.
  releases = {
    "43" = {
      baseUrl = "https://dl.fedoraproject.org/pub/fedora/linux/releases/43/Everything/x86_64/os";
    };
    "44" = {
      baseUrl = "https://dl.fedoraproject.org/pub/fedora/linux/releases/44/Everything/x86_64/os";
    };
  };
  arch = "x86_64";
  runtimePackages = [
    "basesystem"
    "bash"
    "coreutils"
    "findutils"
    "grep"
    "sed"
    "rpm"
    "dnf5"
    "systemd"
    # Fedora 43 protects systemd-udev from removal. Keep device management in
    # the OS baseline so DNF never treats it as an application dependency.
    # This explicit root can go when the base package set already requires it.
    "systemd-udev"
    "glib2"
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
        # systemd-rpm-macros supplies the application's lifecycle macros.
        "systemd-rpm-macros"
      ]
    )
  );
}
