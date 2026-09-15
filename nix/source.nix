{ lib, ... }:
{
  perSystem = { ... }: {
    options.wayShell = lib.mkOption {
      type = lib.types.attrs;
      internal = true;
    };
    config.wayShell = {
      # Both package formats consume exactly this snapshot. In particular, a
      # development build must not silently contribute generated C or objects.
      source = lib.cleanSourceWith {
        src = ../.;
        name = "way-shell-source";
        filter =
          path: type:
          let
            name = baseNameOf path;
            relative = lib.removePrefix (toString ../. + "/") (toString path);
          in
          lib.cleanSourceFilter path type
          && !(builtins.elem name [
            ".cache"
            ".agents"
            ".codex"
            ".github"
            ".vscode"
            "build"
            "dist"
            "target"
            "gschemas.compiled"
          ])
          && !(lib.hasPrefix "result" name)
          && !(builtins.elem relative [
            "nix"
            "flake.nix"
            "flake.lock"
            "way-shell"
            "way-sh/way-sh"
            "gresources.c"
            "gresources.h"
            "compile_commands.json"
            ".gdb_history"
            "src/services/dbus_dbus.c"
            "src/services/dbus_dbus.h"
            "src/services/logind_service/logind_manager_dbus.c"
            "src/services/logind_service/logind_manager_dbus.h"
            "src/services/logind_service/logind_session_dbus.c"
            "src/services/logind_service/logind_session_dbus.h"
            "src/services/media_player_service/media_player_dbus.c"
            "src/services/media_player_service/media_player_dbus.h"
            "src/services/notifications_service/notifications_dbus.c"
            "src/services/notifications_service/notifications_dbus.h"
            "src/services/power_profiles_service/power_profiles_dbus.c"
            "src/services/power_profiles_service/power_profiles_dbus.h"
            "src/services/status_notifier_service/dbusmenu_dbus.c"
            "src/services/status_notifier_service/dbusmenu_dbus.h"
            "src/services/status_notifier_service/status_notifier_host_dbus.c"
            "src/services/status_notifier_service/status_notifier_host_dbus.h"
            "src/services/status_notifier_service/status_notifier_item_dbus.c"
            "src/services/status_notifier_service/status_notifier_item_dbus.h"
            "src/services/status_notifier_service/status_notifier_watcher_dbus.c"
            "src/services/status_notifier_service/status_notifier_watcher_dbus.h"
          ])
          && !(lib.hasSuffix ".o" name)
          && !(lib.hasSuffix ".d" name);
      };
      version = let
        cargoVersion = (builtins.fromTOML (builtins.readFile ../Cargo.toml)).workspace.package.version;
        rpmVersion = lib.removePrefix "Version: " (
          lib.findFirst (lib.hasPrefix "Version: ") (throw "way-shell.spec has no Version") (
            lib.splitString "\n" (builtins.readFile ../way-shell.spec)
          )
        );
      in assert lib.assertMsg (cargoVersion == rpmVersion) "Cargo and RPM versions disagree"; cargoVersion;
      schemaIds = map (match: builtins.elemAt match 0) (
        builtins.filter builtins.isList (
          builtins.split ''<schema[^>]*id="([^"]+)"'' (
            builtins.readFile ../data/org.ldelossa.way-shell.gschema.xml
          )
        )
      );
    };
  };
}
