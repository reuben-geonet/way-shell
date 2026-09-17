{ config, pkgs }:
let
  package = config.packages.way-shell;
in
pkgs.testers.runNixOSTest {
  name = "way-shell-profile-installation";
  nodes.machine = {
    virtualisation.additionalPaths = [ package ];
    nix.settings.experimental-features = [ "nix-command" ];
    nix.settings.substituters = [ ];
    users.users.installer.isNormalUser = true;
    environment.systemPackages = [ pkgs.glib.bin ];
  };
  testScript = ''
    start_all()
    machine.wait_for_unit("multi-user.target")

    def as_user(command):
        import shlex
        return "su - installer -c " + shlex.quote(command)

    machine.fail(as_user("command -v way-shell"))
    machine.succeed(as_user("nix profile add ${package}"))
    machine.succeed(as_user("way-shell --help"))
    machine.succeed(as_user("way-sh --help"))
    machine.succeed(as_user(
        "GSETTINGS_BACKEND=memory gsettings --schemadir "
        "$HOME/.nix-profile/share/gsettings-schemas/${package.name}/glib-2.0/schemas "
        "list-recursively org.ldelossa.way-shell.system"
    ))
    machine.succeed(as_user("nix profile remove --all"))
    machine.fail(as_user("command -v way-shell"))
    machine.fail(as_user("command -v way-sh"))
    machine.succeed(as_user(
        "test ! -e $HOME/.nix-profile/share/gsettings-schemas/${package.name}"
    ))
  '';
}
