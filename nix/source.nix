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
            "src/services/wayland/wlr-foreign-toplevel-management-unstable-v1.c"
            "src/services/wayland/wlr-foreign-toplevel-management-unstable-v1.h"
            "src/services/wayland/wlr-gamma-control-unstable-v1.c"
            "src/services/wayland/wlr-gamma-control-unstable-v1.h"
          ])
          && !(lib.hasSuffix ".o" name);
      };
      version = lib.removePrefix "Version: " (
        lib.findFirst (lib.hasPrefix "Version: ") (throw "way-shell.spec has no Version") (
          lib.splitString "\n" (builtins.readFile ../way-shell.spec)
        )
      );
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
